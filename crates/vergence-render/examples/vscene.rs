//! Draws a demonstration scene with the CPU renderer, times it, and writes a
//! PNG to look at:
//!
//!     cargo run --release -p vergence-render --example vscene -- term out.png --scale 3
//!     cargo run --release -p vergence-render --example vscene -- shapes out.png --scale 2 --threads 1
//!
//! `term` is the stress test's window (toolkit milestone 1): 1264 x 706
//! reference units of terminal, every cell its own colors and its own
//! character. `shapes` is one of everything. `--size WxH` changes the
//! window, `--scene file` also writes the scene's bytes, `--gpu` draws it on
//! the GPU as well and compares (the PNG is then the GPU's picture).

use std::time::Instant;

use vergence_render::{Cpu, Gpu, text};
use vergence_scene::{Color, Image, Point, Rect, Scene};
use vergence_text::Fonts;

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
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const BAR: Color = Color::rgb(51, 61, 82);
const PANE: Color = Color::rgb(46, 52, 54);
const TEXT: Color = Color::rgb(235, 235, 235);

/// A window bar with tabs, as the terminal draws it.
fn bar(s: &mut Scene, fonts: &mut Fonts, tabs: &[&str]) {
    let ui = fonts.face("sans-serif", 400, false);
    s.fill(Rect::new(0.0, 0.0, s.width, 36.0), BAR);
    for (i, name) in ["close", "minimize"].iter().enumerate() {
        let (cx, cy) = (22.0 + i as f32 * 34.0, 20.0);
        if *name == "close" {
            s.line(Point::new(cx - 5.0, cy - 5.0), Point::new(cx + 5.0, cy + 5.0), 1.25, TEXT);
            s.line(Point::new(cx + 5.0, cy - 5.0), Point::new(cx - 5.0, cy + 5.0), 1.25, TEXT);
        } else {
            s.line(Point::new(cx - 5.0, cy + 5.0), Point::new(cx + 5.0, cy + 5.0), 1.25, TEXT);
        }
    }
    let mut x = 78.0;
    let width = ((s.width - x - 60.0) / tabs.len() as f32).min(200.0);
    for (i, title) in tabs.iter().enumerate() {
        let shown = i == tabs.len() - 1;
        // A tab: round at the top, square where it meets the pane (the bottom half is covered).
        s.push_clip(Rect::new(x, 6.0, width - 2.0, 30.0), 0.0);
        s.fill_rounded(Rect::new(x, 6.0, width - 2.0, 44.0), 7.0, if shown { PANE } else { Color::rgba(255, 255, 255, 20) });
        s.line(Point::new(x + 12.0, 17.0), Point::new(x + 20.0, 25.0), 1.25, TEXT);
        s.line(Point::new(x + 20.0, 17.0), Point::new(x + 12.0, 25.0), 1.25, TEXT);
        if let Some(ui) = ui {
            text(s, fonts, ui, 13.3, TEXT, Point::new(x + 32.0, 26.0), title);
        }
        s.pop_clip();
        x += width;
    }
    s.push_clip(Rect::new(x, 6.0, 32.0, 30.0), 0.0);
    s.fill_rounded(Rect::new(x, 6.0, 32.0, 44.0), 7.0, Color::rgba(255, 255, 255, 20));
    s.pop_clip();
    s.line(Point::new(x + 16.0, 15.0), Point::new(x + 16.0, 27.0), 1.25, TEXT);
    s.line(Point::new(x + 10.0, 21.0), Point::new(x + 22.0, 21.0), 1.25, TEXT);
}

/// A terminal full of cells, each with its own colors and character.
fn term(fonts: &mut Fonts, w: f32, h: f32) -> (Scene, usize, f32) {
    let mut s = Scene::new(w, h);
    s.fill(Rect::new(0.0, 0.0, w, h), PANE);
    bar(&mut s, fonts, &["btop", "htop", "seq", "color", "cursor", "unicode"]);
    let mono = fonts.face("monospace", 400, false).expect("a monospace font");
    let size = 13.33;
    let m = fonts.metrics(mono, size);
    let (probe, _) = (fonts.glyph(mono, 'M'), ());
    let cell_w = fonts.advance(probe.0, probe.1, size);
    let cell_h = (m.ascent + m.descent + m.line_gap).ceil();
    let (cols, rows) = (((w - 8.0) / cell_w) as usize, ((h - 36.0 - 8.0) / cell_h) as usize);
    let mut rng = Rng(0x7e57_ab1e_0000_0001);
    let glyphs: Vec<char> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789#@%&*+=-".chars().collect();
    s.push_clip(Rect::new(0.0, 36.0, w, h - 36.0), 0.0);
    for row in 0..rows {
        let top = 40.0 + row as f32 * cell_h;
        for col in 0..cols {
            let v = rng.next();
            let (bg, fg) = (Color::rgb(v as u8, (v >> 8) as u8, (v >> 16) as u8), Color::rgb((v >> 24) as u8, (v >> 32) as u8, (v >> 40) as u8));
            let left = 4.0 + col as f32 * cell_w;
            s.fill(Rect::new(left, top, cell_w, cell_h), bg);
            let c = glyphs[rng.below(glyphs.len() as u64) as usize];
            let mut one = [0u8; 4];
            text(&mut s, fonts, mono, size, fg, Point::new(left, top + m.ascent.round()), c.encode_utf8(&mut one));
        }
    }
    s.pop_clip();
    (s, cols * rows, cell_h)
}

/// One of everything.
fn shapes(fonts: &mut Fonts, w: f32, h: f32) -> Scene {
    let mut s = Scene::new(w, h);
    s.fill(Rect::new(0.0, 0.0, w, h), Color::rgb(24, 33, 41));
    bar(&mut s, fonts, &["Scene", "Text", "Renderer"]);
    let palette = [Color::rgb(74, 155, 184), Color::rgb(224, 164, 58), Color::rgb(201, 77, 71), Color::rgb(120, 190, 120)];
    for (i, c) in palette.iter().enumerate() {
        let x = 24.0 + i as f32 * 110.0;
        s.fill_rounded(Rect::new(x, 60.0, 90.0, 60.0), i as f32 * 10.0, *c);
        s.stroke(Rect::new(x, 140.0, 90.0, 60.0), i as f32 * 10.0, 1.0 + i as f32 * 1.5, *c);
        s.line(Point::new(x, 220.0), Point::new(x + 90.0, 250.0 - i as f32 * 10.0), 1.0 + i as f32, *c);
        s.fill(Rect::new(x + i as f32 * 0.25, 270.0, 90.0, 6.0), Color::rgba(c.r, c.g, c.b, 128));
    }
    // A picture: a small gradient, stretched, inside a round clip.
    let side = 16u32;
    let rgba = (0..side * side).flat_map(|i| [(i % side * 16) as u8, (i / side * 16) as u8, 160, 255]).collect();
    let pic = s.image(Image { width: side, height: side, rgba });
    s.push_clip(Rect::new(480.0, 60.0, 140.0, 140.0), 70.0);
    s.draw_image(pic, Rect::new(480.0, 60.0, 140.0, 140.0), 1.0);
    s.pop_clip();
    let mut y = 310.0;
    for (family, weight, size, line) in [
        ("sans-serif", 400, 28.0, "Vergence: one scene, any density"),
        ("sans-serif", 700, 16.0, "The quick brown fox jumps over the lazy dog. 0123456789"),
        ("monospace", 400, 13.33, "fn main() { println!(\"é ñ ü ─│┌┐ ▁▂▃▄▅▆▇█ א ∑ → ≠\"); }"),
        ("monospace", 700, 13.33, "derickson : ~ : cargo test --workspace"),
        ("serif", 400, 11.0, "Small text, where hinting would matter most at one pixel to the unit."),
    ] {
        if let Some(face) = fonts.face(family, weight, false) {
            text(&mut s, fonts, face, size, TEXT, Point::new(24.0, y), line);
        }
        y += size * 1.7;
    }
    s
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let which = args.first().map(String::as_str).unwrap_or("shapes");
    let out = args.get(1).filter(|a| !a.starts_with("--")).cloned();
    let scale: f32 = option("--scale").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let cpu = option("--threads").and_then(|v| v.parse().ok()).map(Cpu::with_threads).unwrap_or_default();
    let (w, h) = option("--size").and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).unwrap_or(if which == "term" { (1264.0, 706.0) } else { (660.0, 440.0) });

    let t = Instant::now();
    let mut fonts = Fonts::system();
    let indexed = t.elapsed();
    let t = Instant::now();
    let (scene, cells, cell_h) = match which {
        "term" => term(&mut fonts, w, h),
        _ => (shapes(&mut fonts, w, h), 0, 20.0),
    };
    let built = t.elapsed();
    scene.validate().expect("the demonstration scene is valid");
    let bytes = scene.encode().expect("it encodes");
    println!("{which}: {w} x {h} units, {} commands, {} bytes on the wire{}", scene.commands.len(), bytes.len(), if cells > 0 { format!(", {cells} cells") } else { String::new() });
    println!("fonts indexed in {:.1} ms, scene built in {:.1} ms", indexed.as_secs_f64() * 1e3, built.as_secs_f64() * 1e3);
    if let Some(path) = option("--scene") {
        std::fs::write(&path, &bytes).expect("write the scene");
    }

    let background = Color::rgb(0, 0, 0);
    let ms = |f: &mut dyn FnMut()| {
        let t = Instant::now();
        f();
        t.elapsed().as_secs_f64() * 1e3
    };
    let mut pixmap = None;
    let first = ms(&mut || pixmap = Some(cpu.render(&scene, &mut fonts, scale, background).expect("it renders")));
    let mut pixmap = pixmap.expect("rendered");
    let again = (0..5).map(|_| ms(&mut || pixmap = cpu.render(&scene, &mut fonts, scale, background).expect("it renders"))).fold(f64::MAX, f64::min);
    println!("{} x {} pixels at scale {scale}, {} threads: first frame {first:.1} ms (glyphs rasterized: {}), then {again:.1} ms a frame", pixmap.width, pixmap.height, cpu.threads(), fonts.cached());
    // One line of text redrawn, as when a terminal row changes.
    let row = (pixmap.height / 2)..(pixmap.height / 2 + (cell_h * scale).ceil() as u32);
    let line = (0..20).map(|_| ms(&mut || cpu.render_rows(&scene, &mut fonts, scale, background, &mut pixmap, row.clone()).expect("it renders"))).fold(f64::MAX, f64::min);
    println!("one row of text ({} pixel rows) redrawn in {line:.2} ms; picture {:016x}", row.len(), pixmap.hash());

    if args.iter().any(|a| a == "--gpu") {
        match Gpu::new() {
            Err(e) => println!("GPU: none ({e})"),
            Ok(mut gpu) => {
                println!("GPU: {}", gpu.description());
                let mut drawn = None;
                let first = ms(&mut || drawn = Some(gpu.render(&scene, &mut fonts, scale, background)));
                match drawn.expect("ran") {
                    Err(e) => println!("GPU: {e}"),
                    Ok(picture) => {
                        let draw = (0..10).map(|_| ms(&mut || gpu.draw(&scene, &mut fonts, scale, background).expect("it draws"))).fold(f64::MAX, f64::min);
                        let read = (0..5).map(|_| ms(&mut || drop(gpu.render(&scene, &mut fonts, scale, background)))).fold(f64::MAX, f64::min);
                        let (mut worst, mut beyond) = (0u8, 0usize);
                        for (a, b) in picture.data.chunks_exact(4).zip(pixmap.data.chunks_exact(4)) {
                            let d = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
                            worst = worst.max(d);
                            beyond += (d > 1) as usize;
                        }
                        println!("GPU: first frame {first:.1} ms, then {draw:.2} ms a frame drawn ({read:.1} ms with the pixels read back)");
                        println!("GPU against CPU: worst byte difference {worst}, {beyond} of {} pixels more than one step apart", pixmap.width as usize * pixmap.height as usize);
                        pixmap = picture;
                    }
                }
            }
        }
    }

    if let Some(path) = out {
        let file = std::io::BufWriter::new(std::fs::File::create(&path).expect("create the PNG"));
        let mut enc = png::Encoder::new(file, pixmap.width, pixmap.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        enc.write_header().and_then(|mut w| w.write_image_data(&pixmap.straight_rgba())).expect("write the PNG");
        println!("wrote {path}");
    }
}
