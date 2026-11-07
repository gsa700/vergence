//! vergd, the Vergence daemon. Milestone 1: observe only.
//!
//!   vergd probe   print each output and the profile key, then exit
//!   vergd watch   the same, then one block per change (plug, mode, wake)
//!
//! No network socket, no files written (whitepaper §7.3, phase 1). On FreeBSD
//! `watch` also reads devd's local pipe (see watch.rs).

mod watch;
mod x11;

use std::process::ExitCode;

use vergence_edid::{Edid, MadeIn};
use vergence_profile::{duplicate_count, profile_key};

use x11::Setup;

fn usage() -> ExitCode {
    eprintln!("usage: vergd probe | vergd watch");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match (args.first().map(String::as_str), args.len()) {
        (Some("probe"), 1) => x11::read_setup().map(|s| print_setup(&s)),
        (Some("watch"), 1) => watch::watch(),
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
