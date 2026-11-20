//! PDF's blend modes (ISO 32000-2 §11.3.5; PDF milestone 1, step 9): how
//! a painted color mixes with the color already there. The separable modes
//! work on each channel alone; Hue, Saturation, Color and Luminosity on the
//! color as a whole. Colors here are RGB, 0 to 1, not premultiplied.

use crate::Rgba;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Blend {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl Blend {
    /// From a /BM name; Compatible is Normal. None for a name we do not know.
    pub fn from_name(name: &[u8]) -> Option<Blend> {
        Some(match name {
            b"Normal" | b"Compatible" => Blend::Normal,
            b"Multiply" => Blend::Multiply,
            b"Screen" => Blend::Screen,
            b"Overlay" => Blend::Overlay,
            b"Darken" => Blend::Darken,
            b"Lighten" => Blend::Lighten,
            b"ColorDodge" => Blend::ColorDodge,
            b"ColorBurn" => Blend::ColorBurn,
            b"HardLight" => Blend::HardLight,
            b"SoftLight" => Blend::SoftLight,
            b"Difference" => Blend::Difference,
            b"Exclusion" => Blend::Exclusion,
            b"Hue" => Blend::Hue,
            b"Saturation" => Blend::Saturation,
            b"Color" => Blend::Color,
            b"Luminosity" => Blend::Luminosity,
            _ => return None,
        })
    }

    /// B(cb, cs): the backdrop `b` and the source `s` mixed.
    pub fn mix(self, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
        let each = |f: fn(f32, f32) -> f32| [f(b[0], s[0]), f(b[1], s[1]), f(b[2], s[2])];
        match self {
            Blend::Normal => s,
            Blend::Multiply => each(multiply),
            Blend::Screen => each(screen),
            Blend::Overlay => each(|b, s| hard_light(s, b)),
            Blend::Darken => each(f32::min),
            Blend::Lighten => each(f32::max),
            Blend::ColorDodge => each(|b, s| {
                if b <= 0.0 {
                    0.0
                } else if s >= 1.0 {
                    1.0
                } else {
                    (b / (1.0 - s)).min(1.0)
                }
            }),
            Blend::ColorBurn => each(|b, s| {
                if b >= 1.0 {
                    1.0
                } else if s <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - b) / s).min(1.0)
                }
            }),
            Blend::HardLight => each(hard_light),
            Blend::SoftLight => each(|b, s| {
                if s <= 0.5 {
                    b - (1.0 - 2.0 * s) * b * (1.0 - b)
                } else {
                    let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
                    b + (2.0 * s - 1.0) * (d - b)
                }
            }),
            Blend::Difference => each(|b, s| (b - s).abs()),
            Blend::Exclusion => each(|b, s| b + s - 2.0 * b * s),
            Blend::Hue => set_lum(set_sat(s, sat(b)), lum(b)),
            Blend::Saturation => set_lum(set_sat(b, sat(s)), lum(b)),
            Blend::Color => set_lum(s, lum(b)),
            Blend::Luminosity => set_lum(b, lum(s)),
        }
    }
}

fn multiply(b: f32, s: f32) -> f32 {
    b * s
}

fn screen(b: f32, s: f32) -> f32 {
    b + s - b * s
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 { multiply(b, 2.0 * s) } else { screen(b, 2.0 * s - 1.0) }
}

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut c = c;
    if n < 0.0 && l - n > 0.0 {
        c = c.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 && x - l > 0.0 {
        c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    c.map(|v| v.clamp(0.0, 1.0))
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    // The channels' order, smallest first.
    let mut i = [0usize, 1, 2];
    i.sort_by(|a, b| c[*a].total_cmp(&c[*b]));
    let (lo, mid, hi) = (i[0], i[1], i[2]);
    let mut out = [0.0; 3];
    if c[hi] > c[lo] {
        out[mid] = (c[mid] - c[lo]) * s / (c[hi] - c[lo]);
        out[hi] = s;
    }
    out
}

/// Premultiplied `c` at coverage `a` onto the premultiplied pixel `dst`,
/// mixed by `mode`: Cs (1 - ab) + Cb (1 - as) + as ab B(Cb, Cs), each
/// term premultiplied (the general compositing formula, §11.3.6).
pub(crate) fn composite(dst: &mut [u8], c: Rgba, a: f32, mode: Blend) {
    let a = a.clamp(0.0, 1.0);
    let src = [c[0] * a, c[1] * a, c[2] * a];
    let sa = (c[3] * a).clamp(0.0, 1.0);
    if sa <= 0.0 {
        return;
    }
    let back = [dst[0] as f32 / 255.0, dst[1] as f32 / 255.0, dst[2] as f32 / 255.0];
    let ba = dst[3] as f32 / 255.0;
    let straight = |v: [f32; 3], al: f32| if al > 0.0 { v.map(|x| (x / al).clamp(0.0, 1.0)) } else { [0.0; 3] };
    let mixed = mode.mix(straight(back, ba), straight(src, sa));
    for k in 0..3 {
        let v = src[k] * (1.0 - ba) + back[k] * (1.0 - sa) + sa * ba * mixed[k];
        dst[k] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = ((sa + ba - sa * ba) * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn the_separable_modes() {
        let (b, s) = ([0.2, 0.5, 0.8], [0.6, 0.5, 0.1]);
        assert!(near(Blend::Normal.mix(b, s), s));
        assert!(near(Blend::Multiply.mix(b, s), [0.12, 0.25, 0.08]));
        assert!(near(Blend::Screen.mix(b, s), [0.68, 0.75, 0.82]));
        assert!(near(Blend::Darken.mix(b, s), [0.2, 0.5, 0.1]));
        assert!(near(Blend::Lighten.mix(b, s), [0.6, 0.5, 0.8]));
        assert!(near(Blend::Difference.mix(b, s), [0.4, 0.0, 0.7]));
        assert!(near(Blend::Exclusion.mix(b, s), [0.56, 0.5, 0.74]));
        // Overlay is HardLight with the two swapped.
        assert!(near(Blend::Overlay.mix(b, s), Blend::HardLight.mix(s, b)));
        // White and black are neutral where they should be.
        assert!(near(Blend::Multiply.mix(b, [1.0; 3]), b));
        assert!(near(Blend::Screen.mix(b, [0.0; 3]), b));
        assert!(near(Blend::ColorDodge.mix(b, [0.0; 3]), b));
        assert!(near(Blend::ColorBurn.mix(b, [1.0; 3]), b));
        assert!(near(Blend::SoftLight.mix(b, [0.5; 3]), b));
    }

    #[test]
    fn the_non_separable_modes() {
        let (b, s) = ([0.8, 0.2, 0.2], [0.1, 0.3, 0.9]);
        // Luminosity takes the source's lightness and keeps the backdrop's hue.
        let l = Blend::Luminosity.mix(b, s);
        assert!((lum(l) - lum(s)).abs() < 1e-4);
        assert!(l[0] > l[1] && l[1] >= l[2] - 1e-4, "still reddish: {l:?}");
        // Color keeps the backdrop's lightness.
        assert!((lum(Blend::Color.mix(b, s)) - lum(b)).abs() < 1e-4);
        // Gray has no hue or saturation to give.
        let g = [0.5; 3];
        assert!(near(Blend::Saturation.mix(g, s), [lum(g); 3]));
        for m in [Blend::Hue, Blend::Saturation, Blend::Color, Blend::Luminosity] {
            assert!(m.mix(b, s).iter().all(|v| (0.0..=1.0).contains(v)), "{m:?} in range");
        }
    }

    #[test]
    fn multiply_onto_white_is_the_color_and_onto_nothing_too() {
        let mut px = [255u8, 255, 255, 255];
        composite(&mut px, [0.5, 0.25, 0.0, 1.0], 1.0, Blend::Multiply);
        assert_eq!(px, [128, 64, 0, 255]);
        // Onto a transparent pixel any mode paints the source as it is.
        let mut px = [0u8; 4];
        composite(&mut px, [0.5, 0.25, 0.0, 1.0], 1.0, Blend::Difference);
        assert_eq!(px, [128, 64, 0, 255]);
    }

    #[test]
    fn names() {
        assert_eq!(Blend::from_name(b"Compatible"), Some(Blend::Normal));
        assert_eq!(Blend::from_name(b"SoftLight"), Some(Blend::SoftLight));
        assert_eq!(Blend::from_name(b"Nonsense"), None);
    }
}
