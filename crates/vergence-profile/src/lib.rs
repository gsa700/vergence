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

/// The monitor itself, whichever of its inputs it is on: maker, product code
/// and serial number from the EDID's base block, plus the serial string
/// (descriptor 0xFF) if it has one. A monitor gives each input its own EDID
/// (an LG Ultra HD's HDMI and DisplayPort EDIDs differ, so it became two
/// monitors with two scales), but these fields stay
/// the same. None when it can't be told apart from another of its model: no
/// serial number and no serial string.
pub fn model_key(edid: &[u8]) -> Option<String> {
    if edid.len() < 128 || edid[..8] != [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0] {
        return None;
    }
    let serial = u32::from_le_bytes([edid[12], edid[13], edid[14], edid[15]]);
    let text = (0..4)
        .map(|i| &edid[54 + 18 * i..72 + 18 * i])
        .find(|d| d[..3] == [0, 0, 0] && d[3] == 0xff)
        .map(|d| String::from_utf8_lossy(&d[5..18]).split('\n').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty() && s.chars().any(|c| c.is_ascii_alphanumeric() && c != '0'));
    if serial == 0 && text.is_none() {
        return None;
    }
    Some(format!(
        "{:02x}{:02x}-{:02x}{:02x}-{serial}-{}",
        edid[8],
        edid[9],
        edid[11],
        edid[10],
        text.unwrap_or_default()
    ))
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

    fn edid(serial: u32, text: Option<&str>, ext: u8) -> Vec<u8> {
        let mut b = vec![0u8; 256];
        b[..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
        b[8..10].copy_from_slice(&[0x1e, 0x6d]); // GSM
        b[10..12].copy_from_slice(&[0x09, 0x5b]);
        b[12..16].copy_from_slice(&serial.to_le_bytes());
        if let Some(t) = text {
            b[54..59].copy_from_slice(&[0, 0, 0, 0xff, 0]);
            let mut field = [b' '; 13];
            field[..t.len()].copy_from_slice(t.as_bytes());
            field[t.len().min(12)] = b'\n';
            b[59..72].copy_from_slice(&field);
        }
        b[126] = 1;
        b[200] = ext; // the extension block differs between inputs
        b
    }

    #[test]
    fn one_monitor_on_two_inputs() {
        // The LG Ultra HD: serial number only, no serial string; its HDMI and
        // DisplayPort EDIDs differ (here in the extension block).
        let (hdmi, dp) = (edid(52074, None, 1), edid(52074, None, 2));
        assert_ne!(monitor_id(&hdmi), monitor_id(&dp));
        assert_eq!(model_key(&hdmi), model_key(&dp));
        assert_eq!(model_key(&hdmi).as_deref(), Some("1e6d-5b09-52074-"));
        // Another of the same model is another monitor.
        assert_ne!(model_key(&hdmi), model_key(&edid(52075, None, 1)));
        // A serial string alone is enough (the ASUS VS229).
        assert_eq!(model_key(&edid(0, Some("DCLMTF102508"), 1)).as_deref(), Some("1e6d-5b09-0-DCLMTF102508"));
        // Neither: can't be told from its twin.
        assert_eq!(model_key(&edid(0, None, 1)), None);
        assert_eq!(model_key(&edid(0, Some("0000000"), 1)), None);
        // Not an EDID (a connector name stands in for a missing one).
        assert_eq!(model_key(b"HDMI-A-1"), None);
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
