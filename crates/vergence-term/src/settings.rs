//! The desktop's settings the terminal follows: fonts, button layout, light
//! or dark (settings.toml, GNOME's when under GNOME), and the terminal's own
//! switches in desktop.toml's `[terminal]` (cursor, scroll on output,
//! scrollback). The window itself is in main.rs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use vergence_ui::Waker;
use vergence_ui::bar::{self, WinButton};

/// What the desktop's settings say that the terminal follows
/// (~/.config/vergence/settings.toml, the file the settings app writes).
pub(crate) struct Settings {
    pub(crate) left: Vec<WinButton>,
    pub(crate) right: Vec<WinButton>,
    /// Light or dark, for the bar we draw ourselves (bar::desktop_light).
    pub(crate) light: bool,
    pub(crate) ui_family: String,
    pub(crate) ui_size: f32,
    pub(crate) mono_family: String,
    pub(crate) mono_size: f32,
}

/// "Adwaita Mono 10" -> the family and the size in reference units (a point is 96/72 of one).
pub(crate) fn font_name(value: &str, family: &str, points: f32) -> (String, f32) {
    let value = value.trim();
    match value.rsplit_once(' ') {
        Some((name, size)) if size.parse::<f32>().is_ok_and(|s| (4.0..=96.0).contains(&s)) => {
            (name.trim().to_string(), size.parse::<f32>().unwrap_or(points) * 96.0 / 72.0)
        }
        _ if !value.is_empty() => (value.to_string(), points * 96.0 / 72.0),
        _ => (family.to_string(), points * 96.0 / 72.0),
    }
}

fn config_path(name: &str) -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")));
    base.map(|b| b.join("vergence").join(name))
}

pub(crate) fn config(name: &str) -> String {
    config_path(name).and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default()
}

/// Sets `changed` and wakes the window whenever one of the settings files
/// is written (looked at once a second, by their modification times, as
/// the first terminal did; the session bus will tell us instead when the
/// toolkit speaks it).
pub(crate) fn watch_settings(changed: Arc<AtomicBool>, waker: Waker) {
    let stamp = |name: &str| config_path(name).and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
    // The files' times, and GNOME's button layout and light or dark when running there.
    let stamps = move || ([stamp("desktop.toml"), stamp("settings.toml")], bar::host_layout(), bar::host_light());
    std::thread::spawn(move || {
        let mut seen = stamps();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let now = stamps();
            if now != seen {
                seen = now;
                changed.store(true, Ordering::SeqCst);
                waker.wake();
            }
        }
    });
}

impl Settings {
    /// Vergence's settings file; the button layout GNOME's when running under
    /// GNOME, so the terminal fits in there (see docs/bar-milestone-1.md).
    pub(crate) fn load() -> Settings {
        let mut s = Settings::from_text(&config("settings.toml"));
        if let Some(layout) = bar::host_layout() {
            (s.left, s.right) = bar::decoration_layout(&layout);
        }
        s.light = bar::desktop_light();
        // The terminal's own font and size, if its settings chose them (app settings milestone 1).
        let desktop = config("desktop.toml");
        if let Some(f) = vergence_ui::table_value(&desktop, "[terminal]", "font").filter(|f| !f.trim().is_empty()) {
            s.mono_family = f;
        }
        if let Some(z) = vergence_ui::table_value(&desktop, "[terminal]", "font-size")
            .and_then(|z| z.parse::<f32>().ok())
            .filter(|z| (4.0..=96.0).contains(z))
        {
            s.mono_size = z;
        }
        s
    }

    pub(crate) fn from_text(text: &str) -> Settings {
        // Lines of the form "Gtk/FontName" = "Adwaita Sans 10".
        let value = |key: &str| {
            text.lines().find_map(|line| {
                let (k, v) = line.split_once('=')?;
                (k.trim().trim_matches('"') == key).then(|| v.trim().trim_matches('"').to_string())
            })
        };
        let (left, right) =
            bar::decoration_layout(&value("Gtk/DecorationLayout").unwrap_or_else(|| bar::DEFAULT_LAYOUT.into()));
        let (ui_family, ui_size) = font_name(&value("Gtk/FontName").unwrap_or_default(), "sans-serif", 11.0);
        let (mono_family, mono_size) =
            font_name(&value("Gtk/MonospaceFontName").unwrap_or_default(), "monospace", 12.0);
        Settings { left, right, light: false, ui_family, ui_size, mono_family, mono_size }
    }
}

/// The cursor's shape (the settings' `cursor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorShape {
    Block,
    Bar,
    Underline,
}

impl CursorShape {
    pub(crate) fn of(desktop: &str) -> CursorShape {
        match vergence_ui::table_value(desktop, "[terminal]", "cursor").as_deref() {
            Some("bar") => CursorShape::Bar,
            Some("underline") => CursorShape::Underline,
            _ => CursorShape::Block,
        }
    }
}

/// A switch of the terminal's settings (absent: `default`).
/// The settings' "Scroll on output" (see docs/apps-roadmap.md): while
/// scrolled back, new output brings the view down to it. Off by default:
/// the view stays on the text being read. Read by every tab's parse thread.
pub(crate) static SCROLL_ON_OUTPUT: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_scroll_on_output(desktop: &str) {
    SCROLL_ON_OUTPUT.store(term_flag(desktop, "scroll-on-output", false), Ordering::Relaxed);
}

pub(crate) fn term_flag(desktop: &str, key: &str, default: bool) -> bool {
    vergence_ui::table_value(desktop, "[terminal]", key).map_or(default, |v| v == "true")
}

/// Lines each tab keeps (the settings' `scrollback`, 1,000 to 1,000,000).
pub(crate) fn scrollback_of(desktop: &str) -> usize {
    vergence_ui::table_value(desktop, "[terminal]", "scrollback")
        .and_then(|v| v.parse::<usize>().ok())
        .map_or(10_000, |n| n.clamp(1_000, 1_000_000))
}
