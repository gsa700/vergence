//! TrueType fonts as PDFs embed them (FontFile2): our own reader of the
//! tables a page needs: head, maxp, loca and glyf (outlines, simple and
//! composite), cmap (formats 0, 4, 6 and 12) and hmtx. Embedded fonts are
//! often subsets with tables missing or damaged; whatever can be read is.
//!
//! Untrusted: every offset is checked, composite glyphs nest at most
//! [`MAX_COMPOSITE_DEPTH`] deep and have at most [`MAX_POINTS`] points.

use vergence_raster::Path;

const MAX_COMPOSITE_DEPTH: usize = 8;
const MAX_POINTS: usize = 65_536;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn i16_at(b: &[u8], at: usize) -> Option<i16> {
    u16_at(b, at).map(|v| v as i16)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

pub struct TrueType {
    data: Vec<u8>,
    pub units_per_em: f64,
    long_loca: bool,
    num_glyphs: usize,
    loca: Option<(usize, usize)>,
    glyf: Option<(usize, usize)>,
    hmtx: Option<(usize, usize)>,
    long_metrics: usize,
    /// cmap subtables: (platform, encoding, offset into the file).
    cmaps: Vec<(u16, u16, usize)>,
    /// Whether it is a CFF-flavored OpenType font ('OTTO').
    pub cff: Option<(usize, usize)>,
}

impl TrueType {
    pub fn parse(data: Vec<u8>) -> Option<TrueType> {
        // A collection: its first font.
        let base = if data.get(..4)? == b"ttcf" { u32_at(&data, 12)? as usize } else { 0 };
        let n = u16_at(&data, base + 4)? as usize;
        let table = |tag: &[u8; 4]| -> Option<(usize, usize)> {
            (0..n.min(256)).find_map(|i| {
                let r = base + 12 + 16 * i;
                (data.get(r..r + 4)? == tag).then(|| Some((u32_at(&data, r + 8)? as usize, u32_at(&data, r + 12)? as usize))).flatten()
            })
            .filter(|(o, l)| o.checked_add(*l).is_some_and(|e| e <= data.len()))
        };
        let head = table(b"head");
        let units_per_em = head.and_then(|(o, _)| u16_at(&data, o + 18)).filter(|u| (16..=16384).contains(u)).unwrap_or(1000) as f64;
        let long_loca = head.and_then(|(o, _)| i16_at(&data, o + 50)).unwrap_or(0) == 1;
        let num_glyphs = table(b"maxp").and_then(|(o, _)| u16_at(&data, o + 4)).unwrap_or(0) as usize;
        let long_metrics = table(b"hhea").and_then(|(o, _)| u16_at(&data, o + 34)).unwrap_or(0) as usize;
        let (loca, glyf, hmtx, cff) = (table(b"loca"), table(b"glyf"), table(b"hmtx"), table(b"CFF "));
        let mut cmaps = Vec::new();
        if let Some((o, _)) = table(b"cmap") {
            let count = u16_at(&data, o + 2).unwrap_or(0) as usize;
            for i in 0..count.min(64) {
                let r = o + 4 + 8 * i;
                if let (Some(p), Some(e), Some(off)) = (u16_at(&data, r), u16_at(&data, r + 2), u32_at(&data, r + 4)) {
                    cmaps.push((p, e, o + off as usize));
                }
            }
        }
        if glyf.is_none() && cff.is_none() {
            return None;
        }
        Some(TrueType { data, units_per_em, long_loca, num_glyphs, loca, glyf, hmtx, long_metrics, cmaps, cff })
    }

    pub fn num_glyphs(&self) -> usize {
        self.num_glyphs
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Whether it has a cmap subtable for platform `p`, encoding `e`.
    pub fn has_cmap(&self, p: u16, e: u16) -> bool {
        self.cmaps.iter().any(|c| c.0 == p && c.1 == e)
    }

    /// A Unicode character through whichever Unicode cmap the font has:
    /// Windows' full range (3, 10) or BMP (3, 1), then any of the Unicode
    /// platform's encodings (0, *), wider ranges first (step 9: a subset
    /// with only (0, 1), Unicode 1.1, drew one letter of a title).
    pub fn lookup_unicode(&self, u: u32) -> Option<u16> {
        [(3, 10), (3, 1), (0, 6), (0, 4), (0, 3), (0, 2), (0, 1), (0, 0)].iter().find_map(|&(p, e)| self.lookup(p, e, u))
    }

    /// `code` through the cmap subtable for platform `p`, encoding `e`.
    pub fn lookup(&self, p: u16, e: u16, code: u32) -> Option<u16> {
        let &(_, _, at) = self.cmaps.iter().find(|c| c.0 == p && c.1 == e)?;
        let d = &self.data;
        let g = match u16_at(d, at)? {
            0 => (code < 256).then(|| d.get(at + 6 + code as usize).map(|g| *g as u16)).flatten()?,
            4 => {
                if code > 0xFFFF {
                    return None;
                }
                let segs = (u16_at(d, at + 6)? / 2) as usize;
                let (ends, starts, deltas, ranges) = (at + 14, at + 16 + 2 * segs, at + 16 + 4 * segs, at + 16 + 6 * segs);
                let i = (0..segs).find(|i| u16_at(d, ends + 2 * i).is_some_and(|e| e as u32 >= code))?;
                let start = u16_at(d, starts + 2 * i)? as u32;
                if code < start {
                    return None;
                }
                let delta = u16_at(d, deltas + 2 * i)?;
                let range = u16_at(d, ranges + 2 * i)? as usize;
                if range == 0 {
                    (code as u16).wrapping_add(delta)
                } else {
                    let g = u16_at(d, ranges + 2 * i + range + 2 * (code - start) as usize)?;
                    if g == 0 { 0 } else { g.wrapping_add(delta) }
                }
            }
            6 => {
                let (first, count) = (u16_at(d, at + 6)? as u32, u16_at(d, at + 8)? as u32);
                if code < first || code >= first + count {
                    return None;
                }
                u16_at(d, at + 10 + 2 * (code - first) as usize)?
            }
            12 => {
                let groups = u32_at(d, at + 12)? as usize;
                (0..groups.min(1 << 16)).find_map(|i| {
                    let r = at + 16 + 12 * i;
                    let (s, e, g) = (u32_at(d, r)?, u32_at(d, r + 4)?, u32_at(d, r + 8)?);
                    (code >= s && code <= e).then(|| (g + (code - s)) as u16)
                })?
            }
            _ => return None,
        };
        (g != 0).then_some(g)
    }

    /// The glyph's advance in font units (hmtx).
    pub fn advance(&self, gid: u16) -> Option<f64> {
        let (o, _) = self.hmtx?;
        let i = (gid as usize).min(self.long_metrics.saturating_sub(1));
        u16_at(&self.data, o + 4 * i).map(|a| a as f64)
    }

    fn glyph_range(&self, gid: u16) -> Option<(usize, usize)> {
        let (lo, _) = self.loca?;
        let (go, gl) = self.glyf?;
        let i = gid as usize;
        if self.num_glyphs > 0 && i >= self.num_glyphs {
            return None;
        }
        let (a, b) = if self.long_loca { (u32_at(&self.data, lo + 4 * i)? as usize, u32_at(&self.data, lo + 4 * i + 4)? as usize) } else { (u16_at(&self.data, lo + 2 * i)? as usize * 2, u16_at(&self.data, lo + 2 * i + 2)? as usize * 2) };
        (b > a && b <= gl).then_some((go + a, go + b))
    }

    /// The glyph's outline in font units, y up; None if it has none.
    pub fn outline(&self, gid: u16) -> Option<Path> {
        let mut path = Path::new();
        let mut points = 0usize;
        self.glyph_into(gid, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0], &mut path, 0, &mut points)?;
        (!path.is_empty()).then_some(path)
    }

    fn glyph_into(&self, gid: u16, m: &[f64; 6], path: &mut Path, depth: usize, points: &mut usize) -> Option<()> {
        if depth > MAX_COMPOSITE_DEPTH {
            return None;
        }
        let Some((start, end)) = self.glyph_range(gid) else { return Some(()) };
        let g = &self.data[start..end];
        let contours = i16_at(g, 0)?;
        if contours >= 0 {
            return simple(g, contours as usize, m, path, points);
        }
        // Composite: components with offsets and transforms.
        let mut at = 10;
        loop {
            let flags = u16_at(g, at)?;
            let glyph = u16_at(g, at + 2)?;
            at += 4;
            let (dx, dy) = if flags & 1 != 0 {
                let v = (i16_at(g, at)? as f64, i16_at(g, at + 2)? as f64);
                at += 4;
                v
            } else {
                let v = (*g.get(at)? as i8 as f64, *g.get(at + 1)? as i8 as f64);
                at += 2;
                v
            };
            let f2 = |at: usize| i16_at(g, at).map(|v| v as f64 / 16384.0);
            let (a, b, c, d) = if flags & 0x08 != 0 {
                let s = f2(at)?;
                at += 2;
                (s, 0.0, 0.0, s)
            } else if flags & 0x40 != 0 {
                let v = (f2(at)?, 0.0, 0.0, f2(at + 2)?);
                at += 4;
                v
            } else if flags & 0x80 != 0 {
                let v = (f2(at)?, f2(at + 2)?, f2(at + 4)?, f2(at + 6)?);
                at += 8;
                v
            } else {
                (1.0, 0.0, 0.0, 1.0)
            };
            // Offsets as x, y values (ARGS_ARE_XY_VALUES); point matching is rare and left out.
            let (dx, dy) = if flags & 2 != 0 { (dx, dy) } else { (0.0, 0.0) };
            let local = [a, b, c, d, dx, dy];
            let combined = [
                local[0] * m[0] + local[1] * m[2],
                local[0] * m[1] + local[1] * m[3],
                local[2] * m[0] + local[3] * m[2],
                local[2] * m[1] + local[3] * m[3],
                local[4] * m[0] + local[5] * m[2] + m[4],
                local[4] * m[1] + local[5] * m[3] + m[5],
            ];
            self.glyph_into(glyph, &combined, path, depth + 1, points)?;
            if flags & 0x20 == 0 {
                return Some(());
            }
        }
    }
}

/// A simple glyph's contours of on- and off-curve points into the path,
/// quadratic curves as cubics, through `m`.
fn simple(g: &[u8], contours: usize, m: &[f64; 6], path: &mut Path, total: &mut usize) -> Option<()> {
    let ends: Vec<usize> = (0..contours).map(|i| u16_at(g, 10 + 2 * i).map(|v| v as usize)).collect::<Option<_>>()?;
    let n = ends.last().map_or(0, |e| e + 1);
    *total += n;
    if n == 0 || *total > MAX_POINTS {
        return Some(());
    }
    let ilen = u16_at(g, 10 + 2 * contours)? as usize;
    let mut at = 12 + 2 * contours + ilen;
    let mut flags = Vec::with_capacity(n);
    while flags.len() < n {
        let f = *g.get(at)?;
        at += 1;
        flags.push(f);
        if f & 8 != 0 {
            let r = *g.get(at)?;
            at += 1;
            for _ in 0..r {
                if flags.len() < n {
                    flags.push(f);
                }
            }
        }
    }
    let coord = |short: u8, same: u8, at: &mut usize| -> Option<Vec<f64>> {
        let mut v = 0i32;
        let mut out = Vec::with_capacity(n);
        for f in &flags {
            if f & short != 0 {
                let d = *g.get(*at)? as i32;
                *at += 1;
                v += if f & same != 0 { d } else { -d };
            } else if f & same == 0 {
                v += i16_at(g, *at)? as i32;
                *at += 2;
            }
            out.push(v as f64);
        }
        Some(out)
    };
    let xs = coord(2, 16, &mut at)?;
    let ys = coord(4, 32, &mut at)?;
    let tx = |x: f64, y: f64| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
    let mut first = 0;
    for &end in &ends {
        if end < first || end >= n {
            return None;
        }
        let pts: Vec<(f64, f64, bool)> = (first..=end).map(|i| (xs[i], ys[i], flags[i] & 1 != 0)).collect();
        first = end + 1;
        if pts.len() < 2 {
            continue;
        }
        // Start on an on-curve point (or the midpoint of two off-curve ones).
        let k = pts.iter().position(|p| p.2);
        let start = match k {
            Some(i) => (pts[i].0, pts[i].1),
            None => ((pts[0].0 + pts[1].0) / 2.0, (pts[0].1 + pts[1].1) / 2.0),
        };
        let offset = k.unwrap_or(0);
        let (sx, sy) = tx(start.0, start.1);
        path.move_to(sx, sy);
        let mut cur = start;
        let mut ctrl: Option<(f64, f64)> = None;
        let len = pts.len();
        let quad = |path: &mut Path, p0: (f64, f64), c: (f64, f64), p1: (f64, f64)| {
            let c1 = (p0.0 + 2.0 / 3.0 * (c.0 - p0.0), p0.1 + 2.0 / 3.0 * (c.1 - p0.1));
            let c2 = (p1.0 + 2.0 / 3.0 * (c.0 - p1.0), p1.1 + 2.0 / 3.0 * (c.1 - p1.1));
            let (a, b, e) = (tx(c1.0, c1.1), tx(c2.0, c2.1), tx(p1.0, p1.1));
            path.cubic_to(a.0, a.1, b.0, b.1, e.0, e.1);
        };
        for step in 1..=len {
            let p = pts[(offset + step) % len];
            let here = (p.0, p.1);
            match (p.2, ctrl) {
                (true, None) => {
                    let (x, y) = tx(here.0, here.1);
                    path.line_to(x, y);
                    cur = here;
                }
                (true, Some(c)) => {
                    quad(path, cur, c, here);
                    cur = here;
                    ctrl = None;
                }
                (false, None) => ctrl = Some(here),
                (false, Some(c)) => {
                    let mid = ((c.0 + here.0) / 2.0, (c.1 + here.1) / 2.0);
                    quad(path, cur, c, mid);
                    cur = mid;
                    ctrl = Some(here);
                }
            }
        }
        if let Some(c) = ctrl {
            quad(path, cur, c, start);
        }
        path.close();
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font of nothing but a glyf table and one cmap subtable (platform
    /// `p`, encoding `e`, format 4) mapping 'A'..'Z' to glyphs 3..28.
    fn font_with_cmap(p: u16, e: u16) -> TrueType {
        let mut sub = Vec::new();
        let segs: [(u16, u16, u16); 2] = [(0x41, 0x5A, 3u16.wrapping_sub(0x41)), (0xFFFF, 0xFFFF, 1)];
        sub.extend_from_slice(&4u16.to_be_bytes());
        sub.extend_from_slice(&((16 + 8 * segs.len()) as u16).to_be_bytes());
        sub.extend_from_slice(&[0, 0]);
        sub.extend_from_slice(&((2 * segs.len()) as u16).to_be_bytes());
        sub.extend_from_slice(&[0; 6]);
        segs.iter().for_each(|s| sub.extend_from_slice(&s.1.to_be_bytes()));
        sub.extend_from_slice(&[0, 0]);
        segs.iter().for_each(|s| sub.extend_from_slice(&s.0.to_be_bytes()));
        segs.iter().for_each(|s| sub.extend_from_slice(&s.2.to_be_bytes()));
        segs.iter().for_each(|_| sub.extend_from_slice(&[0, 0]));
        let mut cmap = vec![0, 0, 0, 1];
        cmap.extend_from_slice(&p.to_be_bytes());
        cmap.extend_from_slice(&e.to_be_bytes());
        cmap.extend_from_slice(&12u32.to_be_bytes());
        cmap.extend_from_slice(&sub);
        let tables: [(&[u8; 4], Vec<u8>); 2] = [(b"cmap", cmap), (b"glyf", vec![0; 4])];
        let mut font = vec![0, 1, 0, 0, 0, tables.len() as u8, 0, 0, 0, 0, 0, 0];
        let mut at = 12 + 16 * tables.len();
        let mut body = Vec::new();
        for (tag, data) in &tables {
            font.extend_from_slice(*tag);
            font.extend_from_slice(&[0; 4]);
            font.extend_from_slice(&(at as u32).to_be_bytes());
            font.extend_from_slice(&(data.len() as u32).to_be_bytes());
            body.extend_from_slice(data);
            at += data.len();
        }
        font.extend_from_slice(&body);
        TrueType::parse(font).unwrap()
    }

    #[test]
    fn every_unicode_cmap_is_tried() {
        for (p, e) in [(3, 1), (3, 10), (0, 0), (0, 1), (0, 3), (0, 4)] {
            let tt = font_with_cmap(p, e);
            assert_eq!(tt.lookup_unicode('H' as u32), Some(3 + 7), "cmap ({p}, {e})");
            assert_eq!(tt.lookup_unicode('a' as u32), None, "cmap ({p}, {e}): not mapped");
        }
        // A symbol-encoded (3, 0) table is not Unicode.
        assert_eq!(font_with_cmap(3, 0).lookup_unicode('H' as u32), None);
    }
}
