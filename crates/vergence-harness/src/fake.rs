//! `vergence-harness fake`: write a fake monitor's EDID (milestone 4: fake
//! monitors of different densities).
//!
//! The EDID is a 1.3 base block (manufacturer "VRG", the given name, serial,
//! physical size and one preferred timing) plus a CTA-861 extension with an
//! HDMI vendor block. The HDMI block matters: without it Intel's driver
//! treats the sink as DVI and caps it at 165 MHz, too slow for 4K.

use std::error::Error;
use std::path::PathBuf;

pub use vergence_edid::fake::{FHD_60, Spec, UHD_30, build};

fn parse_pair(s: &str, sep: char) -> Option<(u16, u16)> {
    let (a, b) = s.split_once(sep)?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// `fake --name N --product 0x9902 --serial 2 --mode 4k30|1080p60 --size-mm WxH --out FILE`
pub fn run(args: &[String]) -> Result<bool, Box<dyn Error>> {
    let get = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let need = |k: &str| get(k).ok_or_else(|| format!("missing {k}"));
    let name = need("--name")?;
    let product = u16::from_str_radix(need("--product")?.trim_start_matches("0x"), 16)?;
    let serial: u32 = need("--serial")?.parse()?;
    let timing = match need("--mode")?.as_str() {
        "4k30" => UHD_30,
        "1080p60" => FHD_60,
        m => return Err(format!("unknown mode {m} (4k30, 1080p60)").into()),
    };
    let size_mm = parse_pair(&need("--size-mm")?, 'x').ok_or("--size-mm WxH")?;
    let out = PathBuf::from(need("--out")?);
    let bytes = build(&Spec { name: &name, product, serial, timing, size_mm });
    std::fs::write(&out, &bytes)?;
    let e = vergence_edid::Edid::parse(&bytes)?;
    println!(
        "{}: {:?} {} {:?} mm, {} bytes",
        out.display(),
        e.name.clone().unwrap_or_default(),
        e.preferred_timing.map(|t| format!("{}x{}", t.h_active, t.v_active)).unwrap_or_default(),
        e.physical_size_mm(),
        bytes.len()
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vergence_edid::Edid;

    #[test]
    fn fake_4k_parses_back() {
        let b = build(&Spec { name: "VERGE FAKE 4K", product: 0x9902, serial: 2, timing: UHD_30, size_mm: (597, 336) });
        assert_eq!(b.len(), 256);
        assert_eq!(b[..128].iter().fold(0u8, |a, x| a.wrapping_add(*x)), 0);
        assert_eq!(b[128..].iter().fold(0u8, |a, x| a.wrapping_add(*x)), 0);
        let e = Edid::parse(&b).unwrap();
        assert_eq!(e.manufacturer, "VRG");
        assert_eq!(e.name.as_deref(), Some("VERGE FAKE 4K"));
        assert_eq!(e.serial_text.as_deref(), Some("FAKE00000002"));
        assert_eq!(e.product_code, 0x9902);
        assert_eq!(e.extensions, 1);
        assert_eq!(e.physical_size_mm(), Some((597, 336)));
        assert_eq!(e.size_cm, (60, 34));
        let t = e.preferred_timing.unwrap();
        assert_eq!((t.h_active, t.v_active, t.h_blank, t.v_blank), (3840, 2160, 560, 90));
        assert!((t.refresh_hz().unwrap() - 30.0).abs() < 0.01);
        // HDMI vendor block present.
        assert!(b[128..].windows(3).any(|w| w == [0x03, 0x0C, 0x00]));
    }

    #[test]
    fn fake_1080p_parses_back() {
        let b = build(&Spec { name: "VERGE FAKE 24", product: 0x9903, serial: 3, timing: FHD_60, size_mm: (531, 299) });
        let e = Edid::parse(&b).unwrap();
        assert_eq!(e.name.as_deref(), Some("VERGE FAKE 24"));
        let t = e.preferred_timing.unwrap();
        assert_eq!((t.h_active, t.v_active), (1920, 1080));
        assert!((t.refresh_hz().unwrap() - 60.0).abs() < 0.01);
        assert_eq!(e.physical_size_mm(), Some((531, 299)));
    }
}
