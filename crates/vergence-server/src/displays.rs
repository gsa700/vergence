// Scale and position of each screen (2026-11-08; the settings app's Screens
// overview and each screen's Display section write them).
//
// ~/.config/vergence/desktop.toml:
//
//     [displays]
//     primary = "<id>"        # the main screen: the dock's, and the default wallpaper's
//     size = 1.0              # Accessibility's "how big I like things", for every screen
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
// default. The size preference (Accessibility; until 2026-11-17 also on the
// Screens overview, where it read as a scale) multiplies every screen.
// Positions are in points. A setup without an arrangement is laid out left to
// right, tops aligned, in the order they were connected; screens the
// arrangement doesn't name go on the right (drm.rs lay_out).

use serde::Deserialize;
use vergence_profile::scaling::{Distance, derive};

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DisplaysConfig {
    pub size: Option<f64>,
    /// "Windows the same size on every screen" (settings milestone 2): every
    /// screen whose scale is not chosen takes its exact calculated scale,
    /// not the nearest whole number. desktop.toml `[displays] same-size`.
    #[serde(rename = "same-size")]
    pub same_size: Option<bool>,
    /// How much of the text a chosen scale takes away is given back
    /// (Scales::text): 0 none, 1 all, as a power of the ratio.
    /// desktop.toml `[displays] text-strength`; TEXT_STRENGTH unset.
    #[serde(rename = "text-strength")]
    pub text_strength: Option<f64>,
    pub screen: Vec<ScreenConfig>,
    pub arrangement: Vec<Arrangement>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ScreenConfig {
    pub id: String,
    pub distance: Option<String>,
    pub scale: Option<f64>,
    /// A mode chosen in the settings app: "3840x2160@60", "1920x1080@59.94"
    /// (settings milestone 2). The server sets it only if the monitor offers it.
    pub mode: Option<String>,
}

/// "3840x2160@59.94" -> (3840, 2160, 59940): width, height, refresh in millihertz.
pub fn parse_mode(s: &str) -> Option<(u32, u32, u32)> {
    let (size, rate) = s.trim().split_once('@')?;
    let (w, h) = size.split_once('x')?;
    let hz: f64 = rate.trim().parse().ok()?;
    let (w, h): (u32, u32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w > 0 && h > 0 && w <= 16384 && h <= 16384 && hz > 1.0 && hz < 1000.0).then(|| (w, h, (hz * 1000.0).round() as u32))
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
    /// The mode chosen for a screen, if any.
    pub fn mode_for(&self, id: &str) -> Option<(u32, u32, u32)> {
        self.screen.iter().find(|s| s.id == id).and_then(|s| s.mode.as_deref()).and_then(parse_mode)
    }

    pub fn text_strength(&self) -> f64 {
        self.text_strength.filter(|s| s.is_finite()).unwrap_or(TEXT_STRENGTH).clamp(0.0, 1.0)
    }

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
        // The same size everywhere: the exact scale (in the 120ths apps draw
        // at, never below 1); else the sharpest near it.
        let same = self.same_size.unwrap_or(false);
        let default = if same { crate::scale_in_120ths(d.exact.max(1.0)) } else { sharp(d.exact) };
        let calc = match d.note {
            Some(n) => format!("calculated 1 ({n})"),
            None => format!("calculated {:.2} for {:.0} ppi at {} distance", d.exact, d.ppi.unwrap_or(0.0), format!("{dist:?}").to_lowercase()),
        };
        let chosen = conf.and_then(|c| c.scale).filter(|s| s.is_finite() && (0.5..=8.0).contains(s));
        let (base, how) = match chosen {
            Some(s) => (s, format!("chosen {}; the default is {} ({calc})", pct(s), pct(default))),
            None if same => (default, format!("default {}, the same size as on every screen ({calc})", pct(default))),
            None if default == d.exact.max(1.0 / 120.0) => (default, format!("default: {calc}")),
            None => (default, format!("default {}, the sharpest near it ({calc})", pct(default))),
        };
        let size = self.size();
        let how = if size != 1.0 { format!("{how}; size {}", pct(size)) } else { how };
        // Text as large as the calculated scale would show it, whatever scale
        // was chosen (Dave, 2026-11-18: per screen, full compensation): 133%
        // on the LG, calculated 1.69, draws text 1.27 times its size. Before
        // the size preference, which makes text larger with the rest.
        // Full compensation was too much (Dave, 10:33: "maybe we adjust the
        // ratio down a bit"): a smaller scale chosen for room is also a wish
        // for smaller text, and the distances are guesses. So a part of it,
        // as a power: 0.5 gives the LG 1.13x.
        let text_ratio = match d.note {
            None => d.exact / base,
            Some(_) => 1.0,
        };
        let text = text_factor(text_ratio, self.text_strength());
        let how = if (text - 1.0).abs() >= 0.005 { format!("{how}; text {text:.2}x") } else { how };
        Scales { shown: crate::scale_in_120ths(base * size), default, base, text, text_ratio, how }
    }

    /// Where `id` sits in `setup`, if the arrangement says.
    pub fn place(&self, setup: &str, id: &str) -> Option<(i32, i32)> {
        self.arrangement.iter().find(|a| a.setup == setup)?.places.iter().find(|p| p.id == id).map(|p| (p.x, p.y))
    }
}

use vergence_protocol::window::{MAX_TEXT, MIN_TEXT};

/// How much of the lost text size a chosen scale gets back, unless
/// desktop.toml says (DisplaysConfig::text_strength).
pub const TEXT_STRENGTH: f64 = 0.5;

/// A screen's text factor from its ratio (Scales::text_ratio) and the
/// strength; the settings app works it out the same way (disp.rs).
pub fn text_factor(ratio: f64, strength: f64) -> f64 {
    if !(ratio.is_finite() && ratio > 0.0) {
        return 1.0;
    }
    ratio.powf(strength.clamp(0.0, 1.0)).clamp(MIN_TEXT, MAX_TEXT)
}

/// A screen's scales (see scale_for).
#[derive(Debug, Clone, PartialEq)]
pub struct Scales {
    pub shown: f64,
    pub default: f64,
    pub base: f64,
    /// Text drawn this much larger than its size says: `text_ratio` to the
    /// power of the text strength (text_factor).
    pub text: f64,
    /// The calculated scale over the base (1 when the EDID has no size).
    pub text_ratio: f64,
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
    fn text_follows_the_calculated_scale() {
        let lg = |c: &DisplaysConfig| c.scale_for("lg", "DP-1", (3840, 2160), Some((600, 340))).text;
        let near = |a: f64, b: f64| (a - b).abs() < 0.01;
        // techbench's LG (162 ppi, calculated 1.69) chosen at 133%: 1.27 lost,
        // half of it back by default (1.27^0.5 = 1.13), all of it at strength 1.
        let c = cfg("[[displays.screen]]\nid = \"lg\"\nscale = 1.3333333333\n");
        assert!(near(lg(&c), 1.127), "{}", lg(&c));
        let full = cfg("[displays]\ntext-strength = 1.0\n[[displays.screen]]\nid = \"lg\"\nscale = 1.3333333333\n");
        assert!(near(lg(&full), 1.27), "{}", lg(&full));
        let off = cfg("[displays]\ntext-strength = 0\n[[displays.screen]]\nid = \"lg\"\nscale = 1.3333333333\n");
        assert_eq!(lg(&off), 1.0);
        // At its default 200% the text is drawn a little smaller.
        assert!(near(lg(&DisplaysConfig::default()), 0.919), "{}", lg(&DisplaysConfig::default()));
        // The size preference does not take it back: text grows with the rest.
        let c = cfg("[displays]\nsize = 1.25\n[[displays.screen]]\nid = \"lg\"\nscale = 1.3333333333\n");
        assert!(near(lg(&c), 1.127));
        // Hubble's panel at a laptop's distance: 1.04, half of it 1.02.
        let s = DisplaysConfig::default().scale_for("p", "eDP-1", (1920, 1080), Some((344, 193)));
        assert!(near(s.text, 1.02), "{}", s.text);
        // No size from the EDID: as given.
        assert_eq!(DisplaysConfig::default().scale_for("x", "HDMI-A-1", (1920, 1080), None).text, 1.0);
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

#[cfg(test)]
mod mode_tests {
    use super::parse_mode;

    #[test]
    fn modes_as_written() {
        assert_eq!(parse_mode("3840x2160@60"), Some((3840, 2160, 60000)));
        assert_eq!(parse_mode("1920x1080@59.94"), Some((1920, 1080, 59940)));
        assert_eq!(parse_mode(" 1280x720 @ 50 "), Some((1280, 720, 50000)));
        assert_eq!(parse_mode("1920x1080"), None);
        assert_eq!(parse_mode("0x1080@60"), None);
        assert_eq!(parse_mode("99999x1080@60"), None);
        assert_eq!(parse_mode("1920x1080@abc"), None);
    }

    #[test]
    fn the_same_size_everywhere_uses_exact_scales() {
        let plain: super::DisplaysConfig = toml::from_str("").unwrap();
        let same: super::DisplaysConfig = toml::from_str("same-size = true").unwrap();
        // The 14" 4K at desk distance: calculated 3.28, sharp 3.
        let a = plain.scale_for("x", "HDMI-A-1", (3840, 2160), Some((310, 174)));
        let b = same.scale_for("x", "HDMI-A-1", (3840, 2160), Some((310, 174)));
        assert_eq!(a.default, 3.0);
        assert!((b.default - 3.28).abs() < 0.02, "{}", b.default);
        assert_eq!((b.default * 120.0).fract(), 0.0, "in 120ths");
        assert!(b.how.contains("same size"));
    }
}
