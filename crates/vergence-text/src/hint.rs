//! Help for small text (text milestone 1, step 3): what hinting and stem
//! darkening do elsewhere, without a hinting interpreter.
//!
//! At about 100 pixels per inch a 13-point lowercase letter is seven pixels
//! tall and its stems a little over one pixel wide, so a stem straddles two
//! pixels and the top of the x-height is a half-gray row. Two things, both
//! decided by the size in pixels alone (what matters is how many pixels a
//! letter gets, not which screen), and neither moves the layout:
//!
//! - **The x-height on a whole pixel** ([`x_height_scale`]): below
//!   [`SMALL_EM`] pixels the glyph's vertical scale is nudged, at most
//!   [`MAX_NUDGE`], so the face's x-height (OS/2 `sxHeight`) is a whole
//!   number of pixels: the tops of the lowercase letters land on a pixel
//!   edge. FreeType's light hinting does this through its blue zones;
//!   horizontal scale and advances are left alone, so nothing reflows.
//! - **Stem darkening** ([`darkening`], [`dilate`]): strokes widened a
//!   fraction of a pixel, most at the smallest sizes and none from
//!   [`SMALL_EM`] up, as FreeType's stem darkening and macOS's font
//!   smoothing do, so small text keeps its color instead of going gray.
//!
//! `VERGENCE_TEXT_DARKEN=<pixels at 12 px or less>` overrides the amount
//! (0 turns it off; read once per process), for setting it by eye.

use std::sync::OnceLock;

/// The em in pixels from which neither applies.
pub const SMALL_EM: f32 = 24.0;
/// The most the vertical scale is changed for the x-height.
pub const MAX_NUDGE: f32 = 0.08;
/// The widening at 12 pixels to the em or less, in pixels, unless
/// VERGENCE_TEXT_DARKEN says.
pub const DARKEN: f32 = 0.3;

/// The vertical scale that puts an x-height of `x_height_px` pixels (at an
/// em of `em_px` pixels) on a whole pixel; 1 when the em is not small, the
/// face has no x-height, or the nearest whole pixel is too far.
pub fn x_height_scale(x_height_px: f32, em_px: f32) -> f32 {
    if !(em_px > 0.0 && em_px < SMALL_EM && x_height_px >= 2.0 && x_height_px.is_finite()) {
        return 1.0;
    }
    let k = x_height_px.round() / x_height_px;
    if (k - 1.0).abs() <= MAX_NUDGE { k } else { 1.0 }
}

/// How far, in pixels, strokes are widened at an em of `em_px` pixels: the
/// full amount at 12 or less, nothing from SMALL_EM up, straight between.
pub fn darkening(em_px: f32) -> f32 {
    static AMOUNT: OnceLock<f32> = OnceLock::new();
    let full = *AMOUNT.get_or_init(|| {
        std::env::var("VERGENCE_TEXT_DARKEN")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .map_or(DARKEN, |v| v.clamp(0.0, 1.0))
    });
    darkening_of(full, em_px)
}

fn darkening_of(full: f32, em_px: f32) -> f32 {
    if em_px.is_nan() || em_px <= 0.0 || em_px >= SMALL_EM {
        return 0.0;
    }
    full * ((SMALL_EM - em_px) / (SMALL_EM - 12.0)).clamp(0.0, 1.0)
}

/// Coverage rows (`width` wide) widened by `d` pixels to the right: each
/// pixel takes `d` of its left neighbor's coverage on top of its own, so a
/// fully covered stem one pixel wide grows by `d`, and the bitmap one
/// column. Returns the new rows and width; unchanged when `d` is 0.
pub fn dilate(alpha: &[u8], width: usize, d: f32) -> (Vec<u8>, usize) {
    if d <= 0.0 || width == 0 {
        return (alpha.to_vec(), width);
    }
    let height = alpha.len() / width;
    let w = width + 1;
    let mut out = vec![0u8; w * height];
    for y in 0..height {
        let row = &alpha[y * width..(y + 1) * width];
        for x in 0..w {
            let own = if x < width { row[x] as f32 } else { 0.0 };
            let left = if x > 0 { row[x - 1] as f32 } else { 0.0 };
            out[y * w + x] = (own + d * left).min(255.0).round() as u8;
        }
    }
    (out, w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_x_height_lands_on_a_pixel_at_small_sizes() {
        // Inter at 13 px: x-height 0.546 em, 7.1 px -> 7.
        let k = x_height_scale(0.546 * 13.0, 13.0);
        assert!((0.546 * 13.0 * k - 7.0).abs() < 1e-4, "{k}");
        // Large text is left alone, and so is a nudge too big to hide.
        assert_eq!(x_height_scale(0.546 * 30.0, 30.0), 1.0);
        assert_eq!(x_height_scale(2.45, 4.5), 1.0, "2.45 -> 2 is 18%");
        assert_eq!(x_height_scale(0.0, 13.0), 1.0, "no x-height known");
    }

    #[test]
    fn darkening_fades_out_with_size() {
        assert_eq!(darkening_of(0.3, 10.0), 0.3);
        assert_eq!(darkening_of(0.3, 12.0), 0.3);
        assert!((darkening_of(0.3, 18.0) - 0.15).abs() < 1e-6);
        assert_eq!(darkening_of(0.3, 24.0), 0.0);
        assert_eq!(darkening_of(0.3, 48.0), 0.0);
    }

    #[test]
    fn a_stem_grows_by_the_amount() {
        // A one-pixel stem, fully covered, in a row of three.
        let (out, w) = dilate(&[0, 255, 0], 3, 0.3);
        assert_eq!(w, 4);
        assert_eq!(out, vec![0, 255, 77, 0]);
        let ink = |r: &[u8]| r.iter().map(|v| *v as u32).sum::<u32>();
        assert_eq!(ink(&out), 255 + 77);
        // Nothing to do at 0.
        assert_eq!(dilate(&[1, 2, 3, 4], 2, 0.0), (vec![1, 2, 3, 4], 2));
    }
}
