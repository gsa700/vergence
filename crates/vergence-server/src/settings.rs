// The desktop settings service (whitepaper §6.8: settings, one source of truth,
// pushed to every subscriber; bus milestone 1 step 2c, 2026-11-08).
//
// Sources, both re-read within a second of a change:
//   ~/.config/vergence/desktop.toml  [appearance]
//       color-scheme   = "default" | "dark" | "light"
//       accent         = "#3b82f6"          (unset: no accent)
//       contrast       = "normal" | "high"
//       reduced-motion = false
//       frost          = "clear" | "standard" | "heavy"   (the server's own: org.vergence.look)
//       front-tab      = "page" | "frosted"
//       reduce-transparency = false
//   ~/.config/vergence/desktop.toml  [keyboard]   (the settings app's Keyboard page)
//   ~/.config/vergence/desktop.toml  [energy]     screens off and sleep after idle (energy.rs)
//       repeat       = true
//       repeat-delay = 500   (ms before a held key repeats)
//       repeat-rate  = 30    (repeats a second)
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
//   org.gnome.desktop.peripherals.keyboard   repeat b, delay u (ms),
//                                repeat-interval u (ms between repeats)
// Key repeat is also the server's own: every keyboard client gets it at once
// (wl_keyboard.repeat_info), so a change needs no restart.
// Subscribers: the core bus's settings topic (bus.rs) and the portal's
// SettingChanged signal (dbus.rs).

use std::{collections::BTreeMap, path::PathBuf, time::SystemTime};

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use vergence_protocol::bus::{Setting, SettingValue};

use crate::Server;

pub const APPEARANCE: &str = "org.freedesktop.appearance";
/// The desktop's own look (look milestone 1): frost u (0 clear, 1 standard,
/// 2 heavy), front-tab u (0 the page's color, 1 frosted),
/// reduce-transparency b. Only the server and our own tools read these.
pub const LOOK: &str = "org.vergence.look";
pub const INTERFACE: &str = "org.gnome.desktop.interface";
pub const KEYBOARD: &str = "org.gnome.desktop.peripherals.keyboard";
pub const WM: &str = "org.gnome.desktop.wm.preferences";
pub const MOUSE: &str = "org.gnome.desktop.peripherals.mouse";
pub const TOUCHPAD: &str = "org.gnome.desktop.peripherals.touchpad";
/// Key repeat without a [keyboard] table: GNOME's defaults (500 ms, then 30 a second).
pub const REPEAT_DELAY: u32 = 500;
pub const REPEAT_RATE: u32 = 30;

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
    let frost = match text("frost").as_deref() {
        Some("clear") => 0,
        Some("heavy") => 2,
        _ => 1,
    };
    put(LOOK, "frost", SettingValue::U32(frost));
    put(LOOK, "front-tab", SettingValue::U32((text("front-tab").as_deref() == Some("frosted")) as u32));
    put(LOOK, "reduce-transparency", SettingValue::Bool(appearance.get("reduce-transparency").and_then(|v| v.as_bool()).unwrap_or(false)));

    let keyboard = desktop
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("keyboard").and_then(|a| a.as_table()).cloned())
        .unwrap_or_default();
    let int = |k: &str, default: u32, lo: u32, hi: u32| keyboard.get(k).and_then(|v| v.as_integer()).map_or(default, |v| (v.max(0) as u32).clamp(lo, hi));
    let rate = int("repeat-rate", REPEAT_RATE, 1, 100);
    put(KEYBOARD, "repeat", SettingValue::Bool(keyboard.get("repeat").and_then(|v| v.as_bool()).unwrap_or(true)));
    put(KEYBOARD, "delay", SettingValue::U32(int("repeat-delay", REPEAT_DELAY, 100, 2000)));
    put(KEYBOARD, "repeat-interval", SettingValue::U32((1000 + rate / 2) / rate));

    // Mice and touchpads (pointer.rs), under GNOME's names.
    let table = |name: &str| desktop.and_then(|t| t.parse::<toml::Table>().ok()).and_then(|t| t.get(name).and_then(|a| a.as_table()).cloned()).unwrap_or_default();
    let (mouse, pad) = (table("mouse"), table("touchpad"));
    let speed = |t: &toml::Table| t.get("speed").and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64))).filter(|v| v.is_finite()).unwrap_or(0.0).clamp(-1.0, 1.0);
    let flag = |t: &toml::Table, k: &str, d: bool| t.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
    let lines = |t: &toml::Table| t.get("scroll-lines").and_then(|v| v.as_integer()).map_or(crate::pointer::SCROLL_LINES, |v| v.clamp(1, 10) as u32);
    put(MOUSE, "speed", SettingValue::F64(speed(&mouse)));
    put(MOUSE, "accel-profile", SettingValue::Str(if flag(&mouse, "acceleration", true) { "default" } else { "flat" }.into()));
    put(MOUSE, "natural-scroll", SettingValue::Bool(flag(&mouse, "natural-scroll", false)));
    put(MOUSE, "left-handed", SettingValue::Bool(flag(&mouse, "left-handed", false)));
    put(MOUSE, "middle-click-emulation", SettingValue::Bool(flag(&mouse, "middle-click", false)));
    put(MOUSE, "scroll-lines", SettingValue::U32(lines(&mouse)));
    put(MOUSE, "double-click", SettingValue::I32(mouse.get("double-click").and_then(|v| v.as_integer()).map_or(400, |v| v.clamp(100, 2000) as i32)));
    put(TOUCHPAD, "speed", SettingValue::F64(speed(&pad)));
    put(TOUCHPAD, "tap-to-click", SettingValue::Bool(flag(&pad, "tap-to-click", true)));
    put(TOUCHPAD, "tap-and-drag", SettingValue::Bool(flag(&pad, "tap-and-drag", true)));
    put(TOUCHPAD, "natural-scroll", SettingValue::Bool(flag(&pad, "natural-scroll", true)));
    let edge = pad.get("scroll").and_then(|v| v.as_str()) == Some("edge");
    put(TOUCHPAD, "two-finger-scrolling-enabled", SettingValue::Bool(!edge));
    put(TOUCHPAD, "edge-scrolling-enabled", SettingValue::Bool(edge));
    put(TOUCHPAD, "disable-while-typing", SettingValue::Bool(flag(&pad, "disable-while-typing", true)));
    put(TOUCHPAD, "click-method", SettingValue::Str(if pad.get("click").and_then(|v| v.as_str()) == Some("fingers") { "fingers" } else { "areas" }.into()));
    put(TOUCHPAD, "scroll-lines", SettingValue::U32(lines(&pad)));

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
    // Which window buttons, and on which side ("close,minimize:" or
    // ":minimize,maximize,close"): the server's own title bars follow it too.
    // Unset, the default (on the left) is published, so apps follow it as well.
    put(WM, "button-layout", SettingValue::Str(xstr("Gtk/DecorationLayout").unwrap_or_else(|| vergence_bar::DEFAULT_LAYOUT.to_string())));
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
    /// The theme, icons, fonts or pointer changed (the settings app's Theme
    /// page): the server's own drawing follows (pointer, title font), and X apps
    /// get the XSETTINGS again.
    pub fn theme_changed(&mut self, diff: &[Setting]) {
        let changed = |k: &str| diff.iter().any(|s| s.namespace == INTERFACE && s.key == k);
        if changed("cursor-theme") {
            self.pointer_image = crate::cursor::Pointer::load();
            let px = (self.pointer_image.size_points() as f64 * self.x_scale).round() as u32;
            if let Some(wm) = self.xwm.as_mut()
                && let Some((pixels, w, h, hx, hy)) = self.pointer_image.x_default(px)
            {
                let _ = wm.set_cursor(&pixels, (w, h).into(), (hx, hy).into());
            }
        }
        if changed("font-name") {
            self.title_font = crate::text::TextFont::load();
            // Title rasters are drawn again in the new font.
            self.decor.clear();
        }
        if diff.iter().any(|s| s.namespace == KEYBOARD) {
            self.apply_key_repeat();
        }
        if diff.iter().any(|s| s.namespace == MOUSE || s.namespace == TOUCHPAD) {
            self.apply_pointer_settings();
        }
        if diff.iter().any(|s| s.namespace == INTERFACE)
            && let Some(tx) = &self.xsettings_tx
        {
            let _ = tx.send(());
        }
        self.redraw_all();
    }

    /// Key repeat from the settings, to the keyboard and every client that has it.
    /// The key repeat the settings ask for: (repeats a second, delay in ms);
    /// a rate of 0 turns repeat off (wl_keyboard.repeat_info, and the native
    /// protocol's Repeat).
    pub fn key_repeat(&self) -> (i32, i32) {
        let get = |k: &str| self.settings.current.get(&(KEYBOARD.to_string(), k.to_string()));
        let on = !matches!(get("repeat"), Some(SettingValue::Bool(false)));
        let delay = match get("delay") { Some(SettingValue::U32(v)) => *v, _ => REPEAT_DELAY };
        let interval = match get("repeat-interval") { Some(SettingValue::U32(v)) if *v > 0 => *v, _ => 1000 / REPEAT_RATE };
        let rate = if on { (1000 / interval).max(1) } else { 0 };
        (rate as i32, delay as i32)
    }

    pub fn apply_key_repeat(&mut self) {
        let (rate, delay) = self.key_repeat();
        if let Some(k) = self.seat.get_keyboard() {
            k.change_repeat_info(rate, delay);
            tracing::info!("key repeat: {rate} a second after {delay} ms");
        }
        // Native windows' clients too (native.rs).
        self.native_repeat_changed();
    }

    /// Load the settings and re-read them whenever a source file changes.
    pub fn start_settings(&mut self) {
        self.settings.current = load();
        self.settings.stamps = stamps();
        self.apply_key_repeat();
        let res = self.loop_handle.insert_source(Timer::from_duration(std::time::Duration::from_secs(1)), |_, _, state| {
            let now = stamps();
            if now != state.settings.stamps {
                state.settings.stamps = now;
                // Screens' scale and arrangement (displays.rs), when they changed.
                let displays = crate::displays::load();
                if displays != state.displays {
                    state.displays = displays;
                    crate::drm::displays_changed(state);
                }
                // Idle times (energy.rs).
                state.energy_reload();
                // The shelf's settings and the primary screen (shelf.rs).
                state.shelf_reload();
                // X11 applications turned on (state.rs).
                state.x11_follow();
                let new = load();
                let diff = changed(&state.settings.current, &new);
                state.settings.current = new;
                if !diff.is_empty() {
                    tracing::info!("settings: {} changed", diff.iter().map(|s| format!("{} {}", s.namespace, s.key)).collect::<Vec<_>>().join(", "));
                    state.bus_settings_changed(&diff);
                    state.dbus_settings_changed(&diff);
                    state.theme_changed(&diff);
                    state.notify_restyle();
                }
            }
            // Applications installed or removed since the last look (shelf.rs).
            state.shelf_tick();
            state.session_tick();
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
    fn key_repeat() {
        let key = |m: &Map, k: &str| m.get(&(KEYBOARD.to_string(), k.to_string())).cloned();
        let m = build(None, None, 24);
        assert_eq!(key(&m, "delay"), Some(SettingValue::U32(500)));
        assert_eq!(key(&m, "repeat-interval"), Some(SettingValue::U32(33)));
        assert_eq!(key(&m, "repeat"), Some(SettingValue::Bool(true)));
        let m = build(Some("[keyboard]\nrepeat = false\nrepeat-delay = 650\nrepeat-rate = 20\n"), None, 24);
        assert_eq!(key(&m, "delay"), Some(SettingValue::U32(650)));
        assert_eq!(key(&m, "repeat-interval"), Some(SettingValue::U32(50)));
        assert_eq!(key(&m, "repeat"), Some(SettingValue::Bool(false)));
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

    #[test]
    fn mice_and_touchpads_under_gnome_s_names() {
        let get = |m: &Map, ns: &str, k: &str| m.get(&(ns.to_string(), k.to_string())).cloned();
        // Nothing written: libinput's own speed, the touchpad as on a laptop.
        let m = build(Some(""), None, 24);
        assert_eq!(get(&m, MOUSE, "speed"), Some(SettingValue::F64(0.0)));
        assert_eq!(get(&m, MOUSE, "natural-scroll"), Some(SettingValue::Bool(false)));
        assert_eq!(get(&m, TOUCHPAD, "natural-scroll"), Some(SettingValue::Bool(true)));
        assert_eq!(get(&m, TOUCHPAD, "tap-to-click"), Some(SettingValue::Bool(true)));
        assert_eq!(get(&m, MOUSE, "scroll-lines"), Some(SettingValue::U32(3)));
        assert_eq!(get(&m, MOUSE, "double-click"), Some(SettingValue::I32(400)));
        let desktop = "[mouse]\nspeed = 2.5\nacceleration = false\nleft-handed = true\nscroll-lines = 7\ndouble-click = 600\n[touchpad]\nscroll = \"edge\"\nclick = \"fingers\"\ntap-to-click = false\n";
        let m = build(Some(desktop), None, 24);
        assert_eq!(get(&m, MOUSE, "speed"), Some(SettingValue::F64(1.0)), "kept to -1..1");
        assert_eq!(get(&m, MOUSE, "accel-profile"), Some(SettingValue::Str("flat".into())));
        assert_eq!(get(&m, MOUSE, "left-handed"), Some(SettingValue::Bool(true)));
        assert_eq!(get(&m, MOUSE, "scroll-lines"), Some(SettingValue::U32(7)));
        assert_eq!(get(&m, MOUSE, "double-click"), Some(SettingValue::I32(600)));
        assert_eq!(get(&m, TOUCHPAD, "edge-scrolling-enabled"), Some(SettingValue::Bool(true)));
        assert_eq!(get(&m, TOUCHPAD, "two-finger-scrolling-enabled"), Some(SettingValue::Bool(false)));
        assert_eq!(get(&m, TOUCHPAD, "click-method"), Some(SettingValue::Str("fingers".into())));
        assert_eq!(get(&m, TOUCHPAD, "tap-to-click"), Some(SettingValue::Bool(false)));
    }
}
