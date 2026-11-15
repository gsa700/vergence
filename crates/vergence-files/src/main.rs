//! File Viewer (files milestone 1, step 7; docs/files-milestone-1.md §5):
//! Vergence's file manager, named after NeXTSTEP's Workspace Manager window
//! (Dave, 2026-11-15). Dave's words: "with space bar preview ala macOS
//! finder ... remote file system support as well (using native OS tools?)".
//!
//! - **The window**: the toolbar (Back, Forward, Up, the folder's path,
//!   List or Icons, the preview pane, hidden files, a field that narrows the
//!   folder as you type), the places sidebar, the folder as a list or icons
//!   (vergence-widgets files.rs, shared with the Open dialog), the preview
//!   pane at the right. A folder a tab in the bar; one process (hand.rs).
//! - **Preview**: Space shows the chosen file large over the list (pictures
//!   decoded apart by vergence-decode, the start of text, else its mark and
//!   facts); the arrows move on and it follows; Space or Escape closes it.
//! - **Opening**: Enter or a double click opens a folder here, and a file
//!   with the app chosen for its type (the server's Open, as the dock does).
//! - **Choosing**: a click chooses one, Ctrl+click adds or takes away,
//!   Shift+click a run; the arrows move the choice.
//! - **File operations** (ops.rs): New Folder (Ctrl+Shift+N), Rename (F2),
//!   Cut, Copy, Paste (Ctrl+X, C, V; a clash asks: Replace, Keep both,
//!   Skip), Duplicate (Ctrl+D), Move to Trash (Delete), Delete for good
//!   (Shift+Delete, after a question); long copies and moves on a worker
//!   with a progress strip and Stop; Undo (Ctrl+Z) for each. The freedesktop
//!   Trash, shared with other desktops: Restore puts a file back where it
//!   was; Empty Trash asks first.

mod desktop;
mod hand;
mod ops;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use vergence_scene::{Color, Image, Rect};
use vergence_ui::{AppCommand as C, Bar, BarStyle, BarTab, Modifiers, Role, TabEvent};
use vergence_widgets::files::{self, Entry, Kind, Place, SortBy};
use vergence_widgets::prefs::{Choice, Control, Part, Prefs, Row, Rows};
use vergence_widgets::toolbar::{Icon, flexible, group, tool, toggle, toolbar};
use vergence_widgets::{Align, ButtonLook, Element, Input, Length, Padding, Program, Theme, Waker, Window, button, column, drawing, field, label, panel, row, rule, scroll, space, stack};

const APP_ID: &str = "vergence-files";
const SECTION: &str = "[files]";
/// How often the folder shown is looked at for changes.
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(2);
const WATCH_TIMER: u64 = 1;
const DOUBLE: std::time::Duration = std::time::Duration::from_millis(450);
/// The preview pane's width.
const PANE_W: f32 = 260.0;

mod cmd {
    pub const NEW_WINDOW: u32 = 1;
    pub const NEW_TAB: u32 = 2;
    pub const CLOSE_TAB: u32 = 3;
    pub const BACK: u32 = 4;
    pub const FORWARD: u32 = 5;
    pub const UP: u32 = 6;
    pub const LIST: u32 = 7;
    pub const ICONS: u32 = 8;
    pub const PANE: u32 = 9;
    pub const HIDDEN: u32 = 10;
    pub const PREVIEW: u32 = 11;
    pub const OPEN: u32 = 12;
    pub const SELECT_ALL: u32 = 13;
    pub const NEXT_TAB: u32 = 14;
    pub const PREV_TAB: u32 = 15;
    pub const RELOAD: u32 = 16;
    pub const CUT: u32 = 20;
    pub const COPY: u32 = 21;
    pub const PASTE: u32 = 22;
    pub const DUPLICATE: u32 = 23;
    pub const RENAME: u32 = 24;
    pub const TRASH: u32 = 25;
    pub const DELETE: u32 = 26;
    pub const NEW_FOLDER: u32 = 27;
    pub const UNDO: u32 = 28;
    pub const RESTORE: u32 = 29;
    pub const EMPTY_TRASH: u32 = 30;
    pub const SETTINGS: u32 = 40;
    pub const ABOUT: u32 = 41;
}

/// A question in the strip above the folder.
#[derive(Clone, Debug)]
enum Ask {
    /// A paste would replace these names.
    Clash { sources: Vec<PathBuf>, dir: PathBuf, cut: bool, names: Vec<String> },
    /// Delete these for good.
    Forever(Vec<PathBuf>),
    EmptyTrash,
}

/// A name being typed: a new folder's, or a new name for a file.
#[derive(Clone, Debug)]
struct Naming {
    rename: Option<PathBuf>,
    text: String,
}

/// A long copy or move on a worker.
struct Job {
    words: String,
    progress: Arc<ops::Progress>,
    result: Arc<Mutex<Option<(ops::Done, Option<String>)>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewKind {
    List,
    Icons,
}

/// What a tab shows: a folder, or the recent files.
#[derive(Clone, Debug, PartialEq)]
enum Showing {
    Folder(PathBuf),
    Recent,
}

/// One folder in a tab, with its way back and forward.
struct Tab {
    id: u32,
    showing: Showing,
    back: Vec<Showing>,
    forward: Vec<Showing>,
    entries: Vec<Entry>,
    problem: Option<String>,
    /// The folder's modification time when read (changes are looked for).
    stamp: Option<SystemTime>,
    /// What is chosen (by path, so it survives sorting), and the one the arrows move from.
    chosen: Vec<PathBuf>,
    cursor: Option<PathBuf>,
    /// Bumped when the cursor moves, so its row is scrolled to.
    turn: u64,
}

impl Tab {
    fn new(id: u32, showing: Showing) -> Tab {
        let mut t = Tab { id, showing, back: Vec::new(), forward: Vec::new(), entries: Vec::new(), problem: None, stamp: None, chosen: Vec::new(), cursor: None, turn: 0 };
        t.read();
        t
    }

    fn dir(&self) -> Option<&Path> {
        match &self.showing {
            Showing::Folder(p) => Some(p),
            Showing::Recent => None,
        }
    }

    fn name(&self) -> String {
        match &self.showing {
            Showing::Recent => "Recent".into(),
            Showing::Folder(p) if Some(p.as_path()) == files::home().as_deref() => "Home".into(),
            Showing::Folder(p) if p.as_os_str() == "/" => "Computer".into(),
            Showing::Folder(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()),
        }
    }

    fn read(&mut self) {
        self.problem = None;
        self.entries = match &self.showing {
            Showing::Folder(dir) => {
                self.stamp = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
                files::read_folder(dir).unwrap_or_else(|why| {
                    self.problem = Some(why);
                    Vec::new()
                })
            }
            Showing::Recent => files::recent(),
        };
        // What was chosen and is gone is chosen no more.
        let here: std::collections::HashSet<&PathBuf> = self.entries.iter().map(|e| &e.path).collect();
        self.chosen.retain(|p| here.contains(p));
        if self.cursor.as_ref().is_some_and(|c| !here.contains(c)) {
            self.cursor = None;
        }
    }

    fn go(&mut self, to: Showing) {
        if to != self.showing {
            self.back.push(std::mem::replace(&mut self.showing, to));
            self.forward.clear();
        }
        self.chosen.clear();
        self.cursor = None;
        self.read();
    }
}

/// A picture for the preview, decoded apart, for a path.
type Shown = Arc<Mutex<Option<(PathBuf, Option<Arc<Image>>)>>>;

struct Shared {
    inbox: Arc<Mutex<Vec<Vec<PathBuf>>>>,
    next_tab: u32,
    /// What Cut or Copy took, and whether it was a cut.
    clip: Option<(Vec<PathBuf>, bool)>,
    /// What can be undone, the newest last.
    undo: Vec<ops::Done>,
}

type SharedRef = Rc<RefCell<Shared>>;

struct Viewer {
    shared: SharedRef,
    tabs: Vec<Tab>,
    front: u32,
    desktop: String,
    stamp: Option<SystemTime>,
    waker: Option<Waker>,
    view: ViewKind,
    pane: bool,
    hidden: bool,
    sort: (SortBy, bool),
    /// What the field narrows the folder to.
    narrow: String,
    /// Space's preview is open.
    preview: bool,
    /// The picture for the preview and the pane (decoded by a worker).
    picture: Shown,
    asked: RefCell<Option<PathBuf>>,
    notice: Option<String>,
    mods: Modifiers,
    last_click: Option<(Instant, PathBuf)>,
    width: f32,
    prefs_open: Rc<Cell<bool>>,
    ask: Option<Ask>,
    naming: Option<Naming>,
    job: Option<Job>,
}

#[derive(Debug, Clone)]
enum Msg {
    /// A row pulled away: a drag starts.
    DragRow(usize),
    Answer(Option<ops::Clash>),
    Confirm(bool),
    NameTyped(String),
    NameDone,
    NameCancel,
    StopJob,
    Run(u32),
    Go(Place),
    Crumb(PathBuf),
    Click(usize),
    Sort(SortBy),
    Narrow(String),
    OpenChosen,
    OpenInCloseUp,
    DismissNotice,
    ClosePreview,
}

/// desktop.toml's text.
fn desktop_text() -> (String, Option<SystemTime>) {
    match vergence_widgets::prefs::desktop_path() {
        Some(p) => (std::fs::read_to_string(&p).unwrap_or_default(), std::fs::metadata(&p).and_then(|m| m.modified()).ok()),
        None => (String::new(), None),
    }
}


/// Opens files with the apps chosen for their types: the server's Open in a
/// Vergence session (the dock shows them starting), else started here.
pub(crate) fn open_files(paths: &[PathBuf]) -> Result<(), String> {
    let items: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    if std::env::var_os(vergence_protocol::BUS_ENV).is_some()
        && let Ok(mut bus) = vergence_protocol::Client::connect(APP_ID)
    {
        return match bus.request(|serial| vergence_protocol::bus::ClientMsg::Open { serial, items: items.clone(), app: String::new() }) {
            Ok(vergence_protocol::bus::ServerMsg::Done { .. }) => Ok(()),
            Err(vergence_protocol::ClientError::Refused(_, m)) | Ok(vergence_protocol::bus::ServerMsg::Error { message: m, .. }) => Err(m),
            other => Err(format!("{other:?}")),
        };
    }
    let parsed: Vec<vergence_shell::open::Item> = items.iter().map(|i| vergence_shell::open::item(i)).collect();
    let (groups, failed) = vergence_shell::open::plan(&parsed, None, &vergence_shell::apps::installed(), &vergence_types::lists());
    for (entry, files) in groups {
        for argv in vergence_shell::open::commands(&entry, &files) {
            if let Some((p, rest)) = argv.split_first() {
                let _ = std::process::Command::new(p).args(rest).spawn();
            }
        }
    }
    if failed.is_empty() { Ok(()) } else { Err(failed.join("; ")) }
}

impl Viewer {
    fn new(shared: SharedRef) -> Viewer {
        let (desktop, stamp) = desktop_text();
        let view = if vergence_ui::table_value(&desktop, SECTION, "view").as_deref() == Some("icons") { ViewKind::Icons } else { ViewKind::List };
        let pane = vergence_ui::table_value(&desktop, SECTION, "preview-pane").as_deref() == Some("true");
        let hidden = vergence_ui::table_value(&desktop, SECTION, "hidden").as_deref() == Some("true");
        Viewer {
            shared,
            tabs: Vec::new(),
            front: 0,
            desktop,
            stamp,
            waker: None,
            view,
            pane,
            hidden,
            sort: (SortBy::Name, true),
            narrow: String::new(),
            preview: false,
            picture: Arc::default(),
            asked: RefCell::new(None),
            notice: None,
            mods: Modifiers::default(),
            last_click: None,
            width: 900.0,
            prefs_open: Rc::new(Cell::new(false)),
            ask: None,
            naming: None,
            job: None,
        }
    }

    fn new_tab(&mut self, showing: Showing) {
        let id = {
            let mut s = self.shared.borrow_mut();
            s.next_tab += 1;
            s.next_tab
        };
        self.tabs.push(Tab::new(id, showing));
        self.front = id;
        self.narrow.clear();
    }

    /// Paths handed over or given at the start: a folder opens in a tab; a
    /// file opens its folder with the file chosen.
    fn open_paths(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            self.new_tab(Showing::Folder(files::home().unwrap_or_else(|| PathBuf::from("/"))));
            return;
        }
        for p in paths {
            let p = std::path::absolute(p).unwrap_or_else(|_| p.clone());
            if p.is_dir() {
                self.new_tab(Showing::Folder(p));
            } else if let Some(dir) = p.parent() {
                self.new_tab(Showing::Folder(dir.to_path_buf()));
                if let Some(t) = self.front_mut() {
                    t.chosen = vec![p.clone()];
                    t.cursor = Some(p.clone());
                    t.turn += 1;
                }
            }
        }
    }

    fn front_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == self.front).or(self.tabs.first())
    }

    fn front_mut(&mut self) -> Option<&mut Tab> {
        let id = self.front_tab()?.id;
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// The front tab's entries as shown: hidden ones and the narrowing left out, sorted.
    fn visible(&self) -> Vec<&Entry> {
        let Some(t) = self.front_tab() else { return Vec::new() };
        let n = self.narrow.to_lowercase();
        let mut v: Vec<&Entry> = t.entries.iter().filter(|e| self.hidden || !e.name.starts_with('.')).filter(|e| n.is_empty() || e.name.to_lowercase().contains(&n)).collect();
        files::sort(&mut v, self.sort.0, self.sort.1, t.showing == Showing::Recent);
        v.truncate(files::MAX_ROWS);
        v
    }

    fn cursor_entry(&self) -> Option<Entry> {
        let t = self.front_tab()?;
        let c = t.cursor.as_ref()?;
        t.entries.iter().find(|e| &e.path == c).cloned()
    }

    fn go(&mut self, to: Showing) {
        self.narrow.clear();
        if let Some(t) = self.front_mut() {
            t.go(to);
        }
    }

    fn back(&mut self) {
        if let Some(t) = self.front_mut()
            && let Some(s) = t.back.pop()
        {
            t.forward.push(std::mem::replace(&mut t.showing, s));
            t.read();
        }
        self.narrow.clear();
    }

    fn forward(&mut self) {
        if let Some(t) = self.front_mut()
            && let Some(s) = t.forward.pop()
        {
            t.back.push(std::mem::replace(&mut t.showing, s));
            t.read();
        }
        self.narrow.clear();
    }

    fn up(&mut self) {
        let up = self.front_tab().and_then(|t| t.dir()).and_then(Path::parent).map(Path::to_path_buf);
        if let Some(p) = up {
            let came_from = self.front_tab().and_then(|t| t.dir().map(Path::to_path_buf));
            self.go(Showing::Folder(p));
            // The folder come up from is chosen.
            if let (Some(c), Some(t)) = (came_from, self.front_mut()) {
                t.chosen = vec![c.clone()];
                t.cursor = Some(c);
                t.turn += 1;
            }
        }
    }

    /// Opens what is chosen: one folder here; files with their apps.
    fn open_chosen(&mut self) {
        let Some(t) = self.front_tab() else { return };
        let chosen: Vec<Entry> = t.entries.iter().filter(|e| t.chosen.contains(&e.path)).cloned().collect();
        if let [one] = chosen.as_slice()
            && one.dir
        {
            let p = one.path.clone();
            self.preview = false;
            return self.go(Showing::Folder(p));
        }
        let paths: Vec<PathBuf> = chosen.iter().filter(|e| !e.dir).map(|e| e.path.clone()).collect();
        if !paths.is_empty()
            && let Err(e) = open_files(&paths)
        {
            self.notice = Some(e);
        }
    }

    /// A click on visible row `k`, with the modifiers of its press.
    fn click(&mut self, k: usize) {
        let visible: Vec<PathBuf> = self.visible().iter().map(|e| e.path.clone()).collect();
        let Some(p) = visible.get(k).cloned() else { return };
        let now = Instant::now();
        let double = self.last_click.as_ref().is_some_and(|(t, q)| *q == p && now.duration_since(*t) < DOUBLE);
        self.last_click = Some((now, p.clone()));
        let mods = self.mods;
        let Some(t) = self.front_mut() else { return };
        if mods.ctrl {
            match t.chosen.iter().position(|c| *c == p) {
                Some(i) => {
                    t.chosen.remove(i);
                }
                None => t.chosen.push(p.clone()),
            }
            t.cursor = Some(p);
        } else if mods.shift {
            let from = t.cursor.as_ref().and_then(|c| visible.iter().position(|v| v == c)).unwrap_or(k);
            let (a, b) = (from.min(k), from.max(k));
            t.chosen = visible[a..=b].to_vec();
        } else {
            t.chosen = vec![p.clone()];
            t.cursor = Some(p);
            if double {
                self.last_click = None;
                self.open_chosen();
            }
        }
    }

    /// The arrows: the cursor moves by `by` among the visible rows (Shift: the choice grows).
    fn move_cursor(&mut self, by: isize) {
        let visible: Vec<PathBuf> = self.visible().iter().map(|e| e.path.clone()).collect();
        if visible.is_empty() {
            return;
        }
        let shift = self.mods.shift;
        let Some(t) = self.front_mut() else { return };
        let at = t.cursor.as_ref().and_then(|c| visible.iter().position(|v| v == c));
        let k = match at {
            None if by > 0 => 0,
            None => visible.len() - 1,
            Some(i) => (i as isize + by).clamp(0, visible.len() as isize - 1) as usize,
        };
        let p = visible[k].clone();
        if shift {
            if !t.chosen.contains(&p) {
                t.chosen.push(p.clone());
            }
        } else {
            t.chosen = vec![p.clone()];
        }
        t.cursor = Some(p);
        t.turn += 1;
    }

    /// Icons per row in the icon view, for Up and Down.
    fn per_row(&self) -> isize {
        let room = self.width - 160.0 - 24.0 - if self.pane { PANE_W + 8.0 } else { 0.0 };
        ((room + 6.0) / 106.0).floor().max(1.0) as isize
    }

    /// Asks the worker for the cursor's picture (if it is one and not asked already).
    fn want_picture(&self) {
        let Some(e) = self.cursor_entry() else { return };
        if e.kind() != Kind::Picture || self.asked.borrow().as_ref() == Some(&e.path) {
            return;
        }
        *self.asked.borrow_mut() = Some(e.path.clone());
        let (out, waker, path) = (self.picture.clone(), self.waker.clone(), e.path.clone());
        std::thread::spawn(move || {
            let img = vergence_decode::decode_file(&path, Some((1600, 1200))).ok().and_then(|p| {
                let f = p.frames.into_iter().next()?;
                Some(Arc::new(Image { width: p.width, height: p.height, rgba: f.rgba }))
            });
            *out.lock().unwrap_or_else(|e| e.into_inner()) = Some((path, img));
            if let Some(w) = waker {
                w.wake();
            }
        });
    }

    /// The decoded picture of `path`, if the worker has it.
    fn picture_of(&self, path: &Path) -> Option<Arc<Image>> {
        let p = self.picture.lock().unwrap_or_else(|e| e.into_inner());
        p.as_ref().filter(|(q, _)| q == path).and_then(|(_, img)| img.clone())
    }

    fn write_key(&mut self, key: &str, value: Option<&str>) {
        let Some(path) = vergence_widgets::prefs::desktop_path() else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let new = vergence_widgets::prefs::set_key(&text, SECTION, key, value);
        if new != text {
            let beside = path.with_extension("toml.new");
            let _ = std::fs::write(&beside, &new).and_then(|_| std::fs::rename(&beside, &path));
        }
    }

    fn open_settings(&mut self, window: &mut Window) {
        if self.prefs_open.get() {
            return;
        }
        let waker = self.waker.clone();
        let prefs = Prefs::new("File Viewer", APP_ID, SECTION, false, Box::new(FilesRows), self.prefs_open.clone()).on_write(move || {
            if let Some(w) = &waker {
                w.wake();
            }
        });
        window.open_window(vergence_widgets::window(prefs));
    }

    fn run(&mut self, id: u32, window: &mut Window) {
        match id {
            cmd::NEW_WINDOW => {
                let mut w = Viewer::new(self.shared.clone());
                let at = self.front_tab().map(|t| t.showing.clone()).unwrap_or(Showing::Recent);
                w.new_tab(at);
                window.open_window(vergence_widgets::window(w));
            }
            cmd::NEW_TAB => {
                let at = self.front_tab().map(|t| t.showing.clone()).unwrap_or(Showing::Recent);
                self.new_tab(at);
            }
            cmd::CLOSE_TAB => {
                let f = self.front;
                self.close_tab(f, window);
            }
            cmd::BACK => self.back(),
            cmd::FORWARD => self.forward(),
            cmd::UP => self.up(),
            cmd::LIST => {
                self.view = ViewKind::List;
                self.write_key("view", None);
            }
            cmd::ICONS => {
                self.view = ViewKind::Icons;
                self.write_key("view", Some("\"icons\""));
            }
            cmd::PANE => {
                self.pane = !self.pane;
                self.write_key("preview-pane", if self.pane { Some("true") } else { None });
            }
            cmd::HIDDEN => {
                self.hidden = !self.hidden;
                self.write_key("hidden", if self.hidden { Some("true") } else { None });
            }
            cmd::PREVIEW => self.preview = !self.preview && self.cursor_entry().is_some(),
            cmd::OPEN => self.open_chosen(),
            cmd::SELECT_ALL => {
                let all: Vec<PathBuf> = self.visible().iter().map(|e| e.path.clone()).collect();
                if let Some(t) = self.front_mut() {
                    t.chosen = all;
                }
            }
            cmd::NEXT_TAB | cmd::PREV_TAB => {
                let n = self.tabs.len() as isize;
                if n > 1 {
                    let i = self.tabs.iter().position(|t| t.id == self.front).unwrap_or(0) as isize;
                    self.front = self.tabs[(i + if id == cmd::NEXT_TAB { 1 } else { -1 }).rem_euclid(n) as usize].id;
                    self.narrow.clear();
                }
            }
            cmd::RELOAD => {
                if let Some(t) = self.front_mut() {
                    t.read();
                }
            }
            cmd::CUT | cmd::COPY => {
                let paths = self.chosen_paths();
                if !paths.is_empty() {
                    window.copy(&paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n"));
                    self.shared.borrow_mut().clip = Some((paths, id == cmd::CUT));
                }
            }
            cmd::PASTE => self.paste(),
            cmd::DUPLICATE => {
                let paths = self.chosen_paths();
                if let Some(dir) = paths.first().and_then(|p| p.parent()).map(Path::to_path_buf) {
                    self.start_paste(paths, dir, false, ops::Clash::KeepBoth);
                }
            }
            cmd::RENAME => {
                if let [one] = self.chosen_paths().as_slice() {
                    let name = one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.naming = Some(Naming { rename: Some(one.clone()), text: name });
                }
            }
            cmd::NEW_FOLDER => {
                if self.here().is_some() {
                    self.naming = Some(Naming { rename: None, text: "New folder".into() });
                }
            }
            cmd::TRASH => self.trash_chosen(),
            cmd::DELETE => {
                let paths = self.chosen_paths();
                if !paths.is_empty() {
                    self.ask = Some(Ask::Forever(paths));
                }
            }
            cmd::UNDO => {
                let last = self.shared.borrow_mut().undo.pop();
                if let Some(d) = last {
                    self.notice = Some(match ops::undo(&d) {
                        Ok(words) => words,
                        Err(why) => format!("It could not be undone: {why}"),
                    });
                    self.read_all();
                }
            }
            cmd::RESTORE => {
                self.notice = None;
                let names: Vec<String> = self.chosen_paths().iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
                let mut back = 0;
                for n in &names {
                    match ops::restore(n) {
                        Ok(_) => back += 1,
                        Err(e) => self.notice = Some(format!("“{n}” could not be put back: {e}")),
                    }
                }
                if back > 0 && self.notice.is_none() {
                    self.notice = Some(format!("Put back where {} came from.", if back == 1 { "it" } else { "they" }));
                }
                self.read_all();
            }
            cmd::EMPTY_TRASH => self.ask = Some(Ask::EmptyTrash),
            cmd::SETTINGS => self.open_settings(window),
            cmd::ABOUT => {
                let about = vergence_widgets::about::About::new(APP_ID, "File Viewer", env!("CARGO_PKG_VERSION"), "Vergence's file manager: your folders as a list or icons, Preview on the space bar, files opened with the apps chosen for them.", self.theme());
                window.open_window(vergence_widgets::window(about));
            }
            _ => {}
        }
        self.want_picture();
        window.redraw();
    }

    fn close_tab(&mut self, id: u32, window: &mut Window) {
        if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
            self.tabs.remove(i);
            if self.front == id {
                self.front = self.tabs.get(i.min(self.tabs.len().saturating_sub(1))).map_or(0, |t| t.id);
            }
        }
        if self.tabs.is_empty() {
            window.close();
        }
    }

    /// The folder shown, if it is one (not Recent).
    fn here(&self) -> Option<PathBuf> {
        self.front_tab().and_then(|t| t.dir().map(Path::to_path_buf))
    }

    fn chosen_paths(&self) -> Vec<PathBuf> {
        self.front_tab().map(|t| t.chosen.clone()).unwrap_or_default()
    }

    /// Whether the folder shown is the Trash's.
    fn in_trash(&self) -> bool {
        let trash = ops::trash_dir().map(|d| d.join("files"));
        trash.is_some() && self.here() == trash
    }

    /// Every tab read again (after an operation).
    fn read_all(&mut self) {
        for t in &mut self.tabs {
            t.read();
        }
    }

    fn paste(&mut self) {
        let Some(dir) = self.here() else { return };
        let Some((sources, cut)) = self.shared.borrow().clip.clone() else { return };
        let names = ops::clashes(&sources, &dir);
        if names.is_empty() {
            self.start_paste(sources, dir, cut, ops::Clash::KeepBoth);
        } else {
            self.ask = Some(Ask::Clash { sources, dir, cut, names });
        }
    }

    /// A paste (or Duplicate) on a worker, its progress in the strip.
    fn start_paste(&mut self, sources: Vec<PathBuf>, dir: PathBuf, cut: bool, clash: ops::Clash) {
        if self.job.is_some() {
            self.notice = Some("A copy is still going on.".into());
            return;
        }
        let plan = ops::plan_paste(&sources, &dir, cut, clash);
        if plan.is_empty() {
            return;
        }
        if cut {
            // A cut is used once.
            self.shared.borrow_mut().clip = None;
        }
        let words = format!("{} {} item{}", if cut { "Moving" } else { "Copying" }, plan.len(), if plan.len() == 1 { "" } else { "s" });
        let progress = Arc::new(ops::Progress::default());
        let result: Arc<Mutex<Option<(ops::Done, Option<String>)>>> = Arc::default();
        let (p, r, waker) = (progress.clone(), result.clone(), self.waker.clone());
        std::thread::spawn(move || {
            // The window looks again while it runs, and when it ends.
            let ticking = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let (t, w) = (ticking.clone(), waker.clone());
            std::thread::spawn(move || {
                while t.load(std::sync::atomic::Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    if let Some(w) = &w {
                        w.wake();
                    }
                }
            });
            let out = ops::run_paste(&plan, cut, &p);
            *r.lock().unwrap_or_else(|e| e.into_inner()) = Some(out);
            ticking.store(false, std::sync::atomic::Ordering::Relaxed);
            if let Some(w) = waker {
                w.wake();
            }
        });
        self.job = Some(Job { words, progress, result });
    }

    /// A finished job: what it did goes on the undo stack; its error is said.
    fn job_ended(&mut self) -> bool {
        let Some(job) = &self.job else { return false };
        let Some((done, error)) = job.result.lock().unwrap_or_else(|e| e.into_inner()).take() else { return true };
        let made = !matches!(&done, ops::Done::Copied(v) if v.is_empty()) && !matches!(&done, ops::Done::Moved(v) if v.is_empty());
        if made {
            self.shared.borrow_mut().undo.push(done);
        }
        if let Some(e) = error {
            self.notice = Some(e);
        }
        self.job = None;
        self.read_all();
        true
    }

    fn trash_chosen(&mut self) {
        let paths = self.chosen_paths();
        if paths.is_empty() {
            return;
        }
        // This operation's words replace the last one's.
        self.notice = None;
        if self.in_trash() {
            // In the Trash itself, Delete means for good.
            self.ask = Some(Ask::Forever(paths));
            return;
        }
        let mut done = Vec::new();
        for p in &paths {
            match ops::trash(p) {
                Ok(name) => done.push((p.clone(), name)),
                Err(e) => {
                    self.notice = Some(format!("“{}” could not be moved to the Trash: {e}", p.file_name().unwrap_or_default().to_string_lossy()));
                    break;
                }
            }
        }
        if !done.is_empty() {
            if self.notice.is_none() {
                self.notice = Some(format!("{} moved to the Trash. Ctrl+Z puts {} back.", if done.len() == 1 { "1 item".to_string() } else { format!("{} items", done.len()) }, if done.len() == 1 { "it" } else { "them" }));
            }
            self.shared.borrow_mut().undo.push(ops::Done::Trashed(done));
        }
        self.read_all();
    }

    /// The name strip's Enter: the folder made, or the file renamed.
    fn name_done(&mut self) {
        let Some(n) = self.naming.take() else { return };
        let name = n.text.trim().to_string();
        if name.is_empty() || name == "." || name == ".." {
            return;
        }
        if name.contains('/') {
            self.notice = Some("A name cannot have / in it.".into());
            self.naming = Some(n);
            return;
        }
        let done = match &n.rename {
            Some(from) => {
                let to = from.with_file_name(&name);
                if to == *from {
                    return;
                }
                if to.symlink_metadata().is_ok() {
                    self.notice = Some(format!("Something named “{name}” is already here."));
                    self.naming = Some(n);
                    return;
                }
                std::fs::rename(from, &to).map(|_| (ops::Done::Renamed(from.clone(), to.clone()), to))
            }
            None => {
                let Some(dir) = self.here() else { return };
                let to = dir.join(ops::free_name(&name, |x| dir.join(x).symlink_metadata().is_ok()));
                std::fs::create_dir(&to).map(|_| (ops::Done::NewFolder(to.clone()), to))
            }
        };
        match done {
            Ok((d, to)) => {
                self.shared.borrow_mut().undo.push(d);
                self.read_all();
                if let Some(t) = self.front_mut() {
                    t.chosen = vec![to.clone()];
                    t.cursor = Some(to);
                    t.turn += 1;
                }
            }
            Err(e) => self.notice = Some(format!("It could not be done: {}", files::plain(&e))),
        }
    }

    /// A row pulled away: what is chosen goes (this row alone if it is not
    /// among them), as file links and as paths.
    fn drag_row(&mut self, k: usize, window: &mut Window) {
        let visible: Vec<PathBuf> = self.visible().iter().map(|e| e.path.clone()).collect();
        let Some(p) = visible.get(k).cloned() else { return };
        let Some(t) = self.front_mut() else { return };
        if !t.chosen.contains(&p) {
            t.chosen = vec![p.clone()];
            t.cursor = Some(p);
        }
        let paths = t.chosen.clone();
        let uris: String = paths.iter().map(|p| format!("file://{}\r\n", ops::escape(p))).collect();
        let plain = paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n");
        if uris.len() + plain.len() > vergence_protocol::window::MAX_DRAG_BYTES {
            self.notice = Some("Too many files to drag at once: Cut and Paste them.".into());
            return;
        }
        let label = match paths.as_slice() {
            [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            many => format!("{} items", many.len()),
        };
        window.start_drag(&label, vec![("text/uri-list".into(), uris.into_bytes()), ("text/plain".into(), plain.into_bytes())]);
    }

    /// The strips above the folder: a question, a name being typed, a job's progress.
    fn strips(&self, t: &Theme) -> Vec<Element<Msg>> {
        let strip = |items: Vec<Element<Msg>>| -> Element<Msg> { panel(row(items).spacing(8.0).align(Align::Center).width(Length::FILL)).color(t.card).radius(8.0).padding(Padding::hv(6.0, 10.0)).width(Length::FILL) };
        let b = |text: &str, m: Msg, accent: bool| -> Element<Msg> { button(label(text).size(12.0)).look(if accent { ButtonLook::Accent } else { ButtonLook::Plain }).padding(Padding::hv(3.0, 10.0)).on_press(m).name(text) };
        let mut out = Vec::new();
        match &self.ask {
            Some(Ask::Clash { names, .. }) => {
                let what = if names.len() == 1 { format!("“{}” is already here.", names[0]) } else { format!("{} items of these names are already here.", names.len()) };
                out.push(strip(vec![
                    label(what).size(12.0).weight(600).wrap().width(Length::FILL),
                    b("Cancel", Msg::Answer(None), false),
                    b("Skip", Msg::Answer(Some(ops::Clash::Skip)), false),
                    b("Keep both", Msg::Answer(Some(ops::Clash::KeepBoth)), false),
                    b("Replace", Msg::Answer(Some(ops::Clash::Replace)), true),
                ]));
            }
            Some(Ask::Forever(paths)) => {
                let what = if paths.len() == 1 { format!("Delete “{}” for good? It cannot be undone.", paths[0].file_name().unwrap_or_default().to_string_lossy()) } else { format!("Delete {} items for good? They cannot be undone.", paths.len()) };
                out.push(strip(vec![label(what).size(12.0).weight(600).wrap().width(Length::FILL), b("Cancel", Msg::Confirm(false), false), b("Delete", Msg::Confirm(true), true)]));
            }
            Some(Ask::EmptyTrash) => {
                out.push(strip(vec![label("Empty the Trash? What is in it cannot be got back.").size(12.0).weight(600).wrap().width(Length::FILL), b("Cancel", Msg::Confirm(false), false), b("Empty Trash", Msg::Confirm(true), true)]));
            }
            None => {}
        }
        if let Some(n) = &self.naming {
            out.push(strip(vec![
                label(if n.rename.is_some() { "New name" } else { "New folder" }).size(12.0),
                field(n.text.clone(), Msg::NameTyped).on_submit(Msg::NameDone).autofocus().width(Length::FILL).name("Name"),
                b("Cancel", Msg::NameCancel, false),
                b(if n.rename.is_some() { "Rename" } else { "Make" }, Msg::NameDone, true),
            ]));
        }
        if let Some(job) = &self.job {
            use std::sync::atomic::Ordering::Relaxed;
            let (done, total) = (job.progress.done.load(Relaxed), job.progress.total.load(Relaxed).max(1));
            let part = (done as f32 / total as f32).clamp(0.0, 1.0);
            let now = job.progress.now.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let bar = drawing(move |scene, _, r: Rect, t: &Theme| {
                let r = Rect::new(r.x, r.y + (r.h - 6.0) / 2.0, r.w, 6.0);
                scene.fill_rounded(r, 3.0, t.line);
                scene.fill_rounded(Rect::new(r.x, r.y, r.w * part, r.h), 3.0, t.accent);
            })
            .width(Length::Fixed(160.0))
            .height(Length::Fixed(16.0));
            out.push(strip(vec![
                label(format!("{} · {:.0}% · {} of {}", job.words, part * 100.0, files::size_words(done), files::size_words(total))).size(12.0),
                bar,
                label(now).size(11.0).dim().elide().width(Length::FILL),
                b("Stop", Msg::StopJob, false),
            ]));
        }
        out
    }

    // --- Views ---

    fn tool_strip(&self, t: &Theme) -> Element<Msg> {
        let tab = self.front_tab();
        let can_back = tab.is_some_and(|t| !t.back.is_empty());
        let can_forward = tab.is_some_and(|t| !t.forward.is_empty());
        let can_up = tab.and_then(|t| t.dir()).is_some_and(|d| d.parent().is_some());
        let mut items = vec![group(vec![
            tool(Icon::Back, "Back", can_back.then_some(Msg::Run(cmd::BACK))),
            tool(Icon::Forward, "Forward", can_forward.then_some(Msg::Run(cmd::FORWARD))),
            tool(Icon::Up, "Up", can_up.then_some(Msg::Run(cmd::UP))),
        ])];
        // The path: each folder from Home (or the computer) a button.
        let mut crumbs: Vec<Element<Msg>> = Vec::new();
        match tab.map(|t| &t.showing) {
            Some(Showing::Recent) | None => crumbs.push(label("Recent").size(13.0).weight(600)),
            Some(Showing::Folder(dir)) => {
                let home = files::home();
                let (mut at, rest, first) = match home.as_ref().and_then(|h| dir.strip_prefix(h).ok().map(|r| (h.clone(), r.to_path_buf()))) {
                    Some((h, r)) => (h, r, "Home".to_string()),
                    None => (PathBuf::from("/"), dir.strip_prefix("/").map(Path::to_path_buf).unwrap_or_default(), "Computer".to_string()),
                };
                let mut parts = vec![(first, at.clone())];
                for c in rest.components() {
                    at = at.join(c);
                    parts.push((c.as_os_str().to_string_lossy().into_owned(), at.clone()));
                }
                // A deep folder: its last few.
                let skip = parts.len().saturating_sub(5);
                if skip > 0 {
                    crumbs.push(label("…").size(12.0).dim());
                }
                let n = parts.len();
                for (i, (name, p)) in parts.into_iter().enumerate().skip(skip) {
                    if i > skip || skip > 0 {
                        crumbs.push(label("›").size(12.0).dim());
                    }
                    let last = i + 1 == n;
                    crumbs.push(button(label(name.clone()).size(12.0).weight(if last { 600 } else { 400 })).look(ButtonLook::Plain).padding(Padding::hv(3.0, 6.0)).on_press(Msg::Crumb(p)).name(name));
                }
            }
        }
        items.push(row(crumbs).spacing(2.0).align(Align::Center));
        items.push(flexible());
        let any = tab.is_some_and(|t| !t.chosen.is_empty());
        if self.in_trash() {
            items.push(button(label("Put back").size(12.0)).padding(Padding::hv(3.0, 10.0)).on_press_if(any, Msg::Run(cmd::RESTORE)).name("Put back"));
            items.push(button(label("Empty Trash").size(12.0)).padding(Padding::hv(3.0, 10.0)).on_press(Msg::Run(cmd::EMPTY_TRASH)).name("Empty Trash"));
        } else {
            items.push(group(vec![
                tool(Icon::NewFolder, "New folder", self.here().is_some().then_some(Msg::Run(cmd::NEW_FOLDER))),
                tool(Icon::Trash, "Move to the Trash", any.then_some(Msg::Run(cmd::TRASH))),
            ]));
        }
        items.push(field(self.narrow.clone(), Msg::Narrow).placeholder("Narrow this folder").width(Length::Fixed(200.0)).name("Narrow this folder"));
        items.push(group(vec![
            toggle(Icon::List, "As a list", self.view == ViewKind::List, Msg::Run(cmd::LIST)),
            toggle(Icon::Grid, "As icons", self.view == ViewKind::Icons, Msg::Run(cmd::ICONS)),
        ]));
        items.push(group(vec![toggle(Icon::Pane, "Preview pane", self.pane, Msg::Run(cmd::PANE)), toggle(Icon::Hidden, "Hidden files", self.hidden, Msg::Run(cmd::HIDDEN))]));
        toolbar(items, t)
    }

    fn places(&self) -> Vec<Place> {
        let mut p = files::places();
        if let Some(trash) = ops::trash_dir().map(|d| d.join("files")) {
            // After the computer and its disks.
            p.push(Place { name: "Trash".into(), path: Some(trash) });
        }
        p
    }

    fn folder_view(&self, t: &Theme) -> Element<Msg> {
        let Some(tab) = self.front_tab() else { return space() };
        let visible = self.visible();
        let chosen = |_: usize, e: &Entry| tab.chosen.contains(&e.path);
        let click = |k: usize| Msg::Click(k);
        let in_view = tab.cursor.as_ref().and_then(|c| visible.iter().position(|e| &e.path == c)).map(|k| (k, tab.turn << 32 | k as u64));
        let drag = |k: usize| Msg::DragRow(k);
        let v = files::View { entries: &visible, chosen: &chosen, in_view, click: &click, drag: Some(&drag), folders: tab.showing == Showing::Recent };
        let mut body: Vec<Element<Msg>> = Vec::new();
        if let Some(p) = &tab.problem {
            body.push(label(p.clone()).size(12.0).wrap().width(Length::FILL));
        }
        if visible.is_empty() && tab.problem.is_none() {
            let why = if !self.narrow.is_empty() { "Nothing here has that in its name." } else if tab.showing == Showing::Recent { "Nothing opened or saved yet." } else { "This folder is empty." };
            body.push(panel(label(why).size(12.0).dim()).padding(12.0));
        }
        let list = match self.view {
            ViewKind::List => {
                let (head, rows) = files::list(&v, self.sort, Msg::Sort);
                let mut all = vec![head, rule()];
                all.extend(body);
                all.push(scroll(column(rows).spacing(1.0).width(Length::FILL)));
                column(all).spacing(2.0).width(Length::FILL).height(Length::FILL)
            }
            ViewKind::Icons => {
                body.push(scroll(panel(files::icons(&v)).padding(6.0).width(Length::FILL)));
                column(body).spacing(2.0).width(Length::FILL).height(Length::FILL)
            }
        };
        panel(list).color(t.page).radius(8.0).edge(t.line, 1.0).padding(4.0).width(Length::FILL).height(Length::FILL)
    }

    /// A picture fitted into its place.
    fn fitted(img: Arc<Image>) -> Element<Msg> {
        drawing(move |scene, _, r: Rect, _t: &Theme| {
            let s = (r.w / img.width as f32).min(r.h / img.height as f32).min(4.0);
            let (w, h) = (img.width as f32 * s, img.height as f32 * s);
            let id = scene.image(img.clone());
            scene.draw_image(id, Rect::new(r.x + (r.w - w) / 2.0, r.y + (r.h - h) / 2.0, w, h), 1.0);
        })
        .width(Length::FILL)
        .height(Length::FILL)
    }

    /// What a file is, in a few lines: name, kind, size, when it changed.
    fn facts(e: &Entry) -> Vec<Element<Msg>> {
        let kind = if e.dir { "Folder".to_string() } else { vergence_types::describe(e.mime.unwrap_or(vergence_types::of_file(&e.path))) };
        let mut lines = vec![label(e.name.clone()).size(13.0).weight(600).wrap().width(Length::FILL), label(kind).size(11.0).dim()];
        if !e.dir {
            lines.push(label(files::size_words(e.size)).size(11.0).dim());
        } else {
            let n = std::fs::read_dir(&e.path).map(|r| r.count()).unwrap_or(0);
            lines.push(label(format!("{n} item{}", if n == 1 { "" } else { "s" })).size(11.0).dim());
        }
        if let Some(c) = e.changed {
            lines.push(label(format!("Changed {}", files::when(c))).size(11.0).dim());
        }
        lines
    }

    /// The preview of a file: its picture, the start of its text, or its mark.
    fn look_at(&self, e: &Entry, big: bool, t: &Theme) -> Element<Msg> {
        if e.kind() == Kind::Picture {
            if let Some(img) = self.picture_of(&e.path) {
                return Viewer::fitted(img);
            }
            return panel(label("…").size(14.0).dim()).width(Length::FILL).height(Length::FILL);
        }
        if !e.dir
            && e.kind() == Kind::Text
            && let Some(text) = files::text_start(&e.path)
        {
            let lines: Vec<Element<Msg>> = text.lines().take(if big { 60 } else { 24 }).map(|l| label(if l.is_empty() { " ".to_string() } else { l.replace('\t', "    ").chars().take(120).collect() }).size(if big { 12.0 } else { 10.0 }).color(t.dim)).collect();
            return panel(column(lines).spacing(1.0).width(Length::FILL)).color(t.card).radius(6.0).padding(8.0).width(Length::FILL).height(Length::FILL);
        }
        panel(files::kind_icon_sized(e.kind(), if big { 8.0 } else { 5.0 }).width(Length::FILL).height(Length::Fixed(if big { 150.0 } else { 90.0 }))).width(Length::FILL).height(Length::FILL)
    }

    fn pane_view(&self, t: &Theme) -> Element<Msg> {
        let mut parts: Vec<Element<Msg>> = Vec::new();
        match self.cursor_entry() {
            Some(e) => {
                parts.push(panel(self.look_at(&e, false, t)).width(Length::FILL).height(Length::Fixed(200.0)));
                parts.extend(Viewer::facts(&e));
                if !e.dir {
                    parts.push(space().height(Length::Fixed(6.0)));
                    parts.push(button(label("Open").size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(4.0, 12.0)).on_press(Msg::OpenChosen).name("Open"));
                    if matches!(e.kind(), Kind::Picture | Kind::Text) {
                        parts.push(button(label("Open in Close-Up").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(Msg::OpenInCloseUp).name("Open in Close-Up"));
                    }
                }
            }
            None => {
                let n = self.visible().len();
                parts.push(label(self.front_tab().map(Tab::name).unwrap_or_default()).size(13.0).weight(600));
                parts.push(label(format!("{n} item{}", if n == 1 { "" } else { "s" })).size(11.0).dim());
            }
        }
        panel(scroll(column(parts).spacing(4.0).width(Length::FILL))).color(t.card).radius(8.0).padding(10.0).width(Length::Fixed(PANE_W)).height(Length::FILL)
    }

    /// Space's preview, over the folder: the file large, its facts under it.
    fn preview_view(&self, t: &Theme) -> Option<Element<Msg>> {
        let e = self.cursor_entry()?;
        let mut facts = Viewer::facts(&e);
        facts.push(
            row(vec![
                label("Space or Escape closes · the arrows move on · Enter opens").size(10.0).dim().width(Length::FILL),
                button(label("Close").size(12.0)).padding(Padding::hv(3.0, 10.0)).on_press(Msg::ClosePreview).name("Close the preview"),
            ])
            .spacing(8.0)
            .align(Align::Center)
            .width(Length::FILL),
        );
        let card = panel(column(vec![panel(self.look_at(&e, true, t)).width(Length::FILL).height(Length::FILL), column(facts).spacing(2.0).width(Length::FILL)]).spacing(10.0).width(Length::FILL).height(Length::FILL))
            .color(t.card)
            .radius(12.0)
            .edge(t.line, 1.0)
            .padding(14.0)
            .width(Length::FILL)
            .height(Length::FILL);
        let dim = Color { a: 120, ..Color::rgb(0, 0, 0) };
        Some(panel(card).color(dim).radius(0.0).padding(Padding { top: 30.0, bottom: 30.0, left: 60.0, right: 60.0 }).width(Length::FILL).height(Length::FILL))
    }
}

impl Program for Viewer {
    type Message = Msg;

    fn title(&self) -> String {
        self.front_tab().map(Tab::name).unwrap_or_else(|| "File Viewer".into())
    }

    fn app_id(&self) -> String {
        APP_ID.into()
    }

    fn theme(&self) -> Theme {
        Theme::from_desktop(&self.desktop, false, SECTION)
    }

    fn bar_style(&self) -> Option<BarStyle> {
        Some(vergence_ui::bar_style_setting(&self.desktop, SECTION, BarStyle::Light))
    }

    fn under_bar(&self) -> bool {
        true
    }

    fn bar(&self) -> Option<Bar> {
        let tabs = self.tabs.iter().map(|t| BarTab { id: t.id, title: t.name(), closable: true, alert: false, edited: false }).collect();
        Some(Bar::Tabs { tabs, front: self.front_tab().map_or(0, |t| t.id), plus: true, accent: None, menu: Vec::new() })
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let places = self.places();
        let here = self.front_tab().and_then(|t| t.dir().map(Path::to_path_buf));
        let sidebar = files::sidebar(&places, here.as_deref(), |p| Msg::Go(p.clone()), t);
        let mut middle = vec![sidebar, self.folder_view(t)];
        if self.pane {
            middle.push(self.pane_view(t));
        }
        let middle: Element<Msg> = row(middle).spacing(8.0).width(Length::FILL).height(Length::FILL);
        let middle = match self.preview.then(|| self.preview_view(t)).flatten() {
            Some(p) => stack(vec![middle, p]).width(Length::FILL).height(Length::FILL),
            None => middle,
        };
        let mut all = vec![self.tool_strip(t)];
        if let Some(n) = &self.notice {
            all.push(
                panel(row(vec![label(n.clone()).size(12.0).wrap().width(Length::FILL), button(label("OK").size(12.0)).padding(Padding::hv(2.0, 10.0)).on_press(Msg::DismissNotice).name("OK")]).spacing(8.0).align(Align::Center).width(Length::FILL))
                    .padding(Padding::hv(6.0, 12.0))
                    .width(Length::FILL),
            );
        }
        let strips = self.strips(t);
        if !strips.is_empty() {
            all.push(panel(column(strips).spacing(4.0).width(Length::FILL)).padding(Padding { top: 8.0, bottom: 0.0, left: 8.0, right: 8.0 }).radius(0.0).width(Length::FILL));
        }
        all.push(panel(middle).padding(Padding { top: 8.0, bottom: 8.0, left: 8.0, right: 8.0 }).radius(0.0).width(Length::FILL).height(Length::FILL));
        // The foot: how many, how many chosen.
        let n = self.visible().len();
        let chosen = self.front_tab().map_or(0, |t| t.chosen.len());
        let foot = if chosen > 1 { format!("{chosen} of {n} chosen") } else { format!("{n} item{}", if n == 1 { "" } else { "s" }) };
        all.push(rule());
        all.push(panel(row(vec![space().width(Length::FILL), label(foot).size(12.0).dim()]).width(Length::FILL)).radius(0.0).padding(Padding { top: 3.0, bottom: 3.0, left: 12.0, right: 12.0 }).width(Length::FILL));
        column(all).width(Length::FILL).height(Length::FILL)
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        match m {
            Msg::Run(c) => return self.run(c, window),
            Msg::Go(p) => self.go(p.path.map_or(Showing::Recent, Showing::Folder)),
            Msg::Crumb(p) => self.go(Showing::Folder(p)),
            Msg::Click(k) => self.click(k),
            Msg::Sort(by) => self.sort = if self.sort.0 == by { (by, !self.sort.1) } else { (by, by == SortBy::Name) },
            Msg::Narrow(s) => self.narrow = s,
            Msg::OpenChosen => self.open_chosen(),
            Msg::OpenInCloseUp => {
                if let Some(e) = self.cursor_entry() {
                    let _ = std::process::Command::new("vergence-closeup").arg(&e.path).spawn();
                }
            }
            Msg::DismissNotice => self.notice = None,
            Msg::ClosePreview => self.preview = false,
            Msg::DragRow(k) => self.drag_row(k, window),
            Msg::Answer(a) => {
                if let (Some(Ask::Clash { sources, dir, cut, .. }), Some(clash)) = (self.ask.take(), a) {
                    self.start_paste(sources, dir, cut, clash);
                }
            }
            Msg::Confirm(yes) => match self.ask.take() {
                Some(Ask::Forever(paths)) if yes => {
                    for p in &paths {
                        if let Err(e) = ops::remove_tree(p) {
                            self.notice = Some(format!("“{}” could not be deleted: {}", p.file_name().unwrap_or_default().to_string_lossy(), files::plain(&e)));
                            break;
                        }
                        // Deleted out of the Trash: its info goes too.
                        if self.in_trash()
                            && let (Some(dir), Some(n)) = (ops::trash_dir(), p.file_name())
                        {
                            let _ = std::fs::remove_file(dir.join("info").join(format!("{}.trashinfo", n.to_string_lossy())));
                        }
                    }
                    self.read_all();
                }
                Some(Ask::EmptyTrash) if yes => {
                    if let Err(e) = ops::empty_trash() {
                        self.notice = Some(format!("The Trash could not be emptied: {}", files::plain(&e)));
                    }
                    self.read_all();
                }
                _ => {}
            },
            Msg::NameTyped(s) => {
                if let Some(n) = &mut self.naming {
                    n.text = s;
                }
            }
            Msg::NameDone => self.name_done(),
            Msg::NameCancel => self.naming = None,
            Msg::StopJob => {
                if let Some(j) = &self.job {
                    j.progress.stop.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        self.want_picture();
        window.redraw();
    }

    fn saw(&mut self, input: &Input) {
        if let Input::Button { pressed: true, modifiers, .. } = input {
            self.mods = *modifiers;
        }
        if let Input::Key { modifiers, .. } = input {
            self.mods = *modifiers;
        }
    }

    fn key_first(&mut self, input: &Input, window: &mut Window) -> Option<bool> {
        const UP: u32 = 0xff52;
        const DOWN: u32 = 0xff54;
        const LEFT: u32 = 0xff51;
        const RIGHT: u32 = 0xff53;
        const RETURN: u32 = 0xff0d;
        const ESCAPE: u32 = 0xff1b;
        const BACKSPACE: u32 = 0xff08;
        const SPACE: u32 = 0x20;
        const DELETE: u32 = 0xffff;
        const F2: u32 = 0xffbf;
        let Input::Key { sym, pressed: true, modifiers: m, .. } = input else { return None };
        self.mods = *m;
        // A name being typed: its field has the keys (Escape cancels it).
        if self.naming.is_some() {
            if *sym == ESCAPE {
                self.naming = None;
                window.redraw();
                return Some(true);
            }
            return None;
        }
        let typing = !self.narrow.is_empty();
        match *sym {
            ESCAPE if self.ask.is_some() => self.ask = None,
            DELETE if m.shift && !typing => self.run(cmd::DELETE, window),
            DELETE if !typing => self.run(cmd::TRASH, window),
            F2 => self.run(cmd::RENAME, window),
            s if m.ctrl && m.shift && (s == 0x6e || s == 0x4e) => self.run(cmd::NEW_FOLDER, window),
            s if m.ctrl && (s == 0x78 || s == 0x58) && !typing => self.run(cmd::CUT, window),
            s if m.ctrl && (s == 0x63 || s == 0x43) && !typing => self.run(cmd::COPY, window),
            s if m.ctrl && (s == 0x76 || s == 0x56) && !typing => self.run(cmd::PASTE, window),
            s if m.ctrl && (s == 0x64 || s == 0x44) => self.run(cmd::DUPLICATE, window),
            s if m.ctrl && (s == 0x7a || s == 0x5a) && !typing => self.run(cmd::UNDO, window),
            UP if m.alt => self.run(cmd::UP, window),
            LEFT if m.alt => self.run(cmd::BACK, window),
            RIGHT if m.alt => self.run(cmd::FORWARD, window),
            DOWN if m.alt || m.ctrl => self.run(cmd::OPEN, window),
            UP => {
                self.move_cursor(if self.view == ViewKind::Icons { -self.per_row() } else { -1 });
                self.want_picture();
            }
            DOWN => {
                self.move_cursor(if self.view == ViewKind::Icons { self.per_row() } else { 1 });
                self.want_picture();
            }
            LEFT if self.view == ViewKind::Icons || self.preview => {
                self.move_cursor(-1);
                self.want_picture();
            }
            RIGHT if self.view == ViewKind::Icons || self.preview => {
                self.move_cursor(1);
                self.want_picture();
            }
            RETURN => {
                self.preview = false;
                self.run(cmd::OPEN, window)
            }
            ESCAPE if self.preview => self.preview = false,
            ESCAPE if typing => self.narrow.clear(),
            SPACE if !typing => self.run(cmd::PREVIEW, window),
            BACKSPACE if !typing => self.run(cmd::UP, window),
            s if m.ctrl && (s == 0x61 || s == 0x41) && !typing => self.run(cmd::SELECT_ALL, window),
            s if m.ctrl && !m.shift && (s == 0x6e || s == 0x4e) => self.run(cmd::NEW_WINDOW, window),
            s if m.ctrl && (s == 0x74 || s == 0x54) => self.run(cmd::NEW_TAB, window),
            s if m.ctrl && (s == 0x77 || s == 0x57) => self.run(cmd::CLOSE_TAB, window),
            s if m.ctrl && (s == 0x68 || s == 0x48) => self.run(cmd::HIDDEN, window),
            s if m.ctrl && s == 0x2c => self.run(cmd::SETTINGS, window),
            s if m.ctrl && (s == 0x31) => self.run(cmd::LIST, window),
            s if m.ctrl && (s == 0x32) => self.run(cmd::ICONS, window),
            s if m.ctrl && (s == 0x72 || s == 0x52) => self.run(cmd::RELOAD, window),
            _ => return None,
        }
        window.redraw();
        Some(true)
    }

    /// Files dragged here go into the folder shown: moved, or copied with Ctrl.
    fn drag_over(&mut self, _x: f32, _y: f32, kinds: &[String], ctrl: bool) -> u8 {
        if !kinds.iter().any(|k| k == "text/uri-list") || self.here().is_none() || self.in_trash() || self.job.is_some() {
            return vergence_ui::DROP_NONE;
        }
        if ctrl { vergence_ui::DROP_COPY } else { vergence_ui::DROP_MOVE }
    }

    fn dropped(&mut self, _x: f32, _y: f32, action: u8, items: &[(String, Vec<u8>)], window: &mut Window) {
        let Some(dir) = self.here() else { return };
        let paths: Vec<PathBuf> = items.iter().find(|(k, _)| k == "text/uri-list").map(|(_, v)| String::from_utf8_lossy(v).lines().filter_map(|l| vergence_types::path_of_link(l.trim())).collect()).unwrap_or_default();
        if paths.is_empty() {
            return;
        }
        let cut = action == vergence_ui::DROP_MOVE;
        let names = ops::clashes(&paths, &dir);
        if names.is_empty() {
            self.start_paste(paths, dir, cut, ops::Clash::KeepBoth);
        } else {
            self.ask = Some(Ask::Clash { sources: paths, dir, cut, names });
        }
        window.redraw();
    }

    /// A drag from here ended: what moved away is read again.
    fn drag_ended(&mut self, _action: u8, window: &mut Window) {
        self.read_all();
        window.redraw();
    }

    fn resized(&mut self, width: f32, _height: f32) {
        self.width = width;
    }

    fn waker(&mut self, waker: Waker) {
        let first = self.waker.is_none();
        self.waker = Some(waker.clone());
        if first {
            let inbox = self.shared.borrow().inbox.clone();
            let w = waker.clone();
            let _ = hand::listen(inbox, move || w.wake());
        }
        // Once woken, the folder watch's timer is set (woken() has the window).
        waker.wake();
    }

    fn opened(&mut self, _id: u32) {}

    fn woken(&mut self, window: &mut Window) -> bool {
        let mut changed = false;
        let (text, stamp) = desktop_text();
        if stamp != self.stamp {
            (self.desktop, self.stamp) = (text, stamp);
            changed = true;
        }
        let asked: Vec<Vec<PathBuf>> = std::mem::take(&mut *self.shared.borrow().inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for paths in asked {
            self.open_paths(&paths);
            // Handed over by a start the person just made: forward (the server judges).
            window.activate();
            changed = true;
        }
        // A copy or move running (its ticks) or ended.
        if self.job.is_some() {
            self.job_ended();
            changed = true;
        }
        // A picture decoded: drawn if it is the cursor's.
        if self.cursor_entry().is_some_and(|e| self.picture_of(&e.path).is_some()) {
            changed = true;
        }
        window.wake_after(WATCH_TIMER, WATCH_EVERY);
        changed
    }

    fn timer(&mut self, id: u64, window: &mut Window) -> bool {
        if id != WATCH_TIMER {
            return false;
        }
        window.wake_after(WATCH_TIMER, WATCH_EVERY);
        // The folder changed on disk: read again.
        let Some(t) = self.front_mut() else { return false };
        let now = t.dir().and_then(|d| std::fs::metadata(d).and_then(|m| m.modified()).ok());
        if t.dir().is_some() && now != t.stamp {
            t.read();
            return true;
        }
        false
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        Some((
            false,
            vec![
                C::role(cmd::NEW_WINDOW, Role::NewWindow).featured(),
                C::role(cmd::NEW_TAB, Role::NewTab),
                C::role(cmd::CLOSE_TAB, Role::CloseTab),
                C::own(cmd::OPEN, "File", "Open", "Enter").featured(),
                C::own(cmd::PREVIEW, "File", "Preview", "Space").checked(self.preview),
                C::role(cmd::SELECT_ALL, Role::SelectAll),
                C::role(cmd::UNDO, Role::Undo),
                C::role(cmd::CUT, Role::Cut),
                C::role(cmd::COPY, Role::Copy),
                C::role(cmd::PASTE, Role::Paste),
                C::own(cmd::DUPLICATE, "Edit", "Duplicate", "Ctrl+D"),
                C::own(cmd::RENAME, "Edit", "Rename…", "F2"),
                C::own(cmd::NEW_FOLDER, "File", "New folder", "Ctrl+Shift+N").featured(),
                C::own(cmd::TRASH, "File", "Move to the Trash", "Delete"),
                C::own(cmd::DELETE, "File", "Delete for good…", "Shift+Delete"),
                C::own(cmd::EMPTY_TRASH, "File", "Empty Trash…", ""),
                C::own(cmd::LIST, "View", "As a list", "Ctrl+1").checked(self.view == ViewKind::List),
                C::own(cmd::ICONS, "View", "As icons", "Ctrl+2").checked(self.view == ViewKind::Icons),
                C::own(cmd::PANE, "View", "Preview pane", "").checked(self.pane).featured(),
                C::own(cmd::HIDDEN, "View", "Hidden files", "Ctrl+H").checked(self.hidden),
                C::own(cmd::RELOAD, "View", "Read again", "Ctrl+R"),
                C::own(cmd::BACK, "Go", "Back", "Alt+Left"),
                C::own(cmd::FORWARD, "Go", "Forward", "Alt+Right"),
                C::own(cmd::UP, "Go", "Up", "Alt+Up"),
                C::role(cmd::NEXT_TAB, Role::NextTab),
                C::role(cmd::PREV_TAB, Role::PreviousTab),
                C::role(cmd::SETTINGS, Role::Settings),
                C::role(cmd::ABOUT, Role::About),
            ],
        ))
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        self.run(id, window);
    }

    fn tab(&mut self, event: TabEvent, window: &mut Window) {
        match event {
            TabEvent::Chosen(id) => {
                self.front = id;
                self.narrow.clear();
            }
            TabEvent::Close(id) => self.close_tab(id, window),
            TabEvent::CloseOthers(id) => self.tabs.retain(|t| t.id == id),
            TabEvent::New => self.run(cmd::NEW_TAB, window),
            TabEvent::Moved { tab, to } => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == tab) {
                    let t = self.tabs.remove(i);
                    self.tabs.insert((to as usize).min(self.tabs.len()), t);
                }
            }
            TabEvent::Out { tab, .. } => {
                if self.tabs.len() > 1
                    && let Some(i) = self.tabs.iter().position(|t| t.id == tab)
                {
                    let t = self.tabs.remove(i);
                    if self.front == tab {
                        self.front = self.tabs.first().map_or(0, |t| t.id);
                    }
                    let mut w = Viewer::new(self.shared.clone());
                    w.front = t.id;
                    w.tabs.push(t);
                    window.open_window(vergence_widgets::window(w));
                }
            }
            _ => {}
        }
        window.redraw();
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { min: (560.0, 360.0), ..Default::default() }
    }
}

struct FilesRows;

impl Rows for FilesRows {
    fn parts(&self, text: &str) -> Vec<Part> {
        let opacity = vergence_ui::opacity_setting(text, "[appearance]");
        let locked = vergence_ui::desktop_pick(text).map(|light| format!("{} for every app: Settings > Appearance > Style", if light { "Light" } else { "Dark" }));
        let styles = vec![Choice::text("Dark", "dark"), Choice::text("Light", "light")];
        vec![
            Part {
                title: "File Viewer".into(),
                rows: vec![
                    Row::new("view", "New windows show", "A folder as a list or as icons.", Control::Choose { choices: vec![Choice::text("A list", "list"), Choice::text("Icons", "icons")], desktop: None, app_default: 0 }),
                    Row::new("preview-pane", "Preview pane", "The chosen file at the window's right.", Control::Switch { default: false }),
                    Row::new("hidden", "Hidden files", "Names that start with a dot.", Control::Switch { default: false }),
                ],
            },
            Part {
                title: "On this desktop".into(),
                rows: vec![
                    Row::new("style", "Style", "While the desktop's Style is App default.", Control::Choose { choices: styles.clone(), desktop: Some("Built-in (Light)".into()), app_default: 0 }).locked(locked.clone()),
                    Row::new("bar-style", "Title bar", "", Control::Choose { choices: styles, desktop: Some("Built-in (Light)".into()), app_default: 0 }).locked(locked),
                    Row::new("opacity", "Opacity", "30 to 100: how much of the window's ground shows.", Control::Percent { min: 30, desktop: format!("Desktop default ({:.0})", opacity * 100.0), now: (opacity * 100.0).round() as u32 }),
                    Row::new("", "Window places", "Where the desktop puts File Viewer's windows, for each set of screens.", Control::Button { label: "Forget…".into(), id: 1 }),
                ],
            },
        ]
    }

    fn button(&self, id: u32) {
        if id == 1 {
            let _ = std::process::Command::new("vergence-settings").args(["--page", "windows", "--forget", APP_ID]).spawn();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("vergence-files {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    // The desktop's icons (desktop.rs): a process of their own, started by the session.
    if args.iter().any(|a| a == "--desktop") {
        return desktop::run();
    }
    let paths: Vec<PathBuf> = args.iter().filter(|a| !a.starts_with("--")).map(|a| vergence_types::path_of_link(a).unwrap_or_else(|| PathBuf::from(a))).map(|p| std::path::absolute(&p).unwrap_or(p)).collect();
    if hand::hand_over(&paths) {
        return;
    }
    let shared = Rc::new(RefCell::new(Shared { inbox: Arc::default(), next_tab: 0, clip: None, undo: Vec::new() }));
    let mut v = Viewer::new(shared);
    v.open_paths(&paths);
    if let Err(e) = vergence_widgets::run(v, (1040, 680)) {
        eprintln!("vergence-files: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> SharedRef {
        Rc::new(RefCell::new(Shared { inbox: Arc::default(), next_tab: 0, clip: None, undo: Vec::new() }))
    }

    #[test]
    fn a_tab_goes_back_forward_and_up() {
        let dir = std::env::temp_dir().join(format!("vergence-files-{}", std::process::id()));
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        let mut v = Viewer::new(shared());
        v.open_paths(std::slice::from_ref(&dir));
        assert_eq!(v.visible().iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["sub", "a.txt"], "folders first");
        v.go(Showing::Folder(sub.clone()));
        v.up();
        let t = v.front_tab().unwrap();
        assert_eq!(t.showing, Showing::Folder(dir.clone()));
        assert_eq!(t.chosen, vec![sub.clone()], "the folder come up from is chosen");
        v.back();
        assert_eq!(v.front_tab().unwrap().showing, Showing::Folder(sub.clone()));
        v.forward();
        assert_eq!(v.front_tab().unwrap().showing, Showing::Folder(dir.clone()));
        // A file given opens its folder with it chosen.
        v.open_paths(&[dir.join("a.txt")]);
        assert_eq!(v.front_tab().unwrap().chosen, vec![dir.join("a.txt")]);
        v.narrow = "zzz".into();
        assert!(v.visible().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_arrows_and_clicks_choose() {
        let dir = std::env::temp_dir().join(format!("vergence-files-c-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["1.txt", "2.txt", "3.txt"] {
            std::fs::write(dir.join(n), "x").unwrap();
        }
        let mut v = Viewer::new(shared());
        v.open_paths(std::slice::from_ref(&dir));
        v.move_cursor(1);
        v.move_cursor(1);
        assert_eq!(v.front_tab().unwrap().chosen, vec![dir.join("2.txt")]);
        v.mods.shift = true;
        v.click(2);
        assert_eq!(v.front_tab().unwrap().chosen.len(), 2, "Shift+click: a run");
        v.mods = Modifiers { ctrl: true, ..Modifiers::default() };
        v.click(0);
        assert_eq!(v.front_tab().unwrap().chosen.len(), 3, "Ctrl+click adds");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
