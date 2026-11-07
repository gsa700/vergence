//! Mode A and Mode B scaling (milestone 4, docs/milestone-4.md).
//!
//! **Mode B** (native pixels): no RandR transforms. Qt and Avalonia get exact
//! per-monitor factors (`QT_SCREEN_SCALE_FACTORS`,
//! `AVALONIA_SCREEN_SCALE_FACTORS` in session.env, read when they start,
//! so from the next login); `Xft.dpi` stays 96 so they don't multiply it in.
//! GTK, Electron and Java take one whole-number scale: that of the primary
//! monitor, or else the physically largest one.
//!
//! With scaling on, every lit monitor gets its derived scale (or the
//! profile's override); everything is rendered at the whole-number scale
//! `render` and RandR shrinks each monitor's picture onto its panel. The
//! toolkits all get `render`: GTK through XSETTINGS (live), Qt, Electron and
//! Avalonia and Xlib fonts through `Xft.dpi` (new apps), Java through
//! `session.env` (next login). With scaling off, everything goes back to 1.
//! Avalonia must not also get `AVALONIA_GLOBAL_SCALE_FACTOR`: it multiplies
//! with the `Xft.dpi` scale (measured 2026-11-06: drawn at 4x).

use std::collections::BTreeMap;

use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
use x11rb::rust_connection::RustConnection;

use vergence_profile::arrangement::{repack, same_arrangement, OutputSetting, ScaleMode, ScalingConfig, FIXED_ONE};
use vergence_profile::scaling::{factor_fixed, render_scale};
use vergence_xsettings::Value;

use crate::x11::Setup;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// The scale one monitor should have, and why.
pub struct MonitorScale {
    pub connector: String,
    pub name: String,
    /// Rounded to 0.25 steps: decides whether the setup needs scaling.
    pub scale: f64,
    /// The exact scale: what Mode A uses (decision 7, Dave 2026-11-06).
    pub exact: f64,
    pub why: String,
    /// Physical area in mm² (X's size for the output), and whether it is the
    /// primary: Mode B's global scale comes from one of these.
    pub area: u64,
    pub primary: bool,
}

/// What the arrangement should be under `cfg`.
pub struct Target {
    pub mode: ScaleMode,
    /// Mode A: everything is drawn at this. Mode B: the one whole-number
    /// scale GTK, Electron and Java get.
    pub render: u32,
    /// Mode B: (connector, exact scale) for Qt and Avalonia.
    pub per_monitor: Vec<(String, f64)>,
    pub monitors: Vec<MonitorScale>,
    /// The arrangement to apply; equal to the current one if nothing changes.
    pub want: Vec<OutputSetting>,
    pub changes: bool,
}

/// Each lit monitor's scale under `cfg` (all 1 when scaling is off).
pub fn monitor_scales(setup: &Setup, cfg: &ScalingConfig) -> Vec<MonitorScale> {
    setup
        .outputs
        .iter()
        .filter(|o| o.connected && o.current.is_some())
        .map(|o| {
            let id = o.monitor();
            let name = match o.monitor_name() {
                n if n.is_empty() => o.name.clone(),
                n => n,
            };
            let m = cfg.monitor(&id);
            let distance = m.map(|m| m.distance).unwrap_or_default();
            let (scale, exact, why) = if !cfg.enabled {
                (1.0, 1.0, "scaling off".to_string())
            } else if let Some(p) = m.and_then(|m| m.scale) {
                let v = p as f64 / 100.0;
                (v, v, "set in profiles.toml".to_string())
            } else {
                match o.derived_scale(distance, cfg.preference as f64 / 100.0) {
                    Some(d) => match d.ppi {
                        Some(ppi) => (
                            d.scale,
                            d.exact,
                            format!("exactly {:.2}; {ppi:.0} ppi, {distance:?} distance", d.exact).to_lowercase(),
                        ),
                        None => (1.0, 1.0, d.note.unwrap_or("no size").to_string()),
                    },
                    None => (1.0, 1.0, "no native mode".to_string()),
                }
            };
            MonitorScale {
                connector: o.name.clone(),
                name,
                scale,
                exact,
                why,
                area: o.mm.0 as u64 * o.mm.1 as u64,
                primary: o.id == setup.primary,
            }
        })
        .collect()
}

/// Mode A uses each monitor's **exact** scale, so a window is the same
/// physical size on every monitor (decision 7). But a setup scales only if
/// some monitor needs it (its rounded scale is above 1): with no
/// high-density monitor everything stays 1:1, rather than drawing the whole
/// desktop at 2x to correct a few percent.
pub fn target(setup: &Setup, cfg: &ScalingConfig) -> Result<Target, String> {
    let mut monitors = monitor_scales(setup, cfg);
    if monitors.iter().all(|m| m.scale <= 1.0) {
        for m in &mut monitors {
            if m.exact != 1.0 {
                m.why = format!("{}; no monitor here needs scaling", m.why);
            }
            m.exact = 1.0;
        }
    }
    let mode = if cfg.enabled { cfg.mode } else { ScaleMode::A };
    let (render, per_monitor) = match mode {
        ScaleMode::A => (render_scale(&monitors.iter().map(|m| m.exact).collect::<Vec<_>>()), Vec::new()),
        ScaleMode::B => {
            let main = monitors
                .iter()
                .find(|m| m.primary)
                .or_else(|| monitors.iter().max_by_key(|m| m.area));
            let global = main.map(|m| m.exact.round().max(1.0) as u32).unwrap_or(1);
            let per: Vec<(String, f64)> = if monitors.iter().all(|m| m.exact == 1.0) {
                Vec::new()
            } else {
                monitors.iter().map(|m| (m.connector.clone(), m.exact)).collect()
            };
            (global, per)
        }
    };
    let current = setup.arrangement();
    let factors: Vec<u32> = current
        .iter()
        .map(|o| match monitors.iter().find(|m| m.connector == o.connector) {
            Some(m) if o.enabled && mode == ScaleMode::A => factor_fixed(render, m.exact),
            _ => FIXED_ONE,
        })
        .collect();
    let same = current.iter().zip(&factors).all(|(o, f)| o.factor == *f);
    let want = if same { current.clone() } else { repack(&current, &factors)? };
    let changes = !same_arrangement(&current, &want);
    Ok(Target { mode, render, per_monitor, monitors, want, changes })
}

/// Everything the toolkits are given, as one string: when it changes, the
/// settings are written again.
pub fn toolkit_key(t: &Target) -> String {
    format!("{:?} {} {:?}", t.mode, t.render, t.per_monitor)
}

/// `QT_SCREEN_SCALE_FACTORS` / `AVALONIA_SCREEN_SCALE_FACTORS` syntax:
/// "HDMI-1=1.70;HDMI-2=1.07".
pub fn per_monitor_text(per: &[(String, f64)]) -> String {
    per.iter().map(|(c, s)| format!("{c}={s:.2}")).collect::<Vec<_>>().join(";")
}

/// The XSETTINGS laid over the user's for `render` (none at 1). GTK takes
/// the window scale; `Gdk/UnscaledDPI` keeps its fonts from being scaled
/// twice (as GNOME's settings daemon does).
pub fn xsettings_overlay(render: u32, base_cursor: i32) -> BTreeMap<String, Value> {
    let mut m = BTreeMap::new();
    if render > 1 {
        let r = render as i32;
        m.insert("Gdk/WindowScalingFactor".into(), Value::Int(r));
        m.insert("Gdk/UnscaledDPI".into(), Value::Int(96 * 1024));
        m.insert("Xft/DPI".into(), Value::Int(96 * 1024 * r));
        m.insert("Gtk/CursorThemeSize".into(), Value::Int(cursor(base_cursor) * r));
    }
    m
}

fn cursor(base: i32) -> i32 {
    if base > 0 { base } else { 24 }
}

/// Set `Xft.dpi` (96 x `dpi`) and `Xcursor.size` (x `cursor`) in the root
/// window's X resources (read by Qt, Avalonia and Xlib programs when they
/// start), keeping every other line.
pub fn set_resources(conn: &RustConnection, root: u32, dpi: u32, cursor: u32, base_cursor: i32) -> Res<()> {
    let rm = conn.intern_atom(false, b"RESOURCE_MANAGER")?.reply()?.atom;
    let p = conn.get_property(false, root, rm, AtomEnum::STRING, 0, 1 << 20)?.reply()?;
    let old = String::from_utf8_lossy(&p.value).into_owned();
    let new = resources_text(&old, dpi, cursor, base_cursor);
    if new != old {
        conn.change_property(PropMode::REPLACE, root, rm, AtomEnum::STRING, 8, new.len() as u32, new.as_bytes())?;
        conn.flush()?;
    }
    Ok(())
}

fn resources_text(old: &str, dpi_scale: u32, cursor_scale: u32, base_cursor: i32) -> String {
    let mut lines: Vec<&str> = old
        .lines()
        .filter(|l| !l.starts_with("Xft.dpi:") && !l.starts_with("Xcursor.size:"))
        .collect();
    let dpi = format!("Xft.dpi:\t{}", 96 * dpi_scale);
    let size = if cursor_scale > 1 || base_cursor > 0 {
        Some(format!("Xcursor.size:\t{}", cursor(base_cursor) * cursor_scale as i32))
    } else {
        None
    };
    lines.push(&dpi);
    if let Some(s) = &size {
        lines.push(s);
    }
    let mut t = lines.join("\n");
    t.push('\n');
    t
}

/// The launch-time variables for toolkits that read their scale only when
/// they start (decision 5), for `~/.config/vergence/session.env`.
pub fn session_env(render: u32, per_monitor: &[(String, f64)]) -> String {
    let mut t = String::from(
        "# Written by vergd (milestone 4): the scale for toolkits that read it\n\
         # only when they start. Sourced at login from ~/.xprofile.\n",
    );
    // Shows in every program's environment that the login read this file.
    t.push_str(&format!("export VERGENCE_SCALE={render}\n"));
    if render > 1 {
        t.push_str(&format!("export _JAVA_OPTIONS=\"-Dsun.java2d.uiScale={render}\"\n"));
    } else {
        t.push_str("unset _JAVA_OPTIONS AVALONIA_GLOBAL_SCALE_FACTOR\n");
    }
    if per_monitor.is_empty() {
        t.push_str("unset QT_SCREEN_SCALE_FACTORS AVALONIA_SCREEN_SCALE_FACTORS\n");
    } else {
        let f = per_monitor_text(per_monitor);
        t.push_str(&format!("export QT_SCREEN_SCALE_FACTORS=\"{f}\"\n"));
        t.push_str(&format!("export AVALONIA_SCREEN_SCALE_FACTORS=\"{f}\"\n"));
    }
    t
}

/// The line that makes the login read session.env.
pub const XPROFILE_LINE: &str = "[ -r \"$HOME/.config/vergence/session.env\" ] && . \"$HOME/.config/vergence/session.env\"  # vergence";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_only_above_one() {
        assert!(xsettings_overlay(1, 0).is_empty());
        let o = xsettings_overlay(2, 0);
        assert_eq!(o["Gdk/WindowScalingFactor"], Value::Int(2));
        assert_eq!(o["Xft/DPI"], Value::Int(196608));
        assert_eq!(o["Gdk/UnscaledDPI"], Value::Int(98304));
        assert_eq!(o["Gtk/CursorThemeSize"], Value::Int(48));
        assert_eq!(xsettings_overlay(2, 32)["Gtk/CursorThemeSize"], Value::Int(64));
    }

    #[test]
    fn resources_keep_other_lines() {
        let old = "Xft.dpi:\t96\nXcursor.theme:\t\nXcursor.size:\t0\nXcursor.theme_core:\t1\n";
        let two = resources_text(old, 2, 2, 0);
        assert!(two.contains("Xft.dpi:\t192\n") && two.contains("Xcursor.size:\t48\n"), "{two}");
        assert!(two.contains("Xcursor.theme_core:\t1"));
        assert_eq!(two.matches("Xft.dpi").count(), 1);
        let one = resources_text(&two, 1, 1, 0);
        // Mode B: DPI stays 96, cursor follows the global scale.
        let b = resources_text(old, 1, 2, 0);
        assert!(b.contains("Xft.dpi:\t96\n") && b.contains("Xcursor.size:\t48\n"), "{b}");
        assert!(one.contains("Xft.dpi:\t96\n") && !one.contains("Xcursor.size"), "{one}");
    }

    #[test]
    fn session_env_text() {
        assert!(session_env(2, &[]).contains("uiScale=2"));
        let b = session_env(2, &[("HDMI-1".into(), 1.7), ("HDMI-2".into(), 1.0678)]);
        assert!(b.contains("QT_SCREEN_SCALE_FACTORS=\"HDMI-1=1.70;HDMI-2=1.07\""), "{b}");
        assert!(b.contains("AVALONIA_SCREEN_SCALE_FACTORS=\"HDMI-1=1.70;HDMI-2=1.07\""));
        // Avalonia reads Xft.dpi itself; its variable would scale it twice.
        assert!(!session_env(2, &[]).contains("export AVALONIA"));
        assert!(session_env(1, &[]).contains("unset"));
    }
}
