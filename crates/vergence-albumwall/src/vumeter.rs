//! One analog VU meter, drawn and moved like the real thing (the Avalonia
//! AlbumWall's VuMeter.cs, Dave's pick from the front-panel mockups,
//! 2026-09-28: the pair of analog meters with "a vintage yellow(ed) look",
//! no L and R).
//!
//! - **The face** is the mockup's: a cream-to-amber glow, dark brown ink, a
//!   brick-red zone past 0, a black needle, a dark lip over the pivot,
//!   drawn at the mockup's 190 x 124 and scaled to where it is laid out.
//! - **A VU scale is printed in volts.** The needle's deflection follows the
//!   signal's voltage, so the dB marks crowd toward the top: +3 dB is full
//!   scale, 0 VU sits at 71% of the arc (1 / 1.413), -20 at 7%.
//! - **Its needle has weight.** A VU meter reaches 99% of a step in 300 ms
//!   with a little over 1% overshoot: a spring and a damper chasing the
//!   level (13.5 rad/s, damping 0.77), stepped in small steps.
//! - **0 VU at -10 dBFS of RMS**: measured on Dave's library, average levels
//!   run -17 to -10 dBFS with bursts to -5, so the loud records swing around
//!   0 and the quiet ones sit lower, as a meter should show them.
//! - **The battery** (CONTRIBUTING.md): a needle at rest asks for nothing;
//!   the window steps it at 30 Hz only while it moves.

use vergence_scene::{Color, Point, Rect, Scene};
use vergence_text::Fonts;
use vergence_widgets::{Element, Length, drawing};

use crate::Msg;

const FACE_W: f32 = 190.0;
const FACE_H: f32 = 124.0;
const PIVOT: (f32, f32) = (95.0, 132.0);
const ARC_R: f32 = 98.0;
const NEEDLE_R: f32 = 112.0;
/// Where 0 VU sits, in dBFS of RMS: one number to tune by ear.
pub const ZERO_VU_DBFS: f64 = -10.0;
/// +3 dB, as a voltage ratio: full scale.
const FULL_SCALE: f64 = 1.4125;
const OMEGA: f64 = 13.5;
const DAMPING: f64 = 0.77;

/// One needle: where it is and how fast it moves, chasing its target (0 to
/// about 1.05 of the arc).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Needle {
    pub x: f64,
    v: f64,
    target: f64,
    resting: bool,
}

impl Default for Needle {
    fn default() -> Needle {
        Needle { x: 0.0, v: 0.0, target: 0.0, resting: true }
    }
}

impl Needle {
    /// The level to chase, in dBFS of RMS (negative infinity: silence,
    /// paused, nothing playing: the needle falls back to its stop).
    pub fn level(&mut self, dbfs: f64) {
        let volts = if dbfs.is_finite() { 10f64.powf((dbfs - ZERO_VU_DBFS) / 20.0) } else { 0.0 };
        // The needle hits its pin a little past +3 and goes no further.
        let target = (volts / FULL_SCALE).min(1.05);
        if self.resting && target == self.target {
            return;
        }
        self.target = target;
        self.resting = false;
    }

    /// Moved on by `seconds`: semi-implicit Euler in small steps, so a slow
    /// frame cannot throw the needle past where a real one could go.
    pub fn step(&mut self, seconds: f64) {
        if self.resting {
            return;
        }
        let dt = seconds.clamp(0.001, 0.05);
        let steps = (dt / 0.005).ceil() as usize;
        let h = dt / steps as f64;
        for _ in 0..steps {
            let a = OMEGA * OMEGA * (self.target - self.x) - 2.0 * DAMPING * OMEGA * self.v;
            self.v += a * h;
            self.x += self.v * h;
            if self.x < -0.02 {
                (self.x, self.v) = (-0.02, 0.0); // the rest pin
            }
            if self.x > 1.05 {
                (self.x, self.v) = (1.05, 0.0); // the end pin
            }
        }
        // At rest and staying there: nothing more to draw until the level moves.
        if (self.x - self.target).abs() < 0.0005 && self.v.abs() < 0.0005 {
            self.resting = true;
        }
    }

    pub fn resting(&self) -> bool {
        self.resting
    }
}

/// Where a level in dB sits on the arc (by voltage), and that place's angle.
fn position(db: f64) -> f32 {
    (10f64.powf(db / 20.0) / FULL_SCALE) as f32
}

fn at(pos: f32, r: f32) -> (f32, f32) {
    let a = (-48.0 + 96.0 * pos).to_radians();
    (PIVOT.0 + r * a.sin(), PIVOT.1 - r * a.cos())
}

/// The dB marks: which get a longer tick and a number.
const MARKS: [(f64, bool, Option<&str>); 11] = [
    (-20.0, true, Some("20")),
    (-10.0, true, Some("10")),
    (-7.0, true, Some("7")),
    (-5.0, true, Some("5")),
    (-3.0, true, Some("3")),
    (-2.0, false, None),
    (-1.0, false, None),
    (0.0, true, Some("0")),
    (1.0, false, None),
    (2.0, false, None),
    (3.0, true, Some("+3")),
];

const INK: Color = Color::rgb(0x2A, 0x23, 0x18);
const RED: Color = Color::rgb(0xB3, 0x36, 0x2B);
const RIM: Color = Color::rgb(0x5A, 0x4A, 0x2C);
const NEEDLE: Color = Color::rgb(0x15, 0x12, 0x0D);
const LIP: Color = Color::rgb(0x2B, 0x24, 0x18);
const VU_INK: Color = Color::rgb(0x4A, 0x3E, 0x28);

/// The face's glow: cream at its center low on the face to amber at its edge.
fn glow(t: f32) -> Color {
    let stops = [(0.0, (0xF2, 0xDF, 0xA6)), (0.6, (0xE2, 0xC4, 0x7E)), (1.0, (0xC9, 0xA4, 0x5C))];
    let t = t.clamp(0.0, 1.0);
    let (mut a, mut b) = (stops[0], stops[1]);
    if t > 0.6 {
        (a, b) = (stops[1], stops[2]);
    }
    let u = (t - a.0) / (b.0 - a.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * u).round() as u8;
    Color::rgb(m(a.1.0, b.1.0), m(a.1.1, b.1.1), m(a.1.2, b.1.2))
}

/// Draws one meter with its needle at `x`, scaled into `r`.
fn draw(scene: &mut Scene, fonts: &mut Fonts, r: Rect, x: f32, mono: &str, sans: &str) {
    let k = (r.w / FACE_W).min(r.h / FACE_H);
    let (ox, oy) = (r.x + (r.w - FACE_W * k) / 2.0, r.y + (r.h - FACE_H * k) / 2.0);
    let p = |(fx, fy): (f32, f32)| Point::new(ox + fx * k, oy + fy * k);
    let face = Rect::new(ox, oy, FACE_W * k, FACE_H * k);
    scene.push_clip(face, 10.0 * k);
    // The glow, as rings about a center low on the face.
    scene.fill_rounded(face, 10.0 * k, glow(1.0));
    let (cx, cy) = (ox + FACE_W * k * 0.5, oy + FACE_H * k * 0.78);
    let reach = FACE_W * k * 0.75;
    for i in (0..24).rev() {
        let t = i as f32 / 24.0;
        let rr = reach * t.max(0.02);
        scene.fill_rounded(Rect::new(cx - rr, cy - rr, rr * 2.0, rr * 2.0), rr, glow(t));
    }
    // The scale: its arc to 0, the red zone past it, the marks and numbers.
    let arc = |scene: &mut Scene, from: f32, to: f32, radius: f32, width: f32, c: Color| {
        let n = 40;
        for s in 0..n {
            let (a, b) = (from + (to - from) * s as f32 / n as f32, from + (to - from) * (s + 1) as f32 / n as f32);
            scene.line(p(at(a, radius)), p(at(b, radius)), width * k, c);
        }
    };
    arc(scene, position(-20.0), position(0.0), ARC_R, 1.5, INK);
    arc(scene, position(0.0), 1.0, ARC_R + 3.0, 5.0, RED);
    let numbers = fonts.face(mono, 500, false).or_else(|| fonts.face(sans, 500, false));
    let caption = fonts.face(sans, 600, false);
    for (db, major, label) in MARKS {
        let pos = position(db);
        let c = if db > 0.0 { RED } else { INK };
        scene.line(p(at(pos, ARC_R)), p(at(pos, ARC_R + if major { 11.0 } else { 7.0 })), 1.5 * k, c);
        if let (Some(text), Some(face)) = (label, numbers) {
            let size = 11.0 * k;
            let w =
                vergence_render::text(scene, fonts, face, size, Color::rgba(0, 0, 0, 0), Point::new(-1e5, -1e5), text);
            let (lx, ly) = at(pos, ARC_R + 20.0);
            // Kept inside the face: at the arc's ends a centered number would hang off it.
            let x0 = (ox + lx * k - w / 2.0).clamp(ox + 4.0 * k, ox + (FACE_W - 4.0) * k - w);
            vergence_render::text(scene, fonts, face, size, c, Point::new(x0, oy + ly * k + size * 0.35), text);
        }
    }
    if let Some(face) = caption {
        let size = 16.0 * k;
        let w = vergence_render::text(scene, fonts, face, size, Color::rgba(0, 0, 0, 0), Point::new(-1e5, -1e5), "VU");
        vergence_render::text(
            scene,
            fonts,
            face,
            size,
            VU_INK,
            Point::new(p(PIVOT).x - w / 2.0, oy + 84.0 * k + size * 0.8),
            "VU",
        );
    }
    // The needle, then the lip over its pivot (drawn last, over the needle's foot).
    scene.line(p(PIVOT), p(at(x, NEEDLE_R)), 2.0 * k, NEEDLE);
    scene.fill(Rect::new(ox + 0.5 * k, oy + 106.0 * k, (FACE_W - 1.0) * k, (FACE_H - 106.0) * k), LIP);
    scene.pop_clip();
    scene.stroke(face, 10.0 * k, 1.0, RIM);
}

/// The pair of meters, `w` x `h`, needles at `x`.
pub fn pair(x: [f32; 2], w: f32, h: f32) -> Element<Msg> {
    drawing(move |scene, fonts, r: Rect, theme| {
        let gap = 8.0;
        let each = (r.w - gap) / 2.0;
        draw(scene, fonts, Rect::new(r.x, r.y, each, r.h), x[0], &theme.mono, &theme.font);
        draw(scene, fonts, Rect::new(r.x + each + gap, r.y, each, r.h), x[1], &theme.mono, &theme.font);
    })
    .width(Length::Fixed(w))
    .height(Length::Fixed(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_vu_sits_at_71_percent_and_the_marks_crowd_up() {
        assert!((position(0.0) - 0.708).abs() < 0.001);
        assert!((position(3.0) - 1.0).abs() < 0.001);
        assert!((position(-20.0) - 0.0708).abs() < 0.001);
        assert!(position(-3.0) - position(-5.0) > position(-10.0) - position(-20.0) / 2.0 - 0.1);
    }

    #[test]
    fn the_needle_has_a_vu_meters_weight() {
        let mut n = Needle::default();
        n.level(ZERO_VU_DBFS); // 0 VU
        let target = position(0.0) as f64;
        let mut t: f64 = 0.0;
        let mut over: f64 = 0.0;
        let mut at_300 = 0.0;
        while t < 1.5 {
            n.step(1.0 / 120.0);
            t += 1.0 / 120.0;
            over = over.max(n.x / target - 1.0);
            if (t - 0.3).abs() < 1.0 / 240.0 {
                at_300 = n.x / target;
            }
        }
        assert!(at_300 > 0.97, "99% of the step in about 300 ms: {at_300}");
        assert!(over > 0.0 && over < 0.03, "a little overshoot: {over}");
        assert!(n.resting(), "and then it rests");
        // Silence: back to its stop, and resting there.
        n.level(f64::NEG_INFINITY);
        for _ in 0..200 {
            n.step(1.0 / 60.0);
        }
        assert!(n.x.abs() < 0.001 && n.resting(), "at its stop: {}", n.x);
    }
}
