// Window placement memory (whitepaper Contract 2; server milestone 1, 2026-11-07).
//
// The server remembers where each window was on each monitor setup and puts it
// back when the window opens again on that setup. The design is vergd's
// (docs/milestone-3.md) with the server's units:
//
// - A setup is the set of connected monitors' EDIDs (vergence_profile's
//   profile_key); a monitor is named by its EDID (monitor_id), not its port.
//   Places are kept per size of the setup: its key and each monitor's size in
//   points (windows::sized_key), so the same monitors at another resolution
//   or scale keep places of their own, and going back finds the old ones
//   (Dave, 2026-11-12). A size seen for the first time starts from the most
//   recent other size of the same monitors, each place scaled in proportion
//   to its monitor; a move there is kept for that size only.
// - A window is its app's class (X11 WM_CLASS instance/class, Wayland app_id)
//   and its place among open windows of that class. No titles (Dave, 2026-11-06).
//   A native window is its persistence token instead (bar milestone 1, step 5;
//   docs/bar-milestone-1.md, "Windows that stay where they were"): issued by the
//   server, presented again by the program, so each window keeps its own place
//   whatever else opened or closed. A new window (no memory for its token) starts
//   where the last window of its program the user closed was; closing one for
//   good forgets its token everywhere; tokens unseen for 90 days are pruned.
// - Positions are relative to the monitor's top-left, sizes are the content's,
//   all in points: they mean the same physical size on every monitor.
// - Recorded when a move or resize ends, on maximize/restore, and when the
//   window closes. A remembered place decides before the defaults and before the
//   app's own restore, when its monitor is there and the window's title bar is
//   wholly on it (Dave, 2026-11-12; the whole window had to fit until then).
// - ~/.config/vergence/server-windows.toml, 0600 (whitepaper §7.3), the shared
//   WindowStore format; `skip = ["app"]` leaves an app alone. vergd's
//   windows.toml is separate: its units are X pixels.

use crate::win::Win;
use std::{collections::HashMap, os::unix::fs::PermissionsExt, path::PathBuf};

use smithay::{
    output::Output,
    utils::{Logical, Point, Rectangle},
};
use vergence_profile::windows::{WindowId, WindowSetting, WindowStore};

use crate::Server;

/// The smallest window memory records or restores, in points. A foot window
/// was put back at 1x1 at every restart (it had been anchored before its first
/// commit, squeezed to one cell, 7x15, and remembered so, 2026-11-08).
const MIN_W: i32 = 64;
const MIN_H: i32 = 48;

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
    ids: HashMap<Win, WindowId>,
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

    fn save(&mut self) {
        // Windows with a token that have not come back for 90 days are forgotten.
        let pruned = self.store.prune_unseen(today().saturating_sub(90));
        if pruned > 0 {
            tracing::info!("memory: forgot {pruned} window places unseen for 90 days");
        }
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
    pub fn window_identity(&mut self, w: &Win) -> Option<WindowId> {
        // Dialogs, palettes and splash screens are placed by their parent
        // (dialogs.rs), never from memory: a settings dialog took the size of
        // the settings window last closed, its buttons off the screen (kepler,
        // 2026-11-14).
        // A window with a parent counts too: its parent is known when it is
        // created, its kind only once its style arrives, which can be after
        // the server first places it.
        use crate::style::Kind;
        let style = self.window_style(w);
        if matches!(style.kind, Kind::Dialog | Kind::Palette | Kind::Splash) {
            return None;
        }
        // A native window with a parent may be one of those until its style
        // arrives (the toolkit always sends one for such a window); a normal
        // window another opened (a terminal's torn-off tab) is remembered then.
        if style.parent.is_some() && w.native().is_none_or(|n| n.style().1.is_none()) {
            return None;
        }
        if let Some(id) = self.memory.ids.get(w) {
            return Some(id.clone());
        }
        let mut role = String::new();
        let (instance, class) = if let Some(n) = w.native() {
            // A native window says who it is (Contract 2): its application id and
            // role, and, if its program keeps tokens, its token (no numbering then).
            if n.keeps_token {
                let id = WindowId { instance: n.app_id.clone(), class: n.app_id.clone(), role: n.role.clone(), index: 0, token: n.token.clone() };
                if id.instance.is_empty() || self.memory.store.skips(&id) {
                    return None;
                }
                self.memory.ids.insert(w.clone(), id.clone());
                return Some(id);
            }
            role = n.role.clone();
            (n.app_id.clone(), n.app_id.clone())
        } else if let Some(x) = w.x11_surface() {
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
            // No app_id (iced and Slint set none on FreeBSD): the program's name,
            // from the client's process (2026-11-08: the terminal and the settings
            // app had no memory).
            let app = if app.is_empty() { program_of(&self.display_handle, t.wl_surface()).unwrap_or_default() } else { app };
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
            .filter(|i| i.instance == instance && i.class == class && i.role == role)
            .map(|i| i.index)
            .collect();
        let index = (0..).find(|i| !taken.contains(i)).unwrap_or(0);
        let id = WindowId { instance, class, role, index, token: String::new() };
        if self.memory.store.skips(&id) {
            return None;
        }
        self.memory.ids.insert(w.clone(), id.clone());
        Some(id)
    }

    /// `r` for a new window `w`, moved off any other window of its program
    /// whose corner is exactly there: down and right by a bar's height until
    /// the corner is free; where that runs off monitor `og` (a window in the
    /// bottom-right corner), up and left; else `r` itself.
    pub fn clear_of_siblings(&self, w: &Win, mut r: Rectangle<i32, Logical>, og: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        // Its program: a native window's application id, else the class memory
        // knows it by (Wayland app_id, X11 WM_CLASS).
        let class = |x: &Win| x.native().map(|n| n.app_id.clone()).or_else(|| self.memory.ids.get(x).map(|i| i.class.clone()));
        let mine = class(w);
        let taken: Vec<Point<i32, Logical>> = self
            .space
            .elements()
            .filter(|o| *o != w && mine.is_some() && class(o) == mine)
            .filter_map(|o| self.space.element_location(o))
            .collect();
        let start = r.loc;
        let on = |p: Point<i32, Logical>| p.x >= og.loc.x && p.y >= og.loc.y && p.x + r.size.w <= og.loc.x + og.size.w && p.y + r.size.h <= og.loc.y + og.size.h;
        let free = |dir: i32| {
            let mut p = start;
            for _ in 0..32 {
                if !taken.contains(&p) {
                    return on(p).then_some(p);
                }
                p += Point::from((dir * crate::decor::TITLE_H, dir * crate::decor::TITLE_H));
            }
            None
        };
        r.loc = free(1).or_else(|| free(-1)).unwrap_or(start);
        r
    }

    /// Where memory puts this window on the current setup: its content rectangle
    /// and whether it was maximized. None: no place, monitor absent, no fit, or a
    /// size no real window has (see MIN_W).
    pub fn remembered_place(&mut self, w: &Win) -> Option<(Rectangle<i32, Logical>, bool)> {
        let id = self.window_identity(w)?;
        // A native window (a role) with no memory yet takes its program's
        // role-less one: where the same program's Wayland window was (our tools
        // before the native road, 2026-11-11), so moving roads forgets nothing.
        // A window with a token and no memory yet starts where the last window
        // of its program the user closed was (a new terminal window, a torn-off
        // tab's own window asks for its drop point and has no memory: that wins).
        let find = |profile: &vergence_profile::windows::WindowProfile| match profile.get(&id) {
            Some(s) => Some((s.clone(), false)),
            None if !id.token.is_empty() => profile.get(&id.untokened()).or_else(|| profile.get(&WindowId { role: String::new(), ..id.untokened() })).map(|s| (s.clone(), true)),
            None if !id.role.is_empty() => profile.get(&WindowId { role: String::new(), ..id.clone() }).map(|s| (s.clone(), false)),
            None => None,
        };
        // This size of the setup first; else the most recent other size of the
        // same monitors, scaled to this one (a key from before sizes: as it is).
        let key = self.sized_setup();
        let (s, fallback) = match self.memory.store.get(&key).and_then(|p| find(p)) {
            Some(found) => found,
            None => {
                let now = vergence_profile::windows::key_sizes(&key).1;
                let size_in = |sizes: &[(String, u32, u32)], m: &str| sizes.iter().find(|(id, ..)| id == m).map(|(_, w, h)| (*w, *h));
                self.memory.store.variants(&self.memory.setup, &key).into_iter().find_map(|p| {
                    let (s, fallback) = find(p)?;
                    let then = vergence_profile::windows::key_sizes(&p.key).1;
                    let s = match (size_in(&then, &s.monitor), size_in(&now, &s.monitor)) {
                        (Some(from), Some(to)) => s.scaled(from, to),
                        _ => s,
                    };
                    Some((s, fallback))
                })?
            }
        };
        if (s.width as i32) < MIN_W || (s.height as i32) < MIN_H {
            return None;
        }
        let out = self.space.outputs().find(|o| monitor_id_of(o).as_deref() == Some(s.monitor.as_str()))?.clone();
        let og = self.space.output_geometry(&out)?;
        let mut r = Rectangle::new((og.loc.x + s.x, og.loc.y + s.y).into(), (s.width as i32, s.height as i32).into());
        // A new window starting from its program's last-closed place does not
        // land exactly on another of its windows (three new terminals stacked
        // exactly on one another, 2026-11-12).
        if fallback {
            r = self.clear_of_siblings(w, r, og);
        }
        // Where the shelf's strip now stands, the window is moved clear of it (shelf.rs).
        let r = self.clear_of_shelf(r);
        let maximized = s.max_horz && s.max_vert;
        if !maximized && !self.bar_on_a_monitor(r, self.has_title(w)) {
            return None;
        }
        // It was snapped: put in its part once it is shown (snap.rs, opened.rs).
        match vergence_shell::snap::Zone::parse(&s.snap) {
            Some(z) if !maximized => {
                self.snap.recalled.insert(w.clone(), z);
            }
            _ => {
                self.snap.recalled.remove(w);
            }
        }
        Some((r, maximized))
    }

    /// Record where a window is now (its restore size and place if maximized).
    pub fn remember(&mut self, w: &Win) {
        if self.memory.setup.is_empty() {
            return;
        }
        let Some(id) = self.window_identity(w) else { return };
        let maximized = self.restore.get(w).copied();
        // A snapped window (snap.rs) is kept by where it was before, and its part.
        let snapped = self.snap.snapped.get(w).copied();
        let Some(geo) = maximized.or(snapped.map(|s| s.before)).or_else(|| self.space.element_geometry(w)) else { return };
        // A window that hasn't got its size yet (1x1 before its first commit),
        // or was squeezed to it, is not a place to remember.
        if geo.size.w < MIN_W || geo.size.h < MIN_H {
            return;
        }
        // The monitor its center is on; for a window hanging off the edge with
        // its center off every monitor, the one its title bar is on (it is
        // restored there while the bar is wholly on it, place.rs).
        let center = (geo.loc.x + geo.size.w / 2, geo.loc.y + geo.size.h / 2);
        let h = crate::decor::TITLE_H;
        let bar = (geo.loc.x + geo.size.w / 2, if self.has_title(w) { geo.loc.y - h / 2 } else { geo.loc.y + h / 2 });
        let on = |p: (i32, i32)| {
            self.space.outputs().find_map(|o| {
                let g = self.space.output_geometry(o)?;
                g.contains(p).then(|| (monitor_id_of(o), g))
            })
        };
        let Some((mon, og)) = on(center).or_else(|| on(bar)) else {
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
            max_horz: maximized.is_some(),
            max_vert: maximized.is_some(),
            fullscreen: false,
            desktop: None,
            seen: if id.token.is_empty() { 0 } else { today() },
            snap: snapped.map(|s| s.zone.as_str().to_string()).unwrap_or_default(),
        };
        let setup = self.sized_setup();
        let profile = self.memory.store.get_mut_or_new(&setup);
        if profile.put(setting.clone()) {
            profile.updated = now();
            tracing::info!("memory: {} {}", id.describe(), setting.describe_place());
            self.memory.save();
        }
    }

    /// The key places are kept under now: the setup's, with each monitor's
    /// size in points (empty until the setup is known).
    pub fn sized_setup(&self) -> String {
        if self.memory.setup.is_empty() {
            return String::new();
        }
        let sizes: Vec<(String, u32, u32)> = self
            .space
            .outputs()
            .filter_map(|o| Some((monitor_id_of(o)?, self.space.output_geometry(o)?.size)))
            .map(|(id, s)| (id, s.w.max(0) as u32, s.h.max(0) as u32))
            .collect();
        vergence_profile::windows::sized_key(&self.memory.setup, &sizes)
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

/// The name of the program a surface's client runs, from its process id.
fn program_of(dh: &smithay::reexports::wayland_server::DisplayHandle, surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) -> Option<String> {
    use smithay::reexports::wayland_server::Resource;
    let client = surface.client()?;
    let pid = client.get_credentials(dh).ok()?.pid;
    #[cfg(target_os = "linux")]
    if let Ok(s) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        return Some(s.trim().to_string()).filter(|s| !s.is_empty());
    }
    let out = std::process::Command::new("ps").args(["-o", "comm=", "-p", &pid.to_string()]).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

/// Today, in days since 1970-01-01 (for when a tokened window was last seen).
pub fn today() -> u32 {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    (secs / 86_400) as u32
}

impl Server {
    /// The apps window memory keeps places for, and how many each (bus `ListPlaces`).
    pub fn places_apps(&self) -> Vec<(String, u32)> {
        self.memory.store.apps()
    }

    /// Forget an app's places on every setup, or every app's (`app` empty);
    /// saved at once (bus `ForgetPlaces`; settings milestone 2). Open windows
    /// stay where they are and are remembered again when they move.
    pub fn forget_places(&mut self, app: &str) {
        let n = self.memory.store.forget_app((!app.is_empty()).then_some(app));
        tracing::info!("memory: forgot {n} places of {}", if app.is_empty() { "every app" } else { app });
        self.memory.save();
    }

    /// The user closed a window with a token for good: where it was becomes
    /// its program's last-closed place (a new window starts there), and its
    /// token is forgotten on every monitor setup.
    pub fn forget_window(&mut self, w: &Win) {
        let Some(id) = self.memory.ids.get(w).cloned() else { return };
        if id.token.is_empty() {
            return;
        }
        let setup = self.sized_setup();
        if let Some(mut last) = self.memory.store.get(&setup).and_then(|p| p.get(&id)).cloned() {
            last.id = id.untokened();
            last.seen = 0;
            self.memory.store.get_mut_or_new(&setup).put(last);
        }
        let n = self.memory.store.forget_token(&id.token);
        self.memory.ids.remove(w);
        tracing::info!("memory: {} closed for good; {n} places forgotten", id.describe());
        self.memory.save();
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
