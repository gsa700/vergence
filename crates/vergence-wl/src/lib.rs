//! Vergence's own client side of the Wayland wire protocol, for the toolkit's Wayland road.
//!
//! Whitepaper §6.10; toolkit milestone 1, step 2. Used by vergence-ui when a window cannot open
//! on the project's native protocol (another desktop, or `VERGENCE_UI_WAYLAND=1`), in code
//! written by the project, as `vergence-dbus` is the project's own D-Bus. Uses
//! vergence-protocol for shared memory and passing file descriptors.
//!
//! - wire.rs: messages as bytes;
//! - conn.rs: the socket, with file descriptors passed beside the bytes;
//! - `shm` (vergence-protocol's, re-exported): memory shared with the server, for pixels drawn
//!   in software;
//! - client.rs: the requests and events the toolkit uses, by name.
//!
//! Start reading: [`wire`], then [`conn`], then [`client`].
//!
//! No Wayland library is linked, so there is no `wl_egl_window`: a GPU frame reaches the server
//! as a buffer of its own (linux-dmabuf).

pub mod client;
pub mod conn;
/// Shared memory: moved to vergence-protocol on 2026-11-11 (native windows use it too).
pub use vergence_protocol::shm;
pub mod wire;

pub use client::{Client, Event, Interface};
