//! Window memory (milestone 3, docs/milestone-3.md).
//!
//! vergd tracks the ordinary application windows the window manager lists
//! in `_NET_CLIENT_LIST`, remembers where each one is per monitor setup, and
//! puts them back when a setup returns and when an application opens a
//! window it has a place for. Everything goes through the window manager
//! (EWMH client messages to the root window); vergd never moves windows
//! behind its back.
//!
//! - **Recording** happens only when the monitor setup has been stable for
//!   [`STABLE`]: when a monitor goes, the window manager moves its windows
//!   onto the remaining screens, and those moves must not overwrite the
//!   remembered places.
//! - **Positions** are the client window's, set back with static gravity, so
//!   the window manager's frame and GTK's shadow (`_GTK_FRAME_EXTENTS`, part
//!   of the client window) do not shift a window a few pixels per restore.
//! - **No titles** are read or stored (Dave, 2026-11-06).

use std::collections::HashSet;
use std::time::{Duration, Instant};

use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt as _, EventMask,
};
use x11rb::rust_connection::RustConnection;

use vergence_profile::windows::{WindowId, WindowSetting, WindowStore};

use crate::x11::Setup;
use crate::{now_string, short, stamp};

/// How long the monitor setup must be unchanged before window moves are
/// recorded.
pub const STABLE: Duration = Duration::from_secs(3);
/// How long after a window appears it is placed (after the application's
/// own placement).
const PLACE_DELAY: Duration = Duration::from_millis(300);
/// The window file is written at most this often.
const WRITE_EVERY: Duration = Duration::from_secs(2);

type Res<T> = Result<T, Box<dyn std::error::Error>>;

struct Atoms {
    client_list: u32,
    role: u32,
    wtype: u32,
    wtype_normal: u32,
    state: u32,
    max_h: u32,
    max_v: u32,
    fullscreen: u32,
    skip_taskbar: u32,
    hidden: u32,
    desktop: u32,
    moveresize: u32,
}

impl Atoms {
    fn new(conn: &RustConnection) -> Res<Atoms> {
        let a = |n: &[u8]| -> Res<u32> { Ok(conn.intern_atom(false, n)?.reply()?.atom) };
        Ok(Atoms {
            client_list: a(b"_NET_CLIENT_LIST")?,
            role: a(b"WM_WINDOW_ROLE")?,
            wtype: a(b"_NET_WM_WINDOW_TYPE")?,
            wtype_normal: a(b"_NET_WM_WINDOW_TYPE_NORMAL")?,
            state: a(b"_NET_WM_STATE")?,
            max_h: a(b"_NET_WM_STATE_MAXIMIZED_HORZ")?,
            max_v: a(b"_NET_WM_STATE_MAXIMIZED_VERT")?,
            fullscreen: a(b"_NET_WM_STATE_FULLSCREEN")?,
            skip_taskbar: a(b"_NET_WM_STATE_SKIP_TASKBAR")?,
            hidden: a(b"_NET_WM_STATE_HIDDEN")?,
            desktop: a(b"_NET_WM_DESKTOP")?,
            moveresize: a(b"_NET_MOVERESIZE_WINDOW")?,
        })
    }
}

/// One managed window as it is now.
#[derive(Debug, Clone)]
struct Win {
    xid: u32,
    id: WindowId,
    /// Ordinary application window that shows in the taskbar and is not
    /// minimised.
    eligible: bool,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    max_h: bool,
    max_v: bool,
    fs: bool,
    desktop: Option<u32>,
}

fn prop32(conn: &RustConnection, win: u32, prop: u32, len: u32) -> Res<Vec<u32>> {
    let any: u32 = AtomEnum::ANY.into();
    let r = conn.get_property(false, win, prop, any, 0, len)?.reply()?;
    Ok(r.value32().map(|v| v.collect()).unwrap_or_default())
}

fn prop_string(conn: &RustConnection, win: u32, prop: u32) -> Res<Vec<u8>> {
    let any: u32 = AtomEnum::ANY.into();
    Ok(conn.get_property(false, win, prop, any, 0, 256)?.reply()?.value)
}

fn client_list(conn: &RustConnection, root: u32, a: &Atoms) -> Res<Vec<u32>> {
    prop32(conn, root, a.client_list, 4096)
}

fn read_win(conn: &RustConnection, root: u32, a: &Atoms, xid: u32) -> Res<Option<Win>> {
    let class = prop_string(conn, xid, AtomEnum::WM_CLASS.into())?;
    let mut parts = class.split(|&b| b == 0).map(|p| String::from_utf8_lossy(p).into_owned());
    let instance = parts.next().unwrap_or_default();
    let class = parts.next().unwrap_or_default();
    if instance.is_empty() && class.is_empty() {
        return Ok(None);
    }
    let role = stable_role(String::from_utf8_lossy(&prop_string(conn, xid, a.role)?).trim_end_matches('\0'));
    let types = prop32(conn, xid, a.wtype, 32)?;
    let states = prop32(conn, xid, a.state, 32)?;
    let desktop = prop32(conn, xid, a.desktop, 1)?.first().copied();
    let geo = conn.get_geometry(xid)?.reply()?;
    let pos = conn.translate_coordinates(xid, root, 0, 0)?.reply()?;

    let normal = types.is_empty() || types.contains(&a.wtype_normal);
    let eligible = normal && !states.contains(&a.skip_taskbar) && !states.contains(&a.hidden);
    Ok(Some(Win {
        xid,
        id: WindowId { instance, class, role, index: 0 },
        eligible,
        x: pos.dst_x as i32,
        y: pos.dst_y as i32,
        w: geo.width as u32,
        h: geo.height as u32,
        max_h: states.contains(&a.max_h),
        max_v: states.contains(&a.max_v),
        fs: states.contains(&a.fullscreen),
        desktop,
    }))
}

/// A window role worth matching on, or "" for none. Some apps give every
/// window a unique role (xfce4-terminal: "xfce4-terminal-1794016006-744545988"),
/// which would never match after a restart and would add a new entry for
/// every window ever opened. A role with a run of 6 or more digits is such a
/// serial number and is dropped; the window is then known by class and order.
/// Roles like Firefox's "browser" or Thunderbird's "3pane" are kept.
pub fn stable_role(role: &str) -> String {
    let mut run = 0;
    for c in role.chars() {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        if run >= 6 {
            return String::new();
        }
    }
    role.to_string()
}

#[cfg(test)]
mod tests {
    use super::stable_role;

    #[test]
    fn serial_roles_dropped() {
        assert_eq!(stable_role("xfce4-terminal-1794016006-744545988"), "");
        assert_eq!(stable_role("browser"), "browser");
        assert_eq!(stable_role("3pane"), "3pane");
        assert_eq!(stable_role("GtkFileChooserDialog"), "GtkFileChooserDialog");
        assert_eq!(stable_role("win-12345"), "win-12345");
        assert_eq!(stable_role(""), "");
    }
}

/// Every window in `_NET_CLIENT_LIST`, in its order (the order they were
/// mapped), with each window's place among windows of its kind. Windows
/// that vanish while being read are skipped.
fn list(conn: &RustConnection, root: u32, a: &Atoms) -> Res<Vec<Win>> {
    let mut out: Vec<Win> = Vec::new();
    for xid in client_list(conn, root, a)? {
        let Ok(Some(mut w)) = read_win(conn, root, a, xid) else { continue };
        w.id.index = out
            .iter()
            .filter(|o| o.id.instance == w.id.instance && o.id.class == w.id.class && o.id.role == w.id.role)
            .count() as u32;
        out.push(w);
    }
    Ok(out)
}

/// The rectangle of each lit monitor: (monitor ID, name, x, y, w, h).
fn monitors(setup: &Setup) -> Vec<(String, String, i32, i32, i32, i32)> {
    setup
        .outputs
        .iter()
        .filter(|o| o.connected)
        .filter_map(|o| {
            let cur = o.current.as_ref()?;
            let c = setup.crtcs.iter().find(|c| c.id == cur.crtc)?;
            let name = match o.monitor_name() {
                n if n.is_empty() => o.name.clone(),
                n => n,
            };
            Some((o.monitor(), name, c.x as i32, c.y as i32, c.width as i32, c.height as i32))
        })
        .collect()
}

/// Where a window is, as the store keeps it; None if its centre is on no
/// lit monitor.
fn setting_of(w: &Win, setup: &Setup) -> Option<WindowSetting> {
    let cx = w.x + w.w as i32 / 2;
    let cy = w.y + w.h as i32 / 2;
    let (mon, _, mx, my, _, _) = monitors(setup)
        .into_iter()
        .find(|(_, _, x, y, mw, mh)| cx >= *x && cx < x + mw && cy >= *y && cy < y + mh)?;
    Some(WindowSetting {
        id: w.id.clone(),
        monitor: mon,
        x: w.x - mx,
        y: w.y - my,
        width: w.w,
        height: w.h,
        max_horz: w.max_h,
        max_vert: w.max_v,
        fullscreen: w.fs,
        desktop: w.desktop,
    })
}

fn client_message(conn: &RustConnection, root: u32, win: u32, atom: u32, data: [u32; 5]) -> Res<()> {
    let ev = ClientMessageEvent::new(32, win, atom, data);
    conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        ev,
    )?;
    Ok(())
}

/// Source indication 2 = a pager or other tool acting for the user.
const SOURCE_TOOL: u32 = 2;

/// Move `w` to `s`. Returns false if it is already there or its monitor is
/// not lit.
fn place(conn: &RustConnection, root: u32, a: &Atoms, w: &Win, s: &WindowSetting, setup: &Setup) -> Res<bool> {
    let Some((_, _, mx, my, _, _)) = monitors(setup).into_iter().find(|m| m.0 == s.monitor) else {
        return Ok(false);
    };
    let (tx, ty) = (mx + s.x, my + s.y);
    let same = w.x == tx
        && w.y == ty
        && w.w == s.width
        && w.h == s.height
        && w.max_h == s.max_horz
        && w.max_v == s.max_vert
        && w.fs == s.fullscreen
        && (s.desktop.is_none() || w.desktop == s.desktop);
    if same {
        return Ok(false);
    }
    // Un-maximise first: a maximised window ignores moves.
    if w.max_h || w.max_v {
        client_message(conn, root, w.xid, a.state, [0, a.max_h, a.max_v, SOURCE_TOOL, 0])?;
    }
    if w.fs {
        client_message(conn, root, w.xid, a.state, [0, a.fullscreen, 0, SOURCE_TOOL, 0])?;
    }
    // _NET_MOVERESIZE_WINDOW: static gravity (10), x, y, width and height
    // present (bits 8-11), source in bits 12-15.
    let flags = 10 | (0xF << 8) | (SOURCE_TOOL << 12);
    client_message(
        conn,
        root,
        w.xid,
        a.moveresize,
        [flags, tx as u32, ty as u32, s.width, s.height],
    )?;
    if let Some(d) = s.desktop {
        if w.desktop != Some(d) {
            client_message(conn, root, w.xid, a.desktop, [d, SOURCE_TOOL, 0, 0, 0])?;
        }
    }
    if s.max_horz || s.max_vert {
        let h = if s.max_horz { a.max_h } else { 0 };
        let v = if s.max_vert { a.max_v } else { 0 };
        client_message(conn, root, w.xid, a.state, [1, h, v, SOURCE_TOOL, 0])?;
    }
    if s.fullscreen {
        client_message(conn, root, w.xid, a.state, [1, a.fullscreen, 0, SOURCE_TOOL, 0])?;
    }
    conn.flush()?;
    Ok(true)
}

fn monitor_name(setup: &Setup, id: &str) -> String {
    monitors(setup).into_iter().find(|m| m.0 == id).map(|m| m.1).unwrap_or_else(|| id.to_string())
}

pub struct WindowMemory {
    pub enabled: bool,
    root: u32,
    atoms: Atoms,
    known: HashSet<u32>,
    pending: Vec<(u32, Instant)>,
    unstable_at: Instant,
    dirty: bool,
    last_write: Option<Instant>,
}

impl WindowMemory {
    pub fn new(conn: &RustConnection, root: u32, enabled: bool) -> Res<WindowMemory> {
        let atoms = Atoms::new(conn)?;
        let mut wm = WindowMemory {
            enabled,
            root,
            atoms,
            known: HashSet::new(),
            pending: Vec::new(),
            unstable_at: Instant::now(),
            dirty: true,
            last_write: None,
        };
        if enabled {
            conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE))?
                .check()?;
            // Windows open at start are tracked, not placed.
            for xid in client_list(conn, root, &wm.atoms)? {
                wm.watch_window(conn, xid);
            }
        }
        Ok(wm)
    }

    fn watch_window(&mut self, conn: &RustConnection, xid: u32) {
        if self.known.insert(xid) {
            let mask = EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE;
            // The window may already be gone; that's fine.
            if let Ok(c) = conn.change_window_attributes(xid, &ChangeWindowAttributesAux::new().event_mask(mask)) {
                c.ignore_error();
            }
        }
    }

    /// The monitor setup changed (or vergd moved windows): hold recording.
    pub fn mark_unstable(&mut self) {
        self.unstable_at = Instant::now();
        self.dirty = true;
    }

    fn stable(&self) -> bool {
        self.unstable_at.elapsed() >= STABLE
    }

    /// Something happened to a window or the window list.
    pub fn on_window_events(&mut self, conn: &RustConnection) -> Res<()> {
        if !self.enabled {
            return Ok(());
        }
        let now: Vec<u32> = client_list(conn, self.root, &self.atoms)?;
        let current: HashSet<u32> = now.iter().copied().collect();
        self.known.retain(|x| current.contains(x));
        for xid in now {
            if !self.known.contains(&xid) {
                self.watch_window(conn, xid);
                self.pending.push((xid, Instant::now() + PLACE_DELAY));
            }
        }
        self.dirty = true;
        Ok(())
    }

    /// Timers: place newly opened windows, record when stable.
    pub fn tick(&mut self, conn: &RustConnection, setup: &Setup) -> Res<()> {
        if !self.enabled || setup.edids().is_empty() {
            return Ok(());
        }
        let due: Vec<u32> = self.pending.iter().filter(|(_, t)| Instant::now() >= *t).map(|(x, _)| *x).collect();
        if !due.is_empty() && self.stable() {
            self.pending.retain(|(x, _)| !due.contains(x));
            self.place_new(conn, setup, &due)?;
        }
        if self.dirty && self.stable() && self.pending.is_empty() {
            let throttled = self.last_write.is_some_and(|t| t.elapsed() < WRITE_EVERY);
            if !throttled {
                self.record(conn, setup)?;
                self.dirty = false;
            }
        }
        Ok(())
    }

    fn place_new(&mut self, conn: &RustConnection, setup: &Setup, xids: &[u32]) -> Res<()> {
        let store = crate::store::load_windows()?;
        let key = setup.key();
        let Some(prof) = store.get(&key) else { return Ok(()) };
        let mut moved = false;
        for w in list(conn, self.root, &self.atoms)? {
            if !xids.contains(&w.xid) || !w.eligible || store.skips(&w.id) {
                continue;
            }
            if let Some(s) = prof.get(&w.id) {
                if place(conn, self.root, &self.atoms, &w, s, setup)? {
                    println!(
                        "{} opened {}: placed on {} {}",
                        stamp(),
                        w.id.describe(),
                        monitor_name(setup, &s.monitor),
                        s.describe_place()
                    );
                    moved = true;
                }
            }
        }
        if moved {
            // Let the window settle before recording it.
            self.unstable_at = Instant::now() - STABLE + Duration::from_millis(800);
        }
        Ok(())
    }

    /// Put every open window back where it was on this setup.
    pub fn restore_all(&mut self, conn: &RustConnection, setup: &Setup) -> Res<()> {
        if !self.enabled || setup.edids().is_empty() {
            return Ok(());
        }
        let store = crate::store::load_windows()?;
        let key = setup.key();
        let Some(prof) = store.get(&key) else { return Ok(()) };
        let mut n = 0;
        for w in list(conn, self.root, &self.atoms)? {
            if !w.eligible || store.skips(&w.id) {
                continue;
            }
            if let Some(s) = prof.get(&w.id) {
                if place(conn, self.root, &self.atoms, &w, s, setup)? {
                    println!(
                        "{}   moved {} to {} {}",
                        stamp(),
                        w.id.describe(),
                        monitor_name(setup, &s.monitor),
                        s.describe_place()
                    );
                    n += 1;
                }
            }
        }
        if n > 0 {
            println!("{} restored {n} window(s) for {}", stamp(), short(&key));
        }
        self.mark_unstable();
        Ok(())
    }

    /// Save where every eligible window is, for the current setup. Windows
    /// that are closed keep their remembered place (for the next start).
    fn record(&mut self, conn: &RustConnection, setup: &Setup) -> Res<()> {
        let mut store: WindowStore = crate::store::load_windows()?;
        let key = setup.key();
        let wins = list(conn, self.root, &self.atoms)?;
        let skip: Vec<bool> = wins.iter().map(|w| store.skips(&w.id)).collect();
        let prof = store.get_mut_or_new(&key);
        let mut changed = false;
        for (w, skipped) in wins.iter().zip(skip) {
            if !w.eligible || skipped {
                continue;
            }
            if let Some(s) = setting_of(w, setup) {
                changed |= prof.put(s);
            }
        }
        if changed {
            prof.updated = now_string();
            crate::store::save_windows(&store)?;
            self.last_write = Some(Instant::now());
        }
        Ok(())
    }
}

/// `vergd windows`: what is remembered.
pub fn print_store(store: &WindowStore) {
    if !store.skip.is_empty() {
        println!("skipped apps: {}", store.skip.join(", "));
    }
    for p in &store.profiles {
        println!("{}  {}", short(&p.key), p.updated);
        for w in &p.windows {
            println!("    {:<32} on {} {}", w.id.describe(), &w.monitor[..w.monitor.len().min(8)], w.describe_place());
        }
    }
}
