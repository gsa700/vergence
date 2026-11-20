//! JPEG 2000 (ITU-T T.800 / ISO/IEC 15444-1), our own decoder (PDF
//! milestone 1, step 5's last part; Dave, 2026-11-20): what PDF's
//! JPXDecode carries. A JP2 file's boxes (the codestream, its color space,
//! a palette, which channel is opacity) or a bare codestream; tiles,
//! components with their own sampling, any number of quality layers, all
//! five progression orders, precincts, SOP and EPH markers; tier 2 (packet
//! headers: tag trees, coding passes, lengths); tier 1 (the three coding
//! passes per bit plane with the MQ coder of jbig2.rs, which is the same
//! coder, and every code-block style: bypass, reset, terminate all,
//! vertically causal, segmentation symbols); dequantization; the inverse
//! 5/3 (exact integers) and 9/7 (floating point) wavelets; the RCT and ICT
//! component transforms; DC level shifts.
//!
//! Not read: ROI shifts (RGN: drawn without the shift), packed packet
//! headers (PPM, PPT: refused), POC progression changes (the main order is
//! used), Part 2 extensions.
//!
//! **Input is untrusted**: sizes, counts and lengths are bounded, reads past
//! the end give what the standard says an exhausted stream gives, nothing
//! panics (the fuzz test).

use crate::jbig2::Mq;

/// The most samples a picture's component may have (8192 x 8192).
const MAX_PIXELS: usize = 1 << 26;
const MAX_COMPONENTS: usize = 16;
const MAX_TILES: usize = 1 << 16;
const MAX_LEVELS: usize = 32;
/// Code blocks and packets in a whole picture.
const MAX_BLOCKS: usize = 1 << 22;
const MAX_PACKETS: usize = 1 << 24;

/// A decoded picture: each channel at the picture's full size, rows from
/// the top, as unsigned samples of its own bit depth.
#[derive(Debug, Clone)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub channels: Vec<Channel>,
    /// The JP2 file's color space, when it names one we know.
    pub color: Option<Jp2Color>,
    /// The channel that is opacity (JP2's cdef), if any.
    pub alpha: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub depth: u8,
    pub data: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jp2Color {
    Gray,
    Rgb,
    Cmyk,
}

type Res<T> = Result<T, &'static str>;

/// Decodes a JP2 file or a bare codestream.
pub fn decode(data: &[u8]) -> Res<Picture> {
    if data.starts_with(&[0xFF, 0x4F, 0xFF, 0x51]) {
        return codestream(data, None);
    }
    let info = jp2(data)?;
    let stream = info.codestream.ok_or("no codestream")?;
    let mut pic = codestream(stream, info.ycc.then_some(()))?;
    pic.color = info.color;
    // A palette: each output channel a component as it is or one of the
    // palette's columns looked up by a component.
    if let Some(pal) = &info.palette {
        let map: Vec<(usize, u8, usize)> = if info.cmap.is_empty() { (0..pal.columns.len()).map(|c| (0, 1, c)).collect() } else { info.cmap.clone() };
        let mut out = Vec::new();
        for (cmp, kind, col) in map {
            let src = pic.channels.get(cmp).ok_or("palette of a missing component")?;
            if kind == 0 {
                out.push(src.clone());
            } else {
                let (depth, column) = pal.columns.get(col).ok_or("palette column")?;
                let data = src.data.iter().map(|v| column.get(*v as usize).copied().unwrap_or(0)).collect();
                out.push(Channel { depth: *depth, data });
            }
        }
        pic.channels = out;
    }
    pic.alpha = info.opacity.filter(|a| *a < pic.channels.len());
    Ok(pic)
}

// ---------------------------------------------------------------- JP2 boxes

struct Palette {
    /// Each column's depth and entries.
    columns: Vec<(u8, Vec<u16>)>,
}

#[derive(Default)]
struct Jp2<'a> {
    codestream: Option<&'a [u8]>,
    color: Option<Jp2Color>,
    ycc: bool,
    palette: Option<Palette>,
    cmap: Vec<(usize, u8, usize)>,
    opacity: Option<usize>,
}

/// A run of boxes: each one's type and contents.
fn boxes(mut d: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    while d.len() >= 8 && out.len() < 4096 {
        let len = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) as usize;
        let kind = [d[4], d[5], d[6], d[7]];
        let (head, len) = match len {
            0 => (8, d.len()),
            1 if d.len() >= 16 => (16, usize::try_from(u64::from_be_bytes([d[8], d[9], d[10], d[11], d[12], d[13], d[14], d[15]])).unwrap_or(usize::MAX)),
            1 => break,
            n => (8, n),
        };
        if len < head {
            break;
        }
        let end = len.min(d.len());
        out.push((kind, &d[head..end]));
        d = &d[end..];
    }
    out
}

fn jp2(data: &[u8]) -> Res<Jp2<'_>> {
    let mut info = Jp2::default();
    for (kind, body) in boxes(data) {
        match &kind {
            b"jp2c" if info.codestream.is_none() => info.codestream = Some(body),
            b"jp2h" => {
                for (kind, b) in boxes(body) {
                    match &kind {
                        b"colr" if b.len() >= 7 && b[0] == 1 && info.color.is_none() && !info.ycc => {
                            match u32::from_be_bytes([b[3], b[4], b[5], b[6]]) {
                                16 => info.color = Some(Jp2Color::Rgb),
                                17 => info.color = Some(Jp2Color::Gray),
                                12 => info.color = Some(Jp2Color::Cmyk),
                                18 => {
                                    info.color = Some(Jp2Color::Rgb);
                                    info.ycc = true;
                                }
                                _ => {}
                            }
                        }
                        b"pclr" if b.len() >= 3 => {
                            let entries = u16::from_be_bytes([b[0], b[1]]) as usize;
                            let cols = b[2] as usize;
                            let depths: Vec<u8> = b.get(3..3 + cols).ok_or("pclr")?.iter().map(|x| (x & 0x7F) + 1).collect();
                            let mut at = 3 + cols;
                            let mut columns: Vec<(u8, Vec<u16>)> = depths.iter().map(|d| (*d, Vec::with_capacity(entries))).collect();
                            for _ in 0..entries {
                                for (c, d) in depths.iter().enumerate() {
                                    let n = (*d as usize).div_ceil(8);
                                    let v = b.get(at..at + n).ok_or("pclr entries")?.iter().fold(0u32, |a, x| (a << 8) | *x as u32);
                                    at += n;
                                    columns[c].1.push(v.min(u16::MAX as u32) as u16);
                                }
                            }
                            if columns.iter().all(|(d, _)| *d <= 16) {
                                info.palette = Some(Palette { columns });
                            }
                        }
                        b"cmap" => {
                            info.cmap = b.chunks(4).filter(|c| c.len() == 4).map(|c| (u16::from_be_bytes([c[0], c[1]]) as usize, c[2], c[3] as usize)).collect();
                        }
                        b"cdef" if b.len() >= 2 => {
                            let n = u16::from_be_bytes([b[0], b[1]]) as usize;
                            for e in b[2..].chunks(6).take(n).filter(|e| e.len() == 6) {
                                let (ch, typ) = (u16::from_be_bytes([e[0], e[1]]) as usize, u16::from_be_bytes([e[2], e[3]]));
                                if typ == 1 || typ == 2 {
                                    info.opacity = Some(ch);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(info)
}

// ------------------------------------------------------ the codestream

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Res<u8> {
        let v = *self.d.get(self.p).ok_or("short")?;
        self.p += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Res<u16> {
        Ok(u16::from_be_bytes([self.u8()?, self.u8()?]))
    }
    fn u32(&mut self) -> Res<u32> {
        Ok(u32::from_be_bytes([self.u8()?, self.u8()?, self.u8()?, self.u8()?]))
    }
}

#[derive(Debug, Clone, Copy)]
struct CompSiz {
    depth: u8,
    signed: bool,
    dx: usize,
    dy: usize,
}

struct Siz {
    x0: usize,
    y0: usize,
    x1: usize,
    comps: Vec<CompSiz>,
}

/// A component's coding style (COD's or COC's SPcod).
#[derive(Debug, Clone)]
struct Style {
    levels: usize,
    xcb: u8,
    ycb: u8,
    cb: u8,
    reversible: bool,
    /// Per resolution, the precinct size's exponents (15, 15 when none).
    precincts: Vec<(u8, u8)>,
}

#[derive(Debug, Clone)]
struct Quant {
    guard: u8,
    kind: u8,
    /// Per subband: the exponent and mantissa.
    steps: Vec<(i32, u16)>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Global {
    order: u8,
    layers: usize,
    mct: u8,
    sop: bool,
    eph: bool,
}

#[derive(Clone, Default)]
struct Header {
    global: Option<Global>,
    cod: Option<Style>,
    coc: Vec<Option<Style>>,
    qcd: Option<Quant>,
    qcc: Vec<Option<Quant>>,
    packed: bool,
}

// Code-block style bits (Table A.19).
const BYPASS: u8 = 0x01;
const RESET: u8 = 0x02;
const TERMALL: u8 = 0x04;
const VSC: u8 = 0x08;
const SEGSYM: u8 = 0x20;

fn style(r: &mut Reader, precincts: bool) -> Res<Style> {
    let levels = r.u8()? as usize;
    let (xcb, ycb, cb, t) = (r.u8()? + 2, r.u8()? + 2, r.u8()?, r.u8()?);
    if levels > MAX_LEVELS || xcb > 10 || ycb > 10 || xcb + ycb > 12 {
        return Err("coding style");
    }
    let precincts = if precincts {
        (0..=levels).map(|_| r.u8().map(|b| (b & 0x0F, b >> 4))).collect::<Res<Vec<_>>>()?
    } else {
        vec![(15, 15); levels + 1]
    };
    Ok(Style { levels, xcb, ycb, cb, reversible: t == 1, precincts })
}

fn quant(r: &mut Reader, end: usize) -> Res<Quant> {
    let s = r.u8()?;
    let (guard, kind) = (s >> 5, s & 0x1F);
    let mut steps = Vec::new();
    while r.p < end && steps.len() < 3 * MAX_LEVELS + 1 {
        if kind == 0 {
            steps.push(((r.u8()? >> 3) as i32, 0));
        } else {
            let v = r.u16()?;
            steps.push(((v >> 11) as i32, v & 0x7FF));
        }
    }
    if steps.is_empty() || kind > 2 {
        return Err("quantization");
    }
    Ok(Quant { guard, kind, steps })
}

/// Reads a COD, COC, QCD or QCC segment (or notes PPM/PPT) into `h`.
fn marker(h: &mut Header, m: u16, seg: &[u8], ncomp: usize) -> Res<()> {
    let mut r = Reader { d: seg, p: 0 };
    let comp = |r: &mut Reader| -> Res<usize> { if ncomp < 257 { r.u8().map(usize::from) } else { r.u16().map(usize::from) } };
    match m {
        0xFF52 => {
            let s = r.u8()?;
            let order = r.u8()?;
            let layers = r.u16()? as usize;
            let mct = r.u8()?;
            h.global = Some(Global { order, layers: layers.max(1), mct, sop: s & 2 != 0, eph: s & 4 != 0 });
            h.cod = Some(style(&mut r, s & 1 != 0)?);
        }
        0xFF53 => {
            let c = comp(&mut r)?;
            let s = r.u8()?;
            let st = style(&mut r, s & 1 != 0)?;
            if let Some(slot) = h.coc.get_mut(c) {
                *slot = Some(st);
            }
        }
        0xFF5C => h.qcd = Some(quant(&mut r, seg.len())?),
        0xFF5D => {
            let c = comp(&mut r)?;
            let q = quant(&mut r, seg.len())?;
            if let Some(slot) = h.qcc.get_mut(c) {
                *slot = Some(q);
            }
        }
        0xFF60 | 0xFF61 => h.packed = true,
        _ => {}
    }
    Ok(())
}

fn ceil_div(a: usize, b: usize) -> usize {
    if b == 0 { 0 } else { a.div_ceil(b) }
}

/// ceil(a / 2^s), a may be negative (a band's start).
fn ceil_shift(a: i64, s: usize) -> i64 {
    if s >= 63 {
        return if a > 0 { 1 } else { 0 };
    }
    let d = 1i64 << s;
    a.div_euclid(d) + i64::from(a.rem_euclid(d) != 0)
}

fn codestream(d: &[u8], ycc: Option<()>) -> Res<Picture> {
    let mut r = Reader { d, p: 2 };
    // SIZ follows SOC.
    if r.u16()? != 0xFF51 {
        return Err("no SIZ");
    }
    let lsiz = r.u16()? as usize;
    let end = r.p - 2 + lsiz;
    let _rsiz = r.u16()?;
    let (x1, y1, x0, y0) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
    let (tw, th, tx0, ty0) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
    let n = r.u16()? as usize;
    if n == 0 || n > MAX_COMPONENTS || x1 <= x0 || y1 <= y0 || tw == 0 || th == 0 || tx0 > x0 || ty0 > y0 {
        return Err("SIZ");
    }
    let mut comps = Vec::new();
    for _ in 0..n {
        let s = r.u8()?;
        let (dx, dy) = (r.u8()? as usize, r.u8()? as usize);
        let depth = (s & 0x7F) + 1;
        if dx == 0 || dy == 0 || depth > 16 {
            return Err("component");
        }
        comps.push(CompSiz { depth, signed: s & 0x80 != 0, dx, dy });
    }
    r.p = end;
    let siz = Siz { x0, y0, x1, comps };
    let (width, height) = (x1 - x0, y1 - y0);
    if width.saturating_mul(height) > MAX_PIXELS {
        return Err("too large");
    }
    let (ntx, nty) = (ceil_div(x1 - tx0, tw), ceil_div(y1 - ty0, th));
    if ntx.saturating_mul(nty) > MAX_TILES {
        return Err("too many tiles");
    }
    // The main header, up to the first tile-part.
    let mut main = Header { coc: vec![None; n], qcc: vec![None; n], ..Default::default() };
    loop {
        let m = r.u16()?;
        if m == 0xFF90 {
            r.p -= 2;
            break;
        }
        let len = r.u16()? as usize;
        let seg = d.get(r.p..(r.p + len).saturating_sub(2)).ok_or("marker")?;
        marker(&mut main, m, seg, n)?;
        r.p += len.saturating_sub(2);
    }
    // Tile-parts: each tile's header (its first part's) and its data.
    let mut tiles: Vec<(Option<Header>, Vec<u8>)> = (0..ntx * nty).map(|_| (None, Vec::new())).collect();
    while r.p + 12 <= d.len() {
        let start = r.p;
        if r.u16()? != 0xFF90 {
            break;
        }
        let _lsot = r.u16()?;
        let (isot, psot) = (r.u16()? as usize, r.u32()? as usize);
        let (_tpsot, _tnsot) = (r.u8()?, r.u8()?);
        if psot != 0 && psot < 14 {
            break;
        }
        let part_end = if psot == 0 { d.len() } else { (start + psot).min(d.len()) };
        let Some(tile) = tiles.get_mut(isot) else { return Err("tile index") };
        let mut h = tile.0.clone().unwrap_or_else(|| Header { coc: vec![None; n], qcc: vec![None; n], ..Default::default() });
        loop {
            let m = r.u16()?;
            if m == 0xFF93 {
                break;
            }
            let len = r.u16()? as usize;
            let seg = d.get(r.p..(r.p + len).saturating_sub(2)).ok_or("marker")?;
            marker(&mut h, m, seg, n)?;
            r.p += len.saturating_sub(2);
        }
        tile.0 = Some(h);
        if r.p < part_end {
            tile.1.extend_from_slice(&d[r.p..part_end]);
        }
        r.p = part_end;
        if d.get(r.p..r.p + 2) == Some(&[0xFF, 0xD9]) {
            break;
        }
    }
    // Each component's plane, in its own (subsampled) grid.
    let mut planes: Vec<Vec<u16>> = Vec::new();
    for c in &siz.comps {
        let (cw, ch) = (ceil_div(x1, c.dx) - ceil_div(x0, c.dx), ceil_div(y1, c.dy) - ceil_div(y0, c.dy));
        planes.push(vec![if c.signed || c.depth == 0 { 0 } else { 1 << (c.depth - 1) }; cw * ch]);
    }
    let mut budget = Budget { blocks: 0, packets: 0 };
    for (i, (h, data)) in tiles.iter().enumerate() {
        let Some(h) = h else { continue };
        if main.packed || h.packed {
            return Err("packed packet headers");
        }
        let (tx, ty) = (i % ntx, i / ntx);
        let rect = [(tx0 + tx * tw).max(x0), (ty0 + ty * th).max(y0), (tx0 + (tx + 1) * tw).min(x1), (ty0 + (ty + 1) * th).min(y1)];
        tile(&siz, &main, h, rect, data, &mut planes, &mut budget)?;
    }
    // At full size: each sample from its component's grid.
    let mut channels = Vec::new();
    for (c, plane) in siz.comps.iter().zip(&planes) {
        let (cx0, cy0) = (ceil_div(x0, c.dx), ceil_div(y0, c.dy));
        let cw = ceil_div(x1, c.dx) - cx0;
        let data = if c.dx == 1 && c.dy == 1 {
            plane.clone()
        } else {
            let mut v = Vec::with_capacity(width * height);
            for y in 0..height {
                let row = ((y0 + y) / c.dy).saturating_sub(cy0) * cw;
                for x in 0..width {
                    v.push(plane.get(row + ((x0 + x) / c.dx).saturating_sub(cx0)).copied().unwrap_or(0));
                }
            }
            v
        };
        channels.push(Channel { depth: c.depth, data });
    }
    if ycc.is_some() && channels.len() >= 3 && channels[..3].iter().all(|c| c.depth == channels[0].depth) {
        sycc(&mut channels);
    }
    Ok(Picture { width, height, channels, color: None, alpha: None })
}

/// sYCC (JP2 color space 18) to RGB.
fn sycc(ch: &mut [Channel]) {
    let max = ((1u32 << ch[0].depth) - 1) as f32;
    let half = (1u32 << (ch[0].depth - 1)) as f32;
    for i in 0..ch[0].data.len() {
        let (y, cb, cr) = (ch[0].data[i] as f32, ch[1].data.get(i).copied().unwrap_or(0) as f32 - half, ch[2].data.get(i).copied().unwrap_or(0) as f32 - half);
        let rgb = [y + 1.402 * cr, y - 0.344_136 * cb - 0.714_136 * cr, y + 1.772 * cb];
        for (k, v) in rgb.iter().enumerate() {
            if let Some(s) = ch[k].data.get_mut(i) {
                *s = v.round().clamp(0.0, max) as u16;
            }
        }
    }
}

struct Budget {
    blocks: usize,
    packets: usize,
}

// --------------------------------------------------- tiles and their parts

struct Seg {
    data: Vec<u8>,
    passes: u32,
    max: u32,
    raw: bool,
}

#[derive(Default)]
struct Block {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    included: bool,
    lblock: u32,
    zbp: u32,
    passes: u32,
    segs: Vec<Seg>,
    /// Lengths read in the packet header now being read.
    pending: Vec<(usize, u32)>,
}

struct Prec {
    /// The code blocks it holds, as a range in the band's grid.
    bx0: usize,
    by0: usize,
    bw: usize,
    bh: usize,
    incl: TagTree,
    zbp: TagTree,
}

struct Band {
    /// 0 LL, 1 HL, 2 LH, 3 HH.
    kind: usize,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    gw: usize,
    blocks: Vec<Block>,
    precs: Vec<Prec>,
    eps: i32,
    mu: u16,
}

struct Resolution {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    ppx: u8,
    ppy: u8,
    pw: usize,
    ph: usize,
    kx0: usize,
    ky0: usize,
    bands: Vec<Band>,
}

struct TileComp {
    /// The component's sample depth (the nominal range of E-4).
    depth: u8,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    style: Style,
    guard: u8,
    res: Vec<Resolution>,
}

fn tile_comp(c: &CompSiz, rect: [usize; 4], style: Style, q: Quant, budget: &mut Budget) -> Res<TileComp> {
    let (x0, y0, x1, y1) = (ceil_div(rect[0], c.dx), ceil_div(rect[1], c.dy), ceil_div(rect[2], c.dx), ceil_div(rect[3], c.dy));
    let nl = style.levels;
    let mut res = Vec::new();
    for r in 0..=nl {
        let s = nl - r;
        let (rx0, ry0, rx1, ry1) = (ceil_shift(x0 as i64, s) as usize, ceil_shift(y0 as i64, s) as usize, ceil_shift(x1 as i64, s) as usize, ceil_shift(y1 as i64, s) as usize);
        let (ppx, ppy) = style.precincts.get(r).copied().unwrap_or((15, 15));
        if r > 0 && (ppx == 0 || ppy == 0) {
            return Err("precinct size");
        }
        let (kx0, ky0) = (rx0 >> ppx, ry0 >> ppy);
        let pw = if rx1 > rx0 { ceil_shift(rx1 as i64, ppx as usize) as usize - kx0 } else { 0 };
        let ph = if ry1 > ry0 { ceil_shift(ry1 as i64, ppy as usize) as usize - ky0 } else { 0 };
        if pw.saturating_mul(ph) > MAX_BLOCKS {
            return Err("too many precincts");
        }
        let kinds: &[(usize, i64, i64)] = if r == 0 { &[(0, 0, 0)] } else { &[(1, 1, 0), (2, 0, 1), (3, 1, 1)] };
        let n = if r == 0 { nl } else { nl - r + 1 };
        let (pbx, pby) = if r == 0 { (ppx, ppy) } else { (ppx - 1, ppy - 1) };
        let (cbw, cbh) = (style.xcb.min(pbx), style.ycb.min(pby));
        let mut bands = Vec::new();
        for &(kind, xob, yob) in kinds {
            let off = |o: i64| if n == 0 { 0 } else { o << (n - 1) };
            let bx0 = ceil_shift(x0 as i64 - off(xob), n).max(0) as usize;
            let by0 = ceil_shift(y0 as i64 - off(yob), n).max(0) as usize;
            let bx1 = ceil_shift(x1 as i64 - off(xob), n).max(0) as usize;
            let by1 = ceil_shift(y1 as i64 - off(yob), n).max(0) as usize;
            // The code-block grid, anchored at 0.
            let (gx0, gy0) = (bx0 >> cbw, by0 >> cbh);
            let gw = if bx1 > bx0 { ceil_shift(bx1 as i64, cbw as usize) as usize - gx0 } else { 0 };
            let gh = if by1 > by0 { ceil_shift(by1 as i64, cbh as usize) as usize - gy0 } else { 0 };
            budget.blocks = budget.blocks.saturating_add(gw.saturating_mul(gh));
            if budget.blocks > MAX_BLOCKS {
                return Err("too many code blocks");
            }
            let mut blocks = Vec::with_capacity(gw * gh);
            for gy in 0..gh {
                for gx in 0..gw {
                    let (cx, cy) = ((gx0 + gx) << cbw, (gy0 + gy) << cbh);
                    blocks.push(Block { x0: cx.max(bx0), y0: cy.max(by0), x1: (cx + (1 << cbw)).min(bx1), y1: (cy + (1 << cbh)).min(by1), lblock: 3, ..Default::default() });
                }
            }
            let mut precs = Vec::with_capacity(pw * ph);
            for py in 0..ph {
                for px in 0..pw {
                    let (sx0, sx1) = (((kx0 + px) << pbx).max(bx0), ((kx0 + px + 1) << pbx).min(bx1));
                    let (sy0, sy1) = (((ky0 + py) << pby).max(by0), ((ky0 + py + 1) << pby).min(by1));
                    let (b0x, b0y) = ((sx0 >> cbw).saturating_sub(gx0), (sy0 >> cbh).saturating_sub(gy0));
                    let bw = if sx1 > sx0 { (ceil_shift(sx1 as i64, cbw as usize) as usize - gx0).saturating_sub(b0x) } else { 0 };
                    let bh = if sy1 > sy0 { (ceil_shift(sy1 as i64, cbh as usize) as usize - gy0).saturating_sub(b0y) } else { 0 };
                    precs.push(Prec { bx0: b0x, by0: b0y, bw, bh, incl: TagTree::new(bw, bh), zbp: TagTree::new(bw, bh) });
                }
            }
            // Its quantization step (E.1).
            let idx = if r == 0 { 0 } else { 3 * (r - 1) + kind };
            let (eps, mu) = if q.kind == 1 {
                let (e0, m0) = q.steps[0];
                (e0 - nl as i32 + n as i32, m0)
            } else {
                *q.steps.get(idx).ok_or("quantization steps")?
            };
            bands.push(Band { kind, x0: bx0, y0: by0, x1: bx1, y1: by1, gw, blocks, precs, eps, mu });
        }
        res.push(Resolution { x0: rx0, y0: ry0, x1: rx1, y1: ry1, ppx, ppy, pw, ph, kx0, ky0, bands });
    }
    Ok(TileComp { depth: c.depth, x0, y0, x1, y1, style, guard: q.guard, res })
}

fn tile(siz: &Siz, main: &Header, h: &Header, rect: [usize; 4], data: &[u8], planes: &mut [Vec<u16>], budget: &mut Budget) -> Res<()> {
    if rect[2] <= rect[0] || rect[3] <= rect[1] {
        return Ok(());
    }
    let global = h.global.or(main.global).ok_or("no COD")?;
    let mut tcs = Vec::new();
    for (i, c) in siz.comps.iter().enumerate() {
        let style = h.coc[i].clone().or_else(|| h.cod.clone()).or_else(|| main.coc[i].clone()).or_else(|| main.cod.clone()).ok_or("no coding style")?;
        let q = h.qcc[i].clone().or_else(|| h.qcd.clone()).or_else(|| main.qcc[i].clone()).or_else(|| main.qcd.clone()).ok_or("no quantization")?;
        tcs.push(tile_comp(c, rect, style, q, budget)?);
    }
    // The packets, in the progression order (B.12).
    let mut order: Vec<([u64; 5], usize, usize, usize, usize)> = Vec::new();
    for (c, tc) in tcs.iter().enumerate() {
        let cs = siz.comps[c];
        for (r, res) in tc.res.iter().enumerate() {
            let s = tc.style.levels - r;
            for p in 0..res.pw * res.ph {
                let (i, j) = (p % res.pw, p / res.pw);
                // The precinct's place on the reference grid (B.12.1.3).
                let place = |k: usize, sh: u8, d: usize, min: usize| -> u64 { (((k as u128) << (sh as u32 + s as u32).min(100)) * d as u128).max(min as u128).min(u64::MAX as u128) as u64 };
                let (xr, yr) = (place(res.kx0 + i, res.ppx, cs.dx, rect[0]), place(res.ky0 + j, res.ppy, cs.dy, rect[1]));
                for l in 0..global.layers {
                    let (l, r64, c64, p64) = (l as u64, r as u64, c as u64, p as u64);
                    let key = match global.order {
                        0 => [l, r64, c64, p64, 0],
                        1 => [r64, l, c64, p64, 0],
                        2 => [r64, yr, xr, c64, l],
                        3 => [yr, xr, c64, r64, l],
                        _ => [c64, yr, xr, r64, l],
                    };
                    order.push((key, l as usize, c, r, p));
                    if order.len() + budget.packets > MAX_PACKETS {
                        return Err("too many packets");
                    }
                }
            }
        }
    }
    budget.packets += order.len();
    order.sort_by_key(|o| o.0);
    let mut pos = 0;
    for (_, l, c, r, p) in order {
        if pos >= data.len() {
            break;
        }
        let cb = tcs[c].style.cb;
        pos = packet(data, pos, &mut tcs[c].res[r], p, l, global, cb)?;
    }
    // Each component decoded, then the component transform, then into its plane.
    let mut out: Vec<Vec<f32>> = tcs.iter().map(decode_tile_comp).collect();
    if global.mct == 1 && out.len() >= 3 && out[0].len() == out[1].len() && out[1].len() == out[2].len() {
        let (a, rest) = out.split_at_mut(1);
        let (b, c) = rest.split_at_mut(1);
        for ((y0, y1), y2) in a[0].iter_mut().zip(b[0].iter_mut()).zip(c[0].iter_mut()) {
            if tcs[0].style.reversible {
                let (i0, i1, i2) = (*y0 as i32, *y1 as i32, *y2 as i32);
                let g = i0 - ((i2 + i1) >> 2);
                (*y0, *y1, *y2) = ((i2 + g) as f32, g as f32, (i1 + g) as f32);
            } else {
                let (y, cb, cr) = (*y0, *y1, *y2);
                (*y0, *y1, *y2) = (y + 1.402 * cr, y - 0.344_13 * cb - 0.714_14 * cr, y + 1.772 * cb);
            }
        }
    }
    for (c, (tc, v)) in tcs.iter().zip(out).enumerate() {
        let cs = siz.comps[c];
        let (cx0, cy0) = (ceil_div(siz.x0, cs.dx), ceil_div(siz.y0, cs.dy));
        let cw = ceil_div(siz.x1, cs.dx) - cx0;
        let max = ((1u32 << cs.depth) - 1) as f32;
        let shift = if cs.depth == 0 { 0.0 } else { (1u32 << (cs.depth - 1)) as f32 };
        let w = tc.x1 - tc.x0;
        for y in tc.y0..tc.y1 {
            for x in tc.x0..tc.x1 {
                let s = v[(y - tc.y0) * w + (x - tc.x0)];
                // Signed samples are shown shifted into the unsigned range too.
                let s = (s + shift).round().clamp(0.0, max) as u16;
                if let Some(px) = planes[c].get_mut((y - cy0) * cw + (x - cx0)) {
                    *px = s;
                }
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------- tag trees

/// A tag-tree level: its width, height and nodes' (value, low).
type Level = (usize, usize, Vec<(u32, u32)>);

struct TagTree {
    /// Its levels, leaves first.
    levels: Vec<Level>,
}

impl TagTree {
    fn new(w: usize, h: usize) -> TagTree {
        let mut levels = Vec::new();
        let (mut w, mut h) = (w, h);
        loop {
            levels.push((w, h, vec![(u32::MAX, 0); w * h]));
            if w <= 1 && h <= 1 {
                break;
            }
            w = w.div_ceil(2).max(1);
            h = h.div_ceil(2).max(1);
        }
        TagTree { levels }
    }

    /// Whether leaf `x`, `y`'s value is below `t`, reading what it takes (B.10.2).
    fn below(&mut self, x: usize, y: usize, t: u32, bits: &mut HeaderBits) -> bool {
        let depth = self.levels.len();
        let mut low = 0;
        for lv in (0..depth).rev() {
            let (w, _, nodes) = &mut self.levels[lv];
            let Some(node) = nodes.get_mut((y >> lv) * *w + (x >> lv)) else { return false };
            if low > node.1 {
                node.1 = low;
            } else {
                low = node.1;
            }
            while low < t && low < node.0 {
                if bits.bit() == 1 {
                    node.0 = low;
                } else {
                    low += 1;
                }
            }
            node.1 = low;
        }
        self.levels[0].2.get(y * self.levels[0].0 + x).is_some_and(|n| n.0 < t)
    }

    fn value(&mut self, x: usize, y: usize, bits: &mut HeaderBits) -> u32 {
        let mut t = 1;
        while !self.below(x, y, t, bits) && t < 64 {
            t += 1;
        }
        self.levels[0].2.get(y * self.levels[0].0 + x).map_or(0, |n| n.0.min(63))
    }
}

/// A packet header's bits: after a 0xFF byte the next byte holds only 7.
struct HeaderBits<'a> {
    d: &'a [u8],
    p: usize,
    byte: u8,
    left: u8,
}

impl HeaderBits<'_> {
    fn bit(&mut self) -> u32 {
        if self.left == 0 {
            let ff = self.byte == 0xFF;
            self.byte = self.d.get(self.p).copied().unwrap_or(0);
            self.p += 1;
            self.left = if ff { 7 } else { 8 };
        }
        self.left -= 1;
        u32::from((self.byte >> self.left) & 1)
    }

    fn bits(&mut self, n: u32) -> u32 {
        (0..n.min(32)).fold(0, |v, _| (v << 1) | self.bit())
    }

    /// The header's end on a byte (one more byte after a 0xFF).
    fn end(&mut self) -> usize {
        if self.byte == 0xFF {
            self.p += 1;
        }
        self.p
    }
}

/// How many passes a segment starting at pass `k` holds, and whether it is
/// raw (the bypass mode's significance and refinement passes).
fn segment(k: u32, style: u8) -> (u32, bool) {
    let raw = style & BYPASS != 0 && k >= 10 && !k.is_multiple_of(3);
    if style & TERMALL != 0 {
        (1, raw)
    } else if style & BYPASS != 0 {
        if k < 10 {
            (10 - k, false)
        } else if k % 3 == 1 {
            (2, true)
        } else {
            (1, raw)
        }
    } else {
        (u32::MAX, false)
    }
}

/// One packet (B.9, B.10): its header, then its code blocks' bytes.
fn packet(d: &[u8], mut pos: usize, res: &mut Resolution, p: usize, layer: usize, g: Global, style: u8) -> Res<usize> {
    if g.sop && d.get(pos..pos + 2) == Some(&[0xFF, 0x91]) {
        pos += 6;
    }
    let mut bits = HeaderBits { d, p: pos, byte: 0, left: 0 };
    let present = bits.bit() == 1;
    if present {
        for band in res.bands.iter_mut() {
            let Some(prec) = band.precs.get_mut(p) else { continue };
            for j in 0..prec.bh {
                for i in 0..prec.bw {
                    let bi = (prec.by0 + j) * band.gw + prec.bx0 + i;
                    let Some(b) = band.blocks.get_mut(bi) else { continue };
                    let first = !b.included;
                    let inc = if first { prec.incl.below(i, j, layer as u32 + 1, &mut bits) } else { bits.bit() == 1 };
                    if !inc {
                        continue;
                    }
                    if first {
                        b.included = true;
                        b.zbp = prec.zbp.value(i, j, &mut bits);
                    }
                    // The number of new passes (Table B.4).
                    let n = if bits.bit() == 0 {
                        1
                    } else if bits.bit() == 0 {
                        2
                    } else {
                        let v = bits.bits(2);
                        if v < 3 {
                            3 + v
                        } else {
                            let v = bits.bits(5);
                            if v < 31 { 6 + v } else { 37 + bits.bits(7) }
                        }
                    };
                    while bits.bit() == 1 && b.lblock < 32 {
                        b.lblock += 1;
                    }
                    // Lengths, one per segment the new passes reach.
                    let mut left = n;
                    let mut k = b.passes;
                    while left > 0 {
                        let room = match b.segs.last() {
                            Some(s) if s.passes < s.max => s.max - s.passes,
                            _ => {
                                let (max, raw) = segment(k, style);
                                b.segs.push(Seg { data: Vec::new(), passes: 0, max, raw });
                                max
                            }
                        };
                        let take = left.min(room);
                        let len_bits = b.lblock + (31 - take.leading_zeros());
                        let len = bits.bits(len_bits) as usize;
                        let seg = b.segs.len() - 1;
                        b.segs[seg].passes += take;
                        b.pending.push((seg, len as u32));
                        left -= take;
                        k += take;
                    }
                    b.passes = k;
                }
            }
        }
    }
    pos = bits.end();
    if g.eph && d.get(pos..pos + 2) == Some(&[0xFF, 0x92]) {
        pos += 2;
    }
    // The bodies, in the same order.
    for band in res.bands.iter_mut() {
        let Some(prec) = band.precs.get(p) else { continue };
        for j in 0..prec.bh {
            for i in 0..prec.bw {
                let bi = (prec.by0 + j) * band.gw + prec.bx0 + i;
                let Some(b) = band.blocks.get_mut(bi) else { continue };
                for (seg, len) in std::mem::take(&mut b.pending) {
                    let end = (pos + len as usize).min(d.len());
                    if let Some(s) = b.segs.get_mut(seg) {
                        s.data.extend_from_slice(&d[pos.min(end)..end]);
                    }
                    pos = end;
                }
            }
        }
    }
    Ok(pos)
}

// ------------------------------------------------------------------ tier 1

/// The raw (bypass) bits: after a 0xFF byte, the next holds only 7.
struct RawBits<'a> {
    d: &'a [u8],
    p: usize,
    byte: u8,
    left: u8,
}

impl RawBits<'_> {
    fn bit(&mut self) -> u8 {
        if self.left == 0 {
            let ff = self.byte == 0xFF;
            self.byte = self.d.get(self.p).copied().unwrap_or(0xFF);
            self.p += 1;
            self.left = if ff { 7 } else { 8 };
        }
        self.left -= 1;
        (self.byte >> self.left) & 1
    }
}

enum Coder<'a> {
    Mq(Mq<'a>),
    Raw(RawBits<'a>),
}

const UNIFORM: usize = 18;
const RUN: usize = 17;

fn fresh_contexts() -> [u8; 19] {
    let mut cx = [0u8; 19];
    cx[0] = 4 << 1;
    cx[RUN] = 3 << 1;
    cx[UNIFORM] = 46 << 1;
    cx
}

/// Table D.1: the significance context from the neighbours' counts.
fn sig_context(h: u32, v: u32, d: u32, kind: usize) -> usize {
    let (h, v) = if kind == 1 { (v, h) } else { (h, v) };
    if kind == 3 {
        let hv = h + v;
        return match d {
            0 => [0, 1, 2][hv.min(2) as usize],
            1 => [3, 4, 5][hv.min(2) as usize],
            2 => if hv >= 1 { 7 } else { 6 },
            _ => 8,
        };
    }
    match h {
        0 => match v {
            0 => [0, 1, 2][d.min(2) as usize],
            1 => 3,
            _ => 4,
        },
        1 => {
            if v >= 1 {
                7
            } else if d >= 1 {
                6
            } else {
                5
            }
        }
        _ => 8,
    }
}

/// A code block's coefficients (dequantized), w x h, from its segments.
fn tier1(b: &Block, kind: usize, mb: i32, cb: u8, reversible: bool) -> Vec<f32> {
    let (w, h) = (b.x1 - b.x0, b.y1 - b.y0);
    let mut out = vec![0f32; w * h];
    let numbps = mb - b.zbp as i32;
    if w == 0 || h == 0 || !b.included || numbps <= 0 || numbps > 31 {
        return out;
    }
    // A border of one all round, so neighbours need no checks.
    let sw = w + 2;
    let at = |x: usize, y: usize| (y + 1) * sw + x + 1;
    let mut sig = vec![0u8; sw * (h + 2)];
    let mut neg = vec![0u8; sw * (h + 2)];
    let mut visited = vec![0u8; sw * (h + 2)];
    let mut refined = vec![0u8; sw * (h + 2)];
    let mut mag = vec![0u32; w * h];
    // The last plane each coefficient was coded in: its precision.
    let mut last = vec![0u8; w * h];
    let vsc = cb & VSC != 0;
    // Neighbour counts (h, v, d), with the stripe below ignored if causal.
    let counts = |sig: &[u8], x: usize, y: usize| -> (u32, u32, u32) {
        let i = at(x, y);
        let below = !(vsc && y % 4 == 3);
        let (up, down) = (i - sw, i + sw);
        let hh = (sig[i - 1] + sig[i + 1]) as u32;
        let vv = sig[up] as u32 + if below { sig[down] as u32 } else { 0 };
        let dd = (sig[up - 1] + sig[up + 1]) as u32 + if below { (sig[down - 1] + sig[down + 1]) as u32 } else { 0 };
        (hh, vv, dd)
    };
    // Table D.3: the sign's context and the bit it is XORed with.
    let sign_cx = |sig: &[u8], neg: &[u8], x: usize, y: usize| -> (usize, u8) {
        let i = at(x, y);
        let below = !(vsc && y % 4 == 3);
        let c = |j: usize| -> i32 { if sig[j] == 0 { 0 } else if neg[j] != 0 { -1 } else { 1 } };
        let hc = (c(i - 1) + c(i + 1)).clamp(-1, 1);
        let vc = (c(i - sw) + if below { c(i + sw) } else { 0 }).clamp(-1, 1);
        match (hc, vc) {
            (1, 1) => (13, 0),
            (1, 0) => (12, 0),
            (1, -1) => (11, 0),
            (0, 1) => (10, 0),
            (0, 0) => (9, 0),
            (0, -1) => (10, 1),
            (-1, 1) => (11, 1),
            (-1, 0) => (12, 1),
            _ => (13, 1),
        }
    };
    let mut cx = fresh_contexts();
    let mut k = 0u32;
    'segs: for seg in &b.segs {
        let mut coder = if seg.raw { Coder::Raw(RawBits { d: &seg.data, p: 0, byte: 0, left: 0 }) } else { Coder::Mq(Mq::new(&seg.data)) };
        for _ in 0..seg.passes {
            let (kind_of_pass, plane) = if k == 0 { (2, numbps - 1) } else { ((k - 1) % 3, numbps - 1 - (k as i32 + 2) / 3) };
            if plane < 0 {
                break 'segs;
            }
            let bit_of = |coder: &mut Coder, cx: &mut [u8; 19], i: usize| -> u8 {
                match coder {
                    Coder::Mq(m) => m.bit(cx, i),
                    Coder::Raw(r) => r.bit(),
                }
            };
            // A sign: its context and XOR bit when arithmetic, a plain bit when raw.
            let sign_of = |coder: &mut Coder, cx: &mut [u8; 19], sig: &[u8], neg: &[u8], x: usize, y: usize| -> u8 {
                match coder {
                    Coder::Mq(m) => {
                        let (scx, xr) = sign_cx(sig, neg, x, y);
                        m.bit(cx, scx) ^ xr
                    }
                    Coder::Raw(r) => r.bit(),
                }
            };
            let one = 1u32 << plane;
            for y0 in (0..h).step_by(4) {
                for x in 0..w {
                    let mut y = y0;
                    let y_end = (y0 + 4).min(h);
                    if kind_of_pass == 2 && y0 + 4 <= h {
                        // Cleanup: a run of four with nothing around them.
                        let quiet = (y0..y0 + 4).all(|yy| {
                            let i = at(x, yy);
                            let (a, bb, c) = counts(&sig, x, yy);
                            sig[i] == 0 && visited[i] == 0 && a + bb + c == 0
                        });
                        if quiet {
                            if bit_of(&mut coder, &mut cx, RUN) == 0 {
                                continue;
                            }
                            let r = ((bit_of(&mut coder, &mut cx, UNIFORM) << 1) | bit_of(&mut coder, &mut cx, UNIFORM)) as usize;
                            y = y0 + r;
                            let i = at(x, y);
                            neg[i] = sign_of(&mut coder, &mut cx, &sig, &neg, x, y);
                            sig[i] = 1;
                            mag[y * w + x] |= one;
                            last[y * w + x] = plane as u8;
                            y += 1;
                        }
                    }
                    while y < y_end {
                        let i = at(x, y);
                        match kind_of_pass {
                            0 => {
                                // Significance propagation.
                                if sig[i] == 0 {
                                    let (a, bb, c) = counts(&sig, x, y);
                                    if a + bb + c > 0 {
                                        visited[i] = 1;
                                        if bit_of(&mut coder, &mut cx, sig_context(a, bb, c, kind)) == 1 {
                                            neg[i] = sign_of(&mut coder, &mut cx, &sig, &neg, x, y);
                                            sig[i] = 1;
                                            mag[y * w + x] |= one;
                                            last[y * w + x] = plane as u8;
                                        }
                                    }
                                }
                            }
                            1 => {
                                // Magnitude refinement.
                                if sig[i] == 1 && visited[i] == 0 {
                                    let ctx = if refined[i] != 0 {
                                        16
                                    } else {
                                        let (a, bb, c) = counts(&sig, x, y);
                                        if a + bb + c > 0 { 15 } else { 14 }
                                    };
                                    if bit_of(&mut coder, &mut cx, ctx) == 1 {
                                        mag[y * w + x] |= one;
                                    }
                                    last[y * w + x] = plane as u8;
                                    refined[i] = 1;
                                }
                            }
                            _ => {
                                // Cleanup.
                                if sig[i] == 0 && visited[i] == 0 {
                                    let (a, bb, c) = counts(&sig, x, y);
                                    if bit_of(&mut coder, &mut cx, sig_context(a, bb, c, kind)) == 1 {
                                        neg[i] = sign_of(&mut coder, &mut cx, &sig, &neg, x, y);
                                        sig[i] = 1;
                                        mag[y * w + x] |= one;
                                        last[y * w + x] = plane as u8;
                                    }
                                }
                            }
                        }
                        y += 1;
                    }
                }
            }
            if kind_of_pass == 2 {
                visited.iter_mut().for_each(|v| *v = 0);
                if cb & SEGSYM != 0 {
                    for _ in 0..4 {
                        bit_of(&mut coder, &mut cx, UNIFORM);
                    }
                }
            }
            if cb & RESET != 0 {
                cx = fresh_contexts();
            }
            k += 1;
        }
    }
    // Halfway into the last plane each was coded in (reconstruction,
    // E.1.1.2); none for a reversible one coded to plane 0 (it is exact).
    for y in 0..h {
        for x in 0..w {
            let m = mag[y * w + x];
            if m != 0 {
                let lp = last[y * w + x];
                let half = if reversible && lp == 0 { 0.0 } else { (1u32 << lp) as f32 / 2.0 };
                let v = m as f32 + half;
                out[y * w + x] = if neg[at(x, y)] != 0 { -v } else { v };
            }
        }
    }
    out
}

// ------------------------------------------------- the inverse transforms

/// One component of a tile: its code blocks decoded and dequantized, then
/// the wavelet undone resolution by resolution.
fn decode_tile_comp(tc: &TileComp) -> Vec<f32> {
    let reversible = tc.style.reversible;
    let depth_bits = |kind: usize| -> i32 { [0, 1, 1, 2][kind] };
    let mut bands: Vec<Vec<Vec<f32>>> = Vec::new();
    for res in &tc.res {
        let mut rb = Vec::new();
        for band in &res.bands {
            let (w, h) = (band.x1 - band.x0, band.y1 - band.y0);
            let mut v = vec![0f32; w * h];
            let mb = tc.guard as i32 + band.eps - 1;
            // The quantization step (E-3): 1 when reversible.
            let delta = if reversible {
                1.0
            } else {
                let rb = tc.depth as i32 + depth_bits(band.kind);
                2f32.powi(rb - band.eps) * (1.0 + band.mu as f32 / 2048.0)
            };
            for b in &band.blocks {
                let c = tier1(b, band.kind, mb, tc.style.cb, reversible);
                let bw = b.x1 - b.x0;
                for y in b.y0..b.y1 {
                    for x in b.x0..b.x1 {
                        let s = c[(y - b.y0) * bw + (x - b.x0)];
                        v[(y - band.y0) * w + (x - band.x0)] = if reversible { s.trunc() } else { s * delta };
                    }
                }
            }
            rb.push(v);
        }
        bands.push(rb);
    }
    // The lowest resolution's LL, then each level up.
    let mut cur = bands[0][0].clone();
    for (r, (res, prev)) in tc.res.iter().skip(1).zip(&tc.res).enumerate().map(|(i, x)| (i + 1, x)) {
        let (w, h) = (res.x1 - res.x0, res.y1 - res.y0);
        let mut a = vec![0f32; w * h];
        let mut put = |bx: usize, by: usize, ox: usize, oy: usize, bw: usize, bh: usize, src: &[f32]| {
            for v in 0..bh {
                for u in 0..bw {
                    let (x, y) = (2 * (bx + u) + ox, 2 * (by + v) + oy);
                    if x >= res.x0 && y >= res.y0 && x < res.x1 && y < res.y1 {
                        a[(y - res.y0) * w + (x - res.x0)] = src[v * bw + u];
                    }
                }
            }
        };
        put(prev.x0, prev.y0, 0, 0, prev.x1 - prev.x0, prev.y1 - prev.y0, &cur);
        for (band, data) in res.bands.iter().zip(&bands[r]) {
            let (ox, oy) = match band.kind {
                1 => (1, 0),
                2 => (0, 1),
                _ => (1, 1),
            };
            put(band.x0, band.y0, ox, oy, band.x1 - band.x0, band.y1 - band.y0, data);
        }
        // Rows, then columns (F.3.2).
        let mut line = Vec::new();
        for y in 0..h {
            line.clear();
            line.extend_from_slice(&a[y * w..(y + 1) * w]);
            idwt_1d(&mut line, res.x0, reversible);
            a[y * w..(y + 1) * w].copy_from_slice(&line);
        }
        for x in 0..w {
            line.clear();
            line.extend((0..h).map(|y| a[y * w + x]));
            idwt_1d(&mut line, res.y0, reversible);
            for (y, v) in line.iter().enumerate() {
                a[y * w + x] = *v;
            }
        }
        cur = a;
    }
    cur
}

/// The 1D inverse transform (F.3.6) of a signal starting at index `i0`:
/// even indices are low-pass. Exact integers for 5/3.
fn idwt_1d(x: &mut [f32], i0: usize, reversible: bool) {
    let n = x.len();
    if n == 0 {
        return;
    }
    if n == 1 {
        if i0 % 2 == 1 {
            x[0] = if reversible { (x[0] as i32 / 2) as f32 } else { x[0] / 2.0 };
        }
        return;
    }
    const M: usize = 4;
    let period = 2 * (n - 1);
    // Symmetric extension (PSE), M samples each side.
    let ext = |k: isize| -> f32 {
        let m = k.rem_euclid(period as isize) as usize;
        x[m.min(period - m)]
    };
    let len = n + 2 * M;
    let even = |k: usize| (i0 + k).is_multiple_of(2) == M.is_multiple_of(2);
    if reversible {
        let mut y: Vec<i32> = (0..len).map(|k| ext(k as isize - M as isize) as i32).collect();
        for k in 1..len - 1 {
            if even(k) {
                y[k] -= (y[k - 1] + y[k + 1] + 2) >> 2;
            }
        }
        for k in 2..len - 2 {
            if !even(k) {
                y[k] += (y[k - 1] + y[k + 1]) >> 1;
            }
        }
        for (i, v) in x.iter_mut().enumerate() {
            *v = y[i + M] as f32;
        }
    } else {
        const ALPHA: f32 = -1.586_134_3;
        const BETA: f32 = -0.052_980_118;
        const GAMMA: f32 = 0.882_911_1;
        const DELTA: f32 = 0.443_506_87;
        const K: f32 = 1.230_174_1;
        let mut y: Vec<f32> = (0..len).map(|k| ext(k as isize - M as isize)).collect();
        for (k, v) in y.iter_mut().enumerate() {
            *v *= if even(k) { K } else { 1.0 / K };
        }
        let lift = |y: &mut [f32], from: usize, low: bool, c: f32| {
            for k in from..len - from {
                if even(k) == low {
                    y[k] -= c * (y[k - 1] + y[k + 1]);
                }
            }
        };
        lift(&mut y, 1, true, DELTA);
        lift(&mut y, 2, false, GAMMA);
        lift(&mut y, 3, true, BETA);
        lift(&mut y, 4, false, ALPHA);
        x.copy_from_slice(&y[M..M + n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNM file's size, maximum and samples (P5 gray or P6 RGB, 8 or 16 bits).
    fn pnm(d: &[u8]) -> (usize, usize, usize, Vec<u16>) {
        let mut fields = Vec::new();
        let mut i = 0;
        while fields.len() < 4 {
            while d[i].is_ascii_whitespace() {
                i += 1;
            }
            if d[i] == b'#' {
                while d[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            let s = i;
            while !d[i].is_ascii_whitespace() {
                i += 1;
            }
            fields.push(String::from_utf8(d[s..i].to_vec()).unwrap());
        }
        let body = &d[i + 1..];
        let (w, h, max): (usize, usize, usize) = (fields[1].parse().unwrap(), fields[2].parse().unwrap(), fields[3].parse().unwrap());
        let ch = if fields[0] == "P6" { 3 } else { 1 };
        let samples: Vec<u16> = if max > 255 { body.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect() } else { body.iter().map(|b| *b as u16).collect() };
        assert_eq!(samples.len(), w * h * ch);
        (w, h, ch, samples)
    }

    /// The largest difference between a decoded picture and a PNM.
    fn worst(p: &Picture, expected: &[u8]) -> u16 {
        let (w, h, ch, s) = pnm(expected);
        assert_eq!((p.width, p.height), (w, h), "size");
        assert!(p.channels.len() >= ch, "channels");
        let mut worst = 0;
        for i in 0..w * h {
            for c in 0..ch {
                worst = worst.max(p.channels[c].data[i].abs_diff(s[i * ch + c]));
            }
        }
        worst
    }

    macro_rules! file {
        ($name:literal) => {
            include_bytes!(concat!("../testdata/jpx/", $name)).as_slice()
        };
    }

    #[test]
    fn lossless_pictures_come_back_exactly() {
        let rgb = file!("rgb.ppm");
        for (name, data) in [
            ("plain", file!("rgb-plain.j2k")),
            ("tiles, precincts, RPCL, SOP, EPH, layers", file!("rgb-tiles.j2k")),
            ("every code-block style", file!("rgb-modes.j2k")),
            ("PCRL", file!("rgb-pcrl.j2k")),
            ("CPRL, offsets", file!("rgb-cprl.j2k")),
            ("RLCP", file!("rgb-rlcp.j2k")),
            ("no wavelet levels", file!("rgb-nl0.j2k")),
            ("in a JP2 file", file!("rgb.jp2")),
        ] {
            let p = decode(data).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(worst(&p, rgb), 0, "{name}");
        }
        let p = decode(file!("gray-bypass.j2k")).unwrap();
        assert_eq!(worst(&p, file!("gray.pgm")), 0, "gray, bypass");
        let p = decode(file!("g12.j2k")).unwrap();
        assert_eq!(p.channels[0].depth, 12);
        assert_eq!(worst(&p, file!("g12.pgm")), 0, "12 bits");
        let p = decode(file!("rgb.jp2")).unwrap();
        assert_eq!(p.color, Some(Jp2Color::Rgb));
    }

    #[test]
    fn lossy_pictures_match_openjpeg() {
        for (name, data, out) in [
            ("9/7", file!("rgb-lossy.j2k"), file!("rgb-lossy.out.ppm")),
            ("9/7, layers, styles", file!("rgb-lossy-layers.jp2"), file!("rgb-lossy-layers.out.ppm")),
            ("9/7, 12 bits", file!("g12-lossy.j2k"), file!("g12-lossy.out.pgm")),
        ] {
            let p = decode(data).unwrap_or_else(|e| panic!("{name}: {e}"));
            let w = worst(&p, out);
            assert!(w <= 1, "{name}: off by {w}");
        }
    }

    #[test]
    fn a_subsampled_component() {
        let p = decode(file!("gray-sub.j2k")).unwrap();
        let (w, h, _, s) = pnm(file!("gray-sub.out.pgm"));
        // OpenJPEG writes the component at its own size; ours is at the picture's.
        for y in 0..h {
            for x in 0..w {
                let ours = p.channels[0].data[(y * 2).min(p.height - 1) * p.width + (x * 2).min(p.width - 1)];
                assert_eq!(ours, s[y * w + x], "at {x}, {y}");
            }
        }
    }

    #[test]
    fn damage_never_panics() {
        let iters: usize = std::env::var("FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
        let files = [file!("rgb-tiles.j2k"), file!("rgb-modes.j2k"), file!("rgb-lossy-layers.jp2"), file!("g12.j2k")];
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for i in 0..iters {
            let mut d = files[i % files.len()].to_vec();
            for _ in 0..1 + next() % 8 {
                let at = next() as usize % d.len();
                d[at] = next() as u8;
            }
            if next() % 4 == 0 {
                let cut = next() as usize % d.len();
                d.truncate(cut);
            }
            let _ = decode(&d);
        }
    }
}

