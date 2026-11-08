// Where new windows go (server milestone 1, 2026-11-07).
//
// A new window opens on the monitor under the pointer and fits inside it: no
// larger than the monitor, its title bar (when the server draws one) on screen.
// An app that asks for a position (X11 USPosition/PPosition, e.g. AlbumWall
// restoring where it was) gets it when the window then lies wholly on a monitor;
// otherwise the window is centred. Window placement memory (Contract 2, from
// vergd) comes later and will decide before these defaults.

use smithay::{
    desktop::Window,
    utils::{Logical, Point, Rectangle, Size},
    xwayland::X11Surface,
};

use crate::{Server, decor::TITLE_H};

impl Server {
    /// The monitor the pointer is on (else the first), in logical space.
    pub fn output_at_pointer(&self) -> Option<Rectangle<i32, Logical>> {
        let at = self.seat.get_pointer()?.current_location();
        let mut first = None;
        for o in self.space.outputs() {
            let g = self.space.output_geometry(o)?;
            if g.to_f64().contains(at) {
                return Some(g);
            }
            first.get_or_insert(g);
        }
        first
    }

    /// Where content can go on every monitor (below a title bar if `titled`).
    fn content_areas(&self, titled: bool) -> Vec<Rectangle<i32, Logical>> {
        self.space.outputs().filter_map(|o| self.space.output_geometry(o)).map(|g| content_area(g, titled)).collect()
    }

    /// Does `rect` (content) lie wholly on one monitor, below its title bar if titled?
    pub fn fits_a_monitor(&self, rect: Rectangle<i32, Logical>, titled: bool) -> bool {
        self.content_areas(titled).iter().any(|a| a.contains_rect(rect))
    }

    /// Place a new X11 window: returns its content rectangle.
    pub fn place_x11(&self, x: &X11Surface) -> Rectangle<i32, Logical> {
        let titled = !x.is_override_redirect() && !x.is_decorated();
        let geo = x.geometry();
        let Some(out) = self.output_at_pointer() else { return geo };
        let area = content_area(out, titled);
        let size = fit(geo.size, area.size, x.min_size());
        let asked = x.size_hints().is_some_and(|h| h.position.is_some());
        if asked {
            let r = Rectangle::new(geo.loc, size);
            if self.content_areas(titled).iter().any(|a| a.contains_rect(r)) {
                return r;
            }
        }
        Rectangle::new(centre(area, size), size)
    }

    /// The most a mapped window may be: the monitor it is mostly on, below its title bar.
    pub fn room_for(&self, w: &Window) -> Option<Size<i32, Logical>> {
        let geo = self.space.element_geometry(w)?;
        let out = self
            .space
            .outputs()
            .filter_map(|o| {
                let g = self.space.output_geometry(o)?;
                let i = g.intersection(geo)?;
                Some((i.size.w as i64 * i.size.h as i64, g))
            })
            .max_by_key(|(a, _)| *a)
            .map(|(_, g)| g)?;
        Some(content_area(out, self.has_title(w)).size)
    }

    /// Fit and centre a Wayland window once it has a size (its first commit).
    pub fn place_wayland(&mut self, w: &Window) {
        let Some(t) = w.toplevel() else { return };
        self.placed.retain(|p| smithay::utils::IsAlive::alive(p));
        let size = w.geometry().size;
        if size.w <= 0 || size.h <= 0 || !self.placed.insert(w.clone()) {
            return;
        }
        let Some(out) = self.output_at_pointer() else { return };
        let area = content_area(out, self.has_title(w));
        let fitted = fit(size, area.size, None);
        if fitted != size {
            t.with_pending_state(|s| s.size = Some(fitted));
            t.send_pending_configure();
        }
        self.space.map_element(w.clone(), centre(area, fitted), false);
    }
}

/// A monitor's area below a title bar's height (when titled).
pub fn content_area(out: Rectangle<i32, Logical>, titled: bool) -> Rectangle<i32, Logical> {
    let top = if titled { TITLE_H } else { 0 };
    Rectangle::new((out.loc.x, out.loc.y + top).into(), (out.size.w, (out.size.h - top).max(1)).into())
}

/// `size` no larger than `room`, and no smaller than the app's minimum.
pub fn fit(size: Size<i32, Logical>, room: Size<i32, Logical>, min: Option<Size<i32, Logical>>) -> Size<i32, Logical> {
    let min = min.unwrap_or_default();
    (size.w.min(room.w).max(min.w).max(1), size.h.min(room.h).max(min.h).max(1)).into()
}

fn centre(area: Rectangle<i32, Logical>, size: Size<i32, Logical>) -> Point<i32, Logical> {
    (area.loc.x + ((area.size.w - size.w) / 2).max(0), area.loc.y + ((area.size.h - size.h) / 2).max(0)).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn too_tall_window_fits_below_the_title_bar() {
        // AlbumWall's 1131 x 710 on the 14" (1170 x 658 points), with a title bar.
        let area = content_area(r(0, 0, 1170, 658), true);
        assert_eq!(area, r(0, 24, 1170, 634));
        let size = fit((1131, 710).into(), area.size, None);
        assert_eq!(size, (1131, 634).into());
        assert_eq!(centre(area, size), (19, 24).into());
    }

    #[test]
    fn minimum_size_wins_and_untitled_uses_the_whole_monitor() {
        assert_eq!(fit((2000, 300).into(), (1170, 658).into(), Some((1300, 100).into())), (1300, 300).into());
        assert_eq!(content_area(r(1170, 0, 1800, 1013), false), r(1170, 0, 1800, 1013));
    }
}
