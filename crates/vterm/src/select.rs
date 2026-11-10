//! Text selected with the mouse.
//!
//! A selection is counted in cells: a column and a line's absolute number
//! (`Grid::pushed`), so it stays on its text while output scrolls the screen
//! or the view is scrolled back. It grows by the unit it was started with:
//! a press and drag selects characters, a second press on the same cell
//! selects words, a third whole lines.

use crate::grid::{Cell, Grid};

/// A cell: (column, the line's absolute number).
pub type Pos = (usize, usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Cell,
    Word,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    unit: Unit,
    /// What the press took in (first and last cell), and what the pointer is over now.
    origin: (Pos, Pos),
    head: (Pos, Pos),
}

/// Reading order.
fn key(p: Pos) -> (usize, usize) {
    (p.1, p.0)
}

/// Letters, digits and what file names, paths and addresses are made of
/// count as a word; spaces as a run of spaces; anything else stands alone.
fn class(line: &[Cell], mut i: usize) -> u8 {
    while i > 0 && line[i].width == 0 {
        i -= 1;
    }
    let c = line[i].c;
    if c == ' ' {
        0
    } else if c.is_alphanumeric() || "_-./~:@%+#=?&".contains(c) {
        1
    } else {
        2
    }
}

/// The first and last cell of the unit at `pos`.
fn unit_at(grid: &Grid, unit: Unit, pos: Pos) -> (Pos, Pos) {
    let (col, row) = pos;
    let line = grid.abs_line(row).unwrap_or(&[]);
    if unit == Unit::Line {
        return ((0, row), (grid.cols.saturating_sub(1), row));
    }
    if col >= line.len() {
        return (pos, pos);
    }
    // A double-width character is both its cells.
    let (mut a, mut b) = (col, col);
    while a > 0 && line[a].width == 0 {
        a -= 1;
    }
    while b + 1 < line.len() && line[b + 1].width == 0 {
        b += 1;
    }
    let kind = class(line, col);
    if unit == Unit::Word && kind != 2 {
        while a > 0 && class(line, a - 1) == kind {
            a -= 1;
        }
        while b + 1 < line.len() && class(line, b + 1) == kind {
            b += 1;
        }
    }
    ((a, row), (b, row))
}

impl Selection {
    /// A press at `pos`.
    pub fn start(grid: &Grid, unit: Unit, pos: Pos) -> Selection {
        let at = unit_at(grid, unit, pos);
        Selection { unit, origin: at, head: at }
    }

    /// The pointer, button held, is now over `pos`.
    pub fn drag(&mut self, grid: &Grid, pos: Pos) {
        self.head = unit_at(grid, self.unit, pos);
    }

    /// First and last cell selected, in reading order. A press that never
    /// left its character selects nothing.
    pub fn range(&self) -> Option<(Pos, Pos)> {
        if self.unit == Unit::Cell && self.origin == self.head {
            return None;
        }
        let start = if key(self.origin.0) <= key(self.head.0) { self.origin.0 } else { self.head.0 };
        let end = if key(self.origin.1) >= key(self.head.1) { self.origin.1 } else { self.head.1 };
        Some((start, end))
    }

    /// The columns selected on a line (first and last), if any are.
    pub fn span(&self, row: usize, cols: usize) -> Option<(usize, usize)> {
        let ((sc, sr), (ec, er)) = self.range()?;
        if row < sr || row > er || cols == 0 {
            return None;
        }
        let a = if row == sr { sc } else { 0 };
        let b = if row == er { ec.min(cols - 1) } else { cols - 1 };
        (a <= b).then_some((a, b))
    }

    /// The selected text: lines joined with newlines, the spaces that only
    /// fill a line out to the window's edge dropped.
    pub fn text(&self, grid: &Grid) -> Option<String> {
        let ((_, sr), (_, er)) = self.range()?;
        let mut out = Vec::new();
        for row in sr..=er {
            let line = grid.abs_line(row).unwrap_or(&[]);
            let text: String = match self.span(row, line.len()) {
                Some((a, b)) => line[a..=b].iter().filter(|c| c.width != 0).map(|c| c.c).collect(),
                None => String::new(),
            };
            out.push(text.trim_end().to_string());
        }
        let text = out.join("\n");
        (!text.is_empty()).then_some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(text: &str) -> Grid {
        let mut g = Grid::new(60, 4);
        crate::vt::Parser::new().advance(&mut g, text.as_bytes());
        g
    }

    #[test]
    fn a_drag_selects_characters_in_either_direction() {
        let g = grid("hello world\r\nsecond line\r\nthird");
        let mut s = Selection::start(&g, Unit::Cell, (6, 0));
        assert_eq!(s.range(), None, "a press alone selects nothing");
        s.drag(&g, (8, 0));
        assert_eq!(s.text(&g).as_deref(), Some("wor"));
        s.drag(&g, (2, 0));
        assert_eq!(s.text(&g).as_deref(), Some("llo w"), "backwards, the pressed character stays in");
        s.drag(&g, (5, 1));
        assert_eq!(s.text(&g).as_deref(), Some("world\nsecond"), "the rest of a line's cells are not copied as spaces");
        assert_eq!((s.span(0, 60), s.span(1, 60), s.span(2, 60)), (Some((6, 59)), Some((0, 5)), None));
        s.drag(&g, (1, 2));
        assert_eq!(s.text(&g).as_deref(), Some("world\nsecond line\nth"));
    }

    #[test]
    fn a_second_press_selects_words_and_a_third_lines() {
        let g = grid("see ~/src/vergence-ui/lib.rs: (two) words\r\nnext");
        let word = |col| Selection::start(&g, Unit::Word, (col, 0)).text(&g);
        assert_eq!(word(0).as_deref(), Some("see"));
        assert_eq!(word(9).as_deref(), Some("~/src/vergence-ui/lib.rs:"), "a path is one word");
        assert_eq!(word(30).as_deref(), Some("("), "punctuation stands alone");
        assert_eq!(word(32).as_deref(), Some("two"));
        // Dragging on by words: whole words at both ends, either way round.
        let mut s = Selection::start(&g, Unit::Word, (32, 0));
        s.drag(&g, (37, 0));
        assert_eq!(s.text(&g).as_deref(), Some("two) words"));
        s.drag(&g, (1, 0));
        assert_eq!(s.text(&g).as_deref(), Some("see ~/src/vergence-ui/lib.rs: (two"));
        let mut s = Selection::start(&g, Unit::Line, (5, 0));
        assert_eq!(s.text(&g).as_deref(), Some("see ~/src/vergence-ui/lib.rs: (two) words"));
        s.drag(&g, (0, 1));
        assert_eq!(s.text(&g).as_deref(), Some("see ~/src/vergence-ui/lib.rs: (two) words\nnext"));
    }

    #[test]
    fn wide_characters_are_taken_whole() {
        let g = grid("a日本b");
        // Cells: a, 日 (1-2), 本 (3-4), b. A press on 日's right half, dragged to 本's left half.
        let mut s = Selection::start(&g, Unit::Cell, (2, 0));
        s.drag(&g, (3, 0));
        assert_eq!(s.span(0, 60), Some((1, 4)));
        assert_eq!(s.text(&g).as_deref(), Some("日本"));
        assert_eq!(Selection::start(&g, Unit::Word, (4, 0)).text(&g).as_deref(), Some("a日本b"));
    }

    #[test]
    fn a_selection_stays_on_its_text_and_outlives_nothing() {
        let mut g = grid("first\r\nsecond\r\nthird\r\nfourth");
        let mut s = Selection::start(&g, Unit::Word, (0, 1));
        s.drag(&g, (0, 2));
        assert_eq!(s.text(&g).as_deref(), Some("second\nthird"));
        // Output scrolls the screen: the same text, now partly in the scrollback.
        crate::vt::Parser::new().advance(&mut g, b"\r\nfifth\r\nsixth");
        assert_eq!(s.text(&g).as_deref(), Some("second\nthird"));
        // Beyond the text, or on blank cells only: nothing to copy.
        let blank = Selection::start(&g, Unit::Line, (0, 400));
        assert_eq!(blank.text(&g), None);
    }
}
