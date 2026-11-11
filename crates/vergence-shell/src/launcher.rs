//! The applications popup's arithmetic: which applications a typed word
//! finds and in what order, the grid they are laid out in, and where the
//! arrow keys take the selection.

use crate::{Rect, desktop::Entry};

/// The applications to show for what has been typed, as places in
/// `entries`. Nothing typed: every one that is to be shown (not hidden,
/// not NoDisplay), by name. Something typed: those whose name has a word
/// beginning with it, then those with it anywhere in the name, then those
/// with it in a keyword; by name within each. Case is not looked at.
pub fn search(entries: &[Entry], typed: &str) -> Vec<usize> {
    let typed = typed.trim().to_lowercase();
    let mut found: Vec<(u8, String, usize)> = Vec::new();
    for (index, e) in entries.iter().enumerate() {
        if e.hidden || e.no_display {
            continue;
        }
        let name = e.name.to_lowercase();
        let rank = if typed.is_empty() || name.split(|c: char| !c.is_alphanumeric()).any(|word| word.starts_with(&typed)) {
            0
        } else if name.contains(&typed) {
            1
        } else if e.keywords.iter().any(|k| k.to_lowercase().contains(&typed)) {
            2
        } else {
            continue;
        };
        found.push((rank, name, index));
    }
    found.sort();
    found.into_iter().map(|(.., index)| index).collect()
}

/// The grid of applications: so many columns of cells this size, in an
/// area, scrolled down by `scroll` points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub area: Rect,
    pub columns: usize,
    pub cell_w: i32,
    pub cell_h: i32,
    pub scroll: i32,
}

impl Grid {
    /// As many whole columns as `area` is wide (at least one).
    pub fn new(area: Rect, cell_w: i32, cell_h: i32) -> Grid {
        Grid { area, columns: (area.w / cell_w.max(1)).max(1) as usize, cell_w, cell_h, scroll: 0 }
    }

    /// Where cell `index` is; it may be above or below the area.
    pub fn cell(&self, index: usize) -> Rect {
        let (row, column) = ((index / self.columns) as i32, (index % self.columns) as i32);
        Rect::new(self.area.x + column * self.cell_w, self.area.y + row * self.cell_h - self.scroll, self.cell_w, self.cell_h)
    }

    /// The cell at a point of the area, if one of `count` is there.
    pub fn at(&self, count: usize, x: i32, y: i32) -> Option<usize> {
        if x < self.area.x || y < self.area.y || x >= self.area.right() || y >= self.area.bottom() {
            return None;
        }
        let column = ((x - self.area.x) / self.cell_w) as usize;
        let row = ((y - self.area.y + self.scroll) / self.cell_h) as usize;
        let index = row * self.columns + column;
        (column < self.columns && index < count).then_some(index)
    }

    /// The furthest the grid scrolls for `count` cells.
    pub fn max_scroll(&self, count: usize) -> i32 {
        let rows = count.div_ceil(self.columns) as i32;
        (rows * self.cell_h - self.area.h).max(0)
    }

    /// Scrolled as little as shows cell `index` whole.
    pub fn showing(mut self, index: usize, count: usize) -> Grid {
        let top = (index / self.columns) as i32 * self.cell_h;
        self.scroll = self.scroll.min(top).max(top + self.cell_h - self.area.h).clamp(0, self.max_scroll(count));
        self
    }
}

/// An arrow key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Left,
    Right,
    Up,
    Down,
}

/// Where an arrow key takes the selection among `count` cells in `columns`
/// columns: left and right go on through the rows' ends, up and down stay in
/// their column, and none of them leaves the grid.
pub fn step(selected: usize, count: usize, columns: usize, step: Step) -> usize {
    if count == 0 {
        return 0;
    }
    let columns = columns.max(1);
    let selected = selected.min(count - 1);
    match step {
        Step::Left => selected.saturating_sub(1),
        Step::Right => (selected + 1).min(count - 1),
        Step::Up => if selected >= columns { selected - columns } else { selected },
        // Down from the last full row onto a short last row goes to its end.
        Step::Down => {
            if selected + columns < count {
                selected + columns
            } else if selected / columns < (count - 1) / columns {
                count - 1
            } else {
                selected
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, keywords: &[&str]) -> Entry {
        Entry { id: name.to_lowercase().replace(' ', "-"), name: name.into(), exec: vec!["x".into()], keywords: keywords.iter().map(|k| k.to_string()).collect(), ..Entry::default() }
    }

    fn names<'a>(entries: &'a [Entry], typed: &str) -> Vec<&'a str> {
        search(entries, typed).into_iter().map(|i| entries[i].name.as_str()).collect()
    }

    #[test]
    fn typing_finds_by_word_start_then_anywhere_then_keywords() {
        let mut hidden = entry("Fire Hidden", &[]);
        hidden.no_display = true;
        let entries = vec![entry("Thunderbird", &["mail"]), entry("Firefox", &["web", "browser"]), entry("Campfire", &[]), entry("Web Fire Tool", &[]), entry("Files", &["firewire"]), hidden, entry("albumwall", &[])];
        // Nothing typed: all that are shown, by name, whatever their case.
        assert_eq!(names(&entries, ""), ["albumwall", "Campfire", "Files", "Firefox", "Thunderbird", "Web Fire Tool"]);
        // A word's start first, then anywhere in the name, then a keyword.
        assert_eq!(names(&entries, "fire"), ["Firefox", "Web Fire Tool", "Campfire", "Files"]);
        assert_eq!(names(&entries, "  FIRE "), names(&entries, "fire"));
        assert_eq!(names(&entries, "mail"), ["Thunderbird"]);
        assert!(names(&entries, "zzz").is_empty());
    }

    #[test]
    fn the_grid_places_cells_and_finds_them_again() {
        let mut g = Grid::new(Rect::new(100, 50, 330, 200), 100, 80);
        assert_eq!(g.columns, 3);
        assert_eq!(g.cell(0), Rect::new(100, 50, 100, 80));
        assert_eq!(g.cell(4), Rect::new(200, 130, 100, 80));
        assert_eq!(g.at(7, 250, 140), Some(4));
        assert_eq!(g.at(4, 250, 140), None, "no cell there");
        assert_eq!(g.at(7, 410, 60), None, "right of the last column");
        assert_eq!(g.at(7, 99, 60), None);
        // Ten cells are four rows, 320 points, in 200: it scrolls 120.
        assert_eq!(g.max_scroll(10), 120);
        assert_eq!(g.max_scroll(6), 0);
        g = g.showing(9, 10);
        assert_eq!(g.scroll, 120);
        assert_eq!(g.at(10, 110, 240), Some(9));
        g = g.showing(0, 10);
        assert_eq!(g.scroll, 0);
    }

    #[test]
    fn arrows_stay_in_the_grid() {
        // Eight cells in three columns: 0 1 2 / 3 4 5 / 6 7.
        assert_eq!(step(0, 8, 3, Step::Left), 0);
        assert_eq!(step(2, 8, 3, Step::Right), 3, "on through the row's end");
        assert_eq!(step(7, 8, 3, Step::Right), 7);
        assert_eq!(step(1, 8, 3, Step::Up), 1);
        assert_eq!(step(4, 8, 3, Step::Up), 1);
        assert_eq!(step(4, 8, 3, Step::Down), 7);
        assert_eq!(step(5, 8, 3, Step::Down), 7, "onto the short last row's end");
        assert_eq!(step(7, 8, 3, Step::Down), 7);
        assert_eq!(step(0, 0, 3, Step::Down), 0);
        assert_eq!(step(99, 8, 0, Step::Left), 6);
    }
}
