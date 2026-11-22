//! File Viewer (files milestone 1, step 7; docs/files-milestone-1.md §5):
//! Vergence's file manager, named after NeXTSTEP's Workspace Manager window
//! (Dave, 2026-11-15). Dave's words: "with space bar preview ala macOS
//! finder ... remote file system support as well (using native OS tools?)".
//!
//! - **The window**: the toolbar (Back, Forward, Up, the folder's path,
//!   List or Icons, the preview pane, hidden files, a field that narrows the
//!   folder as you type), the places sidebar, the folder as a list or icons
//!   (vergence-widgets files.rs, shared with the Open dialog), the preview
//!   pane at the right. A folder a tab in the bar (tabs back in files
//!   milestone 3, Dave 2026-11-21, after milestone 2 had taken them out):
//!   torn off into windows of their own and joined again; a folder handed
//!   over opens in a tab of the window last used; each window's tabs kept
//!   across restarts (keep.rs). One process for all of them (hand.rs).
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
mod keep;
mod listen;
mod ops;
mod preview;
mod props;
mod search;
mod sidebar;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use vergence_scene::{Image, Rect};
use vergence_ui::{AppCommand as C, Bar, BarStyle, BarTab, MenuLine, Modifiers, Role, TabEvent};
use vergence_widgets::files::{self, Entry, GroupBy, Kind, ListLook, Place, RowSize, SortBy};
use vergence_widgets::prefs::{Choice, Control, Part, Prefs, Row, Rows};
use vergence_widgets::toolbar::{Icon, flexible, group, tool, toggle, toolbar};
use vergence_widgets::{Align, ButtonLook, Element, Input, Length, Padding, Program, Theme, Waker, Window, button, column, drawing, field, label, panel, row, rule, scroll, space, stack};

const APP_ID: &str = "vergence-files";
/// The Trash window's (Dave, 2026-11-18): a process of its own, so the dock
/// shows it with the Trash's icon and window memory keeps its place apart.
const TRASH_ID: &str = "vergence-trash";
const SECTION: &str = "[files]";
/// How often the folder shown is looked at for changes.
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(2);
const WATCH_TIMER: u64 = 1;
/// A click on the one chosen item, then a pause: its name becomes a field
/// (files milestone 3, step 5), unless a second click made it a double.
const RENAME_TIMER: u64 = 2;
/// A double click's longest gap when the desktop says none (Settings > Mouse does).
const DOUBLE: std::time::Duration = std::time::Duration::from_millis(450);
/// The preview pane's width.
const PANE_W: f32 = 260.0;
/// A Trash row's own buttons (Put Back, Delete Forever), each this square.
const ROW_TOOL: f32 = 22.0;

mod cmd {
    pub const NEW_WINDOW: u32 = 1;
    pub const NEW_TAB: u32 = 2;
    pub const CLOSE: u32 = 3;
    pub const CLOSE_TAB: u32 = 17;
    pub const NEXT_TAB: u32 = 14;
    pub const PREV_TAB: u32 = 15;
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
    pub const EJECT: u32 = 31;
    pub const SETTINGS: u32 = 40;
    pub const ABOUT: u32 = 41;
    // The right-click menus' own (backup milestone 1, step 1).
    pub const SHOW_IN_FOLDER: u32 = 50;
    pub const MAKE_LINK: u32 = 51;
    pub const COMPRESS: u32 = 52;
    pub const INFO: u32 = 53;
    pub const VERSIONS: u32 = 54;
    pub const COPY_PATH: u32 = 55;
    pub const TERMINAL: u32 = 56;
    pub const NEW_FILE: u32 = 57;
    pub const SORT_NAME: u32 = 58;
    pub const SORT_SIZE: u32 = 59;
    pub const SORT_CHANGED: u32 = 60;
    pub const SORT_KIND: u32 = 61;
    pub const ROWS_SMALL: u32 = 62;
    pub const ROWS_MEDIUM: u32 = 63;
    pub const ROWS_LARGE: u32 = 64;
    // Group by (files milestone 3, step 4).
    pub const GROUP_KIND: u32 = 65;
    pub const GROUP_DATE: u32 = 66;
    pub const GROUP_SIZE: u32 = 67;
    pub const GROUP_NONE: u32 = 68;
    // A sidebar row's menu (step 3), for the row it was opened on.
    pub const SIDE_NEW_TAB: u32 = 70;
    pub const SIDE_REMOVE: u32 = 71;
    pub const SIDE_SHOW: u32 = 72;
    pub const SIDE_HIDE_RECENT: u32 = 73;
    pub const SIDE_EJECT: u32 = 74;
    pub const MOUNT_IMAGE: u32 = 75;
    /// Open With's applications: this, plus their place in the list.
    pub const OPEN_WITH: u32 = 1000;
    pub const MAX_OPEN_WITH: usize = 16;
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
    /// A new empty file, not a folder.
    file: bool,
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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
    /// The window's counted folder sizes, put into the entries as they are read.
    sizes: Sizes,
    /// In the Trash: where each item came from (its path in the Trash to
    /// the path it had).
    from: std::collections::HashMap<PathBuf, PathBuf>,
    /// How far the folder is scrolled (points, as the toolkit last told),
    /// where to scroll it at the next layout, and where each folder of its
    /// history was left (files milestone 3, step 2: Back and Forward bring a
    /// folder back at its place, for the session).
    scroll: f32,
    want: Option<f32>,
    places: std::collections::HashMap<Showing, f32>,
}

impl Tab {
    fn new(id: u32, showing: Showing, sizes: Sizes) -> Tab {
        let mut t = Tab { id, showing, back: Vec::new(), forward: Vec::new(), entries: Vec::new(), problem: None, stamp: None, chosen: Vec::new(), cursor: None, turn: 0, sizes, from: Default::default(), scroll: 0.0, want: None, places: Default::default() };
        t.read();
        t
    }

    /// Leaving the folder shown: its place remembered.
    fn leave(&mut self) {
        self.places.insert(self.showing.clone(), self.scroll);
    }

    /// Arrived at the folder shown: at its place if it was here before, else the top.
    fn arrive(&mut self) {
        self.scroll = self.places.get(&self.showing).copied().unwrap_or(0.0);
        self.want = Some(self.scroll);
    }

    /// Counted sizes into the folders' entries, where the folder has not
    /// changed since; true if any entry's size changed.
    fn put_sizes(&mut self) -> bool {
        let all = self.sizes.lock().unwrap_or_else(|x| x.into_inner());
        let mut changed = false;
        for e in self.entries.iter_mut().filter(|e| e.dir) {
            if let Some((when, Some(n))) = all.get(&e.path)
                && *when == e.changed
                && e.size != *n
            {
                e.size = *n;
                changed = true;
            }
        }
        changed
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
            Showing::Folder(p) if ops::trash_dir().is_some_and(|t| t.join("files") == *p) => "Trash".into(),
            Showing::Folder(p) if p.as_os_str() == "/" => "Computer".into(),
            Showing::Folder(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()),
        }
    }

    fn read(&mut self) {
        self.problem = None;
        self.entries = match &self.showing {
            Showing::Folder(dir) => {
                self.stamp = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
                let mut v = files::read_folder(dir).unwrap_or_else(|why| {
                    self.problem = Some(why);
                    Vec::new()
                });
                self.from = match ops::trash_dir() {
                    Some(t) if t.join("files") == *dir => as_trashed(&mut v, &t),
                    _ => Default::default(),
                };
                v
            }
            Showing::Recent => files::recent(),
        };
        // Folders already counted keep their sizes (the Size sort uses them).
        self.put_sizes();
        // What was chosen and is gone is chosen no more.
        let here: std::collections::HashSet<&PathBuf> = self.entries.iter().map(|e| &e.path).collect();
        self.chosen.retain(|p| here.contains(p));
        if self.cursor.as_ref().is_some_and(|c| !here.contains(c)) {
            self.cursor = None;
        }
    }

    fn go(&mut self, to: Showing) {
        if to != self.showing {
            self.leave();
            self.back.push(std::mem::replace(&mut self.showing, to));
            self.forward.clear();
            self.arrive();
        }
        self.chosen.clear();
        self.cursor = None;
        self.read();
    }
}

/// The Trash's entries as its list shows them (Dave, 2026-11-18: "just a
/// list of the files and a restore or remove forever button"): each by the
/// name it had (two of a name have names of their own in the Trash), its
/// time the moment it was trashed (its info file's, written then), and
/// where it came from, returned by its path in the Trash.
fn as_trashed(entries: &mut [Entry], trash: &Path) -> std::collections::HashMap<PathBuf, PathBuf> {
    let info = Some(trash.join("info"));
    let all: std::collections::HashMap<String, ops::Trashed> = ops::trashed_in(trash).into_iter().map(|t| (t.name.clone(), t)).collect();
    let mut from = std::collections::HashMap::new();
    for e in entries.iter_mut() {
        let Some(t) = all.get(&e.name) else { continue };
        if let Some(i) = &info
            && let Ok(when) = std::fs::metadata(i.join(format!("{}.trashinfo", t.name))).and_then(|m| m.modified())
        {
            e.changed = Some(when);
        }
        if let Some(n) = t.from.file_name() {
            e.name = n.to_string_lossy().into_owned();
            // By the name it had, else what its bytes said.
            e.mime = files::kind_of(e.dir, &e.name).or(e.mime);
        }
        from.insert(e.path.clone(), t.from.clone());
    }
    from
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
    /// Each window by its toolkit number, to wake it (tabs moved to it,
    /// folders handed to it).
    windows: Vec<(u32, Waker)>,
    /// Tabs dropped on another window: the window, the place in its row, the tab.
    transfers: Vec<(u32, u32, Tab)>,
    /// Folders handed over, for the window last used (by its number).
    opens: Vec<(u32, Vec<PathBuf>)>,
    /// The window the person last pressed or typed in.
    last_used: Option<u32>,
    /// What each window keeps for the next start (keep.rs), and the windows
    /// kept from the last run still to open.
    snaps: Vec<(u32, keep::Snap)>,
    kept: Vec<keep::Snap>,
}

impl Shared {
    fn new() -> Shared {
        Shared { inbox: Arc::default(), next_tab: 0, clip: None, undo: Vec::new(), windows: Vec::new(), transfers: Vec::new(), opens: Vec::new(), last_used: None, snaps: Vec::new(), kept: Vec::new() }
    }
}

type SharedRef = Rc<RefCell<Shared>>;

/// A Shared of its own, for a Properties window opened from the desktop's
/// icons (its renames' undo is nobody's).
fn lone_shared() -> SharedRef {
    Rc::new(RefCell::new(Shared::new()))
}

/// The free bytes of the file system `dir` is on, from `df -kP` (as
/// vergence-sysinfo reads file systems, on Linux and FreeBSD alike).
fn df_free(dir: &Path) -> Option<u64> {
    let out = std::process::Command::new("df").arg("-kP").arg(dir).stderr(std::process::Stdio::null()).output().ok()?;
    parse_df_free(&String::from_utf8_lossy(&out.stdout))
}

/// `df -kP`'s second line: Filesystem, 1024-blocks, Used, Available, ... ;
/// the Available column in bytes.
fn parse_df_free(text: &str) -> Option<u64> {
    let line = text.lines().nth(1)?;
    let kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(kb * 1024)
}

/// A section's fold mark: down when open, right when folded.
fn fold_mark(open: bool) -> Element<Msg> {
    drawing(move |scene, _, r: Rect, t: &Theme| {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let pts = if open { [(-3.0, -1.5), (0.0, 1.5), (3.0, -1.5)] } else { [(-1.5, -3.0), (1.5, 0.0), (-1.5, 3.0)] };
        let p = |(dx, dy): (f32, f32)| vergence_scene::Point::new(cx + dx, cy + dy);
        scene.line(p(pts[0]), p(pts[1]), 1.4, t.dim);
        scene.line(p(pts[1]), p(pts[2]), 1.4, t.dim);
    })
    .width(Length::Fixed(12.0))
    .height(Length::Fixed(12.0))
}

/// The sidebar's width unless pulled, and how narrow and wide it may be.
const SIDE_W: f32 = 184.0;
const SIDE_MIN: f32 = 140.0;
const SIDE_MAX: f32 = 400.0;

/// What a favorite being moved in the sidebar carries (its path).
const SIDEBAR_KIND: &str = "application/x-vergence-sidebar";

/// The home folder (or the root, where there is none).
fn home() -> PathBuf {
    files::home().unwrap_or_else(|| PathBuf::from("/"))
}

/// File Viewer's look (and its Properties windows'), from the desktop file's
/// text. Names and paths are not prose: no spelling marks in the fields.
fn files_theme(desktop: &str) -> Theme {
    Theme { spell: false, ..Theme::from_desktop(desktop, false, SECTION) }
}

struct Viewer {
    shared: SharedRef,
    tabs: Vec<Tab>,
    front: u32,
    /// This window's toolkit number, once open; its persistence token for
    /// window memory (keep.rs); when the session said it is ending.
    me: Option<u32>,
    token: String,
    ending: Option<Instant>,
    /// The path being typed (the path row clicked, or Ctrl+L), Explorer's way.
    path_edit: Option<String>,
    /// The sidebar's place in the window (drawn there, for drops on it), how
    /// far its list is scrolled, and the row its menu was opened on.
    side_rect: Rc<Cell<Rect>>,
    side_scroll: f32,
    /// The sidebar's width (`[files] sidebar-width`), the divider's place as
    /// drawn, and a pull on it: where it began (window points) and the width then.
    side_w: f32,
    /// The server's drive lists seen (files::drives_seen): a new one redraws the sidebar.
    drives_seen: u64,
    /// The drives as last seen, to know which went away.
    drives_last: Vec<PathBuf>,
    divider_rect: Rc<Cell<Rect>>,
    side_pull: Option<(f32, f32)>,
    side_target: Option<sidebar::Line>,
    /// Each folder's disk's free space (`df -kP`, read apart), and what was asked.
    free: Arc<Mutex<std::collections::HashMap<PathBuf, Option<u64>>>>,
    free_asked: RefCell<std::collections::HashSet<PathBuf>>,
    desktop: String,
    stamp: Option<SystemTime>,
    waker: Option<Waker>,
    view: ViewKind,
    pane: bool,
    hidden: bool,
    sort: (SortBy, bool),
    /// What the field narrows the folder to.
    narrow: String,
    /// Space's preview, a window of its own (preview milestone 1), while it is open.
    peek: Option<preview::LinkRef>,
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
    /// A click on the item already chosen alone, and when: renamed in place
    /// once the double-click time has passed with no second click.
    rename_after: Option<(PathBuf, Instant)>,
    /// A disk image being mounted (on a worker): its name, and the answer
    /// once it comes (where it is mounted, or why not).
    mounting: Option<(String, Arc<Mutex<Option<Result<PathBuf, String>>>>)>,
    job: Option<Job>,
    /// Pictures' thumbnails for the icon view, made in the background
    /// (vergence-thumbs; the freedesktop cache other desktops share).
    thumbs: vergence_thumbs::Thumbnailer,
    thumb_waker: Arc<std::sync::Mutex<Option<Waker>>>,
    thumbs_made: usize,
    /// Whether the folder shown is a disk or network mount (Eject), by folder.
    ejectable: RefCell<Option<(PathBuf, bool)>>,
    /// Searching below the folder for the narrowing words (search.rs), and
    /// what it has found, copied when it says more came.
    search: Option<search::Search>,
    search_entries: Vec<Entry>,
    search_done: bool,
    /// Where the last right press was (the menu opens there).
    menu_at: (f32, f32),
    /// The applications the open menu's Open With lists (desktop ids), by place.
    open_with: Vec<String>,
    /// Folders to open in windows of their own (woken() has the window).
    pending: Vec<PathBuf>,
    /// The list's columns' widths (files::COLUMNS), and a pull on an edge:
    /// which column, where it began, the width then.
    widths: [f32; 3],
    pulling: Option<(usize, f32, f32)>,
    /// The Trash window (`vergence-files --trash`): the Trash alone, no
    /// sidebar, path or preview pane, a place of its own.
    trash_window: bool,
    /// Folders' sizes counted by a worker (by path: the folder's own time
    /// when counted, the size or None if it was too big to count), what was
    /// asked, and the worker's queue.
    sizes: Sizes,
    sizes_seen: usize,
    size_asked: RefCell<std::collections::HashSet<(PathBuf, Option<SystemTime>)>>,
    size_queue: std::sync::mpsc::Sender<(PathBuf, Option<SystemTime>)>,
}

/// Folders' sizes as counted: by path, the folder's own time then and the
/// size (None: too much to count).
type Sizes = Arc<Mutex<std::collections::HashMap<PathBuf, (Option<SystemTime>, Option<u64>)>>>;

/// The most entries counted for one folder's size (more: not shown).
const COUNT_MOST: u32 = 400_000;

/// Everything in a folder, on its own file system only (as `du -x`), not
/// following links; None past [`COUNT_MOST`] entries.
fn count_tree(p: &Path, dev: u64, budget: &mut u32) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    let Ok(m) = p.symlink_metadata() else { return Some(0) };
    if m.dev() != dev {
        return Some(0);
    }
    if !m.is_dir() {
        return Some(m.len());
    }
    let mut total = 0u64;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            *budget = budget.checked_sub(1)?;
            total += count_tree(&e.path(), dev, budget)?;
        }
    }
    Some(total)
}

/// The worker that counts folders' sizes, one at a time, waking the window
/// with each.
fn size_worker(sizes: Sizes, wake: Arc<Mutex<Option<Waker>>>) -> std::sync::mpsc::Sender<(PathBuf, Option<SystemTime>)> {
    use std::os::unix::fs::MetadataExt;
    let (tx, rx) = std::sync::mpsc::channel::<(PathBuf, Option<SystemTime>)>();
    let _ = std::thread::Builder::new().name("folder-sizes".into()).spawn(move || {
        for (p, when) in rx {
            let Ok(m) = p.symlink_metadata() else { continue };
            let mut budget = COUNT_MOST;
            let n = count_tree(&p, m.dev(), &mut budget);
            sizes.lock().unwrap_or_else(|x| x.into_inner()).insert(p, (when, n));
            if let Some(w) = wake.lock().unwrap_or_else(|x| x.into_inner()).as_ref() {
                w.wake();
            }
        }
    });
    tx
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
    /// The path row clicked: a field to type a path in; its text; Enter.
    PathEdit,
    PathTyped(String),
    PathGo,
    /// A sidebar row (by its place in sidebar::lines): pressed, its menu,
    /// pulled away to be moved; a heading pressed.
    SideGo(usize),
    SideMenu(usize),
    SideDrag(usize),
    /// A disk's eject button in the sidebar (a drive, a disk image).
    SideEject(usize),
    SideFold(sidebar::Section),
    /// The line between the sidebar and the folder pulled (its touch).
    SideGrip(vergence_widgets::Touch),
    Click(usize),
    Sort(SortBy),
    Narrow(String),
    /// Search below the folder for the narrowing words, or stop.
    SearchBelow,
    OpenChosen,
    OpenInCloseUp,
    DismissNotice,
    /// A right click on a row: its menu (the row chosen first, unless it is).
    RowMenu(usize),
    /// An edge between the list's headings pulled: the column and the touch.
    Grip(usize, vergence_widgets::Touch),
    /// A right click on the ground around the rows: the folder's menu.
    GroundMenu,
    /// A Trash row's own Put Back.
    RowPutBack(usize),
    /// A Trash row's own Delete Forever (asked first).
    RowForever(usize),
}

/// desktop.toml's text.
fn desktop_text() -> (String, Option<SystemTime>) {
    match vergence_widgets::prefs::desktop_path() {
        Some(p) => (std::fs::read_to_string(&p).unwrap_or_default(), std::fs::metadata(&p).and_then(|m| m.modified()).ok()),
        None => (String::new(), None),
    }
}


/// A notification card (the session's, over the bus), as File Viewer.
/// Outside a Vergence session it is only logged.
pub(crate) fn notify(summary: &str, body: &str) {
    if std::env::var_os(vergence_protocol::BUS_ENV).is_some()
        && let Ok(mut bus) = vergence_protocol::Client::connect(APP_ID)
        && bus.request(|serial| vergence_protocol::bus::ClientMsg::Notify { serial, replaces: 0, summary: summary.into(), body: body.into(), urgency: 1, timeout_ms: -1, actions: Vec::new() }).is_ok()
    {
        return;
    }
    eprintln!("vergence-files: {summary}: {body}");
}

/// Opens files with the apps chosen for their types: the server's Open in a
/// Vergence session (the dock shows them starting), else started here.
/// Starts a program and lets it run on its own: a thread waits for it, so
/// it is not left behind as a zombie when it ends (the desktop's process
/// lives all session and started a FileView for each folder opened, each
/// left over once it handed its folder on, 2026-11-18).
pub(crate) fn start(cmd: &mut std::process::Command) -> std::io::Result<()> {
    let mut child = cmd.spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

pub(crate) fn open_files(paths: &[PathBuf]) -> Result<(), String> {
    open_files_with(paths, None)
}

/// Opens files with this application (a desktop id), or each with the one
/// chosen for its type.
pub(crate) fn open_files_with(paths: &[PathBuf], app: Option<&str>) -> Result<(), String> {
    let items: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    if std::env::var_os(vergence_protocol::BUS_ENV).is_some()
        && let Ok(mut bus) = vergence_protocol::Client::connect(APP_ID)
    {
        return match bus.request(|serial| vergence_protocol::bus::ClientMsg::Open { serial, items: items.clone(), app: app.unwrap_or_default().to_string() }) {
            Ok(vergence_protocol::bus::ServerMsg::Done { .. }) => Ok(()),
            Err(vergence_protocol::ClientError::Refused(_, m)) | Ok(vergence_protocol::bus::ServerMsg::Error { message: m, .. }) => Err(m),
            other => Err(format!("{other:?}")),
        };
    }
    let parsed: Vec<vergence_shell::open::Item> = items.iter().map(|i| vergence_shell::open::item(i)).collect();
    let (groups, failed) = vergence_shell::open::plan(&parsed, app, &vergence_shell::apps::installed(), &vergence_types::lists());
    for (entry, files) in groups {
        for argv in vergence_shell::open::commands(&entry, &files) {
            if let Some((p, rest)) = argv.split_first() {
                let _ = crate::start(std::process::Command::new(p).args(rest));
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
        let desktop_now = desktop.clone();
        let thumb_waker: Arc<std::sync::Mutex<Option<Waker>>> = Arc::default();
        let sizes: Sizes = Arc::default();
        let size_queue = size_worker(sizes.clone(), thumb_waker.clone());
        let w = thumb_waker.clone();
        let thumbs = vergence_thumbs::Thumbnailer::new(move || {
            if let Some(w) = w.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                w.wake();
            }
        });
        Viewer {
            shared,
            tabs: Vec::new(),
            front: 0,
            me: None,
            token: String::new(),
            ending: None,
            path_edit: None,
            side_rect: Rc::new(Cell::new(Rect::new(0.0, 0.0, 0.0, 0.0))),
            side_scroll: 0.0,
            side_w: vergence_ui::table_value(&desktop_now, SECTION, "sidebar-width").and_then(|v| v.trim().parse::<f32>().ok()).filter(|w| w.is_finite()).map_or(SIDE_W, |w| w.clamp(SIDE_MIN, SIDE_MAX)),
            drives_seen: 0,
            drives_last: Vec::new(),
            divider_rect: Rc::new(Cell::new(Rect::new(0.0, 0.0, 0.0, 0.0))),
            side_pull: None,
            side_target: None,
            free: Arc::default(),
            free_asked: RefCell::new(std::collections::HashSet::new()),
            desktop,
            stamp,
            waker: None,
            view,
            pane,
            hidden,
            sort: (SortBy::Name, true),
            narrow: String::new(),
            peek: None,
            picture: Arc::default(),
            asked: RefCell::new(None),
            notice: None,
            mods: Modifiers::default(),
            last_click: None,
            width: 900.0,
            prefs_open: Rc::new(Cell::new(false)),
            ask: None,
            naming: None,
            rename_after: None,
            mounting: None,
            job: None,
            thumbs,
            thumb_waker,
            thumbs_made: 0,
            ejectable: RefCell::new(None),
            search: None,
            search_entries: Vec::new(),
            search_done: false,
            menu_at: (0.0, 0.0),
            open_with: Vec::new(),
            pending: Vec::new(),
            widths: vergence_ui::table_value(&desktop_now, SECTION, "columns").map_or(files::COLUMN_WIDTHS, |v| files::parse_widths(&v)),
            pulling: None,
            trash_window: false,
            sizes,
            sizes_seen: 0,
            size_asked: RefCell::new(std::collections::HashSet::new()),
            size_queue,
        }
    }

    fn new_tab(&mut self, showing: Showing) {
        let id = {
            let mut s = self.shared.borrow_mut();
            s.next_tab += 1;
            s.next_tab
        };
        self.tabs.push(Tab::new(id, showing, self.sizes.clone()));
        self.front = id;
        self.clear_narrow();
    }

    /// Paths handed over or given at the start: a folder opens, or a file's
    /// folder with the file chosen; the first in this window if it shows
    /// nothing yet, the rest in windows of their own (`pending`, opened by
    /// woken()). None: the home folder.
    /// The Trash window: the Trash, made if it was not there yet.
    fn trash_window(shared: SharedRef) -> Option<Viewer> {
        let trash = ops::trash_dir()?.join("files");
        let _ = std::fs::create_dir_all(&trash);
        let mut v = Viewer::new(shared);
        v.trash_window = true;
        v.pane = false;
        v.new_tab(Showing::Folder(trash));
        Some(v)
    }

    /// Paths handed over or given at the start, each in a tab of this window
    /// (files milestone 3: a folder handed over opens in a tab, not a window
    /// of its own): a folder, or a file's folder with the file chosen. None
    /// given: the home folder here if the window shows nothing yet, else a
    /// window of its own at home (a start with nothing to open).
    fn open_paths(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            if self.tabs.is_empty() {
                self.new_tab(Showing::Folder(home()));
            } else {
                self.pending.push(home());
            }
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

    /// Tells the others how to wake this window (once it has a number and a waker).
    fn register(&mut self) {
        let (Some(me), Some(w)) = (self.me, self.waker.clone()) else { return };
        let mut s = self.shared.borrow_mut();
        s.windows.retain(|(id, _)| *id != me);
        s.windows.push((me, w));
    }

    /// A window kept from the last run: its token and its tabs.
    fn from_snap(shared: SharedRef, snap: &keep::Snap) -> Viewer {
        let mut v = Viewer::new(shared);
        v.token = snap.token.clone();
        let places = vergence_widgets::session::places();
        for t in &snap.tabs {
            v.new_tab(match &t.what {
                keep::Kept::Folder(p) => Showing::Folder(p.clone()),
                keep::Kept::Recent => Showing::Recent,
            });
            // Scrolled where it was (session milestone 1's rule 2).
            if places && let Some(tab) = v.tabs.last_mut() {
                tab.scroll = t.scroll;
                tab.want = Some(t.scroll);
            }
        }
        if let Some(t) = v.tabs.get(snap.front) {
            v.front = t.id;
        }
        v
    }

    /// What this window keeps for the next start, written when it changed.
    fn keep(&mut self) {
        let Some(me) = self.me else { return };
        if self.token.is_empty() || self.trash_window {
            return;
        }
        let tabs: Vec<keep::KeptTab> = self
            .tabs
            .iter()
            .map(|t| keep::KeptTab {
                what: match &t.showing {
                    Showing::Folder(p) => keep::Kept::Folder(p.clone()),
                    Showing::Recent => keep::Kept::Recent,
                },
                scroll: t.scroll.round(),
            })
            .collect();
        let front = self.tabs.iter().position(|t| t.id == self.front).unwrap_or(0);
        let snap = keep::Snap { token: self.token.clone(), tabs, front };
        let mut s = self.shared.borrow_mut();
        match s.snaps.iter_mut().find(|(id, _)| *id == me) {
            Some((_, old)) if *old == snap => return,
            Some((_, old)) => *old = snap,
            None => s.snaps.push((me, snap)),
        }
        let all: Vec<keep::Snap> = s.snaps.iter().map(|(_, w)| w.clone()).collect();
        keep::save(&all);
    }

    /// Takes a tab out of this window (moved to another, or closed).
    fn remove_tab(&mut self, id: u32) -> Option<Tab> {
        let i = self.tabs.iter().position(|t| t.id == id)?;
        let t = self.tabs.remove(i);
        if self.front == id {
            self.front = self.tabs.get(i.min(self.tabs.len().saturating_sub(1))).map_or(0, |t| t.id);
            self.tab_changed();
        }
        Some(t)
    }

    /// Closes a tab; the window with it, if it was the last.
    fn close_tab(&mut self, id: u32, window: &mut Window) {
        self.remove_tab(id);
        if self.tabs.is_empty() {
            window.close();
        }
    }

    /// Another tab in front: the window's own state for a folder (the
    /// narrowing, a search, the preview) starts again, and the folder is read
    /// again if it changed while behind.
    fn tab_changed(&mut self) {
        self.clear_narrow();
        self.close_peek();
        self.path_edit = None;
        if let Some(t) = self.front_mut() {
            // Back at the place this tab was scrolled to.
            t.want = Some(t.scroll);
            let now = t.dir().and_then(|d| std::fs::metadata(d).and_then(|m| m.modified()).ok());
            if t.dir().is_none() || now != t.stamp {
                t.read();
            }
        }
    }

    fn side_setup(&self) -> sidebar::Setup {
        sidebar::setup(&self.desktop, SECTION)
    }

    /// The sidebar's Locations: the computer, its disks, the Trash, the network's places.
    fn locations(&self) -> Vec<Place> {
        let mine: Vec<PathBuf> = files::home().into_iter().chain(files::user_dirs().into_iter().map(|(_, p)| p)).collect();
        self.places().into_iter().filter(|p| p.path.as_ref().is_some_and(|x| !mine.contains(x))).collect()
    }

    fn side_lines(&self) -> Vec<sidebar::Line> {
        sidebar::lines(&self.side_setup(), &self.locations(), &files::recent_files())
    }

    fn set_favorites(&mut self, favs: &[PathBuf]) {
        let items: Vec<String> = favs.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        self.write_key("favorites", Some(&sidebar::list_value(&items)));
    }

    /// A sidebar row pressed: a folder gone to, a file opened, Recent shown.
    fn side_go(&mut self, i: usize) {
        let Some(sidebar::Line::Item(_, item)) = self.side_lines().get(i).cloned() else { return };
        match item.path {
            None => self.go(Showing::Recent),
            Some(p) if item.file => self.open_file(&p),
            Some(p) => self.go(Showing::Folder(p)),
        }
    }

    /// Mounts a disk image on a worker (files milestone 3, step 7; Dave,
    /// 2026-11-21: "in downloads I have an iso, can i mount it in Fileview as
    /// we do the USB drive?"); woken() opens it in a tab when it is mounted.
    fn mount_image(&mut self, p: PathBuf) {
        if self.mounting.is_some() {
            return;
        }
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let answer: Arc<Mutex<Option<Result<PathBuf, String>>>> = Arc::default();
        let (a, waker) = (answer.clone(), self.thumb_waker.clone());
        std::thread::spawn(move || {
            let r = files::mount_image(&p);
            *a.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            if let Some(w) = waker.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                w.wake();
            }
        });
        self.notice = Some(format!("Mounting “{name}”…"));
        self.mounting = Some((name, answer));
    }

    /// Opens a file with the app chosen for its type, and puts it among the recent ones.
    fn open_file(&mut self, p: &Path) {
        if let Some(dir) = p.parent() {
            files::remember(APP_ID, dir, p);
        }
        if let Err(e) = open_files(&[p.to_path_buf()]) {
            self.notice = Some(e);
        }
    }

    /// A sidebar row's menu (Dave's list: Open in New Tab, Remove from
    /// Sidebar, Show in its folder); the Recent heading's: Hide Recent.
    fn side_menu(&self, line: &sidebar::Line) -> Vec<MenuLine> {
        use sidebar::{Line, Section};
        match line {
            Line::Head(Section::Recent) => vec![MenuLine::item(cmd::SIDE_HIDE_RECENT, "Hide Recent", "")],
            Line::Head(_) => Vec::new(),
            Line::Item(sec, item) => {
                let mut v = Vec::new();
                if !item.file {
                    v.push(MenuLine::item(cmd::SIDE_NEW_TAB, "Open in New Tab", ""));
                }
                if item.path.as_ref().is_some_and(|p| p.parent().is_some()) {
                    v.push(MenuLine::item(cmd::SIDE_SHOW, "Show in Its Folder", ""));
                }
                if *sec == Section::Favorites {
                    v.push(MenuLine::Separator);
                    v.push(MenuLine::item(cmd::SIDE_REMOVE, "Remove from Sidebar", ""));
                }
                if *sec == Section::Locations && item.path.as_deref().is_some_and(files::ejectable) {
                    v.push(MenuLine::Separator);
                    v.push(MenuLine::item(cmd::SIDE_EJECT, "Eject", ""));
                }
                v
            }
        }
    }

    /// Where a drop on the sidebar at window point `y` goes among the favorites.
    fn side_drop_index(&self, x: f32, y: f32) -> Option<usize> {
        let r = self.side_rect.get();
        if x < r.x || x >= r.x + r.w || y < r.y || y >= r.y + r.h {
            return None;
        }
        let lines = self.side_lines();
        sidebar::drop_index(&lines, sidebar::line_at(&lines, y - r.y + self.side_scroll))
    }

    /// The sidebar: its sections, each row of a fixed height (drops are
    /// placed by where they are), in a place that remembers where it was drawn.
    fn sidebar_view(&self, t: &Theme) -> Element<Msg> {
        use sidebar::{Line, Section};
        let here = self.front_tab().and_then(|t| t.dir().map(Path::to_path_buf));
        let recent_here = self.front_tab().is_some_and(|t| t.showing == Showing::Recent);
        let folded = self.side_setup().folded;
        // The disks that can be let go of (read once for the whole sidebar).
        let ejectable: Vec<PathBuf> = self.locations().into_iter().filter_map(|p| p.path).filter(|p| files::ejectable(p)).collect();
        let mut rows: Vec<Element<Msg>> = Vec::new();
        for (i, line) in self.side_lines().into_iter().enumerate() {
            match line {
                Line::Head(sec) => {
                    let open = !folded.contains(&sec);
                    let head = button(row(vec![label(sec.name()).size(11.0).dim().weight(600), space().width(Length::FILL), fold_mark(open)]).align(Align::Center).width(Length::FILL))
                        .look(ButtonLook::Plain)
                        .padding(Padding { top: 6.0, bottom: 2.0, left: 10.0, right: 6.0 })
                        .height(Length::Fixed(sidebar::HEAD_H))
                        .width(Length::FILL)
                        .on_press(Msg::SideFold(sec))
                        .on_menu(Msg::SideMenu(i))
                        .name(format!("{} ({})", sec.name(), if open { "open" } else { "folded" }));
                    rows.push(head);
                }
                Line::Item(sec, item) => {
                    let chosen = match &item.path {
                        None => recent_here,
                        Some(p) => !item.file && Some(p) == here.as_ref(),
                    };
                    let icon = match &item.place {
                        Some(p) => files::place_icon(p),
                        None => item.path.clone().and_then(Entry::of).map_or(("mimes", "text"), |e| files::own_icon_name(e.kind())),
                    };
                    let mark = drawing(move |scene, _, r: Rect, _t: &Theme| {
                        let side = r.w.min(r.h);
                        if let Some(img) = files::own_icon(icon.0, icon.1, (side * 2.5).ceil() as u32) {
                            let id = scene.image(img);
                            scene.draw_image(id, Rect::new(r.x, r.y + (r.h - side) / 2.0, side, side), 1.0);
                        }
                    })
                    .width(Length::Fixed(18.0))
                    .height(Length::Fixed(18.0));
                    let mut cells = vec![mark, label(item.name.clone()).size(13.0).elide().width(Length::FILL)];
                    // A drive or a disk image: its eject button at the row's end, as Finder's.
                    if sec == Section::Locations && item.path.as_deref().is_some_and(|p| ejectable.contains(&p.to_path_buf())) {
                        cells.push(
                            button(vergence_widgets::toolbar::icon(Icon::Eject, 13.0, false))
                                .look(ButtonLook::Plain)
                                .padding(Padding::all(0.0))
                                .width(Length::Fixed(20.0))
                                .height(Length::Fixed(20.0))
                                .on_press(Msg::SideEject(i))
                                .name(format!("Eject {}", item.name)),
                        );
                    }
                    let line = row(cells).spacing(8.0).align(Align::Center).width(Length::FILL);
                    let mut b = button(line)
                        .look(if chosen { ButtonLook::Chosen } else { ButtonLook::Plain })
                        .padding(Padding::hv(0.0, 8.0))
                        .height(Length::Fixed(sidebar::ROW_H))
                        .width(Length::FILL)
                        .on_press(Msg::SideGo(i))
                        .on_menu(Msg::SideMenu(i))
                        .name(item.name.clone());
                    if sec == Section::Favorites {
                        b = b.on_drag(Msg::SideDrag(i));
                    }
                    rows.push(b);
                }
            }
        }
        let rect = self.side_rect.clone();
        let place = drawing(move |_, _, r: Rect, _| rect.set(r)).width(Length::FILL).height(Length::FILL);
        // On the folder's own ground, as Explorer's (Dave, 2026-11-21: "the
        // left pane the same color as the main window"), a line between them.
        let _ = t;
        let list = panel(scroll(column(rows).width(Length::FILL)).name("Sidebar")).radius(0.0).padding(sidebar::PAD).width(Length::FILL).height(Length::FILL);
        stack(vec![place, list]).width(Length::Fixed(self.side_w)).height(Length::FILL)
    }

    /// A double click's longest gap: the desktop's (Settings > Mouse), as
    /// the toolkit's widgets use, so a slow double click opens rather than renames.
    fn double_gap(&self) -> std::time::Duration {
        match self.theme().double_click {
            0 => DOUBLE,
            ms => std::time::Duration::from_millis(ms.clamp(100, 2000) as u64),
        }
    }

    /// The visible row whose name is being changed in place, if any.
    fn renaming_row(&self) -> Option<usize> {
        let target = self.naming.as_ref()?.rename.as_ref()?;
        self.visible().iter().position(|e| e.path == *target)
    }

    /// The name of `p` made a field in its row.
    fn start_rename(&mut self, p: PathBuf) {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.naming = Some(Naming { rename: Some(p), text: name, file: false });
    }

    /// The typed path gone to: a folder, or a file's folder with the file
    /// chosen; "~" is home. Not there: said, and the field stays to mend it.
    fn path_go(&mut self) {
        let Some(typed) = self.path_edit.clone() else { return };
        let typed = typed.trim();
        let p = match typed.strip_prefix('~') {
            Some(rest) => home().join(rest.trim_start_matches('/')),
            None => PathBuf::from(typed),
        };
        if !p.is_absolute() || !p.exists() {
            self.notice = Some(format!("There is no folder or file at {typed}."));
            return;
        }
        self.path_edit = None;
        if p.is_dir() {
            self.go(Showing::Folder(p));
        } else if let Some(dir) = p.parent() {
            self.go(Showing::Folder(dir.to_path_buf()));
            self.choose_only(p.clone());
        }
    }

    /// The free space of the disk the front folder is on, once read (a
    /// worker runs `df -kP` for each folder the first time it is shown).
    fn free_here(&self) -> Option<u64> {
        let dir = self.here()?;
        if let Some(n) = self.free.lock().unwrap_or_else(|e| e.into_inner()).get(&dir) {
            return *n;
        }
        if self.free_asked.borrow_mut().insert(dir.clone()) {
            let (free, waker) = (self.free.clone(), self.thumb_waker.clone());
            std::thread::spawn(move || {
                let n = df_free(&dir);
                free.lock().unwrap_or_else(|e| e.into_inner()).insert(dir, n);
                if let Some(w) = waker.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                    w.wake();
                }
            });
        }
        None
    }

    /// A tab from another window: this window's folder sizes from now on.
    fn take_tab(&mut self, mut t: Tab, at: usize) {
        t.sizes = self.sizes.clone();
        self.front = t.id;
        self.tabs.insert(at.min(self.tabs.len()), t);
        self.tab_changed();
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
        let source = if self.search.is_some() { &self.search_entries } else { &t.entries };
        // What is in the Trash is all shown: a hidden file thrown away is still there.
        let all = self.hidden || self.in_trash();
        let mut v: Vec<&Entry> = source.iter().filter(|e| all || !e.name.starts_with('.')).filter(|e| n.is_empty() || e.name.to_lowercase().contains(&n)).collect();
        let group = if self.shown_view() == ViewKind::List { self.list_look().group } else { GroupBy::None };
        files::sort(&mut v, self.sort.0, self.sort.1, t.showing == Showing::Recent && self.search.is_none(), group);
        v.truncate(files::MAX_ROWS);
        v
    }

    fn cursor_entry(&self) -> Option<Entry> {
        let t = self.front_tab()?;
        let c = t.cursor.as_ref()?;
        t.entries.iter().find(|e| &e.path == c).cloned()
    }

    /// The narrowing words gone, and any search below with them.
    fn clear_narrow(&mut self) {
        self.narrow.clear();
        self.search = None;
        self.search_entries.clear();
    }

    /// Searches below the folder shown for the narrowing words (again, when
    /// they change); off when they are emptied.
    fn search_below(&mut self) {
        self.search = None;
        self.search_entries.clear();
        self.search_done = false;
        let (Some(here), false) = (self.here(), self.narrow.trim().is_empty()) else { return };
        let w = self.thumb_waker.clone();
        self.search = Some(search::Search::start(&here, self.narrow.trim(), self.hidden, move || {
            if let Some(w) = w.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                w.wake();
            }
        }));
    }

    fn go(&mut self, to: Showing) {
        self.clear_narrow();
        if let Some(t) = self.front_mut() {
            t.go(to);
        }
    }

    fn back(&mut self) {
        if let Some(t) = self.front_mut()
            && let Some(s) = t.back.pop()
        {
            t.leave();
            t.forward.push(std::mem::replace(&mut t.showing, s));
            t.arrive();
            t.read();
        }
        self.clear_narrow();
    }

    fn forward(&mut self) {
        if let Some(t) = self.front_mut()
            && let Some(s) = t.forward.pop()
        {
            t.leave();
            t.back.push(std::mem::replace(&mut t.showing, s));
            t.arrive();
            t.read();
        }
        self.clear_narrow();
    }

    fn up(&mut self) {
        if self.in_trash() {
            return;
        }
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
            // The Trash window shows only the Trash: a folder in it opens in FileView.
            if self.trash_window {
                let _ = crate::start(std::process::Command::new("vergence-files").arg(&p));
                return;
            }
            self.close_peek();
            return self.go(Showing::Folder(p));
        }
        // A disk image opened is mounted, as a drive plugged in (step 7).
        if let [one] = chosen.as_slice()
            && files::is_disk_image(one)
            && !self.trash_window
        {
            let p = one.path.clone();
            self.mount_image(p);
            return;
        }
        let paths: Vec<PathBuf> = chosen.iter().filter(|e| !e.dir).map(|e| e.path.clone()).collect();
        // Among the recent files (the sidebar's Recent), as the other apps put theirs.
        for p in &paths {
            if let Some(dir) = p.parent() {
                files::remember(APP_ID, dir, p);
            }
        }
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
        let gap = self.double_gap();
        let double = self.last_click.as_ref().is_some_and(|(t, q)| *q == p && now.duration_since(*t) < gap);
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
                self.rename_after = None;
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

    /// Asks the worker for the cursor's picture (if it is one, or a PDF's
    /// first page, and not asked already).
    fn want_picture(&self) {
        let Some(e) = self.cursor_entry() else { return };
        if e.dir || !vergence_thumbs::has_thumbnail(e.mime) || self.asked.borrow().as_ref() == Some(&e.path) {
            return;
        }
        let pdf = e.mime == Some("application/pdf");
        *self.asked.borrow_mut() = Some(e.path.clone());
        let (out, waker, path) = (self.picture.clone(), self.waker.clone(), e.path.clone());
        std::thread::spawn(move || {
            let img = if pdf {
                vergence_thumbs::pdf_page(&path, 1200).map(Arc::new)
            } else {
                vergence_decode::decode_file(&path, Some((1600, 1200))).ok().and_then(|p| {
                    let f = p.frames.into_iter().next()?;
                    Some(Arc::new(Image { width: p.width, height: p.height, rgba: f.rgba }))
                })
            };
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
        // Seen at once, not at the next look at the file.
        self.desktop = new;
    }

    /// How the list looks, from the settings (and the widths being pulled).
    fn list_look(&self) -> ListLook {
        let key = |k: &str| vergence_ui::table_value(&self.desktop, SECTION, k);
        ListLook {
            rows: key("row-size").map(|v| RowSize::parse(&v)).unwrap_or_default(),
            widths: self.widths,
            kind: true,
            stripes: true,
            // By kind unless said (files milestone 3); the older switch's "false" kept.
            group: match (key("group-by"), key("group-by-kind").as_deref()) {
                (Some(g), _) => GroupBy::parse(&g),
                (None, Some("false")) => GroupBy::None,
                _ => GroupBy::Kind,
            },
        }
    }

    /// A folder's size if counted (asked for if not, unless it is where
    /// counting does not belong: a network mount, the Trash, the system's
    /// own trees).
    fn folder_size(&self, e: &Entry) -> Option<u64> {
        if let Some((when, n)) = self.sizes.lock().unwrap_or_else(|x| x.into_inner()).get(&e.path)
            && *when == e.changed
        {
            return *n;
        }
        // Asked once per folder and time (the mount table read only then).
        if !self.size_asked.borrow_mut().insert((e.path.clone(), e.changed)) {
            return None;
        }
        let skip = self.in_trash() || ["/proc", "/sys", "/dev", "/run"].iter().any(|r| e.path.starts_with(r)) || files::network_mounts().iter().any(|m| e.path.starts_with(m));
        if !skip {
            let _ = self.size_queue.send((e.path.clone(), e.changed));
        }
        None
    }

    fn open_settings(&mut self, window: &mut Window) {
        if self.prefs_open.get() {
            return;
        }
        let waker = self.waker.clone();
        let prefs = Prefs::new("FileView", APP_ID, SECTION, false, Box::new(FilesRows), self.prefs_open.clone()).on_write(move || {
            if let Some(w) = &waker {
                w.wake();
            }
        });
        window.open_window(vergence_widgets::window(prefs));
    }

    fn run(&mut self, id: u32, window: &mut Window) {
        match id {
            // New windows and tabs open at home (files milestone 3, item 7).
            cmd::NEW_WINDOW => {
                let mut w = Viewer::new(self.shared.clone());
                w.new_tab(Showing::Folder(home()));
                window.open_window(vergence_widgets::window(w));
            }
            cmd::NEW_TAB if !self.trash_window => self.new_tab(Showing::Folder(home())),
            cmd::CLOSE => window.close(),
            cmd::CLOSE_TAB => {
                if self.trash_window {
                    window.close();
                } else if let Some(id) = self.front_tab().map(|t| t.id) {
                    self.close_tab(id, window);
                }
            }
            cmd::NEXT_TAB | cmd::PREV_TAB => {
                let n = self.tabs.len() as isize;
                if n > 1 {
                    let i = self.tabs.iter().position(|t| t.id == self.front).unwrap_or(0) as isize;
                    self.front = self.tabs[(i + if id == cmd::NEXT_TAB { 1 } else { -1 }).rem_euclid(n) as usize].id;
                    self.tab_changed();
                }
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
            cmd::PREVIEW => self.toggle_peek(window),
            cmd::OPEN => self.open_chosen(),
            cmd::SELECT_ALL => {
                let all: Vec<PathBuf> = self.visible().iter().map(|e| e.path.clone()).collect();
                if let Some(t) = self.front_mut() {
                    t.chosen = all;
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
            // F2 (Explorer's key) and the menu's Rename: the name a field in its row.
            cmd::RENAME => {
                if let [one] = self.chosen_paths().as_slice() {
                    self.start_rename(one.clone());
                }
            }
            cmd::NEW_FOLDER => {
                if self.here().is_some() {
                    self.naming = Some(Naming { rename: None, text: "New folder".into(), file: false });
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
                    // Only what went wrong is said: what was done shows itself
                    // (Dave, 2026-11-18: "I know I deleted a file").
                    if let Err(why) = ops::undo(&d) {
                        self.notice = Some(format!("It could not be undone: {why}"));
                    }
                    self.read_all();
                }
            }
            cmd::RESTORE => {
                self.notice = None;
                let names: Vec<(String, String)> = self.chosen_paths().iter().filter_map(|p| p.file_name().map(|n| (n.to_string_lossy().into_owned(), self.shown_name(p)))).collect();
                // Only what went wrong is said (Dave, 2026-11-18).
                for (n, shown) in &names {
                    if let Err(e) = ops::restore(n) {
                        self.notice = Some(format!("“{shown}” could not be put back: {e}"));
                    }
                }
                self.read_all();
            }
            cmd::EMPTY_TRASH => self.ask = Some(Ask::EmptyTrash),
            cmd::EJECT => {
                if let Some(here) = self.here().filter(|_| self.here_ejectable()) {
                    let name = here.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let image = files::is_image_mount(&here);
                    for t in self.tabs.iter_mut().filter(|t| t.dir().is_some_and(|d| d.starts_with(&here))) {
                        t.go(Showing::Folder(home()));
                    }
                    match files::eject(&here) {
                        Ok(told) => {
                            if !image && !told {
                                notify(&format!("Safe to remove “{name}”"), "The drive can be pulled out now.");
                            }
                            *self.ejectable.borrow_mut() = None;
                            if let Some(h) = files::home() {
                                self.go(Showing::Folder(h));
                            }
                        }
                        Err(why) => self.notice = Some(format!("“{name}” could not be ejected: {why}")),
                    }
                }
            }
            cmd::NEW_FILE => {
                if self.here().is_some() {
                    self.naming = Some(Naming { rename: None, text: "New file.txt".into(), file: true });
                }
            }
            cmd::MAKE_LINK => self.make_link(),
            cmd::COMPRESS => self.compress(),
            cmd::INFO => {
                // A Properties window for each chosen (a few at most), or the folder's.
                let paths = match self.chosen_paths() {
                    none if none.is_empty() => self.here().into_iter().collect(),
                    some => some,
                };
                for p in paths.into_iter().take(8) {
                    window.open_window(vergence_widgets::window(props::Properties::new(p, self.shared.clone(), self.theme())));
                }
            }
            cmd::COPY_PATH => {
                let paths = self.chosen_paths();
                if !paths.is_empty() {
                    window.copy(&paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n"));
                }
            }
            cmd::TERMINAL => {
                // A folder opens there; a file, in its folder; nothing chosen, here.
                let at = match self.chosen_paths().as_slice() {
                    [one] if one.is_dir() => Some(one.clone()),
                    [one] => one.parent().map(Path::to_path_buf),
                    _ => self.here(),
                };
                if let Some(d) = at
                    && let Err(e) = crate::start(std::process::Command::new("vergence-term").current_dir(&d))
                {
                    self.notice = Some(format!("The Terminal could not be opened: {}", files::plain(&e)));
                }
            }
            cmd::SHOW_IN_FOLDER => {
                if let [one] = self.chosen_paths().as_slice()
                    && let Some(dir) = one.parent()
                {
                    let one = one.clone();
                    self.search = None;
                    self.search_entries.clear();
                    self.clear_narrow();
                    self.go(Showing::Folder(dir.to_path_buf()));
                    self.choose_only(one);
                }
            }
            cmd::SORT_NAME => self.sort = (SortBy::Name, true),
            cmd::SORT_SIZE => self.sort = (SortBy::Size, false),
            cmd::SORT_CHANGED => self.sort = (SortBy::Changed, false),
            cmd::SORT_KIND => self.sort = (SortBy::Kind, true),
            cmd::ROWS_SMALL | cmd::ROWS_MEDIUM | cmd::ROWS_LARGE => {
                let size = match id {
                    cmd::ROWS_SMALL => RowSize::Small,
                    cmd::ROWS_LARGE => RowSize::Large,
                    _ => RowSize::Medium,
                };
                // Medium is the default: no key.
                let words = format!("\"{}\"", size.word());
                self.write_key("row-size", (size != RowSize::Medium).then_some(words.as_str()));
            }
            cmd::GROUP_KIND | cmd::GROUP_DATE | cmd::GROUP_SIZE | cmd::GROUP_NONE => {
                let g = match id {
                    cmd::GROUP_DATE => GroupBy::Date,
                    cmd::GROUP_SIZE => GroupBy::Size,
                    cmd::GROUP_NONE => GroupBy::None,
                    _ => GroupBy::Kind,
                };
                // Kind is the default: no key; the older switch taken away.
                let words = format!("\"{}\"", g.word());
                self.write_key("group-by-kind", None);
                self.write_key("group-by", (g != GroupBy::Kind).then_some(words.as_str()));
            }
            id if (cmd::OPEN_WITH..cmd::OPEN_WITH + cmd::MAX_OPEN_WITH as u32).contains(&id) => {
                if let Some(app) = self.open_with.get((id - cmd::OPEN_WITH) as usize).cloned()
                    && let Err(e) = open_files_with(&self.chosen_paths(), Some(&app))
                {
                    self.notice = Some(format!("It could not be opened: {e}"));
                }
            }
            cmd::SIDE_NEW_TAB | cmd::SIDE_SHOW | cmd::SIDE_REMOVE => {
                if let Some(sidebar::Line::Item(_, item)) = self.side_target.take() {
                    match (id, item.path) {
                        (cmd::SIDE_NEW_TAB, None) => self.new_tab(Showing::Recent),
                        (cmd::SIDE_NEW_TAB, Some(p)) => self.new_tab(Showing::Folder(p)),
                        (cmd::SIDE_SHOW, Some(p)) => {
                            if let Some(dir) = p.parent() {
                                self.go(Showing::Folder(dir.to_path_buf()));
                                self.choose_only(p.clone());
                            }
                        }
                        (cmd::SIDE_REMOVE, Some(p)) => {
                            let favs: Vec<PathBuf> = sidebar::favorites(&self.side_setup()).into_iter().filter(|f| *f != p).collect();
                            self.set_favorites(&favs);
                        }
                        _ => {}
                    }
                }
            }
            cmd::SIDE_HIDE_RECENT => self.write_key("recent", Some("false")),
            cmd::MOUNT_IMAGE => {
                if let [one] = self.chosen_paths().as_slice() {
                    self.mount_image(one.clone());
                }
            }
            cmd::SIDE_EJECT => {
                if let Some(sidebar::Line::Item(_, item)) = self.side_target.take()
                    && let Some(p) = item.path
                {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let image = files::is_image_mount(&p);
                    // Tabs on it go home first: on FreeBSD a look into the folder
                    // after the eject would have the automounter mount it again.
                    for t in self.tabs.iter_mut().filter(|t| t.dir().is_some_and(|d| d.starts_with(&p))) {
                        t.go(Showing::Folder(home()));
                    }
                    match files::eject(&p) {
                        Ok(told) => {
                            // A drive: safe to pull out (the server says so itself when it
                            // ejected); a disk image: put away, nothing to say.
                            if !image && !told {
                                notify(&format!("Safe to remove “{name}”"), "The drive can be pulled out now.");
                            }
                            *self.ejectable.borrow_mut() = None;
                            // Tabs showing it go home.
                            for t in self.tabs.iter_mut().filter(|t| t.dir().is_some_and(|d| d.starts_with(&p))) {
                                t.go(Showing::Folder(home()));
                            }
                        }
                        Err(why) => self.notice = Some(format!("“{name}” could not be ejected: {why}")),
                    }
                }
            }
            cmd::SETTINGS => self.open_settings(window),
            cmd::ABOUT => {
                let about = vergence_widgets::about::About::new(APP_ID, "FileView", env!("CARGO_PKG_VERSION"), "Vergence's file manager: your folders as a list or icons, Preview on the space bar, files opened with the apps chosen for them.", self.theme());
                window.open_window(vergence_widgets::window(about));
            }
            _ => {}
        }
        self.want_picture();
        self.keep();
        window.redraw();
    }

    /// A chosen item's menu (Dave: "a blend of macOS and Windows with Linux
    /// flavoring"): every line always there, faint when it cannot be done.
    fn item_menu(&mut self) -> Vec<MenuLine> {
        let chosen = self.chosen_paths();
        let one = chosen.len() == 1;
        let any = !chosen.is_empty();
        let trash = self.in_trash();
        let results = self.search.is_some() || self.front_tab().is_some_and(|t| t.showing == Showing::Recent);
        let mut lines = vec![MenuLine::item(cmd::OPEN, "Open", "Enter")];
        // A disk image: mounted, as a drive plugged in (step 7).
        let image = one && !trash && self.visible().iter().any(|e| Some(&e.path) == chosen.first() && files::is_disk_image(e));
        if image {
            lines.push(MenuLine::item(cmd::MOUNT_IMAGE, "Mount Disk Image", ""));
        }
        // Open With: the applications for the files' type, when they share one.
        self.open_with.clear();
        let files: Vec<&PathBuf> = chosen.iter().filter(|p| !p.is_dir()).collect();
        let mimes: Vec<String> = files.iter().map(|p| vergence_types::of_file(p).to_string()).collect();
        if !files.is_empty() && files.len() == chosen.len() && mimes.iter().all(|m| *m == mimes[0]) {
            let entries = vergence_shell::apps::installed();
            let ids = vergence_shell::open::apps_for(&mimes[0], &entries, &vergence_types::lists());
            let mut subs = Vec::new();
            for id in ids.into_iter().take(cmd::MAX_OPEN_WITH) {
                let name = entries.iter().find(|e| vergence_shell::open::desktop_id(&e.id) == id).map(|e| e.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| id.trim_end_matches(".desktop").to_string());
                subs.push(MenuLine::item(cmd::OPEN_WITH + self.open_with.len() as u32, &name.chars().take(60).collect::<String>(), ""));
                self.open_with.push(id);
            }
            if subs.is_empty() {
                subs.push(MenuLine::item(cmd::OPEN_WITH + cmd::MAX_OPEN_WITH as u32, "No applications for this type", "").enabled(false));
            }
            lines.push(MenuLine::Sub { label: "Open With".into(), lines: subs });
        }
        if results {
            lines.push(MenuLine::item(cmd::SHOW_IN_FOLDER, "Show in Folder", "").enabled(one));
        }
        lines.push(MenuLine::Separator);
        lines.push(MenuLine::item(cmd::CUT, "Cut", "Ctrl+X").enabled(any));
        lines.push(MenuLine::item(cmd::COPY, "Copy", "Ctrl+C").enabled(any));
        lines.push(MenuLine::item(cmd::PASTE, "Paste", "Ctrl+V").enabled(self.here().is_some()));
        lines.push(MenuLine::item(cmd::DUPLICATE, "Duplicate", "Ctrl+D").enabled(any && !trash));
        lines.push(MenuLine::item(cmd::MAKE_LINK, "Make Link", "").enabled(one && !trash));
        lines.push(MenuLine::Separator);
        lines.push(MenuLine::item(cmd::RENAME, "Rename…", "F2").enabled(one && !trash));
        if trash {
            lines.push(MenuLine::item(cmd::RESTORE, "Put Back", "").enabled(any));
            lines.push(MenuLine::item(cmd::DELETE, "Delete Forever…", "Delete").enabled(any));
        } else {
            lines.push(MenuLine::item(cmd::TRASH, "Move to the Trash", "Delete").enabled(any));
        }
        lines.push(MenuLine::Separator);
        lines.push(MenuLine::item(cmd::COMPRESS, "Compress", "").enabled(any && !trash));
        lines.push(MenuLine::item(cmd::INFO, "Properties", "Ctrl+I").enabled(any));
        // Backup milestone 1, step 4: versions from snapshots and the backup copy.
        lines.push(MenuLine::item(cmd::VERSIONS, "Earlier Versions…", "").enabled(false));
        lines.push(MenuLine::Separator);
        lines.push(MenuLine::item(cmd::COPY_PATH, "Copy Path", "").enabled(any));
        lines.push(MenuLine::item(cmd::TERMINAL, "Open in Terminal", "").enabled(one));
        lines
    }

    /// The menu of the ground around the rows: the folder's own.
    fn ground_menu(&mut self) -> Vec<MenuLine> {
        let here = self.here().is_some();
        let trash = self.in_trash();
        let (by, _) = self.sort;
        let mut lines = vec![
            MenuLine::item(cmd::NEW_FOLDER, "New Folder", "Ctrl+Shift+N").enabled(here && !trash),
            MenuLine::item(cmd::NEW_FILE, "New File", "").enabled(here && !trash),
            MenuLine::item(cmd::PASTE, "Paste", "Ctrl+V").enabled(here && !trash),
            MenuLine::Separator,
            MenuLine::Sub {
                label: "Sort By".into(),
                lines: vec![
                    MenuLine::item(cmd::SORT_NAME, "Name", "").checked(by == SortBy::Name),
                    MenuLine::item(cmd::SORT_SIZE, "Size", "").checked(by == SortBy::Size),
                    MenuLine::item(cmd::SORT_CHANGED, "Date Changed", "").checked(by == SortBy::Changed),
                    MenuLine::item(cmd::SORT_KIND, "Kind", "").checked(by == SortBy::Kind),
                ],
            },
            MenuLine::Sub {
                label: "Group By".into(),
                lines: vec![
                    MenuLine::item(cmd::GROUP_KIND, "Kind", "").checked(self.list_look().group == GroupBy::Kind),
                    MenuLine::item(cmd::GROUP_DATE, "Date", "").checked(self.list_look().group == GroupBy::Date),
                    MenuLine::item(cmd::GROUP_SIZE, "Size", "").checked(self.list_look().group == GroupBy::Size),
                    MenuLine::item(cmd::GROUP_NONE, "None", "").checked(self.list_look().group == GroupBy::None),
                ],
            },
            MenuLine::item(cmd::HIDDEN, "Show Hidden Files", "Ctrl+H").checked(self.hidden),
        ];
        if trash {
            lines.push(MenuLine::Separator);
            lines.push(MenuLine::item(cmd::EMPTY_TRASH, "Empty Trash…", ""));
        }
        lines.push(MenuLine::Separator);
        lines.push(MenuLine::item(cmd::TERMINAL, "Open in Terminal", "").enabled(here));
        lines.push(MenuLine::item(cmd::INFO, "Properties", "Ctrl+I").enabled(here));
        lines
    }

    /// A link to the one chosen item beside it ("name link").
    fn make_link(&mut self) {
        let chosen = self.chosen_paths();
        let [one] = chosen.as_slice() else { return };
        let (Some(dir), Some(name)) = (one.parent(), one.file_name()) else { return };
        let to = dir.join(ops::free_name(&format!("{} link", name.to_string_lossy()), |x| dir.join(x).symlink_metadata().is_ok()));
        match std::os::unix::fs::symlink(one, &to) {
            Ok(()) => {
                self.shared.borrow_mut().undo.push(ops::Done::Made(to.clone()));
                self.read_all();
                self.choose_only(to);
            }
            Err(e) => self.notice = Some(format!("The link could not be made: {}", files::plain(&e))),
        }
    }

    /// The chosen items into one archive beside them: "name.tar.gz" for one,
    /// "Archive.tar.gz" for several, by the system's tar (principle 8), on a
    /// worker; the folder's watch shows it when it is there.
    fn compress(&mut self) {
        let chosen = self.chosen_paths();
        let Some(dir) = chosen.first().and_then(|p| p.parent()).map(Path::to_path_buf) else { return };
        if chosen.iter().any(|p| p.parent() != Some(dir.as_path())) {
            self.notice = Some("Items from different folders cannot be compressed together.".into());
            return;
        }
        let base = match chosen.as_slice() {
            [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into()),
            _ => "Archive".into(),
        };
        let name = ops::free_name(&format!("{base}.tar.gz"), |x| dir.join(x).symlink_metadata().is_ok());
        let to = dir.join(&name);
        let names: Vec<std::ffi::OsString> = chosen.iter().filter_map(|p| p.file_name().map(|n| n.to_os_string())).collect();
        std::thread::spawn(move || {
            // "--" so a name starting with "-" is a name, not an option.
            let out = std::process::Command::new("tar").arg("-czf").arg(&to).arg("-C").arg(&dir).arg("--").args(&names).output();
            match out {
                Ok(o) if o.status.success() => {}
                Ok(o) => {
                    let _ = std::fs::remove_file(&to);
                    notify(&format!("“{name}” could not be made"), String::from_utf8_lossy(&o.stderr).trim());
                }
                Err(e) => notify(&format!("“{name}” could not be made"), &format!("tar: {e}")),
            }
        });
    }

    /// This item alone chosen, the arrows starting from it.
    fn choose_only(&mut self, p: PathBuf) {
        if let Some(t) = self.front_mut() {
            t.chosen = vec![p.clone()];
            t.cursor = Some(p);
            t.turn += 1;
        }
    }

    /// The folder shown, if it is one (not Recent).
    fn here(&self) -> Option<PathBuf> {
        self.front_tab().and_then(|t| t.dir().map(Path::to_path_buf))
    }

    /// Whether the folder shown can be ejected (asked of the mount table
    /// once per folder).
    fn here_ejectable(&self) -> bool {
        let Some(here) = self.here() else { return false };
        if let Some((p, e)) = self.ejectable.borrow().as_ref()
            && *p == here
        {
            return *e;
        }
        let e = files::ejectable(&here);
        *self.ejectable.borrow_mut() = Some((here, e));
        e
    }

    fn chosen_paths(&self) -> Vec<PathBuf> {
        self.front_tab().map(|t| t.chosen.clone()).unwrap_or_default()
    }

    /// A Trash row's own button: that row chosen, then put back, or asked
    /// about deleting it forever.
    fn row_tool(&mut self, k: usize, put_back: bool, window: &mut Window) {
        let Some(p) = self.visible().get(k).map(|e| e.path.clone()) else { return };
        if let Some(t) = self.front_mut() {
            t.chosen = vec![p.clone()];
            t.cursor = Some(p.clone());
        }
        if put_back {
            self.run(cmd::RESTORE, window);
        } else {
            self.notice = None;
            self.ask = Some(Ask::Forever(vec![p]));
        }
    }

    /// The view as shown: the Trash is always a list (Dave, 2026-11-18),
    /// whatever the window's view is for folders.
    fn shown_view(&self) -> ViewKind {
        if self.in_trash() { ViewKind::List } else { self.view }
    }

    /// The name an item is shown by: in the Trash, the name it had.
    fn shown_name(&self, p: &Path) -> String {
        self.front_tab().and_then(|t| t.entries.iter().find(|e| e.path == p)).map(|e| e.name.clone()).unwrap_or_else(|| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
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
            // No notice of it (Dave, 2026-11-18): it shows in the Trash, and Ctrl+Z puts it back.
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
                if n.file {
                    std::fs::OpenOptions::new().write(true).create_new(true).open(&to).map(|_| (ops::Done::Made(to.clone()), to))
                } else {
                    std::fs::create_dir(&to).map(|_| (ops::Done::NewFolder(to.clone()), to))
                }
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
                let what = if paths.len() == 1 { format!("Delete “{}” for good? It cannot be undone.", self.shown_name(&paths[0])) } else { format!("Delete {} items for good? They cannot be undone.", paths.len()) };
                out.push(strip(vec![label(what).size(12.0).weight(600).wrap().width(Length::FILL), b("Cancel", Msg::Confirm(false), false), b("Delete", Msg::Confirm(true), true)]));
            }
            Some(Ask::EmptyTrash) => {
                out.push(strip(vec![label("Empty the Trash? What is in it cannot be got back.").size(12.0).weight(600).wrap().width(Length::FILL), b("Cancel", Msg::Confirm(false), false), b("Empty Trash", Msg::Confirm(true), true)]));
            }
            None => {}
        }
        if let Some(n) = self.naming.as_ref().filter(|n| n.rename.is_none() || self.renaming_row().is_none()) {
            out.push(strip(vec![
                label(if n.rename.is_some() { "New name" } else if n.file { "New file" } else { "New folder" }).size(12.0),
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
        // The Trash is a place of its own: nothing above it to go up to.
        let can_up = !self.in_trash() && tab.and_then(|t| t.dir()).is_some_and(|d| d.parent().is_some());
        let mut items = vec![group(vec![
            tool(Icon::Back, "Back", can_back.then_some(Msg::Run(cmd::BACK))),
            tool(Icon::Forward, "Forward", can_forward.then_some(Msg::Run(cmd::FORWARD))),
            tool(Icon::Up, "Up", can_up.then_some(Msg::Run(cmd::UP))),
        ])];
        // The Trash window goes nowhere: its name is in its title bar.
        if self.trash_window {
            items.clear();
        }
        // The path (Dave, 2026-11-21): the folder's real path in a box drawn
        // as the field is ("the folder name is right above it in the tab so
        // here the real path makes more sense"); a click on it, or Ctrl+L,
        // makes it the field to type a path in (Enter goes there, Escape lets
        // go). Not a field all the time: the folder's keys (Backspace goes
        // up, Delete trashes) must not act while a path is being typed.
        if !self.trash_window {
            match &self.path_edit {
                // A key of its own: the box stood in its place, and a field
                // taken for the same widget would not take the keyboard.
                Some(text) => items.push(field(text.clone(), Msg::PathTyped).on_submit(Msg::PathGo).autofocus().key(0x7061_7468).width(Length::FILL).name("Path")),
                None => {
                    let shown = match tab.map(|t| &t.showing) {
                        Some(Showing::Folder(dir)) => dir.display().to_string(),
                        _ => "Recent".to_string(),
                    };
                    let edge = if t.high_contrast { t.text } else { t.line };
                    items.push(
                        button(panel(label(shown).elide().width(Length::FILL)).color(t.page).edge(edge, 1.0).padding(Padding::hv(0.0, 8.0)).align(Align::Center).width(Length::FILL).height(Length::Fixed(26.0)))
                            .look(ButtonLook::Plain)
                            .padding(Padding::all(0.0))
                            .width(Length::FILL)
                            .on_press(Msg::PathEdit)
                            .name("Path"),
                    );
                }
            }
        } else {
            items.push(flexible());
        }
        let any = tab.is_some_and(|t| !t.chosen.is_empty());
        let trash = self.in_trash();
        // The views and the panes, then what is done here, then the filter
        // at the right (files milestone 3, step 2: Explorer's order).
        if !trash {
            items.push(group(vec![
                toggle(Icon::List, "As a list", self.view == ViewKind::List, Msg::Run(cmd::LIST)),
                toggle(Icon::Grid, "As icons", self.view == ViewKind::Icons, Msg::Run(cmd::ICONS)),
            ]));
        }
        if !self.trash_window {
            let mut panes = vec![toggle(Icon::Pane, "Preview pane", self.pane, Msg::Run(cmd::PANE))];
            if !trash {
                panes.push(toggle(Icon::Hidden, "Hidden files", self.hidden, Msg::Run(cmd::HIDDEN)));
            }
            items.push(group(panes));
        }
        if trash {
            // The chosen ones put back or deleted; the whole Trash emptied
            // (Dave, 2026-11-18: right click and each row have them too).
            let full = tab.is_some_and(|t| !t.entries.is_empty());
            items.push(button(label("Put Back").size(12.0)).padding(Padding::hv(3.0, 10.0)).on_press_if(any, Msg::Run(cmd::RESTORE)).name("Put Back"));
            items.push(button(label("Delete Forever…").size(12.0)).padding(Padding::hv(3.0, 10.0)).on_press_if(any, Msg::Run(cmd::DELETE)).name("Delete Forever"));
            items.push(button(label("Empty Trash…").size(12.0)).padding(Padding::hv(3.0, 10.0)).on_press_if(full, Msg::Run(cmd::EMPTY_TRASH)).name("Empty Trash"));
        } else {
            items.push(group(vec![
                tool(Icon::NewFolder, "New folder", self.here().is_some().then_some(Msg::Run(cmd::NEW_FOLDER))),
                tool(Icon::Trash, "Move to the Trash", any.then_some(Msg::Run(cmd::TRASH))),
            ]));
        }
        // With words typed: search the folders below too.
        if !self.narrow.trim().is_empty() && self.here().is_some() {
            let on = self.search.is_some();
            items.push(button(label("In subfolders").size(11.5)).look(if on { ButtonLook::Chosen } else { ButtonLook::Plain }).padding(Padding::hv(4.0, 8.0)).on_press(Msg::SearchBelow).name("Search in subfolders"));
        }
        items.push(field(self.narrow.clone(), Msg::Narrow).placeholder(if trash { "Narrow the Trash" } else { "Narrow this folder" }).width(Length::Fixed(200.0)).name("Narrow"));
        toolbar(items, t)
    }

    fn places(&self) -> Vec<Place> {
        let mut p = files::places();
        if let Some(trash) = ops::trash_dir().map(|d| d.join("files")) {
            // After the computer and its disks, before the network's places.
            let at = p.iter().position(|x| x.network).unwrap_or(p.len());
            p.insert(at, Place { name: "Trash".into(), path: Some(trash), network: false });
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
        let menu = |k: usize| Msg::RowMenu(k);
        let look = self.list_look();
        let folder_size = |e: &Entry| self.folder_size(e);
        let grip: fn(usize, vergence_widgets::Touch) -> Option<Msg> = |c, t| Some(Msg::Grip(c, t));
        // The icon view's thumbnails: this folder's pictures asked for, in
        // their order; what other folders asked and has not begun is dropped.
        if self.shown_view() == ViewKind::Icons {
            self.thumbs.forget_waiting();
            for e in visible.iter().filter(|e| !e.dir && vergence_thumbs::has_thumbnail(e.mime)) {
                self.thumbs.want(&e.path, e.changed);
            }
        }
        let thumb = |e: &Entry| self.thumbs.get(&e.path, e.changed).flatten();
        // The Trash: where each came from under its name, when it was
        // deleted, and its own Put Back and Delete Forever, shown on the row
        // under the pointer and on the chosen ones (no hover on a touch screen).
        let trash = self.in_trash();
        let under = |e: &Entry| tab.from.get(&e.path).and_then(|p| p.parent()).map(files::tilde);
        let row_tools = |k: usize, e: &Entry| -> Element<Msg> {
            let small = |which: Icon, name: &str, m: Msg| button(vergence_widgets::toolbar::icon(which, 14.0, false)).look(ButtonLook::Plain).padding(Padding::all(0.0)).width(Length::Fixed(ROW_TOOL)).height(Length::Fixed(ROW_TOOL)).on_press(m).name(name);
            let tools = row(vec![small(Icon::PutBack, "Put Back", Msg::RowPutBack(k)), small(Icon::Remove, "Delete Forever", Msg::RowForever(k))]).spacing(4.0).align(Align::Center);
            if tab.chosen.contains(&e.path) { tools } else { tools.shown_on_hover() }
        };
        let v = files::View {
            entries: &visible,
            chosen: &chosen,
            in_view,
            click: &click,
            drag: Some(&drag),
            folders: trash || tab.showing == Showing::Recent || self.search.is_some(),
            thumb: if self.shown_view() == ViewKind::Icons { Some(&thumb) } else { None },
            menu: Some(&menu),
            look: &look,
            folder_size: Some(&folder_size),
            grip: Some(grip),
            under: if trash { Some(&under) } else { None },
            changed_heading: trash.then_some("Deleted"),
            trailing: if trash { Some((&row_tools, 2.0 * ROW_TOOL + 4.0)) } else { None },
            rename: self.renaming_row().and_then(|row| self.naming.as_ref().map(|n| files::Rename { row, text: n.text.clone(), typed: Msg::NameTyped, done: Msg::NameDone })),
        };
        let mut body: Vec<Element<Msg>> = Vec::new();
        if let Some(p) = &tab.problem {
            body.push(label(p.clone()).size(12.0).wrap().width(Length::FILL));
        }
        if visible.is_empty() && tab.problem.is_none() {
            let why = if self.search.is_some() && !self.search_done { "Searching…" } else if self.search.is_some() { "Nothing here or below has that in its name." } else if !self.narrow.is_empty() { "Nothing here has that in its name." } else if tab.showing == Showing::Recent { "Nothing opened or saved yet." } else if trash { "The Trash is empty." } else { "This folder is empty." };
            body.push(panel(label(why).size(12.0).dim()).padding(12.0));
        }
        let list = match self.shown_view() {
            ViewKind::List => {
                let (head, rows) = files::list(&v, self.sort, Msg::Sort);
                let mut all = vec![head, rule()];
                all.extend(body);
                all.push(scroll(column(rows).spacing(1.0).width(Length::FILL)).name("Folder"));
                column(all).spacing(2.0).width(Length::FILL).height(Length::FILL)
            }
            ViewKind::Icons => {
                body.push(scroll(panel(files::icons(&v)).padding(6.0).width(Length::FILL)).name("Folder"));
                column(body).spacing(2.0).width(Length::FILL).height(Length::FILL)
            }
        };
        // The Trash window's folder is a page of its own; FileView's shares
        // its page with the sidebar (view()).
        let p = panel(list).padding(4.0).width(Length::FILL).height(Length::FILL).on_menu(Msg::GroundMenu);
        if self.trash_window { p.color(t.page).radius(8.0).edge(t.line, 1.0) } else { p.radius(0.0) }
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
        if !e.dir && vergence_thumbs::has_thumbnail(e.mime) {
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
        panel(files::entry_icon_sized(e, if big { 8.0 } else { 5.0 }).width(Length::FILL).height(Length::Fixed(if big { 150.0 } else { 90.0 }))).width(Length::FILL).height(Length::FILL)
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
                    if matches!(e.kind(), Kind::Picture | Kind::Text) || e.mime == Some("application/pdf") {
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

    fn peek_open(&self) -> bool {
        self.peek.as_ref().is_some_and(|l| l.borrow().open)
    }

    /// Space: the preview opened over this window for the cursor's file, or closed.
    fn toggle_peek(&mut self, window: &mut Window) {
        if self.peek_open() {
            return self.close_peek();
        }
        let Some(e) = self.cursor_entry() else { return };
        let link = preview::link(e);
        window.open_window(vergence_widgets::window(preview::Peek::new(link.clone(), files_theme(&self.desktop))));
        self.peek = Some(link);
    }

    fn close_peek(&mut self) {
        if let Some(l) = self.peek.take() {
            l.borrow_mut().close = true;
            if let Some(w) = &self.waker {
                w.wake();
            }
        }
    }

    /// The preview follows the cursor: its file given to it when it changed.
    fn sync_peek(&self) {
        let Some(l) = &self.peek else { return };
        let now = self.cursor_entry();
        let mut link = l.borrow_mut();
        if !link.open || link.entry.as_ref().map(|e| &e.path) == now.as_ref().map(|e| &e.path) {
            return;
        }
        link.entry = now;
        link.serial += 1;
        drop(link);
        if let Some(w) = &self.waker {
            w.wake();
        }
    }
}

impl Program for Viewer {
    type Message = Msg;

    fn title(&self) -> String {
        self.front_tab().map(Tab::name).unwrap_or_else(|| "FileView".into())
    }

    fn app_id(&self) -> String {
        if self.trash_window { TRASH_ID.into() } else { APP_ID.into() }
    }

    fn theme(&self) -> Theme {
        files_theme(&self.desktop)
    }

    /// The Trash window opens smaller than a folder's (the server's memory of
    /// its place decides once it has one).
    fn opening_size(&self) -> Option<(u32, u32)> {
        self.trash_window.then_some((720, 460))
    }

    fn bar_style(&self) -> Option<BarStyle> {
        Some(vergence_ui::bar_style_setting(&self.desktop, SECTION, BarStyle::Light))
    }

    fn under_bar(&self) -> bool {
        true
    }

    /// The server's tabs, a folder each (files milestone 3); the Trash
    /// window's plain bar.
    fn bar(&self) -> Option<Bar> {
        if self.trash_window {
            return Some(Bar::Plain);
        }
        let tabs = self.tabs.iter().map(|t| BarTab { id: t.id, title: t.name(), closable: true, alert: false, edited: false }).collect();
        Some(Bar::Tabs { tabs, front: self.front_tab().map_or(0, |t| t.id), plus: true, accent: None, menu: Vec::new() })
    }

    fn tab(&mut self, event: TabEvent, window: &mut Window) {
        // A name being changed is kept before the tabs change.
        if self.renaming_row().is_some() {
            self.name_done();
        }
        match event {
            TabEvent::Chosen(id) => {
                self.front = id;
                self.tab_changed();
            }
            TabEvent::Close(id) => self.close_tab(id, window),
            TabEvent::CloseOthers(id) => {
                self.tabs.retain(|t| t.id == id);
                self.front = id;
                self.tab_changed();
            }
            TabEvent::New => self.run(cmd::NEW_TAB, window),
            TabEvent::Moved { tab, to } => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == tab) {
                    let t = self.tabs.remove(i);
                    self.tabs.insert((to as usize).min(self.tabs.len()), t);
                }
            }
            // Out of the row: a window of its own, its folder, history and choice with it.
            TabEvent::Out { tab, .. } => {
                if self.tabs.len() > 1
                    && let Some(t) = self.remove_tab(tab)
                {
                    let mut w = Viewer::new(self.shared.clone());
                    w.take_tab(t, 0);
                    window.open_window(vergence_widgets::window(w));
                }
            }
            TabEvent::Join { tab, into, to } => {
                if let Some(t) = self.remove_tab(tab) {
                    let waker = {
                        let mut s = self.shared.borrow_mut();
                        s.transfers.push((into, to, t));
                        s.windows.iter().find(|(id, _)| *id == into).map(|(_, w)| w.clone())
                    };
                    if let Some(w) = waker {
                        w.wake();
                    }
                    if self.tabs.is_empty() {
                        window.close();
                    }
                }
            }
            TabEvent::Menu { .. } => {}
        }
        self.keep();
        window.redraw();
    }

    /// Its token from the last run, so window memory puts it back; "new":
    /// ask for one. The Trash window keeps none of its own here.
    fn token(&self) -> String {
        if self.trash_window {
            String::new()
        } else if self.token.is_empty() {
            "new".into()
        } else {
            self.token.clone()
        }
    }

    fn token_given(&mut self, token: &str) {
        self.token = token.to_string();
        self.keep();
    }

    fn session_ending(&mut self) {
        self.ending = Some(Instant::now());
        self.keep();
    }

    fn closed(&mut self) {
        self.close_peek();
        let mut s = self.shared.borrow_mut();
        s.windows.retain(|(id, _)| Some(*id) != self.me);
        if s.last_used == self.me {
            s.last_used = None;
        }
        // Closed for an exit: kept for the next session (session milestone 1).
        if vergence_widgets::session::ending(self.ending) || self.trash_window {
            return;
        }
        // Closed by the user: it is not kept.
        s.snaps.retain(|(id, _)| Some(*id) != self.me);
        let all: Vec<keep::Snap> = s.snaps.iter().map(|(_, w)| w.clone()).collect();
        keep::save(&all);
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        self.sync_peek();
        let mut middle = if self.trash_window {
            vec![self.folder_view(t)]
        } else {
            // The sidebar and the folder on one page, a line between them that
            // is pulled to change the sidebar's width (Dave, 2026-11-21).
            let rect = self.divider_rect.clone();
            let divider = drawing(move |scene, _, r: Rect, t: &Theme| {
                rect.set(r);
                scene.fill(Rect::new(r.x + r.w / 2.0 - 0.5, r.y, 1.0, r.h), t.line);
            })
            .on_touch(|touch| Some(Msg::SideGrip(touch)))
            .cursor(vergence_ui::Cursor::ColResize)
            .width(Length::Fixed(7.0))
            .height(Length::FILL)
            .name("Sidebar width");
            let edge = if t.high_contrast { t.text } else { t.line };
            vec![panel(row(vec![self.sidebar_view(t), divider, self.folder_view(t)]).width(Length::FILL).height(Length::FILL)).color(t.page).radius(8.0).edge(edge, 1.0).width(Length::FILL).height(Length::FILL)]
        };
        if self.pane && !self.trash_window {
            middle.push(self.pane_view(t));
        }
        let middle: Element<Msg> = row(middle).spacing(8.0).width(Length::FILL).height(Length::FILL);
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
        let mut foot = if chosen > 1 { format!("{chosen} of {n} chosen") } else { format!("{n} item{}", if n == 1 { "" } else { "s" }) };
        if self.search.is_some() {
            foot = format!("{foot} {} subfolders{}", if self.search_done { "found in" } else { "found so far in" }, if self.search_entries.len() >= search::MAX { " (the first 5000)" } else { "" });
        }
        all.push(rule());
        // How many and what is chosen at the left; the disk's free space at
        // the right (files milestone 3, step 2).
        let free = if self.trash_window { None } else { self.free_here() };
        let free = free.map(|n| format!("{} free", files::size_words(n))).unwrap_or_default();
        all.push(panel(row(vec![label(foot).size(12.0).dim(), space().width(Length::FILL), label(free).size(12.0).dim()]).width(Length::FILL)).radius(0.0).padding(Padding { top: 3.0, bottom: 3.0, left: 12.0, right: 12.0 }).width(Length::FILL));
        column(all).width(Length::FILL).height(Length::FILL)
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        // A name being changed in its row is kept when anything else is done
        // (Dave's list: "a click elsewhere keeps it"); a click on its own row
        // leaves it be.
        if self.renaming_row().is_some() {
            let own_row = matches!(m, Msg::Click(k) | Msg::DragRow(k) if Some(k) == self.renaming_row());
            if own_row {
                return;
            }
            if !matches!(m, Msg::NameTyped(_) | Msg::NameDone | Msg::NameCancel) {
                self.name_done();
            }
        }
        if let Msg::Click(k) = &m {
            // The one chosen item clicked again, with no keys held: renamed after a pause.
            let p = self.visible().get(*k).map(|e| e.path.clone());
            let alone = p.is_some() && self.front_tab().is_some_and(|t| t.chosen.len() == 1 && t.chosen.first() == p.as_ref());
            let plain = !self.mods.ctrl && !self.mods.shift;
            self.rename_after = None;
            if alone && plain && !self.trash_window && !self.in_trash() && self.front_tab().is_some_and(|t| t.dir().is_some()) {
                self.rename_after = p.map(|p| (p, Instant::now()));
                window.wake_after(RENAME_TIMER, self.double_gap() + std::time::Duration::from_millis(60));
            }
        }
        if matches!(m, Msg::DragRow(_)) {
            self.rename_after = None;
        }
        match m {
            Msg::Run(c) => return self.run(c, window),
            Msg::PathEdit => self.path_edit = Some(self.front_tab().and_then(|t| t.dir()).map(|d| d.display().to_string()).unwrap_or_default()),
            Msg::PathTyped(s) => self.path_edit = Some(s),
            Msg::PathGo => self.path_go(),
            Msg::SideGo(i) => self.side_go(i),
            Msg::SideEject(i) => {
                self.side_target = self.side_lines().get(i).cloned();
                return self.run(cmd::SIDE_EJECT, window);
            }
            Msg::SideGrip(touch) => {
                use vergence_widgets::Touch;
                // In window points (the line moves as it is pulled).
                let at = self.divider_rect.get().x;
                match touch {
                    Touch::Press { x, .. } => self.side_pull = Some((at + x, self.side_w)),
                    Touch::Move { x, pressed: true, .. } => {
                        if let Some((x0, w0)) = self.side_pull {
                            self.side_w = (w0 + (at + x - x0)).clamp(SIDE_MIN, SIDE_MAX).round();
                        }
                    }
                    Touch::Release { .. } | Touch::Leave if self.side_pull.take().is_some() => {
                        let w = format!("{:.0}", self.side_w);
                        self.write_key("sidebar-width", (self.side_w != SIDE_W).then_some(w.as_str()));
                    }
                    _ => {}
                }
            }
            Msg::SideFold(sec) => {
                let mut s = self.side_setup();
                if let Some(k) = s.folded.iter().position(|f| *f == sec) {
                    s.folded.remove(k);
                } else {
                    s.folded.push(sec);
                }
                let names: Vec<String> = s.folded.iter().map(|f| f.name().to_string()).collect();
                let v = sidebar::list_value(&names);
                self.write_key("sidebar-folded", (!names.is_empty()).then_some(v.as_str()));
            }
            Msg::SideMenu(i) => {
                let line = self.side_lines().get(i).cloned();
                if let Some(lines) = line.as_ref().map(|l| self.side_menu(l))
                    && !lines.is_empty()
                {
                    self.side_target = line;
                    window.menu(self.menu_at.0, self.menu_at.1, lines);
                }
            }
            Msg::SideDrag(i) => {
                // A favorite pulled away: moved within the sidebar (a kind of
                // its own: dropped on a folder it does nothing).
                if let Some(sidebar::Line::Item(sidebar::Section::Favorites, item)) = self.side_lines().get(i)
                    && let Some(p) = &item.path
                {
                    window.start_drag(&item.name, vec![(SIDEBAR_KIND.into(), p.to_string_lossy().into_owned().into_bytes())]);
                }
            }
            Msg::Click(k) => self.click(k),
            Msg::Sort(by) => self.sort = if self.sort.0 == by { (by, !self.sort.1) } else { (by, by == SortBy::Name) },
            Msg::Narrow(s) => {
                self.narrow = s;
                // While searching below, the search follows the words.
                if self.search.is_some() {
                    self.search_below();
                }
            }
            Msg::SearchBelow => {
                if self.search.is_some() {
                    self.search = None;
                    self.search_entries.clear();
                } else {
                    self.search_below();
                }
            }
            Msg::OpenChosen => self.open_chosen(),
            Msg::OpenInCloseUp => {
                if let Some(e) = self.cursor_entry() {
                    let _ = crate::start(std::process::Command::new("vergence-closeup").arg(&e.path));
                }
            }
            Msg::DismissNotice => self.notice = None,
            Msg::RowPutBack(k) => self.row_tool(k, true, window),
            Msg::RowForever(k) => self.row_tool(k, false, window),
            Msg::RowMenu(k) => {
                let visible: Vec<PathBuf> = self.visible().iter().map(|e| e.path.clone()).collect();
                if let (Some(p), Some(t)) = (visible.get(k).cloned(), self.front_mut())
                    && !t.chosen.contains(&p)
                {
                    t.chosen = vec![p.clone()];
                    t.cursor = Some(p);
                }
                let lines = self.item_menu();
                window.menu(self.menu_at.0, self.menu_at.1, lines);
            }
            Msg::GroundMenu => {
                if let Some(t) = self.front_mut() {
                    t.chosen.clear();
                }
                let lines = self.ground_menu();
                window.menu(self.menu_at.0, self.menu_at.1, lines);
            }
            Msg::Grip(c, touch) => {
                use vergence_widgets::Touch;
                match touch {
                    Touch::Press { x, .. } => self.pulling = Some((c, x, self.widths[c])),
                    Touch::Move { x, pressed: true, .. } => {
                        // The edge is the column's left: pulled right, it narrows.
                        if let Some((c, x0, w0)) = self.pulling {
                            self.widths[c] = (w0 - (x - x0)).clamp(files::COLUMN_MIN, files::COLUMN_MAX).round();
                        }
                    }
                    // Let go: the widths kept.
                    Touch::Release { .. } | Touch::Leave if self.pulling.take().is_some() => {
                        let words = format!("\"{}\"", self.widths.iter().map(|w| format!("{w:.0}")).collect::<Vec<_>>().join(","));
                        self.write_key("columns", Some(&words));
                    }
                    _ => {}
                }
            }
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
                            self.notice = Some(format!("“{}” could not be deleted: {}", self.shown_name(p), files::plain(&e)));
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
        // A folder gone to: kept for the next start.
        self.keep();
        window.redraw();
    }

    fn saw(&mut self, input: &Input) {
        // The window last used takes the folders handed over.
        if matches!(input, Input::Button { pressed: true, .. } | Input::Key { pressed: true, .. }) && self.me.is_some() {
            self.shared.borrow_mut().last_used = self.me;
        }
        if let Input::Button { pressed: true, modifiers, button, x, y } = input {
            self.mods = *modifiers;
            if *button == vergence_ui::Button::Right {
                self.menu_at = (*x, *y);
            }
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
        // The path being typed: Escape lets go; the field has the other keys.
        if self.path_edit.is_some() {
            if *sym == ESCAPE {
                self.path_edit = None;
                window.redraw();
                return Some(true);
            }
            return None;
        }
        let typing = !self.narrow.is_empty();
        match *sym {
            s if m.ctrl && (s == 0x6c || s == 0x4c) && !self.trash_window => {
                self.path_edit = Some(self.front_tab().and_then(|t| t.dir()).map(|d| d.display().to_string()).unwrap_or_default());
            }
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
                self.move_cursor(if self.shown_view() == ViewKind::Icons { -self.per_row() } else { -1 });
                self.want_picture();
            }
            DOWN => {
                self.move_cursor(if self.shown_view() == ViewKind::Icons { self.per_row() } else { 1 });
                self.want_picture();
            }
            LEFT if self.shown_view() == ViewKind::Icons || self.peek_open() => {
                self.move_cursor(-1);
                self.want_picture();
            }
            RIGHT if self.shown_view() == ViewKind::Icons || self.peek_open() => {
                self.move_cursor(1);
                self.want_picture();
            }
            RETURN => {
                self.close_peek();
                self.run(cmd::OPEN, window)
            }
            ESCAPE if self.peek_open() => self.close_peek(),
            ESCAPE if typing => self.narrow.clear(),
            SPACE if !typing => self.run(cmd::PREVIEW, window),
            BACKSPACE if !typing => self.run(cmd::UP, window),
            s if m.ctrl && (s == 0x61 || s == 0x41) && !typing => self.run(cmd::SELECT_ALL, window),
            s if m.ctrl && !m.shift && (s == 0x6e || s == 0x4e) => self.run(cmd::NEW_WINDOW, window),
            s if m.ctrl && (s == 0x77 || s == 0x57) => self.run(cmd::CLOSE_TAB, window),
            s if m.ctrl && (s == 0x74 || s == 0x54) => self.run(cmd::NEW_TAB, window),
            0xff56 if m.ctrl => self.run(cmd::NEXT_TAB, window),
            0xff55 if m.ctrl => self.run(cmd::PREV_TAB, window),
            s if m.ctrl && (s == 0x68 || s == 0x48) => self.run(cmd::HIDDEN, window),
            s if m.ctrl && (s == 0x69 || s == 0x49) && !typing => self.run(cmd::INFO, window),
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
    fn drag_over(&mut self, x: f32, y: f32, kinds: &[String], ctrl: bool) -> u8 {
        // Over the sidebar's favorites: a shortcut made, or a favorite moved;
        // nothing on the disk changes (a copy, so the source keeps its files).
        if !self.trash_window && self.side_rect.get().w > 0.0 && { let r = self.side_rect.get(); x >= r.x && x < r.x + r.w } {
            let ours = kinds.iter().any(|k| k == SIDEBAR_KIND || k == "text/uri-list");
            return if ours && self.side_drop_index(x, y).is_some() { vergence_ui::DROP_COPY } else { vergence_ui::DROP_NONE };
        }
        if !kinds.iter().any(|k| k == "text/uri-list") || self.here().is_none() || self.job.is_some() {
            return vergence_ui::DROP_NONE;
        }
        // Onto the Trash: thrown away (never copied there).
        if self.in_trash() {
            return vergence_ui::DROP_MOVE;
        }
        if ctrl { vergence_ui::DROP_COPY } else { vergence_ui::DROP_MOVE }
    }

    fn dropped(&mut self, x: f32, y: f32, action: u8, items: &[(String, Vec<u8>)], window: &mut Window) {
        // On the sidebar: into the favorites at the place dropped.
        if !self.trash_window && { let r = self.side_rect.get(); r.w > 0.0 && x >= r.x && x < r.x + r.w } {
            if let Some(at) = self.side_drop_index(x, y) {
                let moved: Vec<PathBuf> = match items.iter().find(|(k, _)| k == SIDEBAR_KIND) {
                    Some((_, v)) => vec![PathBuf::from(String::from_utf8_lossy(v).into_owned())],
                    None => items.iter().find(|(k, _)| k == "text/uri-list").map(|(_, v)| String::from_utf8_lossy(v).lines().filter_map(|l| vergence_types::path_of_link(l.trim())).collect()).unwrap_or_default(),
                };
                let moved: Vec<PathBuf> = moved.into_iter().filter(|p| p.is_absolute() && p.exists()).collect();
                if !moved.is_empty() {
                    let favs = sidebar::put(&sidebar::favorites(&self.side_setup()), &moved, at);
                    self.set_favorites(&favs);
                }
            }
            window.redraw();
            return;
        }
        let Some(dir) = self.here() else { return };
        let paths: Vec<PathBuf> = items.iter().find(|(k, _)| k == "text/uri-list").map(|(_, v)| String::from_utf8_lossy(v).lines().filter_map(|l| vergence_types::path_of_link(l.trim())).collect()).unwrap_or_default();
        if paths.is_empty() {
            return;
        }
        if self.in_trash() {
            // What is in the Trash already stays as it is.
            let trash = ops::trash_dir();
            for p in paths.iter().filter(|p| !trash.as_ref().is_some_and(|t| p.starts_with(t))) {
                if let Err(e) = ops::trash(p) {
                    self.notice = Some(format!("“{}” could not be moved to the Trash: {e}", p.file_name().unwrap_or_default().to_string_lossy()));
                    break;
                }
            }
            self.read_all();
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

    fn scroll_to(&mut self) -> Vec<(String, f32)> {
        self.front_mut().and_then(|t| t.want.take()).map(|w| vec![("Folder".to_string(), w)]).unwrap_or_default()
    }

    fn scrolled(&mut self, name: &str, offset: f32) {
        if name == "Sidebar" {
            self.side_scroll = offset;
        }
        if name == "Folder"
            && let Some(t) = self.front_mut()
        {
            t.scroll = offset;
        }
    }

    /// The mouse's Back and Forward buttons (Linux's BTN_SIDE and
    /// BTN_EXTRA) move through the tab's history, as in a browser.
    fn input(&mut self, input: &Input, window: &mut Window) -> bool {
        match input {
            Input::Button { pressed: true, button: vergence_ui::Button::Other(0x113), .. } => self.run(cmd::BACK, window),
            Input::Button { pressed: true, button: vergence_ui::Button::Other(0x114), .. } => self.run(cmd::FORWARD, window),
            _ => return false,
        }
        true
    }

    fn waker(&mut self, waker: Waker) {
        let first = self.waker.is_none() && self.shared.borrow().windows.is_empty();
        self.waker = Some(waker.clone());
        self.register();
        *self.thumb_waker.lock().unwrap_or_else(|e| e.into_inner()) = Some(waker.clone());
        if first {
            let inbox = self.shared.borrow().inbox.clone();
            let w = waker.clone();
            let _ = hand::listen(if self.trash_window { TRASH_ID } else { APP_ID }, inbox, move || w.wake());
        }
        // Once woken, the folder watch's timer is set (woken() has the window).
        waker.wake();
    }

    fn opened(&mut self, id: u32) {
        self.me = Some(id);
        self.register();
        if self.shared.borrow().last_used.is_none() {
            self.shared.borrow_mut().last_used = Some(id);
        }
    }

    fn woken(&mut self, window: &mut Window) -> bool {
        let mut changed = false;
        // The preview's keys (the arrows, Enter) as if pressed here; closed, let go of.
        if let Some(l) = self.peek.clone() {
            let keys = std::mem::take(&mut l.borrow_mut().keys);
            for sym in keys {
                let key = Input::Key { sym, text: None, pressed: true, repeat: false, composing: false, modifiers: Modifiers::default() };
                changed |= self.key_first(&key, window).is_some();
            }
            if !l.borrow().open && self.peek.as_ref().is_some_and(|p| Rc::ptr_eq(p, &l)) {
                self.peek = None;
                changed = true;
            }
        }
        if let Some(s) = &self.search
            && (s.count() != self.search_entries.len() || s.done() != self.search_done)
        {
            self.search_entries = s.found();
            self.search_done = s.done();
            changed = true;
        }
        // Folders counted: their sizes into the entries (sorting by size uses
        // them). A folder counted again after it changed counts too.
        for t in &mut self.tabs {
            changed |= t.put_sizes();
        }
        let n = self.sizes.lock().unwrap_or_else(|x| x.into_inner()).len();
        if n != self.sizes_seen {
            // A count that came to no size (a folder that can't be read) shows as such.
            self.sizes_seen = n;
            changed = true;
        }
        let made = self.thumbs.made();
        if made != self.thumbs_made {
            self.thumbs_made = made;
            changed = true;
        }
        let (text, stamp) = desktop_text();
        if stamp != self.stamp {
            (self.desktop, self.stamp) = (text, stamp);
            changed = true;
        }
        // The other windows kept from the last run.
        let kept = std::mem::take(&mut self.shared.borrow_mut().kept);
        for snap in kept {
            window.open_window(vergence_widgets::window(Viewer::from_snap(self.shared.clone(), &snap)));
        }
        let asked: Vec<Vec<PathBuf>> = std::mem::take(&mut *self.shared.borrow().inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for paths in asked {
            // The Trash window started again: only forward.
            if self.trash_window {
                window.activate();
                continue;
            }
            // For the window last used, if it is another one: it is woken to take them.
            let other = {
                let s = self.shared.borrow();
                s.last_used.filter(|u| Some(*u) != self.me).and_then(|u| s.windows.iter().find(|(id, _)| *id == u).map(|(id, w)| (*id, w.clone())))
            };
            match other {
                Some((id, w)) => {
                    self.shared.borrow_mut().opens.push((id, paths));
                    w.wake();
                }
                None => {
                    self.open_paths(&paths);
                    // Handed over by a start the person just made: forward (the server judges).
                    window.activate();
                    changed = true;
                }
            }
        }
        // Folders handed over for this window, and tabs dropped on it.
        let (opens, moved) = {
            let mut s = self.shared.borrow_mut();
            let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut s.opens).into_iter().partition(|(id, _)| Some(*id) == self.me);
            s.opens = rest;
            let (tabs, others): (Vec<_>, Vec<_>) = std::mem::take(&mut s.transfers).into_iter().partition(|(into, _, _)| Some(*into) == self.me);
            s.transfers = others;
            (mine, tabs)
        };
        for (_, paths) in opens {
            self.open_paths(&paths);
            window.activate();
            changed = true;
        }
        for (_, to, t) in moved {
            self.take_tab(t, to as usize);
            changed = true;
        }
        if changed {
            self.keep();
        }
        // Folders for windows of their own.
        for p in std::mem::take(&mut self.pending) {
            let mut w = Viewer::new(self.shared.clone());
            w.open_paths(&[p]);
            window.open_window(vergence_widgets::window(w));
        }
        // A disk image mounted: in a tab of its own; or why not.
        let answered = self.mounting.as_ref().and_then(|(n, a)| a.lock().unwrap_or_else(|e| e.into_inner()).take().map(|r| (n.clone(), r)));
        if let Some((name, r)) = answered {
            self.mounting = None;
            match r {
                // Mounted, and shown under Locations: no tab of its own (Dave,
                // 2026-11-21: "A new tab should never appear unless the user calls it").
                Ok(_) => self.notice = None,
                Err(why) => self.notice = Some(format!("“{name}” could not be mounted: {why}")),
            }
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
        if id == RENAME_TIMER {
            // Still the one chosen, and no second click came: its name a field.
            let Some((p, at)) = self.rename_after.take() else { return false };
            let still = self.front_tab().is_some_and(|t| t.chosen == [p.clone()]);
            let latest = self.last_click.as_ref().is_some_and(|(t, q)| *q == p && *t >= at);
            if still && latest && self.naming.is_none() && at.elapsed() >= self.double_gap() {
                self.start_rename(p);
                return true;
            }
            return false;
        }
        if id != WATCH_TIMER {
            return false;
        }
        window.wake_after(WATCH_TIMER, WATCH_EVERY);
        // Where the tabs are scrolled, kept (at most every WATCH_EVERY).
        self.keep();
        // A new drive list from the server: the sidebar's Locations again
        // (and a tab on a drive gone goes home).
        let seen = files::drives_seen();
        if seen != self.drives_seen {
            self.drives_seen = seen;
            *self.ejectable.borrow_mut() = None;
            let drives: Vec<PathBuf> = files::mounted_disks().into_iter().chain(files::network_mounts()).collect();
            // A drive listed before and not now: tabs on it go home.
            let went: Vec<PathBuf> = self.drives_last.iter().filter(|d| !drives.contains(d)).cloned().collect();
            for t in self.tabs.iter_mut().filter(|t| t.dir().is_some_and(|d| went.iter().any(|w| d.starts_with(w)))) {
                t.go(Showing::Folder(home()));
            }
            self.drives_last = drives;
            return true;
        }
        // A drive ejected or pulled out leaves the server's list, and its tabs
        // go home above. One under /media the list does not show as mounted
        // yet is not looked into again here: a look has FreeBSD's automounter
        // mount it (an ejected stick came back that way). Not sent home: a
        // stick just clicked is mounting, and the server sees it at its next
        // look (Dave, 2026-11-21: clicking the stick "dumped" him at Home).
        let mounted: Vec<PathBuf> = files::drive_list().into_iter().filter(|d| d.mounted).map(|d| PathBuf::from(d.path)).collect();
        let off = |d: &Path| d.starts_with("/media/") && !mounted.iter().any(|m| d.starts_with(m));
        // The folder changed on disk: read again.
        let Some(t) = self.front_mut() else { return false };
        if t.dir().is_some_and(off) {
            return false;
        }
        let now = t.dir().and_then(|d| std::fs::metadata(d).and_then(|m| m.modified()).ok());
        if t.dir().is_some() && now != t.stamp {
            t.read();
            return true;
        }
        false
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        if self.trash_window {
            return Some((
                false,
                vec![
                    C::role(cmd::CLOSE, Role::CloseWindow),
                    C::own(cmd::OPEN, "File", "Open", "Enter"),
                    C::own(cmd::PREVIEW, "File", "Preview", "Space").checked(self.peek_open()),
                    C::own(cmd::RESTORE, "File", "Put Back", "").featured(),
                    C::own(cmd::DELETE, "File", "Delete Forever…", "Delete"),
                    C::own(cmd::EMPTY_TRASH, "File", "Empty Trash…", "").featured(),
                    C::own(cmd::INFO, "File", "Properties", "Ctrl+I"),
                    C::role(cmd::SELECT_ALL, Role::SelectAll),
                    C::own(cmd::ROWS_SMALL, "View", "Small rows", "").checked(self.list_look().rows == RowSize::Small),
                    C::own(cmd::ROWS_MEDIUM, "View", "Medium rows", "").checked(self.list_look().rows == RowSize::Medium),
                    C::own(cmd::ROWS_LARGE, "View", "Large rows", "").checked(self.list_look().rows == RowSize::Large),
                    C::own(cmd::RELOAD, "View", "Read again", "Ctrl+R"),
                    C::role(cmd::SETTINGS, Role::Settings),
                    C::role(cmd::ABOUT, Role::About),
                ],
            ));
        }
        Some((
            false,
            vec![
                C::role(cmd::NEW_WINDOW, Role::NewWindow).featured(),
                C::role(cmd::NEW_TAB, Role::NewTab),
                C::role(cmd::CLOSE_TAB, Role::CloseTab),
                C::role(cmd::CLOSE, Role::CloseWindow),
                C::role(cmd::NEXT_TAB, Role::NextTab),
                C::role(cmd::PREV_TAB, Role::PreviousTab),
                C::own(cmd::OPEN, "File", "Open", "Enter").featured(),
                C::own(cmd::PREVIEW, "File", "Preview", "Space").checked(self.peek_open()),
                C::role(cmd::SELECT_ALL, Role::SelectAll),
                C::role(cmd::UNDO, Role::Undo),
                C::role(cmd::CUT, Role::Cut),
                C::role(cmd::COPY, Role::Copy),
                C::role(cmd::PASTE, Role::Paste),
                C::own(cmd::DUPLICATE, "Edit", "Duplicate", "Ctrl+D"),
                C::own(cmd::RENAME, "Edit", "Rename…", "F2"),
                C::own(cmd::NEW_FOLDER, "File", "New folder", "Ctrl+Shift+N").featured(),
                C::own(cmd::INFO, "File", "Properties", "Ctrl+I"),
                C::own(cmd::TRASH, "File", "Move to the Trash", "Delete"),
                C::own(cmd::DELETE, "File", "Delete for good…", "Shift+Delete"),
                C::own(cmd::EMPTY_TRASH, "File", "Empty Trash…", ""),
                C::own(cmd::EJECT, "File", "Eject", "").enabled(self.here_ejectable()),
                C::own(cmd::LIST, "View", "As a list", "Ctrl+1").checked(self.view == ViewKind::List),
                C::own(cmd::ICONS, "View", "As icons", "Ctrl+2").checked(self.view == ViewKind::Icons),
                C::own(cmd::PANE, "View", "Preview pane", "").checked(self.pane).featured(),
                C::own(cmd::HIDDEN, "View", "Hidden files", "Ctrl+H").checked(self.hidden),
                C::own(cmd::ROWS_SMALL, "View", "Small rows", "").checked(self.list_look().rows == RowSize::Small),
                C::own(cmd::ROWS_MEDIUM, "View", "Medium rows", "").checked(self.list_look().rows == RowSize::Medium),
                C::own(cmd::ROWS_LARGE, "View", "Large rows", "").checked(self.list_look().rows == RowSize::Large),
                C::own(cmd::GROUP_KIND, "View", "Group by kind", "").checked(self.list_look().group == GroupBy::Kind),
                C::own(cmd::GROUP_DATE, "View", "Group by date", "").checked(self.list_look().group == GroupBy::Date),
                C::own(cmd::GROUP_SIZE, "View", "Group by size", "").checked(self.list_look().group == GroupBy::Size),
                C::own(cmd::GROUP_NONE, "View", "No groups", "").checked(self.list_look().group == GroupBy::None),
                C::own(cmd::RELOAD, "View", "Read again", "Ctrl+R"),
                C::own(cmd::BACK, "Go", "Back", "Alt+Left"),
                C::own(cmd::FORWARD, "Go", "Forward", "Alt+Right"),
                C::own(cmd::UP, "Go", "Up", "Alt+Up"),
                C::role(cmd::SETTINGS, Role::Settings),
                C::role(cmd::ABOUT, Role::About),
            ],
        ))
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        self.run(id, window);
    }

    fn style(&self) -> vergence_ui::Style {
        let min = if self.trash_window { (420.0, 260.0) } else { (560.0, 360.0) };
        vergence_ui::Style { min, ..Default::default() }
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
                title: "FileView".into(),
                rows: vec![
                    Row::new("view", "New windows show", "A folder as a list or as icons.", Control::Choose { choices: vec![Choice::text("A list", "list"), Choice::text("Icons", "icons")], desktop: None, app_default: 0 }),
                    Row::new("preview-pane", "Preview pane", "The chosen file at the window's right.", Control::Switch { default: false }),
                    Row::new("hidden", "Hidden files", "Names that start with a dot.", Control::Switch { default: false }),
                    Row::new("row-size", "Row size", "How tall the list's rows are.", Control::Choose { choices: vec![Choice::text("Small", "small"), Choice::text("Medium", "medium"), Choice::text("Large", "large")], desktop: None, app_default: 1 }),
                    Row::new("group-by", "Group by", "The list under headings: by kind (Folders, Documents, Pictures…), by date (Today, Yesterday…), by size, or none.", Control::Choose { choices: vec![Choice::text("Kind", "kind"), Choice::text("Date", "date"), Choice::text("Size", "size"), Choice::text("None", "none")], desktop: None, app_default: 0 }),
                    Row::new("recent", "Recent in the sidebar", "The files opened most lately, under the sidebar's Locations.", Control::Switch { default: true }),
                ],
            },
            Part {
                title: "On this desktop".into(),
                rows: vec![
                    Row::new("style", "Style", "While the desktop's Style is App default.", Control::Choose { choices: styles.clone(), desktop: Some("Built-in (Light)".into()), app_default: 0 }).locked(locked.clone()),
                    Row::new("bar-style", "Title bar", "", Control::Choose { choices: styles, desktop: Some("Built-in (Light)".into()), app_default: 0 }).locked(locked),
                    Row::new("opacity", "Opacity", "30 to 100: how much of the window's ground shows.", Control::Percent { min: 30, desktop: format!("Desktop default ({:.0})", opacity * 100.0), now: (opacity * 100.0).round() as u32 }),
                    Row::new("", "Window places", "Where the desktop puts FileView's windows, for each set of screens.", Control::Button { label: "Forget…".into(), id: 1 }),
                ],
            },
        ]
    }

    fn button(&self, id: u32) {
        if id == 1 {
            let _ = crate::start(std::process::Command::new("vergence-settings").args(["--page", "windows", "--forget", APP_ID]));
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
    // The Trash window (Dave, 2026-11-18): the desktop's Trash and the
    // dock's; one, brought forward if it is open already.
    if args.iter().any(|a| a == "--trash") {
        if hand::hand_over(TRASH_ID, &[]) {
            return;
        }
        let shared = Rc::new(RefCell::new(Shared::new()));
        let Some(v) = Viewer::trash_window(shared) else {
            eprintln!("vergence-files: no Trash (no home folder)");
            std::process::exit(1);
        };
        if let Err(e) = vergence_widgets::run(v, (720, 460)) {
            eprintln!("vergence-files: {e}");
            std::process::exit(1);
        }
        return;
    }
    let paths: Vec<PathBuf> = args.iter().filter(|a| !a.starts_with("--")).map(|a| vergence_types::path_of_link(a).unwrap_or_else(|| PathBuf::from(a))).map(|p| std::path::absolute(&p).unwrap_or(p)).collect();
    if hand::hand_over(APP_ID, &paths) {
        return;
    }
    let shared = Rc::new(RefCell::new(Shared::new()));
    // The windows kept from the last run come back (session milestone 1's
    // rules), the first here and the rest once it is open; folders named on
    // the command line join the first. `[session] restore = false`: forgotten.
    let mut kept = if vergence_widgets::session::restore() {
        keep::load()
    } else {
        keep::save(&[]);
        Vec::new()
    };
    let mut v = if kept.is_empty() {
        Viewer::new(shared.clone())
    } else {
        let first = kept.remove(0);
        shared.borrow_mut().kept = kept;
        Viewer::from_snap(shared.clone(), &first)
    };
    if !paths.is_empty() || v.tabs.is_empty() {
        v.open_paths(&paths);
    }
    if let Err(e) = vergence_widgets::run(v, (1040, 680)) {
        eprintln!("vergence-files: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> SharedRef {
        lone_shared()
    }

    #[test]
    fn the_trash_lists_names_they_had_and_where_from() {
        let d = std::env::temp_dir().join(format!("vergence-files-trash-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (files_dir, info) = (d.join("files"), d.join("info"));
        std::fs::create_dir_all(&files_dir).unwrap();
        std::fs::create_dir_all(&info).unwrap();
        // Two notes.txt thrown away: the second has a name of its own in the Trash.
        std::fs::write(files_dir.join("notes.txt"), "one").unwrap();
        std::fs::write(files_dir.join("notes 2.txt"), "two").unwrap();
        std::fs::write(info.join("notes.txt.trashinfo"), "[Trash Info]\nPath=/home/u/Documents/notes.txt\nDeletionDate=2026-11-17T10:00:00\n").unwrap();
        std::fs::write(info.join("notes 2.txt.trashinfo"), "[Trash Info]\nPath=/home/u/src/notes.txt\nDeletionDate=2026-11-18T10:00:00\n").unwrap();
        // One with no info (put there by hand) keeps its own name.
        std::fs::write(files_dir.join("stray"), "").unwrap();
        let mut v = files::read_folder(&files_dir).unwrap();
        let from = as_trashed(&mut v, &d);
        let by = |p: &str| v.iter().find(|e| e.path == files_dir.join(p)).unwrap();
        assert_eq!(by("notes.txt").name, "notes.txt");
        assert_eq!(by("notes 2.txt").name, "notes.txt", "shown by the name it had");
        assert_eq!(from.get(&files_dir.join("notes 2.txt")).map(PathBuf::as_path), Some(Path::new("/home/u/src/notes.txt")));
        assert_eq!(by("notes 2.txt").changed, std::fs::metadata(info.join("notes 2.txt.trashinfo")).and_then(|m| m.modified()).ok(), "deleted when its info was written");
        assert_eq!(by("stray").name, "stray");
        assert!(!from.contains_key(&files_dir.join("stray")));
        std::fs::remove_dir_all(&d).unwrap();
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
        let mut v = Viewer::new(shared());
        v.open_paths(&[dir.join("a.txt")]);
        assert_eq!(v.front_tab().unwrap().chosen, vec![dir.join("a.txt")]);
        // Files milestone 3: more folders handed over open in tabs of this
        // window, the newest in front; nothing given, a window of its own.
        v.open_paths(std::slice::from_ref(&sub));
        assert_eq!(v.tabs.len(), 2);
        assert_eq!(v.front_tab().unwrap().showing, Showing::Folder(sub.clone()));
        assert!(v.pending.is_empty());
        v.open_paths(&[]);
        assert_eq!((v.tabs.len(), v.pending.len()), (2, 1));
        v.narrow = "zzz".into();
        assert!(v.visible().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn free_space_is_read_from_df() {
        let text = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n/dev/nvme0n1p2   479596204 201234567 254000000      45% /\n";
        assert_eq!(parse_df_free(text), Some(254_000_000 * 1024));
        assert_eq!(parse_df_free("only a heading\n"), None);
        // The real thing, on whatever system this is.
        assert!(df_free(&std::env::temp_dir()).is_some_and(|n| n > 0));
    }

    /// Back and Forward bring each folder back where it was scrolled; a
    /// folder new to the tab starts at its top.
    #[test]
    fn folders_come_back_at_their_scroll_places() {
        let dir = std::env::temp_dir().join(format!("vergence-files-places-{}", std::process::id()));
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let mut v = Viewer::new(shared());
        v.open_paths(std::slice::from_ref(&a));
        v.front_mut().unwrap().scroll = 300.0;
        v.go(Showing::Folder(b.clone()));
        assert_eq!(v.front_mut().unwrap().want.take(), Some(0.0), "a new folder: the top");
        v.front_mut().unwrap().scroll = 75.0;
        v.back();
        assert_eq!(v.front_mut().unwrap().want.take(), Some(300.0));
        v.forward();
        assert_eq!(v.front_mut().unwrap().want.take(), Some(75.0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Rename in place (step 5): the one chosen item clicked again, then a
    /// pause, makes its name a field; a double click opens it instead.
    #[test]
    fn a_second_click_and_a_pause_renames_a_double_click_opens() {
        let dir = std::env::temp_dir().join(format!("vergence-files-ren-{}", std::process::id()));
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let mut v = Viewer::new(shared());
        v.open_paths(std::slice::from_ref(&dir));
        let mut w = Window::default();
        // A double click on the chosen folder: it opens, nothing is renamed.
        v.update(Msg::Click(0), &mut w);
        std::thread::sleep(v.double_gap() + std::time::Duration::from_millis(50));
        v.update(Msg::Click(0), &mut w);
        v.update(Msg::Click(0), &mut w);
        assert_eq!(v.front_tab().unwrap().showing, Showing::Folder(sub.clone()), "opened");
        assert!(v.rename_after.is_none());
        std::thread::sleep(v.double_gap() + std::time::Duration::from_millis(80));
        assert!(!v.timer(RENAME_TIMER, &mut w) && v.naming.is_none(), "no rename after a double click");
        // Back up: a click, a pause, a click, a pause: its name a field in its row.
        v.back();
        v.update(Msg::Click(0), &mut w);
        std::thread::sleep(v.double_gap() + std::time::Duration::from_millis(50));
        v.update(Msg::Click(0), &mut w);
        std::thread::sleep(v.double_gap() + std::time::Duration::from_millis(80));
        assert!(v.timer(RENAME_TIMER, &mut w));
        assert_eq!(v.renaming_row(), Some(0));
        // A click on another place keeps the name typed.
        v.update(Msg::NameTyped("renamed".into()), &mut w);
        v.update(Msg::Run(cmd::RELOAD), &mut w);
        assert!(dir.join("renamed").is_dir() && v.naming.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Tabs: each keeps its folder and choice; the next and previous go
    /// round; a tab taken out leaves the next one in front, and a tab from
    /// another window takes this window's folder sizes.
    #[test]
    fn tabs_keep_their_own_folders() {
        let dir = std::env::temp_dir().join(format!("vergence-files-tabs-{}", std::process::id()));
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let mut v = Viewer::new(shared());
        v.open_paths(&[a.clone(), b.clone()]);
        let ids: Vec<u32> = v.tabs.iter().map(|t| t.id).collect();
        assert_eq!(v.front, ids[1], "the last handed over in front");
        v.narrow = "x".into();
        v.front = ids[0];
        v.tab_changed();
        assert!(v.narrow.is_empty(), "the narrowing is the window's: it starts again");
        assert_eq!(v.front_tab().unwrap().showing, Showing::Folder(a.clone()));
        let t = v.remove_tab(ids[0]).unwrap();
        assert_eq!(v.front, ids[1]);
        let mut other = Viewer::new(shared());
        other.take_tab(t, 0);
        assert!(Arc::ptr_eq(&other.tabs[0].sizes, &other.sizes));
        assert_eq!(other.front_tab().unwrap().showing, Showing::Folder(a.clone()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_read_again_keeps_its_counted_size() {
        let dir = std::env::temp_dir().join(format!("vergence-files-s-{}", std::process::id()));
        let (big, small) = (dir.join("big"), dir.join("small"));
        std::fs::create_dir_all(&big).unwrap();
        std::fs::create_dir_all(&small).unwrap();
        let mut v = Viewer::new(shared());
        v.open_paths(std::slice::from_ref(&dir));
        let when = |p: &Path| std::fs::metadata(p).unwrap().modified().ok();
        v.sizes.lock().unwrap().insert(big.clone(), (when(&big), Some(9_000)));
        v.sizes.lock().unwrap().insert(small.clone(), (when(&small), Some(10)));
        v.sort = (files::SortBy::Size, false);
        let names = |v: &Viewer| v.visible().iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        assert!(v.front_mut().unwrap().put_sizes(), "counted sizes go into the entries");
        assert_eq!(names(&v), vec!["big", "small"], "largest first");
        // Read again (a change in the folder): the sizes are there at once.
        v.front_mut().unwrap().read();
        assert_eq!(names(&v), vec!["big", "small"]);
        assert!(!v.front_mut().unwrap().put_sizes(), "nothing more to put");
        // A folder changed since it was counted is not given the old size.
        std::fs::write(small.join("x"), "x").unwrap();
        v.front_mut().unwrap().read();
        let e = v.front_tab().unwrap().entries.iter().find(|e| e.name == "small").unwrap().size;
        assert_ne!(e, 10);
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
