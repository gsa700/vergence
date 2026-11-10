//! Random and damaged text into the desktop entry reader and the Exec
//! splitter: they may refuse anything, they may not panic or run away, and
//! what they give back keeps its promises (docs/shelf-milestone-1.md,
//! Security: entries are files the user or a package installed, and are
//! still read as untrusted input). Deterministic (a fixed seed); FUZZ_ITERS
//! raises the count for a long run:
//! `FUZZ_ITERS=2000000 cargo test --release -p vergence-shell --test fuzz`.

use vergence_shell::desktop::{parse, split_exec};

struct Rng(u64);

impl Rng {
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
    std::env::var("FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000)
}

/// Pieces an entry is made of, and pieces that break one.
const PIECES: [&str; 28] = [
    "[Desktop Entry]\n", "Type=Application\n", "Type=Link\n", "Name=App\n", "Name=\n", "Exec=app %u\n", "Exec=\"app\n", "Exec=\n", "Exec=\"a b\" \\\"c\\\" %% %\n",
    "Icon=app\n", "Terminal=true\n", "NoDisplay=true\n", "Hidden=true\n", "Keywords=a;b;;c\n", "TryExec=/bin/sh\n", "[Desktop Action x]\n", "[", "]", "=", "\\", "\"",
    "%", "\n", " ", "\u{e9}", "\u{1f600}", "\0", "Name[de]=Anwendung\n",
];

#[test]
fn entries_of_random_pieces_never_panic() {
    let mut rng = Rng(0x5e11_f00d);
    let mut read = 0;
    for _ in 0..iters() {
        let mut text = String::new();
        for _ in 0..rng.below(24) {
            text.push_str(PIECES[rng.below(PIECES.len())]);
        }
        if let Some(e) = parse("id", &text) {
            read += 1;
            // What is given back is what was promised.
            assert_eq!(e.id, "id");
            assert!(e.hidden || (!e.name.trim().is_empty() && !e.exec.is_empty() && !e.exec[0].is_empty()), "{text:?} gave {e:?}");
        }
    }
    assert!(read > 0, "some of them were entries");
}

#[test]
fn exec_lines_of_random_characters_never_panic() {
    let mut rng = Rng(0xe8ec_1111);
    let alphabet: Vec<char> = "ab /-=\"\\%$`fFuUick \t\u{e9}\u{4e2d}".chars().collect();
    for _ in 0..iters() {
        let line: String = (0..rng.below(40)).map(|_| alphabet[rng.below(alphabet.len())]).collect();
        if let Some(args) = split_exec(&line) {
            assert!(!args.is_empty() && !args[0].is_empty(), "{line:?} gave {args:?}");
            // Nothing is invented: every argument's characters were in the line.
            assert!(args.iter().all(|a| a.chars().all(|c| line.contains(c))), "{line:?} gave {args:?}");
        }
    }
}

#[test]
fn damaged_entries_never_panic() {
    let good = "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox --name \"My Fox\" %u\nIcon=firefox\nKeywords=web;browser;\nTerminal=false\n";
    assert!(parse("firefox", good).is_some());
    let mut rng = Rng(0xda3a_6ed);
    for _ in 0..iters() {
        let mut bytes = good.as_bytes().to_vec();
        for _ in 0..1 + rng.below(4) {
            if bytes.is_empty() {
                break;
            }
            let at = rng.below(bytes.len());
            match rng.below(4) {
                0 => bytes[at] = rng.next() as u8,
                1 => {
                    bytes.remove(at);
                }
                2 => bytes.insert(at, rng.next() as u8),
                _ => bytes.truncate(at.max(1)),
            }
        }
        // The files are read as text: bytes that are not UTF-8 never reach the reader.
        if let Ok(text) = std::str::from_utf8(&bytes) {
            let _ = parse("firefox", text);
        }
    }
}
