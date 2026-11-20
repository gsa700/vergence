//! A PDF's objects (ISO 32000-2 §7.3): what the parser makes of the bytes.

use std::fmt;

/// An indirect object's number and generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ref {
    pub num: u32,
    pub generation: u16,
}

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} R", self.num, self.generation)
    }
}

/// A dictionary: keys (names, without the slash) in the order written.
/// Lookups are linear; PDF dictionaries are small.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dict(pub Vec<(Vec<u8>, Object)>);

impl Dict {
    pub fn get(&self, key: &str) -> Option<&Object> {
        let key = key.as_bytes();
        // The last one wins, as most readers do with a repeated key.
        self.0.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// The value's name, if it is a name (`/Type /Page` -> "Page").
    pub fn name(&self, key: &str) -> Option<&[u8]> {
        self.get(key).and_then(Object::as_name)
    }

    pub fn int(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Object::as_int)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A stream: its dictionary, and where its raw (still encoded) bytes are in
/// the file, or the bytes themselves (a stream inside an object stream
/// cannot be, but a repaired one can be cut out).
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Object {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    /// A string's bytes, escapes and hex already decoded.
    String(Vec<u8>),
    /// A name's bytes, without the slash, #xx already decoded.
    Name(Vec<u8>),
    Array(Vec<Object>),
    Dict(Dict),
    Stream(Stream),
    Ref(Ref),
}

impl Object {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Object::Int(i) => Some(*i),
            // Some writers put 612.0 where an integer belongs.
            Object::Real(r) if r.is_finite() && r.fract() == 0.0 && r.abs() < 1e15 => Some(*r as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Object::Int(i) => Some(*i as f64),
            Object::Real(r) if r.is_finite() => Some(*r),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Object::Name(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Object]> {
        match self {
            Object::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_ref(&self) -> Option<Ref> {
        match self {
            Object::Ref(r) => Some(*r),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Object::String(s) => Some(s),
            _ => None,
        }
    }
}
