//! Files as the Open/Save dialog and File Viewer show them (files
//! milestone 1, step 6): a folder's entries with their types, sorted as
//! people expect; the places (Home and the user's folders by the XDG
//! user-dirs file, Recent, the computer, mounted disks); the recent files;
//! and the views both draw: the places sidebar, the list with its sortable
//! headings, and the icon view. Moved here from filedialog.rs.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use vergence_scene::{Color, Rect};

use crate::{Align, ButtonLook, Element, Length, Padding, Theme, button, column, drawing, label, panel, row, scroll, space};

/// The most rows listed (a folder of more is narrowed by typing).
pub const MAX_ROWS: usize = 2000;
const MAX_RECENT: usize = 16;
const PREVIEW_BYTES: usize = 8 * 1024;

/// How a list is sorted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    Name,
    Size,
    Changed,
    /// By what the type is called ("Plain Text Document"), then by name.
    Kind,
}

/// How tall the list's rows are (files milestone 2: Finder's View Options).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RowSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl RowSize {
    /// From a setting's words ("small", "medium", "large"); Medium otherwise.
    pub fn parse(s: &str) -> RowSize {
        match s.trim().trim_matches('"') {
            "small" => RowSize::Small,
            "large" => RowSize::Large,
            _ => RowSize::Medium,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            RowSize::Small => "small",
            RowSize::Medium => "medium",
            RowSize::Large => "large",
        }
    }

    /// A row's height in points (one line of name).
    pub fn height(self) -> f32 {
        match self {
            RowSize::Small => 24.0,
            RowSize::Medium => 30.0,
            RowSize::Large => 38.0,
        }
    }

    /// The mark's size, times the small one.
    fn mark(self) -> f32 {
        match self {
            RowSize::Small => 1.0,
            RowSize::Medium => 1.2,
            RowSize::Large => 1.5,
        }
    }

    /// The name's text size; the other columns' a point less.
    fn text(self) -> f32 {
        match self {
            RowSize::Small => 12.0,
            RowSize::Medium => 12.5,
            RowSize::Large => 13.5,
        }
    }
}

/// The list's columns after the name, in order.
pub const COLUMNS: [(&str, SortBy); 3] = [("Date Changed", SortBy::Changed), ("Size", SortBy::Size), ("Kind", SortBy::Kind)];
/// Their widths unless changed, and the narrowest and widest they may be.
pub const COLUMN_WIDTHS: [f32; 3] = [140.0, 80.0, 150.0];
pub const COLUMN_MIN: f32 = 50.0;
pub const COLUMN_MAX: f32 = 420.0;

/// How the list looks (files milestone 2): row size, the columns' widths
/// (as [`COLUMNS`]), whether the Kind column shows, striped rows, and the
/// entries grouped by their family under headings.
#[derive(Clone, Debug, PartialEq)]
pub struct ListLook {
    pub rows: RowSize,
    pub widths: [f32; 3],
    pub kind: bool,
    pub stripes: bool,
    pub group: bool,
}

impl Default for ListLook {
    fn default() -> ListLook {
        ListLook { rows: RowSize::Medium, widths: COLUMN_WIDTHS, kind: true, stripes: true, group: false }
    }
}

/// The widths from a setting's words ("140,80,150"), each kept in bounds;
/// the usual ones for any that are missing or wrong.
pub fn parse_widths(s: &str) -> [f32; 3] {
    let mut w = COLUMN_WIDTHS;
    for (i, part) in s.trim().trim_matches('"').split(',').take(3).enumerate() {
        if let Ok(v) = part.trim().parse::<f32>()
            && v.is_finite()
        {
            w[i] = v.clamp(COLUMN_MIN, COLUMN_MAX);
        }
    }
    w
}

/// What an entry's type is called, for the Kind column.
pub fn kind_words(e: &Entry) -> String {
    if e.dir {
        return "Folder".into();
    }
    match e.mime {
        Some(m) => vergence_types::describe(m),
        None => "Document".into(),
    }
}

/// The heading an entry is grouped under, and the groups' order.
pub fn group_of(e: &Entry) -> (u8, &'static str) {
    match e.kind() {
        Kind::Folder => (0, "Folders"),
        Kind::Document => (1, "Documents"),
        Kind::Text => (2, "Text"),
        Kind::Picture => (3, "Pictures"),
        Kind::Sound => (4, "Sounds"),
        Kind::Video => (5, "Videos"),
        Kind::Archive => (6, "Archives"),
        Kind::Program => (7, "Programs"),
        Kind::Other => (8, "Other"),
    }
}

/// One file or folder as listed.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub dir: bool,
    /// Its type by its name (a folder's is a folder), if the name says.
    pub mime: Option<&'static str>,
    pub size: u64,
    pub changed: Option<SystemTime>,
}

impl Entry {
    /// An entry for a path (links followed: a link to a folder is a folder);
    /// its type by its name, else by its first bytes.
    pub fn of(path: PathBuf) -> Option<Entry> {
        Entry::of_looking(path, true)
    }

    /// An entry for a path. `look`: when the name says nothing of its type,
    /// its first bytes are read (vergence-types: signatures, then text), so
    /// a file with no extension gets its mark (Dave, 2026-11-18: the
    /// desktop's icon for "Vergence-wishlist-2" had no picture). Not on a
    /// network mount, where opening every file to list a folder is slow.
    pub fn of_looking(path: PathBuf, look: bool) -> Option<Entry> {
        let name = path.file_name()?.to_string_lossy().into_owned();
        let m = fs::metadata(&path).ok();
        let dir = m.as_ref().is_some_and(|m| m.is_dir());
        let file = m.as_ref().is_some_and(|m| m.is_file());
        let mime = kind_of(dir, &name).or_else(|| (look && file).then(|| vergence_types::of_file(&path)));
        Some(Entry { mime, dir, size: m.as_ref().map_or(0, |m| m.len()), changed: m.and_then(|m| m.modified().ok()), name, path })
    }

    pub fn kind(&self) -> Kind {
        Kind::of(self.mime)
    }
}

/// A folder's entries, or why it cannot be read (in plain words).
pub fn read_folder(dir: &Path) -> Result<Vec<Entry>, String> {
    match fs::read_dir(dir) {
        Ok(rd) => {
            let look = !network_mounts().iter().any(|m| dir.starts_with(m));
            Ok(rd.flatten().filter_map(|e| Entry::of_looking(e.path(), look)).collect())
        }
        Err(e) => Err(format!("This folder cannot be read: {}", plain(&e))),
    }
}

/// The files opened or saved lately through the dialog, newest first, those still there.
pub fn recent() -> Vec<Entry> {
    recent_files().into_iter().filter_map(Entry::of).collect()
}

/// Entries in order: folders first, then by `by` (names naturally), up or
/// down; `keep` keeps the order they came in for names (the recent files);
/// `group`: by their family first ([`group_of`]), each group sorted so.
pub fn sort(v: &mut [&Entry], by: SortBy, up: bool, keep: bool, group: bool) {
    v.sort_by(|a, b| {
        let first = if group { group_of(a).0.cmp(&group_of(b).0) } else { b.dir.cmp(&a.dir) };
        if first != std::cmp::Ordering::Equal {
            return first;
        }
        let o = match by {
            SortBy::Name if keep => std::cmp::Ordering::Equal,
            SortBy::Name => natural(&a.name, &b.name),
            SortBy::Size => a.size.cmp(&b.size),
            SortBy::Changed => a.changed.cmp(&b.changed),
            SortBy::Kind => kind_words(a).to_lowercase().cmp(&kind_words(b).to_lowercase()).then_with(|| natural(&a.name, &b.name)),
        };
        if up { o } else { o.reverse() }
    });
}

/// A place in the sidebar: its name and where it goes (None: Recent).
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub path: Option<PathBuf>,
    /// Mounted over the network: listed under Network.
    pub network: bool,
}

/// The user's folders by the XDG user-dirs file
/// (~/.config/user-dirs.dirs: `XDG_DESKTOP_DIR="$HOME/Desktop"` and the
/// rest), so a renamed or translated folder is found; the usual names where
/// the file says nothing.
pub fn user_dirs() -> Vec<(String, PathBuf)> {
    let Some(h) = home() else { return Vec::new() };
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| h.join(".config"));
    let text = fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    let said = |key: &str| -> Option<PathBuf> {
        let line = text.lines().find(|l| l.trim_start().starts_with(&format!("XDG_{key}_DIR=")))?;
        let v = line.split_once('=')?.1.trim().trim_matches('"');
        let p = match v.strip_prefix("$HOME") {
            Some(rest) => h.join(rest.trim_start_matches('/')),
            None => PathBuf::from(v),
        };
        // "$HOME" alone means the folder is not wanted.
        (p != h).then_some(p)
    };
    [("Desktop", "DESKTOP"), ("Documents", "DOCUMENTS"), ("Downloads", "DOWNLOAD"), ("Pictures", "PICTURES"), ("Music", "MUSIC"), ("Videos", "VIDEOS")]
        .into_iter()
        .filter_map(|(name, key)| {
            let p = said(key).unwrap_or_else(|| h.join(name));
            p.is_dir().then(|| (name.to_string(), p))
        })
        .collect()
}

/// The places: Home, the user's folders, Recent, the computer, mounted
/// disks, then what is mounted over the network.
pub fn places() -> Vec<Place> {
    let mut out = Vec::new();
    if let Some(h) = home() {
        out.push(Place { name: "Home".into(), path: Some(h), network: false });
        out.extend(user_dirs().into_iter().map(|(name, p)| Place { name, path: Some(p), network: false }));
    }
    out.push(Place { name: "Recent".into(), path: None, network: false });
    out.push(Place { name: "Computer".into(), path: Some(PathBuf::from("/")), network: false });
    let name_of = |m: &Path| m.file_name().map_or_else(|| m.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
    for m in mounted_disks() {
        out.push(Place { name: name_of(&m), path: Some(m), network: false });
    }
    for m in network_mounts() {
        out.push(Place { name: name_of(&m), path: Some(m), network: true });
    }
    out
}

/// Which of Vergence's own icons a place shows in the sidebar: the user's
/// folders their own (by the names user_dirs gives them), the Trash full or
/// empty, drives and the network theirs.
fn place_icon(p: &Place) -> (&'static str, &'static str) {
    if p.network {
        return ("places", "network");
    }
    match (p.name.as_str(), p.path.as_deref()) {
        (_, None) => ("places", "recent"),
        ("Home", _) => ("places", "home"),
        ("Desktop", _) => ("places", "desktop"),
        ("Documents", _) => ("places", "documents"),
        ("Downloads", _) => ("places", "downloads"),
        ("Pictures", _) => ("places", "pictures"),
        ("Music", _) => ("places", "music"),
        ("Videos", _) => ("places", "videos"),
        ("Computer", _) => ("places", "disk"),
        ("Trash", Some(t)) => ("places", if fs::read_dir(t).is_ok_and(|mut d| d.next().is_some()) { "trash-full" } else { "trash" }),
        _ => ("places", "usb"),
    }
}

/// The places sidebar: each a button, the one shown chosen. `here`: the
/// folder shown (None for Recent).
pub fn sidebar<M: Clone + 'static>(places: &[Place], here: Option<&Path>, go: impl Fn(&Place) -> M, t: &Theme) -> Element<M> {
    let mut items: Vec<Element<M>> = Vec::new();
    let mut headed = false;
    for p in places {
        // The network's places under a heading of their own.
        if p.network && !headed {
            headed = true;
            // (A label takes no padding: a space above, one before.)
            items.push(space().height(Length::Fixed(8.0)));
            items.push(row(vec![space().width(Length::Fixed(10.0)), label("Network").size(10.5).dim()]));
        }
        let chosen = p.path.as_deref() == here;
        let (group, name) = place_icon(p);
        let mark = drawing(move |scene, _, r: Rect, _t: &Theme| {
            let side = r.w.min(r.h);
            if let Some(img) = own_icon(group, name, (side * 2.5).ceil() as u32) {
                let id = scene.image(img);
                scene.draw_image(id, Rect::new(r.x, r.y + (r.h - side) / 2.0, side, side), 1.0);
            }
        })
        .width(Length::Fixed(16.0))
        .height(Length::Fixed(16.0));
        let line = row(vec![mark, label(p.name.clone()).size(12.0).width(Length::FILL)]).spacing(7.0).align(Align::Center).width(Length::FILL);
        items.push(button(line).look(if chosen { ButtonLook::Chosen } else { ButtonLook::Plain }).padding(Padding::hv(5.0, 8.0)).on_press(go(p)).width(Length::FILL).name(p.name.clone()));
    }
    panel(scroll(column(items).spacing(1.0).width(Length::FILL))).color(t.card).radius(8.0).padding(6.0).width(Length::Fixed(160.0)).height(Length::FILL)
}

/// A folder's size once counted, for the list's Size column.
pub type FolderSize<'a> = &'a dyn Fn(&Entry) -> Option<u64>;
/// The line under a row's name ([`View::under`]).
pub type Under<'a> = &'a dyn Fn(&Entry) -> Option<String>;
/// A row's own controls ([`View::trailing`]).
pub type Trailing<'a, M> = &'a dyn Fn(usize, &Entry) -> Element<M>;

/// How the list or icon view shows its entries.
pub struct View<'a, M> {
    pub entries: &'a [&'a Entry],
    /// Which rows are chosen.
    pub chosen: &'a dyn Fn(usize, &Entry) -> bool,
    /// The row to keep in view (the one the choice just moved to), and a key that changes with it.
    pub in_view: Option<(usize, u64)>,
    pub click: &'a dyn Fn(usize) -> M,
    /// What pulling a row away sends (the start of a drag), if rows can be dragged.
    pub drag: Option<&'a dyn Fn(usize) -> M>,
    /// Show each file's folder under its name (the recent files).
    pub folders: bool,
    /// A file's thumbnail, if it has one made (the icon view shows it in
    /// place of its mark; files milestone 1, step 10).
    pub thumb: Option<&'a dyn Fn(&Entry) -> Option<std::sync::Arc<vergence_scene::Image>>>,
    /// What a right click on a row sends (its menu), if rows have one.
    pub menu: Option<&'a dyn Fn(usize) -> M>,
    /// How the list looks.
    pub look: &'a ListLook,
    /// A folder's size, once counted (shown as "—" until then).
    pub folder_size: Option<FolderSize<'a>>,
    /// A column's edge pulled (its place in [`COLUMNS`], the touch), if the
    /// widths can be changed.
    pub grip: Option<fn(usize, crate::Touch) -> Option<M>>,
    /// The line under each name in place of its folder (with `folders`):
    /// the Trash's "where it came from".
    pub under: Option<Under<'a>>,
    /// The date column's heading, if not "Date Changed" (the Trash's
    /// "Deleted", whose entries carry the time they were trashed).
    pub changed_heading: Option<&'static str>,
    /// A row's own controls at its end (its index, the entry), with the room
    /// they take kept in the headings too.
    pub trailing: Option<(Trailing<'a, M>, f32)>,
}

/// The list (Details): headings that sort, with grips between them that
/// change the columns' widths, then a row per entry: its mark, name, when it
/// changed, its size and its kind; striped, grouped under headings if the
/// look says, the stripes carried on into the room below the last row.
pub fn list<M: Clone + 'static>(v: &View<M>, sort: (SortBy, bool), by: impl Fn(SortBy) -> M) -> (Element<M>, Vec<Element<M>>) {
    let look = v.look;
    let rs = look.rows;
    let shown: Vec<usize> = (0..COLUMNS.len()).filter(|&c| COLUMNS[c].1 != SortBy::Kind || look.kind).collect();
    let heading = |text: &str, s: SortBy, w: Length, end: bool| -> Element<M> {
        let mark = if sort.0 == s { if sort.1 { " ▲" } else { " ▼" } } else { "" };
        button(label(format!("{text}{mark}")).size(11.0).dim().align(if end { Align::End } else { Align::Start }).width(Length::FILL)).look(ButtonLook::Plain).padding(Padding::hv(2.0, 0.0)).on_press(by(s)).width(w).name(format!("Sort by {text}"))
    };
    // Headings and rows are built alike, so each column's words sit under its
    // heading: the mark's room, the name, then for each column an edge's room
    // (GAP) and the column; the rows' side padding (PAD) on the headings too.
    const GAP: f32 = 14.0;
    const PAD: f32 = 6.0;
    let icon_w = 20.0 * rs.mark() + 6.0;
    let mut head_items: Vec<Element<M>> = vec![space().width(Length::Fixed(icon_w)), heading("Name", SortBy::Name, Length::FILL, false)];
    for &c in &shown {
        let (text, s) = COLUMNS[c];
        let text = if s == SortBy::Changed { v.changed_heading.unwrap_or(text) } else { text };
        // The edge before the column: pulled, the column grows or shrinks.
        let mut g = drawing(|scene, _, r: Rect, t: &Theme| {
            scene.fill(Rect::new(r.x + r.w / 2.0 - 0.5, r.y + 4.0, 1.0, (r.h - 8.0).max(0.0)), t.line);
        }).width(Length::Fixed(GAP)).height(Length::Fixed(20.0));
        if let Some(grip) = v.grip {
            let f = move |t: crate::Touch| grip(c, t);
            g = g.on_touch(f).cursor(vergence_ui::Cursor::ColResize).name(format!("Width of {text}"));
        }
        head_items.push(g);
        head_items.push(heading(text, s, Length::Fixed(look.widths[c]), s == SortBy::Size));
    }
    if let Some((_, w)) = v.trailing {
        head_items.push(space().width(Length::Fixed(GAP + w)));
    }
    let head = panel(row(head_items).spacing(0.0).align(Align::Center).width(Length::FILL)).padding(Padding::hv(0.0, PAD)).width(Length::FILL);
    // A gray that darkens a light ground and lightens a dark one alike.
    const STRIPE: Color = Color { r: 128, g: 128, b: 128, a: 18 };
    let mut rows = Vec::with_capacity(v.entries.len() + 8);
    let mut odd = false;
    let mut group: Option<u8> = None;
    let row_h = if v.folders { rs.height().max(36.0) } else { rs.height() };
    for (k, e) in v.entries.iter().enumerate() {
        if look.group {
            let (g, name) = group_of(e);
            if group != Some(g) {
                group = Some(g);
                odd = false;
                rows.push(panel(label(name).size(11.0).weight(600).dim()).padding(Padding { top: if k == 0 { 2.0 } else { 12.0 }, bottom: 4.0, left: 10.0, right: 0.0 }).width(Length::FILL));
            }
        }
        let mut name_col: Vec<Element<M>> = vec![label(e.name.clone()).size(rs.text()).elide().width(Length::FILL)];
        if v.folders {
            let words = match v.under {
                Some(f) => f(e),
                None => e.path.parent().map(tilde),
            };
            if let Some(w) = words {
                name_col.push(label(w).size(10.0).dim().elide().width(Length::FILL));
            }
        }
        let small = rs.text() - 1.0;
        let mut cells: Vec<Element<M>> = vec![
            entry_icon_sized(e, rs.mark()).width(Length::Fixed(icon_w)).height(Length::Fixed(18.0 * rs.mark())),
            column(name_col).width(Length::FILL),
        ];
        for &c in &shown {
            let w = Length::Fixed(look.widths[c]);
            cells.push(space().width(Length::Fixed(GAP)));
            cells.push(match COLUMNS[c].1 {
                SortBy::Changed => label(e.changed.map(when).unwrap_or_default()).size(small).dim().elide().width(w),
                SortBy::Size => {
                    let words = if e.dir { v.folder_size.and_then(|f| f(e)).map_or_else(|| "—".to_string(), size_words) } else { size_words(e.size) };
                    label(words).size(small).dim().align(Align::End).width(w)
                }
                _ => label(kind_words(e)).size(small).dim().elide().width(w),
            });
        }
        if let Some((f, w)) = v.trailing {
            cells.push(space().width(Length::Fixed(GAP)));
            cells.push(f(k, e).width(Length::Fixed(w)));
        }
        let chosen = (v.chosen)(k, e);
        let ground = if chosen {
            ButtonLook::Chosen
        } else if look.stripes && odd {
            ButtonLook::Custom { fill: Some(STRIPE), edge: None, radius: 6.0 }
        } else {
            ButtonLook::Plain
        };
        let mut b = button(row(cells).spacing(0.0).align(Align::Center).width(Length::FILL))
            .look(ground)
            .padding(Padding::hv(0.0, PAD))
            .on_press((v.click)(k))
            .width(Length::FILL)
            .height(Length::Fixed(row_h))
            .name(e.name.clone());
        odd = !odd;
        if let Some(d) = v.drag {
            b = b.on_drag(d(k));
        }
        if let Some(m) = v.menu {
            b = b.on_menu(m(k));
        }
        if let Some((i, key)) = v.in_view
            && i == k
        {
            // A new row each time the choice moves, so it is scrolled to.
            b = b.key(key).keep_in_view();
        }
        rows.push(b);
    }
    // The stripes go on below the last row, through the room that is left.
    if look.stripes {
        let pitch = row_h + 1.0;
        let first_odd = odd;
        rows.push(
            drawing(move |scene, _, r: Rect, _: &Theme| {
                let mut y = r.y;
                let mut on = first_odd;
                while y < r.y + r.h {
                    if on {
                        scene.fill_rounded(Rect::new(r.x, y, r.w, row_h.min(r.y + r.h - y)), 6.0, STRIPE);
                    }
                    on = !on;
                    y += pitch;
                }
            })
            .width(Length::FILL)
            .height(Length::FILL),
        );
    }
    (head, rows)
}

/// The icon view: a cell per entry, a large mark over its name, the cells
/// flowing in rows as wide as the place.
pub fn icons<M: Clone + 'static>(v: &View<M>) -> Element<M> {
    let cells: Vec<Element<M>> = v
        .entries
        .iter()
        .enumerate()
        .map(|(k, e)| {
            // One line, cut with "…" to the cell (a name without spaces cannot wrap).
            let mark = match v.thumb.and_then(|t| t(e)) {
                Some(img) => crate::picture(img).width(Length::Fixed(80.0)).height(Length::Fixed(52.0)),
                None => entry_icon_sized(e, 3.0).width(Length::Fixed(80.0)).height(Length::Fixed(52.0)),
            };
            let mut b = button(column(vec![mark, label(e.name.clone()).size(11.0).align(Align::Center).elide().width(Length::Fixed(88.0))]).spacing(4.0).align(Align::Center))
                .look(if (v.chosen)(k, e) { ButtonLook::Chosen } else { ButtonLook::Plain })
                .padding(Padding::hv(6.0, 4.0))
                .on_press((v.click)(k))
                .width(Length::Fixed(100.0))
                .name(e.name.clone());
            if let Some(m) = v.menu {
                b = b.on_menu(m(k));
            }
            if let Some(d) = v.drag {
                b = b.on_drag(d(k));
            }
            if let Some((i, key)) = v.in_view
                && i == k
            {
                b = b.key(key).keep_in_view();
            }
            b
        })
        .collect();
    row(cells).spacing(6.0).wrap().width(Length::FILL)
}

/// A list entry's type by its name: a folder, or what the name says.
pub fn kind_of(dir: bool, name: &str) -> Option<&'static str> {
    if dir { Some(vergence_types::FOLDER) } else { vergence_types::of_name(name) }
}

/// The families of file types the list draws a mark for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Text,
    Picture,
    Sound,
    Video,
    Archive,
    Document,
    Program,
    Other,
}

impl Kind {
    /// The family of a type, by its generic icon (vergence-types).
    pub fn of(mime: Option<&str>) -> Kind {
        let Some(m) = mime else { return Kind::Other };
        match vergence_types::icons(m)[1].as_str() {
            "folder" => Kind::Folder,
            "text-x-generic" | "text-x-script" | "text-html" => Kind::Text,
            "image-x-generic" => Kind::Picture,
            "audio-x-generic" => Kind::Sound,
            "video-x-generic" => Kind::Video,
            "package-x-generic" | "media-optical" | "drive-harddisk" => Kind::Archive,
            "x-office-document" | "x-office-spreadsheet" | "x-office-presentation" | "x-office-calendar" | "x-office-address-book" => Kind::Document,
            "application-x-executable" if m != vergence_types::BINARY => Kind::Program,
            _ => Kind::Other,
        }
    }
}

/// A small mark for a family of files, drawn: a folder in the accent
/// color; a page, with lines for text, a hill and sun for a picture, a note
/// for sound, a play mark for video, a band for an archive, a heading for a
/// document; a window for a program.
pub fn kind_icon<M: 'static>(kind: Kind) -> Element<M> {
    kind_icon_sized(kind, 1.0).width(Length::Fixed(20.0)).height(Length::Fixed(18.0))
}

/// The same mark drawn `k` times as large (the icon view's), centered in its
/// place: Vergence's own icon for the kind where it is installed
/// (own_icon), else the mark drawn here.
pub fn kind_icon_sized<M: 'static>(kind: Kind, k: f32) -> Element<M> {
    icon_sized(kind, own_icon_name(kind), k)
}

/// A file's mark: its kind's, or its own type's where Vergence has an icon
/// for that (a PDF's red band).
pub fn entry_icon_sized<M: 'static>(e: &Entry, k: f32) -> Element<M> {
    let own = match e.mime {
        Some("application/pdf") => ("kinds", "pdf"),
        _ if e.dir => special_folder(&e.path).unwrap_or(("places", "folder")),
        _ => own_icon_name(e.kind()),
    };
    icon_sized(e.kind(), own, k)
}

fn icon_sized<M: 'static>(kind: Kind, (group, name): (&'static str, &'static str), k: f32) -> Element<M> {
    drawing(move |scene, _, r: Rect, t: &Theme| {
        // The icon's square: the mark's box (18 x 15 points times k), at most the place.
        let side = (18.0 * k).min(r.w).min(r.h.max(15.0 * k));
        // Pixels enough for a 2x screen and then some (the renderer shrinks it).
        let px = (side * 2.5).ceil() as u32;
        match own_icon(group, name, px) {
            Some(img) => {
                let id = scene.image(img);
                scene.draw_image(id, Rect::new(r.x + (r.w - side) / 2.0, r.y + (r.h - side) / 2.0, side, side), 1.0);
            }
            None => draw_kind(scene, r, k, kind, t),
        }
    })
}

/// Which of Vergence's own icons (tools/make-icons.py) a kind shows.
pub fn own_icon_name(kind: Kind) -> (&'static str, &'static str) {
    match kind {
        Kind::Folder => ("places", "folder"),
        Kind::Program => ("kinds", "program"),
        Kind::Text => ("kinds", "text"),
        Kind::Picture => ("kinds", "picture"),
        Kind::Sound => ("kinds", "sound"),
        Kind::Video => ("kinds", "video"),
        Kind::Archive => ("kinds", "archive"),
        Kind::Document => ("kinds", "document"),
        Kind::Other => ("kinds", "other"),
    }
}

/// The user's own folders' icons (Desktop, Documents ... by user_dirs,
/// read once): a folder that is one of them shows its own, as the sidebar.
fn special_folder(path: &Path) -> Option<(&'static str, &'static str)> {
    thread_local! {
        static DIRS: Vec<(String, PathBuf)> = user_dirs();
    }
    let name = DIRS.with(|d| d.iter().find(|(_, p)| p == path).map(|(n, _)| n.clone()))?;
    Some(("places", match name.as_str() {
        "Desktop" => "desktop",
        "Documents" => "documents",
        "Downloads" => "downloads",
        "Pictures" => "pictures",
        "Music" => "music",
        "Videos" => "videos",
        _ => return None,
    }))
}

thread_local! {
    /// Own icons read so far, by group, name and size (None: not installed).
    static OWN_ICONS: std::cell::RefCell<std::collections::HashMap<(String, u32), Option<std::sync::Arc<vergence_scene::Image>>>> = Default::default();
}

/// One of Vergence's own icons at the smallest size drawn at install that is
/// at least `px` (vergence-harness icons: <name>.<px>.rgba in
/// share/vergence/icons/<group>), read once. None where it is not installed.
pub fn own_icon(group: &str, name: &str, px: u32) -> Option<std::sync::Arc<vergence_scene::Image>> {
    let size = [16u32, 32, 64, 128, 256].into_iter().find(|s| *s >= px).unwrap_or(256);
    // Another icon theme chosen in Settings: its icon, once decoded (Dave,
    // 2026-11-18: "our apps follow it"); ours until then, or if it has none.
    let theme = icon_theme();
    if theme != vergence_shell::icons::THEME
        && let Some(found) = themed_icon(&theme, group, name, size)
    {
        return Some(found);
    }
    vergence_own_icon(group, name, size)
}

thread_local! {
    /// The icon theme chosen, and when it was last read.
    static THEME_READ: std::cell::RefCell<Option<(std::time::Instant, String)>> = const { std::cell::RefCell::new(None) };
}

/// The icon theme chosen in Settings, read again at most every 2 s (a change
/// there reaches open windows at their next drawing).
pub fn icon_theme() -> String {
    THEME_READ.with(|t| {
        let mut t = t.borrow_mut();
        if let Some((at, name)) = t.as_ref()
            && at.elapsed() < std::time::Duration::from_secs(2)
        {
            return name.clone();
        }
        let name = vergence_shell::icons::chosen_theme();
        *t = Some((std::time::Instant::now(), name.clone()));
        name
    })
}

/// Icons of other themes, by theme, group, name and size: decoded (None:
/// the theme has none, ours is shown), and those being decoded.
type Themed = std::collections::HashMap<(String, String, String, u32), Option<std::sync::Arc<vergence_scene::Image>>>;
type Decoding = std::collections::HashSet<(String, String, String, u32)>;
static THEMED: std::sync::LazyLock<std::sync::Mutex<(Themed, Decoding)>> = std::sync::LazyLock::new(Default::default);

/// One of our icons from theme `theme`: decoded on a worker (the sandboxed
/// decoder, vergence-decode, as the dock's), None until it is, or if the
/// theme has no icon of that name.
fn themed_icon(theme: &str, group: &str, name: &str, size: u32) -> Option<std::sync::Arc<vergence_scene::Image>> {
    let key = (theme.to_string(), group.to_string(), name.to_string(), size);
    let mut all = THEMED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = all.0.get(&key) {
        return found.clone();
    }
    if !all.1.insert(key.clone()) {
        return None;
    }
    drop(all);
    std::thread::spawn(move || {
        let (theme, group, name, size) = (&key.0, &key.1, &key.2, key.3);
        let ours = vergence_shell::icons::vergence_icon(group, name);
        let img = vergence_shell::icons::themed(theme, group, name, size)
            .filter(|p| Some(p) != ours.as_ref())
            .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() <= 4 << 20))
            .and_then(|p| vergence_decode::decode_file(&p, Some((size, size))).ok())
            .and_then(|p| {
                let f = p.frames.into_iter().next()?;
                (p.width > 0 && p.height > 0 && p.width <= 1024 && p.height <= 1024).then(|| std::sync::Arc::new(vergence_scene::Image { width: p.width, height: p.height, rgba: f.rgba }))
            });
        let mut all = THEMED.lock().unwrap_or_else(|e| e.into_inner());
        all.1.remove(&key);
        all.0.insert(key, img);
    });
    None
}

/// One of Vergence's own icons, its pixels drawn at install.
fn vergence_own_icon(group: &str, name: &str, size: u32) -> Option<std::sync::Arc<vergence_scene::Image>> {
    let key = (format!("{group}/{name}"), size);
    if let Some(found) = OWN_ICONS.with(|c| c.borrow().get(&key).cloned()) {
        return found;
    }
    let file = format!("{group}/{name}.{size}.rgba");
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
    {
        dirs.push(bin.join("../share/vergence/icons"));
    }
    let data = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(data.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("vergence/icons")));
    let img = dirs.into_iter().find_map(|d| std::fs::read(d.join(&file)).ok()).and_then(|b| {
        if b.len() < 12 || &b[..4] != b"VRGB" {
            return None;
        }
        let w = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        let h = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
        (b.len() == 12 + (w * h * 4) as usize && w > 0 && h > 0).then(|| std::sync::Arc::new(vergence_scene::Image { width: w, height: h, rgba: b[12..].to_vec() }))
    });
    OWN_ICONS.with(|c| c.borrow_mut().insert(key, img.clone()));
    img
}

/// A family's mark drawn into a scene, `k` times its small size, centered in `r`.
pub fn draw_kind(scene: &mut vergence_scene::Scene, r: Rect, k: f32, kind: Kind, t: &Theme) {
    {
        // Drawn in a box 18 x 15 points at scale 1, every length times k.
        let (bw, bh) = (18.0 * k, 15.0 * k);
        let (ox, oy) = (r.x + (r.w - bw) / 2.0 + k, r.y + (r.h - bh) / 2.0);
        let rr = |x: f32, y: f32, w: f32, h: f32| Rect::new(ox + x * k, oy + y * k, w * k, h * k);
        let (x, y) = (0.0f32, 0.0f32);
        let mark = Color { a: 170, ..t.dim };
        match kind {
            Kind::Folder => {
                let c = t.accent;
                scene.fill_rounded(rr(x, y + 1.0, 7.0, 4.0), 1.5 * k, c);
                scene.fill_rounded(rr(x, y + 3.0, 16.0, 11.0), 2.0 * k, c);
            }
            Kind::Program => {
                scene.fill_rounded(rr(x, y + 1.0, 16.0, 13.0), 2.0 * k, t.dim);
                scene.fill_rounded(rr(x + 1.0, y + 4.0, 14.0, 9.0), 1.0 * k, t.page);
                scene.fill(rr(x + 3.0, y + 6.0, 4.0, 1.0), mark);
                scene.fill(rr(x + 3.0, y + 9.0, 7.0, 1.0), mark);
            }
            _ => {
                scene.fill_rounded(rr(x + 2.0, y, 12.0, 15.0), 2.0 * k, t.dim);
                scene.fill_rounded(rr(x + 3.0, y + 1.0, 10.0, 13.0), 1.5 * k, t.page);
                let (cx, cy) = (x + 8.0, y + 7.5);
                match kind {
                    Kind::Text | Kind::Other => {
                        let lines = if kind == Kind::Text { 3 } else { 0 };
                        for i in 0..lines {
                            scene.fill(rr(x + 5.0, y + 4.0 + i as f32 * 3.0, 6.0, 1.0), mark);
                        }
                    }
                    Kind::Picture => {
                        scene.fill_rounded(rr(x + 9.0, y + 3.0, 3.0, 3.0), 1.5 * k, t.accent);
                        scene.fill_rounded(rr(x + 4.0, y + 8.0, 8.0, 4.0), 1.0 * k, Color { a: 200, ..t.accent });
                    }
                    Kind::Sound => {
                        scene.fill_rounded(rr(cx - 3.0, cy + 1.0, 4.0, 3.0), 1.5 * k, mark);
                        scene.fill(rr(cx, cy - 4.0, 1.2, 6.0), mark);
                        scene.fill(rr(cx, cy - 4.0, 3.0, 1.2), mark);
                    }
                    Kind::Video => {
                        for i in 0..4 {
                            let h = 6.0 - i as f32 * 1.5;
                            scene.fill(rr(cx - 2.0 + i as f32, cy - h / 2.0, 1.0, h), mark);
                        }
                    }
                    Kind::Archive => {
                        scene.fill(rr(cx - 1.0, y + 1.0, 2.0, 13.0), mark);
                        scene.fill(rr(cx - 2.0, y + 9.0, 4.0, 3.0), mark);
                    }
                    Kind::Document => {
                        scene.fill(rr(x + 5.0, y + 3.5, 6.0, 2.0), mark);
                        for i in 0..2 {
                            scene.fill(rr(x + 5.0, y + 7.5 + i as f32 * 2.5, 6.0, 1.0), mark);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute())
}

/// A path as shown: ~ for Home.
pub fn tilde(p: &Path) -> String {
    match home().and_then(|h| p.strip_prefix(&h).ok().map(Path::to_path_buf)) {
        Some(r) if r.as_os_str().is_empty() => "~".into(),
        Some(r) => format!("~/{}", r.display()),
        None => p.display().to_string(),
    }
}

pub fn looks_like_path(s: &str) -> bool {
    s.starts_with('/') || s.starts_with('~')
}

/// A typed path made whole: ~ is Home; anything else is from `dir`.
pub fn expand(s: &str, dir: Option<&Path>) -> PathBuf {
    if s == "~" {
        return home().unwrap_or_else(|| PathBuf::from("/"));
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return home().unwrap_or_else(|| PathBuf::from("/")).join(rest);
    }
    let p = PathBuf::from(s);
    if p.is_absolute() { p } else { dir.map_or(p.clone(), |d| d.join(&p)) }
}

/// Names in the order people expect: without case, numbers by their value.
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n = String::new();
                let mut m = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n.push(c);
                    x.next();
                }
                while let Some(d) = y.peek().copied().filter(char::is_ascii_digit) {
                    m.push(d);
                    y.next();
                }
                let (n2, m2) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n2.len().cmp(&m2.len()).then_with(|| n2.cmp(m2));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

pub fn size_words(n: u64) -> String {
    match n {
        0..1000 => format!("{n} bytes"),
        1000..1_000_000 => format!("{:.0} KB", n as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1} MB", n as f64 / 1e6),
        _ => format!("{:.1} GB", n as f64 / 1e9),
    }
}

/// When a file changed, in words: "Today 14:03", "Yesterday 09:12",
/// "Nov 3", "2025-11-03".
pub fn when(t: SystemTime) -> String {
    let secs = |t: SystemTime| t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let (Some(at), Some(now)) = (local(secs(t)), local(secs(SystemTime::now()))) else { return String::new() };
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let day = |d: &Local| (d.year, d.yday);
    if day(&at) == day(&now) {
        format!("Today {:02}:{:02}", at.hour, at.min)
    } else if (at.year == now.year && at.yday + 1 == now.yday) || local(secs(SystemTime::now()) - 86_400).is_some_and(|y| day(&y) == day(&at)) {
        format!("Yesterday {:02}:{:02}", at.hour, at.min)
    } else if at.year == now.year {
        format!("{} {}", MONTHS[at.month.min(11) as usize], at.mday)
    } else {
        format!("{}-{:02}-{:02}", at.year, at.month + 1, at.mday)
    }
}

pub struct Local {
    pub year: i32,
    /// 0 for January.
    pub month: i32,
    pub mday: i32,
    pub yday: i32,
    pub hour: i32,
    pub min: i32,
    pub sec: i32,
}

/// A moment as local "YYYY-MM-DDThh:mm:ss" (the freedesktop Trash's DeletionDate).
pub fn local_stamp(t: SystemTime) -> String {
    let secs = t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    match local(secs) {
        Some(l) => format!("{}-{:02}-{:02}T{:02}:{:02}:{:02}", l.year, l.month + 1, l.mday, l.hour, l.min, l.sec),
        None => String::new(),
    }
}

/// The local time of `secs` since 1970, by localtime_r(3) (glibc's and
/// FreeBSD's struct tm agree on these fields and their order).
pub fn local(secs: i64) -> Option<Local> {
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: i64,
        zone: *const std::ffi::c_char,
    }
    unsafe extern "C" {
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }
    let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
    // SAFETY: localtime_r reads the time and fills the struct we own.
    let r = unsafe { localtime_r(&secs, &mut tm) };
    (!r.is_null()).then_some(Local { year: tm.year + 1900, month: tm.mon, mday: tm.mday, yday: tm.yday, hour: tm.hour, min: tm.min, sec: tm.sec })
}

/// The start of a file, if it is text.
pub fn text_start(p: &Path) -> Option<String> {
    use std::io::Read;
    let mut buf = vec![0u8; PREVIEW_BYTES];
    let n = fs::File::open(p).ok()?.read(&mut buf).ok()?;
    buf.truncate(n);
    if buf.contains(&0) {
        return None;
    }
    // Cut at a character's edge.
    let s = match std::str::from_utf8(&buf) {
        Ok(s) => s.to_string(),
        Err(e) if e.valid_up_to() + 4 >= buf.len() => String::from_utf8_lossy(&buf[..e.valid_up_to()]).into_owned(),
        Err(_) => return None,
    };
    Some(s)
}

/// One line of the system's mount table: what is mounted, where, and its
/// file system's type.
#[derive(Clone, Debug, PartialEq)]
pub struct Mount {
    pub what: String,
    pub at: PathBuf,
    pub kind: String,
}

/// A mount table's text read: Linux /proc/self/mounts, or FreeBSD's
/// `mount -p` (fstab's form); the same columns. Octal escapes (\040 a
/// space) undone.
pub fn parse_mounts(text: &str) -> Vec<Mount> {
    let unescape = |s: &str| -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
                out.push(((b[i + 1] - b'0') << 6) | ((b[i + 2] - b'0') << 3) | (b[i + 3] - b'0'));
                i += 4;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    text.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let mut what = f.next()?.to_string();
            // FreeBSD's automounter: "map -media  /media  autofs ..." (a space in the first field).
            if what == "map" {
                what = format!("map {}", f.next()?);
            }
            let (at, kind) = (f.next()?, f.next()?);
            (!what.starts_with('#')).then(|| Mount { what: unescape(&what), at: PathBuf::from(unescape(at)), kind: kind.to_string() })
        })
        .collect()
}

/// The system's mounts now.
pub fn mounts() -> Vec<Mount> {
    let text = fs::read_to_string("/proc/self/mounts").ok().or_else(|| {
        // FreeBSD: mount -p prints the table in fstab's form. By its full
        // path: a session's PATH has no /sbin (found on techbench 2026-11-19:
        // no drive ever showed on the desktop or in FileView), and a PATH
        // is not a thing to trust for a system tool anyway.
        let mount = if Path::new("/sbin/mount").exists() { "/sbin/mount" } else { "mount" };
        std::process::Command::new(mount).arg("-p").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    });
    parse_mounts(&text.unwrap_or_default())
}

/// Whether a file system type is one reached over the network (NFS, SMB,
/// sshfs and other FUSE network file systems, WebDAV).
pub fn network_kind(kind: &str) -> bool {
    matches!(kind, "nfs" | "nfs4" | "cifs" | "smb3" | "smbfs" | "davfs" | "9p" | "afs") || ["fuse.sshfs", "fusefs.sshfs", "fuse.rclone", "fusefs.rclone", "fuse.gvfsd-fuse"].contains(&kind)
}

/// Disks mounted where removable and extra disks go (network mounts aside).
pub fn mounted_disks() -> Vec<PathBuf> {
    disks_of(&mounts())
}

fn disks_of(table: &[Mount]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = table
        .iter()
        .filter(|m| !network_kind(&m.kind) && m.kind != "autofs")
        .map(|m| m.at.clone())
        .filter(|m| {
            let s = m.to_string_lossy();
            ["/media/", "/mnt/", "/run/media/"].iter().any(|p| s.starts_with(p)) || s == "/mnt"
        })
        .collect();
    // FreeBSD's automounter (vergence-media; install.sh): /media lists the
    // drives plugged in before they are mounted (looking into one mounts it).
    if table.iter().any(|m| m.at == Path::new("/media") && m.kind == "autofs")
        && let Ok(rd) = fs::read_dir("/media")
    {
        // A name stays listed after its drive is pulled out: kept only while
        // it is mounted or its device is there (by name, or by its label).
        let mounted: Vec<&Path> = table.iter().map(|m| m.at.as_path()).collect();
        let present = |name: &std::ffi::OsStr| {
            std::iter::once(PathBuf::from("/dev").join(name)).chain(["msdosfs", "ufs", "ext2fs", "ntfs", "exfat", "iso9660", "label", "gpt"].iter().map(|d| PathBuf::from("/dev").join(d).join(name))).any(|p| p.exists())
        };
        out.extend(rd.flatten().map(|e| e.path()).filter(|p| mounted.contains(&p.as_path()) || p.file_name().is_some_and(present)));
    }
    out.sort();
    out.dedup();
    out
}

/// What the system has mounted over the network, anywhere (files
/// milestone 1, step 10: shown under Network in the sidebar).
pub fn network_mounts() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = mounts().into_iter().filter(|m| network_kind(&m.kind)).map(|m| m.at).collect();
    out.sort();
    out.dedup();
    out
}

/// Whether a folder is a disk or network mount that Eject can let go of.
pub fn ejectable(path: &Path) -> bool {
    let table = mounts();
    table.iter().any(|m| m.at == path && network_kind(&m.kind)) || disks_of(&table).iter().any(|d| d == path)
}

/// Unmounts a disk or a network mount: a removable drive through
/// vergence-media (udisks2, the automounter); sshfs and other FUSE mounts by
/// `fusermount3 -u` / `fusermount -u` on Linux, everything else (and
/// everything on FreeBSD) by `umount`. The system decides whether the user
/// may: the error says why not.
pub fn eject(path: &Path) -> Result<(), String> {
    // A drive the system's media service mounted (udisks2 on Linux, the
    // automounter on FreeBSD) is let go of through it, and powered off.
    if vergence_media::eject(path)? {
        return Ok(());
    }
    let kind = mounts().into_iter().find(|m| m.at == path).map(|m| m.kind).unwrap_or_default();
    let mut tries: Vec<(&str, Vec<&str>)> = Vec::new();
    if kind.starts_with("fuse.") && cfg!(target_os = "linux") {
        tries.push(("fusermount3", vec!["-u"]));
        tries.push(("fusermount", vec!["-u"]));
    }
    tries.push(("umount", vec![]));
    let mut why = String::new();
    for (program, args) in tries {
        match std::process::Command::new(program).args(&args).arg(path).stdin(std::process::Stdio::null()).output() {
            Ok(o) if o.status.success() => return Ok(()),
            Ok(o) => why = String::from_utf8_lossy(&o.stderr).trim().to_string(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => why = e.to_string(),
        }
    }
    Err(if why.is_empty() { "it could not be unmounted.".into() } else { why })
}

pub fn plain(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "you do not have permission.".into(),
        std::io::ErrorKind::NotFound => "it is not there.".into(),
        std::io::ErrorKind::AlreadyExists => "something of that name is already here.".into(),
        _ => {
            let s = e.to_string();
            format!("{}.", s.split(" (os error").next().unwrap_or(&s).trim_end_matches('.'))
        }
    }
}

// --- What is remembered ---

fn state_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| home().map(|h| h.join(".local/state")))?;
    Some(base.join("vergence/file-dialog"))
}

/// The state file's lines: "folder <app>\t<path>" and "file\t<path>", newest files first.
fn state_lines() -> Vec<(String, String)> {
    let Some(p) = state_path() else { return Vec::new() };
    fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter(|(_, v)| v.starts_with('/') && v.len() <= 4096)
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .take(256)
        .collect()
}

pub fn remembered_folder(app: &str) -> Option<PathBuf> {
    let key = format!("folder {app}");
    state_lines().into_iter().find(|(k, _)| *k == key).map(|(_, v)| PathBuf::from(v))
}

pub fn recent_files() -> Vec<PathBuf> {
    state_lines().into_iter().filter(|(k, _)| k == "file").map(|(_, v)| PathBuf::from(v)).collect()
}

/// Keeps `app`'s folder and puts `file` at the top of the recent files.
pub fn remember(app: &str, dir: &Path, file: &Path) {
    let Some(path) = state_path() else { return };
    let key = format!("folder {app}");
    let (dir, file) = (dir.to_string_lossy().into_owned(), file.to_string_lossy().into_owned());
    if dir.contains(['\t', '\n']) || file.contains(['\t', '\n']) {
        return;
    }
    let old = state_lines();
    let mut lines = vec![(key.clone(), dir), ("file".to_string(), file.clone())];
    lines.extend(old.iter().filter(|(k, _)| *k != key && k.starts_with("folder ")).cloned());
    lines.extend(old.iter().filter(|(k, v)| k == "file" && *v != file).take(MAX_RECENT - 1).cloned());
    let text: String = lines.iter().map(|(k, v)| format!("{k}\t{v}\n")).collect();
    if let Some(d) = path.parent() {
        let _ = fs::create_dir_all(d);
    }
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("new");
    let written = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).and_then(|mut f| f.write_all(text.as_bytes()));
    if written.is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}


#[cfg(test)]
mod tests {
    #[test]
    fn the_mount_table_tells_disks_from_the_network() {
        let linux = "/dev/sda1 / ext4 rw 0 0\n/dev/sdb1 /run/media/u/My\\040Disk vfat rw 0 0\nhost:/srv /mnt/srv nfs4 rw 0 0\nu@box:/home /home/u/box fuse.sshfs rw 0 0\n//nas/share /mnt/nas cifs rw 0 0\n";
        let t = super::parse_mounts(linux);
        assert_eq!(t[1].at, std::path::PathBuf::from("/run/media/u/My Disk"));
        assert_eq!(super::disks_of(&t), vec![std::path::PathBuf::from("/run/media/u/My Disk")], "the network's mounts under /mnt are not disks");
        let net: Vec<_> = t.iter().filter(|m| super::network_kind(&m.kind)).map(|m| m.at.to_string_lossy().into_owned()).collect();
        assert_eq!(net, vec!["/mnt/srv", "/home/u/box", "/mnt/nas"]);
        // FreeBSD's mount -p.
        let bsd = "zroot/ROOT/default\t/\tzfs\trw\t0 0\nbox:/home\t/mnt/box\tfusefs.sshfs\trw\t0 0\n/dev/da0s1\t/media/usb\tmsdosfs\trw\t0 0\n";
        let t = super::parse_mounts(bsd);
        assert_eq!(super::disks_of(&t), vec![std::path::PathBuf::from("/media/usb")]);
        assert!(super::network_kind(&t[1].kind));
        let auto = super::parse_mounts("map -vergence_media\t/media\t\t\tautofs\trw\t\t0 0\n");
        assert_eq!((auto[0].what.as_str(), auto[0].at.to_str(), auto[0].kind.as_str()), ("map -vergence_media", Some("/media"), "autofs"));
    }

    use super::*;

    #[test]
    fn a_file_with_no_extension_is_known_by_what_is_in_it() {
        let d = std::env::temp_dir().join(format!("vergence-files-sniff-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("Vergence-wishlist-2"), "Vergence Wish List:\nsome lines\n").unwrap();
        fs::write(d.join("blob"), [0u8, 1, 2, 3, 0, 9]).unwrap();
        fs::write(d.join("picture"), b"\x89PNG\r\n\x1a\n....").unwrap();
        let v = read_folder(&d).unwrap();
        let by = |n: &str| v.iter().find(|e| e.name == n).unwrap();
        assert_eq!(by("Vergence-wishlist-2").kind(), Kind::Text);
        assert_eq!(by("picture").kind(), Kind::Picture);
        assert_eq!(by("blob").kind(), Kind::Other);
        // Not looked into: by the name alone, nothing.
        assert_eq!(Entry::of_looking(d.join("Vergence-wishlist-2"), false).unwrap().mime, None);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn entries_sort_folders_first_then_as_asked() {
        let e = |name: &str, dir: bool, size: u64| Entry { name: name.into(), path: PathBuf::from(format!("/x/{name}")), dir, mime: kind_of(dir, name), size, changed: None };
        let all = [e("b10.txt", false, 5), e("b2.txt", false, 50), e("zeta", true, 0), e("Alpha", true, 0)];
        let mut v: Vec<&Entry> = all.iter().collect();
        sort(&mut v, SortBy::Name, true, false, false);
        assert_eq!(v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["Alpha", "zeta", "b2.txt", "b10.txt"]);
        sort(&mut v, SortBy::Size, false, false, false);
        assert_eq!(v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["Alpha", "zeta", "b2.txt", "b10.txt"], "folders stay first; biggest first");
        assert_eq!(all[2].kind(), Kind::Folder);
        assert_eq!(all[0].kind(), Kind::Text);
    }

    #[test]
    fn details_sort_by_kind_group_and_keep_their_widths() {
        let e = |name: &str, dir: bool| Entry { name: name.into(), path: PathBuf::from(format!("/x/{name}")), dir, mime: kind_of(dir, name), size: 0, changed: None };
        let all = [e("notes.txt", false), e("photo.png", false), e("Music", true), e("a.png", false), e("letter.odt", false)];
        let mut v: Vec<&Entry> = all.iter().collect();
        // By kind: folders first, then by what the type is called, then by name.
        sort(&mut v, SortBy::Kind, true, false, false);
        assert_eq!(v[0].name, "Music");
        let kinds: Vec<String> = v[1..].iter().map(|e| kind_words(e).to_lowercase()).collect();
        let mut sorted = kinds.clone();
        sorted.sort();
        assert_eq!(kinds, sorted);
        let pngs: Vec<&str> = v.iter().filter(|e| e.name.ends_with(".png")).map(|e| e.name.as_str()).collect();
        assert_eq!(pngs, ["a.png", "photo.png"], "the same kind by name");
        // Grouped: Folders, Documents, Text, Pictures, each by name.
        sort(&mut v, SortBy::Name, true, false, true);
        assert_eq!(v.iter().map(|e| group_of(e).1).collect::<Vec<_>>(), ["Folders", "Documents", "Text", "Pictures", "Pictures"]);
        assert_eq!(kind_words(&all[2]), "Folder");
        // Widths from a setting, kept in bounds; the usual for what is missing.
        assert_eq!(parse_widths("\"200,90,160\""), [200.0, 90.0, 160.0]);
        assert_eq!(parse_widths("5,9999"), [COLUMN_MIN, COLUMN_MAX, COLUMN_WIDTHS[2]]);
        assert_eq!(parse_widths("x"), COLUMN_WIDTHS);
        assert_eq!(RowSize::parse("\"large\""), RowSize::Large);
        assert_eq!(RowSize::parse("huge"), RowSize::Medium);
    }
}
