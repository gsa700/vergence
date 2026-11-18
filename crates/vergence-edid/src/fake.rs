//! Fake monitors' EDIDs: a 1.3 base block (manufacturer "VRG", the given
//! name, serial, physical size and one preferred timing) plus a CTA-861
//! extension with an HDMI vendor block. The HDMI block matters: without it
//! Intel's driver treats the sink as DVI and caps it at 165 MHz, too slow
//! for 4K. Used by `vergence-harness fake` (milestone 4) and the server's
//! screens in memory (`vctl screen add --fake`, headless milestone 1).

/// A video timing (CTA-861 numbers).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    pub clock_khz: u32,
    pub h: (u16, u16, u16, u16), // active, front porch, sync, back porch
    pub v: (u16, u16, u16, u16),
    /// CTA video identification code.
    pub vic: u8,
}

/// 1920x1080 at 60 Hz (VIC 16).
pub const FHD_60: Timing = Timing { clock_khz: 148_500, h: (1920, 88, 44, 148), v: (1080, 4, 5, 36), vic: 16 };
/// 3840x2160 at 30 Hz (VIC 95): the most an HDMI 1.4 port carries.
pub const UHD_30: Timing = Timing { clock_khz: 297_000, h: (3840, 176, 88, 296), v: (2160, 8, 10, 72), vic: 95 };

pub struct Spec<'a> {
    pub name: &'a str,
    pub product: u16,
    pub serial: u32,
    pub timing: Timing,
    pub size_mm: (u16, u16),
}

fn checksum(block: &mut [u8]) {
    let sum = block[..127].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    block[127] = 0u8.wrapping_sub(sum);
}

fn dtd(t: Timing, mm: (u16, u16)) -> [u8; 18] {
    let (ha, hf, hs, hb) = t.h;
    let (va, vf, vs, vb) = t.v;
    let hbl = hf + hs + hb;
    let vbl = vf + vs + vb;
    let clk = (t.clock_khz / 10) as u16;
    [
        clk as u8,
        (clk >> 8) as u8,
        ha as u8,
        hbl as u8,
        (((ha >> 8) & 0xF) << 4 | ((hbl >> 8) & 0xF)) as u8,
        va as u8,
        vbl as u8,
        (((va >> 8) & 0xF) << 4 | ((vbl >> 8) & 0xF)) as u8,
        hf as u8,
        hs as u8,
        ((vf & 0xF) << 4 | (vs & 0xF)) as u8,
        (((hf >> 8) & 3) << 6 | ((hs >> 8) & 3) << 4 | ((vf >> 4) & 3) << 2 | ((vs >> 4) & 3)) as u8,
        mm.0 as u8,
        mm.1 as u8,
        (((mm.0 >> 8) & 0xF) << 4 | ((mm.1 >> 8) & 0xF)) as u8,
        0,
        0,
        0x1E, // digital separate sync, +H +V
    ]
}

/// A display descriptor (tag 0xFC name, 0xFF serial) with up to 13 chars.
fn text(tag: u8, s: &str) -> [u8; 18] {
    let mut d = [0u8; 18];
    d[3] = tag;
    let b = s.as_bytes();
    let n = b.len().min(13);
    d[5..5 + n].copy_from_slice(&b[..n]);
    if n < 13 {
        d[5 + n] = 0x0A;
        for x in &mut d[6 + n..] {
            *x = 0x20;
        }
    }
    d
}

pub fn build(s: &Spec) -> Vec<u8> {
    let mut b = vec![0u8; 256];
    b[0..8].copy_from_slice(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0]);
    // "VRG": 5 bits per letter, A = 1.
    let id = (((b'V' - b'@') as u16) << 10) | (((b'R' - b'@') as u16) << 5) | (b'G' - b'@') as u16;
    b[8..10].copy_from_slice(&id.to_be_bytes());
    b[10..12].copy_from_slice(&s.product.to_le_bytes());
    b[12..16].copy_from_slice(&s.serial.to_le_bytes());
    b[16] = 45; // week
    b[17] = (2026 - 1990) as u8;
    b[18] = 1;
    b[19] = 3;
    b[20] = 0x80; // digital input
    b[21] = s.size_mm.0.div_ceil(10) as u8;
    b[22] = s.size_mm.1.div_ceil(10) as u8;
    b[23] = 120; // gamma 2.2
    b[24] = 0x0A; // RGB color, preferred timing in the first descriptor
    b[25..35].copy_from_slice(&[0xEE, 0x91, 0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54]); // sRGB-ish
    for i in 0..8 {
        b[38 + 2 * i] = 1;
        b[39 + 2 * i] = 1; // no standard timings
    }
    b[54..72].copy_from_slice(&dtd(s.timing, s.size_mm));
    b[72..90].copy_from_slice(&text(0xFC, s.name));
    b[90..108].copy_from_slice(&text(0xFF, &format!("FAKE{:08}", s.serial % 100_000_000)));
    // Range limits: 24-75 Hz, 15-160 kHz, 300 MHz.
    b[108..126].copy_from_slice(&[0, 0, 0, 0xFD, 0, 24, 75, 15, 160, 30, 0, 0x0A, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20]);
    b[126] = 1;
    checksum(&mut b[..128]);

    // CTA-861 extension: video block (the preferred VIC first, then
    // 1080p60) and an HDMI vendor block allowing 300 MHz.
    let e = &mut b[128..];
    e[0] = 0x02;
    e[1] = 0x03;
    let mut blocks: Vec<u8> = Vec::new();
    let vics: Vec<u8> = if s.timing.vic == FHD_60.vic { vec![0x80 | 16] } else { vec![0x80 | s.timing.vic, 16] };
    blocks.push((2 << 5) | vics.len() as u8);
    blocks.extend(&vics);
    blocks.extend([(3 << 5) | 7, 0x03, 0x0C, 0x00, 0x10, 0x00, 0x00, 60]); // HDMI, phys 1.0.0.0, 60 x 5 MHz
    e[2] = 4 + blocks.len() as u8; // first DTD offset (none follow)
    e[3] = 0; // no underscan/audio/YCbCr flags
    e[4..4 + blocks.len()].copy_from_slice(&blocks);
    checksum(e);
    b
}
