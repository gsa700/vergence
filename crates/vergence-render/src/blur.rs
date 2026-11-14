//! The frost: what lies under a bar, blurred and tinted (look milestone 1,
//! docs/look-milestone-1.md, step 3).
//!
//! One pass over the picture under the bar: a blur of [`TAPS`] samples laid
//! on a spiral (the golden angle), weighted by a bell, then the style's tint
//! laid over it. **The tint is only as heavy as reading needs**: it starts at
//! the frost's own strength and, pixel by pixel, is raised just far enough
//! that the bar's ink keeps the contrast it is asked for over what is under
//! it. Dark content under a dark bar stays clear; a white page under it gets
//! as much tint as the title needs. So the contrast rule is kept on whatever
//! the window shows, not on a guess about it.
//!
//! The bar is a strip, so one pass with no picture in between is cheap
//! (3840 x 72 pixels at scale 2, a few dozen reads each). [`FROST_SHADER`] is
//! written for the GLES renderer the server composites with today (Smithay's
//! texture-shader conventions: `tex`, `alpha`, `v_coords`, the `//_DEFINES_`
//! line); it moves to vergence-render's own GPU renderer with Smithay's
//! replacement (docs/borrowed-review.md, step 2). [`frost`] is the same
//! arithmetic on the CPU: the reference the tests hold the rule to.
//!
//! Colors are blended as stored (sRGB values), as the renderers do; the
//! contrast is WCAG 2's, on those blended colors.

/// Samples per pixel.
pub const TAPS: usize = 24;
const GOLDEN_ANGLE: f32 = 2.399_963_2;

/// The frost's fragment shader (GLSL ES 1.00) for a texture-shader program.
/// Its own uniforms: `texel` (vec2, one pixel of the picture in texture
/// coordinates), `radius` (float, the spiral's reach in pixels), `tint`
/// (vec4: the style's bar ground, sRGB, and the frost's own strength),
/// `ink` (vec3, sRGB), `need` (float, the contrast to keep) and `corner`
/// (float, the bar's top corners' radius in pixels; 0 for square).
pub const FROST_SHADER: &str = r#"#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;
uniform vec2 texel;
uniform float radius;
uniform vec4 tint;
uniform vec3 ink;
uniform float need;
uniform float corner;

float lin(float c) {
    return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4);
}

float lum(vec3 c) {
    return 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
}

float contrast(vec3 a, vec3 b) {
    float la = lum(a);
    float lb = lum(b);
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05);
}

void main() {
    vec4 sum = vec4(0.0);
    float weights = 0.0;
    for (int i = 0; i < 24; i++) {
        float f = float(i);
        float r = sqrt((f + 0.5) / 24.0);
        float a = f * 2.3999632;
        vec2 offset = vec2(cos(a), sin(a)) * (r * radius);
        float w = exp(-2.0 * r * r);
        vec4 c = texture2D(tex, v_coords + offset * texel);
#if defined(NO_ALPHA)
        c.a = 1.0;
#endif
        sum += c * w;
        weights += w;
    }
    vec4 blurred = sum / weights;
    // A see-through picture shows the tint where it is clear (its colors
    // have their alpha multiplied in).
    vec3 under = blurred.rgb + (1.0 - blurred.a) * tint.rgb;
    float strength = tint.a;
    if (contrast(ink, mix(under, tint.rgb, strength)) < need) {
        float low = strength;
        float high = 1.0;
        for (int k = 0; k < 10; k++) {
            float mid = 0.5 * (low + high);
            if (contrast(ink, mix(under, tint.rgb, mid)) >= need) {
                high = mid;
            } else {
                low = mid;
            }
        }
        strength = high;
    }
    // The bar's top corners, rounded (look milestone 1, step 7; 0: square).
    float cover = 1.0;
    if (corner > 0.0) {
        vec2 p = v_coords / texel;
        if (p.y < corner) {
            vec2 c = vec2(clamp(p.x, corner, 1.0 / texel.x - corner), corner);
            cover = clamp(corner - distance(p, c) + 0.5, 0.0, 1.0);
        }
    }
    gl_FragColor = vec4(mix(under, tint.rgb, strength), 1.0) * (alpha * cover);
}
"#;

/// What a frost is made with: the style's tint (sRGB) and the frost's own
/// strength (0 to 1), the ink that must read on it, the contrast to keep,
/// and the blur's reach in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frost {
    pub tint: [u8; 3],
    pub strength: f32,
    pub ink: [u8; 3],
    pub need: f32,
    pub radius: f32,
}

fn lin(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn lum(c: [f32; 3]) -> f32 {
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

fn contrast(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// The tint's strength over a color `under` (sRGB, 0 to 1) that keeps the
/// ink's contrast: the frost's own, or raised just far enough (as the shader
/// finds it, by halving ten times).
pub fn strength_over(under: [f32; 3], f: &Frost) -> f32 {
    let tint = f.tint.map(|v| v as f32 / 255.0);
    let ink = f.ink.map(|v| v as f32 / 255.0);
    if contrast(ink, mix(under, tint, f.strength)) >= f.need {
        return f.strength;
    }
    let (mut low, mut high) = (f.strength, 1.0f32);
    for _ in 0..10 {
        let mid = 0.5 * (low + high);
        if contrast(ink, mix(under, tint, mid)) >= f.need {
            high = mid;
        } else {
            low = mid;
        }
    }
    high
}

/// The frost of a picture's top `rows` (8-bit RGBA, alpha multiplied in,
/// `width` pixels a row): opaque RGBA of `width` x `rows`, the CPU's
/// reference for [`FROST_SHADER`] (samples taken at the nearest pixel, the
/// picture's edges repeated beyond it).
pub fn frost(rgba: &[u8], width: usize, height: usize, rows: usize, f: &Frost) -> Vec<u8> {
    let rows = rows.min(height);
    let tint = f.tint.map(|v| v as f32 / 255.0);
    let taps: Vec<(f32, f32, f32)> = (0..TAPS)
        .map(|i| {
            let r = ((i as f32 + 0.5) / TAPS as f32).sqrt();
            let a = i as f32 * GOLDEN_ANGLE;
            (a.cos() * r * f.radius, a.sin() * r * f.radius, (-2.0 * r * r).exp())
        })
        .collect();
    let weights: f32 = taps.iter().map(|t| t.2).sum();
    let mut out = vec![0u8; width * rows * 4];
    for y in 0..rows {
        for x in 0..width {
            let mut sum = [0f32; 4];
            for (dx, dy, w) in &taps {
                let sx = ((x as f32 + 0.5 + dx).floor() as isize).clamp(0, width as isize - 1) as usize;
                let sy = ((y as f32 + 0.5 + dy).floor() as isize).clamp(0, height as isize - 1) as usize;
                let p = &rgba[(sy * width + sx) * 4..][..4];
                for c in 0..4 {
                    sum[c] += p[c] as f32 / 255.0 * w;
                }
            }
            let b = sum.map(|v| v / weights);
            let under = [b[0] + (1.0 - b[3]) * tint[0], b[1] + (1.0 - b[3]) * tint[1], b[2] + (1.0 - b[3]) * tint[2]];
            let c = mix(under, tint, strength_over(under, f));
            let o = &mut out[(y * width + x) * 4..][..4];
            for i in 0..3 {
                o[i] = (c[i] * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            o[3] = 255;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DARK: Frost = Frost { tint: [51, 61, 82], strength: 0.5, ink: [235, 235, 235], need: 7.0, radius: 14.0 };
    const LIGHT: Frost = Frost { tint: [228, 230, 235], strength: 0.5, ink: [29, 29, 31], need: 5.85, radius: 14.0 };

    fn picture(width: usize, height: usize, at: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        let mut v = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                v.extend(at(x, y));
            }
        }
        v
    }

    fn worst(out: &[u8], ink: [u8; 3]) -> f32 {
        let ink = ink.map(|v| v as f32 / 255.0);
        out.chunks(4).map(|p| contrast(ink, [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0])).fold(f32::MAX, f32::min)
    }

    /// The rule itself: whatever is under the bar, every pixel of the frost
    /// keeps the ink's contrast (allowing for the result being stored in 8 bits).
    #[test]
    fn the_frost_keeps_the_ink_readable_over_anything() {
        let (w, h) = (64, 40);
        let pictures = [
            picture(w, h, |_, _| [255, 255, 255, 255]),
            picture(w, h, |_, _| [0, 0, 0, 255]),
            picture(w, h, |x, _| if (x / 3) % 2 == 0 { [255, 255, 255, 255] } else { [0, 0, 0, 255] }),
            picture(w, h, |x, y| [(x * 4) as u8, (y * 6) as u8, ((x + y) * 3) as u8, 255]),
            picture(w, h, |x, y| if (x * 7 + y * 13) % 5 == 0 { [255, 255, 0, 255] } else { [20, 40, 200, 255] }),
            // A see-through picture (the terminal's ground at half).
            picture(w, h, |_, _| [128, 128, 128, 128]),
        ];
        for f in [DARK, LIGHT] {
            for p in &pictures {
                let out = frost(p, w, h, 24, &f);
                let c = worst(&out, f.ink);
                assert!(c >= f.need * 0.97, "{f:?}: worst {c:.2} < {}", f.need);
            }
        }
    }

    /// Dark content under a dark bar is not tinted beyond the frost's own
    /// strength: the frost stays as clear as it was set.
    #[test]
    fn readable_content_keeps_the_frost_clear() {
        let under = [0.08, 0.09, 0.11];
        assert_eq!(strength_over(under, &DARK), DARK.strength);
        let white = [1.0, 1.0, 1.0];
        let s = strength_over(white, &DARK);
        // 7:1 over the bare frost (headroom for the shapes on it) takes about 91%.
        assert!(s > 0.85 && s < 0.95, "white under a dark bar: {s}");
        assert_eq!(strength_over(white, &LIGHT), LIGHT.strength);
    }

    /// The blur is a blur: a single bright column spreads to its neighbors.
    #[test]
    fn a_sharp_edge_is_softened() {
        let (w, h) = (40, 30);
        let p = picture(w, h, |x, _| if x == 20 { [255, 255, 255, 255] } else { [0, 0, 0, 255] });
        let f = Frost { tint: [0, 0, 0], strength: 0.0, ink: [255, 255, 255], need: 1.0, radius: 6.0 };
        let out = frost(&p, w, h, 10, &f);
        let at = |x: usize| out[(5 * w + x) * 4];
        assert!(at(20) < 200, "the line itself is spread out: {}", at(20));
        assert!(at(22) > 0 && at(18) > 0, "its neighbors catch some of it");
        assert_eq!(at(0), 0);
    }

    #[test]
    fn the_shader_names_what_the_server_sets() {
        for name in ["texel", "radius", "tint", "ink", "need", "corner", "//_DEFINES_", "v_coords", "alpha"] {
            assert!(FROST_SHADER.contains(name), "{name}");
        }
        assert!(FROST_SHADER.contains(&format!("i < {TAPS}")));
    }
}
