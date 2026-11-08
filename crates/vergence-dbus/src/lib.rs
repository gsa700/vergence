//! Vergence's own D-Bus wire protocol (whitepaper §6.8): authentication,
//! values, messages and match rules, for the D-Bus front end at the session
//! bus's edge. The bus logic (names, routing, the org.freedesktop.DBus
//! methods) lives in the server, which owns the connections.
//!
//! Written from the D-Bus specification; no dependencies (§8).

pub mod auth;
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
