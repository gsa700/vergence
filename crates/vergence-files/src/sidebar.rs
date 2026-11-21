//! FileView's sidebar (files milestone 3, step 3; Dave, 2026-11-21: "I like
//! the explorer left panel and the mac one as well. what I would take away
//! from them is drag and drop reordering, shortcuts from folders and files
//! dropped there from the main window"). Three sections, each folded by its
//! heading:
//!
//! - **Favorites**: Home and the user's folders at first, then whatever the
//!   user makes of it: reordered by dragging, folders and files dropped in
//!   from the main view as shortcuts, any removed. Kept in desktop.toml's
//!   `[files] favorites` (a list of paths) once changed.
//! - **Locations**: the computer, disks, network mounts and the Trash.
//! - **Recent**: the files opened most lately (FileView, Edit, Reader,
//!   Close-Up and the Open dialog keep the list), and All recent. Off with
//!   `[files] recent = false` (Settings, or Hide Recent on its heading).
//!
//! Rows are of fixed heights, so a drop's place is worked out from where it
//! is (the sidebar's own place and how far it is scrolled).

use std::path::{Path, PathBuf};

use vergence_widgets::files::{self, Place};

/// A row's height, a heading's, and the padding around them, in points.
pub const ROW_H: f32 = 26.0;
pub const HEAD_H: f32 = 24.0;
pub const PAD: f32 = 6.0;
/// The most recent files shown.
pub const RECENT_SHOWN: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Favorites,
    Locations,
    Recent,
}

impl Section {
    pub fn name(self) -> &'static str {
        match self {
            Section::Favorites => "Favorites",
            Section::Locations => "Locations",
            Section::Recent => "Recent",
        }
    }
}

/// What a row is: a heading, or a place in a section.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Head(Section),
    Item(Section, Item),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: String,
    /// Where it goes (None: All recent).
    pub path: Option<PathBuf>,
    /// A file, opened rather than gone to.
    pub file: bool,
    /// The place as the shared sidebar knows it (its icon), for places.
    pub place: Option<Place>,
}

/// The sidebar's settings, from desktop.toml's text.
#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    /// None: the default favorites (Home and the user's folders).
    pub favorites: Option<Vec<PathBuf>>,
    pub folded: Vec<Section>,
    pub recent: bool,
}

pub fn setup(desktop: &str, section: &str) -> Setup {
    let key = |k: &str| vergence_ui::table_value(desktop, section, k);
    let folded = key("sidebar-folded")
        .map(|v| parse_list(&v))
        .unwrap_or_default()
        .iter()
        .filter_map(|n| [Section::Favorites, Section::Locations, Section::Recent].into_iter().find(|s| s.name() == n))
        .collect();
    Setup { favorites: key("favorites").map(|v| parse_list(&v).into_iter().map(PathBuf::from).filter(|p| p.is_absolute()).collect()), folded, recent: key("recent").as_deref() != Some("false") }
}

/// The favorites as they stand: the user's own, or Home and the user's folders.
pub fn favorites(s: &Setup) -> Vec<PathBuf> {
    match &s.favorites {
        Some(f) => f.clone(),
        None => files::home().into_iter().chain(files::user_dirs().into_iter().map(|(_, p)| p)).collect(),
    }
}

/// A place's name in the sidebar: Home, a user folder by its own name, else
/// the last part of the path.
fn name_of(p: &Path) -> String {
    if files::home().as_deref() == Some(p) {
        return "Home".into();
    }
    if let Some((n, _)) = files::user_dirs().into_iter().find(|(_, d)| d == p) {
        return n;
    }
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// Every row, in order: each section's heading, then (unless folded) its places.
pub fn lines(s: &Setup, locations: &[Place], recent: &[PathBuf]) -> Vec<Line> {
    let mut out = Vec::new();
    let section = |sec: Section, items: Vec<Item>, out: &mut Vec<Line>| {
        out.push(Line::Head(sec));
        if !s.folded.contains(&sec) {
            out.extend(items.into_iter().map(|i| Line::Item(sec, i)));
        }
    };
    let favs = favorites(s)
        .into_iter()
        .filter(|p| p.exists())
        .map(|p| {
            let file = !p.is_dir();
            // Home and the user's folders show their own marks; any other
            // folder or file its kind's (as a place it would be taken for a disk).
            let own = files::home().as_deref() == Some(p.as_path()) || files::user_dirs().iter().any(|(_, d)| *d == p);
            let place = own.then(|| Place { name: name_of(&p), path: Some(p.clone()), network: false });
            Item { name: name_of(&p), path: Some(p), file, place }
        })
        .collect();
    section(Section::Favorites, favs, &mut out);
    let locs = locations.iter().map(|p| Item { name: p.name.clone(), path: p.path.clone(), file: false, place: Some(p.clone()) }).collect();
    section(Section::Locations, locs, &mut out);
    if s.recent {
        let mut items: Vec<Item> = recent.iter().filter(|p| p.exists()).take(RECENT_SHOWN).map(|p| Item { name: name_of(p), path: Some(p.clone()), file: !p.is_dir(), place: None }).collect();
        items.push(Item { name: "All recent".into(), path: None, file: false, place: Some(Place { name: "Recent".into(), path: None, network: false }) });
        section(Section::Recent, items, &mut out);
    }
    out
}

/// The row at `y` points down the sidebar's list (its padding and the
/// scroll taken in), if any.
pub fn line_at(lines: &[Line], y: f32) -> Option<usize> {
    let mut top = PAD;
    for (i, l) in lines.iter().enumerate() {
        let h = if matches!(l, Line::Head(_)) { HEAD_H } else { ROW_H };
        if y >= top && y < top + h {
            return Some(i);
        }
        top += h;
    }
    None
}

/// Where in the favorites a drop at row `at` puts what is dropped: before
/// the favorite there, after the last if below them (on the Favorites
/// heading: first); None outside the Favorites section.
pub fn drop_index(lines: &[Line], at: Option<usize>) -> Option<usize> {
    let favs_end = lines.iter().position(|l| matches!(l, Line::Head(s) if *s != Section::Favorites)).unwrap_or(lines.len());
    match at {
        Some(0) => Some(0),
        Some(i) if i < favs_end => Some(i - 1),
        // Just below the last favorite (on the next heading): last.
        Some(i) if i == favs_end => Some(favs_end.saturating_sub(1)),
        _ => None,
    }
}

/// The favorites with `paths` put at `at` (each taken from where it was, if
/// it was there already: a reorder).
pub fn put(favs: &[PathBuf], paths: &[PathBuf], at: usize) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = favs.to_vec();
    let mut at = at.min(out.len());
    for p in paths {
        if let Some(i) = out.iter().position(|f| f == p) {
            out.remove(i);
            if i < at {
                at -= 1;
            }
        }
    }
    for (k, p) in paths.iter().enumerate() {
        out.insert((at + k).min(out.len()), p.clone());
    }
    out
}

/// A TOML list of strings, as desktop.toml holds it.
pub fn list_value(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))).collect();
    format!("[{}]", quoted.join(", "))
}

/// A TOML list of basic strings back (what [`list_value`] writes).
pub fn parse_list(v: &str) -> Vec<String> {
    let v = v.trim();
    let Some(inner) = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else { return Vec::new() };
    let mut out = Vec::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(n) = chars.next() {
                        s.push(n);
                    }
                }
                '"' => break,
                c => s.push(c),
            }
        }
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_are_written_and_read_back() {
        let items = vec!["/home/d".to_string(), "/home/d/My \"Odd\" Folder".to_string(), "C:\\not\\here".to_string()];
        assert_eq!(parse_list(&list_value(&items)), items);
        assert_eq!(parse_list("[]"), Vec::<String>::new());
        assert_eq!(parse_list("nonsense"), Vec::<String>::new());
    }

    #[test]
    fn a_reorder_takes_the_favorite_from_where_it_was() {
        let f: Vec<PathBuf> = ["/a", "/b", "/c", "/d"].iter().map(PathBuf::from).collect();
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(put(&f, &[p("/d")], 1), vec![p("/a"), p("/d"), p("/b"), p("/c")]);
        assert_eq!(put(&f, &[p("/a")], 3), vec![p("/b"), p("/c"), p("/a"), p("/d")], "down past two: before the one that was fourth");
        assert_eq!(put(&f, &[p("/new")], 9), vec![p("/a"), p("/b"), p("/c"), p("/d"), p("/new")]);
    }

    #[test]
    fn a_drop_finds_its_row_and_its_place_in_the_favorites() {
        let item = |n: &str| Item { name: n.into(), path: Some(PathBuf::from(n)), file: false, place: None };
        let lines = vec![
            Line::Head(Section::Favorites),
            Line::Item(Section::Favorites, item("/a")),
            Line::Item(Section::Favorites, item("/b")),
            Line::Head(Section::Locations),
            Line::Item(Section::Locations, item("/")),
        ];
        assert_eq!(line_at(&lines, PAD + 1.0), Some(0));
        assert_eq!(line_at(&lines, PAD + HEAD_H + ROW_H + 1.0), Some(2));
        assert_eq!(line_at(&lines, 9999.0), None);
        assert_eq!(drop_index(&lines, Some(0)), Some(0), "on the heading: first");
        assert_eq!(drop_index(&lines, Some(2)), Some(1), "on /b: before it");
        assert_eq!(drop_index(&lines, Some(3)), Some(2), "just below: last");
        assert_eq!(drop_index(&lines, Some(4)), None, "in Locations: not a favorite");
    }

    #[test]
    fn folded_sections_show_their_heading_alone() {
        let s = Setup { favorites: Some(vec![std::env::temp_dir()]), folded: vec![Section::Favorites], recent: false };
        let l = lines(&s, &[], &[]);
        assert_eq!(l, vec![Line::Head(Section::Favorites), Line::Head(Section::Locations)], "Recent off: no section");
    }
}
