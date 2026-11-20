//! CFF fonts (Adobe TN #5176) and their Type 2 charstrings (TN #5177),
//! our own: what most PDFs embed (FontFile3 /Type1C and /CIDFontType0C,
//! and OpenType fonts with a 'CFF ' table). INDEXes, DICTs, charsets,
//! encodings, private dictionaries and subroutines, CID-keyed fonts
//! (FDArray, FDSelect), and the charstring interpreter with flex, hint
//! masks, widths and `endchar`'s accented characters.
//!
//! Untrusted: every offset is checked; the argument stack, subroutine
//! depth and work per glyph are bounded.

use vergence_raster::Path;

use super::encoding::STANDARD;

const MAX_STACK: usize = 48;
const MAX_CALL_DEPTH: usize = 10;
const MAX_OPS: usize = 100_000;

/// CFF's standard strings 0-228 (the ISOAdobe charset's names); 229-390
/// (the expert set, rare in body text) are not kept.
const STD_STRINGS: [&str; 229] = [
    ".notdef", "space", "exclam", "quotedbl", "numbersign", "dollar", "percent", "ampersand", "quoteright", "parenleft", "parenright", "asterisk", "plus", "comma", "hyphen", "period", "slash", "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "colon", "semicolon", "less", "equal", "greater", "question", "at",
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z",
    "bracketleft", "backslash", "bracketright", "asciicircum", "underscore", "quoteleft",
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z",
    "braceleft", "bar", "braceright", "asciitilde", "exclamdown", "cent", "sterling", "fraction", "yen", "florin", "section", "currency", "quotesingle", "quotedblleft", "guillemotleft", "guilsinglleft", "guilsinglright", "fi", "fl", "endash", "dagger", "daggerdbl", "periodcentered", "paragraph", "bullet", "quotesinglbase", "quotedblbase", "quotedblright", "guillemotright", "ellipsis", "perthousand", "questiondown", "grave", "acute", "circumflex", "tilde", "macron", "breve", "dotaccent", "dieresis", "ring", "cedilla", "hungarumlaut", "ogonek", "caron", "emdash",
    "AE", "ordfeminine", "Lslash", "Oslash", "OE", "ordmasculine", "ae", "dotlessi", "lslash", "oslash", "oe", "germandbls",
    "onesuperior", "logicalnot", "mu", "trademark", "Eth", "onehalf", "plusminus", "Thorn", "onequarter", "divide", "brokenbar", "degree", "thorn", "threequarters", "twosuperior", "registered", "minus", "eth", "multiply", "threesuperior", "copyright",
    "Aacute", "Acircumflex", "Adieresis", "Agrave", "Aring", "Atilde", "Ccedilla", "Eacute", "Ecircumflex", "Edieresis", "Egrave", "Iacute", "Icircumflex", "Idieresis", "Igrave", "Ntilde", "Oacute", "Ocircumflex", "Odieresis", "Ograve", "Otilde", "Scaron", "Uacute", "Ucircumflex", "Udieresis", "Ugrave", "Yacute", "Ydieresis", "Zcaron",
    "aacute", "acircumflex", "adieresis", "agrave", "aring", "atilde", "ccedilla", "eacute", "ecircumflex", "edieresis", "egrave", "iacute", "icircumflex", "idieresis", "igrave", "ntilde", "oacute", "ocircumflex", "odieresis", "ograve", "otilde", "scaron", "uacute", "ucircumflex", "udieresis", "ugrave", "yacute", "ydieresis", "zcaron",
];

/// An INDEX's items as byte ranges into the font.
fn index(d: &[u8], at: usize) -> Option<(Vec<(usize, usize)>, usize)> {
    let count = u16::from_be_bytes([*d.get(at)?, *d.get(at + 1)?]) as usize;
    if count == 0 {
        return Some((Vec::new(), at + 2));
    }
    let off_size = *d.get(at + 2)? as usize;
    if !(1..=4).contains(&off_size) {
        return None;
    }
    let offsets_at = at + 3;
    let data_at = offsets_at + (count + 1) * off_size - 1;
    let off = |i: usize| -> Option<usize> { Some(d.get(offsets_at + i * off_size..offsets_at + (i + 1) * off_size)?.iter().fold(0usize, |a, b| a << 8 | *b as usize)) };
    let mut items = Vec::with_capacity(count);
    for i in 0..count {
        let (a, b) = (data_at + off(i)?, data_at + off(i + 1)?);
        if b < a || b > d.len() {
            return None;
        }
        items.push((a, b));
    }
    let end = data_at + off(count)?;
    Some((items, end))
}

/// A DICT's operators and their operands (escaped ones as 1200 + op).
fn dict(d: &[u8]) -> Vec<(u16, Vec<f64>)> {
    let mut out = Vec::new();
    let mut operands = Vec::new();
    let mut i = 0;
    while i < d.len() && out.len() < 256 {
        let b = d[i];
        match b {
            0..=21 => {
                let op = if b == 12 {
                    i += 1;
                    1200 + *d.get(i).unwrap_or(&0) as u16
                } else {
                    b as u16
                };
                out.push((op, std::mem::take(&mut operands)));
                i += 1;
            }
            28 => {
                let v = i16::from_be_bytes([*d.get(i + 1).unwrap_or(&0), *d.get(i + 2).unwrap_or(&0)]);
                operands.push(v as f64);
                i += 3;
            }
            29 => {
                let v = i32::from_be_bytes([*d.get(i + 1).unwrap_or(&0), *d.get(i + 2).unwrap_or(&0), *d.get(i + 3).unwrap_or(&0), *d.get(i + 4).unwrap_or(&0)]);
                operands.push(v as f64);
                i += 5;
            }
            30 => {
                // A real: nibbles until 0xf.
                let mut s = String::new();
                i += 1;
                'n: while let Some(&byte) = d.get(i) {
                    i += 1;
                    for nib in [byte >> 4, byte & 15] {
                        match nib {
                            0..=9 => s.push((b'0' + nib) as char),
                            0xa => s.push('.'),
                            0xb => s.push('E'),
                            0xc => s.push_str("E-"),
                            0xe => s.push('-'),
                            0xf => break 'n,
                            _ => {}
                        }
                    }
                }
                operands.push(s.parse().unwrap_or(0.0));
            }
            32..=246 => {
                operands.push(b as f64 - 139.0);
                i += 1;
            }
            247..=250 => {
                operands.push((b as f64 - 247.0) * 256.0 + *d.get(i + 1).unwrap_or(&0) as f64 + 108.0);
                i += 2;
            }
            251..=254 => {
                operands.push(-(b as f64 - 251.0) * 256.0 - *d.get(i + 1).unwrap_or(&0) as f64 - 108.0);
                i += 2;
            }
            _ => i += 1,
        }
        if operands.len() > MAX_STACK {
            operands.clear();
        }
    }
    out
}

fn get(dict: &[(u16, Vec<f64>)], op: u16) -> Option<&Vec<f64>> {
    dict.iter().find(|(o, _)| *o == op).map(|(_, v)| v)
}

struct Private {
    subrs: Vec<(usize, usize)>,
}

pub struct Cff {
    data: Vec<u8>,
    charstrings: Vec<(usize, usize)>,
    global: Vec<(usize, usize)>,
    strings: Vec<(usize, usize)>,
    /// One private for a name-keyed font; one per FD for a CID font.
    privates: Vec<Private>,
    fd_select: Vec<u8>,
    /// For each glyph, its SID (name-keyed) or CID (CID-keyed).
    charset: Vec<u16>,
    pub cid: bool,
    /// The font's own encoding: code to glyph index.
    encoding: [u16; 256],
    pub matrix: [f64; 6],
}

impl Cff {
    pub fn parse(data: Vec<u8>) -> Option<Cff> {
        let d = &data;
        let hdr = *d.get(2)? as usize;
        let (_names, next) = index(d, hdr)?;
        let (tops, next) = index(d, next)?;
        let (strings, next) = index(d, next)?;
        let (global, _) = index(d, next)?;
        let &(ta, tb) = tops.first()?;
        let top = dict(&d[ta..tb]);
        let cs_at = *get(&top, 17)?.first()? as usize;
        let (charstrings, _) = index(d, cs_at)?;
        let n = charstrings.len();
        if n == 0 {
            return None;
        }
        let cid = get(&top, 1230).is_some();
        let read_private = |dict_ops: &[(u16, Vec<f64>)]| -> Private {
            let mut subrs = Vec::new();
            if let Some(p) = get(dict_ops, 18)
                && p.len() >= 2
            {
                let (size, off) = (p[0].max(0.0) as usize, p[1].max(0.0) as usize);
                if let Some(pd) = d.get(off..off.saturating_add(size)) {
                    let pdict = dict(pd);
                    if let Some(s) = get(&pdict, 19).and_then(|v| v.first()) {
                        subrs = index(d, off + s.max(0.0) as usize).map(|i| i.0).unwrap_or_default();
                    }
                }
            }
            Private { subrs }
        };
        let mut privates = Vec::new();
        let mut fd_select = Vec::new();
        if cid {
            if let Some(fda) = get(&top, 1236).and_then(|v| v.first())
                && let Some((fds, _)) = index(d, *fda as usize)
            {
                for (a, b) in fds.iter().take(256) {
                    privates.push(read_private(&dict(&d[*a..*b])));
                }
            }
            if let Some(fs) = get(&top, 1237).and_then(|v| v.first()) {
                let at = *fs as usize;
                match d.get(at) {
                    Some(0) => fd_select = d.get(at + 1..at + 1 + n).map(<[u8]>::to_vec).unwrap_or_default(),
                    Some(3) => {
                        let ranges = u16::from_be_bytes([*d.get(at + 1)?, *d.get(at + 2)?]) as usize;
                        fd_select = vec![0; n];
                        for r in 0..ranges.min(n) {
                            let p = at + 3 + 3 * r;
                            let first = u16::from_be_bytes([*d.get(p)?, *d.get(p + 1)?]) as usize;
                            let fd = *d.get(p + 2)?;
                            let next = d.get(p + 3..p + 5).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize).unwrap_or(n);
                            if first < next.min(n) {
                                fd_select[first..next.min(n)].fill(fd);
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else {
            privates.push(read_private(&top));
        }
        // Charset: glyph 0 is .notdef; then SIDs (or CIDs).
        let mut charset = vec![0u16; n];
        let cs = get(&top, 15).and_then(|v| v.first()).copied().unwrap_or(0.0) as usize;
        if cs > 2 {
            let mut g = 1;
            match d.get(cs) {
                Some(0) => {
                    while g < n {
                        let p = cs + 1 + 2 * (g - 1);
                        let Some(b) = d.get(p..p + 2) else { break };
                        charset[g] = u16::from_be_bytes([b[0], b[1]]);
                        g += 1;
                    }
                }
                Some(f @ (1 | 2)) => {
                    let mut p = cs + 1;
                    while g < n {
                        let Some(b) = d.get(p..p + 2) else { break };
                        let first = u16::from_be_bytes([b[0], b[1]]);
                        let left = if *f == 1 { *d.get(p + 2)? as usize } else { u16::from_be_bytes([*d.get(p + 2)?, *d.get(p + 3)?]) as usize };
                        p += if *f == 1 { 3 } else { 4 };
                        for k in 0..=left {
                            if g >= n {
                                break;
                            }
                            charset[g] = first.saturating_add(k as u16);
                            g += 1;
                        }
                    }
                }
                _ => {}
            }
        } else {
            // The ISOAdobe charset: glyph i is SID i.
            for (g, c) in charset.iter_mut().enumerate() {
                *c = g as u16;
            }
        }
        // The font's own encoding (name-keyed fonts).
        let mut cff = Cff { data: Vec::new(), charstrings, global, strings, privates, fd_select, charset, cid, encoding: [0; 256], matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0] };
        if let Some(m) = get(&top, 1207)
            && m.len() == 6
            && m.iter().all(|v| v.is_finite())
        {
            cff.matrix = [m[0], m[1], m[2], m[3], m[4], m[5]];
        }
        if !cid {
            let enc = get(&top, 16).and_then(|v| v.first()).copied().unwrap_or(0.0) as usize;
            if enc > 1 {
                cff.read_encoding(d, enc);
            } else {
                // Standard (0): each code's StandardEncoding name, found in the charset.
                for (code, name) in STANDARD.iter().enumerate() {
                    if !name.is_empty() {
                        cff.encoding[code] = cff.gid_of_name_in(d, name).unwrap_or(0);
                    }
                }
            }
        }
        cff.data = data;
        Some(cff)
    }

    fn read_encoding(&mut self, d: &[u8], at: usize) {
        let Some(&format) = d.get(at) else { return };
        let mut p = at + 1;
        match format & 0x7f {
            0 => {
                let n = *d.get(p).unwrap_or(&0) as usize;
                for g in 1..=n {
                    if let Some(&code) = d.get(p + g) {
                        self.encoding[code as usize] = g as u16;
                    }
                }
                p += 1 + n;
            }
            1 => {
                let ranges = *d.get(p).unwrap_or(&0) as usize;
                let mut g = 1u16;
                for r in 0..ranges {
                    let (Some(&first), Some(&left)) = (d.get(p + 1 + 2 * r), d.get(p + 2 + 2 * r)) else { break };
                    for k in 0..=left as usize {
                        let code = first as usize + k;
                        if code < 256 {
                            self.encoding[code] = g;
                        }
                        g = g.saturating_add(1);
                    }
                }
                p += 1 + 2 * ranges;
            }
            _ => return,
        }
        if format & 0x80 != 0 {
            // Supplements: (code, SID) pairs.
            let n = *d.get(p).unwrap_or(&0) as usize;
            for s in 0..n {
                let q = p + 1 + 3 * s;
                let (Some(&code), Some(b)) = (d.get(q), d.get(q + 1..q + 3)) else { break };
                let sid = u16::from_be_bytes([b[0], b[1]]);
                if let Some(g) = self.charset.iter().position(|c| *c == sid) {
                    self.encoding[code as usize] = g as u16;
                }
            }
        }
    }

    fn name_of_sid<'a>(&'a self, d: &'a [u8], sid: u16) -> Option<&'a str> {
        let s = sid as usize;
        if s < STD_STRINGS.len() {
            return Some(STD_STRINGS[s]);
        }
        let &(a, b) = self.strings.get(s.checked_sub(391)?)?;
        std::str::from_utf8(d.get(a..b)?).ok()
    }

    fn gid_of_name_in(&self, d: &[u8], name: &str) -> Option<u16> {
        if self.cid {
            return None;
        }
        self.charset.iter().position(|sid| self.name_of_sid(d, *sid) == Some(name)).map(|g| g as u16)
    }

    /// The glyph named `name` (name-keyed fonts).
    pub fn gid_of_name(&self, name: &str) -> Option<u16> {
        self.gid_of_name_in(&self.data, name).filter(|g| *g != 0)
    }

    /// The glyph for `code` in the font's own encoding.
    pub fn gid_of_code(&self, code: u32) -> Option<u16> {
        self.encoding.get(code as usize).copied().filter(|g| *g != 0)
    }

    /// The glyph for a CID (CID-keyed fonts; otherwise the CID is the glyph).
    pub fn gid_of_cid(&self, cid: u32) -> Option<u16> {
        if !self.cid {
            return u16::try_from(cid).ok().filter(|g| (*g as usize) < self.charstrings.len());
        }
        self.charset.iter().position(|c| *c as u32 == cid).map(|g| g as u16)
    }

    pub fn num_glyphs(&self) -> usize {
        self.charstrings.len()
    }

    /// The glyph's outline in font units (before the font matrix), y up.
    pub fn outline(&self, gid: u16) -> Option<Path> {
        let fd = if self.cid { self.fd_select.get(gid as usize).copied().unwrap_or(0) as usize } else { 0 };
        let local = self.privates.get(fd).map(|p| p.subrs.as_slice()).unwrap_or(&[]);
        let &(a, b) = self.charstrings.get(gid as usize)?;
        let mut run = Run { cff: self, local, path: Path::new(), stack: Vec::new(), x: 0.0, y: 0.0, stems: 0, open: false, width_seen: false, ops: 0, done: false };
        run.exec(&self.data[a..b], 0);
        if run.open {
            run.path.close();
        }
        (!run.path.is_empty()).then_some(run.path)
    }

    /// The outline of a StandardEncoding code (for `endchar`'s accents).
    fn outline_of_standard(&self, code: usize) -> Option<Path> {
        let name = STANDARD.get(code).filter(|n| !n.is_empty())?;
        self.outline(self.gid_of_name(name)?)
    }
}

struct Run<'a> {
    cff: &'a Cff,
    local: &'a [(usize, usize)],
    path: Path,
    stack: Vec<f64>,
    x: f64,
    y: f64,
    stems: usize,
    open: bool,
    width_seen: bool,
    ops: usize,
    done: bool,
}

fn bias(n: usize) -> i64 {
    if n < 1240 {
        107
    } else if n < 33900 {
        1131
    } else {
        32768
    }
}

impl Run<'_> {
    fn move_to(&mut self, dx: f64, dy: f64) {
        if self.open {
            self.path.close();
        }
        self.x += dx;
        self.y += dy;
        self.path.move_to(self.x, self.y);
        self.open = true;
    }

    fn line_to(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
        self.path.line_to(self.x, self.y);
    }

    fn curve(&mut self, d: [f64; 6]) {
        let (x1, y1) = (self.x + d[0], self.y + d[1]);
        let (x2, y2) = (x1 + d[2], y1 + d[3]);
        self.x = x2 + d[4];
        self.y = y2 + d[5];
        self.path.cubic_to(x1, y1, x2, y2, self.x, self.y);
    }

    /// The first stack-clearing operator may carry the glyph's width as an
    /// extra first argument: there is one when the count's parity is not
    /// the operator's own (`expect_even`: stems and rmoveto take pairs;
    /// hmoveto and vmoveto take one).
    fn take_width(&mut self, expect_even: bool) {
        if !self.width_seen {
            self.width_seen = true;
            let odd = self.stack.len() % 2 == 1;
            if odd == expect_even && !self.stack.is_empty() {
                self.stack.remove(0);
            }
        }
    }

    fn exec(&mut self, code: &[u8], depth: usize) {
        if depth > MAX_CALL_DEPTH {
            return;
        }
        let mut i = 0;
        while i < code.len() && !self.done {
            self.ops += 1;
            if self.ops > MAX_OPS {
                self.done = true;
                return;
            }
            let b = code[i];
            i += 1;
            if b >= 32 || b == 28 {
                let v = match b {
                    28 => {
                        let v = i16::from_be_bytes([*code.get(i).unwrap_or(&0), *code.get(i + 1).unwrap_or(&0)]) as f64;
                        i += 2;
                        v
                    }
                    32..=246 => b as f64 - 139.0,
                    247..=250 => {
                        let v = (b as f64 - 247.0) * 256.0 + *code.get(i).unwrap_or(&0) as f64 + 108.0;
                        i += 1;
                        v
                    }
                    251..=254 => {
                        let v = -(b as f64 - 251.0) * 256.0 - *code.get(i).unwrap_or(&0) as f64 - 108.0;
                        i += 1;
                        v
                    }
                    _ => {
                        // 255: a 16.16 fixed number.
                        let v = i32::from_be_bytes([*code.get(i).unwrap_or(&0), *code.get(i + 1).unwrap_or(&0), *code.get(i + 2).unwrap_or(&0), *code.get(i + 3).unwrap_or(&0)]) as f64 / 65536.0;
                        i += 4;
                        v
                    }
                };
                if self.stack.len() < MAX_STACK {
                    self.stack.push(v);
                }
                continue;
            }
            match b {
                // Stems: their count decides hintmask's length.
                1 | 3 | 18 | 23 => {
                    self.take_width(true);
                    self.stems += self.stack.len() / 2;
                    self.stack.clear();
                }
                19 | 20 => {
                    self.take_width(true);
                    self.stems += self.stack.len() / 2;
                    self.stack.clear();
                    i += self.stems.div_ceil(8);
                }
                21 => {
                    self.take_width(true);
                    let (dx, dy) = (self.arg(0), self.arg(1));
                    self.move_to(dx, dy);
                    self.stack.clear();
                }
                22 => {
                    self.take_width(false);
                    let dx = self.arg(0);
                    self.move_to(dx, 0.0);
                    self.stack.clear();
                }
                4 => {
                    self.take_width(false);
                    let dy = self.arg(0);
                    self.move_to(0.0, dy);
                    self.stack.clear();
                }
                5 => {
                    let st = std::mem::take(&mut self.stack);
                    for p in st.as_chunks::<2>().0 {
                        self.line_to(p[0], p[1]);
                    }
                }
                6 | 7 => {
                    let st = std::mem::take(&mut self.stack);
                    let mut horizontal = b == 6;
                    for v in st {
                        if horizontal { self.line_to(v, 0.0) } else { self.line_to(0.0, v) }
                        horizontal = !horizontal;
                    }
                }
                8 => {
                    let st = std::mem::take(&mut self.stack);
                    for c in st.as_chunks::<6>().0 {
                        self.curve(*c);
                    }
                }
                24 => {
                    // rcurveline: curves, then a line.
                    let st = std::mem::take(&mut self.stack);
                    let curves = st.len().saturating_sub(2) / 6;
                    for c in st.as_chunks::<6>().0.iter().take(curves) {
                        self.curve(*c);
                    }
                    if st.len() >= curves * 6 + 2 {
                        self.line_to(st[curves * 6], st[curves * 6 + 1]);
                    }
                }
                25 => {
                    // rlinecurve: lines, then a curve.
                    let st = std::mem::take(&mut self.stack);
                    let lines = st.len().saturating_sub(6) / 2;
                    for p in st.as_chunks::<2>().0.iter().take(lines) {
                        self.line_to(p[0], p[1]);
                    }
                    let c = &st[lines * 2..];
                    if c.len() >= 6 {
                        self.curve([c[0], c[1], c[2], c[3], c[4], c[5]]);
                    }
                }
                26 | 27 => {
                    // vvcurveto / hhcurveto: an optional first delta, then fours.
                    let mut st = std::mem::take(&mut self.stack);
                    let first = if st.len() % 4 == 1 { st.remove(0) } else { 0.0 };
                    for (k, c) in st.as_chunks::<4>().0.iter().enumerate() {
                        let d = if k == 0 { first } else { 0.0 };
                        if b == 26 { self.curve([d, c[0], c[1], c[2], 0.0, c[3]]) } else { self.curve([c[0], d, c[1], c[2], c[3], 0.0]) }
                    }
                }
                30 | 31 => {
                    // vhcurveto / hvcurveto: alternating, a last odd argument ends one.
                    let st = std::mem::take(&mut self.stack);
                    let mut horizontal = b == 31;
                    let mut k = 0;
                    while k + 4 <= st.len() {
                        let last = if st.len() - (k + 4) == 1 { st[k + 4] } else { 0.0 };
                        if horizontal {
                            self.curve([st[k], 0.0, st[k + 1], st[k + 2], last, st[k + 3]]);
                        } else {
                            self.curve([0.0, st[k], st[k + 1], st[k + 2], st[k + 3], last]);
                        }
                        horizontal = !horizontal;
                        k += 4;
                    }
                }
                10 | 29 => {
                    let Some(n) = self.stack.pop() else { continue };
                    let cff = self.cff;
                    let subrs: &[(usize, usize)] = if b == 10 { self.local } else { &cff.global };
                    let idx = n as i64 + bias(subrs.len());
                    if let Some(&(a, e)) = usize::try_from(idx).ok().and_then(|i| subrs.get(i)) {
                        self.exec(&cff.data[a..e], depth + 1);
                    }
                }
                11 => return,
                14 => {
                    // endchar; four more arguments: an accented character (seac).
                    if !self.width_seen {
                        self.width_seen = true;
                        if self.stack.len() == 1 || self.stack.len() == 5 {
                            self.stack.remove(0);
                        }
                    }
                    if self.stack.len() == 4 && depth == 0 {
                        let (adx, ady, bchar, achar) = (self.stack[0], self.stack[1], self.stack[2] as usize, self.stack[3] as usize);
                        if self.open {
                            self.path.close();
                            self.open = false;
                        }
                        if let Some(base) = self.cff.outline_of_standard(bchar) {
                            self.path.append(&base);
                        }
                        if let Some(accent) = self.cff.outline_of_standard(achar) {
                            self.path.append(&accent.transformed(&vergence_raster::Transform::translate(adx, ady)));
                        }
                    }
                    self.done = true;
                    return;
                }
                12 => {
                    let esc = *code.get(i).unwrap_or(&0);
                    i += 1;
                    let st = std::mem::take(&mut self.stack);
                    let a = |k: usize| st.get(k).copied().unwrap_or(0.0);
                    match esc {
                        35 => {
                            self.curve([a(0), a(1), a(2), a(3), a(4), a(5)]);
                            self.curve([a(6), a(7), a(8), a(9), a(10), a(11)]);
                        }
                        34 => {
                            // hflex
                            let y = self.y;
                            self.curve([a(0), 0.0, a(1), a(2), a(3), 0.0]);
                            self.curve([a(4), 0.0, a(5), y - self.y, a(6), 0.0]);
                        }
                        36 => {
                            // hflex1: the second curve ends back at the starting height.
                            self.curve([a(0), a(1), a(2), a(3), a(4), 0.0]);
                            self.curve([a(5), 0.0, a(6), a(7), a(8), -(a(1) + a(3) + a(7))]);
                        }
                        37 => {
                            // flex1: the last point's other coordinate from the sum.
                            let (sx, sy) = (a(0) + a(2) + a(4) + a(6) + a(8), a(1) + a(3) + a(5) + a(7) + a(9));
                            let (lx, ly) = if sx.abs() > sy.abs() { (a(10), -sy) } else { (-sx, a(10)) };
                            self.curve([a(0), a(1), a(2), a(3), a(4), a(5)]);
                            self.curve([a(6), a(7), a(8), a(9), lx, ly]);
                        }
                        _ => {}
                    }
                }
                _ => self.stack.clear(),
            }
        }
    }

    fn arg(&self, k: usize) -> f64 {
        self.stack.get(k).copied().unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny CFF made by hand: one font, .notdef and a square named "A".
    fn tiny() -> Vec<u8> {
        // Charstrings: .notdef = endchar; A = 100 0 rmoveto 0 500 rlineto 500 0 rlineto 0 -500 rlineto endchar.
        let notdef = vec![14u8];
        let num = |v: i32| -> Vec<u8> {
            if (-107..=107).contains(&v) {
                vec![(v + 139) as u8]
            } else if (108..=1131).contains(&v) {
                let w = v - 108;
                vec![(w / 256 + 247) as u8, (w % 256) as u8]
            } else {
                let w = -v - 108;
                vec![(w / 256 + 251) as u8, (w % 256) as u8]
            }
        };
        let mut a = Vec::new();
        for (op_args, op) in [(vec![100, 0], 21u8), (vec![0, 500], 5), (vec![500, 0], 5), (vec![0, -500], 5)] {
            for v in op_args {
                a.extend(num(v));
            }
            a.push(op);
        }
        a.push(14);
        let idx = |items: &[Vec<u8>]| -> Vec<u8> {
            // An empty INDEX is its count alone.
            if items.is_empty() {
                return vec![0, 0];
            }
            let mut out = (items.len() as u16).to_be_bytes().to_vec();
            out.push(1);
            let mut off = 1u8;
            out.push(off);
            for it in items {
                off += it.len() as u8;
                out.push(off);
            }
            for it in items {
                out.extend(it);
            }
            out
        };
        let name = idx(&[b"Tiny".to_vec()]);
        let strings = idx(&[]);
        let global = idx(&[]);
        let charstrings = idx(&[notdef, a]);
        // Charset format 0: glyph 1 is SID 34 ("A").
        let charset = vec![0u8, 0, 34];
        // Top DICT: charset (15) and CharStrings (17) offsets as 2-byte integers (28).
        let header = vec![1u8, 0, 4, 1];
        let top_len = 3 + 1 + 3 + 1;
        let top_index_len = 2 + 1 + 2 + top_len;
        let base = header.len() + name.len() + top_index_len + strings.len() + global.len();
        let cs_at = base + charset.len();
        let top = [vec![28, (base >> 8) as u8, base as u8, 15], vec![28, (cs_at >> 8) as u8, cs_at as u8, 17]].concat();
        let tops = idx(&[top]);
        assert_eq!(tops.len(), top_index_len);
        [header, name, tops, strings, global, charset, charstrings].concat()
    }

    #[test]
    fn a_hand_made_font() {
        let cff = Cff::parse(tiny()).expect("parses");
        assert_eq!(cff.num_glyphs(), 2);
        assert_eq!(cff.gid_of_name("A"), Some(1));
        assert_eq!(cff.gid_of_code(0x41), Some(1), "the Standard encoding finds A");
        let path = cff.outline(1).expect("an outline");
        let lines = path.flatten(&vergence_raster::Transform::IDENTITY, 0.1);
        assert_eq!(lines.len(), 1);
        let xs: Vec<f64> = lines[0].points.iter().map(|p| p.x).collect();
        assert!(xs.contains(&100.0) && xs.contains(&600.0), "{xs:?}");
        assert!(cff.outline(0).is_none(), ".notdef draws nothing");
    }

    #[test]
    fn damage_never_panics() {
        let base = tiny();
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..3000 {
            let mut f = base.clone();
            for _ in 0..4 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let k = x as usize % f.len();
                f[k] = (x >> 32) as u8;
            }
            if let Some(c) = Cff::parse(f) {
                for g in 0..c.num_glyphs().min(4) {
                    let _ = c.outline(g as u16);
                }
            }
        }
    }
}
