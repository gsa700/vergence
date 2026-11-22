//! A chart of the last samples of one or two readings (tools milestone 1:
//! the Hardware app's load, memory and traffic; first the settings app's
//! Wi-Fi traffic): the newest at the right, the first series in the accent,
//! the second a fainter accent, a baseline, a scale from zero to `top`
//! (None: the largest sample, a tenth more). Drawn, so it is as sharp as the
//! screen at any scale.

use vergence_scene::{Color, Point};

use crate::{Element, Length, drawing};

/// A chart `height` points tall of `slots` samples across.
pub fn chart<M: 'static>(series: Vec<Vec<f64>>, top: Option<f64>, slots: usize, height: f32) -> Element<M> {
    drawing(move |scene, _, r, t| {
        let faint = Color::rgba(t.dim.r, t.dim.g, t.dim.b, 50);
        scene.line(Point::new(r.x, r.y + r.h), Point::new(r.x + r.w, r.y + r.h), 1.0, faint);
        let top = top.unwrap_or_else(|| series.iter().flatten().copied().fold(0.0f64, f64::max) * 1.1).max(1e-9);
        let step = r.w / (slots.max(2) - 1) as f32;
        let y_of = |v: f64| r.y + r.h - (v / top).clamp(0.0, 1.0) as f32 * r.h;
        for (k, s) in series.iter().enumerate().rev() {
            if s.len() < 2 {
                continue;
            }
            let c = if k == 0 { t.accent } else { Color::rgba(t.accent.r, t.accent.g, t.accent.b, 110) };
            let x0 = r.x + r.w - step * (s.len() - 1) as f32;
            for (i, p) in s.windows(2).enumerate() {
                scene.line(
                    Point::new(x0 + step * i as f32, y_of(p[0])),
                    Point::new(x0 + step * (i + 1) as f32, y_of(p[1])),
                    1.6,
                    c,
                );
            }
        }
    })
    .height(Length::Fixed(height))
}

/// A bar `fraction` full (0 to 1), as wide as its place.
pub fn meter<M: 'static>(fraction: f64, height: f32) -> Element<M> {
    drawing(move |scene, _, r, t| {
        let rad = (r.h / 2.0).min(4.0);
        scene.fill_rounded(r, rad, Color::rgba(t.dim.r, t.dim.g, t.dim.b, 45));
        let w = (r.w * fraction.clamp(0.0, 1.0) as f32).max(if fraction > 0.0 { r.h } else { 0.0 });
        if w > 0.0 {
            scene.fill_rounded(vergence_scene::Rect::new(r.x, r.y, w, r.h), rad, t.accent);
        }
    })
    .height(Length::Fixed(height))
}
