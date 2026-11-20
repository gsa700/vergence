//! Bytes into objects (ISO 32000-2 §7.2-7.3). **The file is untrusted**:
//! every read is bounds-checked, nesting is limited to [`MAX_DEPTH`], and a
//! string, name or array can be no larger than what is left of the file.

use crate::object::{Dict, Object, Ref, Stream};

/// How deep arrays and dictionaries may nest.
pub const MAX_DEPTH: usize = 64;

pub fn is_white(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

pub fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

fn is_regular(b: u8) -> bool {
    !is_white(b) && !is_delim(b)
}

/// A cursor over a file's bytes.
pub struct Lexer<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

/// A stream's /Length given as an indirect object: the document looks it up.
pub type LengthOf<'f> = &'f dyn Fn(Ref) -> Option<i64>;

impl<'a> Lexer<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> Lexer<'a> {
        Lexer { data, pos: pos.min(data.len()) }
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    /// Past white space and comments.
    pub fn skip_white(&mut self) {
        while let Some(b) = self.peek() {
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(c) = self.peek() {
                    if c == b'\n' || c == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// The next run of regular characters (a keyword or a number), without
    /// moving past it.
    pub fn peek_word(&mut self) -> &'a [u8] {
        self.skip_white();
        let start = self.pos;
        let mut end = start;
        while end < self.data.len() && is_regular(self.data[end]) {
            end += 1;
        }
        &self.data[start..end]
    }

    pub fn word(&mut self) -> &'a [u8] {
        let w = self.peek_word();
        self.pos += w.len();
        w
    }

    /// Takes `kw` if it comes next.
    pub fn keyword(&mut self, kw: &[u8]) -> bool {
        if self.peek_word() == kw {
            self.pos += kw.len();
            true
        } else {
            false
        }
    }

    /// An unsigned integer next (an object number, a generation), not
    /// moving if there is none.
    pub fn uint(&mut self) -> Option<u64> {
        let w = self.peek_word();
        if w.is_empty() || w.len() > 19 || !w.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.pos += w.len();
        std::str::from_utf8(w).ok()?.parse().ok()
    }

    /// One object, a reference `n g R` included. None on anything that is
    /// not an object (a keyword, the end, nesting too deep).
    pub fn object(&mut self, depth: usize, length: LengthOf) -> Option<Object> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.skip_white();
        let b = self.peek()?;
        match b {
            b'/' => {
                self.pos += 1;
                Some(Object::Name(self.name()))
            }
            b'(' => {
                self.pos += 1;
                Some(Object::String(self.literal()))
            }
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                let dict = self.dict_body(depth, length)?;
                // A stream follows its dictionary.
                let save = self.pos;
                if self.keyword(b"stream") {
                    return Some(self.stream(dict, length));
                }
                self.pos = save;
                Some(Object::Dict(dict))
            }
            b'<' => {
                self.pos += 1;
                Some(Object::String(self.hex()))
            }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_white();
                    match self.peek() {
                        None => return Some(Object::Array(items)),
                        Some(b']') => {
                            self.pos += 1;
                            return Some(Object::Array(items));
                        }
                        _ => {
                            let at = self.pos;
                            match self.object(depth + 1, length) {
                                Some(o) => items.push(o),
                                None => {
                                    // A stray keyword or byte: skip it, as readers do.
                                    if self.pos == at {
                                        self.pos += self.peek_word().len().max(1);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            b')' | b'>' | b']' | b'{' | b'}' => None,
            _ => {
                let w = self.peek_word();
                match w {
                    b"true" => {
                        self.pos += 4;
                        Some(Object::Bool(true))
                    }
                    b"false" => {
                        self.pos += 5;
                        Some(Object::Bool(false))
                    }
                    b"null" => {
                        self.pos += 4;
                        Some(Object::Null)
                    }
                    _ => self.number_or_ref(),
                }
            }
        }
    }

    fn number_or_ref(&mut self) -> Option<Object> {
        let start = self.pos;
        let w = self.word();
        let n = number(w)?;
        // `n g R`: two unsigned integers and R.
        if let Object::Int(num) = n
            && (0..=u32::MAX as i64).contains(&num)
            && w.iter().all(u8::is_ascii_digit)
        {
            let after = self.pos;
            if let Some(generation) = self.uint()
                && generation <= u16::MAX as u64
                && self.keyword(b"R")
            {
                return Some(Object::Ref(Ref { num: num as u32, generation: generation as u16 }));
            }
            self.pos = after;
        }
        let _ = start;
        Some(n)
    }

    fn dict_body(&mut self, depth: usize, length: LengthOf) -> Option<Dict> {
        let mut entries = Vec::new();
        loop {
            self.skip_white();
            match self.peek() {
                None => return Some(Dict(entries)),
                Some(b'>') => {
                    self.pos += 1;
                    if self.peek() == Some(b'>') {
                        self.pos += 1;
                    }
                    return Some(Dict(entries));
                }
                Some(b'/') => {
                    self.pos += 1;
                    let key = self.name();
                    let at = self.pos;
                    match self.object(depth + 1, length) {
                        Some(v) => entries.push((key, v)),
                        // A key with no value (seen in the wild): null.
                        None => {
                            entries.push((key, Object::Null));
                            if self.pos == at && self.peek().is_some_and(|b| b != b'/' && b != b'>') {
                                self.pos += self.peek_word().len().max(1);
                            }
                        }
                    }
                }
                Some(_) => {
                    // Junk where a key belongs: skip a word or a byte.
                    let at = self.pos;
                    if self.object(depth + 1, length).is_none() && self.pos == at {
                        self.pos += self.peek_word().len().max(1);
                    }
                }
            }
        }
    }

    /// After `stream`: the end of line, then the data. /Length is trusted
    /// only if `endstream` follows it; else the data runs to `endstream`.
    fn stream(&mut self, dict: Dict, length: LengthOf) -> Object {
        if self.data.get(self.pos) == Some(&b'\r') {
            self.pos += 1;
        }
        if self.data.get(self.pos) == Some(&b'\n') {
            self.pos += 1;
        }
        let start = self.pos;
        let left = self.data.len() - start;
        let told = match dict.get("Length") {
            Some(Object::Ref(r)) => length(*r),
            Some(o) => o.as_int(),
            None => None,
        };
        let fits = told.filter(|n| *n >= 0 && (*n as u64) <= left as u64).map(|n| n as usize).filter(|n| {
            let mut l = Lexer::new(self.data, start + n);
            l.keyword(b"endstream")
        });
        let len = match fits {
            Some(n) => n,
            None => {
                let end = find(&self.data[start..], b"endstream").unwrap_or(left);
                // The end of line before endstream is not data.
                let mut e = end;
                if e > 0 && self.data[start + e - 1] == b'\n' {
                    e -= 1;
                }
                if e > 0 && self.data[start + e - 1] == b'\r' {
                    e -= 1;
                }
                e
            }
        };
        self.pos = start + len;
        self.keyword(b"endstream");
        Object::Stream(Stream { dict, start, len, owner: None })
    }

    /// After `/`: the name's bytes, #xx decoded.
    fn name(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(b) = self.peek() {
            if !is_regular(b) {
                break;
            }
            self.pos += 1;
            if b == b'#'
                && let (Some(h), Some(l)) = (self.data.get(self.pos).and_then(|c| hexval(*c)), self.data.get(self.pos + 1).and_then(|c| hexval(*c)))
            {
                out.push(h << 4 | l);
                self.pos += 2;
                continue;
            }
            out.push(b);
        }
        out
    }

    /// After `(`: to the balancing `)`, escapes decoded.
    fn literal(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 1usize;
        while let Some(b) = self.peek() {
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(b);
                }
                b'\\' => {
                    let Some(e) = self.peek() else { break };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut v = (e - b'0') as u32;
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + (d - b'0') as u32;
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(v as u8);
                        }
                        // A backslash at the end of a line joins the lines.
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                // An end of line inside a string is a newline, whatever it was.
                b'\r' => {
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                _ => out.push(b),
            }
        }
        out
    }

    /// After `<`: hex digits to `>`, white space ignored, an odd last digit
    /// followed by 0.
    fn hex(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut half: Option<u8> = None;
        while let Some(b) = self.peek() {
            self.pos += 1;
            if b == b'>' {
                break;
            }
            let Some(v) = hexval(b) else { continue };
            match half.take() {
                Some(h) => out.push(h << 4 | v),
                None => half = Some(v),
            }
        }
        if let Some(h) = half {
            out.push(h << 4);
        }
        out
    }

    /// `n g obj <object> endobj` here: its number and the object.
    pub fn indirect(&mut self, length: LengthOf) -> Option<(Ref, Object)> {
        let num = self.uint()?;
        let generation = self.uint()?;
        if num > u32::MAX as u64 || generation > u16::MAX as u64 || !self.keyword(b"obj") {
            return None;
        }
        let obj = self.object(0, length).unwrap_or(Object::Null);
        self.keyword(b"endobj");
        Some((Ref { num: num as u32, generation: generation as u16 }, obj))
    }
}

pub fn hexval(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// A number token: an integer, or a real (`.5`, `-3.`, `+1.25`).
pub fn number(w: &[u8]) -> Option<Object> {
    if w.is_empty() || w.len() > 64 {
        return None;
    }
    let body = match w[0] {
        b'+' | b'-' => &w[1..],
        _ => w,
    };
    if body.is_empty() || !body.iter().all(|b| b.is_ascii_digit() || *b == b'.') || body.iter().filter(|b| **b == b'.').count() > 1 || body == b"." {
        return None;
    }
    let s = std::str::from_utf8(w).ok()?;
    if !body.contains(&b'.') {
        // Too large for 64 bits: a real (some writers emit huge integers).
        return Some(s.parse::<i64>().map(Object::Int).unwrap_or_else(|_| Object::Real(s.parse::<f64>().unwrap_or(0.0))));
    }
    let s = if s.ends_with('.') { format!("{s}0") } else { s.to_string() };
    let s = s.replacen("-.", "-0.", 1).replacen("+.", "0.", 1);
    let s = if s.starts_with('.') { format!("0{s}") } else { s };
    s.parse::<f64>().ok().filter(|v| v.is_finite()).map(Object::Real)
}

/// Where `needle` first occurs in `hay`.
pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Where `needle` last occurs in `hay`.
pub fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).rposition(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Option<Object> {
        Lexer::new(s.as_bytes(), 0).object(0, &|_| None)
    }

    #[test]
    fn simple_objects() {
        assert_eq!(parse("42"), Some(Object::Int(42)));
        assert_eq!(parse("-17"), Some(Object::Int(-17)));
        assert_eq!(parse(".5"), Some(Object::Real(0.5)));
        assert_eq!(parse("-.25"), Some(Object::Real(-0.25)));
        assert_eq!(parse("4."), Some(Object::Real(4.0)));
        assert_eq!(parse("true"), Some(Object::Bool(true)));
        assert_eq!(parse("null"), Some(Object::Null));
        assert_eq!(parse("/Type"), Some(Object::Name(b"Type".to_vec())));
        assert_eq!(parse("/A#20B"), Some(Object::Name(b"A B".to_vec())));
        assert_eq!(parse("12 0 R"), Some(Object::Ref(Ref { num: 12, generation: 0 })));
        assert_eq!(parse("12 0 obj"), Some(Object::Int(12)), "not a reference");
    }

    #[test]
    fn strings() {
        assert_eq!(parse("(a (b) c)"), Some(Object::String(b"a (b) c".to_vec())));
        assert_eq!(parse(r"(\(x\) \101\n\\)"), Some(Object::String(b"(x) A\n\\".to_vec())));
        assert_eq!(parse("(line\\\ncontinued)"), Some(Object::String(b"linecontinued".to_vec())));
        assert_eq!(parse("<48 65 6C6C 6F>"), Some(Object::String(b"Hello".to_vec())));
        assert_eq!(parse("<4>"), Some(Object::String(vec![0x40])));
    }

    #[test]
    fn arrays_and_dictionaries() {
        let o = parse("<< /Type /Page /Kids [1 0 R 2 0 R] /Box [0 0 612.0 792] /Nested << /A 1 >> >>").unwrap();
        let d = o.as_dict().unwrap();
        assert_eq!(d.name("Type"), Some(&b"Page"[..]));
        assert_eq!(d.get("Kids").unwrap().as_array().unwrap().len(), 2);
        assert_eq!(d.get("Box").unwrap().as_array().unwrap()[2].as_int(), Some(612));
        assert_eq!(d.get("Nested").unwrap().as_dict().unwrap().int("A"), Some(1));
    }

    #[test]
    fn streams_trust_length_only_when_it_lands_on_endstream() {
        let s = b"<< /Length 5 >>\nstream\nHello\nendstream";
        let Some(Object::Stream(st)) = Lexer::new(s, 0).object(0, &|_| None) else { panic!() };
        assert_eq!(&s[st.start..st.start + st.len], b"Hello");
        // A wrong length: the data runs to endstream.
        let s = b"<< /Length 99 >>\nstream\r\nHello world\r\nendstream";
        let Some(Object::Stream(st)) = Lexer::new(s, 0).object(0, &|_| None) else { panic!() };
        assert_eq!(&s[st.start..st.start + st.len], b"Hello world");
        // An indirect length, looked up.
        let s = b"<< /Length 7 0 R >>\nstream\nabc\nendstream";
        let Some(Object::Stream(st)) = Lexer::new(s, 0).object(0, &|r| (r.num == 7).then_some(3)) else { panic!() };
        assert_eq!(&s[st.start..st.start + st.len], b"abc");
    }

    #[test]
    fn deep_nesting_stops() {
        let deep = "[".repeat(10_000);
        assert!(matches!(parse(&deep), Some(Object::Array(_)) | None));
        let deep = "<<".repeat(10_000);
        let _ = parse(&deep);
    }
}
