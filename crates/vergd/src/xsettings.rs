//! vergd as the XSETTINGS manager (milestone 4, docs/milestone-4.md,
//! decision 2): reading the current manager's settings over the X protocol
//! (the import), and later publishing vergd's own.

use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

use vergence_xsettings::Settings;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

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
    conn.flush()?;
    Ok(())
}
