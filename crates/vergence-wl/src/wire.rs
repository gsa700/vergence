//! Wayland messages as bytes.
//!
//! A message is a header of two 32-bit words, the object it is for, and its
//! size in bytes (header included) in the upper half of the second word with
//! the opcode in the lower, followed by its arguments, each a multiple of
//! four bytes, in the machine's byte order:
//!
//! - `int`, `uint`, `object`, `new_id`: one word;
//! - `fixed`: one word, a signed number in 256ths;
//! - `string`: its length with the closing zero byte in one word, the bytes,
//!   the zero, and padding to a whole word (length 0: no string);
//! - `array`: its length in one word, the bytes, padding;
//! - `fd`: nothing here; the file descriptor travels beside the bytes.
//!
//! [`Writer`] builds one message, [`split`] finds the next message in bytes
//! that arrived, [`Reader`] takes its arguments apart. The server is the
//! project's own, but what it sends is read as carefully as a stranger's:
//! every length is checked against the bytes that are there (tests/fuzz.rs).

use std::{fmt, os::fd::RawFd};

/// The largest message either side sends (the reference library's limit).
pub const MAX_MESSAGE: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// A message's size is smaller than a header, not a multiple of four, or
    /// larger than [`MAX_MESSAGE`].
    Size(usize),
    /// An argument runs past the end of its message.
    Short,
    /// A string without its closing zero byte.
    String,
    /// A message being built grew past [`MAX_MESSAGE`].
    TooLong,
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Size(n) => write!(f, "message size {n} is not valid"),
            WireError::Short => write!(f, "argument runs past the message"),
            WireError::String => write!(f, "string is not terminated"),
            WireError::TooLong => write!(f, "message is too long"),
        }
    }
}

impl std::error::Error for WireError {}

fn padded(n: usize) -> usize {
    n.div_ceil(4) * 4
}

/// Builds one message: a request to `object`.
pub struct Writer {
    buf: Vec<u8>,
    fds: Vec<RawFd>,
}

impl Writer {
    pub fn new(object: u32, opcode: u16) -> Writer {
        let mut buf = Vec::with_capacity(32);
        buf.extend_from_slice(&object.to_ne_bytes());
        buf.extend_from_slice(&(opcode as u32).to_ne_bytes());
        Writer { buf, fds: Vec::new() }
    }

    pub fn uint(mut self, v: u32) -> Self {
        self.buf.extend_from_slice(&v.to_ne_bytes());
        self
    }

    pub fn int(mut self, v: i32) -> Self {
        self.buf.extend_from_slice(&v.to_ne_bytes());
        self
    }

    /// A number in 256ths (out of range: the nearest that fits).
    pub fn fixed(self, v: f64) -> Self {
        self.int((v * 256.0).round().clamp(i32::MIN as f64, i32::MAX as f64) as i32)
    }

    pub fn object(self, id: u32) -> Self {
        self.uint(id)
    }

    /// A string. A zero byte inside it would end it early for the reader, so
    /// the string is cut there.
    pub fn string(mut self, v: &str) -> Self {
        let bytes = v.as_bytes();
        let bytes = &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())];
        self.buf.extend_from_slice(&(bytes.len() as u32 + 1).to_ne_bytes());
        self.buf.extend_from_slice(bytes);
        self.buf.resize(self.buf.len() + padded(bytes.len() + 1) - bytes.len(), 0);
        self
    }

    pub fn array(mut self, v: &[u8]) -> Self {
        self.buf.extend_from_slice(&(v.len() as u32).to_ne_bytes());
        self.buf.extend_from_slice(v);
        self.buf.resize(self.buf.len() + padded(v.len()) - v.len(), 0);
        self
    }

    /// A file descriptor to send with the message. It stays the caller's:
    /// the receiver gets a copy.
    pub fn fd(mut self, fd: RawFd) -> Self {
        self.fds.push(fd);
        self
    }

    /// The finished message and the file descriptors that go with it.
    pub fn finish(mut self) -> Result<(Vec<u8>, Vec<RawFd>), WireError> {
        let len = self.buf.len();
        if len > MAX_MESSAGE {
            return Err(WireError::TooLong);
        }
        let opcode = u32::from_ne_bytes([self.buf[4], self.buf[5], self.buf[6], self.buf[7]]) & 0xffff;
        self.buf[4..8].copy_from_slice(&(((len as u32) << 16) | opcode).to_ne_bytes());
        Ok((self.buf, self.fds))
    }
}

/// A message's header: who it is for, what it is, and how long it is in all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub object: u32,
    pub opcode: u16,
    pub size: usize,
}

/// The message at the front of `buf`: its header, if all of it has arrived
/// (None: wait for more bytes). Its arguments are `buf[8..header.size]`.
pub fn split(buf: &[u8]) -> Result<Option<Header>, WireError> {
    if buf.len() < 8 {
        return Ok(None);
    }
    let object = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let word = u32::from_ne_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let size = (word >> 16) as usize;
    if size < 8 || size % 4 != 0 || size > MAX_MESSAGE {
        return Err(WireError::Size(size));
    }
    Ok((buf.len() >= size).then_some(Header { object, opcode: word as u16, size }))
}

/// Takes a message's arguments apart, in order.
pub struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(arguments: &'a [u8]) -> Reader<'a> {
        Reader { buf: arguments }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        if self.buf.len() < n {
            return Err(WireError::Short);
        }
        let (head, rest) = self.buf.split_at(n);
        self.buf = rest;
        Ok(head)
    }

    pub fn uint(&mut self) -> Result<u32, WireError> {
        let b = self.take(4)?;
        Ok(u32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn int(&mut self) -> Result<i32, WireError> {
        Ok(self.uint()? as i32)
    }

    pub fn fixed(&mut self) -> Result<f64, WireError> {
        Ok(self.int()? as f64 / 256.0)
    }

    /// A string (None: the sender gave none). Bytes that are not UTF-8 are
    /// replaced, not refused: a title or an error text is for showing.
    pub fn string(&mut self) -> Result<Option<String>, WireError> {
        let len = self.uint()? as usize;
        if len == 0 {
            return Ok(None);
        }
        let bytes = self.take(padded(len))?;
        if bytes[len - 1] != 0 {
            return Err(WireError::String);
        }
        let text = &bytes[..len - 1];
        let text = &text[..text.iter().position(|b| *b == 0).unwrap_or(text.len())];
        Ok(Some(String::from_utf8_lossy(text).into_owned()))
    }

    pub fn array(&mut self) -> Result<&'a [u8], WireError> {
        let len = self.uint()? as usize;
        Ok(&self.take(padded(len))?[..len])
    }

    /// An array of words (a toplevel's states, a keyboard's held keys).
    pub fn words(&mut self) -> Result<Vec<u32>, WireError> {
        Ok(self.array()?.chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_built_and_taken_apart() {
        let (bytes, fds) = Writer::new(7, 3).uint(42).int(-5).fixed(1.5).string("hello").object(9).array(&[1, 2, 3, 4, 5]).string("").fd(11).finish().unwrap();
        assert_eq!(fds, [11]);
        assert_eq!(bytes.len() % 4, 0);
        let h = split(&bytes).unwrap().unwrap();
        assert_eq!(h, Header { object: 7, opcode: 3, size: bytes.len() });
        let mut r = Reader::new(&bytes[8..h.size]);
        assert_eq!(r.uint(), Ok(42));
        assert_eq!(r.int(), Ok(-5));
        assert_eq!(r.fixed(), Ok(1.5));
        assert_eq!(r.string(), Ok(Some("hello".into())));
        assert_eq!(r.uint(), Ok(9));
        assert_eq!(r.array(), Ok(&[1u8, 2, 3, 4, 5][..]));
        assert_eq!(r.string(), Ok(Some(String::new())));
        assert_eq!(r.uint(), Err(WireError::Short));
    }

    #[test]
    fn strings_are_padded_as_the_protocol_says() {
        // "wl_shm" is 6 bytes: length 7 with its zero, padded to 8.
        let (bytes, _) = Writer::new(2, 0).uint(1).string("wl_shm").uint(1).uint(3).finish().unwrap();
        assert_eq!(bytes.len(), 8 + 4 + 4 + 8 + 4 + 4);
        assert_eq!(&bytes[12..16], &7u32.to_ne_bytes());
        assert_eq!(&bytes[16..24], b"wl_shm\0\0");
        // Three bytes: length 4, no padding. Four: length 5, padded to 8.
        assert_eq!(Writer::new(1, 0).string("abc").finish().unwrap().0.len(), 8 + 4 + 4);
        assert_eq!(Writer::new(1, 0).string("abcd").finish().unwrap().0.len(), 8 + 4 + 8);
        // A zero byte inside cuts the string, as it would for the reader.
        let (bytes, _) = Writer::new(1, 0).string("ab\0cd").finish().unwrap();
        assert_eq!(Reader::new(&bytes[8..]).string(), Ok(Some("ab".into())));
    }

    #[test]
    fn messages_are_found_one_at_a_time() {
        let (a, _) = Writer::new(1, 1).uint(2).finish().unwrap();
        let (b, _) = Writer::new(3, 0).finish().unwrap();
        let mut stream = a.clone();
        stream.extend_from_slice(&b);
        for cut in 0..a.len() {
            assert_eq!(split(&stream[..cut]), Ok(None), "an unfinished message at {cut}");
        }
        let first = split(&stream).unwrap().unwrap();
        assert_eq!((first.object, first.opcode, first.size), (1, 1, 12));
        let second = split(&stream[first.size..]).unwrap().unwrap();
        assert_eq!((second.object, second.opcode, second.size), (3, 0, 8));
    }

    #[test]
    fn sizes_that_cannot_be_are_refused() {
        let header = |size: u32| {
            let mut b = 1u32.to_ne_bytes().to_vec();
            b.extend_from_slice(&(size << 16).to_ne_bytes());
            b
        };
        assert_eq!(split(&header(0)), Err(WireError::Size(0)));
        assert_eq!(split(&header(4)), Err(WireError::Size(4)));
        assert_eq!(split(&header(10)), Err(WireError::Size(10)));
        assert_eq!(split(&header(4100)), Err(WireError::Size(4100)));
        assert_eq!(split(&header(8)).map(|h| h.map(|h| h.size)), Ok(Some(8)));
        assert_eq!(Writer::new(1, 0).array(&[0; 5000]).finish(), Err(WireError::TooLong));
    }

    #[test]
    fn bad_arguments_are_errors() {
        // A string that claims more bytes than the message has.
        let mut body = 100u32.to_ne_bytes().to_vec();
        body.extend_from_slice(b"abc\0");
        assert_eq!(Reader::new(&body).string(), Err(WireError::Short));
        // A string without its zero.
        let mut body = 4u32.to_ne_bytes().to_vec();
        body.extend_from_slice(b"abcd");
        assert_eq!(Reader::new(&body).string(), Err(WireError::String));
        // No string at all.
        assert_eq!(Reader::new(&0u32.to_ne_bytes()).string(), Ok(None));
        // Bytes that are not UTF-8 are shown as replacements.
        let mut body = 3u32.to_ne_bytes().to_vec();
        body.extend_from_slice(&[0xff, b'a', 0, 0]);
        assert_eq!(Reader::new(&body).string(), Ok(Some("\u{fffd}a".into())));
        // An array longer than what is left.
        assert_eq!(Reader::new(&9u32.to_ne_bytes()).array(), Err(WireError::Short));
        assert_eq!(Reader::new(&u32::MAX.to_ne_bytes()).array(), Err(WireError::Short));
    }
}
