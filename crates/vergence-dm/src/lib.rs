//! VDM, the Vergence display manager (docs/vdm-milestone-1.md). This crate
//! holds what more than one of its parts needs: the password buffer
//! ([`secret`]), the unlock protocol between the user's server and the
//! root daemon `vdm` ([`proto`]), where its sockets are ([`paths`]), the
//! system calls they share ([`sys`]), and our own declarations of PAM
//! ([`pam`]), the operating system's interface for logging in.

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
