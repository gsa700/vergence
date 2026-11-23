// The terminal's screen: a grid of cells, the cursor, scrollback, the
// alternate screen, scrolling regions, and the subset of xterm a shell, vim,
// less and htop need (TERM=xterm-256color). Driven by the parser (vt.rs).

use std::collections::VecDeque;

use crate::vt::Perform;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

/// Which mouse events the program asked for (xterm's DEC private modes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseMode {
    /// None: the mouse selects text.
    #[default]
    Off,
    /// 9 (X10): presses only, no modifiers.
    Press,
    /// 1000: presses and releases (and the wheel).
    Click,
    /// 1002: also motion while a button is held.
    Drag,
    /// 1003: all motion.
    Motion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub c: char,
    pub style: Style,
    /// 2: the left half of a double-width character; 0: its right half.
    pub width: u8,
    /// Combining marks on the character (the accent of an `e` sent as two
    /// characters), in the order they came; '\0' where there is none. A
    /// third mark on one character is dropped.
    pub marks: [char; 2],
}

const NO_MARKS: [char; 2] = ['\0'; 2];

impl Cell {
    /// The cell's text: its character and the marks on it.
    pub fn push_to(&self, out: &mut String) {
        out.push(self.c);
        out.extend(self.marks.iter().filter(|m| **m != '\0'));
    }
}

/// A mark that is drawn on the character before it.
fn combining(c: char) -> bool {
    let u = c as u32;
    (0x0300..=0x036f).contains(&u) || (0x1ab0..=0x1aff).contains(&u) || (0x20d0..=0x20ff).contains(&u)
}

impl Default for Cell {
    fn default() -> Self {
        Cell { c: ' ', style: Style::default(), width: 1, marks: NO_MARKS }
    }
}

const SCROLLBACK: usize = 10_000;

/// Lines each screen keeps above the rows: the terminal's setting, the same
/// for every tab of the process (app settings milestone 1).
static KEEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(SCROLLBACK);

pub fn set_scrollback(lines: usize) {
    KEEP.store(lines.max(1), std::sync::atomic::Ordering::Relaxed);
}

pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    lines: Vec<Vec<Cell>>,
    /// The other screen (primary while the alternate one is shown).
    other: Vec<Vec<Cell>>,
    pub alt: bool,
    pub scrollback: VecDeque<Vec<Cell>>,
    /// Lines ever sent to the scrollback: a line's absolute number (the
    /// selection's rows, while the view is scrolled back) stays put as output
    /// comes. The scrollback's first line is `pushed - scrollback.len()`;
    /// screen row r is `pushed + r`.
    pub pushed: usize,
    pub cx: usize,
    pub cy: usize,
    /// The cursor is past the last column: the next character wraps first.
    wrap_next: bool,
    saved: (usize, usize, Style),
    /// Character sets (VT100): G0 and G1, each ASCII or DEC Special Graphics
    /// (true), and whether G1 is the one in use (Shift Out). ncurses draws
    /// boxes this way (mc, htop, dialog): ESC ( 0, then letters (without it,
    /// mc draws "lqqqk" for a frame). Saved with the cursor.
    charsets: [bool; 2],
    shift_out: bool,
    saved_charsets: ([bool; 2], bool),
    pen: Style,
    top: usize,
    bottom: usize,
    pub autowrap: bool,
    pub origin: bool,
    pub cursor_visible: bool,
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    /// Mouse reporting (modes 9/1000/1002/1003) and its encoding (1006 = SGR).
    pub mouse: MouseMode,
    pub mouse_sgr: bool,
    pub title: String,
    /// Text a program copied with OSC 52 (Claude Code, tmux, vim...), for
    /// the clipboard. Writes only: programs can never read the clipboard.
    pub clipboard: Option<String>,
    /// Replies to the program (device status, attributes).
    pub out: Vec<u8>,
    /// Something changed since the last look.
    pub dirty: bool,
}

/// How many columns a character takes: 2 for wide East Asian characters and
/// emoji, 0 for combining marks, else 1.
pub fn width(c: char) -> usize {
    let u = c as u32;
    if (0x0300..=0x036f).contains(&u)
        || (0x200b..=0x200f).contains(&u)
        || (0xfe00..=0xfe0f).contains(&u)
        || (0x1ab0..=0x1aff).contains(&u)
        || (0x20d0..=0x20ff).contains(&u)
    {
        return 0;
    }
    let wide = [
        (0x1100, 0x115f),
        (0x2e80, 0x303e),
        (0x3041, 0x33ff),
        (0x3400, 0x4dbf),
        (0x4e00, 0x9fff),
        (0xa000, 0xa4cf),
        (0xac00, 0xd7a3),
        (0xf900, 0xfaff),
        (0xfe30, 0xfe4f),
        (0xff00, 0xff60),
        (0xffe0, 0xffe6),
        (0x1f300, 0x1f64f),
        (0x1f900, 0x1f9ff),
        (0x20000, 0x3fffd),
    ];
    if wide.iter().any(|(a, b)| (*a..=*b).contains(&u)) { 2 } else { 1 }
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Grid {
        let (cols, rows) = (cols.max(2), rows.max(1));
        Grid {
            cols,
            rows,
            lines: vec![vec![Cell::default(); cols]; rows],
            other: vec![vec![Cell::default(); cols]; rows],
            alt: false,
            scrollback: VecDeque::new(),
            pushed: 0,
            cx: 0,
            cy: 0,
            wrap_next: false,
            saved: (0, 0, Style::default()),
            charsets: [false; 2],
            shift_out: false,
            saved_charsets: ([false; 2], false),
            pen: Style::default(),
            top: 0,
            bottom: rows - 1,
            autowrap: true,
            origin: false,
            cursor_visible: true,
            app_cursor: false,
            bracketed_paste: false,
            mouse: MouseMode::Off,
            mouse_sgr: false,
            title: String::new(),
            clipboard: None,
            out: Vec::new(),
            dirty: true,
        }
    }

    /// A screen row (tests; drawing goes through abs_line for the scrollback).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn line(&self, y: usize) -> &[Cell] {
        &self.lines[y]
    }

    /// A line by absolute number (scrollback, then the screen); None if it
    /// has left the scrollback or is below the screen.
    pub fn abs_line(&self, n: usize) -> Option<&[Cell]> {
        let first = self.pushed - self.scrollback.len();
        if n < first {
            None
        } else if n < self.pushed {
            self.scrollback.get(n - first).map(|l| l.as_slice())
        } else {
            self.lines.get(n - self.pushed).map(|l| l.as_slice())
        }
    }

    /// A blank cell in the current background (erasing uses the pen's background).
    /// A combining mark goes on the character printed last: the one before
    /// the cursor, or under it when the line is full. It takes no cell.
    fn mark(&mut self, c: char) {
        let y = self.cy;
        let mut x = if self.wrap_next {
            self.cx
        } else if self.cx > 0 {
            self.cx - 1
        } else {
            return;
        };
        if self.lines[y][x].width == 0 && x > 0 {
            // The right half of a double-width character: the mark is on the character.
            x -= 1;
        }
        let cell = &mut self.lines[y][x];
        if cell.c == ' ' {
            return;
        }
        if let Some(slot) = cell.marks.iter_mut().find(|m| **m == '\0') {
            *slot = c;
            self.dirty = true;
        }
    }

    fn blank(&self) -> Cell {
        Cell { c: ' ', style: Style { bg: self.pen.bg, ..Style::default() }, width: 1, marks: NO_MARKS }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let (cols, rows) = (cols.max(2), rows.max(1));
        if (cols, rows) == (self.cols, self.rows) {
            return;
        }
        for screen in [&mut self.lines, &mut self.other] {
            for l in screen.iter_mut() {
                l.resize(cols, Cell::default());
            }
        }
        // Fewer rows: lines below the cursor go first, then the top lines go to
        // the scrollback (primary screen), so the cursor's line stays visible.
        while self.lines.len() > rows {
            if self.lines.len() - 1 > self.cy {
                self.lines.pop();
            } else {
                let l = self.lines.remove(0);
                if !self.alt {
                    self.push_scrollback(l);
                }
                self.cy = self.cy.saturating_sub(1);
            }
        }
        while self.lines.len() < rows {
            self.lines.push(vec![Cell::default(); cols]);
        }
        self.other.resize(rows, vec![Cell::default(); cols]);
        self.cols = cols;
        self.rows = rows;
        self.top = 0;
        self.bottom = rows - 1;
        self.cx = self.cx.min(cols - 1);
        self.cy = self.cy.min(rows - 1);
        self.wrap_next = false;
        self.dirty = true;
    }

    fn push_scrollback(&mut self, l: Vec<Cell>) {
        self.scrollback.push_back(l);
        self.pushed += 1;
        let keep = KEEP.load(std::sync::atomic::Ordering::Relaxed);
        while self.scrollback.len() > keep {
            self.scrollback.pop_front();
        }
    }

    /// Scroll the region up by n (new blank lines at the bottom).
    fn scroll_up(&mut self, n: usize) {
        for _ in 0..n.min(self.bottom + 1 - self.top) {
            let l = self.lines.remove(self.top);
            if self.top == 0 && !self.alt {
                self.push_scrollback(l);
            }
            let b = self.blank();
            self.lines.insert(self.bottom, vec![b; self.cols]);
        }
        self.dirty = true;
    }

    /// Scroll the region down by n (new blank lines at the top).
    fn scroll_down(&mut self, n: usize) {
        for _ in 0..n.min(self.bottom + 1 - self.top) {
            self.lines.remove(self.bottom);
            let b = self.blank();
            self.lines.insert(self.top, vec![b; self.cols]);
        }
        self.dirty = true;
    }

    fn linefeed(&mut self) {
        if self.cy == self.bottom {
            self.scroll_up(1);
        } else if self.cy + 1 < self.rows {
            self.cy += 1;
        }
    }

    fn goto(&mut self, x: usize, y: usize) {
        let (y0, y1) = if self.origin { (self.top, self.bottom) } else { (0, self.rows - 1) };
        self.cx = x.min(self.cols - 1);
        self.cy = (y0 + y).min(y1);
        self.wrap_next = false;
    }

    fn set_alt(&mut self, on: bool) {
        if on == self.alt {
            return;
        }
        std::mem::swap(&mut self.lines, &mut self.other);
        self.alt = on;
        if on {
            for l in self.lines.iter_mut() {
                l.fill(Cell::default());
            }
        }
        self.dirty = true;
    }

    fn erase_line_part(&mut self, y: usize, from: usize, to: usize) {
        let b = self.blank();
        for x in from..to.min(self.cols) {
            self.lines[y][x] = b;
        }
    }

    fn sgr(&mut self, params: &[u16]) {
        if params.is_empty() {
            self.pen = Style::default();
            return;
        }
        let mut i = 0;
        while i < params.len() {
            let p = params[i];
            match p {
                0 => self.pen = Style::default(),
                1 => self.pen.bold = true,
                2 => self.pen.dim = true,
                3 => self.pen.italic = true,
                4 => self.pen.underline = true,
                7 => self.pen.inverse = true,
                21 | 22 => {
                    self.pen.bold = false;
                    self.pen.dim = false;
                }
                23 => self.pen.italic = false,
                24 => self.pen.underline = false,
                27 => self.pen.inverse = false,
                30..=37 => self.pen.fg = Color::Indexed((p - 30) as u8),
                39 => self.pen.fg = Color::Default,
                40..=47 => self.pen.bg = Color::Indexed((p - 40) as u8),
                49 => self.pen.bg = Color::Default,
                90..=97 => self.pen.fg = Color::Indexed((p - 90 + 8) as u8),
                100..=107 => self.pen.bg = Color::Indexed((p - 100 + 8) as u8),
                38 | 48 => {
                    let c = match params.get(i + 1) {
                        Some(5) => {
                            let c = params.get(i + 2).map(|n| Color::Indexed(*n as u8));
                            i += 2;
                            c
                        }
                        Some(2) => {
                            let c = (params.get(i + 2), params.get(i + 3), params.get(i + 4));
                            i += 4;
                            match c {
                                (Some(r), Some(g), Some(b)) => Some(Color::Rgb(*r as u8, *g as u8, *b as u8)),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some(c) = c {
                        if p == 38 {
                            self.pen.fg = c;
                        } else {
                            self.pen.bg = c;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn mode(&mut self, private: bool, params: &[u16], on: bool) {
        for &p in params {
            match (private, p) {
                (true, 1) => self.app_cursor = on,
                (true, 6) => {
                    self.origin = on;
                    self.goto(0, 0);
                }
                (true, 7) => self.autowrap = on,
                (true, 25) => self.cursor_visible = on,
                (true, 47) | (true, 1047) => self.set_alt(on),
                (true, 1049) => {
                    if on {
                        self.save_cursor();
                        self.set_alt(true);
                    } else {
                        self.set_alt(false);
                        self.restore_cursor();
                    }
                }
                (true, 2004) => self.bracketed_paste = on,
                (true, 9 | 1000 | 1002 | 1003) => {
                    let m = match p {
                        9 => MouseMode::Press,
                        1000 => MouseMode::Click,
                        1002 => MouseMode::Drag,
                        _ => MouseMode::Motion,
                    };
                    // Turning a mode off ends reporting only if it is the one on (xterm).
                    if on {
                        self.mouse = m;
                    } else if self.mouse == m {
                        self.mouse = MouseMode::Off;
                    }
                }
                (true, 1006) => self.mouse_sgr = on,
                _ => {}
            }
        }
        self.dirty = true;
    }
}

impl Perform for Grid {
    fn print(&mut self, c: char) {
        let c = if self.charsets[self.shift_out as usize] { special_graphics(c) } else { c };
        let w = width(c);
        if w == 0 {
            if combining(c) {
                self.mark(c);
            }
            return;
        }
        if self.wrap_next {
            if self.autowrap {
                self.cx = 0;
                self.linefeed();
            }
            self.wrap_next = false;
        }
        if w == 2 && self.cx + 1 >= self.cols {
            // A wide character doesn't fit at the end: it goes on the next line.
            if self.autowrap {
                let b = self.blank();
                self.lines[self.cy][self.cx] = b;
                self.cx = 0;
                self.linefeed();
            } else {
                return;
            }
        }
        let (x, y) = (self.cx, self.cy);
        self.lines[y][x] = Cell { c, style: self.pen, width: w as u8, marks: NO_MARKS };
        if w == 2 {
            self.lines[y][x + 1] = Cell { c: ' ', style: self.pen, width: 0, marks: NO_MARKS };
        }
        if x + w >= self.cols {
            self.cx = self.cols - 1;
            self.wrap_next = true;
        } else {
            self.cx = x + w;
        }
        self.dirty = true;
    }

    fn execute(&mut self, b: u8) {
        match b {
            0x08 => {
                self.cx = self.cx.saturating_sub(1);
                self.wrap_next = false;
            }
            0x09 => {
                self.cx = ((self.cx / 8 + 1) * 8).min(self.cols - 1);
                self.wrap_next = false;
            }
            0x0a..=0x0c => {
                self.linefeed();
                self.wrap_next = false;
            }
            0x0d => {
                self.cx = 0;
                self.wrap_next = false;
            }
            // Shift Out and Shift In: G1, then G0 again.
            0x0e => self.shift_out = true,
            0x0f => self.shift_out = false,
            _ => {}
        }
        self.dirty = true;
    }

    fn csi(&mut self, private: Option<u8>, params: &[u16], intermediates: &[u8], last: char) {
        let p = |i: usize, d: usize| params.get(i).map(|v| *v as usize).filter(|v| *v != 0).unwrap_or(d);
        let raw = |i: usize| params.get(i).map(|v| *v as usize).unwrap_or(0);
        if !intermediates.is_empty() {
            return; // DECSCUSR (cursor shape) and the like: not yet.
        }
        let private_q = private == Some(b'?');
        match last {
            'A' => self.cy = self.cy.saturating_sub(p(0, 1)).max(if self.cy >= self.top { self.top } else { 0 }),
            'B' | 'e' => {
                self.cy = (self.cy + p(0, 1)).min(if self.cy <= self.bottom { self.bottom } else { self.rows - 1 })
            }
            'C' | 'a' => self.cx = (self.cx + p(0, 1)).min(self.cols - 1),
            'D' => self.cx = self.cx.saturating_sub(p(0, 1)),
            'E' => {
                self.cy = (self.cy + p(0, 1)).min(self.bottom);
                self.cx = 0;
            }
            'F' => {
                self.cy = self.cy.saturating_sub(p(0, 1)).max(self.top);
                self.cx = 0;
            }
            'G' | '`' => self.cx = (p(0, 1) - 1).min(self.cols - 1),
            'd' => {
                let x = self.cx;
                self.goto(x, p(0, 1) - 1);
            }
            'H' | 'f' => self.goto(p(1, 1) - 1, p(0, 1) - 1),
            'J' => {
                let (x, y) = (self.cx, self.cy);
                match raw(0) {
                    0 => {
                        self.erase_line_part(y, x, self.cols);
                        for yy in y + 1..self.rows {
                            self.erase_line_part(yy, 0, self.cols);
                        }
                    }
                    1 => {
                        for yy in 0..y {
                            self.erase_line_part(yy, 0, self.cols);
                        }
                        self.erase_line_part(y, 0, x + 1);
                    }
                    2 | 3 => {
                        for yy in 0..self.rows {
                            self.erase_line_part(yy, 0, self.cols);
                        }
                        if raw(0) == 3 {
                            self.scrollback.clear();
                        }
                    }
                    _ => {}
                }
            }
            'K' => {
                let (x, y) = (self.cx, self.cy);
                match raw(0) {
                    0 => self.erase_line_part(y, x, self.cols),
                    1 => self.erase_line_part(y, 0, x + 1),
                    2 => self.erase_line_part(y, 0, self.cols),
                    _ => {}
                }
            }
            'L' if (self.top..=self.bottom).contains(&self.cy) => {
                let top = self.top;
                self.top = self.cy;
                self.scroll_down(p(0, 1));
                self.top = top;
            }
            'M' if (self.top..=self.bottom).contains(&self.cy) => {
                let top = self.top;
                self.top = self.cy;
                self.scroll_up_no_scrollback(p(0, 1));
                self.top = top;
            }
            'P' => {
                let (x, y, n) = (self.cx, self.cy, p(0, 1).min(self.cols - self.cx));
                let b = self.blank();
                let l = &mut self.lines[y];
                l.drain(x..x + n);
                l.extend(std::iter::repeat_n(b, n));
            }
            '@' => {
                let (x, y, n) = (self.cx, self.cy, p(0, 1).min(self.cols - self.cx));
                let b = self.blank();
                let cols = self.cols;
                let l = &mut self.lines[y];
                for _ in 0..n {
                    l.insert(x, b);
                }
                l.truncate(cols);
            }
            'X' => {
                let (x, y) = (self.cx, self.cy);
                self.erase_line_part(y, x, x + p(0, 1));
            }
            'S' => self.scroll_up_no_scrollback(p(0, 1)),
            'T' => self.scroll_down(p(0, 1)),
            'r' if !private_q => {
                let (t, b) = (p(0, 1) - 1, p(1, self.rows) - 1);
                if t < b && b < self.rows {
                    self.top = t;
                    self.bottom = b;
                    self.goto(0, 0);
                }
            }
            'm' if private.is_none() => self.sgr(params),
            'h' => self.mode(private_q, params, true),
            'l' => self.mode(private_q, params, false),
            's' if private.is_none() => self.save_cursor(),
            'u' if private.is_none() => self.restore_cursor(),
            'n' if private.is_none() => match raw(0) {
                5 => self.out.extend_from_slice(b"\x1b[0n"),
                6 => self.out.extend_from_slice(format!("\x1b[{};{}R", self.cy + 1, self.cx + 1).as_bytes()),
                _ => {}
            },
            'c' if private.is_none() => self.out.extend_from_slice(b"\x1b[?62;22c"),
            'c' if private == Some(b'>') => self.out.extend_from_slice(b"\x1b[>1;10;0c"),
            _ => {}
        }
        if !matches!(last, 'm' | 'n' | 'c' | 's') {
            self.wrap_next = false;
        }
        self.dirty = true;
    }

    fn esc(&mut self, intermediates: &[u8], last: u8) {
        // Designating a character set: ESC ( for G0, ESC ) for G1; "0" is DEC
        // Special Graphics (line drawing), anything else ("B", "A", "U") ASCII.
        // Other intermediates (ESC # 8 and the like): not yet.
        if let [i] = intermediates {
            if let Some(g) = match i {
                b'(' => Some(0),
                b')' => Some(1),
                _ => None,
            } {
                self.charsets[g] = last == b'0';
            }
            return;
        }
        if !intermediates.is_empty() {
            return;
        }
        match last {
            b'7' => self.save_cursor(),
            b'8' => self.restore_cursor(),
            b'D' => self.linefeed(),
            b'E' => {
                self.cx = 0;
                self.linefeed();
            }
            b'M' => {
                if self.cy == self.top {
                    self.scroll_down(1);
                } else {
                    self.cy = self.cy.saturating_sub(1);
                }
            }
            b'c' => {
                let (c, r) = (self.cols, self.rows);
                let title = std::mem::take(&mut self.title);
                *self = Grid::new(c, r);
                self.title = title;
            }
            _ => {}
        }
        self.wrap_next = false;
        self.dirty = true;
    }

    fn osc(&mut self, data: &[u8]) {
        let s = String::from_utf8_lossy(data);
        if let Some((code, text)) = s.split_once(';')
            && (code == "0" || code == "2")
        {
            self.title = text.chars().take(200).collect();
            self.dirty = true;
        } else if let Some(("52", rest)) = s.split_once(';')
            && let Some((_targets, data)) = rest.split_once(';')
        {
            // "?" asks to read the clipboard: refused (no reply), as most terminals do.
            use base64::Engine;
            if data != "?"
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data.trim())
            {
                self.clipboard = Some(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
    }
}

impl Grid {
    /// DECRC / CSI u: the saved cursor, kept on the screen. The screen may have
    /// shrunk since the save (an editor saves, the window is resized, it
    /// restores): an unclamped restore put the cursor past the last line and
    /// the next character crashed the terminal (found by the fuzz test).
    fn restore_cursor(&mut self) {
        let (x, y, pen) = self.saved;
        self.cx = x.min(self.cols - 1);
        self.cy = y.min(self.rows - 1);
        self.pen = pen;
        (self.charsets, self.shift_out) = self.saved_charsets;
    }

    /// DECSC / CSI s: the cursor, its pen and the character sets.
    fn save_cursor(&mut self) {
        self.saved = (self.cx, self.cy, self.pen);
        self.saved_charsets = (self.charsets, self.shift_out);
    }

    /// Delete lines / scroll up without sending lines to the scrollback.
    fn scroll_up_no_scrollback(&mut self, n: usize) {
        for _ in 0..n.min(self.bottom + 1 - self.top) {
            self.lines.remove(self.top);
            let b = self.blank();
            self.lines.insert(self.bottom, vec![b; self.cols]);
        }
        self.dirty = true;
    }

    /// The screen as text: each row's characters (a wide one's right half
    /// left out, combining marks kept), trailing spaces trimmed, the rows
    /// joined by line ends, the blank rows at the bottom left out (the meaning
    /// tree's value: terminal agent milestone 1; tests).
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| {
                let mut text = String::new();
                l.iter().filter(|c| c.width != 0).for_each(|c| c.push_to(&mut text));
                text.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_string()
    }
}

impl Grid {
    /// The scrollback as text, oldest first, as [`Grid::text`] makes a row:
    /// the last `max_lines` lines, and no more than `max_bytes` (the oldest
    /// left out first), for the meaning tree's scrollback node.
    pub fn scrollback_text(&self, max_lines: usize, max_bytes: usize) -> String {
        let mut rows: Vec<String> = Vec::new();
        let mut bytes = 0;
        for l in self.scrollback.iter().rev().take(max_lines) {
            let mut text = String::new();
            l.iter().filter(|c| c.width != 0).for_each(|c| c.push_to(&mut text));
            let text = text.trim_end().to_string();
            if bytes + text.len() + 1 > max_bytes {
                break;
            }
            bytes += text.len() + 1;
            rows.push(text);
        }
        rows.reverse();
        rows.join("\n")
    }
}

/// DEC Special Graphics: what the VT100 drew for ` through ~ when line
/// drawing was chosen (the table xterm and the VT100 manual give).
fn special_graphics(c: char) -> char {
    match c {
        '_' => '\u{a0}',
        '`' => '◆',
        'a' => '▒',
        'b' => '␉',
        'c' => '␌',
        'd' => '␍',
        'e' => '␊',
        'f' => '°',
        'g' => '±',
        'h' => '␤',
        'i' => '␋',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vt::Parser;

    fn feed(g: &mut Grid, s: &str) {
        Parser::new().advance(g, s.as_bytes());
    }

    /// The meaning tree's screen: a wide character once, trailing spaces gone.
    #[test]
    fn the_screen_as_text_for_its_meaning() {
        let mut g = Grid::new(12, 3);
        feed(&mut g, "a\u{4e2d}b   \r\ne\u{301}x");
        assert_eq!(g.text(), "a\u{4e2d}b\ne\u{301}x", "the wide one's right half left out, the accent kept");
    }

    /// The scrollback, oldest first, the newest kept when it is capped.
    #[test]
    fn the_scrollback_as_text_capped() {
        let mut g = Grid::new(10, 2);
        for i in 1..=6 {
            feed(&mut g, &format!("line {i}\r\n"));
        }
        assert_eq!(g.scrollback_text(100, 1 << 20), "line 1\nline 2\nline 3\nline 4\nline 5");
        assert_eq!(g.scrollback_text(2, 1 << 20), "line 4\nline 5", "the last lines");
        assert_eq!(g.scrollback_text(100, 14), "line 4\nline 5", "no more bytes than asked, the oldest dropped");
    }

    #[test]
    fn line_drawing_as_ncurses_sends_it() {
        // mc's frame, as ncurses draws it with TERM=xterm-256color.
        let mut g = Grid::new(10, 3);
        feed(&mut g, "\x1b(0lqqk\x1b(B ok\r\n\x1b(0x  x\r\nmqqj\x1b(B");
        assert_eq!(g.text(), "┌──┐ ok\n│  │\n└──┘", "line drawing until ESC ( B, letters after it");
    }

    #[test]
    fn shift_out_uses_g1_and_shift_in_goes_back() {
        let mut g = Grid::new(10, 1);
        feed(&mut g, "\x1b)0q\x0eq\x0fq");
        assert_eq!(g.text(), "q─q", "only while G1 is shifted in, and G1 is line drawing");
    }

    #[test]
    fn character_sets_are_saved_with_the_cursor() {
        let mut g = Grid::new(10, 1);
        // Saved at column 0 with line drawing on; a letter in ASCII; restored: back
        // at column 0, line drawing again, so the second q draws a line over it.
        feed(&mut g, "\x1b(0\x1b7\x1b(Bq\x1b8q");
        assert_eq!(g.text(), "─");
    }

    #[test]
    fn printing_wrapping_and_scrolling() {
        let mut g = Grid::new(5, 3);
        feed(&mut g, "abcdefg");
        assert_eq!(g.text(), "abcde\nfg");
        feed(&mut g, "\r\n1\r\n2\r\n3");
        assert_eq!(g.text(), "1\n2\n3");
        assert_eq!(g.scrollback.len(), 2);
        assert_eq!((g.cx, g.cy), (1, 2));
    }

    #[test]
    fn restored_cursor_stays_on_a_smaller_screen() {
        let mut g = Grid::new(80, 24);
        feed(&mut g, "\x1b[24;80H\x1b7\x1b[1;1H");
        g.resize(10, 4);
        feed(&mut g, "\x1b8x");
        assert_eq!((g.cx, g.cy), (9, 3));
        feed(&mut g, "\x1b[4;10H\x1b[s");
        g.resize(5, 2);
        feed(&mut g, "\x1b[uy");
        assert_eq!(g.text().lines().count(), 2);
    }

    #[test]
    fn cursor_moves_and_erasing() {
        let mut g = Grid::new(10, 3);
        feed(&mut g, "hello\x1b[1;3HX\x1b[2;1Hworld\x1b[1;1H\x1b[K");
        assert_eq!(g.text(), "\nworld");
        feed(&mut g, "\x1b[2;3H\x1b[1K");
        assert_eq!(g.text(), "\n   ld");
        feed(&mut g, "\x1b[2J");
        assert_eq!(g.text(), "");
    }

    #[test]
    fn colors() {
        let mut g = Grid::new(10, 2);
        feed(&mut g, "\x1b[1;31ma\x1b[38;5;200mb\x1b[38;2;1;2;3mc\x1b[0md");
        let l = g.line(0);
        assert!(l[0].style.bold && l[0].style.fg == Color::Indexed(1));
        assert_eq!(l[1].style.fg, Color::Indexed(200));
        assert_eq!(l[2].style.fg, Color::Rgb(1, 2, 3));
        assert_eq!(l[3].style, Style::default());
    }

    #[test]
    fn combining_marks_stay_on_their_character() {
        let mut g = Grid::new(6, 2);
        feed(&mut g, "e\u{301}a\u{308}x");
        assert_eq!((g.line(0)[0].c, g.line(0)[0].marks), ('e', ['\u{301}', '\0']));
        assert_eq!(g.line(0)[1].marks[0], '\u{308}');
        assert_eq!((g.line(0)[2].c, g.cx), ('x', 3), "a mark takes no cell");
        assert_eq!(g.text(), "e\u{301}a\u{308}x");
        // Two marks are kept, a third is dropped; a mark with nothing before it too.
        let mut g = Grid::new(6, 2);
        feed(&mut g, "\u{301}e\u{323}\u{302}\u{301}");
        assert_eq!(g.line(0)[0].marks, ['\u{323}', '\u{302}']);
        assert_eq!(g.text(), "e\u{323}\u{302}");
        // On the last column (the cursor waits there) and on a wide character.
        let mut g = Grid::new(4, 2);
        feed(&mut g, "abcd\u{301}日\u{301}");
        assert_eq!(g.line(0)[3].marks[0], '\u{301}');
        assert_eq!((g.line(1)[0].c, g.line(1)[0].marks[0]), ('日', '\u{301}'));
        // Written over, the marks go with the character.
        feed(&mut g, "\rz");
        assert_eq!((g.line(1)[0].c, g.line(1)[0].marks), ('z', NO_MARKS));
    }

    #[test]
    fn wide_characters() {
        let mut g = Grid::new(4, 2);
        feed(&mut g, "a漢b");
        assert_eq!(g.line(0)[1].width, 2);
        assert_eq!(g.line(0)[2].width, 0);
        assert_eq!(g.text(), "a漢b");
        // At the last column a wide character goes to the next line.
        let mut g = Grid::new(4, 2);
        feed(&mut g, "abc漢");
        assert_eq!(g.text(), "abc\n漢");
    }

    #[test]
    fn alternate_screen_and_regions() {
        let mut g = Grid::new(6, 4);
        feed(&mut g, "shell");
        feed(&mut g, "\x1b[?1049h\x1b[Hvim");
        assert_eq!(g.text(), "vim");
        feed(&mut g, "\x1b[?1049l");
        assert_eq!(g.text(), "shell");
        // A region of rows 2-3: scrolling stays inside it.
        let mut g = Grid::new(6, 4);
        feed(&mut g, "1\r\n2\r\n3\r\n4\x1b[2;3r\x1b[3;1H\n");
        assert_eq!(g.text(), "1\n3\n\n4");
    }

    #[test]
    fn replies() {
        let mut g = Grid::new(10, 3);
        feed(&mut g, "\x1b[2;4H\x1b[6n\x1b[c");
        assert_eq!(String::from_utf8_lossy(&g.out), "\x1b[2;4R\x1b[?62;22c");
        feed(&mut g, "\x1b]0;my title\x07");
        assert_eq!(g.title, "my title");
        // OSC 52: a copy (base64 of "hello"); a read request is ignored.
        feed(&mut g, "\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(g.clipboard.take().as_deref(), Some("hello"));
        feed(&mut g, "\x1b]52;c;?\x1b\\");
        assert_eq!(g.clipboard, None);
    }

    #[test]
    fn absolute_lines() {
        let mut g = Grid::new(5, 2);
        feed(&mut g, "a\r\nb\r\nc\r\nd");
        assert_eq!((g.pushed, g.scrollback.len()), (2, 2));
        let text = |n| g.abs_line(n).map(|l| l[0].c);
        assert_eq!([text(0), text(1), text(2), text(3), text(4)], [Some('a'), Some('b'), Some('c'), Some('d'), None]);
        // Clearing the scrollback (ED 3, which clears the screen too here) keeps the numbers.
        feed(&mut g, "\x1b[3J");
        assert_eq!(g.abs_line(1), None);
        assert!(g.abs_line(2).is_some());
        assert_eq!(g.pushed, 2);
    }

    #[test]
    fn mouse_modes() {
        let mut g = Grid::new(10, 3);
        feed(&mut g, "\x1b[?1000h\x1b[?1006h");
        assert_eq!((g.mouse, g.mouse_sgr), (MouseMode::Click, true));
        feed(&mut g, "\x1b[?1002h");
        assert_eq!(g.mouse, MouseMode::Drag);
        // Resetting a mode that isn't on leaves reporting alone.
        feed(&mut g, "\x1b[?1000l");
        assert_eq!(g.mouse, MouseMode::Drag);
        feed(&mut g, "\x1b[?1002l\x1b[?1006l");
        assert_eq!((g.mouse, g.mouse_sgr), (MouseMode::Off, false));
        // A full reset turns it off.
        feed(&mut g, "\x1b[?1003h\x1bc");
        assert_eq!(g.mouse, MouseMode::Off);
    }

    #[test]
    fn resize_keeps_the_cursor_line() {
        let mut g = Grid::new(10, 4);
        feed(&mut g, "1\r\n2\r\n3\r\n4");
        g.resize(10, 2);
        assert_eq!(g.text(), "3\n4");
        assert_eq!(g.cy, 1);
        g.resize(20, 3);
        assert_eq!((g.cols, g.rows), (20, 3));
    }
}
