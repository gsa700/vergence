//! The CPU renderer, pixel by pixel.

use vergence_render::{Cpu, Order, Pixmap, RenderError, Target, text};
use vergence_scene::{CELL_GLYPH, CELL_UNDERLINE, Cell, Color, Glyph, Image, Point, Rect, Scene};
use vergence_text::{Fonts, db::FontDb};

const BLACK: Color = Color::rgb(0, 0, 0);
const CLEAR: Color = Color::rgba(0, 0, 0, 0);
const RED: Color = Color::rgb(255, 0, 0);
const WHITE: Color = Color::rgb(255, 255, 255);

/// No fonts: for scenes without text.
fn none() -> Fonts {
    Fonts::new(FontDb::default())
}

fn draw(scene: &Scene, scale: f32, background: Color) -> Pixmap {
    Cpu::new().render(scene, &mut none(), scale, background).expect("the scene renders")
}

fn near(a: u8, b: u8) -> bool {
    a.abs_diff(b) <= 2
}

#[test]
fn a_fill_covers_exactly_its_pixels() {
    let mut s = Scene::new(10.0, 10.0);
    s.fill(Rect::new(2.0, 3.0, 4.0, 5.0), RED);
    let p = draw(&s, 1.0, BLACK);
    assert_eq!((p.width, p.height), (10, 10));
    for (x, y, inside) in [
        (2, 3, true),
        (5, 7, true),
        (1, 3, false),
        (2, 2, false),
        (6, 7, false),
        (5, 8, false),
        (0, 0, false),
        (9, 9, false),
    ] {
        assert_eq!(p.pixel(x, y), if inside { [255, 0, 0, 255] } else { [0, 0, 0, 255] }, "{x},{y}");
    }
}

#[test]
fn an_edge_between_pixels_is_half_covered() {
    let mut s = Scene::new(10.0, 10.0);
    s.fill(Rect::new(2.5, 2.0, 3.0, 2.5), WHITE);
    let p = draw(&s, 1.0, BLACK);
    assert!(
        near(p.pixel(2, 2)[0], 128) && near(p.pixel(5, 2)[0], 128),
        "left and right edges: {:?} {:?}",
        p.pixel(2, 2),
        p.pixel(5, 2)
    );
    assert_eq!(p.pixel(3, 2), [255, 255, 255, 255]);
    assert!(near(p.pixel(3, 4)[0], 128), "bottom edge: {:?}", p.pixel(3, 4));
    assert!(near(p.pixel(2, 4)[0], 64), "the corner is a quarter: {:?}", p.pixel(2, 4));
    assert_eq!(p.pixel(1, 2), [0, 0, 0, 255]);
}

#[test]
fn the_scale_decides_the_pixels() {
    let mut s = Scene::new(10.0, 10.0);
    s.fill(Rect::new(2.5, 2.5, 3.0, 3.0), RED);
    let p = draw(&s, 2.0, BLACK);
    assert_eq!((p.width, p.height), (20, 20));
    // 2.5 units is pixel 5 exactly at this scale: a sharp edge.
    assert_eq!(p.pixel(5, 5), [255, 0, 0, 255]);
    assert_eq!(p.pixel(4, 5), [0, 0, 0, 255]);
    assert_eq!(p.pixel(10, 10), [255, 0, 0, 255]);
    assert_eq!(p.pixel(11, 10), [0, 0, 0, 255]);
    let p = draw(&s, 2.25, BLACK);
    assert_eq!((p.width, p.height), (23, 23), "every started pixel counts");
}

#[test]
fn colors_blend_over_what_is_there() {
    let mut s = Scene::new(4.0, 4.0);
    s.fill(Rect::new(0.0, 0.0, 4.0, 4.0), Color::rgba(255, 255, 255, 128));
    let p = draw(&s, 1.0, BLACK);
    assert!(near(p.pixel(1, 1)[0], 128) && p.pixel(1, 1)[3] == 255, "{:?}", p.pixel(1, 1));
    // Over nothing: alpha is multiplied into the color.
    let p = draw(&s, 1.0, CLEAR);
    assert_eq!(p.pixel(1, 1), [128, 128, 128, 128]);
    assert_eq!(p.straight_rgba()[..4], [255, 255, 255, 128]);
    // A second layer over the first.
    s.fill(Rect::new(0.0, 0.0, 2.0, 4.0), Color::rgba(255, 0, 0, 128));
    let p = draw(&s, 1.0, BLACK);
    let px = p.pixel(0, 0);
    assert!(near(px[0], 192) && near(px[1], 64) && near(px[2], 64), "{px:?}");
}

#[test]
fn a_circle_is_round() {
    let mut s = Scene::new(20.0, 20.0);
    s.fill_rounded(Rect::new(0.0, 0.0, 20.0, 20.0), 10.0, WHITE);
    let p = draw(&s, 1.0, CLEAR);
    assert_eq!(p.pixel(0, 0)[3], 0, "the corner is outside");
    assert_eq!(p.pixel(10, 10), [255, 255, 255, 255]);
    assert!(p.pixel(10, 0)[3] > 240 && p.pixel(0, 10)[3] > 240, "the top and the side touch the edge");
    let area: f64 = p.data.chunks_exact(4).map(|px| px[3] as f64 / 255.0).sum();
    assert!((area - 314.16).abs() < 4.0, "the area of a circle of radius 10: {area}");
    for y in 0..20 {
        for x in 0..20 {
            assert_eq!(p.pixel(x, y), p.pixel(19 - x, y), "mirror at {x},{y}");
            assert_eq!(p.pixel(x, y), p.pixel(x, 19 - y), "flip at {x},{y}");
        }
    }
    // A radius larger than the shape allows is the largest that fits.
    let mut s = Scene::new(20.0, 20.0);
    s.fill_rounded(Rect::new(0.0, 0.0, 20.0, 20.0), 500.0, WHITE);
    assert_eq!(draw(&s, 1.0, CLEAR), p);
}

#[test]
fn a_stroke_is_only_the_outline() {
    let mut s = Scene::new(20.0, 20.0);
    s.stroke(Rect::new(2.0, 2.0, 16.0, 16.0), 0.0, 2.0, RED);
    let p = draw(&s, 1.0, BLACK);
    for (x, y, on) in [
        (2, 2, true),
        (3, 10, true),
        (4, 10, false),
        (10, 10, false),
        (15, 10, false),
        (16, 10, true),
        (17, 10, true),
        (18, 10, false),
        (10, 3, true),
        (10, 4, false),
        (10, 17, true),
        (10, 18, false),
        (1, 1, false),
    ] {
        assert_eq!(p.pixel(x, y), if on { [255, 0, 0, 255] } else { [0, 0, 0, 255] }, "{x},{y}");
    }
    // Wider than the shape: all of it.
    let mut s = Scene::new(6.0, 6.0);
    s.stroke(Rect::new(1.0, 1.0, 4.0, 4.0), 0.0, 9.0, RED);
    let p = draw(&s, 1.0, BLACK);
    assert_eq!(p.pixel(3, 3), [255, 0, 0, 255]);
    assert_eq!(p.pixel(0, 0), [0, 0, 0, 255]);
    // A rounded outline keeps its corners clear, inside and out.
    let mut s = Scene::new(40.0, 40.0);
    s.stroke(Rect::new(0.0, 0.0, 40.0, 40.0), 12.0, 3.0, WHITE);
    let p = draw(&s, 1.0, CLEAR);
    assert_eq!(p.pixel(0, 0)[3], 0);
    assert_eq!(p.pixel(20, 1)[3], 255);
    assert_eq!(p.pixel(20, 20)[3], 0);
    assert_eq!(p.pixel(6, 6)[3], 0, "inside the outline's corner");
}

#[test]
fn a_clip_keeps_drawing_inside() {
    let mut s = Scene::new(20.0, 20.0);
    s.push_clip(Rect::new(5.0, 5.0, 10.0, 10.0), 0.0);
    s.fill(Rect::new(0.0, 0.0, 20.0, 20.0), RED);
    s.push_clip(Rect::new(0.0, 0.0, 8.0, 8.0), 0.0);
    s.fill(Rect::new(0.0, 0.0, 20.0, 20.0), WHITE);
    s.pop_clip();
    s.pop_clip();
    s.fill(Rect::new(18.0, 18.0, 2.0, 2.0), WHITE);
    let p = draw(&s, 1.0, BLACK);
    assert_eq!(p.pixel(4, 5), [0, 0, 0, 255]);
    assert_eq!(p.pixel(5, 5), [255, 255, 255, 255], "inside both clips");
    assert_eq!(p.pixel(7, 7), [255, 255, 255, 255]);
    assert_eq!(p.pixel(8, 7), [255, 0, 0, 255], "inside the outer clip only");
    assert_eq!(p.pixel(14, 14), [255, 0, 0, 255]);
    assert_eq!(p.pixel(15, 14), [0, 0, 0, 255]);
    assert_eq!(p.pixel(19, 19), [255, 255, 255, 255], "after the clips are closed");
}

#[test]
fn a_rounded_clip_rounds_what_is_inside() {
    let mut s = Scene::new(20.0, 20.0);
    s.push_clip(Rect::new(0.0, 0.0, 20.0, 20.0), 10.0);
    s.fill(Rect::new(0.0, 0.0, 20.0, 20.0), WHITE);
    s.pop_clip();
    let clipped = draw(&s, 1.0, CLEAR);
    // Other shapes are clipped the same way: a line from corner to corner.
    let mut with_line = Scene::new(20.0, 20.0);
    with_line.push_clip(Rect::new(0.0, 0.0, 20.0, 20.0), 10.0);
    with_line.line(Point::new(0.0, 0.0), Point::new(20.0, 20.0), 2.0, RED);
    with_line.pop_clip();
    let with_line = draw(&with_line, 1.0, CLEAR);
    assert_eq!(with_line.pixel(0, 0)[3], 0, "the line's end in the corner is clipped away");
    assert_eq!(with_line.pixel(1, 1)[3], 0);
    assert_eq!(with_line.pixel(10, 10), [255, 0, 0, 255]);
    let mut round = Scene::new(20.0, 20.0);
    round.fill_rounded(Rect::new(0.0, 0.0, 20.0, 20.0), 10.0, WHITE);
    let round = draw(&round, 1.0, CLEAR);
    for y in 0..20 {
        for x in 0..20 {
            assert_eq!(clipped.pixel(x, y)[3], round.pixel(x, y)[3], "the clipped square is the circle at {x},{y}");
        }
    }
    // A clip edge between pixels is smoothed like any edge.
    let mut s = Scene::new(10.0, 10.0);
    s.push_clip(Rect::new(2.5, 0.0, 5.0, 10.0), 0.0);
    s.fill(Rect::new(0.0, 0.0, 10.0, 10.0), WHITE);
    s.pop_clip();
    let p = draw(&s, 1.0, BLACK);
    assert!(near(p.pixel(2, 5)[0], 128) && p.pixel(3, 5)[0] == 255 && p.pixel(1, 5)[0] == 0, "{:?}", p.pixel(2, 5));
}

#[test]
fn a_line_has_width_and_round_ends() {
    let mut s = Scene::new(12.0, 12.0);
    s.line(Point::new(3.0, 6.0), Point::new(9.0, 6.0), 2.0, WHITE);
    let p = draw(&s, 1.0, BLACK);
    assert_eq!(p.pixel(6, 5), [255, 255, 255, 255]);
    assert_eq!(p.pixel(6, 6), [255, 255, 255, 255]);
    assert_eq!(p.pixel(6, 4), [0, 0, 0, 255]);
    assert_eq!(p.pixel(6, 7), [0, 0, 0, 255]);
    assert!(p.pixel(2, 5)[0] > 0 && p.pixel(9, 6)[0] > 0, "the ends reach past the points by half the width");
    assert_eq!(p.pixel(0, 6), [0, 0, 0, 255]);
    // A diagonal is the same either side of itself.
    let mut s = Scene::new(12.0, 12.0);
    s.line(Point::new(2.0, 2.0), Point::new(10.0, 10.0), 1.5, WHITE);
    let p = draw(&s, 1.0, BLACK);
    assert_eq!(p.pixel(6, 6)[0], 255);
    assert_eq!(p.pixel(6, 9), [0, 0, 0, 255]);
    for y in 0..12 {
        for x in 0..12 {
            assert_eq!(p.pixel(x, y), p.pixel(y, x), "{x},{y}");
        }
    }
    // A line of no length is a dot.
    let mut s = Scene::new(8.0, 8.0);
    s.line(Point::new(4.0, 4.0), Point::new(4.0, 4.0), 4.0, WHITE);
    let p = draw(&s, 1.0, BLACK);
    assert_eq!(p.pixel(4, 4)[0], 255);
    assert_eq!(p.pixel(0, 0)[0], 0);
}

#[test]
fn a_picture_is_stretched_smoothly() {
    let mut s = Scene::new(8.0, 4.0);
    let two = s.image(Image { width: 2, height: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 255] });
    s.draw_image(two, Rect::new(0.0, 0.0, 8.0, 4.0), 1.0);
    let p = draw(&s, 1.0, BLACK);
    assert_eq!(p.pixel(0, 0), [255, 0, 0, 255]);
    assert_eq!(p.pixel(7, 3), [0, 0, 255, 255]);
    let mid = p.pixel(4, 2);
    assert!(mid[0] > 60 && mid[0] < 200 && mid[2] > 60 && mid[2] < 200, "between the two: {mid:?}");
    assert!(p.pixel(2, 0)[0] > p.pixel(5, 0)[0], "red fades to the right");
    // Half as strong.
    let mut s = Scene::new(4.0, 4.0);
    let white = s.image(Image { width: 1, height: 1, rgba: vec![255, 255, 255, 255] });
    s.draw_image(white, Rect::new(1.0, 1.0, 2.0, 2.0), 0.5);
    let p = draw(&s, 1.0, BLACK);
    assert!(near(p.pixel(1, 1)[0], 128), "{:?}", p.pixel(1, 1));
    assert_eq!(p.pixel(0, 0), [0, 0, 0, 255]);
    // A clear texel does not darken its neighbor's color.
    let mut s = Scene::new(8.0, 2.0);
    let half = s.image(Image { width: 2, height: 1, rgba: vec![255, 255, 255, 255, 0, 0, 0, 0] });
    s.draw_image(half, Rect::new(0.0, 0.0, 8.0, 2.0), 1.0);
    let p = draw(&s, 1.0, CLEAR);
    let fading = p.straight_rgba();
    let px = &fading[4 * 4..4 * 4 + 4];
    assert!(px[3] > 30 && px[3] < 220 && px[0] > 250, "white at part strength, not gray: {px:?}");
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn color(&mut self) -> Color {
        let v = self.next();
        Color::rgba(v as u8, (v >> 8) as u8, (v >> 16) as u8, 128 + (v >> 24) as u8 / 2)
    }
}

/// A busy scene of every kind of shape, the same every time.
fn busy() -> Scene {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let mut s = Scene::new(300.0, 200.0);
    let pic = s.image(Image {
        width: 3,
        height: 3,
        rgba: (0..36).map(|i| (i * 7) as u8 | if i % 4 == 3 { 0x80 } else { 0 }).collect(),
    });
    let mut open = 0;
    for i in 0..400 {
        let r = Rect::new(rng.unit() * 320.0 - 20.0, rng.unit() * 220.0 - 20.0, rng.unit() * 90.0, rng.unit() * 60.0);
        let c = rng.color();
        match i % 9 {
            0 | 1 => {
                s.fill(r, c);
            }
            2 | 3 => {
                s.fill_rounded(r, rng.unit() * 20.0, c);
            }
            4 => {
                s.stroke(r, rng.unit() * 12.0, 0.5 + rng.unit() * 4.0, c);
            }
            5 => {
                s.line(Point::new(r.x, r.y), Point::new(r.x + r.w, r.y + r.h), 0.5 + rng.unit() * 5.0, c);
            }
            6 => {
                s.draw_image(pic, r, rng.unit());
            }
            7 if open < 3 => {
                s.push_clip(
                    Rect::new(
                        rng.unit() * 150.0,
                        rng.unit() * 100.0,
                        60.0 + rng.unit() * 150.0,
                        40.0 + rng.unit() * 100.0,
                    ),
                    if i % 2 == 0 { 0.0 } else { rng.unit() * 20.0 },
                );
                open += 1;
            }
            _ if open > 0 => {
                s.pop_clip();
                open -= 1;
            }
            _ => {}
        }
    }
    for _ in 0..open {
        s.pop_clip();
    }
    s
}

#[test]
fn threads_do_not_change_the_picture() {
    let s = busy();
    for scale in [1.0, 2.25] {
        let one = Cpu::with_threads(1).render(&s, &mut none(), scale, BLACK).unwrap();
        assert!(one.data.chunks_exact(4).any(|p| p[..3] != [0, 0, 0]), "something was drawn");
        for threads in [2, 3, 6, 16] {
            let many = Cpu::with_threads(threads).render(&s, &mut none(), scale, BLACK).unwrap();
            assert_eq!(many.hash(), one.hash(), "{threads} threads at scale {scale}");
        }
    }
}

/// The busy scene's picture, the same on every machine and operating system
/// the project builds on: the renderer is plain arithmetic, with nothing
/// left to the platform. (No text in it: fonts differ between machines.)
/// A deliberate change to how shapes are drawn changes these numbers; look
/// at the picture (`vscene`) before replacing them.
#[test]
fn the_same_picture_on_every_machine() {
    let s = busy();
    let got = [1.0, 1.5, 3.0].map(|scale| Cpu::new().render(&s, &mut none(), scale, BLACK).unwrap().hash());
    assert_eq!(got.map(|h| format!("{h:016x}")), GOLDEN.map(str::to_string));
}

const GOLDEN: [&str; 3] = ["79a8d73e8542b732", "00fc47af79ea1d5f", "3307122032bd6942"];

#[test]
fn redrawing_rows_gives_what_a_whole_render_gives() {
    let s = busy();
    let cpu = Cpu::new();
    let whole = cpu.render(&s, &mut none(), 1.5, BLACK).unwrap();
    for rows in [0..1, 31..33, 100..101, 64..96, 7..250, 299..300, 0..300, 290..5000] {
        let mut damaged = whole.clone();
        let stride = damaged.width as usize * 4;
        let end = (rows.end as usize).min(damaged.height as usize);
        damaged.data[rows.start as usize * stride..end * stride].fill(0x5a);
        cpu.render_rows(&s, &mut none(), 1.5, BLACK, &mut damaged, rows.clone()).unwrap();
        assert_eq!(damaged.hash(), whole.hash(), "rows {rows:?}");
    }
    // Rows that were not asked for are left alone.
    let mut marked = whole.clone();
    marked.data[..4].copy_from_slice(&[1, 2, 3, 4]);
    cpu.render_rows(&s, &mut none(), 1.5, BLACK, &mut marked, 50..60).unwrap();
    assert_eq!(marked.pixel(0, 0), [1, 2, 3, 4]);
}

#[test]
fn a_window_buffer_is_drawn_in_place_with_blue_first() {
    let s = busy();
    let cpu = Cpu::new();
    for scale in [1.0, 1.5] {
        let rgba = cpu.render(&s, &mut none(), scale, Color::rgb(10, 20, 30)).unwrap();
        let mut bytes = vec![0x5au8; rgba.data.len()];
        let target = Target { data: &mut bytes, width: rgba.width, height: rgba.height, order: Order::Bgra };
        cpu.render_target(&s, &mut none(), scale, Color::rgb(10, 20, 30), target, 0..rgba.height).unwrap();
        // The same picture with red and blue changed places, byte for byte.
        for (i, (got, want)) in bytes.chunks_exact(4).zip(rgba.data.chunks_exact(4)).enumerate() {
            assert_eq!(got, [want[2], want[1], want[0], want[3]], "pixel {i} at scale {scale}");
        }
    }
    // Memory of the wrong size is refused, not written past.
    let mut short = vec![0u8; 100];
    let target = Target { data: &mut short, width: 300, height: 200, order: Order::Bgra };
    assert_eq!(cpu.render_target(&s, &mut none(), 1.0, BLACK, target, 0..200), Err(RenderError::Size));
}

#[test]
fn what_cannot_be_drawn_is_refused() {
    let s = busy();
    let cpu = Cpu::new();
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 65.0] {
        assert_eq!(cpu.render(&s, &mut none(), scale, BLACK), Err(RenderError::Scale), "{scale}");
    }
    assert_eq!(cpu.render(&Scene::new(40_000.0, 10.0), &mut none(), 1.0, BLACK), Err(RenderError::TooLarge));
    assert_eq!(cpu.render(&Scene::new(20_000.0, 20_000.0), &mut none(), 1.0, BLACK), Err(RenderError::TooLarge));
    let mut bad = Scene::new(10.0, 10.0);
    bad.pop_clip();
    assert!(matches!(cpu.render(&bad, &mut none(), 1.0, BLACK), Err(RenderError::Scene(_))));
    let mut bad = Scene::new(10.0, 10.0);
    bad.glyphs(3, 12.0, WHITE, Point::default(), vec![Glyph::default()]);
    assert!(matches!(cpu.render(&bad, &mut none(), 1.0, BLACK), Err(RenderError::Scene(_))));
    let mut wrong = Pixmap::new(5, 5);
    assert_eq!(cpu.render_rows(&s, &mut none(), 1.0, BLACK, &mut wrong, 0..5), Err(RenderError::Size));
}

#[test]
fn a_font_this_machine_lacks_draws_nothing() {
    let mut s = Scene::new(40.0, 20.0);
    let f = s.font("No Such Family Anywhere", 400, false);
    s.glyphs(f, 12.0, WHITE, Point::new(2.0, 14.0), vec![Glyph { id: 36, x: 0.0, y: 0.0 }]);
    let p = draw(&s, 2.0, BLACK);
    assert!(p.data.chunks_exact(4).all(|px| px == [0, 0, 0, 255]));
}

/// The machine's fonts and its monospace face.
fn mono() -> (Fonts, vergence_text::FaceId) {
    let mut fonts = Fonts::system();
    let face = fonts.face("monospace", 400, false).expect("a monospace font on this machine");
    (fonts, face)
}

/// The box around every pixel that is not the background, and how much ink.
fn ink(p: &Pixmap) -> ([u32; 4], f64) {
    let (mut b, mut sum) = ([u32::MAX, u32::MAX, 0, 0], 0.0);
    for y in 0..p.height {
        for x in 0..p.width {
            let px = p.pixel(x, y);
            if px[0] != 0 {
                b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
                sum += px[0] as f64 / 255.0;
            }
        }
    }
    (b, sum)
}

#[test]
fn text_is_drawn_where_it_was_laid_out() {
    let (mut fonts, face) = mono();
    let mut s = Scene::new(120.0, 30.0);
    let width = text(&mut s, &mut fonts, face, 16.0, RED, Point::new(10.0, 20.0), "Vergence");
    assert!(width > 40.0 && width < 110.0, "{width}");
    assert_eq!(s.validate(), Ok(()));
    let p = Cpu::new().render(&s, &mut fonts, 1.0, BLACK).unwrap();
    let (b, small) = ink(&p);
    assert!(small > 100.0, "letters have ink: {small}");
    assert!(b[0] >= 9 && b[2] as f32 <= 10.0 + width + 2.0, "between the origin and the width: {b:?}");
    assert!(b[1] >= 3 && b[3] <= 26, "around the baseline at 20: {b:?}");
    assert!(p.data.chunks_exact(4).all(|px| px[1] == 0 && px[2] == 0), "tinted red, nothing else");
    // Three times the scale: three times the size, about nine times the ink, the same glyphs.
    let big = Cpu::new().render(&s, &mut fonts, 3.0, BLACK).unwrap();
    let (bb, large) = ink(&big);
    assert_eq!((big.width, big.height), (360, 90));
    assert!((large / small - 9.0).abs() < 1.5, "{large} against {small}");
    assert!(bb[0].abs_diff(b[0] * 3) <= 4 && bb[3].abs_diff(b[3] * 3) <= 4, "{bb:?} against {b:?}");
}

#[test]
fn color_does_not_rasterize_a_glyph_again() {
    let (mut fonts, face) = mono();
    let mut s = Scene::new(200.0, 60.0);
    text(&mut s, &mut fonts, face, 14.0, RED, Point::new(4.0, 20.0), "color");
    Cpu::new().render(&s, &mut fonts, 2.0, BLACK).unwrap();
    let kept = fonts.cached();
    assert!(kept >= 4 && kept <= 6, "one bitmap per different letter: {kept}");
    // The same word in forty other colors, at the same place across the line.
    for i in 0..40u8 {
        text(&mut s, &mut fonts, face, 14.0, Color::rgb(i * 6, 255 - i * 6, i), Point::new(4.0, 20.0), "color");
    }
    Cpu::new().render(&s, &mut fonts, 2.0, BLACK).unwrap();
    assert_eq!(fonts.cached(), kept);
}

/// A small terminal screen two ways: as one grid of cells, and as the fills
/// and glyph runs the grid stands for.
fn grid_two_ways(fonts: &mut Fonts, face: vergence_text::FaceId) -> (Scene, Scene) {
    let (columns, rows) = (14usize, 5usize);
    let (origin, cw, ch, baseline, size) = (Point::new(3.3, 5.0), 8.4, 17.0, 13.0, 14.0);
    let name = fonts.name(face);
    let grounds = [CLEAR, Color::rgb(120, 20, 30), Color::rgb(20, 40, 160), Color::rgba(40, 200, 90, 128)];
    let inks = [WHITE, Color::rgb(250, 220, 40), Color::rgba(200, 200, 255, 153)];
    let text: Vec<char> = "Vergence grid of cells, drawn two ways & compared: 0123456789 ()[]{}".chars().collect();
    let mut cells = Vec::new();
    for r in 0..rows {
        for c in 0..columns {
            let ch = text[(r * columns + c) % text.len()];
            cells.push(Cell {
                ground: grounds[(c / 3 + r) % grounds.len()],
                ink: inks[(c / 2 + r) % inks.len()],
                // One character no font has: its cell holds glyph 0, the "no such character" box.
                glyph: if ch == '&' { 0 } else { fonts.glyph(face, ch).1 },
                font: 0,
                flags: if r % 2 == 1 && (3..9).contains(&c) { CELL_UNDERLINE } else { 0 }
                    | if ch == ' ' { 0 } else { CELL_GLYPH },
            });
        }
    }
    let mut grid = Scene::new(130.0, 100.0);
    grid.font(&name.family, name.weight, name.italic);
    grid.fill(Rect::new(0.0, 0.0, 130.0, 100.0), Color::rgb(30, 30, 46));
    grid.cells(origin, cw, ch, baseline, size, columns as u32, cells.clone());

    let mut plain = Scene::new(130.0, 100.0);
    let font = plain.font(&name.family, name.weight, name.italic);
    plain.fill(Rect::new(0.0, 0.0, 130.0, 100.0), Color::rgb(30, 30, 46));
    for (r, row) in cells.chunks(columns).enumerate() {
        let top = origin.y + r as f32 * ch;
        let runs = |same: &dyn Fn(&Cell, &Cell) -> bool| {
            let (mut out, mut x) = (Vec::new(), 0);
            while x < columns {
                let mut end = x + 1;
                while end < columns && same(&row[x], &row[end]) {
                    end += 1;
                }
                out.push((x, end));
                x = end;
            }
            out
        };
        for (x, end) in runs(&|a, b| a.ground == b.ground) {
            if row[x].ground.a != 0 {
                plain.fill(Rect::new(origin.x + x as f32 * cw, top, (end - x) as f32 * cw, ch), row[x].ground);
            }
        }
        for (x, end) in runs(&|a, b| a.flags & CELL_UNDERLINE != 0 && b.flags & CELL_UNDERLINE != 0 && a.ink == b.ink) {
            if row[x].flags & CELL_UNDERLINE != 0 {
                plain.fill(Rect::new(origin.x + x as f32 * cw, top + ch - 2.0, (end - x) as f32 * cw, 1.0), row[x].ink);
            }
        }
        for (c, cell) in row.iter().enumerate().filter(|(_, cell)| cell.flags & CELL_GLYPH != 0) {
            plain.glyphs(
                font,
                size,
                cell.ink,
                Point::new(origin.x, top + baseline),
                vec![Glyph { id: cell.glyph, x: c as f32 * cw, y: 0.0 }],
            );
        }
    }
    (grid, plain)
}

#[test]
fn a_grid_of_cells_is_the_fills_and_glyphs_it_stands_for() {
    let (mut fonts, face) = mono();
    let (grid, plain) = grid_two_ways(&mut fonts, face);
    assert_eq!(grid.validate(), Ok(()));
    assert!(
        grid.commands.len() == 2 && plain.commands.len() > 60,
        "{} commands against {}",
        grid.commands.len(),
        plain.commands.len()
    );
    for scale in [1.0, 1.25, 2.0, 3.0] {
        let a = Cpu::new().render(&grid, &mut fonts, scale, BLACK).unwrap();
        let b = Cpu::new().render(&plain, &mut fonts, scale, BLACK).unwrap();
        assert!(a.data == b.data, "the two pictures differ at scale {scale}");
        // And it is a picture: grounds, ink.
        assert!(a.data.chunks_exact(4).filter(|px| px[0] > 200 && px[1] > 200).count() > 50, "ink at scale {scale}");
    }
    // Inside a clip, a grid is cut as anything is.
    let mut clipped = Scene::new(130.0, 100.0);
    clipped.fonts = grid.fonts.clone();
    clipped.push_clip(Rect::new(0.0, 0.0, 40.0, 30.0), 0.0);
    clipped.commands.push(grid.commands[1].clone());
    clipped.pop_clip();
    let p = Cpu::new().render(&clipped, &mut fonts, 2.0, BLACK).unwrap();
    assert!((0..p.height).all(|y| (80..p.width).all(|x| p.pixel(x, y) == [0, 0, 0, 255])), "nothing right of the clip");
    assert!((60..p.height).all(|y| (0..p.width).all(|x| p.pixel(x, y) == [0, 0, 0, 255])), "nothing below the clip");
    assert!(p.data.chunks_exact(4).any(|px| px[0] > 100), "something inside it");
}
