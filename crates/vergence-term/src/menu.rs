//! The right-click menu: copy and paste for the mouse, and the pane and
//! tab commands. On the native road the server draws it (server_lines), as
//! every window's; elsewhere (under GNOME, the terminal's own bar) it is
//! drawn in the window, over the terminal, and kept inside it; where each line sits is worked out once ([`Menu::new`]) and used both
//! to draw it and to say what is under the pointer.

use vergence_look::RADIUS;
use vergence_render::text;
use vergence_scene::{Color, Point, Rect, Scene};
use vergence_text::{FaceId, Fonts};

const WIDTH: f32 = 236.0;
const EDGE: f32 = 4.0;
const ITEM_H: f32 = 28.0;
const LINE_H: f32 = 9.0;

const fn color(c: vergence_look::Rgb) -> Color {
    Color::rgb(c.0, c.1, c.2)
}
const GROUND: Color = color(vergence_look::MENU);
const INK: Color = color(vergence_look::MENU_INK);
const INK_OFF: Color = color(vergence_look::MENU_INK_OFF);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Copy,
    Paste,
    SplitSide,
    SplitStack,
    NewTab,
    ClosePane,
    /// The section (pane) into a tab of its own, or a window of its own (bar milestone 1, step 7).
    PaneToTab,
    PaneToWindow,
}

impl Item {
    /// What the line says, and the keys that do the same.
    fn words(self) -> (&'static str, &'static str) {
        match self {
            Item::Copy => ("Copy", "Ctrl+C"),
            Item::Paste => ("Paste", "Ctrl+V"),
            Item::SplitSide => ("Split side by side", "Ctrl+Shift+R"),
            Item::SplitStack => ("Split top and bottom", "Ctrl+Shift+D"),
            Item::NewTab => ("New tab", "Ctrl+Shift+T"),
            Item::ClosePane => ("Close pane", "Ctrl+Shift+W"),
            Item::PaneToTab => ("Move to a new tab", ""),
            Item::PaneToWindow => ("Move to a new window", ""),
        }
    }
}

/// The menu's lines from the top; None is a dividing line.
const LINES: [Option<Item>; 11] = [
    Some(Item::Copy),
    Some(Item::Paste),
    None,
    Some(Item::SplitSide),
    Some(Item::SplitStack),
    None,
    Some(Item::NewTab),
    Some(Item::ClosePane),
    None,
    Some(Item::PaneToTab),
    Some(Item::PaneToWindow),
];

/// The same menu for the server to draw, as every window's right-click
/// menu is (bus v20; 2026-11-18: one kind of menu everywhere): its lines,
/// each with the id of the terminal's command that does the same (`id`).
pub fn server_lines(can_copy: bool, id: impl Fn(Item) -> u32) -> Vec<vergence_ui::MenuLine> {
    LINES
        .iter()
        .map(|line| match line {
            None => vergence_ui::MenuLine::Separator,
            Some(item) => {
                let (label, keys) = item.words();
                vergence_ui::MenuLine::Item { id: id(*item), label: label.into(), keys: keys.into(), enabled: *item != Item::Copy || can_copy, checked: None }
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub area: Rect,
    rows: Vec<(Option<Item>, Rect)>,
    /// There is text selected: Copy can be chosen.
    pub can_copy: bool,
}

impl Menu {
    /// A menu opened at a point of a window of this size: its corner at the
    /// point where there is room, else moved up or left to stay in the window.
    pub fn new(x: f32, y: f32, window: (f32, f32), can_copy: bool) -> Menu {
        let height = 2.0 * EDGE + LINES.iter().map(|l| if l.is_some() { ITEM_H } else { LINE_H }).sum::<f32>();
        let left = x.min(window.0 - WIDTH).max(0.0);
        let top = y.min(window.1 - height).max(0.0);
        let mut at = top + EDGE;
        let rows = LINES
            .iter()
            .map(|line| {
                let h = if line.is_some() { ITEM_H } else { LINE_H };
                let row = Rect::new(left + EDGE, at, WIDTH - 2.0 * EDGE, h);
                at += h;
                (*line, row)
            })
            .collect();
        Menu { area: Rect::new(left, top, WIDTH, height), rows, can_copy }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.area.x && x < self.area.x + self.area.w && y >= self.area.y && y < self.area.y + self.area.h
    }

    /// The line under a point, if it is one that can be chosen now.
    pub fn at(&self, x: f32, y: f32) -> Option<Item> {
        let (item, _) = self.rows.iter().find(|(_, r)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)?;
        item.filter(|i| self.enabled(*i))
    }

    fn enabled(&self, item: Item) -> bool {
        item != Item::Copy || self.can_copy
    }

    /// Draws the menu; `hover` is the line under the pointer.
    pub fn draw(&self, scene: &mut Scene, fonts: &mut Fonts, face: Option<FaceId>, size: f32, accent: Color, hover: Option<Item>) {
        scene.fill_rounded(self.area, RADIUS, GROUND);
        scene.stroke(self.area, RADIUS, 1.0, Color::rgba(255, 255, 255, 38));
        for (line, row) in &self.rows {
            let Some(item) = line else {
                scene.fill(Rect::new(row.x + 4.0, row.y + (row.h - 1.0) / 2.0, row.w - 8.0, 1.0), Color::rgba(255, 255, 255, 31));
                continue;
            };
            let chosen = hover == Some(*item) && self.enabled(*item);
            if chosen {
                scene.fill_rounded(*row, RADIUS, Color::rgba(accent.r, accent.g, accent.b, 230));
            }
            let Some(face) = face else { continue };
            let ink = if chosen {
                Color::rgb(255, 255, 255)
            } else if self.enabled(*item) {
                INK
            } else {
                INK_OFF
            };
            let (label, keys) = item.words();
            let m = fonts.metrics(face, size);
            let baseline = (row.y + (row.h + m.ascent - m.descent) / 2.0).round();
            text(scene, fonts, face, size, ink, Point::new(row.x + 10.0, baseline), label);
            // The keys, smaller, against the right edge.
            let small = size - 2.0;
            let width = fonts.layout(face, small, keys).1;
            text(scene, fonts, face, small, if chosen { ink } else { INK_OFF }, Point::new(row.x + row.w - 10.0 - width, baseline), keys);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_stays_in_the_window() {
        let m = Menu::new(100.0, 80.0, (800.0, 600.0), true);
        assert_eq!((m.area.x, m.area.y, m.area.w, m.area.h), (100.0, 80.0, 236.0, 259.0), "at the pointer where there is room");
        let m = Menu::new(790.0, 590.0, (800.0, 600.0), true);
        assert_eq!((m.area.x + m.area.w, m.area.y + m.area.h), (800.0, 600.0), "moved in from the corner");
        let m = Menu::new(50.0, 20.0, (120.0, 100.0), true);
        assert_eq!((m.area.x, m.area.y), (0.0, 0.0), "a window smaller than the menu: from its corner");
    }

    #[test]
    fn what_is_under_the_pointer() {
        let m = Menu::new(100.0, 80.0, (800.0, 600.0), true);
        assert_eq!(m.at(120.0, 90.0), Some(Item::Copy));
        assert_eq!(m.at(120.0, 84.0 + 28.0 + 5.0), Some(Item::Paste));
        assert_eq!(m.at(120.0, 84.0 + 56.0 + 4.0), None, "the dividing line");
        assert_eq!(m.at(120.0, 84.0 + 65.0 + 5.0), Some(Item::SplitSide));
        assert_eq!(m.at(120.0, 84.0 + 93.0 + 5.0), Some(Item::SplitStack));
        assert_eq!(m.at(120.0, 84.0 + 121.0 + 4.0), None, "the second dividing line");
        assert_eq!(m.at(120.0, 84.0 + 130.0 + 5.0), Some(Item::NewTab));
        assert_eq!(m.at(120.0, 84.0 + 158.0 + 27.0), Some(Item::ClosePane));
        assert_eq!(m.at(120.0, 84.0 + 186.0 + 4.0), None, "the third dividing line");
        assert_eq!(m.at(120.0, 84.0 + 195.0 + 5.0), Some(Item::PaneToTab));
        assert_eq!(m.at(120.0, 84.0 + 223.0 + 5.0), Some(Item::PaneToWindow));
        assert_eq!((m.at(101.0, 90.0), m.at(400.0, 90.0)), (None, None), "the edge and beyond");
        assert!(m.contains(101.0, 81.0) && !m.contains(99.0, 90.0) && !m.contains(120.0, 339.5));
        // Nothing selected: Copy cannot be chosen, the rest can.
        let m = Menu::new(100.0, 80.0, (800.0, 600.0), false);
        assert_eq!((m.at(120.0, 90.0), m.at(120.0, 84.0 + 28.0 + 5.0)), (None, Some(Item::Paste)));
    }
}
