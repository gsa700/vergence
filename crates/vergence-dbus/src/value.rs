//! D-Bus values: signatures, and marshalling in either byte order.
//!
//! Alignment is counted from the start of the buffer the reader or writer
//! works on. Message bodies start on an 8-byte boundary of the message, so a
//! body read or written on its own aligns the same way.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The data ended inside a value.
    Short,
    /// A signature that isn't valid D-Bus.
    BadSignature(String),
    /// A string that isn't UTF-8 or lacks its terminating nul.
    BadString,
    /// A boolean other than 0 or 1, a length past its limit, and the like.
    BadValue(&'static str),
    /// Values nested deeper than the protocol allows.
    TooDeep,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Short => write!(f, "data ended early"),
            Error::BadSignature(s) => write!(f, "bad signature {s:?}"),
            Error::BadString => write!(f, "bad string"),
            Error::BadValue(what) => write!(f, "bad {what}"),
            Error::TooDeep => write!(f, "nested too deeply"),
        }
    }
}

impl std::error::Error for Error {}

/// Deepest nesting of containers accepted (the specification allows 32
/// arrays plus 32 structs; one limit for both is enough here).
const MAX_DEPTH: usize = 64;
/// Longest array the specification allows.
const MAX_ARRAY: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Byte(u8),
    Bool(bool),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    Double(f64),
    Str(String),
    Path(String),
    Sig(String),
    /// An index into the message's file descriptors.
    Fd(u32),
    /// Element signature (kept so an empty array still has a type), items.
    Array(String, Vec<Value>),
    Struct(Vec<Value>),
    /// A dictionary entry; only inside an array.
    Entry(Box<Value>, Box<Value>),
    Variant(Box<Value>),
}

impl Value {
    pub fn signature(&self) -> String {
        match self {
            Value::Byte(_) => "y".into(),
            Value::Bool(_) => "b".into(),
            Value::I16(_) => "n".into(),
            Value::U16(_) => "q".into(),
            Value::I32(_) => "i".into(),
            Value::U32(_) => "u".into(),
            Value::I64(_) => "x".into(),
            Value::U64(_) => "t".into(),
            Value::Double(_) => "d".into(),
            Value::Str(_) => "s".into(),
            Value::Path(_) => "o".into(),
            Value::Sig(_) => "g".into(),
            Value::Fd(_) => "h".into(),
            Value::Array(e, _) => format!("a{e}"),
            Value::Struct(vs) => format!("({})", vs.iter().map(Value::signature).collect::<String>()),
            Value::Entry(k, v) => format!("{{{}{}}}", k.signature(), v.signature()),
            Value::Variant(_) => "v".into(),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::Path(s) | Value::Sig(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Value::U32(v) => Some(*v),
            _ => None,
        }
    }

    /// An array of strings ("as").
    pub fn strings<I: IntoIterator<Item = S>, S: Into<String>>(items: I) -> Value {
        Value::Array("s".into(), items.into_iter().map(|s| Value::Str(s.into())).collect())
    }
}

fn is_basic(c: u8) -> bool {
    matches!(c, b'y' | b'b' | b'n' | b'q' | b'i' | b'u' | b'x' | b't' | b'd' | b's' | b'o' | b'g' | b'h')
}

/// End index of the single complete type starting at `i`.
fn type_end(sig: &[u8], i: usize, depth: usize) -> Result<usize, Error> {
    let bad = || Error::BadSignature(String::from_utf8_lossy(sig).into_owned());
    if depth > MAX_DEPTH {
        return Err(Error::TooDeep);
    }
    let c = *sig.get(i).ok_or_else(bad)?;
    match c {
        c if is_basic(c) || c == b'v' => Ok(i + 1),
        b'a' => {
            if sig.get(i + 1) == Some(&b'{') {
                let k = *sig.get(i + 2).ok_or_else(bad)?;
                if !is_basic(k) {
                    return Err(bad());
                }
                let v_end = type_end(sig, i + 3, depth + 1)?;
                if sig.get(v_end) != Some(&b'}') {
                    return Err(bad());
                }
                Ok(v_end + 1)
            } else {
                type_end(sig, i + 1, depth + 1)
            }
        }
        b'(' => {
            let mut j = i + 1;
            if sig.get(j) == Some(&b')') {
                return Err(bad());
            }
            while sig.get(j) != Some(&b')') {
                j = type_end(sig, j, depth + 1)?;
            }
            Ok(j + 1)
        }
        _ => Err(bad()),
    }
}

/// Split a signature into its complete types ("sa{sv}u" -> "s", "a{sv}", "u").
pub fn split_signature(sig: &str) -> Result<Vec<&str>, Error> {
    if sig.len() > 255 {
        return Err(Error::BadSignature(sig.into()));
    }
    let b = sig.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let end = type_end(b, i, 0)?;
        out.push(&sig[i..end]);
        i = end;
    }
    Ok(out)
}

fn alignment(c: u8) -> usize {
    match c {
        b'y' | b'g' | b'v' => 1,
        b'n' | b'q' => 2,
        b'b' | b'i' | b'u' | b'h' | b's' | b'o' | b'a' => 4,
        _ => 8, // x t d ( {
    }
}

pub struct Reader<'a> {
    buf: &'a [u8],
    pub pos: usize,
    big: bool,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8], big: bool) -> Self {
        Reader { buf, pos: 0, big }
    }

    pub fn at(buf: &'a [u8], pos: usize, big: bool) -> Self {
        Reader { buf, pos, big }
    }

    pub fn align(&mut self, n: usize) -> Result<(), Error> {
        let pad = (n - self.pos % n) % n;
        if self.pos + pad > self.buf.len() {
            return Err(Error::Short);
        }
        self.pos += pad;
        Ok(())
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if self.buf.len() - self.pos < n {
            return Err(Error::Short);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn arr<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        if self.big {
            a.reverse();
        }
        Ok(a)
    }

    pub fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }

    pub fn u32(&mut self) -> Result<u32, Error> {
        self.align(4)?;
        Ok(u32::from_le_bytes(self.arr()?))
    }

    fn string(&mut self) -> Result<String, Error> {
        let n = self.u32()? as usize;
        let b = self.take(n)?;
        if self.take(1)? != [0] {
            return Err(Error::BadString);
        }
        String::from_utf8(b.to_vec()).map_err(|_| Error::BadString)
    }

    pub fn signature(&mut self) -> Result<String, Error> {
        let n = self.u8()? as usize;
        let b = self.take(n)?;
        if self.take(1)? != [0] {
            return Err(Error::BadString);
        }
        String::from_utf8(b.to_vec()).map_err(|_| Error::BadString)
    }

    /// One value of the single complete type `sig`.
    pub fn value(&mut self, sig: &str) -> Result<Value, Error> {
        self.value_at(sig, 0)
    }

    fn value_at(&mut self, sig: &str, depth: usize) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return Err(Error::TooDeep);
        }
        let b = sig.as_bytes();
        let c = *b.first().ok_or_else(|| Error::BadSignature(sig.into()))?;
        self.align(alignment(c))?;
        Ok(match c {
            b'y' => Value::Byte(self.u8()?),
            b'b' => match u32::from_le_bytes(self.arr()?) {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                _ => return Err(Error::BadValue("boolean")),
            },
            b'n' => Value::I16(i16::from_le_bytes(self.arr()?)),
            b'q' => Value::U16(u16::from_le_bytes(self.arr()?)),
            b'i' => Value::I32(i32::from_le_bytes(self.arr()?)),
            b'u' => Value::U32(u32::from_le_bytes(self.arr()?)),
            b'h' => Value::Fd(u32::from_le_bytes(self.arr()?)),
            b'x' => Value::I64(i64::from_le_bytes(self.arr()?)),
            b't' => Value::U64(u64::from_le_bytes(self.arr()?)),
            b'd' => Value::Double(f64::from_bits(u64::from_le_bytes(self.arr()?))),
            b's' => Value::Str(self.string()?),
            b'o' => Value::Path(self.string()?),
            b'g' => Value::Sig(self.signature()?),
            b'v' => {
                let inner = self.signature()?;
                let types = split_signature(&inner)?;
                if types.len() != 1 {
                    return Err(Error::BadSignature(inner));
                }
                Value::Variant(Box::new(self.value_at(&inner, depth + 1)?))
            }
            b'a' => {
                let n = u32::from_le_bytes(self.arr()?) as usize;
                if n > MAX_ARRAY {
                    return Err(Error::BadValue("array length"));
                }
                let elem = &sig[1..];
                self.align(alignment(elem.as_bytes()[0]))?;
                let end = self.pos.checked_add(n).filter(|e| *e <= self.buf.len()).ok_or(Error::Short)?;
                let mut items = Vec::new();
                while self.pos < end {
                    items.push(self.value_at(elem, depth + 1)?);
                }
                if self.pos != end {
                    return Err(Error::BadValue("array length"));
                }
                Value::Array(elem.into(), items)
            }
            b'(' => {
                let inner = &sig[1..sig.len() - 1];
                let mut fields = Vec::new();
                for t in split_signature(inner)? {
                    fields.push(self.value_at(t, depth + 1)?);
                }
                Value::Struct(fields)
            }
            b'{' => {
                let inner = &sig[1..sig.len() - 1];
                let types = split_signature(inner)?;
                if types.len() != 2 {
                    return Err(Error::BadSignature(sig.into()));
                }
                let k = self.value_at(types[0], depth + 1)?;
                let v = self.value_at(types[1], depth + 1)?;
                Value::Entry(Box::new(k), Box::new(v))
            }
            _ => return Err(Error::BadSignature(sig.into())),
        })
    }

    /// All values of a signature of several types.
    pub fn values(&mut self, sig: &str) -> Result<Vec<Value>, Error> {
        split_signature(sig)?.into_iter().map(|t| self.value(t)).collect()
    }
}

pub struct Writer {
    pub buf: Vec<u8>,
    big: bool,
}

impl Writer {
    pub fn new(big: bool) -> Self {
        Writer { buf: Vec::new(), big }
    }

    pub fn align(&mut self, n: usize) {
        while self.buf.len() % n != 0 {
            self.buf.push(0);
        }
    }

    fn put(&mut self, mut bytes: Vec<u8>) {
        if self.big {
            bytes.reverse();
        }
        self.buf.extend_from_slice(&bytes);
    }

    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn u32(&mut self, v: u32) {
        self.align(4);
        self.put(v.to_le_bytes().to_vec());
    }

    fn string(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
    }

    pub fn signature(&mut self, s: &str) {
        self.buf.push(s.len() as u8);
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
    }

    pub fn value(&mut self, v: &Value) {
        let sig = v.signature();
        self.align(alignment(sig.as_bytes()[0]));
        match v {
            Value::Byte(b) => self.u8(*b),
            Value::Bool(b) => self.put((*b as u32).to_le_bytes().to_vec()),
            Value::I16(x) => self.put(x.to_le_bytes().to_vec()),
            Value::U16(x) => self.put(x.to_le_bytes().to_vec()),
            Value::I32(x) => self.put(x.to_le_bytes().to_vec()),
            Value::U32(x) | Value::Fd(x) => self.put(x.to_le_bytes().to_vec()),
            Value::I64(x) => self.put(x.to_le_bytes().to_vec()),
            Value::U64(x) => self.put(x.to_le_bytes().to_vec()),
            Value::Double(x) => self.put(x.to_bits().to_le_bytes().to_vec()),
            Value::Str(s) | Value::Path(s) => self.string(s),
            Value::Sig(s) => self.signature(s),
            Value::Variant(inner) => {
                self.signature(&inner.signature());
                self.value(inner);
            }
            Value::Array(elem, items) => {
                let at = self.buf.len();
                self.u32(0);
                self.align(alignment(elem.as_bytes()[0]));
                let start = self.buf.len();
                for i in items {
                    self.value(i);
                }
                let n = (self.buf.len() - start) as u32;
                let mut b = n.to_le_bytes();
                if self.big {
                    b.reverse();
                }
                self.buf[at..at + 4].copy_from_slice(&b);
            }
            Value::Struct(fields) => {
                for f in fields {
                    self.value(f);
                }
            }
            Value::Entry(k, v) => {
                self.value(k);
                self.value(v);
            }
        }
    }
}

/// The signature and marshalled bytes of a list of values, as a message body.
pub fn body(values: &[Value], big: bool) -> (String, Vec<u8>) {
    let mut w = Writer::new(big);
    for v in values {
        w.value(v);
    }
    (values.iter().map(Value::signature).collect(), w.buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict() -> Value {
        Value::Array(
            "{sv}".into(),
            vec![
                Value::Entry(
                    Box::new(Value::Str("UnixUserID".into())),
                    Box::new(Value::Variant(Box::new(Value::U32(1001)))),
                ),
                Value::Entry(
                    Box::new(Value::Str("Name".into())),
                    Box::new(Value::Variant(Box::new(Value::Str("x".into())))),
                ),
            ],
        )
    }

    #[test]
    fn signatures_split() {
        assert_eq!(split_signature("sa{sv}u(ii)av").unwrap(), ["s", "a{sv}", "u", "(ii)", "av"]);
        assert_eq!(split_signature("").unwrap(), Vec::<&str>::new());
        for bad in ["a", "(", "()", "a{vs}", "a{s}", "{sv}", "z", "(i"] {
            assert!(split_signature(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn values_round_trip_in_both_byte_orders() {
        let values = vec![
            Value::Byte(7),
            Value::Bool(true),
            Value::I16(-2),
            Value::U16(65000),
            Value::I32(-70000),
            Value::U32(4_000_000_000),
            Value::I64(-1),
            Value::U64(u64::MAX),
            Value::Double(3.25),
            Value::Str("héllo".into()),
            Value::Path("/org/freedesktop/DBus".into()),
            Value::Sig("a{sv}".into()),
            Value::Fd(0),
            Value::strings(["a", "bb"]),
            Value::Array("s".into(), vec![]),
            Value::Array("x".into(), vec![]),
            Value::Struct(vec![Value::Byte(1), Value::U64(2)]),
            dict(),
            Value::Variant(Box::new(Value::Struct(vec![Value::Str("s".into())]))),
        ];
        for big in [false, true] {
            let (sig, bytes) = body(&values, big);
            assert_eq!(Reader::new(&bytes, big).values(&sig).unwrap(), values);
        }
    }

    #[test]
    fn known_little_endian_bytes() {
        // "su": "ab", 5 -> len 2, 'a' 'b' nul, pad to 8, 5.
        let (sig, bytes) = body(&[Value::Str("ab".into()), Value::U32(5)], false);
        assert_eq!(sig, "su");
        assert_eq!(bytes, [2, 0, 0, 0, b'a', b'b', 0, 0, 5, 0, 0, 0]);
        // An empty array of 8-aligned items still pads to its first item.
        let (_, bytes) = body(&[Value::Byte(1), Value::Array("t".into(), vec![])], false);
        assert_eq!(bytes, [1, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn bad_data_is_rejected() {
        assert_eq!(Reader::new(&[2, 0, 0, 0], false).value("b"), Err(Error::BadValue("boolean")));
        assert_eq!(Reader::new(&[3, 0, 0, 0, b'a', b'b'], false).value("s"), Err(Error::Short));
        assert_eq!(Reader::new(&[1, 0, 0, 0, b'a', 1], false).value("s"), Err(Error::BadString));
        // An array claiming more bytes than there are.
        assert_eq!(Reader::new(&[100, 0, 0, 0, 1, 0, 0, 0], false).value("au"), Err(Error::Short));
        // A variant whose signature is two types.
        assert!(Reader::new(&[2, b'u', b'u', 0, 0, 0, 0, 0, 0, 0, 0, 0], false).value("v").is_err());
    }
}
