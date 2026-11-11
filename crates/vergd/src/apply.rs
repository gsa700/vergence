//! Putting a remembered arrangement back (milestone 2).
//!
//! A plan is worked out and checked against what the server offers before
//! anything is touched: every mode must exist on its output, every enabled
//! output needs a CRTC, and the screen must fit the server's size range. If
//! any check fails, nothing is changed.
//!
//! The plan is then applied in one server grab, so clients see one change:
//! turn off CRTCs that are no longer used or that would not fit the new
//! screen, set the screen size, set every CRTC, set the primary. The result
//! is read back; if it is not what was asked for, the previous arrangement
//! is put back.

use std::collections::HashSet;

use x11rb::connection::Connection as _;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use vergence_profile::arrangement::{same_arrangement, OutputSetting, FIXED_ONE, REFRESH_TOLERANCE_MHZ};

use crate::x11::{self, Output, Setup};

/// One CRTC to set.
#[derive(Debug, Clone)]
pub struct CrtcSet {
    pub crtc: u32,
    pub output: u32,
    pub output_name: String,
    pub mode: u32,
    pub mode_text: String,
    pub x: i16,
    pub y: i16,
    pub rotation: u16,
    /// RandR scale factor, 16.16 fixed point (milestone 4).
    pub factor: u32,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub disable: Vec<u32>,
    pub screen: (u16, u16),
    pub set: Vec<CrtcSet>,
    pub primary: u32,
}

impl Plan {
    pub fn describe(&self, setup: &Setup) -> Vec<String> {
        let mut out = Vec::new();
        for c in &self.disable {
            let what = setup
                .outputs
                .iter()
                .find(|o| o.current.as_ref().map(|cur| cur.crtc) == Some(*c))
                .map(|o| o.name.as_str())
                .unwrap_or("unused");
            out.push(format!("turn off CRTC {c:#x} ({what})"));
        }
        if self.screen != setup.screen {
            out.push(format!(
                "screen {}x{} -> {}x{}",
                setup.screen.0, setup.screen.1, self.screen.0, self.screen.1
            ));
        }
        for s in &self.set {
            let rot = if s.rotation != 0 { format!(" rotated {}", s.rotation) } else { String::new() };
            let scaled = if s.factor != FIXED_ONE {
                format!(" scaled x{:.3}", s.factor as f64 / FIXED_ONE as f64)
            } else {
                String::new()
            };
            out.push(format!(
                "{:<9} {} at +{}+{}{rot}{scaled} on CRTC {:#x}",
                s.output_name, s.mode_text, s.x, s.y, s.crtc
            ));
        }
        let primary = setup.outputs.iter().find(|o| o.id == self.primary).map(|o| o.name.as_str());
        out.push(format!("primary {}", primary.unwrap_or("none")));
        out
    }
}

/// Match each wanted setting to a connected output: by monitor ID, and for
/// identical twins by connector name.
fn match_outputs<'s, 'w>(
    setup: &'s Setup,
    want: &'w [OutputSetting],
) -> Result<Vec<(&'s Output, &'w OutputSetting)>, String> {
    let connected: Vec<&Output> = setup.outputs.iter().filter(|o| o.connected).collect();
    if connected.len() != want.len() {
        return Err(format!(
            "profile has {} monitors, {} are connected",
            want.len(),
            connected.len()
        ));
    }
    let mut used = vec![false; connected.len()];
    let mut pairs = Vec::new();
    for w in want {
        let pick = connected
            .iter()
            .enumerate()
            .filter(|(i, o)| !used[*i] && o.monitor() == w.monitor)
            .min_by_key(|(_, o)| o.name != w.connector)
            .map(|(i, _)| i);
        let Some(i) = pick else {
            return Err(format!("monitor {} ({}) is not connected", w.monitor, w.name));
        };
        used[i] = true;
        pairs.push((connected[i], w));
    }
    Ok(pairs)
}

/// Work out how to reach `want` from `setup`. Changes nothing.
pub fn plan(setup: &Setup, want: &[OutputSetting]) -> Result<Plan, String> {
    let pairs = match_outputs(setup, want)?;
    if !want.iter().any(|w| w.enabled) {
        return Err("profile turns every monitor off; refusing".into());
    }

    // Modes first: every enabled output must offer the wanted mode.
    let mut chosen = Vec::new(); // (output, setting, mode)
    for (o, w) in &pairs {
        if !w.enabled {
            continue;
        }
        let m = o
            .modes
            .iter()
            .filter(|m| m.width == w.width && m.height == w.height)
            .filter(|m| m.refresh_mhz.abs_diff(w.refresh_mhz) <= REFRESH_TOLERANCE_MHZ)
            .min_by_key(|m| m.refresh_mhz.abs_diff(w.refresh_mhz));
        let Some(m) = m else {
            return Err(format!(
                "{} offers no {}x{}@{:.2} mode",
                o.name,
                w.width,
                w.height,
                w.refresh_mhz as f64 / 1000.0
            ));
        };
        if ![0, 90, 180, 270].contains(&w.rotation) {
            return Err(format!("{}: rotation {} is not 0, 90, 180 or 270", o.name, w.rotation));
        }
        chosen.push((*o, *w, m.clone()));
    }

    // CRTCs: an output that is lit keeps its CRTC; the others take a free one.
    let mut taken: HashSet<u32> = HashSet::new();
    let mut crtc_of: Vec<Option<u32>> = vec![None; chosen.len()];
    for (i, (o, _, _)) in chosen.iter().enumerate() {
        if let Some(c) = &o.current {
            crtc_of[i] = Some(c.crtc);
            taken.insert(c.crtc);
        }
    }
    for (i, (o, _, _)) in chosen.iter().enumerate() {
        if crtc_of[i].is_some() {
            continue;
        }
        let free = o.possible_crtcs.iter().copied().find(|c| !taken.contains(c));
        let Some(c) = free else {
            return Err(format!("no free CRTC for {}", o.name));
        };
        crtc_of[i] = Some(c);
        taken.insert(c);
    }

    // Screen size: the extent of every enabled output.
    let mut sw: i32 = 0;
    let mut sh: i32 = 0;
    for (_, w, _) in &chosen {
        if w.x < 0 || w.y < 0 {
            return Err(format!("{} has a negative position", w.name));
        }
        let (ew, eh) = w.extent();
        sw = sw.max(w.x as i32 + ew as i32);
        sh = sh.max(w.y as i32 + eh as i32);
    }
    let (min_w, min_h, max_w, max_h) = setup.screen_range;
    let sw = sw.max(min_w as i32);
    let sh = sh.max(min_h as i32);
    if sw > max_w as i32 || sh > max_h as i32 {
        return Err(format!(
            "screen {sw}x{sh} exceeds this server's maximum {max_w}x{max_h}"
        ));
    }
    let screen = (sw as u16, sh as u16);

    // Turn off: every lit CRTC not in the plan, and planned CRTCs whose
    // current picture would not fit the new screen while it is resized.
    let mut disable = Vec::new();
    for c in &setup.crtcs {
        if c.mode == 0 {
            continue;
        }
        let planned = crtc_of.contains(&Some(c.id));
        let fits = c.x as i32 + c.width as i32 <= sw && c.y as i32 + c.height as i32 <= sh;
        if !planned || !fits {
            disable.push(c.id);
        }
    }

    let set = chosen
        .iter()
        .zip(&crtc_of)
        .map(|((o, w, m), c)| CrtcSet {
            crtc: c.expect("every chosen output has a CRTC"),
            output: o.id,
            output_name: o.name.clone(),
            mode: m.id,
            mode_text: m.to_string(),
            x: w.x,
            y: w.y,
            rotation: w.rotation,
            factor: w.factor,
        })
        .collect();

    let primary = chosen.iter().find(|(_, w, _)| w.primary).map(|(o, _, _)| o.id).unwrap_or(0);

    Ok(Plan { disable, screen, set, primary })
}

fn rotation_bits(deg: u16) -> randr::Rotation {
    match deg {
        90 => randr::Rotation::ROTATE90,
        180 => randr::Rotation::ROTATE180,
        270 => randr::Rotation::ROTATE270,
        _ => randr::Rotation::ROTATE0,
    }
}

/// Carry out a plan in one server grab.
fn execute(conn: &RustConnection, setup: &Setup, p: &Plan) -> Result<(), Box<dyn std::error::Error>> {
    let ts = setup.config_timestamp;
    conn.grab_server()?;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        for &c in &p.disable {
            let r = conn
                .randr_set_crtc_config(c, x11rb::CURRENT_TIME, ts, 0, 0, 0, randr::Rotation::ROTATE0, &[])?
                .reply()?;
            if r.status != randr::SetConfig::SUCCESS {
                return Err(format!("turning off CRTC {c:#x}: {:?}", r.status).into());
            }
        }
        if p.screen != setup.screen {
            // Millimeters at 96 dpi: only used by clients that ignore RandR.
            let mm = |px: u16| (px as u32 * 254 + 480) / 960;
            conn.randr_set_screen_size(setup.root, p.screen.0, p.screen.1, mm(p.screen.0), mm(p.screen.1))?
                .check()?;
        }
        for s in &p.set {
            // The transform is pending until the CRTC is next configured.
            let f = s.factor as i32;
            let t = x11rb::protocol::render::Transform {
                matrix11: f,
                matrix12: 0,
                matrix13: 0,
                matrix21: 0,
                matrix22: f,
                matrix23: 0,
                matrix31: 0,
                matrix32: 0,
                matrix33: FIXED_ONE as i32,
            };
            let filter: &[u8] = if s.factor == FIXED_ONE { b"nearest" } else { b"bilinear" };
            conn.randr_set_crtc_transform(s.crtc, t, filter, &[])?.check()?;
            let r = conn
                .randr_set_crtc_config(
                    s.crtc,
                    x11rb::CURRENT_TIME,
                    ts,
                    s.x,
                    s.y,
                    s.mode,
                    rotation_bits(s.rotation),
                    &[s.output],
                )?
                .reply()?;
            if r.status != randr::SetConfig::SUCCESS {
                return Err(format!("setting {}: {:?}", s.output_name, r.status).into());
            }
        }
        conn.randr_set_output_primary(setup.root, p.primary)?.check()?;
        Ok(())
    })();
    conn.ungrab_server()?;
    conn.flush()?;
    result
}

/// Lit CRTCs that drive no connected monitor. X leaves the CRTC of an
/// unplugged monitor lit (and the screen wide) until someone turns it off.
pub fn stray_crtcs(setup: &Setup) -> Vec<u32> {
    setup
        .crtcs
        .iter()
        .filter(|c| c.mode != 0)
        .filter(|c| {
            !setup
                .outputs
                .iter()
                .any(|o| o.connected && o.current.as_ref().map(|cur| cur.crtc) == Some(c.id))
        })
        .map(|c| c.id)
        .collect()
}

/// What happened.
pub enum Outcome {
    /// Already as wanted; nothing done.
    AlreadyThere,
    /// Applied and read back correctly.
    Applied(Vec<String>),
    /// Refused before touching anything.
    Refused(String),
    /// Applied, but the result was wrong or an error happened; the previous
    /// arrangement was put back (or that failed too, see the message).
    Reverted(String),
}

/// Bring the desktop to `want`. With `dry_run`, only plan.
pub fn apply(want: &[OutputSetting], dry_run: bool) -> Result<Outcome, Box<dyn std::error::Error>> {
    let (conn, root) = x11::connect()?;
    let before = x11::read_setup_on(&conn, root)?;
    let p = match plan(&before, want) {
        Ok(p) => p,
        Err(why) => return Ok(Outcome::Refused(why)),
    };
    // The screen size counts too: X leaves the screen large when a monitor
    // goes, so the pointer could wander off the monitors.
    if same_arrangement(&before.arrangement(), want) && stray_crtcs(&before).is_empty() && p.screen == before.screen {
        return Ok(Outcome::AlreadyThere);
    }
    let steps = p.describe(&before);
    if dry_run {
        return Ok(Outcome::Applied(steps));
    }

    let err = execute(&conn, &before, &p).err().map(|e| e.to_string());
    let after = x11::read_setup_on(&conn, root)?;
    if err.is_none() && same_arrangement(&after.arrangement(), want) {
        return Ok(Outcome::Applied(steps));
    }

    // Put the previous arrangement back.
    let why = err.unwrap_or_else(|| "the result differs from the profile".into());
    let old = before.arrangement();
    let back = plan(&after, &old).map_err(|e| e.to_string()).and_then(|bp| {
        execute(&conn, &after, &bp).map_err(|e| e.to_string())
    });
    match back {
        Ok(()) => Ok(Outcome::Reverted(format!("{why}; previous arrangement restored"))),
        Err(e) => Ok(Outcome::Reverted(format!("{why}; restoring the previous arrangement ALSO failed: {e}"))),
    }
}
