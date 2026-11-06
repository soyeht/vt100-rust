//! A deterministic fuzzer for panics.
//!
//! Terminal output is untrusted, so no byte sequence, resize or scrollback
//! offset may make the parser panic. This test generates operations from a
//! seed (escape-sequence fragments of every kind the parser understands,
//! wide, combining and emoji characters, scroll regions, the alternate
//! screen, resizes, scrollback offsets, random bytes) and, after each one,
//! reads the screen back through every public accessor. A failing seed is
//! reduced to a short operation list and printed.
//!
//! `cargo test --test panic_fuzz` runs a few seconds' worth of seeds.
//! `VT100_FUZZ_SEEDS=200000 cargo test --release --test panic_fuzz -- --ignored`
//! runs a longer one; `VT100_FUZZ_FIRST_SEED` moves its start.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

#[derive(Clone, Debug)]
enum Op {
    Process(Vec<u8>),
    SetSize(u16, u16),
    SetScrollback(usize),
}

#[derive(Clone, Debug)]
struct Case {
    rows: u16,
    cols: u16,
    history: usize,
    ops: Vec<Op>,
}

const TEXT: &[&str] = &[
    "a",
    "hello",
    "x",
    "~",
    "0123456789",
    " ",
    "\u{4e16}\u{754c}", // wide
    "\u{ff21}",
    "e\u{301}",
    "\u{301}",
    "\u{200d}",
    "\u{1f468}\u{200d}\u{1f469}",
    "\u{2764}\u{fe0f}",
    "\u{1f600}",
    "\u{ff}",
    "\u{0}",
    "\u{7f}",
    "\u{a0}",
    "\u{200b}",
    "\u{3042}\u{3099}",
];

const CONTROLS: &[&str] = &[
    "\r", "\n", "\r\n", "\t", "\u{8}", "\u{b}", "\u{c}", "\u{7}", "\u{e}",
    "\u{f}", "\u{0}", "\u{7f}", "\u{1b}7", "\u{1b}8", "\u{1b}M", "\u{1b}D",
    "\u{1b}E", "\u{1b}c", "\u{1b}=", "\u{1b}>", "\u{1b}H",
];

const CSI_FINALS: &[u8] = b"ABCDEFGHJKLMPSTXZ@`adefghlmnrsu";

const PRIVATE_MODES: &[u32] = &[
    1, 6, 7, 25, 47, 1000, 1002, 1003, 1005, 1006, 1047, 1048, 1049, 2004,
];

fn param(rng: &mut Rng) -> u32 {
    match rng.below(10) {
        0 => 0,
        1..=4 => rng.range(1, 5) as u32,
        5..=6 => rng.range(1, 45) as u32,
        7 => rng.range(40, 300) as u32,
        8 => 65535,
        _ => 70000 + rng.below(100_000) as u32,
    }
}

fn csi(rng: &mut Rng) -> Vec<u8> {
    let mut out = b"\x1b[".to_vec();
    let final_byte = if rng.below(5) == 0 {
        // SGR, the long forms included
        let n = rng.range(1, 6);
        let mut parts: Vec<String> = vec![];
        for _ in 0..n {
            match rng.below(6) {
                0 => parts.push(format!("38;5;{}", rng.below(300))),
                1 => parts.push(format!(
                    "48;2;{};{};{}",
                    rng.below(300),
                    rng.below(300),
                    rng.below(300)
                )),
                2 => parts.push(format!("{}", rng.below(110))),
                3 => parts.push("38;2".to_string()),
                4 => parts.push("0".to_string()),
                _ => {
                    parts.push(format!("{}:{}", rng.below(60), rng.below(60)))
                }
            }
        }
        out.extend(parts.join(";").as_bytes());
        b'm'
    } else {
        let f = *rng.pick(CSI_FINALS);
        if (f == b'h' || f == b'l') && rng.below(3) != 0 {
            out.push(b'?');
            out.extend(rng.pick(PRIVATE_MODES).to_string().as_bytes());
        } else {
            let n = rng.below(4);
            let params: Vec<String> = (0..n)
                .map(|_| {
                    if rng.below(6) == 0 {
                        String::new()
                    } else {
                        param(rng).to_string()
                    }
                })
                .collect();
            out.extend(params.join(";").as_bytes());
        }
        f
    };
    out.push(final_byte);
    out
}

fn fragment(rng: &mut Rng) -> Vec<u8> {
    match rng.below(20) {
        0..=6 => {
            let n = rng.range(1, 4);
            (0..n)
                .flat_map(|_| rng.pick(TEXT).bytes().collect::<Vec<_>>())
                .collect()
        }
        7..=9 => rng.pick(CONTROLS).as_bytes().to_vec(),
        10..=16 => csi(rng),
        17 => {
            // a long line, to wrap
            let n = rng.range(1, 90);
            (0..n).map(|_| b'a' + rng.below(26) as u8).collect()
        }
        18 => {
            let n = rng.range(1, 8);
            (0..n).map(|_| rng.below(256) as u8).collect()
        }
        _ => {
            // a scroll region, then output to scroll in it
            let top = rng.range(1, 8);
            let bottom = rng.range(1, 30);
            let mut v = format!("\x1b[{top};{bottom}r").into_bytes();
            let lines = rng.range(0, 12);
            for _ in 0..lines {
                v.extend(b"line\r\n");
            }
            v
        }
    }
}

fn gen_case(seed: u64) -> Case {
    let mut rng =
        Rng(seed.wrapping_mul(0x2545_f491_4f6c_dd1d).wrapping_add(1));
    // a size of zero is not a size, but a caller that takes it from a window
    // that is being closed can hand it over
    let low = if rng.below(25) == 0 { 0 } else { 1 };
    let rows = rng.range(low, 24) as u16;
    let cols = rng.range(low, 40) as u16;
    let history = *rng.pick(&[0usize, 1, 2, 5, 10, 50, 200]);
    let n_ops = rng.range(3, 40);
    let mut ops = vec![];
    for _ in 0..n_ops {
        match rng.below(12) {
            0 => {
                let low = if rng.below(25) == 0 { 0 } else { 1 };
                ops.push(Op::SetSize(
                    rng.range(low, 30) as u16,
                    rng.range(low, 40) as u16,
                ));
            }
            1 | 2 => ops.push(Op::SetScrollback(rng.below(260) as usize)),
            _ => {
                let n = rng.range(1, 14);
                let bytes = (0..n).flat_map(|_| fragment(&mut rng)).collect();
                ops.push(Op::Process(bytes));
            }
        }
    }
    Case {
        rows,
        cols,
        history,
        ops,
    }
}

// reads the screen back through every public accessor, as a client does
fn read_back(screen: &vt100::Screen, prev: &vt100::Screen) {
    let (rows, cols) = screen.size();
    let _ = screen.contents();
    let _ = screen.contents_formatted();
    let _ = screen.state_formatted();
    let _ = screen.contents_diff(prev);
    let _ = screen.state_diff(prev);
    let _ = screen.attributes_formatted();
    let _ = screen.cursor_state_formatted();
    let _ = screen.input_mode_formatted();
    let _ = screen.cursor_position();
    let _ = screen.rows(0, cols).count();
    let _ = screen.rows_formatted(0, cols).count();
    let _ = screen.rows_diff(prev, 0, cols).count();
    let _ = screen.contents_between(0, 0, rows.saturating_sub(1), cols);
    for row in 0..rows {
        let _ = screen.row_wrapped(row);
        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col) {
                let _ = cell.contents();
                {
                    // a wide cell is followed by its continuation, and a
                    // continuation follows a wide cell
                    if cell.is_wide() {
                        assert!(
                            screen.cell(row, col + 1).is_some_and(
                                vt100::Cell::is_wide_continuation
                            ),
                            "wide cell without a continuation"
                        );
                    }
                    if cell.is_wide_continuation() {
                        assert!(
                            col > 0
                                && screen
                                    .cell(row, col - 1)
                                    .is_some_and(vt100::Cell::is_wide),
                            "continuation without a wide cell"
                        );
                    }
                }
            }
        }
    }
}

// runs a case; Err carries the panic location (file:line) and message
fn run(case: &Case) -> Result<(), (String, String)> {
    thread_local! {
        static LOCATION: RefCell<String> = const { RefCell::new(String::new()) };
    }
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let location = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            LOCATION.with(|c| *c.borrow_mut() = location);
        }));
    });
    let reached = std::cell::Cell::new(0);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut parser =
            vt100::Parser::new(case.rows, case.cols, case.history);
        for (index, op) in case.ops.iter().enumerate() {
            reached.set(index);
            let prev = parser.screen().clone();
            match op {
                Op::Process(bytes) => parser.process(bytes),
                Op::SetSize(rows, cols) => {
                    parser.screen_mut().set_size(*rows, *cols)
                }
                Op::SetScrollback(n) => {
                    parser.screen_mut().set_scrollback(*n)
                }
            }
            read_back(parser.screen(), &prev);
            // the incremental view of the scrollback must be the one a full
            // refresh gives
            let mut fresh = parser.screen().clone();
            let offset = fresh.scrollback();
            fresh.set_scrollback(0);
            fresh.set_scrollback(offset);
            assert_eq!(
                parser.screen().contents_formatted(),
                fresh.contents_formatted(),
                "scrollback view differs from a full refresh"
            );
        }
    }));
    result.map_err(|payload| {
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        let location = LOCATION.with(|c| c.borrow().clone());
        (location, format!("op {}: {message}", reached.get()))
    })
}

// reduces a failing case, keeping the same panic location
fn minimize(mut case: Case, location: &str, message: &str) -> Case {
    // nothing after the operation that panicked matters
    if let Some(index) = message
        .strip_prefix("op ")
        .and_then(|m| m.split(':').next())
        .and_then(|n| n.parse::<usize>().ok())
    {
        case.ops.truncate(index + 1);
    }
    let still = |c: &Case| matches!(run(c), Err((l, _)) if l == location);
    loop {
        let mut changed = false;
        // fewer ops
        let mut i = 0;
        while i < case.ops.len() {
            let mut c = case.clone();
            c.ops.remove(i);
            if still(&c) {
                case = c;
                changed = true;
            } else {
                i += 1;
            }
        }
        // fewer bytes, in chunks of decreasing size
        for oi in 0..case.ops.len() {
            let Op::Process(bytes) = &case.ops[oi] else {
                continue;
            };
            let mut chunk = bytes.len().max(1);
            while chunk >= 1 {
                let mut start = 0;
                loop {
                    let Op::Process(bytes) = &case.ops[oi] else {
                        break;
                    };
                    if start >= bytes.len() {
                        break;
                    }
                    let mut c = case.clone();
                    let Op::Process(b) = &mut c.ops[oi] else {
                        break;
                    };
                    b.drain(start..(start + chunk).min(bytes.len()));
                    if still(&c) {
                        case = c;
                        changed = true;
                    } else {
                        start += chunk;
                    }
                }
                chunk /= 2;
            }
        }
        // smaller numbers
        let try_case = |case: &mut Case, f: &dyn Fn(&mut Case)| {
            let mut c = case.clone();
            f(&mut c);
            if still(&c) {
                *case = c;
                true
            } else {
                false
            }
        };
        for n in [1u16, 2, 3, 4, 8] {
            if case.rows > n {
                changed |= try_case(&mut case, &|c| c.rows = n);
            }
            if case.cols > n {
                changed |= try_case(&mut case, &|c| c.cols = n);
            }
        }
        for h in [0usize, 1, 2, 5] {
            if case.history > h {
                changed |= try_case(&mut case, &|c| c.history = h);
            }
        }
        for oi in 0..case.ops.len() {
            match case.ops[oi].clone() {
                Op::SetSize(r, c) => {
                    for (nr, nc) in [(1, c), (r, 1), (2, c), (r, 2)] {
                        if nr <= r && nc <= c && (nr, nc) != (r, c) {
                            changed |= try_case(&mut case, &|k| {
                                k.ops[oi] = Op::SetSize(nr, nc)
                            });
                        }
                    }
                }
                Op::SetScrollback(n) => {
                    for m in [1usize, 2, 3, n / 2] {
                        if m < n {
                            changed |= try_case(&mut case, &|k| {
                                k.ops[oi] = Op::SetScrollback(m)
                            });
                        }
                    }
                }
                Op::Process(_) => {}
            }
        }
        if !changed {
            return case;
        }
    }
}

fn describe(case: &Case) -> String {
    let mut s = format!(
        "Parser::new({}, {}, {})\n",
        case.rows, case.cols, case.history
    );
    for op in &case.ops {
        match op {
            Op::Process(b) => {
                s += &format!("  process({:?})\n", String::from_utf8_lossy(b))
            }
            Op::SetSize(r, c) => s += &format!("  set_size({r}, {c})\n"),
            Op::SetScrollback(n) => s += &format!("  set_scrollback({n})\n"),
        }
    }
    s
}

fn fuzz(first: u64, seeds: u64) {
    let report = std::env::var("VT100_FUZZ_REPORT").is_ok();
    let mut failures: BTreeMap<String, (u64, u64, Case, String)> =
        BTreeMap::new();
    let mut total = 0;
    for seed in first..first + seeds {
        let case = gen_case(seed);
        let started = std::time::Instant::now();
        let outcome = run(&case);
        if report && started.elapsed().as_millis() > 300 {
            eprintln!("slow seed {seed}: {:?}", started.elapsed());
        }
        if let Err((location, message)) = outcome {
            total += 1;
            let entry = failures.entry(location.clone());
            let e = entry.or_insert_with(|| (0, seed, case.clone(), message));
            e.0 += 1;
        }
    }
    if report {
        println!("{total} of {seeds} seeds failed");
    }
    let mut text = String::new();
    for (location, (count, seed, case, message)) in failures {
        let small = minimize(case, &location, &message);
        text += &format!(
            "{location}: {count} seeds (first {seed}): {message}\n{}\n",
            describe(&small)
        );
    }
    if report {
        println!("{text}");
    } else {
        assert!(total == 0, "{total} of {seeds} seeds failed\n{text}");
    }
}

#[test]
fn no_panic_on_generated_terminal_output() {
    let seeds = if cfg!(debug_assertions) { 300 } else { 4000 };
    fuzz(0, seeds);
}

#[test]
#[ignore = "a longer run; see the top of the file"]
fn no_panic_on_generated_terminal_output_long() {
    let seeds = std::env::var("VT100_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);
    let first = std::env::var("VT100_FUZZ_FIRST_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    fuzz(first, seeds);
}
