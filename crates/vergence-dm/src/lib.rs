//! VDM, the Vergence display manager: the parts its root daemon, the server and the settings app
//! share.
//!
//! See docs/vdm-milestone-1.md. The root daemon `vdm` (src/bin/vdm.rs) checks passwords for the
//! lock screen, locks sessions before sleep, runs the login screen and carries out Sleep,
//! Restart and Shut Down; `vdm-try` asks it from a terminal. vergence-server (lock screen, login
//! screen, elevation prompt, power key) and vergence-settings (Lock Screen and Login pages) are
//! its clients. No dependencies: the system calls are declared here, not taken from libc.
//!
//! This crate holds what more than one of its parts needs: the password buffer ([`secret`]), the
//! unlock protocol between the user's server and `vdm` ([`proto`]), where its sockets are
//! ([`paths`]), the system calls they share ([`sys`]), our own declarations of PAM ([`pam`]),
//! the operating system's interface for logging in, and the actions an elevation prompt may
//! approve ([`elevate`]).
//!
//! Start reading: [`proto`] (what crosses the socket), [`secret`] (how a password is held),
//! src/bin/vdm.rs (the daemon), then [`pam`].
//!
//! Rules: `vdm` learns who is asking from the kernel (the peer's uid), never from the request; a
//! password lives only in a [`secret`] buffer (locked memory, wiped, no Debug or Display) and is
//! never logged; a frame is checked against its maximum length before it is read.

pub mod brightness;
pub mod ddc;
pub mod elevate;
pub mod encryption;
pub mod keyboard;
pub mod pam;
pub mod paths;
pub mod power;
pub mod powerkey;
pub mod proto;
pub mod secret;
pub mod sessions;
pub mod sys;
pub mod vt;
