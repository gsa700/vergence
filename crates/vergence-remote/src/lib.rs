//! Vergence Remote (remote milestone 1, docs/remote-milestone-1.md): using a
//! Vergence machine from another as if sitting at it, over SSH.
//!
//! This crate is the protocol both sides speak (proto.rs) and the
//! compression of frames' rectangles (lz.rs, our own, in LZ4's published
//! block format). It has no platform code, so every viewer (on Vergence,
//! the Mac, Windows) and the host's program share it. The host's server
//! draws each frame at the viewer's size and scale and sends only what
//! changed ("Frames, not display lists" in the plan).
//!
//! Start reading: proto.rs's module comment (the stream), then
//! [`proto::Frame::apply`] (where the viewer meets untrusted input).

pub mod lz;
pub mod proto;
