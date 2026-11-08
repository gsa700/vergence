//! vergd as the XSETTINGS manager (milestone 4, docs/milestone-4.md,
//! decision 2). GTK reads its theme, fonts, window scale and DPI from the
//! XSETTINGS manager; vergd is that manager on any window manager, as the
//! phase 2 server will be for X11 apps.
//!
//! - **The settings** live in `~/.config/vergence/settings.toml`. The first
//!   time, they are imported from the manager that is running (Xfce's
//!   xfsettingsd on Hubble), over the X protocol. The file is re-read when
//!   it changes, so it can be edited at any time.
//! - **Scaling** (milestone 4) lays its values over the file's
//!   ([`Manager::set_overlay`]); the file keeps the user's own.
//! - **Taking over** follows the XSETTINGS specification: own the
//!   `_XSETTINGS_S<screen>` selection with a real timestamp, put the
//!   settings in `_XSETTINGS_SETTINGS` on the owner window, and announce it
//!   with a `MANAGER` client message on the root window.
//! - **Losing it:** if another program takes the selection, vergd says so
//!   and does not fight for it. Ownership is checked every
//!   [`CHECK_EVERY`], because vergd's waits swallow events.

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime};

use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask,
    PropMode, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::COPY_DEPTH_FROM_PARENT;

use vergence_xsettings::{Settings, Value};

use crate::{stamp, store};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// How often vergd checks that it still owns the selection and that the
/// settings file has not changed.
const CHECK_EVERY: Duration = Duration::from_secs(2);

pub struct Atoms {
    pub selection: u32,
    pub settings: u32,
    pub manager: u32,
}

impl Atoms {
    pub fn new(conn: &RustConnection, screen: usize) -> Res<Atoms> {
        let a = |n: &str| -> Res<u32> { Ok(conn.intern_atom(false, n.as_bytes())?.reply()?.atom) };
        Ok(Atoms {
            selection: a(&format!("_XSETTINGS_S{screen}"))?,
            settings: a("_XSETTINGS_SETTINGS")?,
            manager: a("MANAGER")?,
        })
    }
}

/// The window that owns the XSETTINGS selection, if any.
pub fn owner(conn: &RustConnection, a: &Atoms) -> Res<Option<u32>> {
    let w = conn.get_selection_owner(a.selection)?.reply()?.owner;
    Ok(if w == 0 { None } else { Some(w) })
}

/// The settings the current manager publishes; None if there is none.
pub fn read_current(conn: &RustConnection, a: &Atoms) -> Res<Option<(u32, Settings)>> {
    let Some(w) = owner(conn, a)? else { return Ok(None) };
    let p = conn.get_property(false, w, a.settings, AtomEnum::ANY, 0, 1 << 20)?.reply()?;
    if p.value.is_empty() {
        return Ok(None);
    }
    Ok(Some((w, Settings::decode(&p.value)?)))
}

// ---- the settings file ----

/// The settings file format lives in vergence-xsettings (the server reads it too).
pub fn to_toml(s: &Settings) -> String {
    vergence_xsettings::to_toml(s)
}

pub fn from_toml(text: &str) -> Res<Settings> {
    Ok(vergence_xsettings::from_toml(text)?)
}

// ---- the manager ----

pub struct Manager {
    a: Atoms,
    win: u32,
    /// The user's settings, from the file.
    base: Settings,
    /// Values laid over the file's (scaling).
    overlay: BTreeMap<String, Value>,
    /// What is published now; its serial grows with every change.
    published: Settings,
    file_mtime: Option<SystemTime>,
    last_check: Instant,
    pub active: bool,
}

/// A real server timestamp, from a property change on our own window.
fn timestamp(conn: &RustConnection, win: u32, prop: u32) -> Res<u32> {
    conn.change_property(PropMode::APPEND, win, prop, AtomEnum::STRING, 8, 0, &[])?;
    conn.flush()?;
    loop {
        if let Event::PropertyNotify(e) = conn.wait_for_event()? {
            if e.window == win {
                return Ok(e.time);
            }
        }
    }
}

impl Manager {
    /// Load (or import) the settings and take the XSETTINGS selection.
    pub fn start(conn: &RustConnection, root: u32) -> Res<Manager> {
        let screen = conn.setup().roots.iter().position(|r| r.root == root).unwrap_or(0);
        let a = Atoms::new(conn, screen)?;
        let base = match store::load_settings()? {
            Some(text) => from_toml(&text).map_err(|e| format!("{}: {e}", store::settings_path().unwrap().display()))?,
            None => match read_current(conn, &a)? {
                Some((w, s)) => {
                    store::save_settings(&to_toml(&s))?;
                    println!(
                        "{} imported {} settings from the XSETTINGS manager (window {w:#x}) into ~/.config/vergence/settings.toml",
                        stamp(),
                        s.settings.len()
                    );
                    s
                }
                None => {
                    println!("{} no XSETTINGS manager to import from; starting with an empty settings.toml", stamp());
                    let s = Settings::default();
                    store::save_settings(&to_toml(&s))?;
                    s
                }
            },
        };

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
        let ts = timestamp(conn, win, a.settings)?;
        // From now on our window sends no events into the main loop.
        conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT))?;
        let before = owner(conn, &a)?;
        conn.set_selection_owner(win, a.selection, ts)?;
        if owner(conn, &a)? != Some(win) {
            return Err("could not take the XSETTINGS selection".into());
        }
        let mut m = Manager {
            a,
            win,
            base,
            overlay: BTreeMap::new(),
            published: Settings::default(),
            file_mtime: store::settings_mtime(),
            last_check: Instant::now(),
            active: true,
        };
        m.publish(conn)?;
        let ev = ClientMessageEvent::new(32, root, m.a.manager, [ts, m.a.selection, win, 0, 0]);
        conn.send_event(false, root, EventMask::STRUCTURE_NOTIFY, ev)?;
        conn.flush()?;
        let from = match before {
            Some(w) => format!(", taken over from window {w:#x}"),
            None => String::new(),
        };
        println!(
            "{} XSETTINGS manager: publishing {} settings{from}",
            stamp(),
            m.published.settings.len()
        );
        Ok(m)
    }

    /// An integer from the user's settings (the file), if set.
    pub fn base_int(&self, name: &str) -> Option<i32> {
        match self.base.get(name) {
            Some(Value::Int(i)) => Some(*i),
            _ => None,
        }
    }

    /// Lay `values` over the file's settings (replacing the previous overlay)
    /// and publish if anything changed.
    pub fn set_overlay(&mut self, conn: &RustConnection, values: BTreeMap<String, Value>) -> Res<()> {
        self.overlay = values;
        self.publish(conn)
    }

    /// Publish file + overlay; the serial only grows when something changed.
    fn publish(&mut self, conn: &RustConnection) -> Res<()> {
        if !self.active {
            return Ok(());
        }
        let mut want: BTreeMap<&str, &Value> = self.base.settings.iter().map(|(k, v)| (k.as_str(), &v.value)).collect();
        for (k, v) in &self.overlay {
            want.insert(k, v);
        }
        let mut changed = false;
        let gone: Vec<String> =
            self.published.settings.keys().filter(|k| !want.contains_key(k.as_str())).cloned().collect();
        for k in gone {
            self.published.settings.remove(&k);
            changed = true;
        }
        let first = self.published.settings.is_empty();
        for (k, v) in want {
            changed |= self.published.set(k, v.clone());
        }
        if changed || first {
            let bytes = self.published.encode();
            conn.change_property(PropMode::REPLACE, self.win, self.a.settings, self.a.settings, 8, bytes.len() as u32, &bytes)?;
            conn.flush()?;
        }
        Ok(())
    }

    /// Every [`CHECK_EVERY`]: still the owner? settings file changed?
    pub fn tick(&mut self, conn: &RustConnection) -> Res<()> {
        if !self.active || self.last_check.elapsed() < CHECK_EVERY {
            return Ok(());
        }
        self.last_check = Instant::now();
        let now = owner(conn, &self.a)?;
        if now != Some(self.win) {
            self.active = false;
            let who = now.map(|w| format!("window {w:#x}")).unwrap_or_else(|| "nobody".into());
            println!("{} XSETTINGS: another program took the selection ({who}); vergd leaves it alone", stamp());
            return Ok(());
        }
        let m = store::settings_mtime();
        if m != self.file_mtime {
            self.file_mtime = m;
            match store::load_settings()?.map(|t| from_toml(&t)) {
                Some(Ok(s)) => {
                    self.base = s;
                    let before = self.published.serial;
                    self.publish(conn)?;
                    if self.published.serial != before {
                        println!("{} XSETTINGS: settings.toml changed, published", stamp());
                    }
                }
                Some(Err(e)) => println!("{} XSETTINGS: settings.toml not used, it does not parse: {e}", stamp()),
                None => {}
            }
        }
        Ok(())
    }
}

/// `vergd xsettings`: what the current XSETTINGS manager publishes.
pub fn print_current() -> Res<()> {
    let (conn, screen) = x11rb::connect(None)?;
    let a = Atoms::new(&conn, screen)?;
    match read_current(&conn, &a)? {
        None => println!("no XSETTINGS manager is running"),
        Some((w, s)) => {
            println!("XSETTINGS manager: window {w:#x}, serial {}, {} settings", s.serial, s.settings.len());
            for (name, v) in &s.settings {
                println!("  {name:<32} {}", v.value);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_round_trip() {
        let mut s = Settings::default();
        s.set("Net/ThemeName", Value::Str("Adwaita".into()));
        s.set("Xft/DPI", Value::Int(98304));
        s.set("Xft/Hinting", Value::Int(-1));
        s.set("Test/Colour", Value::Color(1, 2, 3, 65535));
        let text = to_toml(&s);
        assert!(text.contains("\"Net/ThemeName\" = \"Adwaita\""), "{text}");
        let back = from_toml(&text).unwrap();
        for (k, v) in &s.settings {
            assert_eq!(back.get(k), Some(&v.value), "{k}");
        }
    }

    #[test]
    fn bad_values_are_refused() {
        assert!(from_toml("[settings]\n\"A\" = 1.5\n").is_err());
        assert!(from_toml("[settings]\n\"A\" = [1, 2]\n").is_err());
        assert!(from_toml("[settings]\n\"A\" = 99999999999\n").is_err());
        assert_eq!(from_toml("[settings]\n\"A\" = true\n").unwrap().get("A"), Some(&Value::Int(1)));
        assert!(from_toml("").unwrap().settings.is_empty());
    }
}
