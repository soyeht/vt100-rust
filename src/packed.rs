//! Rows that have scrolled into the scrollback, held compactly.
//!
//! A [`Row`](crate::row::Row) holds every cell of its line, 32 bytes each,
//! whatever the line contains. The scrollback is read only a screenful at a
//! time and is never changed, so a row that enters it is packed: the text of
//! its cells and the changes of their attributes, in one allocation, with the
//! blank end of the line left out. A row is unpacked into full cells when it
//! is in view.
//!
//! The packed form of a row is a sequence of items, applying to the cells of
//! the row in order, starting with the default attributes:
//!
//! * `0x00`: one blank cell;
//! * `0x01..=0x7f`: one cell holding that ASCII character;
//! * `0x80`: the attributes of the cells that follow change, as the mode
//!   byte and then the foreground and background colors (a tag byte, then 0,
//!   1 or 3 bytes);
//! * `0x81`: one cell given in full, as its length byte (which carries the
//!   wide character flags) and then its contents;
//! * `0x82`: a run of blank cells, as a count in two bytes.
//!
//! The cells after the last item are blank with the default attributes.

use crate::attrs::{Attrs, Color};
use crate::cell::Cell;
use crate::row::Row;

const BLANK: u8 = 0x00;
const ASCII_END: u8 = 0x7f;
const ATTRS: u8 = 0x80;
const CELL: u8 = 0x81;
const BLANKS: u8 = 0x82;

/// The shortest run of blank cells that is written as a run.
const MIN_RUN: usize = 4;

const COLOR_DEFAULT: u8 = 0;
const COLOR_IDX: u8 = 1;
const COLOR_RGB: u8 = 2;

/// A row of the scrollback.
#[derive(Clone, Debug)]
pub struct PackedRow {
    data: Box<[u8]>,
    cols: u16,
    wrapped: bool,
}

impl PackedRow {
    /// Packs `row`.
    pub fn pack(row: &Row) -> Self {
        let cells = row.as_slice();
        let default = Attrs::default();
        let end = cells
            .iter()
            .rposition(|cell| !is_blank(cell) || cell.attrs() != &default)
            .map_or(0, |last| last + 1);
        // a cell takes a byte when it is ASCII with the attributes before it
        let mut data = Vec::with_capacity(end);
        let mut attrs = Attrs::default();
        let mut i = 0;
        while i < end {
            let cell = &cells[i];
            if cell.attrs() != &attrs {
                attrs = *cell.attrs();
                write_attrs(&mut data, &attrs);
            }
            if is_blank(cell) {
                let run = cells[i..end]
                    .iter()
                    .take_while(|other| {
                        is_blank(other) && other.attrs() == &attrs
                    })
                    .count();
                write_blanks(&mut data, run);
                i += run;
                continue;
            }
            write_cell(&mut data, cell);
            i += 1;
        }
        Self {
            data: data.into_boxed_slice(),
            cols: row.cols(),
            wrapped: row.wrapped(),
        }
    }

    /// Unpacks the row into the cells it was packed from.
    pub fn unpack(&self) -> Row {
        let mut cells = Vec::with_capacity(usize::from(self.cols));
        let mut attrs = Attrs::default();
        let mut data = &self.data[..];
        while let Some((&item, rest)) = data.split_first() {
            data = rest;
            match item {
                BLANK => cells.push(Cell::from_raw(0, &[], attrs)),
                1..=ASCII_END => {
                    cells.push(Cell::from_raw(1, &[item], attrs));
                }
                ATTRS => {
                    (attrs, data) = read_attrs(data);
                }
                CELL => {
                    let (&len, rest) = split_first(data);
                    let (contents, rest) =
                        rest.split_at(Cell::contents_len(len));
                    cells.push(Cell::from_raw(len, contents, attrs));
                    data = rest;
                }
                _ => {
                    let (count, rest) = data.split_at(2);
                    let count = u16::from_le_bytes([count[0], count[1]]);
                    cells.extend(
                        std::iter::repeat_with(|| {
                            Cell::from_raw(0, &[], attrs)
                        })
                        .take(usize::from(count)),
                    );
                    data = rest;
                }
            }
        }
        cells.resize(usize::from(self.cols), Cell::new());
        Row::from_cells(cells, self.wrapped)
    }
}

/// A blank cell is one with no contents and no flags: all there is to it is
/// its attributes.
fn is_blank(cell: &Cell) -> bool {
    cell.raw_len() == 0
}

fn write_cell(data: &mut Vec<u8>, cell: &Cell) {
    let contents = cell.raw_contents();
    if let ([byte], 1) = (contents, cell.raw_len()) {
        if (1..=ASCII_END).contains(byte) {
            data.push(*byte);
            return;
        }
    }
    data.push(CELL);
    data.push(cell.raw_len());
    data.extend_from_slice(contents);
}

fn write_blanks(data: &mut Vec<u8>, run: usize) {
    if run < MIN_RUN {
        data.resize(data.len() + run, BLANK);
        return;
    }
    // a row has at most as many cells as a u16 counts (see Size)
    let count = u16::try_from(run).unwrap_or(u16::MAX);
    data.push(BLANKS);
    data.extend_from_slice(&count.to_le_bytes());
}

fn write_attrs(data: &mut Vec<u8>, attrs: &Attrs) {
    data.push(ATTRS);
    data.push(attrs.mode);
    write_color(data, attrs.fgcolor);
    write_color(data, attrs.bgcolor);
}

fn write_color(data: &mut Vec<u8>, color: Color) {
    match color {
        Color::Default => data.push(COLOR_DEFAULT),
        Color::Idx(i) => data.extend_from_slice(&[COLOR_IDX, i]),
        Color::Rgb(r, g, b) => data.extend_from_slice(&[COLOR_RGB, r, g, b]),
    }
}

fn read_attrs(data: &[u8]) -> (Attrs, &[u8]) {
    let (&mode, data) = split_first(data);
    let (fgcolor, data) = read_color(data);
    let (bgcolor, data) = read_color(data);
    (
        Attrs {
            fgcolor,
            bgcolor,
            mode,
        },
        data,
    )
}

fn read_color(data: &[u8]) -> (Color, &[u8]) {
    let (&tag, data) = split_first(data);
    match tag {
        COLOR_DEFAULT => (Color::Default, data),
        COLOR_IDX => {
            let (&i, data) = split_first(data);
            (Color::Idx(i), data)
        }
        _ => {
            let (rgb, data) = data.split_at(3);
            (Color::Rgb(rgb[0], rgb[1], rgb[2]), data)
        }
    }
}

// the data was written by `PackedRow::pack`, so there is always another byte
// where one is read
#[allow(clippy::missing_panics_doc)]
fn split_first(data: &[u8]) -> (&u8, &[u8]) {
    data.split_first().expect("packed row ends inside an item")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng as _, SeedableRng as _};

    fn random_color(rng: &mut rand::rngs::StdRng) -> Color {
        match rng.random_range(0..3) {
            0 => Color::Default,
            1 => Color::Idx(rng.random()),
            _ => Color::Rgb(rng.random(), rng.random(), rng.random()),
        }
    }

    fn random_cell(rng: &mut rand::rngs::StdRng) -> Cell {
        let attrs = if rng.random_range(0..3) == 0 {
            Attrs {
                fgcolor: random_color(rng),
                bgcolor: random_color(rng),
                mode: rng.random_range(0..32),
            }
        } else {
            Attrs::default()
        };
        // 0: blank, 1: ASCII, 2: any character, with combining marks, 3: a
        // wide character and its continuation, 4: a continuation alone
        match rng.random_range(0..5) {
            0 => Cell::from_raw(0, &[], attrs),
            1 => {
                let byte = rng.random_range(0..=0x7f_u8);
                Cell::from_raw(1, &[byte], attrs)
            }
            2 => {
                let text = "é\u{301}\u{200d}ß日🙂";
                let end = rng.random_range(1..=text.chars().count());
                let text: String = text.chars().take(end).collect();
                let bytes = text.as_bytes();
                let len = u8::try_from(bytes.len()).unwrap();
                Cell::from_raw(len, bytes, attrs)
            }
            3 => {
                let len = 3 | IS_WIDE;
                Cell::from_raw(len, "日".as_bytes(), attrs)
            }
            _ => Cell::from_raw(IS_WIDE_CONTINUATION, &[], attrs),
        }
    }

    const IS_WIDE: u8 = 0b1000_0000;
    const IS_WIDE_CONTINUATION: u8 = 0b0100_0000;

    fn assert_same(row: &Row, again: &Row) {
        assert_eq!(row.cols(), again.cols());
        assert_eq!(row.wrapped(), again.wrapped());
        for (col, (a, b)) in row.cells().zip(again.cells()).enumerate() {
            assert_eq!(a, b, "column {col}");
        }
    }

    #[test]
    fn a_row_is_the_same_after_it_is_packed_and_unpacked() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        for round in 0..3000 {
            let cols = rng.random_range(1..=120_u16);
            let blank_tail = rng.random_range(0..=cols);
            let cells: Vec<Cell> = (0..cols)
                .map(|col| {
                    if col >= cols - blank_tail {
                        Cell::new()
                    } else {
                        random_cell(&mut rng)
                    }
                })
                .collect();
            let row = Row::from_cells(cells, round % 2 == 0);
            assert_same(&row, &PackedRow::pack(&row).unpack());
        }
    }

    #[test]
    fn a_character_the_marker_bytes_could_be_taken_for_is_kept() {
        // a NUL character is not a blank cell, and a control character is
        // not an item of the packed form
        let attrs = Attrs::default();
        let cells = vec![
            Cell::from_raw(1, &[0], attrs),
            Cell::from_raw(1, &[0x1b], attrs),
            Cell::from_raw(0, &[], attrs),
        ];
        let row = Row::from_cells(cells, false);
        assert_same(&row, &PackedRow::pack(&row).unpack());
    }

    #[test]
    fn a_row_of_text_takes_a_byte_a_character_and_a_blank_end_takes_nothing()
    {
        let mut cells = vec![Cell::new(); 200];
        for (i, byte) in b"some typical log line of sixty characters"
            .iter()
            .enumerate()
        {
            cells[i] = Cell::from_raw(1, &[*byte], Attrs::default());
        }
        let row = Row::from_cells(cells, false);
        let packed = PackedRow::pack(&row);
        assert_eq!(packed.data.len(), 41);
        assert_same(&row, &packed.unpack());
    }

    #[test]
    fn a_blank_end_with_a_background_is_a_run_and_keeps_its_attributes() {
        let blue = Attrs {
            bgcolor: Color::Idx(4),
            ..Attrs::default()
        };
        let mut cells = vec![Cell::from_raw(0, &[], blue); 200];
        cells[0] = Cell::from_raw(1, b"x", Attrs::default());
        let row = Row::from_cells(cells, true);
        let packed = PackedRow::pack(&row);
        assert!(packed.data.len() < 16);
        assert_same(&row, &packed.unpack());
    }
}
