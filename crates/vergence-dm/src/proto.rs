//! The unlock protocol: between a user's server (its lock screen) and vdm,
//! over vdm's unlock socket. vdm runs a PAM conversation for the caller's
//! own account (the kernel says who the caller is) and relays each of PAM's
//! messages; the client answers prompts. Fixed and small (threat model T13,
//! review "rules for the code"): a frame is a two-byte length (big-endian),
//! checked against [`MAX_FRAME`] before anything is read, a kind, its body.
//! Anything unknown ends the connection.
//!
//!   client -> vdm   Hello { version, service }   first, once
//!                   Answer { secret }            to a Prompt
//!                   Cancel                       the user gave up
//!   vdm -> client   Prompt { echo, text }        PAM asks (echo: show what is typed)
//!                   Info { text } / Error { text }
//!                   Done { ok, wait }            the end; wait: seconds before another try is taken
//!
//! On the session socket (step 2, review C4) a user's server stays connected;
//! the connection is the registration (vdm knows its uid from the kernel):
//!
//!   vdm -> server   LockNow                      the machine is about to sleep
//!   server -> vdm   Presented                    the lock is on every screen
//!                   CannotLock { text }          and why (it says so to the user too)

use std::io::{self, Read, Write};

use crate::secret::{self, Secret};

pub const VERSION: u8 = 1;
/// The largest frame either side sends or takes, kind and body.
pub const MAX_FRAME: usize = 2048;
/// The longest text in a Prompt, Info or Error (longer is cut at a character).
pub const MAX_TEXT: usize = 512;

/// What the conversation is for. Only Unlock in this milestone; elevation
/// will have a socket of its own (review M7), never this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Unlock,
    /// The login screen (step 4), on the greeter socket only.
    Login,
}

pub enum ToVdm {
    Hello { version: u8, service: Service },
    Answer(Secret),
    Cancel,
    Presented,
    CannotLock(String),
    /// The login screen: who logs in, and which session (an index into the
    /// list vdm and the greeter both read, the system's own files).
    Login { user: String, session: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromVdm {
    Prompt { echo: bool, text: String },
    Info(String),
    Error(String),
    Done { ok: bool, wait: u16 },
    LockNow,
}

#[derive(Debug)]
pub enum ProtoError {
    Io(io::Error),
    /// The other side broke the format: the connection ends.
    Bad(&'static str),
}

impl From<io::Error> for ProtoError {
    fn from(e: io::Error) -> Self {
        ProtoError::Io(e)
    }
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoError::Io(e) => write!(f, "{e}"),
            ProtoError::Bad(why) => write!(f, "bad message: {why}"),
        }
    }
}

mod kind {
    pub const HELLO: u8 = 1;
    pub const ANSWER: u8 = 2;
    pub const CANCEL: u8 = 3;
    pub const PRESENTED: u8 = 4;
    pub const CANNOT_LOCK: u8 = 5;
    pub const LOGIN: u8 = 6;
    pub const PROMPT: u8 = 16;
    pub const INFO: u8 = 17;
    pub const ERROR: u8 = 18;
    pub const DONE: u8 = 19;
    pub const LOCK_NOW: u8 = 20;
}

/// Text from PAM as it may be shown: no control characters, at most
/// [`MAX_TEXT`] bytes, cut at a character.
pub fn clean(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let c = if c.is_control() { ' ' } else { c };
        if out.len() + c.len_utf8() > MAX_TEXT {
            break;
        }
        out.push(c);
    }
    out
}

fn header(w: &mut impl Write, kind: u8, len: usize) -> io::Result<()> {
    let n = (len + 1) as u16;
    w.write_all(&[(n >> 8) as u8, n as u8, kind])
}

/// Read a frame's header: its kind and body length, checked.
fn read_header(r: &mut impl Read) -> Result<(u8, usize), ProtoError> {
    let mut h = [0u8; 3];
    r.read_exact(&mut h)?;
    let n = u16::from_be_bytes([h[0], h[1]]) as usize;
    if n == 0 || n > MAX_FRAME {
        return Err(ProtoError::Bad("frame length"));
    }
    Ok((h[2], n - 1))
}

fn read_body(r: &mut impl Read, len: usize) -> Result<Vec<u8>, ProtoError> {
    let mut b = vec![0u8; len];
    r.read_exact(&mut b)?;
    Ok(b)
}

pub fn send_to_vdm(w: &mut impl Write, m: &ToVdm) -> io::Result<()> {
    match m {
        ToVdm::Hello { version, service } => {
            header(w, kind::HELLO, 2)?;
            w.write_all(&[*version, match service {
                Service::Unlock => 1,
                Service::Login => 2,
            }])?;
        }
        ToVdm::Answer(s) => {
            header(w, kind::ANSWER, s.len())?;
            w.write_all(s.as_bytes())?;
        }
        ToVdm::Cancel => header(w, kind::CANCEL, 0)?,
        ToVdm::Presented => header(w, kind::PRESENTED, 0)?,
        ToVdm::Login { user, session } => {
            header(w, kind::LOGIN, 1 + user.len())?;
            w.write_all(&[*session])?;
            w.write_all(user.as_bytes())?;
        }
        ToVdm::CannotLock(t) => {
            let t = clean(t);
            header(w, kind::CANNOT_LOCK, t.len())?;
            w.write_all(t.as_bytes())?;
        }
    }
    w.flush()
}

/// vdm's side: the next message, an Answer read straight into a [`Secret`].
pub fn read_to_vdm(r: &mut impl Read) -> Result<ToVdm, ProtoError> {
    let (k, len) = read_header(r)?;
    match k {
        kind::HELLO => {
            let b = read_body(r, len)?;
            match b.as_slice() {
                [version, 1] => Ok(ToVdm::Hello { version: *version, service: Service::Unlock }),
                [version, 2] => Ok(ToVdm::Hello { version: *version, service: Service::Login }),
                [_, _] => Err(ProtoError::Bad("service")),
                _ => Err(ProtoError::Bad("hello")),
            }
        }
        kind::ANSWER => {
            if len > secret::MAX {
                return Err(ProtoError::Bad("answer too long"));
            }
            let mut s = Secret::new().ok_or(ProtoError::Bad("no memory for the answer"))?;
            r.read_exact(&mut s.room()[..len])?;
            s.set_len(len);
            Ok(ToVdm::Answer(s))
        }
        kind::CANCEL if len == 0 => Ok(ToVdm::Cancel),
        kind::PRESENTED if len == 0 => Ok(ToVdm::Presented),
        // A user name: 1 to 64 bytes of letters, digits, '.', '_' and '-' (no other byte reaches PAM).
        kind::LOGIN if (2..=65).contains(&len) => {
            let b = read_body(r, len)?;
            let user = std::str::from_utf8(&b[1..]).map_err(|_| ProtoError::Bad("user"))?;
            if !user.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-')) || user.starts_with('-') {
                return Err(ProtoError::Bad("user"));
            }
            Ok(ToVdm::Login { user: user.to_string(), session: b[0] })
        }
        kind::CANNOT_LOCK if len <= MAX_TEXT => {
            let b = read_body(r, len)?;
            std::str::from_utf8(&b).map(|t| ToVdm::CannotLock(clean(t))).map_err(|_| ProtoError::Bad("text"))
        }
        _ => Err(ProtoError::Bad("kind")),
    }
}

pub fn send_from_vdm(w: &mut impl Write, m: &FromVdm) -> io::Result<()> {
    let text = |w: &mut dyn Write, k: u8, prefix: &[u8], t: &str| -> io::Result<()> {
        let t = clean(t);
        let n = (prefix.len() + t.len() + 1) as u16;
        w.write_all(&[(n >> 8) as u8, n as u8, k])?;
        w.write_all(prefix)?;
        w.write_all(t.as_bytes())
    };
    match m {
        FromVdm::Prompt { echo, text: t } => text(w, kind::PROMPT, &[u8::from(*echo)], t)?,
        FromVdm::Info(t) => text(w, kind::INFO, &[], t)?,
        FromVdm::Error(t) => text(w, kind::ERROR, &[], t)?,
        FromVdm::Done { ok, wait } => {
            header(w, kind::DONE, 3)?;
            w.write_all(&[u8::from(*ok), (*wait >> 8) as u8, *wait as u8])?;
        }
        FromVdm::LockNow => header(w, kind::LOCK_NOW, 0)?,
    }
    w.flush()
}

/// The client's side: the next message from vdm.
pub fn read_from_vdm(r: &mut impl Read) -> Result<FromVdm, ProtoError> {
    let (k, len) = read_header(r)?;
    let b = read_body(r, len)?;
    let text = |b: &[u8]| -> Result<String, ProtoError> {
        if b.len() > MAX_TEXT {
            return Err(ProtoError::Bad("text too long"));
        }
        std::str::from_utf8(b).map(clean).map_err(|_| ProtoError::Bad("text"))
    };
    match (k, b.as_slice()) {
        (kind::PROMPT, [echo @ (0 | 1), rest @ ..]) => Ok(FromVdm::Prompt { echo: *echo == 1, text: text(rest)? }),
        (kind::INFO, rest) => Ok(FromVdm::Info(text(rest)?)),
        (kind::ERROR, rest) => Ok(FromVdm::Error(text(rest)?)),
        (kind::DONE, [ok @ (0 | 1), hi, lo]) => Ok(FromVdm::Done { ok: *ok == 1, wait: u16::from_be_bytes([*hi, *lo]) }),
        (kind::LOCK_NOW, []) => Ok(FromVdm::LockNow),
        _ => Err(ProtoError::Bad("kind")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn messages_from_vdm_round_trip() {
        for m in [
            FromVdm::Prompt { echo: false, text: "Password: ".into() },
            FromVdm::Prompt { echo: true, text: "One-time code:".into() },
            FromVdm::Info("Touch your security key".into()),
            FromVdm::Error("Sorry".into()),
            FromVdm::Done { ok: true, wait: 0 },
            FromVdm::Done { ok: false, wait: 64 },
            FromVdm::LockNow,
        ] {
            let mut b = Vec::new();
            send_from_vdm(&mut b, &m).unwrap();
            assert_eq!(read_from_vdm(&mut Cursor::new(b)).unwrap(), m);
        }
    }

    #[test]
    fn messages_to_vdm_round_trip() {
        let mut b = Vec::new();
        send_to_vdm(&mut b, &ToVdm::Hello { version: VERSION, service: Service::Unlock }).unwrap();
        let mut s = Secret::new().unwrap();
        s.push_str("hunter2");
        send_to_vdm(&mut b, &ToVdm::Answer(s)).unwrap();
        send_to_vdm(&mut b, &ToVdm::Cancel).unwrap();
        send_to_vdm(&mut b, &ToVdm::Presented).unwrap();
        send_to_vdm(&mut b, &ToVdm::CannotLock("no shell\nhere".into())).unwrap();
        send_to_vdm(&mut b, &ToVdm::Login { user: "derickson".into(), session: 2 }).unwrap();
        let mut r = Cursor::new(b);
        assert!(matches!(read_to_vdm(&mut r).unwrap(), ToVdm::Hello { version: VERSION, service: Service::Unlock }));
        match read_to_vdm(&mut r).unwrap() {
            ToVdm::Answer(s) => assert_eq!(s.as_bytes(), b"hunter2"),
            _ => panic!("an answer"),
        }
        assert!(matches!(read_to_vdm(&mut r).unwrap(), ToVdm::Cancel));
        assert!(matches!(read_to_vdm(&mut r).unwrap(), ToVdm::Presented));
        assert!(matches!(read_to_vdm(&mut r).unwrap(), ToVdm::CannotLock(t) if t == "no shell here"));
        assert!(matches!(read_to_vdm(&mut r).unwrap(), ToVdm::Login { user, session: 2 } if user == "derickson"));
        // A name that could be read as an option, or with other bytes, is refused.
        for bad in ["-root", "a b", "x\n", ""] {
            let mut b = Vec::new();
            send_to_vdm(&mut b, &ToVdm::Login { user: bad.into(), session: 0 }).unwrap();
            assert!(read_to_vdm(&mut Cursor::new(b)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn text_is_cleaned() {
        assert_eq!(clean("a\x1b[2Jb\n"), "a [2Jb ");
        assert_eq!(clean(&"é".repeat(400)).len(), MAX_TEXT, "cut at a character");
    }

    #[test]
    fn bad_frames_are_refused() {
        // Length 0, longer than allowed, an unknown kind, a short Done, an answer too long.
        for bytes in [vec![0, 0, 1], vec![0x10, 0, 1], vec![0, 1, 99], vec![0, 2, kind::DONE, 1], vec![0x07, 0xff, kind::ANSWER]] {
            assert!(read_from_vdm(&mut Cursor::new(bytes.clone())).is_err());
            assert!(read_to_vdm(&mut Cursor::new(bytes)).is_err());
        }
    }

    /// Random bytes never panic either reader (threat model T13).
    #[test]
    fn fuzz_the_readers() {
        let iters: usize = std::env::var("FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000);
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..iters {
            let len = (next() % 64) as usize;
            let mut b: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            if b.len() >= 2 && next() % 2 == 0 {
                // A plausible length now and then, so bodies get read too.
                let n = (b.len() - 2) as u16;
                b[0] = (n >> 8) as u8;
                b[1] = n as u8;
            }
            let _ = read_from_vdm(&mut Cursor::new(b.clone()));
            let _ = read_to_vdm(&mut Cursor::new(b));
        }
    }
}
