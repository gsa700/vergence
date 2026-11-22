//! The terminal's colors: the palettes (Tango, Catppuccin, their light
//! ones), the desktop's choice of them, and text kept readable on the light
//! ones. The window itself is in main.rs.

use vergence_scene::Color;

use crate::grid::Color as CellColor;
use crate::settings::config;

/// The terminal's colors: its ground and text, the cursor, and the 16 named ones.
pub(crate) struct Palette {
    pub(crate) bg: Color,
    pub(crate) fg: Color,
    pub(crate) accent: Color,
    pub(crate) ansi: [u32; 16],
    /// The least contrast text keeps against what is behind it (WCAG ratio;
    /// 0: colors as they are). The light palettes' 16 colors were tuned on
    /// black, and Tango's whites and brights all but vanish on white (Dave on
    /// Hubble, 2026-11-13: "the text is light and somewhat hard to see"):
    /// a color under the floor is darkened, its hue kept, until it reads.
    pub(crate) floor: f32,
}

/// The floor of the light palettes: WCAG's for body text.
pub(crate) const LIGHT_FLOOR: f32 = 4.5;

/// `ink` made to keep `floor` against `ground`: as it is if it does, else
/// mixed toward black on a light ground (white on a dark one), the least
/// that reaches it.
pub(crate) fn readable(ink: Color, ground: Color, floor: f32) -> Color {
    let rgb = |c: Color| vergence_look::Rgb(c.r, c.g, c.b);
    let g = rgb(ground);
    if floor <= 0.0 || ink == ground || vergence_look::contrast(rgb(ink), g) >= floor {
        return ink;
    }
    let to = if vergence_look::luminance(g) > 0.18 { 0.0 } else { 255.0 };
    let mix = |v: u8, t: f32| (v as f32 + (to - v as f32) * t).round() as u8;
    for step in 1..=20 {
        let t = step as f32 / 20.0;
        let c = Color::rgba(mix(ink.r, t), mix(ink.g, t), mix(ink.b, t), ink.a);
        if vergence_look::contrast(rgb(c), g) >= floor {
            return c;
        }
    }
    Color::rgba(to as u8, to as u8, to as u8, ink.a)
}

/// `readable`, remembered: every pair of colors a screen has worked out once
/// (2026-11-14: with only the last pair kept, a busy light screen such as
/// btop's alternated between pairs and redid the up to 20 contrast steps cell
/// after cell; tab clicks on Hubble went from 1.85 ms to 3.5 ms on average,
/// 18 ms at worst). Kept per thread, a few thousand pairs at most.
pub(crate) fn readable_cached(ink: Color, ground: Color, floor: f32, floor_bits: u32) -> Color {
    use std::{cell::RefCell, collections::HashMap};
    thread_local! {
        static PAIRS: RefCell<HashMap<(u32, u32, u32), Color>> = RefCell::new(HashMap::new());
    }
    let pack = |c: Color| u32::from_be_bytes([c.r, c.g, c.b, c.a]);
    let key = (pack(ink), pack(ground), floor_bits);
    PAIRS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(c) = m.get(&key) {
            return *c;
        }
        if m.len() >= 4096 {
            m.clear();
        }
        let c = readable(ink, ground, floor);
        m.insert(key, c);
        c
    })
}

pub(crate) const fn rgb(v: u32) -> Color {
    Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Tango: GNOME Terminal's palette, the look 16-color programs were tuned against.
pub(crate) const TANGO: Palette = Palette {
    bg: rgb(0x2e3436),
    fg: rgb(0xd3d7cf),
    accent: rgb(0x729fcf),
    ansi: [
        0x2e3436, 0xcc0000, 0x4e9a06, 0xc4a000, 0x3465a4, 0x75507b, 0x06989a, 0xd3d7cf, 0x555753, 0xef2929, 0x8ae234,
        0xfce94f, 0x729fcf, 0xad7fa8, 0x34e2e2, 0xeeeeec,
    ],
    floor: 0.0,
};

/// Catppuccin Mocha, the first terminal's default.
pub(crate) const CATPPUCCIN: Palette = Palette {
    bg: rgb(0x1e1e2e),
    fg: rgb(0xcdd6f4),
    accent: rgb(0x89b4fa),
    ansi: [
        0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, 0x585b70, 0xf38ba8, 0xa6e3a1,
        0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
    ],
    floor: 0.0,
};

/// Tango on white, for the light desktop.
pub(crate) const TANGO_LIGHT: Palette =
    Palette { bg: rgb(0xffffff), fg: rgb(0x2e3436), accent: rgb(0x3465a4), ansi: TANGO.ansi, floor: LIGHT_FLOOR };

/// Catppuccin Latte, the light one.
pub(crate) const LATTE: Palette = Palette {
    bg: rgb(0xeff1f5),
    fg: rgb(0x4c4f69),
    accent: rgb(0x1e66f5),
    ansi: [
        0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be, 0x6c6f85, 0xd20f39, 0x40a02b,
        0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xbcc0cc,
    ],
    floor: LIGHT_FLOOR,
};

/// "#rrggbb" -> a color.
fn parse_rgb(s: &str) -> Option<Color> {
    let h = s.trim().strip_prefix('#')?;
    (h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| rgb(u32::from_str_radix(h, 16).unwrap_or(0)))
}

/// The colors desktop.toml asks for, as the first terminal read it:
/// `[terminal] scheme` chooses Tango or Catppuccin (Catppuccin if nothing
/// does); under `[appearance]`, `color-scheme = "light"` the light one of
/// the pair, `contrast = "high"` the text on pure black or white, and
/// `accent = "#rrggbb"` the color of the cursor, the selection and the
/// highlights.
pub(crate) fn palette_from(text: &str) -> Palette {
    // The terminal's own colors if its settings chose them, else the desktop's terminal colors.
    let (mut tango, mut own, mut high, mut accent) = (false, None, false, None);
    let mut section = "";
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches('"');
        match (section, key.trim()) {
            ("[terminal]", "scheme") => own = Some(value.eq_ignore_ascii_case("tango")),
            ("[appearance]", "terminal-colors") => tango = value.eq_ignore_ascii_case("tango"),
            ("[appearance]", "contrast") => high = value == "high",
            ("[appearance]", "accent") => accent = parse_rgb(value),
            _ => {}
        }
    }
    let tango = own.unwrap_or(tango);
    // The terminal's own style if it chose one, else the desktop's; dark under App default.
    let light = vergence_ui::content_light(text, "[terminal]", false);
    let mut p = match (tango, light) {
        (true, false) => TANGO,
        (true, true) => TANGO_LIGHT,
        (false, false) => CATPPUCCIN,
        (false, true) => LATTE,
    };
    if high {
        (p.bg, p.fg) = if light { (rgb(0xffffff), rgb(0x000000)) } else { (rgb(0x000000), rgb(0xffffff)) };
    }
    if let Some(accent) = accent {
        p.accent = accent;
    }
    p
}

pub(crate) fn palette() -> Palette {
    palette_from(&config("desktop.toml"))
}

impl Palette {
    pub(crate) fn indexed(&self, i: u8) -> Color {
        match i {
            0..=15 => rgb(self.ansi[i as usize]),
            16..=231 => {
                let n = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Color::rgb(level(n / 36), level((n / 6) % 6), level(n % 6))
            }
            _ => {
                let g = 8 + (i - 232) * 10;
                Color::rgb(g, g, g)
            }
        }
    }

    pub(crate) fn resolve(&self, c: CellColor, default: Color, bold: bool) -> Color {
        match c {
            CellColor::Default => default,
            // Bold makes the first 8 colors bright, as xterm does.
            CellColor::Indexed(i) if bold && i < 8 => self.indexed(i + 8),
            CellColor::Indexed(i) => self.indexed(i),
            CellColor::Rgb(r, g, b) => Color::rgb(r, g, b),
        }
    }
}
