// Scale and position of each screen (2026-11-08; the settings app's Screens
// overview and each screen's Display section write them).
//
// ~/.config/vergence/desktop.toml:
//
//     [displays]
//     primary = "<id>"        # the settings app's: whose wallpaper is the default
//     size = 1.0              # "how big I like things", for every screen
//
//     [[displays.screen]]     # per screen, by its EDID id (any port)
//     id = "a48cece3b53cb939"
//     label = "ASUS VS229"    # for people
//     distance = "desktop"    # optional: laptop | desktop | tv (else guessed)
//     scale = 2.0             # optional: the screen's scale, chosen; else the default
//
//     [[displays.arrangement]]   # where the screens sit, per monitor setup
//     setup = "<profile key>"    # (the set of connected EDIDs, as window memory)
//     places = [ { id = "...", x = 0, y = 0 }, { id = "...", x = 1170, y = 0 } ]
//
// SCALE (Dave, 2026-11-08: "always try and default to the native scale if
// possible for maximum sharpness ... most people have one screen"). A screen's
// default is calculated as the whitepaper says (§3, Contract 1: its pixels and
// millimeters from the EDID, and how far away it is, a built-in laptop panel
// assumed at laptop distance, everything else at desk distance), then snapped
// to the nearest whole number when that is within 25%: whole numbers are sharp
// in every toolkit and need no resampling, and most single monitors get 1x,
// their native pixels. A scale chosen on the screen's page replaces the
// default. The size preference (the Screens overview) multiplies every screen.
// Positions are in points. A setup without an arrangement is laid out left to
// right, tops aligned; screens the arrangement doesn't name go on the right.

use serde::Deserialize;
use vergence_profile::scaling::{Distance, derive};

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DisplaysConfig {
    pub size: Option<f64>,
    pub screen: Vec<ScreenConfig>,
    pub arrangement: Vec<Arrangement>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ScreenConfig {
    pub id: String,
    pub distance: Option<String>,
    pub scale: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Arrangement {
    pub setup: String,
    pub places: Vec<Place>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Place {
    pub id: String,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Default, Deserialize)]
struct DesktopFile {
    #[serde(default)]
    displays: DisplaysConfig,
}

pub fn load() -> DisplaysConfig {
    crate::wallpaper::desktop_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| match toml::from_str::<DesktopFile>(&t) {
            Ok(f) => f.displays,
            Err(e) => {
                tracing::warn!("desktop.toml [displays]: {e}");
                DisplaysConfig::default()
            }
        })
        .unwrap_or_default()
}

pub fn distance(name: Option<&str>) -> Distance {
    match name {
        Some("laptop") => Distance::Laptop,
        Some("tv") => Distance::Tv,
        _ => Distance::Desktop,
    }
}

impl DisplaysConfig {
    fn screen(&self, id: &str) -> Option<&ScreenConfig> {
        self.screen.iter().find(|s| s.id == id)
    }

    /// The size preference, kept to a sane range.
    pub fn size(&self) -> f64 {
        self.size.filter(|s| s.is_finite()).unwrap_or(1.0).clamp(0.5, 3.0)
    }

    /// A screen's scales: the one it is shown at (in 120ths, as apps are told
    /// it), its default, the base (chosen or default, before the size
    /// preference), and how they were found.
    pub fn scale_for(&self, id: &str, connector: &str, px: (u32, u32), mm: Option<(u32, u32)>) -> Scales {
        let conf = self.screen(id);
        let dist = match conf.and_then(|c| c.distance.as_deref()) {
            Some(d) => distance(Some(d)),
            None if is_builtin(connector) => Distance::Laptop,
            None => Distance::Desktop,
        };
        let d = derive(px, mm, dist, 1.0);
        let default = sharp(d.exact);
        let calc = match d.note {
            Some(n) => format!("calculated 1 ({n})"),
            None => format!("calculated {:.2} for {:.0} ppi at {} distance", d.exact, d.ppi.unwrap_or(0.0), format!("{dist:?}").to_lowercase()),
        };
        let chosen = conf.and_then(|c| c.scale).filter(|s| s.is_finite() && (0.5..=8.0).contains(s));
        let (base, how) = match chosen {
            Some(s) => (s, format!("chosen {}; the default is {} ({calc})", pct(s), pct(default))),
            None if default == d.exact.max(1.0 / 120.0) => (default, format!("default: {calc}")),
            None => (default, format!("default {}, the sharpest near it ({calc})", pct(default))),
        };
        let size = self.size();
        let how = if size != 1.0 { format!("{how}; size {}", pct(size)) } else { how };
        Scales { shown: crate::scale_in_120ths(base * size), default, base, how }
    }

    /// Where `id` sits in `setup`, if the arrangement says.
    pub fn place(&self, setup: &str, id: &str) -> Option<(i32, i32)> {
        self.arrangement.iter().find(|a| a.setup == setup)?.places.iter().find(|p| p.id == id).map(|p| (p.x, p.y))
    }
}

/// A screen's scales (see scale_for).
#[derive(Debug, Clone, PartialEq)]
pub struct Scales {
    pub shown: f64,
    pub default: f64,
    pub base: f64,
    pub how: String,
}

fn pct(s: f64) -> String {
    format!("{:.0}%", s * 100.0)
}

/// A built-in panel (laptop, tablet): used closer than a desk monitor.
pub fn is_builtin(connector: &str) -> bool {
    ["eDP", "LVDS", "DSI"].iter().any(|p| connector.starts_with(p))
}

/// The sharpest scale near `exact`: the nearest whole number (at least 1) if
/// it is within 25% of it, else `exact` itself (in 120ths).
pub fn sharp(exact: f64) -> f64 {
    // Below 1 would mean bigger pixels than the panel's: never by default.
    if exact <= 1.0 {
        return 1.0;
    }
    let n = exact.round().max(1.0);
    if (n - exact).abs() <= 0.25 * exact { n } else { crate::scale_in_120ths(exact) }
}

/// Lay out screens (id, width, height in points, in left-to-right order):
/// arranged ones where the arrangement puts them, the rest on the right, tops
/// aligned; then nudged apart if a scale change made them overlap, and moved
/// so the top-left corner of the whole is at 0, 0.
pub fn lay_out(cfg: &DisplaysConfig, setup: &str, screens: &[(String, i32, i32)]) -> Vec<(i32, i32)> {
    let mut pos: Vec<Option<(i32, i32)>> = screens.iter().map(|(id, _, _)| cfg.place(setup, id)).collect();
    let mut right = screens.iter().zip(&pos).filter_map(|((_, w, _), p)| p.map(|(x, _)| x + w)).max().unwrap_or(0);
    for (i, (_, w, _)) in screens.iter().enumerate() {
        if pos[i].is_none() {
            pos[i] = Some((right, 0));
            right += w;
        }
    }
    let mut pos: Vec<(i32, i32)> = pos.into_iter().map(Option::unwrap).collect();
    // Overlaps (a screen got smaller or bigger in points): push the later one
    // (by x) right until clear. A few passes settle any chain.
    for _ in 0..screens.len() {
        let mut moved = false;
        let mut order: Vec<usize> = (0..screens.len()).collect();
        order.sort_by_key(|&i| (pos[i].0, pos[i].1));
        for a in 0..order.len() {
            for b in a + 1..order.len() {
                let (i, j) = (order[a], order[b]);
                let (ax, ay, aw, ah) = (pos[i].0, pos[i].1, screens[i].1, screens[i].2);
                let (bx, by, bw, bh) = (pos[j].0, pos[j].1, screens[j].1, screens[j].2);
                if ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah {
                    pos[j].0 = ax + aw;
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
    let (mx, my) = (pos.iter().map(|p| p.0).min().unwrap_or(0), pos.iter().map(|p| p.1).min().unwrap_or(0));
    pos.iter().map(|(x, y)| (x - mx, y - my)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(text: &str) -> DisplaysConfig {
        toml::from_str::<DesktopFile>(text).unwrap().displays
    }

    #[test]
    fn defaults_snap_to_whole_numbers() {
        assert_eq!(sharp(1.07), 1.0); // the ASUS: native pixels
        assert_eq!(sharp(3.28), 3.0); // the 14" 4K
        assert_eq!(sharp(1.70), 2.0); // a 27" 4K
        assert_eq!(sharp(1.14), 1.0); // a 27" 1440p
        assert_eq!(sharp(0.6), 1.0); // never below 1 by default
        // Between 1 and 2 nothing is near enough: the exact value. Higher up,
        // 25% reaches the next whole number (2.5 -> 3, 20% bigger).
        assert_eq!(sharp(1.5), 1.5);
        assert_eq!(sharp(2.5), 3.0);
    }

    #[test]
    fn scales_default_chosen_and_size() {
        // The ASUS VS229: 1920x1080 on 475x267 mm (103 ppi, exact 1.07).
        let none = DisplaysConfig::default();
        let s = none.scale_for("asus", "HDMI-A-1", (1920, 1080), Some((475, 267)));
        assert_eq!((s.shown, s.default, s.base), (1.0, 1.0, 1.0));
        assert!(s.how.starts_with("default 100%, the sharpest near it"), "{}", s.how);
        // The 14" 4K (315 ppi): 3x; on an eDP connector it is a laptop panel (2.31 -> 2x).
        assert_eq!(none.scale_for("x", "HDMI-A-2", (3840, 2160), Some((310, 174))).shown, 3.0);
        assert_eq!(none.scale_for("x", "eDP-1", (3840, 2160), Some((310, 174))).shown, 2.0);
        // Chosen, and the size preference over it.
        let c = cfg("[displays]\nsize = 1.25\n[[displays.screen]]\nid = \"asus\"\nscale = 2.0\n");
        let s = c.scale_for("asus", "HDMI-A-1", (1920, 1080), Some((475, 267)));
        assert_eq!((s.shown, s.default, s.base), (2.5, 1.0, 2.0));
        // A silly chosen value is ignored.
        let c = cfg("[[displays.screen]]\nid = \"asus\"\nscale = 40.0\n");
        assert_eq!(c.scale_for("asus", "HDMI-A-1", (1920, 1080), Some((475, 267))).base, 1.0);
    }

    #[test]
    fn arrangement_and_defaults() {
        let screens = vec![("a".to_string(), 1170, 658), ("b".to_string(), 1800, 1013)];
        // No arrangement: left to right.
        assert_eq!(lay_out(&DisplaysConfig::default(), "s", &screens), [(0, 0), (1170, 0)]);
        // B on the left, A on its right, lower down.
        let c = cfg("[[displays.arrangement]]\nsetup = \"s\"\nplaces = [{ id = \"b\", x = 0, y = 0 }, { id = \"a\", x = 1800, y = 355 }]\n");
        assert_eq!(lay_out(&c, "s", &screens), [(1800, 355), (0, 0)]);
        // Another setup's arrangement doesn't apply.
        assert_eq!(lay_out(&c, "other", &screens), [(0, 0), (1170, 0)]);
        // A grew (smaller scale) and now overlaps B: pushed clear.
        let c = cfg("[[displays.arrangement]]\nsetup = \"s\"\nplaces = [{ id = \"a\", x = 0, y = 0 }, { id = \"b\", x = 1000, y = 0 }]\n");
        assert_eq!(lay_out(&c, "s", &screens), [(0, 0), (1170, 0)]);
        // Negative places are moved to 0, 0.
        let c = cfg("[[displays.arrangement]]\nsetup = \"s\"\nplaces = [{ id = \"a\", x = -1170, y = -10 }, { id = \"b\", x = 0, y = 0 }]\n");
        assert_eq!(lay_out(&c, "s", &screens), [(0, 0), (1170, 10)]);
    }
}
