//! What a font file says about itself, read by us: the family, weight and
//! slant of each face in it, whether it is fixed-pitch, its weight axis if it
//! is a variable font, and which characters it has.
//!
//! Only the tables that answer those questions are read (`name`, `OS/2`,
//! `head`, `post`, `fvar`, and `cmap` when coverage is asked for), a few
//! kilobytes of a file that may be megabytes, so a whole font directory is
//! indexed quickly. Glyph outlines are ab_glyph's business (lib.rs).
//!
//! **A font file is untrusted input.** Every offset and count is checked
//! against the bytes actually read, tables larger than [`MAX_TABLE`] are
//! ignored, and nothing here can panic on any input (tests/fuzz.rs).

use std::io::{Read, Seek, SeekFrom};

/// The largest table read, and the most faces taken from one collection.
pub const MAX_TABLE: usize = 16 << 20;
pub const MAX_FACES: usize = 256;
const MAX_TABLES: usize = 512;

/// Somewhere font bytes can be read from at an offset: a file, or a slice.
pub trait Source {
    /// Exactly `len` bytes at `offset`, or None if they are not there.
    fn read_at(&mut self, offset: u64, len: usize) -> Option<Vec<u8>>;
}

impl Source for &[u8] {
    fn read_at(&mut self, offset: u64, len: usize) -> Option<Vec<u8>> {
        let start = usize::try_from(offset).ok()?;
        self.get(start..start.checked_add(len)?).map(<[u8]>::to_vec)
    }
}

impl Source for std::fs::File {
    fn read_at(&mut self, offset: u64, len: usize) -> Option<Vec<u8>> {
        if len > MAX_TABLE {
            return None;
        }
        self.seek(SeekFrom::Start(offset)).ok()?;
        let mut buf = vec![0; len];
        self.read_exact(&mut buf).ok()?;
        Some(buf)
    }
}

/// One face of a font file.
#[derive(Debug, Clone, PartialEq)]
pub struct FaceInfo {
    pub family: String,
    /// 1 to 1000 (400 regular, 700 bold): the face's own, or a variable
    /// font's default.
    pub weight: u16,
    /// OS/2 usWidthClass, 1 to 9 (5 normal; 3 condensed; 7 expanded).
    pub width: u16,
    pub italic: bool,
    pub monospace: bool,
    /// Which face of a collection (0 for a plain font file).
    pub index: u32,
    /// A variable font's weight axis: the least and most it can be.
    pub weight_axis: Option<(u16, u16)>,
    /// Where the character map is in the file: offset and length.
    pub cmap: Option<(u64, u32)>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// The faces in a font file (one, or a collection's), skipping any that
/// cannot be read. Empty if this is not a font.
pub fn faces(src: &mut dyn Source) -> Vec<FaceInfo> {
    let Some(head) = src.read_at(0, 12) else { return Vec::new() };
    let offsets: Vec<u32> = match &head[..4] {
        b"ttcf" => {
            let n = (u32_at(&head, 8).unwrap_or(0) as usize).min(MAX_FACES);
            let Some(table) = src.read_at(12, n * 4) else { return Vec::new() };
            (0..n).filter_map(|i| u32_at(&table, i * 4)).collect()
        }
        [0, 1, 0, 0] | b"OTTO" | b"true" => vec![0],
        _ => return Vec::new(),
    };
    offsets.iter().enumerate().filter_map(|(i, off)| face(src, *off as u64, i as u32)).collect()
}

/// A table's offset and length, by tag, from a face's table directory.
struct Tables(Vec<([u8; 4], u32, u32)>);

impl Tables {
    fn read(src: &mut dyn Source, at: u64) -> Option<Tables> {
        let head = src.read_at(at, 12)?;
        let n = (u16_at(&head, 4)? as usize).min(MAX_TABLES);
        let dir = src.read_at(at + 12, n * 16)?;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let r = &dir[i * 16..i * 16 + 16];
            out.push(([r[0], r[1], r[2], r[3]], u32_at(r, 8)?, u32_at(r, 12)?));
        }
        Some(Tables(out))
    }

    fn find(&self, tag: &[u8; 4]) -> Option<(u32, u32)> {
        self.0.iter().find(|t| &t.0 == tag).map(|t| (t.1, t.2))
    }

    /// The table's bytes, at most `cap` of them (None if it is larger).
    fn load(&self, src: &mut dyn Source, tag: &[u8; 4], cap: usize) -> Option<Vec<u8>> {
        let (off, len) = self.find(tag)?;
        if len as usize > cap {
            return None;
        }
        src.read_at(off as u64, len as usize)
    }
}

fn face(src: &mut dyn Source, at: u64, index: u32) -> Option<FaceInfo> {
    let tables = Tables::read(src, at)?;
    let family = family_name(&tables.load(src, b"name", 1 << 20)?)?;
    let os2 = tables.load(src, b"OS/2", 4096);
    let weight = os2.as_deref().and_then(|t| u16_at(t, 4)).filter(|w| (1..=1000).contains(w)).unwrap_or(400);
    let width = os2.as_deref().and_then(|t| u16_at(t, 6)).filter(|w| (1..=9).contains(w)).unwrap_or(5);
    let selection = os2.as_deref().and_then(|t| u16_at(t, 62));
    let mac_style = tables.load(src, b"head", 4096).and_then(|t| u16_at(&t, 44)).unwrap_or(0);
    // OS/2 fsSelection bit 0 (italic) or 9 (oblique); head's macStyle bit 1 if there is no OS/2.
    let italic = selection.map(|s| s & 0x0201 != 0).unwrap_or(mac_style & 2 != 0);
    // Fixed pitch: `post` says so, or the PANOSE classification does (a Latin
    // text face, byte 0 = 2, whose proportion, byte 3, is 9). Noto Sans Mono
    // says no in `post` and yes in PANOSE.
    let post = tables.load(src, b"post", 1 << 20).and_then(|t| u32_at(&t, 12)).is_some_and(|v| v != 0);
    let panose = os2.as_deref().and_then(|t| t.get(32..36)).is_some_and(|p| p[0] == 2 && p[3] == 9);
    let monospace = post || panose;
    let weight_axis = tables.load(src, b"fvar", 1 << 16).and_then(|t| weight_axis(&t));
    let cmap = tables.find(b"cmap").map(|(o, l)| (o as u64, l));
    Some(FaceInfo { family, weight, width, italic, monospace, index, weight_axis, cmap })
}

/// The family from a `name` table: the typographic family (name 16) if there
/// is one, else the family (name 1); a Windows English entry before any
/// other.
fn family_name(t: &[u8]) -> Option<String> {
    let count = u16_at(t, 2)? as usize;
    let strings = u16_at(t, 4)? as usize;
    let mut best: Option<(u32, String)> = None;
    for i in 0..count {
        let r = 6 + i * 12;
        let (platform, encoding, language, id) =
            (u16_at(t, r)?, u16_at(t, r + 2)?, u16_at(t, r + 4)?, u16_at(t, r + 6)?);
        if id != 16 && id != 1 {
            continue;
        }
        let (len, off) = (u16_at(t, r + 8)? as usize, u16_at(t, r + 10)? as usize);
        let Some(bytes) = t.get(strings.checked_add(off)?..strings.checked_add(off)?.checked_add(len)?) else {
            continue;
        };
        let text = match platform {
            // Unicode and Windows: UTF-16, big-endian.
            0 | 3 => {
                let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                String::from_utf16(&units).ok()
            }
            // Macintosh Roman: the ASCII part is enough for a family name.
            1 if encoding == 0 => bytes.iter().all(u8::is_ascii).then(|| String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        };
        let Some(text) = text
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
        else {
            continue;
        };
        // Higher is better: name 16 over name 1, then Windows English, Windows, Unicode, Mac.
        let rank = (if id == 16 { 10 } else { 0 })
            + match (platform, language) {
                (3, 0x0409) => 4,
                (3, _) => 3,
                (0, _) => 2,
                _ => 1,
            };
        if best.as_ref().is_none_or(|(r, _)| rank > *r) {
            best = Some((rank, text));
        }
    }
    best.map(|(_, s)| s)
}

/// The `wght` axis of an `fvar` table: its least and greatest value.
fn weight_axis(t: &[u8]) -> Option<(u16, u16)> {
    let axes = u16_at(t, 4)? as usize;
    let count = u16_at(t, 8)? as usize;
    let size = u16_at(t, 10)? as usize;
    if size < 20 {
        return None;
    }
    for i in 0..count.min(64) {
        let a = axes.checked_add(i.checked_mul(size)?)?;
        if t.get(a..a.checked_add(4)?)? == b"wght" {
            // Fixed 16.16: the whole part is the weight.
            let (min, max) = (u32_at(t, a + 4)? >> 16, u32_at(t, a + 12)? >> 16);
            let ok = |v: u32| (1..=1000).contains(&v);
            return (ok(min) && ok(max) && min <= max).then_some((min as u16, max as u16));
        }
    }
    None
}

/// The characters a font has, as sorted ranges that do not overlap.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Coverage(Vec<(u32, u32)>);

impl Coverage {
    pub fn contains(&self, c: char) -> bool {
        let c = c as u32;
        let i = self.0.partition_point(|r| r.1 < c);
        self.0.get(i).is_some_and(|r| r.0 <= c)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn from_ranges(mut ranges: Vec<(u32, u32)>) -> Coverage {
        ranges.retain(|r| r.0 <= r.1 && r.1 <= 0x10ffff);
        ranges.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
        for r in ranges {
            match out.last_mut() {
                Some(last) if r.0 <= last.1.saturating_add(1) => last.1 = last.1.max(r.1),
                _ => out.push(r),
            }
        }
        Coverage(out)
    }
}

/// The coverage a `cmap` table describes: its full-Unicode subtable (format
/// 12) if it has one, else its 16-bit one (format 4). A font may still map a
/// covered character to "no glyph"; the caller checks the glyph itself.
pub fn coverage(t: &[u8]) -> Coverage {
    let Some(n) = u16_at(t, 2) else { return Coverage::default() };
    let mut best: Option<(u8, usize)> = None;
    for i in 0..(n as usize).min(64) {
        let r = 4 + i * 8;
        let (Some(platform), Some(encoding), Some(off)) = (u16_at(t, r), u16_at(t, r + 2), u32_at(t, r + 4)) else {
            break;
        };
        let off = off as usize;
        let Some(format) = u16_at(t, off) else { continue };
        let rank = match (platform, encoding, format) {
            (3, 10, 12) | (0, 4 | 6, 12) => 4,
            (0, _, 12) => 3,
            (3, 1, 4) | (0, _, 4) => 2,
            _ => 0,
        };
        if rank > 0 && best.is_none_or(|(r, _)| rank > r) {
            best = Some((rank, off));
        }
    }
    let Some((_, off)) = best else { return Coverage::default() };
    let ranges = match u16_at(t, off) {
        Some(12) => format12(t, off),
        Some(4) => format4(t, off),
        _ => None,
    };
    Coverage::from_ranges(ranges.unwrap_or_default())
}

fn format12(t: &[u8], off: usize) -> Option<Vec<(u32, u32)>> {
    let groups = u32_at(t, off.checked_add(12)?)? as usize;
    let first = off.checked_add(16)?;
    // No more groups than the table has bytes for.
    let groups = groups.min(t.len().saturating_sub(first) / 12);
    let mut out = Vec::with_capacity(groups);
    for i in 0..groups {
        let g = first + i * 12;
        out.push((u32_at(t, g)?, u32_at(t, g + 4)?));
    }
    Some(out)
}

fn format4(t: &[u8], off: usize) -> Option<Vec<(u32, u32)>> {
    let segs = (u16_at(t, off.checked_add(6)?)? / 2) as usize;
    let ends = off.checked_add(14)?;
    let starts = ends.checked_add(segs * 2)?.checked_add(2)?;
    let mut out = Vec::with_capacity(segs);
    for i in 0..segs {
        let (start, end) = (u16_at(t, starts + i * 2)?, u16_at(t, ends + i * 2)?);
        // The last segment is the 0xFFFF end marker, not a character.
        if start == 0xffff {
            continue;
        }
        out.push((start as u32, (end as u32).min(0xfffe)));
    }
    Some(out)
}

/// Font bytes made for the tests: just the tables this file reads.
#[cfg(test)]
pub mod testfont {
    pub struct Spec<'a> {
        pub family: &'a str,
        /// A second name, as name 16 (the typographic family).
        pub typographic: Option<&'a str>,
        pub weight: u16,
        /// usWidthClass (0: written as 0, read as normal).
        pub width: u16,
        pub italic: bool,
        pub monospace: bool,
        /// Fixed pitch said only by PANOSE, as Noto Sans Mono does.
        pub panose_mono: bool,
        pub weight_axis: Option<(u16, u16)>,
        /// 16-bit ranges for a format 4 subtable.
        pub ranges: &'a [(u16, u16)],
        /// Ranges for a format 12 subtable (none: no such subtable).
        pub wide: &'a [(u32, u32)],
    }

    impl Default for Spec<'_> {
        fn default() -> Self {
            Spec {
                family: "Test Sans",
                typographic: None,
                weight: 400,
                width: 5,
                italic: false,
                monospace: false,
                panose_mono: false,
                weight_axis: None,
                ranges: &[(0x20, 0x7e)],
                wide: &[],
            }
        }
    }

    fn name_table(spec: &Spec) -> Vec<u8> {
        let mut names: Vec<(u16, Vec<u8>)> = vec![(1, spec.family.encode_utf16().flat_map(u16::to_be_bytes).collect())];
        if let Some(t) = spec.typographic {
            names.push((16, t.encode_utf16().flat_map(u16::to_be_bytes).collect()));
        }
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&(names.len() as u16).to_be_bytes());
        t.extend_from_slice(&((6 + names.len() * 12) as u16).to_be_bytes());
        let mut strings = Vec::new();
        for (id, s) in &names {
            for v in [3u16, 1, 0x0409, *id, s.len() as u16, strings.len() as u16] {
                t.extend_from_slice(&v.to_be_bytes());
            }
            strings.extend_from_slice(s);
        }
        t.extend_from_slice(&strings);
        t
    }

    fn cmap_table(spec: &Spec) -> Vec<u8> {
        let mut f4 = Vec::new();
        let segs = spec.ranges.len() + 1;
        for v in [4u16, (16 + segs * 8) as u16, 0, (segs * 2) as u16, 0, 0, 0] {
            f4.extend_from_slice(&v.to_be_bytes());
        }
        for r in spec.ranges {
            f4.extend_from_slice(&r.1.to_be_bytes());
        }
        f4.extend_from_slice(&0xffffu16.to_be_bytes());
        f4.extend_from_slice(&0u16.to_be_bytes());
        for r in spec.ranges {
            f4.extend_from_slice(&r.0.to_be_bytes());
        }
        f4.extend_from_slice(&0xffffu16.to_be_bytes());
        f4.extend(std::iter::repeat_n(0u8, segs * 4));
        let mut f12 = Vec::new();
        if !spec.wide.is_empty() {
            f12.extend_from_slice(&12u16.to_be_bytes());
            f12.extend_from_slice(&0u16.to_be_bytes());
            f12.extend_from_slice(&((16 + spec.wide.len() * 12) as u32).to_be_bytes());
            f12.extend_from_slice(&0u32.to_be_bytes());
            f12.extend_from_slice(&(spec.wide.len() as u32).to_be_bytes());
            for r in spec.wide {
                for v in [r.0, r.1, 1] {
                    f12.extend_from_slice(&v.to_be_bytes());
                }
            }
        }
        let subtables = 1 + !f12.is_empty() as usize;
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&(subtables as u16).to_be_bytes());
        let first = 4 + subtables * 8;
        for v in [3u16, 1] {
            t.extend_from_slice(&v.to_be_bytes());
        }
        t.extend_from_slice(&(first as u32).to_be_bytes());
        if !f12.is_empty() {
            for v in [3u16, 10] {
                t.extend_from_slice(&v.to_be_bytes());
            }
            t.extend_from_slice(&((first + f4.len()) as u32).to_be_bytes());
        }
        t.extend_from_slice(&f4);
        t.extend_from_slice(&f12);
        t
    }

    fn tables(spec: &Spec) -> Vec<([u8; 4], Vec<u8>)> {
        let mut os2 = vec![0u8; 78];
        os2[4..6].copy_from_slice(&spec.weight.to_be_bytes());
        os2[6..8].copy_from_slice(&spec.width.to_be_bytes());
        os2[62..64].copy_from_slice(&(spec.italic as u16).to_be_bytes());
        if spec.panose_mono {
            os2[32..36].copy_from_slice(&[2, 11, 5, 9]);
        }
        let mut post = vec![0u8; 32];
        post[12..16].copy_from_slice(&(spec.monospace as u32).to_be_bytes());
        let mut out = vec![
            (*b"OS/2", os2),
            (*b"cmap", cmap_table(spec)),
            (*b"head", vec![0u8; 54]),
            (*b"name", name_table(spec)),
            (*b"post", post),
        ];
        if let Some((min, max)) = spec.weight_axis {
            let mut fvar = vec![0u8; 16 + 20];
            fvar[0..2].copy_from_slice(&1u16.to_be_bytes());
            fvar[4..6].copy_from_slice(&16u16.to_be_bytes());
            fvar[8..10].copy_from_slice(&1u16.to_be_bytes());
            fvar[10..12].copy_from_slice(&20u16.to_be_bytes());
            fvar[16..20].copy_from_slice(b"wght");
            fvar[20..24].copy_from_slice(&((min as u32) << 16).to_be_bytes());
            fvar[24..28].copy_from_slice(&(400u32 << 16).to_be_bytes());
            fvar[28..32].copy_from_slice(&((max as u32) << 16).to_be_bytes());
            out.push((*b"fvar", fvar));
        }
        out
    }

    /// One face's table directory and tables, written as if at `base` in a file.
    fn face_at(spec: &Spec, base: usize) -> Vec<u8> {
        let tables = tables(spec);
        let mut out = vec![0, 1, 0, 0];
        out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        out.extend_from_slice(&[0; 6]);
        let mut at = base + 12 + tables.len() * 16;
        for (tag, data) in &tables {
            out.extend_from_slice(tag);
            out.extend_from_slice(&0u32.to_be_bytes());
            out.extend_from_slice(&(at as u32).to_be_bytes());
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            at += data.len();
        }
        for (_, data) in &tables {
            out.extend_from_slice(data);
        }
        out
    }

    /// A font file with one face.
    pub fn font(spec: &Spec) -> Vec<u8> {
        face_at(spec, 0)
    }

    /// A collection file with these faces.
    pub fn collection(specs: &[Spec]) -> Vec<u8> {
        let mut out = b"ttcf".to_vec();
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&(specs.len() as u32).to_be_bytes());
        let mut at = 12 + specs.len() * 4;
        let mut bodies = Vec::new();
        for s in specs {
            out.extend_from_slice(&(at as u32).to_be_bytes());
            let body = face_at(s, at);
            at += body.len();
            bodies.push(body);
        }
        for b in bodies {
            out.extend_from_slice(&b);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testfont::{Spec, collection, font};
    use super::*;

    #[test]
    fn a_face_says_who_it_is() {
        let bytes =
            font(&Spec { family: "Example Mono", weight: 700, italic: true, monospace: true, ..Spec::default() });
        let f = faces(&mut &bytes[..]);
        assert_eq!(f.len(), 1);
        assert_eq!(
            (f[0].family.as_str(), f[0].weight, f[0].italic, f[0].monospace, f[0].index),
            ("Example Mono", 700, true, true, 0)
        );
        assert_eq!(f[0].weight_axis, None);
    }

    #[test]
    fn fixed_pitch_said_only_by_panose_counts() {
        let bytes = font(&Spec { family: "Quiet Mono", panose_mono: true, ..Spec::default() });
        assert!(faces(&mut &bytes[..])[0].monospace);
        assert!(!faces(&mut &font(&Spec::default())[..])[0].monospace);
    }

    #[test]
    fn the_typographic_family_wins() {
        let bytes = font(&Spec {
            family: "Example Sans Light",
            typographic: Some("Example Sans"),
            weight: 300,
            ..Spec::default()
        });
        assert_eq!(faces(&mut &bytes[..])[0].family, "Example Sans");
    }

    #[test]
    fn a_variable_font_has_a_weight_axis() {
        let bytes = font(&Spec { weight_axis: Some((100, 900)), ..Spec::default() });
        assert_eq!(faces(&mut &bytes[..])[0].weight_axis, Some((100, 900)));
    }

    #[test]
    fn a_collection_has_every_face() {
        let bytes = collection(&[
            Spec { family: "One", ..Spec::default() },
            Spec { family: "Two", weight: 700, ..Spec::default() },
        ]);
        let f = faces(&mut &bytes[..]);
        assert_eq!(
            f.iter().map(|f| (f.family.as_str(), f.index, f.weight)).collect::<Vec<_>>(),
            [("One", 0, 400), ("Two", 1, 700)]
        );
    }

    #[test]
    fn coverage_from_both_kinds_of_map() {
        let bytes = font(&Spec { ranges: &[(0x20, 0x7e), (0xa0, 0xff), (0x2500, 0x257f)], ..Spec::default() });
        let f = &faces(&mut &bytes[..])[0];
        let (off, len) = f.cmap.unwrap();
        let cov = coverage(&bytes[off as usize..off as usize + len as usize]);
        assert!(cov.contains('A') && cov.contains('é') && cov.contains('─'));
        assert!(!cov.contains('\u{7f}') && !cov.contains('漢') && !cov.contains('\u{ffff}') && !cov.contains('🙂'));

        // With a full-Unicode subtable, that one is used.
        let bytes = font(&Spec { wide: &[(0x20, 0x7e), (0x1f600, 0x1f64f)], ..Spec::default() });
        let f = &faces(&mut &bytes[..])[0];
        let (off, len) = f.cmap.unwrap();
        let cov = coverage(&bytes[off as usize..off as usize + len as usize]);
        assert!(cov.contains('A') && cov.contains('🙂') && !cov.contains('é'));
    }

    #[test]
    fn not_a_font_is_no_faces() {
        assert!(faces(&mut &b""[..]).is_empty());
        assert!(faces(&mut &b"hello, this is not a font file"[..]).is_empty());
        assert!(coverage(b"").is_empty());
    }

    #[test]
    fn cut_short_anywhere_is_not_a_panic() {
        let bytes = collection(&[
            Spec::default(),
            Spec { weight_axis: Some((100, 900)), wide: &[(0x20, 0x7e)], ..Spec::default() },
        ]);
        for n in 0..bytes.len() {
            let cut = &bytes[..n];
            for f in faces(&mut &cut[..]) {
                if let Some((off, len)) = f.cmap
                    && let Some(t) = cut.get(off as usize..(off as usize).saturating_add(len as usize))
                {
                    let _ = coverage(t);
                }
            }
        }
    }
}
