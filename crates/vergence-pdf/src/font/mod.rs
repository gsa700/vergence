//! Fonts (ISO 32000-2 §9): what a code in a string draws, and how far it
//! moves the pen. Simple fonts (one byte a code, an encoding of glyph
//! names, /Widths) and composite ones (Type 0: a CMap from codes to CIDs,
//! a CIDFont's /W and its CID-to-glyph map).
//!
//! Glyph programs, all ours: embedded TrueType (truetype.rs), CFF (cff.rs:
//! bare in FontFile3 or inside OpenType), Type 1 (type1.rs); Type 3 glyphs
//! are content streams the renderer runs ([`Font::type3`]). Every font can
//! say what text a code stands for ([`Font::text`]: ToUnicode, else the
//! glyph name), for selection and search. A font that is not embedded is
//! drawn with a substitute from the system (metric-compatible ones first),
//! stretched to the PDF's widths so lines keep their length.

pub mod cff;
pub mod encoding;
pub mod truetype;
pub mod type1;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use vergence_raster::{Path, Transform};

use crate::object::{Dict, Object};
use crate::Document;
use cff::Cff;
use truetype::TrueType;
use type1::Type1;

/// A glyph's outline in text space units of one em (y up), shared.
pub type Glyph = Rc<Path>;

enum Program {
    TrueType(TrueType),
    Cff(Box<Cff>),
    Type1(Box<Type1>),
    None,
}

/// A Type 3 font: glyphs as content streams in glyph space.
pub struct Type3 {
    pub procs: Dict,
    pub matrix: Transform,
    pub resources: Option<Dict>,
}

/// How a composite font's bytes become CIDs.
enum CMap {
    /// Two bytes, the CID itself (Identity-H, Identity-V).
    Identity,
    /// Code space ranges (byte length, low, high) and CID ranges.
    Ranges { spaces: Vec<(usize, u32, u32)>, cids: Vec<(u32, u32, u32)> },
}

enum Kind {
    Simple { names: Vec<Option<String>>, widths: Vec<Option<f64>>, symbolic: bool },
    Composite { cmap: CMap, cid_to_gid: Option<Vec<u16>>, widths: Vec<(u32, u32, f64)>, default: f64 },
}

pub struct Font {
    kind: Kind,
    program: Program,
    /// A stand-in for a font not embedded, and whether to stretch it to /Widths.
    substitute: Option<Rc<TrueType>>,
    missing_width: f64,
    cache: RefCell<HashMap<u32, Option<Glyph>>>,
    /// ToUnicode: code to text.
    to_unicode: HashMap<u32, String>,
    type3: Option<Type3>,
}

impl Font {
    /// From a font dictionary.
    pub fn load(doc: &Document, d: &Dict) -> Font {
        let subtype = d.name("Subtype").unwrap_or(b"").to_vec();
        let mut font = if subtype == b"Type0" { Self::composite(doc, d) } else { Self::simple(doc, d, subtype == b"Type3") };
        if let Object::Stream(s) = doc.get_in(d, "ToUnicode") {
            font.to_unicode = parse_to_unicode(&doc.stream(&s).data);
        }
        font
    }

    fn simple(doc: &Document, d: &Dict, type3: bool) -> Font {
        let descriptor = doc.get_in(d, "FontDescriptor").as_dict().cloned().unwrap_or_default();
        let flags = descriptor.int("Flags").unwrap_or(0);
        let symbolic = flags & 4 != 0;
        let program = if type3 { Program::None } else { embedded(doc, &descriptor) };
        let base = base_name(d);
        // Type 3: glyph space to text space by its own matrix; widths in glyph space.
        let type3 = type3.then(|| {
            let v: Vec<f64> = doc.get_in(d, "FontMatrix").as_array().map(|a| a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect()).unwrap_or_default();
            let matrix = if v.len() == 6 && v.iter().all(|x| x.is_finite()) { Transform::new(v[0], v[1], v[2], v[3], v[4], v[5]) } else { Transform::new(0.001, 0.0, 0.0, 0.001, 0.0, 0.0) };
            Type3 { procs: doc.get_in(d, "CharProcs").as_dict().cloned().unwrap_or_default(), matrix, resources: doc.get_in(d, "Resources").as_dict().cloned() }
        });
        let width_scale = type3.as_ref().map_or(0.001, |t| t.matrix.a);
        // The encoding: a base, then /Differences.
        let enc = doc.get_in(d, "Encoding");
        let (base_enc, diffs) = match &enc {
            Object::Name(n) => (Some(n.clone()), None),
            Object::Dict(e) => (e.name("BaseEncoding").map(<[u8]>::to_vec), doc.get_in(e, "Differences").as_array().map(<[Object]>::to_vec)),
            _ => (None, None),
        };
        let table: &[&str; 256] = match base_enc.as_deref() {
            Some(b"WinAnsiEncoding") => &encoding::WIN_ANSI,
            Some(b"MacRomanEncoding") => &encoding::MAC_ROMAN,
            Some(b"StandardEncoding") => &encoding::STANDARD,
            // No encoding named: a symbolic font's own, else Standard (TrueType: WinAnsi in practice).
            _ if matches!(program, Program::TrueType(_)) && !symbolic => &encoding::WIN_ANSI,
            _ => &encoding::STANDARD,
        };
        let mut names: Vec<Option<String>> = table.iter().map(|n| (!n.is_empty()).then(|| n.to_string())).collect();
        if symbolic && base_enc.is_none() {
            names = vec![None; 256];
        }
        // Symbol and ZapfDingbats not embedded: their own encodings, as
        // characters a substitute has (Greek, math signs, the common bullets).
        if matches!(program, Program::None) && type3.is_none() {
            let lower = base.to_ascii_lowercase();
            let special: Option<fn(u8) -> Option<char>> = if lower.starts_with("symbol") {
                Some(symbol_char)
            } else if lower.contains("dingbats") {
                Some(dingbat_char)
            } else {
                None
            };
            if let Some(map) = special {
                for (code, n) in names.iter_mut().enumerate() {
                    if let Some(c) = map(code as u8) {
                        *n = Some(format!("uni{:04X}", c as u32));
                    }
                }
            }
        }
        // A Type 1 font with no base encoding of the PDF's own starts from its built-in one.
        if let Program::Type1(t) = &program
            && base_enc.is_none()
        {
            names = t.encoding.clone();
        }
        if let Some(diffs) = diffs {
            let mut code = 0usize;
            for o in diffs.iter().take(4096) {
                match doc.resolve(o) {
                    Object::Int(i) => code = i.clamp(0, 255) as usize,
                    Object::Name(n) => {
                        if code < 256 {
                            names[code] = Some(String::from_utf8_lossy(&n).into_owned());
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
        let first = doc.get_in(d, "FirstChar").as_int().unwrap_or(0).clamp(0, 255) as usize;
        let mut widths = vec![None; 256];
        if let Object::Array(w) = doc.get_in(d, "Widths") {
            for (i, v) in w.iter().enumerate().take(256) {
                if let Some(x) = doc.resolve(v).as_f64()
                    && first + i < 256
                {
                    widths[first + i] = Some(x * width_scale);
                }
            }
        }
        let substitute = match program {
            Program::None if type3.is_none() => substitute(&base, flags),
            _ => None,
        };
        Font { kind: Kind::Simple { names, widths, symbolic }, program, substitute, missing_width: descriptor.get("MissingWidth").and_then(Object::as_f64).unwrap_or(0.0) / 1000.0, cache: RefCell::new(HashMap::new()), to_unicode: HashMap::new(), type3 }
    }

    fn composite(doc: &Document, d: &Dict) -> Font {
        let cmap = match doc.get_in(d, "Encoding") {
            Object::Name(_) => CMap::Identity,
            Object::Stream(s) => parse_cmap(&doc.stream(&s).data),
            _ => CMap::Identity,
        };
        let desc = match doc.get_in(d, "DescendantFonts") {
            Object::Array(a) => a.first().map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned()).unwrap_or_default(),
            _ => Dict::default(),
        };
        let descriptor = doc.get_in(&desc, "FontDescriptor").as_dict().cloned().unwrap_or_default();
        let program = embedded(doc, &descriptor);
        let cid_to_gid = match doc.get_in(&desc, "CIDToGIDMap") {
            Object::Stream(s) => Some(doc.stream(&s).data.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).take(1 << 20).collect()),
            _ => None,
        };
        let default = doc.get_in(&desc, "DW").as_f64().unwrap_or(1000.0) / 1000.0;
        let mut widths = Vec::new();
        if let Object::Array(w) = doc.get_in(&desc, "W") {
            let mut i = 0;
            while i < w.len() && widths.len() < 1 << 18 {
                let first = doc.resolve(&w[i]).as_int().unwrap_or(0).max(0) as u32;
                match w.get(i + 1).map(|o| doc.resolve(o)) {
                    Some(Object::Array(list)) => {
                        for (k, v) in list.iter().enumerate().take(1 << 16) {
                            if let Some(x) = doc.resolve(v).as_f64() {
                                widths.push((first + k as u32, first + k as u32, x / 1000.0));
                            }
                        }
                        i += 2;
                    }
                    Some(last) => {
                        let last = last.as_int().unwrap_or(0).max(0) as u32;
                        let x = w.get(i + 2).and_then(|o| doc.resolve(o).as_f64()).unwrap_or(1000.0);
                        widths.push((first, last.max(first), x / 1000.0));
                        i += 3;
                    }
                    None => break,
                }
            }
        }
        let flags = descriptor.int("Flags").unwrap_or(0);
        let substitute = match program {
            Program::None => substitute(&base_name(d), flags),
            _ => None,
        };
        Font { kind: Kind::Composite { cmap, cid_to_gid, widths, default }, program, substitute, missing_width: default, cache: RefCell::new(HashMap::new()), to_unicode: HashMap::new(), type3: None }
    }

    /// The codes in a string, each with whether it is the single-byte
    /// space (word spacing applies to it).
    pub fn codes(&self, s: &[u8]) -> Vec<(u32, bool)> {
        match &self.kind {
            Kind::Simple { .. } => s.iter().map(|b| (*b as u32, *b == 32)).collect(),
            Kind::Composite { cmap: CMap::Identity, .. } => s.chunks(2).map(|c| (if c.len() == 2 { (c[0] as u32) << 8 | c[1] as u32 } else { c[0] as u32 }, false)).collect(),
            Kind::Composite { cmap: CMap::Ranges { spaces, .. }, .. } => {
                let mut out = Vec::new();
                let mut i = 0;
                while i < s.len() {
                    // The shortest code-space range the bytes fall in.
                    let mut taken = 1;
                    for len in 1..=4 {
                        if i + len > s.len() {
                            break;
                        }
                        let v = s[i..i + len].iter().fold(0u32, |a, b| a << 8 | *b as u32);
                        if spaces.iter().any(|(l, lo, hi)| *l == len && v >= *lo && v <= *hi) {
                            taken = len;
                            break;
                        }
                    }
                    let v = s[i..i + taken].iter().fold(0u32, |a, b| a << 8 | *b as u32);
                    out.push((v, taken == 1 && v == 32));
                    i += taken;
                }
                out
            }
        }
    }

    fn cid(&self, code: u32) -> u32 {
        match &self.kind {
            Kind::Composite { cmap: CMap::Ranges { cids, .. }, .. } => cids.iter().find(|(lo, hi, _)| code >= *lo && code <= *hi).map_or(0, |(lo, _, c)| c + (code - lo)),
            _ => code,
        }
    }

    /// The pen's move for `code`, in ems.
    pub fn width(&self, code: u32) -> f64 {
        match &self.kind {
            Kind::Simple { widths, .. } => widths.get(code as usize).copied().flatten().or_else(|| self.program_width(code)).unwrap_or(self.missing_width),
            Kind::Composite { widths, default, .. } => {
                let cid = self.cid(code);
                widths.iter().rev().find(|(lo, hi, _)| cid >= *lo && cid <= *hi).map_or(*default, |w| w.2)
            }
        }
    }

    /// The width from the glyph program itself (a font with no /Widths).
    fn program_width(&self, code: u32) -> Option<f64> {
        let tt = match (&self.program, &self.substitute) {
            (Program::TrueType(t), _) => t,
            (_, Some(t)) => t.as_ref(),
            _ => return None,
        };
        let gid = self.gid(tt, code)?;
        Some(tt.advance(gid)? / tt.units_per_em)
    }

    /// The glyph for `code`, in ems, y up (None: nothing to draw).
    pub fn glyph(&self, code: u32) -> Option<Glyph> {
        if let Some(g) = self.cache.borrow().get(&code) {
            return g.clone();
        }
        let g = self.make_glyph(code).map(Rc::new);
        self.cache.borrow_mut().insert(code, g.clone());
        g
    }

    /// A Type 3 font's glyph for `code` (its content stream) and its matrix.
    pub fn type3(&self, code: u32) -> Option<(&Type3, Object)> {
        let t = self.type3.as_ref()?;
        let Kind::Simple { names, .. } = &self.kind else { return None };
        let name = names.get(code as usize).cloned().flatten()?;
        Some((t, t.procs.get(&name)?.clone()))
    }

    pub fn is_type3(&self) -> bool {
        self.type3.is_some()
    }

    /// The text `code` stands for: ToUnicode first, else the glyph name.
    pub fn text(&self, code: u32) -> Option<String> {
        if let Some(t) = self.to_unicode.get(&code) {
            return Some(t.clone());
        }
        match &self.kind {
            Kind::Simple { names, .. } => names.get(code as usize).cloned().flatten().and_then(|n| encoding::unicode_of(&n)).map(String::from),
            Kind::Composite { .. } => None,
        }
    }

    fn make_glyph(&self, code: u32) -> Option<Path> {
        if let Program::Type1(t) = &self.program {
            let Kind::Simple { names, .. } = &self.kind else { return None };
            let name = names.get(code as usize).cloned().flatten().filter(|n| t.has(n)).or_else(|| t.encoding.get(code as usize).cloned().flatten())?;
            let m = t.matrix;
            return Some(t.outline(&name)?.transformed(&Transform::new(m[0], m[1], m[2], m[3], m[4], m[5])));
        }
        if let Program::Cff(cff) = &self.program {
            let gid = self.cff_gid(cff, code)?;
            let m = cff.matrix;
            return Some(cff.outline(gid)?.transformed(&Transform::new(m[0], m[1], m[2], m[3], m[4], m[5])));
        }
        let (tt, stretch) = match (&self.program, &self.substitute) {
            (Program::TrueType(t), _) => (t, false),
            (Program::None, Some(t)) => (t.as_ref(), true),
            _ => return None,
        };
        let gid = self.gid(tt, code)?;
        let path = tt.outline(gid)?;
        let k = 1.0 / tt.units_per_em;
        // A substitute is stretched to the width the PDF gives the glyph.
        let sx = if stretch {
            let own = tt.advance(gid).map(|a| a * k).unwrap_or(0.0);
            let want = self.width(code);
            if own > 0.0 && want > 0.0 { (want / own).clamp(0.5, 2.0) } else { 1.0 }
        } else {
            1.0
        };
        Some(path.transformed(&Transform::scale(k * sx, k)))
    }

    /// The glyph for `code` in a CFF program: a name-keyed font by the
    /// encoding's glyph name, else its own encoding; a CID-keyed one by CID.
    fn cff_gid(&self, cff: &Cff, code: u32) -> Option<u16> {
        match &self.kind {
            Kind::Composite { .. } => cff.gid_of_cid(self.cid(code)),
            Kind::Simple { names, .. } => names.get(code as usize).cloned().flatten().and_then(|n| cff.gid_of_name(&n)).or_else(|| cff.gid_of_code(code)),
        }
    }

    /// The glyph index for `code` in `tt` (ISO 32000-2 §9.6.5.4 for simple
    /// TrueType fonts; the CID-to-GID map for composite ones).
    fn gid(&self, tt: &TrueType, code: u32) -> Option<u16> {
        match &self.kind {
            Kind::Composite { cid_to_gid, .. } => {
                let cid = self.cid(code);
                if self.substitute.is_some() && matches!(self.program, Program::None) {
                    // Not embedded: the CID has no meaning in the substitute; nothing sensible to draw.
                    return None;
                }
                match cid_to_gid {
                    Some(map) => map.get(cid as usize).copied().filter(|g| *g != 0),
                    None => u16::try_from(cid).ok(),
                }
            }
            Kind::Simple { names, symbolic, .. } => {
                let name = names.get(code as usize).cloned().flatten();
                let uni = name.as_deref().and_then(encoding::unicode_of);
                if *symbolic || uni.is_none() {
                    if tt.has_cmap(3, 0)
                        && let Some(g) = tt.lookup(3, 0, 0xF000 | code).or_else(|| tt.lookup(3, 0, code))
                    {
                        return Some(g);
                    }
                    if let Some(g) = tt.lookup(1, 0, code) {
                        return Some(g);
                    }
                }
                if let Some(u) = uni
                    && let Some(g) = tt.lookup_unicode(u as u32)
                {
                    return Some(g);
                }
                if let Some(g) = tt.lookup(1, 0, code).or_else(|| tt.lookup(3, 0, 0xF000 | code)).or_else(|| tt.lookup(3, 1, code)) {
                    return Some(g);
                }
                // A subset with no usable cmap: the code as the glyph index, as viewers do.
                (matches!(self.program, Program::TrueType(_)) && (code as usize) < tt.num_glyphs()).then_some(code as u16)
            }
        }
    }
}

/// The Symbol font's encoding, as far as text needs it: Greek letters on
/// the Latin ones, and the common signs.
fn symbol_char(code: u8) -> Option<char> {
    const UPPER: &str = "ΑΒΧΔΕΦΓΗΙϑΚΛΜΝΟΠΘΡΣΤΥςΩΞΨΖ";
    const LOWER: &str = "αβχδεφγηιϕκλμνοπθρστυϖωξψζ";
    let pick = |s: &str, i: u8| s.chars().nth(i as usize);
    match code {
        b'A'..=b'Z' => pick(UPPER, code - b'A'),
        b'a'..=b'z' => pick(LOWER, code - b'a'),
        0x20..=0x40 | 0x5B | 0x5D | 0x5F | 0x7B..=0x7D => {
            Some(match code {
                0x22 => '∀',
                0x24 => '∃',
                0x27 => '∋',
                0x2A => '∗',
                0x2D => '−',
                0x40 => '≅',
                0x5C => '∴',
                0x5E => '⊥',
                _ => code as char,
            })
        }
        0xA1 => Some('ϒ'),
        0xA2 => Some('′'),
        0xA3 => Some('≤'),
        0xA4 => Some('⁄'),
        0xA5 => Some('∞'),
        0xA7 => Some('♣'),
        0xA8 => Some('♦'),
        0xA9 => Some('♥'),
        0xAA => Some('♠'),
        0xAB => Some('↔'),
        0xAC => Some('←'),
        0xAD => Some('↑'),
        0xAE => Some('→'),
        0xAF => Some('↓'),
        0xB0 => Some('°'),
        0xB1 => Some('±'),
        0xB2 => Some('″'),
        0xB3 => Some('≥'),
        0xB4 => Some('×'),
        0xB5 => Some('∝'),
        0xB6 => Some('∂'),
        0xB7 => Some('•'),
        0xB8 => Some('÷'),
        0xB9 => Some('≠'),
        0xBA => Some('≡'),
        0xBB => Some('≈'),
        0xBC => Some('…'),
        0xC5 => Some('⊕'),
        0xC6 => Some('∅'),
        0xC7 => Some('∩'),
        0xC8 => Some('∪'),
        0xD1 => Some('∇'),
        0xD2 | 0xE2 => Some('®'),
        0xD3 | 0xE3 => Some('©'),
        0xD4 | 0xE4 => Some('™'),
        0xD5 => Some('∏'),
        0xD6 => Some('√'),
        0xD7 => Some('⋅'),
        0xD8 => Some('¬'),
        0xD9 => Some('∧'),
        0xDA => Some('∨'),
        0xDB => Some('⇔'),
        0xDC => Some('⇐'),
        0xDD => Some('⇑'),
        0xDE => Some('⇒'),
        0xDF => Some('⇓'),
        0xE0 => Some('◊'),
        0xE5 => Some('∑'),
        0xF2 => Some('∫'),
        _ => None,
    }
}

/// ZapfDingbats' common marks (bullets, boxes, checks, arrows); the rest
/// are drawn as nothing rather than as the wrong letter.
fn dingbat_char(code: u8) -> Option<char> {
    Some(match code {
        0x20 => ' ',
        0x33 => '✓',
        0x34 => '✔',
        0x35 => '✕',
        0x36 => '✖',
        0x37 => '✗',
        0x38 => '✘',
        0x48 => '★',
        0x6C => '●',
        0x6D => '❍',
        0x6E => '■',
        0x6F => '❏',
        0x70 => '❐',
        0x71 => '❑',
        0x72 => '❒',
        0x73 => '▲',
        0x74 => '▼',
        0x75 => '◆',
        0x76 => '❖',
        0x77 => '◗',
        0xA8 => '♣',
        0xA9 => '♦',
        0xAA => '♥',
        0xAB => '♠',
        0xD5 => '→',
        0xE0 => '➠',
        0xE8 => '➨',
        _ => return None,
    })
}

/// The font's /BaseFont without a subset tag ("ABCDEF+Helvetica").
fn base_name(d: &Dict) -> String {
    let n = d.name("BaseFont").map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
    match n.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => rest.to_string(),
        _ => n,
    }
}

fn embedded(doc: &Document, descriptor: &Dict) -> Program {
    if let Object::Stream(s) = doc.get_in(descriptor, "FontFile") {
        let length1 = doc.get_in(&s.dict, "Length1").as_int().unwrap_or(0).max(0) as usize;
        if let Some(t) = Type1::parse(&doc.stream(&s).data, length1) {
            return Program::Type1(Box::new(t));
        }
    }
    for key in ["FontFile2", "FontFile3"] {
        let Object::Stream(s) = doc.get_in(descriptor, key) else { continue };
        let data = doc.stream(&s).data;
        // FontFile3: bare CFF (/Type1C, /CIDFontType0C), or an OpenType font.
        if key == "FontFile3" && matches!(s.dict.name("Subtype"), Some(b"Type1C" | b"CIDFontType0C")) {
            if let Some(c) = Cff::parse(data) {
                return Program::Cff(Box::new(c));
            }
            continue;
        }
        if let Some(tt) = TrueType::parse(data) {
            if let Some((o, l)) = tt.cff {
                if let Some(c) = tt.data().get(o..o + l).and_then(|b| Cff::parse(b.to_vec())) {
                    return Program::Cff(Box::new(c));
                }
                continue;
            }
            return Program::TrueType(tt);
        }
    }
    Program::None
}

/// Reads every substitute face now: the sandboxed helper calls this before
/// it confines itself, after which it can open no file.
pub fn preload_substitutes() {
    for base in ["Helvetica", "Times-Roman", "Courier"] {
        for flags in [0, 64, 0x40000, 0x40000 | 64] {
            let _ = substitute(base, flags);
        }
    }
}

/// A system font standing in for one not embedded: by its name (Courier,
/// Times, Helvetica and the like) and flags (fixed pitch 1, serif 2,
/// italic 64), metric-compatible faces first, Vergence's own last.
fn substitute(base: &str, flags: i64) -> Option<Rc<TrueType>> {
    let lower = base.to_ascii_lowercase();
    let mono = flags & 1 != 0 || lower.contains("courier") || lower.contains("mono");
    let serif = !mono && (flags & 2 != 0 || lower.contains("times") || lower.contains("serif") || lower.contains("roman") || lower.contains("georgia") || lower.contains("garamond"));
    let bold = lower.contains("bold") || lower.contains("black") || lower.contains("heavy") || flags & 0x40000 != 0;
    let italic = lower.contains("italic") || lower.contains("oblique") || flags & 64 != 0;
    let families: &[&str] = if mono {
        &["Liberation Mono", "Cousine", "Courier New", "DejaVu Sans Mono", "JetBrains Mono", "monospace"]
    } else if serif {
        &["Liberation Serif", "Tinos", "Times New Roman", "DejaVu Serif", "Noto Serif", "serif"]
    } else {
        &["Liberation Sans", "Arimo", "Arial", "Helvetica", "DejaVu Sans", "Noto Sans", "Inter Variable", "sans-serif"]
    };
    let weight = if bold { 700 } else { 400 };
    /// Substitute font files read so far: (first family, weight, italic) to bytes.
    type Found = std::sync::Mutex<HashMap<(String, u16, bool), Option<Vec<u8>>>>;
    static CACHE: OnceLock<Found> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let key = (families[0].to_string(), weight, italic);
    let bytes = {
        let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
        c.entry(key)
            .or_insert_with(|| {
                FONT_DB.with(|db| {
                    families.iter().find_map(|f| {
                        let i = db.find(f, weight, italic)?;
                        let e = &db.faces[i];
                        // Only a file whose first face is the one found (collections aside).
                        (e.info.index == 0).then(|| std::fs::read(&e.path).ok()).flatten()
                    })
                })
            })
            .clone()
    }?;
    TrueType::parse(bytes).filter(|t| t.cff.is_none()).map(Rc::new)
}

thread_local! {
    /// The system's fonts, indexed once per thread (the database is not
    /// shared across threads; a page is drawn on one).
    static FONT_DB: vergence_text::db::FontDb = vergence_text::db::FontDb::system();
}

/// A ToUnicode CMap: codes to text (UTF-16BE), from bfchar and bfrange.
fn parse_to_unicode(data: &[u8]) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let code = |o: &Object| -> Option<u32> { o.as_bytes().filter(|b| !b.is_empty() && b.len() <= 4).map(|b| b.iter().fold(0u32, |a, x| a << 8 | *x as u32)) };
    let text = |b: &[u8]| -> String {
        let units: Vec<u16> = b.chunks(2).map(|c| if c.len() == 2 { u16::from_be_bytes([c[0], c[1]]) } else { c[0] as u16 }).collect();
        String::from_utf16_lossy(&units)
    };
    for op in crate::content::parse(data) {
        match op.name.as_slice() {
            b"endbfchar" => {
                for pair in op.args.chunks(2) {
                    if let [c, t] = pair
                        && let (Some(c), Some(t)) = (code(c), t.as_bytes())
                    {
                        out.insert(c, text(t));
                    }
                }
            }
            b"endbfrange" => {
                for t3 in op.args.chunks(3) {
                    let [lo, hi, dst] = t3 else { continue };
                    let (Some(lo), Some(hi)) = (code(lo), code(hi)) else { continue };
                    if hi < lo || hi - lo > 65_535 {
                        continue;
                    }
                    match dst {
                        // A start value whose last unit counts up.
                        Object::String(start) if !start.is_empty() => {
                            for (k, c) in (lo..=hi).enumerate() {
                                let mut b = start.clone();
                                let n = b.len();
                                let last = if n >= 2 { u16::from_be_bytes([b[n - 2], b[n - 1]]) } else { b[0] as u16 };
                                let v = last.wrapping_add(k as u16);
                                if n >= 2 {
                                    b[n - 2..].copy_from_slice(&v.to_be_bytes());
                                } else {
                                    b[0] = v as u8;
                                }
                                out.insert(c, text(&b));
                            }
                        }
                        Object::Array(list) => {
                            for (c, t) in (lo..=hi).zip(list.iter()) {
                                if let Some(t) = t.as_bytes() {
                                    out.insert(c, text(t));
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
    out
}

/// An embedded CMap's code space and CID ranges (begincodespacerange,
/// begincidrange, begincidchar); anything else is passed over.
fn parse_cmap(data: &[u8]) -> CMap {
    let ops = crate::content::parse(data);
    let mut spaces = Vec::new();
    let mut cids = Vec::new();
    let hex = |o: &Object| -> Option<(usize, u32)> {
        let b = o.as_bytes()?;
        (!b.is_empty() && b.len() <= 4).then(|| (b.len(), b.iter().fold(0u32, |a, x| a << 8 | *x as u32)))
    };
    for op in &ops {
        match op.name.as_slice() {
            b"endcodespacerange" => {
                for pair in op.args.chunks(2) {
                    if let [lo, hi] = pair
                        && let (Some((len, a)), Some((_, b))) = (hex(lo), hex(hi))
                    {
                        spaces.push((len, a, b));
                    }
                }
            }
            b"endcidrange" => {
                for t in op.args.chunks(3) {
                    if let [lo, hi, cid] = t
                        && let (Some((_, a)), Some((_, b)), Some(c)) = (hex(lo), hex(hi), cid.as_int())
                    {
                        cids.push((a, b, c.max(0) as u32));
                    }
                }
            }
            b"endcidchar" => {
                for t in op.args.chunks(2) {
                    if let [code, cid] = t
                        && let (Some((_, a)), Some(c)) = (hex(code), cid.as_int())
                    {
                        cids.push((a, a, c.max(0) as u32));
                    }
                }
            }
            _ => {}
        }
    }
    if spaces.is_empty() {
        return CMap::Identity;
    }
    CMap::Ranges { spaces, cids }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_embedded_cmap_maps_codes_to_cids() {
        let cmap = parse_cmap(b"1 begincodespacerange <00> <7F> <8000> <FFFF> endcodespacerange 2 begincidrange <20> <7E> 1 <8140> <817E> 633 endcidrange 1 begincidchar <8150> 999 endcidchar");
        let font = Font { kind: Kind::Composite { cmap, cid_to_gid: None, widths: vec![], default: 1.0 }, program: Program::None, substitute: None, missing_width: 1.0, cache: RefCell::new(HashMap::new()), to_unicode: HashMap::new(), type3: None };
        let codes = font.codes(b"A\x81\x41 ");
        assert_eq!(codes, vec![(0x41, false), (0x8141, false), (0x20, true)]);
        assert_eq!(font.cid(0x41), 34);
        assert_eq!(font.cid(0x8141), 634);
    }

    #[test]
    fn to_unicode_maps() {
        let m = parse_to_unicode(b"2 beginbfchar <03> <0041> <04> <00660069> endbfchar 1 beginbfrange <10> <12> <0061> <20> <21> [<0058> <0059>] endbfrange");
        assert_eq!(m.get(&3).map(String::as_str), Some("A"));
        assert_eq!(m.get(&4).map(String::as_str), Some("fi"), "a ligature is two letters");
        assert_eq!(m.get(&0x11).map(String::as_str), Some("b"));
        assert_eq!(m.get(&0x21).map(String::as_str), Some("Y"));
    }

    #[test]
    fn symbol_and_dingbats() {
        assert_eq!(symbol_char(0xA3), Some('≤'));
        assert_eq!(symbol_char(b'm'), Some('μ'));
        assert_eq!(symbol_char(b'W'), Some('Ω'));
        assert_eq!(dingbat_char(b'n'), Some('■'));
        assert_eq!(dingbat_char(b'l'), Some('●'));
        assert_eq!(dingbat_char(b'Z'), None);
    }

    #[test]
    fn subset_tags_come_off() {
        let d = Dict(vec![(b"BaseFont".to_vec(), Object::Name(b"ABCDEF+Helvetica-Bold".to_vec()))]);
        assert_eq!(base_name(&d), "Helvetica-Bold");
        let d = Dict(vec![(b"BaseFont".to_vec(), Object::Name(b"Abc+Thing".to_vec()))]);
        assert_eq!(base_name(&d), "Abc+Thing");
    }
}
