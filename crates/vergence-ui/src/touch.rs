//! Fingers, and the two things a finger most often means.
//!
//! A program gets every finger as it is ([`crate::Input::Touch`]), each
//! with a number that tells it from the others while it is down. Most
//! programs want less: a finger that came down and lifted where it was is
//! a tap, and one that moved is a drag. [`Reader`] reads those two out of
//! the first finger's events; other fingers are left to the program.
//!
//! A finger held still for [`LONG_PRESS`] is a long press: the program
//! asks to be woken that long after the finger came down
//! (`Window::wake_after`) and then asks [`Reader::long_press`]. A drag's
//! end says how fast the finger was going, for a flick's momentum.
//!
//! Not read yet: two fingers together.

use std::time::{Duration, Instant};

use crate::Input;

/// How long a finger is held still to be a long press.
pub const LONG_PRESS: Duration = Duration::from_millis(500);

/// What a finger did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Down,
    Moved,
    Up,
    /// The system took the touch back (a palm, a gesture of its own): what
    /// the finger had begun is to be dropped, not finished.
    Cancelled,
}

/// How far a finger may wander, in reference units, and still have tapped.
pub const SLOP: f32 = 10.0;

/// A tap or a drag, read from the first finger.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    /// The finger lifted where it came down: at that place.
    Tap { x: f32, y: f32 },
    /// The finger has moved: where it is, how far since the last told
    /// (the first of a drag tells all the way from where it came down),
    /// and where it came down.
    Drag { x: f32, y: f32, dx: f32, dy: f32, from: (f32, f32) },
    /// The drag is over (lifted, or taken back), with the finger's speed
    /// as it lifted in units a second (0 if it had stopped, or was taken back).
    DragEnd { vx: f32, vy: f32 },
    /// The finger was held still for [`LONG_PRESS`], here. Its lifting
    /// is then no tap.
    LongPress { x: f32, y: f32 },
}

#[derive(Debug, Clone, Copy)]
struct First {
    id: i32,
    down: (f32, f32),
    last: (f32, f32),
    dragging: bool,
    at: Instant,
    /// When it last moved, and how fast it was going then.
    moved: Instant,
    speed: (f32, f32),
    /// It has been a long press.
    long: bool,
}

/// Reads taps and drags out of touch events. One for a window.
#[derive(Debug, Default)]
pub struct Reader {
    first: Option<First>,
}

impl Reader {
    /// What this input comes to, if it is a touch that completes a tap or
    /// carries a drag on.
    pub fn read(&mut self, input: &Input) -> Option<Gesture> {
        self.read_at(input, Instant::now())
    }

    /// [`Reader::read`], the input having come at `now`.
    pub fn read_at(&mut self, input: &Input, now: Instant) -> Option<Gesture> {
        let Input::Touch { id, phase, x, y } = *input else { return None };
        match (phase, self.first.as_mut()) {
            (Phase::Down, None) => {
                self.first = Some(First { id, down: (x, y), last: (x, y), dragging: false, at: now, moved: now, speed: (0.0, 0.0), long: false });
                None
            }
            (Phase::Moved, Some(first)) if first.id == id => {
                if first.long || !first.dragging && (x - first.down.0).abs() <= SLOP && (y - first.down.1).abs() <= SLOP {
                    return None;
                }
                let (dx, dy) = (x - first.last.0, y - first.last.1);
                let dt = now.saturating_duration_since(first.moved).as_secs_f32().max(0.004);
                // The speed, smoothed over the last few moves.
                first.speed = (0.6 * dx / dt + 0.4 * first.speed.0, 0.6 * dy / dt + 0.4 * first.speed.1);
                first.moved = now;
                first.dragging = true;
                first.last = (x, y);
                Some(Gesture::Drag { x, y, dx, dy, from: first.down })
            }
            (Phase::Up, Some(first)) if first.id == id => {
                let first = *first;
                self.first = None;
                // A finger that stopped before it lifted throws nothing.
                let (vx, vy) = if now.saturating_duration_since(first.moved) < Duration::from_millis(80) { first.speed } else { (0.0, 0.0) };
                match (first.long, first.dragging) {
                    (true, _) => None,
                    (_, true) => Some(Gesture::DragEnd { vx, vy }),
                    _ => Some(Gesture::Tap { x: first.down.0, y: first.down.1 }),
                }
            }
            // Taken back: a drag ends, a tap never was. Every finger goes at once.
            (Phase::Cancelled, Some(first)) => {
                let dragging = first.dragging;
                self.first = None;
                dragging.then_some(Gesture::DragEnd { vx: 0.0, vy: 0.0 })
            }
            _ => None,
        }
    }

    /// A long press, if the first finger has been held still since it came
    /// down for [`LONG_PRESS`] by `now`. Told once.
    pub fn long_press(&mut self, now: Instant) -> Option<Gesture> {
        let first = self.first.as_mut().filter(|f| !f.dragging && !f.long && now.saturating_duration_since(f.at) >= LONG_PRESS)?;
        first.long = true;
        Some(Gesture::LongPress { x: first.down.0, y: first.down.1 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(id: i32, phase: Phase, x: f32, y: f32) -> Input {
        Input::Touch { id, phase, x, y }
    }

    #[test]
    fn a_finger_that_lifts_where_it_came_down_tapped() {
        let mut r = Reader::default();
        assert_eq!(r.read(&touch(0, Phase::Down, 100.0, 50.0)), None);
        // A little wander is still a tap, at the place it came down.
        assert_eq!(r.read(&touch(0, Phase::Moved, 104.0, 47.0)), None);
        assert_eq!(r.read(&touch(0, Phase::Up, 104.0, 47.0)), Some(Gesture::Tap { x: 100.0, y: 50.0 }));
        // And the next finger starts afresh.
        assert_eq!(r.read(&touch(3, Phase::Down, 10.0, 10.0)), None);
        assert_eq!(r.read(&touch(3, Phase::Up, 10.0, 10.0)), Some(Gesture::Tap { x: 10.0, y: 10.0 }));
    }

    #[test]
    fn a_finger_that_moves_drags() {
        let mut r = Reader::default();
        r.read(&touch(0, Phase::Down, 100.0, 100.0));
        assert_eq!(r.read(&touch(0, Phase::Moved, 100.0, 108.0)), None);
        // Past the slop: the whole way from where it came down, then step by step.
        assert_eq!(r.read(&touch(0, Phase::Moved, 100.0, 120.0)), Some(Gesture::Drag { x: 100.0, y: 120.0, dx: 0.0, dy: 20.0, from: (100.0, 100.0) }));
        assert_eq!(r.read(&touch(0, Phase::Moved, 97.0, 150.0)), Some(Gesture::Drag { x: 97.0, y: 150.0, dx: -3.0, dy: 30.0, from: (100.0, 100.0) }));
        // Back near the start it is still a drag, not a tap.
        assert_eq!(r.read(&touch(0, Phase::Moved, 100.0, 101.0)), Some(Gesture::Drag { x: 100.0, y: 101.0, dx: 3.0, dy: -49.0, from: (100.0, 100.0) }));
        // Lifted a while after it stopped: no speed to throw with.
        assert_eq!(r.read_at(&touch(0, Phase::Up, 100.0, 101.0), Instant::now() + Duration::from_secs(1)), Some(Gesture::DragEnd { vx: 0.0, vy: 0.0 }));
    }

    #[test]
    fn only_the_first_finger_is_read_and_a_cancel_drops_it() {
        let mut r = Reader::default();
        r.read(&touch(0, Phase::Down, 100.0, 100.0));
        // A second finger neither taps nor drags, and its lifting ends nothing.
        assert_eq!(r.read(&touch(1, Phase::Down, 300.0, 100.0)), None);
        assert_eq!(r.read(&touch(1, Phase::Moved, 300.0, 200.0)), None);
        assert_eq!(r.read(&touch(1, Phase::Up, 300.0, 200.0)), None);
        // Taken back before it moved: no tap.
        assert_eq!(r.read(&touch(0, Phase::Cancelled, 100.0, 100.0)), None);
        assert_eq!(r.read(&touch(0, Phase::Up, 100.0, 100.0)), None);
        // Taken back in a drag: the drag ends.
        r.read(&touch(0, Phase::Down, 0.0, 0.0));
        r.read(&touch(0, Phase::Moved, 0.0, 40.0));
        assert_eq!(r.read(&touch(0, Phase::Cancelled, 0.0, 40.0)), Some(Gesture::DragEnd { vx: 0.0, vy: 0.0 }));
        // What is not a touch is nothing to it.
        assert_eq!(r.read(&Input::PointerLeft), None);
    }

    #[test]
    fn held_still_is_a_long_press() {
        let t0 = Instant::now();
        let mut r = Reader::default();
        r.read_at(&touch(0, Phase::Down, 50.0, 60.0), t0);
        assert_eq!(r.long_press(t0 + Duration::from_millis(400)), None, "not yet");
        r.read_at(&touch(0, Phase::Moved, 53.0, 58.0), t0 + Duration::from_millis(450));
        assert_eq!(r.long_press(t0 + LONG_PRESS), Some(Gesture::LongPress { x: 50.0, y: 60.0 }));
        assert_eq!(r.long_press(t0 + Duration::from_secs(2)), None, "told once");
        // Its lifting is no tap, and moving after it no drag.
        assert_eq!(r.read_at(&touch(0, Phase::Moved, 90.0, 90.0), t0 + Duration::from_secs(2)), None);
        assert_eq!(r.read_at(&touch(0, Phase::Up, 90.0, 90.0), t0 + Duration::from_secs(2)), None);
        // A finger that moved first is no long press.
        r.read_at(&touch(1, Phase::Down, 0.0, 0.0), t0);
        r.read_at(&touch(1, Phase::Moved, 0.0, 30.0), t0 + Duration::from_millis(100));
        assert_eq!(r.long_press(t0 + Duration::from_secs(1)), None);
    }

    #[test]
    fn a_drag_ends_with_the_finger_s_speed() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut r = Reader::default();
        r.read_at(&touch(0, Phase::Down, 0.0, 300.0), t0);
        for k in 1..=5u64 {
            r.read_at(&touch(0, Phase::Moved, 0.0, 300.0 - 20.0 * k as f32), ms(10 * k));
        }
        // 20 units every 10 ms, upwards: about -2000 a second.
        let Some(Gesture::DragEnd { vx, vy }) = r.read_at(&touch(0, Phase::Up, 0.0, 200.0), ms(60)) else { panic!("no end") };
        assert!(vx == 0.0 && (vy + 2000.0).abs() < 100.0, "{vy}");
        // Stopped, then lifted: no speed.
        r.read_at(&touch(0, Phase::Down, 0.0, 300.0), t0);
        r.read_at(&touch(0, Phase::Moved, 0.0, 200.0), ms(10));
        assert_eq!(r.read_at(&touch(0, Phase::Up, 0.0, 200.0), ms(300)), Some(Gesture::DragEnd { vx: 0.0, vy: 0.0 }));
    }
}
