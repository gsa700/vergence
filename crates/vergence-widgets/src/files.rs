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
    /// An entry for a path (links followed: a link to a folder is a folder).
    pub fn of(path: PathBuf) -> Option<Entry> {
        let name = path.file_name()?.to_string_lossy().into_owned();
        let m = fs::metadata(&path).ok();
        let dir = m.as_ref().is_some_and(|m| m.is_dir());
        Some(Entry { mime: kind_of(dir, &name), dir, size: m.as_ref().map_or(0, |m| m.len()), changed: m.and_then(|m| m.modified().ok()), name, path })
    }

    pub fn kind(&self) -> Kind {
        Kind::of(self.mime)
    }
}

/// A folder's entries, or why it cannot be read (in plain words).
pub fn read_folder(dir: &Path) -> Result<Vec<Entry>, String> {
    match fs::read_dir(dir) {
        Ok(rd) => Ok(rd.flatten().filter_map(|e| Entry::of(e.path())).collect()),
        Err(e) => Err(format!("This folder cannot be read: {}", plain(&e))),
    }
}

/// The files opened or saved lately through the dialog, newest first, those still there.
pub fn recent() -> Vec<Entry> {
    recent_files().into_iter().filter_map(Entry::of).collect()
}

/// Entries in order: folders first, then by `by` (names naturally), up or
/// down; `keep` keeps the order they came in for names (the recent files).
pub fn sort(v: &mut [&Entry], by: SortBy, up: bool, keep: bool) {
    v.sort_by(|a, b| {
        let o = b.dir.cmp(&a.dir).then_with(|| match by {
            SortBy::Name if keep => std::cmp::Ordering::Equal,
            SortBy::Name => natural(&a.name, &b.name),
            SortBy::Size => a.size.cmp(&b.size),
            SortBy::Changed => a.changed.cmp(&b.changed),
        });
        if up || b.dir != a.dir { o } else { o.reverse() }
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
        items.push(button(label(p.name.clone()).size(12.0).width(Length::FILL)).look(if chosen { ButtonLook::Chosen } else { ButtonLook::Plain }).padding(Padding::hv(5.0, 10.0)).on_press(go(p)).width(Length::FILL).name(p.name.clone()));
    }
    panel(scroll(column(items).spacing(1.0).width(Length::FILL))).color(t.card).radius(8.0).padding(6.0).width(Length::Fixed(160.0)).height(Length::FILL)
}

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
}

/// The list: headings that sort (`sort`, the one now and its direction),
/// then a row per entry: its mark, name, size and when it changed.
pub fn list<M: Clone + 'static>(v: &View<M>, sort: (SortBy, bool), by: impl Fn(SortBy) -> M) -> (Element<M>, Vec<Element<M>>) {
    let heading = |text: &str, s: SortBy, w: Length, end: bool| -> Element<M> {
        let mark = if sort.0 == s { if sort.1 { " ▲" } else { " ▼" } } else { "" };
        button(label(format!("{text}{mark}")).size(11.0).dim().align(if end { Align::End } else { Align::Start }).width(Length::FILL)).look(ButtonLook::Plain).padding(Padding::hv(2.0, 6.0)).on_press(by(s)).width(w).name(format!("Sort by {text}"))
    };
    let size_w = Length::Fixed(80.0);
    let when_w = Length::Fixed(130.0);
    let head = row(vec![space().width(Length::Fixed(22.0)), heading("Name", SortBy::Name, Length::FILL, false), heading("Size", SortBy::Size, size_w, true), heading("Changed", SortBy::Changed, when_w, false)]).spacing(4.0).align(Align::Center).width(Length::FILL);
    let mut rows = Vec::with_capacity(v.entries.len());
    for (k, e) in v.entries.iter().enumerate() {
        let mut name_col: Vec<Element<M>> = vec![label(e.name.clone()).size(12.0).elide().width(Length::FILL)];
        if v.folders && let Some(p) = e.path.parent() {
            name_col.push(label(tilde(p)).size(10.0).dim().elide().width(Length::FILL));
        }
        let mut b = button(
            row(vec![
                kind_icon(e.kind()),
                column(name_col).width(Length::FILL),
                label(if e.dir { String::new() } else { size_words(e.size) }).size(11.0).dim().align(Align::End).width(size_w),
                label(e.changed.map(when).unwrap_or_default()).size(11.0).dim().width(when_w),
            ])
            .spacing(4.0)
            .align(Align::Center)
            .width(Length::FILL),
        )
        .look(if (v.chosen)(k, e) { ButtonLook::Chosen } else { ButtonLook::Plain })
        .padding(Padding::hv(3.0, 6.0))
        .on_press((v.click)(k))
        .width(Length::FILL)
        .name(e.name.clone());
        if let Some(d) = v.drag {
            b = b.on_drag(d(k));
        }
        if let Some((i, key)) = v.in_view
            && i == k
        {
            // A new row each time the choice moves, so it is scrolled to.
            b = b.key(key).keep_in_view();
        }
        rows.push(b);
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
                None => kind_icon_sized(e.kind(), 3.0).width(Length::Fixed(80.0)).height(Length::Fixed(52.0)),
            };
            let mut b = button(column(vec![mark, label(e.name.clone()).size(11.0).align(Align::Center).elide().width(Length::Fixed(88.0))]).spacing(4.0).align(Align::Center))
                .look(if (v.chosen)(k, e) { ButtonLook::Chosen } else { ButtonLook::Plain })
                .padding(Padding::hv(6.0, 4.0))
                .on_press((v.click)(k))
                .width(Length::Fixed(100.0))
                .name(e.name.clone());
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

/// The same mark drawn `k` times as large (the icon view's), centered in its place.
pub fn kind_icon_sized<M: 'static>(kind: Kind, k: f32) -> Element<M> {
    drawing(move |scene, _, r: Rect, t: &Theme| draw_kind(scene, r, k, kind, t))
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
        // FreeBSD: mount -p prints the table in fstab's form.
        std::process::Command::new("mount").arg("-p").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
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
    fn entries_sort_folders_first_then_as_asked() {
        let e = |name: &str, dir: bool, size: u64| Entry { name: name.into(), path: PathBuf::from(format!("/x/{name}")), dir, mime: kind_of(dir, name), size, changed: None };
        let all = [e("b10.txt", false, 5), e("b2.txt", false, 50), e("zeta", true, 0), e("Alpha", true, 0)];
        let mut v: Vec<&Entry> = all.iter().collect();
        sort(&mut v, SortBy::Name, true, false);
        assert_eq!(v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["Alpha", "zeta", "b2.txt", "b10.txt"]);
        sort(&mut v, SortBy::Size, false, false);
        assert_eq!(v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["Alpha", "zeta", "b2.txt", "b10.txt"], "folders stay first; biggest first");
        assert_eq!(all[2].kind(), Kind::Folder);
        assert_eq!(all[0].kind(), Kind::Text);
    }
}
