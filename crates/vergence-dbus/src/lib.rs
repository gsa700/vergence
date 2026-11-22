//! Vergence's own D-Bus wire protocol: authentication, values, messages and match rules.
//!
//! For the D-Bus front end at the session bus's edge (whitepaper §6.8), which vergence-server
//! runs so that programs written for D-Bus reach the desktop, and a small client
//! ([`client`]: the system bus, for udisks2 in vergence-media). The bus logic (names, routing,
//! the org.freedesktop.DBus methods) lives in the server (its dbus.rs), which owns the
//! connections.
//!
//! Written from the D-Bus specification; no dependencies (§8).
//!
//! Start reading: [`value`] (signatures and marshalling), [`message`] (header and body),
//! [`auth`] (SASL EXTERNAL, server side), then [`matchrule`] and [`client`].
//!
//! Rule: every byte from a peer is untrusted; a bad message is an error, never a panic (the fuzz
//! tests run with `cargo test`).

pub mod auth;
pub mod client;
pub mod matchrule;
pub mod message;
pub mod value;

pub use message::Message;
pub use value::Value;

/// The bus's own name, and where its methods live.
pub const BUS_NAME: &str = "org.freedesktop.DBus";
pub const BUS_PATH: &str = "/org/freedesktop/DBus";

/// Is `name` a valid bus name (unique ":1.5" or well-known "org.example.App")?
pub fn valid_bus_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 255 {
        return false;
    }
    let (unique, rest) = match name.strip_prefix(':') {
        Some(r) => (true, r),
        None => (false, name),
    };
    let parts: Vec<&str> = rest.split('.').collect();
    parts.len() >= 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                && (unique || !p.as_bytes()[0].is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn bus_names() {
        for ok in [":1.5", "org.freedesktop.Notifications", "org.mozilla.firefox.ZGVmYXVsdA__", "a-b.c_d"] {
            assert!(super::valid_bus_name(ok), "{ok}");
        }
        for bad in ["", "org", "org..x", "1org.x", "org.x/y", ".org.x"] {
            assert!(!super::valid_bus_name(bad), "{bad}");
        }
    }
}
