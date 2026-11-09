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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub c: char,
    pub style: Style,
    /// 2: the left half of a double-width character; 0: its right half.
    pub width: u8,
}

impl Default for Cell {
    fn default() -> Self {
        Cell { c: ' ', style: Style::default(), width: 1 }
    }
}

const SCROLLBACK: usize = 10_000;

pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    lines: Vec<Vec<Cell>>,
    /// The other screen (primary while the alternate one is shown).
    other: Vec<Vec<Cell>>,
    pub alt: bool,
    pub scrollback: VecDeque<Vec<Cell>>,
    pub cx: usize,
    pub cy: usize,
    /// The cursor is past the last column: the next character wraps first.
    wrap_next: bool,
    saved: (usize, usize, Style),
    pen: Style,
    top: usize,
    bottom: usize,
    pub autowrap: bool,
    pub origin: bool,
    pub cursor_visible: bool,
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    pub title: String,
    /// Replies to the program (device status, attributes).
    pub out: Vec<u8>,
    /// Something changed since the last look.
    pub dirty: bool,
}

/// How many columns a character takes: 2 for wide East Asian characters and
/// emoji, 0 for combining marks, else 1.
pub fn width(c: char) -> usize {
    let u = c as u32;
    if (0x0300..=0x036f).contains(&u) || (0x200b..=0x200f).contains(&u) || (0xfe00..=0xfe0f).contains(&u) || (0x1ab0..=0x1aff).contains(&u) || (0x20d0..=0x20ff).contains(&u) {
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
            cx: 0,
            cy: 0,
            wrap_next: false,
            saved: (0, 0, Style::default()),
            pen: Style::default(),
            top: 0,
            bottom: rows - 1,
            autowrap: true,
            origin: false,
            cursor_visible: true,
            app_cursor: false,
            bracketed_paste: false,
            title: String::new(),
            out: Vec::new(),
            dirty: true,
        }
    }

    pub fn line(&self, y: usize) -> &[Cell] {
        &self.lines[y]
    }

    /// A blank cell in the current background (erasing uses the pen's background).
    fn blank(&self) -> Cell {
        Cell { c: ' ', style: Style { bg: self.pen.bg, ..Style::default() }, width: 1 }
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
        if self.scrollback.len() > SCROLLBACK {
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
                        self.saved = (self.cx, self.cy, self.pen);
                        self.set_alt(true);
                    } else {
                        self.set_alt(false);
                        self.restore_cursor();
                    }
                }
                (true, 2004) => self.bracketed_paste = on,
                _ => {}
            }
        }
        self.dirty = true;
    }
}

impl Perform for Grid {
    fn print(&mut self, c: char) {
        let w = width(c);
        if w == 0 {
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
        self.lines[y][x] = Cell { c, style: self.pen, width: w as u8 };
        if w == 2 {
            self.lines[y][x + 1] = Cell { c: ' ', style: self.pen, width: 0 };
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
            'B' | 'e' => self.cy = (self.cy + p(0, 1)).min(if self.cy <= self.bottom { self.bottom } else { self.rows - 1 }),
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
            's' if private.is_none() => self.saved = (self.cx, self.cy, self.pen),
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
        if !intermediates.is_empty() {
            return; // Character sets (ESC ( B): UTF-8 only.
        }
        match last {
            b'7' => self.saved = (self.cx, self.cy, self.pen),
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
        }
    }
}

impl Grid {
    /// DECRC / CSI u: the saved cursor, kept on the screen. The screen may have
    /// shrunk since the save (an editor saves, the window is resized, it
    /// restores): an unclamped restore put the cursor past the last line and
    /// the next character crashed the terminal (fuzz test, 2026-11-08).
    fn restore_cursor(&mut self) {
        let (x, y, pen) = self.saved;
        self.cx = x.min(self.cols - 1);
        self.cy = y.min(self.rows - 1);
        self.pen = pen;
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

    /// The screen as text (tests; copying, later).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.iter().filter(|c| c.width != 0).map(|c| c.c).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vt::Parser;

    fn feed(g: &mut Grid, s: &str) {
        Parser::new().advance(g, s.as_bytes());
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
    fn colours() {
        let mut g = Grid::new(10, 2);
        feed(&mut g, "\x1b[1;31ma\x1b[38;5;200mb\x1b[38;2;1;2;3mc\x1b[0md");
        let l = g.line(0);
        assert!(l[0].style.bold && l[0].style.fg == Color::Indexed(1));
        assert_eq!(l[1].style.fg, Color::Indexed(200));
        assert_eq!(l[2].style.fg, Color::Rgb(1, 2, 3));
        assert_eq!(l[3].style, Style::default());
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
