//! Vergence's own client side of the Wayland wire protocol (whitepaper
//! §6.10; toolkit milestone 1, step 2).
//!
//! The toolkit talks to the display server as a Wayland client for now, in
//! code written by the project, as `vergence-dbus` is the project's own
//! D-Bus. The native window protocol takes this crate's place behind the
//! toolkit's window code when it exists.
//!
//! - wire.rs: messages as bytes;
//! - conn.rs: the socket, with file descriptors passed beside the bytes;
//! - shm.rs: memory shared with the server, for pixels drawn in software;
//! - client.rs: the requests and events the toolkit uses, by name.
//!
//! No Wayland library is linked, so there is no `wl_egl_window`: a GPU frame
//! will reach the server as a buffer of its own (linux-dmabuf, step 3).

pub mod client;
pub mod conn;
pub mod shm;
pub mod wire;

pub use client::{Client, Event, Interface};
