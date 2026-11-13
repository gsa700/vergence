//! Window memory (milestone 3): where each window was on each monitor setup.
//!
//! A window is recognized by its class (`WM_CLASS` instance and class), its
//! role (`WM_WINDOW_ROLE`, often empty) and its place among open windows of
//! the same class (0 = the first opened). **Titles are never stored**
//! (Dave, 2026-11-06): they carry document names, email subjects and web
//! page titles.
//!
//! Positions are relative to the top-left corner of the monitor the window
//! is on, so a window follows its monitor when the arrangement changes.

use serde::{Deserialize, Serialize};

/// Workspace value meaning "on every workspace" (EWMH sticky).
pub const ALL_DESKTOPS: u32 = 0xFFFF_FFFF;

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WindowId {
    pub instance: String,
    pub class: String,
    #[serde(default)]
    pub role: String,
    /// Place among open windows of the same instance/class/role, for windows
    /// with no token (0 for those with one).
    #[serde(default)]
    pub index: u32,
    /// The window's persistence token (Contract 2; bar milestone 1, step 5):
    /// given by the server, presented again by the program on a later run, so
    /// the window is itself whatever else opened or closed meanwhile. Empty
    /// for windows of programs that keep none (they go by `index`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
}

impl WindowId {
    /// e.g. "Navigator/firefox #1", or "vergence-term/vergence-term (main) 3f2a…" with a token.
    pub fn describe(&self) -> String {
        let role = if self.role.is_empty() { String::new() } else { format!(" ({})", self.role) };
        if self.token.is_empty() {
            format!("{}/{}{role} #{}", self.instance, self.class, self.index + 1)
        } else {
            format!("{}/{}{role} {}…", self.instance, self.class, &self.token[..self.token.len().min(4)])
        }
    }

    /// The same program and role with no token: where the last of its
    /// windows the user closed was, the place a new window of it starts from.
    pub fn untokened(&self) -> WindowId {
        WindowId { token: String::new(), index: 0, ..self.clone() }
    }
}

/// Where one window was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSetting {
    #[serde(flatten)]
    pub id: WindowId,
    /// Monitor ID (see `crate::monitor_id`) the window's center was on.
    pub monitor: String,
    /// Position of the client window relative to the monitor's top-left,
    /// in the monitor's panel pixels (X pixels divided by its RandR scale
    /// factor; the same as X pixels when unscaled).
    pub x: i32,
    pub y: i32,
    /// Size in app units: X pixels divided by the render scale (milestone 4
    /// Mode A; the same as X pixels when unscaled).
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub max_horz: bool,
    #[serde(default)]
    pub max_vert: bool,
    #[serde(default)]
    pub fullscreen: bool,
    /// Workspace number, or [`ALL_DESKTOPS`]; None if the window manager
    /// does not say.
    #[serde(default)]
    pub desktop: Option<u32>,
    /// For a window with a token: the day it was last seen (days since
    /// 1970-01-01), so the store can forget windows that never come back.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub seen: u32,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

impl WindowSetting {
    pub fn same_place(&self, o: &WindowSetting) -> bool {
        self == o
    }

    /// e.g. "+100+50 1200x900 maximized, workspace 2".
    pub fn describe_place(&self) -> String {
        let mut s = format!("+{}+{} {}x{}", self.x, self.y, self.width, self.height);
        if self.fullscreen {
            s.push_str(" fullscreen");
        } else if self.max_horz && self.max_vert {
            s.push_str(" maximized");
        } else if self.max_horz {
            s.push_str(" maximized across");
        } else if self.max_vert {
            s.push_str(" maximized up/down");
        }
        match self.desktop {
            Some(ALL_DESKTOPS) => s.push_str(", all workspaces"),
            Some(d) => s.push_str(&format!(", workspace {}", d + 1)),
            None => {}
        }
        s
    }
}

/// The windows of one monitor setup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowProfile {
    pub key: String,
    #[serde(default)]
    pub updated: String,
    #[serde(default, rename = "window")]
    pub windows: Vec<WindowSetting>,
}

impl WindowProfile {
    pub fn get(&self, id: &WindowId) -> Option<&WindowSetting> {
        self.windows.iter().find(|w| &w.id == id)
    }

    /// Insert or replace by window ID; returns true if anything changed.
    pub fn put(&mut self, w: WindowSetting) -> bool {
        match self.windows.iter_mut().find(|x| x.id == w.id) {
            Some(x) if *x == w => false,
            Some(x) => {
                *x = w;
                true
            }
            None => {
                self.windows.push(w);
                true
            }
        }
    }
}

/// Every setup's windows, plus the opt-out list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowStore {
    /// Applications vergd leaves alone: matched against the instance or
    /// class, case-insensitive (e.g. "thunderbird").
    #[serde(default)]
    pub skip: Vec<String>,
    #[serde(default, rename = "profile")]
    pub profiles: Vec<WindowProfile>,
}

impl WindowStore {
    pub fn from_toml(text: &str) -> Result<WindowStore, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("window store serializes")
    }

    pub fn get(&self, key: &str) -> Option<&WindowProfile> {
        self.profiles.iter().find(|p| p.key == key)
    }

    pub fn get_mut_or_new(&mut self, key: &str) -> &mut WindowProfile {
        if let Some(i) = self.profiles.iter().position(|p| p.key == key) {
            return &mut self.profiles[i];
        }
        self.profiles.push(WindowProfile { key: key.to_string(), ..Default::default() });
        self.profiles.last_mut().expect("just pushed")
    }

    pub fn skips(&self, id: &WindowId) -> bool {
        self.skip.iter().any(|s| s.eq_ignore_ascii_case(&id.class) || s.eq_ignore_ascii_case(&id.instance))
    }

    /// Forget a window by its token, on every monitor setup (the user closed
    /// it for good). Returns how many places were forgotten.
    pub fn forget_token(&mut self, token: &str) -> usize {
        if token.is_empty() {
            return 0;
        }
        let mut n = 0;
        for p in &mut self.profiles {
            let before = p.windows.len();
            p.windows.retain(|w| w.id.token != token);
            n += before - p.windows.len();
        }
        n
    }

    /// Forget windows with a token not seen since `day` (days since 1970);
    /// windows without a token are kept. Returns how many were forgotten.
    pub fn prune_unseen(&mut self, day: u32) -> usize {
        let mut n = 0;
        for p in &mut self.profiles {
            let before = p.windows.len();
            p.windows.retain(|w| w.id.token.is_empty() || w.seen >= day);
            n += before - p.windows.len();
        }
        n
    }

    /// Remove the profiles whose key starts with `prefix` (non-empty).
    pub fn forget(&mut self, prefix: &str) -> usize {
        if prefix.is_empty() {
            return 0;
        }
        let before = self.profiles.len();
        self.profiles.retain(|p| !p.key.starts_with(prefix));
        before - self.profiles.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(class: &str, index: u32, x: i32) -> WindowSetting {
        WindowSetting {
            id: WindowId { instance: class.to_lowercase(), class: class.into(), role: String::new(), index, token: String::new() },
            monitor: "m1".into(),
            x,
            y: 10,
            width: 800,
            height: 600,
            max_horz: false,
            max_vert: false,
            fullscreen: false,
            desktop: Some(0),
            seen: 0,
        }
    }

    #[test]
    fn round_trip_and_flattened_ids() {
        let mut s = WindowStore { skip: vec!["thunderbird".into()], profiles: vec![] };
        let p = s.get_mut_or_new("k1");
        p.put(w("Firefox", 0, 5));
        p.put(w("Firefox", 1, 900));
        let text = s.to_toml();
        assert!(text.contains("[[profile.window]]") && text.contains("instance = \"firefox\""), "{text}");
        assert!(!text.to_lowercase().contains("title"));
        assert_eq!(WindowStore::from_toml(&text).unwrap(), s);
    }

    #[test]
    fn put_reports_changes() {
        let mut p = WindowProfile::default();
        assert!(p.put(w("A", 0, 1)));
        assert!(!p.put(w("A", 0, 1)));
        assert!(p.put(w("A", 0, 2)));
        assert!(p.put(w("A", 1, 2)));
        assert_eq!(p.windows.len(), 2);
    }

    #[test]
    fn skip_matches_class_or_instance() {
        let s = WindowStore { skip: vec!["THUNDERBIRD".into()], profiles: vec![] };
        let id = WindowId { instance: "Mail".into(), class: "thunderbird".into(), ..Default::default() };
        assert!(s.skips(&id));
        let other = WindowId { instance: "x".into(), class: "y".into(), ..Default::default() };
        assert!(!s.skips(&other));
    }

    #[test]
    fn describe() {
        let mut a = w("Firefox", 0, 5);
        a.max_horz = true;
        a.max_vert = true;
        a.desktop = Some(1);
        assert_eq!(a.describe_place(), "+5+10 800x600 maximized, workspace 2");
        assert_eq!(a.id.describe(), "firefox/Firefox #1");
    }

    fn tokened(token: &str, seen: u32, x: i32) -> WindowSetting {
        let mut t = w("vergence-term", 0, x);
        t.id.token = token.into();
        t.seen = seen;
        t
    }

    #[test]
    fn a_token_names_one_window_on_every_setup_and_forgetting_it_takes_it_from_all() {
        let mut s = WindowStore::default();
        s.get_mut_or_new("home").put(tokened("aa", 100, 10));
        s.get_mut_or_new("home").put(tokened("bb", 100, 20));
        s.get_mut_or_new("desk").put(tokened("aa", 100, 30));
        s.get_mut_or_new("desk").put(w("Firefox", 0, 5));
        let text = s.to_toml();
        assert!(text.contains("token = \"aa\"") && text.contains("seen = 100"), "{text}");
        assert!(!text.contains("token = \"\"") && !text.contains("seen = 0"), "untokened windows write no token or seen: {text}");
        assert_eq!(WindowStore::from_toml(&text).unwrap(), s);
        assert_eq!(s.forget_token("aa"), 2);
        assert_eq!(s.get("home").unwrap().windows.len(), 1);
        assert_eq!(s.get("desk").unwrap().windows.len(), 1, "Firefox, untokened, stays");
        assert_eq!(s.forget_token(""), 0);
    }

    #[test]
    fn windows_not_seen_for_long_are_pruned_and_untokened_ones_kept() {
        let mut s = WindowStore::default();
        let p = s.get_mut_or_new("home");
        p.put(tokened("old", 10, 1));
        p.put(tokened("new", 200, 2));
        p.put(w("Firefox", 0, 3));
        assert_eq!(s.prune_unseen(100), 1);
        let ids: Vec<String> = s.get("home").unwrap().windows.iter().map(|w| w.id.describe()).collect();
        assert_eq!(ids.len(), 2, "{ids:?}");
    }

    #[test]
    fn untokened_is_the_programs_last_closed_place() {
        let t = tokened("cafe1234", 1, 0).id;
        let u = t.untokened();
        assert_eq!((u.token.as_str(), u.index, u.class.as_str()), ("", 0, "vergence-term"));
        assert!(t.describe().ends_with("cafe…"));
    }
}
