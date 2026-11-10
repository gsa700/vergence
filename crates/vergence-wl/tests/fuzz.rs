//! Random and damaged bytes as if from the display server: reading them may
//! fail, it may not panic. Deterministic (a fixed seed); FUZZ_ITERS raises
//! the count: `FUZZ_ITERS=2000000 cargo test --release -p vergence-wl --test fuzz`.

use vergence_wl::{
    Interface,
    client::decode,
    wire::{Reader, Writer, split},
};

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

const INTERFACES: [Interface; 17] = [
    Interface::Display,
    Interface::Registry,
    Interface::Callback,
    Interface::Compositor,
    Interface::Surface,
    Interface::Shm,
    Interface::ShmPool,
    Interface::Buffer,
    Interface::WmBase,
    Interface::XdgSurface,
    Interface::Toplevel,
    Interface::Viewporter,
    Interface::Viewport,
    Interface::FractionalScaleManager,
    Interface::FractionalScale,
    Interface::DecorationManager,
    Interface::Decoration,
];

/// Everything a client does with bytes from the server.
fn read(bytes: &[u8]) {
    let mut rest = bytes;
    for _ in 0..64 {
        match split(rest) {
            Ok(Some(h)) => {
                let args = &rest[8..h.size];
                for interface in INTERFACES {
                    let _ = decode(interface, h.object, h.opcode, args);
                }
                let mut r = Reader::new(args);
                let _ = (r.string(), r.array().map(<[u8]>::len), r.words(), r.fixed(), r.int());
                rest = &rest[h.size..];
            }
            _ => break,
        }
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng(0x7a71_a2d0_0000_0001);
    for _ in 0..iters() {
        let bytes: Vec<u8> = (0..rng.below(120)).map(|_| rng.next() as u8).collect();
        read(&bytes);
    }
}

#[test]
fn damaged_events_never_panic() {
    let mut rng = Rng(0x0e7e_4751_0000_0003);
    let bases: Vec<Vec<u8>> = vec![
        Writer::new(1, 0).uint(5).uint(2).string("invalid arguments").finish().unwrap().0,
        Writer::new(2, 0).uint(3).string("wp_fractional_scale_manager_v1").uint(1).finish().unwrap().0,
        Writer::new(9, 0).int(800).int(600).array(&[4, 0, 0, 0, 1, 0, 0, 0]).finish().unwrap().0,
        Writer::new(7, 0).uint(360).finish().unwrap().0,
        Writer::new(4, 1).finish().unwrap().0,
    ];
    for _ in 0..iters() {
        let mut b = bases[rng.below(bases.len())].clone();
        if rng.below(3) == 0 {
            b.extend_from_slice(&bases[rng.below(bases.len())]);
        }
        for _ in 0..1 + rng.below(3) {
            match rng.below(4) {
                0 => {
                    let at = rng.below(b.len());
                    b[at] ^= 1 << rng.below(8);
                }
                // A length or a size replaced by an edge value.
                1 => {
                    if b.len() >= 4 {
                        let at = rng.below(b.len() / 4) * 4;
                        let v: u32 = [0, 1, 4, 8, 0xffff, 0x7fff_ffff, u32::MAX, (b.len() as u32) << 16][rng.below(8)];
                        b[at..at + 4].copy_from_slice(&v.to_ne_bytes());
                    }
                }
                2 => b.truncate(rng.below(b.len() + 1)),
                _ => {
                    let at = rng.below(b.len() + 1);
                    let extra: Vec<u8> = (0..1 + rng.below(8)).map(|_| rng.next() as u8).collect();
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
