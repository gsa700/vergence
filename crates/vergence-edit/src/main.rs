//! Vergence's text editor: a native plain-text editor with tabs, autocomplete and line numbers.
//!
//! Editor milestone 1, docs/editor-milestone-1.md: an editor like TextEdit on the Mac or GNOME's
//! Text Editor. Plain text for now (other formats, such as Markdown and RTF, later), on the
//! toolkit's text area.
//!
//! - **Windows and tabs**: one process for all its windows (hand.rs); a document a tab, in the
//!   bar the server draws; tabs dragged between windows or out into their own. A tab shows its
//!   file's name, with a dot while it has changes not saved.
//! - **Files** (files.rs): UTF-8, line endings and a final newline kept as found; saving writes
//!   a new file and renames it over the old, so a crash never leaves half a file. Text that is
//!   not UTF-8 opens read-only.
//! - **Closing with changes asks first**: Save, Don't save, Cancel.
//! - **Its settings** (Ctrl+,): font, text size, line numbers, wrapping, tabs, the current
//!   line's shade, its look; desktop.toml `[editor]`.
//!
//! Uses vergence-widgets (the text area, the file dialog, spelling through vergence-spell),
//! vergence-ui for the window and vergence-types for what a file is. Start reading: this file
//! (the app), tab.rs (a document in a tab), files.rs (reading and saving), then hand.rs.
//!
//! Rule: a save never overwrites in place; it writes a new file and renames it over the old.

mod commands;
mod files;
mod find;
mod hand;
mod keep;
mod save;
mod session;
mod settings;
mod tab;
mod view;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use vergence_ui::{AppCommand as C, Bar, BarStyle, BarTab, TabEvent};
use vergence_widgets::edit::Ending;
use vergence_widgets::{Element, Input, Program, Theme, Waker, Window};

use commands::cmd;
use find::Find;
use save::{Answer, Asking, Pick, Picking};
use settings::desktop_text;
use tab::{DocTab, is_prose, stamp};
use view::grouped;

const APP_ID: &str = "vergence-edit";
const SECTION: &str = "[editor]";

/// The text size's range for now, as the terminal's.
const ZOOM_MIN: f32 = 0.5;
const ZOOM_MAX: f32 = 3.0;

/// What all the editor's windows share (they run on one thread).
struct Shared {
    /// Each open window's toolkit number and the means to wake it.
    windows: Vec<(u32, Waker)>,
    /// Tabs on their way to another window: (that window, where in its row, the tab).
    transfers: Vec<(u32, u32, DocTab)>,
    /// Files other starts handed over (hand.rs).
    inbox: Arc<Mutex<Vec<Vec<PathBuf>>>>,
    /// The waker the hand-over thread calls: the first window's.
    listener: Arc<Mutex<Option<Waker>>>,
    /// The ids tabs are given, across windows (so a joined tab keeps its id).
    next_tab: u32,
    /// What each window keeps (keep.rs), by its toolkit number.
    snaps: Vec<(u32, keep::WindowSnap)>,
    /// Windows kept from the last run still to open (the first window opens them).
    pending: Vec<keep::WindowSnap>,
}

/// Timers of the editor's own.
const KEEP_TIMER: u64 = 1;
const WATCH_TIMER: u64 = 2;
/// How long after a change what is kept is written.
const KEEP_AFTER: std::time::Duration = std::time::Duration::from_secs(1);
/// How often open files are looked at for changes on disk.
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

type SharedRef = Rc<RefCell<Shared>>;

#[derive(Debug, Clone)]
enum Msg {
    Changed,
    DismissNotice,
    ToggleSpaces,
    ToggleEnding,
    FindTyped(String),
    WithTyped(String),
    FindNext,
    FindPrev,
    FindCase,
    FindWhole,
    FindReplace,
    FindClose,
    ReplaceOne,
    ReplaceAll,
    Reload,
    KeepMine,
    GotoTyped(String),
    GotoGo,
    GotoClose,
}

struct EditWindow {
    shared: SharedRef,
    /// When the session said it is ending (an exit closes the window next):
    /// the close is then the session's, and the window stays kept.
    ending: Option<std::time::Instant>,
    tabs: Vec<DocTab>,
    front: u32,
    me: Option<u32>,
    waker: Option<Waker>,
    desktop: String,
    stamp: Option<SystemTime>,
    /// Ctrl+= and Ctrl+−, for this window while it is open.
    zoom: f32,
    prefs_open: Rc<Cell<bool>>,
    asking: Option<Asking>,
    /// Tabs still to close (each asked about first if changed), and whether
    /// the window closes after them.
    closing: Vec<u32>,
    closing_window: bool,
    notice: Option<String>,
    picking: Option<Pick>,
    find: Option<Find>,
    /// Go to line (Ctrl+L): what is typed.
    goto: Option<String>,
    /// The window's persistence token (the server's; kept for next time).
    token: String,
    keep_pending: bool,
    watching: bool,
    /// The page's color as last drawn (with the wallpaper's tint): the tab in
    /// front wears it, so the two match.
    page_drawn: Cell<Option<u32>>,
    /// Bumped to give the keyboard back to the text (a new area id takes it)
    /// or to the find field.
    focus_turn: u64,
}

impl EditWindow {
    fn new(shared: SharedRef, tabs: Vec<DocTab>) -> EditWindow {
        let (desktop, stamp) = desktop_text();
        let front = tabs.first().map_or(0, |t| t.id);
        let mut w = EditWindow {
            shared,
            ending: None,
            tabs,
            front,
            me: None,
            waker: None,
            desktop,
            stamp,
            zoom: 1.0,
            prefs_open: Rc::new(Cell::new(false)),
            asking: None,
            closing: Vec::new(),
            closing_window: false,
            notice: None,
            picking: None,
            find: None,
            focus_turn: 0,
            goto: None,
            token: String::new(),
            keep_pending: false,
            watching: false,
            page_drawn: Cell::new(None),
        };
        if w.tabs.is_empty() {
            let t = w.untitled_tab();
            w.front = t.id;
            w.tabs.push(t);
        }
        w.apply_options();
        w
    }

    /// Joins the window list once both its number and its waker are known
    /// (the toolkit gives the number first); the first window also takes
    /// what other starts hand over.
    fn register(&mut self) {
        let (Some(id), Some(w)) = (self.me, self.waker.clone()) else { return };
        let mut s = self.shared.borrow_mut();
        if s.windows.iter().any(|(i, _)| *i == id) {
            return;
        }
        s.windows.push((id, w.clone()));
        let mut l = s.listener.lock().unwrap_or_else(|e| e.into_inner());
        if l.is_none() {
            *l = Some(w.clone());
        }
        // Woken once, to start watching its files (and open the windows kept).
        w.wake();
    }
}

impl Program for EditWindow {
    type Message = Msg;

    fn title(&self) -> String {
        match self.front_tab() {
            Some(t) if t.modified() => format!("{} •", t.name()),
            Some(t) => t.name(),
            None => "Edit".into(),
        }
    }

    fn app_id(&self) -> String {
        APP_ID.into()
    }

    /// Light under App default, as a page of paper.
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
        let tabs = self
            .tabs
            .iter()
            .map(|t| BarTab { id: t.id, title: t.name(), closable: true, alert: false, edited: t.modified() })
            .collect();
        // The tab in front is the page's color, as the terminal's: joined to
        // the text, and the server knows the page's style, so text scrolled
        // under a bar of the other style shows through inverted (readable)
        // rather than all but hidden by the frost.
        let page = self.page_drawn.get().unwrap_or_else(|| {
            let p = self.theme().page;
            (p.r as u32) << 16 | (p.g as u32) << 8 | p.b as u32
        });
        Some(Bar::Tabs {
            tabs,
            front: self.front_tab().map_or(0, |t| t.id),
            plus: true,
            accent: Some(page),
            menu: Vec::new(),
        })
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        self.on_view(t)
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        match m {
            Msg::Changed => {
                // One of our own lines in the text's right-click menu.
                if let Some(id) = self.doc().and_then(|d| d.borrow_mut().take_command()) {
                    self.run(id, window);
                }
                if self.find.is_some() {
                    self.refind();
                }
            }
            Msg::FindTyped(n) => {
                if let Some(f) = self.find.as_mut() {
                    f.needle = n;
                    f.found_in = (0, u64::MAX);
                    f.replaced = None;
                }
                self.find_from_origin();
            }
            Msg::WithTyped(w) => {
                if let Some(f) = self.find.as_mut() {
                    f.with = w;
                }
            }
            Msg::FindNext | Msg::FindPrev => {
                if let Some(f) = self.find.as_mut() {
                    f.replaced = None;
                }
                self.find_step(matches!(m, Msg::FindPrev));
            }
            Msg::FindCase | Msg::FindWhole => {
                if let Some(f) = self.find.as_mut() {
                    if matches!(m, Msg::FindCase) {
                        f.case = !f.case;
                    } else {
                        f.whole = !f.whole;
                    }
                    f.found_in = (0, u64::MAX);
                }
                self.find_from_origin();
            }
            Msg::FindReplace => {
                if let Some(f) = self.find.as_mut() {
                    f.replace = !f.replace;
                }
            }
            Msg::FindClose => self.close_find(),
            Msg::Reload => {
                let i = self.front_index();
                self.reload(i);
            }
            Msg::KeepMine => {
                let i = self.front_index();
                if let Some(t) = self.tabs.get_mut(i) {
                    t.changed_on_disk = false;
                    t.disk = t.path.as_deref().and_then(stamp);
                }
            }
            Msg::GotoTyped(g) => self.goto = Some(g),
            Msg::GotoGo => {
                let g = self.goto.clone().unwrap_or_default();
                if self.go_to(&g) {
                    self.goto = None;
                    self.focus_turn += 1;
                }
            }
            Msg::GotoClose => {
                self.goto = None;
                self.focus_turn += 1;
            }
            Msg::ReplaceOne => {
                // The match chosen is replaced; then on to the next.
                if self.chosen_match()
                    && let (Some(doc), Some(f)) = (self.doc(), self.find.as_ref())
                {
                    let with = f.with.clone();
                    doc.borrow_mut().insert(&with);
                }
                self.find_step(false);
            }
            Msg::ReplaceAll => {
                self.refind();
                if let (Some(doc), Some(f)) = (self.doc(), self.find.as_mut()) {
                    let n = doc.borrow_mut().replace_all(&f.matches, &f.with.clone());
                    f.replaced = Some(n);
                    f.found_in = (0, u64::MAX);
                }
                self.refind();
            }
            Msg::DismissNotice => self.notice = None,
            Msg::ToggleSpaces => {
                let i = self.front_index();
                if let Some(t) = self.tabs.get_mut(i) {
                    let now = t.doc.borrow().options().spaces;
                    t.spaces = Some(!now);
                }
                self.apply_options();
            }
            Msg::ToggleEnding => {
                if let Some(d) = self.doc() {
                    let mut d = d.borrow_mut();
                    let e = if d.buffer().ending() == Ending::Lf { Ending::CrLf } else { Ending::Lf };
                    d.set_ending(e);
                }
            }
        }
        self.keep_soon(window);
        window.redraw();
    }

    /// Keys the text area left: the app's own.
    fn input(&mut self, input: &Input, window: &mut Window) -> bool {
        self.on_input(input, window)
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        self.on_commands()
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        self.run(id, window);
        self.keep_soon(window);
        window.redraw();
    }

    /// Files dropped here open in tabs (files milestone 1, step 8).
    fn drag_over(&mut self, _x: f32, _y: f32, kinds: &[String], _ctrl: bool) -> u8 {
        if kinds.iter().any(|k| k == "text/uri-list") { vergence_ui::DROP_COPY } else { vergence_ui::DROP_NONE }
    }

    fn dropped(&mut self, _x: f32, _y: f32, _action: u8, items: &[(String, Vec<u8>)], window: &mut Window) {
        let paths: Vec<PathBuf> = items
            .iter()
            .find(|(k, _)| k == "text/uri-list")
            .map(|(_, v)| {
                String::from_utf8_lossy(v).lines().filter_map(|l| vergence_types::path_of_link(l.trim())).collect()
            })
            .unwrap_or_default();
        if !paths.is_empty() {
            self.open_files(&paths);
            window.redraw();
        }
    }

    fn timer(&mut self, id: u64, window: &mut Window) -> bool {
        match id {
            KEEP_TIMER => {
                self.keep_now();
                false
            }
            WATCH_TIMER => {
                window.wake_at(WATCH_TIMER, std::time::Instant::now() + WATCH_EVERY);
                let changed = self.check_disk();
                if changed {
                    self.keep_soon(window);
                }
                changed
            }
            _ => false,
        }
    }

    /// Its token from the last run, so window memory puts it back; "new": ask for one.
    fn token(&self) -> String {
        if self.token.is_empty() { "new".into() } else { self.token.clone() }
    }

    fn token_given(&mut self, token: &str) {
        self.token = token.to_string();
        self.keep_now();
    }

    fn tab(&mut self, event: TabEvent, window: &mut Window) {
        self.on_tab(event, window)
    }

    fn close_asked(&mut self, window: &mut Window) -> bool {
        if !self.tabs.iter().any(DocTab::modified) {
            return true;
        }
        self.closing = self.tabs.iter().map(|t| t.id).collect();
        self.closing_window = true;
        self.go_on_closing(window);
        false
    }

    fn opened(&mut self, id: u32) {
        self.me = Some(id);
        self.register();
    }

    fn session_ending(&mut self) {
        self.ending = Some(std::time::Instant::now());
    }

    fn closed(&mut self) {
        let mut s = self.shared.borrow_mut();
        s.windows.retain(|(id, _)| Some(*id) != self.me);
        // Closed by the user (its questions answered): it is not kept.
        // Closed for an exit: kept for the next session (bus v33).
        if !vergence_widgets::session::ending(self.ending) {
            s.snaps.retain(|(id, _)| Some(*id) != self.me);
        }
        let all: Vec<keep::WindowSnap> = s.snaps.iter().map(|(_, w)| w.clone()).collect();
        keep::save(&all);
        // Hand-overs go to another window now.
        let next = s.windows.first().map(|(_, w)| w.clone());
        *s.listener.lock().unwrap_or_else(|e| e.into_inner()) = next.clone();
        if let Some(w) = next {
            w.wake();
        }
    }

    fn waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
        self.register();
    }

    fn woken(&mut self, window: &mut Window) -> bool {
        let mut changed = self.reread();
        if !self.watching {
            self.watching = true;
            window.wake_at(WATCH_TIMER, std::time::Instant::now() + WATCH_EVERY);
            self.keep_soon(window);
        }
        // The other windows kept from the last run.
        let pending = std::mem::take(&mut self.shared.borrow_mut().pending);
        for snap in pending {
            let w = EditWindow::from_snap(self.shared.clone(), &snap);
            window.open_window(vergence_widgets::window(w));
        }
        // The file dialog answered.
        let answer = self.picking.as_ref().and_then(|p| p.answer.borrow_mut().take());
        if let Some(paths) = answer
            && let Some(p) = self.picking.take()
        {
            self.picked(p.what, paths, window);
            changed = true;
        }
        // A question answered.
        if let Some(a) = self.asking.as_ref().and_then(|q| q.answer.take()) {
            self.answered(a, window);
            changed = true;
        } else if self.asking.as_ref().is_some_and(|q| Rc::strong_count(&q.answer) == 1) {
            // The question was closed without an answer: as Cancel.
            self.answered(Answer::Cancel, window);
            changed = true;
        }
        // Tabs joined to this window.
        let mine: Vec<(u32, DocTab)> = {
            let mut s = self.shared.borrow_mut();
            let (mine, rest): (Vec<_>, Vec<_>) =
                std::mem::take(&mut s.transfers).into_iter().partition(|(into, _, _)| Some(*into) == self.me);
            s.transfers = rest;
            mine.into_iter().map(|(_, to, t)| (to, t)).collect()
        };
        for (to, t) in mine {
            self.front = t.id;
            self.tabs.insert((to as usize).min(self.tabs.len()), t);
            changed = true;
        }
        if changed {
            self.apply_options();
        }
        // Files other starts handed over, if this window takes them.
        let listening = {
            let s = self.shared.borrow();
            s.windows.first().map(|(id, _)| Some(*id) == self.me).unwrap_or(false)
        };
        if listening {
            let requests: Vec<Vec<PathBuf>> =
                std::mem::take(&mut *self.shared.borrow().inbox.lock().unwrap_or_else(|e| e.into_inner()));
            for files in requests {
                if files.is_empty() {
                    window.open_window(vergence_widgets::window(EditWindow::new(self.shared.clone(), Vec::new())));
                } else {
                    self.open_files(&files);
                    // Handed over by a start the person just made: forward (the server judges).
                    window.activate();
                    changed = true;
                }
            }
        }
        changed
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { min: (360.0, 240.0), ..Default::default() }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("vergence-edit {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let paths: Vec<PathBuf> = args.iter().filter(|a| !a.starts_with("--")).map(|a| cwd.join(a)).collect();
    // Another editor running takes the files.
    if hand::hand_over(&paths) {
        return;
    }
    let inbox = Arc::new(Mutex::new(Vec::new()));
    let listener: Arc<Mutex<Option<Waker>>> = Arc::new(Mutex::new(None));
    let wake = listener.clone();
    hand::listen(inbox.clone(), move || {
        if let Some(w) = wake.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            w.wake();
        }
    });
    let shared = Rc::new(RefCell::new(Shared {
        windows: Vec::new(),
        transfers: Vec::new(),
        inbox,
        listener,
        next_tab: 0,
        snaps: Vec::new(),
        pending: Vec::new(),
    }));
    // The windows kept from the last run come back, the first here and the
    // rest once it is open; files named on the command line join the first.
    let mut kept = keep::load();
    let mut w = if kept.is_empty() {
        EditWindow::new(shared.clone(), Vec::new())
    } else {
        let first = kept.remove(0);
        shared.borrow_mut().pending = kept;
        EditWindow::from_snap(shared.clone(), &first)
    };
    w.open_files(&paths);
    if let Err(e) = vergence_widgets::run(w, (760, 560)) {
        eprintln!("vergence-edit: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests;
