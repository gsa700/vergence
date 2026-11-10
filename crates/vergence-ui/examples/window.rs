//! The toolkit's first window: a scene on the display server, drawn at the
//! monitor's exact scale by our own renderer, through our own Wayland code.
//!
//!     cargo run --release -p vergence-ui --example window
//!     cargo run --release -p vergence-ui --example window -- --animate --frames 300
//!
//! `--animate` keeps a bar moving, to see frames arrive in step with the
//! monitor; `--frames N` closes after N frames and prints the rate, for
//! running over SSH. `--own-frame` asks for no title bar from the server.

use vergence_render::text;
use vergence_scene::{Color, Point, Rect, Scene};
use vergence_text::Fonts;
use vergence_ui::{App, Frame};

struct Demo {
    animate: bool,
    own_frame: bool,
    stop_after: Option<u64>,
    /// How long drawing took, frame by frame, and when the last few were shown.
    draw_ms: Vec<f64>,
    last: Option<(f32, f32, f32)>,
}

const GROUND: Color = Color::rgb(24, 33, 41);
const INK: Color = Color::rgb(227, 233, 238);
const SOFT: Color = Color::rgb(154, 169, 181);
const TEAL: Color = Color::rgb(74, 155, 184);
const AMBER: Color = Color::rgb(224, 164, 58);

impl App for Demo {
    fn title(&self) -> String {
        "Vergence toolkit".into()
    }

    fn app_id(&self) -> String {
        "vergence-demo".into()
    }

    fn own_frame(&self) -> bool {
        self.own_frame
    }

    fn animating(&self) -> bool {
        self.animate
    }

    fn scene(&mut self, f: &Frame, fonts: &mut Fonts) -> Scene {
        let (w, h) = (f.width, f.height);
        let mut s = Scene::new(w, h);
        s.fill(Rect::new(0.0, 0.0, w, h), GROUND);
        s.stroke(Rect::new(0.5, 0.5, w - 1.0, h - 1.0), 0.0, 1.0, Color::rgb(45, 59, 70));
        let sans = fonts.face("sans-serif", 400, false);
        let bold = fonts.face("sans-serif", 700, false);
        let mono = fonts.face("monospace", 400, false);
        if let Some(face) = bold {
            text(&mut s, fonts, face, 26.0, INK, Point::new(24.0, 48.0), "A window of our own");
        }
        if let Some(face) = sans {
            text(&mut s, fonts, face, 14.0, SOFT, Point::new(24.0, 74.0), "Scene, text, renderer and Wayland code written by the project.");
        }
        if let Some(face) = mono {
            let pixels = ((w * f.scale).ceil(), (h * f.scale).ceil());
            let line = format!("{w} x {h} units  scale {:.4}  {} x {} pixels  frame {}", f.scale, pixels.0, pixels.1, f.number);
            text(&mut s, fonts, face, 13.33, AMBER, Point::new(24.0, 104.0), &line);
            if let Some(avg) = (!self.draw_ms.is_empty()).then(|| self.draw_ms.iter().rev().take(30).sum::<f64>() / self.draw_ms.len().min(30) as f64) {
                text(&mut s, fonts, face, 13.33, SOFT, Point::new(24.0, 124.0), &format!("drawn in {avg:.1} ms a frame, in software"));
            }
        }
        // A row of shapes that fills the width, so resizing shows.
        let n = ((w - 48.0) / 96.0).floor().max(1.0) as usize;
        for i in 0..n {
            let x = 24.0 + i as f32 * 96.0;
            let c = if i % 2 == 0 { TEAL } else { AMBER };
            s.fill_rounded(Rect::new(x, 150.0, 80.0, 48.0), (i * 6) as f32, c);
            s.stroke(Rect::new(x, 212.0, 80.0, 48.0), (i * 6) as f32, 1.5, c);
        }
        // A hairline grid: one unit wide, which is one, two or three sharp pixels by the scale.
        let mut y = 280.0;
        while y < h - 40.0 {
            s.fill(Rect::new(24.0, y, w - 48.0, 1.0), Color::rgba(255, 255, 255, 28));
            y += 20.0;
        }
        if self.animate {
            // Back and forth across the window every four seconds.
            let phase = (f.time / 4.0).fract();
            let t = if phase < 0.5 { phase * 2.0 } else { 2.0 - phase * 2.0 } as f32;
            s.fill_rounded(Rect::new(24.0 + t * (w - 48.0 - 60.0), h - 30.0, 60.0, 12.0), 6.0, AMBER);
        }
        self.last = Some((w, h, f.scale));
        s
    }

    fn shown(&mut self, f: &Frame, draw_ms: f64) -> bool {
        self.draw_ms.push(draw_ms);
        match self.stop_after {
            Some(n) if f.number + 1 >= n => {
                let (w, h, scale) = self.last.unwrap_or_default();
                let mean = self.draw_ms.iter().sum::<f64>() / self.draw_ms.len() as f64;
                println!("{} frames in {:.2} s: {:.1} a second; {w} x {h} units at scale {scale:.4}; drawing {mean:.1} ms a frame (first {:.1})", f.number + 1, f.time, (f.number + 1) as f64 / f.time.max(0.001), self.draw_ms[0]);
                false
            }
            _ => true,
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok());
    let mut demo = Demo { animate: flag("--animate"), own_frame: flag("--own-frame"), stop_after: value("--frames"), draw_ms: Vec::new(), last: None };
    if let Err(e) = vergence_ui::run(&mut demo, (640, 420)) {
        eprintln!("vergence-ui: {e}");
        std::process::exit(1);
    }
}
