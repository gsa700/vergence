//! The shell's arithmetic and, later, its knowledge of applications
//! (whitepaper §6.9; docs/shelf-milestone-1.md).
//!
//! Step 2 is the applications a system has ([`apps`], from desktop
//! entries read by [`desktop`]) and their icons ([`icons`]).
//!
//! Step 1 is here: the shelf's column on the right edge of its screen and
//! the dock that stands in it, the **work area** the column leaves (what maximising, placing and fitting windows,
//! the desktop menu and notification cards keep inside), and moving a
//! remembered window clear of the strip. Rectangles are in points, in the
//! desktop's own space (a screen's rectangle is where the arrangement put
//! it).

pub mod apps;
pub mod desktop;
pub mod icons;
pub mod running;

/// A rectangle in points: its top-left corner and its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    /// True if the two share any area.
    pub const fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.right() && other.x < self.right() && self.y < other.bottom() && other.y < self.bottom()
    }

    /// True if `other` lies wholly inside.
    pub const fn holds(&self, other: &Rect) -> bool {
        other.x >= self.x && other.y >= self.y && other.right() <= self.right() && other.bottom() <= self.bottom()
    }
}

/// The shelf's width, in points (the same physical width on every screen),
/// and the room it keeps from the screen's edge and from the windows.
pub const STRIP_W: i32 = vergence_look::SHELF_W as i32;
pub const GAP: i32 = vergence_look::SHELF_GAP as i32;

/// The column the shelf stands in: the right edge of its screen, top to
/// bottom, as wide as the shelf and the room on both its sides. Windows keep
/// out of all of it, though the shelf itself (a dock, [`dock`]) is only as
/// tall as what it holds. A screen narrower than three shelves has none
/// (nothing would be left to work in).
pub fn strip(screen: Rect) -> Option<Rect> {
    let w = STRIP_W + 2 * GAP;
    (screen.w >= 3 * STRIP_W && screen.h > 0).then(|| Rect::new(screen.right() - w, screen.y, w, screen.h))
}

/// The shelf itself (Dave, 2026-11-10: "like a dock"): a panel as tall as
/// the apps button's place and `items` places under it, in the middle of
/// its column's height, a gap in from the screen's edge. On a screen too
/// short for all of them it is as tall as the screen allows, and holds the
/// items that fit.
pub fn dock(screen: Rect, items: usize) -> Option<Rect> {
    let column = strip(screen)?;
    let room = ((column.h - 2 * GAP) / running::SLOT).max(1);
    let places = (items as i32 + 1).min(room);
    let h = places * running::SLOT;
    Some(Rect::new(column.x + GAP, column.y + (column.h - h) / 2, STRIP_W, h))
}

/// What a screen with the shelf leaves for windows: all of it but the
/// shelf's column.
pub fn work_area(screen: Rect) -> Rect {
    match strip(screen) {
        Some(s) => Rect::new(screen.x, screen.y, screen.w - s.w, screen.h),
        None => screen,
    }
}

/// A window's remembered place on a screen that now has the shelf: moved
/// left until it is clear of the shelf's column, as long as it then still
/// starts on the screen. A window too wide for that, or one the column does
/// not touch, is given back as it was (the caller's own fitting deals with
/// the first).
pub fn clear_of_strip(window: Rect, screen: Rect) -> Rect {
    let Some(strip) = strip(screen) else { return window };
    if !window.overlaps(&strip) {
        return window;
    }
    let moved = Rect::new(strip.x - window.w, window.y, window.w, window.h);
    if moved.x >= screen.x { moved } else { window }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hubble's two screens as the server lays them out: the 14" 4K at scale 3
    // (1280 x 720 points) and the ASUS at 1 (1920 x 1080) to its right.
    const SMALL: Rect = Rect::new(0, 180, 1280, 720);
    const LARGE: Rect = Rect::new(1280, 0, 1920, 1080);

    #[test]
    fn the_column_is_the_right_edge_and_the_same_width_everywhere() {
        assert_eq!(strip(SMALL), Some(Rect::new(1280 - 60, 180, 60, 720)));
        assert_eq!(strip(LARGE), Some(Rect::new(1280 + 1920 - 60, 0, 60, 1080)));
        assert_eq!(strip(Rect::new(0, 0, 100, 500)), None, "too narrow a screen has no shelf");
    }

    #[test]
    fn the_dock_is_as_tall_as_what_it_holds() {
        // The apps button's place and three items: four places, in the middle of the height.
        assert_eq!(dock(LARGE, 3), Some(Rect::new(1280 + 1920 - 54, (1080 - 192) / 2, 48, 192)));
        // Nothing running: the apps button's place alone.
        assert_eq!(dock(SMALL, 0), Some(Rect::new(1280 - 54, 180 + (720 - 48) / 2, 48, 48)));
        // More than the screen can hold: as tall as it allows, a gap from top and bottom.
        let full = dock(SMALL, 40).unwrap();
        assert_eq!(full.h, 14 * 48);
        assert!(full.y >= SMALL.y + GAP && full.bottom() <= SMALL.bottom() - GAP);
        // It stands inside its column, a gap from the screen's edge and from the windows.
        let (d, column) = (dock(LARGE, 5).unwrap(), strip(LARGE).unwrap());
        assert!(column.holds(&d) && d.x - column.x == GAP && column.right() - d.right() == GAP);
        assert_eq!(dock(Rect::new(0, 0, 100, 500), 3), None);
        // The places of its items are the ones `running` gives for it.
        assert_eq!(running::slot(d, 0), Some(Rect::new(d.x, d.y + 48, 48, 48)));
        assert!(running::slot(d, 4).is_some() && running::slot(d, 5).is_none());
    }

    #[test]
    fn the_work_area_is_the_screen_less_the_column() {
        assert_eq!(work_area(SMALL), Rect::new(0, 180, 1220, 720));
        assert_eq!(work_area(LARGE), Rect::new(1280, 0, 1860, 1080));
        let w = work_area(LARGE);
        let s = strip(LARGE).unwrap();
        assert!(!w.overlaps(&s) && w.right() == s.x && LARGE.holds(&w) && LARGE.holds(&s));
        assert_eq!(work_area(Rect::new(0, 0, 100, 500)), Rect::new(0, 0, 100, 500));
    }

    #[test]
    fn a_remembered_window_is_moved_clear_of_the_column() {
        // Its right 40 points would be in the column: moved left by that much.
        let w = Rect::new(1280 + 1400, 100, 500, 400);
        let moved = clear_of_strip(w, LARGE);
        assert_eq!(moved, Rect::new(1280 + 1860 - 500, 100, 500, 400));
        assert!(work_area(LARGE).holds(&moved));
        // One the column does not touch stays.
        let clear = Rect::new(1300, 50, 600, 400);
        assert_eq!(clear_of_strip(clear, LARGE), clear);
        // One wider than the work area is left for the caller to fit.
        let wide = Rect::new(1290, 0, 1900, 500);
        assert_eq!(clear_of_strip(wide, LARGE), wide);
        // A window on the other screen is not the shelf's business.
        let other = Rect::new(600, 300, 600, 400);
        assert_eq!(clear_of_strip(other, LARGE), other);
    }
}
