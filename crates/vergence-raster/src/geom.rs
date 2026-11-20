//! Points, affine transforms and paths, and curves flattened into lines.

/// The most line segments a path may become: a hostile file can ask for a
/// curve of any size, but not for unbounded work.
pub const MAX_SEGMENTS: usize = 4_000_000;
/// The most lines one curve becomes.
const MAX_CURVE_STEPS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// x' = a x + c y + e, y' = b x + d y + f (PDF's [a b c d e f]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Transform {
    fn default() -> Transform {
        Transform::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Transform = Transform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn new(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Transform {
        Transform { a, b, c, d, e, f }
    }

    pub fn translate(x: f64, y: f64) -> Transform {
        Transform::new(1.0, 0.0, 0.0, 1.0, x, y)
    }

    pub fn scale(sx: f64, sy: f64) -> Transform {
        Transform::new(sx, 0.0, 0.0, sy, 0.0, 0.0)
    }

    pub fn apply(&self, p: Point) -> Point {
        Point::new(self.a * p.x + self.c * p.y + self.e, self.b * p.x + self.d * p.y + self.f)
    }

    /// This transform, then `next` (PDF's `cm` puts the new matrix first:
    /// `m.then(&ctm)`).
    pub fn then(&self, next: &Transform) -> Transform {
        Transform::new(
            self.a * next.a + self.b * next.c,
            self.a * next.b + self.b * next.d,
            self.c * next.a + self.d * next.c,
            self.c * next.b + self.d * next.d,
            self.e * next.a + self.f * next.c + next.e,
            self.e * next.b + self.f * next.d + next.f,
        )
    }

    pub fn invert(&self) -> Option<Transform> {
        let det = self.a * self.d - self.b * self.c;
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Transform::new(a, b, c, d, -(self.e * a + self.f * c), -(self.e * b + self.f * d)))
    }

    /// How much it scales lengths, on average (the square root of the
    /// determinant's size).
    pub fn scale_factor(&self) -> f64 {
        (self.a * self.d - self.b * self.c).abs().sqrt()
    }

    pub fn is_finite(&self) -> bool {
        [self.a, self.b, self.c, self.d, self.e, self.f].iter().all(|v| v.is_finite())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    Move(Point),
    Line(Point),
    Cubic(Point, Point, Point),
    Close,
}

/// A path as PDF builds it: subpaths of lines and cubic curves.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path {
    pub segs: Vec<Seg>,
}

impl Path {
    pub fn new() -> Path {
        Path::default()
    }

    pub fn move_to(&mut self, x: f64, y: f64) -> &mut Path {
        self.segs.push(Seg::Move(Point::new(x, y)));
        self
    }

    pub fn line_to(&mut self, x: f64, y: f64) -> &mut Path {
        self.segs.push(Seg::Line(Point::new(x, y)));
        self
    }

    pub fn cubic_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64) -> &mut Path {
        self.segs.push(Seg::Cubic(Point::new(x1, y1), Point::new(x2, y2), Point::new(x, y)));
        self
    }

    pub fn close(&mut self) -> &mut Path {
        self.segs.push(Seg::Close);
        self
    }

    /// PDF's `re`: a rectangle as a closed subpath.
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) -> &mut Path {
        self.move_to(x, y).line_to(x + w, y).line_to(x + w, y + h).line_to(x, y + h).close()
    }

    /// A circle from four cubic curves (for round joins and caps, and tests).
    pub fn circle(&mut self, cx: f64, cy: f64, r: f64) -> &mut Path {
        let k = 0.552_284_749_830_793_4 * r;
        self.move_to(cx + r, cy)
            .cubic_to(cx + r, cy + k, cx + k, cy + r, cx, cy + r)
            .cubic_to(cx - k, cy + r, cx - r, cy + k, cx - r, cy)
            .cubic_to(cx - r, cy - k, cx - k, cy - r, cx, cy - r)
            .cubic_to(cx + k, cy - r, cx + r, cy - k, cx + r, cy)
            .close()
    }

    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    /// The path with every point through `t`.
    pub fn transformed(&self, t: &Transform) -> Path {
        let segs = self
            .segs
            .iter()
            .map(|s| match *s {
                Seg::Move(p) => Seg::Move(t.apply(p)),
                Seg::Line(p) => Seg::Line(t.apply(p)),
                Seg::Cubic(a, b, c) => Seg::Cubic(t.apply(a), t.apply(b), t.apply(c)),
                Seg::Close => Seg::Close,
            })
            .collect();
        Path { segs }
    }

    /// Another path's subpaths added to this one.
    pub fn append(&mut self, other: &Path) {
        self.segs.extend_from_slice(&other.segs);
    }

    /// The path through `t`, each subpath flattened into a polyline (closed
    /// ones marked), curves within `tolerance` of the true curve in the
    /// transformed space. Points that are not finite drop their subpath.
    pub fn flatten(&self, t: &Transform, tolerance: f64) -> Vec<Polyline> {
        let tol = if tolerance.is_finite() && tolerance > 0.0 { tolerance } else { 0.1 };
        let mut out: Vec<Polyline> = Vec::new();
        let mut cur: Option<Polyline> = None;
        let mut total = 0usize;
        let mut start = Point::new(0.0, 0.0);
        let keep = |p: Polyline, out: &mut Vec<Polyline>| {
            if p.points.len() > 1 && p.points.iter().all(Point::is_finite) {
                out.push(p);
            }
        };
        for seg in &self.segs {
            if total > MAX_SEGMENTS {
                break;
            }
            match *seg {
                Seg::Move(p) => {
                    if let Some(done) = cur.take() {
                        keep(done, &mut out);
                    }
                    let q = t.apply(p);
                    start = q;
                    cur = Some(Polyline { points: vec![q], closed: false });
                }
                Seg::Line(p) => {
                    let q = t.apply(p);
                    cur.get_or_insert_with(|| Polyline { points: vec![start], closed: false }).points.push(q);
                    total += 1;
                }
                Seg::Cubic(c1, c2, p) => {
                    let line = cur.get_or_insert_with(|| Polyline { points: vec![start], closed: false });
                    let p0 = *line.points.last().expect("a polyline starts with a point");
                    let (c1, c2, p3) = (t.apply(c1), t.apply(c2), t.apply(p));
                    // Wang's bound: steps so that each line is within tol of the curve.
                    let dd = |a: Point, b: Point, c: Point| ((a.x - 2.0 * b.x + c.x).powi(2) + (a.y - 2.0 * b.y + c.y).powi(2)).sqrt();
                    let m = dd(p0, c1, c2).max(dd(c1, c2, p3));
                    let steps = if m.is_finite() { ((3.0 * m / (4.0 * tol)).sqrt().ceil() as usize).clamp(1, MAX_CURVE_STEPS) } else { 1 };
                    for i in 1..=steps {
                        let s = i as f64 / steps as f64;
                        let u = 1.0 - s;
                        let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * s, 3.0 * u * s * s, s * s * s);
                        line.points.push(Point::new(w0 * p0.x + w1 * c1.x + w2 * c2.x + w3 * p3.x, w0 * p0.y + w1 * c1.y + w2 * c2.y + w3 * p3.y));
                    }
                    total += steps;
                }
                Seg::Close => {
                    // A subpath after a close starts where the closed one began.
                    if let Some(mut p) = cur.take() {
                        p.closed = true;
                        start = p.points[0];
                        keep(p, &mut out);
                    }
                }
            }
        }
        if let Some(done) = cur.take() {
            keep(done, &mut out);
        }
        out
    }
}

/// A flattened subpath.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Polyline {
    pub points: Vec<Point>,
    pub closed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_compose_as_pdf_does() {
        let scale = Transform::scale(2.0, 3.0);
        let shift = Transform::translate(10.0, 20.0);
        // Scale first, then shift.
        let both = scale.then(&shift);
        assert_eq!(both.apply(Point::new(1.0, 1.0)), Point::new(12.0, 23.0));
        let inv = both.invert().unwrap();
        let back = inv.apply(Point::new(12.0, 23.0));
        assert!((back.x - 1.0).abs() < 1e-12 && (back.y - 1.0).abs() < 1e-12);
        assert!(Transform::scale(0.0, 1.0).invert().is_none());
    }

    #[test]
    fn curves_flatten_within_tolerance() {
        let mut p = Path::new();
        p.circle(0.0, 0.0, 100.0);
        let lines = p.flatten(&Transform::IDENTITY, 0.1);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].closed);
        for q in &lines[0].points {
            let r = (q.x * q.x + q.y * q.y).sqrt();
            assert!((r - 100.0).abs() < 0.1, "{r}");
        }
        assert!(lines[0].points.len() > 40 && lines[0].points.len() < 400, "{}", lines[0].points.len());
    }

    #[test]
    fn bad_numbers_drop_their_subpath() {
        let mut p = Path::new();
        p.move_to(0.0, 0.0).line_to(f64::NAN, 1.0).move_to(0.0, 0.0).line_to(5.0, 5.0);
        let lines = p.flatten(&Transform::IDENTITY, 0.1);
        assert_eq!(lines.len(), 1);
        let mut huge = Path::new();
        huge.move_to(0.0, 0.0).cubic_to(1e300, 1e300, -1e300, 1e300, 0.0, 0.0);
        let _ = huge.flatten(&Transform::IDENTITY, 0.1);
    }
}
