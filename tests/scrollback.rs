//! The scrollback holds rows packed (see `src/packed.rs`). These tests check
//! that what is read back is what was drawn: every row that scrolls out of a
//! small screen is compared with the same row of a screen tall enough to
//! never scroll, which keeps its rows as they were drawn.

use rand::{Rng as _, SeedableRng as _};

/// What a cell shows, and how.
type Shown = (String, bool, bool, vt100::Color, vt100::Color, [bool; 5]);

fn shown(cell: &vt100::Cell) -> Shown {
    (
        cell.contents().to_string(),
        cell.is_wide(),
        cell.is_wide_continuation(),
        cell.fgcolor(),
        cell.bgcolor(),
        [
            cell.bold(),
            cell.dim(),
            cell.italic(),
            cell.underline(),
            cell.inverse(),
        ],
    )
}

/// The cells and the wrap flag of the row `row` of what `screen` shows.
fn row_of(screen: &vt100::Screen, row: u16) -> (Vec<Shown>, bool) {
    let (_, cols) = screen.size();
    let cells = (0..cols)
        .map(|col| shown(screen.cell(row, col).unwrap()))
        .collect();
    (cells, screen.row_wrapped(row))
}

const SCREEN_ROWS: u16 = 5;
const TALL_ROWS: u16 = 1000;

/// Feeds `bytes` to a small screen with `scrollback` rows of history and to a
/// tall one, and checks that every view of the small one, at every position
/// in its history, shows the rows of the tall one it should.
fn assert_history_is_what_was_drawn(
    bytes: &[u8],
    cols: u16,
    scrollback: usize,
) {
    let mut small = vt100::Parser::new(SCREEN_ROWS, cols, scrollback);
    let mut tall = vt100::Parser::new(TALL_ROWS, cols, 0);
    small.process(bytes);
    tall.process(bytes);

    // the tall screen never scrolled, so its cursor row counts the lines drawn
    let drawn = usize::from(tall.screen().cursor_position().0) + 1;
    assert!(
        drawn < usize::from(TALL_ROWS),
        "the output fits the tall screen"
    );
    let scrolled = drawn.saturating_sub(usize::from(SCREEN_ROWS));

    small.screen_mut().set_scrollback(usize::MAX);
    let kept = small.screen().scrollback();
    assert_eq!(kept, scrolled.min(scrollback));
    for offset in 0..=kept {
        small.screen_mut().set_scrollback(offset);
        for row in 0..SCREEN_ROWS {
            let line = scrolled - offset + usize::from(row);
            assert_eq!(
                row_of(small.screen(), row),
                row_of(tall.screen(), u16::try_from(line).unwrap()),
                "{offset} rows back, row {row}"
            );
        }
    }
}

#[test]
fn every_attribute_and_color_comes_back() {
    let mut bytes = vec![];
    for code in ["1", "2", "3", "4", "7", "1;3;4;7"] {
        bytes.extend(format!("\x1b[{code}mmode {code}\x1b[m\r\n").bytes());
    }
    for color in (30..=37).chain(90..=97) {
        bytes.extend(format!("\x1b[{color}mfg {color}\x1b[m\r\n").bytes());
    }
    for color in (40..=47).chain(100..=107) {
        bytes.extend(format!("\x1b[{color}mbg {color}\x1b[m\r\n").bytes());
    }
    for index in [0, 7, 8, 15, 16, 100, 231, 232, 255] {
        bytes.extend(
            format!(
                "\x1b[38;5;{index};48;5;{}mindexed {index}\x1b[m\r\n",
                255 - index
            )
            .bytes(),
        );
    }
    for rgb in [(0, 0, 0), (1, 2, 3), (255, 128, 7), (255, 255, 255)] {
        bytes.extend(
            format!(
                "\x1b[38;2;{};{};{};48;2;{};{};{}mrgb\x1b[m\r\n",
                rgb.0, rgb.1, rgb.2, rgb.2, rgb.1, rgb.0
            )
            .bytes(),
        );
    }
    // several attributes in one row, changing along it
    bytes.extend(b"a\x1b[1mb\x1b[31mc\x1b[44md\x1b[0me\x1b[4mf\x1b[mg\r\n");
    assert_history_is_what_was_drawn(&bytes, 40, 1000);
}

#[test]
fn a_blank_end_of_a_row_keeps_its_background() {
    // the rest of the line is erased with a background color, and so are
    // a few cells in the middle
    let bytes = b"text\x1b[44m\x1b[K\x1b[m\r\n\
        \x1b[41m  \x1b[m gap \x1b[42m\x1b[2X\x1b[m\r\n\
        \x1b[7m\x1b[K\x1b[m\r\n\
        \x1b[2Kblank\r\n";
    assert_history_is_what_was_drawn(bytes, 30, 100);
}

#[test]
fn wide_and_combining_characters_come_back() {
    let mut bytes = vec![];
    bytes.extend("日本語のテキスト\r\n".bytes());
    bytes.extend(
        "e\u{301} a\u{308}\u{323} 🙂 👨\u{200d}👩\u{200d}👧\r\n".bytes(),
    );
    // a wide character at the margin moves to the next row
    bytes.extend("x".repeat(9).bytes());
    bytes.extend("日\r\n".bytes());
    bytes.extend("\x1b[31m全角\x1b[44m混在\x1b[m ascii\r\n".bytes());
    assert_history_is_what_was_drawn(&bytes, 10, 100);
}

#[test]
fn wrapped_lines_keep_their_wrap_flag() {
    let mut bytes = vec![];
    for length in [3, 10, 11, 25, 30, 31, 95] {
        bytes.extend("w".repeat(length).bytes());
        bytes.extend(b"\r\n");
    }
    assert_history_is_what_was_drawn(&bytes, 10, 100);
}

#[test]
fn a_row_with_a_character_that_is_not_ascii_or_printable_comes_back() {
    let bytes = "tab\there\r\n\u{a0}nbsp \u{7f}\u{1f}x\r\n".as_bytes();
    assert_history_is_what_was_drawn(bytes, 30, 100);
}

#[test]
fn random_output_comes_back() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x5c01);
    let pieces = [
        "plain words ",
        "日本語",
        "e\u{301}",
        "🙂",
        "\t",
        "x",
        "   ",
        "0123456789",
    ];
    let sgr = [
        "\x1b[m",
        "\x1b[1m",
        "\x1b[2m",
        "\x1b[3m",
        "\x1b[4m",
        "\x1b[7m",
        "\x1b[31m",
        "\x1b[92m",
        "\x1b[44m",
        "\x1b[38;5;200m",
        "\x1b[48;2;10;20;30m",
        "\x1b[22m",
        "\x1b[39m",
        "\x1b[49m",
    ];
    for round in 0..60 {
        let cols = rng.random_range(4..=40_u16);
        let mut bytes = String::new();
        for _ in 0..rng.random_range(10..80) {
            match rng.random_range(0..10) {
                0..=3 => {
                    bytes.push_str(pieces[rng.random_range(0..pieces.len())])
                }
                4..=6 => bytes.push_str(sgr[rng.random_range(0..sgr.len())]),
                7 => bytes.push_str("\x1b[K"),
                _ => bytes.push_str("\r\n"),
            }
        }
        bytes.push_str("\r\n");
        let scrollback = if round % 3 == 0 { 7 } else { 500 };
        assert_history_is_what_was_drawn(bytes.as_bytes(), cols, scrollback);
    }
}

fn numbered(from: usize, to: usize) -> Vec<u8> {
    (from..to)
        .flat_map(|line| format!("line {line}\r\n").into_bytes())
        .collect()
}

/// The text of the row `row` of what `screen` shows, without its blank end.
fn text_of(screen: &vt100::Screen, row: u16) -> String {
    screen
        .rows(0, screen.size().1)
        .nth(usize::from(row))
        .unwrap()
        .trim_end()
        .to_string()
}

#[test]
fn the_oldest_rows_are_dropped_when_the_history_is_full() {
    let mut parser = vt100::Parser::new(5, 20, 10);
    parser.process(&numbered(0, 50));
    // 50 lines and the cursor's line: 46 rows scrolled out, 10 are kept
    parser.screen_mut().set_scrollback(usize::MAX);
    assert_eq!(parser.screen().scrollback(), 10);
    assert_eq!(text_of(parser.screen(), 0), "line 36");
    parser.screen_mut().set_scrollback(1);
    assert_eq!(text_of(parser.screen(), 0), "line 45");
    assert_eq!(text_of(parser.screen(), 1), "line 46");
    parser.screen_mut().set_scrollback(0);
    assert_eq!(text_of(parser.screen(), 0), "line 46");
    assert_eq!(text_of(parser.screen(), 3), "line 49");
}

#[test]
fn a_view_in_the_history_stays_on_its_rows_while_output_arrives() {
    let mut parser = vt100::Parser::new(5, 20, 100);
    parser.process(&numbered(0, 30));
    parser.screen_mut().set_scrollback(8);
    let before: Vec<String> =
        (0..5).map(|row| text_of(parser.screen(), row)).collect();
    assert_eq!(before[0], "line 18");

    parser.process(&numbered(30, 40));
    assert_eq!(parser.screen().scrollback(), 18);
    let after: Vec<String> =
        (0..5).map(|row| text_of(parser.screen(), row)).collect();
    assert_eq!(before, after);

    // back to the live screen, and into the history again
    parser.screen_mut().set_scrollback(0);
    assert_eq!(text_of(parser.screen(), 3), "line 39");
    parser.screen_mut().set_scrollback(18);
    assert_eq!(
        before,
        (0..5)
            .map(|row| text_of(parser.screen(), row))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_view_at_the_oldest_row_moves_when_output_pushes_a_row_out() {
    let mut parser = vt100::Parser::new(5, 20, 12);
    parser.process(&numbered(0, 30));
    parser.screen_mut().set_scrollback(usize::MAX);
    assert_eq!(parser.screen().scrollback(), 12);
    assert_eq!(text_of(parser.screen(), 0), "line 14");

    parser.process(&numbered(30, 33));
    assert_eq!(parser.screen().scrollback(), 12);
    assert_eq!(text_of(parser.screen(), 0), "line 17");
}

#[test]
fn the_history_in_view_follows_a_resize() {
    let mut parser = vt100::Parser::new(5, 20, 100);
    parser.process(&numbered(0, 30));
    parser.screen_mut().set_scrollback(3);
    assert_eq!(text_of(parser.screen(), 0), "line 23");

    // more rows: the view shows the same three history rows first
    parser.screen_mut().set_size(8, 20);
    assert_eq!(parser.screen().scrollback(), 3);
    let rows: Vec<String> =
        (0..8).map(|row| text_of(parser.screen(), row)).collect();
    assert_eq!(rows[..3], ["line 23", "line 24", "line 25"]);

    // a view deeper than the screen is all history, and has more rows to
    // fill when the screen grows, fewer when it shrinks
    parser.screen_mut().set_scrollback(20);
    let rows: Vec<String> =
        (0..8).map(|row| text_of(parser.screen(), row)).collect();
    let want: Vec<String> =
        (6..14).map(|line| format!("line {line}")).collect();
    assert_eq!(rows, want);
    parser.screen_mut().set_size(11, 20);
    let rows: Vec<String> =
        (0..11).map(|row| text_of(parser.screen(), row)).collect();
    let want: Vec<String> =
        (6..17).map(|line| format!("line {line}")).collect();
    assert_eq!(rows, want);
    parser.screen_mut().set_size(2, 20);
    let rows: Vec<String> =
        (0..2).map(|row| text_of(parser.screen(), row)).collect();
    assert_eq!(rows, ["line 6", "line 7"]);
}

#[test]
fn the_formatted_contents_of_a_view_in_the_history_draw_the_same_rows() {
    let mut small = vt100::Parser::new(5, 20, 100);
    let mut again = vt100::Parser::new(5, 20, 0);
    let bytes = b"\x1b[1;31mred bold\x1b[m\r\n\x1b[4mplain\x1b[m\r\n";
    small.process(bytes);
    small.process(&numbered(0, 10));
    small.screen_mut().set_scrollback(usize::MAX);
    let formatted = small.screen().contents_formatted();
    again.process(&formatted);
    assert_eq!(text_of(again.screen(), 0), text_of(small.screen(), 0));
    assert_eq!(again.screen().cell(0, 0), small.screen().cell(0, 0));
}
