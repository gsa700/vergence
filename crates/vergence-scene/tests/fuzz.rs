//! Random and damaged bytes into the scene decoder: it may refuse anything,
//! it may not panic, and whatever it accepts must be a valid scene that
//! writes back to the same bytes (whitepaper §6.6: scenes are read from
//! untrusted processes). Deterministic (a fixed seed); FUZZ_ITERS raises the
//! count for a long run:
//! `FUZZ_ITERS=2000000 cargo test --release -p vergence-scene --test fuzz`.

use vergence_scene::{CELL_GLYPH, CELL_UNDERLINE, Cell, Color, Glyph, Image, Point, Rect, Scene};

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
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn iters() -> usize {
    std::env::var("FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000)
}

fn rect(rng: &mut Rng) -> Rect {
    Rect::new(rng.unit() * 800.0 - 100.0, rng.unit() * 600.0 - 100.0, rng.unit() * 400.0, rng.unit() * 300.0)
}

fn color(rng: &mut Rng) -> Color {
    let v = rng.next();
    Color::rgba(v as u8, (v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8)
}

/// A valid scene of a random shape.
fn scene(rng: &mut Rng) -> Scene {
    let mut s = Scene::new(1.0 + rng.unit() * 2000.0, 1.0 + rng.unit() * 1200.0);
    let fonts = 1 + rng.below(3);
    for i in 0..fonts {
        s.font(["Noto Sans Mono", "Liberation Mono", "Sans"][i], [400, 700, 300][rng.below(3)], rng.below(2) == 0);
    }
    let fonts = s.fonts.len();
    let images = rng.below(3);
    for _ in 0..images {
        let (w, h) = (1 + rng.below(4) as u32, 1 + rng.below(4) as u32);
        let rgba = (0..w * h * 4).map(|_| rng.next() as u8).collect();
        s.image(Image { width: w, height: h, rgba });
    }
    let mut open = 0;
    for _ in 0..rng.below(24) {
        match rng.below(9) {
            0 | 1 => {
                let (r, c) = (rect(rng), color(rng));
                s.fill_rounded(r, rng.unit() * 12.0, c);
            }
            8 => {
                let columns = 1 + rng.below(12);
                let cells = (0..columns * rng.below(6))
                    .map(|_| Cell {
                        ground: color(rng),
                        ink: color(rng),
                        glyph: rng.below(3000) as u32,
                        font: rng.below(fonts) as u16,
                        flags: [0, CELL_GLYPH, CELL_GLYPH, CELL_UNDERLINE | CELL_GLYPH, CELL_UNDERLINE][rng.below(5)],
                    })
                    .collect();
                s.cells(
                    Point::new(rng.unit() * 600.0, rng.unit() * 400.0),
                    4.0 + rng.unit() * 12.0,
                    8.0 + rng.unit() * 20.0,
                    rng.unit() * 20.0,
                    6.0 + rng.unit() * 30.0,
                    columns as u32,
                    cells,
                );
            }
            2 => {
                let (r, c) = (rect(rng), color(rng));
                s.stroke(r, rng.unit() * 8.0, rng.unit() * 3.0, c);
            }
            3 => {
                let c = color(rng);
                s.line(
                    Point::new(rng.unit() * 500.0, rng.unit() * 500.0),
                    Point::new(rng.unit() * 500.0, rng.unit() * 500.0),
                    rng.unit() * 4.0,
                    c,
                );
            }
            4 | 5 => {
                let glyphs = (0..rng.below(40))
                    .map(|i| Glyph { id: rng.below(3000) as u32, x: i as f32 * 8.1, y: 0.0 })
                    .collect();
                let c = color(rng);
                s.glyphs(
                    rng.below(fonts) as u32,
                    6.0 + rng.unit() * 30.0,
                    c,
                    Point::new(rng.unit() * 600.0, rng.unit() * 400.0),
                    glyphs,
                );
            }
            6 if images > 0 => {
                let r = rect(rng);
                s.draw_image(rng.below(images) as u32, r, rng.unit());
            }
            _ if open < 8 && rng.below(2) == 0 => {
                let r = rect(rng);
                s.push_clip(r, rng.unit() * 6.0);
                open += 1;
            }
            _ if open > 0 => {
                s.pop_clip();
                open -= 1;
            }
            _ => {}
        }
    }
    for _ in 0..open {
        s.pop_clip();
    }
    s
}

/// What must hold for any bytes at all.
fn check(bytes: &[u8]) {
    if let Ok(s) = Scene::decode(bytes) {
        assert_eq!(s.validate(), Ok(()), "decode returned an invalid scene");
        // NaN never passes validation, so equal scenes write equal bytes.
        assert_eq!(s.encode().expect("a decoded scene encodes"), bytes, "decode then encode changed the bytes");
    }
}

#[test]
fn random_scenes_survive_the_wire() {
    let mut rng = Rng::new(0x5ce9e);
    for _ in 0..iters() / 10 {
        let s = scene(&mut rng);
        assert_eq!(s.validate(), Ok(()));
        let bytes = s.encode().unwrap();
        assert_eq!(Scene::decode(&bytes).as_ref(), Ok(&s));
    }
}

#[test]
fn damaged_scenes_never_panic() {
    let mut rng = Rng::new(0xdead_beef);
    let mut base = scene(&mut rng).encode().unwrap();
    for i in 0..iters() {
        if i % 50 == 0 {
            base = scene(&mut rng).encode().unwrap();
        }
        let mut b = base.clone();
        match rng.below(6) {
            // Flip bits.
            0 | 1 => {
                for _ in 0..1 + rng.below(4) {
                    let at = rng.below(b.len());
                    b[at] ^= 1 << rng.below(8);
                }
            }
            // Overwrite a byte, biased to the values counts and floats break on.
            2 => {
                let at = rng.below(b.len());
                b[at] = [0, 1, 0x7f, 0x80, 0xff, 0xc0][rng.below(6)];
            }
            // Overwrite four bytes with an edge value (a count, a NaN, an infinity).
            3 => {
                if b.len() >= 4 {
                    let at = rng.below(b.len() - 3);
                    let v: u32 =
                        [0, 1, u32::MAX, 0x7fc0_0000, 0x7f80_0000, 0xff80_0000, 1 << 20, 1 << 31][rng.below(8)];
                    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
            // Cut it short.
            4 => b.truncate(rng.below(b.len() + 1)),
            // Add or duplicate a piece.
            _ => {
                let at = rng.below(b.len());
                let extra: Vec<u8> = (0..1 + rng.below(16)).map(|_| rng.next() as u8).collect();
                b.splice(at..at, extra);
            }
        }
        check(&b);
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng::new(0x1234_5678_9abc);
    for _ in 0..iters() {
        let mut b: Vec<u8> = (0..rng.below(200)).map(|_| rng.next() as u8).collect();
        // Half the time behind a real header, to get past the first checks.
        if rng.below(2) == 0 {
            let mut head = b"VSCN\x01\x00".to_vec();
            head.extend_from_slice(&64.0f32.to_bits().to_le_bytes());
            head.extend_from_slice(&64.0f32.to_bits().to_le_bytes());
            head.append(&mut b);
            b = head;
        }
        check(&b);
    }
}
