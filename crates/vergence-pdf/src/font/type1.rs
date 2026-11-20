//! Type 1 fonts (Adobe's "Type 1 Font Format"), our own: what older PDFs
//! and TeX embed as FontFile. The clear part's /Encoding and /FontMatrix;
//! the eexec-encrypted part (binary or hex) with /lenIV, /Subrs and
//! /CharStrings; each charstring decrypted and run by the Type 1 charstring
//! interpreter, flex and hint replacement (the standard othersubrs 0 to 3)
//! and `seac` accents included.
//!
//! Untrusted: sizes are bounded, the stack, subroutine depth and work per
//! glyph too.

use std::collections::HashMap;

use vergence_raster::{Path, Transform};

use super::encoding::STANDARD;

const MAX_STACK: usize = 48;
const MAX_CALL_DEPTH: usize = 10;
const MAX_OPS: usize = 100_000;
const MAX_GLYPHS: usize = 65_536;

pub struct Type1 {
    charstrings: HashMap<String, Vec<u8>>,
    subrs: Vec<Vec<u8>>,
    /// The font's own encoding: code to glyph name.
    pub encoding: Vec<Option<String>>,
    pub matrix: [f64; 6],
}

fn decrypt(data: &[u8], key: u16, skip: usize) -> Vec<u8> {
    let mut r = key;
    let mut out = Vec::with_capacity(data.len());
    for &c in data {
        let p = c ^ (r >> 8) as u8;
        r = (c as u16).wrapping_add(r).wrapping_mul(52845).wrapping_add(22719);
        out.push(p);
    }
    out.into_iter().skip(skip).collect()
}

/// A PostScript token after `at`: (token, its end).
fn token(d: &[u8], mut at: usize) -> Option<(&[u8], usize)> {
    while at < d.len() && (d[at].is_ascii_whitespace() || d[at] == 0) {
        at += 1;
    }
    let start = at;
    if at >= d.len() {
        return None;
    }
    if d[at] == b'/' {
        at += 1;
    }
    while at < d.len() && !d[at].is_ascii_whitespace() && !matches!(d[at], b'/' | b'[' | b']' | b'{' | b'}' | b'(' | b')') {
        at += 1;
    }
    if at == start {
        at += 1;
    }
    Some((&d[start..at], at))
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

impl Type1 {
    /// From a FontFile stream's bytes and its /Length1 (the clear part).
    pub fn parse(data: &[u8], length1: usize) -> Option<Type1> {
        // The clear part ends where eexec starts; /Length1 is often wrong.
        let clear_end = find(data, b"eexec", 0).map(|p| p + 5).unwrap_or(length1.min(data.len()));
        let clear = &data[..clear_end.min(data.len())];
        let mut font = Type1 { charstrings: HashMap::new(), subrs: Vec::new(), encoding: vec![None; 256], matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0] };
        if let Some(e) = find(clear, b"/Encoding", 0) {
            if token(clear, e + 9).is_some_and(|(t, _)| t == b"StandardEncoding") {
                font.encoding = STANDARD.iter().map(|n| (!n.is_empty()).then(|| n.to_string())).collect();
            } else {
                // dup <code> /<name> put
                let mut at = e;
                let stop = find(clear, b"readonly def", e).or_else(|| find(clear, b"def", e + 9)).unwrap_or(clear.len());
                while let Some(d) = find(clear, b"dup", at).filter(|d| *d < stop) {
                    let (Some((code, a)), Some((name, b))) = (token(clear, d + 3), token(clear, d + 3).and_then(|(_, a)| token(clear, a))) else { break };
                    let _ = a;
                    if let (Ok(code), Some(name)) = (std::str::from_utf8(code).unwrap_or("").parse::<usize>(), name.strip_prefix(b"/"))
                        && code < 256
                    {
                        font.encoding[code] = Some(String::from_utf8_lossy(name).into_owned());
                    }
                    at = b;
                }
            }
        }
        if let Some(m) = find(clear, b"/FontMatrix", 0)
            && let Some(open) = find(clear, b"[", m)
            && let Some(close) = find(clear, b"]", open)
        {
            let v: Vec<f64> = String::from_utf8_lossy(&clear[open + 1..close]).split_whitespace().filter_map(|t| t.parse().ok()).collect();
            if v.len() == 6 && v.iter().all(|x| x.is_finite()) {
                font.matrix = [v[0], v[1], v[2], v[3], v[4], v[5]];
            }
        }
        // The encrypted part: binary, or hex digits.
        let mut enc_start = clear_end;
        while enc_start < data.len() && data[enc_start].is_ascii_whitespace() {
            enc_start += 1;
        }
        let rest = data.get(enc_start..)?;
        let hex = rest.len() >= 4 && rest[..4].iter().all(u8::is_ascii_hexdigit);
        let raw: Vec<u8> = if hex {
            let digits: Vec<u8> = rest.iter().filter(|b| b.is_ascii_hexdigit()).map(|b| (*b as char).to_digit(16).unwrap_or(0) as u8).collect();
            digits.as_chunks::<2>().0.iter().map(|p| p[0] << 4 | p[1]).collect()
        } else {
            rest.to_vec()
        };
        let private = decrypt(&raw, 55665, 4);
        let len_iv = find(&private, b"/lenIV", 0).and_then(|p| token(&private, p + 6)).and_then(|(t, _)| std::str::from_utf8(t).ok()?.parse::<i64>().ok()).unwrap_or(4);
        let chars = |data: &[u8]| -> Vec<u8> { if len_iv < 0 { data.to_vec() } else { decrypt(data, 4330, len_iv as usize) } };
        // Subrs: dup <i> <n> RD <n bytes> NP
        if let Some(s) = find(&private, b"/Subrs", 0) {
            let mut at = s + 6;
            let end = find(&private, b"/CharStrings", s).unwrap_or(private.len());
            while let Some(d) = find(&private, b"dup", at).filter(|d| *d < end) {
                let Some((idx, a)) = token(&private, d + 3) else { break };
                let Some((len, b)) = token(&private, a) else { break };
                let Some((_rd, c)) = token(&private, b) else { break };
                let (Ok(idx), Ok(len)) = (std::str::from_utf8(idx).unwrap_or("").parse::<usize>(), std::str::from_utf8(len).unwrap_or("").parse::<usize>()) else {
                    at = d + 3;
                    continue;
                };
                let start = c + 1;
                let Some(bytes) = private.get(start..start + len) else { break };
                if idx < 4096 {
                    if font.subrs.len() <= idx {
                        font.subrs.resize(idx + 1, Vec::new());
                    }
                    font.subrs[idx] = chars(bytes);
                }
                at = start + len;
            }
        }
        // CharStrings: /<name> <n> RD <n bytes> ND
        let cs = find(&private, b"/CharStrings", 0)?;
        let mut at = cs + 12;
        while font.charstrings.len() < MAX_GLYPHS {
            let Some(slash) = find(&private, b"/", at) else { break };
            let Some((name, a)) = token(&private, slash) else { break };
            let Some((len, b)) = token(&private, a) else { break };
            let Ok(len) = std::str::from_utf8(len).unwrap_or("").parse::<usize>() else {
                at = slash + 1;
                continue;
            };
            let Some((_rd, c)) = token(&private, b) else { break };
            let start = c + 1;
            let Some(bytes) = private.get(start..start + len) else { break };
            font.charstrings.insert(String::from_utf8_lossy(&name[1..]).into_owned(), chars(bytes));
            at = start + len;
        }
        (!font.charstrings.is_empty()).then_some(font)
    }

    pub fn has(&self, name: &str) -> bool {
        self.charstrings.contains_key(name)
    }

    /// The glyph's outline in glyph units (before the font matrix), y up.
    pub fn outline(&self, name: &str) -> Option<Path> {
        self.outline_at(name, 0)
    }

    fn outline_at(&self, name: &str, depth: usize) -> Option<Path> {
        let cs = self.charstrings.get(name)?;
        let mut run = Run { font: self, path: Path::new(), stack: Vec::new(), ps: Vec::new(), x: 0.0, y: 0.0, sbx: 0.0, open: false, flex: None, ops: 0, done: false, depth };
        run.exec(cs, 0);
        if run.open {
            run.path.close();
        }
        (!run.path.is_empty()).then_some(run.path)
    }
}

struct Run<'a> {
    font: &'a Type1,
    path: Path,
    stack: Vec<f64>,
    /// The PostScript operand stack othersubrs hand results back on.
    ps: Vec<f64>,
    x: f64,
    y: f64,
    sbx: f64,
    open: bool,
    /// Points gathered while a flex is under way.
    flex: Option<Vec<(f64, f64)>>,
    ops: usize,
    done: bool,
    /// Nesting through seac.
    depth: usize,
}

impl Run<'_> {
    fn move_by(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
        if let Some(f) = &mut self.flex {
            f.push((self.x, self.y));
            return;
        }
        if self.open {
            self.path.close();
        }
        self.path.move_to(self.x, self.y);
        self.open = true;
    }

    fn line_by(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
        self.path.line_to(self.x, self.y);
    }

    fn curve_by(&mut self, d: [f64; 6]) {
        let (x1, y1) = (self.x + d[0], self.y + d[1]);
        let (x2, y2) = (x1 + d[2], y1 + d[3]);
        self.x = x2 + d[4];
        self.y = y2 + d[5];
        self.path.cubic_to(x1, y1, x2, y2, self.x, self.y);
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
            let v = code[i];
            i += 1;
            if v >= 32 {
                let n = match v {
                    32..=246 => v as f64 - 139.0,
                    247..=250 => {
                        let n = (v as f64 - 247.0) * 256.0 + *code.get(i).unwrap_or(&0) as f64 + 108.0;
                        i += 1;
                        n
                    }
                    251..=254 => {
                        let n = -(v as f64 - 251.0) * 256.0 - *code.get(i).unwrap_or(&0) as f64 - 108.0;
                        i += 1;
                        n
                    }
                    _ => {
                        let n = i32::from_be_bytes([*code.get(i).unwrap_or(&0), *code.get(i + 1).unwrap_or(&0), *code.get(i + 2).unwrap_or(&0), *code.get(i + 3).unwrap_or(&0)]) as f64;
                        i += 4;
                        n
                    }
                };
                if self.stack.len() < MAX_STACK {
                    self.stack.push(n);
                }
                continue;
            }
            let st = std::mem::take(&mut self.stack);
            let a = |k: usize| st.get(k).copied().unwrap_or(0.0);
            match v {
                13 => {
                    // hsbw: side bearing x, width.
                    self.sbx = a(0);
                    self.x = a(0);
                    self.y = 0.0;
                }
                21 => self.move_by(a(0), a(1)),
                22 => self.move_by(a(0), 0.0),
                4 => self.move_by(0.0, a(0)),
                5 => self.line_by(a(0), a(1)),
                6 => self.line_by(a(0), 0.0),
                7 => self.line_by(0.0, a(0)),
                8 => self.curve_by([a(0), a(1), a(2), a(3), a(4), a(5)]),
                30 => self.curve_by([0.0, a(0), a(1), a(2), a(3), 0.0]),
                31 => self.curve_by([a(0), 0.0, a(1), a(2), 0.0, a(3)]),
                9 => {
                    if self.open {
                        self.path.close();
                        self.open = false;
                    }
                }
                10 => {
                    let n = a(st.len().saturating_sub(1));
                    // The arguments below the subr number stay for it.
                    self.stack = st[..st.len().saturating_sub(1)].to_vec();
                    let font = self.font;
                    if let Some(sub) = usize::try_from(n as i64).ok().and_then(|k| font.subrs.get(k)) {
                        self.exec(sub, depth + 1);
                    }
                    continue;
                }
                11 => {
                    self.stack = st;
                    return;
                }
                14 => {
                    self.done = true;
                    return;
                }
                1 | 3 => {}
                12 => {
                    let esc = *code.get(i).unwrap_or(&0);
                    i += 1;
                    match esc {
                        7 => {
                            // sbw
                            self.sbx = a(0);
                            self.x = a(0);
                            self.y = a(1);
                        }
                        12 => {
                            // div: leaves the quotient.
                            let mut rest = st.clone();
                            let (b, c) = (rest.pop().unwrap_or(1.0), rest.pop().unwrap_or(0.0));
                            rest.push(if b == 0.0 { 0.0 } else { c / b });
                            self.stack = rest;
                            continue;
                        }
                        16 => {
                            // callothersubr: othersubr# and its argument count on top.
                            let mut rest = st.clone();
                            let (Some(other), Some(count)) = (rest.pop(), rest.pop()) else { continue };
                            let count = (count.max(0.0) as usize).min(rest.len());
                            let args: Vec<f64> = rest.split_off(rest.len() - count);
                            match other as i64 {
                                1 => self.flex = Some(Vec::new()),
                                0 => {
                                    // Flex ends: seven points gathered (the first, the reference
                                    // point, is not drawn), two curves; the end point is handed back.
                                    if let Some(p) = self.flex.take()
                                        && p.len() >= 7
                                    {
                                        self.path.cubic_to(p[1].0, p[1].1, p[2].0, p[2].1, p[3].0, p[3].1);
                                        self.path.cubic_to(p[4].0, p[4].1, p[5].0, p[5].1, p[6].0, p[6].1);
                                        self.x = p[6].0;
                                        self.y = p[6].1;
                                    }
                                    self.ps = vec![self.y, self.x];
                                }
                                // Hint replacement: hands back the subr number for `pop`.
                                3 => self.ps = vec![3.0],
                                _ => self.ps = args.into_iter().rev().collect(),
                            }
                            self.stack = rest;
                            continue;
                        }
                        17 => {
                            // pop: a value back from the othersubr.
                            let mut rest = st.clone();
                            rest.push(self.ps.pop().unwrap_or(0.0));
                            self.stack = rest;
                            continue;
                        }
                        33 => {
                            // setcurrentpoint
                            self.x = a(0);
                            self.y = a(1);
                        }
                        6 => {
                            // seac: asb adx ady bchar achar.
                            if self.depth == 0 {
                                let (asb, adx, ady, bchar, achar) = (a(0), a(1), a(2), a(3) as usize, a(4) as usize);
                                let font = self.font;
                                let name = |c: usize| STANDARD.get(c).filter(|n| !n.is_empty()).copied();
                                if let Some(base) = name(bchar).and_then(|n| font.outline_at(n, 1)) {
                                    self.path.append(&base);
                                }
                                if let Some(accent) = name(achar).and_then(|n| font.outline_at(n, 1)) {
                                    self.path.append(&accent.transformed(&Transform::translate(adx - asb + self.sbx, ady)));
                                }
                            }
                            self.done = true;
                            return;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encrypt(plain: &[u8], key: u16) -> Vec<u8> {
        let mut r = key;
        plain
            .iter()
            .map(|&p| {
                let c = p ^ (r >> 8) as u8;
                r = (c as u16).wrapping_add(r).wrapping_mul(52845).wrapping_add(22719);
                c
            })
            .collect()
    }

    /// A Type 1 font made by hand: a square "A", encoded at 65, with a subr.
    fn tiny() -> Vec<u8> {
        let num = |v: i32| -> Vec<u8> { if (-107..=107).contains(&v) { vec![(v + 139) as u8] } else { let w = v - 108; vec![(w / 256 + 247) as u8, (w % 256) as u8] } };
        // hsbw 0 600; rmoveto 100 0; rlineto 0 500 (through subr 0); rlineto 400 0; rlineto 0 -500; closepath; endchar.
        let mut cs = Vec::new();
        cs.extend(num(0));
        cs.extend(num(600));
        cs.push(13);
        cs.extend(num(100));
        cs.extend(num(0));
        cs.push(21);
        cs.extend(num(0));
        cs.push(10);
        cs.extend(num(400));
        cs.extend(num(0));
        cs.push(5);
        cs.extend(num(0));
        cs.extend(num(-500));
        cs.push(5);
        cs.push(9);
        cs.push(14);
        let subr = [num(0), num(500), vec![5, 11]].concat();
        let charstring = |b: &[u8]| encrypt(&[vec![0u8; 4], b.to_vec()].concat(), 4330);
        let (cs_e, subr_e) = (charstring(&cs), charstring(&subr));
        let mut private = b"dup /Private 8 dict dup begin /lenIV 4 def /Subrs 1 array\n".to_vec();
        private.extend(format!("dup 0 {} RD ", subr_e.len()).as_bytes());
        private.extend(&subr_e);
        private.extend(b" NP\nND\n2 index /CharStrings 1 dict dup begin\n");
        private.extend(format!("/A {} RD ", cs_e.len()).as_bytes());
        private.extend(&cs_e);
        private.extend(b" ND\nend\n");
        let clear = b"%!PS-AdobeFont-1.0: Tiny\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /A put\nreadonly def\ncurrentfile eexec\n".to_vec();
        [clear, encrypt(&[b"abcd".to_vec(), private].concat(), 55665)].concat()
    }

    #[test]
    fn a_hand_made_font() {
        let f = tiny();
        let t = Type1::parse(&f, 0).expect("parses");
        assert_eq!(t.encoding[65].as_deref(), Some("A"));
        let lines = t.outline("A").expect("an outline").flatten(&Transform::IDENTITY, 0.1);
        let xs: Vec<f64> = lines[0].points.iter().map(|p| p.x).collect();
        let ys: Vec<f64> = lines[0].points.iter().map(|p| p.y).collect();
        assert!(xs.contains(&100.0) && xs.contains(&500.0) && ys.contains(&500.0), "{xs:?} {ys:?}");
    }

    #[test]
    fn hex_eexec_and_damage() {
        let f = tiny();
        let at = f.windows(5).position(|w| w == b"eexec").unwrap() + 6;
        let hex: String = f[at..].iter().map(|b| format!("{b:02x}")).collect();
        let mut h = f[..at].to_vec();
        h.extend(hex.as_bytes());
        assert!(Type1::parse(&h, 0).is_some_and(|t| t.has("A")), "the hex form reads the same");
        let mut x = 0x9e37_79b9u64;
        for _ in 0..2000 {
            let mut g = f.clone();
            for _ in 0..3 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let k = x as usize % g.len();
                g[k] = (x >> 24) as u8;
            }
            if let Some(t) = Type1::parse(&g, 0) {
                let _ = t.outline("A");
            }
        }
    }
}
