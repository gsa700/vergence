// Scale and arrangement in desktop.toml [displays], as the settings app edits
// them (the server's displays.rs reads them; format documented there).

use toml::{Table, Value};

pub const DISTANCES: [&str; 3] = ["laptop", "desktop", "tv"];

fn displays_mut(t: &mut Table) -> &mut Table {
    if !t.get("displays").is_some_and(Value::is_table) {
        t.insert("displays".into(), Value::Table(Table::new()));
    }
    t.get_mut("displays").and_then(Value::as_table_mut).unwrap()
}

fn list_mut<'a>(t: &'a mut Table, key: &str) -> &'a mut Vec<Value> {
    if !t.get(key).is_some_and(Value::is_array) {
        t.insert(key.into(), Value::Array(Vec::new()));
    }
    t.get_mut(key).and_then(Value::as_array_mut).unwrap()
}

pub fn size(desktop: &Table) -> f64 {
    desktop.get("displays").and_then(|d| d.get("size")).and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64))).unwrap_or(1.0)
}

/// "Windows the same size on every screen" (`[displays] same-size`), off by default.
pub fn same_size(desktop: &Table) -> bool {
    desktop.get("displays").and_then(|d| d.get("same-size")).and_then(Value::as_bool).unwrap_or(false)
}

pub fn set_same_size(desktop: &mut Table, on: bool) {
    let d = displays_mut(desktop);
    if on {
        d.insert("same-size".into(), true.into());
    } else {
        d.remove("same-size");
    }
}

pub fn set_size(desktop: &mut Table, size: f64) {
    displays_mut(desktop).insert("size".into(), ((size * 100.0).round() / 100.0).into());
}

/// A screen's viewing distance (index into DISTANCES) and hand scale.
pub fn screen(desktop: &Table, id: &str) -> (usize, Option<f64>) {
    let entry = desktop
        .get("displays")
        .and_then(|d| d.get("screen"))
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|s| s.get("id").and_then(Value::as_str) == Some(id)));
    let dist = entry
        .and_then(|e| e.get("distance"))
        .and_then(Value::as_str)
        .and_then(|d| DISTANCES.iter().position(|x| *x == d))
        .unwrap_or(1);
    let scale = entry.and_then(|e| e.get("scale")).and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64)));
    (dist, scale)
}

/// Set a screen's chosen scale (None: back to its default), leaving its
/// viewing distance (if the file has one) as it is.
pub fn set_scale(desktop: &mut Table, id: &str, label: &str, scale: Option<f64>) {
    let list = list_mut(displays_mut(desktop), "screen");
    let i = match list.iter().position(|s| s.get("id").and_then(Value::as_str) == Some(id)) {
        Some(i) => i,
        None => {
            let mut t = Table::new();
            t.insert("id".into(), id.into());
            list.push(Value::Table(t));
            list.len() - 1
        }
    };
    let t = list[i].as_table_mut().unwrap();
    t.insert("label".into(), label.into());
    match scale {
        Some(s) => t.insert("scale".into(), ((s * 1000.0).round() / 1000.0).into()),
        None => t.remove("scale"),
    };
}

/// The size of controls a screen's choice names (look milestone 1, step 8):
/// 0 Automatic (unset), 1 Compact, 2 Standard, 3 Large.
pub const CONTROLS: [&str; 4] = ["automatic", "compact", "standard", "large"];

pub fn controls(desktop: &Table, id: &str) -> usize {
    desktop
        .get("displays")
        .and_then(|d| d.get("screen"))
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|s| s.get("id").and_then(Value::as_str) == Some(id)))
        .and_then(|e| e.get("controls"))
        .and_then(Value::as_str)
        .and_then(|c| CONTROLS.iter().position(|x| *x == c))
        .unwrap_or(0)
}

/// Set a screen's size of controls (Automatic takes the key away).
pub fn set_controls(desktop: &mut Table, id: &str, label: &str, choice: usize) {
    let list = list_mut(displays_mut(desktop), "screen");
    let i = match list.iter().position(|s| s.get("id").and_then(Value::as_str) == Some(id)) {
        Some(i) => i,
        None => {
            let mut t = Table::new();
            t.insert("id".into(), id.into());
            list.push(Value::Table(t));
            list.len() - 1
        }
    };
    let t = list[i].as_table_mut().unwrap();
    t.insert("label".into(), label.into());
    match choice.min(3) {
        0 => t.remove("controls"),
        c => t.insert("controls".into(), CONTROLS[c].into()),
    };
}

/// The scale slider's marks: every whole number from 1 to `max`, and the
/// default if it falls between them.
pub fn detents(default: f64, max: f64) -> Vec<f64> {
    let mut d: Vec<f64> = (1..=max.floor() as i32).map(f64::from).collect();
    if !d.contains(&default) {
        d.push(default);
        d.sort_by(f64::total_cmp);
    }
    d
}

/// The scales between `lo` and `hi` at which a `w` x `h` pixel screen is a
/// whole number of points both ways, so nothing lands between pixels. A
/// scale goes to apps in 120ths (wp_fractional_scale), so these are the
/// k/120 where k divides both w*120 and h*120: on 3840x2160, 125%, 150%,
/// 160%, 166⅔% and others; 144% is not one (2666.67 points wide).
pub fn clean_scales(w: u32, h: u32, lo: f64, hi: f64) -> Vec<f64> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let (w, h) = (u64::from(w) * 120, u64::from(h) * 120);
    let (first, last) = ((lo * 120.0).ceil().max(1.0) as u64, (hi * 120.0).floor() as u64);
    (first..=last).filter(|k| w % k == 0 && h % k == 0).map(|k| k as f64 / 120.0).collect()
}

/// Where a dragged scale settles: a strong mark (whole number, the default)
/// within 6%, else a clean scale within 2% (they are close together, so
/// they hold lightly), else where it is.
pub fn snap_scale(v: f64, strong: &[f64], clean: &[f64]) -> f64 {
    stick(v, strong, 0.06).or_else(|| stick(v, clean, 0.02)).unwrap_or(v)
}

/// Every mark the slider shows: the strong ones and the clean ones, once.
pub fn all_marks(strong: &[f64], clean: &[f64]) -> Vec<f64> {
    let mut m: Vec<f64> = strong.iter().chain(clean).copied().collect();
    m.sort_by(f64::total_cmp);
    m.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    m
}

/// The mark `v` is close enough to stick to, if any.
pub fn stick(v: f64, detents: &[f64], within: f64) -> Option<f64> {
    detents.iter().copied().filter(|d| (d - v).abs() <= within).min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs()))
}

/// Set a screen's viewing distance and hand scale (None: calculated).
#[allow(dead_code)]
pub fn set_screen(desktop: &mut Table, id: &str, label: &str, distance: usize, scale: Option<f64>) {
    let list = list_mut(displays_mut(desktop), "screen");
    let i = match list.iter().position(|s| s.get("id").and_then(Value::as_str) == Some(id)) {
        Some(i) => i,
        None => {
            let mut t = Table::new();
            t.insert("id".into(), id.into());
            list.push(Value::Table(t));
            list.len() - 1
        }
    };
    let t = list[i].as_table_mut().unwrap();
    t.insert("label".into(), label.into());
    t.insert("distance".into(), DISTANCES[distance.min(2)].into());
    match scale {
        Some(s) => t.insert("scale".into(), ((s * 1000.0).round() / 1000.0).into()),
        None => t.remove("scale"),
    };
}

/// A screen in the arrangement: id, top-left and size, in points.
#[derive(Debug, Clone, PartialEq)]
pub struct Rect {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Screen `moved` dropped with its top-left at (x, y): the place nearest to
/// that where it touches another screen edge to edge without covering any
/// (edges within `snap` points line up). Others stay where they are.
pub fn snap(screens: &[Rect], moved: usize, x: i32, y: i32, snap: i32) -> (i32, i32) {
    let m = &screens[moved];
    let others: Vec<&Rect> = screens.iter().enumerate().filter(|(i, _)| *i != moved).map(|(_, r)| r).collect();
    if others.is_empty() {
        return (0, 0);
    }
    let align = |v: i32, lines: &[i32]| lines.iter().copied().find(|l| (v - l).abs() <= snap).unwrap_or(v);
    let mut best: Option<((i32, i32), i64)> = None;
    for o in &others {
        // Beside it (left or right): any height that still shares an edge; above
        // or below it: any left that still shares an edge.
        let cy = align(y.clamp(o.y - m.h + 1, o.y + o.h - 1), &[o.y, o.y + o.h - m.h, o.y + (o.h - m.h) / 2]);
        let cx = align(x.clamp(o.x - m.w + 1, o.x + o.w - 1), &[o.x, o.x + o.w - m.w, o.x + (o.w - m.w) / 2]);
        for c in [(o.x + o.w, cy), (o.x - m.w, cy), (cx, o.y + o.h), (cx, o.y - m.h)] {
            let r = Rect { x: c.0, y: c.1, ..m.clone() };
            if others.iter().any(|q| overlaps(&r, q)) {
                continue;
            }
            let d = (c.0 - x) as i64 * (c.0 - x) as i64 + (c.1 - y) as i64 * (c.1 - y) as i64;
            if best.is_none_or(|(_, bd)| d < bd) {
                best = Some((c, d));
            }
        }
    }
    best.map(|(c, _)| c).unwrap_or((m.x, m.y))
}

/// Record where every screen sits in `setup` (top-left of the whole at 0, 0).
pub fn set_arrangement(desktop: &mut Table, setup: &str, screens: &[Rect]) {
    let (mx, my) = (screens.iter().map(|r| r.x).min().unwrap_or(0), screens.iter().map(|r| r.y).min().unwrap_or(0));
    let places: Vec<Value> = screens
        .iter()
        .map(|r| {
            let mut t = Table::new();
            t.insert("id".into(), r.id.clone().into());
            t.insert("x".into(), ((r.x - mx) as i64).into());
            t.insert("y".into(), ((r.y - my) as i64).into());
            Value::Table(t)
        })
        .collect();
    let list = list_mut(displays_mut(desktop), "arrangement");
    list.retain(|a| a.get("setup").and_then(Value::as_str) != Some(setup));
    let mut t = Table::new();
    t.insert("setup".into(), setup.into());
    t.insert("places".into(), Value::Array(places));
    list.push(Value::Table(t));
}

#[cfg(test)]
mod tests {

    use super::*;

    fn r(id: &str, x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { id: id.into(), x, y, w, h }
    }

    #[test]
    fn snapping_edge_to_edge() {
        let s = vec![r("a", 0, 0, 1170, 658), r("b", 1170, 0, 1800, 1013)];
        // A dragged to the right of B, a little low: beside B, its top kept.
        assert_eq!(snap(&s, 0, 3000, 120, 40), (2970, 120));
        // ... near B's top: lined up with it.
        assert_eq!(snap(&s, 0, 3000, 25, 40), (2970, 0));
        // Dropped over B: the nearest free place touching it.
        let (x, y) = snap(&s, 0, 1500, 100, 40);
        let moved = r("a", x, y, 1170, 658);
        assert!(!overlaps(&moved, &s[1]));
        // Below B, centered under it.
        assert_eq!(snap(&s, 0, 1480, 1100, 40), (1485, 1013));
    }

    #[test]
    fn scale_marks() {
        assert_eq!(detents(3.0, 5.0), [1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(detents(1.5, 3.0), [1.0, 1.5, 2.0, 3.0]);
        let d = detents(1.5, 3.0);
        assert_eq!(stick(1.96, &d, 0.06), Some(2.0));
        assert_eq!(stick(1.8, &d, 0.06), None);
        assert_eq!(stick(1.53, &d, 0.06), Some(1.5));
    }

    #[test]
    fn clean_scales_land_on_whole_points() {
        let k = |v: &[f64]| v.iter().map(|s| (s * 120.0).round() as u32).collect::<Vec<_>>();
        // 3840x2160 (Techbench's LG, 2026-11-17).
        let lg = clean_scales(3840, 2160, 0.75, 2.0);
        assert_eq!(k(&lg), [90, 96, 100, 120, 128, 144, 150, 160, 180, 192, 200, 225, 240]);
        for s in &lg {
            // In 120ths, as apps get it: 3840 * 120 / k points, exactly.
            let k = (s * 120.0).round() as u32;
            assert_eq!(3840 * 120 % k, 0, "{s}");
            assert_eq!(2160 * 120 % k, 0, "{s}");
        }
        // 144%, where the slider once stopped, is not clean; 125% and 150% are.
        assert!(!lg.iter().any(|s| (s - 1.4417).abs() < 0.001));
        assert!(lg.contains(&1.25) && lg.contains(&1.5));
        // 1920x1080 has the same stops but 106⅔%.
        assert_eq!(k(&clean_scales(1920, 1080, 0.75, 2.0)), [90, 96, 100, 120, 144, 150, 160, 180, 192, 200, 225, 240]);
        assert!(clean_scales(0, 1080, 0.75, 2.0).is_empty());
    }

    #[test]
    fn strong_marks_win_and_clean_ones_hold_lightly() {
        let strong = detents(2.0, 4.0);
        let clean = clean_scales(3840, 2160, 0.75, 4.0);
        assert_eq!(snap_scale(1.97, &strong, &clean), 2.0);
        assert_eq!(snap_scale(1.49, &strong, &clean), 1.5);
        assert_eq!(snap_scale(1.26, &strong, &clean), 1.25);
        assert_eq!(snap_scale(1.40, &strong, &clean), 1.40);
        let all = all_marks(&strong, &clean);
        assert_eq!(all.iter().filter(|m| **m == 2.0).count(), 1);
        assert!(all.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn edits_keep_the_rest_of_the_file() {
        let mut d: Table = "[displays]\nprimary = \"a\"\n[wallpaper]\nmode = \"fill\"\n".parse().unwrap();
        set_size(&mut d, 1.25);
        set_screen(&mut d, "b", "ASUS", 2, Some(1.5));
        assert_eq!(size(&d), 1.25);
        assert_eq!(screen(&d, "b"), (2, Some(1.5)));
        set_screen(&mut d, "b", "ASUS", 0, None);
        assert_eq!(screen(&d, "b"), (0, None));
        assert_eq!(screen(&d, "x"), (1, None));
        set_scale(&mut d, "b", "ASUS", Some(2.0));
        assert_eq!(screen(&d, "b"), (0, Some(2.0))); // distance kept
        set_scale(&mut d, "b", "ASUS", None);
        assert_eq!(screen(&d, "b"), (0, None));
        set_arrangement(&mut d, "s1", &[r("a", 100, 50, 10, 10), r("b", 110, 50, 10, 10)]);
        set_arrangement(&mut d, "s1", &[r("a", 0, 0, 10, 10), r("b", 10, 0, 10, 10)]);
        let text = toml::to_string(&d).unwrap();
        assert_eq!(text.matches("setup = \"s1\"").count(), 1, "{text}");
        assert!(d["displays"]["primary"].as_str() == Some("a") && d.get("wallpaper").is_some());
    }
}
