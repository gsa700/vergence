//! Reading the monitor setup from the X server over RandR 1.5 (x11rb).
//!
//! This module is the only place that talks to X, so that recorded setups
//! can stand in for a live server later (milestone 1, test automation (5)).

use std::fmt;

use x11rb::connection::Connection as _;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};

/// A video mode as RandR reports it.
#[derive(Debug, Clone)]
pub struct Mode {
    pub width: u16,
    pub height: u16,
    pub refresh_hz: Option<f64>,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.refresh_hz {
            Some(hz) => write!(f, "{}x{}@{hz:.2}", self.width, self.height),
            None => write!(f, "{}x{}", self.width, self.height),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Current {
    pub mode: Mode,
    pub x: i16,
    pub y: i16,
}

#[derive(Debug, Clone)]
pub struct Output {
    /// Connector name as X names it (DP-1, HDMI-2...).
    pub name: String,
    pub connected: bool,
    /// Raw EDID from the RandR "EDID" output property.
    pub edid: Option<Vec<u8>>,
    /// The output's first preferred mode.
    pub native: Option<Mode>,
    /// Mode and position if the output is lit.
    pub current: Option<Current>,
    /// Physical size as X reports it (from the EDID, as X read it).
    pub mm: (u32, u32),
}

impl Output {
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
            Some(c) => format!("connected {who} {} at +{}+{}", c.mode, c.x, c.y),
            None => format!("connected {who} off"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Setup {
    pub outputs: Vec<Output>,
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
}

fn mode_of(info: &randr::ModeInfo) -> Mode {
    let total = info.htotal as u64 * info.vtotal as u64;
    let mut hz = if total == 0 { None } else { Some(info.dot_clock as f64 / total as f64) };
    if info.mode_flags.contains(randr::ModeFlag::INTERLACE) {
        hz = hz.map(|h| h * 2.0);
    }
    if info.mode_flags.contains(randr::ModeFlag::DOUBLE_SCAN) {
        hz = hz.map(|h| h / 2.0);
    }
    Mode { width: info.width, height: info.height, refresh_hz: hz }
}

/// Read every output of the default screen. Uses
/// RRGetScreenResourcesCurrent, which does not make the server re-probe.
pub fn read_setup() -> Result<Setup, Box<dyn std::error::Error>> {
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

    let edid_atom = conn.intern_atom(true, b"EDID")?.reply()?.atom;
    let res = conn.randr_get_screen_resources_current(root)?.reply()?;
    let mode = |id: u32| res.modes.iter().find(|m| m.id == id).map(mode_of);

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

        let native = if info.num_preferred > 0 {
            info.modes.first().and_then(|&m| mode(m))
        } else {
            None
        };

        let current = if info.crtc != 0 {
            let c = conn.randr_get_crtc_info(info.crtc, res.config_timestamp)?.reply()?;
            if c.mode != 0 {
                mode(c.mode).map(|m| Current { mode: m, x: c.x, y: c.y })
            } else {
                None
            }
        } else {
            None
        };

        outputs.push(Output {
            name,
            connected,
            edid,
            native,
            current,
            mm: (info.mm_width, info.mm_height),
        });
    }
    Ok(Setup { outputs })
}
