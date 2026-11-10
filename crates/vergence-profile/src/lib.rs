//! Profile keys: one key per set of connected monitors (milestone 1).
//!
//! The key is SHA-256 over the sorted EDID blobs, so it does not depend on
//! which port each monitor is in. Each blob is prefixed with its length so
//! two different sets can never hash the same byte stream.
//!
//! Each blob is first cut to the blocks its base block declares
//! ([`canonical`]): some drivers always read 256 bytes, and a monitor with a
//! 128-byte EEPROM then returns its base block twice (47% of the linuxhw
//! database has such trailing bytes). The key must not depend on how much a
//! driver read.
//!
//! Open question (docs/milestone-1.md): two identical monitors without serial
//! numbers have identical EDIDs. The key still differs from a one-monitor
//! setup (both blobs are hashed), but swapping them cannot be told apart.
//! [`duplicate_count`] reports that case so `vergd` can say so.
//!
//! Milestone 2 adds [`arrangement`]: what each profile remembers (modes,
//! positions, primary) and the file that holds the profiles. Milestone 3
//! adds [`windows`]: where each window was on each setup. Milestone 4 adds
//! [`scaling`]: each monitor's scale from its physical size, and Mode A.

pub mod arrangement;
pub mod scaling;
pub mod windows;

/// The primary screen among the connected ones (`ids`, in the order the
/// server lists them: left to right, then top to bottom): the one recorded
/// in the settings (`[displays] primary`) if it is connected, else the
/// first. One rule for the settings app, which shows and changes it, and
/// the server, whose shelf stands on it.
pub fn primary<'a>(recorded: Option<&str>, ids: &'a [String]) -> Option<&'a str> {
    match recorded {
        Some(r) if ids.iter().any(|i| i == r) => ids.iter().find(|i| *i == r).map(String::as_str),
        _ => ids.first().map(String::as_str),
    }
}

use sha2::{Digest, Sha256};

/// A short, stable ID for one monitor: the first 16 hex digits of SHA-256
/// over its canonical EDID. Profiles name monitors by this, not by port.
pub fn monitor_id(edid: &[u8]) -> String {
    let d = Sha256::digest(canonical(edid));
    d.iter().take(8).map(|x| format!("{x:02x}")).collect()
}

/// Version of the key format, hashed first so a future change of format
/// never collides with an old key.
const KEY_FORMAT: &[u8] = b"vergence-profile-key-v1";

/// The part of an EDID blob the key uses: the base block plus the extension
/// blocks it declares (byte 126). A blob shorter than that, or shorter than
/// one block, is used whole.
pub fn canonical(blob: &[u8]) -> &[u8] {
    if blob.len() < 128 {
        return blob;
    }
    let declared = 128 * (1 + blob[126] as usize);
    if blob.len() >= declared { &blob[..declared] } else { blob }
}

/// The profile key of a set of EDID blobs, as 64 lowercase hex digits.
pub fn profile_key<B: AsRef<[u8]>>(edids: &[B]) -> String {
    let mut blobs: Vec<&[u8]> = edids.iter().map(|b| canonical(b.as_ref())).collect();
    blobs.sort_unstable();
    let mut h = Sha256::new();
    h.update(KEY_FORMAT);
    h.update((blobs.len() as u32).to_be_bytes());
    for b in blobs {
        h.update((b.len() as u32).to_be_bytes());
        h.update(b);
    }
    h.finalize().iter().map(|x| format!("{x:02x}")).collect()
}

/// How many blobs are exact copies of another blob in the set (0 = all
/// distinct).
pub fn duplicate_count<B: AsRef<[u8]>>(edids: &[B]) -> usize {
    let mut blobs: Vec<&[u8]> = edids.iter().map(|b| canonical(b.as_ref())).collect();
    blobs.sort_unstable();
    blobs.windows(2).filter(|w| w[0] == w[1]).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_does_not_matter() {
        let a = vec![1u8, 2, 3];
        let b = vec![9u8, 8];
        assert_eq!(profile_key(&[&a, &b]), profile_key(&[&b, &a]));
    }

    #[test]
    fn sets_differ() {
        let a = vec![1u8, 2, 3];
        let b = vec![9u8, 8];
        assert_ne!(profile_key(&[&a]), profile_key(&[&a, &b]));
        assert_ne!(profile_key(&[&a]), profile_key(&[&a, &a]));
        let none: [&[u8]; 0] = [];
        assert_ne!(profile_key(&none), profile_key(&[&a]));
    }

    #[test]
    fn no_concatenation_collision() {
        // [1,2]+[3] vs [1]+[2,3]: same bytes in a row, different sets.
        assert_ne!(profile_key(&[&[1u8, 2][..], &[3]]), profile_key(&[&[1u8][..], &[2, 3]]));
    }

    #[test]
    fn key_is_hex_sha256() {
        let k = profile_key(&[&[0u8; 128][..]]);
        assert_eq!(k.len(), 64);
        assert!(k.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn trailing_copy_does_not_change_key() {
        let mut base = vec![0u8; 128];
        base[0] = 7; // ext count (byte 126) stays 0
        let mut doubled = base.clone();
        doubled.extend_from_slice(&base);
        assert_eq!(canonical(&doubled), &base[..]);
        assert_eq!(profile_key(&[&base]), profile_key(&[&doubled]));
    }

    #[test]
    fn declared_extensions_are_kept() {
        let mut b = vec![0u8; 256];
        b[126] = 1;
        b[200] = 5;
        assert_eq!(canonical(&b).len(), 256);
        assert_eq!(canonical(&b[..200]).len(), 200); // short: used whole
        assert_eq!(canonical(&b[..10]).len(), 10);
    }

    #[test]
    fn duplicates() {
        let a = vec![1u8];
        let b = vec![2u8];
        assert_eq!(duplicate_count(&[&a, &b]), 0);
        assert_eq!(duplicate_count(&[&a, &a]), 1);
        assert_eq!(duplicate_count(&[&a, &b, &a, &a]), 2);
    }
}
