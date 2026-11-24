//! Damage for every highlighter: random lines built from the characters
//! that matter to them (quotes, brackets, backslashes, comment markers,
//! letters, multi-byte characters), many lines in a row with the state
//! carried. A highlighter must never panic, and its spans must be in
//! order, not overlapping, inside the line and on character boundaries.
//!
//! Deterministic (a fixed seed); FUZZ_ITERS raises the count for a long run.

use crate::{LangId, State, highlight, languages};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PIECES: &[&str] = &[
    "'", "\"", "'''", "\"\"\"", "\\", "{", "}", "{{", "(", ")", "[", "]", "#", "//", "/*", "*/", "<!--", "-->", "<",
    ">", "</", "?>", "<?php", "$", "@", "`", "```", "*", "_", "**", "f", "r", "b", "rb", "def ", "class ", "if ",
    "None", "0x", "1e", "9", ".", " ", "\t", "é", "漢", "🙂", "\u{feff}", "a", "Z", "=", ":", "-", "+", "&", ";",
];

fn check(lang: LangId, rng: &mut Rng, lines: usize) {
    let mut state = State::default();
    for _ in 0..lines {
        let mut line = String::new();
        for _ in 0..rng.below(40) {
            line.push_str(PIECES[rng.below(PIECES.len())]);
        }
        let spans = highlight(lang, &mut state, &line);
        for s in &spans {
            assert!(s.start < s.end && s.end <= line.len(), "{line:?}: {s:?}");
            assert!(line.is_char_boundary(s.start) && line.is_char_boundary(s.end), "{line:?}: {s:?}");
        }
        for w in spans.windows(2) {
            assert!(w[0].end <= w[1].start, "{line:?}: {spans:?}");
        }
    }
}

#[test]
fn every_highlighter_survives_damage() {
    let iters: usize = std::env::var("FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(400);
    let mut rng = Rng(0x5eed_1951_2026_1124);
    for i in 0..languages().len() {
        for _ in 0..iters {
            check(LangId(i as u16), &mut rng, 20);
        }
    }
}
