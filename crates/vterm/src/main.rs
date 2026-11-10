//! Vergence's terminal on the project's own toolkit (toolkit milestone 1,
//! step 4).
//!
//! The screen, the escape-sequence parser and the pty are the ones the
//! first terminal was built on (vergence-term's grid.rs, vt.rs and pty.rs,
//! which never had the borrowed toolkit in them): they are used from there
//! as they are until that terminal is retired and they move here. What is
//! new is everything around them:
//!
//! - **The program's output is parsed on a thread of its own**, into the
//!   screen behind a lock. The window's thread never parses and is never
//!   held up by a flood: when the screen has changed it is woken, asks for a
//!   frame, and the frame shows whatever the screen holds when the monitor
//!   is ready for it. However fast the output, at most one frame is drawn
//!   for each the monitor shows.
//! - **The picture is a scene** (vergence-scene), drawn on the GPU or, where
//!   that is not to be had, in software: a rectangle for each run of cells
//!   with a background, glyphs placed cell by cell, each found in the font
//!   or in the nearest font that has it.
//! - **Keys arrive in order** through the toolkit and go to the program as
//!   xterm's sequences.
//!
//! So far: terminals in tabs under a bar of the window's own (bar.rs), with
//! the window's buttons on the side the desktop's settings say and tabs
//! that shrink to fit; typing, paste, the scrollback (wheel,
//! Shift+PageUp/PageDown), resizing, titles, the program's own copies (OSC
//! 52), the fonts and colour scheme from the desktop's settings. Ctrl+Shift+T
//! opens a tab, Ctrl+Shift+W closes one, Ctrl+PageUp/PageDown change tab.
//! The mouse selects text (select.rs): a drag by characters, a second press
//! by words, a third by lines, Shift and a press to extend; held past the
//! top or bottom edge the view scrolls under it. Ctrl+Shift+C copies. A
//! program that asked for the mouse gets it (mouse.rs: presses, releases,
//! moves and the wheel, as xterm reports them) unless Shift is held, which
//! keeps the mouse for selecting.
//! A thin scrollbar at the right edge (scrollbar.rs) shows where the view
//! is in the scrollback and can be dragged.
//! The desktop's settings are followed while the terminal runs: the two
//! files are looked at once a second, and a change of fonts, colours or
//! button side shows at the next frame.
//! Panes and the right-click menu come next.

mod bar;
#[allow(dead_code)]
#[path = "../../vergence-term/src/grid.rs"]
mod grid;
mod mouse;
#[allow(dead_code)]
#[path = "../../vergence-term/src/pty.rs"]
mod pty;
mod scrollbar;
mod select;
#[allow(dead_code)]
#[path = "../../vergence-term/src/vt.rs"]
mod vt;

use std::{
    collections::HashMap,
    io::Write,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use bar::{BAR_H, Layout, Part, WinButton};
use grid::{Cell, Color as CellColor, Grid, MouseMode};
use scrollbar::Scrollbar;
use select::{Pos, Selection, Unit};
use vergence_scene::{Color, Glyph, Point, Rect, Scene};
use vergence_text::{FaceId, Fonts};
use vergence_ui::{App, Button, Cursor, Frame, Input, Modifiers, Waker, Window, sym};

/// Space around the cells, in reference units.
const PAD: f32 = 4.0;
/// Two presses on the bar within this are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// What the desktop's settings say that the terminal follows
/// (~/.config/vergence/settings.toml, the file the settings app writes).
struct Settings {
    left: Vec<WinButton>,
    right: Vec<WinButton>,
    ui_family: String,
    ui_size: f32,
    mono_family: String,
    mono_size: f32,
}

/// "Adwaita Mono 10" -> the family and the size in reference units (a point is 96/72 of one).
fn font_name(value: &str, family: &str, points: f32) -> (String, f32) {
    let value = value.trim();
    match value.rsplit_once(' ') {
        Some((name, size)) if size.parse::<f32>().is_ok_and(|s| (4.0..=96.0).contains(&s)) => (name.trim().to_string(), size.parse::<f32>().unwrap_or(points) * 96.0 / 72.0),
        _ if !value.is_empty() => (value.to_string(), points * 96.0 / 72.0),
        _ => (family.to_string(), points * 96.0 / 72.0),
    }
}

fn config_path(name: &str) -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")));
    base.map(|b| b.join("vergence").join(name))
}

fn config(name: &str) -> String {
    config_path(name).and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default()
}

/// Sets `changed` and wakes the window whenever one of the settings files
/// is written (looked at once a second, by their modification times, as
/// the first terminal does; the session bus will tell us instead when the
/// toolkit speaks it).
fn watch_settings(changed: Arc<AtomicBool>, waker: Waker) {
    let stamp = |name: &str| config_path(name).and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
    let stamps = move || [stamp("desktop.toml"), stamp("settings.toml")];
    std::thread::spawn(move || {
        let mut seen = stamps();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let now = stamps();
            if now != seen {
                seen = now;
                changed.store(true, Ordering::SeqCst);
                waker.wake();
            }
        }
    });
}

impl Settings {
    fn from_text(text: &str) -> Settings {
        // Lines of the form "Gtk/FontName" = "Adwaita Sans 10".
        let value = |key: &str| {
            text.lines().find_map(|line| {
                let (k, v) = line.split_once('=')?;
                (k.trim().trim_matches('"') == key).then(|| v.trim().trim_matches('"').to_string())
            })
        };
        let (left, right) = bar::decoration_layout(&value("Gtk/DecorationLayout").unwrap_or_else(|| ":minimize,maximize,close".into()));
        let (ui_family, ui_size) = font_name(&value("Gtk/FontName").unwrap_or_default(), "sans-serif", 10.0);
        let (mono_family, mono_size) = font_name(&value("Gtk/MonospaceFontName").unwrap_or_default(), "monospace", 10.0);
        Settings { left, right, ui_family, ui_size, mono_family, mono_size }
    }
}
/// While a selection is held past the top or bottom edge, the view moves a line this often.
const EDGE_TICK: Duration = Duration::from_millis(40);
/// Lines the view moves for one notch of the wheel.
const WHEEL_LINES: isize = 3;

/// The terminal's colours: its ground and text, the cursor, and the 16 named ones.
struct Palette {
    bg: Color,
    fg: Color,
    accent: Color,
    ansi: [u32; 16],
}

const fn rgb(v: u32) -> Color {
    Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Tango: GNOME Terminal's palette, the look 16-colour programs were tuned against.
const TANGO: Palette = Palette {
    bg: rgb(0x2e3436),
    fg: rgb(0xd3d7cf),
    accent: rgb(0x729fcf),
    ansi: [0x2e3436, 0xcc0000, 0x4e9a06, 0xc4a000, 0x3465a4, 0x75507b, 0x06989a, 0xd3d7cf, 0x555753, 0xef2929, 0x8ae234, 0xfce94f, 0x729fcf, 0xad7fa8, 0x34e2e2, 0xeeeeec],
};

/// Catppuccin Mocha, the first terminal's default.
const CATPPUCCIN: Palette = Palette {
    bg: rgb(0x1e1e2e),
    fg: rgb(0xcdd6f4),
    accent: rgb(0x89b4fa),
    ansi: [0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, 0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8],
};

/// The scheme chosen in desktop.toml (`[terminal] scheme = "tango"`), as the
/// first terminal reads it; Catppuccin if nothing is.
fn palette() -> Palette {
    let text = config("desktop.toml");
    let mut section = "";
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
        } else if section == "[terminal]"
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "scheme"
            && value.trim().trim_matches('"').eq_ignore_ascii_case("tango")
        {
            return TANGO;
        }
    }
    CATPPUCCIN
}

impl Palette {
    fn indexed(&self, i: u8) -> Color {
        match i {
            0..=15 => rgb(self.ansi[i as usize]),
            16..=231 => {
                let n = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Color::rgb(level(n / 36), level((n / 6) % 6), level(n % 6))
            }
            _ => {
                let g = 8 + (i - 232) * 10;
                Color::rgb(g, g, g)
            }
        }
    }

    fn resolve(&self, c: CellColor, default: Color, bold: bool) -> Color {
        match c {
            CellColor::Default => default,
            // Bold makes the first 8 colours bright, as xterm does.
            CellColor::Indexed(i) if bold && i < 8 => self.indexed(i + 8),
            CellColor::Indexed(i) => self.indexed(i),
            CellColor::Rgb(r, g, b) => Color::rgb(r, g, b),
        }
    }
}

/// What the parser's thread and the window's thread share.
struct Screen {
    grid: Grid,
    parser: vt::Parser,
    /// Lines the view is scrolled back from the live screen.
    scroll: usize,
    /// The program has ended.
    exited: bool,
    /// Text the program copied (OSC 52), on its way to the clipboard.
    copied: Option<String>,
    /// The text selected with the mouse.
    selection: Option<Selection>,
}

impl Screen {
    /// The alternate screen has no scrollback to show; else up to all of it.
    fn clamp_scroll(&mut self) {
        let most = if self.grid.alt { 0 } else { self.grid.scrollback.len() };
        self.scroll = self.scroll.min(most);
    }

    fn scroll_by(&mut self, lines: isize) {
        self.scroll = (self.scroll as isize + lines).max(0) as usize;
        self.clamp_scroll();
    }

    /// The line shown at row `r` (from the scrollback while scrolled back).
    fn shown(&self, r: usize) -> &[Cell] {
        self.grid.abs_line(self.top_line() + r).unwrap_or(&[])
    }

    /// The scrollbar in a window of this size, if there is anything to
    /// scroll (the alternate screen has no scrollback).
    fn scrollbar(&self, width: f32, height: f32) -> Option<Scrollbar> {
        let back = if self.grid.alt { 0 } else { self.grid.scrollback.len() };
        Scrollbar::new(Rect::new(0.0, BAR_H, width, height - BAR_H), back, self.grid.rows, self.scroll)
    }

    /// The absolute number of the line shown in the top row.
    fn top_line(&self) -> usize {
        self.grid.pushed - self.scroll
    }

    /// The cell under a point of the window, as a selection counts it;
    /// points outside the cells count as the nearest one.
    fn cell_at(&self, x: f32, y: f32, cell_w: f32, cell_h: f32) -> Pos {
        let (col, row) = self.screen_cell(x, y, cell_w, cell_h);
        (col, self.top_line() + row)
    }

    /// The column and row of the screen under a point of the window (what
    /// mouse reporting tells the program); the nearest, for points outside.
    fn screen_cell(&self, x: f32, y: f32, cell_w: f32, cell_h: f32) -> (usize, usize) {
        let col = ((x - PAD) / cell_w).floor().max(0.0) as usize;
        let row = ((y - BAR_H - PAD) / cell_h).floor().max(0.0) as usize;
        (col.min(self.grid.cols - 1), row.min(self.grid.rows - 1))
    }

    /// Moves the end of the selection being dragged; true if that changed it.
    fn drag_selection(&mut self, pos: Pos) -> bool {
        let Some(mut selection) = self.selection else { return false };
        selection.drag(&self.grid, pos);
        let changed = self.selection != Some(selection);
        self.selection = Some(selection);
        changed
    }
}

/// Text metrics, known once the fonts have been asked.
struct Metrics {
    regular: FaceId,
    bold: FaceId,
    /// The bar's font, if the machine has one.
    ui: Option<FaceId>,
    cell_w: f32,
    cell_h: f32,
    baseline: f32,
}

fn lock(screen: &Mutex<Screen>) -> MutexGuard<'_, Screen> {
    // A panic on the other thread must not take this one with it.
    screen.lock().unwrap_or_else(|e| e.into_inner())
}

/// One terminal: a program, its screen, and the thread that reads it.
struct Session {
    screen: Arc<Mutex<Screen>>,
    pty: pty::Pty,
    title: String,
}

impl Session {
    /// Starts the user's shell on a new terminal of cols x rows. Its output
    /// is read and parsed on a thread of its own, which wakes the window.
    fn start(cols: usize, rows: usize, waker: Waker) -> std::io::Result<Session> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let pty = pty::Pty::spawn(&shell, cols as u16, rows as u16)?;
        let reader = pty.reader()?;
        let mut replies = reader.try_clone()?;
        let screen = Arc::new(Mutex::new(Screen { grid: Grid::new(cols, rows), parser: vt::Parser::new(), scroll: 0, exited: false, copied: None, selection: None }));
        let shared = screen.clone();
        std::thread::spawn(move || {
            pty::read_loop(reader, |bytes| {
                let out = {
                    let mut guard = lock(&shared);
                    let s = &mut *guard;
                    let (before, alt) = (s.grid.pushed, s.grid.alt);
                    s.parser.advance(&mut s.grid, &bytes);
                    if s.grid.alt != alt {
                        // Another screen: what was selected is not on it.
                        s.selection = None;
                    }
                    if s.grid.pushed < before {
                        // A full reset: the line numbers started again.
                        s.selection = None;
                        s.scroll = 0;
                    } else if s.scroll > 0 {
                        // Scrolled back: stay on the same text as new lines arrive.
                        s.scroll += s.grid.pushed - before;
                    }
                    s.clamp_scroll();
                    if let Some(text) = s.grid.clipboard.take() {
                        s.copied = Some(text);
                    }
                    std::mem::take(&mut s.grid.out)
                };
                if !out.is_empty() {
                    let _ = replies.write_all(&out);
                }
                waker.wake();
                true
            });
            lock(&shared).exited = true;
            waker.wake();
        });
        Ok(Session { screen, pty, title: String::new() })
    }

    fn label(&self) -> String {
        if self.title.is_empty() { "Terminal".into() } else { self.title.clone() }
    }
}

struct Terminal {
    sessions: Vec<Session>,
    /// The tab in front.
    shown: usize,
    waker: Option<Waker>,
    palette: Palette,
    settings: Settings,
    metrics: Option<Metrics>,
    focused: bool,
    /// The window's size at the last frame, and the grid size that fits it: for the bar's layout and new tabs.
    width: f32,
    height: f32,
    /// The scrollbar's thumb is held: where in it it was taken hold of.
    thumb: Option<f32>,
    fits: (usize, usize),
    hover: Option<Part>,
    /// True while the pointer is over the terminal, below the bar.
    over_pane: bool,
    maximized: bool,
    bar_press: Option<Instant>,
    /// The left button is held on the text: the selection follows the pointer.
    selecting: bool,
    /// The last press on the text: when, on which cell, and which in a row it was (1 to 3).
    presses: Option<(Instant, Pos, usize)>,
    /// The selection is held past the top (true) or bottom edge, at this column: the view scrolls.
    edge: Option<(bool, usize)>,
    edge_tick: Instant,
    /// Shift is down: the mouse selects even where a program has asked for it.
    shift: bool,
    /// The button the program was told is down, until it is told it is up.
    reported: Option<Button>,
    /// The screen cell the program was last told the pointer is on.
    reported_cell: Option<(usize, usize)>,
    /// A settings file was written since it was last read.
    settings_changed: Arc<AtomicBool>,
    /// How many tabs to open with (`--tabs N`; one unless asked).
    open_with: usize,
}

impl Terminal {
    fn new() -> Terminal {
        Terminal {
            sessions: Vec::new(),
            shown: 0,
            waker: None,
            palette: palette(),
            settings: Settings::from_text(&config("settings.toml")),
            metrics: None,
            focused: false,
            width: 820.0,
            height: 520.0,
            thumb: None,
            fits: (80, 24),
            hover: None,
            over_pane: false,
            maximized: false,
            bar_press: None,
            selecting: false,
            presses: None,
            edge: None,
            edge_tick: Instant::now(),
            shift: false,
            reported: None,
            reported_cell: None,
            settings_changed: Arc::new(AtomicBool::new(false)),
            open_with: 1,
        }
    }

    fn layout(&self) -> Layout {
        Layout::new(self.width, self.sessions.len(), self.shown, &self.settings.left, &self.settings.right)
    }

    fn new_tab(&mut self, window: &mut Window) {
        let Some(waker) = self.waker.clone() else { return };
        match Session::start(self.fits.0, self.fits.1, waker) {
            Ok(s) => {
                self.sessions.push(s);
                self.shown = self.sessions.len() - 1;
            }
            Err(e) => eprintln!("vterm: could not start the shell: {e}"),
        }
        window.redraw();
    }

    /// Closes a tab (its program is ended); the window with the last one.
    fn close_tab(&mut self, index: usize, window: &mut Window) {
        if index >= self.sessions.len() {
            return;
        }
        self.sessions.remove(index);
        if self.sessions.is_empty() {
            window.close();
            return;
        }
        if self.shown > index || self.shown >= self.sessions.len() {
            self.shown = self.shown.saturating_sub(1);
        }
        self.retitle(window);
        window.redraw();
    }

    fn show_tab(&mut self, index: usize, window: &mut Window) {
        if index < self.sessions.len() && index != self.shown {
            self.shown = index;
            self.retitle(window);
            window.redraw();
        }
    }

    /// The left button went down on the text: a selection starts (by
    /// characters, then words, then lines for presses on the same cell in
    /// quick succession), or with Shift the one there is reaches to here.
    fn press_text(&mut self, x: f32, y: f32, m: Modifiers, window: &mut Window) {
        let Some((cw, ch)) = self.metrics.as_ref().map(|mt| (mt.cell_w, mt.cell_h)) else { return };
        let Some(session) = self.sessions.get(self.shown) else { return };
        let mut guard = lock(&session.screen);
        let s = &mut *guard;
        // A program that asked for the mouse has it (report()); Shift takes it
        // back, and scrolled back the text under the pointer is ours.
        if s.grid.mouse != MouseMode::Off && !m.shift && s.scroll == 0 {
            return;
        }
        let pos = s.cell_at(x, y, cw, ch);
        let now = Instant::now();
        let nth = match self.presses {
            Some((then, at, n)) if at == pos && now.duration_since(then) < DOUBLE_CLICK => n % 3 + 1,
            _ => 1,
        };
        self.presses = Some((now, pos, nth));
        if nth == 1 && m.shift && s.selection.is_some() {
            s.drag_selection(pos);
        } else {
            s.selection = Some(Selection::start(&s.grid, [Unit::Cell, Unit::Word, Unit::Line][nth - 1], pos));
        }
        self.selecting = true;
        window.redraw();
    }

    /// The scrollbar of the tab in front, if it has one now.
    fn scrollbar(&self) -> Option<Scrollbar> {
        let session = self.sessions.get(self.shown)?;
        lock(&session.screen).scrollbar(self.width, self.height)
    }

    /// The held thumb follows the pointer, and the view with it.
    fn drag_thumb(&mut self, y: f32, grab: f32, window: &mut Window) {
        let Some(session) = self.sessions.get(self.shown) else { return };
        let mut s = lock(&session.screen);
        if let Some(bar) = s.scrollbar(self.width, self.height) {
            let to = bar.scroll_for(y - grab);
            if to != s.scroll {
                s.scroll = to;
                s.clamp_scroll();
                window.redraw();
            }
        }
    }

    /// Tells the front tab's program about a mouse event at a point of the
    /// window, if it asked for such events and Shift is not keeping the
    /// mouse for selecting. True if the event was the program's.
    fn report(&mut self, action: mouse::Action, x: f32, y: f32, m: Modifiers) -> bool {
        let Some((cw, ch)) = self.metrics.as_ref().map(|mt| (mt.cell_w, mt.cell_h)) else { return false };
        let Some(session) = self.sessions.get_mut(self.shown) else { return false };
        let s = lock(&session.screen);
        if s.grid.mouse == MouseMode::Off || s.scroll > 0 {
            // Scrolled back, what is under the pointer is not the program's screen.
            return false;
        }
        let cell = s.screen_cell(x, y, cw, ch);
        if matches!(action, mouse::Action::Motion(_)) && self.reported_cell == Some(cell) {
            return true;
        }
        let bytes = mouse::bytes(s.grid.mouse, s.grid.mouse_sgr, action, cell, m);
        drop(s);
        self.reported_cell = Some(cell);
        if let Some(bytes) = bytes {
            session.pty.write(&bytes);
        }
        true
    }

    /// The pointer moved with the button held on the text.
    fn drag_text(&mut self, x: f32, y: f32, window: &mut Window) {
        let Some((cw, ch)) = self.metrics.as_ref().map(|mt| (mt.cell_w, mt.cell_h)) else { return };
        let Some(session) = self.sessions.get(self.shown) else { return };
        let mut s = lock(&session.screen);
        let pos = s.cell_at(x, y, cw, ch);
        let bottom = BAR_H + PAD + s.grid.rows as f32 * ch;
        // Past the top or bottom edge the view scrolls for as long as it is held there (scene()).
        self.edge = if y < BAR_H + PAD {
            Some((true, pos.0))
        } else if y >= bottom {
            Some((false, pos.0))
        } else {
            None
        };
        if s.drag_selection(pos) || self.edge.is_some() {
            window.redraw();
        }
    }

    /// The window is called what its front tab is.
    fn retitle(&self, window: &mut Window) {
        if let Some(s) = self.sessions.get(self.shown) {
            window.set_title(&s.label());
        }
    }

    /// What a key sends to the program (xterm's sequences).
    fn key_bytes(sym_: u32, text: Option<&str>, m: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
        let csi = |s: &str| Some(format!("\x1b[{s}").into_bytes());
        let cursor = |c: char| Some(if app_cursor { format!("\x1bO{c}") } else { format!("\x1b[{c}") }.into_bytes());
        const KP_ENTER: u32 = 0xff8d;
        const BACK_TAB: u32 = 0xfe20;
        const INSERT: u32 = 0xff63;
        const F1: u32 = 0xffbe;
        match sym_ {
            sym::RETURN | KP_ENTER => return Some(b"\r".to_vec()),
            sym::BACKSPACE => return Some(if m.ctrl { vec![0x08] } else { vec![0x7f] }),
            sym::TAB => return Some(if m.shift { b"\x1b[Z".to_vec() } else { b"\t".to_vec() }),
            BACK_TAB => return Some(b"\x1b[Z".to_vec()),
            sym::ESCAPE => return Some(vec![0x1b]),
            sym::UP => return cursor('A'),
            sym::DOWN => return cursor('B'),
            sym::RIGHT => return cursor('C'),
            sym::LEFT => return cursor('D'),
            sym::HOME => return cursor('H'),
            sym::END => return cursor('F'),
            sym::PAGE_UP => return csi("5~"),
            sym::PAGE_DOWN => return csi("6~"),
            INSERT => return csi("2~"),
            sym::DELETE => return csi("3~"),
            s if (F1..F1 + 4).contains(&s) => return Some(vec![0x1b, b'O', b'P' + (s - F1) as u8]),
            s if (F1 + 4..F1 + 12).contains(&s) => return csi(["15~", "17~", "18~", "19~", "20~", "21~", "23~", "24~"][(s - F1 - 4) as usize]),
            0x20 if m.ctrl => return Some(vec![0]),
            _ => {}
        }
        // Ctrl+letter: the control character.
        if m.ctrl
            && let Some(ch) = char::from_u32(sym_).map(|c| c.to_ascii_lowercase())
        {
            let b = match ch {
                'a'..='z' => Some(ch as u8 - b'a' + 1),
                '[' => Some(0x1b),
                '\\' => Some(0x1c),
                ']' => Some(0x1d),
                '6' => Some(0x1e),
                '-' | '/' => Some(0x1f),
                _ => None,
            };
            if let Some(b) = b {
                return Some(if m.alt { vec![0x1b, b] } else { vec![b] });
            }
        }
        let t = text.filter(|t| !t.is_empty())?;
        let mut out = Vec::new();
        if m.alt {
            out.push(0x1b);
        }
        out.extend_from_slice(t.as_bytes());
        Some(out)
    }
}

impl App for Terminal {
    fn title(&self) -> String {
        "Terminal".into()
    }

    fn app_id(&self) -> String {
        "vterm".into()
    }

    fn own_frame(&self) -> bool {
        true
    }

    fn background(&self) -> Color {
        self.palette.bg
    }

    fn animating(&self) -> bool {
        // A selection held past the edge scrolls the view frame after frame.
        self.edge.is_some()
    }

    fn waker(&mut self, waker: Waker) {
        self.waker = Some(waker.clone());
        watch_settings(self.settings_changed.clone(), waker.clone());
        for _ in 0..self.open_with {
            match Session::start(self.fits.0, self.fits.1, waker.clone()) {
                Ok(s) => self.sessions.push(s),
                Err(e) => {
                    eprintln!("vterm: could not start the shell: {e}");
                    std::process::exit(1);
                }
            }
        }
    }

    fn woken(&mut self, window: &mut Window) {
        if self.settings_changed.swap(false, Ordering::SeqCst) {
            // Colours, fonts and the buttons' side as they are now; the cell's size is
            // worked out again at the next frame, and the grids follow it.
            self.palette = palette();
            self.settings = Settings::from_text(&config("settings.toml"));
            self.metrics = None;
            window.redraw();
        }
        // Tabs whose program has ended go; the window goes with the last.
        let mut index = 0;
        while index < self.sessions.len() {
            if lock(&self.sessions[index].screen).exited {
                self.close_tab(index, window);
                if self.sessions.is_empty() {
                    return;
                }
            } else {
                index += 1;
            }
        }
        for (i, session) in self.sessions.iter_mut().enumerate() {
            let mut s = lock(&session.screen);
            if let Some(text) = s.copied.take() {
                window.copy(&text);
            }
            if s.grid.title != session.title {
                session.title = s.grid.title.clone();
                // The bar shows every tab's title; the window carries the front one's.
                window.redraw();
                if i == self.shown {
                    window.set_title(if session.title.is_empty() { "Terminal" } else { &session.title });
                }
            }
            // Only the tab in front is drawn: output in the others costs no frame.
            if std::mem::take(&mut s.grid.dirty) && i == self.shown {
                window.redraw();
            }
        }
    }

    fn input(&mut self, input: &Input, window: &mut Window) {
        const SHIFT_L: u32 = 0xffe1;
        const SHIFT_R: u32 = 0xffe2;
        match input {
            // The key itself says Shift went down or up; a button says how things stand.
            Input::Key { sym: SHIFT_L | SHIFT_R, pressed, repeat: false, .. } => self.shift = *pressed,
            Input::Button { modifiers, .. } => self.shift = modifiers.shift,
            Input::Focus(false) => self.shift = false,
            _ => {}
        }
        match input {
            Input::PointerMoved { y, .. } if self.thumb.is_some() => self.drag_thumb(*y, self.thumb.unwrap_or(0.0), window),
            Input::Button { button: Button::Left, pressed: false, .. } if self.thumb.is_some() => self.thumb = None,
            Input::PointerMoved { x, y } if self.selecting => self.drag_text(*x, *y, window),
            Input::PointerMoved { x, y } => {
                let part = self.layout().at(*x, *y);
                self.over_pane = part.is_none() && *y >= BAR_H;
                // A program with the mouse hears where the pointer is (if it asked for moves),
                // and while a button it was told of is held, wherever the pointer goes.
                let on_bar = self.over_pane && self.reported.is_none() && self.scrollbar().is_some_and(|bar| bar.holds(*x, *y));
                let theirs = !on_bar && !self.shift && (self.over_pane || self.reported.is_some()) && self.report(mouse::Action::Motion(self.reported), *x, *y, Modifiers::default());
                window.set_cursor(if part.is_some() || theirs || on_bar { Cursor::Default } else { Cursor::Text });
                if part != self.hover {
                    self.hover = part;
                    window.redraw();
                }
            }
            Input::PointerLeft => {
                self.over_pane = false;
                if self.hover.take().is_some() {
                    window.redraw();
                }
            }
            Input::Button { button: Button::Left, pressed: false, .. } if self.selecting => {
                self.selecting = false;
                self.edge = None;
                // A press that selected nothing leaves nothing selected.
                if let Some(session) = self.sessions.get(self.shown) {
                    let mut s = lock(&session.screen);
                    if s.selection.is_some_and(|sel| sel.range().is_none()) {
                        s.selection = None;
                    }
                }
                window.redraw();
            }
            Input::Button { button, pressed: false, x, y, modifiers } if self.reported == Some(*button) => {
                self.reported = None;
                self.report(mouse::Action::Release(*button), *x, *y, *modifiers);
            }
            Input::Button { button, pressed: true, x, y, modifiers } => match (button, self.layout().at(*x, *y)) {
                // The scrollbar: a press takes hold of the thumb, which comes to the pointer if it was not under it.
                (Button::Left, None) if *y >= BAR_H && self.reported.is_none() && self.scrollbar().is_some_and(|bar| bar.holds(*x, *y)) => {
                    let grab = self.scrollbar().map_or(0.0, |bar| bar.grab(*y));
                    self.thumb = Some(grab);
                    self.drag_thumb(*y, grab, window);
                }
                (_, None) if *y >= BAR_H && !modifiers.shift && self.reported.is_none() && self.report(mouse::Action::Press(*button), *x, *y, *modifiers) => {
                    self.reported = Some(*button);
                }
                (Button::Left, None) if *y >= BAR_H => self.press_text(*x, *y, *modifiers, window),
                (Button::Left, Some(Part::Button(WinButton::Close))) => window.close(),
                (Button::Left, Some(Part::Button(WinButton::Minimize))) => window.minimize(),
                (Button::Left, Some(Part::Button(WinButton::Maximize))) => {
                    self.maximized = !self.maximized;
                    window.set_maximized(self.maximized);
                }
                (Button::Left, Some(Part::Tab(i))) => self.show_tab(i, window),
                (Button::Left, Some(Part::CloseTab(i))) | (Button::Middle, Some(Part::Tab(i) | Part::CloseTab(i))) => self.close_tab(i, window),
                (Button::Left, Some(Part::Plus)) => self.new_tab(window),
                (Button::Left, Some(Part::Bar)) => {
                    // Twice on the bare bar: larger or back. Once: the user moves the window.
                    if self.bar_press.take().is_some_and(|t| t.elapsed() < DOUBLE_CLICK) {
                        self.maximized = !self.maximized;
                        window.set_maximized(self.maximized);
                    } else {
                        self.bar_press = Some(Instant::now());
                        window.start_move();
                    }
                }
                _ => {}
            },
            Input::Key { sym: key, text, pressed: true, composing: false, modifiers, .. } => {
                let m = *modifiers;
                let letter = |c: char| *key == c as u32 || *key == c.to_ascii_uppercase() as u32;
                // The terminal's own shortcuts: with Ctrl+Shift, so the shell keeps Ctrl+T and the rest.
                if m.ctrl && m.shift && letter('t') {
                    return self.new_tab(window);
                }
                if m.ctrl && m.shift && letter('w') {
                    return self.close_tab(self.shown, window);
                }
                if m.ctrl && !m.shift && (*key == sym::PAGE_UP || *key == sym::PAGE_DOWN) {
                    let n = self.sessions.len().max(1);
                    return self.show_tab(if *key == sym::PAGE_UP { (self.shown + n - 1) % n } else { (self.shown + 1) % n }, window);
                }
                let Some(session) = self.sessions.get_mut(self.shown) else { return };
                let mut s = lock(&session.screen);
                if m.ctrl && m.shift && letter('c') {
                    if let Some(text) = s.selection.and_then(|sel| sel.text(&s.grid)) {
                        window.copy(&text);
                    }
                } else if m.ctrl && m.shift && letter('v') {
                    window.paste();
                } else if m.shift && (*key == sym::PAGE_UP || *key == sym::PAGE_DOWN) {
                    let page = s.grid.rows.saturating_sub(1).max(1) as isize;
                    s.scroll_by(if *key == sym::PAGE_UP { page } else { -page });
                } else if let Some(bytes) = Terminal::key_bytes(*key, text.as_deref(), m, s.grid.app_cursor) {
                    // Typing goes to the live screen, and what was selected is let go.
                    s.scroll = 0;
                    s.selection = None;
                    drop(s);
                    session.pty.write(&bytes);
                }
                window.redraw();
            }
            Input::Paste(text) => {
                let Some(session) = self.sessions.get_mut(self.shown) else { return };
                let bracketed = lock(&session.screen).grid.bracketed_paste;
                // A pasted line end is the Return key; and the paste cannot end its own brackets.
                let text = text.replace("\r\n", "\r").replace('\n', "\r").replace("\x1b[201~", "");
                let bytes = if bracketed { format!("\x1b[200~{text}\x1b[201~") } else { text };
                session.pty.write(bytes.as_bytes());
            }
            Input::Scroll { x, y, dy, .. } if self.over_pane => {
                let notches = (dy / 15.0).round() as isize;
                // A program with the mouse gets the wheel, a report for each notch.
                if !self.shift && notches != 0 {
                    let action = if notches < 0 { mouse::Action::WheelUp } else { mouse::Action::WheelDown };
                    if self.report(action, *x, *y, Modifiers::default()) {
                        for _ in 1..notches.unsigned_abs().min(20) {
                            self.report(action, *x, *y, Modifiers::default());
                        }
                        return;
                    }
                }
                let Some(session) = self.sessions.get_mut(self.shown) else { return };
                let mut s = lock(&session.screen);
                if s.grid.alt {
                    // A full-screen program has no scrollback: the wheel is its arrow keys.
                    let key = if s.grid.app_cursor { "\x1bO" } else { "\x1b[" }.to_string() + if notches < 0 { "A" } else { "B" };
                    drop(s);
                    session.pty.write(key.repeat((notches.unsigned_abs() * WHEEL_LINES as usize).min(60)).as_bytes());
                } else {
                    s.scroll_by(-notches * WHEEL_LINES);
                    window.redraw();
                }
            }
            Input::Focus(on) => {
                self.focused = *on;
                window.redraw();
            }
            _ => {}
        }
    }

    fn scene(&mut self, frame: &Frame, fonts: &mut Fonts) -> Scene {
        let p = &self.palette;
        (self.width, self.height) = (frame.width, frame.height);
        let mut scene = Scene::new(frame.width, frame.height);
        scene.fill(Rect::new(0.0, 0.0, frame.width, frame.height), p.bg);
        let size = self.settings.mono_size;
        if self.metrics.is_none() {
            // The settings' fonts, or the machine's own if it does not have them.
            let mono = &self.settings.mono_family;
            self.metrics = fonts.face(mono, 400, false).or_else(|| fonts.face("monospace", 400, false)).map(|regular| {
                let bold = fonts.face(mono, 700, false).or_else(|| fonts.face("monospace", 700, false)).unwrap_or(regular);
                let ui = fonts.face(&self.settings.ui_family, 400, false).or_else(|| fonts.face("sans-serif", 400, false));
                let m = fonts.metrics(regular, size);
                let (face, glyph) = fonts.glyph(regular, 'M');
                Metrics { regular, bold, ui, cell_w: fonts.advance(face, glyph, size), cell_h: (m.ascent + m.descent + m.line_gap).ceil(), baseline: m.ascent.round() }
            });
        }
        let Some(mt) = &self.metrics else { return scene };
        let (cw, ch) = (mt.cell_w, mt.cell_h);
        // The grid that fits under the bar.
        let cols = (((frame.width - 2.0 * PAD) / cw).floor().max(2.0) as usize).min(2000);
        let rows = (((frame.height - BAR_H - 2.0 * PAD) / ch).floor().max(1.0) as usize).min(1000);
        self.fits = (cols, rows);

        let titles: Vec<String> = self.sessions.iter().map(Session::label).collect();
        let layout = Layout::new(frame.width, self.sessions.len(), self.shown, &self.settings.left, &self.settings.right);
        bar::draw(&mut scene, fonts, &layout, &bar::Look { focused: self.focused, pane: p.bg, pane_ink: p.fg, font: mt.ui, font_size: self.settings.ui_size, titles: &titles, shown: self.shown, hover: self.hover });

        let Some(session) = self.sessions.get(self.shown) else { return scene };
        let mut guard = lock(&session.screen);
        let s = &mut *guard;
        // The front tab's grid follows the window's size, and its program is told.
        if (cols, rows) != (s.grid.cols, s.grid.rows) {
            if cols != s.grid.cols {
                // Lines are cut or padded: the selection's cells are not what they were.
                s.selection = None;
            }
            s.grid.resize(cols, rows);
            s.clamp_scroll();
            session.pty.resize(cols as u16, rows as u16);
        }
        // A selection held past the top or bottom edge: the view moves a line at a time and
        // the selection's end goes with the edge row.
        if let Some((up, col)) = self.edge
            && self.edge_tick.elapsed() >= EDGE_TICK
        {
            self.edge_tick = Instant::now();
            s.scroll_by(if up { 1 } else { -1 });
            let row = s.top_line() + if up { 0 } else { s.grid.rows - 1 };
            s.drag_selection((col, row));
        }
        let (selection, top_line) = (s.selection, s.top_line());
        scene.push_clip(Rect::new(0.0, BAR_H, frame.width, (frame.height - BAR_H).max(0.0)), 0.0);

        // Scene font numbers for the faces used in this frame.
        let mut numbers: HashMap<FaceId, u32> = HashMap::new();
        let mut number = |scene: &mut Scene, fonts: &Fonts, face: FaceId| {
            *numbers.entry(face).or_insert_with(|| {
                let n = fonts.name(face);
                scene.font(&n.family, n.weight, n.italic)
            })
        };
        for r in 0..s.grid.rows {
            let line = s.shown(r);
            let top = BAR_H + PAD + r as f32 * ch;
            // Backgrounds and underlines: one rectangle for each run of cells with the same look.
            let mut x = 0;
            while x < line.len() {
                let style = line[x].style;
                let mut end = x + 1;
                while end < line.len() && line[end].style == style {
                    end += 1;
                }
                let (mut fg, mut bg) = (p.resolve(style.fg, p.fg, style.bold), p.resolve(style.bg, p.bg, false));
                if style.inverse {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if bg != p.bg {
                    scene.fill(Rect::new(PAD + x as f32 * cw, top, (end - x) as f32 * cw, ch), bg);
                }
                if style.underline {
                    scene.fill(Rect::new(PAD + x as f32 * cw, top + ch - 2.0, (end - x) as f32 * cw, 1.0), fg);
                }
                x = end;
            }
            // Glyphs, each at its cell: a run for each stretch in one font and colour.
            let mut run: Option<(u32, Color, Vec<Glyph>)> = None;
            let origin = Point::new(PAD, top + mt.baseline);
            let mut put = |scene: &mut Scene, font: u32, fg: Color, placed: Glyph| match &mut run {
                Some((f, c, glyphs)) if *f == font && *c == fg => glyphs.push(placed),
                _ => {
                    if let Some((f, c, glyphs)) = run.take() {
                        scene.glyphs(f, size, c, origin, glyphs);
                    }
                    run = Some((font, fg, vec![placed]));
                }
            };
            for (col, cell) in line.iter().enumerate() {
                if cell.width == 0 || cell.c == ' ' {
                    continue;
                }
                let st = cell.style;
                let mut fg = if st.inverse { p.resolve(st.bg, p.bg, false) } else { p.resolve(st.fg, p.fg, st.bold) };
                if st.dim {
                    fg.a = 153;
                }
                let own = if st.bold { mt.bold } else { mt.regular };
                let (face, glyph) = fonts.glyph(own, cell.c);
                let font = number(&mut scene, fonts, face);
                put(&mut scene, font, fg, Glyph { id: glyph, x: col as f32 * cw, y: 0.0 });
                // Combining marks, drawn on the character. Fonts make them one of two ways:
                // with no advance, drawn back over what came before the pen (so the pen
                // goes after the character), or as wide as a cell, drawn in place (so it
                // goes where the character's did).
                for mark in cell.marks.into_iter().filter(|m| *m != '\0') {
                    let (face, glyph) = fonts.glyph(own, mark);
                    let font = number(&mut scene, fonts, face);
                    let after = fonts.advance(face, glyph, size) < cw * 0.5;
                    put(&mut scene, font, fg, Glyph { id: glyph, x: (col + if after { cell.width.max(1) as usize } else { 0 }) as f32 * cw, y: 0.0 });
                }
            }
            if let Some((f, c, glyphs)) = run.take() {
                scene.glyphs(f, size, c, origin, glyphs);
            }
            // What is selected, tinted over its text.
            if let Some((a, b)) = selection.and_then(|sel| sel.span(top_line + r, s.grid.cols)) {
                scene.fill(Rect::new(PAD + a as f32 * cw, top, (b + 1 - a) as f32 * cw, ch), Color::rgba(p.accent.r, p.accent.g, p.accent.b, 89));
            }
        }
        // The cursor: a block with the keyboard, an outline without. It moves down with the
        // screen while scrolled back, out of sight past the bottom.
        let cursor_row = s.grid.cy + s.scroll;
        if s.grid.cursor_visible && cursor_row < s.grid.rows {
            let at = Rect::new(PAD + s.grid.cx as f32 * cw, BAR_H + PAD + cursor_row as f32 * ch, cw, ch);
            if self.focused {
                scene.fill(at, Color::rgba(p.accent.r, p.accent.g, p.accent.b, 178));
            } else {
                scene.stroke(at, 0.0, 1.0, p.accent);
            }
        }
        // The scrollbar: faint at the live screen, clearer while scrolled back or held.
        if let Some(bar) = s.scrollbar(frame.width, frame.height) {
            scene.fill_rounded(bar.track, scrollbar::WIDTH / 2.0, Color::rgba(p.fg.r, p.fg.g, p.fg.b, 15));
            let alpha = if self.thumb.is_some() { 166 } else if s.scroll > 0 { 128 } else { 64 };
            scene.fill_rounded(bar.thumb, scrollbar::WIDTH / 2.0, Color::rgba(p.fg.r, p.fg.g, p.fg.b, alpha));
        }
        // Scrolled back: how far, in the top right corner (left of the scrollbar).
        if s.scroll > 0 {
            let label = if s.scroll == 1 { "1 line back".to_string() } else { format!("{} lines back", s.scroll) };
            let width = label.chars().count() as f32 * cw + 16.0;
            let at = Rect::new(frame.width - width - PAD - scrollbar::GRAB, BAR_H + PAD, width, ch + 4.0);
            scene.fill_rounded(at, 6.0, Color::rgba(p.accent.r, p.accent.g, p.accent.b, 235));
            let font = number(&mut scene, fonts, mt.regular);
            let glyphs = label.chars().enumerate().map(|(i, c)| Glyph { id: fonts.glyph(mt.regular, c).1, x: i as f32 * cw, y: 0.0 }).collect();
            scene.glyphs(font, size, p.bg, Point::new(at.x + 8.0, at.y + 2.0 + mt.baseline), glyphs);
        }
        scene.pop_clip();
        scene
    }
}

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("vterm {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let mut terminal = Terminal::new();
    let args: Vec<String> = std::env::args().collect();
    if let Some(n) = args.iter().position(|a| a == "--tabs").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<usize>().ok()) {
        terminal.open_with = n.clamp(1, 64);
    }
    if let Err(e) = vergence_ui::run(&mut terminal, (820, 520)) {
        eprintln!("vterm: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers { shift: false, ctrl: false, alt: false, logo: false };

    #[test]
    fn keys_send_what_xterm_sends() {
        let k = |s: u32, t: Option<&str>, m: Modifiers| Terminal::key_bytes(s, t, m, false);
        let ctrl = Modifiers { ctrl: true, ..NONE };
        let alt = Modifiers { alt: true, ..NONE };
        assert_eq!(k('a' as u32, Some("a"), NONE), Some(b"a".to_vec()));
        assert_eq!(k(0xe9, Some("é"), NONE), Some("é".as_bytes().to_vec()));
        assert_eq!(k(sym::RETURN, None, NONE), Some(b"\r".to_vec()));
        assert_eq!(k(sym::BACKSPACE, None, NONE), Some(vec![0x7f]));
        assert_eq!(k(sym::BACKSPACE, None, ctrl), Some(vec![0x08]));
        assert_eq!(k(sym::TAB, None, Modifiers { shift: true, ..NONE }), Some(b"\x1b[Z".to_vec()));
        assert_eq!(k(sym::UP, None, NONE), Some(b"\x1b[A".to_vec()));
        assert_eq!(Terminal::key_bytes(sym::UP, None, NONE, true), Some(b"\x1bOA".to_vec()), "application cursor keys");
        assert_eq!(k(sym::PAGE_DOWN, None, NONE), Some(b"\x1b[6~".to_vec()));
        assert_eq!(k(sym::DELETE, None, NONE), Some(b"\x1b[3~".to_vec()));
        assert_eq!(k(0xffbe, None, NONE), Some(b"\x1bOP".to_vec()), "F1");
        assert_eq!(k(0xffc1, None, NONE), Some(b"\x1bOS".to_vec()), "F4");
        assert_eq!(k(0xffc2, None, NONE), Some(b"\x1b[15~".to_vec()), "F5");
        assert_eq!(k(0xffc9, None, NONE), Some(b"\x1b[24~".to_vec()), "F12");
        assert_eq!(k('c' as u32, None, ctrl), Some(vec![3]));
        assert_eq!(k('C' as u32, None, Modifiers { shift: true, ..ctrl }), Some(vec![3]));
        assert_eq!(k('[' as u32, None, ctrl), Some(vec![0x1b]));
        assert_eq!(k(0x20, Some(" "), ctrl), Some(vec![0]));
        assert_eq!(k('x' as u32, Some("x"), alt), Some(b"\x1bx".to_vec()));
        assert_eq!(k('d' as u32, None, Modifiers { ctrl: true, alt: true, ..NONE }), Some(vec![0x1b, 4]));
        // A modifier by itself, or a key that types nothing, sends nothing.
        assert_eq!(k(0xffe1, None, NONE), None);
        assert_eq!(k(0xffeb, None, NONE), None);
    }

    #[test]
    fn colours_come_from_the_palette_or_the_cube() {
        let p = TANGO;
        assert_eq!(p.resolve(CellColor::Default, p.fg, false), p.fg);
        assert_eq!(p.resolve(CellColor::Indexed(1), p.fg, false), rgb(0xcc0000));
        assert_eq!(p.resolve(CellColor::Indexed(1), p.fg, true), rgb(0xef2929), "bold is the bright one");
        assert_eq!(p.resolve(CellColor::Indexed(9), p.fg, true), rgb(0xef2929));
        assert_eq!(p.indexed(16), Color::rgb(0, 0, 0));
        assert_eq!(p.indexed(231), Color::rgb(255, 255, 255));
        assert_eq!(p.indexed(196), Color::rgb(255, 0, 0));
        assert_eq!(p.indexed(232), Color::rgb(8, 8, 8));
        assert_eq!(p.indexed(255), Color::rgb(238, 238, 238));
        assert_eq!(p.resolve(CellColor::Rgb(1, 2, 3), p.fg, false), Color::rgb(1, 2, 3));
    }

    #[test]
    fn the_settings_file_is_followed() {
        let s = Settings::from_text("[settings]\n\"Gtk/DecorationLayout\" = \"close,minimize:\"\n\"Gtk/FontName\" = \"Adwaita Sans 10\"\n\"Gtk/MonospaceFontName\" = \"Adwaita Mono 11\"\n");
        assert_eq!((s.left.as_slice(), s.right.len()), (&[WinButton::Close, WinButton::Minimize][..], 0));
        assert_eq!((s.ui_family.as_str(), s.mono_family.as_str()), ("Adwaita Sans", "Adwaita Mono"));
        assert!((s.ui_size - 13.333).abs() < 0.01 && (s.mono_size - 14.667).abs() < 0.01);
        // Nothing set: buttons on the right, the machine's own fonts at 10 points.
        let s = Settings::from_text("");
        assert_eq!((s.left.len(), s.right.as_slice()), (0, &[WinButton::Minimize, WinButton::Maximize, WinButton::Close][..]));
        assert_eq!((s.ui_family.as_str(), s.mono_family.as_str()), ("sans-serif", "monospace"));
        assert_eq!(font_name("Cantarell", "sans-serif", 10.0).0, "Cantarell");
        assert_eq!(font_name("Noto Sans 400", "sans-serif", 10.0), ("Noto Sans 400".to_string(), 10.0 * 96.0 / 72.0), "a size that is not one is part of the name");
    }

    #[test]
    fn the_view_scrolls_within_the_scrollback() {
        let mut s = Screen { grid: Grid::new(20, 4), parser: vt::Parser::new(), scroll: 0, exited: false, copied: None, selection: None };
        for i in 0..10 {
            let line = format!("line {i}\r\n");
            s.parser.advance(&mut s.grid, line.as_bytes());
        }
        let text = |s: &Screen, r: usize| s.shown(r).iter().map(|c| c.c).collect::<String>().trim_end().to_string();
        assert_eq!(text(&s, 0), "line 7");
        s.scroll_by(3);
        assert_eq!((s.scroll, text(&s, 0)), (3, "line 4".to_string()));
        s.scroll_by(100);
        assert_eq!(text(&s, 0), "line 0", "no further than the first line kept");
        s.scroll_by(-1000);
        assert_eq!((s.scroll, text(&s, 0)), (0, "line 7".to_string()));
    }
}
