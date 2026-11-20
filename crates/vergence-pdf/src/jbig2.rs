//! JBIG2 (ITU-T T.88; ISO 32000-2 §7.4.7), our own: the MQ arithmetic
//! decoder, integer and symbol-ID decoding, generic regions (templates
//! 0-3, typical prediction, MMR through ccitt.rs), refinement regions,
//! symbol dictionaries (arithmetic, with refinement and aggregation) and
//! text regions (arithmetic), the page and its stripes, and the PDF's
//! /JBIG2Globals. What scanned books and manuals are often stored as.
//!
//! Not yet: Huffman-coded dictionaries and text regions (custom and
//! standard tables), halftone and pattern regions. A stream that needs one
//! of them comes back as None (the picture stays a placeholder).
//!
//! Untrusted: bitmap sizes, symbol and instance counts, and the number of
//! decisions decoded are all bounded; past the end of its data the MQ
//! decoder reads 0xFF as the standard says, and a budget stops runaway
//! loops.

use std::collections::HashMap;

const MAX_PIXELS: usize = 1 << 27;
const MAX_SYMBOLS: usize = 1 << 20;
const MAX_INSTANCES: usize = 1 << 22;
/// Decisions one stream may decode (a 600 dpi letter page is ~34 M).
const BUDGET: u64 = if cfg!(test) { 1 << 22 } else { 1 << 30 };

/// A bitmap, one byte per pixel, 1 black.
#[derive(Debug, Clone, PartialEq)]
pub struct Bitmap {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Bitmap {
    fn new(w: usize, h: usize, fill: u8) -> Option<Bitmap> {
        if w.checked_mul(h)? > MAX_PIXELS {
            return None;
        }
        Some(Bitmap { w, h, px: vec![fill; w * h] })
    }

    fn get(&self, x: isize, y: isize) -> u8 {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            0
        } else {
            self.px[y as usize * self.w + x as usize]
        }
    }

    /// Draws `src` with its top left at (x, y): 0 OR, 1 AND, 2 XOR, 3
    /// XNOR, 4 REPLACE.
    fn compose(&mut self, src: &Bitmap, x: isize, y: isize, op: u8) {
        for sy in 0..src.h {
            let ty = y + sy as isize;
            if ty < 0 || ty as usize >= self.h {
                continue;
            }
            for sx in 0..src.w {
                let tx = x + sx as isize;
                if tx < 0 || tx as usize >= self.w {
                    continue;
                }
                let s = src.px[sy * src.w + sx];
                let d = &mut self.px[ty as usize * self.w + tx as usize];
                *d = match op {
                    1 => *d & s,
                    2 => *d ^ s,
                    3 => !(*d ^ s) & 1,
                    4 => s,
                    _ => *d | s,
                };
            }
        }
    }
}

/// Table E.1: Qe, the next index after an MPS and after an LPS, and
/// whether an LPS swaps the sense of the MPS.
#[rustfmt::skip]
const QE: [(u32, u8, u8, bool); 47] = [
    (0x5601, 1, 1, true), (0x3401, 2, 6, false), (0x1801, 3, 9, false), (0x0AC1, 4, 12, false),
    (0x0521, 5, 29, false), (0x0221, 38, 33, false), (0x5601, 7, 6, true), (0x5401, 8, 14, false),
    (0x4801, 9, 14, false), (0x3801, 10, 14, false), (0x3001, 11, 17, false), (0x2401, 12, 18, false),
    (0x1C01, 13, 20, false), (0x1601, 29, 21, false), (0x5601, 15, 14, true), (0x5401, 16, 14, false),
    (0x5101, 17, 15, false), (0x4801, 18, 16, false), (0x3801, 19, 17, false), (0x3401, 20, 18, false),
    (0x3001, 21, 19, false), (0x2801, 22, 19, false), (0x2401, 23, 20, false), (0x2201, 24, 21, false),
    (0x1C01, 25, 22, false), (0x1801, 26, 23, false), (0x1601, 27, 24, false), (0x1401, 28, 25, false),
    (0x1201, 29, 26, false), (0x1101, 30, 27, false), (0x0AC1, 31, 28, false), (0x09C1, 32, 29, false),
    (0x08A1, 33, 30, false), (0x0521, 34, 31, false), (0x0441, 35, 32, false), (0x02A1, 36, 33, false),
    (0x0221, 37, 34, false), (0x0141, 38, 35, false), (0x0111, 39, 36, false), (0x0085, 40, 37, false),
    (0x0049, 41, 38, false), (0x0025, 42, 39, false), (0x0015, 43, 40, false), (0x0009, 44, 41, false),
    (0x0005, 45, 42, false), (0x0001, 45, 43, false), (0x5601, 46, 46, false),
];

/// The MQ decoder (Annex E, the software conventions of E.3), JPEG 2000's
/// too (jpx.rs: T.800 Annex C is the same coder). A context's
/// state is one byte: its table index shifted left once, its MPS in bit 0.
pub(crate) struct Mq<'a> {
    data: &'a [u8],
    bp: usize,
    c_high: u32,
    c_low: u32,
    a: u32,
    ct: i32,
    left: u64,
}

impl<'a> Mq<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Mq<'a> {
        let mut m = Mq { data, bp: 0, c_high: 0, c_low: 0, a: 0, ct: 0, left: BUDGET };
        m.c_high = m.byte(0);
        m.byte_in();
        m.c_high = ((m.c_high << 7) & 0xFFFF) | ((m.c_low >> 9) & 0x7F);
        m.c_low = (m.c_low << 7) & 0xFFFF;
        m.ct -= 7;
        m.a = 0x8000;
        m
    }

    fn byte(&self, i: usize) -> u32 {
        self.data.get(i).copied().unwrap_or(0xFF) as u32
    }

    fn byte_in(&mut self) {
        if self.byte(self.bp) == 0xFF {
            if self.byte(self.bp + 1) > 0x8F {
                self.c_low += 0xFF00;
                self.ct = 8;
            } else {
                self.bp += 1;
                self.c_low += self.byte(self.bp) << 9;
                self.ct = 7;
            }
        } else {
            self.bp += 1;
            self.c_low += self.byte(self.bp) << 8;
            self.ct = 8;
        }
        if self.c_low > 0xFFFF {
            self.c_high += self.c_low >> 16;
            self.c_low &= 0xFFFF;
        }
    }

    /// Out of budget: everything after decodes as 0, and loops stop.
    fn spent(&self) -> bool {
        self.left == 0
    }

    pub(crate) fn bit(&mut self, cx: &mut [u8], i: usize) -> u8 {
        if self.left == 0 {
            return 0;
        }
        self.left -= 1;
        let Some(state) = cx.get_mut(i) else { return 0 };
        let (mut index, mut mps) = ((*state >> 1) as usize, *state & 1);
        let (qe, nmps, nlps, switch) = QE[index.min(46)];
        let mut a = self.a.wrapping_sub(qe);
        let d;
        if self.c_high < qe {
            if a < qe {
                a = qe;
                d = mps;
                index = nmps as usize;
            } else {
                a = qe;
                d = 1 ^ mps;
                if switch {
                    mps = d;
                }
                index = nlps as usize;
            }
        } else {
            self.c_high -= qe;
            if a & 0x8000 != 0 {
                self.a = a;
                return mps;
            }
            if a < qe {
                d = 1 ^ mps;
                if switch {
                    mps = d;
                }
                index = nlps as usize;
            } else {
                d = mps;
                index = nmps as usize;
            }
        }
        loop {
            if self.ct == 0 {
                self.byte_in();
            }
            a = (a << 1) & 0xFFFF;
            self.c_high = ((self.c_high << 1) & 0xFFFF) | ((self.c_low >> 15) & 1);
            self.c_low = (self.c_low << 1) & 0xFFFF;
            self.ct -= 1;
            if a & 0x8000 != 0 {
                break;
            }
        }
        self.a = a;
        *state = ((index as u8) << 1) | mps;
        d
    }

    /// An integer (A.2): None is OOB.
    fn int(&mut self, cx: &mut [u8]) -> Option<i64> {
        let mut prev = 1usize;
        let mut bits = |m: &mut Mq, n: u32| -> i64 {
            let mut v = 0i64;
            for _ in 0..n {
                let b = m.bit(cx, prev) as usize;
                prev = if prev < 256 { (prev << 1) | b } else { (((prev << 1) | b) & 511) | 256 };
                v = (v << 1) | b as i64;
            }
            v
        };
        let sign = bits(self, 1);
        let v = if bits(self, 1) == 0 {
            bits(self, 2)
        } else if bits(self, 1) == 0 {
            bits(self, 4) + 4
        } else if bits(self, 1) == 0 {
            bits(self, 6) + 20
        } else if bits(self, 1) == 0 {
            bits(self, 8) + 84
        } else if bits(self, 1) == 0 {
            bits(self, 12) + 340
        } else {
            bits(self, 32) + 4436
        };
        match (sign, v) {
            (0, v) => Some(v),
            (_, 0) => None,
            (_, v) => Some(-v),
        }
    }

    /// A symbol ID (A.3), `len` bits.
    fn id(&mut self, cx: &mut [u8], len: u32) -> usize {
        let mut prev = 1usize;
        for _ in 0..len {
            let b = self.bit(cx, prev) as usize;
            prev = (prev << 1) | b;
        }
        prev - (1 << len)
    }
}

/// Each integer procedure's contexts, the generic and refinement ones.
struct Cx {
    dh: Vec<u8>,
    dw: Vec<u8>,
    ex: Vec<u8>,
    ai: Vec<u8>,
    dt: Vec<u8>,
    fs: Vec<u8>,
    ds: Vec<u8>,
    it: Vec<u8>,
    ri: Vec<u8>,
    rdw: Vec<u8>,
    rdh: Vec<u8>,
    rdx: Vec<u8>,
    rdy: Vec<u8>,
    id: Vec<u8>,
    gb: Vec<u8>,
    gr: Vec<u8>,
}

impl Cx {
    fn new(id_len: u32) -> Cx {
        let i = || vec![0u8; 512];
        Cx { dh: i(), dw: i(), ex: i(), ai: i(), dt: i(), fs: i(), ds: i(), it: i(), ri: i(), rdw: i(), rdh: i(), rdx: i(), rdy: i(), id: vec![0; 1 << (id_len.min(24) + 1)], gb: vec![0; 1 << 16], gr: vec![0; 1 << 14] }
    }
}

/// The fixed pixels of each generic template (6.2.5.3), before the
/// adaptive ones.
const TEMPLATES: [&[(i8, i8)]; 4] = [
    &[(-1, -2), (0, -2), (1, -2), (-2, -1), (-1, -1), (0, -1), (1, -1), (2, -1), (-4, 0), (-3, 0), (-2, 0), (-1, 0)],
    &[(-1, -2), (0, -2), (1, -2), (2, -2), (-2, -1), (-1, -1), (0, -1), (1, -1), (2, -1), (-3, 0), (-2, 0), (-1, 0)],
    &[(-1, -2), (0, -2), (1, -2), (-2, -1), (-1, -1), (0, -1), (1, -1), (-2, 0), (-1, 0)],
    &[(-3, -1), (-2, -1), (-1, -1), (0, -1), (1, -1), (-4, 0), (-3, 0), (-2, 0), (-1, 0)],
];
/// The context typical prediction's SLTP bit is read with, per template.
const SLTP: [usize; 4] = [0x9B25, 0x0795, 0x00E5, 0x0195];

/// A generic region by arithmetic coding (6.2.5).
fn generic(m: &mut Mq, cx: &mut [u8], w: usize, h: usize, template: u8, tpgdon: bool, at: &[(i8, i8)]) -> Option<Bitmap> {
    let t = (template & 3) as usize;
    let mut pixels: Vec<(i8, i8)> = TEMPLATES[t].to_vec();
    pixels.extend(at.iter().take(if t == 0 { 4 } else { 1 }));
    pixels.sort_by_key(|&(x, y)| (y, x));
    let mut b = Bitmap::new(w, h, 0)?;
    let mut ltp = 0u8;
    for y in 0..h {
        if m.spent() {
            break;
        }
        if tpgdon {
            ltp ^= m.bit(cx, SLTP[t]);
            if ltp == 1 {
                if y > 0 {
                    b.px.copy_within((y - 1) * w..y * w, y * w);
                }
                continue;
            }
        }
        for x in 0..w {
            let mut c = 0usize;
            for &(dx, dy) in &pixels {
                c = (c << 1) | b.get(x as isize + dx as isize, y as isize + dy as isize) as usize;
            }
            b.px[y * w + x] = m.bit(cx, c);
        }
    }
    Some(b)
}

/// A generic region by MMR (6.2.6): T.6 with 1 black.
fn generic_mmr(data: &[u8], w: usize, h: usize) -> Option<Bitmap> {
    Bitmap::new(w, h, 0)?;
    let p = crate::ccitt::Params { k: -1, columns: w, rows: h, end_of_line: false, byte_align: false, end_of_block: true, black_is_1: true };
    let packed = crate::ccitt::decode(data, &p);
    let stride = w.div_ceil(8);
    let mut b = Bitmap::new(w, h, 0)?;
    for y in 0..h.min(packed.len() / stride.max(1)) {
        for x in 0..w {
            b.px[y * w + x] = (packed[y * stride + x / 8] >> (7 - x % 8)) & 1;
        }
    }
    Some(b)
}

const REFINE_CODING: [&[(i8, i8)]; 2] = [&[(0, -1), (1, -1), (-1, 0)], &[(-1, -1), (0, -1), (1, -1), (-1, 0)]];
const REFINE_REFERENCE: [&[(i8, i8)]; 2] = [&[(0, -1), (1, -1), (-1, 0), (0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)], &[(0, -1), (-1, 0), (0, 0), (1, 0), (0, 1), (1, 1)]];
const REFINE_SLTP: [usize; 2] = [0x0020, 0x0008];

/// A refinement region (6.3): `reference` moved by (dx, dy).
#[allow(clippy::too_many_arguments)]
fn refine(m: &mut Mq, cx: &mut [u8], w: usize, h: usize, template: u8, reference: &Bitmap, dx: isize, dy: isize, tpgron: bool, at: &[(i8, i8)]) -> Option<Bitmap> {
    let t = (template & 1) as usize;
    let mut coding: Vec<(i8, i8)> = REFINE_CODING[t].to_vec();
    let mut refs: Vec<(i8, i8)> = REFINE_REFERENCE[t].to_vec();
    if t == 0 {
        coding.push(at.first().copied().unwrap_or((-1, -1)));
        refs.push(at.get(1).copied().unwrap_or((-1, -1)));
    }
    let mut b = Bitmap::new(w, h, 0)?;
    let mut ltp = 0u8;
    for y in 0..h {
        if m.spent() {
            break;
        }
        if tpgron {
            ltp ^= m.bit(cx, REFINE_SLTP[t]);
        }
        for x in 0..w {
            let (rx, ry) = (x as isize - dx, y as isize - dy);
            if ltp == 1 {
                let v = reference.get(rx, ry);
                let same = (-1..=1).all(|j| (-1..=1).all(|i| reference.get(rx + i, ry + j) == v));
                if same {
                    b.px[y * w + x] = v;
                    continue;
                }
            }
            let mut c = 0usize;
            for &(cx_, cy) in &coding {
                c = (c << 1) | b.get(x as isize + cx_ as isize, y as isize + cy as isize) as usize;
            }
            for &(px, py) in &refs {
                c = (c << 1) | reference.get(rx + px as isize, ry + py as isize) as usize;
            }
            b.px[y * w + x] = m.bit(cx, c);
        }
    }
    Some(b)
}

/// A text region's parameters (7.4.3.1.1, and 6.5.8.2's for aggregation).
struct Text {
    w: usize,
    h: usize,
    refine: bool,
    strips: i64,
    corner: u8,
    transposed: bool,
    op: u8,
    default: u8,
    ds_offset: i64,
    r_template: u8,
    r_at: [(i8, i8); 2],
    instances: usize,
    id_len: u32,
}

/// A text region by arithmetic coding (6.4).
fn text(m: &mut Mq, c: &mut Cx, p: &Text, syms: &[Bitmap]) -> Option<Bitmap> {
    let mut b = Bitmap::new(p.w, p.h, p.default)?;
    let mut strip_t = -m.int(&mut c.dt)? * p.strips;
    let mut first_s = 0i64;
    let mut n = 0usize;
    while n < p.instances.min(MAX_INSTANCES) && !m.spent() {
        strip_t += m.int(&mut c.dt)? * p.strips;
        first_s += m.int(&mut c.fs)?;
        let mut s = first_s;
        loop {
            let cur_t = if p.strips > 1 { m.int(&mut c.it)? } else { 0 };
            let t = strip_t + cur_t;
            let id = m.id(&mut c.id, p.id_len);
            let refined = p.refine && m.int(&mut c.ri)? != 0;
            let mut sym = std::borrow::Cow::Borrowed(syms.get(id)?);
            if refined {
                let rdw = m.int(&mut c.rdw)?;
                let rdh = m.int(&mut c.rdh)?;
                let rdx = m.int(&mut c.rdx)?;
                let rdy = m.int(&mut c.rdy)?;
                let w = (sym.w as i64 + rdw).clamp(0, 1 << 16) as usize;
                let h = (sym.h as i64 + rdh).clamp(0, 1 << 16) as usize;
                let r = refine(m, &mut c.gr, w, h, p.r_template, &sym, ((rdw >> 1) + rdx) as isize, ((rdh >> 1) + rdy) as isize, false, &p.r_at)?;
                sym = std::borrow::Cow::Owned(r);
            }
            let (sw, sh) = (sym.w as i64, sym.h as i64);
            // The reference corner: 0 bottom left, 1 top left, 2 bottom right, 3 top right.
            let (x, y);
            if !p.transposed {
                if p.corner >= 2 {
                    s += sw - 1;
                }
                x = if p.corner >= 2 { s - sw + 1 } else { s };
                y = if p.corner & 1 == 1 { t } else { t - sh + 1 };
                if p.corner < 2 {
                    s += sw - 1;
                }
            } else {
                if p.corner & 1 == 0 {
                    s += sh - 1;
                }
                x = if p.corner >= 2 { t - sw + 1 } else { t };
                y = if p.corner & 1 == 1 { s } else { s - sh + 1 };
                if p.corner & 1 == 1 {
                    s += sh - 1;
                }
            }
            b.compose(&sym, x as isize, y as isize, p.op);
            n += 1;
            if n >= p.instances || m.spent() {
                break;
            }
            match m.int(&mut c.ds) {
                Some(ds) => s += ds + p.ds_offset,
                None => break,
            }
        }
    }
    Some(b)
}

fn u32_at(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

fn at_pixels(d: &[u8], n: usize) -> Option<Vec<(i8, i8)>> {
    let b = d.get(..n * 2)?;
    Some(b.chunks(2).map(|p| (p[0] as i8, p[1] as i8)).collect())
}

fn id_len(n: usize) -> u32 {
    let mut l = 0;
    while (1usize << l) < n {
        l += 1;
    }
    l
}

/// A symbol dictionary segment (7.4.2, 6.5): its exported symbols.
fn symbol_dict(d: &[u8], inputs: &[Bitmap]) -> Option<Vec<Bitmap>> {
    let flags = u16::from_be_bytes(d.get(0..2)?.try_into().ok()?);
    let huff = flags & 1 != 0;
    let refagg = flags & 2 != 0;
    let template = ((flags >> 10) & 3) as u8;
    let r_template = ((flags >> 12) & 1) as u8;
    if huff {
        return None;
    }
    let mut i = 2;
    let at = at_pixels(&d[i..], if template == 0 { 4 } else { 1 })?;
    i += at.len() * 2;
    let mut r_at = [(-1, -1), (-1, -1)];
    if refagg && r_template == 0 {
        let a = at_pixels(&d[i..], 2)?;
        r_at = [a[0], a[1]];
        i += 4;
    }
    let exported = (u32_at(d, i)? as usize).min(MAX_SYMBOLS);
    let new = (u32_at(d, i + 4)? as usize).min(MAX_SYMBOLS);
    i += 8;
    let total = inputs.len() + new;
    let id_bits = id_len(total);
    let mut c = Cx::new(id_bits);
    let mut m = Mq::new(d.get(i..)?);
    let mut syms: Vec<Bitmap> = inputs.to_vec();
    let mut height = 0i64;
    while syms.len() < total && !m.spent() {
        height += m.int(&mut c.dh)?;
        let mut width = 0i64;
        while let Some(dw) = m.int(&mut c.dw) {
            width += dw;
            if syms.len() >= total || m.spent() {
                break;
            }
            let (w, h) = (width.clamp(0, 1 << 16) as usize, height.clamp(0, 1 << 16) as usize);
            let bitmap = if !refagg {
                generic(&mut m, &mut c.gb, w, h, template, false, &at)?
            } else {
                let count = m.int(&mut c.ai)?;
                if count == 1 {
                    let id = m.id(&mut c.id, id_bits);
                    let rdx = m.int(&mut c.rdx)?;
                    let rdy = m.int(&mut c.rdy)?;
                    let reference = syms.get(id)?.clone();
                    refine(&mut m, &mut c.gr, w, h, r_template, &reference, rdx as isize, rdy as isize, false, &r_at)?
                } else {
                    let p = Text { w, h, refine: true, strips: 1, corner: 1, transposed: false, op: 0, default: 0, ds_offset: 0, r_template, r_at, instances: count.clamp(0, MAX_INSTANCES as i64) as usize, id_len: id_bits };
                    let known = syms.clone();
                    text(&mut m, &mut c, &p, &known)?
                }
            };
            syms.push(bitmap);
        }
    }
    // Which symbols are exported: runs of alternating off and on.
    let mut out = Vec::new();
    let mut on = false;
    let mut k = 0usize;
    let mut turns = 0usize;
    while k < syms.len() && out.len() < exported && !m.spent() && turns <= syms.len() * 2 + 4 {
        turns += 1;
        let run = m.int(&mut c.ex)?.max(0) as usize;
        for s in syms.iter().skip(k).take(run) {
            if on {
                out.push(s.clone());
            }
        }
        k = k.saturating_add(run);
        on = !on;
    }
    Some(out)
}

/// A region segment's information field (7.4.1): width, height, x, y and
/// the combination operator.
fn region_info(d: &[u8]) -> Option<(usize, u32, isize, isize, u8)> {
    Some((u32_at(d, 0)? as usize, u32_at(d, 4)?, u32_at(d, 8)? as isize, u32_at(d, 12)? as isize, *d.get(16)? & 7))
}

/// A text region segment (7.4.3).
fn text_region(d: &[u8], syms: &[Bitmap]) -> Option<(Bitmap, isize, isize, u8)> {
    let (w, h, x, y, op) = region_info(d)?;
    let flags = u16::from_be_bytes(d.get(17..19)?.try_into().ok()?);
    if flags & 1 != 0 {
        return None;
    }
    let refine_on = flags & 2 != 0;
    let r_template = ((flags >> 15) & 1) as u8;
    let mut i = 19;
    let mut r_at = [(-1, -1), (-1, -1)];
    if refine_on && r_template == 0 {
        let a = at_pixels(&d[i..], 2)?;
        r_at = [a[0], a[1]];
        i += 4;
    }
    let instances = u32_at(d, i)? as usize;
    i += 4;
    let ds = ((flags >> 10) & 0x1F) as i64;
    let p = Text {
        w,
        h: h as usize,
        refine: refine_on,
        strips: 1 << ((flags >> 2) & 3),
        corner: ((flags >> 4) & 3) as u8,
        transposed: flags & 0x40 != 0,
        op: ((flags >> 7) & 3) as u8,
        default: ((flags >> 9) & 1) as u8,
        ds_offset: if ds >= 16 { ds - 32 } else { ds },
        r_template,
        r_at,
        instances,
        id_len: id_len(syms.len()),
    };
    let mut c = Cx::new(p.id_len);
    let mut m = Mq::new(d.get(i..)?);
    Some((text(&mut m, &mut c, &p, syms)?, x, y, op))
}

/// A generic region segment (7.4.6).
fn generic_region(d: &[u8]) -> Option<(Bitmap, isize, isize, u8)> {
    let (w, mut h, x, y, op) = region_info(d)?;
    let flags = *d.get(17)?;
    let mmr = flags & 1 != 0;
    let template = (flags >> 1) & 3;
    let tpgdon = flags & 8 != 0;
    if h == 0xFFFF_FFFF {
        // Height unknown: the row count ends the data (7.2.7).
        h = u32_at(d, d.len().checked_sub(4)?)?;
    }
    let h = h as usize;
    if mmr {
        return Some((generic_mmr(d.get(18..)?, w, h)?, x, y, op));
    }
    let at = at_pixels(d.get(18..)?, if template == 0 { 4 } else { 1 })?;
    let mut m = Mq::new(d.get(18 + at.len() * 2..)?);
    let mut cx = vec![0u8; 1 << 16];
    Some((generic(&mut m, &mut cx, w, h, template, tpgdon, &at)?, x, y, op))
}

/// One segment's header (7.2): number, type, segments referred to, and its data.
struct Segment<'a> {
    number: u32,
    kind: u8,
    refers: Vec<u32>,
    data: &'a [u8],
}

fn segments(d: &[u8]) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 11 <= d.len() && out.len() < 1 << 20 {
        let Some(s) = segment(d, &mut i) else { break };
        let end = s.kind == 51;
        out.push(s);
        if end {
            break;
        }
    }
    out
}

fn segment<'a>(d: &'a [u8], i: &mut usize) -> Option<Segment<'a>> {
    let number = u32_at(d, *i)?;
    let flags = *d.get(*i + 4)?;
    let kind = flags & 63;
    let mut p = *i + 5;
    let first = *d.get(p)?;
    let count;
    if first >> 5 == 7 {
        count = (u32_at(d, p)? & 0x1FFF_FFFF) as usize;
        p += 4 + (count + 8) / 8;
    } else {
        count = (first >> 5) as usize;
        p += 1;
    }
    if count > 1 << 16 {
        return None;
    }
    let size = if number <= 256 { 1 } else if number <= 65536 { 2 } else { 4 };
    let mut refers = Vec::with_capacity(count);
    for _ in 0..count {
        let b = d.get(p..p + size)?;
        refers.push(b.iter().fold(0u32, |v, &x| v << 8 | x as u32));
        p += size;
    }
    p += if flags & 64 != 0 { 4 } else { 1 };
    let len = u32_at(d, p)?;
    p += 4;
    let start = p;
    let end = if len == 0xFFFF_FFFF {
        // Only an immediate generic region may leave its length unknown:
        // its data ends with a marker and the row count (7.2.7).
        let mmr = d.get(start + 17).is_some_and(|f| f & 1 != 0);
        let marker: [u8; 2] = if mmr { [0, 0] } else { [0xFF, 0xAC] };
        let mut k = start + 18;
        loop {
            if k + 6 > d.len() {
                break d.len();
            }
            if d[k..k + 2] == marker {
                break k + 6;
            }
            k += 1;
        }
    } else {
        start.checked_add(len as usize)?.min(d.len())
    };
    *i = end;
    Some(Segment { number, kind, refers, data: &d[start..end] })
}

/// Decodes a JBIG2 stream as PDF embeds it (no file header), with the
/// /JBIG2Globals stream's segments first: the page, 1 black.
pub fn decode(globals: Option<&[u8]>, data: &[u8]) -> Option<Bitmap> {
    let mut dicts: HashMap<u32, Vec<Bitmap>> = HashMap::new();
    let mut page: Option<Bitmap> = None;
    let mut striped = false;
    let all: Vec<Segment> = globals.map(segments).unwrap_or_default().into_iter().chain(segments(data)).collect();
    for s in &all {
        match s.kind {
            0 => {
                let inputs: Vec<Bitmap> = s.refers.iter().filter_map(|r| dicts.get(r)).flatten().cloned().collect();
                if let Some(out) = symbol_dict(s.data, &inputs) {
                    dicts.insert(s.number, out);
                }
            }
            48 => {
                let w = u32_at(s.data, 0)? as usize;
                let h = u32_at(s.data, 4)?;
                let default = (*s.data.get(16)? >> 2) & 1;
                striped = h == 0xFFFF_FFFF;
                page = Some(Bitmap::new(w, if striped { 0 } else { h as usize }, default)?);
            }
            4 | 6 | 7 | 36 | 38 | 39 => {
                let region = if s.kind <= 7 {
                    let syms: Vec<Bitmap> = s.refers.iter().filter_map(|r| dicts.get(r)).flatten().cloned().collect();
                    text_region(s.data, &syms)
                } else {
                    generic_region(s.data)
                };
                let (Some(p), Some((r, x, y, op))) = (page.as_mut(), region) else { continue };
                let bottom = (y.max(0) as usize).saturating_add(r.h);
                if striped && bottom > p.h && p.w.saturating_mul(bottom) <= MAX_PIXELS {
                    p.px.resize(p.w * bottom, 0);
                    p.h = bottom;
                }
                p.compose(&r, x, y, op);
            }
            50 => {
                // End of stripe: the page is at least this tall.
                if let (Some(p), Some(row)) = (page.as_mut(), u32_at(s.data, 0)) {
                    let h = row as usize + 1;
                    if striped && h > p.h && p.w.saturating_mul(h) <= MAX_PIXELS {
                        p.px.resize(p.w * h, 0);
                        p.h = h;
                    }
                }
            }
            _ => {}
        }
    }
    page
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T.88 Annex H.2: the MQ coder's test sequence, every decision in one
    /// context.
    #[test]
    fn mq_decoder_test_sequence() {
        let coded = [0x84, 0xC7, 0x3B, 0xFC, 0xE1, 0xA1, 0x43, 0x04, 0x02, 0x20, 0x00, 0x00, 0x41, 0x0D, 0xBB, 0x86, 0xF4, 0x31, 0x7F, 0xFF, 0x88, 0xFF, 0x37, 0x47, 0x1A, 0xDB, 0x6A, 0xDF, 0xFF, 0xAC];
        let want = [0x00, 0x02, 0x00, 0x51, 0x00, 0x00, 0x00, 0xC0, 0x03, 0x52, 0x87, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x82, 0xC0, 0x20, 0x00, 0xFC, 0xD7, 0x9E, 0xF6, 0xBF, 0x7F, 0xED, 0x90, 0x4F, 0x46, 0xA3, 0xBF];
        let mut m = Mq::new(&coded);
        let mut cx = [0u8; 1];
        let got: Vec<u8> = (0..want.len()).map(|_| (0..8).fold(0u8, |v, _| v << 1 | m.bit(&mut cx, 0))).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn region_operators() {
        let mut page = Bitmap::new(4, 1, 0).unwrap();
        page.px = vec![0, 1, 0, 1];
        let src = Bitmap { w: 4, h: 1, px: vec![0, 0, 1, 1] };
        for (op, want) in [(0, [0, 1, 1, 1]), (1, [0, 0, 0, 1]), (2, [0, 1, 1, 0]), (3, [1, 0, 0, 1]), (4, [0, 0, 1, 1])] {
            let mut p = page.clone();
            p.compose(&src, 0, 0, op);
            assert_eq!(p.px, want, "op {op}");
        }
    }

    #[test]
    fn segment_headers() {
        // Segment 3, a text region (6) on page 1, referring to segments 1 and 2, 2 bytes of data.
        let d = [0, 0, 0, 3, 6, 0b0100_0000, 1, 2, 1, 0, 0, 0, 2, 0xAB, 0xCD];
        let s = segments(&d);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].number, s[0].kind, s[0].refers.clone(), s[0].data), (3, 6, vec![1, 2], &[0xAB, 0xCD][..]));
    }

    #[test]
    fn damage_never_panics() {
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let page = [0, 0, 0, 0, 48, 0, 1, 0, 0, 0, 19, 0, 0, 0, 64, 0, 0, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        for kind in [0u8, 6, 38, 36, 39, 50] {
            for n in 0..60 {
                let mut seg = vec![0, 0, 0, 1, kind, 0, 1, 0, 0, 0, 40];
                for _ in 0..40 {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    seg.push((x >> 32) as u8);
                }
                if kind != 0 && n % 4 != 0 {
                    // Small regions mostly, so the run is quick.
                    seg[11..19].fill(0);
                    seg[14] = 40;
                    seg[18] = 30;
                }
                let mut d = page.to_vec();
                d.extend(&seg);
                let _ = decode(None, &d);
            }
        }
    }
}
