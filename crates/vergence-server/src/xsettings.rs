// Sharp X11 apps (server milestone 1): Xwayland runs at one client scale, the
// highest monitor scale, so X apps draw at the densest monitor's pixels and the
// server scales each X window down on the others (whitepaper §8: legacy windows
// scaled per monitor, input mapped). X apps learn the density from:
//
// - XSETTINGS (GTK): the user's ~/.config/vergence/settings.toml (vergd's file)
//   with the scale laid over it: Gdk/WindowScalingFactor = the whole part (GTK 3
//   scales windows by integers), Gdk/UnscaledDPI so text comes out at the exact
//   scale, Xft/DPI, Gtk/CursorThemeSize. Served by a small manager on its own
//   thread and X connection, as the phase 1 daemon does on Xorg.
// - X resources (Qt, Xft apps): Xft.dpi, Xcursor.size/theme, through xrdb.
// - Avalonia reads Xft.dpi too. (Its AVALONIA_*_SCALE_FACTOR(S) variables multiply
//   that, so the server clears them for the apps it starts.)
//
// Apps that ignore DPI (xclock, xterm's bitmap fonts) come out small; for them,
// VERGENCE_XWAYLAND_SCALE=1 (soft on dense monitors instead).
//
// When settings.toml changes (the settings app's Theme page), the server pokes
// the XSETTINGS thread, which reads it again and republishes (theme, icons,
// fonts, cursor) with the next serial; GTK X apps follow at once. The scale
// stays the one Xwayland started with.

use std::collections::BTreeMap;

use vergence_xsettings::{Settings, Value};
use x11rb::{
    COPY_DEPTH_FROM_PARENT,
    connection::Connection as _,
    protocol::{
        Event,
        xproto::{ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, WindowClass},
    },
    rust_connection::RustConnection,
};

/// The values laid over the user's settings for X apps drawing at `scale`.
pub fn overlay(scale: f64, cursor_points: u32) -> BTreeMap<String, Value> {
    let whole = (scale.floor() as i32).max(1);
    let dpi = 96.0 * 1024.0 * scale;
    let mut m = BTreeMap::new();
    m.insert("Gdk/WindowScalingFactor".into(), Value::Int(whole));
    m.insert("Gdk/UnscaledDPI".into(), Value::Int((dpi / whole as f64).round() as i32));
    m.insert("Xft/DPI".into(), Value::Int(dpi.round() as i32));
    m.insert("Gtk/CursorThemeSize".into(), Value::Int((cursor_points as f64 * scale).round() as i32));
    m
}

/// The user's XSETTINGS file (shared with vergd), or none.
fn user_settings() -> Settings {
    let Some(home) = std::env::var_os("HOME") else { return Settings::default() };
    let path = std::path::Path::new(&home).join(".config/vergence/settings.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => vergence_xsettings::from_toml(&text).unwrap_or_else(|e| {
            tracing::warn!("{}: {e}", path.display());
            Settings::default()
        }),
        Err(_) => Settings::default(),
    }
}

/// Tell X apps on `display` to draw at `scale`: X resources now (before any app
/// starts), XSETTINGS from a thread that keeps the selection while Xwayland runs.
/// A message down the returned channel republishes them from the files.
pub fn publish(display: u32, scale: f64, cursor_points: u32, cursor_theme: &str) -> std::sync::mpsc::Sender<()> {
    resources(display, scale, cursor_points, cursor_theme);
    let settings = settings_for(scale, cursor_points);
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        if let Err(e) = serve(display, settings, rx, scale, cursor_points) {
            tracing::warn!("XSETTINGS manager: {e}");
        }
    });
    tx
}

/// The user's XSETTINGS with the scale laid over them.
fn settings_for(scale: f64, cursor_points: u32) -> Settings {
    let mut settings = user_settings();
    for (name, value) in overlay(scale, cursor_points) {
        settings.set(&name, value);
    }
    settings
}

/// The X resources Qt, Xft and Avalonia read: Xft.dpi, the cursor.
fn resources(display: u32, scale: f64, cursor_points: u32, cursor_theme: &str) {
    let dpi = (96.0 * scale).round() as u32;
    let cursor = (cursor_points as f64 * scale).round() as u32;
    let resources = format!("Xft.dpi: {dpi}\nXcursor.size: {cursor}\nXcursor.theme: {cursor_theme}\n");
    match std::process::Command::new("xrdb")
        .args(["-merge", "-nocpp", "-"])
        .env("DISPLAY", format!(":{display}"))
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        Ok(mut child) => {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(resources.as_bytes());
            }
            let _ = child.wait();
        }
        Err(e) => tracing::warn!("xrdb: {e} (Qt and Xft apps will not know the scale)"),
    }
    tracing::info!("X apps: scale {scale:.4}, Xft.dpi {dpi}, cursor {cursor_theme}");
}

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// Own the XSETTINGS selection on `display` and publish `settings` until the
/// selection is taken or the X server goes away; a poke on `again` reads the
/// files again and republishes (next serial).
fn serve(
    display: u32,
    mut settings: Settings,
    again: std::sync::mpsc::Receiver<()>,
    scale: f64,
    cursor_points: u32,
) -> Res<()> {
    settings.serial = 1;
    let (conn, screen) = RustConnection::connect(Some(&format!(":{display}")))?;
    let root = conn.setup().roots[screen].root;
    let atom = |n: &str| -> Res<u32> { Ok(conn.intern_atom(false, n.as_bytes())?.reply()?.atom) };
    let selection = atom(&format!("_XSETTINGS_S{screen}"))?;
    let prop = atom("_XSETTINGS_SETTINGS")?;
    let manager = atom("MANAGER")?;

    let win = conn.generate_id()?;
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        win,
        root,
        -1,
        -1,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new().override_redirect(1).event_mask(EventMask::PROPERTY_CHANGE),
    )?;
    conn.change_property(PropMode::REPLACE, win, prop, prop, 8, settings.encode().len() as u32, &settings.encode())?;
    conn.flush()?;
    // A real timestamp for the selection, from that property change.
    let ts = loop {
        if let Event::PropertyNotify(e) = conn.wait_for_event()?
            && e.window == win
        {
            break e.time;
        }
    };
    conn.set_selection_owner(win, selection, ts)?;
    if conn.get_selection_owner(selection)?.reply()?.owner != win {
        return Err("could not take the XSETTINGS selection".into());
    }
    let ev = ClientMessageEvent::new(32, root, manager, [ts, selection, win, 0, 0]);
    conn.send_event(false, root, EventMask::STRUCTURE_NOTIFY, ev)?;
    conn.flush()?;
    loop {
        while let Some(event) = conn.poll_for_event()? {
            if let Event::SelectionClear(e) = event
                && e.selection == selection
            {
                tracing::info!("XSETTINGS selection taken by another manager");
                return Ok(());
            }
        }
        match again.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(()) => {
                let serial = settings.serial.wrapping_add(1);
                settings = settings_for(scale, cursor_points);
                settings.serial = serial;
                let theme = match settings.get("Gtk/CursorThemeName") {
                    Some(Value::Str(t)) if !t.is_empty() => t.clone(),
                    _ => "Adwaita".into(),
                };
                resources(display, scale, cursor_points, &theme);
                let data = settings.encode();
                conn.change_property(PropMode::REPLACE, win, prop, prop, 8, data.len() as u32, &data)?;
                conn.flush()?;
                tracing::info!("XSETTINGS: republished (serial {serial})");
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_for_the_14_inch() {
        // 394/120: GTK windows at 3x, text at exactly 3.2833 (315 dpi).
        let o = overlay(394.0 / 120.0, 24);
        assert_eq!(o["Gdk/WindowScalingFactor"], Value::Int(3));
        assert_eq!(o["Xft/DPI"], Value::Int(322765));
        assert_eq!(o["Gdk/UnscaledDPI"], Value::Int(107588));
        assert_eq!(o["Gtk/CursorThemeSize"], Value::Int(79));
    }

    #[test]
    fn overlay_at_scale_one() {
        let o = overlay(1.0, 24);
        assert_eq!(o["Gdk/WindowScalingFactor"], Value::Int(1));
        assert_eq!(o["Xft/DPI"], Value::Int(98304));
        assert_eq!(o["Gdk/UnscaledDPI"], Value::Int(98304));
    }
}
