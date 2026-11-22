//! Reader (Reader milestone 1; docs/reader-milestone-1.md): PDF documents
//! in an app of their own, split from Close-Up, since a PDF wants a
//! different window from a picture. Close-Up keeps pictures and hands PDFs here.
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

mod commands;
mod hand;
mod info;
mod keep;
mod keys;
mod pdf;
mod settings;
mod sidebar;
mod tab;
mod tool;
mod update;
mod view;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use vergence_scene::Rect;
use vergence_ui::{AppCommand as C, Bar, BarStyle, BarTab, TabEvent};
use vergence_widgets::{Element, Input, Program, Theme, Touch, Waker, Window};

use commands::cmd;
use sidebar::Side;
use tab::{Content, Loaded, Tab, load};
use tool::hand_remembered;

const APP_ID: &str = "vergence-reader";
const SECTION: &str = "[reader]";
/// A document's page and zoom saved a moment after they stop changing.
const PLACE_TIMER: u64 = 2;
/// Open files looked at for changes this often (as Edit does).
const WATCH_TIMER: u64 = 3;
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

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
    /// What each window keeps for the next start (keep.rs), by its id; and
    /// the windows kept from the last run still to open.
    snaps: Vec<(u32, keep::Snap)>,
    pending: Vec<keep::Snap>,
}

type SharedRef = Rc<RefCell<Shared>>;

/// ~/.config/vergence/desktop.toml.
fn desktop_text() -> (String, Option<SystemTime>) {
    match vergence_widgets::prefs::desktop_path() {
        Some(p) => {
            (std::fs::read_to_string(&p).unwrap_or_default(), std::fs::metadata(&p).and_then(|m| m.modified()).ok())
        }
        None => (String::new(), None),
    }
}

struct Viewer {
    shared: SharedRef,
    /// When the session said it is ending (an exit closes the window next):
    /// the close is then the session's, and the window stays kept.
    ending: Option<std::time::Instant>,
    /// This window's id with the server, once it has one.
    me: Option<u32>,
    /// Its token, for window memory to put it back (keep.rs).
    token: String,
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
    /// A scroll bar held: which, and where on its thumb it was taken
    /// (window points from the thumb's start).
    bar_drag: Option<(pdf::Bar, f32)>,
    /// The thumbnail strip's bar held, and where on its thumb.
    thumb_bar_drag: Option<f32>,
    /// The Hand: a drag always moves the pages (else text is selected).
    hand: bool,
    /// The pointer is over a scroll bar (the arrow there).
    over_bar: bool,
    /// The pointer is over a link: the pointing hand, as Preview and
    /// browsers show.
    over_link: bool,
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
            ending: None,
            me: None,
            token: String::new(),
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
            bar_drag: None,
            thumb_bar_drag: None,
            hand: hand_remembered(),
            over_bar: false,
            over_link: false,
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

    /// A window kept from the last run: its token and its documents.
    fn from_snap(shared: SharedRef, snap: &keep::Snap) -> Viewer {
        let mut v = Viewer::new(shared);
        v.token = snap.token.clone();
        for t in &snap.tabs {
            v.queue(t.path.clone());
            if let Some(last) = v.tabs.last_mut() {
                last.kept = t.at.clone();
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
        if self.token.is_empty() {
            return;
        }
        // Each document with its place: where it is shown, or (not open
        // yet) the place it was kept with.
        let tabs: Vec<keep::KeptTab> = self
            .tabs
            .iter()
            .map(|t| {
                let at = match &t.content {
                    Content::Pdf(d) if d.restore.is_none() => {
                        let (page, within) = d.place();
                        Some(keep::Place {
                            page,
                            within: (within * 10.0).round() / 10.0,
                            mode: pdf::places::mode_text(d.mode),
                        })
                    }
                    Content::Pdf(d) => d.restore.map(|(page, within)| keep::Place {
                        page,
                        within,
                        mode: pdf::places::mode_text(d.mode),
                    }),
                    _ => t.kept.clone(),
                };
                keep::KeptTab { path: t.path.clone(), at }
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

    /// This window known to the others (tabs dropped on it).
    fn register(&mut self) {
        let (Some(id), Some(w)) = (self.me, self.waker.clone()) else { return };
        let mut s = self.shared.borrow_mut();
        if !s.windows.iter().any(|(i, _)| *i == id) {
            s.windows.push((id, w));
        }
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
        // The place within the window kept too, for a restart.
        self.keep();
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

    /// Light under App default, with a dark bar, which helps the app stand
    /// out a bit; the ground around the pages stays dark.
    /// Always solid, with no opacity control: a see-through window helps no
    /// one reading, whatever the desktop's opacity.
    fn theme(&self) -> Theme {
        let mut t = Theme::from_desktop(&self.desktop, false, SECTION);
        t.opacity = 1.0;
        t
    }

    fn bar_style(&self) -> Option<BarStyle> {
        Some(vergence_ui::bar_style_setting(&self.desktop, SECTION, BarStyle::Dark))
    }

    /// The toolbar reaches up under the bar: one band (toolbar.rs).
    fn under_bar(&self) -> bool {
        true
    }

    /// The server's tabs, one document each (see docs/reader-milestone-1.md); each
    /// keeps its own view. + opens a document.
    fn bar(&self) -> Option<Bar> {
        let tabs = self
            .tabs
            .iter()
            .map(|t| BarTab { id: t.id, title: t.title(), closable: true, alert: false, edited: false })
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

    fn opened(&mut self, id: u32) {
        self.me = Some(id);
        self.register();
    }

    /// Its token from the last run, so window memory puts it back; "new": ask for one.
    fn token(&self) -> String {
        if self.token.is_empty() { "new".into() } else { self.token.clone() }
    }

    fn token_given(&mut self, token: &str) {
        self.token = token.to_string();
        self.keep();
    }

    fn session_ending(&mut self) {
        self.ending = Some(std::time::Instant::now());
        // Its place as it is now, for the next session.
        self.keep();
    }

    fn closed(&mut self) {
        let mut s = self.shared.borrow_mut();
        s.windows.retain(|(id, _)| Some(*id) != self.me);
        // Closed for an exit: kept for the next session (bus v33).
        if vergence_widgets::session::ending(self.ending) {
            return;
        }
        // Closed by the user: it is not kept.
        s.snaps.retain(|(id, _)| Some(*id) != self.me);
        let all: Vec<keep::Snap> = s.snaps.iter().map(|(_, w)| w.clone()).collect();
        keep::save(&all);
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        self.on_view(t)
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        self.on_update(m, window)
    }

    fn input(&mut self, input: &Input, window: &mut Window) -> bool {
        self.on_input(input, window)
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
        let pending: Vec<(u32, PathBuf)> = self
            .tabs
            .iter()
            .filter(|t| matches!(t.content, Content::Loading))
            .map(|t| (t.id, t.path.clone()))
            .collect();
        for (id, path) in pending {
            load(path, id, &self.shared.borrow(), Some(waker.clone()));
        }
    }

    fn woken(&mut self, window: &mut Window) -> bool {
        self.on_woken(window)
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
        let paths: Vec<PathBuf> = items
            .iter()
            .find(|(k, _)| k == "text/uri-list")
            .map(|(_, v)| {
                String::from_utf8_lossy(v).lines().filter_map(|l| vergence_types::path_of_link(l.trim())).collect()
            })
            .unwrap_or_default();
        self.take(&paths);
        window.redraw();
    }

    fn scale(&mut self, scale: f32) -> bool {
        self.scale = scale;
        true
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        self.on_commands()
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        self.run(id, window);
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { min: (360.0, 280.0), ..Default::default() }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("vergence-reader {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let files: Vec<PathBuf> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|a| vergence_types::path_of_link(a).unwrap_or_else(|| PathBuf::from(a)))
        .map(|p| std::path::absolute(&p).unwrap_or(p))
        .collect();
    // One Reader for all its windows: a running one takes the files.
    if hand::hand_over(&files) {
        return;
    }
    let shared = Rc::new(RefCell::new(Shared {
        inbox: Arc::default(),
        loaded: Arc::default(),
        next_tab: 0,
        windows: Vec::new(),
        transfers: Vec::new(),
        snaps: Vec::new(),
        pending: Vec::new(),
    }));
    // The windows kept from the last run come back, the first here and the
    // rest once it is open; files named on the command line join the first.
    // `[session] restore = false`: the windows kept are forgotten, not
    // reopened when Reader is next started by hand (session milestone 1).
    let mut kept = if vergence_widgets::session::restore() {
        keep::load()
    } else {
        keep::save(&[]);
        Vec::new()
    };
    let mut viewer = if kept.is_empty() {
        Viewer::new(shared.clone())
    } else {
        let first = kept.remove(0);
        shared.borrow_mut().pending = kept;
        Viewer::from_snap(shared.clone(), &first)
    };
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
    use crate::update::openable;

    #[test]
    fn real_titles_and_file_names() {
        use crate::tab::title_is_real;
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
