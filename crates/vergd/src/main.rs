//! vergd, the Vergence daemon.
//!
//!   vergd probe                 print each output and the profile key, then exit
//!   vergd watch [--observe-only] [--no-windows] [--no-xsettings]
//!                               the same, then one block per change (plug, mode,
//!                               wake); remembers and restores each setup's
//!                               arrangement (milestone 2) and its windows
//!                               (milestone 3) unless switched off
//!   vergd windows               list the remembered window places
//!   vergd profiles              list the remembered setups
//!   vergd forget <key-prefix>   drop a remembered setup
//!   vergd apply [--dry-run]     restore the current setup's profile now
//!
//! No network socket (whitepaper §7.3, phase 1). The only file written is
//! the profile file (store.rs), owner-only. On FreeBSD `watch` also reads
//! devd's local pipe (see watch.rs).

mod apply;
mod scale;
mod store;
mod watch;
mod windows;
mod x11;
mod xsettings;

use vergence_profile::scaling::Distance;
use std::process::ExitCode;

use vergence_edid::{Edid, MadeIn};
use vergence_profile::{duplicate_count, profile_key};

use x11::Setup;

fn usage() -> ExitCode {
    eprintln!(
        "usage: vergd probe\n       vergd watch [--observe-only] [--no-windows] [--no-xsettings]\n       vergd profiles\n       vergd windows\n       vergd xsettings\n       vergd scale [on|off|status|mode a|mode b]\n       \
         vergd forget <key-prefix>\n       vergd apply [--dry-run]"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match a.as_slice() {
        ["probe"] => x11::read_setup().map(|s| print_setup(&s)),
        ["watch", flags @ ..]
            if flags.iter().all(|f| ["--observe-only", "--no-windows", "--no-xsettings"].contains(f)) =>
        {
            watch::watch(
                flags.contains(&"--observe-only"),
                flags.contains(&"--no-windows"),
                flags.contains(&"--no-xsettings"),
            )
        }
        ["windows"] => store::load_windows().map(|s| windows::print_store(&s)).map_err(Into::into),
        ["xsettings"] => xsettings::print_current(),
        ["scale"] | ["scale", "status"] => scale_status(),
        ["scale", "on"] => scale_set(true),
        ["scale", "off"] => scale_set(false),
        ["scale", "mode", m] if ["a", "b"].contains(m) => scale_mode(m),
        ["profiles"] => profiles(),
        ["forget", prefix] => forget(prefix),
        ["apply"] => apply_now(false),
        ["apply", "--dry-run"] => apply_now(true),
        _ => return usage(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vergd: {e}");
            ExitCode::FAILURE
        }
    }
}

fn profiles() -> Result<(), Box<dyn std::error::Error>> {
    let s = store::load()?;
    if s.profiles.is_empty() {
        println!("no profiles in {}", store::path()?.display());
        return Ok(());
    }
    let current = x11::read_setup().ok().map(|s| s.key());
    for p in &s.profiles {
        let here = if current.as_deref() == Some(p.key.as_str()) { "  (connected now)" } else { "" };
        println!("{}  {}{here}", short(&p.key), p.updated);
        for o in &p.outputs {
            println!("    {}", o.describe());
        }
    }
    Ok(())
}

fn forget(prefix: &str) -> Result<(), Box<dyn std::error::Error>> {
    if prefix.len() < 4 {
        return Err("give at least 4 characters of the key".into());
    }
    let mut s = store::load()?;
    let gone = s.forget(prefix);
    match gone.len() {
        0 => println!("no profile starts with {prefix}"),
        1 => {
            store::save(&s)?;
            let mut w = store::load_windows()?;
            if w.forget(&gone[0]) > 0 {
                store::save_windows(&w)?;
            }
            println!("forgot {} (arrangement and windows)", short(&gone[0]));
        }
        n => {
            return Err(format!("{n} profiles start with {prefix}; give more of the key").into());
        }
    }
    Ok(())
}

fn apply_now(dry_run: bool) -> Result<(), Box<dyn std::error::Error>> {
    let setup = x11::read_setup()?;
    let key = setup.key();
    let s = store::load()?;
    let Some(p) = s.get(&key) else {
        println!("no profile for the current setup {}", short(&key));
        return Ok(());
    };
    match apply::apply(&p.outputs, dry_run)? {
        apply::Outcome::AlreadyThere => println!("already as saved"),
        apply::Outcome::Applied(steps) => {
            println!("{}:", if dry_run { "would do" } else { "applied" });
            for st in steps {
                println!("  {st}");
            }
        }
        apply::Outcome::Refused(why) => return Err(format!("not applying: {why}").into()),
        apply::Outcome::Reverted(why) => return Err(format!("apply failed: {why}").into()),
    }
    Ok(())
}

/// The first 8 hex digits of a key, for people.
pub fn short(key: &str) -> String {
    format!("{}…", &key[..key.len().min(8)])
}

fn local_tm() -> Option<libc::tm> {
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() { None } else { Some(tm) }
}

/// Local time as HH:MM:SS.
pub fn stamp() -> String {
    match local_tm() {
        Some(tm) => format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec),
        None => "--:--:--".into(),
    }
}

/// Local date and time, YYYY-MM-DD HH:MM:SS.
pub fn now_string() -> String {
    match local_tm() {
        Some(tm) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        ),
        None => String::new(),
    }
}

/// The full report: every output, then the key line.
pub fn print_setup(setup: &Setup) {
    for o in &setup.outputs {
        if !o.connected {
            println!("{:<9} disconnected", o.name);
            continue;
        }
        let mut line = format!("{:<9} connected", o.name);
        match &o.edid {
            None => line.push_str("  no EDID"),
            Some(raw) => match Edid::parse(raw) {
                Err(e) => line.push_str(&format!("  bad EDID ({} B): {e}", raw.len())),
                Ok(e) => line.push_str(&describe(&e, raw.len())),
            },
        }
        if let Some(m) = &o.native {
            line.push_str(&format!("\n          native  {m}"));
        }
        if let Some(d) = o.derived_scale(Distance::Desktop, 1.0) {
            match (d.ppi, d.note) {
                (Some(ppi), _) => line.push_str(&format!(
                    "\n          scale   {} ({ppi:.0} ppi at desk distance, exactly {:.2})",
                    d.scale, d.exact
                )),
                (None, note) => line.push_str(&format!("\n          scale   1 ({})", note.unwrap_or("no size"))),
            }
        }
        match &o.current {
            Some(c) => line.push_str(&format!("\n          current {} at +{}+{}", c.mode, c.x, c.y)),
            None => line.push_str("\n          current off"),
        }
        if o.mm != (0, 0) {
            line.push_str(&format!("\n          X says  {}x{} mm", o.mm.0, o.mm.1));
        }
        println!("{line}");
    }
    println!("{}", key_line(setup));
}

/// "profile key <hex> (N monitors with EDID)", plus a warning line for
/// identical twins.
pub fn key_line(setup: &Setup) -> String {
    let edids = setup.edids();
    let n = edids.len();
    let mut s = format!(
        "profile key {} ({n} monitor{} with EDID)",
        profile_key(&edids),
        if n == 1 { "" } else { "s" }
    );
    let dups = duplicate_count(&edids);
    if dups > 0 {
        s.push_str(&format!(
            "\nwarning: {dups} monitor(s) have an EDID identical to another; \
             the key cannot tell which port each is in"
        ));
    }
    s
}

fn describe(e: &Edid, len: usize) -> String {
    let mut s = format!("  {} {:#06x}", e.manufacturer, e.product_code);
    if let Some(n) = &e.name {
        s.push_str(&format!(" \"{n}\""));
    }
    match (e.serial_number, &e.serial_text) {
        (0, None) => s.push_str("  no serial"),
        (0, Some(t)) => s.push_str(&format!("  serial {t}")),
        (n, None) => s.push_str(&format!("  serial {n}")),
        (n, Some(t)) => s.push_str(&format!("  serial {n} / {t}")),
    }
    match e.physical_size_mm() {
        Some((w, h)) => s.push_str(&format!("\n          size    {w}x{h} mm")),
        None => s.push_str("\n          size    unknown (EDID gives 0)"),
    }
    if e.size_cm != (0, 0) {
        s.push_str(&format!(" (base block {}x{} cm)", e.size_cm.0, e.size_cm.1));
    }
    let made = match e.made {
        MadeIn::Week { week: 0, year } => format!("{year}"),
        MadeIn::Week { week, year } => format!("week {week} {year}"),
        MadeIn::ModelYear(y) => format!("model year {y}"),
    };
    s.push_str(&format!(
        "\n          EDID    {len} B, v{}.{}, {made}",
        e.version.0, e.version.1
    ));
    if let Some(t) = e.preferred_timing {
        match t.refresh_hz() {
            Some(hz) => s.push_str(&format!(", preferred {}x{}@{hz:.2}", t.h_active, t.v_active)),
            None => s.push_str(&format!(", preferred {}x{}", t.h_active, t.v_active)),
        }
    }
    s
}

/// `vergd scale [status]`: each monitor's scale, and what vergd would do.
fn scale_status() -> Result<(), Box<dyn std::error::Error>> {
    let setup = x11::read_setup()?;
    let st = store::load()?;
    let cfg = &st.scaling;
    println!(
        "scaling {}, Mode {:?}, preference {}%",
        if cfg.enabled { "ON" } else { "off (vergd scale on)" },
        cfg.mode,
        cfg.preference
    );
    for m in scale::monitor_scales(&setup, &cfg.clone()) {
        println!("  {:<9} {:<16} scale {:<5} ({})", m.connector, m.name, m.scale, m.why);
    }
    // What it would be when on, for the off case.
    let mut on = cfg.clone();
    on.enabled = true;
    if !cfg.enabled {
        for m in scale::monitor_scales(&setup, &on) {
            println!("  {:<9} {:<16} would be {} when on ({})", m.connector, m.name, m.scale, m.why);
        }
    }
    match scale::target(&setup, cfg) {
        Ok(t) => {
            match t.mode {
                vergence_profile::arrangement::ScaleMode::A => println!("render scale {}", t.render),
                vergence_profile::arrangement::ScaleMode::B => println!(
                    "GTK, Electron and Java scale {}; Qt and Avalonia per monitor: {}",
                    t.render,
                    scale::per_monitor_text(&t.per_monitor)
                ),
            }
            if t.changes {
                println!("vergd will change the arrangement to:");
                for o in &t.want {
                    println!("  {}", o.describe());
                }
            } else {
                println!("the arrangement already matches");
            }
        }
        Err(e) => println!("cannot scale this arrangement: {e}"),
    }
    Ok(())
}

/// `vergd scale on|off`: switch scaling in profiles.toml; the running vergd
/// applies it within 2 s.
fn scale_set(on: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut st = store::load()?;
    st.scaling.enabled = on;
    store::save(&st)?;
    println!("scaling {} in ~/.config/vergence/profiles.toml", if on { "ON" } else { "off" });
    if on && store::ensure_xprofile_line()? {
        println!("added a line to ~/.xprofile so the next login reads ~/.config/vergence/session.env");
    }
    println!("the running vergd applies it within 2 s (see its log)");
    Ok(())
}

/// `vergd scale mode a|b`.
fn scale_mode(m: &str) -> Result<(), Box<dyn std::error::Error>> {
    use vergence_profile::arrangement::ScaleMode;
    let mut st = store::load()?;
    st.scaling.mode = if m == "b" { ScaleMode::B } else { ScaleMode::A };
    store::save(&st)?;
    println!(
        "Mode {:?} in ~/.config/vergence/profiles.toml (scaling is {}); the running vergd applies it within 2 s",
        st.scaling.mode,
        if st.scaling.enabled { "on" } else { "off" }
    );
    Ok(())
}
