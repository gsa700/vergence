//! A window for the display checks (tools/display-check.sh, testing method
//! layer 2): flat magenta, with a line one pixel wide on each
//! edge in its own color, top red, bottom blue, left yellow, right cyan,
//! drawn at the window's scale so each is exactly one row or column of the
//! screen's pixels. A shot of the screen then says, pixel by pixel, whether
//! the window was shown whole (all four lines there), where it starts and
//! ends, and what the server drew next to it (its bar above).
//!
//! `--rich` adds what a GPU and the CPU reference may draw differently:
//! text at several sizes, rounded shapes, outlines, hairlines and soft
//! colors, all fixed, so a window drawn on the GPU and one drawn by the CPU
//! renderer (VERGENCE_RENDER=cpu) can be compared pixel for pixel (headless
//! milestone 1, step 4). `--title T` names the window (to tell two apart).
//!
//!     cargo build --release -p vergence-ui --example testpattern
//!     vctl spawn ~/src/vergence/target/release/examples/testpattern --size 580 420

use vergence_render::text;
use vergence_scene::{Color, Point, Rect, Scene};
use vergence_text::Fonts;
use vergence_ui::{App, Frame};

pub const BODY: Color = Color::rgb(200, 40, 160);
pub const TOP: Color = Color::rgb(255, 0, 0);
pub const BOTTOM: Color = Color::rgb(0, 0, 255);
pub const LEFT: Color = Color::rgb(255, 255, 0);
pub const RIGHT: Color = Color::rgb(0, 255, 255);

struct Pattern {
    rich: bool,
    title: String,
}

impl App for Pattern {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn app_id(&self) -> String {
        "vergence-testpattern".into()
    }

    fn scene(&mut self, f: &Frame, fonts: &mut Fonts) -> Scene {
        let (w, h) = (f.width, f.height);
        let mut s = Scene::new(w, h);
        // One pixel of the screen, in points.
        let p = 1.0 / f.scale.max(0.01);
        s.fill(Rect::new(0.0, 0.0, w, h), BODY);
        s.fill(Rect::new(0.0, 0.0, p, h), LEFT);
        s.fill(Rect::new(w - p, 0.0, p, h), RIGHT);
        s.fill(Rect::new(0.0, 0.0, w, p), TOP);
        s.fill(Rect::new(0.0, h - p, w, p), BOTTOM);
        if self.rich {
            rich(&mut s, fonts, w, h);
        }
        s
    }
}

/// Inside the edge lines: everything fixed (no time, no frame count).
fn rich(s: &mut Scene, fonts: &mut Fonts, w: f32, h: f32) {
    let ink = Color::rgb(245, 245, 250);
    s.fill_rounded(Rect::new(12.0, 12.0, w - 24.0, h - 24.0), 10.0, Color::rgb(40, 44, 58));
    if let Some(face) = fonts.face("sans-serif", 400, false) {
        text(s, fonts, face, 22.0, ink, Point::new(24.0, 44.0), "Vergence pattern Ag 0123");
        text(
            s,
            fonts,
            face,
            13.0,
            Color::rgb(180, 190, 210),
            Point::new(24.0, 66.0),
            "Small text: the quick brown fox jumps over the lazy dog.",
        );
    }
    if let Some(face) = fonts.face("monospace", 400, false) {
        text(s, fonts, face, 12.0, Color::rgb(120, 220, 160), Point::new(24.0, 86.0), "fn main() { 1 + 1 == 2 }");
    }
    for i in 0..5 {
        let x = 24.0 + i as f32 * 52.0;
        s.fill_rounded(Rect::new(x, 100.0, 44.0, 30.0), i as f32 * 4.0, Color::rgb(60 + i * 40, 120, 220 - i * 30));
        s.stroke(Rect::new(x, 138.0, 44.0, 30.0), i as f32 * 4.0, 1.5, Color::rgb(230, 170 - i * 20, 60));
    }
    // Hairlines one point apart.
    for i in 0..10 {
        s.fill(Rect::new(24.0 + i as f32 * 4.0, 176.0, 1.0, (h - 200.0).max(4.0)), Color::rgb(200, 200, 200));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let size = args
        .iter()
        .position(|a| a == "--size")
        .and_then(|i| Some((args.get(i + 1)?.parse().ok()?, args.get(i + 2)?.parse().ok()?)))
        .unwrap_or((580, 420));
    let rich = args.iter().any(|a| a == "--rich");
    let title = args
        .iter()
        .position(|a| a == "--title")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "Test pattern".into());
    if let Err(e) = vergence_ui::run(&mut Pattern { rich, title }, size) {
        eprintln!("testpattern: {e}");
        std::process::exit(1);
    }
}
