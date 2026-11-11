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
//! - **Units that survive scaling** (milestone 4): positions are kept in
//!   the monitor's **panel pixels** (X pixels divided by the monitor's
//!   RandR factor), sizes in **app units** (X pixels divided by the render
//!   scale, which is how the apps size themselves). Turning Mode A on or
//!   off then keeps every window at the same place on its panel, at its
//!   app's own size. Unscaled, both units are plain X pixels, so memory
//!   recorded before milestone 4 means the same.
//! - **Rescue:** after a monitor change, a window with no place on the new
//!   setup that the window manager left mostly off-screen (xfwm4 does this
//!   to windows on other workspaces) is moved fully onto the nearest monitor.

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
    workarea: u32,
    frame_extents: u32,
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
            workarea: a(b"_NET_WORKAREA")?,
            frame_extents: a(b"_NET_FRAME_EXTENTS")?,
        })
    }
}

/// One managed window as it is now.
#[derive(Debug, Clone)]
struct Win {
    xid: u32,
    id: WindowId,
    /// Ordinary application window that shows in the taskbar and is not
    /// minimized.
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
    use super::{rescue_target, stable_role};

    #[test]
    fn rescue_targets() {
        let asus = (0, 27, 1920, 1053);
        let fake = (1920, 0, 1920, 1080);
        // Thunderbird left at x=1896 on a 1920-wide screen: moved left just enough.
        assert_eq!(rescue_target((1896, 300, 1300, 880), &[asus]), Some((0, 620, 200)));
        // Half or more visible: left alone.
        assert_eq!(rescue_target((1000, 300, 1300, 800), &[asus]), None);
        // Straddling two monitors, all visible: left alone.
        assert_eq!(rescue_target((1500, 300, 1000, 800), &[asus, fake]), None);
        // Off every monitor: the nearest one, here the fake.
        assert_eq!(rescue_target((4000, 100, 800, 600), &[asus, fake]), Some((1, 3040, 100)));
        // Above the panel: pushed below it.
        assert_eq!(rescue_target((100, -700, 800, 720), &[asus]), Some((0, 100, 27)));
        // Larger than the monitor: top-left corner on screen.
        assert_eq!(rescue_target((3000, 500, 2500, 1200), &[asus]), Some((0, 0, 27)));
        assert_eq!(rescue_target((10, 10, 100, 100), &[]), None);
    }

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

/// Where a window is, as the store keeps it; None if its center is on no
/// lit monitor.
fn setting_of(w: &Win, setup: &Setup, render: u32) -> Option<WindowSetting> {
    let cx = w.x + w.w as i32 / 2;
    let cy = w.y + w.h as i32 / 2;
    let (mon, _, mx, my, _, _) = monitors(setup)
        .into_iter()
        .find(|(_, _, x, y, mw, mh)| cx >= *x && cx < x + mw && cy >= *y && cy < y + mh)?;
    let f = factor_of(setup, &mon);
    let r = render.max(1) as f64;
    Some(WindowSetting {
        id: w.id.clone(),
        x: ((w.x - mx) as f64 / f).round() as i32,
        y: ((w.y - my) as f64 / f).round() as i32,
        width: (w.w as f64 / r).round() as u32,
        height: (w.h as f64 / r).round() as u32,
        monitor: mon,
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
fn place(
    conn: &RustConnection,
    root: u32,
    a: &Atoms,
    w: &Win,
    s: &WindowSetting,
    setup: &Setup,
    render: u32,
) -> Res<bool> {
    let Some((_, _, mx, my, _, _)) = monitors(setup).into_iter().find(|m| m.0 == s.monitor) else {
        return Ok(false);
    };
    // Back from panel pixels and app units to X pixels (see the top).
    let f = factor_of(setup, &s.monitor);
    let tx = mx + (s.x as f64 * f).round() as i32;
    let ty = my + (s.y as f64 * f).round() as i32;
    let (tw, th) = (s.width * render.max(1), s.height * render.max(1));
    let same = w.x == tx
        && w.y == ty
        && w.w == tw
        && w.h == th
        && w.max_h == s.max_horz
        && w.max_v == s.max_vert
        && w.fs == s.fullscreen
        && (s.desktop.is_none() || w.desktop == s.desktop);
    if same {
        return Ok(false);
    }
    // Un-maximize first: a maximized window ignores moves.
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
        [flags, tx as u32, ty as u32, tw, th],
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

/// A rectangle: x, y, width, height.
type Rect = (i32, i32, i32, i32);

fn overlap(a: Rect, b: Rect) -> i64 {
    let w = (a.0 + a.2).min(b.0 + b.2) - a.0.max(b.0);
    let h = (a.1 + a.3).min(b.1 + b.3) - a.1.max(b.1);
    if w > 0 && h > 0 { w as i64 * h as i64 } else { 0 }
}

/// Where to move a window (its frame, `r`) so it is fully on one of
/// `areas`, or None if at least half of it is visible already. The area it
/// overlaps most wins, else the one nearest its center; the window moves as
/// little as it can, and keeps its top-left corner on screen if it is
/// larger than the area.
fn rescue_target(r: Rect, areas: &[Rect]) -> Option<(usize, i32, i32)> {
    if areas.is_empty() || r.2 <= 0 || r.3 <= 0 {
        return None;
    }
    let visible: i64 = areas.iter().map(|a| overlap(r, *a)).sum();
    if visible * 2 >= r.2 as i64 * r.3 as i64 {
        return None;
    }
    let (cx, cy) = (r.0 as i64 + r.2 as i64 / 2, r.1 as i64 + r.3 as i64 / 2);
    let dist = |a: &Rect| {
        let dx = (a.0 as i64 + a.2 as i64 / 2) - cx;
        let dy = (a.1 as i64 + a.3 as i64 / 2) - cy;
        dx * dx + dy * dy
    };
    let i = (0..areas.len()).max_by_key(|&i| (overlap(r, areas[i]), -dist(&areas[i])))?;
    let a = areas[i];
    let fit = |pos: i32, len: i32, start: i32, size: i32| {
        if len >= size { start } else { pos.clamp(start, start + size - len) }
    };
    Some((i, fit(r.0, r.2, a.0, a.2), fit(r.1, r.3, a.1, a.3)))
}

/// Each lit monitor's usable area on `desktop`: the monitor cut to the
/// window manager's work area (which leaves out panels).
fn usable_areas(conn: &RustConnection, root: u32, a: &Atoms, setup: &Setup, desktop: Option<u32>) -> Vec<(String, Rect)> {
    let d = match desktop {
        Some(d) if d != vergence_profile::windows::ALL_DESKTOPS => d as usize,
        _ => 0,
    };
    let wa = prop32(conn, root, a.workarea, 4096).unwrap_or_default();
    let work: Option<Rect> = wa.get(d * 4..d * 4 + 4).map(|v| (v[0] as i32, v[1] as i32, v[2] as i32, v[3] as i32));
    monitors(setup)
        .into_iter()
        .map(|(_, name, x, y, w, h)| {
            let m = (x, y, w, h);
            let r = match work {
                Some(k) if overlap(m, k) > 0 => {
                    let x0 = x.max(k.0);
                    let y0 = y.max(k.1);
                    (x0, y0, (x + w).min(k.0 + k.2) - x0, (y + h).min(k.1 + k.3) - y0)
                }
                _ => m,
            };
            (name, r)
        })
        .collect()
}

/// Move `w` fully onto a monitor if it is mostly off every one. Returns
/// the monitor's name if it moved it.
fn rescue(conn: &RustConnection, root: u32, a: &Atoms, w: &Win, setup: &Setup) -> Res<Option<String>> {
    if w.max_h || w.max_v || w.fs {
        return Ok(None); // the window manager fits those to a monitor itself
    }
    let fe = prop32(conn, w.xid, a.frame_extents, 4)?;
    let (l, r, t, b) = match fe[..] {
        [l, r, t, b] => (l as i32, r as i32, t as i32, b as i32),
        _ => (0, 0, 0, 0),
    };
    let frame = (w.x - l, w.y - t, w.w as i32 + l + r, w.h as i32 + t + b);
    let areas = usable_areas(conn, root, a, setup, w.desktop);
    let rects: Vec<Rect> = areas.iter().map(|x| x.1).collect();
    let Some((i, fx, fy)) = rescue_target(frame, &rects) else { return Ok(None) };
    // Position only (bits 8, 9), static gravity: the client's own corner.
    let flags = 10 | (0x3 << 8) | (SOURCE_TOOL << 12);
    client_message(conn, root, w.xid, a.moveresize, [flags, (fx + l) as u32, (fy + t) as u32, 0, 0])?;
    conn.flush()?;
    Ok(Some(areas[i].0.clone()))
}

/// The RandR scale factor of the monitor with this ID (1 if unscaled).
fn factor_of(setup: &Setup, id: &str) -> f64 {
    setup
        .outputs
        .iter()
        .find(|o| o.connected && o.monitor() == id)
        .and_then(|o| o.current.as_ref())
        .map(|c| c.factor as f64 / vergence_profile::arrangement::FIXED_ONE as f64)
        .unwrap_or(1.0)
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
    /// The render scale the toolkits have now (milestone 4), for app units.
    render: u32,
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
            render: 1,
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

    /// The toolkits' render scale changed (Mode A on or off).
    pub fn set_render(&mut self, render: u32) {
        self.render = render.max(1);
        self.mark_unstable();
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
                if place(conn, self.root, &self.atoms, &w, s, setup, self.render)? {
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

    /// Put every open window back where it was on this setup; bring the
    /// ones with no place here fully on screen if they are mostly off it.
    pub fn restore_all(&mut self, conn: &RustConnection, setup: &Setup) -> Res<()> {
        if !self.enabled || setup.edids().is_empty() {
            return Ok(());
        }
        let store = crate::store::load_windows()?;
        let key = setup.key();
        let prof = store.get(&key);
        let lit: Vec<String> = monitors(setup).into_iter().map(|m| m.0).collect();
        let mut n = 0;
        for w in list(conn, self.root, &self.atoms)? {
            if !w.eligible || store.skips(&w.id) {
                continue;
            }
            match prof.and_then(|p| p.get(&w.id)).filter(|s| lit.contains(&s.monitor)) {
                Some(s) => {
                    if place(conn, self.root, &self.atoms, &w, s, setup, self.render)? {
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
                None => {
                    if let Some(m) = rescue(conn, self.root, &self.atoms, &w, setup)? {
                        println!("{}   brought {} fully onto {} (it was mostly off-screen)", stamp(), w.id.describe(), m);
                        n += 1;
                    }
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
            if let Some(s) = setting_of(w, setup, self.render) {
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
