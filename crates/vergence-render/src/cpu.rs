//! The CPU renderer: the reference.
//!
//! Drawing happens in two parts. **Preparing** walks the scene once, on the
//! calling thread: positions become pixels at the scale asked for, clips
//! become a scissor rectangle (and, for rounded or off-pixel clips, a list
//! of shapes to test against), glyphs are fetched from the text cache at
//! the exact pixel size, and each prepared shape is noted against the bands
//! of rows it touches. **Drawing** then fills those bands, [`BAND`] rows
//! each, on as many threads as the machine has cores; a band is touched by
//! one thread only, so no pixel is ever shared.
//!
//! Edges are smoothed by coverage: a straight edge by the exact area of the
//! pixel it covers, a round corner or a line by the distance from the pixel's
//! center. The result does not depend on the number of threads or on which
//! rows were asked for: [`Cpu::render_rows`] redraws a range of rows of an
//! existing picture to exactly what a whole render would put there.

use std::{
    ops::Range,
    sync::{Arc, Mutex},
};

use vergence_scene::{Color, Command, Image, Rect, Scene};
use vergence_text::{Bitmap, FaceId, Fonts};

use crate::{Pixmap, RenderError, pixel_size};

/// Rows in a band, the unit of work a thread takes.
pub const BAND: usize = 32;

/// The order of a pixel's four bytes in memory. Alpha is last in both, and
/// multiplied into the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// Red, green, blue, alpha: a [`Pixmap`], a PNG.
    Rgba,
    /// Blue, green, red, alpha: what display servers call ARGB8888 on a
    /// little-endian machine, the format of a window's buffer.
    Bgra,
}

/// Memory to draw into: `width` x `height` pixels, four bytes each, rows
/// from the top with no gaps. A window's buffer, drawn in place.
pub struct Target<'a> {
    pub data: &'a mut [u8],
    pub width: u32,
    pub height: u32,
    pub order: Order,
}

/// A rectangle in pixels with rounded corners (radius 0: square).
#[derive(Debug, Clone, Copy)]
pub(crate) struct RRect {
    pub(crate) x0: f32,
    pub(crate) y0: f32,
    pub(crate) x1: f32,
    pub(crate) y1: f32,
    pub(crate) radius: f32,
}

/// How much of pixel `p` (from `p` to `p + 1`) lies between `lo` and `hi`.
fn edge(lo: f32, hi: f32, p: i32) -> f32 {
    ((p as f32 + 1.0).min(hi) - (p as f32).max(lo)).clamp(0.0, 1.0)
}

impl RRect {
    pub(crate) fn device(r: &Rect, scale: f32, radius: f32) -> RRect {
        let (x0, y0, x1, y1) = (r.x * scale, r.y * scale, (r.x + r.w) * scale, (r.y + r.h) * scale);
        RRect { x0, y0, x1, y1, radius: (radius * scale).min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0) }
    }

    /// The same shape drawn in by `by` on every side (None if nothing is left).
    fn inset(&self, by: f32) -> Option<RRect> {
        let r = RRect {
            x0: self.x0 + by,
            y0: self.y0 + by,
            x1: self.x1 - by,
            y1: self.y1 - by,
            radius: (self.radius - by).max(0.0),
        };
        (r.x1 > r.x0 && r.y1 > r.y0).then_some(r)
    }

    /// Whether pixel row `y` (or column `x`) crosses a rounded corner.
    fn corner_row(&self, y: i32) -> bool {
        self.radius > 0.0 && ((y as f32) < self.y0 + self.radius || (y + 1) as f32 > self.y1 - self.radius)
    }

    fn corner_col(&self, x: i32) -> bool {
        self.radius > 0.0 && ((x as f32) < self.x0 + self.radius || (x + 1) as f32 > self.x1 - self.radius)
    }

    /// How much of the pixel at `x`, `y` the shape covers, 0 to 1.
    fn cover(&self, x: i32, y: i32) -> f32 {
        if !(self.corner_row(y) && self.corner_col(x)) {
            return edge(self.x0, self.x1, x) * edge(self.y0, self.y1, y);
        }
        // In a corner: the distance from the pixel's center to the round edge.
        let (cx, cy) = ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0);
        let qx = (x as f32 + 0.5 - cx).abs() - ((self.x1 - self.x0) / 2.0 - self.radius);
        let qy = (y as f32 + 0.5 - cy).abs() - ((self.y1 - self.y0) / 2.0 - self.radius);
        let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - self.radius;
        (0.5 - d).clamp(0.0, 1.0)
    }

    /// The pixels it can touch: x0, y0, x1, y1 (the last two not included).
    pub(crate) fn bounds(&self) -> [i32; 4] {
        [self.x0.floor() as i32, self.y0.floor() as i32, self.x1.ceil() as i32, self.y1.ceil() as i32]
    }
}

pub(crate) enum Shape {
    Fill(RRect),
    /// The outer shape less the inner one (None: the outline is the whole shape).
    Stroke(RRect, Option<RRect>),
    /// A line from a to b, `half` its width.
    Line {
        ax: f32,
        ay: f32,
        bx: f32,
        by: f32,
        half: f32,
    },
    /// A glyph's coverage with its top-left pixel at x, y, and what each
    /// coverage value paints for its ink (textlook.rs; the GPU works it out
    /// in its shader instead).
    Glyph {
        x: i32,
        y: i32,
        bitmap: Arc<Bitmap>,
        curve: Arc<[u8; 256]>,
    },
    Image {
        to: RRect,
        image: usize,
        opacity: f32,
    },
    /// A grid of cells left as it is in the scene (the command at this
    /// place in its list), for a renderer that makes its own shapes of it:
    /// the GPU's. Its `bounds` are the clip's, not the grid's.
    Cells(usize),
}

pub(crate) struct Prim {
    pub(crate) shape: Shape,
    /// Not multiplied by alpha.
    pub(crate) color: [u8; 4],
    /// The pixels it may touch, inside its clip: x0, y0, x1, y1.
    pub(crate) bounds: [i32; 4],
    /// Which list of clip shapes it is tested against (0: none).
    pub(crate) chain: u32,
}

pub(crate) fn intersect(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]
}

/// A color's bytes in the target's order (alpha not multiplied in).
fn straight(c: &Color, order: Order) -> [u8; 4] {
    match order {
        Order::Rgba => [c.r, c.g, c.b, c.a],
        Order::Bgra => [c.b, c.g, c.r, c.a],
    }
}

/// a * b / 255, rounded.
fn mul255(a: u32, b: u32) -> u32 {
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}

/// Puts color `c` (alpha not multiplied in) over one pixel, at `cover` (0 to 255).
#[inline]
fn blend(dst: &mut [u8], c: [u8; 4], cover: u32) {
    let a = mul255(c[3] as u32, cover);
    if a == 0 {
        return;
    }
    if a == 255 {
        dst[..4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        return;
    }
    let keep = 255 - a;
    for i in 0..3 {
        dst[i] = (mul255(c[i] as u32, a) + mul255(dst[i] as u32, keep)).min(255) as u8;
    }
    dst[3] = (a + mul255(dst[3] as u32, keep)).min(255) as u8;
}

fn to255(cover: f32) -> u32 {
    (cover.clamp(0.0, 1.0) * 255.0 + 0.5) as u32
}

/// The same color over the pixels x0..x1 of a row.
fn span(row: &mut [u8], x0: i32, x1: i32, c: [u8; 4], cover: u32) {
    if x0 >= x1 || cover == 0 {
        return;
    }
    let pixels = &mut row[x0 as usize * 4..x1 as usize * 4];
    if cover == 255 && c[3] == 255 {
        let solid = [c[0], c[1], c[2], 255];
        for p in pixels.chunks_exact_mut(4) {
            p.copy_from_slice(&solid);
        }
    } else {
        for p in pixels.chunks_exact_mut(4) {
            blend(p, c, cover);
        }
    }
}

pub(crate) struct Prepared<'a> {
    pub(crate) prims: Vec<Prim>,
    /// Lists of clip shapes; `chains[0]` is empty.
    pub(crate) chains: Vec<Vec<RRect>>,
    /// The scene's fonts on this machine (None: it has none such).
    pub(crate) faces: Vec<Option<FaceId>>,
    /// For each band, the shapes that touch it, in drawing order.
    bands: Vec<Vec<u32>>,
    pub(crate) images: &'a [std::sync::Arc<Image>],
    order: Order,
}

/// `banded`: note each shape against the bands of rows it touches, which
/// only this renderer needs (the GPU's draws the shapes in their order).
pub(crate) fn prepare<'a>(
    scene: &'a Scene,
    fonts: &mut Fonts,
    scale: f32,
    w: u32,
    h: u32,
    order: Order,
    banded: bool,
) -> Prepared<'a> {
    let faces: Vec<Option<FaceId>> = scene.fonts.iter().map(|f| fonts.face(&f.family, f.weight, f.italic)).collect();
    let whole = [0, 0, w as i32, h as i32];
    let mut prims: Vec<Prim> = Vec::with_capacity(scene.commands.len());
    let mut chains: Vec<Vec<RRect>> = vec![Vec::new()];
    // Each ink's coverage table (textlook.rs), by its luminance byte, made once a frame.
    let (gamma, contrast) = crate::textlook::look();
    let mut curves: Vec<Option<Arc<[u8; 256]>>> = vec![None; 256];
    let mut curve = |c: &Color| -> Arc<[u8; 256]> {
        let l = crate::textlook::luma_byte(c.r, c.g, c.b);
        curves[l].get_or_insert_with(|| Arc::new(crate::textlook::table(l, gamma, contrast))).clone()
    };
    // The scissor rectangle and the chain in force, one entry per open clip.
    let mut clips: Vec<([i32; 4], u32)> = vec![(whole, 0)];
    for (at, command) in scene.commands.iter().enumerate() {
        let (scissor, chain) = *clips.last().expect("the outermost clip is never popped");
        let mut add = |shape: Shape, color: [u8; 4], bounds: [i32; 4]| {
            let bounds = intersect(bounds, scissor);
            if bounds[0] < bounds[2] && bounds[1] < bounds[3] && color[3] != 0 {
                prims.push(Prim { shape, color, bounds, chain });
            }
        };
        match command {
            Command::Fill { rect, radius, color } => {
                let r = RRect::device(rect, scale, *radius);
                add(Shape::Fill(r), straight(color, order), r.bounds());
            }
            Command::Stroke { rect, radius, width, color } => {
                let r = RRect::device(rect, scale, *radius);
                add(Shape::Stroke(r, r.inset(width * scale)), straight(color, order), r.bounds());
            }
            Command::Line { from, to, width, color } => {
                let (ax, ay, bx, by, half) =
                    (from.x * scale, from.y * scale, to.x * scale, to.y * scale, width * scale / 2.0);
                let bounds = [
                    (ax.min(bx) - half).floor() as i32 - 1,
                    (ay.min(by) - half).floor() as i32 - 1,
                    (ax.max(bx) + half).ceil() as i32 + 1,
                    (ay.max(by) + half).ceil() as i32 + 1,
                ];
                add(Shape::Line { ax, ay, bx, by, half }, straight(color, order), bounds);
            }
            Command::Glyphs { font, size, color, origin, glyphs } => {
                // A font this machine does not have draws nothing: another
                // font's glyph numbers would be other characters.
                let Some(face) = faces[*font as usize] else { continue };
                // The instance for this size (optical sizes), as layout measured it.
                let face = fonts.sized(face, *size);
                for g in glyphs {
                    let x = (origin.x + g.x) * scale;
                    let (mut whole_x, mut sub) = (x.floor(), ((x - x.floor()) * 4.0).round() as u8);
                    if sub == 4 {
                        whole_x += 1.0;
                        sub = 0;
                    }
                    // Baselines sit on whole pixels, so text is as sharp as it can be.
                    let baseline = ((origin.y + g.y) * scale).round();
                    let Some(bitmap) = fonts.raster(face, g.id, size * scale, sub) else { continue };
                    let (px, py) = (whole_x as i32 + bitmap.left, baseline as i32 + bitmap.top);
                    let bounds = [px, py, px + bitmap.width as i32, py + bitmap.height as i32];
                    add(Shape::Glyph { x: px, y: py, bitmap, curve: curve(color) }, straight(color, order), bounds);
                }
            }
            Command::Image { image, rect, opacity } => {
                let to = RRect::device(rect, scale, 0.0);
                add(
                    Shape::Image { to, image: *image as usize, opacity: *opacity },
                    [255, 255, 255, to255(*opacity) as u8],
                    to.bounds(),
                );
            }
            Command::PushClip { rect, radius } => {
                let r = RRect::device(rect, scale, *radius);
                let aligned = r.radius == 0.0 && [r.x0, r.y0, r.x1, r.y1].iter().all(|v| (v - v.round()).abs() < 1e-3);
                let chain = if aligned {
                    chain
                } else {
                    let mut list = chains[chain as usize].clone();
                    list.push(r);
                    chains.push(list);
                    chains.len() as u32 - 1
                };
                let bounds = if aligned {
                    [r.x0.round() as i32, r.y0.round() as i32, r.x1.round() as i32, r.y1.round() as i32]
                } else {
                    r.bounds()
                };
                clips.push((intersect(scissor, bounds), chain));
            }
            Command::PopClip => {
                // A valid scene never pops more than it pushed.
                if clips.len() > 1 {
                    clips.pop();
                }
            }
            Command::Cells { .. } if !banded => {
                // The GPU's renderer makes its instances from the cells themselves.
                if scissor[0] < scissor[2] && scissor[1] < scissor[3] {
                    prims.push(Prim { shape: Shape::Cells(at), color: [0; 4], bounds: scissor, chain });
                }
            }
            Command::Cells { origin, cell_w, cell_h, baseline, size, columns, cells } => {
                // The same shapes, in the same order and by the same arithmetic, as the
                // fills and glyph runs this command stands for: a row's grounds
                // (neighbors of one color as one rectangle), its underlines, its glyphs.
                let columns = *columns as usize;
                for (r, row) in cells.chunks_exact(columns).enumerate() {
                    let top = origin.y + r as f32 * cell_h;
                    let mut x = 0;
                    while x < columns {
                        let ground = row[x].ground;
                        let mut end = x + 1;
                        while end < columns && row[end].ground == ground {
                            end += 1;
                        }
                        if ground.a != 0 {
                            let shape = RRect::device(
                                &Rect::new(origin.x + x as f32 * cell_w, top, (end - x) as f32 * cell_w, *cell_h),
                                scale,
                                0.0,
                            );
                            add(Shape::Fill(shape), straight(&ground, order), shape.bounds());
                        }
                        x = end;
                    }
                    let lined = |c: &vergence_scene::Cell| c.flags & vergence_scene::CELL_UNDERLINE != 0;
                    let mut x = 0;
                    while x < columns {
                        let mut end = x + 1;
                        if lined(&row[x]) {
                            while end < columns && lined(&row[end]) && row[end].ink == row[x].ink {
                                end += 1;
                            }
                            let shape = RRect::device(
                                &Rect::new(
                                    origin.x + x as f32 * cell_w,
                                    top + cell_h - 2.0,
                                    (end - x) as f32 * cell_w,
                                    1.0,
                                ),
                                scale,
                                0.0,
                            );
                            add(Shape::Fill(shape), straight(&row[x].ink, order), shape.bounds());
                        }
                        x = end;
                    }
                    // Baselines sit on whole pixels, as a glyph run's do.
                    let line = ((top + baseline) * scale).round() as i32;
                    for (c, cell) in row.iter().enumerate() {
                        if cell.flags & vergence_scene::CELL_GLYPH == 0 {
                            continue;
                        }
                        let Some(face) = faces[cell.font as usize] else { continue };
                        let face = fonts.sized(face, *size);
                        let x = (origin.x + c as f32 * cell_w) * scale;
                        let (mut whole_x, mut sub) = (x.floor(), ((x - x.floor()) * 4.0).round() as u8);
                        if sub == 4 {
                            whole_x += 1.0;
                            sub = 0;
                        }
                        let Some(bitmap) = fonts.raster(face, cell.glyph, size * scale, sub) else { continue };
                        let (px, py) = (whole_x as i32 + bitmap.left, line + bitmap.top);
                        let bounds = [px, py, px + bitmap.width as i32, py + bitmap.height as i32];
                        add(
                            Shape::Glyph { x: px, y: py, bitmap, curve: curve(&cell.ink) },
                            straight(&cell.ink, order),
                            bounds,
                        );
                    }
                }
            }
        }
    }
    let mut bands = vec![Vec::new(); if banded { (h as usize).div_ceil(BAND) } else { 0 }];
    for (i, p) in prims.iter().enumerate().filter(|_| banded) {
        for band in p.bounds[1] as usize / BAND..=(p.bounds[3] as usize - 1) / BAND {
            bands[band].push(i as u32);
        }
    }
    Prepared { prims, chains, faces, bands, images: &scene.images, order }
}

/// How much of a pixel the shapes of a clip chain let through.
fn chain_cover(chain: &[RRect], x: i32, y: i32) -> f32 {
    chain.iter().map(|r| r.cover(x, y)).product()
}

impl Prepared<'_> {
    /// Draws one shape into one row.
    fn draw(&self, p: &Prim, row: &mut [u8], y: i32) {
        let (x0, x1) = (p.bounds[0], p.bounds[2]);
        let chain = &self.chains[p.chain as usize];
        // A shape given by a coverage function, pixel by pixel, from xa to xb.
        let each = |row: &mut [u8], xa: i32, xb: i32, cover: &dyn Fn(i32) -> f32| {
            for x in xa.max(x0)..xb.min(x1) {
                // Clamped before a clip is applied: a line's coverage runs past 1 inside it, and
                // times a clip's 0.3 that would be more than 0.3 (found by the GPU renderer
                // disagreeing).
                let mut c = cover(x).clamp(0.0, 1.0);
                if c > 0.0 && !chain.is_empty() {
                    c *= chain_cover(chain, x, y);
                }
                let c = to255(c);
                if c != 0 {
                    blend(&mut row[x as usize * 4..], p.color, c);
                }
            }
        };
        match &p.shape {
            Shape::Fill(r) => {
                if !chain.is_empty() {
                    return each(row, x0, x1, &|x| r.cover(x, y));
                }
                let rows = edge(r.y0, r.y1, y);
                // The columns that are covered in full across this row, and the edges beside them.
                let inset = if r.corner_row(y) { r.radius } else { 0.0 };
                let (full0, full1) =
                    (((r.x0 + inset).ceil() as i32).clamp(x0, x1), ((r.x1 - inset).floor() as i32).clamp(x0, x1));
                if full0 >= full1 {
                    return each(row, x0, x1, &|x| r.cover(x, y));
                }
                each(row, x0, full0, &|x| r.cover(x, y));
                span(row, full0, full1, p.color, to255(rows));
                each(row, full1, x1, &|x| r.cover(x, y));
            }
            Shape::Stroke(outer, inner) => {
                let cover = |x: i32| outer.cover(x, y) - inner.map_or(0.0, |i| i.cover(x, y));
                match inner {
                    // A row through the middle: only the two sides have anything.
                    Some(i) if !i.corner_row(y) && (y as f32) >= i.y0 && (y + 1) as f32 <= i.y1 => {
                        each(row, x0, i.x0.ceil() as i32 + 1, &cover);
                        each(row, (i.x1.floor() as i32 - 1).max(i.x0.ceil() as i32 + 1), x1, &cover);
                    }
                    _ => each(row, x0, x1, &cover),
                }
            }
            Shape::Line { ax, ay, bx, by, half } => {
                let (dx, dy) = (bx - ax, by - ay);
                let len2 = dx * dx + dy * dy;
                each(row, x0, x1, &|x| {
                    let (px, py) = (x as f32 + 0.5 - ax, y as f32 + 0.5 - ay);
                    let t = if len2 > 0.0 { ((px * dx + py * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                    let (ex, ey) = (px - dx * t, py - dy * t);
                    0.5 - ((ex * ex + ey * ey).sqrt() - half)
                });
            }
            Shape::Glyph { x: gx, y: gy, bitmap, curve } => {
                let w = bitmap.width as usize;
                let line = &bitmap.alpha[(y - gy) as usize * w..][..w];
                for x in x0.max(*gx)..x1.min(gx + w as i32) {
                    let mut a = curve[line[(x - gx) as usize] as usize] as u32;
                    if a != 0 && !chain.is_empty() {
                        a = to255(a as f32 / 255.0 * chain_cover(chain, x, y));
                    }
                    if a != 0 {
                        blend(&mut row[x as usize * 4..], p.color, a);
                    }
                }
            }
            // Only a renderer that asked for grids whole is given them.
            Shape::Cells(_) => {}
            Shape::Image { to, image, opacity } => {
                let im = &self.images[*image];
                let (iw, ih) = (im.width as usize, im.height as usize);
                // A picture's bytes are red first; the target's may be blue first.
                let (first, third) = if self.order == Order::Rgba { (0, 2) } else { (2, 0) };
                let texel = |tx: usize, ty: usize| -> [f32; 4] {
                    let i = (ty * iw + tx) * 4;
                    [im.rgba[i + first] as f32, im.rgba[i + 1] as f32, im.rgba[i + third] as f32, im.rgba[i + 3] as f32]
                };
                // The picture's row under this pixel row, between two of its own rows.
                let v = ((y as f32 + 0.5 - to.y0) / (to.y1 - to.y0) * ih as f32 - 0.5).clamp(0.0, ih as f32 - 1.0);
                let (v0, vf) = (v.floor() as usize, v - v.floor());
                let v1 = (v0 + 1).min(ih - 1);
                for x in x0..x1 {
                    let mut cover = to.cover(x, y) * opacity;
                    if cover > 0.0 && !chain.is_empty() {
                        cover *= chain_cover(chain, x, y);
                    }
                    if cover <= 0.0 {
                        continue;
                    }
                    let u = ((x as f32 + 0.5 - to.x0) / (to.x1 - to.x0) * iw as f32 - 0.5).clamp(0.0, iw as f32 - 1.0);
                    let (u0, uf) = (u.floor() as usize, u - u.floor());
                    let u1 = (u0 + 1).min(iw - 1);
                    let (a, b, c, d) = (texel(u0, v0), texel(u1, v0), texel(u0, v1), texel(u1, v1));
                    // Mixed with alpha multiplied in, so a clear texel's color does not bleed.
                    let mut mixed = [0.0f32; 4];
                    for (t, wgt) in
                        [(a, (1.0 - uf) * (1.0 - vf)), (b, uf * (1.0 - vf)), (c, (1.0 - uf) * vf), (d, uf * vf)]
                    {
                        let alpha = t[3] / 255.0 * wgt;
                        mixed[0] += t[0] * alpha;
                        mixed[1] += t[1] * alpha;
                        mixed[2] += t[2] * alpha;
                        mixed[3] += alpha;
                    }
                    if mixed[3] <= 0.0 {
                        continue;
                    }
                    let color = [
                        (mixed[0] / mixed[3] + 0.5) as u8,
                        (mixed[1] / mixed[3] + 0.5) as u8,
                        (mixed[2] / mixed[3] + 0.5) as u8,
                        255,
                    ];
                    blend(&mut row[x as usize * 4..], color, to255(mixed[3] * cover));
                }
            }
        }
    }

    /// Clears and draws the rows `rows` of band `band`, whose pixels are `pixels`.
    fn draw_band(&self, band: usize, pixels: &mut [u8], stride: usize, rows: &Range<usize>, background: [u8; 4]) {
        let first = band * BAND;
        for (i, row) in pixels.chunks_exact_mut(stride).enumerate() {
            let y = first + i;
            if !rows.contains(&y) {
                continue;
            }
            for p in row.chunks_exact_mut(4) {
                p.copy_from_slice(&background);
            }
            for index in &self.bands[band] {
                let p = &self.prims[*index as usize];
                if (p.bounds[1]..p.bounds[3]).contains(&(y as i32)) {
                    self.draw(p, row, y as i32);
                }
            }
        }
    }
}

/// The CPU renderer.
pub struct Cpu {
    threads: usize,
}

impl Default for Cpu {
    fn default() -> Self {
        Cpu::new()
    }
}

impl Cpu {
    /// Using every core the machine has.
    pub fn new() -> Cpu {
        Cpu { threads: std::thread::available_parallelism().map_or(1, usize::from) }
    }

    pub fn with_threads(threads: usize) -> Cpu {
        Cpu { threads: threads.max(1) }
    }

    pub fn threads(&self) -> usize {
        self.threads
    }

    /// The scene at `scale` pixels to the reference unit, over `background`.
    pub fn render(
        &self,
        scene: &Scene,
        fonts: &mut Fonts,
        scale: f32,
        background: Color,
    ) -> Result<Pixmap, RenderError> {
        let (w, h) = pixel_size(scene, scale)?;
        let mut pixmap = Pixmap::new(w, h);
        self.render_rows(scene, fonts, scale, background, &mut pixmap, 0..h)?;
        Ok(pixmap)
    }

    /// Redraws the pixel rows `rows` of `into`, a picture of this scene's
    /// size at this scale, leaving the others as they are. What it draws is
    /// exactly what [`Cpu::render`] would put in those rows.
    pub fn render_rows(
        &self,
        scene: &Scene,
        fonts: &mut Fonts,
        scale: f32,
        background: Color,
        into: &mut Pixmap,
        rows: Range<u32>,
    ) -> Result<(), RenderError> {
        let target = Target { width: into.width, height: into.height, data: &mut into.data, order: Order::Rgba };
        self.render_target(scene, fonts, scale, background, target, rows)
    }

    /// As [`Cpu::render_rows`], into any memory of the right size, in either
    /// byte order: a window's buffer is drawn in place, with no copy.
    pub fn render_target(
        &self,
        scene: &Scene,
        fonts: &mut Fonts,
        scale: f32,
        background: Color,
        into: Target<'_>,
        rows: Range<u32>,
    ) -> Result<(), RenderError> {
        let (w, h) = pixel_size(scene, scale)?;
        if (into.width, into.height) != (w, h) || into.data.len() != w as usize * h as usize * 4 {
            return Err(RenderError::Size);
        }
        scene.validate()?;
        let rows = rows.start.min(h) as usize..rows.end.min(h) as usize;
        if rows.is_empty() {
            return Ok(());
        }
        let prepared = prepare(scene, fonts, scale, w, h, into.order, true);
        let stride = w as usize * 4;
        let a = background.a as u32;
        let ground = straight(&background, into.order);
        let background = [
            mul255(ground[0] as u32, a) as u8,
            mul255(ground[1] as u32, a) as u8,
            mul255(ground[2] as u32, a) as u8,
            background.a,
        ];
        let work: Vec<(usize, &mut [u8])> = into
            .data
            .chunks_mut(BAND * stride)
            .enumerate()
            .filter(|(band, _)| band * BAND < rows.end && (band + 1) * BAND > rows.start)
            .collect();
        let threads = self.threads.min(work.len());
        let queue = Mutex::new(work);
        let run = || {
            loop {
                let next = queue.lock().unwrap_or_else(|e| e.into_inner()).pop();
                let Some((band, pixels)) = next else { break };
                prepared.draw_band(band, pixels, stride, &rows, background);
            }
        };
        if threads <= 1 {
            run();
        } else {
            std::thread::scope(|s| {
                for _ in 0..threads {
                    s.spawn(&run);
                }
            });
        }
        Ok(())
    }
}
