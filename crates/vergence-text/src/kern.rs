//! Kerning from a font's `GPOS` table, read by us (text milestone 1, step 1).
//!
//! Modern fonts keep their kerning in `GPOS` only (Inter has no legacy
//! `kern` table), and ab_glyph reads only `kern`, so until 2026-11-19 Inter
//! was set at its bare advances. This reads the `kern` feature's pair
//! adjustment lookups (formats 1 and 2, through extension lookups too) for
//! the default script and Latin, and a variable font's adjustments at the
//! face's own axis settings (GDEF's item variation store), so bold is kerned
//! as bold.
//!
//! What is used of a pair's adjustment: the first glyph's x-advance (and its
//! variation delta). That is how kerning is written in practice; placements
//! and the second glyph's value are left out, as are the size-specific
//! hinting deltas of old device tables.
//!
//! **A font file is untrusted input**, as in sfnt.rs: every offset and count
//! is checked against the bytes there, counts are bounded, lookups only ever
//! move forward through bounded loops, and nothing here can panic.

/// The most lookups and subtables taken (Inter has a few of each).
const MAX_LOOKUPS: usize = 256;
const MAX_SUBTABLES: usize = 1024;
const MAX_REGIONS: usize = 4096;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn i16_at(b: &[u8], at: usize) -> Option<i16> {
    u16_at(b, at).map(|v| v as i16)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// An F2DOT14 value.
fn f2dot14(b: &[u8], at: usize) -> Option<f32> {
    i16_at(b, at).map(|v| v as f32 / 16384.0)
}

/// A table of face `index` of a font file (or collection), by tag.
pub fn table<'a>(file: &'a [u8], index: u32, tag: &[u8; 4]) -> Option<&'a [u8]> {
    let at = if file.get(..4)? == b"ttcf" {
        if index >= u32_at(file, 8)? {
            return None;
        }
        u32_at(file, 12 + 4 * index as usize)? as usize
    } else if index == 0 {
        0
    } else {
        return None;
    };
    let n = (u16_at(file, at + 4)? as usize).min(512);
    for i in 0..n {
        let r = at + 12 + 16 * i;
        if file.get(r..r + 4)? == tag {
            let (off, len) = (u32_at(file, r + 8)? as usize, u32_at(file, r + 12)? as usize);
            if len > crate::sfnt::MAX_TABLE {
                return None;
            }
            return file.get(off..off.checked_add(len)?);
        }
    }
    None
}

/// A variable font's axis by tag: its least, default and most values.
pub fn axis(file: &[u8], index: u32, tag: &[u8; 4]) -> Option<(f32, f32, f32)> {
    let fvar = table(file, index, b"fvar")?;
    let (first, count, size) =
        (u16_at(fvar, 4)? as usize, (u16_at(fvar, 8)? as usize).min(64), (u16_at(fvar, 10)? as usize).max(20));
    let fixed = |at: usize| u32_at(fvar, at).map(|v| v as i32 as f32 / 65536.0);
    (0..count).find_map(|i| {
        let a = first + i * size;
        if fvar.get(a..a + 4)? != tag {
            return None;
        }
        let (min, def, max) = (fixed(a + 4)?, fixed(a + 8)?, fixed(a + 12)?);
        (min.is_finite() && max.is_finite() && min <= def && def <= max).then_some((min, def, max))
    })
}

/// A variable font's normalized coordinates (-1 to 1 per axis, after
/// `avar`), for the axis settings given by tag in the font's own units
/// (`wght` 400); axes not given are at their defaults.
pub fn coordinates(file: &[u8], index: u32, settings: &[([u8; 4], f32)]) -> Vec<f32> {
    let Some(fvar) = table(file, index, b"fvar") else { return Vec::new() };
    let (Some(first), Some(count), Some(size)) = (u16_at(fvar, 4), u16_at(fvar, 8), u16_at(fvar, 10)) else {
        return Vec::new();
    };
    let (first, count, size) = (first as usize, (count as usize).min(64), (size as usize).max(20));
    let fixed = |at: usize| u32_at(fvar, at).map(|v| v as i32 as f32 / 65536.0);
    let mut coords = Vec::with_capacity(count);
    for i in 0..count {
        let a = first + i * size;
        let (Some(tag), Some(min), Some(def), Some(max)) =
            (fvar.get(a..a + 4), fixed(a + 4), fixed(a + 8), fixed(a + 12))
        else {
            break;
        };
        let v = settings.iter().find(|(t, _)| t == tag).map_or(def, |(_, v)| *v);
        let v = if v.is_finite() { v.clamp(min.min(def), max.max(def)) } else { def };
        let n = if v < def && def > min {
            (v - def) / (def - min)
        } else if v > def && max > def {
            (v - def) / (max - def)
        } else {
            0.0
        };
        coords.push(n);
    }
    // avar: each axis's segment map, piecewise linear.
    if let Some(avar) = table(file, index, b"avar") {
        let mut at = 8;
        let axes = u16_at(avar, 6).map_or(0, |n| n as usize).min(coords.len());
        for c in coords.iter_mut().take(axes) {
            let Some(maps) = u16_at(avar, at) else { break };
            let maps = maps as usize;
            let pairs: Vec<(f32, f32)> = (0..maps)
                .filter_map(|k| Some((f2dot14(avar, at + 2 + 4 * k)?, f2dot14(avar, at + 4 + 4 * k)?)))
                .collect();
            at += 2 + 4 * maps;
            if pairs.len() != maps || maps < 2 {
                continue;
            }
            *c = segment(&pairs, *c);
        }
    }
    coords.iter().map(|c| (c.clamp(-1.0, 1.0) * 16384.0).round() / 16384.0).collect()
}

fn segment(pairs: &[(f32, f32)], c: f32) -> f32 {
    if c <= pairs[0].0 {
        return pairs[0].1;
    }
    for w in pairs.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if c <= x1 {
            return if x1 > x0 { y0 + (c - x0) / (x1 - x0) * (y1 - y0) } else { y1 };
        }
    }
    pairs[pairs.len() - 1].1
}

/// A face's kerning: the pair adjustment subtables of its `kern` feature,
/// by lookup, and its variation store at the face's coordinates.
pub struct Kerning {
    gpos: Vec<u8>,
    /// Per lookup, in lookup order, its pair subtables' offsets in `gpos`.
    lookups: Vec<Vec<usize>>,
    deltas: Option<Deltas>,
}

struct Deltas {
    gdef: Vec<u8>,
    /// The item variation store's offset in `gdef`.
    store: usize,
    /// Each region's scalar at the face's coordinates.
    scalars: Vec<f32>,
}

impl Kerning {
    /// The kerning of face `index` at these normalized coordinates
    /// ([`coordinates`]). None if the font has no `GPOS` kerning (the caller
    /// falls back to a legacy `kern` table).
    pub fn read(file: &[u8], index: u32, coords: &[f32]) -> Option<Kerning> {
        let gpos = table(file, index, b"GPOS")?;
        let lookups = pair_lookups(gpos)?;
        if lookups.is_empty() {
            return None;
        }
        let deltas =
            if coords.iter().any(|c| *c != 0.0) { Deltas::read(table(file, index, b"GDEF")?, coords) } else { None };
        Some(Kerning { gpos: gpos.to_vec(), lookups, deltas })
    }

    /// The adjustment between glyphs `a` and `b`, in font units (negative:
    /// closer).
    pub fn pair(&self, a: u16, b: u16) -> f32 {
        let mut total = 0.0;
        for lookup in &self.lookups {
            // Within a lookup the first subtable that applies is the one.
            if let Some(v) = lookup.iter().find_map(|st| self.subtable(*st, a, b)) {
                total += v;
            }
        }
        total
    }

    fn subtable(&self, st: usize, a: u16, b: u16) -> Option<f32> {
        let g = &self.gpos;
        let format = u16_at(g, st)?;
        let coverage = st + u16_at(g, st + 2)? as usize;
        let (vf1, vf2) = (u16_at(g, st + 4)?, u16_at(g, st + 6)?);
        let (size1, size2) = (value_size(vf1), value_size(vf2));
        let index = covered(g, coverage, a)?;
        match format {
            1 => {
                if index >= u16_at(g, st + 8)? as usize {
                    return None;
                }
                let set = st + u16_at(g, st + 10 + 2 * index)? as usize;
                let n = u16_at(g, set)? as usize;
                let rec = 2 + size1 + size2;
                let (mut lo, mut hi) = (0, n);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let at = set + 2 + mid * rec;
                    let second = u16_at(g, at)?;
                    if second == b {
                        // Device offsets in a pair set are from the pair set (as HarfBuzz reads them).
                        return self.value(set, at + 2, vf1);
                    } else if second < b {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                None
            }
            2 => {
                let (cd1, cd2) = (st + u16_at(g, st + 8)? as usize, st + u16_at(g, st + 10)? as usize);
                let (n1, n2) = (u16_at(g, st + 12)? as usize, u16_at(g, st + 14)? as usize);
                let (c1, c2) = (class(g, cd1, a)? as usize, class(g, cd2, b)? as usize);
                if c1 >= n1 || c2 >= n2 {
                    return None;
                }
                let at = st + 16 + (c1 * n2 + c2) * (size1 + size2);
                self.value(st, at, vf1)
            }
            _ => None,
        }
    }

    /// A value record's x-advance and its variation delta.
    fn value(&self, base: usize, at: usize, format: u16) -> Option<f32> {
        let g = &self.gpos;
        let mut v = 0.0;
        if format & 0x0004 != 0 {
            v += i16_at(g, at + 2 * (format & 0x0003).count_ones() as usize)? as f32;
        }
        if format & 0x0040 != 0 {
            let off = u16_at(g, at + 2 * (format & 0x003f).count_ones() as usize)? as usize;
            if off != 0 {
                let dev = base + off;
                // 0x8000: a variation index; formats 1 to 3 are per-size hinting deltas, left out.
                if u16_at(g, dev + 4) == Some(0x8000)
                    && let Some(d) = &self.deltas
                {
                    v += d.delta(u16_at(g, dev)?, u16_at(g, dev + 2)?).unwrap_or(0.0);
                }
            }
        }
        Some(v)
    }

    /// How many lookups and pair subtables were found (tests).
    pub fn subtables(&self) -> usize {
        self.lookups.iter().map(Vec::len).sum()
    }
}

/// The record's size in bytes: two per field present.
fn value_size(format: u16) -> usize {
    2 * (format & 0x00ff).count_ones() as usize
}

/// The glyph's index in a coverage table, if it is covered.
fn covered(b: &[u8], at: usize, glyph: u16) -> Option<usize> {
    let n = u16_at(b, at + 2)? as usize;
    match u16_at(b, at)? {
        1 => {
            let (mut lo, mut hi) = (0, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let g = u16_at(b, at + 4 + 2 * mid)?;
                if g == glyph {
                    return Some(mid);
                } else if g < glyph {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            None
        }
        2 => {
            let (start, end, first) = range(b, at + 4, n, glyph)?;
            (glyph >= start && glyph <= end).then(|| first as usize + (glyph - start) as usize)
        }
        _ => None,
    }
}

/// The glyph's class in a class definition (0 when it has none).
fn class(b: &[u8], at: usize, glyph: u16) -> Option<u16> {
    match u16_at(b, at)? {
        1 => {
            let (start, n) = (u16_at(b, at + 2)?, u16_at(b, at + 4)?);
            if glyph < start || glyph - start >= n {
                return Some(0);
            }
            Some(u16_at(b, at + 6 + 2 * (glyph - start) as usize).unwrap_or(0))
        }
        2 => {
            let n = u16_at(b, at + 2)? as usize;
            Some(range(b, at + 4, n, glyph).filter(|(s, e, _)| glyph >= *s && glyph <= *e).map_or(0, |r| r.2))
        }
        _ => None,
    }
}

/// The range record (start, end, value) holding `glyph`, by binary search
/// over `n` six-byte records at `at`.
fn range(b: &[u8], at: usize, n: usize, glyph: u16) -> Option<(u16, u16, u16)> {
    let (mut lo, mut hi) = (0, n);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let r = at + 6 * mid;
        let (start, end) = (u16_at(b, r)?, u16_at(b, r + 2)?);
        if glyph < start {
            hi = mid;
        } else if glyph > end {
            lo = mid + 1;
        } else {
            return Some((start, end, u16_at(b, r + 4)?));
        }
    }
    None
}

/// The pair adjustment subtables of the `kern` feature's lookups.
fn pair_lookups(g: &[u8]) -> Option<Vec<Vec<usize>>> {
    let (scripts, features, lookups) = (u16_at(g, 4)? as usize, u16_at(g, 6)? as usize, u16_at(g, 8)? as usize);
    let feature_count = u16_at(g, features)? as usize;
    let is_kern = |i: usize| g.get(features + 2 + 6 * i..features + 6 + 6 * i) == Some(b"kern");
    // The features the default script and Latin use by default.
    let mut chosen: Vec<usize> = Vec::new();
    for s in 0..(u16_at(g, scripts)? as usize).min(512) {
        let rec = scripts + 2 + 6 * s;
        let Some(tag) = g.get(rec..rec + 4) else { break };
        if tag != b"DFLT" && tag != b"latn" {
            continue;
        }
        let script = scripts + u16_at(g, rec + 4)? as usize;
        let lang = u16_at(g, script)? as usize;
        if lang == 0 {
            continue;
        }
        let lang = script + lang;
        if let Some(req) = u16_at(g, lang + 2).filter(|r| *r != 0xffff) {
            chosen.push(req as usize);
        }
        for k in 0..u16_at(g, lang + 4)? as usize {
            chosen.push(u16_at(g, lang + 6 + 2 * k)? as usize);
        }
    }
    chosen.retain(|i| *i < feature_count && is_kern(*i));
    if chosen.is_empty() {
        chosen = (0..feature_count.min(4096)).filter(|i| is_kern(*i)).collect();
    }
    let mut indices: Vec<usize> = Vec::new();
    for f in chosen {
        let feature = features + u16_at(g, features + 6 + 6 * f)? as usize;
        for k in 0..u16_at(g, feature + 2)? as usize {
            indices.push(u16_at(g, feature + 4 + 2 * k)? as usize);
        }
    }
    indices.sort_unstable();
    indices.dedup();
    let lookup_count = u16_at(g, lookups)? as usize;
    let mut out = Vec::new();
    let mut total = 0;
    for i in indices.into_iter().filter(|i| *i < lookup_count).take(MAX_LOOKUPS) {
        let lookup = lookups + u16_at(g, lookups + 2 + 2 * i)? as usize;
        let kind = u16_at(g, lookup)?;
        let n = u16_at(g, lookup + 4)? as usize;
        let mut subtables = Vec::new();
        for k in 0..n {
            if total >= MAX_SUBTABLES {
                break;
            }
            let st = lookup + u16_at(g, lookup + 6 + 2 * k)? as usize;
            let st = match kind {
                2 => st,
                // An extension lookup: format 1, the real type, a 32-bit offset.
                9 if u16_at(g, st) == Some(1) && u16_at(g, st + 2) == Some(2) => st + u32_at(g, st + 4)? as usize,
                _ => continue,
            };
            if matches!(u16_at(g, st), Some(1 | 2)) {
                subtables.push(st);
                total += 1;
            }
        }
        if !subtables.is_empty() {
            out.push(subtables);
        }
    }
    Some(out)
}

impl Deltas {
    fn read(gdef: &[u8], coords: &[f32]) -> Option<Deltas> {
        // GDEF 1.3 keeps the item variation store's offset at 14.
        if u16_at(gdef, 0)? != 1 || u16_at(gdef, 2)? < 3 {
            return None;
        }
        let store = u32_at(gdef, 14)? as usize;
        if store == 0 || u16_at(gdef, store)? != 1 {
            return None;
        }
        let regions = store + u32_at(gdef, store + 2)? as usize;
        let (axes, count) = (u16_at(gdef, regions)? as usize, (u16_at(gdef, regions + 2)? as usize).min(MAX_REGIONS));
        let mut scalars = Vec::with_capacity(count);
        for r in 0..count {
            let mut scalar = 1.0f32;
            for a in 0..axes {
                let at = regions + 4 + (r * axes + a) * 6;
                let (Some(start), Some(peak), Some(end)) =
                    (f2dot14(gdef, at), f2dot14(gdef, at + 2), f2dot14(gdef, at + 4))
                else {
                    scalar = 0.0;
                    break;
                };
                let c = coords.get(a).copied().unwrap_or(0.0);
                scalar *= axis_scalar(start, peak, end, c);
                if scalar == 0.0 {
                    break;
                }
            }
            scalars.push(scalar);
        }
        Some(Deltas { gdef: gdef.to_vec(), store, scalars })
    }

    fn delta(&self, outer: u16, inner: u16) -> Option<f32> {
        let (b, s) = (&self.gdef, self.store);
        if outer as usize >= u16_at(b, s + 6)? as usize {
            return None;
        }
        let data = s + u32_at(b, s + 8 + 4 * outer as usize)? as usize;
        let (items, words, regions) = (u16_at(b, data)?, u16_at(b, data + 2)?, u16_at(b, data + 4)? as usize);
        if inner >= items {
            return None;
        }
        let long = words & 0x8000 != 0;
        let wide = ((words & 0x7fff) as usize).min(regions);
        let (big, small) = if long { (4, 2) } else { (2, 1) };
        let row = data + 6 + 2 * regions + inner as usize * (wide * big + (regions - wide) * small);
        let mut total = 0.0;
        let mut at = row;
        for j in 0..regions {
            let region = u16_at(b, data + 6 + 2 * j)? as usize;
            let d = match (j < wide, long) {
                (true, true) => u32_at(b, at)? as i32 as f32,
                (true, false) | (false, true) => i16_at(b, at)? as f32,
                (false, false) => *b.get(at)? as i8 as f32,
            };
            at += if j < wide { big } else { small };
            total += self.scalars.get(region).copied().unwrap_or(0.0) * d;
        }
        Some(total)
    }
}

/// How much a region's axis applies at coordinate `c` (OpenType's rules).
fn axis_scalar(start: f32, peak: f32, end: f32, c: f32) -> f32 {
    if start > peak || peak > end || (start < 0.0 && end > 0.0 && peak != 0.0) || peak == 0.0 || c == peak {
        return 1.0;
    }
    if c <= start || c >= end {
        return 0.0;
    }
    if c < peak { (c - start) / (peak - start) } else { (end - c) / (end - peak) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fonts").join(name))
            .expect("the shipped fonts")
    }

    /// Glyph numbers from the font's cmap, through ab_glyph.
    fn glyph(file: &[u8], c: char) -> u16 {
        use ab_glyph::Font;
        ab_glyph::FontRef::try_from_slice(file).unwrap().glyph_id(c).0
    }

    #[test]
    fn inter_is_kerned_from_gpos() {
        let file = shipped("InterVariable.ttf");
        assert!(table(&file, 0, b"kern").is_none(), "Inter has no legacy kern table: the reason for this file");
        let k = Kerning::read(&file, 0, &coordinates(&file, 0, &[(*b"wght", 400.0)])).expect("Inter's GPOS kerning");
        assert!(k.subtables() > 0);
        for pair in ["AV", "To", "Ty", "Yo", "LT"] {
            let mut c = pair.chars();
            let (a, b) = (glyph(&file, c.next().unwrap()), glyph(&file, c.next().unwrap()));
            assert!(k.pair(a, b) < -20.0, "{pair}: {}", k.pair(a, b));
        }
        let h = glyph(&file, 'H');
        assert!(k.pair(h, h).abs() < 20.0, "HH is barely kerned: {}", k.pair(h, h));
    }

    #[test]
    fn bold_is_kerned_as_bold() {
        let file = shipped("InterVariable.ttf");
        let (a, v) = (glyph(&file, 'A'), glyph(&file, 'V'));
        let at = |w: f32| Kerning::read(&file, 0, &coordinates(&file, 0, &[(*b"wght", w)])).unwrap().pair(a, v);
        let (thin, regular, black) = (at(100.0), at(400.0), at(900.0));
        eprintln!("AV: 100 {thin}, 400 {regular}, 900 {black}");
        assert!(thin != regular || black != regular, "the variation deltas apply");
    }

    /// HarfBuzz 11's kerning of "AVATo.Ty" in Inter 4.1, at three weights
    /// (hb-shape with and without `kern` on kepler, 2026-11-19).
    #[test]
    fn the_same_as_harfbuzz() {
        let file = shipped("InterVariable.ttf");
        let text: Vec<u16> = "AVATo.Ty".chars().map(|c| glyph(&file, c)).collect();
        for (w, want) in [
            (400.0, [-140, -140, -174, -160, 0, -69, -128]),
            (700.0, [-162, -162, -182, -160, 0, -69, -128]),
            (900.0, [-180, -180, -189, -160, 0, -69, -128]),
        ] {
            let k = Kerning::read(&file, 0, &coordinates(&file, 0, &[(*b"wght", w)])).unwrap();
            let got: Vec<i32> = text.windows(2).map(|p| k.pair(p[0], p[1]).round() as i32).collect();
            assert_eq!(got, want, "weight {w}");
        }
    }

    #[test]
    fn coordinates_are_normalized() {
        let file = shipped("InterVariable.ttf");
        // Inter's axes: opsz 14..32 (default 14), wght 100..900 (default 400).
        assert_eq!(coordinates(&file, 0, &[]), vec![0.0, 0.0]);
        let c = coordinates(&file, 0, &[(*b"wght", 900.0), (*b"opsz", 32.0)]);
        assert_eq!(c, vec![1.0, 1.0]);
        let c = coordinates(&file, 0, &[(*b"wght", 100.0)]);
        assert_eq!(c[1], -1.0);
        let mid = coordinates(&file, 0, &[(*b"wght", 700.0)])[1];
        assert!(mid > 0.0 && mid < 1.0, "{mid}");
    }

    #[test]
    fn axes_are_read() {
        let file = shipped("InterVariable.ttf");
        assert_eq!(axis(&file, 0, b"opsz"), Some((14.0, 14.0, 32.0)));
        assert_eq!(axis(&file, 0, b"wght"), Some((100.0, 400.0, 900.0)));
        assert_eq!(axis(&shipped("JetBrainsMono-Variable.ttf"), 0, b"opsz"), None);
    }

    #[test]
    fn a_fixed_pitch_font_keeps_its_cells() {
        let file = shipped("JetBrainsMono-Variable.ttf");
        if let Some(k) = Kerning::read(&file, 0, &[]) {
            let (a, v) = (glyph(&file, 'A'), glyph(&file, 'V'));
            assert_eq!(k.pair(a, v), 0.0);
        }
    }

    #[test]
    fn damaged_tables_do_not_panic() {
        let file = shipped("InterVariable.ttf");
        let coords = coordinates(&file, 0, &[(*b"wght", 700.0)]);
        let mut rng = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        // Damage aimed at GPOS and GDEF, where this file reads.
        let spans: Vec<(usize, usize)> = [b"GPOS", b"GDEF", b"fvar", b"avar"]
            .iter()
            .filter_map(|t| table(&file, 0, t).map(|s| (s.as_ptr() as usize - file.as_ptr() as usize, s.len())))
            .collect();
        let iters: usize = std::env::var("FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(200);
        for _ in 0..iters {
            let mut f = file.clone();
            for _ in 0..16 {
                let (start, len) = spans[next() as usize % spans.len()];
                let i = start + next() as usize % len;
                f[i] = next() as u8;
            }
            if let Some(k) = Kerning::read(&f, 0, &coords) {
                for _ in 0..64 {
                    k.pair(next() as u16 % 3000, next() as u16 % 3000);
                }
            }
            let _ = coordinates(&f, 0, &[(*b"wght", 650.0)]);
        }
        // Truncated anywhere.
        for len in (0..file.len()).step_by(file.len() / 97) {
            let _ = Kerning::read(&file[..len], 0, &coords).map(|k| k.pair(36, 57));
        }
    }
}
