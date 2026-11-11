//! The wire format: frames, and the field encoding inside them.
//!
//! A frame is `[u32 length][u16 opcode][payload]`, little-endian, where
//! `length` counts the opcode and the payload (so it is at least 2) and is at
//! most [`MAX_FRAME`]. Fields inside a payload are written in order with no
//! tags: integers little-endian, `f64` as its IEEE bits, strings and byte
//! strings as `u32 length` + bytes (strings must be UTF-8), lists as
//! `u32 count` + items. Every message type says its fields in order.
//!
//! Trailing bytes after the last field are allowed and ignored, so a later
//! protocol version can add fields at the end of a message without breaking
//! older readers.

use std::fmt;

/// Largest frame (opcode + payload) either side accepts. Anything larger is
/// a protocol error and ends the connection.
pub const MAX_FRAME: usize = 64 * 1024;

/// Longest string or list count accepted inside a payload (bounded by the
/// frame anyway; this keeps a bad count from reserving memory).
const MAX_COUNT: usize = MAX_FRAME;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// The payload ended before a field did.
    Short,
    /// A string was not UTF-8.
    Utf8,
    /// A length or count larger than the protocol allows.
    TooLong(usize),
    /// A value outside its field's range (an unknown enum value, say).
    BadValue(&'static str),
    /// An opcode this side doesn't know.
    UnknownOpcode(u16),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Short => write!(f, "message ended early"),
            WireError::Utf8 => write!(f, "string is not UTF-8"),
            WireError::TooLong(n) => write!(f, "length {n} is too long"),
            WireError::BadValue(what) => write!(f, "bad value for {what}"),
            WireError::UnknownOpcode(op) => write!(f, "unknown opcode {op}"),
        }
    }
}

impl std::error::Error for WireError {}

/// Writes one frame. Start it with [`Encoder::frame`], add fields, then
/// [`Encoder::finish`].
pub struct Encoder {
    buf: Vec<u8>,
}

impl Encoder {
    pub fn frame(opcode: u16) -> Self {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(&[0; 4]);
        buf.extend_from_slice(&opcode.to_le_bytes());
        Encoder { buf }
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn f64(&mut self, v: f64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_bits().to_le_bytes());
        self
    }

    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.u8(v as u8)
    }

    pub fn str(&mut self, v: &str) -> &mut Self {
        self.bytes(v.as_bytes())
    }

    pub fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.u32(v.len() as u32);
        self.buf.extend_from_slice(v);
        self
    }

    /// A list count; the caller writes that many items after it.
    pub fn count(&mut self, n: usize) -> &mut Self {
        self.u32(n as u32)
    }

    /// The finished frame, or `TooLong` if it exceeds [`MAX_FRAME`].
    pub fn finish(mut self) -> Result<Vec<u8>, WireError> {
        let len = self.buf.len() - 4;
        if len > MAX_FRAME {
            return Err(WireError::TooLong(len));
        }
        self.buf[..4].copy_from_slice(&(len as u32).to_le_bytes());
        Ok(self.buf)
    }
}

/// Reads the fields of one frame's payload.
pub struct Decoder<'a> {
    data: &'a [u8],
}

impl<'a> Decoder<'a> {
    pub fn new(payload: &'a [u8]) -> Self {
        Decoder { data: payload }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        if self.data.len() < n {
            return Err(WireError::Short);
        }
        let (head, rest) = self.data.split_at(n);
        self.data = rest;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let mut a = [0; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }

    pub fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub fn i32(&mut self) -> Result<i32, WireError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub fn f64(&mut self) -> Result<f64, WireError> {
        Ok(f64::from_bits(u64::from_le_bytes(self.array()?)))
    }

    pub fn bool(&mut self) -> Result<bool, WireError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(WireError::BadValue("bool")),
        }
    }

    pub fn bytes(&mut self) -> Result<&'a [u8], WireError> {
        let n = self.u32()? as usize;
        if n > MAX_COUNT {
            return Err(WireError::TooLong(n));
        }
        self.take(n)
    }

    pub fn str(&mut self) -> Result<String, WireError> {
        let b = self.bytes()?;
        std::str::from_utf8(b).map(str::to_owned).map_err(|_| WireError::Utf8)
    }

    /// A list count, checked against what is left: every item takes at least
    /// `min_item` bytes, so a count that can't fit is rejected up front.
    pub fn count(&mut self, min_item: usize) -> Result<usize, WireError> {
        let n = self.u32()? as usize;
        if n > MAX_COUNT || n.saturating_mul(min_item.max(1)) > self.data.len() {
            return Err(WireError::TooLong(n));
        }
        Ok(n)
    }
}

/// Collects bytes as they arrive on a stream and hands out whole frames.
#[derive(Default)]
pub struct FrameReader {
    buf: Vec<u8>,
}

/// One frame off the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub opcode: u16,
    pub payload: Vec<u8>,
}

impl FrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add bytes read from the stream.
    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// The next whole frame, `Ok(None)` if more bytes are needed, or an error
    /// if the stream is not speaking the protocol (the connection should end).
    pub fn next_frame(&mut self) -> Result<Option<Frame>, WireError> {
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_le_bytes(self.buf[..4].try_into().unwrap()) as usize;
        if len > MAX_FRAME {
            return Err(WireError::TooLong(len));
        }
        if len < 2 {
            return Err(WireError::Short);
        }
        if self.buf.len() < 4 + len {
            return Ok(None);
        }
        let opcode = u16::from_le_bytes(self.buf[4..6].try_into().unwrap());
        let payload = self.buf[6..4 + len].to_vec();
        self.buf.drain(..4 + len);
        Ok(Some(Frame { opcode, payload }))
    }

    /// Bytes held that don't yet make a whole frame.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_round_trip() {
        let mut e = Encoder::frame(7);
        e.u8(1).u16(0xbeef).u32(0xdead_beef).i32(-5).f64(3.2833).bool(true).str("héllo").bytes(&[1, 2, 3]).count(2);
        let frame = e.finish().unwrap();
        let mut r = FrameReader::new();
        r.push(&frame);
        let f = r.next_frame().unwrap().unwrap();
        assert_eq!(f.opcode, 7);
        let mut d = Decoder::new(&f.payload);
        assert_eq!(d.u8().unwrap(), 1);
        assert_eq!(d.u16().unwrap(), 0xbeef);
        assert_eq!(d.u32().unwrap(), 0xdead_beef);
        assert_eq!(d.i32().unwrap(), -5);
        assert_eq!(d.f64().unwrap(), 3.2833);
        assert!(d.bool().unwrap());
        assert_eq!(d.str().unwrap(), "héllo");
        assert_eq!(d.bytes().unwrap(), &[1, 2, 3]);
        assert_eq!(d.u32().unwrap(), 2);
        assert_eq!(d.u8(), Err(WireError::Short));
    }

    #[test]
    fn frames_arrive_in_pieces() {
        let a = { let mut e = Encoder::frame(1); e.str("one"); e.finish().unwrap() };
        let b = { let mut e = Encoder::frame(2); e.u32(2); e.finish().unwrap() };
        let all: Vec<u8> = a.iter().chain(b.iter()).copied().collect();
        let mut r = FrameReader::new();
        let mut got = Vec::new();
        for byte in all {
            r.push(&[byte]);
            while let Some(f) = r.next_frame().unwrap() {
                got.push(f.opcode);
            }
        }
        assert_eq!(got, [1, 2]);
        assert_eq!(r.pending(), 0);
    }

    #[test]
    fn oversized_and_undersized_frames_are_errors() {
        let mut r = FrameReader::new();
        r.push(&((MAX_FRAME as u32 + 1).to_le_bytes()));
        assert_eq!(r.next_frame(), Err(WireError::TooLong(MAX_FRAME + 1)));
        let mut r = FrameReader::new();
        r.push(&1u32.to_le_bytes());
        r.push(&[0]);
        assert_eq!(r.next_frame(), Err(WireError::Short));
        let mut e = Encoder::frame(1);
        e.bytes(&vec![0; MAX_FRAME]);
        assert!(matches!(e.finish(), Err(WireError::TooLong(_))));
    }

    #[test]
    fn bad_strings_and_counts_are_rejected() {
        let mut d = Decoder::new(&[2, 0, 0, 0, 0xff, 0xfe]);
        assert_eq!(d.str(), Err(WireError::Utf8));
        // A count of a million items in a 4-byte payload.
        let million = 1_000_000u32.to_le_bytes();
        let mut d = Decoder::new(&million);
        assert!(matches!(d.count(1), Err(WireError::TooLong(_))));
        let mut d = Decoder::new(&[2]);
        assert_eq!(d.bool(), Err(WireError::BadValue("bool")));
    }
}
