// Times output that arrives while the view is scrolled back into the history,
// which scrolls the screen once per line. Run with `--release`.
use std::time::Instant;

fn main() {
    let lines = 200_000;
    for (name, offset) in
        [("view at the bottom", 0), ("view scrolled back", 300)]
    {
        let mut parser = vt100::Parser::new(50, 160, 10_000);
        for i in 0..2_000 {
            parser.process(format!("warm up line {i}\r\n").as_bytes());
        }
        parser.screen_mut().set_scrollback(offset);
        let started = Instant::now();
        for i in 0..lines {
            parser.process(
                format!("\x1b[32mline {i}\x1b[0m: the quick brown fox\r\n")
                    .as_bytes(),
            );
        }
        let elapsed = started.elapsed();
        println!(
            "{name}: {:.2} us per line",
            elapsed.as_secs_f64() * 1e6 / f64::from(lines)
        );
    }
}
