//! A small EDID parser: the 128-byte base block and the descriptors Vergence needs (identity,
//! physical size, native mode).
//!
//! Extension blocks are kept in the raw bytes but not parsed. Used by vergence-server (each
//! monitor's identity and size, drm.rs) and vergence-harness (sweeps over the linuxhw EDID
//! database, fake monitors); [`fake`] writes EDIDs for test monitors. No dependencies.
//!
//! Start reading: [`Edid::parse`], then [`DetailedTiming`], then [`fake`].
//!
//! Rule: an EDID comes from hardware and is not trusted: every read is bounds checked, and a
//! short blob, a bad header or a bad checksum is an error, not a guess.

pub mod fake;

use std::fmt;

/// Size of the EDID base block.
pub const BLOCK_LEN: usize = 128;

const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdidError {
    /// Fewer than 128 bytes.
    TooShort(usize),
    /// The first 8 bytes are not the EDID header.
    BadHeader,
    /// The base block does not sum to 0 mod 256.
    BadChecksum { sum: u8 },
}

impl fmt::Display for EdidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EdidError::TooShort(n) => write!(f, "EDID too short ({n} bytes, need {BLOCK_LEN})"),
            EdidError::BadHeader => write!(f, "EDID header missing"),
            EdidError::BadChecksum { sum } => {
                write!(f, "EDID base block checksum wrong (sums to {sum:#04x})")
            }
        }
    }
}

impl std::error::Error for EdidError {}

/// When the monitor was made, as the EDID states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MadeIn {
    /// Week 1-54 (0 = week not given) of a year.
    Week { week: u8, year: u16 },
    /// Week byte 0xFF: the year is the model year, not the manufacture date.
    ModelYear(u16),
}

/// A detailed timing descriptor, reduced to what Vergence uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetailedTiming {
    pub pixel_clock_khz: u32,
    pub h_active: u16,
    pub v_active: u16,
    pub h_blank: u16,
    pub v_blank: u16,
    /// Image size in millimeters; more precise than the base block's cm.
    pub width_mm: u16,
    pub height_mm: u16,
    pub interlaced: bool,
}

impl DetailedTiming {
    /// Vertical refresh in Hz, or None if the totals are zero.
    pub fn refresh_hz(&self) -> Option<f64> {
        let total = (self.h_active as u64 + self.h_blank as u64) * (self.v_active as u64 + self.v_blank as u64);
        if total == 0 {
            return None;
        }
        let hz = self.pixel_clock_khz as f64 * 1000.0 / total as f64;
        Some(if self.interlaced { hz * 2.0 } else { hz })
    }
}

/// A parsed EDID.
#[derive(Debug, Clone, PartialEq)]
pub struct Edid {
    /// Three-letter PNP manufacturer ID, e.g. "ACI" (ASUS).
    pub manufacturer: String,
    pub product_code: u16,
    /// Numeric serial; 0 means not given.
    pub serial_number: u32,
    /// Serial number text descriptor (0xFF), if present.
    pub serial_text: Option<String>,
    /// Monitor name descriptor (0xFC), if present.
    pub name: Option<String>,
    pub made: MadeIn,
    /// EDID version and revision, e.g. (1, 3).
    pub version: (u8, u8),
    /// Physical size from the base block, in cm. (0, 0) = unknown.
    pub size_cm: (u8, u8),
    /// The first detailed timing: by the EDID rules, the preferred (native) mode.
    pub preferred_timing: Option<DetailedTiming>,
    /// Number of extension blocks the base block announces.
    pub extensions: u8,
}

impl Edid {
    /// Parse an EDID blob (base block plus any extensions).
    pub fn parse(bytes: &[u8]) -> Result<Edid, EdidError> {
        if bytes.len() < BLOCK_LEN {
            return Err(EdidError::TooShort(bytes.len()));
        }
        let b = &bytes[..BLOCK_LEN];
        if b[..8] != HEADER {
            return Err(EdidError::BadHeader);
        }
        let sum = b.iter().fold(0u8, |acc, &x| acc.wrapping_add(x));
        if sum != 0 {
            return Err(EdidError::BadChecksum { sum });
        }

        let mfr = u16::from_be_bytes([b[8], b[9]]);
        let letter = |shift: u16| {
            let v = ((mfr >> shift) & 0x1f) as u8;
            if (1..=26).contains(&v) { (b'A' + v - 1) as char } else { '?' }
        };
        let manufacturer: String = [letter(10), letter(5), letter(0)].iter().collect();

        let year = 1990 + b[17] as u16;
        let made = if b[16] == 0xff { MadeIn::ModelYear(year) } else { MadeIn::Week { week: b[16], year } };

        let mut edid = Edid {
            manufacturer,
            product_code: u16::from_le_bytes([b[10], b[11]]),
            serial_number: u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
            serial_text: None,
            name: None,
            made,
            version: (b[18], b[19]),
            size_cm: (b[21], b[22]),
            preferred_timing: None,
            extensions: b[126],
        };

        for (i, d) in b[54..126].chunks_exact(18).enumerate() {
            if d[0] != 0 || d[1] != 0 {
                // Only the first descriptor is the preferred timing.
                if i == 0 {
                    edid.preferred_timing = Some(parse_timing(d));
                }
                continue;
            }
            match d[3] {
                0xfc => edid.name = descriptor_text(d),
                0xff => edid.serial_text = descriptor_text(d),
                _ => {}
            }
        }
        Ok(edid)
    }

    /// The best physical size in mm: the preferred timing's mm size if it is
    /// plausible, else the base block's cm size, else None.
    pub fn physical_size_mm(&self) -> Option<(u16, u16)> {
        if let Some(t) = self.preferred_timing {
            if t.width_mm > 0 && t.height_mm > 0 {
                return Some((t.width_mm, t.height_mm));
            }
        }
        let (w, h) = self.size_cm;
        if w > 0 && h > 0 { Some((w as u16 * 10, h as u16 * 10)) } else { None }
    }
}

fn parse_timing(d: &[u8]) -> DetailedTiming {
    DetailedTiming {
        pixel_clock_khz: u16::from_le_bytes([d[0], d[1]]) as u32 * 10,
        h_active: d[2] as u16 | ((d[4] as u16 & 0xf0) << 4),
        h_blank: d[3] as u16 | ((d[4] as u16 & 0x0f) << 8),
        v_active: d[5] as u16 | ((d[7] as u16 & 0xf0) << 4),
        v_blank: d[6] as u16 | ((d[7] as u16 & 0x0f) << 8),
        width_mm: d[12] as u16 | ((d[14] as u16 & 0xf0) << 4),
        height_mm: d[13] as u16 | ((d[14] as u16 & 0x0f) << 8),
        interlaced: d[17] & 0x80 != 0,
    }
}

/// Text of a display descriptor: bytes 5..18, ended by 0x0A, padded with
/// spaces. Non-printable bytes become '?'. Empty text is None.
fn descriptor_text(d: &[u8]) -> Option<String> {
    let raw = &d[5..18];
    let end = raw.iter().position(|&c| c == 0x0a).unwrap_or(raw.len());
    let s: String = raw[..end].iter().map(|&c| if (0x20..0x7f).contains(&c) { c as char } else { '?' }).collect();
    let s = s.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASUS: &[u8] = include_bytes!("../tests/data/asus-vs229.bin");
    const FAKE: &[u8] = include_bytes!("../tests/data/verge-fake-1.bin");

    #[test]
    fn asus_vs229() {
        let e = Edid::parse(ASUS).unwrap();
        assert_eq!(e.manufacturer, "ACI");
        assert_eq!(e.product_code, 0x22d3);
        assert_eq!(e.serial_number, 123456);
        assert_eq!(e.serial_text.as_deref(), Some("TESTSN123456"));
        assert_eq!(e.name.as_deref(), Some("ASUS VS229"));
        assert_eq!(e.version, (1, 3));
        assert_eq!(e.size_cm, (48, 27));
        assert_eq!(e.extensions, 1);
        let t = e.preferred_timing.unwrap();
        assert_eq!((t.h_active, t.v_active), (1920, 1080));
        let hz = t.refresh_hz().unwrap();
        assert!((hz - 60.0).abs() < 0.1, "refresh {hz}");
        assert_eq!(e.physical_size_mm(), Some((t.width_mm, t.height_mm)));
        assert!(matches!(e.made, MadeIn::Week { year: 2013, .. }));
    }

    #[test]
    fn fake_monitor_differs_only_in_identity() {
        let a = Edid::parse(ASUS).unwrap();
        let f = Edid::parse(FAKE).unwrap();
        assert_eq!(f.name.as_deref(), Some("VERGE FAKE 1"));
        assert_eq!(f.serial_text.as_deref(), Some("FAKE00000001"));
        assert_eq!(f.product_code, 0x9901);
        assert_eq!(f.serial_number, 424242);
        assert_eq!(f.extensions, 0);
        assert_eq!(f.preferred_timing, a.preferred_timing);
    }

    #[test]
    fn rejects_short() {
        assert_eq!(Edid::parse(&ASUS[..127]), Err(EdidError::TooShort(127)));
        assert_eq!(Edid::parse(&[]), Err(EdidError::TooShort(0)));
    }

    #[test]
    fn rejects_bad_header() {
        let mut b = ASUS.to_vec();
        b[0] = 0x01;
        assert_eq!(Edid::parse(&b), Err(EdidError::BadHeader));
    }

    #[test]
    fn rejects_bad_checksum() {
        let mut b = ASUS.to_vec();
        b[20] ^= 0x01;
        assert!(matches!(Edid::parse(&b), Err(EdidError::BadChecksum { .. })));
    }

    #[test]
    fn zero_size_is_unknown() {
        let mut b = FAKE.to_vec();
        b[21] = 0;
        b[22] = 0;
        // also zero the timing's mm size (bytes 12-14 of descriptor 0)
        b[54 + 12] = 0;
        b[54 + 13] = 0;
        b[54 + 14] = 0;
        b[127] = 0;
        let s = b[..127].iter().fold(0u8, |a, &x| a.wrapping_add(x));
        b[127] = 0u8.wrapping_sub(s);
        let e = Edid::parse(&b).unwrap();
        assert_eq!(e.physical_size_mm(), None);
    }

    #[test]
    fn never_panics_on_garbage() {
        // Valid header and checksum, everything else arbitrary.
        for seed in 0u32..2000 {
            let mut b = [0u8; 128];
            let mut x = seed.wrapping_mul(2654435761).wrapping_add(1);
            for byte in b.iter_mut() {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                *byte = x as u8;
            }
            b[..8].copy_from_slice(&HEADER);
            let s = b[..127].iter().fold(0u8, |a, &v| a.wrapping_add(v));
            b[127] = 0u8.wrapping_sub(s);
            let e = Edid::parse(&b).unwrap();
            let _ = e.physical_size_mm();
            let _ = e.preferred_timing.map(|t| t.refresh_hz());
        }
    }
}
