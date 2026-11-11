// Text for the server's own drawing (title bars, 2026-11-07).
//
// One line of text, rasterized with ab_glyph (pure Rust) straight into an
// output's physical pixels, so it is sharp at every monitor's scale: each
// monitor gets its own raster, drawn 1:1. The font is the desktop's: the family
// and size of Gtk/FontName in ~/.config/vergence/settings.toml (vergd's file,
// "Sans 10" on Hubble), found with fc-match; sizes are in points (1/72 in at the
// reference 96 units per inch, like GTK). Kerning, no shaping: enough for window
// titles in Latin scripts; full shaping comes with the real text stack (Vello).

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};

pub struct TextFont {
    font: FontVec,
    /// Size in points.
    pub points: f32,
}

impl TextFont {
    /// The desktop font (Gtk/FontName), or None if no font file can be found.
    pub fn load() -> Option<TextFont> {
        let (family, points) = desktop_font();
        let path = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", &family])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .filter(|p| !p.is_empty())?;
        let bytes = std::fs::read(&path).ok()?;
        let font = FontVec::try_from_vec(bytes).ok()?;
        tracing::info!("title font: {family} {points} pt ({path})");
        Some(TextFont { font, points })
    }

    /// Width of `text` in one line, in logical units (points at 96 per inch).
    pub fn width(&self, text: &str) -> f32 {
        let em = self.points as f64 * 96.0 / 72.0;
        let units = self.font.units_per_em().unwrap_or(1000.0) as f64;
        let px = PxScale::from((em * self.font.height_unscaled() as f64 / units) as f32);
        let sf = self.font.as_scaled(px);
        let mut x = 0.0;
        let mut prev = None;
        for c in text.chars() {
            let g = sf.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, g);
            }
            x += sf.h_advance(g);
            prev = Some(g);
        }
        x
    }

    /// `text` in one line, `w` x `h` premultiplied RGBA pixels, `color` (straight
    /// RGB 0-1), left aligned, centered vertically; cut with an ellipsis if too long.
    /// `scale` is the output's (physical pixels per point).
    pub fn render(&self, text: &str, w: usize, h: usize, scale: f64, color: [f32; 3]) -> Vec<u8> {
        let mut out = vec![0u8; w * h * 4];
        if w == 0 || h == 0 {
            return out;
        }
        // Em in physical pixels: points at 96 units per inch, times the output scale.
        let em = self.points as f64 * 96.0 / 72.0 * scale;
        let units = self.font.units_per_em().unwrap_or(1000.0) as f64;
        let px = PxScale::from((em * self.font.height_unscaled() as f64 / units) as f32);
        let sf = self.font.as_scaled(px);

        let advance = |s: &str| -> f32 {
            let mut x = 0.0;
            let mut prev = None;
            for c in s.chars() {
                let g = sf.glyph_id(c);
                if let Some(p) = prev {
                    x += sf.kern(p, g);
                }
                x += sf.h_advance(g);
                prev = Some(g);
            }
            x
        };
        let mut line: String = text.chars().filter(|c| !c.is_control()).collect();
        if advance(&line) > w as f32 {
            let ell = "\u{2026}";
            while !line.is_empty() && advance(&format!("{line}{ell}")) > w as f32 {
                line.pop();
            }
            line = format!("{}{ell}", line.trim_end());
        }

        let baseline = ((h as f32 - (sf.ascent() - sf.descent())) / 2.0 + sf.ascent()).round();
        let mut x = 0.0f32;
        let mut prev = None;
        for c in line.chars() {
            let id = sf.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            let glyph = id.with_scale_and_position(px, point(x, baseline));
            x += sf.h_advance(id);
            prev = Some(id);
            let Some(outline) = self.font.outline_glyph(glyph) else { continue };
            let b = outline.px_bounds();
            outline.draw(|gx, gy, cov| {
                let (px_x, px_y) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if px_x < 0 || px_y < 0 || px_x as usize >= w || px_y as usize >= h {
                    return;
                }
                let i = (px_y as usize * w + px_x as usize) * 4;
                let a = cov.clamp(0.0, 1.0);
                // Over what is there (glyphs may overlap a little).
                let keep = 1.0 - a;
                for (k, c) in color.iter().enumerate() {
                    out[i + k] = (c * a * 255.0 + out[i + k] as f32 * keep).round() as u8;
                }
                out[i + 3] = (a * 255.0 + out[i + 3] as f32 * keep).round() as u8;
            });
        }
        out
    }
}

/// Family and size from Gtk/FontName ("Sans 10"), else Sans 10.
fn desktop_font() -> (String, f32) {
    let name = std::env::var_os("HOME")
        .map(|h| std::path::Path::new(&h).join(".config/vergence/settings.toml"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| vergence_xsettings::from_toml(&t).ok())
        .and_then(|s| match s.get("Gtk/FontName") {
            Some(vergence_xsettings::Value::Str(v)) => Some(v.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "Sans 10".into());
    parse_font_name(&name)
}

/// "Noto Sans Bold 10.5" -> ("Noto Sans Bold", 10.5); no number -> 10 pt.
fn parse_font_name(name: &str) -> (String, f32) {
    let name = name.trim();
    match name.rsplit_once(' ') {
        Some((family, size)) if size.parse::<f32>().is_ok_and(|s| s > 0.0) => (family.to_string(), size.parse().unwrap()),
        _ => (name.to_string(), 10.0),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_font_name;

    #[test]
    fn font_names() {
        assert_eq!(parse_font_name("Sans 10"), ("Sans".into(), 10.0));
        assert_eq!(parse_font_name("Noto Sans Bold 10.5"), ("Noto Sans Bold".into(), 10.5));
        assert_eq!(parse_font_name("Cantarell"), ("Cantarell".into(), 10.0));
    }
}
