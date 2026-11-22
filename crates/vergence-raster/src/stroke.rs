//! Strokes into polygons: a quad for every segment, a join at every
//! corner, a cap at every open end, dashes cut first; every polygon wound
//! the same way, so filling them all with the non-zero rule gives the
//! stroke's outline however the pieces overlap. Worked out in user space
//! (where PDF gives widths and dashes), then transformed, so a stroke under
//! a stretching matrix comes out with the right elliptical pen.

use crate::geom::{Point, Polyline, Transform};

/// The most pieces dashes may cut a path into; a pattern finer than half a
/// device pixel is drawn solid, as other readers do.
const MAX_DASHES: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Join {
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub width: f64,
    pub cap: Cap,
    pub join: Join,
    pub miter_limit: f64,
    pub dash: Vec<f64>,
    pub dash_phase: f64,
}

impl Default for Stroke {
    fn default() -> Stroke {
        Stroke { width: 1.0, cap: Cap::Butt, join: Join::Miter, miter_limit: 10.0, dash: Vec::new(), dash_phase: 0.0 }
    }
}

/// The polygons (in device space, through `t`) that fill as this stroke of
/// `lines` (in user space). Never thinner than one device pixel.
pub fn stroke(lines: &[Polyline], style: &Stroke, t: &Transform) -> Vec<Polyline> {
    let scale = t.scale_factor();
    if !(scale.is_finite() && scale > 0.0) {
        return Vec::new();
    }
    let width = if style.width.is_finite() { style.width.abs() } else { 1.0 };
    let half = width.max(1.0 / scale) / 2.0;
    // Round parts within a tenth of a device pixel.
    let tolerance = 0.1 / scale;
    let pieces = dashed(lines, style, scale);
    let mut out = Vec::new();
    for piece in &pieces {
        outline(piece, half, style, tolerance, &mut out);
        if out.len() > 4 * MAX_DASHES {
            break;
        }
    }
    for poly in &mut out {
        for p in &mut poly.points {
            *p = t.apply(*p);
        }
    }
    out
}

/// The subpaths cut by the dash pattern (or as they are without one).
fn dashed(lines: &[Polyline], style: &Stroke, scale: f64) -> Vec<Polyline> {
    let pattern: Vec<f64> = style.dash.iter().map(|d| if d.is_finite() { d.abs() } else { 0.0 }).collect();
    let total: f64 = pattern.iter().sum();
    // Drawn solid: no pattern, one finer than half a device pixel, or one
    // that would cut the path into more pieces than is reasonable.
    let length: f64 = lines
        .iter()
        .map(|l| {
            l.points.windows(2).map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt()).sum::<f64>()
        })
        .sum();
    if pattern.is_empty()
        || !total.is_finite()
        || total * scale < 0.5
        || !length.is_finite()
        || length / total > MAX_DASHES as f64
    {
        return lines.iter().map(dedup).collect();
    }
    // An odd-length pattern repeats to even length.
    let pattern: Vec<f64> =
        if pattern.len() % 2 == 1 { pattern.iter().chain(pattern.iter()).copied().collect() } else { pattern };
    let mut out = Vec::new();
    for line in lines {
        let line = dedup(line);
        let mut pts = line.points.clone();
        if line.closed && pts.len() > 1 {
            pts.push(pts[0]);
        }
        // Where in the pattern the phase puts the start.
        let mut idx = 0usize;
        let mut left = pattern[0];
        let mut phase = style.dash_phase.rem_euclid(total);
        while phase > 0.0 {
            if phase >= left {
                phase -= left;
                idx = (idx + 1) % pattern.len();
                left = pattern[idx];
            } else {
                left -= phase;
                phase = 0.0;
            }
        }
        let mut cur: Vec<Point> = if idx.is_multiple_of(2) { vec![pts[0]] } else { Vec::new() };
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
            let mut at = 0.0;
            while len - at > left {
                at += left;
                let p = Point::new(a.x + (b.x - a.x) * at / len, a.y + (b.y - a.y) * at / len);
                if idx.is_multiple_of(2) {
                    cur.push(p);
                    out.push(Polyline { points: std::mem::take(&mut cur), closed: false });
                    if out.len() > MAX_DASHES {
                        return out;
                    }
                } else {
                    cur = vec![p];
                }
                idx = (idx + 1) % pattern.len();
                left = pattern[idx];
            }
            left -= len - at;
            if idx.is_multiple_of(2) {
                cur.push(b);
            }
        }
        if idx.is_multiple_of(2) && cur.len() > 1 {
            out.push(Polyline { points: cur, closed: false });
        }
    }
    out
}

/// Without repeated points (they have no direction).
fn dedup(line: &Polyline) -> Polyline {
    let mut points: Vec<Point> = Vec::with_capacity(line.points.len());
    for p in &line.points {
        if points.last().is_none_or(|q| (q.x - p.x).abs() > 1e-9 || (q.y - p.y).abs() > 1e-9) {
            points.push(*p);
        }
    }
    let mut closed = line.closed;
    if closed && points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    if points.len() < 3 {
        closed = closed && points.len() == 2;
    }
    Polyline { points, closed }
}

fn outline(line: &Polyline, half: f64, style: &Stroke, tol: f64, out: &mut Vec<Polyline>) {
    let pts = &line.points;
    if pts.is_empty() {
        return;
    }
    if pts.len() == 1 {
        // A zero-length subpath: a dot with round caps, a square with square ones.
        let p = pts[0];
        match style.cap {
            Cap::Round => push(out, circle(p, half, tol)),
            Cap::Square => push(
                out,
                vec![
                    Point::new(p.x - half, p.y - half),
                    Point::new(p.x + half, p.y - half),
                    Point::new(p.x + half, p.y + half),
                    Point::new(p.x - half, p.y + half),
                ],
            ),
            Cap::Butt => {}
        }
        return;
    }
    let n = pts.len();
    let segs = if line.closed { n } else { n - 1 };
    for i in 0..segs {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let nrm = normal(a, b, half);
        push(
            out,
            vec![
                Point::new(a.x + nrm.x, a.y + nrm.y),
                Point::new(b.x + nrm.x, b.y + nrm.y),
                Point::new(b.x - nrm.x, b.y - nrm.y),
                Point::new(a.x - nrm.x, a.y - nrm.y),
            ],
        );
    }
    // Joins at inner corners, and at the start of a closed one.
    let corners: Vec<usize> = if line.closed { (0..n).collect() } else { (1..n - 1).collect() };
    for i in corners {
        let (prev, v, next) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        join(out, prev, v, next, half, style, tol);
    }
    if !line.closed {
        cap(out, pts[1], pts[0], half, style.cap, tol);
        cap(out, pts[n - 2], pts[n - 1], half, style.cap, tol);
    }
}

/// The left normal of a to b, `half` long.
fn normal(a: Point, b: Point, half: f64) -> Point {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt().max(1e-300);
    Point::new(-dy / len * half, dx / len * half)
}

fn join(out: &mut Vec<Polyline>, prev: Point, v: Point, next: Point, half: f64, style: &Stroke, tol: f64) {
    let (n1, n2) = (normal(prev, v, half), normal(v, next, half));
    let turn = (v.x - prev.x) * (next.y - v.y) - (v.y - prev.y) * (next.x - v.x);
    if turn.abs() < 1e-12 && n1.x * n2.x + n1.y * n2.y > 0.0 {
        return;
    }
    match style.join {
        Join::Round => push(out, circle(v, half, tol)),
        Join::Bevel => bevel(out, v, n1, n2),
        Join::Miter => {
            // The outer side: right of a left turn, left of a right one.
            let s = if turn > 0.0 { -1.0 } else { 1.0 };
            let (o1, o2) = (Point::new(n1.x * s, n1.y * s), Point::new(n2.x * s, n2.y * s));
            let cos = (o1.x * o2.x + o1.y * o2.y) / (half * half);
            let ratio = (2.0 / (1.0 + cos).max(1e-12)).sqrt();
            if ratio <= style.miter_limit.max(1.0) {
                let k = 1.0 / (1.0 + cos).max(1e-12);
                let tip = Point::new(v.x + (o1.x + o2.x) * k, v.y + (o1.y + o2.y) * k);
                push(out, vec![v, Point::new(v.x + o1.x, v.y + o1.y), tip, Point::new(v.x + o2.x, v.y + o2.y)]);
            } else {
                bevel(out, v, n1, n2);
            }
        }
    }
}

fn bevel(out: &mut Vec<Polyline>, v: Point, n1: Point, n2: Point) {
    push(out, vec![v, Point::new(v.x + n1.x, v.y + n1.y), Point::new(v.x + n2.x, v.y + n2.y)]);
    push(out, vec![v, Point::new(v.x - n1.x, v.y - n1.y), Point::new(v.x - n2.x, v.y - n2.y)]);
}

/// The cap at `end`, the line arriving from `from`.
fn cap(out: &mut Vec<Polyline>, from: Point, end: Point, half: f64, cap: Cap, tol: f64) {
    match cap {
        Cap::Butt => {}
        Cap::Round => push(out, circle(end, half, tol)),
        Cap::Square => {
            let n = normal(from, end, half);
            // Along the line, half a width further.
            let d = Point::new(n.y, -n.x);
            push(
                out,
                vec![
                    Point::new(end.x + n.x, end.y + n.y),
                    Point::new(end.x + n.x + d.x, end.y + n.y + d.y),
                    Point::new(end.x - n.x + d.x, end.y - n.y + d.y),
                    Point::new(end.x - n.x, end.y - n.y),
                ],
            );
        }
    }
}

fn circle(c: Point, r: f64, tol: f64) -> Vec<Point> {
    // Enough sides that no side strays more than a quarter of tol from the
    // circle (a dot's area must come out right, not only its outline).
    let tol = tol / 4.0;
    let steps =
        if r > tol { (std::f64::consts::PI / (1.0 - tol / r).clamp(-1.0, 1.0).acos()).ceil() as usize } else { 4 };
    let steps = steps.clamp(4, 256);
    (0..steps)
        .map(|i| {
            let a = i as f64 / steps as f64 * std::f64::consts::TAU;
            Point::new(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}

/// A closed polygon, wound positively (reversed if need be).
fn push(out: &mut Vec<Polyline>, mut points: Vec<Point>) {
    let area: f64 = (0..points.len())
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    if area < 0.0 {
        points.reverse();
    }
    if area.abs() > 0.0 {
        out.push(Polyline { points, closed: true });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Path;
    use crate::raster::{FillRule, fill};

    fn area_of(p: &Path, s: &Stroke, t: &Transform) -> f64 {
        let lines = p.flatten(&Transform::IDENTITY, 0.01);
        let polys = stroke(&lines, s, t);
        fill(&polys, FillRule::NonZero, 400, 400).map_or(0.0, |c| c.area())
    }

    #[test]
    fn a_straight_stroke_is_length_by_width() {
        let mut p = Path::new();
        p.move_to(10.5, 50.0).line_to(110.5, 50.0);
        let s = Stroke { width: 4.0, ..Default::default() };
        assert!((area_of(&p, &s, &Transform::IDENTITY) - 400.0).abs() < 0.5);
        // Square caps add half a width at each end; round ones a circle's worth.
        let sq = Stroke { cap: Cap::Square, ..s.clone() };
        assert!((area_of(&p, &sq, &Transform::IDENTITY) - 416.0).abs() < 0.5);
        let rd = Stroke { cap: Cap::Round, ..s.clone() };
        let want = 400.0 + std::f64::consts::PI * 4.0;
        assert!((area_of(&p, &rd, &Transform::IDENTITY) - want).abs() < 1.0);
        // Twice the scale: twice as long and twice as wide.
        assert!((area_of(&p, &s, &Transform::scale(2.0, 2.0)) - 1600.0).abs() < 2.0);
    }

    #[test]
    fn corners_join_without_gaps() {
        let mut p = Path::new();
        p.move_to(50.0, 50.0).line_to(150.0, 50.0).line_to(150.0, 150.0);
        let miter = Stroke { width: 10.0, ..Default::default() };
        let bevel = Stroke { join: Join::Bevel, ..miter.clone() };
        let round = Stroke { join: Join::Round, ..miter.clone() };
        let (m, b, r) = (
            area_of(&p, &miter, &Transform::IDENTITY),
            area_of(&p, &bevel, &Transform::IDENTITY),
            area_of(&p, &round, &Transform::IDENTITY),
        );
        // Two strokes of 100 x 10 overlapping in a 5 x 5 corner, plus the join's outer part.
        let base = 2000.0 - 25.0;
        assert!((m - (base + 25.0)).abs() < 1.0, "miter {m}");
        assert!((b - (base + 12.5)).abs() < 1.0, "bevel {b}");
        assert!((r - (base + 25.0 * std::f64::consts::PI / 4.0)).abs() < 1.0, "round {r}");
        // A sharp turn past the miter limit falls back to a bevel.
        let mut sharp = Path::new();
        sharp.move_to(50.0, 50.0).line_to(150.0, 52.0).line_to(50.0, 54.0);
        let tight = Stroke { miter_limit: 2.0, ..miter.clone() };
        let loose = Stroke { miter_limit: 1000.0, ..miter.clone() };
        assert!(area_of(&sharp, &loose, &Transform::IDENTITY) > area_of(&sharp, &tight, &Transform::IDENTITY) + 50.0);
    }

    #[test]
    fn dashes_cut_the_line() {
        let mut p = Path::new();
        p.move_to(0.0, 10.0).line_to(100.0, 10.0);
        let s = Stroke { width: 2.0, dash: vec![10.0, 10.0], ..Default::default() };
        let pieces = dashed(&p.flatten(&Transform::IDENTITY, 0.1), &s, 1.0);
        assert_eq!(pieces.len(), 5);
        assert!((area_of(&p, &s, &Transform::IDENTITY) - 100.0).abs() < 0.5);
        // A phase shifts the pattern: starting 5 into an "on" dash.
        let shifted = Stroke { dash_phase: 5.0, ..s.clone() };
        let pieces = dashed(&p.flatten(&Transform::IDENTITY, 0.1), &shifted, 1.0);
        assert_eq!(pieces[0].points[1].x, 5.0);
        // A pattern finer than half a pixel is drawn solid.
        let fine = Stroke { dash: vec![0.1, 0.1], ..s.clone() };
        assert_eq!(dashed(&p.flatten(&Transform::IDENTITY, 0.1), &fine, 1.0).len(), 1);
    }

    #[test]
    fn hairlines_are_one_device_pixel() {
        let mut p = Path::new();
        p.move_to(10.0, 50.5).line_to(110.0, 50.5);
        let s = Stroke { width: 0.0, ..Default::default() };
        assert!((area_of(&p, &s, &Transform::IDENTITY) - 100.0).abs() < 0.5);
        // Under a large scale a zero width is still one device pixel, not scaled up.
        assert!((area_of(&p, &s, &Transform::scale(3.0, 3.0)) - 300.0).abs() < 1.0);
    }

    #[test]
    fn a_dot_for_a_zero_length_round_cap() {
        let mut p = Path::new();
        p.move_to(50.0, 50.0).line_to(50.0, 50.0);
        let s = Stroke { width: 10.0, cap: Cap::Round, ..Default::default() };
        let a = area_of(&p, &s, &Transform::IDENTITY);
        assert!((a - std::f64::consts::PI * 25.0).abs() < 1.0, "{a}");
    }
}
