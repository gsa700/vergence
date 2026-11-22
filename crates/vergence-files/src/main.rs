//! File Viewer (files milestone 1, step 7; docs/files-milestone-1.md §5):
//! Vergence's file manager, named after NeXTSTEP's Workspace Manager window,
//! with Space to preview as in macOS's Finder, and remote file systems
//! through the system's own tools (see docs/apps-roadmap.md).
//!
//! - **The window**: the toolbar (Back, Forward, Up, the folder's path,
//!   List or Icons, the preview pane, hidden files, a field that narrows the
//!   folder as you type), the places sidebar, the folder as a list or icons
//!   (vergence-widgets files.rs, shared with the Open dialog), the preview
//!   pane at the right. A folder a tab in the bar (tabs back in files
//!   milestone 3, after milestone 2 had taken them out; see docs/files-milestone-3.md):
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

mod actions;
mod commands;
mod desktop;
mod dnd;
mod drives;
mod hand;
mod keep;
mod keys;
mod listen;
mod ops;
mod pane;
mod preview;
mod props;
mod search;
mod settings;
mod sidebar;
mod sidebar_actions;
mod tab;
mod view;
mod watch;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use vergence_scene::{Image, Rect};
use vergence_ui::{AppCommand as C, Bar, BarStyle, BarTab, Modifiers, TabEvent};
use vergence_widgets::files::{self, Entry, SortBy};
use vergence_widgets::{Element, Input, Program, Theme, Waker, Window};

use commands::cmd;
use tab::{COUNT_MOST, Showing, Sizes, Tab, size_worker};
use view::ViewKind;

const APP_ID: &str = "vergence-files";
/// The Trash window's: a process of its own, so the dock
/// shows it with the Trash's icon and window memory keeps its place apart.
const TRASH_ID: &str = "vergence-trash";
const SECTION: &str = "[files]";
/// A click on the one chosen item, then a pause: its name becomes a field
/// (files milestone 3, step 5), unless a second click made it a double.
const RENAME_TIMER: u64 = 2;
/// A double click's longest gap when the desktop says none (Settings > Mouse does).
const DOUBLE: std::time::Duration = std::time::Duration::from_millis(450);
/// The preview pane's width.
const PANE_W: f32 = 260.0;
/// A Trash row's own buttons (Put Back, Delete Forever), each this square.
const ROW_TOOL: f32 = 22.0;

/// A question in the strip above the folder.
#[derive(Clone, Debug)]
enum Ask {
    /// A paste would replace these names.
    Clash {
        sources: Vec<PathBuf>,
        dir: PathBuf,
        cut: bool,
        names: Vec<String>,
    },
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
        Shared {
            inbox: Arc::default(),
            next_tab: 0,
            clip: None,
            undo: Vec::new(),
            windows: Vec::new(),
            transfers: Vec::new(),
            opens: Vec::new(),
            last_used: None,
            snaps: Vec::new(),
            kept: Vec::new(),
        }
    }
}

type SharedRef = Rc<RefCell<Shared>>;

/// A Shared of its own, for a Properties window opened from the desktop's
/// icons (its renames' undo is nobody's).
fn lone_shared() -> SharedRef {
    Rc::new(RefCell::new(Shared::new()))
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
        Some(p) => {
            (std::fs::read_to_string(&p).unwrap_or_default(), std::fs::metadata(&p).and_then(|m| m.modified()).ok())
        }
        None => (String::new(), None),
    }
}

/// A notification card (the session's, over the bus), as File Viewer.
/// Outside a Vergence session it is only logged.
pub(crate) fn notify(summary: &str, body: &str) {
    if std::env::var_os(vergence_protocol::BUS_ENV).is_some()
        && let Ok(mut bus) = vergence_protocol::Client::connect(APP_ID)
        && bus
            .request(|serial| vergence_protocol::bus::ClientMsg::Notify {
                serial,
                replaces: 0,
                summary: summary.into(),
                body: body.into(),
                urgency: 1,
                timeout_ms: -1,
                actions: Vec::new(),
            })
            .is_ok()
    {
        return;
    }
    eprintln!("vergence-files: {summary}: {body}");
}

/// Opens files with the apps chosen for their types: the server's Open in a
/// Vergence session (the dock shows them starting), else started here.
/// Starts a program and lets it run on its own: a thread waits for it, so
/// it is not left behind as a zombie when it ends (the desktop's process
/// lives all session and starts a FileView for each folder opened, each
/// left over once it hands its folder on).
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
        return match bus.request(|serial| vergence_protocol::bus::ClientMsg::Open {
            serial,
            items: items.clone(),
            app: app.unwrap_or_default().to_string(),
        }) {
            Ok(vergence_protocol::bus::ServerMsg::Done { .. }) => Ok(()),
            Err(vergence_protocol::ClientError::Refused(_, m))
            | Ok(vergence_protocol::bus::ServerMsg::Error { message: m, .. }) => Err(m),
            other => Err(format!("{other:?}")),
        };
    }
    let parsed: Vec<vergence_shell::open::Item> = items.iter().map(|i| vergence_shell::open::item(i)).collect();
    let (groups, failed) =
        vergence_shell::open::plan(&parsed, app, &vergence_shell::apps::installed(), &vergence_types::lists());
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
        let view = if vergence_ui::table_value(&desktop, SECTION, "view").as_deref() == Some("icons") {
            ViewKind::Icons
        } else {
            ViewKind::List
        };
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
            side_w: vergence_ui::table_value(&desktop_now, SECTION, "sidebar-width")
                .and_then(|v| v.trim().parse::<f32>().ok())
                .filter(|w| w.is_finite())
                .map_or(SIDE_W, |w| w.clamp(SIDE_MIN, SIDE_MAX)),
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
            widths: vergence_ui::table_value(&desktop_now, SECTION, "columns")
                .map_or(files::COLUMN_WIDTHS, |v| files::parse_widths(&v)),
            pulling: None,
            trash_window: false,
            sizes,
            sizes_seen: 0,
            size_asked: RefCell::new(std::collections::HashSet::new()),
            size_queue,
        }
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
        let tabs = self
            .tabs
            .iter()
            .map(|t| BarTab { id: t.id, title: t.name(), closable: true, alert: false, edited: false })
            .collect();
        Some(Bar::Tabs {
            tabs,
            front: self.front_tab().map_or(0, |t| t.id),
            plus: true,
            accent: None,
            menu: Vec::new(),
        })
    }

    fn tab(&mut self, event: TabEvent, window: &mut Window) {
        self.on_tab(event, window)
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
        self.window_view(t)
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        // A name being changed in its row is kept when anything else is done,
        // a click elsewhere included (see docs/files-milestone-3.md); a click on
        // its own row leaves it be.
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
            let alone =
                p.is_some() && self.front_tab().is_some_and(|t| t.chosen.len() == 1 && t.chosen.first() == p.as_ref());
            let plain = !self.mods.ctrl && !self.mods.shift;
            self.rename_after = None;
            if alone
                && plain
                && !self.trash_window
                && !self.in_trash()
                && self.front_tab().is_some_and(|t| t.dir().is_some())
            {
                self.rename_after = p.map(|p| (p, Instant::now()));
                window.wake_after(RENAME_TIMER, self.double_gap() + std::time::Duration::from_millis(60));
            }
        }
        if matches!(m, Msg::DragRow(_)) {
            self.rename_after = None;
        }
        match m {
            Msg::Run(c) => return self.run(c, window),
            Msg::PathEdit => {
                self.path_edit =
                    Some(self.front_tab().and_then(|t| t.dir()).map(|d| d.display().to_string()).unwrap_or_default())
            }
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
                    window.start_drag(
                        &item.name,
                        vec![(SIDEBAR_KIND.into(), p.to_string_lossy().into_owned().into_bytes())],
                    );
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
                        let words = format!(
                            "\"{}\"",
                            self.widths.iter().map(|w| format!("{w:.0}")).collect::<Vec<_>>().join(",")
                        );
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
                            self.notice =
                                Some(format!("“{}” could not be deleted: {}", self.shown_name(p), files::plain(&e)));
                            break;
                        }
                        // Deleted out of the Trash: its info goes too.
                        if self.in_trash()
                            && let (Some(dir), Some(n)) = (ops::trash_dir(), p.file_name())
                        {
                            let _ = std::fs::remove_file(
                                dir.join("info").join(format!("{}.trashinfo", n.to_string_lossy())),
                            );
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
        if matches!(input, Input::Button { pressed: true, .. } | Input::Key { pressed: true, .. }) && self.me.is_some()
        {
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
        self.on_key_first(input, window)
    }

    fn drag_over(&mut self, x: f32, y: f32, kinds: &[String], ctrl: bool) -> u8 {
        self.on_drag_over(x, y, kinds, ctrl)
    }

    fn dropped(&mut self, x: f32, y: f32, action: u8, items: &[(String, Vec<u8>)], window: &mut Window) {
        self.on_dropped(x, y, action, items, window)
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
            Input::Button { pressed: true, button: vergence_ui::Button::Other(0x113), .. } => {
                self.run(cmd::BACK, window)
            }
            Input::Button { pressed: true, button: vergence_ui::Button::Other(0x114), .. } => {
                self.run(cmd::FORWARD, window)
            }
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
        self.on_woken(window)
    }

    fn timer(&mut self, id: u64, window: &mut Window) -> bool {
        self.on_timer(id, window)
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        self.command_list()
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        self.run(id, window);
    }

    fn style(&self) -> vergence_ui::Style {
        let min = if self.trash_window { (420.0, 260.0) } else { (560.0, 360.0) };
        vergence_ui::Style { min, ..Default::default() }
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
    // The Trash window (see docs/backup-milestone-1.md): the desktop's Trash and the
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
    let paths: Vec<PathBuf> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|a| vergence_types::path_of_link(a).unwrap_or_else(|| PathBuf::from(a)))
        .map(|p| std::path::absolute(&p).unwrap_or(p))
        .collect();
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
mod tests;
