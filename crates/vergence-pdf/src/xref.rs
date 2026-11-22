//! Where each object is (ISO 32000-2 §7.5): cross-reference tables and
//! streams, newest first through /Prev, the hybrid /XRefStm, and, when
//! they are missing or wrong, a repair scan of the whole file for
//! `n g obj`, as every reader that copes with real files does.

use std::collections::{HashMap, HashSet};

use crate::lexer::{Lexer, find, is_white, rfind};
use crate::object::{Dict, Object};

/// The most objects a file may claim, and the most sections chained.
pub const MAX_OBJECTS: usize = 8 << 20;
const MAX_SECTIONS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Free,
    /// At a byte offset, with its generation.
    At {
        offset: usize,
        generation: u16,
    },
    /// The `idx`th object of object stream `stm`.
    InStream {
        stm: u32,
        idx: u32,
    },
}

pub struct Table {
    pub entries: HashMap<u32, Entry>,
    /// The newest trailer (a table's, or an xref stream's dictionary).
    pub trailer: Dict,
}

/// The cross-reference sections from `startxref`, newest first. None if
/// there is no usable chain (the caller repairs).
pub fn read(data: &[u8], header: usize) -> Option<Table> {
    let tail = data.len().saturating_sub(4096);
    let at = tail + rfind(&data[tail..], b"startxref")?;
    let mut l = Lexer::new(data, at + 9);
    let first = l.uint()? as usize;
    let mut table = Table { entries: HashMap::new(), trailer: Dict::default() };
    let mut seen = HashSet::new();
    let mut next = Some(first);
    let mut sections = 0;
    while let Some(off) = next.take() {
        sections += 1;
        if sections > MAX_SECTIONS || !seen.insert(off) {
            break;
        }
        // Some files count offsets from the %PDF header, not the file's start.
        let section = section(data, off, &mut table)
            .or_else(|| if header > 0 { section(data, off + header, &mut table) } else { None });
        let Some(dict) = section else {
            // The first section unreadable: repair. A later one: what we have.
            if sections == 1 {
                return None;
            }
            break;
        };
        if table.trailer.is_empty() {
            table.trailer = dict.clone();
        }
        if let Some(stm) = dict.int("XRefStm").filter(|v| *v > 0) {
            let mut scratch = Table { entries: HashMap::new(), trailer: Dict::default() };
            if stream_section(data, stm as usize, &mut scratch).is_some() {
                for (k, v) in scratch.entries {
                    table.entries.entry(k).or_insert(v);
                }
            }
        }
        next = dict.int("Prev").filter(|v| *v >= 0).map(|v| v as usize);
    }
    (!table.entries.is_empty()).then_some(table)
}

/// One section at `off`: a table with its trailer, or an xref stream.
/// Entries already known (from newer sections) are kept.
fn section(data: &[u8], off: usize, table: &mut Table) -> Option<Dict> {
    let mut l = Lexer::new(data, off);
    if l.keyword(b"xref") {
        loop {
            let save = l.pos;
            let (Some(start), Some(count)) = (l.uint(), l.uint()) else {
                l.pos = save;
                break;
            };
            if start as usize > MAX_OBJECTS || count as usize > MAX_OBJECTS {
                return None;
            }
            for i in 0..count as u32 {
                let (Some(offset), Some(generation)) = (l.uint(), l.uint()) else { return None };
                let kind = l.word();
                let num = start as u32 + i;
                let entry = match kind {
                    b"n" if offset > 0 => {
                        Entry::At { offset: offset as usize, generation: generation.min(u16::MAX as u64) as u16 }
                    }
                    _ => Entry::Free,
                };
                table.entries.entry(num).or_insert(entry);
            }
        }
        if !l.keyword(b"trailer") {
            return None;
        }
        return l.object(0, &|_| None).and_then(|o| o.as_dict().cloned());
    }
    stream_section(data, off, table)
}

/// An xref stream (PDF 1.5) at `off`: /W field widths, /Index ranges.
fn stream_section(data: &[u8], off: usize, table: &mut Table) -> Option<Dict> {
    let mut l = Lexer::new(data, off);
    let (_, obj) = l.indirect(&|_| None)?;
    let Object::Stream(s) = obj else { return None };
    if s.dict.name("Type") != Some(b"XRef") {
        return None;
    }
    let w: Vec<usize> =
        s.dict.get("W")?.as_array()?.iter().map(|o| o.as_int().unwrap_or(0).clamp(0, 8) as usize).collect();
    if w.len() < 3 {
        return None;
    }
    let size = s.dict.int("Size").unwrap_or(0).clamp(0, MAX_OBJECTS as i64);
    let index: Vec<i64> = match s.dict.get("Index").and_then(Object::as_array) {
        Some(a) => a.iter().filter_map(Object::as_int).collect(),
        None => vec![0, size],
    };
    let raw = data.get(s.start..s.start + s.len)?;
    let (names, parms) = crate::filter::filters_of(&s.dict);
    let decoded = crate::filter::decode(raw, &names, &parms).data;
    let row = w[0] + w[1] + w[2];
    if row == 0 {
        return None;
    }
    let field =
        |r: &[u8], from: usize, width: usize| r[from..from + width].iter().fold(0u64, |a, b| a << 8 | *b as u64);
    let mut rows = decoded.chunks_exact(row);
    for pair in index.chunks(2) {
        let [start, count] = pair else { break };
        if *start < 0 || *count < 0 || *start as usize > MAX_OBJECTS || *count as usize > MAX_OBJECTS {
            return None;
        }
        for i in 0..*count {
            let Some(r) = rows.next() else { return Some(s.dict.clone()) };
            // A missing type field means 1.
            let kind = if w[0] == 0 { 1 } else { field(r, 0, w[0]) };
            let (a, b) = (field(r, w[0], w[1]), field(r, w[0] + w[1], w[2]));
            let entry = match kind {
                1 if a > 0 => Entry::At { offset: a as usize, generation: b.min(u16::MAX as u64) as u16 },
                2 => Entry::InStream { stm: a.min(u32::MAX as u64) as u32, idx: b.min(u32::MAX as u64) as u32 },
                _ => Entry::Free,
            };
            table.entries.entry((*start + i) as u32).or_insert(entry);
        }
    }
    Some(s.dict.clone())
}

/// Every `n g obj` in the file (the last one of a number wins, as later
/// updates do), and the trailer: the last `trailer` dictionary that names
/// a /Root, else an xref stream's that does, else one made up around the
/// first catalog found.
pub fn repair(data: &[u8]) -> Table {
    let mut entries = HashMap::new();
    let mut i = 0;
    while let Some(k) = find(&data[i..], b"obj") {
        let at = i + k;
        i = at + 3;
        // "obj" standing alone, after "num generation ".
        if data.get(at + 3).is_some_and(|b| !is_white(*b) && !crate::lexer::is_delim(*b)) {
            continue;
        }
        if let Some((num, generation, start)) = number_pair_before(data, at)
            && (num as usize) < MAX_OBJECTS
        {
            entries.insert(num, Entry::At { offset: start, generation });
        }
        if entries.len() >= MAX_OBJECTS {
            break;
        }
    }
    let mut trailer = Dict::default();
    let mut j = data.len();
    while let Some(k) = rfind(&data[..j], b"trailer") {
        let mut l = Lexer::new(data, k + 7);
        if let Some(d) = l.object(0, &|_| None).and_then(|o| o.as_dict().cloned())
            && d.has("Root")
        {
            trailer = d;
            break;
        }
        j = k;
    }
    if !trailer.has("Root") {
        // An xref stream's dictionary, or the catalog itself.
        let mut by_offset: Vec<(u32, usize)> = entries
            .iter()
            .filter_map(|(n, e)| if let Entry::At { offset, .. } = e { Some((*n, *offset)) } else { None })
            .collect();
        by_offset.sort_by_key(|(_, o)| std::cmp::Reverse(*o));
        for (num, off) in by_offset {
            let mut l = Lexer::new(data, off);
            let Some((_, obj)) = l.indirect(&|_| None) else { continue };
            let Some(d) = obj.as_dict() else { continue };
            if d.name("Type") == Some(b"XRef") && d.has("Root") {
                trailer = d.clone();
                break;
            }
            if d.name("Type") == Some(b"Catalog") {
                trailer = Dict(vec![(b"Root".to_vec(), Object::Ref(crate::object::Ref { num, generation: 0 }))]);
                break;
            }
        }
    }
    Table { entries, trailer }
}

/// For "obj" at `at`: the `num generation` before it, and where `num` starts.
fn number_pair_before(data: &[u8], at: usize) -> Option<(u32, u16, usize)> {
    let mut p = at;
    let skip_white = |p: &mut usize| {
        while *p > 0 && is_white(data[*p - 1]) {
            *p -= 1;
        }
    };
    let digits = |p: &mut usize| -> Option<u64> {
        let end = *p;
        while *p > 0 && data[*p - 1].is_ascii_digit() && end - *p < 10 {
            *p -= 1;
        }
        if *p == end {
            return None;
        }
        std::str::from_utf8(&data[*p..end]).ok()?.parse().ok()
    };
    if p == 0 || !is_white(data[p - 1]) {
        return None;
    }
    skip_white(&mut p);
    let generation = digits(&mut p)?;
    if p == 0 || !is_white(data[p - 1]) {
        return None;
    }
    skip_white(&mut p);
    let num = digits(&mut p)?;
    // The number starts a token.
    if p > 0 && !is_white(data[p - 1]) && !crate::lexer::is_delim(data[p - 1]) {
        return None;
    }
    (num <= u32::MAX as u64 && generation <= u16::MAX as u64).then_some((num as u32, generation as u16, p))
}
