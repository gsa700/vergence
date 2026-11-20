//! Polygons into coverage: for each pixel, how much of it lies inside.
//!
//! Exact-area accumulation (the method of Raph Levien's font-rs and of
//! most font rasterizers): every edge adds, to the cells it crosses, the
//! signed area it sweeps; a running sum along each row then gives each
//! pixel's winding-weighted coverage. Non-zero takes its size clamped to
//! 1; even-odd folds it (0 at even windings, 1 at odd). Edges come out
//! exact for a single edge per pixel and very close where several meet.

use crate::geom::{Point, Polyline};

/// The largest area, in pixels, one coverage may have.
pub const MAX_AREA: usize = 1 << 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// Coverage (0 to 1) for the pixels of a rectangle at `x`, `y`.
#[derive(Debug, Clone, PartialEq)]
pub struct Coverage {
    pub x: i32,
    pub y: i32,
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

impl Coverage {
    pub fn get(&self, px: i32, py: i32) -> f32 {
        let (dx, dy) = (px - self.x, py - self.y);
        if dx < 0 || dy < 0 || dx as usize >= self.w || dy as usize >= self.h {
            return 0.0;
        }
        self.data[dy as usize * self.w + dx as usize]
    }

    /// The total covered area, in pixels (tests).
    pub fn area(&self) -> f64 {
        self.data.iter().map(|v| *v as f64).sum()
    }
}

/// The coverage of these polylines (each closed for filling) within the
/// rectangle 0, 0, `width`, `height`. None if nothing of it is inside.
pub fn fill(lines: &[Polyline], rule: FillRule, width: usize, height: usize) -> Option<Coverage> {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for l in lines {
        for p in &l.points {
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
    }
    if x0 > x1 {
        return None;
    }
    // Pixels touched, within the target. Area to the left of the box still
    // counts (it covers what is right of an edge), so x starts at the box's
    // left or 0; nothing to the right of x1 is ever covered.
    let bx0 = (x0.floor().max(0.0) as i64).min(width as i64);
    let by0 = (y0.floor().max(0.0) as i64).min(height as i64);
    let bx1 = (x1.ceil().min(width as f64) as i64).max(bx0);
    let by1 = (y1.ceil().min(height as f64) as i64).max(by0);
    let (w, h) = ((bx1 - bx0) as usize, (by1 - by0) as usize);
    if w == 0 || h == 0 || w.checked_mul(h)? > MAX_AREA {
        return None;
    }
    // Each row two cells wider: an edge at the right end writes one past it.
    let stride = w + 2;
    let mut acc = vec![0f32; stride * h];
    let (ox, oy) = (bx0 as f64, by0 as f64);
    for l in lines {
        let n = l.points.len();
        for i in 0..n {
            let a = l.points[i];
            let b = l.points[(i + 1) % n];
            edge(&mut acc, stride, w, h, Point::new(a.x - ox, a.y - oy), Point::new(b.x - ox, b.y - oy));
        }
    }
    let mut data = vec![0f32; w * h];
    for row in 0..h {
        let mut sum = 0f32;
        for col in 0..w {
            sum += acc[row * stride + col];
            let v = sum.abs();
            data[row * w + col] = match rule {
                FillRule::NonZero => v.min(1.0),
                FillRule::EvenOdd => {
                    let f = v % 2.0;
                    if f > 1.0 { 2.0 - f } else { f }
                }
            };
        }
    }
    Some(Coverage { x: bx0 as i32, y: by0 as i32, w, h, data })
}

/// One edge's signed area into the accumulation rows (font-rs's draw_line,
/// with x kept within the row and y clipped to the rows there are).
fn edge(acc: &mut [f32], stride: usize, w: usize, h: usize, p0: Point, p1: Point) {
    if p0.y == p1.y || !p0.is_finite() || !p1.is_finite() {
        return;
    }
    let (dir, p0, p1) = if p0.y < p1.y { (1.0f64, p0, p1) } else { (-1.0, p1, p0) };
    if p1.y <= 0.0 || p0.y >= h as f64 {
        return;
    }
    let dxdy = (p1.x - p0.x) / (p1.y - p0.y);
    let ystart = p0.y.max(0.0);
    let yend = p1.y.min(h as f64);
    let mut x = p0.x + (ystart - p0.y) * dxdy;
    let wf = w as f64;
    let row0 = ystart.floor() as usize;
    let row1 = (yend.ceil() as usize).min(h);
    for row in row0..row1 {
        let top = (row as f64).max(ystart);
        let bottom = ((row + 1) as f64).min(yend);
        let dy = bottom - top;
        if dy <= 0.0 {
            continue;
        }
        let xnext = x + dxdy * dy;
        let d = (dy * dir) as f32;
        let line = row * stride;
        // Outside the row on the left counts as at its left edge; on the
        // right, as one cell past it (never summed into a pixel).
        let (xa, xb) = (x.clamp(0.0, wf + 1.0), xnext.clamp(0.0, wf + 1.0));
        let (lo, hi) = if xa < xb { (xa, xb) } else { (xb, xa) };
        let lo_floor = lo.floor();
        let lo_i = lo_floor as usize;
        let hi_ceil = hi.ceil();
        let hi_i = hi_ceil as usize;
        let at = |acc: &mut [f32], i: usize, v: f32| {
            if let Some(c) = acc.get_mut(line + i.min(stride - 1)) {
                *c += v;
            }
        };
        if hi_i <= lo_i + 1 {
            // Within one cell: the part right of the edge's middle.
            let xmf = (0.5 * (xa + xb) - lo_floor) as f32;
            at(acc, lo_i, d - d * xmf);
            at(acc, lo_i + 1, d * xmf);
        } else {
            let s = (1.0 / (hi - lo)) as f32;
            let x0f = (lo - lo_floor) as f32;
            let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
            let x1f = (hi - hi_ceil + 1.0) as f32;
            let am = 0.5 * s * x1f * x1f;
            at(acc, lo_i, d * a0);
            if hi_i == lo_i + 2 {
                at(acc, lo_i + 1, d * (1.0 - a0 - am));
            } else {
                let a1 = s * (1.5 - x0f);
                at(acc, lo_i + 1, d * (a1 - a0));
                for xi in lo_i + 2..hi_i - 1 {
                    at(acc, xi, d * s);
                }
                let a2 = a1 + (hi_i - lo_i - 3) as f32 * s;
                at(acc, hi_i - 1, d * (1.0 - a2 - am));
            }
            at(acc, hi_i, d * am);
        }
        x = xnext;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Path, Transform};

    fn cover(p: &Path, rule: FillRule, w: usize, h: usize) -> Coverage {
        fill(&p.flatten(&Transform::IDENTITY, 0.05), rule, w, h).expect("something inside")
    }

    #[test]
    fn a_square_on_whole_pixels_is_exact() {
        let mut p = Path::new();
        p.rect(2.0, 3.0, 4.0, 5.0);
        let c = cover(&p, FillRule::NonZero, 20, 20);
        assert_eq!(c.area(), 20.0);
        assert_eq!(c.get(2, 3), 1.0);
        assert_eq!(c.get(5, 7), 1.0);
        assert_eq!(c.get(6, 7), 0.0);
        assert_eq!(c.get(1, 3), 0.0);
    }

    #[test]
    fn half_pixel_edges_are_half() {
        let mut p = Path::new();
        p.rect(1.5, 1.0, 2.0, 1.0);
        let c = cover(&p, FillRule::NonZero, 10, 10);
        assert!((c.get(1, 1) - 0.5).abs() < 1e-6, "{}", c.get(1, 1));
        assert!((c.get(2, 1) - 1.0).abs() < 1e-6);
        assert!((c.get(3, 1) - 0.5).abs() < 1e-6);
        assert!((c.area() - 2.0).abs() < 1e-5);
    }

    #[test]
    fn a_triangle_and_a_circle_have_their_areas() {
        let mut t = Path::new();
        t.move_to(1.0, 1.0).line_to(41.0, 1.0).line_to(1.0, 31.0).close();
        assert!((cover(&t, FillRule::NonZero, 64, 64).area() - 600.0).abs() < 1e-3);
        let mut c = Path::new();
        c.circle(50.0, 50.0, 30.0);
        // Chords within 0.05 px of the curve lose at most about 2/3 of that
        // times the perimeter (6.3 px here); the cubic circle itself is 0.03% out.
        let area = cover(&c, FillRule::NonZero, 100, 100).area();
        assert!((area - std::f64::consts::PI * 900.0).abs() < 6.5, "{area}");
    }

    #[test]
    fn the_two_rules_differ_on_a_ring() {
        // Two squares the same way round: non-zero fills the middle, even-odd does not.
        let mut p = Path::new();
        p.rect(0.0, 0.0, 10.0, 10.0).rect(3.0, 3.0, 4.0, 4.0);
        assert_eq!(cover(&p, FillRule::NonZero, 20, 20).get(5, 5), 1.0);
        assert_eq!(cover(&p, FillRule::EvenOdd, 20, 20).get(5, 5), 0.0);
        assert_eq!(cover(&p, FillRule::EvenOdd, 20, 20).get(1, 1), 1.0);
        // The inner one the other way round: a hole under both rules.
        let mut q = Path::new();
        q.rect(0.0, 0.0, 10.0, 10.0).move_to(3.0, 3.0).line_to(3.0, 7.0).line_to(7.0, 7.0).line_to(7.0, 3.0).close();
        assert_eq!(cover(&q, FillRule::NonZero, 20, 20).get(5, 5), 0.0);
    }

    #[test]
    fn shapes_past_the_edges_are_cut_cleanly() {
        let mut p = Path::new();
        p.rect(-50.0, -50.0, 60.0, 60.0);
        let c = cover(&p, FillRule::NonZero, 20, 20);
        assert_eq!((c.x, c.y, c.w, c.h), (0, 0, 10, 10));
        assert_eq!(c.area(), 100.0);
        // Running off the right: only the inside counts, nothing wraps to the next row.
        let mut q = Path::new();
        q.rect(15.0, 0.0, 100.0, 2.0);
        let c = cover(&q, FillRule::NonZero, 20, 20);
        assert_eq!(c.area(), 10.0);
        let mut far = Path::new();
        far.rect(100.0, 100.0, 5.0, 5.0);
        assert!(fill(&far.flatten(&Transform::IDENTITY, 0.1), FillRule::NonZero, 20, 20).is_none());
    }
}
