//! Our own vector rasterizer (PDF milestone 1, step 2;
//! docs/pdf-milestone-1.md): what PDF pages need and Vergence's scenes do
//! not have. Paths of lines and cubic curves (geom.rs), filled by the
//! non-zero or even-odd rule with exact-area anti-aliasing (raster.rs),
//! stroked with widths, caps, joins, a miter limit and dashes (stroke.rs),
//! clipped by any path (a [`Mask`]), and pictures drawn through any
//! transform with smoothing. A [`Canvas`] holds premultiplied RGBA.
//!
//! **Input is untrusted** (it comes from PDF files): segment counts,
//! areas and canvas sizes are bounded, numbers that are not finite drop
//! what they belong to, and nothing here panics on any input (the fuzz
//! test). Not constant-time, not SIMD: correct first, measured later.
//! (Later it can stand in for resvg's renderer too: docs/borrowed-review.md.)

pub mod geom;
pub mod raster;
pub mod stroke;

pub use geom::{Path, Point, Polyline, Transform};
pub use raster::{Coverage, FillRule};
pub use stroke::{Cap, Join, Stroke};

/// The most pixels a canvas may have (8192 x 8192).
pub const MAX_PIXELS: usize = 1 << 26;
/// Curves within this many device pixels of the true curve.
const TOLERANCE: f64 = 0.1;

/// Straight (not premultiplied) RGBA, each 0 to 1.
pub type Rgba = [f32; 4];

/// Coverage for the whole canvas, 0 to 1: what a clip lets through.
#[derive(Debug, Clone, PartialEq)]
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

impl Mask {
    /// Everything let through.
    pub fn full(width: usize, height: usize) -> Option<Mask> {
        let n = width.checked_mul(height).filter(|n| *n <= MAX_PIXELS)?;
        Some(Mask { width, height, data: vec![1.0; n] })
    }

    /// This mask narrowed to `path` (PDF's W and W*: the new clip is the
    /// old one intersected with the path).
    pub fn intersect(&mut self, path: &Path, t: &Transform, rule: FillRule) {
        let cov = fill_coverage(path, t, rule, self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let c = cov.as_ref().map_or(0.0, |c| c.get(x as i32, y as i32));
                self.data[y * self.width + x] *= c;
            }
        }
    }

    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.data.get(y * self.width + x).copied().unwrap_or(0.0)
    }
}

/// What a clip lets through: most clips in real pages are rectangles,
/// kept exact and cheap; anything else becomes a [`Mask`].
#[derive(Debug, Clone, PartialEq)]
pub enum Clip {
    /// Device x0, y0, x1, y1.
    Rect([f64; 4]),
    Mask(Mask),
}

impl Clip {
    /// The whole of a `width` x `height` target.
    pub fn full(width: usize, height: usize) -> Clip {
        Clip::Rect([0.0, 0.0, width as f64, height as f64])
    }

    /// How much of pixel `x`, `y` it lets through.
    pub fn at(&self, x: usize, y: usize) -> f32 {
        match self {
            Clip::Rect([x0, y0, x1, y1]) => {
                let (fx, fy) = (x as f64, y as f64);
                let w = ((fx + 1.0).min(*x1) - fx.max(*x0)).max(0.0);
                let h = ((fy + 1.0).min(*y1) - fy.max(*y0)).max(0.0);
                (w * h) as f32
            }
            Clip::Mask(m) => m.at(x, y),
        }
    }

    /// This clip narrowed to `path` through `t` (PDF's W and W*). A
    /// rectangle within a rectangle stays a rectangle.
    pub fn intersect(&self, path: &Path, t: &Transform, rule: FillRule, width: usize, height: usize) -> Clip {
        if let (Clip::Rect(r), Some(n)) = (self, device_rect(path, t)) {
            return Clip::Rect([r[0].max(n[0]), r[1].max(n[1]), r[2].min(n[2]).max(r[0].max(n[0])), r[3].min(n[3]).max(r[1].max(n[1]))]);
        }
        let Some(mut mask) = Mask::full(width, height) else { return Clip::Rect([0.0; 4]) };
        if let Clip::Mask(old) = self {
            mask.data.clone_from(&old.data);
        } else {
            for y in 0..height {
                for x in 0..width {
                    mask.data[y * width + x] = self.at(x, y);
                }
            }
        }
        mask.intersect(path, t, rule);
        Clip::Mask(mask)
    }
}

/// The device rectangle a path is, if it is one rectangle with sides along
/// the axes (PDF's `re`, under a matrix without rotation).
fn device_rect(path: &Path, t: &Transform) -> Option<[f64; 4]> {
    let lines = path.flatten(t, TOLERANCE);
    let [only] = lines.as_slice() else { return None };
    let mut pts = only.points.clone();
    if pts.len() == 5 && pts[0] == pts[4] {
        pts.pop();
    }
    if pts.len() != 4 {
        return None;
    }
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let axis = (0..4).all(|i| {
        let (a, b) = (pts[i], pts[(i + 1) % 4]);
        near(a.x, b.x) || near(a.y, b.y)
    });
    if !axis {
        return None;
    }
    let (x0, x1) = (pts.iter().map(|p| p.x).fold(f64::MAX, f64::min), pts.iter().map(|p| p.x).fold(f64::MIN, f64::max));
    let (y0, y1) = (pts.iter().map(|p| p.y).fold(f64::MAX, f64::min), pts.iter().map(|p| p.y).fold(f64::MIN, f64::max));
    Some([x0, y0, x1, y1])
}

/// A picture to draw: premultiplied RGBA, rows from the top.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

/// Premultiplied RGBA pixels, rows from the top.
#[derive(Debug, Clone, PartialEq)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

/// A path's coverage on a `width` x `height` target.
pub fn fill_coverage(path: &Path, t: &Transform, rule: FillRule, width: usize, height: usize) -> Option<Coverage> {
    if !t.is_finite() {
        return None;
    }
    raster::fill(&path.flatten(t, TOLERANCE), rule, width, height)
}

/// A stroke's coverage on a `width` x `height` target.
pub fn stroke_coverage(path: &Path, style: &Stroke, t: &Transform, width: usize, height: usize) -> Option<Coverage> {
    if !t.is_finite() {
        return None;
    }
    let scale = t.scale_factor().max(1e-9);
    // Flattened in user space, finely enough for the device.
    let lines = path.flatten(&Transform::IDENTITY, TOLERANCE / scale);
    raster::fill(&stroke::stroke(&lines, style, t), FillRule::NonZero, width, height)
}

impl Canvas {
    /// A transparent canvas; None if it would be too large.
    pub fn new(width: usize, height: usize) -> Option<Canvas> {
        let n = width.checked_mul(height).filter(|n| *n <= MAX_PIXELS && *n > 0)?;
        Some(Canvas { width, height, data: vec![0; n * 4] })
    }

    /// Fills the whole canvas with one color (a page's white).
    pub fn clear(&mut self, color: Rgba) {
        let px = premul_bytes(color);
        for p in self.data.as_chunks_mut::<4>().0 {
            *p = px;
        }
    }

    pub fn fill(&mut self, path: &Path, t: &Transform, rule: FillRule, color: Rgba, clip: Option<&Clip>) {
        if let Some(cov) = fill_coverage(path, t, rule, self.width, self.height) {
            self.paint(&cov, clip, &|_, _| premul(color));
        }
    }

    pub fn stroke(&mut self, path: &Path, t: &Transform, style: &Stroke, color: Rgba, clip: Option<&Clip>) {
        if let Some(cov) = stroke_coverage(path, style, t, self.width, self.height) {
            self.paint(&cov, clip, &|_, _| premul(color));
        }
    }

    /// Paints `cov` (times the clip) with a color per pixel centre (device
    /// x, y), premultiplied: what shadings and patterns use.
    pub fn paint(&mut self, cov: &Coverage, clip: Option<&Clip>, color: &dyn Fn(f64, f64) -> Rgba) {
        for row in 0..cov.h {
            let y = cov.y as usize + row;
            if y >= self.height {
                break;
            }
            for col in 0..cov.w {
                let x = cov.x as usize + col;
                if x >= self.width {
                    break;
                }
                let mut a = cov.data[row * cov.w + col];
                if let Some(m) = clip {
                    a *= m.at(x, y);
                }
                if a <= 0.0 {
                    continue;
                }
                let c = color(x as f64 + 0.5, y as f64 + 0.5);
                over(&mut self.data[(y * self.width + x) * 4..][..4], c, a);
            }
        }
    }

    /// Draws `img` with `t` taking its pixel space (0..width, 0..height,
    /// rows from the top) to the canvas; `smooth` samples between pixels,
    /// `alpha` fades it. Only the device pixels it can reach are visited.
    pub fn draw_image(&mut self, img: &Image, t: &Transform, smooth: bool, alpha: f32, clip: Option<&Clip>) {
        if img.width == 0 || img.height == 0 || img.data.len() < img.width * img.height * 4 {
            return;
        }
        let Some(inv) = t.invert() else { return };
        let corners = [(0.0, 0.0), (img.width as f64, 0.0), (0.0, img.height as f64), (img.width as f64, img.height as f64)].map(|(x, y)| t.apply(Point::new(x, y)));
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for c in corners {
            x0 = x0.min(c.x);
            y0 = y0.min(c.y);
            x1 = x1.max(c.x);
            y1 = y1.max(c.y);
        }
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            return;
        }
        // The picture's own edges, anti-aliased like any shape.
        let mut outline = Path::new();
        outline.rect(0.0, 0.0, img.width as f64, img.height as f64);
        let Some(cov) = fill_coverage(&outline, t, FillRule::NonZero, self.width, self.height) else { return };
        let (w, h) = (img.width as f64, img.height as f64);
        let sample = |u: f64, v: f64| -> Rgba {
            let px = |x: usize, y: usize| -> [f32; 4] {
                let i = (y * img.width + x) * 4;
                [img.data[i] as f32 / 255.0, img.data[i + 1] as f32 / 255.0, img.data[i + 2] as f32 / 255.0, img.data[i + 3] as f32 / 255.0]
            };
            if !smooth {
                let (x, y) = ((u.floor().clamp(0.0, w - 1.0)) as usize, (v.floor().clamp(0.0, h - 1.0)) as usize);
                return px(x, y);
            }
            let (fx, fy) = ((u - 0.5).clamp(0.0, w - 1.0), (v - 0.5).clamp(0.0, h - 1.0));
            let (ix, iy) = (fx.floor() as usize, fy.floor() as usize);
            let (jx, jy) = ((ix + 1).min(img.width - 1), (iy + 1).min(img.height - 1));
            let (tx, ty) = ((fx - ix as f64) as f32, (fy - iy as f64) as f32);
            let (a, b, c, d) = (px(ix, iy), px(jx, iy), px(ix, jy), px(jx, jy));
            std::array::from_fn(|k| (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + d[k] * tx) * ty)
        };
        let alpha = alpha.clamp(0.0, 1.0);
        self.paint(&cov, clip, &|x, y| {
            let p = inv.apply(Point::new(x, y));
            let c = sample(p.x, p.y);
            [c[0] * alpha, c[1] * alpha, c[2] * alpha, c[3] * alpha]
        });
    }

    /// A pixel as straight RGBA bytes (tests, and the PNG writer).
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        let p = &self.data[i..i + 4];
        if p[3] == 0 {
            return [0, 0, 0, 0];
        }
        let un = |c: u8| ((c as u32 * 255 + p[3] as u32 / 2) / p[3] as u32).min(255) as u8;
        [un(p[0]), un(p[1]), un(p[2]), p[3]]
    }
}

/// A straight color as premultiplied.
pub fn premul(c: Rgba) -> Rgba {
    let a = c[3].clamp(0.0, 1.0);
    [c[0].clamp(0.0, 1.0) * a, c[1].clamp(0.0, 1.0) * a, c[2].clamp(0.0, 1.0) * a, a]
}

fn premul_bytes(c: Rgba) -> [u8; 4] {
    premul(c).map(|v| (v * 255.0).round() as u8)
}

/// Source over: premultiplied `c` at coverage `a` onto the pixel.
fn over(dst: &mut [u8], c: Rgba, a: f32) {
    let a = a.min(1.0);
    let sa = c[3] * a;
    for k in 0..4 {
        let d = dst[k] as f32 / 255.0;
        let v = c[k] * a + d * (1.0 - sa);
        dst[k] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: Rgba = [0.0, 0.0, 0.0, 1.0];
    const RED: Rgba = [1.0, 0.0, 0.0, 1.0];

    #[test]
    fn fill_and_clip() {
        let mut c = Canvas::new(40, 40).unwrap();
        c.clear([1.0, 1.0, 1.0, 1.0]);
        let mut sq = Path::new();
        sq.rect(5.0, 5.0, 20.0, 20.0);
        // A clip to the left half of the square.
        let mut left = Path::new();
        left.rect(0.0, 0.0, 15.0, 40.0);
        let clip = Clip::full(40, 40).intersect(&left, &Transform::IDENTITY, FillRule::NonZero, 40, 40);
        assert!(matches!(clip, Clip::Rect(_)), "a rectangle stays a rectangle");
        c.fill(&sq, &Transform::IDENTITY, FillRule::NonZero, RED, Some(&clip));
        assert_eq!(c.pixel(10, 10), [255, 0, 0, 255]);
        assert_eq!(c.pixel(20, 10), [255, 255, 255, 255], "clipped away");
        assert_eq!(c.pixel(2, 2), [255, 255, 255, 255]);
        // A circle clip becomes a mask, and still clips.
        let mut round = Path::new();
        round.circle(20.0, 20.0, 5.0);
        let clip = Clip::full(40, 40).intersect(&round, &Transform::IDENTITY, FillRule::NonZero, 40, 40);
        assert!(matches!(clip, Clip::Mask(_)));
        assert_eq!(clip.at(20, 20), 1.0);
        assert_eq!(clip.at(2, 2), 0.0);
    }

    #[test]
    fn half_coverage_blends_half() {
        let mut c = Canvas::new(10, 10).unwrap();
        c.clear([1.0, 1.0, 1.0, 1.0]);
        let mut p = Path::new();
        p.rect(2.5, 2.0, 3.0, 3.0);
        c.fill(&p, &Transform::IDENTITY, FillRule::NonZero, BLACK, None);
        assert_eq!(c.pixel(2, 3), [128, 128, 128, 255]);
        assert_eq!(c.pixel(3, 3), [0, 0, 0, 255]);
    }

    #[test]
    fn a_pdf_page_transform_flips_y() {
        // PDF user space has y up: a 100 x 100 page at 1x flipped into the canvas.
        let mut c = Canvas::new(100, 100).unwrap();
        let page = Transform::new(1.0, 0.0, 0.0, -1.0, 0.0, 100.0);
        let mut p = Path::new();
        p.rect(0.0, 0.0, 10.0, 10.0);
        c.fill(&p, &page, FillRule::NonZero, BLACK, None);
        assert_eq!(c.pixel(5, 95)[3], 255, "bottom left in PDF is bottom left on screen");
        assert_eq!(c.pixel(5, 5)[3], 0);
    }

    #[test]
    fn images_scale_and_smooth() {
        // A 2 x 1 picture, red then blue, drawn 20 x 10.
        let img = Image { width: 2, height: 1, data: vec![255, 0, 0, 255, 0, 0, 255, 255] };
        let mut c = Canvas::new(20, 10).unwrap();
        c.draw_image(&img, &Transform::scale(10.0, 10.0), false, 1.0, None);
        assert_eq!(c.pixel(2, 5), [255, 0, 0, 255]);
        assert_eq!(c.pixel(17, 5), [0, 0, 255, 255]);
        let mut s = Canvas::new(20, 10).unwrap();
        s.draw_image(&img, &Transform::scale(10.0, 10.0), true, 1.0, None);
        let mid = s.pixel(10, 5);
        assert!(mid[0] > 80 && mid[2] > 80, "blended between: {mid:?}");
    }

    #[test]
    fn strokes_draw() {
        let mut c = Canvas::new(50, 50).unwrap();
        let mut p = Path::new();
        p.move_to(5.0, 25.5).line_to(45.0, 25.5);
        c.stroke(&p, &Transform::IDENTITY, &Stroke { width: 3.0, ..Default::default() }, BLACK, None);
        assert_eq!(c.pixel(20, 25)[3], 255);
        assert_eq!(c.pixel(20, 20)[3], 0);
    }

    #[test]
    fn sizes_and_numbers_out_of_reason() {
        assert!(Canvas::new(0, 10).is_none());
        assert!(Canvas::new(100_000, 100_000).is_none());
        let mut c = Canvas::new(10, 10).unwrap();
        let mut p = Path::new();
        p.rect(0.0, 0.0, 5.0, 5.0);
        c.fill(&p, &Transform::new(f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0), FillRule::NonZero, BLACK, None);
        c.draw_image(&Image { width: 1, height: 1, data: vec![] }, &Transform::IDENTITY, true, 1.0, None);
        c.draw_image(&Image { width: 1, height: 1, data: vec![1, 2, 3, 255] }, &Transform::scale(0.0, 0.0), true, 1.0, None);
        assert!(c.data.iter().all(|b| *b == 0));
    }

    #[test]
    fn random_paths_never_panic() {
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let num = |n: &mut dyn FnMut() -> u64| -> f64 {
            match n() % 10 {
                0 => f64::NAN,
                1 => 1e30,
                2 => -1e30,
                _ => (n() % 400) as f64 - 100.0,
            }
        };
        let mut c = Canvas::new(64, 64).unwrap();
        for _ in 0..500 {
            let mut p = Path::new();
            for _ in 0..(next() % 12) {
                match next() % 4 {
                    0 => p.move_to(num(&mut next), num(&mut next)),
                    1 => p.line_to(num(&mut next), num(&mut next)),
                    2 => p.cubic_to(num(&mut next), num(&mut next), num(&mut next), num(&mut next), num(&mut next), num(&mut next)),
                    _ => p.close(),
                };
            }
            let rule = if next() % 2 == 0 { FillRule::NonZero } else { FillRule::EvenOdd };
            c.fill(&p, &Transform::IDENTITY, rule, RED, None);
            let s = Stroke { width: num(&mut next).abs().min(50.0), cap: Cap::Round, join: Join::Round, dash: vec![num(&mut next).abs().min(20.0), 3.0], ..Default::default() };
            c.stroke(&p, &Transform::scale(0.5, 2.0), &s, BLACK, None);
        }
    }
}
