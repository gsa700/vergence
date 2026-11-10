//! The stress test's jobs (toolkit milestone 1, step 5): programs that
//! write to the terminal they run in as fast as it takes the bytes, and
//! say how fast that was in a report file, one line a second.
//!
//!     flood colour  REPORT    lines, a different 24-bit colour in every cell
//!     flood cursor  REPORT    batches of 250 writes at random places
//!     flood unicode REPORT    lines of wide, combining and symbol characters
//!     flood time    REPORT COMMAND [ARGS]   COMMAND again and again, each run timed
//!
//! The terminal's speed is the limit: a write blocks while the terminal
//! has not read, so the rate reported is the rate it parsed at.

use std::{
    fs::OpenOptions,
    io::{self, Write},
    time::{Duration, Instant},
};

/// A small generator of its own (xorshift), so the jobs need nothing.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// The terminal's size in cells (columns, rows); 80 by 24 if it will not say.
fn size() -> (u64, u64) {
    #[repr(C)]
    struct WinSize {
        rows: u16,
        cols: u16,
        x: u16,
        y: u16,
    }
    #[cfg(target_os = "linux")]
    const TIOCGWINSZ: std::ffi::c_ulong = 0x5413;
    #[cfg(not(target_os = "linux"))]
    const TIOCGWINSZ: std::ffi::c_ulong = 0x4008_7468;
    unsafe extern "C" {
        fn ioctl(fd: i32, request: std::ffi::c_ulong, ...) -> i32;
    }
    let mut w = WinSize { rows: 0, cols: 0, x: 0, y: 0 };
    // SAFETY: the request fills a struct of this layout; fd 1 is ours.
    let ok = unsafe { ioctl(1, TIOCGWINSZ, &mut w as *mut WinSize) } == 0;
    if ok && w.cols > 0 && w.rows > 0 { (w.cols as u64, w.rows as u64) } else { (80, 24) }
}

/// Appends a line to the report.
fn report(path: &str, line: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

/// Writes what `unit` makes for a terminal of the size given it (columns,
/// rows), for ever, and reports the units a second. The size is asked
/// again every second: the terminal is often given its real one only
/// after the job has started.
fn flood(path: &str, what: &str, mut unit: impl FnMut(&mut Vec<u8>, &mut Random, u64, u64)) -> io::Result<()> {
    let mut out = io::stdout().lock();
    let mut random = Random(0x9e37_79b9_7f4a_7c15);
    let mut buffer = Vec::with_capacity(1 << 16);
    let (mut since, mut count) = (Instant::now(), 0u64);
    let (mut cols, mut rows) = size();
    loop {
        buffer.clear();
        unit(&mut buffer, &mut random, cols, rows);
        out.write_all(&buffer)?;
        count += 1;
        let passed = since.elapsed();
        if passed >= Duration::from_secs(1) {
            out.flush()?;
            report(path, &format!("{what} {:.0} {cols}x{rows}", count as f64 / passed.as_secs_f64()));
            (since, count) = (Instant::now(), 0);
            (cols, rows) = size();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(job), Some(path)) = (args.first(), args.get(1)) else {
        eprintln!("flood colour|cursor|unicode REPORT, or flood time REPORT COMMAND [ARGS]");
        std::process::exit(2);
    };
    let result = match job.as_str() {
        "colour" => flood(path, "colour-lines/s", |b, r, cols, _| {
            for _ in 0..cols {
                let v = r.next();
                let _ = write!(b, "\x1b[38;2;{};{};{};48;2;{};{};{}m{}", v as u8, (v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8, (v >> 32) as u8, (v >> 40) as u8, (b'!' + (v >> 48) as u8 % 94) as char);
            }
            b.extend_from_slice(b"\x1b[0m\r\n");
        }),
        "cursor" => flood(path, "cursor-batches/s", |b, r, cols, rows| {
            for _ in 0..250 {
                let v = r.next();
                let _ = write!(b, "\x1b[{};{}H\x1b[38;5;{}m{}", 1 + r.below(rows), 1 + r.below(cols), v as u8, (b'!' + (v >> 8) as u8 % 94) as char);
            }
        }),
        "unicode" => flood(path, "unicode-lines/s", |b, r, cols, _| {
            const PIECES: [&str; 12] = ["日本語", "한국어", "Việt", "e\u{301}a\u{308}", "Ελληνικά", "Кириллица", "│┼─╬", "▀▄█░", "→⇒∀∃", "😀🚀", "ａｂｃ", "żółć"];
            let mut width = 0;
            while width + 8 < cols {
                let piece = PIECES[r.below(PIECES.len() as u64) as usize];
                b.extend_from_slice(piece.as_bytes());
                b.push(b' ');
                // Wide enough a guess: no line is to wrap.
                width += 2 * piece.chars().count() as u64 + 1;
            }
            b.extend_from_slice(b"\r\n");
        }),
        "time" => {
            let Some(command) = args.get(2) else {
                eprintln!("flood time REPORT COMMAND [ARGS]");
                std::process::exit(2);
            };
            loop {
                let started = Instant::now();
                match std::process::Command::new(command).args(&args[3..]).status() {
                    Ok(_) => report(path, &format!("{command}-seconds {:.3}", started.elapsed().as_secs_f64())),
                    Err(e) => {
                        eprintln!("flood: {command}: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
        other => {
            eprintln!("flood: no job {other}");
            std::process::exit(2);
        }
    };
    // The terminal went away: that is how these end.
    drop(result);
}
