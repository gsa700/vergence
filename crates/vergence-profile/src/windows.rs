//! Window memory (milestone 3): where each window was on each monitor setup.
//!
//! A window is recognized by its class (`WM_CLASS` instance and class), its
//! role (`WM_WINDOW_ROLE`, often empty) and its place among open windows of
//! the same class (0 = the first opened). **Titles are never stored**
//! (decision 2a in docs/milestone-3.md): they carry document names, email
//! subjects and web page titles.
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
    /// The app a window belongs to, by name: its class, else its instance,
    /// lower case ("firefox", "vergence-term").
    pub fn app_name(&self) -> String {
        if self.class.is_empty() { self.instance.to_lowercase() } else { self.class.to_lowercase() }
    }

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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The part of its monitor it was snapped to ("left", "top-right", ...;
    /// windows milestone 1), empty if none: x, y, width and height are then
    /// where it goes back to, as for a maximized window.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub snap: String,
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
        if !self.snap.is_empty() {
            s.push_str(&format!(" snapped {}", self.snap));
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

/// A setup's key for one size of its monitors: the monitors' key (their
/// EDIDs, [`crate::profile_key`]) and each monitor's size in points, so the
/// same monitors at another resolution or scale keep places of their own
/// (see docs/bar-milestone-1.md). `sizes`: (monitor id, width, height), in any order.
pub fn sized_key(base: &str, sizes: &[(String, u32, u32)]) -> String {
    let mut v: Vec<String> = sizes.iter().map(|(id, w, h)| format!("{id}={w}x{h}")).collect();
    v.sort();
    format!("{base}|{}", v.join(","))
}

/// A key's monitors' key and its monitors' sizes (none for a key from before
/// sizes were kept, or one that cannot be read).
pub fn key_sizes(key: &str) -> (&str, Vec<(String, u32, u32)>) {
    let (base, monitors) = key_monitors(key);
    (base, monitors.into_iter().map(|m| (m.id, m.width, m.height)).collect())
}

/// A monitor in a sized key: its id, its size in points, and its scale in
/// 120ths when the key says (keys since 2026-11-18: "id=WxH@S").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMonitor {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub scale120: Option<u32>,
}

impl KeyMonitor {
    /// Its size in pixels, when the key gives its scale.
    pub fn pixels(&self) -> Option<(u32, u32)> {
        let s = self.scale120?;
        Some(((self.width as u64 * s as u64 / 120) as u32, (self.height as u64 * s as u64 / 120) as u32))
    }
}

/// A key's monitors' key and its monitors, with their scales where known.
pub fn key_monitors(key: &str) -> (&str, Vec<KeyMonitor>) {
    let Some((base, rest)) = key.split_once('|') else { return (key, Vec::new()) };
    let monitors = rest
        .split(',')
        .filter_map(|m| {
            let (id, rest) = m.split_once('=')?;
            let (wh, scale120) = match rest.split_once('@') {
                Some((wh, s)) => (wh, Some(s.parse().ok()?)),
                None => (rest, None),
            };
            let (w, h) = wh.split_once('x')?;
            Some(KeyMonitor { id: id.to_string(), width: w.parse().ok()?, height: h.parse().ok()?, scale120 })
        })
        .collect();
    (base, monitors)
}

/// A sized key with each monitor's scale too (in 120ths): a place recorded
/// at another scale of the same mode keeps its size in points, one at
/// another resolution scales in proportion.
pub fn sized_key_scaled(base: &str, monitors: &[(String, u32, u32, u32)]) -> String {
    let mut v: Vec<String> = monitors.iter().map(|(id, w, h, s)| format!("{id}={w}x{h}@{s}")).collect();
    v.sort();
    format!("{base}|{}", v.join(","))
}

impl WindowSetting {
    /// This place on its monitor after a scale change (same pixels, `to`
    /// points now): the same size and place in points, pulled inside the
    /// monitor where it no longer fits: a scale change keeps the point size.
    pub fn kept(&self, to: (u32, u32)) -> WindowSetting {
        let width = self.width.min(to.0.max(1));
        let height = self.height.min(to.1.max(1));
        WindowSetting {
            x: self.x.min(to.0 as i32 - width as i32).max(0),
            y: self.y.min(to.1 as i32 - height as i32).max(0),
            width,
            height,
            ..self.clone()
        }
    }

    /// This place on its monitor at size `from`, in proportion on the same
    /// monitor at size `to` (points): a window a third of the way across
    /// stays a third of the way across, and as large a part of the monitor.
    pub fn scaled(&self, from: (u32, u32), to: (u32, u32)) -> WindowSetting {
        if from == to || from.0 == 0 || from.1 == 0 {
            return self.clone();
        }
        let (fx, fy) = (to.0 as f64 / from.0 as f64, to.1 as f64 / from.1 as f64);
        WindowSetting {
            x: (self.x as f64 * fx).round() as i32,
            y: (self.y as f64 * fy).round() as i32,
            width: ((self.width as f64 * fx).round() as u32).max(1),
            height: ((self.height as f64 * fy).round() as u32).max(1),
            ..self.clone()
        }
    }
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

    /// The other sizes of the setup whose monitors' key is `base` (and a
    /// profile from before sizes were kept, keyed by `base` alone), the most
    /// recently changed first; `except` is left out.
    pub fn variants(&self, base: &str, except: &str) -> Vec<&WindowProfile> {
        let mut v: Vec<&WindowProfile> =
            self.profiles.iter().filter(|p| p.key != except && key_sizes(&p.key).0 == base).collect();
        v.sort_by(|a, b| b.updated.cmp(&a.updated));
        v
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

    /// The apps with places (their class, else instance) and how many places
    /// each, on every setup, by name.
    pub fn apps(&self) -> Vec<(String, u32)> {
        let mut out: Vec<(String, u32)> = Vec::new();
        for w in self.profiles.iter().flat_map(|p| &p.windows) {
            let app = w.id.app_name();
            match out.iter_mut().find(|(a, _)| *a == app) {
                Some(e) => e.1 += 1,
                None => out.push((app, 1)),
            }
        }
        out.sort();
        out
    }

    /// Forget every place of an app (by [`WindowId::app_name`]), or of every
    /// app when `app` is None; how many were forgotten.
    pub fn forget_app(&mut self, app: Option<&str>) -> usize {
        let mut n = 0;
        for p in &mut self.profiles {
            let before = p.windows.len();
            p.windows.retain(|w| app.is_some_and(|a| w.id.app_name() != a));
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

    #[test]
    fn sized_keys_round_trip_and_places_scale_in_proportion() {
        let k = sized_key("abc", &[("m2".into(), 1280, 720), ("m1".into(), 1920, 1080)]);
        assert_eq!(k, "abc|m1=1920x1080,m2=1280x720", "sorted, so the order monitors come in does not matter");
        let (base, sizes) = key_sizes(&k);
        assert_eq!((base, sizes.len()), ("abc", 2));
        assert!(sizes.contains(&("m2".into(), 1280, 720)));
        assert_eq!(key_sizes("abc"), ("abc", vec![]), "a key from before sizes: no sizes");
        // Keys with scales (2026-11-18), and the older ones without.
        let k = sized_key_scaled("abc", &[("m1".into(), 1920, 1080, 120), ("m2".into(), 1280, 720, 240)]);
        assert_eq!(k, "abc|m1=1920x1080@120,m2=1280x720@240");
        let (_, m) = key_monitors(&k);
        assert_eq!(m[1].pixels(), Some((2560, 1440)));
        assert_eq!(key_sizes(&k).1[1], ("m2".to_string(), 1280, 720));
        assert_eq!(key_monitors("abc|m1=1920x1080").1[0].scale120, None);
        // A scale change keeps points, pulled inside a smaller monitor.
        let w2 = WindowSetting {
            id: WindowId::default(),
            monitor: "m1".into(),
            x: 1500,
            y: 100,
            width: 420,
            height: 260,
            ..WindowSetting::default()
        };
        let k2 = w2.kept((1280, 720));
        assert_eq!((k2.x, k2.y, k2.width, k2.height), (860, 100, 420, 260));
        let huge = WindowSetting { width: 3000, height: 2000, ..w2.clone() }.kept((1280, 720));
        assert_eq!((huge.x, huge.y, huge.width, huge.height), (0, 0, 1280, 720));
        let w = WindowSetting {
            id: WindowId::default(),
            monitor: "m1".into(),
            x: 640,
            y: 360,
            width: 960,
            height: 540,
            ..WindowSetting::default()
        };
        let s = w.scaled((1920, 1080), (1280, 720));
        assert_eq!((s.x, s.y, s.width, s.height), (427, 240, 640, 360));
        assert_eq!(w.scaled((1920, 1080), (1920, 1080)), w);
        let mut store = WindowStore::default();
        store.get_mut_or_new("abc").updated = "2026-11-01 10:00:00".into();
        store.get_mut_or_new("abc|m1=1280x720").updated = "2026-11-12 10:00:00".into();
        store.get_mut_or_new("abc|m1=1920x1080").updated = "2026-11-10 10:00:00".into();
        store.get_mut_or_new("other|m1=1920x1080");
        let keys: Vec<&str> = store.variants("abc", "abc|m1=1920x1080").iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["abc|m1=1280x720", "abc"], "newest first, the current one and other setups left out");
    }

    fn w(class: &str, index: u32, x: i32) -> WindowSetting {
        WindowSetting {
            id: WindowId {
                instance: class.to_lowercase(),
                class: class.into(),
                role: String::new(),
                index,
                token: String::new(),
            },
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
            snap: String::new(),
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
        assert!(
            !text.contains("token = \"\"") && !text.contains("seen = 0"),
            "untokened windows write no token or seen: {text}"
        );
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

    #[test]
    fn apps_listed_and_forgotten() {
        let mut store = WindowStore::default();
        store.get_mut_or_new("a").windows.extend([w("Firefox", 0, 0), w("vergence-term", 0, 0)]);
        store.get_mut_or_new("b").windows.extend([w("vergence-term", 0, 0), w("vergence-term", 1, 100)]);
        assert_eq!(store.apps(), vec![("firefox".to_string(), 1), ("vergence-term".to_string(), 3)]);
        assert_eq!(store.forget_app(Some("vergence-term")), 3);
        assert_eq!(store.apps(), vec![("firefox".to_string(), 1)]);
        assert_eq!(store.forget_app(None), 1);
        assert!(store.apps().is_empty());
    }
}
