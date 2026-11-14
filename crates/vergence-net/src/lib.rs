//! Vergence's networks (settings milestone 1, step 3).
//!
//! - [`wpa`]: a client of wpa_supplicant's control socket, the text protocol
//!   wpa_cli speaks (Dave, 2026-11-14: "wpa_supplicant is fine"): status,
//!   scans and their results, the known networks, joining one with its
//!   password, forgetting one, and the events (a scan done, connected,
//!   disconnected, a wrong password). The same on Void and FreeBSD.
//! - [`ifaces`]: the machine's network interfaces, whether each is up and
//!   has a carrier, and their addresses (getifaddrs, on both systems).
//!
//! Everything read back from the network (network names, scan results) is
//! untrusted text: decoded, control characters dropped, length limited.
//! Passwords go from the caller straight to wpa_supplicant, checked and
//! quoted, and are never logged or kept here (whitepaper §7.3).

pub mod ifaces;
pub mod wpa;
