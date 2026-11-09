// One terminal: its screen, parser, shell, selection and drawing (a pane).

use iced::keyboard::{self, Key, key::Named};
use iced::widget::canvas::{self, Frame, Geometry};
use iced::{Color, Font, Pixels, Point, Rectangle, Renderer, Size};

use crate::grid::{Cell, Color as TColor, Grid, MouseMode};
use crate::look::{Look, Palette};
use crate::{pty, vt};

/// Space around the grid, in points.
pub const PAD: f32 = 6.0;
/// The scrollbar's width (in the right padding), and the width that grabs it.
pub const SCROLLBAR_W: f32 = 5.0;
pub const SCROLLBAR_GRAB: f32 = 12.0;

/// A cell position: (column, row) on the screen.
pub type CellPos = (usize, usize);

pub struct Term {
    pub id: u64,
    /// The text selected with the mouse: anchor and end, in either order;
    /// rows are absolute line numbers (Grid::pushed), so a selection stays on
    /// its text while the view scrolls or output comes.
    pub selection: Option<(CellPos, CellPos)>,
    /// How many lines the view is scrolled back into the scrollback (0: the live screen).
    pub scroll: usize,
    pub grid: Grid,
    parser: vt::Parser,
    pub pty: pty::Pty,
    pub cache: canvas::Cache,
    pub font_size: f32,
    pub font: Font,
    pub cell: Size,
    pub palette: Palette,
}

/// A cell's size for a look: the monospace font's own advance (measured, so
/// text runs, backgrounds and the cursor line up in any font), lines 1.25 em.
pub fn cell_for(look: &Look) -> Size {
    use iced::advanced::text::Paragraph as _;
    let size = look.font_size;
    const SAMPLE: &str = "MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM";
    let p = iced::advanced::graphics::text::Paragraph::with_text(iced::advanced::Text {
        content: SAMPLE,
        bounds: Size::INFINITE,
        size: Pixels(size),
        line_height: iced::widget::text::LineHeight::default(),
        font: look.mono_font(),
        align_x: iced::advanced::text::Alignment::Default,
        align_y: iced::alignment::Vertical::Top,
        shaping: iced::advanced::text::Shaping::Basic,
        wrapping: iced::advanced::text::Wrapping::None,
    });
    let measured = p.min_bounds().width / SAMPLE.len() as f32;
    // A font that can't be measured (or isn't monospace-like): DejaVu's 0.602 em.
    let advance = if (size * 0.3..=size * 1.0).contains(&measured) { measured } else { size * 0.6021 };
    Size::new(advance.max(1.0), (size * 1.25).ceil())
}

impl Term {
    /// A new terminal running the user's shell, cols x rows.
    pub fn new(id: u64, cols: usize, rows: usize, look: &Look) -> std::io::Result<(Term, std::fs::File)> {
        let size = look.font_size;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let pty = pty::Pty::spawn(&shell, cols as u16, rows as u16)?;
        let reader = pty.reader()?;
        Ok((
            Term { id, selection: None, scroll: 0, grid: Grid::new(cols, rows), parser: vt::Parser::new(), pty, cache: canvas::Cache::new(), font_size: size, font: look.mono_font(), cell: cell_for(look), palette: look.palette() },
            reader,
        ))
    }

    /// The desktop's look changed: colours and font size (the caller refits the pane).
    pub fn set_look(&mut self, look: &Look) {
        self.palette = look.palette();
        self.font_size = look.font_size;
        self.font = look.mono_font();
        self.cell = cell_for(look);
        self.cache.clear();
    }

    /// The shell's output: onto the screen; replies back to the shell.
    /// Returns text a program copied (OSC 52), for the clipboard.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<String> {
        let before = self.grid.pushed;
        self.parser.advance(&mut self.grid, bytes);
        if self.grid.pushed < before {
            // A full reset (ESC c): the line numbers started again.
            self.selection = None;
            self.scroll = 0;
        } else if self.scroll > 0 {
            // Scrolled back: stay on the same text as new lines arrive.
            self.scroll += self.grid.pushed - before;
        }
        self.clamp_scroll();
        if !self.grid.out.is_empty() {
            let out = std::mem::take(&mut self.grid.out);
            self.pty.write(&out);
        }
        if self.grid.dirty {
            self.grid.dirty = false;
            self.cache.clear();
        }
        self.grid.clipboard.take()
    }

    /// Fit the grid to an area of `size` points.
    pub fn fit(&mut self, size: Size) {
        let cols = ((size.width - 2.0 * PAD) / self.cell.width).floor().max(2.0) as usize;
        let rows = ((size.height - 2.0 * PAD) / self.cell.height).floor().max(1.0) as usize;
        if (cols, rows) != (self.grid.cols, self.grid.rows) {
            self.grid.resize(cols, rows);
            self.clamp_scroll();
            self.pty.resize(cols as u16, rows as u16);
            self.cache.clear();
        }
    }

    /// The cell under a point in the pane (points, from its top-left): the
    /// screen's column and row (what mouse reporting tells the program).
    pub fn cell_at(&self, x: f32, y: f32) -> CellPos {
        let col = ((x - PAD) / self.cell.width).floor().max(0.0) as usize;
        let row = ((y - PAD) / self.cell.height).floor().max(0.0) as usize;
        (col.min(self.grid.cols - 1), row.min(self.grid.rows - 1))
    }

    /// The cell under a point as the selection counts it: the row is the
    /// absolute line number of what is shown there.
    pub fn text_cell_at(&self, x: f32, y: f32) -> CellPos {
        let (col, row) = self.cell_at(x, y);
        (col, self.top_line() + row)
    }

    /// The absolute number of the line shown at the top of the pane.
    fn top_line(&self) -> usize {
        self.grid.pushed - self.scroll
    }

    /// The line shown at row `r` of the pane (scrollback while scrolled back).
    fn shown_line(&self, r: usize) -> &[Cell] {
        self.grid.abs_line(self.top_line() + r).unwrap_or(&[])
    }

    /// The alternate screen has no scrollback to show; else up to all of it.
    fn clamp_scroll(&mut self) {
        let max = if self.grid.alt { 0 } else { self.grid.scrollback.len() };
        if self.scroll > max {
            self.scroll = max;
            self.cache.clear();
        }
    }

    /// Scroll the view: positive back into the scrollback, negative toward the live screen.
    pub fn scroll_by(&mut self, lines: isize) {
        let was = self.scroll;
        self.scroll = self.scroll.saturating_add_signed(lines);
        self.clamp_scroll();
        if self.scroll != was {
            self.cache.clear();
        }
    }

    /// Jump the view to `lines` back (the scrollbar).
    pub fn scroll_to(&mut self, lines: usize) {
        let was = self.scroll;
        self.scroll = lines;
        self.clamp_scroll();
        if self.scroll != was {
            self.cache.clear();
        }
    }

    /// While a selection is dragged past the pane's top (or bottom) edge and
    /// the view scrolls, its end follows to the edge row at column `col`.
    pub fn drag_to_edge(&mut self, col: usize, top: bool) {
        if let Some((anchor, _)) = self.selection {
            let row = self.top_line() + if top { 0 } else { self.grid.rows - 1 };
            self.select(Some((anchor, (col.min(self.grid.cols - 1), row))));
        }
    }

    /// The scrollbar in a pane of `size`: its track and thumb, or None when
    /// there is nothing to scroll (no scrollback, or the alternate screen).
    pub fn scrollbar(&self, size: Size) -> Option<(Rectangle, Rectangle)> {
        let back = self.grid.scrollback.len();
        if self.grid.alt || back == 0 {
            return None;
        }
        let track = Rectangle::new(Point::new(size.width - SCROLLBAR_W - 2.0, PAD), Size::new(SCROLLBAR_W, (size.height - 2.0 * PAD).max(1.0)));
        let total = (back + self.grid.rows) as f32;
        let h = (track.height * self.grid.rows as f32 / total).max(24.0_f32.min(track.height));
        // The view's top line from the oldest: 0 at the top, `back` live.
        let y = track.y + (back - self.scroll) as f32 / back as f32 * (track.height - h);
        Some((track, Rectangle::new(Point::new(track.x, y), Size::new(SCROLLBAR_W, h))))
    }

    /// How far back the view goes for the thumb's top at `y` (the scrollbar dragged).
    pub fn scroll_for_thumb(&self, size: Size, y: f32) -> Option<usize> {
        let (track, thumb) = self.scrollbar(size)?;
        let room = (track.height - thumb.height).max(1.0);
        let frac = ((y - track.y) / room).clamp(0.0, 1.0);
        let back = self.grid.scrollback.len();
        Some(back - (frac * back as f32).round() as usize)
    }

    /// Back to the live screen (typing, pasting).
    pub fn scroll_to_bottom(&mut self) {
        if self.scroll != 0 {
            self.scroll = 0;
            self.cache.clear();
        }
    }

    /// Scrolled-back views can be paged: a screenful less a line.
    pub fn page(&self) -> isize {
        self.grid.rows.saturating_sub(1).max(1) as isize
    }

    pub fn select(&mut self, sel: Option<(CellPos, CellPos)>) {
        if sel != self.selection {
            self.selection = sel;
            self.cache.clear();
        }
    }

    /// The selection in reading order (start, end inclusive), if it isn't empty.
    fn ordered(&self) -> Option<(CellPos, CellPos)> {
        let (a, b) = self.selection?;
        let (s, e) = if (a.1, a.0) <= (b.1, b.0) { (a, b) } else { (b, a) };
        (s != e).then_some((s, e))
    }

    fn selected(&self, col: usize, row: usize) -> bool {
        self.ordered().is_some_and(|((sc, sr), (ec, er))| (row > sr || (row == sr && col >= sc)) && (row < er || (row == er && col <= ec)))
    }

    /// The selected text: lines joined with newlines, trailing spaces dropped.
    pub fn selected_text(&self) -> Option<String> {
        let ((sc, sr), (ec, er)) = self.ordered()?;
        let mut out = Vec::new();
        for r in sr..=er {
            let line = self.grid.abs_line(r).unwrap_or(&[]);
            let (a, b) = (if r == sr { sc } else { 0 }, if r == er { ec } else { self.grid.cols - 1 });
            if line.is_empty() || a >= line.len() {
                out.push(String::new());
                continue;
            }
            let text: String = line[a..=b.min(line.len() - 1)].iter().filter(|c| c.width != 0).map(|c| c.c).collect();
            out.push(text.trim_end().to_string());
        }
        Some(out.join("\n"))
    }

    /// Text pasted in: bracketed if the program asked for it.
    pub fn paste(&mut self, text: &str) {
        self.scroll_to_bottom();
        let clean = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.grid.bracketed_paste {
            self.pty.write(b"\x1b[200~");
            self.pty.write(clean.as_bytes());
            self.pty.write(b"\x1b[201~");
        } else {
            self.pty.write(clean.as_bytes());
        }
    }

    pub fn title(&self) -> String {
        if self.grid.title.is_empty() { "Terminal".into() } else { self.grid.title.clone() }
    }
}

/// What a key sends to the program (xterm's sequences).
pub fn key_bytes(key: &Key, mods: keyboard::Modifiers, text: Option<&str>, app_cursor: bool) -> Option<Vec<u8>> {
    let csi = |s: &str| Some(format!("\x1b[{s}").into_bytes());
    let cursor = |c: char| Some(if app_cursor { format!("\x1bO{c}") } else { format!("\x1b[{c}") }.into_bytes());
    if let Key::Named(n) = key {
        return match n {
            Named::Enter => Some(b"\r".to_vec()),
            Named::Backspace => Some(if mods.control() { vec![0x08] } else { vec![0x7f] }),
            Named::Tab => Some(if mods.shift() { b"\x1b[Z".to_vec() } else { b"\t".to_vec() }),
            Named::Escape => Some(vec![0x1b]),
            Named::ArrowUp => cursor('A'),
            Named::ArrowDown => cursor('B'),
            Named::ArrowRight => cursor('C'),
            Named::ArrowLeft => cursor('D'),
            Named::Home => cursor('H'),
            Named::End => cursor('F'),
            Named::PageUp => csi("5~"),
            Named::PageDown => csi("6~"),
            Named::Insert => csi("2~"),
            Named::Delete => csi("3~"),
            Named::F1 => Some(b"\x1bOP".to_vec()),
            Named::F2 => Some(b"\x1bOQ".to_vec()),
            Named::F3 => Some(b"\x1bOR".to_vec()),
            Named::F4 => Some(b"\x1bOS".to_vec()),
            Named::F5 => csi("15~"),
            Named::F6 => csi("17~"),
            Named::F7 => csi("18~"),
            Named::F8 => csi("19~"),
            Named::F9 => csi("20~"),
            Named::F10 => csi("21~"),
            Named::F11 => csi("23~"),
            Named::F12 => csi("24~"),
            Named::Space => Some(if mods.control() { vec![0] } else { b" ".to_vec() }),
            _ => None,
        };
    }
    // Ctrl+letter: the control character.
    if mods.control()
        && let Key::Character(c) = key
        && let Some(ch) = c.chars().next()
    {
        let ch = ch.to_ascii_lowercase();
        let b = match ch {
            'a'..='z' => Some(ch as u8 - b'a' + 1),
            '[' => Some(0x1b),
            '\\' => Some(0x1c),
            ']' => Some(0x1d),
            '6' => Some(0x1e),
            '-' | '/' => Some(0x1f),
            _ => None,
        };
        return b.map(|b| if mods.alt() { vec![0x1b, b] } else { vec![b] });
    }
    let t = text.filter(|t| !t.is_empty() && !t.chars().any(|c| c.is_control()))?;
    let mut out = Vec::new();
    if mods.alt() {
        out.push(0x1b);
    }
    out.extend_from_slice(t.as_bytes());
    Some(out)
}

/// A mouse button, as the program sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// A mouse event for the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press(MouseButton),
    Release(MouseButton),
    /// The pointer moved to another cell, with a button held or not.
    Motion(Option<MouseButton>),
    WheelUp,
    WheelDown,
}

/// What a mouse event sends to the program (xterm's mouse reporting), or
/// None if its mode doesn't ask for the event. `cell` is (column, row) from 0.
/// SGR (mode 1006): ESC [ < code ; x ; y M (press, motion) or m (release).
/// Otherwise the old encoding, ESC [ M and three bytes of 32 + value, which
/// cannot say a column or row past 223 (then nothing is sent).
pub fn mouse_bytes(mode: MouseMode, sgr: bool, action: MouseAction, cell: CellPos, mods: keyboard::Modifiers) -> Option<Vec<u8>> {
    let base = |b: MouseButton| match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let (mut code, release) = match (mode, action) {
        (MouseMode::Off, _) => return None,
        (MouseMode::Press, MouseAction::Release(_) | MouseAction::Motion(_)) => return None,
        (MouseMode::Click, MouseAction::Motion(_)) | (MouseMode::Drag, MouseAction::Motion(None)) => return None,
        (_, MouseAction::Press(b)) => (base(b), false),
        // The old encoding can't say which button was released: 3.
        (_, MouseAction::Release(b)) => (if sgr { base(b) } else { 3 }, true),
        (_, MouseAction::Motion(b)) => (32 + b.map_or(3, base), false),
        (_, MouseAction::WheelUp) => (64, false),
        (_, MouseAction::WheelDown) => (65, false),
    };
    // X10 mode reports no modifiers.
    if mode != MouseMode::Press {
        code += if mods.shift() { 4 } else { 0 } + if mods.alt() { 8 } else { 0 } + if mods.control() { 16 } else { 0 };
    }
    let (x, y) = (cell.0 + 1, cell.1 + 1);
    if sgr {
        return Some(format!("\x1b[<{code};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes());
    }
    if 32 + x > 255 || 32 + y > 255 {
        return None;
    }
    Some(vec![0x1b, b'[', b'M', 32 + code as u8, (32 + x) as u8, (32 + y) as u8])
}

/// The wheel on the alternate screen when the program didn't ask for the
/// mouse (less, man): arrow keys, `lines` of them, as most terminals do.
pub fn wheel_keys(up: bool, app_cursor: bool, lines: usize) -> Vec<u8> {
    let c = if up { 'A' } else { 'B' };
    let one = if app_cursor { format!("\x1bO{c}") } else { format!("\x1b[{c}") };
    one.repeat(lines).into_bytes()
}

/// xterm's 256 colours: 16 ANSI, a 6x6x6 cube, 24 greys.
fn indexed(p: &Palette, i: u8) -> Color {
    match i {
        0..=15 => p.ansi[i as usize],
        16..=231 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color::from_rgb8(level(n / 36), level((n / 6) % 6), level(n % 6))
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            Color::from_rgb8(g, g, g)
        }
    }
}

fn resolve(p: &Palette, c: TColor, default: Color, bold: bool) -> Color {
    match c {
        TColor::Default => default,
        // Bold makes the first 8 colours bright, as xterm does.
        TColor::Indexed(i) if bold && i < 8 => indexed(p, i + 8),
        TColor::Indexed(i) => indexed(p, i),
        TColor::Rgb(r, g, b) => Color::from_rgb8(r, g, b),
    }
}

impl Term {
    /// The pane's picture (cached until the screen, selection or focus changes).
    pub fn draw(&self, renderer: &Renderer, bounds: Rectangle, focused: bool) -> Vec<Geometry> {
        let p = self.palette;
        let geo = self.cache.draw(renderer, bounds.size(), |frame: &mut Frame| {
            frame.fill_rectangle(Point::ORIGIN, bounds.size(), p.bg);
            let (cw, ch) = (self.cell.width, self.cell.height);
            for y in 0..self.grid.rows {
                let line = self.shown_line(y);
                let top = PAD + y as f32 * ch;
                // Runs of cells with the same look: one background, one text each.
                let mut x = 0;
                while x < line.len() {
                    let style = line[x].style;
                    let mut end = x + 1;
                    while end < line.len() && line[end].style == style {
                        end += 1;
                    }
                    let (mut fg, mut bg) = (resolve(&p, style.fg, p.fg, style.bold), resolve(&p, style.bg, p.bg, false));
                    if style.inverse {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    if style.dim {
                        fg.a = 0.6;
                    }
                    if bg != p.bg {
                        frame.fill_rectangle(Point::new(PAD + x as f32 * cw, top), Size::new((end - x) as f32 * cw, ch), bg);
                    }
                    let text: String = line[x..end].iter().filter(|c: &&Cell| c.width != 0).map(|c| c.c).collect();
                    if !text.trim().is_empty() {
                        frame.fill_text(canvas::Text {
                            content: text,
                            position: Point::new(PAD + x as f32 * cw, top + (ch - self.font_size) / 2.0),
                            color: fg,
                            size: Pixels(self.font_size),
                            font: if style.bold { Font { weight: iced::font::Weight::Bold, ..self.font } } else { self.font },
                            ..canvas::Text::default()
                        });
                    }
                    if style.underline {
                        frame.fill_rectangle(Point::new(PAD + x as f32 * cw, top + ch - 2.0), Size::new((end - x) as f32 * cw, 1.0), fg);
                    }
                    x = end;
                }
            }
            // The selection, over the text.
            if self.ordered().is_some() {
                let top = self.top_line();
                for r in 0..self.grid.rows {
                    let mut c = 0;
                    while c < self.grid.cols {
                        if self.selected(c, top + r) {
                            let start = c;
                            while c < self.grid.cols && self.selected(c, top + r) {
                                c += 1;
                            }
                            frame.fill_rectangle(Point::new(PAD + start as f32 * cw, PAD + r as f32 * ch), Size::new((c - start) as f32 * cw, ch), Color { a: 0.35, ..p.accent });
                        } else {
                            c += 1;
                        }
                    }
                }
            }
            // The cursor moves down with the screen while scrolled back, out of sight past the bottom.
            let cursor_row = self.grid.cy + self.scroll;
            if self.grid.cursor_visible && cursor_row < self.grid.rows {
                let at = Point::new(PAD + self.grid.cx as f32 * cw, PAD + cursor_row as f32 * ch);
                if focused {
                    frame.fill_rectangle(at, Size::new(cw, ch), Color { a: 0.7, ..p.accent });
                } else {
                    // Unfocused: an outline, so the pane with the keyboard is clear.
                    frame.stroke(&canvas::Path::rectangle(at, Size::new(cw, ch)), canvas::Stroke::default().with_color(p.accent).with_width(1.0));
                }
            }
            // The scrollbar: faint at the live screen, clearer while scrolled back.
            if let Some((track, thumb)) = self.scrollbar(bounds.size()) {
                frame.fill(&canvas::Path::rounded_rectangle(track.position(), track.size(), (SCROLLBAR_W / 2.0).into()), Color { a: 0.06, ..p.fg });
                let a = if self.scroll > 0 { 0.55 } else { 0.25 };
                frame.fill(&canvas::Path::rounded_rectangle(thumb.position(), thumb.size(), (SCROLLBAR_W / 2.0).into()), Color { a, ..p.fg });
            }
            // Scrolled back: how far, in the top right corner (left of the scrollbar).
            if self.scroll > 0 {
                let label = if self.scroll == 1 { "1 line back".to_string() } else { format!("{} lines back", self.scroll) };
                let size = (self.font_size * 0.85).round();
                let w = label.chars().count() as f32 * size * 0.6021 + 16.0;
                let h = size * 1.6;
                let at = Point::new(bounds.width - w - PAD - SCROLLBAR_GRAB, PAD);
                frame.fill(&canvas::Path::rounded_rectangle(at, Size::new(w, h), 6.0.into()), Color { a: 0.92, ..p.accent });
                frame.fill_text(canvas::Text {
                    content: label,
                    position: Point::new(at.x + 8.0, at.y + (h - size) / 2.0),
                    color: p.bg,
                    size: Pixels(size),
                    font: self.font,
                    ..canvas::Text::default()
                });
            }
        });
        vec![geo]
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    const NONE: keyboard::Modifiers = keyboard::Modifiers::empty();

    fn sgr(mode: MouseMode, a: MouseAction, cell: CellPos, m: keyboard::Modifiers) -> Option<String> {
        mouse_bytes(mode, true, a, cell, m).map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn sgr_mouse() {
        use MouseAction::*;
        use MouseButton::*;
        // htop's F10: a left click at column 70, row 24 (from 1).
        assert_eq!(sgr(MouseMode::Click, Press(Left), (69, 23), NONE).as_deref(), Some("\x1b[<0;70;24M"));
        assert_eq!(sgr(MouseMode::Click, Release(Left), (69, 23), NONE).as_deref(), Some("\x1b[<0;70;24m"));
        assert_eq!(sgr(MouseMode::Click, Press(Right), (0, 0), keyboard::Modifiers::CTRL).as_deref(), Some("\x1b[<18;1;1M"));
        assert_eq!(sgr(MouseMode::Click, WheelUp, (4, 2), NONE).as_deref(), Some("\x1b[<64;5;3M"));
        assert_eq!(sgr(MouseMode::Click, WheelDown, (4, 2), NONE).as_deref(), Some("\x1b[<65;5;3M"));
        // Motion: only with a button held in 1002, always in 1003.
        assert_eq!(sgr(MouseMode::Click, Motion(Some(Left)), (1, 1), NONE), None);
        assert_eq!(sgr(MouseMode::Drag, Motion(Some(Left)), (1, 1), NONE).as_deref(), Some("\x1b[<32;2;2M"));
        assert_eq!(sgr(MouseMode::Drag, Motion(None), (1, 1), NONE), None);
        assert_eq!(sgr(MouseMode::Motion, Motion(None), (1, 1), NONE).as_deref(), Some("\x1b[<35;2;2M"));
        // Off: nothing; X10: presses only, no modifiers.
        assert_eq!(sgr(MouseMode::Off, Press(Left), (0, 0), NONE), None);
        assert_eq!(sgr(MouseMode::Press, Release(Left), (0, 0), NONE), None);
        assert_eq!(sgr(MouseMode::Press, Press(Middle), (0, 0), keyboard::Modifiers::ALT).as_deref(), Some("\x1b[<1;1;1M"));
    }

    #[test]
    fn old_mouse_encoding() {
        let b = mouse_bytes(MouseMode::Click, false, MouseAction::Press(MouseButton::Left), (69, 23), NONE).unwrap();
        assert_eq!(b, [0x1b, b'[', b'M', 32, 32 + 70, 32 + 24]);
        // Releases are button 3; past column 223 nothing can be sent.
        let b = mouse_bytes(MouseMode::Click, false, MouseAction::Release(MouseButton::Right), (0, 0), NONE).unwrap();
        assert_eq!(b[3], 35);
        assert_eq!(mouse_bytes(MouseMode::Click, false, MouseAction::Press(MouseButton::Left), (230, 0), NONE), None);
    }

    #[test]
    fn wheel_on_the_alternate_screen() {
        assert_eq!(wheel_keys(true, false, 3), b"\x1b[A\x1b[A\x1b[A");
        assert_eq!(wheel_keys(false, true, 1), b"\x1bOB");
    }
}
