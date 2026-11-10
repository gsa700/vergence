//! The GPU renderer against the reference: the same scenes drawn by both
//! must agree to within a rounding step (toolkit milestone 1, acceptance 3).
//! A machine with no GL at all skips these, and says so.

use vergence_render::{Cpu, Gpu, GpuError, Pixmap, text};
use vergence_scene::{Color, Image, Point, Rect, Scene};
use vergence_text::{Fonts, db::FontDb};

const BLACK: Color = Color::rgb(0, 0, 0);

fn gpu() -> Option<Gpu> {
    match Gpu::new() {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("skipped: no GPU renderer on this machine ({e})");
            None
        }
    }
}

/// How far two pictures are apart: the largest difference in any byte, and
/// how many pixels differ by more than one step in some byte.
fn apart(a: &Pixmap, b: &Pixmap) -> (u8, usize) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let mut worst = 0;
    let mut beyond = 0;
    for (p, q) in a.data.chunks_exact(4).zip(b.data.chunks_exact(4)) {
        let d = p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        worst = worst.max(d);
        beyond += (d > 1) as usize;
    }
    (worst, beyond)
}

/// Draws a scene both ways and holds them to the tolerance: no byte more
/// than `worst` apart, and at most `per_mille` pixels in a thousand more
/// than one step apart.
fn agree(gpu: &mut Gpu, scene: &Scene, fonts: &mut Fonts, scale: f32, background: Color, worst: u8, per_mille: usize, what: &str) {
    let reference = Cpu::new().render(scene, fonts, scale, background).unwrap();
    let drawn = gpu.render(scene, fonts, scale, background).unwrap();
    let (far, beyond) = apart(&reference, &drawn);
    let pixels = (reference.width * reference.height) as usize;
    eprintln!("{what} at scale {scale}: worst byte difference {far}, {beyond} of {pixels} pixels more than one step apart");
    assert!(far <= worst && beyond * 1000 <= pixels * per_mille, "{what} at scale {scale}: worst byte difference {far} (allowed {worst}), {beyond} of {pixels} pixels more than one step apart (allowed {per_mille} in a thousand)");
}

fn none() -> Fonts {
    Fonts::new(FontDb::default())
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

/// Every kind of shape, many of each, with one rounded clip at a time.
fn busy() -> Scene {
    let mut rng = Rng(0x0bad_cafe_1234_5677);
    let mut s = Scene::new(300.0, 200.0);
    let pic = s.image(Image { width: 3, height: 3, rgba: (0..36).map(|i| (i * 7) as u8 | if i % 4 == 3 { 0x80 } else { 0 }).collect() });
    let mut open = false;
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
            7 if !open => {
                s.push_clip(Rect::new(rng.unit() * 150.0, rng.unit() * 100.0, 60.0 + rng.unit() * 150.0, 40.0 + rng.unit() * 100.0), if i % 2 == 0 { 0.0 } else { rng.unit() * 20.0 });
                open = true;
            }
            _ if open => {
                s.pop_clip();
                open = false;
            }
            _ => {}
        }
    }
    if open {
        s.pop_clip();
    }
    s
}

#[test]
fn simple_shapes_agree_with_the_reference() {
    let Some(mut gpu) = gpu() else { return };
    eprintln!("GPU renderer: {}", gpu.description());
    let white = Color::rgb(255, 255, 255);
    let mut scenes: Vec<(&str, Scene)> = Vec::new();
    let mut s = Scene::new(20.0, 20.0);
    s.fill(Rect::new(2.0, 3.0, 4.0, 5.0), Color::rgb(255, 0, 0));
    s.fill(Rect::new(8.5, 2.0, 3.0, 2.5), white);
    s.fill(Rect::new(1.0, 12.0, 18.0, 6.0), Color::rgba(255, 255, 255, 128));
    scenes.push(("fills", s));
    let mut s = Scene::new(40.0, 40.0);
    s.fill_rounded(Rect::new(0.0, 0.0, 20.0, 20.0), 10.0, white);
    s.fill_rounded(Rect::new(21.5, 3.25, 16.0, 30.0), 5.5, Color::rgb(0, 200, 90));
    s.stroke(Rect::new(2.0, 22.0, 16.0, 16.0), 0.0, 2.0, Color::rgb(255, 0, 0));
    s.stroke(Rect::new(22.0, 22.0, 16.5, 15.5), 6.0, 1.5, white);
    scenes.push(("rounded and outlined", s));
    let mut s = Scene::new(30.0, 30.0);
    s.line(Point::new(3.0, 6.0), Point::new(26.0, 6.0), 2.0, white);
    s.line(Point::new(2.0, 10.0), Point::new(27.0, 28.0), 1.5, Color::rgb(90, 160, 255));
    s.line(Point::new(15.0, 20.0), Point::new(15.0, 20.0), 5.0, Color::rgb(255, 200, 0));
    scenes.push(("lines", s));
    let mut s = Scene::new(30.0, 30.0);
    s.push_clip(Rect::new(5.0, 5.0, 20.0, 20.0), 10.0);
    s.fill(Rect::new(0.0, 0.0, 30.0, 30.0), white);
    s.line(Point::new(0.0, 0.0), Point::new(30.0, 30.0), 3.0, Color::rgb(255, 0, 0));
    s.pop_clip();
    s.push_clip(Rect::new(2.5, 0.0, 6.0, 30.0), 0.0);
    s.fill(Rect::new(0.0, 0.0, 30.0, 4.0), Color::rgb(0, 255, 0));
    s.pop_clip();
    scenes.push(("clips", s));
    let mut s = Scene::new(16.0, 8.0);
    let two = s.image(Image { width: 2, height: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 255] });
    let half = s.image(Image { width: 2, height: 2, rgba: vec![255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0, 128] });
    s.draw_image(two, Rect::new(0.0, 0.0, 8.0, 4.0), 1.0);
    s.draw_image(half, Rect::new(8.5, 0.5, 7.0, 7.0), 0.6);
    scenes.push(("pictures", s));
    for (what, scene) in &scenes {
        for scale in [1.0, 2.0, 2.25] {
            agree(&mut gpu, scene, &mut none(), scale, BLACK, 2, 1, what);
            agree(&mut gpu, scene, &mut none(), scale, Color::rgba(0, 0, 0, 0), 2, 1, what);
        }
    }
}

#[test]
fn a_busy_scene_agrees_with_the_reference() {
    let Some(mut gpu) = gpu() else { return };
    let s = busy();
    for scale in [1.0, 1.5, 3.0] {
        // Hundreds of translucent layers over each other: each may round differently by a step.
        agree(&mut gpu, &s, &mut none(), scale, Color::rgb(10, 20, 30), 4, 30, "the busy scene");
    }
}

#[test]
fn text_agrees_with_the_reference() {
    let Some(mut gpu) = gpu() else { return };
    let mut fonts = Fonts::system();
    let Some(mono) = fonts.face("monospace", 400, false) else {
        eprintln!("skipped: no monospace font");
        return;
    };
    let mut s = Scene::new(360.0, 120.0);
    s.fill(Rect::new(0.0, 0.0, 360.0, 120.0), Color::rgb(46, 52, 54));
    let mut rng = Rng(0x7e57_0000_0000_0005);
    for row in 0..6 {
        for col in 0..40 {
            let c = rng.color();
            let cell = Rect::new(4.0 + col as f32 * 8.5, 6.0 + row as f32 * 18.0, 8.5, 18.0);
            s.fill(cell, Color::rgb(c.b / 3, c.r / 3, c.g / 3));
            let ch = (b'!' + ((row * 40 + col) % 90) as u8) as char;
            let mut one = [0u8; 4];
            text(&mut s, &mut fonts, mono, 13.33, Color::rgb(c.r, c.g, c.b), Point::new(cell.x, cell.y + 14.0), ch.encode_utf8(&mut one));
        }
    }
    for scale in [1.0, 2.25, 3.0] {
        // Never more than two steps apart. How many pixels are two apart depends on the
        // font: every partly covered pixel of a glyph is rounded twice by the CPU renderer
        // (the coverage times the colour's alpha, then the blend) and once by the GPU. At
        // scale 1 most of a glyph's pixels are partly covered: 0.4% of the picture on
        // canary's Noto Sans Mono and 1.4% with Techbench's variable one, against one
        // pixel on Hubble (2026-11-09).
        agree(&mut gpu, &s, &mut fonts, scale, BLACK, 2, 30, "a screen of text");
    }
    // The same glyphs again are already in the atlas; and the picture is the same.
    let first = gpu.render(&s, &mut fonts, 3.0, BLACK).unwrap();
    let second = gpu.render(&s, &mut fonts, 3.0, BLACK).unwrap();
    assert_eq!(first.hash(), second.hash());
}

#[test]
fn what_the_gpu_does_not_draw_yet_is_refused_not_wrong() {
    let Some(mut gpu) = gpu() else { return };
    let mut s = Scene::new(40.0, 40.0);
    s.push_clip(Rect::new(0.0, 0.0, 40.0, 40.0), 12.0);
    s.push_clip(Rect::new(5.0, 5.0, 30.0, 30.0), 8.0);
    s.fill(Rect::new(0.0, 0.0, 40.0, 40.0), Color::rgb(255, 255, 255));
    s.pop_clip();
    s.pop_clip();
    assert!(matches!(gpu.render(&s, &mut none(), 1.0, BLACK), Err(GpuError::Unsupported(_))));
    // And the usual refusals are the reference's.
    assert!(matches!(gpu.render(&s, &mut none(), 0.0, BLACK), Err(GpuError::Render(_))));
    let mut bad = Scene::new(10.0, 10.0);
    bad.pop_clip();
    assert!(matches!(gpu.render(&bad, &mut none(), 1.0, BLACK), Err(GpuError::Render(_))));
    // The renderer still works afterwards.
    let mut ok = Scene::new(10.0, 10.0);
    ok.fill(Rect::new(1.0, 1.0, 3.0, 3.0), Color::rgb(255, 0, 0));
    assert_eq!(gpu.render(&ok, &mut none(), 1.0, BLACK).unwrap().pixel(2, 2), [255, 0, 0, 255]);
}
