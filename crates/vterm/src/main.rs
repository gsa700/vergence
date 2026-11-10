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
//! This first part is one terminal in one window under the server's title
//! bar: typing, paste, the scrollback (wheel, Shift+PageUp/PageDown),
//! resizing, the window's title, the program's own copies (OSC 52). Tabs,
//! panes, selecting with the mouse, mouse reporting, the scrollbar and the
//! settings followed live come next.

#[allow(dead_code)]
#[path = "../../vergence-term/src/grid.rs"]
mod grid;
#[allow(dead_code)]
#[path = "../../vergence-term/src/pty.rs"]
mod pty;
#[allow(dead_code)]
#[path = "../../vergence-term/src/vt.rs"]
mod vt;

use std::{
    collections::HashMap,
    io::Write,
    sync::{Arc, Mutex, MutexGuard},
};

use grid::{Cell, Color as CellColor, Grid};
use vergence_scene::{Color, Glyph, Point, Rect, Scene};
use vergence_text::{FaceId, Fonts};
use vergence_ui::{App, Frame, Input, Modifiers, Waker, Window, sym};

/// Space around the cells, in reference units, and the text size: 10 points.
const PAD: f32 = 4.0;
const SIZE: f32 = 13.33;
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
    let base = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")));
    let text = base.and_then(|b| std::fs::read_to_string(b.join("vergence/desktop.toml")).ok()).unwrap_or_default();
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
        self.grid.abs_line(self.grid.pushed - self.scroll + r).unwrap_or(&[])
    }
}

/// Text metrics, known once the fonts have been asked.
struct Metrics {
    regular: FaceId,
    bold: FaceId,
    cell_w: f32,
    cell_h: f32,
    baseline: f32,
}

struct Terminal {
    screen: Arc<Mutex<Screen>>,
    pty: pty::Pty,
    /// The shell's output, until the reading thread takes it.
    reader: Option<std::fs::File>,
    palette: Palette,
    metrics: Option<Metrics>,
    focused: bool,
    title: String,
}

fn lock(screen: &Mutex<Screen>) -> MutexGuard<'_, Screen> {
    // A panic on the other thread must not take this one with it.
    screen.lock().unwrap_or_else(|e| e.into_inner())
}

impl Terminal {
    fn new() -> std::io::Result<Terminal> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let pty = pty::Pty::spawn(&shell, 80, 24)?;
        let reader = pty.reader()?;
        let screen = Screen { grid: Grid::new(80, 24), parser: vt::Parser::new(), scroll: 0, exited: false, copied: None };
        Ok(Terminal { screen: Arc::new(Mutex::new(screen)), pty, reader: Some(reader), palette: palette(), metrics: None, focused: false, title: String::new() })
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

    fn background(&self) -> Color {
        self.palette.bg
    }

    fn waker(&mut self, waker: Waker) {
        // The reading thread: the program's output onto the screen, its replies back, the window woken.
        let Some(reader) = self.reader.take() else { return };
        let Ok(mut replies) = reader.try_clone() else { return };
        let screen = self.screen.clone();
        std::thread::spawn(move || {
            pty::read_loop(reader, |bytes| {
                let out = {
                    let mut guard = lock(&screen);
                    let s = &mut *guard;
                    let before = s.grid.pushed;
                    s.parser.advance(&mut s.grid, &bytes);
                    if s.grid.pushed < before {
                        // A full reset: the line numbers started again.
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
            lock(&screen).exited = true;
            waker.wake();
        });
    }

    fn woken(&mut self, window: &mut Window) {
        let mut s = lock(&self.screen);
        if s.exited {
            window.close();
            return;
        }
        if let Some(text) = s.copied.take() {
            window.copy(&text);
        }
        if s.grid.title != self.title {
            self.title = s.grid.title.clone();
            window.set_title(if self.title.is_empty() { "Terminal" } else { &self.title });
        }
        if std::mem::take(&mut s.grid.dirty) {
            window.redraw();
        }
    }

    fn input(&mut self, input: &Input, window: &mut Window) {
        match input {
            Input::Key { sym: key, text, pressed: true, composing: false, modifiers, .. } => {
                let m = *modifiers;
                let letter = |c: char| *key == c as u32 || *key == c.to_ascii_uppercase() as u32;
                let mut s = lock(&self.screen);
                if m.ctrl && m.shift && letter('v') {
                    window.paste();
                } else if m.shift && (*key == sym::PAGE_UP || *key == sym::PAGE_DOWN) {
                    let page = s.grid.rows.saturating_sub(1).max(1) as isize;
                    s.scroll_by(if *key == sym::PAGE_UP { page } else { -page });
                } else if let Some(bytes) = Terminal::key_bytes(*key, text.as_deref(), m, s.grid.app_cursor) {
                    // Typing goes to the live screen.
                    s.scroll = 0;
                    drop(s);
                    self.pty.write(&bytes);
                }
                window.redraw();
            }
            Input::Paste(text) => {
                let bracketed = lock(&self.screen).grid.bracketed_paste;
                // A pasted line end is the Return key; and the paste cannot end its own brackets.
                let text = text.replace("\r\n", "\r").replace('\n', "\r").replace("\x1b[201~", "");
                let bytes = if bracketed { format!("\x1b[200~{text}\x1b[201~") } else { text };
                self.pty.write(bytes.as_bytes());
            }
            Input::Scroll { dy, .. } => {
                let notches = (dy / 15.0).round() as isize;
                let mut s = lock(&self.screen);
                if s.grid.alt {
                    // A full-screen program has no scrollback: the wheel is its arrow keys.
                    let key = if s.grid.app_cursor { "\x1bO" } else { "\x1b[" }.to_string() + if notches < 0 { "A" } else { "B" };
                    drop(s);
                    self.pty.write(key.repeat((notches.unsigned_abs() * WHEEL_LINES as usize).min(60)).as_bytes());
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
        let mut scene = Scene::new(frame.width, frame.height);
        scene.fill(Rect::new(0.0, 0.0, frame.width, frame.height), p.bg);
        if self.metrics.is_none() {
            self.metrics = fonts.face("monospace", 400, false).map(|regular| {
                let bold = fonts.face("monospace", 700, false).unwrap_or(regular);
                let m = fonts.metrics(regular, SIZE);
                let (face, glyph) = fonts.glyph(regular, 'M');
                Metrics { regular, bold, cell_w: fonts.advance(face, glyph, SIZE), cell_h: (m.ascent + m.descent + m.line_gap).ceil(), baseline: m.ascent.round() }
            });
        }
        let Some(mt) = &self.metrics else { return scene };
        let (cw, ch) = (mt.cell_w, mt.cell_h);

        let mut guard = lock(&self.screen);
        let s = &mut *guard;
        // The grid follows the window's size, and the program is told.
        let cols = (((frame.width - 2.0 * PAD) / cw).floor().max(2.0) as usize).min(2000);
        let rows = (((frame.height - 2.0 * PAD) / ch).floor().max(1.0) as usize).min(1000);
        if (cols, rows) != (s.grid.cols, s.grid.rows) {
            s.grid.resize(cols, rows);
            s.clamp_scroll();
            self.pty.resize(cols as u16, rows as u16);
        }

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
            let top = PAD + r as f32 * ch;
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
            for (col, cell) in line.iter().enumerate() {
                if cell.width == 0 || cell.c == ' ' {
                    continue;
                }
                let st = cell.style;
                let mut fg = if st.inverse { p.resolve(st.bg, p.bg, false) } else { p.resolve(st.fg, p.fg, st.bold) };
                if st.dim {
                    fg.a = 153;
                }
                let (face, glyph) = fonts.glyph(if st.bold { mt.bold } else { mt.regular }, cell.c);
                let font = number(&mut scene, fonts, face);
                let placed = Glyph { id: glyph, x: col as f32 * cw, y: 0.0 };
                match &mut run {
                    Some((f, c, glyphs)) if *f == font && *c == fg => glyphs.push(placed),
                    _ => {
                        if let Some((f, c, glyphs)) = run.take() {
                            scene.glyphs(f, SIZE, c, origin, glyphs);
                        }
                        run = Some((font, fg, vec![placed]));
                    }
                }
            }
            if let Some((f, c, glyphs)) = run {
                scene.glyphs(f, SIZE, c, origin, glyphs);
            }
        }
        // The cursor: a block with the keyboard, an outline without. It moves down with the
        // screen while scrolled back, out of sight past the bottom.
        let cursor_row = s.grid.cy + s.scroll;
        if s.grid.cursor_visible && cursor_row < s.grid.rows {
            let at = Rect::new(PAD + s.grid.cx as f32 * cw, PAD + cursor_row as f32 * ch, cw, ch);
            if self.focused {
                scene.fill(at, Color::rgba(p.accent.r, p.accent.g, p.accent.b, 178));
            } else {
                scene.stroke(at, 0.0, 1.0, p.accent);
            }
        }
        // Scrolled back: how far, in the top right corner.
        if s.scroll > 0 {
            let label = if s.scroll == 1 { "1 line back".to_string() } else { format!("{} lines back", s.scroll) };
            let width = label.chars().count() as f32 * cw + 16.0;
            let at = Rect::new(frame.width - width - PAD, PAD, width, ch + 4.0);
            scene.fill_rounded(at, 6.0, Color::rgba(p.accent.r, p.accent.g, p.accent.b, 235));
            let font = number(&mut scene, fonts, mt.regular);
            let glyphs = label.chars().enumerate().map(|(i, c)| Glyph { id: fonts.glyph(mt.regular, c).1, x: i as f32 * cw, y: 0.0 }).collect();
            scene.glyphs(font, SIZE, p.bg, Point::new(at.x + 8.0, at.y + 2.0 + mt.baseline), glyphs);
        }
        scene
    }
}

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("vterm {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let mut terminal = match Terminal::new() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("vterm: could not start the shell: {e}");
            std::process::exit(1);
        }
    };
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
    fn the_view_scrolls_within_the_scrollback() {
        let mut s = Screen { grid: Grid::new(20, 4), parser: vt::Parser::new(), scroll: 0, exited: false, copied: None };
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
