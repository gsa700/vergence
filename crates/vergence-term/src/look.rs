// The terminal's look, from the desktop's settings (step 4, 2026-11-08):
//   ~/.config/vergence/desktop.toml  [appearance]
//       color-scheme = "default" | "dark" | "light"   (default: dark)
//       accent       = "#rrggbb"                       (cursor, selection, highlights)
//       contrast     = "normal" | "high"               (black or white behind the text)
//   ~/.config/vergence/settings.toml  (the settings app's Theme page)
//       "Gtk/MonospaceFontName" = "Adwaita Mono 10"   the terminal's text
//       "Gtk/FontName"          = "Noto Sans 10"      tab titles, the menu
//       "Gtk/DecorationLayout"  = ":minimize,maximize,close"   the window buttons
// The same files the server's settings service reads (vergence-server
// settings.rs); a change applies within a second, to every tab and pane.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use std::collections::HashMap;
use std::sync::Mutex;

use iced::{Color, Font};

/// A window button in the title bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinButton {
    Minimize,
    Maximize,
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub dark: bool,
    pub high_contrast: bool,
    pub accent: Option<Color>,
    /// The monospace font: family (None: the system's monospace) and size in
    /// logical pixels (points at 96 per inch).
    pub mono_family: Option<String>,
    pub font_size: f32,
    /// The interface font (tab titles, the menu): family and size in logical pixels.
    pub ui_family: Option<String>,
    pub ui_size: f32,
    /// The window buttons before the tabs (left) and after them (right), in order.
    pub buttons_left: Vec<WinButton>,
    pub buttons_right: Vec<WinButton>,
}

impl Default for Look {
    fn default() -> Look {
        Look {
            dark: true,
            high_contrast: false,
            accent: None,
            mono_family: None,
            font_size: 10.0 * 96.0 / 72.0,
            ui_family: None,
            ui_size: 10.0 * 96.0 / 72.0,
            buttons_left: Vec::new(),
            buttons_right: vec![WinButton::Minimize, WinButton::Maximize, WinButton::Close],
        }
    }
}

/// "Adwaita Mono 10" -> (Some("Adwaita Mono"), Some(10.0)); a bare family has no size.
fn font_spec(v: &str) -> (Option<String>, Option<f32>) {
    let v = v.trim();
    match v.rsplit_once(' ') {
        Some((family, n)) if n.parse::<f32>().is_ok() => {
            let pt = n.parse::<f32>().ok().filter(|pt| (4.0..=72.0).contains(pt));
            let family = family.trim();
            ((!family.is_empty()).then(|| family.to_string()), pt)
        }
        _ => ((!v.is_empty()).then(|| v.to_string()), None),
    }
}

/// GTK's decoration layout ("menu:minimize,maximize,close"): the buttons we
/// draw on each side of the colon, in order (others, like the app menu, are skipped).
fn decoration_layout(v: &str) -> (Vec<WinButton>, Vec<WinButton>) {
    let side = |s: &str| {
        s.split(',')
            .filter_map(|b| match b.trim() {
                "minimize" => Some(WinButton::Minimize),
                "maximize" => Some(WinButton::Maximize),
                "close" => Some(WinButton::Close),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    match v.split_once(':') {
        Some((l, r)) => (side(l), side(r)),
        None => (Vec::new(), side(v)),
    }
}

/// A font by family name (iced keeps names as &'static str: each family is
/// kept once, however often the settings change).
pub fn font(family: Option<&str>, fallback: Font) -> Font {
    static NAMES: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);
    let Some(family) = family else { return fallback };
    let mut names = NAMES.lock().unwrap();
    let name = *names.get_or_insert_with(HashMap::new).entry(family.to_string()).or_insert_with(|| Box::leak(family.to_string().into_boxed_str()));
    Font { family: iced::font::Family::Name(name), ..fallback }
}

/// A terminal's colours: background, text, the accent (cursor, selection),
/// and the 16 ANSI colours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub bg: Color,
    pub fg: Color,
    pub accent: Color,
    pub ansi: [Color; 16],
}

fn rgb(v: u32) -> Color {
    Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// "#rrggbb" -> a colour.
fn parse_rgb(s: &str) -> Option<Color> {
    let h = s.trim().strip_prefix('#')?;
    (h.len() == 6).then_some(())?;
    u32::from_str_radix(h, 16).ok().map(rgb)
}

impl Look {
    /// The look from the two files' text (either may be missing).
    pub fn from_text(desktop: Option<&str>, xsettings: Option<&str>) -> Look {
        let mut look = Look::default();
        let appearance = desktop.and_then(|t| t.parse::<toml::Table>().ok()).and_then(|t| t.get("appearance").and_then(|a| a.as_table()).cloned()).unwrap_or_default();
        let text = |k: &str| appearance.get(k).and_then(|v| v.as_str()).map(str::to_owned);
        look.dark = text("color-scheme").as_deref() != Some("light");
        look.high_contrast = text("contrast").as_deref() == Some("high");
        look.accent = text("accent").as_deref().and_then(parse_rgb);
        let xs = xsettings.and_then(|t| vergence_xsettings::from_toml(t).ok()).unwrap_or_default();
        let xstr = |k: &str| match xs.get(k) {
            Some(vergence_xsettings::Value::Str(v)) if !v.trim().is_empty() => Some(v.clone()),
            _ => None,
        };
        if let Some(v) = xstr("Gtk/MonospaceFontName") {
            let (family, pt) = font_spec(&v);
            look.mono_family = family;
            if let Some(pt) = pt {
                look.font_size = pt * 96.0 / 72.0;
            }
        }
        if let Some(v) = xstr("Gtk/FontName") {
            let (family, pt) = font_spec(&v);
            look.ui_family = family;
            if let Some(pt) = pt {
                look.ui_size = pt * 96.0 / 72.0;
            }
        }
        if let Some(v) = xstr("Gtk/DecorationLayout") {
            (look.buttons_left, look.buttons_right) = decoration_layout(&v);
        }
        look
    }

    /// The look from the files now.
    pub fn load() -> Look {
        let read = |p: Option<PathBuf>| p.and_then(|p| std::fs::read_to_string(p).ok());
        Look::from_text(read(config("desktop.toml")).as_deref(), read(config("settings.toml")).as_deref())
    }

    /// The terminal's font (the system monospace if none is set).
    pub fn mono_font(&self) -> Font {
        font(self.mono_family.as_deref(), Font::MONOSPACE)
    }

    /// The interface font (the system sans if none is set).
    pub fn ui_font(&self) -> Font {
        font(self.ui_family.as_deref(), Font::DEFAULT)
    }

    /// The colours: Catppuccin's Mocha (dark) and Latte (light) palettes;
    /// high contrast puts the text on pure black or white.
    pub fn palette(&self) -> Palette {
        let mut p = if self.dark {
            Palette {
                bg: rgb(0x1e1e2e),
                fg: rgb(0xcdd6f4),
                accent: rgb(0x89b4fa),
                ansi: [
                    rgb(0x45475a), rgb(0xf38ba8), rgb(0xa6e3a1), rgb(0xf9e2af), rgb(0x89b4fa), rgb(0xf5c2e7), rgb(0x94e2d5), rgb(0xbac2de),
                    rgb(0x585b70), rgb(0xf38ba8), rgb(0xa6e3a1), rgb(0xf9e2af), rgb(0x89b4fa), rgb(0xf5c2e7), rgb(0x94e2d5), rgb(0xa6adc8),
                ],
            }
        } else {
            Palette {
                bg: rgb(0xeff1f5),
                fg: rgb(0x4c4f69),
                accent: rgb(0x1e66f5),
                ansi: [
                    rgb(0x5c5f77), rgb(0xd20f39), rgb(0x40a02b), rgb(0xdf8e1d), rgb(0x1e66f5), rgb(0xea76cb), rgb(0x179299), rgb(0xacb0be),
                    rgb(0x6c6f85), rgb(0xd20f39), rgb(0x40a02b), rgb(0xdf8e1d), rgb(0x1e66f5), rgb(0xea76cb), rgb(0x179299), rgb(0xbcc0cc),
                ],
            }
        };
        if self.high_contrast {
            (p.bg, p.fg) = if self.dark { (Color::BLACK, Color::WHITE) } else { (Color::WHITE, Color::BLACK) };
        }
        if let Some(a) = self.accent {
            p.accent = a;
        }
        p
    }
}

fn config(name: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("vergence").join(name))
}

/// Call `changed` with the new look whenever the files change (checked once
/// a second, by their modification times), until it returns false.
pub fn watch(mut current: Look, mut changed: impl FnMut(Look) -> bool) {
    let stamp = |n: &str| config(n).and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
    let stamps = || -> [Option<SystemTime>; 2] { [stamp("desktop.toml"), stamp("settings.toml")] };
    let mut seen = stamps();
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let now = stamps();
        if now == seen {
            continue;
        }
        seen = now;
        let look = Look::load();
        if look != current {
            current = look.clone();
            if !changed(look) {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_from_the_settings_files() {
        let l = Look::from_text(None, None);
        assert_eq!(l, Look::default());
        assert!(l.dark);
        let l = Look::from_text(Some("[wallpaper]\nmode = \"fill\"\n[appearance]\ncolor-scheme = \"light\"\naccent = \"#ff8800\"\ncontrast = \"high\"\n"), Some("[settings]\n\"Gtk/MonospaceFontName\" = \"Adwaita Mono 12\"\n"));
        assert!(!l.dark && l.high_contrast);
        assert_eq!(l.accent, Some(Color::from_rgb8(0xff, 0x88, 0x00)));
        assert_eq!(l.font_size, 16.0);
        assert_eq!(l.mono_family.as_deref(), Some("Adwaita Mono"));
        let p = l.palette();
        assert_eq!((p.bg, p.fg, p.accent), (Color::WHITE, Color::BLACK, Color::from_rgb8(0xff, 0x88, 0x00)));
        // "default" is dark; a broken accent is no accent.
        let l = Look::from_text(Some("[appearance]\ncolor-scheme = \"default\"\naccent = \"blue\"\n"), None);
        assert!(l.dark && l.accent.is_none());
    }

    #[test]
    fn fonts_and_buttons() {
        assert_eq!(font_spec("Noto Sans 10"), (Some("Noto Sans".into()), Some(10.0)));
        assert_eq!(font_spec("Monospace"), (Some("Monospace".into()), None));
        assert_eq!(decoration_layout(":minimize,maximize,close"), (vec![], vec![WinButton::Minimize, WinButton::Maximize, WinButton::Close]));
        assert_eq!(decoration_layout("close,maximize:appmenu"), (vec![WinButton::Close, WinButton::Maximize], vec![]));
        let l = Look::from_text(None, Some("[settings]\n\"Gtk/FontName\" = \"Inter 11\"\n\"Gtk/DecorationLayout\" = \":close\"\n"));
        assert_eq!((l.ui_family.as_deref(), l.buttons_right.as_slice()), (Some("Inter"), &[WinButton::Close][..]));
        assert!((l.ui_size - 11.0 * 96.0 / 72.0).abs() < 1e-4);
    }
}
