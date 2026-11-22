//! What a profile remembers about a monitor setup (milestone 2): for each
//! monitor, whether it is on, its mode, position, rotation and whether it is
//! the primary. Monitors are named by [`crate::monitor_id`], not by port.
//!
//! The store is one TOML file, hand-editable:
//!
//! ```toml
//! [[profile]]
//! key = "8ec9c14b…"
//! updated = "2026-11-06 19:10:00"
//!
//! [[profile.output]]
//! monitor = "5a1c…"
//! name = "ASUS VS229"
//! connector = "HDMI-2"
//! enabled = true
//! width = 1920
//! height = 1080
//! refresh_mhz = 60000
//! x = 0
//! y = 0
//! rotation = 0
//! primary = true
//! ```

use serde::{Deserialize, Serialize};

/// Refresh rates within this many millihertz count as the same mode.
pub const REFRESH_TOLERANCE_MHZ: u32 = 500;

/// 1.0 in RandR's 16.16 fixed point.
pub const FIXED_ONE: u32 = 65536;

fn fixed_one() -> u32 {
    FIXED_ONE
}

fn is_fixed_one(f: &u32) -> bool {
    *f == FIXED_ONE
}

fn hundred() -> u32 {
    100
}

/// `len` pixels scaled by a 16.16 factor, rounded up (as the X server sizes
/// a transformed CRTC).
pub fn scaled_len(len: u16, factor: u32) -> u16 {
    ((len as u64 * factor as u64).div_ceil(FIXED_ONE as u64)).min(u16::MAX as u64) as u16
}

/// One monitor's place in a setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSetting {
    /// [`crate::monitor_id`] of the monitor's EDID, or "`port:<connector>`"
    /// for a monitor without an EDID.
    pub monitor: String,
    /// The monitor's name, for people reading the file. Not used to match.
    #[serde(default)]
    pub name: String,
    /// Where the monitor was last seen. Used only to tell identical twins
    /// apart.
    #[serde(default)]
    pub connector: String,
    pub enabled: bool,
    #[serde(default)]
    pub width: u16,
    #[serde(default)]
    pub height: u16,
    /// Refresh rate in millihertz (60000 = 60.000 Hz).
    #[serde(default)]
    pub refresh_mhz: u32,
    #[serde(default)]
    pub x: i16,
    #[serde(default)]
    pub y: i16,
    /// Degrees: 0, 90, 180 or 270.
    #[serde(default)]
    pub rotation: u16,
    #[serde(default)]
    pub primary: bool,
    /// The RandR scale factor of the picture (milestone 4, Mode A), in
    /// 16.16 fixed point: 65536 = 1 (not scaled), 131072 = 2 (drawn at twice
    /// the panel's pixels and shrunk onto it). Left out of the file when 1.
    #[serde(default = "fixed_one", skip_serializing_if = "is_fixed_one")]
    pub factor: u32,
}

impl OutputSetting {
    /// Same place on the desktop: everything but the name and connector,
    /// with refresh rates compared within [`REFRESH_TOLERANCE_MHZ`].
    pub fn same_place(&self, other: &OutputSetting) -> bool {
        if self.monitor != other.monitor || self.enabled != other.enabled {
            return false;
        }
        if !self.enabled {
            return true;
        }
        self.width == other.width
            && self.height == other.height
            && self.refresh_mhz.abs_diff(other.refresh_mhz) <= REFRESH_TOLERANCE_MHZ
            && self.x == other.x
            && self.y == other.y
            && self.rotation == other.rotation
            && self.primary == other.primary
            && self.factor == other.factor
    }

    /// Width and height on the screen, after rotation and scaling.
    pub fn extent(&self) -> (u16, u16) {
        let (w, h) = (scaled_len(self.width, self.factor), scaled_len(self.height, self.factor));
        if self.rotation == 90 || self.rotation == 270 { (h, w) } else { (w, h) }
    }

    /// e.g. "ASUS VS229 1920x1080@60.00 +0+0 primary", or "ASUS VS229 off".
    pub fn describe(&self) -> String {
        let name = if self.name.is_empty() { self.monitor.as_str() } else { self.name.as_str() };
        if !self.enabled {
            return format!("{name} off");
        }
        let mut s = format!(
            "{name} {}x{}@{:.2} +{}+{}",
            self.width,
            self.height,
            self.refresh_mhz as f64 / 1000.0,
            self.x,
            self.y
        );
        if self.rotation != 0 {
            s.push_str(&format!(" rotated {}", self.rotation));
        }
        if self.factor != FIXED_ONE {
            let (w, h) = self.extent();
            s.push_str(&format!(" scaled x{:.3} ({w}x{h})", self.factor as f64 / FIXED_ONE as f64));
        }
        if self.primary {
            s.push_str(" primary");
        }
        s
    }
}

/// True if both lists hold the same monitors in the same places (order and
/// connectors don't matter; identical twins are matched by connector).
pub fn same_arrangement(a: &[OutputSetting], b: &[OutputSetting]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut used = vec![false; b.len()];
    for x in a {
        let found = b
            .iter()
            .enumerate()
            .filter(|(i, y)| !used[*i] && y.monitor == x.monitor)
            .min_by_key(|(_, y)| y.connector != x.connector)
            .map(|(i, _)| i);
        match found {
            Some(i) if x.same_place(&b[i]) => used[i] = true,
            _ => return false,
        }
    }
    true
}

/// Lay monitors out again after their sizes on screen change (Mode A),
/// keeping their order: side by side in a row (top-aligned) if they were
/// side by side, stacked in a column (left-aligned) if they were stacked.
/// `factors` are the new scale factors, in the order of `current`. Mirrored
/// or overlapping monitors are refused: they can't keep their places when
/// their sizes differ.
///
/// Row or column is decided from the monitors' **unscaled** sizes: tools
/// that place monitors (`xrandr --right-of`) use the mode
/// size even when the picture is scaled, so scaled sizes can overlap a
/// neighbor that the user put side by side.
///
/// Monitors that overlap only partly (Xfce's display settings left one at
/// x=0 and the other at x=250) are laid out in the order of
/// their left edges, or failing that their top edges; only monitors at the
/// same position (mirrored) are refused.
pub fn repack(current: &[OutputSetting], factors: &[u32]) -> Result<Vec<OutputSetting>, String> {
    assert_eq!(current.len(), factors.len());
    let lit: Vec<usize> = (0..current.len()).filter(|&i| current[i].enabled).collect();
    let span = |i: usize| {
        let o = &current[i];
        let (w, h) = if o.rotation == 90 || o.rotation == 270 { (o.height, o.width) } else { (o.width, o.height) };
        (o.x as i32, o.y as i32, o.x as i32 + w as i32, o.y as i32 + h as i32)
    };
    let mut row = true;
    let mut col = true;
    let mut overlap = false;
    for (n, &a) in lit.iter().enumerate() {
        for &b in &lit[n + 1..] {
            let (ax0, ay0, ax1, ay1) = span(a);
            let (bx0, by0, bx1, by1) = span(b);
            if (ax0, ay0) == (bx0, by0) {
                return Err(format!(
                    "{} and {} are at the same place (mirrored); arrange them side by side first",
                    current[a].name, current[b].name
                ));
            }
            let apart_x = ax1 <= bx0 || bx1 <= ax0;
            let apart_y = ay1 <= by0 || by1 <= ay0;
            overlap |= !apart_x && !apart_y;
            row &= apart_x;
            col &= apart_y;
        }
    }
    if overlap || (!row && !col) {
        // Go by order: left edges if they all differ, else top edges.
        let distinct = |f: &dyn Fn(usize) -> i16| {
            let mut v: Vec<i16> = lit.iter().map(|&i| f(i)).collect();
            v.sort_unstable();
            v.windows(2).all(|w| w[0] != w[1])
        };
        if distinct(&|i| current[i].x) {
            row = true;
        } else if distinct(&|i| current[i].y) {
            row = false;
        } else {
            return Err("monitors overlap and share edges; arrange them side by side first".into());
        }
    }
    let mut out: Vec<OutputSetting> = current.to_vec();
    for (o, &f) in out.iter_mut().zip(factors) {
        o.factor = f;
    }
    let mut order = lit.clone();
    if row {
        order.sort_by_key(|&i| (current[i].x, current[i].y));
    } else {
        order.sort_by_key(|&i| (current[i].y, current[i].x));
    }
    let mut at: i32 = 0;
    for i in order {
        let (w, h) = out[i].extent();
        if row {
            out[i].x = at as i16;
            out[i].y = 0;
            at += w as i32;
        } else {
            out[i].x = 0;
            out[i].y = at as i16;
            at += h as i32;
        }
        if at > i16::MAX as i32 {
            return Err(format!("the screen would be {at} pixels long, more than X allows"));
        }
    }
    Ok(out)
}

/// Mode A (render at one scale, RandR shrinks each monitor) or Mode B
/// (native pixels, per-monitor factors to the toolkits that take them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScaleMode {
    #[default]
    A,
    B,
}

/// Scaling settings (milestone 4, docs/milestone-4.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScalingConfig {
    /// Off until `vergd scale on` (decision 4).
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: ScaleMode,
    /// "How big I like things", in percent (100 = the reference size).
    #[serde(default = "hundred")]
    pub preference: u32,
    /// Per-monitor viewing distance and overrides.
    #[serde(default, rename = "monitor", skip_serializing_if = "Vec::is_empty")]
    pub monitors: Vec<MonitorScaling>,
}

impl Default for ScalingConfig {
    fn default() -> Self {
        ScalingConfig { enabled: false, mode: ScaleMode::A, preference: 100, monitors: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorScaling {
    /// [`crate::monitor_id`].
    pub monitor: String,
    /// For people reading the file.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub distance: crate::scaling::Distance,
    /// A scale in percent that replaces the derived one (e.g. 150).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<u32>,
}

impl ScalingConfig {
    pub fn monitor(&self, id: &str) -> Option<&MonitorScaling> {
        self.monitors.iter().find(|m| m.monitor == id)
    }
}

/// One remembered setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub key: String,
    #[serde(default)]
    pub updated: String,
    #[serde(default, rename = "output")]
    pub outputs: Vec<OutputSetting>,
}

/// Every remembered setup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub scaling: ScalingConfig,
    #[serde(default, rename = "profile")]
    pub profiles: Vec<Profile>,
}

impl Store {
    pub fn from_toml(text: &str) -> Result<Store, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn to_toml(&self) -> String {
        // Serializing plain structs of strings, numbers and bools cannot fail.
        toml::to_string(self).expect("profile store serializes")
    }

    pub fn get(&self, key: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.key == key)
    }

    /// Insert or replace the profile with this key.
    pub fn put(&mut self, profile: Profile) {
        match self.profiles.iter_mut().find(|p| p.key == profile.key) {
            Some(p) => *p = profile,
            None => self.profiles.push(profile),
        }
    }

    /// Remove the profiles whose key starts with `prefix`; returns their keys.
    /// An empty prefix removes nothing.
    pub fn forget(&mut self, prefix: &str) -> Vec<String> {
        if prefix.is_empty() {
            return Vec::new();
        }
        let gone: Vec<String> =
            self.profiles.iter().filter(|p| p.key.starts_with(prefix)).map(|p| p.key.clone()).collect();
        self.profiles.retain(|p| !p.key.starts_with(prefix));
        gone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(monitor: &str, conn: &str, x: i16) -> OutputSetting {
        OutputSetting {
            monitor: monitor.into(),
            name: "M".into(),
            connector: conn.into(),
            enabled: true,
            width: 1920,
            height: 1080,
            refresh_mhz: 60000,
            x,
            y: 0,
            rotation: 0,
            primary: x == 0,
            factor: FIXED_ONE,
        }
    }

    #[test]
    fn factor_is_left_out_when_one_and_kept_otherwise() {
        let mut s = Store::default();
        let mut b = out("m2", "DP-1", 1920);
        b.factor = 2 * FIXED_ONE;
        s.put(Profile { key: "k".into(), updated: String::new(), outputs: vec![out("m1", "HDMI-1", 0), b] });
        s.scaling.enabled = true;
        let text = s.to_toml();
        assert_eq!(text.matches("factor =").count(), 1, "{text}");
        assert!(text.contains("[scaling]") && text.contains("enabled = true"), "{text}");
        assert_eq!(Store::from_toml(&text).unwrap(), s);
        // Old files (no factor, no [scaling]) still load.
        let old = Store::from_toml("[[profile]]\nkey = \"k\"\n[[profile.output]]\nmonitor = \"m\"\nenabled = true\n")
            .unwrap();
        assert_eq!(old.profiles[0].outputs[0].factor, FIXED_ONE);
        assert!(!old.scaling.enabled);
        assert_eq!(old.scaling.preference, 100);
    }

    #[test]
    fn extent_includes_the_factor() {
        let mut a = out("m", "X", 0);
        a.width = 3840;
        a.height = 2160;
        a.factor = 74898; // 2 / 1.75
        assert_eq!(a.extent(), (4389, 2469));
        a.factor = 2 * FIXED_ONE;
        assert_eq!(a.extent(), (7680, 4320));
    }

    #[test]
    fn repack_row_column_and_refusals() {
        // ASUS left, 4K right: row, top-aligned, 4K now 4389 wide at x=3840.
        let asus = out("asus", "HDMI-2", 0);
        let mut k = out("k", "HDMI-1", 1920);
        k.width = 3840;
        k.height = 2160;
        let r = repack(&[asus.clone(), k.clone()], &[2 * FIXED_ONE, 74898]).unwrap();
        assert_eq!((r[0].x, r[0].y, r[0].extent()), (0, 0, (3840, 2160)));
        assert_eq!((r[1].x, r[1].y, r[1].extent()), (3840, 0, (4389, 2469)));
        // Order kept: 4K left of the ASUS.
        let mut k2 = k.clone();
        k2.x = 0;
        let mut a2 = asus.clone();
        a2.x = 3840;
        let r = repack(&[a2, k2], &[2 * FIXED_ONE, 74898]).unwrap();
        assert_eq!((r[0].x, r[1].x), (4389, 0));
        // Stacked: column.
        let mut below = out("b", "DP-1", 0);
        below.y = 1080;
        let r = repack(&[asus.clone(), below], &[2 * FIXED_ONE, 2 * FIXED_ONE]).unwrap();
        assert_eq!((r[1].x, r[1].y), (0, 2160));
        // Back to 1: positions shrink again.
        let r = repack(&r, &[FIXED_ONE, FIXED_ONE]).unwrap();
        assert_eq!((r[1].x, r[1].y), (0, 1080));
        // Placed by xrandr --right-of a scaled monitor: next to its unscaled
        // size, so overlapping its scaled one. Still a row.
        let mut k3 = k.clone();
        k3.x = 3590;
        k3.factor = 76963;
        let mut a3 = asus.clone();
        a3.factor = 122_573;
        let mut t = out("t", "HDMI-3", 3590 + 3840);
        t.factor = FIXED_ONE;
        let r = repack(&[a3, k3, t], &[122_573, 76963, 136_533]).unwrap();
        assert_eq!(r[1].x, r[0].extent().0 as i16);
        assert_eq!(r[2].x, r[1].x + r[1].extent().0 as i16);
        // Partly overlapping (Xfce left the 4K at 0 and the ASUS at 250): a
        // row in the order of the left edges.
        let mut kk = k.clone();
        kk.x = 0;
        let mut aa = asus.clone();
        aa.x = 250;
        let r = repack(&[aa, kk], &[2 * FIXED_ONE, 74898]).unwrap();
        assert_eq!((r[1].x, r[0].x), (0, 4389));
        // Mirrored: refused.
        assert!(repack(&[asus.clone(), out("m", "DP-1", 0)], &[FIXED_ONE, FIXED_ONE]).is_err());
        // Disabled monitors are carried along untouched.
        let mut off = out("off", "DP-2", 0);
        off.enabled = false;
        let r = repack(&[asus, off.clone()], &[2 * FIXED_ONE, FIXED_ONE]).unwrap();
        assert!(!r[1].enabled);
    }

    #[test]
    fn toml_round_trip() {
        let mut s = Store::default();
        s.put(Profile {
            key: "abc".into(),
            updated: "t".into(),
            outputs: vec![out("m1", "HDMI-1", 0), out("m2", "DP-1", 1920)],
        });
        let text = s.to_toml();
        assert!(text.contains("[[profile]]") && text.contains("[[profile.output]]"), "{text}");
        assert_eq!(Store::from_toml(&text).unwrap(), s);
        assert_eq!(Store::from_toml("").unwrap(), Store::default());
    }

    #[test]
    fn put_replaces_and_forget_by_prefix() {
        let mut s = Store::default();
        s.put(Profile { key: "abc1".into(), updated: String::new(), outputs: vec![] });
        s.put(Profile { key: "abd2".into(), updated: String::new(), outputs: vec![] });
        s.put(Profile { key: "abc1".into(), updated: "new".into(), outputs: vec![] });
        assert_eq!(s.profiles.len(), 2);
        assert_eq!(s.get("abc1").unwrap().updated, "new");
        assert_eq!(s.forget(""), Vec::<String>::new());
        assert_eq!(s.forget("abc"), vec!["abc1".to_string()]);
        assert_eq!(s.profiles.len(), 1);
    }

    #[test]
    fn arrangement_ignores_order_and_small_refresh_differences() {
        let a = vec![out("m1", "HDMI-1", 0), out("m2", "DP-1", 1920)];
        let mut b = vec![out("m2", "DP-2", 1920), out("m1", "HDMI-1", 0)];
        b[0].refresh_mhz = 59950;
        assert!(same_arrangement(&a, &b));
        b[0].x = 1900;
        assert!(!same_arrangement(&a, &b));
    }

    #[test]
    fn twins_matched_by_connector() {
        let a = vec![out("tw", "HDMI-1", 0), out("tw", "HDMI-2", 1920)];
        let b = vec![out("tw", "HDMI-2", 1920), out("tw", "HDMI-1", 0)];
        assert!(same_arrangement(&a, &b));
        let swapped = vec![out("tw", "HDMI-1", 1920), out("tw", "HDMI-2", 0)];
        assert!(!same_arrangement(&a, &swapped));
    }

    #[test]
    fn disabled_ignores_geometry_and_rotation_swaps_extent() {
        let mut a = out("m", "X", 0);
        a.enabled = false;
        let mut b = a.clone();
        b.width = 640;
        assert!(a.same_place(&b));
        let mut r = out("m", "X", 0);
        r.rotation = 90;
        assert_eq!(r.extent(), (1080, 1920));
    }
}
