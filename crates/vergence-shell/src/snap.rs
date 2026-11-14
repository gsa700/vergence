//! Snapping (docs/windows-milestone-1.md, step 2): which part of a screen a
//! window dragged to its edge goes to, as plain arithmetic the server applies.
//!
//! The pointer, not the window, decides: within [`EDGE`] points of a screen's
//! left or right edge, that half; of the top edge, maximized (or the top half,
//! by the setting); within [`CORNER`] points of a corner along either edge, that
//! quarter; the bottom edge alone does nothing (the dock is usually there). An
//! edge with another screen beyond it is *shared*: the server snaps there only
//! if the settings say so, and then only after the pointer has stayed.
//!
//! The rectangles are in the screen's work area (the dock's band left out as
//! its own settings say, [`crate::work_area`]), and include the window's bar.

use crate::Rect;

/// How near a screen's edge the pointer must be, in points.
pub const EDGE: i32 = 2;
/// How near a corner, along the edge, makes a quarter instead of a half.
pub const CORNER: i32 = 24;

/// A part of a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Zone {
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// The top edge: maximized, or the top half.
    Top,
}

impl Zone {
    /// As window memory writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Zone::Left => "left",
            Zone::Right => "right",
            Zone::TopLeft => "top-left",
            Zone::TopRight => "top-right",
            Zone::BottomLeft => "bottom-left",
            Zone::BottomRight => "bottom-right",
            Zone::Top => "top",
        }
    }

    pub fn parse(s: &str) -> Option<Zone> {
        [Zone::Left, Zone::Right, Zone::TopLeft, Zone::TopRight, Zone::BottomLeft, Zone::BottomRight, Zone::Top].into_iter().find(|z| z.as_str() == s)
    }
}

/// The zone the pointer at (`x`, `y`) points to on `screen`, and whether the
/// edge it is at is shared with another of `screens` (the screen itself may
/// be among them).
pub fn zone_at(x: f64, y: f64, screen: Rect, screens: &[Rect]) -> Option<(Zone, bool)> {
    let (x, y) = (x.floor() as i32, y.floor() as i32);
    if x < screen.x || y < screen.y || x >= screen.right() || y >= screen.bottom() {
        return None;
    }
    let left = x - screen.x < EDGE;
    let right = screen.right() - 1 - x < EDGE;
    let top = y - screen.y < EDGE;
    let bottom = screen.bottom() - 1 - y < EDGE;
    let near_top = y - screen.y < CORNER;
    let near_bottom = screen.bottom() - 1 - y < CORNER;
    let near_left = x - screen.x < CORNER;
    let near_right = screen.right() - 1 - x < CORNER;
    // Another screen touching this edge where the pointer is.
    let others = || screens.iter().filter(move |o| **o != screen);
    let beyond_left = others().any(|o| o.right() == screen.x && y >= o.y && y < o.bottom());
    let beyond_right = others().any(|o| o.x == screen.right() && y >= o.y && y < o.bottom());
    let beyond_top = others().any(|o| o.bottom() == screen.y && x >= o.x && x < o.right());
    let beyond_bottom = others().any(|o| o.y == screen.bottom() && x >= o.x && x < o.right());
    if left || right {
        let shared = if left { beyond_left } else { beyond_right };
        let zone = match (left, near_top, near_bottom) {
            (true, true, _) => Zone::TopLeft,
            (true, _, true) => Zone::BottomLeft,
            (true, ..) => Zone::Left,
            (false, true, _) => Zone::TopRight,
            (false, _, true) => Zone::BottomRight,
            (false, ..) => Zone::Right,
        };
        return Some((zone, shared));
    }
    if top {
        let zone = if near_left { Zone::TopLeft } else if near_right { Zone::TopRight } else { Zone::Top };
        return Some((zone, beyond_top));
    }
    if bottom && (near_left || near_right) {
        return Some((if near_left { Zone::BottomLeft } else { Zone::BottomRight }, beyond_bottom));
    }
    None
}

/// The rectangle of `zone` in `area` (a screen's work area), and whether the
/// window is then maximized (the top edge, unless `top_half`).
pub fn zone_rect(zone: Zone, area: Rect, top_half: bool) -> (Rect, bool) {
    let (hw, hh) = (area.w / 2, area.h / 2);
    let (rw, bh) = (area.w - hw, area.h - hh);
    let r = match zone {
        Zone::Left => Rect::new(area.x, area.y, hw, area.h),
        Zone::Right => Rect::new(area.x + hw, area.y, rw, area.h),
        Zone::TopLeft => Rect::new(area.x, area.y, hw, hh),
        Zone::TopRight => Rect::new(area.x + hw, area.y, rw, hh),
        Zone::BottomLeft => Rect::new(area.x, area.y + hh, hw, bh),
        Zone::BottomRight => Rect::new(area.x + hw, area.y + hh, rw, bh),
        Zone::Top if top_half => Rect::new(area.x, area.y, area.w, hh),
        Zone::Top => return (area, true),
    };
    (r, false)
}

/// An arrow pressed with Super (the window keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrow {
    Left,
    Right,
    Up,
    Down,
}

/// How a window stands now, for the window keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    Free,
    Maximized,
    Snapped(Zone),
}

/// What a window key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAct {
    /// Into this part of the screen at this place in the list given.
    Snap(Zone, usize),
    Maximize,
    /// Back to where it was before it was maximized or snapped.
    Restore,
    Minimize,
    Nothing,
}

/// The screen next to `screens[at]` in `dir` (left or right): one beside it
/// whose span up and down overlaps its own, the nearest first.
pub fn beside(screens: &[Rect], at: usize, right: bool) -> Option<usize> {
    let s = screens.get(at)?;
    screens
        .iter()
        .enumerate()
        .filter(|(i, o)| *i != at && o.y < s.bottom() && s.y < o.bottom() && if right { o.x >= s.right() - 1 } else { o.right() <= s.x + 1 })
        .min_by_key(|(_, o)| if right { o.x - s.right() } else { s.x - o.right() })
        .map(|(i, _)| i)
}

/// Super with an arrow (docs/windows-milestone-1.md, "Keys for windows"):
///
/// - Left and Right snap to that half; from one half the other way to the
///   other half, and on from there to the far half of the screen beside, so
///   a window walks across every half of every screen. Quarters move along
///   their row the same way.
/// - Up maximizes; from a lower quarter, the upper one.
/// - Down from an upper quarter gives the lower one; from maximized or
///   another part, back to where it was; from nowhere, minimized.
pub fn key_act(standing: Standing, arrow: Arrow, at: usize, screens: &[Rect]) -> KeyAct {
    use Zone::*;
    let (right, horizontal) = match arrow {
        Arrow::Left => (false, true),
        Arrow::Right => (true, true),
        _ => (false, false),
    };
    if horizontal {
        // The part on the near side (where Left or Right goes on this screen),
        // and the one on the far side (where it goes on the screen beside).
        let (near, far) = match (standing, right) {
            (Standing::Snapped(TopLeft | TopRight), true) => (TopRight, TopLeft),
            (Standing::Snapped(TopLeft | TopRight), false) => (TopLeft, TopRight),
            (Standing::Snapped(BottomLeft | BottomRight), true) => (BottomRight, BottomLeft),
            (Standing::Snapped(BottomLeft | BottomRight), false) => (BottomLeft, BottomRight),
            (_, true) => (Right, Left),
            (_, false) => (Left, Right),
        };
        return if standing == Standing::Snapped(near) {
            match beside(screens, at, right) {
                Some(next) => KeyAct::Snap(far, next),
                None => KeyAct::Nothing,
            }
        } else {
            KeyAct::Snap(near, at)
        };
    }
    match (standing, arrow) {
        (Standing::Snapped(BottomLeft), Arrow::Up) => KeyAct::Snap(TopLeft, at),
        (Standing::Snapped(BottomRight), Arrow::Up) => KeyAct::Snap(TopRight, at),
        (Standing::Maximized, Arrow::Up) => KeyAct::Nothing,
        (_, Arrow::Up) => KeyAct::Maximize,
        (Standing::Snapped(TopLeft), Arrow::Down) => KeyAct::Snap(BottomLeft, at),
        (Standing::Snapped(TopRight), Arrow::Down) => KeyAct::Snap(BottomRight, at),
        (Standing::Maximized | Standing::Snapped(_), Arrow::Down) => KeyAct::Restore,
        (Standing::Free, Arrow::Down) => KeyAct::Minimize,
        _ => KeyAct::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Rect = Rect::new(0, 0, 1920, 1080);

    #[test]
    fn the_left_and_right_edges_give_halves() {
        assert_eq!(zone_at(0.0, 500.0, S, &[S]), Some((Zone::Left, false)));
        assert_eq!(zone_at(1919.5, 500.0, S, &[S]), Some((Zone::Right, false)));
        assert_eq!(zone_at(10.0, 500.0, S, &[S]), None, "ten points in is not the edge");
    }

    #[test]
    fn corners_along_either_edge_give_quarters() {
        assert_eq!(zone_at(0.0, 10.0, S, &[S]).unwrap().0, Zone::TopLeft);
        assert_eq!(zone_at(10.0, 0.0, S, &[S]).unwrap().0, Zone::TopLeft);
        assert_eq!(zone_at(1919.0, 1079.0, S, &[S]).unwrap().0, Zone::BottomRight);
        assert_eq!(zone_at(1900.0, 1079.0, S, &[S]).unwrap().0, Zone::BottomRight);
        assert_eq!(zone_at(0.0, 1060.0, S, &[S]).unwrap().0, Zone::BottomLeft);
    }

    #[test]
    fn the_top_edge_maximizes_and_the_bottom_edge_alone_does_nothing() {
        assert_eq!(zone_at(900.0, 0.0, S, &[S]), Some((Zone::Top, false)));
        assert_eq!(zone_at(900.0, 1079.0, S, &[S]), None);
    }

    #[test]
    fn an_edge_with_a_screen_beyond_is_shared_and_an_outer_one_is_not() {
        let r = Rect::new(1920, 0, 1280, 720);
        // The right screen is shorter: past its bottom the left screen's edge is outer.
        assert_eq!(zone_at(1919.0, 500.0, S, &[S, r]), Some((Zone::Right, true)));
        assert_eq!(zone_at(1919.0, 900.0, S, &[S, r]), Some((Zone::Right, false)));
        assert_eq!(zone_at(1920.0, 500.0, r, &[S, r]), Some((Zone::Left, true)));
        assert_eq!(zone_at(3199.0, 500.0, r, &[S, r]), Some((Zone::Right, false)));
        // Stacked: the top screen's bottom edge and the lower one's top edge.
        let below = Rect::new(0, 1080, 1920, 1080);
        assert_eq!(zone_at(900.0, 1080.0, below, &[S, below]), Some((Zone::Top, true)));
        assert_eq!(zone_at(5.0, 1079.0, S, &[S, below]), Some((Zone::BottomLeft, true)));
    }

    #[test]
    fn the_rectangles_fill_the_work_area_without_gaps() {
        // A work area beside a 64-point dock on the left, odd sizes.
        let a = Rect::new(64, 0, 1855, 1079);
        let (l, _) = zone_rect(Zone::Left, a, false);
        let (r, _) = zone_rect(Zone::Right, a, false);
        assert_eq!((l.x, l.right(), r.x, r.right()), (64, 64 + 927, 64 + 927, 64 + 1855));
        let (tl, _) = zone_rect(Zone::TopLeft, a, false);
        let (bl, _) = zone_rect(Zone::BottomLeft, a, false);
        assert_eq!((tl.bottom(), bl.y, bl.bottom()), (539, 539, 1079));
        assert_eq!(zone_rect(Zone::Top, a, false), (a, true));
        assert_eq!(zone_rect(Zone::Top, a, true), (Rect::new(64, 0, 1855, 539), false));
    }

    #[test]
    fn zones_read_back_as_written() {
        for z in [Zone::Left, Zone::Right, Zone::TopLeft, Zone::TopRight, Zone::BottomLeft, Zone::BottomRight, Zone::Top] {
            assert_eq!(Zone::parse(z.as_str()), Some(z));
        }
        assert_eq!(Zone::parse("middle"), None);
    }

    #[test]
    fn a_point_off_the_screen_is_no_zone() {
        assert_eq!(zone_at(-1.0, 5.0, S, &[S]), None);
        assert_eq!(zone_at(1920.0, 5.0, S, &[S]), None);
    }

    #[test]
    fn super_right_walks_a_window_across_every_half_of_every_screen() {
        let screens = [S, Rect::new(1920, 0, 1280, 720)];
        let mut standing = Standing::Free;
        let mut at = 0;
        let mut seen = Vec::new();
        for _ in 0..4 {
            match key_act(standing, Arrow::Right, at, &screens) {
                KeyAct::Snap(z, i) => {
                    standing = Standing::Snapped(z);
                    at = i;
                    seen.push((z, i));
                }
                KeyAct::Nothing => break,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(seen, [(Zone::Right, 0), (Zone::Left, 1), (Zone::Right, 1)], "and no screen beyond the last");
        // Back again with Left.
        assert_eq!(key_act(Standing::Snapped(Zone::Left), Arrow::Left, 1, &screens), KeyAct::Snap(Zone::Right, 0));
        assert_eq!(key_act(Standing::Snapped(Zone::Right), Arrow::Left, 1, &screens), KeyAct::Snap(Zone::Left, 1));
    }

    #[test]
    fn from_a_free_or_maximized_window_left_and_right_give_that_half() {
        assert_eq!(key_act(Standing::Free, Arrow::Left, 0, &[S]), KeyAct::Snap(Zone::Left, 0));
        assert_eq!(key_act(Standing::Maximized, Arrow::Right, 0, &[S]), KeyAct::Snap(Zone::Right, 0));
    }

    #[test]
    fn quarters_move_along_their_row_and_up_and_down_between_rows() {
        assert_eq!(key_act(Standing::Snapped(Zone::TopLeft), Arrow::Right, 0, &[S]), KeyAct::Snap(Zone::TopRight, 0));
        assert_eq!(key_act(Standing::Snapped(Zone::BottomRight), Arrow::Left, 0, &[S]), KeyAct::Snap(Zone::BottomLeft, 0));
        assert_eq!(key_act(Standing::Snapped(Zone::TopLeft), Arrow::Down, 0, &[S]), KeyAct::Snap(Zone::BottomLeft, 0));
        assert_eq!(key_act(Standing::Snapped(Zone::BottomRight), Arrow::Up, 0, &[S]), KeyAct::Snap(Zone::TopRight, 0));
    }

    #[test]
    fn up_maximizes_and_down_restores_or_minimizes() {
        assert_eq!(key_act(Standing::Free, Arrow::Up, 0, &[S]), KeyAct::Maximize);
        assert_eq!(key_act(Standing::Snapped(Zone::Left), Arrow::Up, 0, &[S]), KeyAct::Maximize);
        assert_eq!(key_act(Standing::Maximized, Arrow::Up, 0, &[S]), KeyAct::Nothing);
        assert_eq!(key_act(Standing::Maximized, Arrow::Down, 0, &[S]), KeyAct::Restore);
        assert_eq!(key_act(Standing::Snapped(Zone::Right), Arrow::Down, 0, &[S]), KeyAct::Restore);
        assert_eq!(key_act(Standing::Free, Arrow::Down, 0, &[S]), KeyAct::Minimize);
    }

    #[test]
    fn the_screen_beside_is_one_that_overlaps_up_and_down() {
        let above = Rect::new(1920, -1080, 1920, 1080);
        let right = Rect::new(1920, 300, 1280, 720);
        let left = Rect::new(-1280, 0, 1280, 1024);
        let screens = [S, above, right, left];
        assert_eq!(beside(&screens, 0, true), Some(2), "not the one diagonally above");
        assert_eq!(beside(&screens, 0, false), Some(3));
        assert_eq!(beside(&screens, 3, false), None);
    }
}
