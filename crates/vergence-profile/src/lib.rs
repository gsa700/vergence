//! Profile keys: one key per set of connected monitors (milestone 1).
//!
//! The key is SHA-256 over the sorted raw EDID blobs, so it does not depend
//! on which port each monitor is in. Each blob is prefixed with its length so
//! two different sets can never hash the same byte stream.
//!
//! Open question (docs/milestone-1.md): two identical monitors without serial
//! numbers have identical EDIDs. The key still differs from a one-monitor
//! setup (both blobs are hashed), but swapping them cannot be told apart.
//! [`duplicate_count`] reports that case so `vergd` can say so.

use sha2::{Digest, Sha256};

/// Version of the key format, hashed first so a future change of format
/// never collides with an old key.
const KEY_FORMAT: &[u8] = b"vergence-profile-key-v1";

/// The profile key of a set of EDID blobs, as 64 lowercase hex digits.
pub fn profile_key<B: AsRef<[u8]>>(edids: &[B]) -> String {
    let mut blobs: Vec<&[u8]> = edids.iter().map(|b| b.as_ref()).collect();
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
    let mut blobs: Vec<&[u8]> = edids.iter().map(|b| b.as_ref()).collect();
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
    fn duplicates() {
        let a = vec![1u8];
        let b = vec![2u8];
        assert_eq!(duplicate_count(&[&a, &b]), 0);
        assert_eq!(duplicate_count(&[&a, &a]), 1);
        assert_eq!(duplicate_count(&[&a, &b, &a, &a]), 2);
    }
}
