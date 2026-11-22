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
    /// The interface font's family, and the size of text that names none
    /// (the desktop's, Gtk/FontName in points: vergence_ui::DesktopFonts).
    pub font: String,
    pub text_size: f32,
    /// The monospace font's family and size (Gtk/MonospaceFontName), for
    /// text areas in monospace (Edit), before the screen's text factor.
    pub mono: String,
    pub mono_size: f32,
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
    /// (desktop.toml `[appearance] tint-strength`: subtle, standard, strong).
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
            text_size: vergence_ui::DEFAULT_TEXT,
            mono: "monospace".into(),
            mono_size: vergence_ui::DEFAULT_MONO_TEXT,
            text_factor: 1.0,
            opacity: 1.0,
            double_click: 400,
            top: 0.0,
            spell: true,
            tint_mix: TINT_MIX,
        };
        if high_contrast {
            // Text on pure black or white, edges as strong as the text.
            (t.text, t.page) = if dark {
                (Color::rgb(255, 255, 255), Color::rgb(0, 0, 0))
            } else {
                (Color::rgb(0, 0, 0), Color::rgb(255, 255, 255))
            };
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
    /// "`[settings]`") or `[appearance] window-opacity`.
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
        t.set_fonts(&vergence_ui::DesktopFonts::current());
        t.opacity = vergence_ui::opacity_setting(text, section);
        let on = |v: Option<String>| v.map(|v| v != "false");
        t.tint_mix = tint_amount(vergence_ui::table_value(text, "[appearance]", "tint-strength").as_deref());
        t.spell = on(vergence_ui::table_value(text, section, "spellcheck"))
            .or_else(|| on(vergence_ui::table_value(text, "[text]", "spellcheck")))
            .unwrap_or(true);
        t
    }

    /// The desktop's fonts (text milestone 1, step 5).
    pub fn set_fonts(&mut self, f: &vergence_ui::DesktopFonts) {
        self.font = f.family.clone();
        self.text_size = f.size.clamp(6.0, 48.0);
        self.mono = f.mono.clone();
        self.mono_size = f.mono_size.clamp(6.0, 72.0);
    }

    /// The page's ground, as opaque as the theme says.
    pub fn ground(&self) -> Color {
        Color { a: (self.opacity.clamp(0.0, 1.0) * 255.0).round() as u8, ..self.page }
    }

    /// The theme with some of `tint` (the wallpaper's color behind the window)
    /// in its grounds: the page, the sidebar and the cards, by the desktop's
    /// strength (25% unless it says otherwise). Text keeps its colors.
    ///
    /// The wallpaper's color only, not its brightness (defaults milestone 1,
    /// Dave 2026-11-22: "color only"): each ground moves toward the tint's hue
    /// and saturation at the ground's own lightness. Mixing the raw color did
    /// nothing under a dark wallpaper (Dusk's average moved the dark page by
    /// one level) and only lightened dark windows under a light one.
    pub fn tinted(&self, tint: Color) -> Theme {
        let k = self.tint_mix.clamp(0.0, 0.6);
        let (th, _, ts) = hls(tint);
        let mix = |c: Color| {
            let (_, gl, _) = hls(c);
            let target = from_hls(th, gl, ts);
            let m = |a: u8, b: f32| (a as f32 + (b - a as f32) * k).round().clamp(0.0, 255.0) as u8;
            Color { r: m(c.r, target.0), g: m(c.g, target.1), b: m(c.b, target.2), a: c.a }
        };
        Theme { page: mix(self.page), sidebar: mix(self.sidebar), card: mix(self.card), ..self.clone() }
    }
}

/// How much of the wallpaper's color a tinted window's grounds take unless
/// the desktop says: 25% (Dave chose it from a swatch sheet, 2026-11-22).
pub const TINT_MIX: f32 = 0.25;

/// The tint's strength (desktop.toml `[appearance] tint-strength`): a
/// percentage 0 to 60 (Settings' slider, in fives), or a name (subtle 15%,
/// standard 25%, strong 40%). A new key with the color-only tint: the old
/// `tint-amount` (0 to 10% of the raw color) meant something else, so it is
/// not read.
pub fn tint_amount(value: Option<&str>) -> f32 {
    match value {
        Some("subtle") => 0.15,
        Some("standard") => 0.25,
        Some("strong") => 0.40,
        Some(v) => v.trim().parse::<f32>().map_or(TINT_MIX, |p| (p / 100.0).clamp(0.0, 0.6)),
        None => TINT_MIX,
    }
}

/// A color as hue, lightness and saturation, each 0 to 1 (HLS).
fn hls(c: Color) -> (f32, f32, f32) {
    let (r, g, b) = (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if max - min < 1e-6 {
        return (0.0, l, 0.0);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    (h, l, s)
}

/// Hue, lightness and saturation back to 0-255 channels (unrounded).
fn from_hls(h: f32, l: f32, s: f32) -> (f32, f32, f32) {
    if s < 1e-6 {
        return (l * 255.0, l * 255.0, l * 255.0);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let channel = |t: f32| {
        let t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        v * 255.0
    };
    (channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
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
    fn the_tint_strength_comes_from_the_desktop() {
        let t = |text: &str| Theme::from_desktop(text, true, "[x]").tint_mix;
        assert_eq!(t(""), TINT_MIX);
        assert_eq!(t("[appearance]\ntint-strength = \"subtle\"\n"), 0.15);
        assert_eq!(t("[appearance]\ntint-strength = \"standard\"\n"), 0.25);
        assert_eq!(t("[appearance]\ntint-strength = \"strong\"\n"), 0.40);
        assert_eq!(t("[appearance]\ntint-strength = 35\n"), 0.35);
        assert_eq!(t("[appearance]\ntint-strength = 90\n"), 0.6, "at most 60%");
        assert_eq!(t("[appearance]\ntint-amount = 8\n"), TINT_MIX, "the old key is not read");
        let base = Theme::from_desktop("[appearance]\ntint-strength = \"strong\"\n", true, "[x]");
        let red = base.tinted(Color::rgb(255, 0, 0));
        assert!(red.page.r > base.page.r, "the page takes some red");
    }

    #[test]
    fn the_tint_is_color_not_brightness() {
        let base = Theme::new(true, Color::rgb(0, 122, 255), false);
        let lum = |c: Color| 0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32;
        // A light gray wallpaper: no hue, so a dark window keeps its grounds.
        let mist = base.tinted(Color::rgb(0xe0, 0xe0, 0xe0));
        assert_eq!(mist.page, base.page, "a gray wallpaper does not lighten a dark window");
        // A dark indigo wallpaper (Dusk): the page turns bluer at about its own lightness.
        let dusk = base.tinted(Color::rgb(0x23, 0x28, 0x49));
        assert!(dusk.page.b > dusk.page.r, "the page takes the indigo");
        assert!((lum(dusk.page) - lum(base.page)).abs() < 3.0, "and keeps its lightness");
    }

    #[test]
    fn hls_goes_both_ways() {
        for c in
            [Color::rgb(0x23, 0x28, 0x49), Color::rgb(255, 0, 0), Color::rgb(0x1c, 0x1c, 0x1e), Color::rgb(8, 46, 56)]
        {
            let (h, l, s) = hls(c);
            let (r, g, b) = from_hls(h, l, s);
            assert!(
                (r - c.r as f32).abs() < 1.0 && (g - c.g as f32).abs() < 1.0 && (b - c.b as f32).abs() < 1.0,
                "{c:?}"
            );
        }
    }

    use super::*;

    #[test]
    fn the_desktop_s_words() {
        let t =
            Theme::from_desktop("[appearance]\ncolor-scheme = \"light\"\naccent = \"#ff8000\"\n", true, "[settings]");
        assert!(!t.dark);
        assert_eq!(t.accent, Color::rgb(255, 128, 0));
        assert_eq!(t.page, c(PALETTE_LIGHT.page));
        let t = Theme::from_desktop("[appearance]\ncolor-scheme = \"default\"\n", false, "[settings]");
        assert!(!t.dark, "the program's own choice");
        let t = Theme::from_desktop(
            "[appearance]\ncontrast = \"high\"\nreduced-motion = true\n[settings]\nopacity = 0.8\n",
            true,
            "[settings]",
        );
        assert!(t.high_contrast && t.reduced_motion && t.dark);
        assert_eq!((t.text, t.page), (Color::rgb(255, 255, 255), Color::rgb(0, 0, 0)));
        assert_eq!(t.ground().a, 204);
        let t = Theme::from_desktop("[wallpaper]\ncolor-scheme = \"light\"\n", true, "[settings]");
        assert!(t.dark, "another table's word");
    }
}
