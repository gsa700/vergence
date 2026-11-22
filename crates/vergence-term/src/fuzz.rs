//! Random escape sequences into the parser and the screen: nothing may panic,
//! whatever a program in the terminal prints (security pass 2026-11-08).
//! Deterministic (a fixed seed); FUZZ_ITERS raises the count for a long run:
//! `FUZZ_ITERS=200000 cargo test --release -p vergence-term fuzz`.

use crate::grid::Grid;
use crate::vt::Parser;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn iters() -> usize {
    std::env::var("FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(3_000)
}

const NUMBERS: [&str; 10] = ["", "0", "1", "2", "7", "80", "65535", "65536", "99999999", "4294967296"];

fn number(rng: &mut Rng) -> String {
    if rng.below(3) == 0 { rng.below(300).to_string() } else { NUMBERS[rng.below(10)].into() }
}

/// One token of terminal output, biased towards escape sequences and edge values.
fn token(rng: &mut Rng, out: &mut Vec<u8>) {
    match rng.below(16) {
        0..=2 => out.push(b' ' + rng.below(95) as u8),
        3 => {
            let c = ['é', '漢', '🙂', '\u{301}', '\u{200b}', 'ا', '\u{ffff}', '\u{10ffff}'][rng.below(8)];
            let mut b = [0; 4];
            out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
        }
        4 => out.push(if rng.below(8) == 0 { 0x7f } else { rng.below(0x20) as u8 }),
        5 => out.push(0x80 + rng.below(0x80) as u8),
        6..=10 => {
            out.extend_from_slice(b"\x1b[");
            if rng.below(3) == 0 {
                out.push(b"?><="[rng.below(4)]);
            }
            let n = rng.below(5);
            for i in 0..n {
                if i > 0 {
                    out.push(if rng.below(6) == 0 { b':' } else { b';' });
                }
                out.extend_from_slice(number(rng).as_bytes());
            }
            if rng.below(6) == 0 {
                out.push(b" !\"$'"[rng.below(5)]);
            }
            out.push(b'@' + rng.below(63) as u8);
        }
        11 => {
            out.extend_from_slice(b"\x1b]");
            out.extend_from_slice(number(rng).as_bytes());
            out.push(b';');
            let n = if rng.below(10) == 0 { 5000 } else { rng.below(40) };
            for _ in 0..n {
                out.push(b' ' + rng.below(95) as u8);
            }
            if rng.below(2) == 0 {
                out.push(0x07);
            } else {
                out.extend_from_slice(b"\x1b\\");
            }
        }
        12 => {
            out.push(0x1b);
            out.push(0x20 + rng.below(0x60) as u8);
        }
        13 => {
            out.extend_from_slice(b"\x1bP");
            for _ in 0..rng.below(30) {
                out.push(rng.next() as u8);
            }
            out.extend_from_slice(b"\x1b\\");
        }
        14 => out.extend_from_slice(b"\x1b[?1049h"),
        _ => out.extend_from_slice(b"\x1b[?1049l"),
    }
}

fn run(rng: &mut Rng, grid: &mut Grid, stream: &[u8]) {
    let mut parser = Parser::new();
    let mut i = 0;
    while i < stream.len() {
        let n = (1 + rng.below(40)).min(stream.len() - i);
        parser.advance(grid, &stream[i..i + n]);
        i += n;
        if rng.below(10) == 0 {
            grid.resize(rng.below(130), rng.below(60));
        }
        grid.out.clear();
    }
    let _ = grid.text();
}

#[test]
fn random_output_never_panics() {
    // One seed per case, so a failure replays alone: FUZZ_CASE=<n>.
    let cases: Vec<usize> = match std::env::var("FUZZ_CASE").ok().and_then(|v| v.parse().ok()) {
        Some(n) => vec![n],
        None => (0..iters()).collect(),
    };
    for case in cases {
        let mut rng = Rng::new(0x5eed_0000_0000 + case as u64);
        let (cols, rows) = (rng.below(130), rng.below(60));
        let mut grid = Grid::new(cols, rows);
        let mut stream = Vec::new();
        let tokens = 1 + rng.below(200);
        for _ in 0..tokens {
            token(&mut rng, &mut stream);
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&mut rng, &mut grid, &stream)));
        if r.is_err() {
            let shown: String = stream
                .iter()
                .map(|b| if (0x20..0x7f).contains(b) { (*b as char).to_string() } else { format!("\\x{b:02x}") })
                .collect();
            panic!("grid panicked; FUZZ_CASE={case} ({cols}x{rows}); stream: {shown}");
        }
    }
}

/// Sequences with edge values, each on a fresh small screen and on the
/// smallest one, in every cursor position they can be reached from.
#[test]
fn edge_sequences_never_panic() {
    const SEQS: [&[u8]; 40] = [
        b"\x1b[99999;99999H",
        b"\x1b[0;0H",
        b"\x1b[H\x1b[J",
        b"\x1b[1J",
        b"\x1b[2J",
        b"\x1b[3J",
        b"\x1b[9J",
        b"\x1b[K",
        b"\x1b[1K",
        b"\x1b[2K",
        b"\x1b[1000@",
        b"\x1b[1000P",
        b"\x1b[1000X",
        b"\x1b[1000L",
        b"\x1b[1000M",
        b"\x1b[1000A",
        b"\x1b[1000B",
        b"\x1b[1000C",
        b"\x1b[1000D",
        b"\x1b[1000G",
        b"\x1b[1000d",
        b"\x1b[1000E",
        b"\x1b[1000F",
        b"\x1b[1000S",
        b"\x1b[1000T",
        b"\x1b[r",
        b"\x1b[0;0r",
        b"\x1b[5;2r",
        b"\x1b[2;1r\x1b[M",
        b"\x1b[1;1r\x1b[L",
        b"\x1b[2;3r\x1bM\x1bM\x1bM",
        b"\x1b[s\x1b[u",
        b"\x1b7\x1b8",
        b"\x1b[?6h\x1b[99;99H",
        b"\x1b[?7l",
        b"\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t",
        b"\xe6\xbc\xa2\xe6\xbc\xa2\xe6\xbc\xa2",
        b"\x1b[?1049h\x1b[2J\x1b[?1049l",
        b"\x08\x08\x08\x08",
        b"\x1b[65535;65535;65535;65535;65535m",
    ];
    for (cols, rows) in [(2, 1), (3, 2), (10, 4), (80, 24)] {
        for seq in SEQS {
            for pre in [
                &b""[..],
                b"\x1b[999;999H",
                b"\x1b[1;1H",
                b"\x1b[?6h\x1b[2;3r\x1b[2;1H",
                b"abcdefghijklmnopqrstuvwxyz\n\n\n\n\n",
            ] {
                let mut grid = Grid::new(cols, rows);
                let mut parser = Parser::new();
                parser.advance(&mut grid, pre);
                parser.advance(&mut grid, seq);
                parser.advance(&mut grid, b"x\xe6\xbc\xa2\n");
                let _ = grid.text();
                grid.resize(rows, cols);
                parser.advance(&mut grid, seq);
                let _ = grid.text();
            }
        }
    }
}
