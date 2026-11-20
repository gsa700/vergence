//! Our own PDF reader (PDF milestone 1, docs/pdf-milestone-1.md, step 1).
//!
//! [`Document::open`] takes a file's bytes and finds its objects: the
//! cross-reference sections from `startxref` (tables, PDF 1.5 streams, the
//! hybrid kind), or, when they are missing or wrong, a repair scan
//! (xref.rs). [`Document::get`] reads an object where the table says,
//! inside an object stream if it is there; [`Document::stream`] undoes a
//! stream's filters (filter/); [`Document::pages`] walks the page tree
//! with what pages inherit from it.
//!
//! **Every PDF is untrusted input.** Nothing here may panic, loop or grow
//! without bound on any file: nesting and reference chains are limited,
//! decoded streams are capped, reference cycles are cut, and the fuzz tests
//! (tests.rs) feed it damage. In Vergence this runs inside the sandboxed
//! helper, never in Close-Up itself.
//!
//! Encrypted files (the standard security handler, crypt/): opened with
//! the empty user password when that works, as most are;
//! [`Document::needs_password`] and [`Document::unlock`] for the rest.
//! Strings are decrypted as objects are read, streams before their filters.

pub mod ccitt;
pub mod color;
pub mod content;
pub mod crypt;
pub mod filter;
pub mod font;
pub mod function;
pub mod image;
pub mod jbig2;
pub mod lexer;
pub mod object;
pub mod render;
pub mod xref;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub use filter::Decoded;
pub use render::render_page;
use lexer::Lexer;
pub use object::{Dict, Object, Ref, Stream};
use xref::Entry;

/// How many references a lookup follows, how deep the page tree may go,
/// and the most pages taken.
const MAX_CHAIN: usize = 32;
const MAX_TREE_DEPTH: usize = 64;
pub const MAX_PAGES: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No `%PDF-` near the start.
    NotPdf,
    /// No catalog (document root) found, even by repair.
    NoCatalog,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotPdf => write!(f, "not a PDF file"),
            Error::NoCatalog => write!(f, "no document catalog, even after repair"),
        }
    }
}

/// A page: its dictionary and what it inherits from the page tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub reference: Option<Ref>,
    pub dict: Dict,
    pub resources: Option<Dict>,
    /// In points: x0, y0, x1, y1 (US Letter if the file says nothing usable).
    pub media_box: [f64; 4],
    pub crop_box: Option<[f64; 4]>,
    /// 0, 90, 180 or 270.
    pub rotate: u16,
}

pub struct Document {
    data: Vec<u8>,
    entries: HashMap<u32, Entry>,
    trailer: Dict,
    repaired: bool,
    /// Object streams parsed so far, by number: (object number, object).
    object_streams: RefCell<HashMap<u32, Members>>,
    /// Objects being read now, to cut reference cycles.
    reading: RefCell<HashSet<u32>>,
    /// Built when the table pointed somewhere wrong: every `n g obj`.
    scanned: RefCell<Option<Rc<HashMap<u32, Entry>>>>,
    /// An encrypted file's handler, and its keys once a password worked.
    security: Option<crypt::Security>,
    keys: Option<crypt::Keys>,
    /// The /Encrypt dictionary's own number: it is never encrypted.
    encrypt_ref: Option<Ref>,
}

impl Document {
    pub fn open(data: Vec<u8>) -> Result<Document, Error> {
        let header = lexer::find(&data[..data.len().min(1024)], b"%PDF-").ok_or(Error::NotPdf)?;
        let (table, repaired) = match xref::read(&data, header) {
            Some(t) if t.trailer.has("Root") => (t, false),
            _ => (xref::repair(&data), true),
        };
        let mut doc = Document {
            data,
            entries: table.entries,
            trailer: table.trailer,
            repaired,
            object_streams: RefCell::new(HashMap::new()),
            reading: RefCell::new(HashSet::new()),
            scanned: RefCell::new(None),
            security: None,
            keys: None,
            encrypt_ref: None,
        };
        doc.set_up_security();
        if repaired {
            doc.add_object_stream_members();
        }
        if doc.catalog().is_none() && !doc.repaired {
            // The table led somewhere with no catalog: repair after all.
            let t = xref::repair(&doc.data);
            doc.entries = t.entries;
            doc.trailer = t.trailer;
            doc.repaired = true;
            doc.set_up_security();
            doc.add_object_stream_members();
        }
        if doc.catalog().is_none() && !doc.needs_password() {
            return Err(Error::NoCatalog);
        }
        Ok(doc)
    }

    /// Reads /Encrypt and tries the empty user password.
    fn set_up_security(&mut self) {
        let Some(enc) = self.trailer.get("Encrypt").cloned() else { return };
        self.encrypt_ref = enc.as_ref();
        let Some(dict) = self.resolve(&enc).as_dict().cloned() else { return };
        let id0 = match self.trailer.get("ID").map(|o| self.resolve(o)) {
            Some(Object::Array(a)) => a.first().and_then(|o| o.as_bytes()).map(<[u8]>::to_vec).unwrap_or_default(),
            _ => Vec::new(),
        };
        self.security = crypt::Security::new(&dict, &id0);
        self.keys = self.security.as_ref().and_then(|s| s.unlock(b""));
        self.object_streams.borrow_mut().clear();
    }

    /// Whether the file is encrypted with a password it cannot be opened
    /// without (or with a handler this reader does not have).
    pub fn needs_password(&self) -> bool {
        self.encrypted() && self.keys.is_none()
    }

    /// Tries `password` as the user's, then the owner's; true if it opened
    /// the file.
    pub fn unlock(&mut self, password: &[u8]) -> bool {
        let Some(keys) = self.security.as_ref().and_then(|s| s.unlock(password)) else { return false };
        self.keys = Some(keys);
        self.object_streams.borrow_mut().clear();
        true
    }

    /// The security handler's revision (2 to 6), for an encrypted file.
    pub fn encryption_revision(&self) -> Option<i64> {
        self.security.as_ref().map(crypt::Security::revision)
    }

    /// Whether the cross-references had to be rebuilt by scanning.
    pub fn repaired(&self) -> bool {
        self.repaired
    }

    pub fn trailer(&self) -> &Dict {
        &self.trailer
    }

    /// Whether the file is encrypted (its /Encrypt entry).
    pub fn encrypted(&self) -> bool {
        self.trailer.has("Encrypt")
    }

    /// The version in the header ("1.7"), if readable.
    pub fn version(&self) -> Option<String> {
        let at = lexer::find(&self.data[..self.data.len().min(1024)], b"%PDF-")?;
        let v: Vec<u8> = self.data[at + 5..].iter().take(4).take_while(|b| b.is_ascii_digit() || **b == b'.').copied().collect();
        String::from_utf8(v).ok().filter(|s| !s.is_empty())
    }

    pub fn catalog(&self) -> Option<Dict> {
        let root = self.trailer.get("Root")?;
        self.resolve(root).as_dict().cloned()
    }

    /// An object by reference; Null if it is not there or cannot be read.
    pub fn get(&self, r: Ref) -> Object {
        if !self.reading.borrow_mut().insert(r.num) {
            return Object::Null;
        }
        let obj = self.read(r);
        self.reading.borrow_mut().remove(&r.num);
        obj
    }

    fn read(&self, r: Ref) -> Object {
        match self.entries.get(&r.num).copied() {
            Some(Entry::At { offset, .. }) => self.at(offset, r).or_else(|| self.by_scan(r)).map_or(Object::Null, |o| self.decrypted(o, r)),
            Some(Entry::InStream { stm, idx }) => self.in_stream(stm, idx, r.num).unwrap_or(Object::Null),
            Some(Entry::Free) | None => self.by_scan(r).map_or(Object::Null, |o| self.decrypted(o, r)),
        }
    }

    /// An object read from the file itself: its strings decrypted, a
    /// stream marked with its number (decrypted when its data is asked
    /// for). Objects inside object streams came decrypted with their stream.
    fn decrypted(&self, o: Object, r: Ref) -> Object {
        let mut o = o;
        if let Object::Stream(s) = &mut o {
            s.owner = Some(r);
        }
        match &self.keys {
            Some(k) if Some(r) != self.encrypt_ref => decrypt_strings(o, k, r, 0),
            _ => o,
        }
    }

    /// The object at `offset`, if it is the one asked for.
    fn at(&self, offset: usize, r: Ref) -> Option<Object> {
        let length = |l: Ref| self.get(l).as_int();
        let mut lx = Lexer::new(&self.data, offset);
        let (got, obj) = lx.indirect(&length)?;
        (got.num == r.num).then_some(obj)
    }

    /// When the table pointed wrong: look the object up by scanning.
    fn by_scan(&self, r: Ref) -> Option<Object> {
        if self.repaired {
            return None;
        }
        let scan = self.scanned.borrow_mut().get_or_insert_with(|| Rc::new(xref::repair(&self.data).entries)).clone();
        match scan.get(&r.num)? {
            Entry::At { offset, .. } => self.at(*offset, r),
            _ => None,
        }
    }

    fn in_stream(&self, stm: u32, idx: u32, num: u32) -> Option<Object> {
        let objects = self.object_stream(stm)?;
        // By index, checked against the number; else by number.
        match objects.get(idx as usize) {
            Some((n, o)) if *n == num => Some(o.clone()),
            _ => objects.iter().find(|(n, _)| *n == num).map(|(_, o)| o.clone()),
        }
    }

    fn object_stream(&self, stm: u32) -> Option<Members> {
        if let Some(known) = self.object_streams.borrow().get(&stm) {
            return Some(known.clone());
        }
        let Object::Stream(s) = self.get(Ref { num: stm, generation: 0 }) else { return None };
        let n = s.dict.int("N")?.clamp(0, xref::MAX_OBJECTS as i64) as usize;
        let first = s.dict.int("First")?.max(0) as usize;
        let data = self.stream(&s).data;
        let mut header = Lexer::new(&data, 0);
        let mut pairs = Vec::with_capacity(n.min(65536));
        for _ in 0..n {
            let (Some(num), Some(off)) = (header.uint(), header.uint()) else { break };
            pairs.push((num.min(u32::MAX as u64) as u32, off as usize));
        }
        let mut objects = Vec::with_capacity(pairs.len());
        for (num, off) in pairs {
            let mut l = Lexer::new(&data, first.saturating_add(off));
            // Streams cannot live in object streams, so no lengths are needed.
            let obj = l.object(0, &|_| None).unwrap_or(Object::Null);
            objects.push((num, obj));
        }
        let objects = Rc::new(objects);
        self.object_streams.borrow_mut().insert(stm, objects.clone());
        Some(objects)
    }

    /// After a repair: objects inside object streams, which the scan cannot
    /// see, are entered from each /Type /ObjStm found.
    fn add_object_stream_members(&mut self) {
        let candidates: Vec<u32> = self.entries.iter().filter_map(|(n, e)| matches!(e, Entry::At { .. }).then_some(*n)).collect();
        let mut add = Vec::new();
        for num in candidates {
            let Object::Stream(s) = self.get(Ref { num, generation: 0 }) else { continue };
            if s.dict.name("Type") != Some(b"ObjStm") {
                continue;
            }
            if let Some(objects) = self.object_stream(num) {
                for (idx, (n, _)) in objects.iter().enumerate() {
                    add.push((*n, Entry::InStream { stm: num, idx: idx as u32 }));
                }
            }
        }
        for (n, e) in add {
            self.entries.entry(n).or_insert(e);
        }
    }

    /// Follows references to a direct object (Null where a chain breaks or
    /// runs too long).
    pub fn resolve(&self, o: &Object) -> Object {
        let mut cur = o.clone();
        for _ in 0..MAX_CHAIN {
            match cur {
                Object::Ref(r) => cur = self.get(r),
                other => return other,
            }
        }
        Object::Null
    }

    /// A dictionary entry, references followed.
    pub fn get_in(&self, d: &Dict, key: &str) -> Object {
        d.get(key).map_or(Object::Null, |o| self.resolve(o))
    }

    /// A stream's bytes with its filters undone (its /Filter and
    /// /DecodeParms may be references).
    pub fn stream(&self, s: &Stream) -> Decoded {
        let Some(raw) = self.data.get(s.start..s.start.saturating_add(s.len).min(self.data.len())) else {
            return Decoded { data: Vec::new(), image: None, ok: false };
        };
        let mut dict = s.dict.clone();
        for key in ["Filter", "DecodeParms", "F", "DP"] {
            if let Some(Object::Ref(r)) = dict.get(key).cloned() {
                let v = self.get(r);
                dict.0.retain(|(k, _)| k != key.as_bytes());
                dict.0.push((key.as_bytes().to_vec(), v));
            }
        }
        // Parameters inside an array may be references too.
        if let Some(Object::Array(a)) = dict.get("DecodeParms").cloned() {
            let resolved: Vec<Object> = a.iter().map(|o| self.resolve(o)).collect();
            dict.0.retain(|(k, _)| k != b"DecodeParms");
            dict.0.push((b"DecodeParms".to_vec(), Object::Array(resolved)));
        }
        let (names, parms) = filter::filters_of(&dict);
        let decrypted;
        let raw = match (&self.keys, s.owner) {
            (Some(k), Some(owner)) if self.stream_encrypted(&dict, &names, &parms, k) => {
                decrypted = k.decrypt(k.streams, owner.num, owner.generation, raw);
                &decrypted[..]
            }
            _ => raw,
        };
        filter::decode(raw, &names, &parms)
    }

    /// Whether a stream's data is encrypted: not the cross-reference
    /// streams, not metadata when /EncryptMetadata is false, not a stream
    /// whose first filter is /Crypt with the Identity filter.
    fn stream_encrypted(&self, dict: &Dict, names: &[Vec<u8>], parms: &[Option<Dict>], k: &crypt::Keys) -> bool {
        if k.streams == crypt::Method::None || dict.name("Type") == Some(b"XRef") {
            return false;
        }
        if dict.name("Type") == Some(b"Metadata") && !k.encrypt_metadata {
            return false;
        }
        if names.first().map(Vec::as_slice) == Some(b"Crypt") {
            let named = parms.first().cloned().flatten().and_then(|p| p.name("Name").map(<[u8]>::to_vec));
            return named.as_deref().is_some_and(|n| n != b"Identity");
        }
        true
    }

    /// The pages in order, each with what it inherits (Resources,
    /// MediaBox, CropBox, Rotate).
    pub fn pages(&self) -> Vec<Page> {
        let mut out = Vec::new();
        let Some(cat) = self.catalog() else { return out };
        let Some(root) = cat.get("Pages") else { return out };
        let mut seen = HashSet::new();
        self.walk(root, &Inherited::default(), 0, &mut seen, &mut out);
        out
    }

    fn walk(&self, node: &Object, inh: &Inherited, depth: usize, seen: &mut HashSet<u32>, out: &mut Vec<Page>) {
        if depth > MAX_TREE_DEPTH || out.len() >= MAX_PAGES {
            return;
        }
        let reference = node.as_ref();
        if let Some(r) = reference
            && !seen.insert(r.num)
        {
            return;
        }
        let Some(d) = self.resolve(node).as_dict().cloned() else { return };
        let inh = Inherited {
            resources: self.get_in(&d, "Resources").as_dict().cloned().or_else(|| inh.resources.clone()),
            media_box: rect(&self.get_in(&d, "MediaBox"), self).or(inh.media_box),
            crop_box: rect(&self.get_in(&d, "CropBox"), self).or(inh.crop_box),
            rotate: self.get_in(&d, "Rotate").as_int().or(inh.rotate),
        };
        let kids = self.get_in(&d, "Kids");
        // A node with kids is a tree node, whatever its /Type says (files lie).
        if let Some(kids) = kids.as_array().filter(|k| !k.is_empty() || d.name("Type") == Some(b"Pages")) {
            for k in kids {
                self.walk(k, &inh, depth + 1, seen, out);
            }
            return;
        }
        let rotate = inh.rotate.unwrap_or(0).rem_euclid(360);
        out.push(Page {
            reference,
            dict: d,
            resources: inh.resources,
            media_box: inh.media_box.unwrap_or([0.0, 0.0, 612.0, 792.0]),
            crop_box: inh.crop_box,
            rotate: (rotate - rotate % 90) as u16,
        });
    }

    /// A page's content streams, decoded and joined with newlines.
    pub fn contents(&self, page: &Page) -> Vec<u8> {
        let mut out = Vec::new();
        let c = self.get_in(&page.dict, "Contents");
        let parts: Vec<Object> = match c {
            Object::Array(a) => a.iter().map(|o| self.resolve(o)).collect(),
            other => vec![other],
        };
        for p in parts {
            if let Object::Stream(s) = p {
                out.extend_from_slice(&self.stream(&s).data);
                out.push(b'\n');
            }
        }
        out
    }
}

/// Every string inside `o` decrypted with object `r`'s key (nesting capped
/// as the lexer caps it).
fn decrypt_strings(o: Object, k: &crypt::Keys, r: Ref, depth: usize) -> Object {
    if depth > lexer::MAX_DEPTH || k.strings == crypt::Method::None {
        return o;
    }
    let dict = |d: Dict| Dict(d.0.into_iter().map(|(key, v)| (key, decrypt_strings(v, k, r, depth + 1))).collect());
    match o {
        Object::String(s) => Object::String(k.decrypt(k.strings, r.num, r.generation, &s)),
        Object::Array(a) => Object::Array(a.into_iter().map(|v| decrypt_strings(v, k, r, depth + 1)).collect()),
        Object::Dict(d) => Object::Dict(dict(d)),
        Object::Stream(mut s) => {
            s.dict = dict(s.dict);
            Object::Stream(s)
        }
        other => other,
    }
}

/// An object stream's objects: (object number, object), in order.
type Members = Rc<Vec<(u32, Object)>>;

#[derive(Default, Clone)]
struct Inherited {
    resources: Option<Dict>,
    media_box: Option<[f64; 4]>,
    crop_box: Option<[f64; 4]>,
    rotate: Option<i64>,
}

/// A rectangle array, normalized (x0 < x1, y0 < y1); None unless it has
/// four numbers and some area.
fn rect(o: &Object, doc: &Document) -> Option<[f64; 4]> {
    let a = o.as_array()?;
    if a.len() < 4 {
        return None;
    }
    let v: Vec<f64> = a.iter().take(4).filter_map(|x| doc.resolve(x).as_f64()).collect();
    if v.len() < 4 {
        return None;
    }
    let r = [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])];
    (r[2] - r[0] > 0.0 && r[3] - r[1] > 0.0).then_some(r)
}
