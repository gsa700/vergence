//! The GPU renderer against the reference: the same scenes drawn by both
//! must agree to within a rounding step (toolkit milestone 1, acceptance 3).
//! A machine with no GL at all skips these, and says so.

use vergence_render::{Cpu, Gpu, GpuError, Pixmap, text};
use vergence_scene::{CELL_GLYPH, CELL_UNDERLINE, Cell, Color, Glyph, Image, Point, Rect, Scene};
use vergence_text::{Fonts, db::FontDb};

const BLACK: Color = Color::rgb(0, 0, 0);

/// The graphics libraries are loaded before the test harness starts its
/// threads: on FreeBSD a thread that existed before they were loaded
/// crashes at its first drawing call (vergence-render's gl.rs), and the
/// harness runs these tests on several threads at once. A function the
/// system runs as the program is loaded, before `main`.
#[used]
#[unsafe(link_section = ".init_array")]
static LOAD_FIRST: extern "C" fn() = {
    extern "C" fn load_first() {
        vergence_render::preload_gpu_libraries();
    }
    load_first
};

/// The GPU tests take turns: on FreeBSD (drm-kmod 6.12.85, Mesa iris, Intel
/// gen9) two threads of one process mapping GPU buffers at once can deadlock
/// in the kernel, the thread faulting on a mapping waiting forever for a busy
/// page while the others wait on the process's mapping lock, unkillable until
/// a reboot (kepler 2026-11-14: in parallel it hung by the 19th run, one at a
/// time 200 runs never did; tools/gpu-report.sh caught both stacks). Not ours
/// to fix there; the tests stop doing it.
static TURNS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A GPU renderer, held for one test at a time.
struct Turn {
    gpu: Gpu,
    _turn: std::sync::MutexGuard<'static, ()>,
}

impl std::ops::Deref for Turn {
    type Target = Gpu;
    fn deref(&self) -> &Gpu {
        &self.gpu
    }
}

impl std::ops::DerefMut for Turn {
    fn deref_mut(&mut self) -> &mut Gpu {
        &mut self.gpu
    }
}

fn gpu() -> Option<Turn> {
    // A test that failed holding the turn leaves it poisoned; the next still runs.
    let turn = TURNS.lock().unwrap_or_else(|e| e.into_inner());
    match Gpu::new() {
        Ok(gpu) => Some(Turn { gpu, _turn: turn }),
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
        // (the coverage times the color's alpha, then the blend) and once by the GPU. At
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
fn a_buffer_for_the_display_server_holds_the_same_picture() {
    let Some(mut gpu) = gpu() else { return };
    if !gpu.shares_buffers() {
        eprintln!("skipped: this GL makes no buffers a display server can show ({})", gpu.description());
        return;
    }
    let s = busy();
    for scale in [1.0, 2.25] {
        let own = gpu.render(&s, &mut none(), scale, Color::rgb(10, 20, 30)).unwrap();
        let buffer = gpu.buffer(own.width, own.height).unwrap();
        assert!(buffer.stride >= own.width * 4 && buffer.format == 0x3432_5241, "stride {} format {:#x} modifier {:#x}", buffer.stride, buffer.format, buffer.modifier);
        eprintln!("buffer {} x {}: stride {}, offset {}, modifier {:#x}", buffer.width, buffer.height, buffer.stride, buffer.offset, buffer.modifier);
        gpu.draw_into(&buffer, &s, &mut none(), scale, Color::rgb(10, 20, 30)).unwrap();
        // The same renderer into the shared buffer and into its own texture: the same pixels.
        assert_eq!(gpu.read(&buffer).unwrap().hash(), own.hash(), "at scale {scale}");
        // A scene of another size is refused, not stretched.
        let small = Scene::new(10.0, 10.0);
        assert!(matches!(gpu.draw_into(&buffer, &small, &mut none(), 1.0, BLACK), Err(GpuError::Render(_))));
        gpu.release(buffer);
    }
    assert!(gpu.buffer(0, 10).is_err());
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

/// A grid of cells goes to the GPU by a road of its own (cells to instances,
/// no shapes between): it must come out as the fills and glyph runs it
/// stands for do, to the byte, and agree with the reference as text does.
#[test]
fn a_grid_of_cells_is_drawn_as_its_fills_and_glyphs() {
    let Some(mut gpu) = gpu() else { return };
    let mut fonts = Fonts::system();
    let Some(mono) = fonts.face("monospace", 400, false) else {
        eprintln!("skipped: no monospace font");
        return;
    };
    let name = fonts.name(mono);
    let (columns, rows) = (40usize, 6usize);
    let (origin, cw, ch, baseline, size) = (Point::new(4.0, 6.0), 8.5, 18.0, 14.0, 13.33);
    let mut rng = Rng(0x7e57_0000_0000_0009);
    let mut cells = Vec::new();
    for i in 0..columns * rows {
        let c = rng.color();
        let letter = (b'!' + (i % 90) as u8) as char;
        cells.push(Cell {
            // Runs of one ground, grounds of their own, and none.
            ground: match i % 7 {
                0 | 1 => Color::rgb(70, 20, 90),
                2 => Color::rgba(0, 0, 0, 0),
                _ => Color::rgb(c.b / 3, c.r / 3, c.g / 3),
            },
            ink: Color::rgba(c.r, c.g, c.b, if i % 11 == 0 { 153 } else { 255 }),
            glyph: fonts.glyph(mono, letter).1,
            font: 0,
            flags: if i % 5 == 4 { 0 } else { CELL_GLYPH } | if (i / 9) % 3 == 0 { CELL_UNDERLINE } else { 0 },
        });
    }
    let two_ways = |clip: Option<f32>| {
        let mut grid = Scene::new(360.0, 120.0);
        let mut plain = Scene::new(360.0, 120.0);
        for s in [&mut grid, &mut plain] {
            s.font(&name.family, name.weight, name.italic);
            s.fill(Rect::new(0.0, 0.0, 360.0, 120.0), Color::rgb(46, 52, 54));
            if let Some(radius) = clip {
                s.push_clip(Rect::new(20.5, 10.0, 300.0, 90.0), radius);
            }
        }
        grid.cells(origin, cw, ch, baseline, size, columns as u32, cells.clone());
        for (r, row) in cells.chunks(columns).enumerate() {
            let top = origin.y + r as f32 * ch;
            let mut x = 0;
            while x < columns {
                let mut end = x + 1;
                while end < columns && row[end].ground == row[x].ground {
                    end += 1;
                }
                if row[x].ground.a != 0 {
                    plain.fill(Rect::new(origin.x + x as f32 * cw, top, (end - x) as f32 * cw, ch), row[x].ground);
                }
                x = end;
            }
            let lined = |c: &Cell| c.flags & CELL_UNDERLINE != 0;
            let mut x = 0;
            while x < columns {
                let mut end = x + 1;
                if lined(&row[x]) {
                    while end < columns && lined(&row[end]) && row[end].ink == row[x].ink {
                        end += 1;
                    }
                    plain.fill(Rect::new(origin.x + x as f32 * cw, top + ch - 2.0, (end - x) as f32 * cw, 1.0), row[x].ink);
                }
                x = end;
            }
            for (c, cell) in row.iter().enumerate().filter(|(_, cell)| cell.flags & CELL_GLYPH != 0) {
                plain.glyphs(0, size, cell.ink, Point::new(origin.x, top + baseline), vec![Glyph { id: cell.glyph, x: c as f32 * cw, y: 0.0 }]);
            }
        }
        if clip.is_some() {
            grid.pop_clip();
            plain.pop_clip();
        }
        (grid, plain)
    };
    for clip in [None, Some(0.0), Some(9.0)] {
        let (grid, plain) = two_ways(clip);
        assert_eq!(grid.validate(), Ok(()));
        for scale in [1.0, 2.25, 3.0] {
            let a = gpu.render(&grid, &mut fonts, scale, BLACK).unwrap();
            let b = gpu.render(&plain, &mut fonts, scale, BLACK).unwrap();
            assert!(a.data == b.data, "a grid and its fills and glyphs differ on the GPU at scale {scale}, clip {clip:?}");
            // Three steps, where a screen of opaque text is held to two: some of these
            // inks are translucent, which is one more multiplication the CPU renderer
            // rounds and the GPU does not (three apart seen with Techbench's variable
            // font at scale 2.25, 2026-11-10; two on Hubble and canary).
            agree(&mut gpu, &grid, &mut fonts, scale, BLACK, 3, 30, "a grid of cells");
        }
    }
}

/// The server's frost shader on this GPU against its CPU reference
/// (vergence-render's blur, look milestone 1): the same picture frosted
/// both ways must agree but for the GPU's rounding (and, rarely, a pixel
/// where the tint's bisection lands one step apart).
#[test]
fn the_frost_shader_on_the_gpu_matches_its_cpu_reference() {
    use vergence_render::blur::{FROST_SHADER, Frost, frost};
    let Some(mut g) = gpu() else { return };
    let (w, h) = (96usize, 48usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let text = (x / 3 + y / 2) % 4 == 0;
            rgba.extend(if text { [235u8, 235, 235, 255] } else { [(x * 2) as u8, 30, (y * 4) as u8, 255] });
        }
    }
    for f in [
        Frost { tint: [51, 61, 82], strength: 0.5, ink: [235, 235, 235], need: 7.0, radius: 10.0, invert: false, cap: false },
        Frost { tint: [228, 230, 235], strength: 0.3, ink: [29, 29, 31], need: 5.85, radius: 6.0, invert: false, cap: false },
        Frost { tint: [228, 230, 235], strength: 0.3, ink: [29, 29, 31], need: 14.0, radius: 6.0, invert: true, cap: false },
        Frost { tint: [216, 219, 226], strength: 0.5, ink: [29, 29, 31], need: 5.85, radius: 6.0, invert: false, cap: true },
    ] {
        let cpu = frost(&rgba, w, h, h, &f);
        let unit = |c: [u8; 3]| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0];
        let t = unit(f.tint);
        let gpu = match g.shader_pass(FROST_SHADER, &rgba, w as u32, h as u32, &[
            ("texel", &[1.0 / w as f32, 1.0 / h as f32]),
            ("radius", &[f.radius]),
            ("tint", &[t[0], t[1], t[2], f.strength]),
            ("ink", &unit(f.ink)),
            ("need", &[f.need]),
            ("corner", &[0.0]),
            ("invert", &[if f.invert { 1.0 } else { 0.0 }]),
            ("cap", &[if f.cap { 1.0 } else { 0.0 }]),
        ]) {
            Ok(p) => p,
            Err(e) => panic!("the frost shader on this GPU: {e}"),
        };
        let (mut worst, mut far, mut total) = (0u8, 0usize, 0u64);
        for (p, q) in gpu.data.chunks_exact(4).zip(cpu.chunks_exact(4)) {
            let d = p[..3].iter().zip(&q[..3]).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            worst = worst.max(d);
            far += (d > 3) as usize;
            total += d as u64;
        }
        let mean = total as f64 / (w * h) as f64;
        assert!(mean < 1.0 && far * 100 <= w * h, "{f:?}: mean {mean:.2}, {far} pixels off by more than 3, worst {worst}");
    }
}

/// The rounded corners' shader on this GPU against its arithmetic.
#[test]
fn the_round_shader_on_the_gpu_matches_its_arithmetic() {
    use vergence_render::round::{ROUND_SHADER, cover};
    let Some(mut g) = gpu() else { return };
    let (w, h, r) = (64u32, 40u32, 9.0f32);
    let rgba = vec![200u8; (w * h * 4) as usize];
    // Bottom corners only, then the top ones too (a window drawn under its bar).
    for top in [false, true] {
        let out = g.shader_pass(ROUND_SHADER, &rgba, w, h, &[("size", &[w as f32, h as f32]), ("radius", &[r]), ("top", &[if top { 1.0 } else { 0.0 }])]).expect("the round shader on this GPU");
        for y in 0..h {
            for x in 0..w {
                let want = (200.0 * cover(x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32, r, top)).round() as i32;
                let got = out.data[((y * w + x) * 4 + 3) as usize] as i32;
                assert!((want - got).abs() <= 2, "top {top} ({x}, {y}): alpha {got}, want {want}");
            }
        }
    }
}
