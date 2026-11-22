//! How much of a pixel a glyph's coverage paints, by the ink's lightness
//! (text milestone 1, step 2).
//!
//! The renderers blend in sRGB values as stored. For a glyph's edges that
//! keeps dark ink on a light ground at a familiar weight, but makes light
//! ink on a dark ground thinner than it should be: a half-covered pixel
//! lands half way in sRGB values, about a fifth of the light. Skia (Chrome,
//! Android) answers this with a table chosen by the ink's luminance, and so
//! do we, in its terms:
//!
//! - the ground is taken to be the ink's opposite (Skia's assumption: the
//!   renderer does not read what is under a glyph to decide);
//! - the coverage is first lifted by `contrast`, more the darker the ink;
//! - the blend is worked out as if colors mixed at `gamma` (1: as stored,
//!   which is today's look; 2.2: physically linear), and the coverage that
//!   gives that result when blended as stored is what is painted.
//!
//! Light ink gets fuller edges; dark ink stays about as before. Both
//! renderers look the answer up in the same tables, one per ink luminance
//! byte ([`curves`]): the CPU per glyph, the GPU from a texture of them
//! ([`GLSL`]), so the two paint the same.
//!
//! `VERGENCE_TEXT_LOOK="gamma,contrast"` overrides the defaults (for
//! setting them by eye; read once per process).

use std::sync::OnceLock;

/// The defaults, set by eye on an LG (4K, 27") and an ASUS (1080p, 21.5")
/// monitor, light and dark.
pub const GAMMA: f32 = 1.45;
pub const CONTRAST: f32 = 0.5;

/// The gamma and contrast in force: the defaults or VERGENCE_TEXT_LOOK's.
pub fn look() -> (f32, f32) {
    static LOOK: OnceLock<(f32, f32)> = OnceLock::new();
    *LOOK.get_or_init(|| {
        std::env::var("VERGENCE_TEXT_LOOK")
            .ok()
            .and_then(|v| {
                let (g, c) = v.split_once(',')?;
                let (g, c) = (g.trim().parse::<f32>().ok()?, c.trim().parse::<f32>().ok()?);
                (g.is_finite() && c.is_finite()).then_some((g.clamp(1.0, 3.0), c.clamp(0.0, 1.0)))
            })
            .unwrap_or((GAMMA, CONTRAST))
    })
}

/// The ink's luminance from its sRGB values (0 to 1), as Rec. 709 weighs
/// the channels.
pub fn luminance(r: f32, g: f32, b: f32) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// The coverage to paint for coverage `a` (0 to 1) of ink with luminance
/// `s` (0 to 1).
pub fn cover(a: f32, s: f32, gamma: f32, contrast: f32) -> f32 {
    let s = s.clamp(0.0, 1.0);
    let ink = s.powf(gamma);
    let ground = 1.0 - ink;
    let ground_v = ground.powf(1.0 / gamma);
    let a = (a + contrast * ground * a * (1.0 - a)).clamp(0.0, 1.0);
    if (s - ground_v).abs() < 1.0 / 256.0 {
        return a;
    }
    let mixed = ink * a + ground * (1.0 - a);
    ((mixed.powf(1.0 / gamma) - ground_v) / (s - ground_v)).clamp(0.0, 1.0)
}

/// The ink's luminance as a byte, 0 to 255: which row of [`curves`] it
/// takes. The GPU shader works out the same from the same color.
pub fn luma_byte(r: u8, g: u8, b: u8) -> usize {
    (luminance(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0).clamp(0.0, 1.0) * 255.0).round() as usize
}

/// The table for one ink luminance (a byte): coverage in 0..=255 to the
/// coverage to paint.
pub fn table(luma: usize, gamma: f32, contrast: f32) -> [u8; 256] {
    let s = luma.min(255) as f32 / 255.0;
    let mut t = [0u8; 256];
    for (i, v) in t.iter_mut().enumerate() {
        *v = (cover(i as f32 / 255.0, s, gamma, contrast) * 255.0).round() as u8;
    }
    t
}

/// Every ink luminance's table, row by row (256 x 256 bytes): the GPU's
/// texture of them, so it paints exactly what the CPU's tables say.
pub fn curves(gamma: f32, contrast: f32) -> Vec<u8> {
    (0..256).flat_map(|l| table(l, gamma, contrast)).collect()
}

/// The GPU's look-up of [`curves`] (a 256 x 256 one-byte texture,
/// `u_curves`): coverage across, the ink's luminance byte down.
pub const GLSL: &str = r#"
uniform highp sampler2D u_curves;
float text_cover(float a, vec3 c) {
    int ai = int(floor(clamp(a, 0.0, 1.0) * 255.0 + 0.5));
    int li = int(floor(clamp(dot(c, vec3(0.2126, 0.7152, 0.0722)), 0.0, 1.0) * 255.0 + 0.5));
    return texelFetch(u_curves, ivec2(ai, li), 0).r;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_and_everything_stay_so() {
        for s in [0.0, 0.3, 0.5, 0.73, 1.0] {
            assert_eq!(cover(0.0, s, GAMMA, CONTRAST), 0.0, "{s}");
            assert!((cover(1.0, s, GAMMA, CONTRAST) - 1.0).abs() < 1e-5, "{s}");
        }
    }

    #[test]
    fn light_ink_gets_fuller_edges_and_dark_ink_keeps_its_weight() {
        let light = cover(0.5, 1.0, GAMMA, CONTRAST);
        let dark = cover(0.5, 0.0, GAMMA, CONTRAST);
        assert!(light > 0.58, "white ink, half covered: {light}");
        assert!((dark - 0.5).abs() < 0.05, "black ink, half covered: {dark}");
    }

    #[test]
    fn gamma_one_and_no_contrast_is_todays_look() {
        for s in [0.0, 0.2, 0.9, 1.0] {
            for a in [0.1, 0.4, 0.8] {
                assert!((cover(a, s, 1.0, 0.0) - a).abs() < 1e-5, "{s} {a}");
            }
        }
    }

    #[test]
    fn the_texture_holds_every_table() {
        let all = curves(GAMMA, CONTRAST);
        assert_eq!(all.len(), 65536);
        assert_eq!(&all[200 * 256..201 * 256], &table(200, GAMMA, CONTRAST)[..]);
        assert_eq!(luma_byte(255, 255, 255), 255);
        assert_eq!(luma_byte(0, 0, 0), 0);
    }

    #[test]
    fn the_curve_only_rises() {
        for s in [0, 64, 128, 153, 191, 255] {
            let t = table(s, GAMMA, CONTRAST);
            assert!(t.windows(2).all(|w| w[0] <= w[1]), "{s}");
            assert_eq!((t[0], t[255]), (0, 255));
        }
    }
}
