//! The scrollbar: a thin bar in the padding at the terminal's right edge
//! that shows where the view is in the scrollback, and can be dragged.

use vergence_scene::Rect;

/// The bar's width, and the width at the right edge that takes hold of it.
pub const WIDTH: f32 = 5.0;
pub const GRAB: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scrollbar {
    pub track: Rect,
    pub thumb: Rect,
    /// Lines of scrollback: how far the view can go.
    back: usize,
}

impl Scrollbar {
    /// The bar for a terminal that fills `area`, with `back` lines of
    /// scrollback above a screen of `rows`, the view `scroll` lines back.
    /// None when there is nothing to scroll.
    pub fn new(area: Rect, back: usize, rows: usize, scroll: usize) -> Option<Scrollbar> {
        if back == 0 || area.h < 8.0 {
            return None;
        }
        let track = Rect::new(area.x + area.w - WIDTH - 2.0, area.y + 4.0, WIDTH, area.h - 8.0);
        // The thumb is the screen's share of all the lines, but never too small to take hold of.
        let h = (track.h * rows as f32 / (back + rows) as f32).max(24.0_f32.min(track.h));
        // The view's top line counted from the oldest: 0 at the top, `back` at the live screen.
        let y = track.y + (back - scroll.min(back)) as f32 / back as f32 * (track.h - h);
        Some(Scrollbar { track, thumb: Rect::new(track.x, y, WIDTH, h), back })
    }

    /// True for a point at the right edge, where a press is the bar's.
    pub fn holds(&self, x: f32, y: f32) -> bool {
        x >= self.track.x + WIDTH + 2.0 - GRAB && y >= self.track.y - 4.0 && y < self.track.y + self.track.h + 4.0
    }

    /// Where in the thumb a press at `y` takes hold of it: where it was
    /// pressed, or its middle if the press was elsewhere on the track (the
    /// thumb then jumps there).
    pub fn grab(&self, y: f32) -> f32 {
        if y >= self.thumb.y && y < self.thumb.y + self.thumb.h { y - self.thumb.y } else { self.thumb.h / 2.0 }
    }

    /// How many lines back the view is with the thumb's top at `y`.
    pub fn scroll_for(&self, y: f32) -> usize {
        let room = (self.track.h - self.thumb.h).max(1.0);
        let part = ((y - self.track.y) / room).clamp(0.0, 1.0);
        self.back - (part * self.back as f32).round() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0.0, 40.0, 800.0, 408.0);

    #[test]
    fn the_thumb_shows_where_the_view_is() {
        assert_eq!(Scrollbar::new(AREA, 0, 24, 0), None, "nothing to scroll, no bar");
        // 72 lines back and 24 on screen: the thumb is a quarter of the track.
        let live = Scrollbar::new(AREA, 72, 24, 0).unwrap();
        assert_eq!((live.track.x, live.track.y, live.track.h), (793.0, 44.0, 400.0));
        assert_eq!((live.thumb.h, live.thumb.y), (100.0, 344.0), "at the bottom on the live screen");
        let top = Scrollbar::new(AREA, 72, 24, 72).unwrap();
        assert_eq!(top.thumb.y, 44.0);
        let middle = Scrollbar::new(AREA, 72, 24, 36).unwrap();
        assert_eq!(middle.thumb.y, 194.0);
        // A long scrollback: the thumb stays large enough to take hold of.
        assert_eq!(Scrollbar::new(AREA, 10_000, 24, 0).unwrap().thumb.h, 24.0);
        // Scrolled further than there is (the scrollback was just cleared): not past the top.
        assert_eq!(Scrollbar::new(AREA, 72, 24, 500).unwrap().thumb.y, 44.0);
    }

    #[test]
    fn dragging_the_thumb_moves_the_view() {
        let bar = Scrollbar::new(AREA, 72, 24, 0).unwrap();
        assert_eq!(bar.scroll_for(bar.thumb.y), 0, "where it is, the view stays");
        assert_eq!(bar.scroll_for(44.0), 72);
        assert_eq!(bar.scroll_for(194.0), 36);
        assert_eq!((bar.scroll_for(-500.0), bar.scroll_for(5000.0)), (72, 0), "no further than the ends");
        // A press on the thumb holds it there; elsewhere on the track, by its middle.
        assert_eq!(bar.grab(354.0), 10.0);
        assert_eq!(bar.grab(100.0), 50.0);
        assert_eq!(bar.scroll_for(100.0 - bar.grab(100.0)), 71);
        // The right edge is the bar's, the text beside it is not.
        assert!(bar.holds(795.0, 200.0) && bar.holds(789.0, 41.0));
        assert!(!bar.holds(780.0, 200.0) && !bar.holds(795.0, 30.0));
    }
}
