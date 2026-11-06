//! A row that scrolls out while the view is in the history updates the rows
//! in view without unpacking them again (`Grid::push_scrollback`). After every
//! line, the view must be the one a full refresh gives, which is what
//! `set_scrollback` does.

fn assert_same_as_a_full_refresh(parser: &vt100::Parser, context: &str) {
    let mut fresh = parser.screen().clone();
    let offset = fresh.scrollback();
    fresh.set_scrollback(0);
    fresh.set_scrollback(offset);
    assert_eq!(
        parser.screen().contents_formatted(),
        fresh.contents_formatted(),
        "{context}"
    );
    assert_eq!(parser.screen().contents(), fresh.contents(), "{context}");
}

/// Feeds numbered lines one by one for every combination of the height of the
/// screen, the size of the history and the place of the view: short of the
/// screen's height, at it, beyond it and at the oldest row, where the full
/// history drops its oldest row.
#[test]
fn the_view_follows_each_line_the_way_a_full_refresh_does() {
    for rows in [1u16, 2, 3, 5] {
        for history in [1usize, 2, 3, 7, 20] {
            for offset in [1usize, 2, 3, 4, 6, 10, 100] {
                let mut parser = vt100::Parser::new(rows, 12, history);
                for line in 0..60 {
                    if line % 7 == 3 {
                        parser.screen_mut().set_scrollback(offset);
                    }
                    parser.process(format!("line {line}\r\n").as_bytes());
                    assert_same_as_a_full_refresh(
                        &parser,
                        &format!(
                            "{rows} rows, {history} of history, view {offset}, line {line}"
                        ),
                    );
                }
            }
        }
    }
}

/// Several lines in one scroll (`CSI n S`, and output that wraps).
#[test]
fn a_scroll_of_several_lines_updates_the_view() {
    for history in [4usize, 30] {
        for offset in [1usize, 3, 5, 9] {
            let mut parser = vt100::Parser::new(5, 8, history);
            parser.process(b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\ng\r\nh\r\ni\r\n");
            parser.screen_mut().set_scrollback(offset);
            for count in [1, 2, 5, 9, 40] {
                parser.process(format!("\x1b[{count}S").as_bytes());
                assert_same_as_a_full_refresh(
                    &parser,
                    &format!(
                        "history {history}, view {offset}, scroll {count}"
                    ),
                );
                parser.process(b"0123456789abcdefghijklmnop");
                assert_same_as_a_full_refresh(
                    &parser,
                    &format!(
                        "history {history}, view {offset}, wrap {count}"
                    ),
                );
            }
        }
    }
}
