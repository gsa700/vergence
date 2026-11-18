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
    /// The screen's text factor (vergence_ui::Frame::text), already in
    /// `text_size`; for sizes an app names itself (vergence_look::text_size).
    pub text_factor: f32,
    /// How opaque the window's ground is (`vergence_ui::opacity_setting`).
    pub opacity: f32,
    /// The longest time between two clicks of a double-click, in ms.
    pub double_click: u32,
    /// The strip at the window's top under the server's frosted bar, in
    /// points, for a program that draws under it (`Program::under_bar`; 0
    /// otherwise): what is laid out there shows through the frost and takes
    /// no presses.
    pub top: f32,
    /// Words spelled wrong are marked in text fields and areas
    /// (desktop.toml `[text] spellcheck`, or the app's own `spellcheck`).
    pub spell: bool,
    /// How much of the wallpaper's color a tinted window's grounds take
    /// (desktop.toml `[appearance] tint-amount`: subtle, standard, strong).
    pub tint_mix: f32,
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
            text_factor: 1.0,
            opacity: 1.0,
            double_click: 400,
            top: 0.0,
            spell: true,
            tint_mix: TINT_MIX,
        };
        if high_contrast {
            // Text on pure black or white, edges as strong as the text.
            (t.text, t.page) = if dark { (Color::rgb(255, 255, 255), Color::rgb(0, 0, 0)) } else { (Color::rgb(0, 0, 0), Color::rgb(255, 255, 255)) };
            t.line = t.dim;
            t.dim = t.text;
        }
        t
    }

    /// From desktop.toml's text: the program's own `style` in its `section`
    /// if it chose one, else `[appearance]` color-scheme ("dark" or
    /// "light"; anything else leaves the program's own choice,
    /// `default_dark`), accent ("#rrggbb"), contrast ("high"),
    /// reduced-motion; `opacity` in the program's `section` (as
    /// "[settings]") or `[appearance] window-opacity`.
    pub fn from_desktop(text: &str, default_dark: bool, section: &str) -> Theme {
        let (mut dark, mut accent, mut high, mut motion) = (default_dark, None, false, false);
        let mut double_click = 400;
        let mut within = "";
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                within = line;
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            if within == "[mouse]" && key.trim() == "double-click" {
                double_click = toml_value(value).parse::<u32>().map_or(400, |v| v.clamp(100, 2000));
            }
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
        // The app's own style, if it chose one, wins (app settings milestone 1).
        let dark = !vergence_ui::content_light(text, section, !dark);
        let mut t = Theme::new(dark, accent.unwrap_or(c(vergence_look::ACCENT)), high);
        t.reduced_motion = motion;
        t.double_click = double_click;
        t.opacity = vergence_ui::opacity_setting(text, section);
        let on = |v: Option<String>| v.map(|v| v != "false");
        t.tint_mix = tint_amount(vergence_ui::table_value(text, "[appearance]", "tint-amount").as_deref());
        t.spell = on(vergence_ui::table_value(text, section, "spellcheck")).or_else(|| on(vergence_ui::table_value(text, "[text]", "spellcheck"))).unwrap_or(true);
        t
    }

    /// The page's ground, as opaque as the theme says.
    pub fn ground(&self) -> Color {
        Color { a: (self.opacity.clamp(0.0, 1.0) * 255.0).round() as u8, ..self.page }
    }

    /// The theme with a little of `tint` (the wallpaper's color behind the
    /// window) in its grounds: the page, the sidebar and the cards, by the
    /// desktop's amount (8% unless it says otherwise). Text keeps its colors.
    pub fn tinted(&self, tint: Color) -> Theme {
        let k = self.tint_mix.clamp(0.0, 0.3);
        let mix = |c: Color| {
            let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k).round() as u8;
            Color { r: m(c.r, tint.r), g: m(c.g, tint.g), b: m(c.b, tint.b), a: c.a }
        };
        Theme { page: mix(self.page), sidebar: mix(self.sidebar), card: mix(self.card), ..self.clone() }
    }
}

/// How much of the wallpaper's color a tinted window's grounds take: the
/// Standard amount.
pub const TINT_MIX: f32 = 0.08;

/// The tint's amount (Dave, 2026-11-15: "needs settings to tone it down",
/// then a slider): a percentage 0 to 10 in halves (Settings' slider), or a name
/// (subtle 4%, standard 8%, strong 14%).
pub fn tint_amount(value: Option<&str>) -> f32 {
    match value {
        Some("subtle") => 0.04,
        Some("strong") => 0.14,
        Some(v) => v.trim().parse::<f32>().map_or(TINT_MIX, |p| (p / 100.0).clamp(0.0, 0.2)),
        None => TINT_MIX,
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
    #[test]
    fn the_tint_amount_comes_from_the_desktop() {
        let t = |text: &str| Theme::from_desktop(text, true, "[x]").tint_mix;
        assert_eq!(t(""), TINT_MIX);
        assert_eq!(t("[appearance]\ntint-amount = \"subtle\"\n"), 0.04);
        assert_eq!(t("[appearance]\ntint-amount = \"strong\"\n"), 0.14);
        assert_eq!(t("[appearance]\ntint-amount = 5\n"), 0.05);
        assert_eq!(t("[appearance]\ntint-amount = 90\n"), 0.2, "at most 20%");
        let base = Theme::from_desktop("[appearance]\ntint-amount = \"strong\"\n", true, "[x]");
        let red = base.tinted(Color::rgb(255, 0, 0));
        assert!(red.page.r > base.page.r, "the page takes some red");
    }

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
