# vt100

> **This is the Soyeht fork of [doy/vt100-rust](https://github.com/doy/vt100-rust),
> based on version 0.16.2.** Its public API is the same as upstream's 0.16.2.
> Two things change, on the branch `panic-fixes` (the first alone is the
> branch `compact-scrollback`): how the scrollback is held in memory, and the
> panics the parser had on untrusted output.
>
> Upstream stores every row of the scrollback as a full-width row of 32-byte
> cells, whatever the line holds: with 10,000 lines of history, 80 to 200
> columns and ordinary output that is 27 to 71 MB for one terminal, and a
> window with many terminals has many of them. Here a row that scrolls out of
> the screen is packed (`src/packed.rs`): its text, the changes of its
> attributes and its wrap flag, in one allocation, with the blank end of the
> line left out. A row is unpacked into full cells only while it is in view:
> `Grid::set_scrollback` and the scrolling that happens while the view is in
> the history unpack the rows the view shows, and `visible_rows`, `Screen::cell`
> and everything else that reads rows go on handing out ordinary rows. The
> live screen is untouched and keeps full cells. The same 10,000 lines take
> about 1 MB for one terminal, and scrolling costs about the same CPU.
>
> The changes are in `src/packed.rs` (new), `src/grid.rs` (`scrollback` holds
> packed rows; the rows in view are unpacked next to it; the row that scrolls
> out of the screen is reused as the blank row that comes in), and small
> accessors in `src/cell.rs` and `src/row.rs`. `tests/scrollback.rs` compares
> every row of the history, at every scroll position, with the same output
> drawn on a screen too tall to scroll. While the view is in the history, a
> row that scrolls out updates the rows in view without unpacking them again
> (`tests/scrollback_view.rs`).
>
> Terminal output is untrusted, and 0.16.2 panics on a line that wraps on a
> screen of one row, a character wider than the screen, a resize that cuts a
> wide character in two, and a size of zero; these are fixed in `src/grid.rs`,
> `src/row.rs` and `src/screen.rs`. `tests/panic_fuzz.rs` is a seeded fuzzer
> that reads the screen back after each operation and reduces a failing seed
> to a short list of operations (`cargo test --test panic_fuzz`, and a longer
> run with `--ignored`, see the top of the file).
>
> To follow upstream, rebase the branch on the upstream tag you want.

This crate parses a terminal byte stream and provides an in-memory
representation of the rendered contents.

## Overview

This is essentially the terminal parser component of a graphical terminal
emulator pulled out into a separate crate. Although you can use this crate
to build a graphical terminal emulator, it also contains functionality
necessary for implementing terminal applications that want to run other
terminal applications - programs like `screen` or `tmux` for example.

## Synopsis

```rust
let mut parser = vt100::Parser::new(24, 80, 0);

let screen = parser.screen().clone();
parser.process(b"this text is \x1b[31mRED\x1b[m");
assert_eq!(
    parser.screen().cell(0, 13).unwrap().fgcolor(),
    vt100::Color::Idx(1),
);

let screen = parser.screen().clone();
parser.process(b"\x1b[3D\x1b[32mGREEN");
assert_eq!(
    parser.screen().contents_formatted(),
    &b"\x1b[?25h\x1b[m\x1b[H\x1b[Jthis text is \x1b[32mGREEN"[..],
);
assert_eq!(
    parser.screen().contents_diff(&screen),
    &b"\x1b[1;14H\x1b[32mGREEN"[..],
);
```
