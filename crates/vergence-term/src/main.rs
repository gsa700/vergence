//! Vergence's terminal, on the project's own toolkit (toolkit milestone 1).
//!
//! The screen, the escape-sequence parser and the pty (grid.rs, vt.rs,
//! pty.rs) are the ones written for the first terminal, whose interface was
//! built on a borrowed toolkit (iced, 2026-11-08) and retired on 2026-11-10
//! when this one passed the stress test that one failed. This interface was
//! written beside it under the name `vterm`, which the plan's Progress and
//! the stress test's scripts still use. What it does differently:
//!
//! - **The program's output is parsed on a thread of its own**, into the
//!   screen behind a lock. The window's thread never parses and is never
//!   held up by a flood: when the screen has changed it is woken, asks for a
//!   frame, and the frame shows whatever the screen holds when the monitor
//!   is ready for it. However fast the output, at most one frame is drawn
//!   for each the monitor shows.
//! - **The picture is a scene** (vergence-scene), drawn on the GPU or, where
//!   that is not to be had, in software: a terminal's screen is one grid of
//!   cells, each with its ground, its ink and its glyph, found in the font
//!   or in the nearest font that has it.
//! - **Keys arrive in order** through the toolkit and go to the program as
//!   xterm's sequences.
//!
//! So far: terminals in tabs under a bar of the window's own (the toolkit's, vergence_ui::bar), with
//! the window's buttons on the side the desktop's settings say and tabs
//! that shrink to fit; typing, paste, the scrollback (wheel,
//! Shift+PageUp/PageDown, Shift+Home/End), resizing, titles, the program's own copies (OSC
//! 52), the fonts and color scheme from the desktop's settings. Ctrl+Shift+T
//! opens a tab, Ctrl+Shift+W closes one, Ctrl+PageUp/PageDown and
//! Ctrl+Tab/Ctrl+Shift+Tab change tab.
//! The mouse selects text (select.rs): a drag by characters, a second press
//! by words, a third by lines, Shift and a press to extend; held past the
//! top or bottom edge the view scrolls under it. Ctrl+Shift+C copies. A
//! program that asked for the mouse gets it (mouse.rs: presses, releases,
//! moves and the wheel, as xterm reports them) unless Shift is held, which
//! keeps the mouse for selecting.
//! A thin scrollbar at the right edge (scrollbar.rs) shows where the view
//! is in the scrollback and can be dragged.
//! The desktop's settings are followed while the terminal runs: the two
//! files are looked at once a second, and a change of fonts, colors or
//! button side shows at the next frame.
//! The right button opens a menu (menu.rs) with copy, paste and the tab
//! commands; where a program has the mouse, Shift and the right button.
//! A tab's room can be divided among several terminals (panes.rs):
//! Ctrl+Shift+R side by side, Ctrl+Shift+D one above the other, as often
//! as wanted; a press makes a pane the one the keys go to, the line between
//! two can be dragged, and Ctrl+Shift+W closes the pane (the tab with its
//! last).
//! Ctrl+C copies when text is selected (and is the interrupt key when
//! none is); Ctrl+V pastes. Ctrl+Shift+C and Ctrl+Shift+V do the same.

#[cfg(test)]
mod fuzz;
#[allow(dead_code)]
mod grid;
mod confirm;
mod prefs;
mod menu;
mod mouse;
mod panes;
#[allow(dead_code)]
mod program;
mod pty;
mod scrollbar;
mod select;
#[allow(dead_code)]
mod vt;

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    io::Write,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

use vergence_ui::bar::{self, BAR_H, Layout, Part, WinButton};
use grid::{Cell, Color as CellColor, Grid, MouseMode};
use menu::{Item, Menu};
use panes::{Axis, Divider, Node};
use scrollbar::Scrollbar;
use select::{Pos, Selection, Unit};
use vergence_scene::{CELL_GLYPH, CELL_UNDERLINE, Cell as SceneCell, Color, Glyph, Point, Rect, Scene};
use vergence_text::{FaceId, Fonts};
use vergence_ui::{App, Button, Cursor, Edges, Frame, Input, Modifiers, Waker, Window, sym};

/// Space around the cells, in reference units.
const PAD: f32 = 4.0;
/// The room at the right edge kept for the scrollbar (its width, its margin
/// from the edge, and a little air before the text).
const RIGHT_GUTTER: f32 = scrollbar::WIDTH + 2.0 + 4.0;
/// Two presses on the bar within this are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// What the desktop's settings say that the terminal follows
/// (~/.config/vergence/settings.toml, the file the settings app writes).
struct Settings {
    left: Vec<WinButton>,
    right: Vec<WinButton>,
    /// Light or dark, for the bar we draw ourselves (bar::desktop_light).
    light: bool,
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
/// the first terminal did; the session bus will tell us instead when the
/// toolkit speaks it).
fn watch_settings(changed: Arc<AtomicBool>, waker: Waker) {
    let stamp = |name: &str| config_path(name).and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
    // The files' times, and GNOME's button layout and light or dark when running there.
    let stamps = move || ([stamp("desktop.toml"), stamp("settings.toml")], bar::host_layout(), bar::host_light());
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
    /// Vergence's settings file; the button layout GNOME's when running under
    /// GNOME (Dave, 2026-11-12).
    fn load() -> Settings {
        let mut s = Settings::from_text(&config("settings.toml"));
        if let Some(layout) = bar::host_layout() {
            (s.left, s.right) = bar::decoration_layout(&layout);
        }
        s.light = bar::desktop_light();
        // The terminal's own font and size, if its settings chose them (app settings milestone 1).
        let desktop = config("desktop.toml");
        if let Some(f) = vergence_ui::table_value(&desktop, "[terminal]", "font").filter(|f| !f.trim().is_empty()) {
            s.mono_family = f;
        }
        if let Some(z) = vergence_ui::table_value(&desktop, "[terminal]", "font-size").and_then(|z| z.parse::<f32>().ok()).filter(|z| (4.0..=96.0).contains(z)) {
            s.mono_size = z;
        }
        s
    }

    fn from_text(text: &str) -> Settings {
        // Lines of the form "Gtk/FontName" = "Adwaita Sans 10".
        let value = |key: &str| {
            text.lines().find_map(|line| {
                let (k, v) = line.split_once('=')?;
                (k.trim().trim_matches('"') == key).then(|| v.trim().trim_matches('"').to_string())
            })
        };
        let (left, right) = bar::decoration_layout(&value("Gtk/DecorationLayout").unwrap_or_else(|| bar::DEFAULT_LAYOUT.into()));
        let (ui_family, ui_size) = font_name(&value("Gtk/FontName").unwrap_or_default(), "sans-serif", 10.0);
        let (mono_family, mono_size) = font_name(&value("Gtk/MonospaceFontName").unwrap_or_default(), "monospace", 10.0);
        Settings { left, right, light: false, ui_family, ui_size, mono_family, mono_size }
    }
}
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

/// The terminal's colors: its ground and text, the cursor, and the 16 named ones.
struct Palette {
    bg: Color,
    fg: Color,
    accent: Color,
    ansi: [u32; 16],
    /// The least contrast text keeps against what is behind it (WCAG ratio;
    /// 0: colors as they are). The light palettes' 16 colors were tuned on
    /// black, and Tango's whites and brights all but vanish on white (Dave on
    /// Hubble, 2026-11-13: "the text is light and somewhat hard to see"):
    /// a color under the floor is darkened, its hue kept, until it reads.
    floor: f32,
}

/// The floor of the light palettes: WCAG's for body text.
const LIGHT_FLOOR: f32 = 4.5;

/// `ink` made to keep `floor` against `ground`: as it is if it does, else
/// mixed toward black on a light ground (white on a dark one), the least
/// that reaches it.
fn readable(ink: Color, ground: Color, floor: f32) -> Color {
    let rgb = |c: Color| vergence_look::Rgb(c.r, c.g, c.b);
    let g = rgb(ground);
    if floor <= 0.0 || ink == ground || vergence_look::contrast(rgb(ink), g) >= floor {
        return ink;
    }
    let to = if vergence_look::luminance(g) > 0.18 { 0.0 } else { 255.0 };
    let mix = |v: u8, t: f32| (v as f32 + (to - v as f32) * t).round() as u8;
    for step in 1..=20 {
        let t = step as f32 / 20.0;
        let c = Color::rgba(mix(ink.r, t), mix(ink.g, t), mix(ink.b, t), ink.a);
        if vergence_look::contrast(rgb(c), g) >= floor {
            return c;
        }
    }
    Color::rgba(to as u8, to as u8, to as u8, ink.a)
}

/// `readable`, remembered: every pair of colors a screen has worked out once
/// (2026-11-14: with only the last pair kept, a busy light screen such as
/// btop's alternated between pairs and redid the up to 20 contrast steps cell
/// after cell; tab clicks on Hubble went from 1.85 ms to 3.5 ms on average,
/// 18 ms at worst). Kept per thread, a few thousand pairs at most.
fn readable_cached(ink: Color, ground: Color, floor: f32, floor_bits: u32) -> Color {
    use std::{cell::RefCell, collections::HashMap};
    thread_local! {
        static PAIRS: RefCell<HashMap<(u32, u32, u32), Color>> = RefCell::new(HashMap::new());
    }
    let pack = |c: Color| u32::from_be_bytes([c.r, c.g, c.b, c.a]);
    let key = (pack(ink), pack(ground), floor_bits);
    PAIRS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(c) = m.get(&key) {
            return *c;
        }
        if m.len() >= 4096 {
            m.clear();
        }
        let c = readable(ink, ground, floor);
        m.insert(key, c);
        c
    })
}

const fn rgb(v: u32) -> Color {
    Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Tango: GNOME Terminal's palette, the look 16-color programs were tuned against.
const TANGO: Palette = Palette {
    bg: rgb(0x2e3436),
    fg: rgb(0xd3d7cf),
    accent: rgb(0x729fcf),
    ansi: [0x2e3436, 0xcc0000, 0x4e9a06, 0xc4a000, 0x3465a4, 0x75507b, 0x06989a, 0xd3d7cf, 0x555753, 0xef2929, 0x8ae234, 0xfce94f, 0x729fcf, 0xad7fa8, 0x34e2e2, 0xeeeeec],
    floor: 0.0,
};

/// Catppuccin Mocha, the first terminal's default.
const CATPPUCCIN: Palette = Palette {
    bg: rgb(0x1e1e2e),
    fg: rgb(0xcdd6f4),
    accent: rgb(0x89b4fa),
    ansi: [0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, 0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8],
    floor: 0.0,
};

/// Tango on white, for the light desktop.
const TANGO_LIGHT: Palette = Palette { bg: rgb(0xffffff), fg: rgb(0x2e3436), accent: rgb(0x3465a4), ansi: TANGO.ansi, floor: LIGHT_FLOOR };

/// Catppuccin Latte, the light one.
const LATTE: Palette = Palette {
    bg: rgb(0xeff1f5),
    fg: rgb(0x4c4f69),
    accent: rgb(0x1e66f5),
    ansi: [0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be, 0x6c6f85, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xbcc0cc],
    floor: LIGHT_FLOOR,
};

/// "#rrggbb" -> a color.
fn parse_rgb(s: &str) -> Option<Color> {
    let h = s.trim().strip_prefix('#')?;
    (h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| rgb(u32::from_str_radix(h, 16).unwrap_or(0)))
}

/// The colors desktop.toml asks for, as the first terminal read it:
/// `[terminal] scheme` chooses Tango or Catppuccin (Catppuccin if nothing
/// does); under `[appearance]`, `color-scheme = "light"` the light one of
/// the pair, `contrast = "high"` the text on pure black or white, and
/// `accent = "#rrggbb"` the color of the cursor, the selection and the
/// highlights.
fn palette_from(text: &str) -> Palette {
    // The terminal's own colors if its settings chose them, else the desktop's terminal colors.
    let (mut tango, mut own, mut high, mut accent) = (false, None, false, None);
    let mut section = "";
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches('"');
        match (section, key.trim()) {
            ("[terminal]", "scheme") => own = Some(value.eq_ignore_ascii_case("tango")),
            ("[appearance]", "terminal-colors") => tango = value.eq_ignore_ascii_case("tango"),
            ("[appearance]", "contrast") => high = value == "high",
            ("[appearance]", "accent") => accent = parse_rgb(value),
            _ => {}
        }
    }
    let tango = own.unwrap_or(tango);
    // The terminal's own style if it chose one, else the desktop's; dark under App default.
    let light = vergence_ui::content_light(text, "[terminal]", false);
    let mut p = match (tango, light) {
        (true, false) => TANGO,
        (true, true) => TANGO_LIGHT,
        (false, false) => CATPPUCCIN,
        (false, true) => LATTE,
    };
    if high {
        (p.bg, p.fg) = if light { (rgb(0xffffff), rgb(0x000000)) } else { (rgb(0x000000), rgb(0xffffff)) };
    }
    if let Some(accent) = accent {
        p.accent = accent;
    }
    p
}

fn palette() -> Palette {
    palette_from(&config("desktop.toml"))
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
            // Bold makes the first 8 colors bright, as xterm does.
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
        self.scroll = (self.scroll as isize).saturating_add(lines).max(0) as usize;
        self.clamp_scroll();
    }

    /// The line shown at row `r` (from the scrollback while scrolled back).
    fn shown(&self, r: usize) -> &[Cell] {
        self.grid.abs_line(self.top_line() + r).unwrap_or(&[])
    }

    /// The scrollbar of a terminal that has this room, if there is anything
    /// to scroll (the alternate screen has no scrollback).
    fn scrollbar(&self, area: Rect) -> Option<Scrollbar> {
        let back = if self.grid.alt { 0 } else { self.grid.scrollback.len() };
        Scrollbar::new(area, back, self.grid.rows, self.scroll)
    }

    /// The absolute number of the line shown in the top row.
    fn top_line(&self) -> usize {
        self.grid.pushed - self.scroll
    }

    /// The cell under a point of the window, as a selection counts it, for
    /// a terminal in `area` with cells of `cell` (width, height); points
    /// outside the cells count as the nearest one.
    fn cell_at(&self, x: f32, y: f32, area: Rect, cell: (f32, f32)) -> Pos {
        let (col, row) = self.screen_cell(x, y, area, cell);
        (col, self.top_line() + row)
    }

    /// The column and row of the screen under a point of the window (what
    /// mouse reporting tells the program); the nearest, for points outside.
    fn screen_cell(&self, x: f32, y: f32, area: Rect, cell: (f32, f32)) -> (usize, usize) {
        let col = ((x - area.x - PAD) / cell.0).floor().max(0.0) as usize;
        let row = ((y - area.y - PAD) / cell.1).floor().max(0.0) as usize;
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
    /// What the parsing thread has to say since the window last looked
    /// ([`NEWS_DRAW`], [`NEWS_STATE`]), kept outside the screen's lock: a
    /// terminal in a flood holds that nearly all the time, and the window
    /// would wait for it at every wake only to learn "draw me".
    news: Arc<AtomicU8>,
    /// The foreground process group last looked at, whether it runs as the
    /// superuser, and when that was asked (look milestone 1: the red tab).
    root_seen: std::cell::Cell<Option<(u32, bool, Instant)>>,
    /// Started with a command (`-e`), not the shell: the program is the
    /// pty's own process, and running while it lives.
    command: bool,
}

/// The screen changed: a frame is wanted if this terminal is in front. Up
/// from the first output after a frame until the next frame draws it.
const NEWS_DRAW: u8 = 1;
/// The title, the clipboard or the program's life changed: look into the screen.
const NEWS_STATE: u8 = 2;

impl Session {
    /// Starts the user's shell, or `command` (`-e`), on a new terminal of
    /// cols x rows. Its output is read and parsed on a thread of its own,
    /// which wakes the window.
    fn start(cols: usize, rows: usize, waker: Waker, command: Option<Vec<String>>, cwd: Option<&str>) -> std::io::Result<Session> {
        let is_command = matches!(command.as_deref(), Some([_, ..]));
        let pty = match command.as_deref() {
            Some([program, args @ ..]) => pty::Pty::spawn(program, args, cols as u16, rows as u16, cwd)?,
            _ => pty::Pty::spawn(&std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()), &[], cols as u16, rows as u16, cwd)?,
        };
        let reader = pty.reader()?;
        let mut replies = reader.try_clone()?;
        let screen = Arc::new(Mutex::new(Screen { grid: Grid::new(cols, rows), parser: vt::Parser::new(), scroll: 0, exited: false, copied: None, selection: None }));
        let shared = screen.clone();
        let news = Arc::new(AtomicU8::new(0));
        let said = news.clone();
        // Named, so that a look at the threads says which is parsing.
        std::thread::Builder::new().name("parse".into()).spawn(move || {
            pty::read_loop(reader, |bytes| {
                let (out, bits) = {
                    let mut guard = lock(&shared);
                    let s = &mut *guard;
                    let (before, alt) = (s.grid.pushed, s.grid.alt);
                    let title = s.grid.title.clone();
                    s.parser.advance(&mut s.grid, &bytes);
                    let mut bits = if std::mem::take(&mut s.grid.dirty) { NEWS_DRAW } else { 0 };
                    if s.grid.title != title {
                        bits |= NEWS_STATE;
                    }
                    if s.grid.alt != alt {
                        // Another screen: what was selected is not on it.
                        s.selection = None;
                    }
                    if s.grid.pushed < before {
                        // A full reset: the line numbers started again.
                        s.selection = None;
                        s.scroll = 0;
                    } else if s.scroll > 0 && SCROLL_ON_OUTPUT.load(Ordering::Relaxed) && s.grid.pushed != before {
                        // Scrolled back, with "Scroll on output" on: new lines bring the view back down.
                        s.scroll = 0;
                    } else if s.scroll > 0 {
                        // Scrolled back: stay on the same text as new lines arrive.
                        s.scroll += s.grid.pushed - before;
                    }
                    s.clamp_scroll();
                    if let Some(text) = s.grid.clipboard.take() {
                        s.copied = Some(text);
                        bits |= NEWS_STATE;
                    }
                    (std::mem::take(&mut s.grid.out), bits)
                };
                if !out.is_empty() {
                    let _ = replies.write_all(&out);
                }
                // The window is woken for news it has not been told already: "draw"
                // stays up until a frame has been drawn, so a flood wakes it once a
                // frame, not once for every piece read.
                if bits & !said.fetch_or(bits, Ordering::SeqCst) != 0 {
                    waker.wake();
                }
                true
            });
            lock(&shared).exited = true;
            said.fetch_or(NEWS_STATE, Ordering::SeqCst);
            waker.wake();
        })?;
        Ok(Session { screen, pty, title: String::new(), news, root_seen: std::cell::Cell::new(None), command: is_command })
    }

    fn label(&self) -> String {
        if self.title.is_empty() { "Terminal".into() } else { self.title.clone() }
    }

    /// The program the shell is running, if it is running one: the
    /// terminal's foreground process group is then not the shell's own
    /// (settings milestone 2: closing asks first). Its name, or "a program".
    fn running(&self) -> Option<String> {
        if lock(&self.screen).exited {
            return None;
        }
        let group = self.pty.foreground()?;
        // A command's own process counts; a shell's, only another group.
        (self.command || group != self.pty.pid()).then(|| program::name_of(group).unwrap_or_else(|| "a program".into()))
    }

    /// Whether the program the user is typing to runs as the superuser
    /// (sudo -s, su, sudo's own password prompt, sudo make install while it
    /// runs, a script's sudo). The foreground group is asked each time (a
    /// system call); whether any of its processes is the superuser when the
    /// group changes, or every two seconds while it does not (Linux reads
    /// /proc; FreeBSD runs one `ps`, so at most one every two seconds a pane).
    /// A terminal started as root is red throughout.
    fn as_root(&self) -> bool {
        // Every process of the group counts, the shell's own group included: a
        // script leads the group the sudo it runs is in (found on Hubble).
        let Some(group) = self.pty.foreground() else { return false };
        if let Some((seen, root, at)) = self.root_seen.get()
            && seen == group
            && at.elapsed() < Duration::from_secs(2)
        {
            return root;
        }
        let root = program::group_has_root(group);
        self.root_seen.set(Some((group, root, Instant::now())));
        root
    }

    /// Makes the terminal's grid cols x rows, if it is not, and tells its program.
    fn fit(&self, s: &mut Screen, cols: usize, rows: usize) {
        if (cols, rows) == (s.grid.cols, s.grid.rows) {
            return;
        }
        if cols != s.grid.cols {
            // Lines are cut or padded: the selection's cells are not what they were.
            s.selection = None;
        }
        s.grid.resize(cols, rows);
        s.clamp_scroll();
        self.pty.resize(cols as u16, rows as u16);
    }
}

/// A tab: its room divided among one or more terminals.
struct Tab {
    /// Its name to the server, which draws the bar on the native road (bar milestone 1).
    id: u32,
    tree: Node,
    /// The terminals, by the numbers the tree knows them by.
    panes: Vec<(u64, Session)>,
    /// The one the keys go to.
    active: u64,
}

impl Tab {
    fn session(&self, id: u64) -> Option<&Session> {
        self.panes.iter().find(|(own, _)| *own == id).map(|(_, s)| s)
    }

    fn session_mut(&mut self, id: u64) -> Option<&mut Session> {
        self.panes.iter_mut().find(|(own, _)| *own == id).map(|(_, s)| s)
    }

    /// The tab is called what its active terminal is.
    /// A pane of the tab runs its program as the superuser.
    fn as_root(&self) -> bool {
        self.panes.iter().any(|(_, s)| s.as_root())
    }

    fn label(&self) -> String {
        self.session(self.active).map_or_else(|| "Terminal".into(), Session::label)
    }

    /// The programs its panes' shells are running.
    fn running(&self) -> Vec<String> {
        self.panes.iter().filter_map(|(_, s)| s.running()).collect()
    }
}

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
}

impl Terminal {
    /// A finger dragged `lines` lines down (up, if negative) on the pane at
    /// x, y: the text goes with it. In the scrollback that is the way back
    /// through it; a full-screen program, which has none, gets arrow keys, as
    /// it does from the wheel.
    fn finger_scroll(&mut self, x: f32, y: f32, lines: isize, window: &mut Window) {
        let Some((id, area)) = self.pane_at(x, y).filter(|_| y >= self.bar_h) else { return };
        // A program that asked for the mouse gets the wheel, a notch for every
        // WHEEL_LINES lines the finger has gone.
        let mouse = self.session_mut(id).is_some_and(|session| {
            let s = lock(&session.screen);
            s.grid.mouse != MouseMode::Off && s.scroll == 0
        });
        if mouse {
            self.dragged_lines += lines;
            while self.dragged_lines.abs() >= WHEEL_LINES {
                let up = self.dragged_lines > 0;
                self.dragged_lines -= if up { WHEEL_LINES } else { -WHEEL_LINES };
                self.report(id, area, if up { mouse::Action::WheelUp } else { mouse::Action::WheelDown }, x, y, Modifiers::default());
            }
            return;
        }
        let Some(session) = self.session_mut(id) else { return };
        let mut s = lock(&session.screen);
        if s.grid.alt {
            let key = if s.grid.app_cursor { "\x1bO" } else { "\x1b[" }.to_string() + if lines > 0 { "A" } else { "B" };
            drop(s);
            session.pty.write(key.repeat(lines.unsigned_abs().min(60)).as_bytes());
        } else {
            s.scroll_by(lines);
            window.redraw();
        }
    }

    /// A finger dragged `dy` on the text it came down on at `from`: the
    /// text goes with it, a line for every line's height.
    fn finger_drag(&mut self, dy: f32, from: (f32, f32), window: &mut Window) {
        let line = self.cell().map_or(18.0, |(_, h)| h.max(1.0));
        self.dragged += dy;
        let lines = (self.dragged / line).trunc();
        self.dragged -= lines * line;
        if lines != 0.0 {
            self.finger_scroll(from.0, from.1, lines as isize, window);
        }
    }

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
            prefs_open: Rc::new(std::cell::Cell::new(false)),
            cursor_shape: CursorShape::of(&config("desktop.toml")),
            blink: term_flag(&config("desktop.toml"), "blink", true),
            ask_close: term_flag(&config("desktop.toml"), "ask-close", true),
            blink_on: true,
            bar_style: vergence_ui::bar_style_setting(&config("desktop.toml"), "[terminal]", vergence_ui::BarStyle::Desktop),
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
        }
    }

    /// Tell the program what this window keeps (its token, its tabs' working
    /// directories), for the state file (program.rs).
    fn keep(&mut self) {
        let tabs = self.tabs.iter().filter_map(|t| t.session(t.active)).filter_map(|s| program::cwd_of(s.pty.pid())).collect();
        self.program.borrow_mut().update(self.key, program::WindowState { token: self.token.clone(), tabs });
    }

    /// Another window of this process: sharing its program, opened for a
    /// later start's request or to restore one kept from the last run.
    fn window_for(&self, present: String, start_dirs: Vec<String>, start_cwd: Option<String>, run: Option<Vec<String>>) -> Terminal {
        let mut t = Terminal::new();
        t.program = self.program.clone();
        t.key = t.program.borrow_mut().add();
        t.inbox = self.inbox.clone();
        t.moves = self.moves.clone();
        t.wids = self.wids.clone();
        t.open_with = start_dirs.len().max(1);
        t.present = present;
        t.start_dirs = start_dirs;
        t.start_cwd = start_cwd;
        t.run = run;
        t
    }

    /// Windows asked for by later starts, and windows from the last run still
    /// to open: whichever window hears first opens them.
    fn open_asked(&mut self, window: &mut Window) {
        let asked: Vec<program::OpenRequest> = std::mem::take(&mut *self.inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for r in asked {
            let run = (!r.run.is_empty()).then_some(r.run);
            let cwd = (!r.cwd.is_empty()).then_some(r.cwd);
            window.open_window(Box::new(self.window_for(String::new(), Vec::new(), cwd, run)));
        }
        let restore = std::mem::take(&mut self.program.borrow_mut().restore);
        for w in restore {
            window.open_window(Box::new(self.window_for(w.token, w.tabs, None, None)));
        }
    }

    /// A window of its own for `tab` (taken from this one), asking to be at `place`.
    fn tab_to_window(&mut self, tab: Tab, place: Option<vergence_ui::BarPlace>, window: &mut Window) {
        let mut t = self.window_for(String::new(), Vec::new(), None, None);
        t.moved = vec![tab];
        t.ask = place;
        // As large as the window it came from (Dave, 2026-11-12).
        t.ask_size = (self.width > 0.0 && self.height > 0.0).then(|| (self.width.round() as u32, self.height.round() as u32));
        window.open_window(Box::new(t));
    }

    /// Takes the tab with this id out of the window (the last one stays: a
    /// window keeps a tab).
    fn take_tab(&mut self, id: u32, window: &mut Window) -> Option<Tab> {
        if self.tabs.len() < 2 {
            return None;
        }
        let i = self.tabs.iter().position(|t| t.id == id)?;
        let front = self.tabs.get(self.shown).map(|t| t.id);
        let tab = self.tabs.remove(i);
        self.shown = front.and_then(|f| self.tabs.iter().position(|t| t.id == f)).unwrap_or_else(|| i.min(self.tabs.len() - 1));
        self.held = None;
        self.retitle(window);
        self.keep();
        window.redraw();
        Some(tab)
    }

    /// The active section (pane) of the front tab, out of its tab: its
    /// terminal keeps running. None if it is the tab's only one.
    fn take_pane(&mut self, window: &mut Window) -> Option<Tab> {
        let tab = self.tabs.get_mut(self.shown)?;
        if tab.panes.len() < 2 {
            return None;
        }
        let id = tab.active;
        let leaves = tab.tree.leaves();
        if !tab.tree.remove(id) {
            return None;
        }
        let i = tab.panes.iter().position(|(own, _)| *own == id)?;
        let (_, session) = tab.panes.remove(i);
        let at = leaves.iter().position(|l| *l == id).unwrap_or(0);
        tab.active = leaves[if at > 0 { at - 1 } else { 1 }];
        self.held = None;
        window.redraw();
        Some(Tab { id: NEXT_TAB.fetch_add(1, Ordering::SeqCst), tree: Node::Leaf(id), panes: vec![(id, session)], active: id })
    }

    /// Tabs other windows of the process sent here (a tab joined to this window).
    fn take_moves(&mut self, window: &mut Window) {
        let mine: Vec<(u64, Tab, usize)> = {
            let mut moves = self.moves.borrow_mut();
            let (mine, rest): (Vec<_>, Vec<_>) = moves.drain(..).partition(|(k, _, _)| *k == self.key);
            *moves = rest;
            mine
        };
        for (_, tab, to) in mine {
            let to = to.min(self.tabs.len());
            self.tabs.insert(to, tab);
            self.shown = to;
            self.fitted = (0.0, 0.0, 0.0, 0.0);
            self.retitle(window);
            self.keep();
            window.redraw();
        }
    }

    fn layout(&self) -> Layout {
        Layout::new(self.width, self.tabs.len(), self.shown, &self.settings.left, &self.settings.right)
    }

    /// What of our own bar is at a point; nothing when the server draws the bar.
    fn bar_at(&self, x: f32, y: f32) -> Option<Part> {
        if self.own_bar { self.layout().at(x, y) } else { None }
    }

    /// The room under the bar, which the front tab's panes share.
    fn room(&self) -> Rect {
        Rect::new(0.0, self.bar_h, self.width, (self.height - self.bar_h).max(0.0))
    }

    /// Where the front tab's panes and the lines between them are.
    fn areas(&self) -> (Vec<(u64, Rect)>, Vec<Divider>) {
        self.tabs.get(self.shown).map(|tab| tab.tree.layout(self.room())).unwrap_or_default()
    }

    /// The pane under a point, and its room.
    fn pane_at(&self, x: f32, y: f32) -> Option<(u64, Rect)> {
        self.areas().0.into_iter().find(|(_, r)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
    }

    /// The front tab's active pane, and its room.
    fn active(&self) -> Option<(u64, Rect)> {
        let active = self.tabs.get(self.shown)?.active;
        self.areas().0.into_iter().find(|(id, _)| *id == active)
    }

    /// A terminal of the front tab.
    fn session(&self, id: u64) -> Option<&Session> {
        self.tabs.get(self.shown)?.session(id)
    }

    fn session_mut(&mut self, id: u64) -> Option<&mut Session> {
        self.tabs.get_mut(self.shown)?.session_mut(id)
    }

    /// The terminal the keys go to.
    fn front(&self) -> Option<&Session> {
        let tab = self.tabs.get(self.shown)?;
        tab.session(tab.active)
    }

    fn front_mut(&mut self) -> Option<&mut Session> {
        let tab = self.tabs.get_mut(self.shown)?;
        tab.session_mut(tab.active)
    }

    /// A cell's width and height, once the fonts are known.
    fn cell(&self) -> Option<(f32, f32)> {
        self.metrics.as_ref().map(|mt| (mt.cell_w, mt.cell_h))
    }

    /// Starts a terminal and gives it a number.
    fn start(&mut self, cols: usize, rows: usize) -> Option<(u64, Session)> {
        let waker = self.waker.clone()?;
        // A restored tab starts where its shell stood; then where the request said.
        let cwd = if self.start_dirs.is_empty() { self.start_cwd.clone() } else { Some(self.start_dirs.remove(0)) };
        // `-e`'s program runs in the first terminal only; the others get the shell.
        match Session::start(cols, rows, waker, self.run.take(), cwd.as_deref()) {
            Ok(session) => {
                Some((NEXT_PANE.fetch_add(1, Ordering::SeqCst), session))
            }
            Err(e) => {
                eprintln!("vergence-term: could not start the program: {e}");
                None
            }
        }
    }

    /// The terminal's settings window (Ctrl+, or Settings… in the app menu);
    /// one at a time.
    fn open_settings(&mut self, window: &mut Window) {
        if self.prefs_open.get() {
            return;
        }
        let rows = Box::new(prefs::TermRows::new());
        window.open_window(vergence_widgets::window(vergence_widgets::prefs::Prefs::new("Terminal", "vergence-term", "[terminal]", true, rows, self.prefs_open.clone())));
    }

    /// A new window of this terminal, with a shell of its own.
    fn new_window(&mut self, window: &mut Window) {
        window.open_window(Box::new(self.window_for(String::new(), Vec::new(), None, None)));
    }

    /// Text size for a while: a step larger or smaller, or (None) as the settings say.
    fn zoom_by(&mut self, step: Option<f32>, window: &mut Window) {
        let z = match step {
            Some(s) => ((self.zoom + s) * 10.0).round() / 10.0,
            None => 1.0,
        };
        let z = z.clamp(ZOOM_MIN, ZOOM_MAX);
        if (z - self.zoom).abs() > 0.001 {
            self.zoom = z;
            self.metrics = None;
            window.redraw();
        }
    }

    fn new_tab(&mut self, window: &mut Window) {
        if let Some((id, session)) = self.start(self.fits.0, self.fits.1) {
            self.tabs.push(Tab { id: NEXT_TAB.fetch_add(1, Ordering::SeqCst), tree: Node::Leaf(id), panes: vec![(id, session)], active: id });
            self.shown = self.tabs.len() - 1;
            self.retitle(window);
        }
        self.keep();
        window.redraw();
    }

    /// Divides the active pane's room in two, with a new terminal in the
    /// second half, which the keys then go to.
    /// What a line of the right-click menu, or the same command from the app
    /// menu, does.
    fn do_item(&mut self, item: Item, window: &mut Window) {
        match item {
            Item::Copy => {
                self.copy(window);
            }
            Item::Paste => window.paste(),
            Item::SplitSide => self.split(Axis::Side, window),
            Item::SplitStack => self.split(Axis::Stack, window),
            Item::NewTab => self.new_tab(window),
            Item::ClosePane => self.close_active(window),
            Item::PaneToTab => {
                if let Some(t) = self.take_pane(window) {
                    self.tabs.push(t);
                    self.shown = self.tabs.len() - 1;
                    self.fitted = (0.0, 0.0, 0.0, 0.0);
                    self.retitle(window);
                    self.keep();
                }
            }
            Item::PaneToWindow => {
                // A section of a tab with others; a tab of its own otherwise.
                let moved = match self.take_pane(window) {
                    Some(t) => Some(t),
                    None => self.tabs.get(self.shown).map(|t| t.id).and_then(|id| self.take_tab(id, window)),
                };
                if let Some(t) = moved {
                    // A bar's height down and right of this window, as large.
                    let step = vergence_ui::bar::BAR_H as f64;
                    self.tab_to_window(t, Some(vergence_ui::BarPlace::Parent { x: step, y: step }), window);
                }
            }
        }
    }

    fn split(&mut self, axis: Axis, window: &mut Window) {
        self.split_active(axis);
        self.retitle(window);
        window.redraw();
    }

    fn split_active(&mut self, axis: Axis) {
        let Some(active) = self.tabs.get(self.shown).map(|tab| tab.active) else { return };
        // The new terminal starts at the size of the one it divides; the next frame fits both.
        let size = self.session(active).map(|s| {
            let s = lock(&s.screen);
            (s.grid.cols, s.grid.rows)
        });
        let Some((id, session)) = self.start(size.map_or(self.fits.0, |s| s.0), size.map_or(self.fits.1, |s| s.1)) else { return };
        let tab = &mut self.tabs[self.shown];
        if tab.tree.split(active, axis, id) {
            tab.panes.push((id, session));
            tab.active = id;
        }
    }

    /// Closes a tab (its programs are ended); the window with the last one.
    fn close_tab(&mut self, index: usize, window: &mut Window) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        self.held = None;
        if !self.tabs.is_empty() {
            self.keep();
        }
        if self.tabs.is_empty() {
            window.close();
            return;
        }
        if self.shown > index || self.shown >= self.tabs.len() {
            self.shown = self.shown.saturating_sub(1);
        }
        self.retitle(window);
        window.redraw();
    }

    /// Closes a pane (its program is ended): its neighbor gets its room.
    /// The tab goes with its last pane.
    fn close_pane(&mut self, index: usize, id: u64, window: &mut Window) {
        let Some(tab) = self.tabs.get_mut(index) else { return };
        if tab.panes.len() <= 1 {
            return self.close_tab(index, window);
        }
        let leaves = tab.tree.leaves();
        if !tab.tree.remove(id) {
            return;
        }
        tab.panes.retain(|(own, _)| *own != id);
        if tab.active == id {
            // The keys go to the pane that came before it, or the first.
            let at = leaves.iter().position(|l| *l == id).unwrap_or(0);
            tab.active = leaves[if at > 0 { at - 1 } else { 1 }];
        }
        self.held = None;
        self.retitle(window);
        window.redraw();
    }

    fn close_active(&mut self, window: &mut Window) {
        if let Some(active) = self.tabs.get(self.shown).map(|tab| tab.active) {
            self.close_pane(self.shown, active, window);
        }
    }

    fn show_tab(&mut self, index: usize, window: &mut Window) {
        if index < self.tabs.len() && index != self.shown {
            if self.measuring {
                self.asked = Some((index, self.frame_now, Instant::now()));
                eprintln!("press for tab {index} at {:.3}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() % 1000.0));
            }
            self.shown = index;
            self.held = None;
            self.retitle(window);
            window.redraw();
        }
    }

    /// The left button went down on the active pane's text: a selection
    /// starts (by characters, then words, then lines for presses on the
    /// same cell in quick succession), or with Shift the one there is
    /// reaches to here.
    fn press_text(&mut self, x: f32, y: f32, m: Modifiers, window: &mut Window) {
        let (Some(cell), Some((_, area)), Some(session)) = (self.cell(), self.active(), self.front()) else { return };
        let mut guard = lock(&session.screen);
        let s = &mut *guard;
        // A program that asked for the mouse has it (report()); Shift takes it
        // back, and scrolled back the text under the pointer is ours.
        if s.grid.mouse != MouseMode::Off && !m.shift && s.scroll == 0 {
            return;
        }
        let pos = s.cell_at(x, y, area, cell);
        let now = Instant::now();
        let nth = match self.presses {
            Some((then, at, n)) if at == pos && now.duration_since(then) < DOUBLE_CLICK => n % 3 + 1,
            _ => 1,
        };
        if nth == 1 && m.shift && s.selection.is_some() {
            s.drag_selection(pos);
        } else {
            s.selection = Some(Selection::start(&s.grid, [Unit::Cell, Unit::Word, Unit::Line][nth - 1], pos));
        }
        drop(guard);
        self.presses = Some((now, pos, nth));
        self.held = Some(Held::Text);
        window.redraw();
    }

    /// The active pane's selected text, if there is any.
    fn selected_text(&self) -> Option<String> {
        let s = lock(&self.front()?.screen);
        s.selection.and_then(|sel| sel.text(&s.grid))
    }

    /// Copies what is selected in the active pane; false if nothing is.
    fn copy(&mut self, window: &mut Window) -> bool {
        match self.selected_text() {
            Some(text) => {
                window.copy(&text);
                true
            }
            None => false,
        }
    }

    /// Input while the menu is open is the menu's: a press on a line does
    /// what it says, a press anywhere else or Escape closes it. True if the
    /// input was used up here.
    fn menu_input(&mut self, input: &Input, window: &mut Window) -> bool {
        let Some(menu) = &self.menu else { return false };
        match input {
            Input::PointerMoved { x, y } => {
                let over = menu.at(*x, *y);
                window.set_cursor(Cursor::Default);
                if over != self.menu_hover {
                    self.menu_hover = over;
                    window.redraw();
                }
                true
            }
            Input::Button { button, pressed: true, x, y, .. } => {
                let chosen = if *button == Button::Left { menu.at(*x, *y) } else { None };
                // A press on the menu's own ground, between its lines, leaves it open.
                if chosen.is_none() && *button == Button::Left && menu.contains(*x, *y) {
                    return true;
                }
                self.menu = None;
                self.menu_hover = None;
                window.redraw();
                if let Some(item) = chosen {
                    self.do_item(item, window);
                }
                true
            }
            Input::Button { .. } | Input::Scroll { .. } => true,
            Input::Key { sym: sym::ESCAPE, pressed: true, .. } | Input::Focus(false) => {
                self.menu = None;
                self.menu_hover = None;
                window.redraw();
                matches!(input, Input::Key { .. })
            }
            _ => false,
        }
    }

    /// The scrollbar of a pane of the front tab, if it has one now.
    fn scrollbar(&self, id: u64, area: Rect) -> Option<Scrollbar> {
        lock(&self.session(id)?.screen).scrollbar(area)
    }

    /// The held thumb follows the pointer, and the active pane's view with it.
    fn drag_thumb(&mut self, y: f32, grab: f32, window: &mut Window) {
        let (Some((_, area)), Some(session)) = (self.active(), self.front()) else { return };
        let mut s = lock(&session.screen);
        if let Some(bar) = s.scrollbar(area) {
            let to = bar.scroll_for(y - grab);
            if to != s.scroll {
                s.scroll = to;
                s.clamp_scroll();
                window.redraw();
            }
        }
    }

    /// Tells the program in a pane of the front tab about a mouse event at
    /// a point of the window, if it asked for such events. True if the
    /// event was the program's.
    fn report(&mut self, id: u64, area: Rect, action: mouse::Action, x: f32, y: f32, m: Modifiers) -> bool {
        let Some(cell) = self.cell() else { return false };
        let last = self.reported_cell;
        let Some(session) = self.session_mut(id) else { return false };
        let s = lock(&session.screen);
        if s.grid.mouse == MouseMode::Off || s.scroll > 0 {
            // Scrolled back, what is under the pointer is not the program's screen.
            return false;
        }
        let at = s.screen_cell(x, y, area, cell);
        if matches!(action, mouse::Action::Motion(_)) && last == Some((id, at)) {
            return true;
        }
        let bytes = mouse::bytes(s.grid.mouse, s.grid.mouse_sgr, action, at, m);
        drop(s);
        if let Some(bytes) = bytes {
            session.pty.write(&bytes);
        }
        self.reported_cell = Some((id, at));
        true
    }

    /// The pointer moved with the button held on the active pane's text.
    fn drag_text(&mut self, x: f32, y: f32, window: &mut Window) {
        let (Some(cell), Some((_, area)), Some(session)) = (self.cell(), self.active(), self.front()) else { return };
        let mut s = lock(&session.screen);
        let pos = s.cell_at(x, y, area, cell);
        let bottom = area.y + PAD + s.grid.rows as f32 * cell.1;
        // Past the top or bottom edge the view scrolls for as long as it is held there (scene()).
        let edge = if y < area.y + PAD {
            Some((true, pos.0))
        } else if y >= bottom {
            Some((false, pos.0))
        } else {
            None
        };
        if s.drag_selection(pos) || edge.is_some() {
            window.redraw();
        }
        drop(s);
        self.edge = edge;
    }

    /// The pointer moved with nothing held: what it is over, the shape it
    /// takes, and word of it to a program that asked to hear.
    fn hover(&mut self, x: f32, y: f32, window: &mut Window) {
        let part = self.bar_at(x, y);
        let pane = if part.is_none() && y >= self.bar_h { self.pane_at(x, y) } else { None };
        self.over = pane.map(|(id, _)| id);
        let divider = if part.is_none() { self.areas().1.into_iter().find(|d| d.holds(x, y)) } else { None };
        let cursor = if let Some(d) = divider {
            Cursor::Resize(if d.axis == Axis::Side { Edges { right: true, ..Edges::default() } } else { Edges { bottom: true, ..Edges::default() } })
        } else if let Some((id, area)) = pane {
            // The scrollbar's edge and a program that has the mouse get the arrow; text, the text bar.
            let on_bar = self.scrollbar(id, area).is_some_and(|bar| bar.holds(x, y));
            let theirs = !on_bar && !self.shift && self.report(id, area, mouse::Action::Motion(None), x, y, Modifiers::default());
            if on_bar || theirs { Cursor::Default } else { Cursor::Text }
        } else {
            Cursor::Default
        };
        window.set_cursor(cursor);
        if part != self.hover {
            self.hover = part;
            window.redraw();
        }
    }

    /// A button went down below the bar.
    fn press(&mut self, button: Button, x: f32, y: f32, m: Modifiers, window: &mut Window) {
        // The line between two panes: the left button takes hold of it.
        if button == Button::Left
            && let Some(d) = self.areas().1.into_iter().find(|d| d.holds(x, y))
        {
            self.held = Some(Held::Divider(d.path));
            return;
        }
        let Some((id, area)) = self.pane_at(x, y) else { return };
        // A press on a pane makes it the one the keys go to.
        if let Some(tab) = self.tabs.get_mut(self.shown)
            && tab.active != id
        {
            tab.active = id;
            self.retitle(window);
            window.redraw();
        }
        // The scrollbar: a press takes hold of the thumb, which comes to the pointer if it was not under it.
        if button == Button::Left
            && let Some(bar) = self.scrollbar(id, area).filter(|bar| bar.holds(x, y))
        {
            let grab = bar.grab(y);
            self.held = Some(Held::Thumb(grab));
            return self.drag_thumb(y, grab, window);
        }
        if !m.shift && self.report(id, area, mouse::Action::Press(button), x, y, m) {
            self.held = Some(Held::Reported(button));
            return;
        }
        match button {
            Button::Left => self.press_text(x, y, m, window),
            Button::Right => {
                self.menu = Some(Menu::new(x, y, (self.width, self.height), self.selected_text().is_some()));
                self.menu_hover = None;
                window.redraw();
            }
            _ => {}
        }
    }

    /// The window is called what its front tab is.
    fn retitle(&self, window: &mut Window) {
        if let Some(tab) = self.tabs.get(self.shown) {
            window.set_title(&tab.label());
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

/// The scene's number for a face, given once in a frame.
fn font_number(scene: &mut Scene, fonts: &Fonts, numbers: &mut HashMap<FaceId, u32>, face: FaceId) -> u32 {
    *numbers.entry(face).or_insert_with(|| {
        let n = fonts.name(face);
        scene.font(&n.family, n.weight, n.italic)
    })
}

/// How a pane is drawn.
struct PaneLook<'a> {
    palette: &'a Palette,
    metrics: &'a Metrics,
    size: f32,
    /// The keys go to this pane, and the window has the keyboard.
    keyboard: bool,
    /// The scrollbar's thumb is held.
    thumb_held: bool,
    /// How much room above the pane lies under the window's bar: filled with
    /// the lines before the top row (none on the alternate screen).
    above: f32,
    /// The cursor's shape, and whether it shows now (it blinks).
    cursor: CursorShape,
    cursor_on: bool,
}

/// Draws one terminal in its room.
fn draw_pane(scene: &mut Scene, fonts: &mut Fonts, numbers: &mut HashMap<FaceId, u32>, s: &Screen, area: Rect, look: &PaneLook) {
    let (p, mt, size) = (look.palette, look.metrics, look.size);
    let (cw, ch) = (mt.cell_w, mt.cell_h);
    let (left, first) = (area.x + PAD, area.y + PAD);
    let (selection, top_line) = (s.selection, s.top_line());
    // The font and glyph of each plain character, regular and bold, found once
    // a frame: a screen of text asks for the same hundred some thousands of times.
    let mut plain: [[Option<(u32, u32)>; 128]; 2] = [[None; 128]; 2];
    // Under the window's bar, above the top row: the lines before it, as many
    // as reach into the strip (look milestone 1). The alternate screen has none.
    let extra = if s.grid.alt || look.above <= 0.0 { 0 } else { (((look.above + PAD) / ch).ceil() as usize).min(top_line) };
    scene.push_clip(area, 0.0);
    // The screen is one grid of cells ([`vergence_scene::Command::Cells`]): each
    // cell its ground (none where it is the terminal's own), its ink, its glyph.
    let columns = s.grid.cols;
    let mut cells: Vec<SceneCell> = Vec::with_capacity(columns * (s.grid.rows + extra));
    // Combining marks are few, and drawn over the grid as glyph runs of their own.
    let mut marks: Vec<(usize, u32, Color, Point, Glyph)> = Vec::new();
    let mut floor_memo = ((p.fg, p.fg), p.fg);
    let floor_bits = p.floor.to_bits();
    for r in 0..s.grid.rows + extra {
        let line = if r < extra { s.grid.abs_line(top_line - (extra - r)).unwrap_or(&[]) } else { s.shown(r - extra) };
        let top = first + (r as f32 - extra as f32) * ch;
        for (col, cell) in line.iter().take(columns).enumerate() {
            let st = cell.style;
            let (mut ink, mut ground) = (p.resolve(st.fg, p.fg, st.bold), p.resolve(st.bg, p.bg, false));
            if st.inverse {
                // The text takes the ground's color as it is, not its bright form.
                (ink, ground) = (ground, ink);
            }
            // The palette's floor (light ones): runs of one color pair are the
            // rule, so the last pair's answer is kept.
            if p.floor > 0.0 {
                if floor_memo.0 != (ink, ground) {
                    floor_memo = ((ink, ground), readable_cached(ink, ground, p.floor, floor_bits));
                }
                ink = floor_memo.1;
            }
            if st.dim {
                ink.a = 153;
            }
            let mut out = SceneCell { ground: if ground == p.bg { Color::rgba(0, 0, 0, 0) } else { ground }, ink, glyph: 0, font: 0, flags: if st.underline { CELL_UNDERLINE } else { 0 } };
            if cell.width != 0 && cell.c != ' ' {
                let own = if st.bold { mt.bold } else { mt.regular };
                let (font, glyph) = match plain[st.bold as usize].get_mut(cell.c as usize) {
                    Some(Some(known)) => *known,
                    slot => {
                        let (face, glyph) = fonts.glyph(own, cell.c);
                        let found = (font_number(scene, fonts, numbers, face), glyph);
                        if let Some(slot) = slot {
                            *slot = Some(found);
                        }
                        found
                    }
                };
                (out.glyph, out.font, out.flags) = (glyph, font as u16, out.flags | CELL_GLYPH);
                // Fonts make marks one of two ways: with no advance, drawn back over
                // what came before the pen (so the pen goes after the character), or as
                // wide as a cell, drawn in place (so it goes where the character's did).
                for mark in cell.marks.into_iter().filter(|m| *m != '\0') {
                    let (face, glyph) = fonts.glyph(own, mark);
                    let font = font_number(scene, fonts, numbers, face);
                    let after = fonts.advance(face, glyph, size) < cw * 0.5;
                    marks.push((r, font, ink, Point::new(left, top + mt.baseline), Glyph { id: glyph, x: (col + if after { cell.width.max(1) as usize } else { 0 }) as f32 * cw, y: 0.0 }));
                }
            }
            cells.push(out);
        }
        // A line shorter than the screen is wide (kept from a narrower one) ends in nothing.
        cells.resize((r + 1) * columns, SceneCell::default());
    }
    // The lines before the top row only in the strip under the bar, cut at
    // its edge: none of them shows in the pane's own room.
    let below = cells.split_off(extra * columns);
    if extra > 0 {
        scene.pop_clip();
        scene.push_clip(Rect::new(area.x, area.y - look.above, area.w, look.above), 0.0);
        scene.cells(Point::new(left, first - extra as f32 * ch), cw, ch, mt.baseline, size, columns as u32, cells);
        for (_, font, ink, origin, glyph) in marks.iter().filter(|m| m.0 < extra) {
            scene.glyphs(*font, size, *ink, *origin, vec![*glyph]);
        }
        scene.pop_clip();
        scene.push_clip(area, 0.0);
    }
    scene.cells(Point::new(left, first), cw, ch, mt.baseline, size, columns as u32, below);
    for (_, font, ink, origin, glyph) in marks.into_iter().filter(|m| m.0 >= extra) {
        scene.glyphs(font, size, ink, origin, vec![glyph]);
    }
    // What is selected, tinted over its text.
    for r in 0..s.grid.rows {
        if let Some((a, b)) = selection.and_then(|sel| sel.span(top_line + r, s.grid.cols)) {
            scene.fill(Rect::new(left + a as f32 * cw, first + r as f32 * ch, (b + 1 - a) as f32 * cw, ch), Color::rgba(p.accent.r, p.accent.g, p.accent.b, 89));
        }
    }
    // The cursor: a block where the keys go, an outline elsewhere. It moves down with the
    // screen while scrolled back, out of sight past the bottom.
    let cursor_row = s.grid.cy + s.scroll;
    if s.grid.cursor_visible && cursor_row < s.grid.rows {
        let at = Rect::new(left + s.grid.cx as f32 * cw, first + cursor_row as f32 * ch, cw, ch);
        // Its shape as the settings say: a block, a bar before the cell, a line under it.
        let shape = match look.cursor {
            CursorShape::Block => at,
            CursorShape::Bar => Rect::new(at.x, at.y, 2.0, at.h),
            CursorShape::Underline => Rect::new(at.x, at.y + at.h - 2.0, at.w, 2.0),
        };
        if look.keyboard {
            if look.cursor_on {
                scene.fill(shape, Color::rgba(p.accent.r, p.accent.g, p.accent.b, if look.cursor == CursorShape::Block { 178 } else { 230 }));
            }
        } else if look.cursor == CursorShape::Block {
            scene.stroke(at, 0.0, 1.0, p.accent);
        } else {
            scene.fill(shape, Color::rgba(p.accent.r, p.accent.g, p.accent.b, 140));
        }
    }
    // The scrollbar: faint at the live screen, clearer while scrolled back or held.
    if let Some(bar) = s.scrollbar(area) {
        scene.fill_rounded(bar.track, scrollbar::RADIUS, Color::rgba(p.fg.r, p.fg.g, p.fg.b, 15));
        let alpha = if look.thumb_held { 166 } else if s.scroll > 0 { 128 } else { 64 };
        scene.fill_rounded(bar.thumb, scrollbar::RADIUS, Color::rgba(p.fg.r, p.fg.g, p.fg.b, alpha));
    }
    // Scrolled back: how far, in the top right corner (left of the scrollbar).
    if s.scroll > 0 {
        let label = if s.scroll == 1 { "1 line back".to_string() } else { format!("{} lines back", s.scroll) };
        let width = label.chars().count() as f32 * cw + 16.0;
        let at = Rect::new(area.x + area.w - width - PAD - scrollbar::GRAB, first, width, ch + 4.0);
        scene.fill_rounded(at, bar::RADIUS, Color::rgba(p.accent.r, p.accent.g, p.accent.b, 235));
        let font = font_number(scene, fonts, numbers, mt.regular);
        let glyphs = label.chars().enumerate().map(|(i, c)| Glyph { id: fonts.glyph(mt.regular, c).1, x: i as f32 * cw, y: 0.0 }).collect();
        scene.glyphs(font, size, p.bg, Point::new(at.x + 8.0, at.y + 2.0 + mt.baseline), glyphs);
    }
    scene.pop_clip();
}

/// The app menu's command ids (app settings milestone 1).
mod cmd {
    pub const COPY: u32 = 1;
    pub const PASTE: u32 = 2;
    pub const NEW_TAB: u32 = 3;
    pub const CLOSE_TAB: u32 = 4;
    pub const NEXT_TAB: u32 = 5;
    pub const PREVIOUS_TAB: u32 = 6;
    pub const SPLIT_SIDE: u32 = 7;
    pub const SPLIT_STACK: u32 = 8;
    pub const PANE_TO_TAB: u32 = 9;
    pub const PANE_TO_WINDOW: u32 = 10;
    pub const NEW_WINDOW: u32 = 11;
    pub const ZOOM_IN: u32 = 12;
    pub const ZOOM_OUT: u32 = 13;
    pub const ZOOM_RESET: u32 = 14;
    pub const SETTINGS: u32 = 15;
}

/// The cursor's shape (the settings' `cursor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorShape {
    Block,
    Bar,
    Underline,
}

impl CursorShape {
    fn of(desktop: &str) -> CursorShape {
        match vergence_ui::table_value(desktop, "[terminal]", "cursor").as_deref() {
            Some("bar") => CursorShape::Bar,
            Some("underline") => CursorShape::Underline,
            _ => CursorShape::Block,
        }
    }
}

/// A switch of the terminal's settings (absent: `default`).
/// The settings' "Scroll on output" (Dave's list, 2026-11-15): while
/// scrolled back, new output brings the view down to it. Off by default:
/// the view stays on the text being read. Read by every tab's parse thread.
static SCROLL_ON_OUTPUT: AtomicBool = AtomicBool::new(false);

fn set_scroll_on_output(desktop: &str) {
    SCROLL_ON_OUTPUT.store(term_flag(desktop, "scroll-on-output", false), Ordering::Relaxed);
}

fn term_flag(desktop: &str, key: &str, default: bool) -> bool {
    vergence_ui::table_value(desktop, "[terminal]", key).map_or(default, |v| v == "true")
}

/// Lines each tab keeps (the settings' `scrollback`, 1,000 to 1,000,000).
fn scrollback_of(desktop: &str) -> usize {
    vergence_ui::table_value(desktop, "[terminal]", "scrollback").and_then(|v| v.parse::<usize>().ok()).map_or(10_000, |n| n.clamp(1_000, 1_000_000))
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
            window.open_window(vergence_widgets::window(confirm::CloseDialog { heading, detail, close: self.close_now.clone(), open: self.asking.clone(), waker: self.waker.clone(), theme }));
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
            tabs: tabs.iter().map(|(id, title, root)| vergence_ui::BarTab { id: *id, title: title.chars().take(200).collect(), closable: true, alert: *root, edited: false }).collect(),
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
        use vergence_ui::{AppCommand as C, Role};
        let selected = self.front().is_some_and(|s| lock(&s.screen).selection.is_some());
        let several = self.tabs.len() > 1;
        // The app menu holds the terminal's own few (Dave, 2026-11-14: the app
        // button is the app; a pane's and a tab's commands are on their right
        // click); the rest are listed under Keyboard shortcuts.
        let percent = format!("{}%", (self.zoom * 100.0).round());
        let list = vec![
            C::role(cmd::SETTINGS, Role::Settings),
            C::role(cmd::NEW_WINDOW, Role::NewWindow).featured(),
            C::role(cmd::NEW_TAB, Role::NewTab).featured(),
            C::role(cmd::ZOOM_OUT, Role::ZoomOut).enabled(self.zoom > ZOOM_MIN + 0.001).featured(),
            C { label: percent, ..C::role(cmd::ZOOM_RESET, Role::ZoomReset).featured() },
            C::role(cmd::ZOOM_IN, Role::ZoomIn).enabled(self.zoom < ZOOM_MAX - 0.001).featured(),
            C::role(cmd::COPY, Role::Copy).enabled(selected),
            C::role(cmd::PASTE, Role::Paste),
            C::role(cmd::CLOSE_TAB, Role::CloseTab),
            C::role(cmd::NEXT_TAB, Role::NextTab).enabled(several),
            C::role(cmd::PREVIOUS_TAB, Role::PreviousTab).enabled(several),
            C::own(cmd::SPLIT_SIDE, "Tab", "Split side by side", "Ctrl+Shift+R"),
            C::own(cmd::SPLIT_STACK, "Tab", "Split top and bottom", "Ctrl+Shift+D"),
            C::own(cmd::PANE_TO_TAB, "Tab", "Move section to a new tab", ""),
            C::own(cmd::PANE_TO_WINDOW, "Tab", "Move to a new window", ""),
        ];
        if self.commands_told.as_ref() == Some(&list) {
            return None;
        }
        self.commands_told = Some(list.clone());
        Some((true, list))
    }

    fn bar_style(&mut self) -> Option<vergence_ui::BarStyle> {
        (self.bar_style_told != Some(self.bar_style)).then(|| {
            self.bar_style_told = Some(self.bar_style);
            self.bar_style
        })
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        let item = match id {
            cmd::COPY => Item::Copy,
            cmd::PASTE => Item::Paste,
            cmd::NEW_TAB => Item::NewTab,
            cmd::CLOSE_TAB => Item::ClosePane,
            cmd::SPLIT_SIDE => Item::SplitSide,
            cmd::SPLIT_STACK => Item::SplitStack,
            cmd::PANE_TO_TAB => Item::PaneToTab,
            cmd::PANE_TO_WINDOW => Item::PaneToWindow,
            cmd::NEW_WINDOW => return self.new_window(window),
            cmd::SETTINGS => return self.open_settings(window),
            cmd::ZOOM_IN => return self.zoom_by(Some(ZOOM_STEP), window),
            cmd::ZOOM_OUT => return self.zoom_by(Some(-ZOOM_STEP), window),
            cmd::ZOOM_RESET => return self.zoom_by(None, window),
            cmd::NEXT_TAB | cmd::PREVIOUS_TAB => {
                let n = self.tabs.len().max(1);
                let to = if id == cmd::NEXT_TAB { (self.shown + 1) % n } else { (self.shown + n - 1) % n };
                return self.show_tab(to, window);
            }
            _ => return,
        };
        self.do_item(item, window);
    }

    fn tab(&mut self, event: vergence_ui::TabEvent, window: &mut Window) {
        use vergence_ui::TabEvent;
        let index = |id: u32| self.tabs.iter().position(|t| t.id == id);
        match event {
            TabEvent::Chosen(id) => {
                if let Some(i) = index(id) {
                    self.show_tab(i, window);
                }
            }
            TabEvent::Close(id) => {
                if let Some(i) = index(id) {
                    self.close_tab(i, window);
                }
            }
            TabEvent::CloseOthers(id) => {
                while let Some(i) = self.tabs.iter().position(|t| t.id != id) {
                    self.close_tab(i, window);
                }
            }
            TabEvent::New => self.new_tab(window),
            TabEvent::Moved { tab, to } => {
                if let Some(from) = index(tab) {
                    let front = self.tabs.get(self.shown).map(|t| t.id);
                    bar::reorder(&mut self.tabs, from, to as usize);
                    self.keep();
                    if let Some(f) = front.and_then(|f| self.tabs.iter().position(|t| t.id == f)) {
                        self.shown = f;
                    }
                    window.redraw();
                }
            }
            // Dragged out (or "Move to a new window"): a window of its own, where it was dropped.
            TabEvent::Out { tab, place } => {
                if let Some(t) = self.take_tab(tab, window) {
                    self.tab_to_window(t, Some(place), window);
                }
            }
            // Dropped on another window of this process: it goes there.
            TabEvent::Join { tab, into, to } => {
                let target = self.wids.borrow().iter().find(|(w, _)| *w == into).map(|(_, k)| *k);
                // A window's only tab may go too: the window closes behind it.
                let last = || self.tabs.len() == 1 && self.tabs[0].id == tab;
                let take = if target.is_some() && last() {
                    self.held = None;
                    window.close();
                    self.tabs.pop()
                } else if target.is_some() {
                    self.take_tab(tab, window)
                } else {
                    None
                };
                if let Some(key) = target
                    && let Some(t) = take
                {
                    self.moves.borrow_mut().push((key, t, to as usize));
                    if let Some(w) = &self.waker {
                        w.wake();
                    }
                }
            }
            // The terminal adds no entries of its own to a tab's menu yet.
            TabEvent::Menu { .. } => {}
        }
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
                Some((id, session)) => self.tabs.push(Tab { id: NEXT_TAB.fetch_add(1, Ordering::SeqCst), tree: Node::Leaf(id), panes: vec![(id, session)], active: id }),
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
        let now = Instant::now();
        match id {
            // The cursor blinks while the window has the keyboard.
            TIMER_BLINK => {
                if self.focused && self.blink {
                    self.blink_on = !self.blink_on;
                    window.redraw();
                    window.wake_after(TIMER_BLINK, BLINK);
                } else if !self.blink_on {
                    self.blink_on = true;
                    window.redraw();
                }
            }
            TIMER_ROOT => {
                let now_root: Vec<bool> = self.tabs.iter().map(Tab::as_root).collect();
                let told: Vec<bool> = self.bar_told.as_ref().map(|(tabs, ..)| tabs.iter().map(|t| t.2).collect()).unwrap_or_default();
                if now_root != told {
                    window.redraw();
                }
                window.wake_after(TIMER_ROOT, ROOT_TICK);
            }
            // A finger held still on the text is the right button there: the menu.
            TIMER_LONG_PRESS => {
                if let Some(vergence_ui::touch::Gesture::LongPress { x, y }) = self.fingers.long_press(now)
                    && y >= self.bar_h
                {
                    let modifiers = Modifiers { shift: false, ctrl: false, alt: false, logo: false };
                    self.input(&Input::PointerMoved { x, y }, window);
                    self.input(&Input::Button { button: Button::Right, pressed: true, x, y, modifiers }, window);
                    self.input(&Input::Button { button: Button::Right, pressed: false, x, y, modifiers }, window);
                }
            }
            TIMER_FLICK => {
                let Some((v, from, last)) = self.flick else { return };
                let dt = now.saturating_duration_since(last).as_secs_f32().min(0.1);
                self.finger_drag(v * dt, from, window);
                let v = v * (-FLICK_FRICTION * dt).exp();
                if v.abs() < FLICK_STOP {
                    self.flick = None;
                    (self.dragged, self.dragged_lines) = (0.0, 0);
                } else {
                    self.flick = Some((v, from, now));
                    window.wake_after(TIMER_FLICK, FLICK_TICK);
                }
            }
            _ => {}
        }
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
            self.bar_style = vergence_ui::bar_style_setting(&config("desktop.toml"), "[terminal]", vergence_ui::BarStyle::Desktop);
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
        while ended {
            let gone = self.tabs.iter().enumerate().find_map(|(index, tab)| tab.panes.iter().find(|(_, s)| lock(&s.screen).exited).map(|(id, _)| (index, *id)));
            let Some((index, id)) = gone else { break };
            self.close_pane(index, id, window);
            if self.tabs.is_empty() {
                return;
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
        // A finger (vergence_ui::touch): a tap is a click where it tapped; a
        // drag up or down scrolls what is under where the finger came down,
        // the text moving with the finger.
        if let Input::Touch { phase, .. } = input {
            use vergence_ui::touch::{Gesture, LONG_PRESS, Phase};
            match phase {
                // A finger coming down stops a flick, and may be a long press.
                Phase::Down => {
                    self.flick = None;
                    window.wake_after(TIMER_LONG_PRESS, LONG_PRESS);
                }
                Phase::Up | Phase::Cancelled => window.cancel_timer(TIMER_LONG_PRESS),
                Phase::Moved => {}
            }
            match self.fingers.read(input) {
                Some(Gesture::Tap { x, y }) => {
                    let modifiers = Modifiers { shift: false, ctrl: false, alt: false, logo: false };
                    self.input(&Input::PointerMoved { x, y }, window);
                    self.input(&Input::Button { button: Button::Left, pressed: true, x, y, modifiers }, window);
                    self.input(&Input::Button { button: Button::Left, pressed: false, x, y, modifiers }, window);
                    self.input(&Input::PointerLeft, window);
                }
                // A drag that began on the bar is the pointer's with its button
                // held: the window is moved by its bar, a tab along the row.
                Some(Gesture::Drag { x, y, from, .. }) if from.1 < self.bar_h => {
                    if self.finger_pointer.is_none() {
                        let modifiers = Modifiers { shift: false, ctrl: false, alt: false, logo: false };
                        self.input(&Input::PointerMoved { x: from.0, y: from.1 }, window);
                        self.input(&Input::Button { button: Button::Left, pressed: true, x: from.0, y: from.1, modifiers }, window);
                    }
                    self.finger_pointer = Some((x, y));
                    self.input(&Input::PointerMoved { x, y }, window);
                }
                Some(Gesture::Drag { dy, from, .. }) => {
                    window.cancel_timer(TIMER_LONG_PRESS);
                    self.drag_from = Some(from);
                    self.finger_drag(dy, from, window);
                }
                Some(Gesture::DragEnd { vy, .. }) => {
                    // A flick of the text goes on, slowing, after the finger lifts.
                    if self.finger_pointer.is_none()
                        && let Some(from) = self.drag_from.take()
                        && vy.abs() > FLICK_MIN
                    {
                        self.flick = Some((vy.clamp(-FLICK_MAX, FLICK_MAX), from, Instant::now()));
                        window.wake_after(TIMER_FLICK, FLICK_TICK);
                    } else {
                        (self.dragged, self.dragged_lines) = (0.0, 0);
                    }
                    if let Some((x, y)) = self.finger_pointer.take() {
                        let modifiers = Modifiers { shift: false, ctrl: false, alt: false, logo: false };
                        self.input(&Input::Button { button: Button::Left, pressed: false, x, y, modifiers }, window);
                        self.input(&Input::PointerLeft, window);
                    }
                }
                Some(Gesture::LongPress { .. }) | None => {}
            }
            return;
        }
        if self.menu_input(input, window) {
            return;
        }
        match input {
            // With something held, the pointer's moves are that thing's.
            Input::PointerMoved { x, y } if self.held.is_some() => match &self.held {
                Some(Held::Text) => self.drag_text(*x, *y, window),
                Some(Held::Thumb(grab)) => self.drag_thumb(*y, *grab, window),
                Some(Held::Divider(path)) => {
                    let path = path.clone();
                    if let Some(d) = self.areas().1.into_iter().find(|d| d.path == path)
                        && let Some(tab) = self.tabs.get_mut(self.shown)
                    {
                        tab.tree.set_share(&path, d.share_at(*x, *y));
                        window.redraw();
                    }
                }
                Some(Held::Reported(button)) => {
                    // The program hears of the drag wherever the pointer goes.
                    let button = *button;
                    if let Some((id, area)) = self.active() {
                        self.report(id, area, mouse::Action::Motion(Some(button)), *x, *y, Modifiers::default());
                    }
                }
                None => {}
            },
            Input::PointerMoved { x, y } => self.hover(*x, *y, window),
            Input::PointerLeft => {
                self.over = None;
                if self.hover.take().is_some() {
                    window.redraw();
                }
            }
            Input::Button { button, pressed: false, x, y, modifiers } if self.held.is_some() => match &self.held {
                Some(Held::Reported(told)) if told == button => {
                    self.held = None;
                    if let Some((id, area)) = self.active() {
                        self.report(id, area, mouse::Action::Release(*button), *x, *y, *modifiers);
                    }
                }
                Some(Held::Reported(_)) => {}
                Some(Held::Text) if *button == Button::Left => {
                    self.held = None;
                    self.edge = None;
                    // A press that selected nothing leaves nothing selected.
                    if let Some(session) = self.front() {
                        let mut s = lock(&session.screen);
                        if s.selection.is_some_and(|sel| sel.range().is_none()) {
                            s.selection = None;
                        }
                    }
                    window.redraw();
                }
                Some(Held::Thumb(_) | Held::Divider(_)) if *button == Button::Left => {
                    self.held = None;
                    window.redraw();
                }
                _ => {}
            },
            // One thing is held at a time: other buttons wait for its release.
            Input::Button { pressed: true, .. } if self.held.is_some() => {}
            Input::Button { button, pressed: true, x, y, modifiers } => match (button, self.bar_at(*x, *y)) {
                (_, None) if *y >= self.bar_h => self.press(*button, *x, *y, *modifiers, window),
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
                // Typing shows the cursor at once; it blinks again from there.
                if self.blink && !self.blink_on {
                    self.blink_on = true;
                    window.redraw();
                }
                let letter = |c: char| *key == c as u32 || *key == c.to_ascii_uppercase() as u32;
                // The terminal's own shortcuts: with Ctrl+Shift, so the shell keeps Ctrl+T and the rest.
                // Ctrl+, : the terminal's settings, as every app's.
                if m.ctrl && !m.alt && !m.shift && *key == 0x2c {
                    return self.open_settings(window);
                }
                // Text size for a while: Ctrl with = (or +), - and 0, as GNOME Terminal.
                // Ctrl+_ (Shift with -) stays the shell's undo.
                if m.ctrl && !m.alt {
                    match *key {
                        0x3d | 0x2b | 0xffab => return self.zoom_by(Some(ZOOM_STEP), window),
                        0x2d | 0xffad if !m.shift => return self.zoom_by(Some(-ZOOM_STEP), window),
                        0x30 | 0xffb0 if !m.shift => return self.zoom_by(None, window),
                        _ => {}
                    }
                }
                if m.ctrl && m.shift {
                    if letter('n') {
                        return self.new_window(window);
                    }
                    if letter('t') {
                        return self.new_tab(window);
                    }
                    if letter('w') {
                        return self.close_active(window);
                    }
                    if letter('r') {
                        return self.split(Axis::Side, window);
                    }
                    if letter('d') {
                        return self.split(Axis::Stack, window);
                    }
                    if letter('c') {
                        self.copy(window);
                        return;
                    }
                }
                // The next tab and the one before: Ctrl+PageDown and Ctrl+PageUp, or
                // Ctrl+Tab and, with Shift, back (which arrives as its own key).
                const BACK_TAB: u32 = 0xfe20;
                let step = match *key {
                    sym::PAGE_UP if m.ctrl && !m.shift => Some(false),
                    sym::PAGE_DOWN if m.ctrl && !m.shift => Some(true),
                    sym::TAB if m.ctrl && !m.alt => Some(!m.shift),
                    BACK_TAB if m.ctrl && !m.alt => Some(false),
                    _ => None,
                };
                if let Some(forward) = step {
                    let n = self.tabs.len().max(1);
                    return self.show_tab(if forward { (self.shown + 1) % n } else { (self.shown + n - 1) % n }, window);
                }
                // Ctrl+V pastes, with Shift or without (Dave, 2026-11-09). The control
                // character it used to send (the shell's "take the next key as it is") is
                // given up for it.
                if m.ctrl && !m.alt && letter('v') {
                    return window.paste();
                }
                // Ctrl+C copies when there is something selected, and lets it go, so that
                // the next Ctrl+C is the interrupt key again; with nothing selected it
                // always is.
                if m.ctrl && !m.alt && !m.shift && letter('c') && self.copy(window) {
                    if let Some(session) = self.front() {
                        lock(&session.screen).selection = None;
                    }
                    return window.redraw();
                }
                let Some(session) = self.front_mut() else { return };
                let mut s = lock(&session.screen);
                // The scrollback: with Shift, PageUp and PageDown a page, Home and End
                // its two ends. On the alternate screen there is none, and the keys
                // are the program's.
                if m.shift && !m.ctrl && !m.alt && !s.grid.alt && matches!(*key, sym::PAGE_UP | sym::PAGE_DOWN | sym::HOME | sym::END) {
                    let page = s.grid.rows.saturating_sub(1).max(1) as isize;
                    s.scroll_by(match *key {
                        sym::PAGE_UP => page,
                        sym::PAGE_DOWN => -page,
                        sym::HOME => isize::MAX,
                        _ => isize::MIN,
                    });
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
                let Some(session) = self.front_mut() else { return };
                let bracketed = lock(&session.screen).grid.bracketed_paste;
                // A pasted line end is the Return key; and the paste cannot end its own brackets.
                let text = text.replace("\r\n", "\r").replace('\n', "\r").replace("\x1b[201~", "");
                let bytes = if bracketed { format!("\x1b[200~{text}\x1b[201~") } else { text };
                session.pty.write(bytes.as_bytes());
            }
            Input::Scroll { x, y, dy, .. } => {
                // The wheel is the pane's under the pointer, whichever has the keys.
                let Some((id, area)) = self.pane_at(*x, *y).filter(|_| *y >= self.bar_h) else { return };
                let notches = (dy / 15.0).round() as isize;
                // A program with the mouse gets the wheel, a report for each notch.
                if !self.shift && notches != 0 {
                    let action = if notches < 0 { mouse::Action::WheelUp } else { mouse::Action::WheelDown };
                    if self.report(id, area, action, *x, *y, Modifiers::default()) {
                        for _ in 1..notches.unsigned_abs().min(20) {
                            self.report(id, area, action, *x, *y, Modifiers::default());
                        }
                        return;
                    }
                }
                let Some(session) = self.session_mut(id) else { return };
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
                self.blink_on = true;
                if *on && self.blink {
                    window.wake_after(TIMER_BLINK, BLINK);
                }
                window.redraw();
            }
            _ => {}
        }
    }

    fn scene(&mut self, frame: &Frame, fonts: &mut Fonts) -> Scene {
        (self.width, self.height) = (frame.width, frame.height);
        // For measuring (VTERM_FRAMES=1): the count of frames every thirtieth, and
        // for a tab that was asked for, how many frames and how long it took to be drawn.
        if self.measuring {
            if frame.number % 30 == 0 {
                eprintln!("frame {}", frame.number);
            }
            if let Some((tab, after, when)) = self.asked.take() {
                eprintln!("tab {tab} asked after frame {after}, drawn in frame {}, {:.1} ms later", frame.number, when.elapsed().as_secs_f64() * 1000.0);
            }
            self.frame_now = frame.number;
        }
        let mut scene = Scene::new(frame.width, frame.height);
        // Room for as much as the last frame held, so the list is not grown step by step.
        scene.commands.reserve(self.last_commands);
        // The ground is the window's background; drawn again here only when
        // nothing is to show through it.
        if self.opacity >= 1.0 {
            scene.fill(Rect::new(0.0, 0.0, frame.width, frame.height), self.palette.bg);
        }
        let size = self.settings.mono_size * self.zoom;
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
        let (panes, dividers) = self.areas();
        // A selection held past the top or bottom edge: the view moves a line at a time and
        // the selection's end goes with the edge row.
        let scroll_edge = self.edge.filter(|_| self.edge_tick.elapsed() >= EDGE_TICK);
        if scroll_edge.is_some() {
            self.edge_tick = Instant::now();
        }
        let p = &self.palette;
        let Some(mt) = &self.metrics else { return scene };
        let (cw, ch) = (mt.cell_w, mt.cell_h);
        // The grid that fits a room: the whole of it for a new tab, a pane's for its terminal.
        // Columns stop short of the scrollbar at the right edge (Dave's list,
        // 2026-11-16: the text ran under it), always, so a terminal's width
        // does not change when its first line scrolls away.
        let fit = |w: f32, h: f32| ((((w - PAD - RIGHT_GUTTER) / cw).floor().max(2.0) as usize).min(2000), (((h - 2.0 * PAD) / ch).floor().max(1.0) as usize).min(1000));
        // The server draws the bar on the native road: the room starts at the top
        // then, or below the strip we draw under its bar.
        self.own_bar = !frame.bar_by_server;
        self.bar_h = if frame.bar_by_server { frame.top } else { BAR_H };
        self.fits = fit(frame.width, frame.height - self.bar_h);
        // The tabs behind are fitted as well, whenever the window's or the cells'
        // size changes: a program in a tab nobody is looking at is told its real
        // size all the same (it was left at the size it started with until shown).
        let fitted = (frame.width, frame.height, cw, ch);
        if self.fitted != fitted {
            self.fitted = fitted;
            let room = self.room();
            for (_, tab) in self.tabs.iter().enumerate().filter(|(index, _)| *index != self.shown) {
                for (id, area) in tab.tree.layout(room).0 {
                    if let Some(session) = tab.session(id) {
                        let (cols, rows) = fit(area.w, area.h);
                        session.fit(&mut lock(&session.screen), cols, rows);
                    }
                }
            }
        }

        if !frame.bar_by_server {
            let titles: Vec<String> = self.tabs.iter().map(Tab::label).collect();
            let alerts: Vec<bool> = self.tabs.iter().map(Tab::as_root).collect();
            let layout = Layout::new(frame.width, self.tabs.len(), self.shown, &self.settings.left, &self.settings.right);
            bar::draw(&mut scene, fonts, &layout, &bar::Look { bar: vergence_look::bar_look(self.settings.light), frosted: false, focused: self.focused, pane: p.bg, pane_ink: p.fg, font: mt.ui, font_size: self.settings.ui_size, titles: &titles, alerts: &alerts, edited: &[], title: "", shown: self.shown, hover: self.hover, dragged: None, tools: &[], app_icon: None, menu_open: false, root: false });
        }

        let Some(tab) = self.tabs.get(self.shown) else { return scene };
        // Scene font numbers for the faces used in this frame.
        let mut numbers: HashMap<FaceId, u32> = HashMap::new();
        for (id, area) in &panes {
            let Some(session) = tab.session(*id) else { continue };
            let active = *id == tab.active;
            // What is drawn now is drawn: output after this asks for a frame again.
            session.news.fetch_and(!NEWS_DRAW, Ordering::SeqCst);
            let mut guard = lock(&session.screen);
            let s = &mut *guard;
            // The terminal's grid follows its room, and its program is told.
            let (cols, rows) = fit(area.w, area.h);
            session.fit(s, cols, rows);
            if active && let Some((up, col)) = scroll_edge {
                s.scroll_by(if up { 1 } else { -1 });
                let row = s.top_line() + if up { 0 } else { s.grid.rows - 1 };
                s.drag_selection((col, row));
            }
            // A pane along the top of a window that draws under its bar fills the strip above it.
            let above = if !self.own_bar && area.y <= self.bar_h + 0.5 { self.bar_h } else { 0.0 };
            let look = PaneLook { palette: p, metrics: mt, size, keyboard: active && self.focused, thumb_held: active && matches!(self.held, Some(Held::Thumb(_))), above, cursor: self.cursor_shape, cursor_on: !self.blink || self.blink_on };
            draw_pane(&mut scene, fonts, &mut numbers, s, *area, &look);
        }
        // The lines between panes; the one held, in the accent.
        for d in &dividers {
            let held = matches!(&self.held, Some(Held::Divider(path)) if *path == d.path);
            scene.fill(d.line, if held { p.accent } else { Color::rgba(p.fg.r, p.fg.g, p.fg.b, 56) });
        }
        if let Some(menu) = &self.menu {
            menu.draw(&mut scene, fonts, mt.ui, self.settings.ui_size, p.accent, self.menu_hover);
        }
        self.last_commands = scene.commands.len();
        scene
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
    if let Some(n) = args.iter().position(|a| a == "--tabs").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<usize>().ok()) {
        terminal.open_with = n.clamp(1, 64);
    }
    if let Some(n) = args.iter().position(|a| a == "--panes").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<usize>().ok()) {
        terminal.open_panes = n.clamp(1, 16);
    }
    terminal.open_maximized = args.iter().any(|a| a == "--maximized");
    // One process for all the terminal's windows (program.rs), on either road
    // since bar milestone 1, step 4b: a later start asks the running one for a
    // window and is done. A terminal started to be measured keeps a process of
    // its own.
    let measured = args.iter().any(|a| a == "--tabs" || a == "--panes") || std::env::var_os("VTERM_FRAMES").is_some();
    let native = std::env::var_os("VERGENCE_BUS").is_some() && std::env::var("VERGENCE_UI_WAYLAND").as_deref() != Ok("1");
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
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers { shift: false, ctrl: false, alt: false, logo: false };

    #[test]
    fn colors_follow_the_desktop() {
        let same = |a: &Palette, b: &Palette| (a.bg, a.fg, a.accent, a.ansi) == (b.bg, b.fg, b.accent, b.ansi);
        assert!(same(&palette_from(""), &CATPPUCCIN));
        assert!(same(&palette_from("[terminal]\nscheme = \"Tango\"\n"), &TANGO));
        assert!(same(&palette_from("[appearance]\ncolor-scheme = \"light\"\n"), &LATTE));
        assert!(same(&palette_from("[appearance]\ncolor-scheme = \"default\"\n"), &CATPPUCCIN));
        assert!(same(&palette_from("[appearance]\ncolor-scheme = \"light\"\n[terminal]\nscheme = \"tango\"\n"), &TANGO_LIGHT));
        // A key of the same name in another section is not ours.
        assert!(same(&palette_from("[wallpaper]\nscheme = \"tango\"\ncontrast = \"high\"\n"), &CATPPUCCIN));
        // High contrast: the text on black or white; the 16 colors stay.
        let p = palette_from("[appearance]\ncontrast = \"high\"\naccent = \"#ff8800\"\n");
        assert_eq!((p.bg, p.fg, p.accent), (Color::rgb(0, 0, 0), Color::rgb(255, 255, 255), Color::rgb(0xff, 0x88, 0)));
        assert_eq!(p.ansi, CATPPUCCIN.ansi);
        let p = palette_from("[appearance]\ncolor-scheme = \"light\"\ncontrast = \"high\"\n");
        assert_eq!((p.bg, p.fg), (Color::rgb(255, 255, 255), Color::rgb(0, 0, 0)));
        // A broken accent is no accent.
        for bad in ["blue", "#12345", "#+12345", "#gggggg", ""] {
            assert_eq!(palette_from(&format!("[appearance]\naccent = \"{bad}\"\n")).accent, CATPPUCCIN.accent, "{bad}");
        }
    }

    #[test]
    fn the_scrollback_keys_reach_its_ends() {
        let mut s = Screen { grid: Grid::new(10, 4), parser: vt::Parser::new(), scroll: 0, exited: false, copied: None, selection: None };
        for i in 0..10 {
            let line = format!("line {i}\r\n");
            s.parser.advance(&mut s.grid, line.as_bytes());
        }
        assert!(s.grid.scrollback.len() > 3);
        s.scroll_by(2);
        s.scroll_by(isize::MAX);
        assert_eq!(s.scroll, s.grid.scrollback.len());
        s.scroll_by(isize::MIN);
        assert_eq!(s.scroll, 0);
    }

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
    fn colors_come_from_the_palette_or_the_cube() {
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
        // The dark palettes keep their colors as they are.
        assert_eq!(TANGO.floor, 0.0);
        assert_eq!(readable(rgb(0x3465a4), TANGO.bg, TANGO.floor), rgb(0x3465a4));
    }

    #[test]
    fn the_settings_file_is_followed() {
        let s = Settings::from_text("[settings]\n\"Gtk/DecorationLayout\" = \"close,minimize:\"\n\"Gtk/FontName\" = \"Adwaita Sans 10\"\n\"Gtk/MonospaceFontName\" = \"Adwaita Mono 11\"\n");
        assert_eq!((s.left.as_slice(), s.right.len()), (&[WinButton::Close, WinButton::Minimize][..], 0));
        assert_eq!((s.ui_family.as_str(), s.mono_family.as_str()), ("Adwaita Sans", "Adwaita Mono"));
        assert!((s.ui_size - 13.333).abs() < 0.01 && (s.mono_size - 14.667).abs() < 0.01);
        // Nothing set: buttons on the right, the machine's own fonts at 10 points.
        let s = Settings::from_text("");
        // No layout chosen: vergence-bar's default, on the left (2026-11-12).
        assert_eq!((s.left.as_slice(), s.right.len()), (&[WinButton::Close, WinButton::Minimize, WinButton::Maximize][..], 0));
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

    /// On the light palettes every one of the 16 colors, plain or bold, reads
    /// at the floor on the page (Hubble, 2026-11-13: Tango's whites and brights
    /// vanished on white); a color that already reads is left alone; the hue
    /// stays (red stays reddest).
    #[test]
    fn the_remembered_answer_is_the_worked_out_one() {
        for (ink, ground) in [(rgb(0xef2929), rgb(0xffffff)), (rgb(0x2e3436), rgb(0xffffff)), (rgb(0xeeeeec), rgb(0x000000))] {
            let worked = readable(ink, ground, LIGHT_FLOOR);
            assert_eq!(readable_cached(ink, ground, LIGHT_FLOOR, LIGHT_FLOOR.to_bits()), worked);
            assert_eq!(readable_cached(ink, ground, LIGHT_FLOOR, LIGHT_FLOOR.to_bits()), worked, "and again from memory");
        }
    }

    #[test]
    fn light_palettes_keep_text_readable() {
        let rgb3 = |c: Color| vergence_look::Rgb(c.r, c.g, c.b);
        for p in [&TANGO_LIGHT, &LATTE] {
            for i in 0..16u8 {
                for bold in [false, true] {
                    let ink = readable(p.resolve(CellColor::Indexed(i), p.fg, bold), p.bg, p.floor);
                    let c = vergence_look::contrast(rgb3(ink), rgb3(p.bg));
                    assert!(c >= LIGHT_FLOOR - 0.01, "color {i} bold {bold}: {c:.2}");
                }
            }
        }
        assert_eq!(readable(rgb(0x2e3436), rgb(0xffffff), LIGHT_FLOOR), rgb(0x2e3436), "the text color already reads");
        let red = readable(rgb(0xef2929), rgb(0xffffff), LIGHT_FLOOR);
        assert!(red.r > red.g && red.r > red.b, "still red: {red:?}");
        // A cell with a ground of its own: kept against that ground, not the page.
        let on_black = readable(rgb(0xeeeeec), rgb(0x000000), LIGHT_FLOOR);
        assert_eq!(on_black, rgb(0xeeeeec));
    }

}
