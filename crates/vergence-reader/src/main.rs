//! Reader (Reader milestone 1; docs/reader-milestone-1.md): PDF documents
//! in an app of their own, split from Close-Up (Dave, 2026-11-20: "We need
//! to have a unique window spec for when Close-Up is displaying PDF vs
//! images... Maybe we should make it its own application?"; named Reader
//! the same day). Close-Up keeps pictures and hands PDFs here.
//!
//! - **Documents** are read and drawn by vergence-pdf's confined helper,
//!   never in this process (pdf.rs): pages in a column, scrolled
//!   continuously, fit to the width (the default), the whole page, actual
//!   size or any zoom; drawn at the screen's pixels; a sidebar (S) with the
//!   pages' thumbnails and the outline; links followed (an address outside
//!   the document is asked about first); passwords asked for; each
//!   document's page and zoom remembered; text selected by dragging on it,
//!   copied (Ctrl+C), the page's text selected (Ctrl+A), and Find (Ctrl+F)
//!   across the whole document, every match marked.
//! - **Windows and tabs**: one process (hand.rs); each document in a tab
//!   (the server's), each tab with its own view; dragged out of the row a
//!   tab is a window of its own, dropped on another window it joins it.
//!   Files handed over, chosen in Open… or dropped open in tabs. Files
//!   changed on disk are read again, the view kept.
//! - **Its settings** (Ctrl+,): the ground around the pages, the style and
//!   the bar; desktop.toml `[reader]`.

mod hand;
mod info;
mod pdf;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use vergence_scene::{Color, Rect};
use vergence_ui::{AppCommand as C, Bar, BarStyle, BarTab, Role, TabEvent};
use vergence_widgets::prefs::{Choice, Control, Part, Prefs, Row, Rows};
use vergence_widgets::{Align, ButtonLook, Element, Input, Length, Padding, Program, Theme, Touch, Waker, Window, button, column, drawing, field, label, panel, row, rule, scroll, vrule};

const APP_ID: &str = "vergence-reader";
const SECTION: &str = "[reader]";
/// A document's page and zoom saved a moment after they stop changing.
const PLACE_TIMER: u64 = 2;
/// Open files looked at for changes this often (as Edit does).
const WATCH_TIMER: u64 = 3;
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

mod cmd {
    pub const NEW_WINDOW: u32 = 1;
    pub const OPEN: u32 = 2;
    pub const CLOSE_TAB: u32 = 3;
    pub const ZOOM_IN: u32 = 4;
    pub const ZOOM_OUT: u32 = 5;
    pub const FIT: u32 = 6;
    pub const ACTUAL: u32 = 7;
    pub const TURN_RIGHT: u32 = 8;
    pub const TURN_LEFT: u32 = 9;
    pub const NEXT: u32 = 10;
    pub const PREVIOUS: u32 = 11;
    pub const OPEN_WITH: u32 = 14;
    pub const FIT_PAGE: u32 = 15;
    pub const SIDEBAR: u32 = 16;
    pub const COPY: u32 = 17;
    pub const SELECT_ALL: u32 = 18;
    pub const FIND: u32 = 19;
    pub const FIND_NEXT: u32 = 20;
    pub const FIND_PREVIOUS: u32 = 21;
    pub const BACK: u32 = 22;
    pub const FORWARD: u32 = 23;
    pub const GO_TO_PAGE: u32 = 24;
    pub const VIEW_CONTINUOUS: u32 = 25;
    pub const VIEW_SINGLE: u32 = 26;
    pub const VIEW_TWO: u32 = 27;
    pub const NIGHT: u32 = 28;
    pub const INFO: u32 = 29;
    pub const SETTINGS: u32 = 40;
    pub const ABOUT: u32 = 41;
}

/// What a file turned out to be, from the worker that read it.
enum Loaded {
    Pdf(pdf::Pdf),
    /// A PDF that needs its password, open in its helper.
    Locked(vergence_pdf::helper::Viewer),
    /// Why it cannot be shown here, and whether another app might.
    Problem { why: String, elsewhere: bool },
}

/// What all windows share (one thread).
struct Shared {
    /// Files other starts handed over (hand.rs), and from the Open dialog.
    inbox: Arc<Mutex<Vec<Vec<PathBuf>>>>,
    /// Files read by the workers: (tab id, what it was).
    loaded: Arc<Mutex<Vec<(u32, Loaded)>>>,
    next_tab: u32,
    /// The windows, by the server's id, and how to wake each.
    windows: Vec<(u32, Waker)>,
    /// Tabs dropped on another window: (into, at, the tab).
    transfers: Vec<(u32, u32, Tab)>,
}

type SharedRef = Rc<RefCell<Shared>>;

/// A tab: one document, and the pages gone to by links, the outline,
/// find and the page field (back and forward through them).
struct Tab {
    id: u32,
    path: PathBuf,
    content: Content,
    back: Vec<usize>,
    forward: Vec<usize>,
    /// The file as last read (modified time, length): changed on disk, it
    /// is read again.
    stamp: Option<(SystemTime, u64)>,
    /// A document read again: its view to put back (zoom, view, turn,
    /// night, page) once it is open.
    keep: Option<(pdf::Mode, pdf::View, u16, bool, usize)>,
}

/// A file's modified time and length, to see that it changed.
fn stamp_of(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

fn new_tab(id: u32, path: PathBuf) -> Tab {
    let stamp = stamp_of(&path);
    Tab { id, path, content: Content::Loading, back: Vec::new(), forward: Vec::new(), stamp, keep: None }
}

enum Content {
    Loading,
    Pdf(pdf::Pdf),
    /// The helper (taken while a password is tried), the password typed, and
    /// whether the last one was wrong.
    Locked { viewer: Option<vergence_pdf::helper::Viewer>, password: String, wrong: bool },
    Problem { why: String, elsewhere: bool },
}

impl Tab {
    fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.display().to_string())
    }

    /// The document's own title (its Info /Title) when it has a real one,
    /// else the file's name.
    fn title(&self) -> String {
        if let Content::Pdf(d) = &self.content
            && let Some(meta) = d.shared.meta.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
            && let Some((_, t)) = meta.iter().find(|(k, _)| k == "Title")
            && title_is_real(t)
        {
            return t.clone();
        }
        self.name()
    }
}

/// Whether a document's Title says something: not empty, not "untitled",
/// not a file's name (as many programs write: "Microsoft Word - a.docx").
fn title_is_real(t: &str) -> bool {
    let low = t.trim().to_lowercase();
    let file_like = [".pdf", ".doc", ".docx", ".odt", ".rtf", ".txt", ".tex", ".indd", ".ps", ".xls", ".xlsx", ".ppt", ".pptx"].iter().any(|e| low.ends_with(e));
    low.chars().count() >= 3 && !["untitled", "title", "document", "(anonymous)"].contains(&low.as_str()) && !low.starts_with("microsoft word - ") && !file_like
}

/// Reads a document in a worker: opened in its helper, or why not.
fn load(path: PathBuf, id: u32, shared: &Shared, waker: Option<Waker>) {
    let out = shared.loaded.clone();
    std::thread::spawn(move || {
        let mime = vergence_types::of_file(&path);
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let loaded = if mime == "application/pdf" {
            match pdf::open(&path, waker.clone()) {
                Ok(pdf::Opened::Ready(d)) => Loaded::Pdf(d),
                Ok(pdf::Opened::Locked(v)) => Loaded::Locked(v),
                Err(why) => Loaded::Problem { why: format!("“{name}” could not be shown: {why}"), elsewhere: false },
            }
        } else if mime == vergence_types::FOLDER {
            Loaded::Problem { why: "This is a folder.".into(), elsewhere: true }
        } else {
            Loaded::Problem { why: format!("“{name}” is {} ({mime}), not a PDF document.", vergence_types::describe(mime)), elsewhere: true }
        };
        out.lock().unwrap_or_else(|e| e.into_inner()).push((id, loaded));
        if let Some(w) = waker {
            w.wake();
        }
    });
}

/// ~/.config/vergence/desktop.toml.
fn desktop_text() -> (String, Option<SystemTime>) {
    match vergence_widgets::prefs::desktop_path() {
        Some(p) => (std::fs::read_to_string(&p).unwrap_or_default(), std::fs::metadata(&p).and_then(|m| m.modified()).ok()),
        None => (String::new(), None),
    }
}

struct Viewer {
    shared: SharedRef,
    /// This window's id with the server, once it has one.
    me: Option<u32>,
    tabs: Vec<Tab>,
    front: u32,
    desktop: String,
    stamp: Option<SystemTime>,
    waker: Option<Waker>,
    scale: f32,
    /// The pages' place as last drawn.
    area: Rc<Cell<Rect>>,
    /// A drag: where it is now.
    drag: Option<(f32, f32)>,
    prefs_open: Rc<Cell<bool>>,
    /// Ctrl held (the wheel zooms then; scroll events carry no keys).
    ctrl: bool,
    /// The status line's page number while it is being typed.
    page_field: Option<String>,
    /// Text to put on the clipboard (the window does it in update).
    copy_out: Option<String>,
    /// The place saved last: path, page, mode.
    saved_place: Option<(PathBuf, usize, String)>,
    /// The sidebar: shown, and what it shows.
    sidebar: bool,
    side: Side,
    /// The thumbnails' place as last drawn.
    thumb_area: Rc<Cell<Rect>>,
    /// A press on a page: where, and whether it moved since (a drag, not a click).
    press_at: Option<((f32, f32), bool)>,
    /// An address outside the document a link asked to open, waiting for a yes.
    pending_uri: Option<String>,
    /// The Find bar, shown with its query.
    finding: Option<String>,
    /// Dragging out a text selection (not scrolling).
    selecting: bool,
    /// The page field's key (a new one drops its focus, or with
    /// `page_focus`, takes the keyboard: Go to Page).
    page_key: u64,
    page_focus: bool,
    /// The files are being watched for changes (the timer armed).
    watching: bool,
}

/// Addresses a document's links may open (through vergence-open, after asking).
fn openable(uri: &str) -> bool {
    let lower = uri.to_ascii_lowercase();
    ["http://", "https://", "mailto:", "ftp://"].iter().any(|s| lower.starts_with(s))
}

/// A document with its remembered page and zoom, to be gone to once placed.
fn with_place(mut d: pdf::Pdf, path: &Path) -> pdf::Pdf {
    if let Some((page, mode)) = pdf::places::load(path) {
        d.mode = mode;
        d.restore = Some(page.min(d.pages.len() - 1));
    }
    d
}

/// What the sidebar shows.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Side {
    Pages,
    Contents,
    Results,
}

#[derive(Debug, Clone)]
enum Msg {
    Touch(Touch),
    /// A command from the toolbar.
    Run(u32),
    Password(String),
    Unlock,
    PageField(String),
    GoToPage,
    ThumbTouch(Touch),
    /// The sidebar's tab.
    SideTab(Side),
    /// An outline entry chosen (its index).
    Outline(usize),
    /// A search result chosen (its index).
    Result(usize),
    OpenUri,
    CancelUri,
    FindText(String),
    FindDone,
    OpenWith,
}

impl Viewer {
    fn new(shared: SharedRef) -> Viewer {
        let (desktop, stamp) = desktop_text();
        Viewer {
            shared,
            me: None,
            tabs: Vec::new(),
            front: 0,
            desktop,
            stamp,
            waker: None,
            scale: 1.0,
            area: Rc::new(Cell::new(Rect::new(0.0, 0.0, 1.0, 1.0))),
            drag: None,
            prefs_open: Rc::new(Cell::new(false)),
            ctrl: false,
            page_field: None,
            copy_out: None,
            saved_place: None,
            sidebar: false,
            side: Side::Pages,
            thumb_area: Rc::new(Cell::new(Rect::new(0.0, 0.0, 1.0, 1.0))),
            press_at: None,
            pending_uri: None,
            finding: None,
            selecting: false,
            page_key: 0,
            page_focus: false,
            watching: false,
        }
    }

    /// Before a jump (a link, the outline, find, the page field): the page
    /// now, to come back to.
    fn mark(&mut self) {
        let r = self.area.get();
        let Some(t) = self.front_mut() else { return };
        let Content::Pdf(d) = &t.content else { return };
        let here = d.current(r);
        if t.back.last() != Some(&here) {
            t.back.push(here);
            if t.back.len() > 100 {
                t.back.remove(0);
            }
        }
        t.forward.clear();
    }

    /// Back (or forward) to the page gone from.
    fn go_back(&mut self, forward: bool) {
        let r = self.area.get();
        let Some(t) = self.front_mut() else { return };
        let Content::Pdf(d) = &mut t.content else { return };
        let here = d.current(r);
        let (from, to) = if forward { (&mut t.forward, &mut t.back) } else { (&mut t.back, &mut t.forward) };
        if let Some(p) = from.pop() {
            to.push(here);
            d.go_to(p, r);
        }
    }

    fn setting(&self, key: &str) -> Option<String> {
        vergence_ui::table_value(&self.desktop, SECTION, key)
    }

    fn reread(&mut self) -> bool {
        let (text, stamp) = desktop_text();
        if stamp == self.stamp {
            return false;
        }
        (self.desktop, self.stamp) = (text, stamp);
        true
    }

    /// Files to show, each in a tab of this window (one already open is
    /// brought to the front).
    fn take(&mut self, paths: &[PathBuf]) {
        self.open_paths(paths);
    }

    /// Takes a tab out of this window (to another, or one of its own).
    fn remove_tab(&mut self, id: u32) -> Option<Tab> {
        let i = self.tabs.iter().position(|t| t.id == id)?;
        let t = self.tabs.remove(i);
        if self.front == id {
            self.front = self.tabs.get(i.min(self.tabs.len().saturating_sub(1))).map_or(0, |t| t.id);
        }
        Some(t)
    }

    /// Closes a tab; the window with it, if it was the last.
    fn close_tab(&mut self, id: u32, window: &mut Window) {
        self.remove_tab(id);
        if self.tabs.is_empty() {
            window.close();
        }
        self.tab_changed();
    }

    /// Another document in front: the search goes on in it, the page field
    /// starts again.
    fn tab_changed(&mut self) {
        self.page_field = None;
        self.pending_uri = None;
        if let Some(q) = self.finding.clone()
            && let Some(d) = self.pdf_mut()
            && d.shared.search.lock().unwrap_or_else(|e| e.into_inner()).query != q.trim()
        {
            d.start_search(&q);
        }
    }

    /// Files changed on disk since they were read: read again, the view
    /// kept (as Edit does).
    fn watch(&mut self) {
        let r = self.area.get();
        let waker = self.waker.clone();
        let shared = self.shared.clone();
        for t in self.tabs.iter_mut() {
            if matches!(t.content, Content::Loading) {
                continue;
            }
            let now = stamp_of(&t.path);
            if now.is_none() || now == t.stamp {
                continue;
            }
            t.stamp = now;
            if let Content::Pdf(d) = &t.content {
                t.keep = Some((d.mode, d.view, d.turn, d.night, d.current(r)));
            }
            t.content = Content::Loading;
            load(t.path.clone(), t.id, &shared.borrow(), waker.clone());
        }
    }

    /// This window known to the others (tabs dropped on it).
    fn register(&mut self) {
        let (Some(id), Some(w)) = (self.me, self.waker.clone()) else { return };
        let mut s = self.shared.borrow_mut();
        if !s.windows.iter().any(|(i, _)| *i == id) {
            s.windows.push((id, w));
        }
    }

    fn next_id(&self) -> u32 {
        let mut s = self.shared.borrow_mut();
        s.next_tab += 1;
        s.next_tab
    }

    /// A file shown once the window can be woken (its `waker` reads it).
    fn queue(&mut self, p: PathBuf) {
        let id = self.next_id();
        self.tabs.push(new_tab(id, p));
        self.front = id;
    }

    fn open_paths(&mut self, paths: &[PathBuf]) {
        for p in paths {
            let p = std::path::absolute(p).unwrap_or_else(|_| p.clone());
            // Already open: brought to the front.
            if let Some(t) = self.tabs.iter().find(|t| t.path == p) {
                self.front = t.id;
                continue;
            }
            let id = self.next_id();
            load(p.clone(), id, &self.shared.borrow(), self.waker.clone());
            self.tabs.push(new_tab(id, p));
            self.front = id;
        }
    }

    fn front_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == self.front).or(self.tabs.first())
    }

    fn front_mut(&mut self) -> Option<&mut Tab> {
        let id = self.front_tab()?.id;
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// The front document, if it is open.
    fn pdf_mut(&mut self) -> Option<&mut pdf::Pdf> {
        match &mut self.front_mut()?.content {
            Content::Pdf(d) => Some(d),
            _ => None,
        }
    }

    fn is_pdf(&self) -> bool {
        self.front_tab().is_some_and(|t| matches!(t.content, Content::Pdf(_)))
    }

    /// Saves the front document's page and zoom soon (once they stop changing).
    fn place_soon(&self, window: &mut Window) {
        if self.is_pdf() {
            window.wake_after(PLACE_TIMER, std::time::Duration::from_millis(700));
        }
    }

    fn save_place(&mut self) {
        let r = self.area.get();
        let Some(t) = self.front_tab() else { return };
        let Content::Pdf(d) = &t.content else { return };
        let (page, mode) = (d.current(r), d.mode);
        let key = (t.path.clone(), page, format!("{mode:?}"));
        if self.saved_place.as_ref() != Some(&key) {
            pdf::places::save(&t.path, page, mode);
            self.saved_place = Some(key);
        }
    }

    /// Tries the typed password on a locked document; opens it if it is right.
    fn unlock(&mut self) {
        let waker = self.waker.clone();
        let Some(t) = self.front_mut() else { return };
        let Content::Locked { viewer, password, wrong } = &mut t.content else { return };
        let typed = std::mem::take(password);
        let Some(mut v) = viewer.take() else { return };
        match v.unlock(typed.as_bytes()) {
            Ok(true) => {
                t.content = match pdf::start(v, waker) {
                    Ok(d) => Content::Pdf(with_place(d, &t.path)),
                    Err(why) => Content::Problem { why, elsewhere: false },
                }
            }
            Ok(false) => {
                *viewer = Some(v);
                *wrong = true;
            }
            Err(e) => t.content = Content::Problem { why: e.to_string(), elsewhere: false },
        }
    }

    /// A document command (zoom, pages, text); false if none is open or the
    /// command is not one of these.
    fn pdf_run(&mut self, id: u32) -> bool {
        let r = self.area.get();
        let Some(d) = self.pdf_mut() else { return false };
        let z = d.zoom(r);
        match id {
            cmd::ZOOM_IN => d.set_mode(pdf::Mode::Zoom(z * 1.25), None, r),
            cmd::ZOOM_OUT => d.set_mode(pdf::Mode::Zoom(z / 1.25), None, r),
            cmd::FIT => d.set_mode(pdf::Mode::Width, None, r),
            cmd::FIT_PAGE => d.set_mode(pdf::Mode::Page, None, r),
            cmd::ACTUAL => d.set_mode(pdf::Mode::Zoom(1.0), None, r),
            cmd::NEXT => {
                let c = d.current(r);
                d.go_to(c + 1, r);
            }
            cmd::PREVIOUS => {
                let c = d.current(r);
                d.go_to(c.saturating_sub(1), r);
            }
            cmd::SIDEBAR => self.sidebar = !self.sidebar,
            cmd::TURN_RIGHT => {
                let t = (d.turn + 90) % 360;
                d.set_turn(t, r);
            }
            cmd::TURN_LEFT => {
                let t = (d.turn + 270) % 360;
                d.set_turn(t, r);
            }
            cmd::VIEW_CONTINUOUS => d.set_view(pdf::View::Continuous, r),
            cmd::VIEW_SINGLE => d.set_view(pdf::View::Single, r),
            cmd::VIEW_TWO => d.set_view(pdf::View::Two, r),
            cmd::NIGHT => d.night = !d.night,
            cmd::BACK | cmd::FORWARD => {
                self.go_back(id == cmd::FORWARD);
                self.follow_thumb();
                return true;
            }
            cmd::GO_TO_PAGE => {
                self.page_key += 1;
                self.page_focus = true;
                self.page_field = None;
                return true;
            }
            cmd::COPY | cmd::SELECT_ALL | cmd::FIND | cmd::FIND_NEXT | cmd::FIND_PREVIOUS => return self.pdf_text_command(id),
            _ => return false,
        }
        self.follow_thumb();
        true
    }

    /// Copy, Select All and Find.
    fn pdf_text_command(&mut self, id: u32) -> bool {
        // The first match gone to: where reading was is kept for Back.
        if matches!(id, cmd::FIND_NEXT | cmd::FIND_PREVIOUS) && self.front_tab().is_some_and(|t| matches!(&t.content, Content::Pdf(d) if d.found.is_none())) {
            self.mark();
        }
        let r = self.area.get();
        let finding = self.finding.is_some();
        let (mut copy, mut find) = (None, None);
        let Some(d) = self.pdf_mut() else { return false };
        match id {
            cmd::COPY => copy = d.selected_text(),
            cmd::SELECT_ALL => {
                let c = d.current(r);
                d.select_page(c);
            }
            cmd::FIND => {
                if !finding {
                    // Starts with the selection's text when it is short.
                    let q = d.selected_text().filter(|t| t.len() <= 80 && !t.contains('\n')).unwrap_or_default();
                    d.start_search(&q);
                    find = Some(q);
                }
            }
            cmd::FIND_NEXT | cmd::FIND_PREVIOUS => {
                let step = if id == cmd::FIND_NEXT { 1 } else { -1 };

                let next = match d.found {
                    Some(i) => i as isize + step,
                    // The first match at or after the place in view.
                    None => {
                        let c = d.current(r);
                        let se = d.shared.search.lock().unwrap_or_else(|e| e.into_inner());
                        let i = se.matches.iter().position(|m| m.0 >= c).unwrap_or(0) as isize;
                        drop(se);
                        if step < 0 { i - 1 } else { i }
                    }
                };
                d.show_match(next, r);
            }
            _ => return false,
        }
        if copy.is_some() {
            self.copy_out = copy;
        }
        if find.is_some() {
            self.finding = find;
        }
        true
    }

    /// The sidebar's thumbnails kept on the current page.
    fn follow_thumb(&mut self) {
        let (r, tr) = (self.area.get(), self.thumb_area.get());
        if let Some(d) = self.pdf_mut() {
            let c = d.current(r);
            d.show_thumb(c, tr);
        }
    }

    /// A link followed: a place in the document, or an address asked about.
    fn follow(&mut self, action: vergence_pdf::outline::Action) {
        let r = self.area.get();
        match action {
            vergence_pdf::outline::Action::GoTo(t) => {
                self.mark();
                if let Some(d) = self.pdf_mut() {
                    d.go_to_target(t, r);
                }
                self.follow_thumb();
            }
            vergence_pdf::outline::Action::Uri(u) => self.pending_uri = Some(u),
        }
    }

    /// The wheel and keys: the wheel scrolls (Ctrl: zooms about the
    /// pointer); arrows, Page Up/Down and Space scroll; Home and End go to
    /// the first and last page; Left and Right go a page when it fits whole.
    fn pdf_input(&mut self, input: &Input, window: &mut Window) -> bool {
        let r = self.area.get();
        let ctrl = self.ctrl;
        if self.pdf_mut().is_none() {
            return false;
        }
        match input {
            Input::Scroll { x, y, dx, dy } => {
                let tr = self.thumb_area.get();
                if self.sidebar && self.side == Side::Pages && *x >= tr.x && *x <= tr.x + tr.w && *y >= tr.y && *y <= tr.y + tr.h {
                    let Some(d) = self.pdf_mut() else { return false };
                    d.thumb_top += dy;
                    d.clamp_thumbs(tr);
                    window.redraw();
                    return true;
                }
                let Some(d) = self.pdf_mut() else { return false };
                if *x < r.x || *x > r.x + r.w || *y < r.y || *y > r.y + r.h {
                    return false;
                }
                if ctrl {
                    let z = d.zoom(r);
                    let k = 1.0015f32.powf(-dy.clamp(-200.0, 200.0) * 10.0);
                    d.set_mode(pdf::Mode::Zoom(z * k), Some(*y), r);
                } else {
                    d.scroll(*dy, *dx, r);
                }
            }
            Input::Key { sym, pressed: true, modifiers: m, text, .. } if !m.ctrl && !m.alt => {
                if text.as_deref() == Some("s") {
                    self.sidebar = !self.sidebar;
                    window.redraw();
                    return true;
                }
                if text.as_deref() == Some("n") {
                    self.pdf_run(cmd::NIGHT);
                    window.redraw();
                    return true;
                }
                let Some(d) = self.pdf_mut() else { return false };
                const LEFT: u32 = 0xff51;
                const UP: u32 = 0xff52;
                const RIGHT: u32 = 0xff53;
                const DOWN: u32 = 0xff54;
                const PAGE_UP: u32 = 0xff55;
                const PAGE_DOWN: u32 = 0xff56;
                const HOME: u32 = 0xff50;
                const END: u32 = 0xff57;
                let screen = r.h - 2.0 * pdf::MARGIN - 40.0;
                let command = match (*sym, text.as_deref().unwrap_or("")) {
                    (UP, _) => {
                        d.scroll(-48.0, 0.0, r);
                        None
                    }
                    (DOWN, _) => {
                        d.scroll(48.0, 0.0, r);
                        None
                    }
                    (PAGE_UP, _) => {
                        d.scroll(-screen, 0.0, r);
                        None
                    }
                    (PAGE_DOWN, _) => {
                        d.scroll(screen, 0.0, r);
                        None
                    }
                    (0x20, _) => {
                        d.scroll(if m.shift { -screen } else { screen }, 0.0, r);
                        None
                    }
                    (HOME, _) => {
                        d.go_to(0, r);
                        None
                    }
                    (END, _) => {
                        let last = d.pages.len() - 1;
                        d.go_to(last, r);
                        None
                    }
                    (LEFT | RIGHT, _) if d.page_fits(r) => {
                        let c = d.current(r);
                        d.go_to(if *sym == LEFT { c.saturating_sub(1) } else { c + 1 }, r);
                        None
                    }
                    (LEFT, _) => {
                        d.scroll(0.0, 48.0, r);
                        None
                    }
                    (RIGHT, _) => {
                        d.scroll(0.0, -48.0, r);
                        None
                    }
                    (_, "+" | "=") => Some(cmd::ZOOM_IN),
                    (_, "-") => Some(cmd::ZOOM_OUT),
                    (_, "0") => Some(cmd::FIT),
                    (_, "1") => Some(cmd::ACTUAL),
                    (_, "w" | "W") => Some(cmd::FIT_PAGE),
                    _ => return false,
                };
                if let Some(c) = command {
                    self.pdf_run(c);
                }
            }
            _ => return false,
        }
        self.follow_thumb();
        window.redraw();
        true
    }

    fn open_dialog(&mut self, window: &mut Window) {
        let inbox = self.shared.borrow().inbox.clone();
        let waker = self.waker.clone();
        let dir = self.front_tab().and_then(|t| t.path.parent().map(Path::to_path_buf));
        let mut d = vergence_widgets::filedialog::FileDialog::open(APP_ID, "Open", self.theme(), move |paths| {
            if !paths.is_empty() {
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push(paths);
                if let Some(w) = &waker {
                    w.wake();
                }
            }
        })
        .multiple(true)
        .filters(vec![vergence_widgets::filedialog::Filter::of_types("PDF documents", &["application/pdf"]), vergence_widgets::filedialog::Filter::new("All files", &[])]);
        if let Some(dir) = dir {
            d = d.folder(&dir);
        }
        window.open_window(vergence_widgets::window(d));
    }

    fn open_settings(&mut self, window: &mut Window) {
        if self.prefs_open.get() {
            return;
        }
        let waker = self.waker.clone();
        let prefs = Prefs::new("Reader", APP_ID, SECTION, true, Box::new(ReaderRows), self.prefs_open.clone()).on_write(move || {
            if let Some(w) = &waker {
                w.wake();
            }
        });
        window.open_window(vergence_widgets::window(prefs));
    }

    /// The front file with the app chosen for it (a file that is not a PDF).
    fn open_with(&self) {
        let Some(t) = self.front_tab() else { return };
        let _ = std::process::Command::new("vergence-open").arg(&t.path).spawn();
    }

    fn run(&mut self, id: u32, window: &mut Window) {
        if self.pdf_run(id) {
            if let Some(t) = self.copy_out.take() {
                window.copy(&t);
            }
            self.place_soon(window);
            window.redraw();
            return;
        }
        match id {
            cmd::NEW_WINDOW => {
                let w = Viewer::new(self.shared.clone());
                window.open_window(vergence_widgets::window(w));
            }
            cmd::OPEN => self.open_dialog(window),
            cmd::CLOSE_TAB => {
                let id = self.front_tab().map(|t| t.id);
                match id {
                    Some(id) => self.close_tab(id, window),
                    None => window.close(),
                }
            }
            cmd::OPEN_WITH => self.open_with(),
            cmd::INFO => {
                if let Some(t) = self.front_tab()
                    && let Content::Pdf(d) = &t.content
                {
                    let meta = d.shared.meta.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
                    let w = info::Info::new(&t.path, &d.sizes, &meta, self.theme());
                    window.open_window(vergence_widgets::window(w));
                }
            }
            cmd::SETTINGS => self.open_settings(window),
            cmd::ABOUT => {
                let about = vergence_widgets::about::About::new(APP_ID, "Reader", env!("CARGO_PKG_VERSION"), "PDF documents: pages, the outline, links, find and selection, read and drawn apart from the app by Vergence's own PDF engine.", self.theme());
                window.open_window(vergence_widgets::window(about));
            }
            _ => {}
        }
        window.redraw();
    }

    fn pdf_view(&self, d: &pdf::Pdf) -> Element<Msg> {
        let layout = d.0.clone();
        let scale = self.scale;
        let area = self.area.clone();
        let waker = self.waker.clone();
        let ground = match self.setting("ground").as_deref() {
            Some("light") => Some(Color::rgb(0xe4, 0xe4, 0xe6)),
            Some("window") => None,
            _ => Some(Color::rgb(0x2a, 0x2a, 0x2c)),
        };
        drawing(move |scene, _, r, theme| {
            if area.get() != r {
                area.set(r);
                if let Some(w) = &waker {
                    w.wake();
                }
            }
            layout.draw(scene, r, scale.max(0.25), ground.unwrap_or(theme.page), theme.accent);
        })
        .on_touch(|t| Some(Msg::Touch(t)))
        .name("Pages")
        .value(self.where_now(d))
        .width(Length::FILL)
        .height(Length::FILL)
    }

    /// Where reading is, in words, for the page view's meaning (agents, the
    /// magnifier, a screen reader): "Page 3 of 253, continuous, 129%
    /// (width)", with the view, the turn and the night look when not the
    /// usual.
    fn where_now(&self, d: &pdf::Pdf) -> String {
        let r = self.area.get();
        let c = d.current(r);
        let mut s = match d.view {
            pdf::View::Two if c + 1 < d.pages.len() && c > 0 => format!("Pages {} and {} of {}", c + 1, c + 2, d.pages.len()),
            _ => format!("Page {} of {}", c + 1, d.pages.len()),
        };
        s.push_str(match d.view {
            pdf::View::Continuous => ", continuous",
            pdf::View::Single => ", one page",
            pdf::View::Two => ", two pages",
        });
        let mode = match d.mode {
            pdf::Mode::Width => " (width)",
            pdf::Mode::Page => " (page)",
            pdf::Mode::Zoom(_) => "",
        };
        s.push_str(&format!(", {:.0}%{mode}", d.zoom(r) * 100.0));
        if d.turn != 0 {
            s.push_str(&format!(", turned {}°", d.turn));
        }
        if d.night {
            s.push_str(", night look");
        }
        s
    }

    /// The sidebar: the pages' thumbnails, the outline, or find's results.
    fn sidebar_view(&self, d: &pdf::Pdf) -> Element<Msg> {
        let outline = d.shared.outline.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let has_outline = outline.as_ref().is_some_and(|o| !o.is_empty());
        let searching = self.finding.as_ref().is_some_and(|q| !q.trim().is_empty());
        let side = match self.side {
            Side::Contents if !has_outline => Side::Pages,
            Side::Results if !searching => Side::Pages,
            s => s,
        };
        let contents = side == Side::Contents;
        let tab = |text: &str, on: bool, m: Msg| button(label(text).size(12.0)).look(if on { ButtonLook::Chosen } else { ButtonLook::Plain }).padding(Padding::hv(3.0, 10.0)).on_press(m).name(text);
        let mut tabs = vec![tab("Pages", side == Side::Pages, Msg::SideTab(Side::Pages))];
        if has_outline {
            tabs.push(tab("Contents", contents, Msg::SideTab(Side::Contents)));
        }
        if searching {
            tabs.push(tab("Results", side == Side::Results, Msg::SideTab(Side::Results)));
        }
        let head = panel(row(tabs).spacing(6.0)).radius(0.0).padding(Padding::hv(6.0, 8.0)).width(Length::FILL);
        let body: Element<Msg> = if side == Side::Results {
            let se = d.shared.search.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let items: Vec<Element<Msg>> = se
                .matches
                .iter()
                .zip(se.snippets.iter())
                .enumerate()
                .take(2000)
                .map(|(i, ((page, _), text))| {
                    let look = if d.found == Some(i) { ButtonLook::Chosen } else { ButtonLook::Plain };
                    let head = label(format!("Page {}", page + 1)).size(11.0);
                    let head = if d.found == Some(i) { head } else { head.dim() };
                    let lines = column(vec![head, label(text.clone()).size(12.0).wrap().width(Length::FILL)]).spacing(1.0).width(Length::FILL);
                    let mut b = button(lines).look(look).padding(Padding::hv(4.0, 8.0)).width(Length::FILL).on_press(Msg::Result(i)).name(format!("Page {}: {text}", page + 1));
                    if d.found == Some(i) {
                        b = b.key(2_000_000 + i as u64).keep_in_view();
                    }
                    b
                })
                .collect();
            if items.is_empty() {
                panel(label(if se.done { "Not found" } else { "Searching…" }).size(12.0).dim()).padding(12.0).width(Length::FILL).height(Length::FILL)
            } else {
                scroll(column(items).spacing(1.0).width(Length::FILL))
            }
        } else if contents {
            // The entry for the place in view: the last one starting at or
            // above a quarter of the way down the window.
            let r = self.area.get();
            let (tops, z) = (d.tops(), d.zoom(r));
            let view = d.top + (r.h * 0.25) / z;
            let here = outline.as_ref().and_then(|o| o.iter().rposition(|e| e.target.is_some_and(|t| tops.get(t.page).is_some_and(|y| y + t.top.unwrap_or(0.0) <= view))));
            let items: Vec<Element<Msg>> = outline
                .iter()
                .flat_map(|o| o.iter().enumerate().take(4000))
                .map(|(i, e)| {
                    let indent = 8.0 + 12.0 * e.level.min(6) as f32;
                    let look = if Some(i) == here { ButtonLook::Chosen } else { ButtonLook::Plain };
                    let title = if e.title.is_empty() { "(untitled)".to_string() } else { e.title.clone() };
                    let mut b = button(label(title.clone()).size(12.0).elide().align(Align::Start).width(Length::FILL)).look(look).padding(Padding { top: 3.0, bottom: 3.0, left: indent, right: 8.0 }).width(Length::FILL).name(title);
                    if e.target.is_some() {
                        b = b.on_press(Msg::Outline(i));
                    }
                    if Some(i) == here {
                        // A new key each time the place moves: scrolled into sight.
                        b = b.key(1_000_000 + i as u64).keep_in_view();
                    }
                    b
                })
                .collect();
            scroll(column(items).spacing(1.0).width(Length::FILL))
        } else {
            let layout = d.0.clone();
            let (scale, area, tarea) = (self.scale, self.area.clone(), self.thumb_area.clone());
            let waker = self.waker.clone();
            drawing(move |scene, _, r, theme| {
                if tarea.get() != r {
                    tarea.set(r);
                    if let Some(w) = &waker {
                        w.wake();
                    }
                }
                let current = layout.current(area.get());
                layout.draw_thumbs(scene, r, scale, current, theme.page, theme.accent);
            })
            .on_touch(|t| Some(Msg::ThumbTouch(t)))
            .name("Page thumbnails")
            .value(format!("Page {} of {} marked", d.current(self.area.get()) + 1, d.pages.len()))
            .width(Length::FILL)
            .height(Length::FILL)
        };
        column(vec![head, rule(), body]).width(Length::Fixed(pdf::THUMB_W + 48.0)).height(Length::FILL)
    }

    /// The Find bar: the query, how it is going, previous and next.
    fn find_bar(&self, q: &str, d: &pdf::Pdf) -> Element<Msg> {
        let se = d.shared.search.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let n = se.matches.len();
        let how = if se.query.is_empty() {
            String::new()
        } else if se.done && se.chars_seen == 0 {
            "This PDF has no text to search: its pages are pictures (a scan).".to_string()
        } else if se.done && n == 0 {
            "Not found".to_string()
        } else {
            let at = d.found.map_or(String::new(), |i| format!("{} of ", i + 1));
            let going = if se.done { String::new() } else { format!(" (searching, page {} of {})", se.next, d.pages.len()) };
            format!("{at}{n} found{going}")
        };
        let small = |text: &str, m: Option<Msg>| {
            let b = button(label(text).size(12.0)).padding(Padding::hv(2.0, 10.0)).name(text);
            match m {
                Some(m) => b.on_press(m),
                None => b,
            }
        };
        let row_items: Vec<Element<Msg>> = vec![
            label("Find").size(12.0).dim(),
            field(q.to_string(), Msg::FindText).autofocus().on_submit(Msg::Run(cmd::FIND_NEXT)).name("Find").width(Length::Fixed(260.0)),
            small("Previous", (n > 0).then_some(Msg::Run(cmd::FIND_PREVIOUS))),
            small("Next", (n > 0).then_some(Msg::Run(cmd::FIND_NEXT))),
            label(how).size(12.0).dim().elide().width(Length::FILL),
            small("Done", Some(Msg::FindDone)),
        ];
        // A key of its own: its field is new when the bar opens (and so
        // takes the keyboard), not mistaken for what stood in its place.
        panel(row(row_items).spacing(10.0).align(Align::Center).width(Length::FILL)).radius(0.0).padding(Padding { top: 4.0, bottom: 4.0, left: 12.0, right: 12.0 }).width(Length::FILL).key(0x4649_4e44)
    }

    /// The toolbar: the sidebar; back and forward; the page (a field to
    /// type one into) with previous and next; zoom (out, in, the width, the
    /// whole page, actual size); the views (continuous, one page, two);
    /// turning; Find and Open at the end.
    fn toolbar(&self, t: &Tab, th: &Theme) -> Element<Msg> {
        use vergence_widgets::toolbar::{Icon, flexible, group, tool, toggle, toolbar};
        let Content::Pdf(d) = &t.content else {
            return toolbar(vec![flexible(), tool(Icon::Open, "Open…", Some(Msg::Run(cmd::OPEN)))], th);
        };
        let r = self.area.get();
        let c = d.current(r);
        let shown = self.page_field.clone().unwrap_or_else(|| (c + 1).to_string());
        let mut page = field(shown, Msg::PageField).on_submit(Msg::GoToPage).name("Page number").width(Length::Fixed(48.0)).key(0x5041_4745_0000 + self.page_key);
        if self.page_focus {
            page = page.autofocus();
        }
        let items = vec![
            toggle(Icon::Pane, "Sidebar", self.sidebar, Msg::Run(cmd::SIDEBAR)),
            group(vec![tool(Icon::Back, "Back", (!t.back.is_empty()).then_some(Msg::Run(cmd::BACK))), tool(Icon::Forward, "Forward", (!t.forward.is_empty()).then_some(Msg::Run(cmd::FORWARD)))]),
            group(vec![
                tool(Icon::Previous, "Previous page", (c > 0).then_some(Msg::Run(cmd::PREVIOUS))),
                page,
                label(format!("of {}", d.pages.len())).size(12.0).dim(),
                tool(Icon::Next, "Next page", (c + 1 < d.pages.len()).then_some(Msg::Run(cmd::NEXT))),
            ]),
            group(vec![
                tool(Icon::ZoomOut, "Zoom out", Some(Msg::Run(cmd::ZOOM_OUT))),
                tool(Icon::ZoomIn, "Zoom in", Some(Msg::Run(cmd::ZOOM_IN))),
                toggle(Icon::FitWidth, "Fit to the width", d.mode == pdf::Mode::Width, Msg::Run(cmd::FIT)),
                toggle(Icon::Fit, "The whole page", d.mode == pdf::Mode::Page, Msg::Run(cmd::FIT_PAGE)),
                tool(Icon::Actual, "Actual size", Some(Msg::Run(cmd::ACTUAL))),
            ]),
            group(vec![
                toggle(Icon::Continuous, "Continuous", d.view == pdf::View::Continuous, Msg::Run(cmd::VIEW_CONTINUOUS)),
                toggle(Icon::OnePage, "One page", d.view == pdf::View::Single, Msg::Run(cmd::VIEW_SINGLE)),
                toggle(Icon::TwoPages, "Two pages", d.view == pdf::View::Two, Msg::Run(cmd::VIEW_TWO)),
            ]),
            group(vec![tool(Icon::TurnLeft, "Turn left", Some(Msg::Run(cmd::TURN_LEFT))), tool(Icon::TurnRight, "Turn right", Some(Msg::Run(cmd::TURN_RIGHT)))]),
            flexible(),
            tool(Icon::Info, "Info", Some(Msg::Run(cmd::INFO))),
            toggle(Icon::Search, "Find", self.finding.is_some(), if self.finding.is_some() { Msg::FindDone } else { Msg::Run(cmd::FIND) }),
            tool(Icon::Open, "Open…", Some(Msg::Run(cmd::OPEN))),
        ];
        toolbar(items, th)
    }

    fn status(&self, t: &Tab) -> Element<Msg> {
        let mut items: Vec<Element<Msg>> = vec![label(t.name()).size(12.0).width(Length::FILL)];
        if let Content::Pdf(d) = &t.content {
            let r = self.area.get();
            items.push(label(format!("Page {} of {}", d.current(r) + 1, d.pages.len())).size(12.0).dim());
            let mode = match d.mode {
                pdf::Mode::Width => " (width)",
                pdf::Mode::Page => " (page)",
                pdf::Mode::Zoom(_) => "",
            };
            items.push(label(format!("{:.0}%{mode}", d.zoom(r) * 100.0)).size(12.0).dim());
            if let Some(why) = d.shared.failure(d.current(r)) {
                items.push(label(why).size(12.0).dim());
            }
            if let Some(u) = &self.pending_uri {
                let shown: String = if u.chars().count() > 60 { u.chars().take(57).chain("…".chars()).collect() } else { u.clone() };
                if openable(u) {
                    items.insert(1, label(format!("Open {shown}?")).size(12.0));
                    items.insert(2, button(label("Open").size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(2.0, 10.0)).on_press(Msg::OpenUri).name("Open the address"));
                    items.insert(3, button(label("Cancel").size(12.0)).padding(Padding::hv(2.0, 10.0)).on_press(Msg::CancelUri).name("Cancel"));
                } else {
                    items.insert(1, label(format!("Reader does not open links like {shown}")).size(12.0));
                    items.insert(2, button(label("OK").size(12.0)).padding(Padding::hv(2.0, 10.0)).on_press(Msg::CancelUri).name("OK"));
                }
            }
        }
        panel(row(items).spacing(14.0).align(Align::Center).width(Length::FILL)).radius(0.0).padding(Padding { top: 3.0, bottom: 3.0, left: 12.0, right: 12.0 }).width(Length::FILL)
    }
}

impl Program for Viewer {
    type Message = Msg;

    fn title(&self) -> String {
        self.front_tab().map(Tab::title).unwrap_or_else(|| "Reader".into())
    }

    fn app_id(&self) -> String {
        APP_ID.into()
    }

    /// Dark under App default: pages read best on a dark ground.
    fn theme(&self) -> Theme {
        Theme::from_desktop(&self.desktop, true, SECTION)
    }

    fn bar_style(&self) -> Option<BarStyle> {
        Some(vergence_ui::bar_style_setting(&self.desktop, SECTION, BarStyle::Dark))
    }

    /// The toolbar reaches up under the bar: one band (toolbar.rs).
    fn under_bar(&self) -> bool {
        true
    }

    /// The server's tabs, one document each (Dave: "Ok, tab view"); each
    /// keeps its own view. + opens a document.
    fn bar(&self) -> Option<Bar> {
        let tabs = self.tabs.iter().map(|t| BarTab { id: t.id, title: t.title(), closable: true, alert: false, edited: false }).collect();
        Some(Bar::Tabs { tabs, front: self.front_tab().map_or(0, |t| t.id), plus: true, accent: None, menu: Vec::new() })
    }

    fn tab(&mut self, event: TabEvent, window: &mut Window) {
        match event {
            TabEvent::Chosen(id) => {
                self.front = id;
                self.tab_changed();
            }
            TabEvent::Close(id) => self.close_tab(id, window),
            TabEvent::CloseOthers(id) => {
                self.tabs.retain(|t| t.id == id);
                self.front = id;
            }
            TabEvent::New => self.run(cmd::OPEN, window),
            TabEvent::Moved { tab, to } => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == tab) {
                    let t = self.tabs.remove(i);
                    self.tabs.insert((to as usize).min(self.tabs.len()), t);
                }
            }
            // Out of the row: a window of its own, its view with it.
            TabEvent::Out { tab, .. } => {
                if self.tabs.len() > 1
                    && let Some(t) = self.remove_tab(tab)
                {
                    let mut w = Viewer::new(self.shared.clone());
                    w.front = t.id;
                    w.tabs.push(t);
                    window.open_window(vergence_widgets::window(w));
                    self.tab_changed();
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
        window.redraw();
    }

    fn opened(&mut self, id: u32) {
        self.me = Some(id);
        self.register();
    }

    fn closed(&mut self) {
        self.shared.borrow_mut().windows.retain(|(id, _)| Some(*id) != self.me);
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let Some(tab) = self.front_tab() else {
            return panel(label("Nothing open: Ctrl+O opens a PDF document.").size(13.0).dim()).padding(Padding { top: t.top + 24.0, bottom: 24.0, left: 24.0, right: 24.0 }).width(Length::FILL).height(Length::FILL);
        };
        let body: Element<Msg> = match &tab.content {
            Content::Loading => panel(label("Opening…").size(13.0).dim()).padding(24.0).width(Length::FILL).height(Length::FILL),
            Content::Pdf(d) if self.sidebar => row(vec![self.sidebar_view(d), vrule(), self.pdf_view(d)]).width(Length::FILL).height(Length::FILL),
            Content::Pdf(d) => self.pdf_view(d),
            Content::Locked { password, wrong, .. } => {
                let mut items: Vec<Element<Msg>> = vec![
                    label(format!("“{}” is protected by a password.", tab.name())).size(13.0).wrap().width(Length::FILL),
                    field(password.clone(), Msg::Password).secret().autofocus().placeholder("Password").on_submit(Msg::Unlock).name("Password").width(Length::Fixed(280.0)),
                ];
                if *wrong {
                    items.push(label("That password does not open it.").size(12.0).dim());
                }
                items.push(button(label("Open").size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(4.0, 12.0)).on_press(Msg::Unlock).name("Open"));
                panel(column(items).spacing(12.0).width(Length::Fixed(420.0))).padding(24.0).width(Length::FILL).height(Length::FILL)
            }
            Content::Problem { why, elsewhere } => {
                let mut items: Vec<Element<Msg>> = vec![label(why.clone()).size(13.0).wrap().width(Length::FILL)];
                if *elsewhere {
                    items.push(button(label("Open with its app").size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(4.0, 12.0)).on_press(Msg::OpenWith).name("Open with its app"));
                }
                panel(column(items).spacing(12.0).width(Length::Fixed(420.0))).padding(24.0).width(Length::FILL).height(Length::FILL)
            }
        };
        let mut all = vec![self.toolbar(tab, t), panel(body).radius(0.0).width(Length::FILL).height(Length::FILL)];
        if let (Some(q), Content::Pdf(d)) = (&self.finding, &tab.content) {
            all.push(rule());
            all.push(self.find_bar(q, d));
        }
        all.push(rule());
        all.push(self.status(tab));
        column(all).width(Length::FILL).height(Length::FILL)
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        match m {
            Msg::Touch(Touch::Press { x, y }) => {
                // A press on text starts a selection; anywhere else a drag scrolls.
                self.drag = Some((x, y));
                self.press_at = Some(((x, y), false));
                let r = self.area.get();
                let spot = self.front_tab().and_then(|t| match &t.content {
                    Content::Pdf(d) => d.text_at(r, r.x + x, r.y + y),
                    _ => None,
                });
                self.selecting = spot.is_some();
                if let Some(d) = self.pdf_mut() {
                    d.selection = spot.map(|s| pdf::Selection { anchor: s, head: s });
                }
            }
            Msg::Touch(Touch::Move { x, y, pressed }) => {
                let r = self.area.get();
                let moved_far = |press_at: &mut Option<((f32, f32), bool)>| {
                    if let Some(((px, py), moved)) = press_at
                        && ((x - *px).powi(2) + (y - *py).powi(2)).sqrt() > 4.0
                    {
                        *moved = true;
                    }
                };
                if pressed && self.selecting {
                    moved_far(&mut self.press_at);
                    if let Some(d) = self.pdf_mut()
                        && let Some(spot) = d.spot_near(r, r.x + x, r.y + y)
                        && let Some(sel) = &mut d.selection
                    {
                        sel.head = spot;
                    }
                } else if pressed && let Some((sx, sy)) = self.drag {
                    if let Some(d) = self.pdf_mut() {
                        d.scroll(sy - y, x - sx, r);
                    }
                    self.drag = Some((x, y));
                    moved_far(&mut self.press_at);
                    self.follow_thumb();
                    self.place_soon(window);
                }
            }
            Msg::Touch(Touch::Release { .. }) => {
                self.drag = None;
                let was_selecting = std::mem::take(&mut self.selecting);
                // A click (no drag) clears a selection begun by it.
                if was_selecting
                    && matches!(self.press_at, Some((_, false)))
                    && let Some(d) = self.pdf_mut()
                {
                    d.selection = None;
                }
                // A click on a page (no drag): a link there is followed.
                if let Some(((x, y), false)) = self.press_at.take() {
                    let r = self.area.get();
                    let action = self.front_tab().and_then(|t| match &t.content {
                        Content::Pdf(d) => d.link_at(r, r.x + x, r.y + y),
                        _ => None,
                    });
                    if let Some(a) = action {
                        self.follow(a);
                        self.place_soon(window);
                    }
                }
            }
            Msg::Touch(Touch::Leave) => {
                self.drag = None;
                self.press_at = None;
                self.selecting = false;
            }
            Msg::Run(c) => return self.run(c, window),
            Msg::Password(p) => {
                if let Some(t) = self.front_mut()
                    && let Content::Locked { password, .. } = &mut t.content
                {
                    *password = p;
                }
            }
            Msg::Unlock => self.unlock(),
            Msg::ThumbTouch(Touch::Press { y, .. }) => {
                self.mark();
                let (r, tr) = (self.area.get(), self.thumb_area.get());
                if let Some(d) = self.pdf_mut()
                    && let Some(p) = d.thumb_at(tr, tr.y + y)
                {
                    d.go_to(p, r);
                }
                self.place_soon(window);
            }
            Msg::ThumbTouch(_) => return,
            Msg::SideTab(side) => self.side = side,
            Msg::Result(i) => {
                if self.front_tab().is_some_and(|t| matches!(&t.content, Content::Pdf(d) if d.found != Some(i))) {
                    self.mark();
                }
                let r = self.area.get();
                if let Some(d) = self.pdf_mut() {
                    d.show_match(i as isize, r);
                }
                self.follow_thumb();
                self.place_soon(window);
            }
            Msg::Outline(i) => {
                let target = self.front_tab().and_then(|t| match &t.content {
                    Content::Pdf(d) => d.shared.outline.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|o| o.get(i)).and_then(|e| e.target),
                    _ => None,
                });
                if let Some(t) = target {
                    self.follow(vergence_pdf::outline::Action::GoTo(t));
                }
                self.place_soon(window);
            }
            Msg::OpenUri => {
                if let Some(u) = self.pending_uri.take()
                    && openable(&u)
                {
                    let _ = std::process::Command::new("vergence-open").arg(u).spawn();
                }
            }
            Msg::CancelUri => self.pending_uri = None,
            Msg::FindText(q) => {
                if let Some(d) = self.pdf_mut() {
                    d.start_search(&q);
                }
                if self.sidebar && !q.trim().is_empty() {
                    self.side = Side::Results;
                }
                self.finding = Some(q);
            }
            Msg::FindDone => {
                self.finding = None;
                if let Some(d) = self.pdf_mut() {
                    d.start_search("");
                }
            }
            Msg::PageField(text) => self.page_field = Some(text.chars().filter(char::is_ascii_digit).take(6).collect()),
            Msg::GoToPage => {
                let r = self.area.get();
                if let Some(n) = self.page_field.take().and_then(|t| t.parse::<usize>().ok()) {
                    self.mark();
                    if let Some(d) = self.pdf_mut() {
                        d.go_to(n.saturating_sub(1), r);
                    }
                }
                // A new field: the keyboard goes back to the pages.
                self.page_key += 1;
                self.page_focus = false;
                self.follow_thumb();
                self.place_soon(window);
            }
            Msg::OpenWith => self.open_with(),
        }
        window.redraw();
    }

    fn input(&mut self, input: &Input, window: &mut Window) -> bool {
        if let Input::Key { sym, pressed, modifiers, .. } = input {
            self.ctrl = if matches!(*sym, 0xffe3 | 0xffe4) { *pressed } else { modifiers.ctrl };
        }
        if let Input::Focus(false) = input {
            self.ctrl = false;
        }
        if self.front_tab().is_some_and(|t| matches!(t.content, Content::Locked { .. })) {
            return false;
        }
        if let Input::Key { sym, pressed: true, modifiers: m, .. } = input {
            const ESCAPE: u32 = 0xff1b;
            const F3: u32 = 0xffc0;
            const KP_ADD: u32 = 0xffab;
            const KP_SUB: u32 = 0xffad;
            let c = match *sym {
                0x63 | 0x43 if m.ctrl => Some(cmd::COPY),
                0x61 | 0x41 if m.ctrl => Some(cmd::SELECT_ALL),
                0x66 | 0x46 if m.ctrl => Some(cmd::FIND),
                0x67 | 0x47 if m.ctrl => Some(if m.shift { cmd::FIND_PREVIOUS } else { cmd::FIND_NEXT }),
                0x6f | 0x4f if m.ctrl => Some(cmd::OPEN),
                0x6e | 0x4e if m.ctrl => Some(cmd::NEW_WINDOW),
                0x77 | 0x57 if m.ctrl => Some(cmd::CLOSE_TAB),
                0x2c if m.ctrl => Some(cmd::SETTINGS),
                0x3d | 0x2b | KP_ADD if m.ctrl => Some(cmd::ZOOM_IN),
                0x2d | KP_SUB if m.ctrl => Some(cmd::ZOOM_OUT),
                0x30 if m.ctrl => Some(cmd::FIT),
                0x67 | 0x47 if m.ctrl && m.alt => Some(cmd::GO_TO_PAGE),
                0x72 | 0x52 if m.ctrl => Some(cmd::TURN_RIGHT),
                0x69 | 0x49 if m.ctrl => Some(cmd::INFO),
                0x6c | 0x4c if m.ctrl => Some(cmd::TURN_LEFT),
                0x5b if m.ctrl => Some(cmd::BACK),
                0x5d if m.ctrl => Some(cmd::FORWARD),
                0xff51 if m.alt => Some(cmd::BACK),
                0xff53 if m.alt => Some(cmd::FORWARD),
                F3 => Some(if m.shift { cmd::FIND_PREVIOUS } else { cmd::FIND_NEXT }),
                ESCAPE if self.page_focus => {
                    self.page_key += 1;
                    self.page_focus = false;
                    self.page_field = None;
                    window.redraw();
                    return true;
                }
                ESCAPE if self.is_pdf() => {
                    if self.finding.is_some() {
                        self.update(Msg::FindDone, window);
                    } else if let Some(d) = self.pdf_mut() {
                        d.selection = None;
                        window.redraw();
                    }
                    return true;
                }
                _ => None,
            };
            if let Some(c) = c {
                self.run(c, window);
                return true;
            }
        }
        if self.pdf_input(input, window) {
            self.place_soon(window);
            return true;
        }
        false
    }

    fn waker(&mut self, waker: Waker) {
        // The first window listens for other starts' files.
        let first = self.waker.is_none() && self.shared.borrow().windows.is_empty();
        self.waker = Some(waker.clone());
        self.register();
        // Documents brought in a moved tab wake this window from now on.
        for t in &self.tabs {
            if let Content::Pdf(d) = &t.content {
                d.shared.set_waker(waker.clone());
            }
        }
        if first {
            let inbox = self.shared.borrow().inbox.clone();
            let w = waker.clone();
            let _ = hand::listen(inbox, move || w.wake());
        }
        // More files from the command line wait for windows of their own.
        if !self.shared.borrow().inbox.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
            waker.wake();
        }
        // Files asked for before the window could wake (the command line's).
        let pending: Vec<(u32, PathBuf)> = self.tabs.iter().filter(|t| matches!(t.content, Content::Loading)).map(|t| (t.id, t.path.clone())).collect();
        for (id, path) in pending {
            load(path, id, &self.shared.borrow(), Some(waker.clone()));
        }
    }

    fn woken(&mut self, window: &mut Window) -> bool {
        // Woken by the drawing (its place changed) or a page drawn: look again.
        let mut changed = self.reread() || self.is_pdf();
        let asked: Vec<Vec<PathBuf>> = std::mem::take(&mut *self.shared.borrow().inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for paths in asked {
            self.take(&paths);
            // Handed over by a start (or the Open dialog): forward, if the server agrees.
            window.activate();
            changed = true;
        }
        // Tabs dropped on this window.
        let moved: Vec<(u32, u32, Tab)> = {
            let mut s = self.shared.borrow_mut();
            let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut s.transfers).into_iter().partition(|(into, _, _)| Some(*into) == self.me);
            s.transfers = rest;
            mine
        };
        for (_, to, t) in moved {
            if let (Content::Pdf(d), Some(w)) = (&t.content, &self.waker) {
                d.shared.set_waker(w.clone());
            }
            self.front = t.id;
            self.tabs.insert((to as usize).min(self.tabs.len()), t);
            self.tab_changed();
            changed = true;
        }
        if !self.watching {
            self.watching = true;
            window.wake_after(WATCH_TIMER, WATCH_EVERY);
        }
        let done: Vec<(u32, Loaded)> = {
            let shared = self.shared.borrow();
            let mut all = shared.loaded.lock().unwrap_or_else(|e| e.into_inner());
            let (mine, rest): (Vec<_>, Vec<_>) = all.drain(..).partition(|(id, _)| self.tabs.iter().any(|t| t.id == *id));
            *all = rest;
            mine
        };
        for (id, loaded) in done {
            if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                t.content = match loaded {
                    Loaded::Pdf(d) => Content::Pdf(with_place(d, &t.path)),
                    Loaded::Locked(v) => Content::Locked { viewer: Some(v), password: String::new(), wrong: false },
                    Loaded::Problem { why, elsewhere } => Content::Problem { why, elsewhere },
                };
                changed = true;
            }
        }
        // A document read again: its view put back.
        let r = self.area.get();
        if r.w > 1.0 {
            for t in self.tabs.iter_mut() {
                if let (Content::Pdf(d), Some((mode, view, turn, night, page))) = (&mut t.content, t.keep) {
                    d.restore = None;
                    d.mode = mode;
                    d.view = view;
                    d.night = night;
                    d.set_turn(turn, r);
                    d.go_to(page, r);
                    t.keep = None;
                    changed = true;
                }
            }
        }
        // A remembered page, gone to once the view has its place.
        if r.w > 1.0
            && let Some(d) = self.pdf_mut()
            && let Some(page) = d.restore.take()
        {
            d.go_to(page, r);
            changed = true;
        }
        changed
    }

    fn timer(&mut self, id: u64, window: &mut Window) -> bool {
        if id == PLACE_TIMER {
            self.save_place();
        }
        if id == WATCH_TIMER {
            self.watch();
            window.wake_after(WATCH_TIMER, WATCH_EVERY);
        }
        false
    }

    /// Files dropped here open in tabs.
    fn drag_over(&mut self, _x: f32, _y: f32, kinds: &[String], _ctrl: bool) -> u8 {
        if kinds.iter().any(|k| k == "text/uri-list") { vergence_ui::DROP_COPY } else { vergence_ui::DROP_NONE }
    }

    fn dropped(&mut self, _x: f32, _y: f32, _action: u8, items: &[(String, Vec<u8>)], window: &mut Window) {
        let paths: Vec<PathBuf> = items.iter().find(|(k, _)| k == "text/uri-list").map(|(_, v)| String::from_utf8_lossy(v).lines().filter_map(|l| vergence_types::path_of_link(l.trim())).collect()).unwrap_or_default();
        self.take(&paths);
        window.redraw();
    }

    fn scale(&mut self, scale: f32) -> bool {
        self.scale = scale;
        true
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        let mut list = vec![C::role(cmd::OPEN, Role::Open).featured(), C::role(cmd::NEW_WINDOW, Role::NewWindow), C::role(cmd::CLOSE_TAB, Role::CloseWindow)];
        if let Some(Content::Pdf(d)) = self.front_tab().map(|t| &t.content) {
            list.extend([
                C::role(cmd::ZOOM_IN, Role::ZoomIn).featured(),
                C::role(cmd::ZOOM_OUT, Role::ZoomOut).featured(),
                C::own(cmd::FIT, "View", "Fit to the width", "0").checked(d.mode == pdf::Mode::Width),
                C::own(cmd::FIT_PAGE, "View", "The whole page", "W").checked(d.mode == pdf::Mode::Page),
                C::own(cmd::SIDEBAR, "View", "Sidebar", "S").checked(self.sidebar).featured(),
                C::role(cmd::COPY, Role::Copy),
                C::role(cmd::SELECT_ALL, Role::SelectAll),
                C::role(cmd::FIND, Role::Find).featured(),
                C::role(cmd::FIND_NEXT, Role::FindNext),
                C::own(cmd::ACTUAL, "View", "Actual size", "1"),
                C::own(cmd::NEXT, "Go", "Next page", "Right").featured(),
                C::own(cmd::PREVIOUS, "Go", "Previous page", "Left"),
                C::own(cmd::BACK, "Go", "Back", "Ctrl+["),
                C::own(cmd::FORWARD, "Go", "Forward", "Ctrl+]"),
                C::own(cmd::GO_TO_PAGE, "Go", "Go to Page…", "Ctrl+Alt+G"),
                C::own(cmd::VIEW_CONTINUOUS, "View", "Continuous", "").checked(d.view == pdf::View::Continuous),
                C::own(cmd::VIEW_SINGLE, "View", "One page", "").checked(d.view == pdf::View::Single),
                C::own(cmd::VIEW_TWO, "View", "Two pages", "").checked(d.view == pdf::View::Two),
                C::own(cmd::TURN_LEFT, "View", "Turn left", "Ctrl+L"),
                C::own(cmd::TURN_RIGHT, "View", "Turn right", "Ctrl+R").featured(),
                C::own(cmd::NIGHT, "View", "Night look", "N").checked(d.night),
                C::own(cmd::INFO, "File", "Info", "Ctrl+I"),
            ]);
        }
        list.extend([C::role(cmd::SETTINGS, Role::Settings), C::role(cmd::ABOUT, Role::About)]);
        Some((false, list))
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        self.run(id, window);
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { min: (360.0, 280.0), ..Default::default() }
    }
}

struct ReaderRows;

impl Rows for ReaderRows {
    fn parts(&self, text: &str) -> Vec<Part> {
        let opacity = vergence_ui::opacity_setting(text, "[appearance]");
        let locked = vergence_ui::desktop_pick(text).map(|light| format!("{} for every app: Settings > Appearance > Style", if light { "Light" } else { "Dark" }));
        let styles = vec![Choice::text("Dark", "dark"), Choice::text("Light", "light")];
        vec![
            Part {
                title: "Reader".into(),
                rows: vec![Row::new("ground", "Around the pages", "What shows between and beside the pages.", Control::Choose { choices: vec![Choice::text("Dark", "dark"), Choice::text("Light", "light"), Choice::text("The window's color", "window")], desktop: None, app_default: 0 })],
            },
            Part {
                title: "On this desktop".into(),
                rows: vec![
                    Row::new("style", "Style", "While the desktop's Style is App default.", Control::Choose { choices: styles.clone(), desktop: Some("Built-in (Dark)".into()), app_default: 0 }).locked(locked.clone()),
                    Row::new("bar-style", "Title bar", "", Control::Choose { choices: styles, desktop: Some("Built-in (Dark)".into()), app_default: 0 }).locked(locked),
                    Row::new("opacity", "Opacity", "30 to 100: how much of the window's ground shows.", Control::Percent { min: 30, desktop: format!("Desktop default ({:.0})", opacity * 100.0), now: (opacity * 100.0).round() as u32 }),
                    Row::new("", "Window places", "Where the desktop puts Reader's windows, for each set of screens.", Control::Button { label: "Forget…".into(), id: 1 }),
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
        println!("vergence-reader {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let files: Vec<PathBuf> = args.iter().filter(|a| !a.starts_with("--")).map(|a| vergence_types::path_of_link(a).unwrap_or_else(|| PathBuf::from(a))).map(|p| std::path::absolute(&p).unwrap_or(p)).collect();
    // One Reader for all its windows: a running one takes the files.
    if hand::hand_over(&files) {
        return;
    }
    let shared = Rc::new(RefCell::new(Shared { inbox: Arc::default(), loaded: Arc::default(), next_tab: 0, windows: Vec::new(), transfers: Vec::new() }));
    let mut viewer = Viewer::new(shared);
    // The files, each in a tab, read once the window can be woken (waker()).
    for p in files {
        viewer.queue(p);
    }
    if let Err(e) = vergence_widgets::run(viewer, (980, 760)) {
        eprintln!("vergence-reader: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::openable;

    #[test]
    fn real_titles_and_file_names() {
        use super::title_is_real;
        assert!(title_is_real("Log4OM 2 User Manual"));
        assert!(!title_is_real("Microsoft Word - manual.docx"));
        assert!(!title_is_real("report.pdf"));
        assert!(!title_is_real("Untitled"));
        assert!(!title_is_real(" "));
    }

    #[test]
    fn links_open_only_known_addresses() {
        assert!(openable("https://example.org/x"));
        assert!(openable("MAILTO:someone@example.org"));
        assert!(!openable("file:///etc/passwd"));
        assert!(!openable("javascript:alert(1)"));
    }
}
