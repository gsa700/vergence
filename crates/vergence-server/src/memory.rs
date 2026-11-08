// Window placement memory (whitepaper Contract 2; server milestone 1, 2026-11-07).
//
// The server remembers where each window was on each monitor setup and puts it
// back when the window opens again on that setup. The design is vergd's
// (docs/milestone-3.md) with the server's units:
//
// - A setup is the set of connected monitors' EDIDs (vergence_profile's
//   profile_key); a monitor is named by its EDID (monitor_id), not its port.
// - A window is its app's class (X11 WM_CLASS instance/class, Wayland app_id)
//   and its place among open windows of that class. No titles (Dave, 2026-11-06).
// - Positions are relative to the monitor's top-left, sizes are the content's,
//   all in points: they mean the same physical size on every monitor.
// - Recorded when a move or resize ends, on maximise/restore, and when the
//   window closes. A remembered place decides before the defaults and before the
//   app's own restore, when its monitor is there and the window fits it.
// - ~/.config/vergence/server-windows.toml, 0600 (whitepaper §7.3), the shared
//   WindowStore format; `skip = ["app"]` leaves an app alone. vergd's
//   windows.toml is separate: its units are X pixels.

use std::{collections::HashMap, os::unix::fs::PermissionsExt, path::PathBuf};

use smithay::{
    desktop::Window,
    output::Output,
    utils::{Logical, Rectangle},
};
use vergence_profile::windows::{WindowId, WindowSetting, WindowStore};

use crate::Server;

/// A monitor's EDID-based ID, kept on its Output (set by the backend).
pub struct MonitorId(pub String);

pub fn monitor_id_of(o: &Output) -> Option<String> {
    o.user_data().get::<MonitorId>().map(|m| m.0.clone())
}

pub struct Memory {
    store: WindowStore,
    /// The current monitor setup's key ("" until the backend knows it).
    pub setup: String,
    /// Each open window's identity (fixed when it opened: its class index).
    ids: HashMap<Window, WindowId>,
}

fn path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/vergence/server-windows.toml"))
}

const HEADER: &str = "# Vergence server window memory: where each window was on each monitor\n\
# setup, in points relative to its monitor. No window titles are stored. Apps\n\
# the server should not place: skip = [\"...\"] (instance or class, any case).\n\
# Written by vergence-server (crates/vergence-server/src/memory.rs).\n\n";

impl Memory {
    pub fn load() -> Memory {
        let store = path()
            .and_then(|p| std::fs::read_to_string(&p).ok())
            .and_then(|t| match WindowStore::from_toml(&t) {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!("server-windows.toml: {e} (starting empty; the file is kept until the next save)");
                    None
                }
            })
            .unwrap_or_default();
        Memory { store, setup: String::new(), ids: HashMap::new() }
    }

    fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = p.with_extension("toml.tmp");
        let body = format!("{HEADER}{}", self.store.to_toml());
        let res = std::fs::write(&tmp, body)
            .and_then(|_| std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)))
            .and_then(|_| std::fs::rename(&tmp, &p));
        if let Err(e) = res {
            tracing::warn!("{}: {e}", p.display());
        }
    }
}

impl Server {
    /// Who a window is: class and its place among open windows of that class.
    /// None for windows the memory leaves alone (menus, skipped apps, no class).
    pub fn window_identity(&mut self, w: &Window) -> Option<WindowId> {
        if let Some(id) = self.memory.ids.get(w) {
            return Some(id.clone());
        }
        let (instance, class) = if let Some(x) = w.x11_surface() {
            if x.is_override_redirect() || x.is_transient_for().is_some() {
                return None;
            }
            (x.instance(), x.class())
        } else {
            let t = w.toplevel()?;
            let app = smithay::wayland::compositor::with_states(t.wl_surface(), |s| {
                s.data_map
                    .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().unwrap().app_id.clone())
                    .unwrap_or_default()
            });
            (app.clone(), app)
        };
        if instance.is_empty() && class.is_empty() {
            return None;
        }
        self.memory.ids.retain(|w, _| smithay::utils::IsAlive::alive(w));
        let taken: Vec<u32> = self
            .memory
            .ids
            .values()
            .filter(|i| i.instance == instance && i.class == class)
            .map(|i| i.index)
            .collect();
        let index = (0..).find(|i| !taken.contains(i)).unwrap_or(0);
        let id = WindowId { instance, class, role: String::new(), index };
        if self.memory.store.skips(&id) {
            return None;
        }
        self.memory.ids.insert(w.clone(), id.clone());
        Some(id)
    }

    /// Where memory puts this window on the current setup: its content rectangle
    /// and whether it was maximised. None: no place, monitor absent, or no fit.
    pub fn remembered_place(&mut self, w: &Window) -> Option<(Rectangle<i32, Logical>, bool)> {
        let id = self.window_identity(w)?;
        let s = self.memory.store.get(&self.memory.setup)?.get(&id)?.clone();
        let out = self.space.outputs().find(|o| monitor_id_of(o).as_deref() == Some(s.monitor.as_str()))?.clone();
        let og = self.space.output_geometry(&out)?;
        let r = Rectangle::new((og.loc.x + s.x, og.loc.y + s.y).into(), (s.width as i32, s.height as i32).into());
        let maximised = s.max_horz && s.max_vert;
        if !maximised && !self.fits_a_monitor(r, self.has_title(w)) {
            return None;
        }
        Some((r, maximised))
    }

    /// Record where a window is now (its restore size and place if maximised).
    pub fn remember(&mut self, w: &Window) {
        if self.memory.setup.is_empty() {
            return;
        }
        let Some(id) = self.window_identity(w) else { return };
        let maximised = self.restore.get(w).copied();
        let Some(geo) = maximised.or_else(|| self.space.element_geometry(w)) else { return };
        let centre = (geo.loc.x + geo.size.w / 2, geo.loc.y + geo.size.h / 2);
        let Some((mon, og)) = self.space.outputs().find_map(|o| {
            let g = self.space.output_geometry(o)?;
            g.contains(centre).then(|| (monitor_id_of(o), g))
        }) else {
            return;
        };
        let Some(mon) = mon else { return };
        let setting = WindowSetting {
            id: id.clone(),
            monitor: mon,
            x: geo.loc.x - og.loc.x,
            y: geo.loc.y - og.loc.y,
            width: geo.size.w.max(1) as u32,
            height: geo.size.h.max(1) as u32,
            max_horz: maximised.is_some(),
            max_vert: maximised.is_some(),
            fullscreen: false,
            desktop: None,
        };
        let setup = self.memory.setup.clone();
        let profile = self.memory.store.get_mut_or_new(&setup);
        if profile.put(setting.clone()) {
            profile.updated = now();
            tracing::info!("memory: {} {}", id.describe(), setting.describe_place());
            self.memory.save();
        }
    }

    /// The setup changed (or became known): its key from the monitors' EDIDs.
    pub fn set_setup(&mut self, edids: &[Vec<u8>]) {
        let key = vergence_profile::profile_key(edids);
        if key != self.memory.setup {
            tracing::info!("memory: monitor setup {}", &key[..12]);
            self.memory.setup = key;
        }
    }
}

fn now() -> String {
    std::process::Command::new("date")
        .arg("+%Y-%m-%d %H:%M:%S")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}
