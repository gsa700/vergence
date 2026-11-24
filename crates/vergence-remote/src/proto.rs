//! Vergence Remote's messages (remote milestone 1, step 2;
//! docs/remote-milestone-1.md). Both sides speak over one SSH channel (the
//! viewer runs `ssh <host> vergence-remote`; standard input and output are
//! the channel). Nothing listens on the network.
//!
//! The stream starts with the 8 bytes `VRMT` and the version (u32,
//! big-endian) from each side; versions must match exactly, as the bus's do.
//! Then messages: a type byte, a length (u32, big-endian) and that many
//! bytes. Numbers are big-endian; text is a u16 length and UTF-8.
//!
//! Every message from the other side is untrusted input: lengths are capped
//! ([`MAX_MESSAGE`]), unknown types are refused, a frame's rectangles must lie
//! inside its view and decode to exactly their size ([`Frame::apply`]).
//!
//! The viewer starts with all zeros (transparent black) in its picture: the
//! host sends only what differs from what it sent, so the first frame of a
//! view (and the first after a resize) is everything but zeros.

use std::io::{self, Read, Write};

use crate::lz;

pub const MAGIC: &[u8; 4] = b"VRMT";
/// Raised with every change to a message's layout.
pub const VERSION: u32 = 3;
/// The longest message either side accepts.
pub const MAX_MESSAGE: usize = 128 << 20;
/// The largest view, each way.
pub const MAX_SIDE: u32 = 16_384;
/// The most rectangles in one frame.
pub const MAX_RECTS: usize = 4096;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The other side broke the protocol: what, in words.
    Bad(&'static str),
    /// The other side speaks another version.
    Version(u32),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Bad(why) => write!(f, "the other side broke the protocol: {why}"),
            Error::Version(v) => write!(f, "the other side speaks version {v}, this one {VERSION}"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

impl From<lz::Bad> for Error {
    fn from(b: lz::Bad) -> Error {
        Error::Bad(b.0)
    }
}

/// How a rectangle's pixels are carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// RGBA as they are.
    Raw = 0,
    /// RGBA compressed as one LZ4-format block (lz.rs).
    Lz = 1,
}

/// One changed rectangle of a view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub encoding: Encoding,
    pub data: Vec<u8>,
}

impl Rect {
    /// A rectangle of RGBA pixels, compressed if that is smaller.
    pub fn new(x: u32, y: u32, w: u32, h: u32, rgba: &[u8]) -> Rect {
        let c = lz::compress(rgba);
        if c.len() < rgba.len() {
            Rect { x, y, w, h, encoding: Encoding::Lz, data: c }
        } else {
            Rect { x, y, w, h, encoding: Encoding::Raw, data: rgba.to_vec() }
        }
    }
}

/// A frame: the view's size and scale, and what changed in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Counts up; the viewer acknowledges it ([`ToHost::Ack`]).
    pub seq: u32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub rects: Vec<Rect>,
}

impl Frame {
    /// Each rectangle decoded into `picture` (RGBA, `width` x `height`),
    /// which is made anew (zeros) when the frame's size differs from it.
    /// Every rectangle is checked first: inside the view, and decoding to
    /// exactly its size; a frame with any bad rectangle changes nothing.
    pub fn apply(&self, picture: &mut Vec<u8>, size: &mut (u32, u32)) -> Result<(), Error> {
        if self.width == 0 || self.height == 0 || self.width > MAX_SIDE || self.height > MAX_SIDE {
            return Err(Error::Bad("a view of no size or too large"));
        }
        let mut decoded = Vec::with_capacity(self.rects.len());
        for r in &self.rects {
            if r.w == 0 || r.h == 0 {
                return Err(Error::Bad("an empty rectangle"));
            }
            let (right, bottom) = (r.x.checked_add(r.w), r.y.checked_add(r.h));
            if right.is_none_or(|v| v > self.width) || bottom.is_none_or(|v| v > self.height) {
                return Err(Error::Bad("a rectangle outside the view"));
            }
            let len = r.w as usize * r.h as usize * 4;
            let px = match r.encoding {
                Encoding::Raw if r.data.len() == len => r.data.clone(),
                Encoding::Raw => return Err(Error::Bad("a rectangle of the wrong size")),
                Encoding::Lz => lz::decompress(&r.data, len)?,
            };
            decoded.push(px);
        }
        let stride = self.width as usize * 4;
        if *size != (self.width, self.height) || picture.len() != stride * self.height as usize {
            *picture = vec![0; stride * self.height as usize];
            *size = (self.width, self.height);
        }
        for (r, px) in self.rects.iter().zip(decoded) {
            let row = r.w as usize * 4;
            for y in 0..r.h as usize {
                let at = (r.y as usize + y) * stride + r.x as usize * 4;
                picture[at..at + row].copy_from_slice(&px[y * row..(y + 1) * row]);
            }
        }
        Ok(())
    }
}

/// What the viewer sends.
#[derive(Debug, Clone, PartialEq)]
pub enum ToHost {
    /// First: the viewer's name (shown on the host's Dockworker tile), its
    /// size in pixels, and the screen it wants (empty: the main one).
    Hello {
        name: String,
        width: u32,
        height: u32,
        screen: String,
    },
    Resize {
        width: u32,
        height: u32,
    },
    /// Frame `seq` is shown: the host may send the next.
    Ack {
        seq: u32,
    },
    /// A key by its evdev code, down or up.
    Key {
        code: u32,
        pressed: bool,
    },
    /// The pointer at a place in the view's pixels.
    Pointer {
        x: f32,
        y: f32,
    },
    Button {
        code: u32,
        pressed: bool,
    },
    /// Scrolling, in the view's pixels (or steps for a wheel's clicks).
    Scroll {
        dx: f32,
        dy: f32,
    },
    Bye,
    /// Blank the desk (version 2): the host's monitors locked behind a
    /// curtain while this viewer goes on using the session; or lift it.
    Privacy {
        on: bool,
    },
    /// The viewer's Caps Lock and Num Lock (version 3): the host sets its own
    /// to match, so letters come out as the viewer's keyboard shows.
    Locks {
        caps: bool,
        num: bool,
    },
}

/// What the host sends.
#[derive(Debug, Clone, PartialEq)]
pub enum ToViewer {
    /// The host's name and its screens (names and logical sizes).
    Welcome {
        host: String,
        screens: Vec<(String, u32, u32)>,
    },
    Frame(Frame),
    /// Something to say ("Ended from the desk").
    Notice {
        text: String,
    },
    Bye {
        reason: String,
    },
    /// Whether the desk is blanked now (version 2): the answer to
    /// [`ToHost::Privacy`], and `false` when the curtain went.
    Privacy {
        on: bool,
    },
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let b = &s.as_bytes()[..s.len().min(u16::MAX as usize)];
    out.extend((b.len() as u16).to_be_bytes());
    out.extend(b);
}

/// Reads fields from a message's bytes, refusing to run past its end.
struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], Error> {
        if self.0.len() < n {
            return Err(Error::Bad("a message shorter than its fields"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f32(&mut self) -> Result<f32, Error> {
        let v = f32::from_bits(self.u32()?);
        if !v.is_finite() {
            return Err(Error::Bad("a number that is not a number"));
        }
        Ok(v)
    }
    fn str(&mut self) -> Result<String, Error> {
        let n = self.u16()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| Error::Bad("text that is not UTF-8"))
    }
    fn done(&self) -> Result<(), Error> {
        if self.0.is_empty() { Ok(()) } else { Err(Error::Bad("a message longer than its fields")) }
    }
}

fn frame_out(w: &mut impl Write, kind: u8, body: &[u8]) -> io::Result<()> {
    if body.len() > MAX_MESSAGE {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "a message too long"));
    }
    w.write_all(&[kind])?;
    w.write_all(&(body.len() as u32).to_be_bytes())?;
    w.write_all(body)
}

fn frame_in(r: &mut impl Read) -> Result<(u8, Vec<u8>), Error> {
    let mut head = [0u8; 5];
    r.read_exact(&mut head)?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > MAX_MESSAGE {
        return Err(Error::Bad("a message too long"));
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    Ok((head[0], body))
}

/// The opening: magic and version, sent by each side.
pub fn write_opening(w: &mut impl Write) -> io::Result<()> {
    w.write_all(MAGIC)?;
    w.write_all(&VERSION.to_be_bytes())?;
    w.flush()
}

pub fn read_opening(r: &mut impl Read) -> Result<(), Error> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    if &b[..4] != MAGIC {
        return Err(Error::Bad("not Vergence Remote"));
    }
    let v = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    if v != VERSION {
        return Err(Error::Version(v));
    }
    Ok(())
}

impl ToHost {
    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        let mut b = Vec::new();
        let kind = match self {
            ToHost::Hello { name, width, height, screen } => {
                put_str(&mut b, name);
                b.extend(width.to_be_bytes());
                b.extend(height.to_be_bytes());
                put_str(&mut b, screen);
                1
            }
            ToHost::Resize { width, height } => {
                b.extend(width.to_be_bytes());
                b.extend(height.to_be_bytes());
                2
            }
            ToHost::Ack { seq } => {
                b.extend(seq.to_be_bytes());
                3
            }
            ToHost::Key { code, pressed } => {
                b.extend(code.to_be_bytes());
                b.push(*pressed as u8);
                4
            }
            ToHost::Pointer { x, y } => {
                b.extend(x.to_bits().to_be_bytes());
                b.extend(y.to_bits().to_be_bytes());
                5
            }
            ToHost::Button { code, pressed } => {
                b.extend(code.to_be_bytes());
                b.push(*pressed as u8);
                6
            }
            ToHost::Scroll { dx, dy } => {
                b.extend(dx.to_bits().to_be_bytes());
                b.extend(dy.to_bits().to_be_bytes());
                7
            }
            ToHost::Bye => 8,
            ToHost::Privacy { on } => {
                b.push(*on as u8);
                9
            }
            ToHost::Locks { caps, num } => {
                b.push(*caps as u8);
                b.push(*num as u8);
                10
            }
        };
        frame_out(w, kind, &b)
    }

    pub fn read(r: &mut impl Read) -> Result<ToHost, Error> {
        let (kind, body) = frame_in(r)?;
        let mut f = Fields(&body);
        let size = |f: &mut Fields| -> Result<(u32, u32), Error> {
            let (w, h) = (f.u32()?, f.u32()?);
            if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
                return Err(Error::Bad("a viewer of no size or too large"));
            }
            Ok((w, h))
        };
        let m = match kind {
            1 => {
                let name = f.str()?;
                let (width, height) = size(&mut f)?;
                ToHost::Hello { name, width, height, screen: f.str()? }
            }
            2 => {
                let (width, height) = size(&mut f)?;
                ToHost::Resize { width, height }
            }
            3 => ToHost::Ack { seq: f.u32()? },
            4 => ToHost::Key { code: f.u32()?, pressed: f.u8()? != 0 },
            5 => ToHost::Pointer { x: f.f32()?, y: f.f32()? },
            6 => ToHost::Button { code: f.u32()?, pressed: f.u8()? != 0 },
            7 => ToHost::Scroll { dx: f.f32()?, dy: f.f32()? },
            8 => ToHost::Bye,
            9 => ToHost::Privacy { on: f.u8()? != 0 },
            10 => ToHost::Locks { caps: f.u8()? != 0, num: f.u8()? != 0 },
            _ => return Err(Error::Bad("an unknown message")),
        };
        f.done()?;
        Ok(m)
    }
}

impl ToViewer {
    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        let mut b = Vec::new();
        let kind = match self {
            ToViewer::Welcome { host, screens } => {
                put_str(&mut b, host);
                b.extend((screens.len().min(u16::MAX as usize) as u16).to_be_bytes());
                for (name, w, h) in screens.iter().take(u16::MAX as usize) {
                    put_str(&mut b, name);
                    b.extend(w.to_be_bytes());
                    b.extend(h.to_be_bytes());
                }
                1
            }
            ToViewer::Frame(fr) => {
                b.extend(fr.seq.to_be_bytes());
                b.extend(fr.width.to_be_bytes());
                b.extend(fr.height.to_be_bytes());
                b.extend(fr.scale.to_bits().to_be_bytes());
                b.extend((fr.rects.len() as u32).to_be_bytes());
                for r in &fr.rects {
                    for v in [r.x, r.y, r.w, r.h] {
                        b.extend(v.to_be_bytes());
                    }
                    b.push(r.encoding as u8);
                    b.extend((r.data.len() as u32).to_be_bytes());
                    b.extend(&r.data);
                }
                2
            }
            ToViewer::Notice { text } => {
                put_str(&mut b, text);
                3
            }
            ToViewer::Bye { reason } => {
                put_str(&mut b, reason);
                4
            }
            ToViewer::Privacy { on } => {
                b.push(*on as u8);
                5
            }
        };
        frame_out(w, kind, &b)
    }

    pub fn read(r: &mut impl Read) -> Result<ToViewer, Error> {
        let (kind, body) = frame_in(r)?;
        let mut f = Fields(&body);
        let m = match kind {
            1 => {
                let host = f.str()?;
                let n = f.u16()? as usize;
                let mut screens = Vec::with_capacity(n.min(64));
                for _ in 0..n {
                    screens.push((f.str()?, f.u32()?, f.u32()?));
                }
                ToViewer::Welcome { host, screens }
            }
            2 => {
                let (seq, width, height, scale) = (f.u32()?, f.u32()?, f.u32()?, f.f32()?);
                let n = f.u32()? as usize;
                if n > MAX_RECTS {
                    return Err(Error::Bad("too many rectangles"));
                }
                let mut rects = Vec::with_capacity(n);
                for _ in 0..n {
                    let (x, y, w, h) = (f.u32()?, f.u32()?, f.u32()?, f.u32()?);
                    let encoding = match f.u8()? {
                        0 => Encoding::Raw,
                        1 => Encoding::Lz,
                        _ => return Err(Error::Bad("an unknown encoding")),
                    };
                    let len = f.u32()? as usize;
                    rects.push(Rect { x, y, w, h, encoding, data: f.take(len)?.to_vec() });
                }
                ToViewer::Frame(Frame { seq, width, height, scale, rects })
            }
            3 => ToViewer::Notice { text: f.str()? },
            4 => ToViewer::Bye { reason: f.str()? },
            5 => ToViewer::Privacy { on: f.u8()? != 0 },
            _ => return Err(Error::Bad("an unknown message")),
        };
        f.done()?;
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_go_and_come_back() {
        let to_host = [
            ToHost::Hello { name: "Control".into(), width: 2880, height: 1800, screen: String::new() },
            ToHost::Resize { width: 1280, height: 720 },
            ToHost::Ack { seq: 7 },
            ToHost::Key { code: 30, pressed: true },
            ToHost::Pointer { x: 10.5, y: 20.25 },
            ToHost::Button { code: 0x110, pressed: false },
            ToHost::Scroll { dx: 0.0, dy: -3.0 },
            ToHost::Bye,
            ToHost::Privacy { on: true },
            ToHost::Locks { caps: true, num: false },
        ];
        let mut b = Vec::new();
        for m in &to_host {
            m.write(&mut b).unwrap();
        }
        let mut r = b.as_slice();
        for m in &to_host {
            assert_eq!(&ToHost::read(&mut r).unwrap(), m);
        }
        let px = vec![200u8; 8 * 4 * 4];
        let to_viewer = [
            ToViewer::Welcome { host: "hubble".into(), screens: vec![("eDP-1".into(), 1920, 1080)] },
            ToViewer::Frame(Frame {
                seq: 1,
                width: 64,
                height: 32,
                scale: 1.5,
                rects: vec![Rect::new(2, 3, 8, 4, &px)],
            }),
            ToViewer::Notice { text: "Ended from the desk".into() },
            ToViewer::Bye { reason: "bye".into() },
            ToViewer::Privacy { on: false },
        ];
        let mut b = Vec::new();
        for m in &to_viewer {
            m.write(&mut b).unwrap();
        }
        let mut r = b.as_slice();
        for m in &to_viewer {
            assert_eq!(&ToViewer::read(&mut r).unwrap(), m);
        }
    }

    #[test]
    fn the_opening_checks_the_version() {
        let mut b = Vec::new();
        write_opening(&mut b).unwrap();
        assert!(read_opening(&mut b.as_slice()).is_ok());
        let mut other = b.clone();
        other[7] = 99;
        assert!(matches!(read_opening(&mut other.as_slice()), Err(Error::Version(_))));
        assert!(matches!(read_opening(&mut &b"SSH-2.0-"[..]), Err(Error::Bad(_))));
    }

    #[test]
    fn a_frame_applies_and_hostile_ones_change_nothing() {
        let red = [255u8, 0, 0, 255].repeat(4 * 2);
        let ok = Frame { seq: 1, width: 8, height: 4, scale: 1.0, rects: vec![Rect::new(1, 1, 4, 2, &red)] };
        let (mut pic, mut size) = (Vec::new(), (0, 0));
        ok.apply(&mut pic, &mut size).unwrap();
        assert_eq!(size, (8, 4));
        assert_eq!(&pic[(8 + 1) * 4..(8 + 1) * 4 + 4], &[255, 0, 0, 255]);
        assert_eq!(&pic[0..4], &[0, 0, 0, 0], "outside the rectangle: zeros");
        let before = pic.clone();
        let bad = |rects: Vec<Rect>| Frame { seq: 2, width: 8, height: 4, scale: 1.0, rects };
        let outside = Rect { x: 6, y: 0, w: 4, h: 1, encoding: Encoding::Raw, data: vec![0; 16] };
        let wrap = Rect { x: u32::MAX, y: 0, w: 2, h: 1, encoding: Encoding::Raw, data: vec![0; 8] };
        let short = Rect { x: 0, y: 0, w: 2, h: 2, encoding: Encoding::Raw, data: vec![0; 15] };
        let lying = Rect { x: 0, y: 0, w: 2, h: 2, encoding: Encoding::Lz, data: lz::compress(&[1; 8]) };
        for r in [outside, wrap, short, lying] {
            // A good rectangle first: the bad one after it must stop the whole frame.
            assert!(bad(vec![Rect::new(0, 0, 1, 1, &[9, 9, 9, 9]), r]).apply(&mut pic, &mut size).is_err());
            assert_eq!(pic, before, "a bad frame changes nothing");
        }
        let huge = Frame { seq: 3, width: MAX_SIDE + 1, height: 4, scale: 1.0, rects: Vec::new() };
        assert!(huge.apply(&mut pic, &mut size).is_err());
    }

    #[test]
    fn hostile_messages_are_refused() {
        // Too long, unknown, short, long, not UTF-8, not a number, a huge viewer.
        let mut too_long = vec![2u8];
        too_long.extend(((MAX_MESSAGE + 1) as u32).to_be_bytes());
        assert!(ToViewer::read(&mut too_long.as_slice()).is_err());
        assert!(ToHost::read(&mut [99u8, 0, 0, 0, 0].as_slice()).is_err());
        assert!(ToHost::read(&mut [3u8, 0, 0, 0, 2, 0, 0].as_slice()).is_err());
        assert!(ToHost::read(&mut [8u8, 0, 0, 0, 1, 0].as_slice()).is_err());
        assert!(ToViewer::read(&mut [3u8, 0, 0, 0, 4, 0, 2, 0xff, 0xfe].as_slice()).is_err());
        let mut nan = vec![5u8, 0, 0, 0, 8];
        nan.extend(f32::NAN.to_bits().to_be_bytes());
        nan.extend(1.0f32.to_bits().to_be_bytes());
        assert!(ToHost::read(&mut nan.as_slice()).is_err());
        let mut big = vec![2u8, 0, 0, 0, 8];
        big.extend((MAX_SIDE + 1).to_be_bytes());
        big.extend(10u32.to_be_bytes());
        assert!(ToHost::read(&mut big.as_slice()).is_err());
        // A frame claiming more rectangles than allowed.
        let mut many = Vec::new();
        for v in [1u32, 8, 4] {
            many.extend(v.to_be_bytes());
        }
        many.extend(1.0f32.to_bits().to_be_bytes());
        many.extend(((MAX_RECTS + 1) as u32).to_be_bytes());
        let mut msg = vec![2u8];
        msg.extend((many.len() as u32).to_be_bytes());
        msg.extend(many);
        assert!(ToViewer::read(&mut msg.as_slice()).is_err());
    }
}
