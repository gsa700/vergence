// The desktop moving aside (overview milestone 1, step 6;
// docs/overview-milestone-1.md). Dave, 2026-11-18: "Click on the desktop makes
// the windows move aside as on the macOS. This is a cool feature."; on unless
// switched off (his 2a: desktop.toml `[windows] desktop-aside = false`).
//
// A click on the desktop's bare ground (the desktop window says so, bus v23
// `GroundClicked`; with no icons, a press on the bare wallpaper) slides every
// window someone works in (desktop.rs ordinary_window: Firefox and other
// windows that draw their own bar too) off toward its screen's nearer side, left or right, a
// strip of it still showing. Another click on the ground, or a press on any
// window's strip, slides them all back to where they were. A short slide;
// none with Reduce motion. Super+D (minimize all) is a separate thing.
//
// The places are not remembered: window memory keeps where they were.

use std::time::{Duration, Instant};

use smithay::{
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::{Logical, Point},
};

use crate::{Server, win::Win};

/// How much of a window stays on its screen, in points.
const STRIP: i32 = 48;
/// The slide's length and its step.
const SLIDE: Duration = Duration::from_millis(220);
const STEP: Duration = Duration::from_millis(16);

/// The windows moved aside, each with where it was and where it went.
#[derive(Default)]
pub struct Aside {
    pub moved: Vec<(Win, Point<i32, Logical>, Point<i32, Logical>)>,
    /// The slide under way: when it began, and whether it is going out.
    sliding: Option<(Instant, bool)>,
}

impl Aside {
    pub fn active(&self) -> bool {
        !self.moved.is_empty()
    }
}

/// Eased: quick to start, slow to stop.
fn ease(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

impl Server {
    /// The desktop's bare ground was clicked: aside, or back.
    pub fn ground_clicked(&mut self) {
        if self.aside.active() {
            return self.aside_back();
        }
        if !self.snap.config.desktop_aside {
            return;
        }
        let screens: Vec<smithay::utils::Rectangle<i32, Logical>> = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).collect();
        let windows: Vec<Win> = self.space.elements().filter(|w| self.ordinary_window(w) && !self.is_minimized(w)).cloned().collect();
        let mut moved = Vec::new();
        // Windows near the middle (a twentieth of the screen either way) go
        // to the sides by turns, so they spread instead of piling on one.
        let mut turn = false;
        for w in windows {
            let Some(g) = self.space.element_geometry(&w) else { continue };
            let mid = (g.loc.x + g.size.w / 2, g.loc.y + g.size.h / 2);
            let Some(screen) = screens.iter().find(|s| s.contains(mid)).or(screens.first()).copied() else { continue };
            let center = screen.loc.x + screen.size.w / 2;
            let left = if (mid.0 - center).abs() < screen.size.w / 20 {
                turn = !turn;
                turn
            } else {
                mid.0 < center
            };
            let x = if left { screen.loc.x + STRIP - g.size.w } else { screen.loc.x + screen.size.w - STRIP };
            let from = self.space.element_location(&w).unwrap_or(g.loc);
            // The element's place is its geometry's less any offset (its shadow).
            let to = Point::from((from.x + (x - g.loc.x), from.y));
            moved.push((w, from, to));
        }
        if moved.is_empty() {
            return;
        }
        tracing::info!("aside: {} windows", moved.len());
        self.aside.moved = moved;
        self.aside_slide(true);
    }

    /// Every window moved aside back where it was.
    pub fn aside_back(&mut self) {
        if !self.aside.active() {
            return;
        }
        tracing::info!("aside: back");
        self.aside_slide(false);
    }

    /// A press on a window while they are aside: all come back.
    pub fn aside_pressed(&mut self, w: &Win) {
        if self.aside.active() && !self.is_desktop(w) {
            self.aside_back();
        }
    }

    fn aside_slide(&mut self, out: bool) {
        if !self.motion_wanted() {
            self.aside.sliding = None;
            return self.aside_step_to(if out { 1.0 } else { 0.0 }, out);
        }
        let first = self.aside.sliding.is_none();
        self.aside.sliding = Some((Instant::now(), out));
        if first {
            let _ = self.loop_handle.insert_source(Timer::from_duration(STEP), |_, _, st| {
                let Some((since, out)) = st.aside.sliding else { return TimeoutAction::Drop };
                let t = since.elapsed().as_secs_f64() / SLIDE.as_secs_f64();
                let k = ease(t);
                st.aside_step_to(if out { k } else { 1.0 - k }, out);
                if t >= 1.0 {
                    st.aside.sliding = None;
                    return TimeoutAction::Drop;
                }
                TimeoutAction::ToDuration(STEP)
            });
        }
    }

    /// Each window `k` of the way out (0 where it was, 1 aside); at 0 on the
    /// way back, the list is let go.
    fn aside_step_to(&mut self, k: f64, out: bool) {
        let moved: Vec<(Win, Point<i32, Logical>, Point<i32, Logical>)> = self.aside.moved.iter().filter(|(w, ..)| smithay::utils::IsAlive::alive(w)).cloned().collect();
        for (w, from, to) in &moved {
            let at = Point::from(((from.x as f64 + (to.x - from.x) as f64 * k).round() as i32, (from.y as f64 + (to.y - from.y) as f64 * k).round() as i32));
            // Moved where it is in the stack, not raised.
            self.space.relocate_element(w, at);
        }
        if !out && k <= 0.0 {
            self.aside.moved.clear();
        }
        self.redraw_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slide_starts_quick_and_ends_where_it_should() {
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        assert_eq!(ease(2.0), 1.0);
        assert!(ease(0.25) > 0.5, "most of the way in the first quarter");
    }
}
