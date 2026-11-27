//! The transport glyphs: previous, play, pause, next, stop. The Avalonia
//! AlbumWall's (BuildTransport), point for point: paths in a small box,
//! stroked 1.6 wide, the triangles filled. One copy, drawn by AlbumWall's play
//! bar and by the server on a card (Now Playing's), so the two never differ.

use vergence_scene::{Color, Point, Rect, Scene};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Previous,
    Play,
    Pause,
    Next,
    Stop,
}

/// A triangle `w` wide and `h` tall pointing right (or left), its middle at
/// `cy`: filled in thin bands, as every renderer fills rectangles alike.
pub fn triangle(scene: &mut Scene, x: f32, cy: f32, w: f32, h: f32, right: bool, c: Color) {
    let bands = (h * 2.0).max(8.0) as usize;
    for k in 0..bands {
        let t = (k as f32 + 0.5) / bands as f32;
        let len = w * (1.0 - (2.0 * t - 1.0).abs());
        let y = cy - h / 2.0 + t * h;
        let x0 = if right { x } else { x + w - len };
        scene.fill(Rect::new(x0, y - h / bands as f32 / 2.0, len, h / bands as f32 + 0.05), c);
    }
}

/// The glyph's box (points), centered in whatever holds it.
pub fn size(g: Glyph) -> (f32, f32) {
    match g {
        Glyph::Previous | Glyph::Next => (10.0, 12.0),
        Glyph::Play => (12.0, 14.0),
        Glyph::Pause => (11.0, 14.0),
        Glyph::Stop => (11.0, 11.0),
    }
}

/// Glyph `g` in `c`, centered in `r`.
pub fn draw(scene: &mut Scene, g: Glyph, r: Rect, c: Color) {
    draw_at(scene, g, r, c, 1.0);
}

/// Glyph `g` in `c`, centered in `r`, `f` times its size (a badge's: smaller).
pub fn draw_at(scene: &mut Scene, g: Glyph, r: Rect, c: Color, f: f32) {
    let (bw, bh) = size(g);
    let (bw, bh) = (bw * f, bh * f);
    let (x0, y0) = (r.x + (r.w - bw) / 2.0, r.y + (r.h - bh) / 2.0);
    let line = |scene: &mut Scene, a: (f32, f32), b: (f32, f32)| {
        scene.line(Point::new(x0 + a.0 * f, y0 + a.1 * f), Point::new(x0 + b.0 * f, y0 + b.1 * f), 1.6 * f, c);
    };
    match g {
        Glyph::Previous => {
            line(scene, (1.0, 0.0), (1.0, 12.0));
            triangle(scene, x0 + f, y0 + 6.0 * f, 8.0 * f, 12.0 * f, false, c);
        }
        Glyph::Next => {
            line(scene, (9.0, 0.0), (9.0, 12.0));
            triangle(scene, x0 + f, y0 + 6.0 * f, 8.0 * f, 12.0 * f, true, c);
        }
        Glyph::Play => triangle(scene, x0 + 2.0 * f, y0 + 7.0 * f, 10.0 * f, 14.0 * f, true, c),
        Glyph::Pause => {
            line(scene, (2.0, 0.0), (2.0, 14.0));
            line(scene, (9.0, 0.0), (9.0, 14.0));
        }
        Glyph::Stop => {
            scene.fill_rounded(Rect::new(x0, y0, bw, bh), 1.5 * f, c);
        }
    }
}
