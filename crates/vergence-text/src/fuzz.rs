//! Damaged and random bytes into the font reader (sfnt.rs): a font file is
//! untrusted input, and reading one may fail but may not panic or loop.
//! Deterministic (a fixed seed); FUZZ_ITERS raises the count for a long run:
//! `FUZZ_ITERS=2000000 cargo test --release -p vergence-text fuzz`.

use crate::sfnt::{
    self,
    testfont::{Spec, collection, font},
};

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
    std::env::var("FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000)
}

/// Everything the reader does with a file, on these bytes.
fn read(bytes: &[u8]) {
    for f in sfnt::faces(&mut &bytes[..]) {
        assert!(!f.family.is_empty() && (1..=1000).contains(&f.weight));
        if let Some((min, max)) = f.weight_axis {
            assert!(min <= max && max <= 1000);
        }
        if let Some((off, len)) = f.cmap
            && let Some(table) = usize::try_from(off).ok().and_then(|o| bytes.get(o..o.checked_add(len as usize)?))
        {
            let cov = sfnt::coverage(table);
            let _ = cov.contains('A') || cov.contains('\u{10ffff}');
        }
    }
    // And the same bytes taken as a character map on their own.
    let _ = sfnt::coverage(bytes);
}

fn bases() -> Vec<Vec<u8>> {
    vec![
        font(&Spec::default()),
        font(&Spec {
            family: "Example Mono",
            typographic: Some("Example"),
            weight: 700,
            italic: true,
            monospace: true,
            weight_axis: Some((100, 900)),
            ..Spec::default()
        }),
        font(&Spec {
            ranges: &[(0x20, 0x7e), (0xa0, 0x17f), (0x2500, 0x257f)],
            wide: &[(0x20, 0x7e), (0x1f300, 0x1f64f)],
            ..Spec::default()
        }),
        collection(&[Spec::default(), Spec { family: "Second", wide: &[(0x4e00, 0x9fff)], ..Spec::default() }]),
    ]
}

#[test]
fn damaged_fonts_never_panic() {
    let bases = bases();
    let mut rng = Rng::new(0xf0f0_1234);
    for _ in 0..iters() {
        let mut b = bases[rng.below(bases.len())].clone();
        for _ in 0..1 + rng.below(3) {
            match rng.below(5) {
                0 => {
                    let at = rng.below(b.len());
                    b[at] ^= 1 << rng.below(8);
                }
                1 => {
                    let at = rng.below(b.len());
                    b[at] = [0, 1, 0x7f, 0x80, 0xff][rng.below(5)];
                }
                // An offset, a length or a count replaced by an edge value.
                2 => {
                    if b.len() >= 4 {
                        let at = rng.below(b.len() - 3);
                        let v: u32 = [0, 1, 0xffff, 0x10000, u32::MAX, 0x7fff_ffff, b.len() as u32, b.len() as u32 - 1]
                            [rng.below(8)];
                        b[at..at + 4].copy_from_slice(&v.to_be_bytes());
                    }
                }
                3 => b.truncate(rng.below(b.len() + 1)),
                _ => {
                    let at = rng.below(b.len().max(1));
                    let extra: Vec<u8> = (0..1 + rng.below(12)).map(|_| rng.next() as u8).collect();
                    let at = at.min(b.len());
                    b.splice(at..at, extra);
                }
            }
            if b.is_empty() {
                break;
            }
        }
        read(&b);
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng::new(0x0bad_f00d);
    for _ in 0..iters() {
        let mut b: Vec<u8> = (0..rng.below(400)).map(|_| rng.next() as u8).collect();
        // Often behind a real signature, to get past the first check.
        match rng.below(4) {
            0 => b.splice(0..0, [0, 1, 0, 0, 0, 5]).for_each(drop),
            1 => b.splice(0..0, *b"ttcf\0\x01\0\0\0\0\0\x03").for_each(drop),
            2 => b.splice(0..0, *b"OTTO\0\x09").for_each(drop),
            _ => {}
        }
        read(&b);
    }
}
