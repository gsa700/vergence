// Native windows on whole pixels at a fractional scale (2026-11-18).
//
// At a scale p/q in lowest terms (133⅓% = 4/3), q points are a whole number
// of pixels (3 points = 4 pixels), and nothing shorter is. A native window
// whose place and size are whole multiples of q points, counted from its
// screen's corner, starts and ends on a pixel: its body, its bar and the
// frost under the bar all round the same way, wherever it is dragged. Off
// that grid each edge rounds on its own, and the bar's outline showed or was
// cut off a pixel at a time as the window moved (techbench, 133⅓%).
//
// Only scales with a short step get the grid: the slider's clean stops
// (4/3, 5/4, 6/5, 3/2, 15/8 ...). A scale such as 1.4417 (173/120) would
// need 120-point steps; those windows go where they are put, as before.

use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::state::Server;
use crate::win::Win;

/// The longest step the grid uses, in points.
pub const LONGEST: i32 = 8;

/// The grid step for a scale, in points: q for a scale p/q (scales are kept
/// in 120ths), 1 for a whole scale or one whose step would be too long.
pub fn step_of(scale: f64) -> i32 {
    if !(scale.is_finite() && scale > 0.0) {
        return 1;
    }
    let p = (scale * 120.0).round() as i64;
    if p <= 0 {
        return 1;
    }
    let q = (120 / gcd(p, 120)) as i32;
    if q <= LONGEST { q } else { 1 }
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a.abs() } else { gcd(b, a % b) }
}

/// A length on the grid: the nearest multiple of `step`, at least one step.
pub fn snap_len(v: i32, step: i32) -> i32 {
    if step <= 1 {
        return v;
    }
    (((v as f64) / step as f64).round() as i32).max(1) * step
}

/// A coordinate on the grid that starts at `origin` (the screen's corner).
pub fn snap_at(v: i32, origin: i32, step: i32) -> i32 {
    if step <= 1 {
        return v;
    }
    origin + ((v - origin) as f64 / step as f64).round() as i32 * step
}

impl Server {
    /// The grid of the screen a rectangle is mostly on (the first screen
    /// when it is on none): its step and its corner.
    pub fn grid_for(&self, rect: Rectangle<i32, Logical>) -> (i32, Point<i32, Logical>) {
        let on = self
            .space
            .outputs()
            .filter_map(|o| {
                let g = self.space.output_geometry(o)?;
                let i = g.intersection(rect)?;
                Some((i.size.w as i64 * i.size.h as i64, o.clone(), g))
            })
            .max_by_key(|(a, ..)| *a)
            .map(|(_, o, g)| (o, g))
            .or_else(|| {
                let o = self.space.outputs().next()?.clone();
                let g = self.space.output_geometry(&o)?;
                Some((o, g))
            });
        match on {
            Some((o, g)) => (step_of(o.current_scale().fractional_scale()), g.loc),
            None => (1, Point::from((0, 0))),
        }
    }

    /// The grid at a point (bar heights, decor.rs).
    pub fn grid_step_at(&self, p: Point<i32, Logical>) -> i32 {
        self.grid_for(Rectangle::new(p, (1, 1).into())).0
    }

    /// Does this window keep to the grid? Native windows do, but not the
    /// desktop's icons (they fill the work area) or a full-screen window
    /// (it fills its screen).
    pub(crate) fn on_grid(&self, w: &Win) -> bool {
        w.native().is_some() && !self.is_desktop(w) && !self.fullscreen.contains_key(w)
    }

    /// Put a window at `loc`: Space::map_element, with a native window's
    /// corner moved to the nearest point of its screen's grid.
    pub fn put(&mut self, w: Win, loc: impl Into<Point<i32, Logical>>, activate: bool) {
        let loc = self.grid_loc(&w, loc.into());
        self.space.map_element(w, loc, activate);
    }

    /// Where `put` would put a window asked to be at `loc`.
    pub fn grid_loc(&self, w: &Win, loc: Point<i32, Logical>) -> Point<i32, Logical> {
        if !self.on_grid(w) {
            return loc;
        }
        let size = self.space.element_geometry(w).map_or(Size::from((1, 1)), |g| g.size);
        let (step, origin) = self.grid_for(Rectangle::new(loc, size));
        (snap_at(loc.x, origin.x, step), snap_at(loc.y, origin.y, step)).into()
    }

    /// A native window's size on the grid of the screen it is on (it is
    /// told at `scale`, the scale of that screen).
    pub fn grid_size(size: Size<i32, Logical>, scale: f64) -> Size<i32, Logical> {
        let step = step_of(scale);
        (snap_len(size.w, step), snap_len(size.h, step)).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_of_clean_scales() {
        assert_eq!(step_of(1.0), 1);
        assert_eq!(step_of(2.0), 1);
        assert_eq!(step_of(4.0 / 3.0), 3);
        assert_eq!(step_of(1.25), 4);
        assert_eq!(step_of(1.5), 2);
        assert_eq!(step_of(1.2), 5);
        assert_eq!(step_of(15.0 / 8.0), 8);
        assert_eq!(step_of(0.75), 4);
        // 173/120: no useful grid.
        assert_eq!(step_of(173.0 / 120.0), 1);
        assert_eq!(step_of(0.0), 1);
        assert_eq!(step_of(f64::NAN), 1);
    }

    #[test]
    fn grid_lengths_and_places_are_whole_pixels() {
        for scale in [4.0 / 3.0, 1.25, 1.5, 1.2, 15.0 / 8.0, 0.75] {
            let step = step_of(scale);
            for v in [1, 28, 580, 773, 1441] {
                let px = snap_len(v, step) as f64 * scale;
                assert!((px - px.round()).abs() < 1e-9, "{v} pt at {scale}: {px} px");
                assert!(v < step || (snap_len(v, step) - v).abs() <= step / 2, "{v} moved too far");
            }
            for (v, origin) in [(0, 0), (-7, 0), (1001, 2880), (5000, 2880), (-1450, -2880)] {
                let x = snap_at(v, origin, step);
                let px = (x - origin) as f64 * scale;
                assert!((px - px.round()).abs() < 1e-9, "{v} from {origin} at {scale}: {px} px");
                assert!((x - v).abs() <= step / 2 + 1);
            }
        }
    }

    #[test]
    fn whole_scales_change_nothing() {
        assert_eq!(snap_len(581, 1), 581);
        assert_eq!(snap_at(13, 7, 1), 13);
        assert_eq!(snap_len(1, 3), 3);
    }
}
