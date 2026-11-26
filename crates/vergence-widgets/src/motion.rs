//! A program's own motion ([`crate::Program::animating`]): a change that
//! takes time (an album unfolding, a cover turning over, a fade), worked
//! out from the clock each frame. With reduced motion (the desktop's, or
//! the program's own setting) every change is instant.

use std::time::{Duration, Instant};

/// One change from 0 to 1 over a time, eased.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tween {
    pub start: Instant,
    pub duration: Duration,
}

impl Tween {
    /// A change starting now, taking `ms` milliseconds (none with `reduced`).
    pub fn new(ms: u64, reduced: bool) -> Tween {
        Tween { start: Instant::now(), duration: Duration::from_millis(if reduced { 0 } else { ms }) }
    }

    /// How far it has come at `now`, 0 to 1, not eased.
    pub fn linear(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        (now.saturating_duration_since(self.start).as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// How far it has come, eased out (fast at first, settling at the end).
    pub fn out(&self, now: Instant) -> f32 {
        ease_out(self.linear(now))
    }

    /// Eased in and out (a turn that starts and ends gently).
    pub fn in_out(&self, now: Instant) -> f32 {
        ease_in_out(self.linear(now))
    }

    pub fn running(&self, now: Instant) -> bool {
        self.linear(now) < 1.0
    }
}

/// Cubic ease-out.
pub fn ease_out(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// Cubic ease-in-out.
pub fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// `a` to `b` by `t`.
pub fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tween_runs_its_time_and_reduced_motion_is_instant() {
        let t = Tween::new(330, false);
        assert!(t.running(t.start));
        assert_eq!(t.linear(t.start), 0.0);
        assert_eq!(t.out(t.start + Duration::from_millis(330)), 1.0);
        assert!(!t.running(t.start + Duration::from_millis(400)));
        let r = Tween::new(330, true);
        assert_eq!(r.out(r.start), 1.0);
        assert!(!r.running(r.start));
    }

    #[test]
    fn easing_starts_at_zero_and_ends_at_one() {
        for f in [ease_out, ease_in_out] {
            assert_eq!(f(0.0), 0.0);
            assert_eq!(f(1.0), 1.0);
            assert!(f(0.5) > 0.0 && f(0.5) < 1.0);
        }
        assert!(ease_out(0.25) > 0.25, "out moves fast first");
        assert_eq!(mix(10.0, 20.0, 0.5), 15.0);
    }
}
