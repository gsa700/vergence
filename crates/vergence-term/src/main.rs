//! Vergence's terminal, on the project's own toolkit: tabs, panes, scrollback, selection and
//! xterm's sequences.
//!
//! Toolkit milestone 1. The screen, the escape-sequence parser and the pty (grid.rs, vt.rs,
//! pty.rs) are the ones written for the first terminal, whose interface was built on a borrowed
//! toolkit and retired once this one passed the stress test that one failed. This interface was
//! written beside it under the name `vterm`, which the plan's Progress and the stress test's
//! scripts still use. What it does differently:
//!
//! - **The program's output is parsed on a thread of its own**, into the screen behind a lock.
//!   The window's thread never parses and is never held up by a flood: when the screen has
//!   changed it is woken, asks for a frame, and the frame shows whatever the screen holds when
//!   the monitor is ready for it. However fast the output, at most one frame is drawn for each
//!   the monitor shows.
//! - **The picture is a scene** (vergence-scene), drawn on the GPU or, where that is not to be
//!   had, in software: a terminal's screen is one grid of cells, each with its ground, its ink
//!   and its glyph, found in the font or in the nearest font that has it.
//! - **Keys arrive in order** through the toolkit and go to the program as xterm's sequences.
//!
//! What it has: tabs under a bar of the window's own (vergence-bar, through `vergence_ui::bar`)
//! and panes in a tab (panes.rs: Ctrl+Shift+R side by side, Ctrl+Shift+D one above the other);
//! typing, paste, the scrollback, resizing, titles, the program's own copies (OSC 52); text
//! selected with the mouse (select.rs), the mouse for programs that ask (mouse.rs, Shift keeps it
//! for selecting), a scrollbar (scrollbar.rs), a right-click menu (menu.rs); the fonts, colors
//! and button side from the desktop's settings, followed while it runs (settings.rs). Ctrl+C
//! copies when text is selected and is the interrupt key when none is; Ctrl+V pastes.
//!
//! Uses vergence-ui and vergence-widgets (window, settings window), vergence-scene,
//! vergence-text and vergence-render (drawing), vergence-look, vergence-sysinfo.
//!
//! Start reading: this file (the app and its loop), session.rs (one terminal: the parsing thread
//! and its screen), grid.rs and vt.rs (the screen and the parser), draw.rs (a frame), then
//! input.rs.
//!
//! Rules: the window's thread never parses the program's output. Random escape sequences must
//! never panic the parser (fuzz.rs runs with `cargo test`).

mod commands;
mod confirm;
mod draw;
#[cfg(test)]
mod fuzz;
#[allow(dead_code)]
mod grid;
mod input;
mod meaning;
mod menu;
mod mouse;
mod palette;
mod panes;
mod pointer;
mod prefs;
#[allow(dead_code)]
mod program;
mod pty;
mod scrollbar;
mod select;
mod session;
mod settings;
mod tabs;
#[allow(dead_code)]
mod vt;

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use draw::Metrics;
use menu::{Item, Menu};
use palette::{Palette, palette};
use panes::{Axis, Node};
use select::Pos;
use session::{NEWS_DRAW, NEWS_STATE, Tab, lock};
use settings::{CursorShape, Settings, config, scrollback_of, set_scroll_on_output, term_flag, watch_settings};
use vergence_scene::{Color, Scene};
use vergence_text::Fonts;
use vergence_ui::bar::{BAR_H, Part};
use vergence_ui::{App, Button, Frame, Input, Waker, Window};

/// Space around the cells, in reference units.
const PAD: f32 = 4.0;
/// The room at the right edge kept for the scrollbar (its width, its margin
/// from the edge, and a little air before the text).
const RIGHT_GUTTER: f32 = scrollbar::WIDTH + 2.0 + 4.0;
/// Two presses on the bar within this are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// While a selection is held past the top or bottom edge, the view moves a line this often.
const EDGE_TICK: Duration = Duration::from_millis(40);
/// Lines the view moves for one notch of the wheel.
const WHEEL_LINES: isize = 3;
/// The terminal's timers (`Window::wake_after`).
const TIMER_LONG_PRESS: u64 = 1;
const TIMER_FLICK: u64 = 2;
/// Once a second: has a tab's program become the superuser, or stopped
/// being (look milestone 1)? A frame, and the bar told, only if one has.
const TIMER_ROOT: u64 = 3;
const ROOT_TICK: Duration = Duration::from_secs(1);
/// A flick: the least speed that throws the text (units a second), the
/// most, how fast it slows (a fraction of its speed lost a second, as an
/// exponent), where it stops, and how often it moves the text.
const FLICK_MIN: f32 = 300.0;
const FLICK_MAX: f32 = 8000.0;
const FLICK_FRICTION: f32 = 3.0;
const FLICK_STOP: f32 = 40.0;
const FLICK_TICK: std::time::Duration = std::time::Duration::from_millis(16);

/// What the pointer has hold of, from a press to its release.
enum Held {
    /// The text: the selection follows the pointer.
    Text,
    /// The scrollbar's thumb, and where in it it was taken hold of.
    Thumb(f32),
    /// The line between two panes (the split it belongs to).
    Divider(Vec<bool>),
    /// A button the program in the active pane was told is down.
    Reported(Button),
}

struct Terminal {
    /// The program's windows and what it keeps of them (program.rs), shared
    /// by every window of this process; this window's key there.
    program: Rc<RefCell<program::Program>>,
    key: u64,
    /// The token to present when the window is created (one kept from an
    /// earlier run), and the one the server gave.
    present: String,
    token: String,
    /// Requests from later starts of vergence-term (program.rs), for whichever window hears of them first.
    inbox: Arc<Mutex<Vec<program::OpenRequest>>>,
    /// The first window of the process: it listens for later starts.
    primary: bool,
    /// The window is on the native road (it was given a token).
    native: bool,
    /// Where the first tabs' programs start (restored tabs), and where the
    /// rest do (a window opened from a request); none: where we stand.
    start_dirs: Vec<String>,
    start_cwd: Option<String>,
    /// The toolkit's number for this window (App::opened): the server's tab
    /// events name other windows by it.
    wid: u32,
    /// Tabs this window opens with, brought from another window (a tab torn
    /// off, a section moved to a window of its own), and where it asks to be.
    moved: Vec<Tab>,
    ask: Option<vergence_ui::BarPlace>,
    /// The size a window opened for a tab or section asks for: the one it left's.
    ask_size: Option<(u32, u32)>,
    /// Tabs on their way between windows of this process (a tab joined to
    /// another window): the window's key, the tab, its place there. Shared.
    moves: Rc<RefCell<Vec<(u64, Tab, usize)>>>,
    /// Each window's toolkit number and key, shared.
    wids: Rc<RefCell<Vec<(u32, u64)>>>,
    tabs: Vec<Tab>,
    /// The tab in front.
    shown: usize,

    /// Where the rows start: below our own bar (BAR_H) when we draw it;
    /// below the strip under the server's bar when it draws it and we draw
    /// under it (look milestone 1: only old lines show there); else 0.
    bar_h: f32,
    /// We draw our own bar (other desktops); else the server draws it.
    own_bar: bool,
    /// The superuser check's timer is running.
    root_timer: bool,
    /// The bar as last described to the server: each tab's id and title, the front one.
    bar_told: Option<(Vec<(u32, String, bool)>, u32, u32)>,
    /// The app menu's commands as last sent (app settings milestone 1).
    commands_told: Option<Vec<vergence_ui::AppCommand>>,
    /// The settings window, if open (app settings milestone 1).
    prefs_open: Rc<std::cell::Cell<bool>>,
    /// Its settings: the cursor's shape, whether it blinks, the close question.
    cursor_shape: CursorShape,
    blink: bool,
    ask_close: bool,
    /// The blinking cursor shows now; since when the keys last moved it.
    blink_on: bool,
    /// Text size for a while (app settings milestone 1): the font's size
    /// times this, from the app menu's Text size row or Ctrl+= / Ctrl+- /
    /// Ctrl+0; not kept (the lasting size is the terminal's settings').
    zoom: f32,
    /// The screen's text factor (vergence_ui::Frame::text).
    text: f32,
    /// The bar style it asks for, and the one last told the server.
    bar_style: vergence_ui::BarStyle,
    bar_style_told: Option<vergence_ui::BarStyle>,
    waker: Option<Waker>,
    /// The close question's answer (confirm.rs): close on the next wake.
    close_now: Rc<std::cell::Cell<bool>>,
    /// The close question is showing.
    asking: Rc<std::cell::Cell<bool>>,
    palette: Palette,
    /// How opaque the ground is (vergence_ui::opacity_setting): less than 1,
    /// the desktop shows through behind the text.
    opacity: f32,
    settings: Settings,
    metrics: Option<Metrics>,
    focused: bool,
    /// The window's size at the last frame, and the grid size that fits it: for the bar's layout and new tabs.
    width: f32,
    height: f32,
    fits: (usize, usize),
    /// The window's and the cells' sizes the tabs behind were last fitted to.
    fitted: (f32, f32, f32, f32),
    hover: Option<Part>,
    /// The pane the pointer is over, if it is over one.
    over: Option<u64>,
    maximized: bool,
    bar_press: Option<Instant>,
    held: Option<Held>,
    /// Taps and drags, read from the fingers on the window.
    fingers: vergence_ui::touch::Reader,
    /// How far a finger has dragged that has not yet come to a whole line,
    /// and the lines not yet come to a wheel's notch.
    dragged: f32,
    dragged_lines: isize,
    /// A finger's drag that began on the bar is the pointer's, button held:
    /// where it is now.
    finger_pointer: Option<(f32, f32)>,
    /// Where the finger now dragging came down.
    drag_from: Option<(f32, f32)>,
    /// A flick going on by itself: its speed (units a second), where it
    /// began, and when it last moved the text.
    flick: Option<(f32, (f32, f32), Instant)>,
    /// The last press on the text: when, on which cell, and which in a row it was (1 to 3).
    presses: Option<(Instant, Pos, usize)>,
    /// The selection is held past the top (true) or bottom edge, at this column: the view scrolls.
    edge: Option<(bool, usize)>,
    edge_tick: Instant,
    /// Shift is down: the mouse selects even where a program has asked for it.
    shift: bool,
    /// The pane and screen cell a program was last told the pointer is on.
    reported_cell: Option<(u64, (usize, usize))>,
    /// The right-click menu, while it is open, and the line of it under the pointer.
    menu: Option<Menu>,
    menu_hover: Option<Item>,
    /// A settings file was written since it was last read.
    settings_changed: Arc<AtomicBool>,
    /// How many tabs to open with (`--tabs N`; one unless asked), and how
    /// many panes in the first (`--panes N`), for trying things out.
    open_with: usize,
    open_panes: usize,
    /// `--maximized`: the window is to fill its screen as soon as it is open.
    open_maximized: bool,
    /// `-e PROGRAM ARGS...`: what the first terminal runs instead of the
    /// shell (as xterm's); the tab closes when it ends, like a shell's. The
    /// shelf starts desktop entries that want a terminal (htop) this way.
    run: Option<Vec<String>>,
    /// How many commands the last frame's scene held.
    last_commands: usize,
    /// VTERM_FRAMES is set: frames and tab changes are written to the error output.
    measuring: bool,
    /// The number of the last frame drawn, and a tab asked for and not yet drawn
    /// (which, after which frame, when); kept only while measuring.
    frame_now: u64,
    asked: Option<(usize, u64, Instant)>,
    /// Each pane's scrollback text as last made for the meaning tree.
    scrollback_cache: meaning::ScrollbackCache,
    /// The meaning tree last given to the toolkit (sent again only when it differs).
    meaning_told: Option<vergence_ui::MeaningTree>,
}

impl Terminal {
    fn new() -> Terminal {
        let program = Rc::new(RefCell::new(program::Program::default()));
        let key = program.borrow_mut().add();
        Terminal {
            program,
            key,
            present: String::new(),
            token: String::new(),
            inbox: Arc::new(Mutex::new(Vec::new())),
            primary: false,
            native: false,
            start_dirs: Vec::new(),
            start_cwd: None,
            wid: 0,
            moved: Vec::new(),
            ask: None,
            ask_size: None,
            moves: Rc::new(RefCell::new(Vec::new())),
            wids: Rc::new(RefCell::new(Vec::new())),
            tabs: Vec::new(),
            shown: 0,

            bar_h: BAR_H,
            own_bar: true,
            root_timer: false,
            bar_told: None,
            commands_told: None,
            zoom: 1.0,
            text: 1.0,
            prefs_open: Rc::new(std::cell::Cell::new(false)),
            cursor_shape: CursorShape::of(&config("desktop.toml")),
            blink: term_flag(&config("desktop.toml"), "blink", true),
            ask_close: term_flag(&config("desktop.toml"), "ask-close", true),
            blink_on: true,
            bar_style: vergence_ui::bar_style_setting(
                &config("desktop.toml"),
                "[terminal]",
                vergence_ui::BarStyle::Desktop,
            ),
            bar_style_told: None,

            waker: None,
            close_now: Rc::new(std::cell::Cell::new(false)),
            asking: Rc::new(std::cell::Cell::new(false)),
            palette: palette(),
            opacity: vergence_ui::opacity_setting(&config("desktop.toml"), "[terminal]"),
            settings: Settings::load(),
            metrics: None,
            focused: false,
            width: 820.0,
            height: 520.0,
            fits: (80, 24),
            fitted: (0.0, 0.0, 0.0, 0.0),
            hover: None,
            over: None,
            maximized: false,
            bar_press: None,
            held: None,
            fingers: Default::default(),
            dragged: 0.0,
            dragged_lines: 0,
            finger_pointer: None,
            drag_from: None,
            flick: None,
            presses: None,
            edge: None,
            edge_tick: Instant::now(),
            shift: false,
            reported_cell: None,
            menu: None,
            menu_hover: None,
            settings_changed: Arc::new(AtomicBool::new(false)),
            open_with: 1,
            open_panes: 1,
            open_maximized: false,
            run: None,
            last_commands: 0,
            measuring: std::env::var_os("VTERM_FRAMES").is_some(),
            frame_now: 0,
            asked: None,
            scrollback_cache: Default::default(),
            meaning_told: None,
        }
    }
}

/// The cursor's blink: on and off this long.
const BLINK: Duration = Duration::from_millis(530);
const TIMER_BLINK: u64 = 4;

/// Text size for a while: steps of a tenth, from half to three times.
const ZOOM_STEP: f32 = 0.1;
const ZOOM_MIN: f32 = 0.5;
const ZOOM_MAX: f32 = 3.0;

impl App for Terminal {
    fn title(&self) -> String {
        "Terminal".into()
    }

    fn app_id(&self) -> String {
        "vergence-term".into()
    }

    fn own_frame(&self) -> bool {
        true
    }

    // Bar milestone 1: in a Vergence session the server draws the tabs.
    fn tabs_by_server(&self) -> bool {
        true
    }

    /// The output runs up under the frosted bar (look milestone 1); the
    /// rows start below it.
    fn under_bar(&self) -> bool {
        true
    }

    /// Small enough for a corner, never smaller than a few lines (window
    /// styles milestone 1); no largest size, so it can be maximized.
    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { min: (320.0, 160.0), ..Default::default() }
    }

    // Step 6: the terminal keeps its windows' tokens (program.rs).
    fn token(&self) -> String {
        if self.present.is_empty() { "new".into() } else { self.present.clone() }
    }

    fn token_given(&mut self, token: &str) {
        self.token = token.to_string();
        self.native = true;
        self.keep();
    }

    /// Several tabs, or a program running in one: ask first (confirm.rs).
    fn close_asked(&mut self, window: &mut Window) -> bool {
        if !self.ask_close {
            return true;
        }
        let running: Vec<String> = self.tabs.iter().flat_map(Tab::running).collect();
        let Some((heading, detail)) = confirm::question(self.tabs.len(), &running) else { return true };
        if !self.asking.get() {
            self.asking.set(true);
            let theme = vergence_widgets::Theme::from_desktop(&config("desktop.toml"), true, "[terminal]");
            window.open_window(vergence_widgets::window(confirm::CloseDialog {
                heading,
                detail,
                close: self.close_now.clone(),
                open: self.asking.clone(),
                waker: self.waker.clone(),
                theme,
            }));
        }
        false
    }

    fn closed(&mut self) {
        self.program.borrow_mut().remove(self.key);
        self.wids.borrow_mut().retain(|(_, k)| *k != self.key);
    }

    fn opened(&mut self, id: u32) {
        self.wid = id;
        self.wids.borrow_mut().push((id, self.key));
    }

    fn place(&self) -> vergence_ui::BarPlace {
        self.ask.clone().unwrap_or(vergence_ui::BarPlace::Anywhere)
    }

    fn opening_size(&self) -> Option<(u32, u32)> {
        self.ask_size
    }

    /// The window's tabs and terminals (meaning.rs), when they changed since
    /// last asked. Nothing is acted on yet (step 4).
    fn meaning(&mut self) -> Option<vergence_ui::MeaningTree> {
        let now = self.meaning_now();
        if self.meaning_told.as_ref() == Some(&now) {
            return None;
        }
        self.meaning_told = Some(now.clone());
        Some(now)
    }

    fn bar(&mut self) -> Option<vergence_ui::Bar> {
        let tabs: Vec<(u32, String, bool)> = self.tabs.iter().map(|t| (t.id, t.label(), t.as_root())).collect();
        let front = self.tabs.get(self.shown).map_or(0, |t| t.id);
        // The tab in front is the page's color (look milestone 1): our ground.
        let bg = self.palette.bg;
        let page = (bg.r as u32) << 16 | (bg.g as u32) << 8 | bg.b as u32;
        if self.bar_told.as_ref().is_some_and(|(told, f, a)| *told == tabs && *f == front && *a == page) {
            return None;
        }
        let bar = vergence_ui::Bar::Tabs {
            tabs: tabs
                .iter()
                .map(|(id, title, root)| vergence_ui::BarTab {
                    id: *id,
                    title: title.chars().take(200).collect(),
                    closable: true,
                    alert: *root,
                    edited: false,
                })
                .collect(),
            front,
            plus: true,
            accent: Some(page),
            menu: Vec::new(),
        };
        self.bar_told = Some((tabs, front, page));
        Some(bar)
    }

    /// The app menu (app settings milestone 1): Edit's Copy and Paste, File's
    /// New tab and Close tab, Tab's next and previous, and the terminal's own
    /// splitting and moving in Tab; Copy, Paste, New tab and the two splits
    /// on the first panel. A terminal: roles' Ctrl+letter keys take
    /// Shift (Ctrl+Shift+T), as its shortcuts always have.
    fn commands(&mut self) -> Option<(bool, Vec<vergence_ui::AppCommand>)> {
        self.on_commands()
    }

    fn bar_style(&mut self) -> Option<vergence_ui::BarStyle> {
        (self.bar_style_told != Some(self.bar_style)).then(|| {
            self.bar_style_told = Some(self.bar_style);
            self.bar_style
        })
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        self.on_command(id, window)
    }

    fn tab(&mut self, event: vergence_ui::TabEvent, window: &mut Window) {
        self.on_tab(event, window)
    }

    fn background(&self) -> Color {
        let bg = self.palette.bg;
        Color::rgba(bg.r, bg.g, bg.b, (self.opacity * 255.0).round() as u8)
    }

    fn animating(&self) -> bool {
        // A selection held past the edge scrolls the view frame after frame.
        self.edge.is_some()
    }

    fn waker(&mut self, waker: Waker) {
        self.waker = Some(waker.clone());
        // The first window: later starts hand over to it (program.rs), and on
        // the native road windows kept from the last run are opened.
        if self.primary {
            let w = waker.clone();
            if !program::listen(self.inbox.clone(), move || w.wake()) {
                eprintln!("vergence-term: no socket for later starts; they open windows of their own");
            }
            if !self.program.borrow().restore.is_empty() {
                waker.wake();
            }
        }
        watch_settings(self.settings_changed.clone(), waker);
        // A window opened for tabs from another window starts with them, no new shell.
        if !self.moved.is_empty() {
            self.tabs = std::mem::take(&mut self.moved);
            self.open_with = 0;
        }
        for _ in 0..self.open_with {
            match self.start(self.fits.0, self.fits.1) {
                Some((id, session)) => self.tabs.push(Tab {
                    id: NEXT_TAB.fetch_add(1, Ordering::SeqCst),
                    tree: Node::Leaf(id),
                    panes: vec![(id, session)],
                    active: id,
                }),
                None => std::process::exit(1),
            }
        }
        // The first tab divided: side by side, then each new pane the other way.
        for n in 1..self.open_panes {
            self.split_active(if n % 2 == 1 { Axis::Side } else { Axis::Stack });
        }
        // Its tabs are there now (the first window's token came before them).
        if !self.token.is_empty() {
            self.keep();
        }
    }

    fn timer(&mut self, id: u64, window: &mut Window) {
        self.on_timer(id, window)
    }

    fn woken(&mut self, window: &mut Window) {
        // The close question answered Close.
        if self.close_now.replace(false) {
            window.close();
            return;
        }
        // The superuser check's timer, armed at the first wake (output, settings).
        if !self.root_timer {
            self.root_timer = true;
            window.wake_after(TIMER_ROOT, ROOT_TICK);
        }
        self.open_asked(window);
        self.take_moves(window);
        // Asked once the window has shown a frame: a request made before the
        // server has placed the window is lost to the place it remembers.
        if self.open_maximized && self.last_commands > 0 {
            self.open_maximized = false;
            self.maximized = true;
            window.set_maximized(true);
        }
        if self.settings_changed.swap(false, Ordering::SeqCst) {
            // Colors, fonts and the buttons' side as they are now; the cell's size is
            // worked out again at the next frame, and the grids follow it.
            self.palette = palette();
            self.opacity = vergence_ui::opacity_setting(&config("desktop.toml"), "[terminal]");
            self.bar_style =
                vergence_ui::bar_style_setting(&config("desktop.toml"), "[terminal]", vergence_ui::BarStyle::Desktop);
            let desktop = config("desktop.toml");
            self.cursor_shape = CursorShape::of(&desktop);
            (self.blink, self.ask_close) = (term_flag(&desktop, "blink", true), term_flag(&desktop, "ask-close", true));
            grid::set_scrollback(scrollback_of(&desktop));
            set_scroll_on_output(&desktop);
            if self.blink && self.focused {
                window.wake_after(TIMER_BLINK, BLINK);
            }
            self.settings = Settings::load();
            self.metrics = None;
            window.redraw();
        }
        // Each terminal's news, taken without its screen's lock; only one whose
        // title, clipboard or program's life changed is looked into.
        let shown = self.shown;
        let mut ended = false;
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            let active = tab.active;
            for (id, session) in tab.panes.iter_mut() {
                // "Draw" is left up: the frame that draws the terminal takes it down.
                let news = session.news.fetch_and(!NEWS_STATE, Ordering::SeqCst);
                // Only the tab in front is drawn: output in the others costs no frame.
                if news & NEWS_DRAW != 0 && index == shown {
                    window.redraw();
                }
                if news & NEWS_STATE == 0 {
                    continue;
                }
                let mut s = lock(&session.screen);
                ended |= s.exited;
                if let Some(text) = s.copied.take() {
                    window.copy(&text);
                }
                if s.grid.title != session.title {
                    session.title = s.grid.title.clone();
                    // The bar shows every tab's title (its active pane's); the window carries the front one's.
                    window.redraw();
                    if index == shown && *id == active {
                        window.set_title(if session.title.is_empty() { "Terminal" } else { &session.title });
                    }
                }
            }
        }
        // Panes whose program has ended go; a tab goes with its last pane, the window with its last tab.
        if ended {
            loop {
                let gone = self.tabs.iter().enumerate().find_map(|(index, tab)| {
                    tab.panes.iter().find(|(_, s)| lock(&s.screen).exited).map(|(id, _)| (index, *id))
                });
                let Some((index, id)) = gone else { break };
                self.close_pane(index, id, window);
                if self.tabs.is_empty() {
                    return;
                }
            }
        }
    }

    fn input(&mut self, input: &Input, window: &mut Window) {
        self.on_input(input, window)
    }

    fn scene(&mut self, frame: &Frame, fonts: &mut Fonts) -> Scene {
        self.on_scene(frame, fonts)
    }
}

/// Numbers for terminals (panes) and tabs, unique in the process, so a tab
/// can move from one window to another (bar milestone 1, step 7).
static NEXT_PANE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static NEXT_TAB: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("vergence-term {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    grid::set_scrollback(scrollback_of(&config("desktop.toml")));
    set_scroll_on_output(&config("desktop.toml"));
    let mut terminal = Terminal::new();
    let mut args: Vec<String> = std::env::args().collect();
    // Everything after -e is the program and its arguments, ours or not.
    if let Some(i) = args.iter().position(|a| a == "-e") {
        let run: Vec<String> = args.split_off(i).into_iter().skip(1).collect();
        if run.is_empty() {
            eprintln!("vergence-term: -e wants a program to run");
            std::process::exit(2);
        }
        terminal.run = Some(run);
    }
    if let Some(n) =
        args.iter().position(|a| a == "--tabs").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<usize>().ok())
    {
        terminal.open_with = n.clamp(1, 64);
    }
    if let Some(n) =
        args.iter().position(|a| a == "--panes").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<usize>().ok())
    {
        terminal.open_panes = n.clamp(1, 16);
    }
    terminal.open_maximized = args.iter().any(|a| a == "--maximized");
    // One process for all the terminal's windows (program.rs), on either road
    // since bar milestone 1, step 4b: a later start asks the running one for a
    // window and is done. A terminal started to be measured keeps a process of
    // its own.
    let measured = args.iter().any(|a| a == "--tabs" || a == "--panes") || std::env::var_os("VTERM_FRAMES").is_some();
    let native =
        std::env::var_os("VERGENCE_BUS").is_some() && std::env::var("VERGENCE_UI_WAYLAND").as_deref() != Ok("1");
    if !measured {
        let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        if program::hand_over(&program::OpenRequest { cwd, run: terminal.run.clone().unwrap_or_default() }) {
            return;
        }
        terminal.primary = true;
        // The windows of the last run, unless this start runs a program of its
        // own; native road only: they come back by their tokens, which another
        // desktop has no use for (and its windows, having none, write nothing).
        if native && terminal.run.is_none() {
            let mut kept = program::load();
            if !kept.is_empty() {
                let first = kept.remove(0);
                terminal.open_with = first.tabs.len().max(1);
                terminal.present = first.token;
                terminal.start_dirs = first.tabs;
                terminal.program.borrow_mut().restore = kept;
            }
        }
    }
    if let Err(e) = vergence_ui::run(&mut terminal, (820, 520)) {
        eprintln!("vergence-term: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests;
