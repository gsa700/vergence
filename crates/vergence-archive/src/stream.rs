//! The helper's answer on its pipe: a stream of our own, simple enough that
//! the side writing files trusts nothing but its framing. Numbers are
//! big-endian (as the PDF helper's pipe).
//!
//! - `E` an entry: kind (u8: [`Kind`]), permission bits (u32), modified
//!   time (i64 seconds), size (i64, -1 when the archive does not say), its
//!   name and its link target (each a u32 length and the bytes as the
//!   archive has them; the target empty unless a link).
//! - `D` a piece of the last entry's contents: a u32 length and the bytes
//!   (at most [`MAX_CHUNK`]).
//! - `P` the archive needs a password; nothing follows.
//! - `X` it could not be read: a u32 length and why, in UTF-8; nothing
//!   follows.
//! - `Z` the end.

use std::io::{self, Read, Write};

/// The most bytes one `D` carries.
pub const MAX_CHUNK: usize = 1 << 20;
/// The longest name or link target passed on.
pub const MAX_NAME: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File = 0,
    Dir = 1,
    Symlink = 2,
    Hardlink = 3,
    /// A device, a pipe, a socket: never made.
    Other = 4,
}

impl Kind {
    fn from(b: u8) -> Option<Kind> {
        Some(match b {
            0 => Kind::File,
            1 => Kind::Dir,
            2 => Kind::Symlink,
            3 => Kind::Hardlink,
            4 => Kind::Other,
            _ => return None,
        })
    }
}

/// One entry's header, as the archive says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    pub perm: u32,
    pub mtime: i64,
    /// None when the archive does not say.
    pub size: Option<u64>,
    pub name: Vec<u8>,
    pub link: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Entry(Header),
    Data(Vec<u8>),
    NeedsPassword,
    Failed(String),
    End,
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    let b = &b[..b.len().min(MAX_NAME)];
    out.extend((b.len() as u32).to_be_bytes());
    out.extend(b);
}

pub fn write_entry(w: &mut impl Write, h: &Header) -> io::Result<()> {
    let mut out = vec![b'E', h.kind as u8];
    out.extend(h.perm.to_be_bytes());
    out.extend(h.mtime.to_be_bytes());
    out.extend(h.size.map_or(-1i64, |s| s.min(i64::MAX as u64) as i64).to_be_bytes());
    put_bytes(&mut out, &h.name);
    put_bytes(&mut out, &h.link);
    w.write_all(&out)
}

pub fn write_data(w: &mut impl Write, data: &[u8]) -> io::Result<()> {
    for piece in data.chunks(MAX_CHUNK) {
        w.write_all(b"D")?;
        w.write_all(&(piece.len() as u32).to_be_bytes())?;
        w.write_all(piece)?;
    }
    Ok(())
}

pub fn write_password(w: &mut impl Write) -> io::Result<()> {
    w.write_all(b"P")
}

pub fn write_failed(w: &mut impl Write, why: &str) -> io::Result<()> {
    let mut out = vec![b'X'];
    put_bytes(&mut out, why.as_bytes());
    w.write_all(&out)
}

pub fn write_end(w: &mut impl Write) -> io::Result<()> {
    w.write_all(b"Z")
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("the helper's answer: {what}"))
}

fn u32_of(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}

fn i64_of(r: &mut impl Read) -> io::Result<i64> {
    let mut b = [0; 8];
    r.read_exact(&mut b)?;
    Ok(i64::from_be_bytes(b))
}

fn bytes_of(r: &mut impl Read, max: usize) -> io::Result<Vec<u8>> {
    let n = u32_of(r)? as usize;
    if n > max {
        return Err(bad("too long"));
    }
    let mut b = vec![0; n];
    r.read_exact(&mut b)?;
    Ok(b)
}

/// The next message. A stream that ends without `Z`, `P` or `X` is an
/// error (the helper crashed or was stopped).
pub fn read_message(r: &mut impl Read) -> io::Result<Message> {
    let mut op = [0; 1];
    if let Err(e) = r.read_exact(&mut op) {
        return Err(if e.kind() == io::ErrorKind::UnexpectedEof { bad("it ended early") } else { e });
    }
    Ok(match op[0] {
        b'E' => {
            let mut k = [0; 1];
            r.read_exact(&mut k)?;
            let kind = Kind::from(k[0]).ok_or_else(|| bad("an unknown kind"))?;
            let perm = u32_of(r)?;
            let mtime = i64_of(r)?;
            let size = i64_of(r)?;
            let name = bytes_of(r, MAX_NAME)?;
            let link = bytes_of(r, MAX_NAME)?;
            Message::Entry(Header { kind, perm, mtime, size: u64::try_from(size).ok(), name, link })
        }
        b'D' => Message::Data(bytes_of(r, MAX_CHUNK)?),
        b'P' => Message::NeedsPassword,
        b'X' => Message::Failed(String::from_utf8_lossy(&bytes_of(r, MAX_NAME)?).into_owned()),
        b'Z' => Message::End,
        _ => return Err(bad("an unknown message")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_go_and_come_back() {
        let h = Header {
            kind: Kind::Symlink,
            perm: 0o755,
            mtime: -3,
            size: None,
            name: b"a/b".to_vec(),
            link: b"../c".to_vec(),
        };
        let mut out = Vec::new();
        write_entry(&mut out, &h).unwrap();
        write_data(&mut out, &vec![7; MAX_CHUNK + 5]).unwrap();
        write_failed(&mut out, "bad").unwrap();
        write_end(&mut out).unwrap();
        let mut r = out.as_slice();
        assert_eq!(read_message(&mut r).unwrap(), Message::Entry(h));
        assert_eq!(read_message(&mut r).unwrap(), Message::Data(vec![7; MAX_CHUNK]));
        assert_eq!(read_message(&mut r).unwrap(), Message::Data(vec![7; 5]));
        assert_eq!(read_message(&mut r).unwrap(), Message::Failed("bad".into()));
        assert_eq!(read_message(&mut r).unwrap(), Message::End);
        assert!(read_message(&mut r).is_err(), "nothing after the end");
    }

    #[test]
    fn a_lying_length_is_refused() {
        let mut out = vec![b'D'];
        out.extend(((MAX_CHUNK + 1) as u32).to_be_bytes());
        assert!(read_message(&mut out.as_slice()).is_err());
        assert!(read_message(&mut b"Q".as_slice()).is_err());
    }
}
