//! Reading the monitor setup from the X server over RandR 1.5 (x11rb).
//!
//! This module and apply.rs are the only places that talk to X, so that
//! recorded setups can stand in for a live server later (milestone 1, test
//! automation (5)).

use vergence_profile::scaling::{self, Derived, Distance};
use std::fmt;

use x11rb::connection::Connection as _;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

use vergence_profile::arrangement::OutputSetting;
use vergence_profile::monitor_id;

/// A video mode as RandR reports it.
#[derive(Debug, Clone)]
pub struct Mode {
    pub id: u32,
    pub width: u16,
    pub height: u16,
    /// Refresh in millihertz, or 0 if the timings are empty.
    pub refresh_mhz: u32,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.refresh_mhz == 0 {
            write!(f, "{}x{}", self.width, self.height)
        } else {
            write!(f, "{}x{}@{:.2}", self.width, self.height, self.refresh_mhz as f64 / 1000.0)
        }
    }
}

#[derive(Debug, Clone)]
pub struct Current {
    pub crtc: u32,
    pub mode: Mode,
    pub x: i16,
    pub y: i16,
    /// Degrees: 0, 90, 180 or 270.
    pub rotation: u16,
}

#[derive(Debug, Clone)]
pub struct Output {
    pub id: u32,
    /// Connector name as X names it (DP-1, HDMI-2...).
    pub name: String,
    pub connected: bool,
    /// Raw EDID from the RandR "EDID" output property.
    pub edid: Option<Vec<u8>>,
    /// The output's first preferred mode.
    pub native: Option<Mode>,
    /// Every mode the output offers.
    pub modes: Vec<Mode>,
    /// CRTCs that can drive this output.
    pub possible_crtcs: Vec<u32>,
    /// Mode and position if the output is lit.
    pub current: Option<Current>,
    /// Physical size as X reports it (from the EDID, as X read it).
    pub mm: (u32, u32),
}

/// A CRTC and what it drives now.
#[derive(Debug, Clone)]
pub struct Crtc {
    pub id: u32,
    pub mode: u32,
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone)]
pub struct Setup {
    pub root: u32,
    pub config_timestamp: u32,
    pub outputs: Vec<Output>,
    pub crtcs: Vec<Crtc>,
    pub primary: u32,
    pub screen: (u16, u16),
    /// (min width, min height, max width, max height).
    pub screen_range: (u16, u16, u16, u16),
}

impl Output {
    /// The monitor ID a profile uses for this output.
    pub fn monitor(&self) -> String {
        match &self.edid {
            Some(e) => monitor_id(e),
            None => format!("port:{}", self.name),
        }
    }

    /// The monitor's name from its EDID, or "".
    pub fn monitor_name(&self) -> String {
        self.edid
            .as_deref()
            .and_then(|e| vergence_edid::Edid::parse(e).ok())
            .and_then(|e| e.name)
            .unwrap_or_default()
    }

    /// One line describing what matters for change detection: connection,
    /// which monitor, and the current mode and position.
    pub fn summary(&self) -> String {
        if !self.connected {
            return "disconnected".into();
        }
        let who = match self.edid.as_deref().map(vergence_edid::Edid::parse) {
            None => "no EDID".to_string(),
            Some(Err(e)) => format!("bad EDID ({e})"),
            Some(Ok(e)) => {
                let name = e.name.as_deref().unwrap_or("unnamed");
                format!("\"{name}\" ({} {:#06x})", e.manufacturer, e.product_code)
            }
        };
        match &self.current {
            Some(c) => {
                let rot = if c.rotation != 0 { format!(" rotated {}", c.rotation) } else { String::new() };
                format!("connected {who} {} at +{}+{}{rot}", c.mode, c.x, c.y)
            }
            None => format!("connected {who} off"),
        }
    }
}

impl Output {
    /// This monitor's derived scale (milestone 4), from its EDID's physical
    /// size and native resolution; None without a native mode.
    pub fn derived_scale(&self, distance: Distance, preference: f64) -> Option<Derived> {
        let n = self.native.as_ref()?;
        let mm = self
            .edid
            .as_deref()
            .and_then(|r| vergence_edid::Edid::parse(r).ok())
            .and_then(|e| e.physical_size_mm())
            .map(|(w, h)| (w as u32, h as u32));
        Some(scaling::derive((n.width as u32, n.height as u32), mm, distance, preference))
    }
}

impl Setup {
    /// The raw EDIDs of the connected outputs.
    pub fn edids(&self) -> Vec<&[u8]> {
        self.outputs
            .iter()
            .filter(|o| o.connected)
            .filter_map(|o| o.edid.as_deref())
            .collect()
    }

    pub fn key(&self) -> String {
        vergence_profile::profile_key(&self.edids())
    }

    /// The current arrangement of the connected outputs, as a profile stores it.
    pub fn arrangement(&self) -> Vec<OutputSetting> {
        self.outputs
            .iter()
            .filter(|o| o.connected)
            .map(|o| {
                let mut s = OutputSetting {
                    monitor: o.monitor(),
                    name: o.monitor_name(),
                    connector: o.name.clone(),
                    enabled: false,
                    width: 0,
                    height: 0,
                    refresh_mhz: 0,
                    x: 0,
                    y: 0,
                    rotation: 0,
                    primary: false,
                };
                if let Some(c) = &o.current {
                    s.enabled = true;
                    s.width = c.mode.width;
                    s.height = c.mode.height;
                    s.refresh_mhz = c.mode.refresh_mhz;
                    s.x = c.x;
                    s.y = c.y;
                    s.rotation = c.rotation;
                    s.primary = o.id == self.primary;
                }
                s
            })
            .collect()
    }
}

fn mode_of(info: &randr::ModeInfo) -> Mode {
    let total = info.htotal as u64 * info.vtotal as u64;
    let mut mhz = if total == 0 {
        0
    } else {
        ((info.dot_clock as u64 * 1000 + total / 2) / total) as u32
    };
    if info.mode_flags.contains(randr::ModeFlag::INTERLACE) {
        mhz *= 2;
    }
    if info.mode_flags.contains(randr::ModeFlag::DOUBLE_SCAN) {
        mhz /= 2;
    }
    Mode { id: info.id, width: info.width, height: info.height, refresh_mhz: mhz }
}

pub fn rotation_degrees(r: randr::Rotation) -> u16 {
    if r.contains(randr::Rotation::ROTATE90) {
        90
    } else if r.contains(randr::Rotation::ROTATE180) {
        180
    } else if r.contains(randr::Rotation::ROTATE270) {
        270
    } else {
        0
    }
}

/// Connect to the X server named by DISPLAY and check RandR 1.5.
pub fn connect() -> Result<(RustConnection, u32), Box<dyn std::error::Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;
    let ver = conn.randr_query_version(1, 5)?.reply()?;
    if (ver.major_version, ver.minor_version) < (1, 5) {
        return Err(format!(
            "X server has RandR {}.{}, vergd needs 1.5",
            ver.major_version, ver.minor_version
        )
        .into());
    }
    Ok((conn, root))
}

/// Read every output of the default screen.
pub fn read_setup() -> Result<Setup, Box<dyn std::error::Error>> {
    let (conn, root) = connect()?;
    read_setup_on(&conn, root)
}

/// Read the setup over an existing connection.
///
/// Uses RRGetScreenResourcesCurrent, which does not make the server
/// re-probe. Exception: if a connected output has no EDID property, the
/// server is asked to re-probe once (RRGetScreenResources) and the setup is
/// read again. Found on Hubble 2026-11-06: Xorg (modesetting, FreeBSD) reads
/// the EDID at start-up but does not publish the RandR EDID property until
/// the first re-probe.
pub fn read_setup_on(conn: &RustConnection, root: u32) -> Result<Setup, Box<dyn std::error::Error>> {
    let setup = read_outputs(conn, root)?;
    if setup.outputs.iter().any(|o| o.connected && o.edid.is_none()) {
        conn.randr_get_screen_resources(root)?.reply()?;
        return read_outputs(conn, root);
    }
    Ok(setup)
}

fn read_outputs(conn: &RustConnection, root: u32) -> Result<Setup, Box<dyn std::error::Error>> {
    // Looked up each time: the atom only exists once a server has set an EDID.
    let edid_atom = conn.intern_atom(true, b"EDID")?.reply()?.atom;
    let res = conn.randr_get_screen_resources_current(root)?.reply()?;
    let mode = |id: u32| res.modes.iter().find(|m| m.id == id).map(mode_of);

    let mut crtcs = Vec::new();
    let mut crtc_rot = std::collections::HashMap::new();
    for &c in &res.crtcs {
        let ci = conn.randr_get_crtc_info(c, res.config_timestamp)?.reply()?;
        crtc_rot.insert(c, rotation_degrees(ci.rotation));
        crtcs.push(Crtc { id: c, mode: ci.mode, x: ci.x, y: ci.y, width: ci.width, height: ci.height });
    }

    let mut outputs = Vec::new();
    for &out in &res.outputs {
        let info = conn.randr_get_output_info(out, res.config_timestamp)?.reply()?;
        let name = String::from_utf8_lossy(&info.name).into_owned();
        let connected = info.connection == randr::Connection::CONNECTED;

        let edid = if connected && edid_atom != 0 {
            let any: u32 = AtomEnum::ANY.into();
            // 256 longs = 1024 bytes: base block plus up to 7 extensions.
            let p = conn
                .randr_get_output_property(out, edid_atom, any, 0, 256, false, false)?
                .reply()?;
            if p.format == 8 && !p.data.is_empty() { Some(p.data) } else { None }
        } else {
            None
        };

        let modes: Vec<Mode> = info.modes.iter().filter_map(|&m| mode(m)).collect();
        let native = if info.num_preferred > 0 { modes.first().cloned() } else { None };

        let current = crtcs
            .iter()
            .find(|c| c.id == info.crtc && c.mode != 0)
            .and_then(|c| {
                mode(c.mode).map(|m| Current {
                    crtc: c.id,
                    mode: m,
                    x: c.x,
                    y: c.y,
                    rotation: crtc_rot.get(&c.id).copied().unwrap_or(0),
                })
            });

        outputs.push(Output {
            id: out,
            name,
            connected,
            edid,
            native,
            modes,
            possible_crtcs: info.crtcs.clone(),
            current,
            mm: (info.mm_width, info.mm_height),
        });
    }

    let primary = conn.randr_get_output_primary(root)?.reply()?.output;
    let geo = conn.get_geometry(root)?.reply()?;
    let range = conn.randr_get_screen_size_range(root)?.reply()?;
    Ok(Setup {
        root,
        config_timestamp: res.config_timestamp,
        outputs,
        crtcs,
        primary,
        screen: (geo.width, geo.height),
        screen_range: (range.min_width, range.min_height, range.max_width, range.max_height),
    })
}
