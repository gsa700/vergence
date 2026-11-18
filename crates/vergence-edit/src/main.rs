//! Vergence's text editor (editor milestone 1, docs/editor-milestone-1.md;
//! Dave: "a native text editor for Vergence. Like TextEdit on Mac or GNOME
//! Edit. Autocomplete and line numbers required."). Plain text for now
//! ("other formats later md rtf etc"), on the toolkit's text area.
//!
//! - **Windows and tabs**: one process for all its windows (hand.rs); a
//!   document a tab, in the bar the server draws; tabs dragged between
//!   windows or out into their own. A tab shows its file's name, with a dot
//!   while it has changes not saved.
//! - **Files** (files.rs): UTF-8, line endings and a final newline kept as
//!   found; saving writes a new file and renames it over the old, so a
//!   crash never leaves half a file. Text that is not UTF-8 opens read-only.
//! - **Closing with changes asks first**: Save, Don't save, Cancel.
//! - **Its settings** (Ctrl+,): font, text size, line numbers, wrapping,
//!   tabs, the current line's shade, its look; desktop.toml `[editor]`.

mod files;
mod hand;
mod keep;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use vergence_ui::{AppCommand as C, Bar, BarStyle, BarTab, Role, TabEvent};
use vergence_widgets::edit::Ending;
use vergence_widgets::prefs::{Choice, Control, Part, Prefs, Row, Rows};
use vergence_widgets::{Align, AreaOptions, ButtonLook, Element, Input, Length, Padding, Program, TextDoc, Theme, Waker, Window, button, column, field, label, panel, row, rule, space, text_area};

const APP_ID: &str = "vergence-edit";
const SECTION: &str = "[editor]";

/// The text size's range for now, as the terminal's.
const ZOOM_MIN: f32 = 0.5;
const ZOOM_MAX: f32 = 3.0;

mod cmd {
    pub const NEW_WINDOW: u32 = 1;
    pub const NEW_TAB: u32 = 2;
    pub const OPEN: u32 = 3;
    pub const SAVE: u32 = 4;
    pub const SAVE_AS: u32 = 5;
    pub const CLOSE_TAB: u32 = 6;
    pub const UNDO: u32 = 10;
    pub const REDO: u32 = 11;
    pub const CUT: u32 = 12;
    pub const COPY: u32 = 13;
    pub const PASTE: u32 = 14;
    pub const SELECT_ALL: u32 = 15;
    pub const FIND: u32 = 16;
    pub const FIND_NEXT: u32 = 17;
    pub const FIND_PREV: u32 = 18;
    pub const REPLACE: u32 = 19;
    pub const ZOOM_IN: u32 = 20;
    pub const ZOOM_OUT: u32 = 21;
    pub const ZOOM_RESET: u32 = 22;
    pub const LINE_NUMBERS: u32 = 23;
    pub const WRAP: u32 = 24;
    pub const NEXT_TAB: u32 = 30;
    pub const PREV_TAB: u32 = 31;
    pub const GO_TO_LINE: u32 = 32;
    pub const SETTINGS: u32 = 40;
    pub const ABOUT: u32 = 41;
}

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

/// A file's modification time and length: changed on disk if these differ.
fn stamp(p: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

type SharedRef = Rc<RefCell<Shared>>;

/// One document in a tab.
struct DocTab {
    id: u32,
    doc: TextDoc,
    path: Option<PathBuf>,
    /// "Untitled", "Untitled 2", … for a document never saved.
    untitled: u32,
    read_only: Option<String>,
    bom: bool,
    /// Tab inserts spaces (from the file's own indenting, or the status line).
    spaces: Option<bool>,
    /// The file as last read or written (step 8: changes on disk are noticed).
    disk: Option<(SystemTime, u64)>,
    /// The file changed on disk while this tab had changes of its own.
    changed_on_disk: bool,
}

impl DocTab {
    fn name(&self) -> String {
        match &self.path {
            Some(p) => files::name(p),
            None if self.untitled <= 1 => "Untitled".into(),
            None => format!("Untitled {}", self.untitled),
        }
    }

    fn modified(&self) -> bool {
        self.doc.borrow().modified()
    }
}

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

/// The most matches lit and counted.
const MAX_MATCHES: usize = 10_000;

/// The find bar (step 5): what is looked for, with what it is replaced,
/// how, and what was found in the document in front.
#[derive(Default)]
struct Find {
    needle: String,
    with: String,
    case: bool,
    whole: bool,
    replace: bool,
    matches: Vec<(vergence_widgets::edit::Pos, vergence_widgets::edit::Pos)>,
    /// The document's revision and tab the matches were found in.
    found_in: (u32, u64),
    /// Where the caret was when the bar opened: typing searches from there.
    origin: vergence_widgets::edit::Pos,
    /// After Replace all: how many.
    replaced: Option<usize>,
}

/// An answer to "Save changes?".
#[derive(Debug, Clone, Copy, PartialEq)]
enum Answer {
    Save,
    Discard,
    Cancel,
}

/// What a file dialog was opened for.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Picking {
    Open,
    /// Save tab `tab` under a new name; then close it (the close question's Save).
    SaveAs { tab: u32, then_close: bool },
}

/// A file dialog open over the window: what for, and where its answer lands.
struct Pick {
    what: Picking,
    answer: Rc<RefCell<Option<Vec<PathBuf>>>>,
}

/// A question open over the window, and what it leads to.
struct Asking {
    tab: u32,
    answer: Rc<Cell<Option<Answer>>>,
}

struct EditWindow {
    shared: SharedRef,
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

/// ~/.config/vergence/desktop.toml and when it last changed.
fn desktop_text() -> (String, Option<SystemTime>) {
    match vergence_widgets::prefs::desktop_path() {
        Some(p) => (std::fs::read_to_string(&p).unwrap_or_default(), std::fs::metadata(&p).and_then(|m| m.modified()).ok()),
        None => (String::new(), None),
    }
}

impl EditWindow {
    fn new(shared: SharedRef, tabs: Vec<DocTab>) -> EditWindow {
        let (desktop, stamp) = desktop_text();
        let front = tabs.first().map_or(0, |t| t.id);
        let mut w = EditWindow { shared, tabs, front, me: None, waker: None, desktop, stamp, zoom: 1.0, prefs_open: Rc::new(Cell::new(false)), asking: None, closing: Vec::new(), closing_window: false, notice: None, picking: None, find: None, focus_turn: 0, goto: None, token: String::new(), keep_pending: false, watching: false, page_drawn: Cell::new(None) };
        if w.tabs.is_empty() {
            let t = w.untitled_tab();
            w.front = t.id;
            w.tabs.push(t);
        }
        w.apply_options();
        w
    }

    fn setting(&self, key: &str) -> Option<String> {
        vergence_ui::table_value(&self.desktop, SECTION, key)
    }

    fn flag(&self, key: &str, default: bool) -> bool {
        match self.setting(key).as_deref() {
            Some("true") => true,
            Some("false") => false,
            _ => default,
        }
    }

    /// How a document's area shows and edits, from the settings.
    fn options_for(&self, t: &DocTab) -> AreaOptions {
        let size = self.setting("text-size").and_then(|s| s.parse::<f32>().ok()).filter(|s| (6.0..=72.0).contains(s));
        let base = size.unwrap_or_else(|| self.theme().text_size);
        AreaOptions {
            font: self.setting("font").filter(|f| !f.is_empty()).unwrap_or_else(|| "monospace".into()),
            size: Some((base * self.zoom).clamp(6.0, 96.0)),
            line_numbers: self.flag("line-numbers", true),
            wrap: self.flag("wrap", true),
            tab_width: self.setting("tab-width").and_then(|s| s.parse().ok()).unwrap_or(4),
            spaces: t.spaces.unwrap_or_else(|| self.flag("spaces", true)),
            current_line: self.flag("current-line", true),
            editable: t.read_only.is_none(),
            // The right-click menu as full as the terminal's (Dave's list):
            // the editor's own lines after the text's.
            commands: vec![("New Tab".into(), cmd::NEW_TAB, "Ctrl+T"), ("New Window".into(), cmd::NEW_WINDOW, "Ctrl+N"), ("Find…".into(), cmd::FIND, "Ctrl+F")],
            complete: self.flag("complete", true),
            // Its own choice, else the desktop's ([text] spellcheck), on by default.
            spell: match self.setting("spellcheck").as_deref() {
                Some("true") => true,
                Some("false") => false,
                _ => vergence_ui::table_value(&self.desktop, "[text]", "spellcheck").as_deref() != Some("false"),
            },
        }
    }

    fn apply_options(&mut self) {
        for i in 0..self.tabs.len() {
            let o = self.options_for(&self.tabs[i]);
            self.tabs[i].doc.borrow_mut().set_options(o);
            // Completion also offers the words of the window's other documents.
            let others: Vec<vergence_widgets::WeakDoc> = self.tabs.iter().enumerate().filter(|(k, _)| *k != i).map(|(_, t)| t.doc.downgrade()).collect();
            self.tabs[i].doc.borrow_mut().set_words(Some(other_words(others)));
        }
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

    fn new_tab_id(&self) -> u32 {
        let mut s = self.shared.borrow_mut();
        s.next_tab += 1;
        s.next_tab
    }

    fn untitled_tab(&self) -> DocTab {
        // The lowest number no open Untitled here has.
        let n = (1..).find(|n| !self.tabs.iter().any(|t| t.path.is_none() && t.untitled == *n)).unwrap_or(1);
        DocTab { id: self.new_tab_id(), doc: TextDoc::new(""), path: None, untitled: n, read_only: None, bom: false, spaces: None, disk: None, changed_on_disk: false }
    }

    /// A tab for `path`, read in; None (and a notice) if it cannot be.
    fn file_tab(&mut self, path: &Path) -> Option<DocTab> {
        match files::load(path) {
            Ok(l) => {
                let doc = TextDoc::new(&l.text);
                doc.borrow_mut().mark_saved();
                let spaces = doc.borrow().buffer().uses_tabs().map(|tabs| !tabs);
                Some(DocTab { id: self.new_tab_id(), doc, path: Some(path.to_path_buf()), untitled: 0, read_only: l.read_only, bom: l.bom, spaces, disk: stamp(path), changed_on_disk: false })
            }
            Err(e) => {
                self.notice = Some(e);
                None
            }
        }
    }

    /// Opens files in tabs here (one already open comes to the front instead).
    fn open_files(&mut self, paths: &[PathBuf]) {
        for p in paths {
            if let Some(t) = self.tabs.iter().find(|t| t.path.as_deref() == Some(p.as_path())) {
                self.front = t.id;
                continue;
            }
            if let Some(t) = self.file_tab(p) {
                // An untouched empty Untitled gives way to the first file.
                let lone = self.tabs.len() == 1 && self.tabs[0].path.is_none() && !self.tabs[0].modified() && self.tabs[0].doc.borrow().buffer().bytes() == 0;
                if lone {
                    self.tabs.clear();
                }
                self.front = t.id;
                self.tabs.push(t);
            }
        }
        self.apply_options();
    }

    fn front_tab(&self) -> Option<&DocTab> {
        self.tabs.iter().find(|t| t.id == self.front).or(self.tabs.first())
    }

    fn front_index(&self) -> usize {
        self.tabs.iter().position(|t| t.id == self.front).unwrap_or(0)
    }

    fn doc(&self) -> Option<TextDoc> {
        self.front_tab().map(|t| t.doc.clone())
    }

    /// Saves tab `id`. False if it could not be (a notice says why).
    fn save(&mut self, id: u32) -> bool {
        let Some(t) = self.tabs.iter().find(|t| t.id == id) else { return false };
        if t.read_only.is_some() {
            self.notice = Some(format!("{} is open read-only.", t.name()));
            return false;
        }
        let Some(path) = t.path.clone() else { return false };
        let text = t.doc.borrow().text();
        match files::save(&path, &text, t.bom) {
            Ok(()) => {
                t.doc.borrow_mut().mark_saved();
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                    t.disk = stamp(&path);
                    t.changed_on_disk = false;
                }
                true
            }
            Err(e) => {
                self.notice = Some(e);
                false
            }
        }
    }

    /// What this window keeps for next time (keep.rs).
    fn snap(&self) -> keep::WindowSnap {
        let tabs = self
            .tabs
            .iter()
            .filter(|t| t.path.is_some() || t.modified())
            .map(|t| {
                let d = t.doc.borrow();
                let c = d.caret();
                keep::TabSnap { path: t.path.clone(), untitled: t.untitled, text: t.modified().then(|| d.text()), caret: (c.line, c.col) }
            })
            .collect::<Vec<_>>();
        let front = self.tabs.iter().filter(|t| t.path.is_some() || t.modified()).position(|t| t.id == self.front).unwrap_or(0);
        keep::WindowSnap { token: self.token.clone(), front, tabs }
    }

    /// Writes what every window keeps, this one's as it is now.
    fn keep_now(&mut self) {
        self.keep_pending = false;
        let Some(me) = self.me else { return };
        let snap = self.snap();
        let mut s = self.shared.borrow_mut();
        match s.snaps.iter_mut().find(|(id, _)| *id == me) {
            Some(slot) => slot.1 = snap,
            None => s.snaps.push((me, snap)),
        }
        let all: Vec<keep::WindowSnap> = s.snaps.iter().map(|(_, w)| w.clone()).filter(|w| !w.tabs.is_empty() || !w.token.is_empty()).collect();
        keep::save(&all);
    }

    /// What is kept is written a second from now (once, however many changes).
    fn keep_soon(&mut self, window: &mut Window) {
        if !self.keep_pending {
            self.keep_pending = true;
            window.wake_at(KEEP_TIMER, std::time::Instant::now() + KEEP_AFTER);
        }
    }

    /// A window as it was kept: its token, its files, its unsaved texts.
    fn from_snap(shared: SharedRef, snap: &keep::WindowSnap) -> EditWindow {
        let mut w = EditWindow::new(shared, Vec::new());
        w.tabs.clear();
        w.token = snap.token.clone();
        for t in &snap.tabs {
            let tab = match (&t.text, &t.path) {
                // Unsaved work: its text, marked as not saved.
                (Some(text), path) => {
                    let doc = TextDoc::new(text);
                    doc.borrow_mut().mark_unsaved();
                    let spaces = doc.borrow().buffer().uses_tabs().map(|tabs| !tabs);
                    Some(DocTab { id: w.new_tab_id(), doc, path: path.clone(), untitled: t.untitled, read_only: None, bom: false, spaces, disk: path.as_deref().and_then(stamp), changed_on_disk: false })
                }
                (None, Some(p)) => w.file_tab(p),
                (None, None) => None,
            };
            if let Some(tab) = tab {
                let caret = vergence_widgets::edit::Pos::new(t.caret.0, t.caret.1);
                tab.doc.borrow_mut().set_selection(caret, caret);
                w.tabs.push(tab);
            }
        }
        if w.tabs.is_empty() {
            let t = w.untitled_tab();
            w.tabs.push(t);
        }
        w.front = w.tabs.get(snap.front).or(w.tabs.first()).map_or(0, |t| t.id);
        w.apply_options();
        w
    }

    /// Open files changed on disk: one without changes of its own is read
    /// again (the caret kept where it was); one with them asks (a strip).
    fn check_disk(&mut self) -> bool {
        let mut changed = false;
        for i in 0..self.tabs.len() {
            let Some(p) = self.tabs[i].path.clone() else { continue };
            let now = stamp(&p);
            if now.is_none() || now == self.tabs[i].disk || self.tabs[i].changed_on_disk {
                continue;
            }
            if self.tabs[i].modified() {
                self.tabs[i].changed_on_disk = true;
            } else {
                self.reload(i);
            }
            changed = true;
        }
        changed
    }

    /// Reads tab `i`'s file again, the caret kept on its line.
    fn reload(&mut self, i: usize) {
        let Some(p) = self.tabs[i].path.clone() else { return };
        match files::load(&p) {
            Ok(l) => {
                let t = &mut self.tabs[i];
                let mut d = t.doc.borrow_mut();
                let c = d.caret();
                d.set_text(&l.text);
                d.mark_saved();
                d.set_selection(c, c);
                drop(d);
                t.bom = l.bom;
                t.read_only = l.read_only;
                t.disk = stamp(&p);
                t.changed_on_disk = false;
            }
            Err(e) => self.notice = Some(e),
        }
    }

    /// Go to line: "12", or "12:5" for a column too.
    fn go_to(&mut self, what: &str) -> bool {
        let Some(doc) = self.doc() else { return false };
        let mut parts = what.trim().splitn(2, [':', ',', ' ']);
        let Some(line) = parts.next().and_then(|l| l.trim().parse::<usize>().ok()).filter(|l| *l > 0) else { return false };
        let col = parts.next().and_then(|c| c.trim().parse::<usize>().ok()).unwrap_or(1).max(1);
        let mut d = doc.borrow_mut();
        let line = (line - 1).min(d.buffer().line_count() - 1);
        // The column in characters, as the status line counts them.
        let text = d.buffer().line(line).to_string();
        let byte = text.char_indices().nth(col - 1).map_or(text.len(), |(b, _)| b);
        let p = vergence_widgets::edit::Pos::new(line, byte);
        d.set_selection(p, p);
        true
    }

    /// Opens the find bar (with Replace, if `replace`): the selection, if on
    /// one line, is what it looks for.
    fn open_find(&mut self, replace: bool) {
        let Some(doc) = self.doc() else { return };
        let (sel, (a, _)) = { let d = doc.borrow(); (d.selected_text(), d.selection()) };
        let f = self.find.get_or_insert_with(Find::default);
        if !sel.is_empty() && !sel.contains('\n') {
            f.needle = sel;
        }
        f.replace |= replace;
        f.origin = a;
        f.found_in = (0, u64::MAX);
        self.focus_turn += 1;
        self.refind();
    }

    fn close_find(&mut self) {
        if self.find.take().is_some() {
            for t in &self.tabs {
                t.doc.borrow_mut().set_marks(Vec::new());
            }
            self.focus_turn += 1;
        }
    }

    /// Finds again in the document in front, if it or what is looked for changed.
    fn refind(&mut self) {
        let Some(t) = self.front_tab() else { return };
        let (id, doc) = (t.id, t.doc.clone());
        let Some(f) = self.find.as_mut() else { return };
        let rev = doc.borrow().revision();
        if f.found_in == (id, rev) {
            return;
        }
        f.matches = doc.borrow().find_all(&f.needle, f.case, f.whole, MAX_MATCHES);
        f.found_in = (id, rev);
        doc.borrow_mut().set_marks(f.matches.clone());
    }

    /// Chooses the next match after the selection (or before, `back`), round the ends.
    fn find_step(&mut self, back: bool) {
        self.refind();
        let Some(doc) = self.doc() else { return };
        let Some(f) = self.find.as_ref() else { return };
        if f.matches.is_empty() {
            return;
        }
        let (a, b) = doc.borrow().selection();
        let k = if back {
            f.matches.iter().rposition(|m| m.0 < a).unwrap_or(f.matches.len() - 1)
        } else {
            f.matches.iter().position(|m| m.0 >= b && (m.0, m.1) != (a, b)).unwrap_or(0)
        };
        let (ma, mb) = f.matches[k];
        doc.borrow_mut().set_selection(ma, mb);
    }

    /// Typing in the find field: the first match from where the bar opened.
    fn find_from_origin(&mut self) {
        self.refind();
        let Some(doc) = self.doc() else { return };
        let Some(f) = self.find.as_ref() else { return };
        let hit = f.matches.iter().find(|m| m.0 >= f.origin).or(f.matches.first()).copied();
        match hit {
            Some((a, b)) => doc.borrow_mut().set_selection(a, b),
            None => {
                let o = f.origin;
                doc.borrow_mut().set_selection(o, o);
            }
        }
    }

    /// The match chosen, if the selection is one.
    fn chosen_match(&self) -> bool {
        let (Some(doc), Some(f)) = (self.doc(), self.find.as_ref()) else { return false };
        let sel = doc.borrow().selection();
        f.matches.iter().any(|m| (m.0, m.1) == sel)
    }

    /// Opens the file dialog: Open, or Save as for tab `tab`.
    fn pick(&mut self, what: Picking, window: &mut Window) {
        if self.picking.is_some() {
            return;
        }
        let answer: Rc<RefCell<Option<Vec<PathBuf>>>> = Rc::new(RefCell::new(None));
        let (slot, waker) = (answer.clone(), self.waker.clone());
        let done = move |paths: Vec<PathBuf>| {
            *slot.borrow_mut() = Some(paths);
            if let Some(w) = &waker {
                w.wake();
            }
        };
        let theme = self.theme();
        // The dialog starts in the document's own folder, if it has one.
        let here = match what {
            Picking::Open => self.front_tab(),
            Picking::SaveAs { tab, .. } => self.tabs.iter().find(|t| t.id == tab),
        }
        .and_then(|t| t.path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)));
        let dialog = match what {
            Picking::Open => vergence_widgets::filedialog::FileDialog::open(APP_ID, "Open", theme, done).filters(vec![
                vergence_widgets::filedialog::Filter::new("All files", &[]),
                // Every kind of text: scripts, Markdown, TOML, source code (files milestone 1).
                vergence_widgets::filedialog::Filter::of_types("Text files", &["text/plain"]),
            ]),
            Picking::SaveAs { tab, .. } => {
                let name = self.tabs.iter().find(|t| t.id == tab).map_or_else(|| "Untitled.txt".into(), suggested_name);
                vergence_widgets::filedialog::FileDialog::save(APP_ID, "Save as", &name, theme, done)
            }
        };
        let dialog = match here {
            Some(d) => dialog.folder(&d),
            None => dialog,
        };
        self.picking = Some(Pick { what, answer });
        window.open_window(vergence_widgets::window(dialog));
    }

    /// The file dialog answered (nothing: cancelled).
    fn picked(&mut self, what: Picking, paths: Vec<PathBuf>, window: &mut Window) {
        match what {
            Picking::Open => self.open_files(&paths),
            Picking::SaveAs { tab, then_close } => {
                let Some(p) = paths.into_iter().next() else {
                    // Cancelled: a close waiting on it stops.
                    if then_close {
                        self.closing.clear();
                        self.closing_window = false;
                    }
                    return;
                };
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                    t.path = Some(p);
                    t.read_only = None;
                }
                if self.save(tab) && then_close {
                    self.closing.retain(|t| *t != tab);
                    self.remove_tab(tab);
                    self.go_on_closing(window);
                } else if then_close {
                    self.closing.clear();
                    self.closing_window = false;
                }
                self.apply_options();
            }
        }
    }

    /// Saves tab `id`, asking where first if it has never been saved.
    fn save_or_ask(&mut self, id: u32, window: &mut Window) {
        let untitled = self.tabs.iter().any(|t| t.id == id && t.path.is_none());
        if untitled {
            self.pick(Picking::SaveAs { tab: id, then_close: false }, window);
        } else {
            self.save(id);
        }
    }

    /// Closes tab `id` (asking first if it has changes); the window goes
    /// with its last tab.
    fn close_tab(&mut self, id: u32, window: &mut Window) {
        if !self.closing.contains(&id) {
            self.closing.push(id);
        }
        self.go_on_closing(window);
    }

    /// Works through the tabs to close: those without changes go at once;
    /// the first with changes is asked about.
    fn go_on_closing(&mut self, window: &mut Window) {
        if self.asking.is_some() {
            return;
        }
        while let Some(&id) = self.closing.first() {
            match self.tabs.iter().find(|t| t.id == id) {
                Some(t) if t.modified() => {
                    self.front = id;
                    let answer = Rc::new(Cell::new(None));
                    let q = SaveQuestion { name: t.name(), answer: answer.clone(), waker: self.waker.clone(), theme: self.theme() };
                    self.asking = Some(Asking { tab: id, answer });
                    window.open_window(vergence_widgets::window(q));
                    return;
                }
                _ => {
                    self.closing.remove(0);
                    self.remove_tab(id);
                }
            }
        }
        if self.tabs.is_empty() || self.closing_window {
            self.closing_window = false;
            window.close();
        }
    }

    fn remove_tab(&mut self, id: u32) -> Option<DocTab> {
        let i = self.tabs.iter().position(|t| t.id == id)?;
        let t = self.tabs.remove(i);
        if self.front == id
            && let Some(n) = self.tabs.get(i.min(self.tabs.len().saturating_sub(1)))
        {
            self.front = n.id;
        }
        Some(t)
    }

    /// The question about a tab was answered.
    fn answered(&mut self, a: Answer, window: &mut Window) {
        let Some(asking) = self.asking.take() else { return };
        match a {
            Answer::Cancel => {
                self.closing.clear();
                self.closing_window = false;
            }
            // Never saved: where to, first; the closing goes on after.
            Answer::Save if self.tabs.iter().any(|t| t.id == asking.tab && t.path.is_none()) => {
                self.pick(Picking::SaveAs { tab: asking.tab, then_close: true }, window);
            }
            Answer::Save if !self.save(asking.tab) => {
                // Not saved: stop, and say why.
                self.closing.clear();
                self.closing_window = false;
            }
            _ => {
                self.closing.retain(|t| *t != asking.tab);
                self.remove_tab(asking.tab);
                self.go_on_closing(window);
            }
        }
    }

    fn open_settings(&mut self, window: &mut Window) {
        if self.prefs_open.get() {
            return;
        }
        let waker = self.waker.clone();
        let prefs = Prefs::new("Edit", APP_ID, SECTION, false, Box::new(EditRows::new()), self.prefs_open.clone()).on_write(move || {
            if let Some(w) = &waker {
                w.wake();
            }
        });
        window.open_window(vergence_widgets::window(prefs));
    }

    /// desktop.toml again, if it changed.
    fn reread(&mut self) -> bool {
        let (text, stamp) = desktop_text();
        if stamp == self.stamp {
            return false;
        }
        (self.desktop, self.stamp) = (text, stamp);
        self.apply_options();
        true
    }

    fn write_key(&mut self, key: &str, value: Option<&str>) {
        let Some(path) = vergence_widgets::prefs::desktop_path() else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let new = vergence_widgets::prefs::set_key(&text, SECTION, key, value);
        if new != text {
            let beside = path.with_extension("toml.new");
            if std::fs::write(&beside, &new).and_then(|_| std::fs::rename(&beside, &path)).is_ok() {
                self.reread();
            }
        }
    }

    fn toggle_setting(&mut self, key: &str, default: bool) {
        let now = self.flag(key, default);
        // Back to the default is the key taken away.
        let value = if !now == default { None } else { Some(if now { "false" } else { "true" }) };
        self.write_key(key, value);
    }

    fn set_zoom(&mut self, z: f32) {
        self.zoom = z.clamp(ZOOM_MIN, ZOOM_MAX);
        self.apply_options();
    }

    fn step_tab(&mut self, by: isize) {
        let n = self.tabs.len() as isize;
        if n > 1 {
            let i = (self.front_index() as isize + by).rem_euclid(n) as usize;
            self.front = self.tabs[i].id;
        }
    }

    /// The menu's commands, as the app's own keys too.
    fn run(&mut self, id: u32, window: &mut Window) {
        let doc = self.doc();
        match id {
            cmd::NEW_WINDOW => window.open_window(vergence_widgets::window(EditWindow::new(self.shared.clone(), Vec::new()))),
            cmd::NEW_TAB => {
                let t = self.untitled_tab();
                self.front = t.id;
                self.tabs.push(t);
                self.apply_options();
            }
            cmd::OPEN => self.pick(Picking::Open, window),
            cmd::SAVE => self.save_or_ask(self.front, window),
            cmd::SAVE_AS => self.pick(Picking::SaveAs { tab: self.front, then_close: false }, window),
            cmd::CLOSE_TAB => self.close_tab(self.front, window),
            cmd::UNDO => {
                if let Some(d) = doc {
                    d.borrow_mut().undo();
                }
            }
            cmd::REDO => {
                if let Some(d) = doc {
                    d.borrow_mut().redo();
                }
            }
            cmd::CUT | cmd::COPY => {
                if let Some(d) = doc {
                    let text = d.borrow().selected_text();
                    if !text.is_empty() {
                        window.copy(&text);
                        if id == cmd::CUT {
                            d.borrow_mut().insert("");
                        }
                    }
                }
            }
            cmd::PASTE => window.paste(),
            cmd::SELECT_ALL => {
                if let Some(d) = doc {
                    d.borrow_mut().select_all();
                }
            }
            cmd::ZOOM_IN => self.set_zoom(self.zoom * 1.1),
            cmd::ZOOM_OUT => self.set_zoom(self.zoom / 1.1),
            cmd::ZOOM_RESET => self.set_zoom(1.0),
            cmd::LINE_NUMBERS => self.toggle_setting("line-numbers", true),
            cmd::WRAP => self.toggle_setting("wrap", true),
            cmd::FIND => self.open_find(false),
            cmd::GO_TO_LINE => {
                self.goto = Some(String::new());
                self.focus_turn += 1;
            }
            cmd::REPLACE => self.open_find(true),
            cmd::FIND_NEXT | cmd::FIND_PREV => {
                if self.find.is_none() {
                    self.open_find(false);
                }
                self.find_step(id == cmd::FIND_PREV);
            }
            cmd::NEXT_TAB => self.step_tab(1),
            cmd::PREV_TAB => self.step_tab(-1),
            cmd::SETTINGS => self.open_settings(window),
            cmd::ABOUT => {
                let about = vergence_widgets::about::About::new(APP_ID, "Edit", env!("CARGO_PKG_VERSION"), "A plain-text editor: tabs, line numbers, wrapping, undo; files saved whole or not at all.", self.theme());
                window.open_window(vergence_widgets::window(about));
            }
            _ => {}
        }
    }
}

impl EditWindow {
    fn find_bar(&self, f: &Find, t: &Theme) -> Element<Msg> {
        let small = |text: &str, m: Msg, on: bool, name: &str| -> Element<Msg> {
            button(label(text).size(12.0)).look(if on { ButtonLook::Chosen } else { ButtonLook::Plain }).padding(Padding::hv(3.0, 8.0)).on_press(m).name(name)
        };
        let count = match (f.replaced, f.needle.is_empty(), f.matches.len()) {
            (Some(n), _, _) => format!("Replaced {}", grouped(n)),
            (_, true, _) => String::new(),
            (_, _, 0) => "Not found".into(),
            (_, _, n) => {
                let at = self.doc().and_then(|d| {
                    let sel = d.borrow().selection();
                    f.matches.iter().position(|m| (m.0, m.1) == sel)
                });
                let n = if n >= MAX_MATCHES { format!("{}+", grouped(n)) } else { grouped(n) };
                match at {
                    Some(k) => format!("{} of {n}", grouped(k + 1)),
                    None => format!("{n} found"),
                }
            }
        };
        let find_field = field(f.needle.clone(), Msg::FindTyped).placeholder("Find").on_submit(Msg::FindNext).key(self.focus_turn).autofocus().width(Length::FILL).name("Find");
        let mut rows: Vec<Element<Msg>> = vec![row(vec![
            find_field,
            label(count).size(12.0).dim().width(Length::Fixed(96.0)),
            small("‹", Msg::FindPrev, false, "Previous match"),
            small("›", Msg::FindNext, false, "Next match"),
            small("Aa", Msg::FindCase, f.case, "Match case"),
            small("Whole words", Msg::FindWhole, f.whole, "Whole words"),
            small("Replace", Msg::FindReplace, f.replace, "Show Replace"),
            small("×", Msg::FindClose, false, "Close"),
        ])
        .spacing(4.0)
        .align(Align::Center)
        .width(Length::FILL)];
        if f.replace {
            let editable = self.front_tab().is_some_and(|t| t.read_only.is_none());
            rows.push(
                row(vec![
                    field(f.with.clone(), Msg::WithTyped).placeholder("Replace with").on_submit(Msg::ReplaceOne).width(Length::FILL).name("Replace with"),
                    button(label("Replace").size(12.0)).padding(Padding::hv(3.0, 10.0)).on_press_if(editable, Msg::ReplaceOne).name("Replace"),
                    button(label("Replace all").size(12.0)).padding(Padding::hv(3.0, 10.0)).on_press_if(editable && !f.matches.is_empty(), Msg::ReplaceAll).name("Replace all"),
                ])
                .spacing(6.0)
                .align(Align::Center)
                .width(Length::FILL),
            );
        }
        let _ = t;
        panel(column(rows).spacing(6.0).width(Length::FILL)).radius(0.0).padding(Padding::hv(6.0, 10.0)).width(Length::FILL)
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
        let tabs = self.tabs.iter().map(|t| BarTab { id: t.id, title: t.name(), closable: true, alert: false, edited: t.modified() }).collect();
        // The tab in front is the page's color, as the terminal's: joined to
        // the text, and the server knows the page's style, so text scrolled
        // under a bar of the other style shows through inverted (readable)
        // rather than all but hidden by the frost.
        let page = self.page_drawn.get().unwrap_or_else(|| {
            let p = self.theme().page;
            (p.r as u32) << 16 | (p.g as u32) << 8 | p.b as u32
        });
        Some(Bar::Tabs { tabs, front: self.front_tab().map_or(0, |t| t.id), plus: true, accent: Some(page), menu: Vec::new() })
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        // The page as drawn now (tinted with the wallpaper, if it is), for the tab in front.
        self.page_drawn.set(Some((t.page.r as u32) << 16 | (t.page.g as u32) << 8 | t.page.b as u32));
        let Some(tab) = self.front_tab() else { return space() };
        // Strips above the text (notices, the find bar), on a shade over the
        // window's ground so its opacity shows through as everywhere else.
        let shade = vergence_scene::Color { a: if t.dark { 20 } else { 14 }, ..t.text };
        let mut parts: Vec<Element<Msg>> = Vec::new();
        let strip = |text: String, dismiss: bool| -> Element<Msg> {
            let mut items: Vec<Element<Msg>> = vec![label(text).size(12.0).wrap().width(Length::FILL)];
            if dismiss {
                items.push(button(label("OK").size(12.0)).padding(Padding::hv(2.0, 10.0)).on_press(Msg::DismissNotice).name("OK"));
            }
            panel(row(items).spacing(8.0).align(Align::Center).width(Length::FILL)).padding(Padding::hv(6.0, 12.0)).width(Length::FILL)
        };
        if let Some(n) = &self.notice {
            parts.push(strip(n.clone(), true));
        }
        if let Some(r) = &tab.read_only {
            parts.push(strip(r.clone(), false));
        }
        // Changed on disk while it has changes here: which to keep.
        if tab.changed_on_disk {
            parts.push(
                panel(
                    row(vec![
                        label(format!("“{}” changed on disk.", tab.name())).size(12.0).wrap().width(Length::FILL),
                        button(label("Keep mine").size(12.0)).padding(Padding::hv(2.0, 10.0)).on_press(Msg::KeepMine).name("Keep mine"),
                        button(label("Reload").size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(2.0, 10.0)).on_press(Msg::Reload).name("Reload from disk"),
                    ])
                    .spacing(8.0)
                    .align(Align::Center)
                    .width(Length::FILL),
                )
                .padding(Padding::hv(6.0, 12.0))
                .width(Length::FILL),
            );
        }
        if let Some(g) = &self.goto {
            parts.push(
                panel(
                    row(vec![
                        label("Go to line").size(12.0),
                        field(g.clone(), Msg::GotoTyped).placeholder("Line, or line:column").on_submit(Msg::GotoGo).key(self.focus_turn).autofocus().width(Length::Fixed(200.0)).name("Line"),
                        label(format!("of {}", grouped(tab.doc.borrow().buffer().line_count()))).size(12.0).dim(),
                        space().width(Length::FILL),
                        button(label("×").size(12.0)).look(ButtonLook::Plain).padding(Padding::hv(2.0, 8.0)).on_press(Msg::GotoClose).name("Close"),
                    ])
                    .spacing(8.0)
                    .align(Align::Center)
                    .width(Length::FILL),
                )
                .padding(Padding::hv(6.0, 12.0))
                .width(Length::FILL),
            );
        }
        if let Some(f) = &self.find {
            parts.push(self.find_bar(f, t));
        }
        // With strips, they go under the server's bar (padding is a panel's,
        // not a column's) and the text below them; without, the text itself
        // starts under the bar and scrolls up beneath it, frosted, as the
        // terminal's does (the text area keeps its first line below the bar).
        let mut all: Vec<Element<Msg>> = Vec::new();
        if !parts.is_empty() {
            all.push(panel(column(std::mem::take(&mut parts)).spacing(1.0).width(Length::FILL)).color(shade).padding(Padding { top: t.top, ..Padding::default() }).radius(0.0).width(Length::FILL));
        }
        // The area's id changes with focus_turn, so it takes the keyboard back
        // when the find bar closes (its state is the document's, not the id's).
        let area = text_area(&tab.doc).key((self.focus_turn << 32) | tab.id as u64).on_change(Msg::Changed).name(tab.name());
        all.push(if self.find.is_none() && self.goto.is_none() { area.autofocus() } else { area });
        // The status line: where the caret is, how Tab indents, the line endings.
        let d = tab.doc.borrow();
        let (l, c) = d.line_column();
        let (a, b) = d.selection();
        let mut status: Vec<Element<Msg>> = vec![space().width(Length::FILL)];
        // Words (Dave, 2026-11-15: "a word counter in there somewhere").
        let words = grouped(d.word_count());
        status.push(label(if a != b { format!("{} of {words} words", grouped(d.selected_words())) } else { format!("{words} words") }).size(12.0).dim());
        if a != b {
            let n = d.selected_text().chars().count();
            status.push(label(format!("{} characters selected", grouped(n))).size(12.0).dim());
        }
        status.push(label(format!("Line {l}, Column {c}")).size(12.0).dim());
        let o = d.options();
        let indent = if o.spaces { format!("Spaces: {}", o.tab_width) } else { format!("Tab width: {}", o.tab_width) };
        let ending = if d.buffer().ending() == Ending::CrLf { "CRLF" } else { "LF" };
        let small = |text: String, m: Msg, name: &str| -> Element<Msg> { button(label(text).size(12.0).dim()).look(ButtonLook::Plain).padding(Padding::hv(1.0, 6.0)).on_press(m).name(name) };
        status.push(small(indent, Msg::ToggleSpaces, "Indent with spaces or tabs"));
        status.push(small(ending.into(), Msg::ToggleEnding, "Line endings"));
        status.push(label("UTF-8").size(12.0).dim());
        all.push(rule());
        all.push(panel(row(status).spacing(14.0).align(Align::Center).width(Length::FILL)).radius(0.0).padding(Padding { top: 3.0, bottom: 3.0, left: 12.0, right: 12.0 }).width(Length::FILL));
        column(all).width(Length::FILL).height(Length::FILL)
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
        const PAGE_UP: u32 = 0xff55;
        const PAGE_DOWN: u32 = 0xff56;
        const ESCAPE: u32 = 0xff1b;
        let Input::Key { sym, pressed: true, modifiers: m, .. } = input else { return false };
        if *sym == ESCAPE && self.goto.is_some() {
            self.goto = None;
            self.focus_turn += 1;
            return true;
        }
        if *sym == ESCAPE && self.find.is_some() && !self.doc().is_some_and(|d| d.borrow().completing()) {
            self.close_find();
            return true;
        }
        if !(m.ctrl || m.logo) || m.alt {
            return false;
        }
        let letter = char::from_u32(*sym).map(|c| c.to_ascii_lowercase());
        let id = match (letter, *sym) {
            (Some('s'), _) if m.shift => cmd::SAVE_AS,
            (Some('s'), _) => cmd::SAVE,
            (Some('n'), _) => cmd::NEW_WINDOW,
            (Some('o'), _) => cmd::OPEN,
            (Some('t'), _) => cmd::NEW_TAB,
            (Some('w'), _) => cmd::CLOSE_TAB,
            (Some('=' | '+'), _) => cmd::ZOOM_IN,
            (Some('-'), _) => cmd::ZOOM_OUT,
            (Some('0'), _) => cmd::ZOOM_RESET,
            (Some(','), _) => cmd::SETTINGS,
            (Some('f'), _) => cmd::FIND,
            (Some('l'), _) => cmd::GO_TO_LINE,
            (Some('h'), _) => cmd::REPLACE,
            (Some('g'), _) if m.shift => cmd::FIND_PREV,
            (Some('g'), _) => cmd::FIND_NEXT,
            (_, PAGE_DOWN) => cmd::NEXT_TAB,
            (_, PAGE_UP) => cmd::PREV_TAB,
            _ => return false,
        };
        self.run(id, window);
        self.keep_soon(window);
        true
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        let (undo, redo, selected, editable) = match self.front_tab() {
            Some(t) => {
                let d = t.doc.borrow();
                let (a, b) = d.selection();
                (d.can_undo(), d.can_redo(), a != b, t.read_only.is_none())
            }
            None => (false, false, false, false),
        };
        Some((
            false,
            vec![
                C::role(cmd::NEW_TAB, Role::NewTab).featured(),
                C::role(cmd::NEW_WINDOW, Role::NewWindow),
                C::role(cmd::OPEN, Role::Open).featured(),
                C::role(cmd::SAVE, Role::Save).enabled(editable).featured(),
                C::role(cmd::SAVE_AS, Role::SaveAs),
                C::role(cmd::CLOSE_TAB, Role::CloseTab),
                C::role(cmd::UNDO, Role::Undo).enabled(undo && editable),
                C::role(cmd::REDO, Role::Redo).enabled(redo && editable),
                C::role(cmd::CUT, Role::Cut).enabled(selected && editable),
                C::role(cmd::COPY, Role::Copy).enabled(selected),
                C::role(cmd::PASTE, Role::Paste).enabled(editable),
                C::role(cmd::SELECT_ALL, Role::SelectAll),
                C::role(cmd::FIND, Role::Find).featured(),
                C::role(cmd::FIND_NEXT, Role::FindNext),
                C::own(cmd::FIND_PREV, "Edit", "Find previous", "Ctrl+Shift+G"),
                C::role(cmd::REPLACE, Role::Replace).enabled(editable),
                C::own(cmd::GO_TO_LINE, "Edit", "Go to line…", "Ctrl+L"),
                // The app menu's "− 100% +" Text size row, as the terminal's
                // (Dave's list, 2026-11-16): for now, this window; the lasting
                // size is the editor's own settings window's.
                C::role(cmd::ZOOM_OUT, Role::ZoomOut).enabled(self.zoom > ZOOM_MIN + 0.001).featured(),
                C { label: format!("{}%", (self.zoom * 100.0).round()), ..C::role(cmd::ZOOM_RESET, Role::ZoomReset).featured() },
                C::role(cmd::ZOOM_IN, Role::ZoomIn).enabled(self.zoom < ZOOM_MAX - 0.001).featured(),
                C::own(cmd::LINE_NUMBERS, "View", "Line numbers", "").checked(self.flag("line-numbers", true)).featured(),
                C::own(cmd::WRAP, "View", "Wrap lines", "").checked(self.flag("wrap", true)),
                C::role(cmd::NEXT_TAB, Role::NextTab),
                C::role(cmd::PREV_TAB, Role::PreviousTab),
                C::role(cmd::SETTINGS, Role::Settings),
                C::role(cmd::ABOUT, Role::About),
            ],
        ))
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
        let paths: Vec<PathBuf> = items.iter().find(|(k, _)| k == "text/uri-list").map(|(_, v)| String::from_utf8_lossy(v).lines().filter_map(|l| vergence_types::path_of_link(l.trim())).collect()).unwrap_or_default();
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
        match event {
            TabEvent::Chosen(id) => {
                self.front = id;
                if self.find.is_some() {
                    self.refind();
                }
            }
            TabEvent::Close(id) => self.close_tab(id, window),
            TabEvent::CloseOthers(id) => {
                for t in self.tabs.iter().map(|t| t.id).filter(|t| *t != id).collect::<Vec<_>>() {
                    if !self.closing.contains(&t) {
                        self.closing.push(t);
                    }
                }
                self.go_on_closing(window);
            }
            TabEvent::New => self.run(cmd::NEW_TAB, window),
            TabEvent::Moved { tab, to } => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == tab) {
                    let t = self.tabs.remove(i);
                    self.tabs.insert((to as usize).min(self.tabs.len()), t);
                }
            }
            TabEvent::Out { tab, .. } => {
                if self.tabs.len() > 1
                    && let Some(t) = self.remove_tab(tab)
                {
                    let mut w = EditWindow::new(self.shared.clone(), vec![t]);
                    w.zoom = self.zoom;
                    w.apply_options();
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
        self.keep_soon(window);
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

    fn closed(&mut self) {
        let mut s = self.shared.borrow_mut();
        s.windows.retain(|(id, _)| Some(*id) != self.me);
        // Closed by the user (its questions answered): it is not kept.
        s.snaps.retain(|(id, _)| Some(*id) != self.me);
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
            let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut s.transfers).into_iter().partition(|(into, _, _)| Some(*into) == self.me);
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
            let requests: Vec<Vec<PathBuf>> = std::mem::take(&mut *self.shared.borrow().inbox.lock().unwrap_or_else(|e| e.into_inner()));
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

/// Words from other documents for completion: each document's words with
/// how often each comes, found again only when it changed.
fn other_words(docs: Vec<vergence_widgets::WeakDoc>) -> vergence_widgets::Words {
    type Counted = Vec<(String, u32)>;
    let cache: RefCell<Vec<(u64, Counted)>> = RefCell::new(vec![(u64::MAX, Vec::new()); docs.len()]);
    Rc::new(move |prefix: &str, limit: usize| {
        let want = prefix.to_lowercase();
        let mut cache = cache.borrow_mut();
        let mut found: Vec<(String, u32)> = Vec::new();
        for (k, weak) in docs.iter().enumerate() {
            let Some(doc) = weak.upgrade() else { continue };
            let d = doc.borrow();
            if cache[k].0 != d.revision() {
                let mut counts: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
                for line in d.buffer().lines() {
                    for w in line.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '\'')) {
                        let w = w.trim_matches('\'');
                        if w.chars().count() >= 3 {
                            *counts.entry(w).or_insert(0) += 1;
                        }
                    }
                }
                cache[k] = (d.revision(), counts.into_iter().map(|(w, n)| (w.to_string(), n)).collect());
            }
            for (w, n) in &cache[k].1 {
                if w.len() > prefix.len() && w.to_lowercase().starts_with(&want) {
                    match found.iter_mut().find(|(f, _)| f == w) {
                        Some(f) => f.1 += n,
                        None => found.push((w.clone(), *n)),
                    }
                }
            }
        }
        found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        found.into_iter().take(limit).map(|(w, _)| w).collect()
    })
}

/// The name the Save dialog suggests: the file's own, or for a new
/// document its first words (as TextEdit does), else "Untitled".
fn suggested_name(t: &DocTab) -> String {
    if let Some(p) = &t.path {
        return files::name(p);
    }
    let doc = t.doc.borrow();
    let first = doc.buffer().lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let clean: String = first.chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()).collect();
    let words: Vec<&str> = clean.split_whitespace().collect();
    let mut name = String::new();
    for w in words {
        if name.chars().count() + w.chars().count() > 40 {
            break;
        }
        if !name.is_empty() {
            name.push(' ');
        }
        name.push_str(w);
    }
    let name = name.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    if name.is_empty() { format!("{}.txt", t.name()) } else { format!("{name}.txt") }
}

/// 1234567 as "1,234,567".
fn grouped(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "Save changes?", a modal dialog over the window. The answer goes back
/// through `answer` and a wake of the window.
struct SaveQuestion {
    name: String,
    answer: Rc<Cell<Option<Answer>>>,
    waker: Option<Waker>,
    theme: Theme,
}

impl Program for SaveQuestion {
    type Message = Answer;
    fn title(&self) -> String {
        "Save changes?".into()
    }
    fn app_id(&self) -> String {
        APP_ID.into()
    }
    fn theme(&self) -> Theme {
        self.theme.clone()
    }
    fn view(&self, _: &Theme) -> Element<Answer> {
        let b = |text: &str, a: Answer, look: ButtonLook| -> Element<Answer> { button(label(text).size(12.0)).look(look).padding(Padding::hv(4.0, 12.0)).on_press(a).name(text) };
        panel(
            column(vec![
                label(format!("Save the changes to “{}”?", self.name)).size(15.0).weight(600).wrap().width(Length::FILL),
                label("If you don't save them, they are lost.").dim().wrap().width(Length::FILL),
                row(vec![b("Don't save", Answer::Discard, ButtonLook::Normal), space().width(Length::FILL), b("Cancel", Answer::Cancel, ButtonLook::Normal), b("Save", Answer::Save, ButtonLook::Accent)]).spacing(8.0).align(Align::Center).width(Length::FILL),
            ])
            .spacing(10.0)
            .width(Length::FILL),
        )
        .padding(20.0)
        .radius(0.0)
        .width(Length::FILL)
        .height(Length::FILL)
    }
    fn update(&mut self, a: Answer, window: &mut Window) {
        self.answer.set(Some(a));
        window.close();
    }
    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { kind: vergence_ui::WinKind::Dialog, modal: true, min: (420.0, 150.0), max: (0.0, 0.0), alone: false }
    }
}

impl Drop for SaveQuestion {
    /// Answered or closed, the window hears of it.
    fn drop(&mut self) {
        if let Some(w) = &self.waker {
            w.wake();
        }
    }
}

/// Its settings window's rows.
struct EditRows {
    /// Monospaced families first, then the rest (fc-list, once).
    fonts: Vec<String>,
}

impl EditRows {
    fn new() -> EditRows {
        let list = |args: &[&str]| -> Vec<String> {
            let out = std::process::Command::new("fc-list").args(args).output();
            let mut v: Vec<String> = out.ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default().lines().filter_map(|l| l.split(',').next().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect();
            v.sort_by_key(|n| n.to_lowercase());
            v.dedup();
            v
        };
        let mut fonts = list(&[":spacing=mono", "family"]);
        let rest: Vec<String> = list(&[":", "family"]).into_iter().filter(|f| !fonts.contains(f)).collect();
        fonts.extend(rest);
        EditRows { fonts }
    }
}

impl Rows for EditRows {
    fn parts(&self, text: &str) -> Vec<Part> {
        let fonts: Vec<Choice> = self.fonts.iter().map(|f| Choice::text(f, f)).collect();
        let sizes: Vec<Choice> = [8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28].iter().map(|s| Choice::number(&s.to_string(), *s)).collect();
        let tabs: Vec<Choice> = [2, 3, 4, 8].iter().map(|s| Choice::number(&s.to_string(), *s)).collect();
        let opacity = vergence_ui::opacity_setting(text, "[appearance]");
        let locked = vergence_ui::desktop_pick(text).map(|light| format!("{} for every app: Settings > Appearance > Style", if light { "Light" } else { "Dark" }));
        let styles = vec![Choice::text("Dark", "dark"), Choice::text("Light", "light")];
        vec![
            Part {
                title: "Text".into(),
                rows: vec![
                    Row::new("font", "Font", "", Control::Choose { choices: fonts, desktop: Some("Monospace".into()), app_default: 0 }),
                    Row::new("text-size", "Text size", "Ctrl+= and Ctrl+− change it for a while.", Control::Choose { choices: sizes, desktop: Some("Desktop default".into()), app_default: 0 }),
                    Row::new("line-numbers", "Line numbers", "", Control::Switch { default: true }),
                    Row::new("wrap", "Wrap lines", "Long lines go on below at the window's edge; off, the text scrolls sideways.", Control::Switch { default: true }),
                    Row::new("current-line", "Shade the current line", "", Control::Switch { default: true }),
                    Row::new("spellcheck", "Check spelling", "Words spelled wrong are underlined in red; a right click on one gives suggestions.", Control::Choose { choices: vec![Choice::text("On", "true"), Choice::text("Off", "false")], desktop: Some(format!("Desktop default ({})", if vergence_ui::table_value(text, "[text]", "spellcheck").as_deref() == Some("false") { "Off" } else { "On" })), app_default: 0 }),
                    Row::new("complete", "Suggest words as you type", "The likely rest of a word shows in gray; Tab takes it. Alt+Escape, F5 or Ctrl+Space lists them all.", Control::Switch { default: true }),
                ],
            },
            Part {
                title: "Indenting".into(),
                rows: vec![
                    Row::new("tab-width", "Tab width", "How many spaces wide a tab is.", Control::Choose { choices: tabs, desktop: Some("4".into()), app_default: 0 }),
                    Row::new("spaces", "Indent with spaces", "Tab inserts spaces; a file indented with tabs keeps its tabs.", Control::Switch { default: true }),
                ],
            },
            Part {
                title: "On this desktop".into(),
                rows: vec![
                    Row::new("style", "Style", "While the desktop's Style is App default.", Control::Choose { choices: styles.clone(), desktop: Some("Built-in (Light)".into()), app_default: 0 }).locked(locked.clone()),
                    Row::new("bar-style", "Title bar", "", Control::Choose { choices: styles, desktop: Some("Built-in (Light)".into()), app_default: 0 }).locked(locked),
                    Row::new("opacity", "Opacity", "30 to 100: how much of the window's ground shows; the text stays as it is.", Control::Percent { min: 30, desktop: format!("Desktop default ({:.0})", opacity * 100.0), now: (opacity * 100.0).round() as u32 }),
                    Row::new("", "Window places", "Where the desktop puts the editor's windows, for each set of screens.", Control::Button { label: "Forget…".into(), id: 1 }),
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
    let shared = Rc::new(RefCell::new(Shared { windows: Vec::new(), transfers: Vec::new(), inbox, listener, next_tab: 0, snaps: Vec::new(), pending: Vec::new() }));
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
mod tests {
    use super::*;

    fn shared() -> SharedRef {
        Rc::new(RefCell::new(Shared { windows: Vec::new(), transfers: Vec::new(), inbox: Arc::new(Mutex::new(Vec::new())), listener: Arc::new(Mutex::new(None)), next_tab: 0, snaps: Vec::new(), pending: Vec::new() }))
    }

    #[test]
    fn untitled_documents_are_numbered_and_a_file_replaces_an_empty_one() {
        let mut w = EditWindow::new(shared(), Vec::new());
        assert_eq!(w.tabs.len(), 1);
        assert_eq!(w.tabs[0].name(), "Untitled");
        let t = w.untitled_tab();
        assert_eq!(t.name(), "Untitled 2");
        w.tabs.push(t);
        w.tabs.remove(0);
        assert_eq!(w.untitled_tab().name(), "Untitled", "a number given back is used again");
        w.tabs.clear();
        let fresh = w.untitled_tab();
        w.tabs.push(fresh);
        let dir = std::env::temp_dir().join(format!("vergence-edit-main-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("tabbed.txt");
        std::fs::write(&p, "a\n\tb\n\tc\n").unwrap();
        w.open_files(std::slice::from_ref(&p));
        assert_eq!(w.tabs.len(), 1, "the empty Untitled gave way");
        assert_eq!(w.tabs[0].name(), "tabbed.txt");
        assert!(!w.tabs[0].doc.borrow().options().spaces, "a file indented with tabs keeps tabs");
        assert!(!w.tabs[0].modified());
        // Opening it again brings the same tab forward.
        w.open_files(std::slice::from_ref(&p));
        assert_eq!(w.tabs.len(), 1);
        w.tabs[0].doc.borrow_mut().insert("x");
        assert!(w.tabs[0].modified());
        assert!(w.save(w.tabs[0].id));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "xa\n\tb\n\tc\n");
        assert!(!w.tabs[0].modified());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_new_document_is_named_by_its_first_words() {
        let w = EditWindow::new(shared(), Vec::new());
        let t = &w.tabs[0];
        assert_eq!(suggested_name(t), "Untitled.txt");
        t.doc.borrow_mut().insert("\n  Shopping list: eggs/milk, and a great many other things to get\nmore");
        assert_eq!(suggested_name(t), "Shopping list eggsmilk, and a great many.txt");
    }

    #[test]
    fn a_window_is_kept_and_put_back_with_its_unsaved_work() {
        let dir = std::env::temp_dir().join(format!("vergence-edit-keep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("saved.txt");
        std::fs::write(&p, "one\ntwo\nthree\n").unwrap();
        let mut w = EditWindow::new(shared(), Vec::new());
        w.tabs[0].doc.borrow_mut().insert("a note not saved");
        w.open_files(std::slice::from_ref(&p));
        w.tabs[1].doc.borrow_mut().set_selection(vergence_widgets::edit::Pos::new(2, 1), vergence_widgets::edit::Pos::new(2, 1));
        w.token = "tok".into();
        let snap = w.snap();
        assert_eq!(snap.tabs.len(), 2);
        assert_eq!(snap.tabs[0].text.as_deref(), Some("a note not saved"));
        assert_eq!(snap.tabs[1].text, None, "a saved file is read again, not copied");
        let back = EditWindow::from_snap(shared(), &snap);
        assert_eq!(back.token, "tok");
        assert_eq!(back.tabs[0].name(), "Untitled");
        assert!(back.tabs[0].modified(), "unsaved work stays unsaved");
        assert_eq!(back.tabs[0].doc.borrow().text(), "a note not saved");
        assert_eq!(back.tabs[1].doc.borrow().caret(), vergence_widgets::edit::Pos::new(2, 1));
        assert_eq!(back.front, back.tabs[1].id, "the file was in front");
        // An empty Untitled is not kept.
        let empty = EditWindow::new(shared(), Vec::new());
        assert!(empty.snap().tabs.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_changed_on_disk_is_read_again_or_asked_about() {
        let dir = std::env::temp_dir().join(format!("vergence-edit-disk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("watched.txt");
        std::fs::write(&p, "first\n").unwrap();
        let mut w = EditWindow::new(shared(), Vec::new());
        w.open_files(std::slice::from_ref(&p));
        assert!(!w.check_disk(), "nothing changed");
        std::fs::write(&p, "second, longer\n").unwrap();
        assert!(w.check_disk());
        assert_eq!(w.tabs[0].doc.borrow().text(), "second, longer\n", "no changes here: read again");
        w.tabs[0].doc.borrow_mut().insert("mine ");
        std::fs::write(&p, "third, longer still\n").unwrap();
        assert!(w.check_disk());
        assert!(w.tabs[0].changed_on_disk, "changes here: asked");
        assert!(w.tabs[0].doc.borrow().text().starts_with("mine "));
        assert!(w.go_to("1:3"));
        assert_eq!(w.tabs[0].doc.borrow().caret(), vergence_widgets::edit::Pos::new(0, 2));
        assert!(!w.go_to("zero"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn closing_removes_tabs_and_moves_the_front() {
        let mut w = EditWindow::new(shared(), Vec::new());
        let (a, b) = (w.untitled_tab(), w.untitled_tab());
        let (ida, idb) = (a.id, b.id);
        w.tabs.push(a);
        w.tabs.push(b);
        w.front = ida;
        w.remove_tab(ida);
        assert_eq!(w.front, idb, "the next tab comes to the front");
        w.step_tab(1);
        assert_eq!(w.front, w.tabs[0].id, "and round");
    }
}
