//! A small JSON reader for what servers answer (a music server's Subsonic
//! answers, AlbumWall milestone 2): values read into a tree, numbers as
//! f64 (and their text, for ids too large for one), strings with every
//! escape. Untrusted input: nesting is limited ([`MAX_DEPTH`]), a bad text
//! is an error, never a panic, and the tests give it random bytes.

pub const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    /// A member of an object, by its key (the first if several).
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// A string, or a number written as one (ids are either).
    pub fn text(&self) -> Option<String> {
        match self {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) if n.fract() == 0.0 && n.abs() < 1e15 => Some(format!("{}", *n as i64)),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The items of an array; a single value as a list of one (some
    /// servers send one item without the brackets); nothing as empty.
    pub fn items(&self) -> Vec<&Value> {
        match self {
            Value::Array(a) => a.iter().collect(),
            Value::Null => Vec::new(),
            v => vec![v],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not JSON this can read: {}", self.0)
    }
}

impl std::error::Error for Error {}

/// Reads one JSON value, the whole text (white space around it allowed).
pub fn parse(text: &[u8]) -> Result<Value, Error> {
    let mut p = Parser { b: text, i: 0 };
    p.space();
    let v = p.value(0)?;
    p.space();
    if p.i != p.b.len() {
        return Err(p.err("more after the value"));
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, what: &str) -> Error {
        Error(format!("{what} at byte {}", self.i))
    }

    fn space(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, word: &[u8]) -> bool {
        if self.b[self.i..].starts_with(word) {
            self.i += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return Err(self.err("nested too deep"));
        }
        match self.b.get(self.i) {
            None => Err(self.err("nothing")),
            Some(b'{') => {
                self.i += 1;
                let mut members = Vec::new();
                self.space();
                if self.eat(b"}") {
                    return Ok(Value::Object(members));
                }
                loop {
                    self.space();
                    if self.b.get(self.i) != Some(&b'"') {
                        return Err(self.err("a key"));
                    }
                    let k = self.string()?;
                    self.space();
                    if !self.eat(b":") {
                        return Err(self.err("a colon"));
                    }
                    self.space();
                    let v = self.value(depth + 1)?;
                    members.push((k, v));
                    self.space();
                    if self.eat(b",") {
                        continue;
                    }
                    if self.eat(b"}") {
                        return Ok(Value::Object(members));
                    }
                    return Err(self.err("a comma or the object's end"));
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.space();
                if self.eat(b"]") {
                    return Ok(Value::Array(items));
                }
                loop {
                    self.space();
                    items.push(self.value(depth + 1)?);
                    self.space();
                    if self.eat(b",") {
                        continue;
                    }
                    if self.eat(b"]") {
                        return Ok(Value::Array(items));
                    }
                    return Err(self.err("a comma or the array's end"));
                }
            }
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') if self.eat(b"true") => Ok(Value::Bool(true)),
            Some(b'f') if self.eat(b"false") => Ok(Value::Bool(false)),
            Some(b'n') if self.eat(b"null") => Ok(Value::Null),
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            Some(_) => Err(self.err("a value")),
        }
    }

    fn number(&mut self) -> Result<Value, Error> {
        let start = self.i;
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        let digits = |p: &mut Self| {
            let s = p.i;
            while p.i < p.b.len() && p.b[p.i].is_ascii_digit() {
                p.i += 1;
            }
            p.i > s
        };
        if !digits(self) {
            return Err(self.err("a number"));
        }
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            if !digits(self) {
                return Err(self.err("a number's fraction"));
            }
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !digits(self) {
                return Err(self.err("a number's exponent"));
            }
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| self.err("a number"))?;
        text.parse::<f64>().map(Value::Number).map_err(|_| self.err("a number"))
    }

    fn hex4(&mut self) -> Result<u32, Error> {
        let h = self.b.get(self.i..self.i + 4).ok_or_else(|| self.err("an escape"))?;
        let s = std::str::from_utf8(h).map_err(|_| self.err("an escape"))?;
        let n = u32::from_str_radix(s, 16).map_err(|_| self.err("an escape"))?;
        self.i += 4;
        Ok(n)
    }

    fn string(&mut self) -> Result<String, Error> {
        self.i += 1; // the opening quote
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.b.get(self.i) else { return Err(self.err("a string's end")) };
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.b.get(self.i) else { return Err(self.err("an escape")) };
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) {
                                // A surrogate pair, or a lone half (replaced).
                                if self.b[self.i..].starts_with(b"\\u") {
                                    self.i += 2;
                                    let low = self.hex4()?;
                                    cp = if (0xDC00..0xE000).contains(&low) {
                                        0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00)
                                    } else {
                                        0xFFFD
                                    };
                                } else {
                                    cp = 0xFFFD;
                                }
                            } else if (0xDC00..0xE000).contains(&cp) {
                                cp = 0xFFFD;
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return Err(self.err("an escape")),
                    }
                }
                c if c < 0x20 => return Err(self.err("a control character in a string")),
                c => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| self.err("a string that is not UTF-8"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_read() {
        let v = parse(
            br#" {"subsonic-response": {"status": "ok", "version": "1.16.1",
            "albumList2": {"album": [{"id": "al-1", "name": "Kill \u0027Em All", "year": 1983, "x": null, "t": true},
            {"id": 42, "name": "\u00e9\ud83c\udfb5"}]}}} "#,
        )
        .unwrap();
        let r = v.get("subsonic-response").unwrap();
        assert_eq!(r.get("status").and_then(Value::text).as_deref(), Some("ok"));
        let albums = r.get("albumList2").unwrap().get("album").unwrap().items();
        assert_eq!(albums.len(), 2);
        assert_eq!(albums[0].get("name").and_then(Value::text).as_deref(), Some("Kill 'Em All"));
        assert_eq!(albums[0].get("year").and_then(Value::number), Some(1983.0));
        assert_eq!(albums[0].get("t").and_then(Value::bool), Some(true));
        assert_eq!(albums[1].get("id").and_then(Value::text).as_deref(), Some("42"));
        assert_eq!(albums[1].get("name").and_then(Value::text).as_deref(), Some("é🎵"));
        // One item without brackets reads as a list of one.
        assert_eq!(parse(br#"{"a":{"x":1}}"#).unwrap().get("a").unwrap().items().len(), 1);
        assert_eq!(parse(b"-1.5e2").unwrap(), Value::Number(-150.0));
    }

    #[test]
    fn bad_texts_are_errors() {
        for bad in [
            &b""[..],
            b"{",
            b"{\"a\" 1}",
            b"[1,]",
            b"[1 2]",
            b"\"\\x\"",
            b"\"a\nb\"",
            b"01x",
            b"tru",
            b"{} {}",
            b"\"\xff\"",
            b"1.",
            b"-",
        ] {
            assert!(parse(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
        let deep = "[".repeat(MAX_DEPTH + 2) + &"]".repeat(MAX_DEPTH + 2);
        assert!(parse(deep.as_bytes()).is_err());
        let ok = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(parse(ok.as_bytes()).is_ok());
    }

    #[test]
    fn random_bytes_are_errors_not_panics() {
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let pieces: [&[u8]; 10] = [b"{", b"}", b"[", b"]", b"\"", b":", b",", b"\\u", b"1e", b"null"];
        for _ in 0..30_000 {
            let n = (next() % 48) as usize;
            let mut buf = Vec::new();
            for _ in 0..n {
                if next() % 2 == 0 {
                    buf.extend_from_slice(pieces[(next() % pieces.len() as u64) as usize]);
                } else {
                    buf.push(next() as u8);
                }
            }
            let _ = parse(&buf);
        }
    }
}
