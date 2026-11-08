// The desktop settings service (whitepaper §6.8: settings, one source of truth,
// pushed to every subscriber; bus milestone 1 step 2c, 2026-11-08).
//
// Sources, both re-read within a second of a change:
//   ~/.config/vergence/desktop.toml  [appearance]
//       color-scheme   = "default" | "dark" | "light"
//       accent         = "#3b82f6"          (unset: no accent)
//       contrast       = "normal" | "high"
//       reduced-motion = false
//   ~/.config/vergence/settings.toml  the XSETTINGS file (theme, icons, cursor,
//       fonts), shared with vergd.
//
// Published under the namespaces toolkits already ask for, so the D-Bus edge's
// settings portal passes them on unchanged:
//   org.freedesktop.appearance   color-scheme u, accent-color (ddd), contrast u,
//                                reduced-motion u
//   org.gnome.desktop.interface  color-scheme s, gtk-theme s, icon-theme s,
//                                cursor-theme s, cursor-size i, font-name s,
//                                monospace-font-name s, cursor-blink b,
//                                cursor-blink-time i, enable-animations b
// Subscribers: the core bus's settings topic (bus.rs) and the portal's
// SettingChanged signal (dbus.rs).

use std::{collections::BTreeMap, path::PathBuf, time::SystemTime};

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use vergence_protocol::bus::{Setting, SettingValue};

use crate::Server;

pub const APPEARANCE: &str = "org.freedesktop.appearance";
pub const INTERFACE: &str = "org.gnome.desktop.interface";

pub type Map = BTreeMap<(String, String), SettingValue>;

#[derive(Default)]
pub struct SettingsState {
    pub current: Map,
    stamps: Vec<Option<SystemTime>>,
}

fn config(name: &str) -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/vergence").join(name))
}

fn stamps() -> Vec<Option<SystemTime>> {
    ["desktop.toml", "settings.toml"]
        .iter()
        .map(|n| config(n).and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok()))
        .collect()
}

/// "#rrggbb" -> channels 0..1.
pub fn parse_rgb(s: &str) -> Option<(f64, f64, f64)> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f64 / 255.0);
    Some((c(0)?, c(2)?, c(4)?))
}

/// The settings from the two files' text (either may be missing).
pub fn build(desktop: Option<&str>, xsettings: Option<&str>, cursor_points: u32) -> Map {
    let mut m = Map::new();
    let mut put = |ns: &str, key: &str, v: SettingValue| {
        m.insert((ns.to_string(), key.to_string()), v);
    };

    let appearance = desktop
        .and_then(|t| t.parse::<toml::Table>().map_err(|e| tracing::warn!("desktop.toml: {e}")).ok())
        .and_then(|t| t.get("appearance").and_then(|a| a.as_table()).cloned())
        .unwrap_or_default();
    let text = |k: &str| appearance.get(k).and_then(|v| v.as_str()).map(str::to_owned);
    let scheme = text("color-scheme").unwrap_or_default();
    let (scheme_u, scheme_gnome) = match scheme.as_str() {
        "dark" => (1, "prefer-dark"),
        "light" => (2, "prefer-light"),
        _ => (0, "default"),
    };
    put(APPEARANCE, "color-scheme", SettingValue::U32(scheme_u));
    put(INTERFACE, "color-scheme", SettingValue::Str(scheme_gnome.into()));
    if let Some((r, g, b)) = text("accent").as_deref().and_then(parse_rgb) {
        put(APPEARANCE, "accent-color", SettingValue::Rgb(r, g, b));
    }
    let high = text("contrast").as_deref() == Some("high");
    put(APPEARANCE, "contrast", SettingValue::U32(high as u32));
    let reduced = appearance.get("reduced-motion").and_then(|v| v.as_bool()).unwrap_or(false);
    put(APPEARANCE, "reduced-motion", SettingValue::U32(reduced as u32));
    put(INTERFACE, "enable-animations", SettingValue::Bool(!reduced));

    let xs = xsettings.and_then(|t| vergence_xsettings::from_toml(t).map_err(|e| tracing::warn!("settings.toml: {e}")).ok()).unwrap_or_default();
    let xstr = |k: &str| match xs.get(k) {
        Some(vergence_xsettings::Value::Str(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    };
    let xint = |k: &str| match xs.get(k) {
        Some(vergence_xsettings::Value::Int(i)) => Some(*i),
        _ => None,
    };
    for (key, x) in [
        ("gtk-theme", "Net/ThemeName"),
        ("icon-theme", "Net/IconThemeName"),
        ("cursor-theme", "Gtk/CursorThemeName"),
        ("font-name", "Gtk/FontName"),
        ("monospace-font-name", "Gtk/MonospaceFontName"),
    ] {
        if let Some(v) = xstr(x) {
            put(INTERFACE, key, SettingValue::Str(v));
        }
    }
    // Cursor size in points (the same on every monitor): the server's, not
    // XSETTINGS' pixel value, which is scaled for X apps.
    put(INTERFACE, "cursor-size", SettingValue::I32(cursor_points as i32));
    if let Some(b) = xint("Net/CursorBlink") {
        put(INTERFACE, "cursor-blink", SettingValue::Bool(b != 0));
    }
    if let Some(t) = xint("Net/CursorBlinkTime") {
        put(INTERFACE, "cursor-blink-time", SettingValue::I32(t));
    }
    m
}

fn load() -> Map {
    let read = |n: &str| config(n).and_then(|p| std::fs::read_to_string(p).ok());
    let points = std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(24);
    build(read("desktop.toml").as_deref(), read("settings.toml").as_deref(), points)
}

/// Settings in `new` that are not the same in `old`.
pub fn changed(old: &Map, new: &Map) -> Vec<Setting> {
    new.iter()
        .filter(|(k, v)| old.get(*k) != Some(*v))
        .map(|((ns, key), v)| Setting { namespace: ns.clone(), key: key.clone(), value: v.clone() })
        .collect()
}

pub fn all(m: &Map) -> Vec<Setting> {
    changed(&Map::new(), m)
}

impl Server {
    /// Load the settings and re-read them whenever a source file changes.
    pub fn start_settings(&mut self) {
        self.settings.current = load();
        self.settings.stamps = stamps();
        let res = self.loop_handle.insert_source(Timer::from_duration(std::time::Duration::from_secs(1)), |_, _, state| {
            let now = stamps();
            if now != state.settings.stamps {
                state.settings.stamps = now;
                let new = load();
                let diff = changed(&state.settings.current, &new);
                state.settings.current = new;
                if !diff.is_empty() {
                    tracing::info!("settings: {} changed", diff.iter().map(|s| format!("{} {}", s.namespace, s.key)).collect::<Vec<_>>().join(", "));
                    state.bus_settings_changed(&diff);
                    state.dbus_settings_changed(&diff);
                }
            }
            TimeoutAction::ToDuration(std::time::Duration::from_secs(1))
        });
        if let Err(e) = res {
            tracing::warn!("settings: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_and_xsettings_map_to_toolkit_namespaces() {
        let desktop = "[wallpaper]\nmode = \"fill\"\n[appearance]\ncolor-scheme = \"dark\"\naccent = \"#3366ff\"\ncontrast = \"high\"\nreduced-motion = true\n";
        let xs = "[settings]\n\"Net/ThemeName\" = \"Adwaita\"\n\"Gtk/FontName\" = \"Sans 10\"\n\"Net/CursorBlinkTime\" = 1200\n\"Net/IconThemeName\" = \"\"\n";
        let m = build(Some(desktop), Some(xs), 24);
        let get = |ns: &str, k: &str| m.get(&(ns.to_string(), k.to_string())).cloned();
        assert_eq!(get(APPEARANCE, "color-scheme"), Some(SettingValue::U32(1)));
        assert_eq!(get(INTERFACE, "color-scheme"), Some(SettingValue::Str("prefer-dark".into())));
        assert_eq!(get(APPEARANCE, "accent-color"), Some(SettingValue::Rgb(0.2, 0.4, 1.0)));
        assert_eq!(get(APPEARANCE, "contrast"), Some(SettingValue::U32(1)));
        assert_eq!(get(APPEARANCE, "reduced-motion"), Some(SettingValue::U32(1)));
        assert_eq!(get(INTERFACE, "enable-animations"), Some(SettingValue::Bool(false)));
        assert_eq!(get(INTERFACE, "gtk-theme"), Some(SettingValue::Str("Adwaita".into())));
        assert_eq!(get(INTERFACE, "font-name"), Some(SettingValue::Str("Sans 10".into())));
        assert_eq!(get(INTERFACE, "cursor-blink-time"), Some(SettingValue::I32(1200)));
        assert_eq!(get(INTERFACE, "cursor-size"), Some(SettingValue::I32(24)));
        assert_eq!(get(INTERFACE, "icon-theme"), None); // empty is unset
    }

    #[test]
    fn defaults_without_files() {
        let m = build(None, None, 24);
        assert_eq!(m.get(&(APPEARANCE.into(), "color-scheme".into())), Some(&SettingValue::U32(0)));
        assert!(!m.contains_key(&(APPEARANCE.into(), "accent-color".into())));
    }

    #[test]
    fn only_changes_are_reported() {
        let a = build(Some("[appearance]\ncolor-scheme = \"light\"\n"), None, 24);
        let b = build(Some("[appearance]\ncolor-scheme = \"dark\"\n"), None, 24);
        let keys: Vec<String> = changed(&a, &b).into_iter().map(|s| s.key).collect();
        assert_eq!(keys, ["color-scheme", "color-scheme"]);
        assert!(changed(&b, &b).is_empty());
        assert_eq!(parse_rgb("#ff0080"), Some((1.0, 0.0, 128.0 / 255.0)));
        assert_eq!(parse_rgb("red"), None);
    }
}
