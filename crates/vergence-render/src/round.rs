//! A window's rounded corners (look milestone 1, step 7): its picture drawn
//! with its bottom corners cut to the desktop's one radius, the edge of the
//! cut softened over a pixel. The top corners belong to the bar, which the
//! server draws rounded itself (and the frost under it, [`crate::blur`]);
//! but a window that draws under its bar has its own top corners there too,
//! and they are cut as well (`top`; otherwise a light window's corners show
//! as white specks round a dark bar).
//!
//! Written for the GLES renderer the server composites with today (Smithay's
//! texture-shader conventions, as the frost); it moves to vergence-render's
//! own GPU renderer with Smithay's replacement.

/// The shader (GLSL ES 1.00). Its own uniforms: `size` (vec2, the picture in
/// pixels), `radius` (float, the corner's radius in pixels) and `top` (1.0:
/// the top corners are cut too, for a window drawn under its bar).
pub const ROUND_SHADER: &str = r#"#version 100

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
uniform vec2 size;
uniform float radius;
uniform float top;

void main() {
    vec4 color = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif
    vec2 p = v_coords * size;
    float cover = 1.0;
    if (p.y > size.y - radius) {
        vec2 c = vec2(clamp(p.x, radius, size.x - radius), size.y - radius);
        cover = clamp(radius - distance(p, c) + 0.5, 0.0, 1.0);
    } else if (top > 0.5 && p.y < radius) {
        vec2 c = vec2(clamp(p.x, radius, size.x - radius), radius);
        cover = clamp(radius - distance(p, c) + 0.5, 0.0, 1.0);
    }
    gl_FragColor = color * alpha * cover;
}
"#;

/// How much of the pixel whose center is `(x, y)` (in pixels from the
/// picture's top left) a picture of `w` x `h` with bottom corners (and top
/// ones, if `top`) of `radius` covers: the shader's arithmetic, for tests.
pub fn cover(x: f32, y: f32, w: f32, h: f32, radius: f32, top: bool) -> f32 {
    let cy = if y > h - radius {
        h - radius
    } else if top && y < radius {
        radius
    } else {
        return 1.0;
    };
    let cx = x.clamp(radius, w - radius);
    (radius - ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() + 0.5).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_bottom_corners_are_cut() {
        let (w, h, r) = (100.0, 60.0, 10.0);
        assert_eq!(cover(0.5, 0.5, w, h, r, false), 1.0, "top left stays: the bar's corner");
        assert_eq!(cover(50.0, 59.5, w, h, r, false), 1.0, "the bottom edge between the corners stays");
        assert_eq!(cover(0.5, 59.5, w, h, r, false), 0.0, "the bottom left corner pixel is cut");
        assert_eq!(cover(99.5, 59.5, w, h, r, false), 0.0, "and the bottom right one");
        assert_eq!(cover(0.5, 0.5, w, h, r, true), 0.0, "under its bar, the top left is cut too");
        assert_eq!(cover(99.5, 0.5, w, h, r, true), 0.0, "and the top right");
        assert_eq!(cover(50.0, 0.5, w, h, r, true), 1.0, "the top edge between them stays");
        let on_curve = cover(10.0 - 10.0 * 0.7071, 50.0 + 10.0 * 0.7071, w, h, r, false);
        assert!(on_curve > 0.3 && on_curve < 0.7, "the curve itself is softened: {on_curve}");
    }

    #[test]
    fn the_shader_names_what_the_server_sets() {
        for name in ["size", "radius", "top", "//_DEFINES_", "v_coords", "alpha"] {
            assert!(ROUND_SHADER.contains(name), "{name}");
        }
    }
}
