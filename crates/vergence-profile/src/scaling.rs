//! Scaling (milestone 4, docs/milestone-4.md): the scale of each monitor,
//! derived from its physical size, and the numbers for Mode A.
//!
//! The unit is the CSS reference pixel: one pixel at 96 dpi seen from
//! 71 cm (28 in). A monitor's scale is
//!
//! ```text
//! scale = (ppi / 96) x (viewing distance / 71 cm) x preference
//! ```
//!
//! rounded to steps of 0.25, never below 1. Missing or impossible physical
//! sizes give scale 1 with a note saying why (whitepaper §10 item 3).
//!
//! Pure functions only: the display server reuses this unchanged.

use serde::{Deserialize, Serialize};

/// The reference viewing distance of the CSS pixel, in millimeters.
pub const REFERENCE_DISTANCE_MM: u32 = 710;
/// Scales are rounded to this step.
pub const STEP: f64 = 0.25;

/// How far the user usually is from a monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Distance {
    /// 71 cm, the reference distance.
    #[default]
    Desktop,
    /// 50 cm.
    Laptop,
    /// 2.5 m.
    Tv,
}

impl Distance {
    pub fn mm(self) -> u32 {
        match self {
            Distance::Desktop => REFERENCE_DISTANCE_MM,
            Distance::Laptop => 500,
            Distance::Tv => 2500,
        }
    }
}

/// The derived scale of one monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct Derived {
    /// Pixels per inch, when the physical size is usable.
    pub ppi: Option<f64>,
    /// The scale before rounding.
    pub exact: f64,
    /// Rounded to [`STEP`], at least 1.
    pub scale: f64,
    /// Why the physical size was not used, if it wasn't.
    pub note: Option<&'static str>,
}

/// Pixels per inch from the native resolution and physical size: the mean
/// of the horizontal and vertical densities.
pub fn ppi(px: (u32, u32), mm: (u32, u32)) -> f64 {
    let h = px.0 as f64 / (mm.0 as f64 / 25.4);
    let v = px.1 as f64 / (mm.1 as f64 / 25.4);
    (h + v) / 2.0
}

/// Why a physical size can't be trusted, or None if it can.
pub fn implausible(px: (u32, u32), mm: (u32, u32)) -> Option<&'static str> {
    if mm.0 == 0 || mm.1 == 0 || px.0 == 0 || px.1 == 0 {
        return Some("no physical size in the EDID");
    }
    // Some EDIDs store the aspect ratio here (16 x 9 cm, 160 x 90 mm).
    if (mm.0, mm.1) == (160, 90) || (mm.0, mm.1) == (160, 100) || mm.0 < 100 && mm.1 < 100 {
        return Some("the EDID's size looks like an aspect ratio, not a size");
    }
    let d = ppi(px, mm);
    if !(40.0..=600.0).contains(&d) {
        return Some("the EDID's size gives an impossible density");
    }
    // Pixel and millimeter aspect ratios must roughly agree (square pixels).
    let ratio = (px.0 as f64 / px.1 as f64) / (mm.0 as f64 / mm.1 as f64);
    if !(0.85..=1.15).contains(&ratio) {
        return Some("the EDID's size does not match its resolution's shape");
    }
    None
}

/// Round to the nearest [`STEP`], never below 1.
pub fn round_scale(x: f64) -> f64 {
    ((x / STEP).round() * STEP).max(1.0)
}

/// The scale of a monitor with native resolution `px` and physical size
/// `mm` (None if the EDID has none), seen from `distance`, for a user whose
/// size preference is `preference` (1.0 = the reference size).
pub fn derive(px: (u32, u32), mm: Option<(u32, u32)>, distance: Distance, preference: f64) -> Derived {
    let mm = mm.unwrap_or((0, 0));
    if let Some(note) = implausible(px, mm) {
        return Derived { ppi: None, exact: 1.0, scale: 1.0, note: Some(note) };
    }
    let d = ppi(px, mm);
    let exact = d / 96.0 * (distance.mm() as f64 / REFERENCE_DISTANCE_MM as f64) * preference;
    Derived { ppi: Some(d), exact, scale: round_scale(exact), note: None }
}

/// Mode A for a set of monitors: everything is rendered at the whole-number
/// scale `render` (the largest monitor scale, rounded up), and RandR shrinks
/// each monitor's picture onto its panel by `render / scale`.
#[derive(Debug, Clone, PartialEq)]
pub struct ModeA {
    pub render: u32,
    pub monitors: Vec<ModeAMonitor>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModeAMonitor {
    /// The RandR scale factor (rendered size / native size), >= 1.
    pub factor: f64,
    /// The size the monitor takes in the X screen, in pixels.
    pub rendered: (u32, u32),
}

/// The whole-number scale everything is rendered at: the largest monitor
/// scale, rounded up (GTK takes whole numbers only).
pub fn render_scale(scales: &[f64]) -> u32 {
    scales.iter().copied().fold(1.0, f64::max).ceil() as u32
}

/// The RandR factor `render / scale` in 16.16 fixed point.
pub fn factor_fixed(render: u32, scale: f64) -> u32 {
    (render as f64 / scale * crate::arrangement::FIXED_ONE as f64).round() as u32
}

/// `monitors`: (native resolution, scale) for each lit monitor.
pub fn mode_a(monitors: &[((u32, u32), f64)]) -> ModeA {
    let scales: Vec<f64> = monitors.iter().map(|m| m.1).collect();
    let render = render_scale(&scales);
    let monitors = monitors
        .iter()
        .map(|&((w, h), s)| {
            let f = factor_fixed(render, s);
            let len = |x: u32| crate::arrangement::scaled_len(x as u16, f) as u32;
            ModeAMonitor { factor: f as f64 / crate::arrangement::FIXED_ONE as f64, rendered: (len(w), len(h)) }
        })
        .collect();
    ModeA { render, monitors }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: Distance = Distance::Desktop;

    #[test]
    fn worked_examples() {
        // ASUS VS229 (Hubble): 102 ppi -> 1.06 -> 1.0.
        let a = derive((1920, 1080), Some((475, 267)), D, 1.0);
        assert!((a.ppi.unwrap() - 102.0).abs() < 1.0, "{a:?}");
        assert_eq!(a.scale, 1.0);
        // 24" 1080p: 92 ppi -> 1.0.
        assert_eq!(derive((1920, 1080), Some((531, 299)), D, 1.0).scale, 1.0);
        // 27" 4K: 163 ppi -> 1.70 -> 1.75.
        let k = derive((3840, 2160), Some((597, 336)), D, 1.0);
        assert!((k.exact - 1.70).abs() < 0.02, "{k:?}");
        assert_eq!(k.scale, 1.75);
        // 32" 6K: 218 ppi -> 2.25 (the whitepaper's 200% is the user's choice).
        assert_eq!(derive((6016, 3384), Some((700, 394)), D, 1.0).scale, 2.25);
    }

    #[test]
    fn distance_and_preference() {
        // 13" laptop 2560x1600 (~232 ppi) at 50 cm: 1.70 -> 1.75.
        assert_eq!(derive((2560, 1600), Some((286, 179)), Distance::Laptop, 1.0).scale, 1.75);
        // 55" 4K TV (80 ppi) at 2.5 m: 2.9 -> 3.0.
        assert_eq!(derive((3840, 2160), Some((1210, 680)), Distance::Tv, 1.0).scale, 3.0);
        // A user who likes things 25% bigger.
        assert_eq!(derive((1920, 1080), Some((475, 267)), D, 1.25).scale, 1.25);
    }

    #[test]
    fn bad_sizes_fall_back_to_one() {
        for (px, mm) in [
            ((1920, 1080), None),
            ((1920, 1080), Some((0, 0))),
            ((1920, 1080), Some((160, 90))), // aspect ratio, not a size
            ((1920, 1080), Some((16, 9))),
            ((1920, 1080), Some((10000, 5600))), // 5 ppi
            ((1920, 1080), Some((475, 475))), // wrong shape
        ] {
            let d = derive(px, mm, D, 1.0);
            assert_eq!(d.scale, 1.0, "{mm:?}");
            assert!(d.note.is_some(), "{mm:?}");
        }
    }

    #[test]
    fn rounding() {
        assert_eq!(round_scale(0.6), 1.0);
        assert_eq!(round_scale(1.12), 1.0);
        assert_eq!(round_scale(1.13), 1.25);
        assert_eq!(round_scale(1.70), 1.75);
        assert_eq!(round_scale(2.0), 2.0);
    }

    #[test]
    fn mode_a_numbers() {
        // ASUS (1.0) + 27" 4K (1.75): render at 2; RandR 2.0 and 1.143.
        let m = mode_a(&[((1920, 1080), 1.0), ((3840, 2160), 1.75)]);
        assert_eq!(m.render, 2);
        assert_eq!(m.monitors[0].factor, 2.0);
        assert_eq!(m.monitors[0].rendered, (3840, 2160));
        assert!((m.monitors[1].factor - 1.142857).abs() < 1e-4);
        assert_eq!(m.monitors[1].rendered, (4389, 2469));
        // Only scale-1 monitors: render 1, nothing changes.
        let one = mode_a(&[((1920, 1080), 1.0), ((1920, 1080), 1.0)]);
        assert_eq!(one.render, 1);
        assert!(one.monitors.iter().all(|x| x.factor == 1.0 && x.rendered == (1920, 1080)));
        // The whitepaper's example: 1.5 / 1.0 / 2.0 -> 2; 1.333, 2, 1.
        let w = mode_a(&[((3840, 2160), 1.5), ((1920, 1080), 1.0), ((6016, 3384), 2.0)]);
        assert_eq!(w.render, 2);
        let total: u32 = w.monitors.iter().map(|x| x.rendered.0).sum();
        assert_eq!(total, 14976);
    }
}
