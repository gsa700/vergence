//! The desktop's icons (files milestone 1, step 9; docs/files-milestone-1.md
//! §6; Dave, 2026-11-15: "on many systems but especially windows i use it
//! like a scratch folder: app launchers and random files ... include it as
//! windows does"). `vergence-files --desktop`: a window of the Desktop kind
//! (bus v17), which the server sizes to the main screen's work area and keeps
//! under every window; its ground is clear, so the wallpaper shows.
//!
//! - **The icons**: ~/Desktop's files (the XDG desktop folder), each a mark
//!   (a picture's own, small; a launcher's application icon) over its name,
//!   which carries a shadow to read on any wallpaper.
//! - **Where they stand**: a grid from the top left down, then across, as
//!   Windows has it; dragged elsewhere, an icon keeps its cell, kept per size
//!   of the desktop (a set of screens: Dave, "the desktop folder layout needs
//!   to track the monitor memories as well") in
//!   ~/.local/state/vergence/desktop-places; a new file takes the next free cell.
//! - **What they do**: a click chooses (Ctrl+click adds), a double click
//!   opens (a folder in File Viewer, a file with its app, a launcher starts
//!   its program); dragged, they go to File Viewer, the dock, an app;
//!   dropped here, files are moved in; Delete moves to the Trash; F2 renames.
//! - **Launchers**: a desktop entry that is executable (the dock's "Add to
//!   desktop" makes them so) starts at once; any other (a download, a copy)
//!   asks first, once ("Allow this launcher to start a program?"), as GNOME
//!   does: a file from the web must not start a program by a double click.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use vergence_scene::{Color, Image, Point, Rect};
use vergence_ui::Modifiers;
use vergence_widgets::files::{self, Entry, Kind};
use vergence_widgets::{Align, ButtonLook, Element, Input, Length, Padding, Program, Theme, Touch, Waker, Window, button, column, drawing, field, label, panel, row, space, stack};

use crate::ops;

const APP_ID: &str = "vergence-desktop";
/// A cell of the grid, and the margin around it all, in points.
const CELL_W: f32 = 96.0;
const CELL_H: f32 = 100.0;
const MARGIN: f32 = 12.0;
/// The icon's square in its cell.
const ICON: f32 = 56.0;
const WATCH_TIMER: u64 = 1;
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(2);
const DOUBLE: std::time::Duration = std::time::Duration::from_millis(450);

/// The user's Desktop folder: the XDG user-dirs file's, else ~/Desktop.
pub fn desktop_dir() -> PathBuf {
    files::user_dirs().into_iter().find(|(n, _)| n == "Desktop").map(|(_, p)| p).unwrap_or_else(|| files::home().unwrap_or_else(|| PathBuf::from("/")).join("Desktop"))
}

fn places_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| files::home().map(|h| h.join(".local/state")))?;
    Some(base.join("vergence/desktop-places"))
}

/// Every setup's icon cells: lines "setup<TAB>col<TAB>row<TAB>name".
fn read_places() -> Vec<(String, i32, i32, String)> {
    let Some(p) = places_path() else { return Vec::new() };
    std::fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut f = l.splitn(4, '\t');
            Some((f.next()?.to_string(), f.next()?.parse().ok()?, f.next()?.parse().ok()?, f.next()?.to_string()))
        })
        .take(20_000)
        .collect()
}

fn write_places(all: &[(String, i32, i32, String)]) {
    let Some(p) = places_path() else { return };
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let text: String = all.iter().filter(|(_, _, _, n)| !n.contains(['\t', '\n'])).map(|(s, c, r, n)| format!("{s}\t{c}\t{r}\t{n}\n")).collect();
    let tmp = p.with_extension("new");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// Cells for names: the placed keep theirs (if inside the grid and free),
/// the rest take free cells down each column, then across.
pub fn arrange(names: &[String], placed: &HashMap<String, (i32, i32)>, cols: i32, rows: i32) -> Vec<(i32, i32)> {
    let rows = rows.max(1);
    let mut taken: HashSet<(i32, i32)> = HashSet::new();
    let mut out: Vec<Option<(i32, i32)>> = vec![None; names.len()];
    for (i, n) in names.iter().enumerate() {
        if let Some(&(c, r)) = placed.get(n)
            && c >= 0
            && r >= 0
            && c < cols.max(1)
            && r < rows
            && taken.insert((c, r))
        {
            out[i] = Some((c, r));
        }
    }
    let mut next = (0, 0);
    for slot in out.iter_mut().filter(|s| s.is_none()) {
        while taken.contains(&next) {
            next = if next.1 + 1 >= rows { (next.0 + 1, 0) } else { (next.0, next.1 + 1) };
        }
        taken.insert(next);
        *slot = Some(next);
    }
    out.into_iter().map(|s| s.unwrap_or((0, 0))).collect()
}

/// The icons every desktop has before its files (Dave, 2026-11-15: "a
/// default set of icons there: Home, Trash, Mounted Volumes"): they open in
/// File Viewer and take drops (into Home or a disk; the Trash trashes), but
/// are not files of the Desktop folder: they cannot be renamed or trashed,
/// and dragged they only move on the desktop (their drag carries no file
/// links, so nothing can move Home itself). `[desktop] home`, `trash`,
/// `volumes` = false leaves one out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fixed {
    Home,
    Trash,
    Volume,
}

/// A drag of the desktop's own fixed icons: their keys, a line each.
const FIXED_KIND: &str = "application/x-vergence-desktop-icons";

#[derive(Clone, Debug)]
enum Ask {
    /// May this launcher start a program?
    Trust(PathBuf),
    /// A new name for a file.
    Rename(PathBuf, String),
}

#[derive(Clone, Debug)]
pub enum Msg {
    Touch(Touch),
    Allow,
    NameTyped(String),
    NameDone,
    Cancel,
}

pub struct Desktop {
    dir: PathBuf,
    entries: Vec<Entry>,
    /// The fixed icons among the entries (by their key, which is their entry's
    /// name: "::home", "::trash", "::volume:/mnt/x"), what, and what they are called.
    fixed: HashMap<String, (Fixed, String)>,
    stamp: Option<SystemTime>,
    /// This setup's placed cells, by name.
    placed: HashMap<String, (i32, i32)>,
    size: (f32, f32),
    chosen: Vec<PathBuf>,
    press: Option<(f32, f32, Option<usize>)>,
    dragged: bool,
    last_click: Option<(Instant, usize)>,
    mods: Modifiers,
    waker: Option<Waker>,
    /// Marks decoded by workers: a picture's own, a launcher's icon.
    marks: Arc<Mutex<HashMap<PathBuf, Option<Arc<Image>>>>>,
    asked: RefCell<HashSet<PathBuf>>,
    ask: Option<Ask>,
    notice: Option<String>,
    desktop_text: String,
}

impl Desktop {
    pub fn new() -> Desktop {
        let desktop_text = vergence_widgets::prefs::desktop_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        let mut d = Desktop {
            dir: desktop_dir(),
            entries: Vec::new(),
            fixed: HashMap::new(),
            stamp: None,
            placed: HashMap::new(),
            size: (0.0, 0.0),
            chosen: Vec::new(),
            press: None,
            dragged: false,
            last_click: None,
            mods: Modifiers::default(),
            waker: None,
            marks: Arc::default(),
            asked: RefCell::default(),
            ask: None,
            notice: None,
            desktop_text,
        };
        let _ = std::fs::create_dir_all(&d.dir);
        d.read();
        d
    }

    /// This set of screens, as its desktop's size says.
    fn setup(&self) -> String {
        format!("{:.0}x{:.0}", self.size.0, self.size.1)
    }

    fn read(&mut self) {
        self.stamp = std::fs::metadata(&self.dir).and_then(|m| m.modified()).ok();
        let mut e: Vec<Entry> = files::read_folder(&self.dir).unwrap_or_default().into_iter().filter(|e| !e.name.starts_with('.')).collect();
        e.sort_by(|a, b| files::natural(&a.name, &b.name));
        // The fixed icons first: Home, the disks mounted, the Trash.
        let wants = |k: &str| vergence_ui::table_value(&self.desktop_text, "[desktop]", k).as_deref() != Some("false");
        let mut fixed: Vec<(String, Fixed, String, PathBuf)> = Vec::new();
        if wants("home") && let Some(h) = files::home() {
            fixed.push(("::home".into(), Fixed::Home, "Home".into(), h));
        }
        if wants("volumes") {
            for m in files::mounted_disks() {
                let name = m.file_name().map_or_else(|| m.display().to_string(), |n| n.to_string_lossy().into_owned());
                fixed.push((format!("::volume:{}", m.display()), Fixed::Volume, name, m));
            }
        }
        if wants("trash") && let Some(t) = ops::trash_dir() {
            fixed.push(("::trash".into(), Fixed::Trash, "Trash".into(), t.join("files")));
        }
        self.fixed = fixed.iter().map(|(k, f, n, _)| (k.clone(), (*f, n.clone()))).collect();
        let mut all: Vec<Entry> = fixed.into_iter().map(|(k, _, _, p)| Entry { name: k, path: p, dir: true, mime: Some(vergence_types::FOLDER), size: 0, changed: None }).collect();
        all.extend(e);
        self.entries = all;
        let here: HashSet<&PathBuf> = self.entries.iter().map(|e| &e.path).collect();
        self.chosen.retain(|p| here.contains(p));
    }

    fn load_places(&mut self) {
        let setup = self.setup();
        self.placed = read_places().into_iter().filter(|(s, ..)| *s == setup).map(|(_, c, r, n)| (n, (c, r))).collect();
    }

    fn save_places(&self) {
        let setup = self.setup();
        let mut all: Vec<(String, i32, i32, String)> = read_places().into_iter().filter(|(s, ..)| *s != setup).collect();
        all.extend(self.placed.iter().map(|(n, (c, r))| (setup.clone(), *c, *r, n.clone())));
        write_places(&all);
    }

    fn grid(&self) -> (i32, i32) {
        let cols = ((self.size.0 - 2.0 * MARGIN) / CELL_W).floor().max(1.0) as i32;
        let rows = ((self.size.1 - 2.0 * MARGIN) / CELL_H).floor().max(1.0) as i32;
        (cols, rows)
    }

    /// Each entry's cell rectangle, in the entries' order.
    fn cells(&self) -> Vec<Rect> {
        let (cols, rows) = self.grid();
        let names: Vec<String> = self.entries.iter().map(|e| e.name.clone()).collect();
        arrange(&names, &self.placed, cols, rows).into_iter().map(|(c, r)| Rect::new(MARGIN + c as f32 * CELL_W, MARGIN + r as f32 * CELL_H, CELL_W, CELL_H)).collect()
    }

    fn cell_at(&self, x: f32, y: f32) -> (i32, i32) {
        let (cols, rows) = self.grid();
        (((x - MARGIN) / CELL_W).floor().clamp(0.0, (cols - 1) as f32) as i32, ((y - MARGIN) / CELL_H).floor().clamp(0.0, (rows - 1) as f32) as i32)
    }

    fn icon_at(&self, x: f32, y: f32) -> Option<usize> {
        self.cells().iter().position(|c| x >= c.x + 4.0 && x < c.x + c.w - 4.0 && y >= c.y && y < c.y + c.h - 4.0)
    }

    fn fixed_of(&self, e: &Entry) -> Option<Fixed> {
        self.fixed.get(&e.name).map(|(f, _)| *f)
    }

    /// The chosen files of the Desktop folder (the fixed icons left out).
    fn chosen_files(&self) -> Vec<PathBuf> {
        self.entries.iter().filter(|e| self.chosen.contains(&e.path) && self.fixed_of(e).is_none()).map(|e| e.path.clone()).collect()
    }

    /// Whether an entry is a launcher (a desktop entry file).
    fn launcher(e: &Entry) -> bool {
        e.name.ends_with(".desktop") && !e.dir
    }

    /// A launcher's entry, read.
    fn entry_of(e: &Entry) -> Option<vergence_shell::desktop::Entry> {
        let text = std::fs::read_to_string(&e.path).ok()?;
        vergence_shell::desktop::parse(e.name.trim_end_matches(".desktop"), &text)
    }

    /// The marks to decode: pictures (small) and launchers' icons.
    fn want_marks(&self) {
        for e in &self.entries {
            if self.asked.borrow().contains(&e.path) {
                continue;
            }
            let source = if Desktop::launcher(e) {
                Desktop::entry_of(e).filter(|x| !x.icon.is_empty()).and_then(|x| if x.icon.starts_with('/') { Some(PathBuf::from(x.icon)) } else { vergence_shell::icons::find("", &x.icon, 128) })
            } else if e.kind() == Kind::Picture {
                Some(e.path.clone())
            } else {
                None
            };
            let Some(source) = source else { continue };
            self.asked.borrow_mut().insert(e.path.clone());
            let (out, waker, key) = (self.marks.clone(), self.waker.clone(), e.path.clone());
            std::thread::spawn(move || {
                let img = vergence_decode::decode_file(&source, Some((128, 128))).ok().and_then(|p| {
                    let f = p.frames.into_iter().next()?;
                    Some(Arc::new(Image { width: p.width, height: p.height, rgba: f.rgba }))
                });
                out.lock().unwrap_or_else(|e| e.into_inner()).insert(key, img);
                if let Some(w) = waker {
                    w.wake();
                }
            });
        }
    }

    /// Opens an icon: a folder in File Viewer, a launcher's program (asking
    /// first if it is not trusted), a file with its app.
    fn open(&mut self, i: usize) {
        let Some(e) = self.entries.get(i).cloned() else { return };
        // A folder, Home, a disk, the Trash: in File Viewer.
        if e.dir {
            // The Trash may not have been made yet.
            if self.fixed_of(&e) == Some(Fixed::Trash) {
                let _ = std::fs::create_dir_all(&e.path);
            }
            let _ = std::process::Command::new("vergence-files").arg(&e.path).spawn();
            return;
        }
        if Desktop::launcher(&e) {
            use std::os::unix::fs::PermissionsExt;
            let trusted = std::fs::metadata(&e.path).is_ok_and(|m| m.permissions().mode() & 0o100 != 0);
            if !trusted {
                self.ask = Some(Ask::Trust(e.path.clone()));
                return;
            }
            return self.start_launcher(&e);
        }
        if let Err(why) = crate::open_files(&[e.path.clone()]) {
            self.notice = Some(why);
        }
    }

    fn start_launcher(&mut self, e: &Entry) {
        let Some(entry) = Desktop::entry_of(e) else {
            self.notice = Some(format!("“{}” is not a launcher this can start.", e.name));
            return;
        };
        for argv in vergence_shell::open::commands(&entry, &[]) {
            if let Some((p, rest)) = argv.split_first() {
                let _ = std::process::Command::new(p).args(rest).stdin(std::process::Stdio::null()).spawn();
            }
        }
    }

    fn view_icons(&self, t: &Theme) -> Element<Msg> {
        let cells = self.cells();
        let marks = self.marks.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let items: Vec<(Rect, String, Kind, Option<Arc<Image>>, bool, Option<Fixed>)> = self
            .entries
            .iter()
            .zip(cells)
            .map(|(e, c)| {
                let name = match self.fixed.get(&e.name) {
                    Some((_, n)) => n.clone(),
                    None if Desktop::launcher(e) => Desktop::entry_of(e).map(|x| x.name).unwrap_or_else(|| e.name.clone()),
                    None => e.name.clone(),
                };
                (c, name, e.kind(), marks.get(&e.path).cloned().flatten(), self.chosen.contains(&e.path), self.fixed_of(e))
            })
            .collect();
        let accent = t.accent;
        drawing(move |scene, fonts, r: Rect, t: &Theme| {
            let face = fonts.face("sans-serif", 400, false);
            for (c, name, kind, img, chosen, fixed) in &items {
                let c = Rect::new(r.x + c.x, r.y + c.y, c.w, c.h);
                if *chosen {
                    scene.fill_rounded(Rect::new(c.x + 4.0, c.y + 2.0, c.w - 8.0, c.h - 6.0), 8.0, Color { a: 110, ..accent });
                }
                let icon = Rect::new(c.x + (c.w - ICON) / 2.0, c.y + 8.0, ICON, ICON);
                match img {
                    _ if fixed.is_some() => draw_fixed(scene, icon, fixed.unwrap_or(Fixed::Home), t),
                    Some(img) => {
                        let s = (ICON / img.width as f32).min(ICON / img.height as f32);
                        let (w, h) = (img.width as f32 * s, img.height as f32 * s);
                        let id = scene.image(img.clone());
                        scene.draw_image(id, Rect::new(icon.x + (ICON - w) / 2.0, icon.y + (ICON - h) / 2.0, w, h), 1.0);
                    }
                    None => files::draw_kind(scene, icon, 3.2, *kind, t),
                }
                // The name, one line, cut to the cell, white with a shadow: it reads on any picture.
                let Some(f) = face else { continue };
                let size = 12.0;
                let room = c.w - 8.0;
                let mut text = name.clone();
                if fonts.layout(f, size, &text).1 > room {
                    let chars: Vec<char> = name.chars().collect();
                    let mut n = chars.len();
                    while n > 1 && fonts.layout(f, size, &(chars[..n].iter().collect::<String>() + "…")).1 > room {
                        n -= 1;
                    }
                    text = chars[..n].iter().collect::<String>() + "…";
                }
                let w = fonts.layout(f, size, &text).1;
                let at = Point::new(c.x + (c.w - w) / 2.0, icon.y + ICON + 18.0);
                vergence_render::text(scene, fonts, f, size, Color::rgba(0, 0, 0, 200), Point::new(at.x + 1.0, at.y + 1.0), &text);
                vergence_render::text(scene, fonts, f, size, Color::rgb(255, 255, 255), at, &text);
            }
        })
        .on_touch(|t| Some(Msg::Touch(t)))
        .width(Length::FILL)
        .height(Length::FILL)
    }

    fn strip(&self, t: &Theme) -> Option<Element<Msg>> {
        let b = |text: &str, m: Msg, accent: bool| -> Element<Msg> { button(label(text).size(12.0)).look(if accent { ButtonLook::Accent } else { ButtonLook::Plain }).padding(Padding::hv(3.0, 10.0)).on_press(m).name(text) };
        let items: Vec<Element<Msg>> = match (&self.ask, &self.notice) {
            (Some(Ask::Trust(p)), _) => vec![
                label(format!(
                    "Allow “{}” to start a program? This launcher came from elsewhere; allow it only if you trust it.",
                    self.entries.iter().find(|e| &e.path == p).and_then(Desktop::entry_of).map(|x| x.name).unwrap_or_else(|| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
                ))
                .size(12.0)
                .wrap()
                .width(Length::Fixed(380.0)),
                b("Cancel", Msg::Cancel, false),
                b("Allow and start", Msg::Allow, true),
            ],
            (Some(Ask::Rename(_, text)), _) => vec![label("New name").size(12.0), field(text.clone(), Msg::NameTyped).on_submit(Msg::NameDone).autofocus().width(Length::Fixed(260.0)).name("Name"), b("Cancel", Msg::Cancel, false), b("Rename", Msg::NameDone, true)],
            (None, Some(n)) => vec![label(n.clone()).size(12.0).wrap().width(Length::Fixed(380.0)), b("OK", Msg::Cancel, false)],
            (None, None) => return None,
        };
        let card = panel(row(items).spacing(8.0).align(Align::Center)).color(t.card).radius(10.0).edge(t.line, 1.0).padding(Padding::hv(8.0, 12.0));
        Some(column(vec![space().height(Length::Fixed(16.0)), row(vec![space().width(Length::FILL), card, space().width(Length::FILL)]).width(Length::FILL)]).width(Length::FILL))
    }
}

impl Program for Desktop {
    type Message = Msg;

    fn title(&self) -> String {
        "Desktop".into()
    }

    fn app_id(&self) -> String {
        APP_ID.into()
    }

    /// A clear ground: the wallpaper shows between the icons.
    fn theme(&self) -> Theme {
        let mut t = Theme::from_desktop(&self.desktop_text, true, "[desktop]");
        t.opacity = 0.0;
        t
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { kind: vergence_ui::WinKind::Desktop, ..Default::default() }
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let icons = self.view_icons(t);
        match self.strip(t) {
            Some(s) => stack(vec![icons, s]).width(Length::FILL).height(Length::FILL),
            None => icons,
        }
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        match m {
            Msg::Touch(Touch::Press { x, y }) => {
                let hit = self.icon_at(x, y);
                self.press = Some((x, y, hit));
                self.dragged = false;
                match hit {
                    Some(i) => {
                        let p = self.entries[i].path.clone();
                        if self.mods.ctrl {
                            match self.chosen.iter().position(|c| *c == p) {
                                Some(k) => {
                                    self.chosen.remove(k);
                                }
                                None => self.chosen.push(p),
                            }
                        } else if !self.chosen.contains(&p) {
                            self.chosen = vec![p];
                        }
                        let now = Instant::now();
                        let double = self.last_click.is_some_and(|(t, j)| j == i && now.duration_since(t) < DOUBLE);
                        self.last_click = Some((now, i));
                        if double {
                            self.last_click = None;
                            self.open(i);
                        }
                    }
                    None => self.chosen.clear(),
                }
            }
            Msg::Touch(Touch::Move { x, y, pressed: true }) => {
                // An icon pulled away: a drag of what is chosen.
                if let Some((px, py, Some(_))) = self.press
                    && !self.dragged
                    && ((x - px).powi(2) + (y - py).powi(2)) > 36.0
                {
                    self.dragged = true;
                    // The fixed icons only move on the desktop: their drag has no file links.
                    let keys: Vec<String> = self.entries.iter().filter(|e| self.chosen.contains(&e.path) && self.fixed_of(e).is_some()).map(|e| e.name.clone()).collect();
                    if !keys.is_empty() {
                        let label = if keys.len() == 1 { self.fixed.get(&keys[0]).map(|(_, n)| n.clone()).unwrap_or_default() } else { format!("{} items", keys.len()) };
                        window.start_drag(&label, vec![(FIXED_KIND.into(), keys.join("\n").into_bytes())]);
                        window.redraw();
                        return;
                    }
                    let paths = self.chosen.clone();
                    let uris: String = paths.iter().map(|p| format!("file://{}\r\n", ops::escape(p))).collect();
                    let plain = paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n");
                    let label = match paths.as_slice() {
                        [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                        many => format!("{} items", many.len()),
                    };
                    if uris.len() + plain.len() <= vergence_protocol::window::MAX_DRAG_BYTES {
                        window.start_drag(&label, vec![("text/uri-list".into(), uris.into_bytes()), ("text/plain".into(), plain.into_bytes())]);
                    }
                }
            }
            Msg::Touch(_) => {}
            Msg::Allow => {
                if let Some(Ask::Trust(p)) = self.ask.take() {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::metadata(&p).and_then(|m| {
                        let mut perm = m.permissions();
                        perm.set_mode(perm.mode() | 0o100);
                        std::fs::set_permissions(&p, perm)
                    });
                    if let Some(e) = self.entries.iter().find(|e| e.path == p).cloned() {
                        self.start_launcher(&e);
                    }
                }
            }
            Msg::NameTyped(s) => {
                if let Some(Ask::Rename(_, t)) = &mut self.ask {
                    *t = s;
                }
            }
            Msg::NameDone => {
                if let Some(Ask::Rename(from, name)) = self.ask.take() {
                    let name = name.trim().to_string();
                    let to = from.with_file_name(&name);
                    if !name.is_empty() && !name.contains('/') && to != from {
                        if to.symlink_metadata().is_ok() {
                            self.notice = Some(format!("Something named “{name}” is already here."));
                        } else if let Err(e) = std::fs::rename(&from, &to) {
                            self.notice = Some(format!("It could not be renamed: {}", files::plain(&e)));
                        } else {
                            // The cell goes with the new name.
                            let old = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            if let Some(cell) = self.placed.remove(&old) {
                                self.placed.insert(name.clone(), cell);
                                self.save_places();
                            }
                            self.read();
                        }
                    }
                }
            }
            Msg::Cancel => {
                self.ask = None;
                self.notice = None;
            }
        }
        self.want_marks();
        window.redraw();
    }

    fn saw(&mut self, input: &Input) {
        match input {
            Input::Button { modifiers, .. } | Input::Key { modifiers, .. } => self.mods = *modifiers,
            _ => {}
        }
    }

    fn key_first(&mut self, input: &Input, window: &mut Window) -> Option<bool> {
        const RETURN: u32 = 0xff0d;
        const ESCAPE: u32 = 0xff1b;
        const DELETE: u32 = 0xffff;
        const F2: u32 = 0xffbf;
        const LEFT: u32 = 0xff51;
        const RIGHT: u32 = 0xff53;
        const UP: u32 = 0xff52;
        const DOWN: u32 = 0xff54;
        let Input::Key { sym, pressed: true, modifiers: m, .. } = input else { return None };
        if self.ask.is_some() {
            if *sym == ESCAPE {
                self.ask = None;
                window.redraw();
                return Some(true);
            }
            return None;
        }
        let at = self.chosen.last().and_then(|p| self.entries.iter().position(|e| &e.path == p));
        let (_, rows) = self.grid();
        let step = |by: i32, d: &mut Desktop| {
            let n = d.entries.len() as i32;
            if n == 0 {
                return;
            }
            let i = match at {
                Some(i) => (i as i32 + by).clamp(0, n - 1),
                None => 0,
            };
            d.chosen = vec![d.entries[i as usize].path.clone()];
        };
        match *sym {
            RETURN => {
                if let Some(i) = at {
                    self.open(i);
                }
            }
            ESCAPE => self.chosen.clear(),
            DELETE if !self.chosen_files().is_empty() => {
                for p in self.chosen_files() {
                    if let Err(e) = ops::trash(&p) {
                        self.notice = Some(format!("“{}” could not be moved to the Trash: {e}", p.file_name().unwrap_or_default().to_string_lossy()));
                        break;
                    }
                }
                self.read();
            }
            F2 => {
                if let [one] = self.chosen_files().as_slice() {
                    let name = one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.ask = Some(Ask::Rename(one.clone(), name));
                }
            }
            s if m.ctrl && (s == 0x61 || s == 0x41) => self.chosen = self.entries.iter().map(|e| e.path.clone()).collect(),
            // The arrows move through the icons in their order (down a column, then across).
            UP => step(-1, self),
            DOWN => step(1, self),
            LEFT => step(-rows, self),
            RIGHT => step(rows, self),
            _ => return None,
        }
        window.redraw();
        Some(true)
    }

    fn resized(&mut self, width: f32, height: f32) {
        let was = self.setup();
        self.size = (width, height);
        if self.setup() != was {
            self.load_places();
        }
    }

    fn waker(&mut self, waker: Waker) {
        self.waker = Some(waker.clone());
        waker.wake();
    }

    fn woken(&mut self, window: &mut Window) -> bool {
        window.wake_after(WATCH_TIMER, WATCH_EVERY);
        self.want_marks();
        true
    }

    fn timer(&mut self, id: u64, window: &mut Window) -> bool {
        if id != WATCH_TIMER {
            return false;
        }
        window.wake_after(WATCH_TIMER, WATCH_EVERY);
        let now = std::fs::metadata(&self.dir).and_then(|m| m.modified()).ok();
        if now != self.stamp {
            self.read();
            self.want_marks();
            return true;
        }
        false
    }

    /// Files dragged here: moved in (Ctrl copies); the desktop's own: placed where let go.
    fn drag_over(&mut self, x: f32, y: f32, kinds: &[String], ctrl: bool) -> u8 {
        if kinds.iter().any(|k| k == FIXED_KIND) {
            return vergence_ui::DROP_MOVE;
        }
        if !kinds.iter().any(|k| k == "text/uri-list") {
            return vergence_ui::DROP_NONE;
        }
        // Over the Trash: moved there.
        if self.icon_at(x, y).and_then(|i| self.entries.get(i)).and_then(|e| self.fixed_of(e)) == Some(Fixed::Trash) {
            return vergence_ui::DROP_MOVE;
        }
        if ctrl { vergence_ui::DROP_COPY } else { vergence_ui::DROP_MOVE }
    }

    fn dropped(&mut self, x: f32, y: f32, action: u8, items: &[(String, Vec<u8>)], window: &mut Window) {
        let (cols, rows) = self.grid();
        let (c0, r0) = self.cell_at(x, y);
        // The fixed icons, moved on the desktop.
        let fixed_keys: Vec<String> = items.iter().find(|(k, _)| k == FIXED_KIND).map(|(_, v)| String::from_utf8_lossy(v).lines().map(str::to_string).filter(|k| self.fixed.contains_key(k)).collect()).unwrap_or_default();
        let paths: Vec<PathBuf> = items.iter().find(|(k, _)| k == "text/uri-list").map(|(_, v)| String::from_utf8_lossy(v).lines().filter_map(|l| vergence_types::path_of_link(l.trim())).collect()).unwrap_or_default();
        if paths.is_empty() && fixed_keys.is_empty() {
            return;
        }
        // Onto an icon that takes files: the Trash, Home, a disk, a folder.
        let onto = self.icon_at(x, y).and_then(|i| self.entries.get(i).cloned()).filter(|e| fixed_keys.is_empty() && !paths.contains(&e.path));
        if let Some(target) = onto {
            if self.fixed_of(&target) == Some(Fixed::Trash) {
                for p in &paths {
                    if let Err(e) = ops::trash(p) {
                        self.notice = Some(format!("“{}” could not be moved to the Trash: {e}", p.file_name().unwrap_or_default().to_string_lossy()));
                        break;
                    }
                }
                self.read();
                window.redraw();
                return;
            }
            if target.dir {
                let plan = ops::plan_paste(&paths, &target.path, action == vergence_ui::DROP_MOVE, ops::Clash::KeepBoth);
                let (_, error) = ops::run_paste(&plan, action == vergence_ui::DROP_MOVE, &Arc::new(ops::Progress::default()));
                if let Some(e) = error {
                    self.notice = Some(e);
                }
                self.read();
                window.redraw();
                return;
            }
        }
        let names: Vec<String> = if !fixed_keys.is_empty() {
            fixed_keys
        } else if paths.iter().all(|p| p.parent() == Some(self.dir.as_path())) && action == vergence_ui::DROP_MOVE {
            // The desktop's own icons, moved: to where they were let go.
            paths.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect()
        } else {
            let plan = ops::plan_paste(&paths, &self.dir, action == vergence_ui::DROP_MOVE, ops::Clash::KeepBoth);
            let progress = Arc::new(ops::Progress::default());
            let (_, error) = ops::run_paste(&plan, action == vergence_ui::DROP_MOVE, &progress);
            if let Some(e) = error {
                self.notice = Some(e);
            }
            plan.iter().filter_map(|(_, to)| to.file_name().map(|n| n.to_string_lossy().into_owned())).collect()
        };
        // Each to the drop's cell or the next free one down from it.
        let mut cell = (c0, r0);
        for n in names {
            self.placed.remove(&n);
            let taken: HashSet<(i32, i32)> = self.placed.values().copied().collect();
            let mut guard = 0;
            while taken.contains(&cell) && guard < cols * rows {
                cell = if cell.1 + 1 >= rows { ((cell.0 + 1) % cols.max(1), 0) } else { (cell.0, cell.1 + 1) };
                guard += 1;
            }
            self.placed.insert(n, cell);
            cell = if cell.1 + 1 >= rows { ((cell.0 + 1) % cols.max(1), 0) } else { (cell.0, cell.1 + 1) };
        }
        self.save_places();
        self.read();
        self.want_marks();
        window.redraw();
    }

    fn drag_ended(&mut self, _action: u8, window: &mut Window) {
        self.read();
        window.redraw();
    }
}

/// A fixed icon's mark: a house, a drive, a trash can, drawn in the icon's square.
fn draw_fixed(scene: &mut vergence_scene::Scene, r: Rect, f: Fixed, t: &Theme) {
    let k = r.w / 16.0;
    let p = |x: f32, y: f32| Point::new(r.x + x * k, r.y + y * k);
    let ink = Color::rgb(255, 255, 255);
    let w = 1.2 * k;
    match f {
        Fixed::Home => {
            scene.fill_rounded(Rect::new(r.x + 3.0 * k, r.y + 7.0 * k, 10.0 * k, 8.0 * k), 1.0 * k, t.accent);
            for (a, b) in [((1.5, 8.0), (8.0, 1.8)), ((8.0, 1.8), (14.5, 8.0))] {
                scene.line(p(a.0, a.1), p(b.0, b.1), w * 1.4, t.accent);
            }
            scene.fill_rounded(Rect::new(r.x + 6.8 * k, r.y + 10.0 * k, 2.4 * k, 5.0 * k), 0.5 * k, ink);
        }
        Fixed::Volume => {
            scene.fill_rounded(Rect::new(r.x + 1.5 * k, r.y + 5.0 * k, 13.0 * k, 7.0 * k), 1.5 * k, Color::rgb(0x8a, 0x92, 0xa3));
            scene.fill_rounded(Rect::new(r.x + 2.5 * k, r.y + 9.5 * k, 11.0 * k, 1.0 * k), 0.5 * k, Color::rgb(0x5d, 0x65, 0x75));
            scene.fill_rounded(Rect::new(r.x + 11.5 * k, r.y + 6.5 * k, 1.5 * k, 1.5 * k), 0.75 * k, Color::rgb(0x6f, 0xd0, 0x80));
        }
        Fixed::Trash => {
            let body = Color::rgb(0xd5, 0xd8, 0xde);
            scene.fill_rounded(Rect::new(r.x + 3.5 * k, r.y + 4.5 * k, 9.0 * k, 10.5 * k), 1.2 * k, body);
            scene.fill_rounded(Rect::new(r.x + 2.5 * k, r.y + 3.0 * k, 11.0 * k, 1.6 * k), 0.8 * k, body);
            scene.fill_rounded(Rect::new(r.x + 6.0 * k, r.y + 1.8 * k, 4.0 * k, 1.4 * k), 0.6 * k, body);
            for x in [5.8, 8.0, 10.2] {
                scene.line(p(x, 6.5), p(x, 13.5), w, Color::rgb(0x8a, 0x92, 0xa3));
            }
        }
    }
}

/// The desktop's window: runs until the session ends.
pub fn run() {
    if let Err(e) = vergence_widgets::run(Desktop::new(), (1280, 720)) {
        eprintln!("vergence-files --desktop: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_take_free_cells_down_then_across_and_keep_their_places() {
        let names: Vec<String> = ["a", "b", "c", "d", "e"].iter().map(|s| s.to_string()).collect();
        let mut placed = HashMap::new();
        assert_eq!(arrange(&names, &placed, 3, 2), vec![(0, 0), (0, 1), (1, 0), (1, 1), (2, 0)]);
        placed.insert("e".to_string(), (0, 0));
        placed.insert("c".to_string(), (9, 9));
        assert_eq!(arrange(&names, &placed, 3, 2), vec![(0, 1), (1, 0), (1, 1), (2, 0), (0, 0)], "e keeps its cell; c's is off the grid, so it is free");
    }

    #[test]
    fn places_round_trip() {
        let all = vec![("1280x720".to_string(), 1, 2, "a b.txt".to_string()), ("2560x1440".to_string(), 0, 0, "x".to_string())];
        let text: String = all.iter().map(|(s, c, r, n)| format!("{s}\t{c}\t{r}\t{n}\n")).collect();
        let back: Vec<(String, i32, i32, String)> = text
            .lines()
            .filter_map(|l| {
                let mut f = l.splitn(4, '\t');
                Some((f.next()?.to_string(), f.next()?.parse().ok()?, f.next()?.parse().ok()?, f.next()?.to_string()))
            })
            .collect();
        assert_eq!(back, all);
    }
}
