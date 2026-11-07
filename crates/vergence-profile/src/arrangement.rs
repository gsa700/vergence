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

/// One monitor's place in a setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSetting {
    /// [`crate::monitor_id`] of the monitor's EDID, or "port:<connector>"
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
    }

    /// Width and height on the screen, after rotation.
    pub fn extent(&self) -> (u16, u16) {
        if self.rotation == 90 || self.rotation == 270 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
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
        }
    }

    #[test]
    fn toml_round_trip() {
        let mut s = Store::default();
        s.put(Profile { key: "abc".into(), updated: "t".into(), outputs: vec![out("m1", "HDMI-1", 0), out("m2", "DP-1", 1920)] });
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
