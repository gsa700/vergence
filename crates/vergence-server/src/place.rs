// Where new windows go (server milestone 1, 2026-11-07).
//
// A new window opens on the monitor under the pointer and fits inside it: no
// larger than the monitor, its title bar (when the server draws one) on screen.
// An app that asks for a position (X11 USPosition/PPosition, e.g. AlbumWall
// restoring where it was) gets it when the window then lies wholly on a monitor;
// otherwise the window is centered. Window placement memory (Contract 2, from
// vergd) comes later and will decide before these defaults.

use crate::win::Win;
use smithay::{
    utils::{Logical, Point, Rectangle, Size},
    xwayland::X11Surface,
};

use crate::Server;
#[cfg(test)]
use crate::decor::TITLE_H;

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
        self.space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o))
            .map(|g| content_area(self.work_area(g), if titled { self.bar_h_in(g) } else { 0 }))
            .collect()
    }

    /// Does `rect` (content) lie wholly on one monitor, below its title bar if titled?
    pub fn fits_a_monitor(&self, rect: Rectangle<i32, Logical>, titled: bool) -> bool {
        self.content_areas(titled).iter().any(|a| a.contains_rect(rect))
    }

    /// Is a window with content `rect` grabbable where it is: its title bar
    /// (for a window without one, its top strip of the same height) wholly on
    /// one monitor? A remembered place is restored then, even with the rest of
    /// the window hanging off the edge (Dave, 2026-11-12: windows go back
    /// exactly where they were put, as long as they can always be grabbed).
    pub fn bar_on_a_monitor(&self, rect: Rectangle<i32, Logical>, titled: bool) -> bool {
        let h = crate::decor::TITLE_H;
        let bar = if titled {
            Rectangle::new((rect.loc.x, rect.loc.y - h).into(), (rect.size.w, h).into())
        } else {
            Rectangle::new(rect.loc, (rect.size.w, h.min(rect.size.h)).into())
        };
        self.space.outputs().filter_map(|o| self.space.output_geometry(o)).any(|g| g.contains_rect(bar))
    }

    /// Place a new X11 window: returns its content rectangle.
    pub fn place_x11(&self, x: &X11Surface) -> Rectangle<i32, Logical> {
        let titled = !x.is_override_redirect() && !x.is_decorated();
        let geo = x.geometry();
        let Some(out) = self.output_at_pointer() else { return geo };
        let area = content_area(self.work_area(out), if titled { self.bar_h_in(out) } else { 0 });
        let size = fit(geo.size, area.size, x.min_size());
        let asked = x.size_hints().is_some_and(|h| h.position.is_some());
        if asked {
            let r = Rectangle::new(geo.loc, size);
            if self.content_areas(titled).iter().any(|a| a.contains_rect(r)) {
                return r;
            }
        }
        Rectangle::new(center(area, size), size)
    }

    /// The most a mapped window may be: the monitor it is mostly on, below its title bar.
    pub fn room_for(&self, w: &Win) -> Option<Size<i32, Logical>> {
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
        Some(content_area(self.work_area(out), if self.has_title(w) { self.bar_h_in(out) } else { 0 }).size)
    }

    /// Fit and center a Wayland window once it has a size (its first commit).
    pub fn place_wayland(&mut self, w: &Win) {
        let Some(t) = w.toplevel() else { return };
        self.placed.retain(|p| smithay::utils::IsAlive::alive(p));
        let size = w.geometry().size;
        if size.w <= 0 || size.h <= 0 || !self.placed.insert(w.clone()) {
            return;
        }
        // Where memory had it on this setup (memory.rs).
        if let Some((r, max)) = self.remembered_place(w) {
            if r.size != size {
                t.with_pending_state(|s| s.size = Some(r.size));
                t.send_pending_configure();
            }
            self.put(w.clone(), r.loc, false);
            if max {
                self.toggle_maximize(w);
            }
            self.window_opened(w);
            return;
        }
        let Some(out) = self.output_at_pointer() else { return };
        let area = content_area(self.work_area(out), if self.has_title(w) { self.bar_h_in(out) } else { 0 });
        let fitted = fit(size, area.size, None);
        if fitted != size {
            t.with_pending_state(|s| s.size = Some(fitted));
            t.send_pending_configure();
        }
        // Not exactly on another window of its program (three of the toolkit's
        // Wayland windows opened at once stacked in the middle, 2026-11-12).
        let r = self.clear_of_siblings(w, Rectangle::new(center(area, fitted), fitted), out);
        self.put(w.clone(), r.loc, false);
        self.window_opened(w);
    }
}

impl Server {
    /// Each window's monitor (by EDID ID) and its place on it, before monitors change.
    pub fn window_anchors(&self) -> Vec<(Win, String, Point<i32, Logical>)> {
        self.space
            .elements()
            .filter_map(|w| {
                let geo = self.space.element_geometry(w)?;
                let center = (geo.loc.x + geo.size.w / 2, geo.loc.y + geo.size.h / 2);
                self.space.outputs().find_map(|o| {
                    let g = self.space.output_geometry(o)?;
                    if !g.contains(center) {
                        return None;
                    }
                    Some((w.clone(), crate::memory::monitor_id_of(o)?, geo.loc - g.loc))
                })
            })
            .collect()
    }

    /// The monitors changed: every window to its remembered place on the new setup;
    /// else, if its monitor is still there, to the same place on it (the layout may
    /// have moved it); else, if it is left on no monitor, onto the nearest one,
    /// fitted. The pointer too, if its monitor went.
    pub fn rearrange_windows(&mut self, anchors: Vec<(Win, String, Point<i32, Logical>)>) {
        let windows: Vec<Win> = self.space.elements().cloned().collect();
        for w in windows {
            if w.x11_surface().is_some_and(|x| x.is_override_redirect()) {
                continue;
            }
            if let Some((r, max)) = self.remembered_place(&w) {
                if self.restore.contains_key(&w) {
                    self.toggle_maximize(&w);
                }
                self.unsnap(&w);
                self.set_rect(&w, r);
                if max {
                    self.toggle_maximize(&w);
                }
                // Snapped on this setup: into its part, fitted to this screen (snap.rs).
                self.snap_recalled(&w);
                tracing::info!("hotplug: window back to its place {:?}", r);
                continue;
            }
            let Some(geo) = self.space.element_geometry(&w) else { continue };
            // Its monitor is still here: the same place on it.
            let kept = anchors.iter().find(|(a, _, _)| *a == w).and_then(|(_, mon, off)| {
                let o =
                    self.space.outputs().find(|o| crate::memory::monitor_id_of(o).as_deref() == Some(mon.as_str()))?;
                self.space.output_geometry(o).map(|g| g.loc + *off)
            });
            if let Some(loc) = kept {
                if loc != geo.loc {
                    self.set_rect(&w, Rectangle::new(loc, geo.size));
                }
                continue;
            }
            let center = (geo.loc.x + geo.size.w / 2, geo.loc.y + geo.size.h / 2);
            let on_one = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).any(|g| g.contains(center));
            if on_one {
                continue;
            }
            // Rescue: nearest monitor (by center distance), fitted, centered.
            let Some(out) = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).min_by_key(|g| {
                let (cx, cy) = (g.loc.x + g.size.w / 2, g.loc.y + g.size.h / 2);
                ((cx - center.0) as i64).pow(2) + ((cy - center.1) as i64).pow(2)
            }) else {
                continue;
            };
            if self.restore.contains_key(&w) {
                self.toggle_maximize(&w);
            }
            if let Some(before) = self.unsnap(&w) {
                self.set_rect(&w, before);
            }
            let size = self.space.element_geometry(&w).map(|g| g.size).unwrap_or(geo.size);
            let area = content_area(self.work_area(out), if self.has_title(&w) { self.bar_h_in(out) } else { 0 });
            let fitted = fit(size, area.size, w.x11_surface().and_then(|x| x.min_size()));
            let r = Rectangle::new(center_of(area, fitted), fitted);
            tracing::info!("hotplug: window rescued onto {:?}", r);
            self.set_rect(&w, r);
        }
        // The pointer, if its monitor went.
        if let Some(p) = self.seat.get_pointer() {
            let at = p.current_location();
            let on =
                self.space.outputs().filter_map(|o| self.space.output_geometry(o)).any(|g| g.to_f64().contains(at));
            let first = self.space.outputs().next().and_then(|o| self.space.output_geometry(o));
            if !on && let Some(g) = first {
                let c = ((g.loc.x + g.size.w / 2) as f64, (g.loc.y + g.size.h / 2) as f64);
                self.pointer_to(c.into(), smithay::backend::input::InputTime::now());
            }
        }
    }

    /// Put a window's content at `r` (X11: configured; Wayland and native:
    /// asked to take the size). Not remembered: the server placed it after a
    /// monitor change, the user did not (a place derived for a new size of
    /// the setup is derived again until the user moves the window there).
    pub fn set_rect(&mut self, w: &Win, r: Rectangle<i32, Logical>) {
        if let Some(x) = w.x11_surface() {
            let _ = x.configure(r);
        } else if let Some(t) = w.toplevel() {
            if w.geometry().size != r.size {
                t.with_pending_state(|s| s.size = Some(r.size));
                t.send_pending_configure();
            }
        } else if let Some(n) = w.native()
            && w.geometry().size != r.size
        {
            self.native_configure(n, r.size, false);
        }
        self.put(w.clone(), r.loc, false);
    }
}

fn center_of(area: Rectangle<i32, Logical>, size: Size<i32, Logical>) -> Point<i32, Logical> {
    center(area, size)
}

/// A monitor's area below a title bar `top` points tall (0: none; the bar's
/// height follows the screen's size of controls).
pub fn content_area(out: Rectangle<i32, Logical>, top: i32) -> Rectangle<i32, Logical> {
    Rectangle::new((out.loc.x, out.loc.y + top).into(), (out.size.w, (out.size.h - top).max(1)).into())
}

/// `size` no larger than `room`, and no smaller than the app's minimum.
pub fn fit(size: Size<i32, Logical>, room: Size<i32, Logical>, min: Option<Size<i32, Logical>>) -> Size<i32, Logical> {
    let min = min.unwrap_or_default();
    (size.w.min(room.w).max(min.w).max(1), size.h.min(room.h).max(min.h).max(1)).into()
}

pub(crate) fn center(area: Rectangle<i32, Logical>, size: Size<i32, Logical>) -> Point<i32, Logical> {
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
        let area = content_area(r(0, 0, 1170, 658), TITLE_H);
        assert_eq!(area, r(0, TITLE_H, 1170, 658 - TITLE_H));
        let size = fit((1131, 710).into(), area.size, None);
        assert_eq!(size, (1131, 658 - TITLE_H).into());
        assert_eq!(center(area, size), (19, TITLE_H).into());
    }

    #[test]
    fn minimum_size_wins_and_untitled_uses_the_whole_monitor() {
        assert_eq!(fit((2000, 300).into(), (1170, 658).into(), Some((1300, 100).into())), (1300, 300).into());
        assert_eq!(content_area(r(1170, 0, 1800, 1013), 0), r(1170, 0, 1800, 1013));
    }
}
