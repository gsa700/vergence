//! Offline sweep: parse every EDID in the database and count what we find.

use std::collections::BTreeMap;
use std::panic;
use std::path::{Path, PathBuf};

use vergence_edid::{Edid, EdidError};

use crate::db;

/// Up to this many example paths are kept per category.
const EXAMPLES: usize = 3;

#[derive(Default)]
struct Tally {
    counts: BTreeMap<&'static str, usize>,
    examples: BTreeMap<&'static str, Vec<PathBuf>>,
}

impl Tally {
    fn add(&mut self, what: &'static str, path: &Path) {
        *self.counts.entry(what).or_default() += 1;
        let ex = self.examples.entry(what).or_default();
        if ex.len() < EXAMPLES {
            ex.push(path.to_path_buf());
        }
    }
}

pub fn run(root: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    let files = db::files(root)?;
    let mut t = Tally::default();

    for path in &files {
        let raw = match db::read_edid(path) {
            Ok(Some(r)) => r,
            Ok(None) => {
                t.add("file: no hex block", path);
                continue;
            }
            Err(_) => {
                t.add("file: unreadable", path);
                continue;
            }
        };
        let parsed = panic::catch_unwind(|| Edid::parse(&raw));
        let e = match parsed {
            Err(_) => {
                t.add("PANIC in parser", path);
                continue;
            }
            Ok(Err(EdidError::TooShort(_))) => {
                t.add("rejected: too short", path);
                continue;
            }
            Ok(Err(EdidError::BadHeader)) => {
                t.add("rejected: bad header", path);
                continue;
            }
            Ok(Err(EdidError::BadChecksum { .. })) => {
                t.add("rejected: bad checksum", path);
                continue;
            }
            Ok(Ok(e)) => e,
        };
        t.add("parsed", path);
        let declared = 128 * (1 + e.extensions as usize);
        if raw.len() > declared {
            if raw[declared..].chunks(128).all(|c| c == &raw[..c.len()]) {
                t.add("extra bytes: copy of the base block (wrap-around read)", path);
            } else {
                t.add("extra bytes: other data past the declared blocks", path);
            }
        } else if raw.len() < declared {
            t.add("short: fewer blocks than the base block declares", path);
        }
        if e.manufacturer.contains('?') {
            t.add("manufacturer ID invalid", path);
        }
        if e.name.is_none() {
            t.add("no monitor name", path);
        }
        match (e.serial_number, &e.serial_text) {
            (0, None) => t.add("no serial at all", path),
            (0, Some(_)) => t.add("serial: text only", path),
            (_, None) => t.add("serial: number only", path),
            _ => {}
        }
        match e.preferred_timing {
            None => t.add("no preferred timing", path),
            Some(pt) => {
                if pt.refresh_hz().is_none() {
                    t.add("preferred timing: zero totals", path);
                }
                if pt.width_mm == 0 || pt.height_mm == 0 {
                    t.add("preferred timing: no mm size", path);
                }
            }
        }
        match e.physical_size_mm() {
            None => t.add("size: unknown (all zero)", path),
            Some((w, h)) => {
                // Implausible: smaller than a phone, bigger than 3 m, or a
                // shape no monitor has.
                let ratio = w.max(h) as f64 / w.min(h).max(1) as f64;
                if w < 50 || h < 30 || w > 3000 || h > 3000 || ratio > 4.0 {
                    t.add("size: implausible", path);
                }
                // mm (timing) and cm (base block) disagree by more than 2 cm.
                let (cw, ch) = e.size_cm;
                if cw > 0 && ch > 0 {
                    let dw = (w as i32 - cw as i32 * 10).abs();
                    let dh = (h as i32 - ch as i32 * 10).abs();
                    if dw > 20 || dh > 20 {
                        t.add("size: mm and cm disagree by >2 cm", path);
                    }
                }
            }
        }
    }

    println!("{} files under {}", files.len(), root.display());
    let width = t.counts.keys().map(|k| k.len()).max().unwrap_or(0);
    for (what, n) in &t.counts {
        let pct = 100.0 * *n as f64 / files.len().max(1) as f64;
        println!("  {what:<width$}  {n:>7}  {pct:5.1}%");
    }
    println!("examples:");
    for (what, ex) in &t.examples {
        if *what == "parsed" {
            continue;
        }
        for p in ex {
            let rel = p.strip_prefix(root).unwrap_or(p);
            println!("  {what}: {}", rel.display());
        }
    }
    let panics = t.counts.get("PANIC in parser").copied().unwrap_or(0);
    Ok(panics == 0)
}
