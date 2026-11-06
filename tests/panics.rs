//! Terminal output is untrusted, so none of this may panic. Each test is a
//! sequence that did, reduced by `tests/panic_fuzz.rs`.

/// A character is not drawn on a screen too narrow for it.
#[test]
fn a_wide_character_on_a_screen_of_one_column_is_ignored() {
    let mut parser = vt100::Parser::new(1, 1, 0);
    parser.process("o".as_bytes());
    parser.process("\u{4e16}".as_bytes());
    parser.process("\u{3042}".as_bytes());
    assert_eq!(parser.screen().contents(), "o");
    parser.process("\u{4e16}\u{754c}x".as_bytes());
    assert_eq!(parser.screen().contents(), "x");

    let mut parser = vt100::Parser::new(3, 1, 5);
    parser.process("a\u{4e16}b\r\n\u{1f600}c".as_bytes());
    // the line wraps at every character, and contents() joins what wrapped
    assert_eq!(parser.screen().contents(), "ab\nc");
}

/// A line that wraps on a screen of one row scrolls it.
#[test]
fn a_line_that_wraps_on_a_screen_of_one_row_scrolls() {
    let mut parser = vt100::Parser::new(1, 1, 0);
    parser.process(b"kh");
    assert_eq!(parser.screen().contents(), "h");

    let mut parser = vt100::Parser::new(1, 5, 3);
    parser.process(b"0123456789abcdef");
    assert_eq!(parser.screen().contents(), "f");
    parser.screen_mut().set_scrollback(1);
    assert_eq!(parser.screen().contents(), "abcde");
    assert!(!parser.screen().row_wrapped(0));
}

/// A resize that cuts a wide character in two leaves no half of it behind.
#[test]
fn a_resize_that_cuts_a_wide_character_leaves_no_half_of_it() {
    let mut parser = vt100::Parser::new(1, 2, 0);
    parser.process("\u{4e16}".as_bytes());
    parser.screen_mut().set_size(1, 1);
    assert!(!parser.screen().cell(0, 0).unwrap().is_wide());
    parser.process(b"\x1b[X\x1b[K");
    parser.process(b"t");
    assert_eq!(parser.screen().contents(), "t");

    let mut parser = vt100::Parser::new(2, 4, 0);
    parser.process("ab\u{754c}".as_bytes());
    parser.screen_mut().set_size(2, 3);
    assert!(!parser.screen().cell(0, 2).unwrap().is_wide());
    assert_eq!(parser.screen().contents(), "ab");
    parser.process("\u{301}\u{4e16}".as_bytes());
    parser.process(b"\x1b[1;3H\x1b[2@\x1b[1P\x1b[1X");
    parser.screen_mut().set_size(1, 2);
    parser.process("\u{4e16}\u{4e16}".as_bytes());
}

/// A screen of no rows or no columns is a screen of one.
#[test]
fn a_size_of_zero_is_a_size_of_one() {
    for (rows, cols) in [(0, 0), (0, 5), (5, 0)] {
        let mut parser = vt100::Parser::new(rows, cols, 3);
        assert_eq!(parser.screen().size(), (rows.max(1), cols.max(1)));
        parser.process(
            "a\u{4e16}b\r\n\x1b[K\x1b[@\x1b[P\x1b[L\x1b[M".as_bytes(),
        );
        parser.screen_mut().set_size(0, 0);
        assert_eq!(parser.screen().size(), (1, 1));
        parser.process(b"xy\x1b[S\x1b[T");
        parser.screen_mut().set_size(rows, cols);
        parser.process(b"z");
    }
}

/// Inserting more cells than the row has left is the same as inserting what
/// fits, and does not take time for the rest.
#[test]
fn inserting_a_great_many_cells_is_bounded_by_the_row() {
    let mut parser = vt100::Parser::new(2, 10, 0);
    parser.process(b"abcdef\x1b[1;3H\x1b[65535@");
    assert_eq!(parser.screen().contents(), "ab");
    parser.process(b"X\x1b[1;1H\x1b[3@Y");
    assert_eq!(parser.screen().contents(), "Y  abX");
    parser.process(b"\x1b[1;9H\x1b[65535@");
    assert_eq!(parser.screen().contents(), "Y  abX");
}
