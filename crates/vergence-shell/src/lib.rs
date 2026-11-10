//! The shell's arithmetic and, later, its knowledge of applications
//! (whitepaper §6.9; docs/shelf-milestone-1.md).
//!
//! Step 1 is here: the shelf's strip on the right edge of its screen, the
//! **work area** it leaves (what maximising, placing and fitting windows,
//! the desktop menu and notification cards keep inside), and moving a
//! remembered window clear of the strip. Rectangles are in points, in the
//! desktop's own space (a screen's rectangle is where the arrangement put
//! it).

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

/// The strip's width, in points (the same physical width on every screen).
pub const STRIP_W: i32 = vergence_look::SHELF_W as i32;

/// The shelf's strip on a screen: its right edge, top to bottom. A screen
/// narrower than three strips has none (nothing would be left to work in).
pub fn strip(screen: Rect) -> Option<Rect> {
    (screen.w >= 3 * STRIP_W && screen.h > 0).then(|| Rect::new(screen.right() - STRIP_W, screen.y, STRIP_W, screen.h))
}

/// What a screen with the shelf leaves for windows: all of it but the strip.
pub fn work_area(screen: Rect) -> Rect {
    match strip(screen) {
        Some(s) => Rect::new(screen.x, screen.y, screen.w - s.w, screen.h),
        None => screen,
    }
}

/// A window's remembered place on a screen that now has the shelf: moved
/// left until it is clear of the strip, as long as it then still starts on
/// the screen. A window too wide for that, or one the strip does not
/// touch, is given back as it was (the caller's own fitting deals with the
/// first).
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
    fn the_strip_is_the_right_edge_and_the_same_width_everywhere() {
        assert_eq!(strip(SMALL), Some(Rect::new(1280 - 48, 180, 48, 720)));
        assert_eq!(strip(LARGE), Some(Rect::new(1280 + 1920 - 48, 0, 48, 1080)));
        assert_eq!(strip(Rect::new(0, 0, 100, 500)), None, "too narrow a screen has no shelf");
    }

    #[test]
    fn the_work_area_is_the_screen_less_the_strip() {
        assert_eq!(work_area(SMALL), Rect::new(0, 180, 1232, 720));
        assert_eq!(work_area(LARGE), Rect::new(1280, 0, 1872, 1080));
        let w = work_area(LARGE);
        let s = strip(LARGE).unwrap();
        assert!(!w.overlaps(&s) && w.right() == s.x && LARGE.holds(&w) && LARGE.holds(&s));
        assert_eq!(work_area(Rect::new(0, 0, 100, 500)), Rect::new(0, 0, 100, 500));
    }

    #[test]
    fn a_remembered_window_is_moved_clear_of_the_strip() {
        // Its right 28 points would be under the strip: moved left by that much.
        let w = Rect::new(1280 + 1400, 100, 500, 400);
        let moved = clear_of_strip(w, LARGE);
        assert_eq!(moved, Rect::new(1280 + 1872 - 500, 100, 500, 400));
        assert!(work_area(LARGE).holds(&moved));
        // One the strip does not touch stays.
        let clear = Rect::new(1300, 50, 600, 400);
        assert_eq!(clear_of_strip(clear, LARGE), clear);
        // One wider than the work area is left for the caller to fit.
        let wide = Rect::new(1290, 0, 1900, 500);
        assert_eq!(clear_of_strip(wide, LARGE), wide);
        // A window on the other screen is not the strip's business.
        let other = Rect::new(600, 300, 600, 400);
        assert_eq!(clear_of_strip(other, LARGE), other);
    }
}
