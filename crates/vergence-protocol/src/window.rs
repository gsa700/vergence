//! Windows on the native protocol (whitepaper §6.3 to §6.5; native milestone
//! 1): a program's windows, their pictures, the input the user gives them,
//! the clipboard, where they ask to be (Contract 2) and what they mean
//! (Contract 3). On the same connection as the session bus (bus.rs), with
//! opcodes from 100 up.
//!
//! **File descriptors** travel beside the frame (sock.rs). A message that
//! carries any says how many as its first field, a `u8`, and they are taken
//! in the order they arrived; a message claiming descriptors that did not
//! come is refused, and the connection with it.
//!
//! Sizes and places are in **reference units** (Contract 1), as `f64`,
//! except a picture's own size and its damage, which are its pixels.

use std::{
    fmt,
    os::fd::{AsRawFd, OwnedFd, RawFd},
    sync::Arc,
};

use crate::wire::{Decoder, Encoder, WireError};

/// A file descriptor in a message. Shared, so a message can be cloned; equal
/// when it is the same descriptor.
#[derive(Clone)]
pub struct Fd(pub Arc<OwnedFd>);

impl Fd {
    pub fn new(fd: OwnedFd) -> Fd {
        Fd(Arc::new(fd))
    }

    pub fn raw(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl fmt::Debug for Fd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fd({})", self.raw())
    }
}

impl PartialEq for Fd {
    fn eq(&self, other: &Fd) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Where descriptors for a message being read come from: the ones that
/// arrived, in order.
pub trait FdSource {
    fn take_fd(&mut self) -> Option<OwnedFd>;
}

impl FdSource for std::collections::VecDeque<OwnedFd> {
    fn take_fd(&mut self) -> Option<OwnedFd> {
        self.pop_front()
    }
}

/// No descriptors at all (a reader that cannot have any).
pub struct NoFds;

impl FdSource for NoFds {
    fn take_fd(&mut self) -> Option<OwnedFd> {
        None
    }
}

// Limits, besides the frame's 64 KB.
pub const MAX_DAMAGE: usize = 256;
pub const MAX_PLANES: usize = 4;
pub const MAX_KINDS: usize = 32;
pub const MAX_NODES: usize = 4096;
pub const MAX_STATES: usize = 16;
/// A picture's longest side, in pixels.
pub const MAX_SIDE: u32 = 16_384;

/// Shared-memory picture formats, as DRM fourcc codes (the same numbering as
/// a GPU buffer's): four bytes a pixel, blue, green, red, then alpha (or
/// unused), little-endian; alpha premultiplied. The server shows these two.
pub const SHM_ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
pub const SHM_XRGB8888: u32 = u32::from_le_bytes(*b"XR24");

/// A part of a picture that changed, in its pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Damage {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// One plane of a GPU buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct Plane {
    pub fd: Fd,
    pub offset: u32,
    pub stride: u32,
}

/// A window's picture: shared memory, or a GPU buffer (linux-dmabuf's
/// terms: a fourcc format, a modifier, its planes).
#[derive(Debug, Clone, PartialEq)]
pub enum Buffer {
    Shm { fd: Fd, width: u32, height: u32, stride: u32, format: u32 },
    Dmabuf { planes: Vec<Plane>, width: u32, height: u32, format: u32, modifier: u64 },
}

/// Where a window asks to be (Contract 2): the server decides.
#[derive(Debug, Clone, PartialEq)]
pub enum Place {
    /// Wherever the server thinks best (the monitor under the pointer, or
    /// where this window was last on this monitor setup).
    Anywhere,
    /// At a point of a monitor, by its id, from its top left corner.
    Monitor { id: String, x: f64, y: f64 },
    /// At a point from its parent window's top left corner.
    Parent { x: f64, y: f64 },
}

/// One widget of a window's meaning (Contract 3), as the toolkit lays it out.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Stable between updates; the parent's, 0 for the window itself.
    pub id: u64,
    pub parent: u64,
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    pub states: Vec<String>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// What a client asks of its windows.
#[derive(Debug, Clone, PartialEq)]
pub enum WinRequest {
    /// A new window, `window` an id the client chose (not 0). Its identity
    /// (Contract 2): the connection's application id, `role`, and `token`
    /// (empty, or one the server gave the same window on an earlier run).
    /// Reply: [`WinEvent::Created`], then a [`WinEvent::Configure`].
    Create { serial: u32, window: u32, role: String, token: String, title: String, width: f64, height: f64, own_frame: bool, parent: u32, place: Place },
    Destroy { window: u32 },
    Title { window: u32, title: String },
    /// The configure with this serial is drawn for.
    AckConfigure { window: u32, serial: u32 },
    /// A new picture, and what changed since the last one (empty: all of it).
    Attach { window: u32, buffer: Buffer, damage: Vec<Damage> },
    /// The pointer's shape over this window (the cursor-shape protocol's numbers).
    Cursor { window: u32, shape: u16 },
    /// Move or resize the window with the press of `input_serial`; `edges`
    /// top 1, bottom 2, left 4, right 8.
    Move { window: u32, input_serial: u32 },
    Resize { window: u32, input_serial: u32, edges: u8 },
    Maximize { window: u32, on: bool },
    Minimize { window: u32 },
    /// Ask again where to be (Contract 2).
    Place { window: u32, place: Place },
    /// The window's meaning now (Contract 3): its widgets, the focused one
    /// (0: none) and the text caret, if a field has one.
    Meaning { window: u32, focus: u64, caret: Option<(f64, f64, f64, f64)>, nodes: Vec<Node> },
    /// This client has what is to be pasted, in these kinds of data, in
    /// answer to the press or key of `input_serial`.
    Copy { input_serial: u32, kinds: Vec<String> },
    /// The clipboard's data in this kind, please. Reply: [`WinEvent::ClipboardData`].
    Paste { serial: u32, kind: String },
}

/// What the server tells a client about its windows.
#[derive(Debug, Clone, PartialEq)]
pub enum WinEvent {
    /// The window exists; `token` names it on later runs (Contract 2).
    Created { serial: u32, window: u32, token: String },
    /// Draw at this size, for this scale (Contract 1); answer with AckConfigure.
    Configure { window: u32, serial: u32, width: f64, height: f64, scale: f64, maximized: bool, activated: bool },
    /// The user asked for the window to close.
    Close { window: u32 },
    /// The last picture is on the screen: draw the next when there is one.
    Frame { window: u32, time_ms: u32 },
    PointerEnter { window: u32, serial: u32, x: f64, y: f64 },
    PointerLeave { window: u32 },
    PointerMotion { window: u32, time_ms: u32, x: f64, y: f64 },
    PointerButton { window: u32, serial: u32, time_ms: u32, button: u32, pressed: bool },
    /// Scrolling: `source` 0 wheel, 1 finger, 2 continuous, 3 wheel tilt;
    /// the amounts in reference units, and in 120ths of a notch for a wheel.
    PointerAxis { window: u32, time_ms: u32, source: u8, dx: f64, dy: f64, v120x: i32, v120y: i32 },
    /// The keymap (xkb's text format, `size` bytes, in a descriptor).
    Keymap { fd: Fd, size: u32 },
    KeyboardEnter { window: u32, serial: u32 },
    KeyboardLeave { window: u32 },
    Key { window: u32, serial: u32, time_ms: u32, key: u32, pressed: bool },
    Modifiers { depressed: u32, latched: u32, locked: u32, group: u32 },
    /// Repeats a second (0: none) and the delay before the first, in ms.
    Repeat { rate: i32, delay: i32 },
    /// A finger: `phase` 0 down, 1 moved, 2 up, 3 canceled.
    Touch { window: u32, time_ms: u32, id: i32, phase: u8, x: f64, y: f64 },
    /// There is something to paste, in these kinds.
    ClipboardOffer { kinds: Vec<String> },
    /// Someone is pasting what this client copied: write it in `kind` to `fd`, then close it.
    ClipboardSend { kind: String, fd: Fd },
    /// What was asked for with Paste, to be read from `fd`.
    ClipboardData { serial: u32, fd: Fd },
}

pub(crate) mod op {
    // Client to server.
    pub const CREATE: u16 = 100;
    pub const DESTROY: u16 = 101;
    pub const TITLE: u16 = 102;
    pub const ACK_CONFIGURE: u16 = 103;
    pub const ATTACH: u16 = 104;
    pub const CURSOR: u16 = 105;
    pub const MOVE: u16 = 106;
    pub const RESIZE: u16 = 107;
    pub const MAXIMIZE: u16 = 108;
    pub const MINIMIZE: u16 = 109;
    pub const PLACE: u16 = 110;
    pub const MEANING: u16 = 111;
    pub const COPY: u16 = 112;
    pub const PASTE: u16 = 113;
    // Server to client.
    pub const CREATED: u16 = 100;
    pub const CONFIGURE: u16 = 101;
    pub const CLOSE: u16 = 102;
    pub const FRAME: u16 = 103;
    pub const POINTER_ENTER: u16 = 104;
    pub const POINTER_LEAVE: u16 = 105;
    pub const POINTER_MOTION: u16 = 106;
    pub const POINTER_BUTTON: u16 = 107;
    pub const POINTER_AXIS: u16 = 108;
    pub const KEYMAP: u16 = 109;
    pub const KEYBOARD_ENTER: u16 = 110;
    pub const KEYBOARD_LEAVE: u16 = 111;
    pub const KEY: u16 = 112;
    pub const MODIFIERS: u16 = 113;
    pub const REPEAT: u16 = 114;
    pub const TOUCH: u16 = 115;
    pub const CLIPBOARD_OFFER: u16 = 116;
    pub const CLIPBOARD_SEND: u16 = 117;
    pub const CLIPBOARD_DATA: u16 = 118;

    /// Whether an opcode is a window message's (either way).
    pub fn is_window(opcode: u16) -> bool {
        (100..200).contains(&opcode)
    }
}

pub use op::is_window as is_window_opcode;

fn take(fds: &mut dyn FdSource, d: &mut Decoder) -> Result<Vec<Fd>, WireError> {
    let n = d.u8()? as usize;
    if n > crate::sock::MAX_FDS {
        return Err(WireError::BadValue("descriptor count"));
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(Fd::new(fds.take_fd().ok_or(WireError::BadValue("a descriptor that did not come"))?));
    }
    Ok(out)
}

fn finite(v: f64, what: &'static str) -> Result<f64, WireError> {
    if v.is_finite() && v.abs() <= 1e7 { Ok(v) } else { Err(WireError::BadValue(what)) }
}

fn put_place(e: &mut Encoder, p: &Place) {
    match p {
        Place::Anywhere => e.u8(0),
        Place::Monitor { id, x, y } => e.u8(1).str(id).f64(*x).f64(*y),
        Place::Parent { x, y } => e.u8(2).f64(*x).f64(*y),
    };
}

fn get_place(d: &mut Decoder) -> Result<Place, WireError> {
    Ok(match d.u8()? {
        0 => Place::Anywhere,
        1 => Place::Monitor { id: d.str()?, x: finite(d.f64()?, "place")?, y: finite(d.f64()?, "place")? },
        2 => Place::Parent { x: finite(d.f64()?, "place")?, y: finite(d.f64()?, "place")? },
        _ => return Err(WireError::BadValue("place")),
    })
}

fn get_kinds(d: &mut Decoder) -> Result<Vec<String>, WireError> {
    let n = d.count(4)?;
    if n > MAX_KINDS {
        return Err(WireError::TooLong(n));
    }
    (0..n).map(|_| d.str()).collect()
}

fn put_kinds(e: &mut Encoder, kinds: &[String]) {
    e.count(kinds.len());
    for k in kinds {
        e.str(k);
    }
}

fn size(w: u32, h: u32) -> Result<(), WireError> {
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE { Err(WireError::BadValue("picture size")) } else { Ok(()) }
}

impl WinRequest {
    /// The frame, and the descriptors to send beside it (borrowed from the
    /// message, which must outlive the write).
    pub fn encode(&self) -> Result<(Vec<u8>, Vec<RawFd>), WireError> {
        let mut fds = Vec::new();
        let e = match self {
            WinRequest::Create { serial, window, role, token, title, width, height, own_frame, parent, place } => {
                let mut e = Encoder::frame(op::CREATE);
                e.u32(*serial).u32(*window).str(role).str(token).str(title).f64(*width).f64(*height).bool(*own_frame).u32(*parent);
                put_place(&mut e, place);
                e
            }
            WinRequest::Destroy { window } => {
                let mut e = Encoder::frame(op::DESTROY);
                e.u32(*window);
                e
            }
            WinRequest::Title { window, title } => {
                let mut e = Encoder::frame(op::TITLE);
                e.u32(*window).str(title);
                e
            }
            WinRequest::AckConfigure { window, serial } => {
                let mut e = Encoder::frame(op::ACK_CONFIGURE);
                e.u32(*window).u32(*serial);
                e
            }
            WinRequest::Attach { window, buffer, damage } => {
                let mut e = Encoder::frame(op::ATTACH);
                match buffer {
                    Buffer::Shm { fd, width, height, stride, format } => {
                        fds.push(fd.raw());
                        e.u8(1).u32(*window).u8(0).u32(*width).u32(*height).u32(*stride).u32(*format);
                    }
                    Buffer::Dmabuf { planes, width, height, format, modifier } => {
                        fds.extend(planes.iter().map(|p| p.fd.raw()));
                        e.u8(planes.len() as u8).u32(*window).u8(1).u32(*width).u32(*height).u32(*format).u64(*modifier);
                        for p in planes {
                            e.u32(p.offset).u32(p.stride);
                        }
                    }
                }
                e.count(damage.len());
                for d in damage {
                    e.i32(d.x).i32(d.y).i32(d.w).i32(d.h);
                }
                e
            }
            WinRequest::Cursor { window, shape } => {
                let mut e = Encoder::frame(op::CURSOR);
                e.u32(*window).u16(*shape);
                e
            }
            WinRequest::Move { window, input_serial } => {
                let mut e = Encoder::frame(op::MOVE);
                e.u32(*window).u32(*input_serial);
                e
            }
            WinRequest::Resize { window, input_serial, edges } => {
                let mut e = Encoder::frame(op::RESIZE);
                e.u32(*window).u32(*input_serial).u8(*edges);
                e
            }
            WinRequest::Maximize { window, on } => {
                let mut e = Encoder::frame(op::MAXIMIZE);
                e.u32(*window).bool(*on);
                e
            }
            WinRequest::Minimize { window } => {
                let mut e = Encoder::frame(op::MINIMIZE);
                e.u32(*window);
                e
            }
            WinRequest::Place { window, place } => {
                let mut e = Encoder::frame(op::PLACE);
                e.u32(*window);
                put_place(&mut e, place);
                e
            }
            WinRequest::Meaning { window, focus, caret, nodes } => {
                let mut e = Encoder::frame(op::MEANING);
                e.u32(*window).u64(*focus);
                match caret {
                    Some((x, y, w, h)) => e.bool(true).f64(*x).f64(*y).f64(*w).f64(*h),
                    None => e.bool(false),
                };
                e.count(nodes.len());
                for n in nodes {
                    e.u64(n.id).u64(n.parent).str(&n.role).str(&n.name);
                    match &n.value {
                        Some(v) => e.bool(true).str(v),
                        None => e.bool(false),
                    };
                    e.count(n.states.len());
                    for s in &n.states {
                        e.str(s);
                    }
                    e.f64(n.x).f64(n.y).f64(n.w).f64(n.h);
                }
                e
            }
            WinRequest::Copy { input_serial, kinds } => {
                let mut e = Encoder::frame(op::COPY);
                e.u32(*input_serial);
                put_kinds(&mut e, kinds);
                e
            }
            WinRequest::Paste { serial, kind } => {
                let mut e = Encoder::frame(op::PASTE);
                e.u32(*serial).str(kind);
                e
            }
        };
        Ok((e.finish()?, fds))
    }

    /// A window request from a frame's payload, taking the descriptors it
    /// carries from `fds`.
    pub fn decode(opcode: u16, payload: &[u8], fds: &mut dyn FdSource) -> Result<WinRequest, WireError> {
        let mut d = Decoder::new(payload);
        Ok(match opcode {
            op::CREATE => WinRequest::Create {
                serial: d.u32()?,
                window: d.u32()?,
                role: d.str()?,
                token: d.str()?,
                title: d.str()?,
                width: finite(d.f64()?, "width")?,
                height: finite(d.f64()?, "height")?,
                own_frame: d.bool()?,
                parent: d.u32()?,
                place: get_place(&mut d)?,
            },
            op::DESTROY => WinRequest::Destroy { window: d.u32()? },
            op::TITLE => WinRequest::Title { window: d.u32()?, title: d.str()? },
            op::ACK_CONFIGURE => WinRequest::AckConfigure { window: d.u32()?, serial: d.u32()? },
            op::ATTACH => {
                let mut got = take(fds, &mut d)?;
                let window = d.u32()?;
                let buffer = match d.u8()? {
                    0 => {
                        let (width, height, stride, format) = (d.u32()?, d.u32()?, d.u32()?, d.u32()?);
                        size(width, height)?;
                        if got.len() != 1 || (stride as u64) < width as u64 * 4 || stride as u64 * height as u64 > 1 << 30 {
                            return Err(WireError::BadValue("shared memory picture"));
                        }
                        Buffer::Shm { fd: got.remove(0), width, height, stride, format }
                    }
                    1 => {
                        let (width, height, format, modifier) = (d.u32()?, d.u32()?, d.u32()?, d.u64()?);
                        size(width, height)?;
                        if got.is_empty() || got.len() > MAX_PLANES {
                            return Err(WireError::BadValue("GPU picture planes"));
                        }
                        let mut planes = Vec::with_capacity(got.len());
                        for fd in got {
                            planes.push(Plane { fd, offset: d.u32()?, stride: d.u32()? });
                        }
                        Buffer::Dmabuf { planes, width, height, format, modifier }
                    }
                    _ => return Err(WireError::BadValue("picture kind")),
                };
                let n = d.count(16)?;
                if n > MAX_DAMAGE {
                    return Err(WireError::TooLong(n));
                }
                let damage = (0..n).map(|_| Ok(Damage { x: d.i32()?, y: d.i32()?, w: d.i32()?, h: d.i32()? })).collect::<Result<_, WireError>>()?;
                WinRequest::Attach { window, buffer, damage }
            }
            op::CURSOR => WinRequest::Cursor { window: d.u32()?, shape: d.u16()? },
            op::MOVE => WinRequest::Move { window: d.u32()?, input_serial: d.u32()? },
            op::RESIZE => WinRequest::Resize { window: d.u32()?, input_serial: d.u32()?, edges: d.u8()? },
            op::MAXIMIZE => WinRequest::Maximize { window: d.u32()?, on: d.bool()? },
            op::MINIMIZE => WinRequest::Minimize { window: d.u32()? },
            op::PLACE => WinRequest::Place { window: d.u32()?, place: get_place(&mut d)? },
            op::MEANING => {
                let window = d.u32()?;
                let focus = d.u64()?;
                let caret = if d.bool()? { Some((finite(d.f64()?, "caret")?, finite(d.f64()?, "caret")?, finite(d.f64()?, "caret")?, finite(d.f64()?, "caret")?)) } else { None };
                let n = d.count(16 + 8 + 1 + 4 + 32)?;
                if n > MAX_NODES {
                    return Err(WireError::TooLong(n));
                }
                let mut nodes = Vec::with_capacity(n);
                for _ in 0..n {
                    let (id, parent, role, name) = (d.u64()?, d.u64()?, d.str()?, d.str()?);
                    let value = if d.bool()? { Some(d.str()?) } else { None };
                    let ns = d.count(4)?;
                    if ns > MAX_STATES {
                        return Err(WireError::TooLong(ns));
                    }
                    let states = (0..ns).map(|_| d.str()).collect::<Result<_, _>>()?;
                    let (x, y, w, h) = (finite(d.f64()?, "bounds")?, finite(d.f64()?, "bounds")?, finite(d.f64()?, "bounds")?, finite(d.f64()?, "bounds")?);
                    nodes.push(Node { id, parent, role, name, value, states, x, y, w, h });
                }
                WinRequest::Meaning { window, focus, caret, nodes }
            }
            op::COPY => WinRequest::Copy { input_serial: d.u32()?, kinds: get_kinds(&mut d)? },
            op::PASTE => WinRequest::Paste { serial: d.u32()?, kind: d.str()? },
            other => return Err(WireError::UnknownOpcode(other)),
        })
    }

    /// The serial of a request that has a reply.
    pub fn serial(&self) -> Option<u32> {
        match self {
            WinRequest::Create { serial, .. } | WinRequest::Paste { serial, .. } => Some(*serial),
            _ => None,
        }
    }
}

impl WinEvent {
    pub fn encode(&self) -> Result<(Vec<u8>, Vec<RawFd>), WireError> {
        let mut fds = Vec::new();
        let e = match self {
            WinEvent::Created { serial, window, token } => {
                let mut e = Encoder::frame(op::CREATED);
                e.u32(*serial).u32(*window).str(token);
                e
            }
            WinEvent::Configure { window, serial, width, height, scale, maximized, activated } => {
                let mut e = Encoder::frame(op::CONFIGURE);
                e.u32(*window).u32(*serial).f64(*width).f64(*height).f64(*scale).bool(*maximized).bool(*activated);
                e
            }
            WinEvent::Close { window } => {
                let mut e = Encoder::frame(op::CLOSE);
                e.u32(*window);
                e
            }
            WinEvent::Frame { window, time_ms } => {
                let mut e = Encoder::frame(op::FRAME);
                e.u32(*window).u32(*time_ms);
                e
            }
            WinEvent::PointerEnter { window, serial, x, y } => {
                let mut e = Encoder::frame(op::POINTER_ENTER);
                e.u32(*window).u32(*serial).f64(*x).f64(*y);
                e
            }
            WinEvent::PointerLeave { window } => {
                let mut e = Encoder::frame(op::POINTER_LEAVE);
                e.u32(*window);
                e
            }
            WinEvent::PointerMotion { window, time_ms, x, y } => {
                let mut e = Encoder::frame(op::POINTER_MOTION);
                e.u32(*window).u32(*time_ms).f64(*x).f64(*y);
                e
            }
            WinEvent::PointerButton { window, serial, time_ms, button, pressed } => {
                let mut e = Encoder::frame(op::POINTER_BUTTON);
                e.u32(*window).u32(*serial).u32(*time_ms).u32(*button).bool(*pressed);
                e
            }
            WinEvent::PointerAxis { window, time_ms, source, dx, dy, v120x, v120y } => {
                let mut e = Encoder::frame(op::POINTER_AXIS);
                e.u32(*window).u32(*time_ms).u8(*source).f64(*dx).f64(*dy).i32(*v120x).i32(*v120y);
                e
            }
            WinEvent::Keymap { fd, size } => {
                fds.push(fd.raw());
                let mut e = Encoder::frame(op::KEYMAP);
                e.u8(1).u32(*size);
                e
            }
            WinEvent::KeyboardEnter { window, serial } => {
                let mut e = Encoder::frame(op::KEYBOARD_ENTER);
                e.u32(*window).u32(*serial);
                e
            }
            WinEvent::KeyboardLeave { window } => {
                let mut e = Encoder::frame(op::KEYBOARD_LEAVE);
                e.u32(*window);
                e
            }
            WinEvent::Key { window, serial, time_ms, key, pressed } => {
                let mut e = Encoder::frame(op::KEY);
                e.u32(*window).u32(*serial).u32(*time_ms).u32(*key).bool(*pressed);
                e
            }
            WinEvent::Modifiers { depressed, latched, locked, group } => {
                let mut e = Encoder::frame(op::MODIFIERS);
                e.u32(*depressed).u32(*latched).u32(*locked).u32(*group);
                e
            }
            WinEvent::Repeat { rate, delay } => {
                let mut e = Encoder::frame(op::REPEAT);
                e.i32(*rate).i32(*delay);
                e
            }
            WinEvent::Touch { window, time_ms, id, phase, x, y } => {
                let mut e = Encoder::frame(op::TOUCH);
                e.u32(*window).u32(*time_ms).i32(*id).u8(*phase).f64(*x).f64(*y);
                e
            }
            WinEvent::ClipboardOffer { kinds } => {
                let mut e = Encoder::frame(op::CLIPBOARD_OFFER);
                put_kinds(&mut e, kinds);
                e
            }
            WinEvent::ClipboardSend { kind, fd } => {
                fds.push(fd.raw());
                let mut e = Encoder::frame(op::CLIPBOARD_SEND);
                e.u8(1).str(kind);
                e
            }
            WinEvent::ClipboardData { serial, fd } => {
                fds.push(fd.raw());
                let mut e = Encoder::frame(op::CLIPBOARD_DATA);
                e.u8(1).u32(*serial);
                e
            }
        };
        Ok((e.finish()?, fds))
    }

    pub fn decode(opcode: u16, payload: &[u8], fds: &mut dyn FdSource) -> Result<WinEvent, WireError> {
        let mut d = Decoder::new(payload);
        let one = |fds: &mut dyn FdSource, d: &mut Decoder| -> Result<Fd, WireError> {
            let mut got = take(fds, d)?;
            if got.len() != 1 {
                return Err(WireError::BadValue("descriptor count"));
            }
            Ok(got.remove(0))
        };
        Ok(match opcode {
            op::CREATED => WinEvent::Created { serial: d.u32()?, window: d.u32()?, token: d.str()? },
            op::CONFIGURE => WinEvent::Configure {
                window: d.u32()?,
                serial: d.u32()?,
                width: finite(d.f64()?, "width")?,
                height: finite(d.f64()?, "height")?,
                scale: finite(d.f64()?, "scale")?,
                maximized: d.bool()?,
                activated: d.bool()?,
            },
            op::CLOSE => WinEvent::Close { window: d.u32()? },
            op::FRAME => WinEvent::Frame { window: d.u32()?, time_ms: d.u32()? },
            op::POINTER_ENTER => WinEvent::PointerEnter { window: d.u32()?, serial: d.u32()?, x: d.f64()?, y: d.f64()? },
            op::POINTER_LEAVE => WinEvent::PointerLeave { window: d.u32()? },
            op::POINTER_MOTION => WinEvent::PointerMotion { window: d.u32()?, time_ms: d.u32()?, x: d.f64()?, y: d.f64()? },
            op::POINTER_BUTTON => WinEvent::PointerButton { window: d.u32()?, serial: d.u32()?, time_ms: d.u32()?, button: d.u32()?, pressed: d.bool()? },
            op::POINTER_AXIS => WinEvent::PointerAxis { window: d.u32()?, time_ms: d.u32()?, source: d.u8()?, dx: d.f64()?, dy: d.f64()?, v120x: d.i32()?, v120y: d.i32()? },
            op::KEYMAP => {
                let fd = one(fds, &mut d)?;
                WinEvent::Keymap { fd, size: d.u32()? }
            }
            op::KEYBOARD_ENTER => WinEvent::KeyboardEnter { window: d.u32()?, serial: d.u32()? },
            op::KEYBOARD_LEAVE => WinEvent::KeyboardLeave { window: d.u32()? },
            op::KEY => WinEvent::Key { window: d.u32()?, serial: d.u32()?, time_ms: d.u32()?, key: d.u32()?, pressed: d.bool()? },
            op::MODIFIERS => WinEvent::Modifiers { depressed: d.u32()?, latched: d.u32()?, locked: d.u32()?, group: d.u32()? },
            op::REPEAT => WinEvent::Repeat { rate: d.i32()?, delay: d.i32()? },
            op::TOUCH => WinEvent::Touch { window: d.u32()?, time_ms: d.u32()?, id: d.i32()?, phase: d.u8()?, x: d.f64()?, y: d.f64()? },
            op::CLIPBOARD_OFFER => WinEvent::ClipboardOffer { kinds: get_kinds(&mut d)? },
            op::CLIPBOARD_SEND => {
                let fd = one(fds, &mut d)?;
                WinEvent::ClipboardSend { kind: d.str()?, fd }
            }
            op::CLIPBOARD_DATA => {
                let fd = one(fds, &mut d)?;
                WinEvent::ClipboardData { serial: d.u32()?, fd }
            }
            other => return Err(WireError::UnknownOpcode(other)),
        })
    }

    /// The serial of an event that is a reply.
    pub fn reply_serial(&self) -> Option<u32> {
        match self {
            WinEvent::Created { serial, .. } | WinEvent::ClipboardData { serial, .. } => Some(*serial),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sock;
    use crate::wire::FrameReader;
    use std::collections::VecDeque;
    use std::os::unix::net::UnixStream;

    /// Sends `bytes` and `fds` across a socket, and reads them back as one frame and its descriptors.
    fn cross(bytes: &[u8], fds: &[RawFd]) -> (u16, Vec<u8>, VecDeque<OwnedFd>) {
        let (a, b) = UnixStream::pair().unwrap();
        sock::send_all(&a, bytes, fds).unwrap();
        let mut got = VecDeque::new();
        let mut reader = FrameReader::new();
        let mut buf = [0u8; 65536];
        loop {
            let n = sock::recv_with_fds(&b, &mut buf, &mut got, 64).unwrap();
            reader.push(&buf[..n]);
            if let Some(f) = reader.next_frame().unwrap() {
                return (f.opcode, f.payload, got);
            }
        }
    }

    fn shm() -> Fd {
        Fd::new(sock::pipe().unwrap().0)
    }

    #[test]
    fn every_request_survives_the_wire() {
        let a = shm();
        let planes = vec![Plane { fd: shm(), offset: 0, stride: 1024 }, Plane { fd: shm(), offset: 4096, stride: 512 }];
        let all = vec![
            WinRequest::Create { serial: 7, window: 1, role: "main".into(), token: "".into(), title: "Terminal".into(), width: 880.0, height: 640.0, own_frame: true, parent: 0, place: Place::Monitor { id: "abc".into(), x: 10.0, y: 20.5 } },
            WinRequest::Destroy { window: 1 },
            WinRequest::Title { window: 1, title: "x".into() },
            WinRequest::AckConfigure { window: 1, serial: 9 },
            WinRequest::Attach { window: 1, buffer: Buffer::Shm { fd: a.clone(), width: 100, height: 50, stride: 400, format: 0 }, damage: vec![Damage { x: 0, y: 0, w: 10, h: 10 }] },
            WinRequest::Attach { window: 1, buffer: Buffer::Dmabuf { planes, width: 256, height: 128, format: 0x3432_5258, modifier: 1 << 56 }, damage: vec![] },
            WinRequest::Cursor { window: 1, shape: 2 },
            WinRequest::Move { window: 1, input_serial: 4 },
            WinRequest::Resize { window: 1, input_serial: 4, edges: 9 },
            WinRequest::Maximize { window: 1, on: true },
            WinRequest::Minimize { window: 1 },
            WinRequest::Place { window: 1, place: Place::Parent { x: 5.0, y: 6.0 } },
            WinRequest::Meaning {
                window: 1,
                focus: 3,
                caret: Some((1.0, 2.0, 1.5, 14.0)),
                nodes: vec![Node { id: 3, parent: 0, role: "button".into(), name: "Save".into(), value: None, states: vec!["focused".into()], x: 1.0, y: 2.0, w: 30.0, h: 20.0 }],
            },
            WinRequest::Copy { input_serial: 5, kinds: vec!["text/plain;charset=utf-8".into()] },
            WinRequest::Paste { serial: 8, kind: "text/plain".into() },
        ];
        for m in all {
            let (bytes, fds) = m.encode().unwrap();
            let (opcode, payload, mut got) = cross(&bytes, &fds);
            let back = WinRequest::decode(opcode, &payload, &mut got).unwrap();
            // Descriptors arrive as new ones: compare everything else.
            assert_eq!(format!("{:?}", strip(&back)), format!("{:?}", strip(&m)), "{m:?}");
            assert!(got.is_empty(), "every descriptor claimed");
        }
    }

    /// The message with its descriptors' numbers hidden.
    fn strip(m: &WinRequest) -> String {
        let s = format!("{m:?}");
        let mut out = String::new();
        let mut rest = s.as_str();
        while let Some(i) = rest.find("Fd(") {
            out += &rest[..i + 3];
            let j = rest[i..].find(')').unwrap();
            out += "_";
            rest = &rest[i + j..];
        }
        out + rest
    }

    #[test]
    fn every_event_survives_the_wire() {
        let all = vec![
            WinEvent::Created { serial: 7, window: 1, token: "t1".into() },
            WinEvent::Configure { window: 1, serial: 2, width: 880.0, height: 640.0, scale: 1.5, maximized: false, activated: true },
            WinEvent::Close { window: 1 },
            WinEvent::Frame { window: 1, time_ms: 1234 },
            WinEvent::PointerEnter { window: 1, serial: 3, x: 1.5, y: 2.5 },
            WinEvent::PointerLeave { window: 1 },
            WinEvent::PointerMotion { window: 1, time_ms: 5, x: 3.0, y: 4.0 },
            WinEvent::PointerButton { window: 1, serial: 6, time_ms: 7, button: 0x110, pressed: true },
            WinEvent::PointerAxis { window: 1, time_ms: 8, source: 0, dx: 0.0, dy: 15.0, v120x: 0, v120y: 120 },
            WinEvent::Keymap { fd: shm(), size: 4096 },
            WinEvent::KeyboardEnter { window: 1, serial: 9 },
            WinEvent::KeyboardLeave { window: 1 },
            WinEvent::Key { window: 1, serial: 10, time_ms: 11, key: 30, pressed: true },
            WinEvent::Modifiers { depressed: 1, latched: 0, locked: 2, group: 0 },
            WinEvent::Repeat { rate: 30, delay: 500 },
            WinEvent::Touch { window: 1, time_ms: 12, id: 0, phase: 1, x: 5.0, y: 6.0 },
            WinEvent::ClipboardOffer { kinds: vec!["text/plain".into()] },
            WinEvent::ClipboardSend { kind: "text/plain".into(), fd: shm() },
            WinEvent::ClipboardData { serial: 13, fd: shm() },
        ];
        for m in all {
            let (bytes, fds) = m.encode().unwrap();
            let (opcode, payload, mut got) = cross(&bytes, &fds);
            let back = WinEvent::decode(opcode, &payload, &mut got).unwrap();
            let hide = |s: String| s.split("Fd(").enumerate().map(|(i, p)| if i == 0 { p.to_string() } else { p[p.find(')').unwrap()..].to_string() }).collect::<Vec<_>>().join("Fd(");
            assert_eq!(hide(format!("{back:?}")), hide(format!("{m:?}")));
            assert!(got.is_empty());
        }
    }

    #[test]
    fn a_picture_that_does_not_add_up_is_refused() {
        let ok = WinRequest::Attach { window: 1, buffer: Buffer::Shm { fd: shm(), width: 100, height: 50, stride: 400, format: 0 }, damage: vec![] };
        let (bytes, _) = ok.encode().unwrap();
        let payload = &bytes[6..];
        // The descriptor did not come.
        assert!(WinRequest::decode(op::ATTACH, payload, &mut NoFds).is_err());
        // A stride too short for the width.
        let bad = WinRequest::Attach { window: 1, buffer: Buffer::Shm { fd: shm(), width: 100, height: 50, stride: 100, format: 0 }, damage: vec![] };
        let (bytes, fds) = bad.encode().unwrap();
        let (opcode, payload, mut got) = cross(&bytes, &fds);
        assert!(WinRequest::decode(opcode, &payload, &mut got).is_err());
        // A side too large.
        let huge = WinRequest::Attach { window: 1, buffer: Buffer::Shm { fd: shm(), width: MAX_SIDE + 1, height: 1, stride: (MAX_SIDE + 1) * 4, format: 0 }, damage: vec![] };
        let (bytes, fds) = huge.encode().unwrap();
        let (opcode, payload, mut got) = cross(&bytes, &fds);
        assert!(WinRequest::decode(opcode, &payload, &mut got).is_err());
    }
}
