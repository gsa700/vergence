//! The colors, text and corners widgets are drawn with: `vergence-look`'s,
//! with the user's light or dark, accent and contrast laid over them.

use vergence_look::{PALETTE_DARK, PALETTE_LIGHT, Palette, Rgb};
use vergence_scene::Color;

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub dark: bool,
    pub high_contrast: bool,
    pub reduced_motion: bool,
    pub accent: Color,
    pub sidebar: Color,
    pub page: Color,
    pub card: Color,
    pub line: Color,
    pub dim: Color,
    pub text: Color,
    pub hover: Color,
    /// The interface font's family, and the size of text that names none.
    pub font: String,
    pub text_size: f32,
    /// How opaque the window's ground is (`vergence_ui::opacity_setting`).
    pub opacity: f32,
}

fn c(rgb: Rgb) -> Color {
    Color::rgb(rgb.0, rgb.1, rgb.2)
}

impl Theme {
    pub fn new(dark: bool, accent: Color, high_contrast: bool) -> Theme {
        let p: Palette = if dark { PALETTE_DARK } else { PALETTE_LIGHT };
        let mut t = Theme {
            dark,
            high_contrast,
            reduced_motion: false,
            accent,
            sidebar: c(p.sidebar),
            page: c(p.page),
            card: c(p.card),
            line: c(p.line),
            dim: c(p.dim),
            text: c(p.text),
            hover: Color { a: p.hover_alpha, ..c(p.hover) },
            font: "sans-serif".into(),
            text_size: 13.0,
            opacity: 1.0,
        };
        if high_contrast {
            // Text on pure black or white, edges as strong as the text.
            (t.text, t.page) = if dark { (Color::rgb(255, 255, 255), Color::rgb(0, 0, 0)) } else { (Color::rgb(0, 0, 0), Color::rgb(255, 255, 255)) };
            t.line = t.dim;
            t.dim = t.text;
        }
        t
    }

    /// From desktop.toml's text: `[appearance]` color-scheme ("dark" or
    /// "light"; anything else leaves the program's own choice,
    /// `default_dark`), accent ("#rrggbb"), contrast ("high"),
    /// reduced-motion; `opacity` in the program's `section` (as
    /// "[settings]") or `[appearance] window-opacity`.
    pub fn from_desktop(text: &str, default_dark: bool, section: &str) -> Theme {
        let (mut dark, mut accent, mut high, mut motion) = (default_dark, None, false, false);
        let mut within = "";
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                within = line;
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            if within != "[appearance]" {
                continue;
            }
            let value = toml_value(value);
            match key.trim() {
                "color-scheme" if value == "dark" => dark = true,
                "color-scheme" if value == "light" => dark = false,
                "accent" => accent = parse_hex(value),
                "contrast" => high = value == "high",
                "reduced-motion" => motion = value == "true",
                _ => {}
            }
        }
        let mut t = Theme::new(dark, accent.unwrap_or(c(vergence_look::ACCENT)), high);
        t.reduced_motion = motion;
        t.opacity = vergence_ui::opacity_setting(text, section);
        t
    }

    /// The page's ground, as opaque as the theme says.
    pub fn ground(&self) -> Color {
        Color { a: (self.opacity.clamp(0.0, 1.0) * 255.0).round() as u8, ..self.page }
    }
}

impl Default for Theme {
    fn default() -> Theme {
        Theme::new(true, c(vergence_look::ACCENT), false)
    }
}

/// A value as written after `=`: a quoted string's inside, else the text
/// before any comment.
fn toml_value(v: &str) -> &str {
    let v = v.trim();
    match v.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(""),
        None => v.split('#').next().unwrap_or("").trim(),
    }
}

/// "#rrggbb" to a color.
pub fn parse_hex(s: &str) -> Option<Color> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_s_words() {
        let t = Theme::from_desktop("[appearance]\ncolor-scheme = \"light\"\naccent = \"#ff8000\"\n", true, "[settings]");
        assert!(!t.dark);
        assert_eq!(t.accent, Color::rgb(255, 128, 0));
        assert_eq!(t.page, c(PALETTE_LIGHT.page));
        let t = Theme::from_desktop("[appearance]\ncolor-scheme = \"default\"\n", false, "[settings]");
        assert!(!t.dark, "the program's own choice");
        let t = Theme::from_desktop("[appearance]\ncontrast = \"high\"\nreduced-motion = true\n[settings]\nopacity = 0.8\n", true, "[settings]");
        assert!(t.high_contrast && t.reduced_motion && t.dark);
        assert_eq!((t.text, t.page), (Color::rgb(255, 255, 255), Color::rgb(0, 0, 0)));
        assert_eq!(t.ground().a, 204);
        let t = Theme::from_desktop("[wallpaper]\ncolor-scheme = \"light\"\n", true, "[settings]");
        assert!(t.dark, "another table's word");
    }
}
