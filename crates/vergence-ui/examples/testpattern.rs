//! A window for the display checks (tools/display-check.sh, testing method
//! layer 2, 2026-11-18): flat magenta, with a line one pixel wide on each
//! edge in its own color, top red, bottom blue, left yellow, right cyan,
//! drawn at the window's scale so each is exactly one row or column of the
//! screen's pixels. A shot of the screen then says, pixel by pixel, whether
//! the window was shown whole (all four lines there), where it starts and
//! ends, and what the server drew next to it (its bar above).
//!
//!     cargo build --release -p vergence-ui --example testpattern
//!     vctl spawn ~/src/vergence/target/release/examples/testpattern --size 580 420

use vergence_scene::{Color, Rect, Scene};
use vergence_text::Fonts;
use vergence_ui::{App, Frame};

pub const BODY: Color = Color::rgb(200, 40, 160);
pub const TOP: Color = Color::rgb(255, 0, 0);
pub const BOTTOM: Color = Color::rgb(0, 0, 255);
pub const LEFT: Color = Color::rgb(255, 255, 0);
pub const RIGHT: Color = Color::rgb(0, 255, 255);

struct Pattern;

impl App for Pattern {
    fn title(&self) -> String {
        "Test pattern".into()
    }

    fn app_id(&self) -> String {
        "vergence-testpattern".into()
    }

    fn scene(&mut self, f: &Frame, _fonts: &mut Fonts) -> Scene {
        let (w, h) = (f.width, f.height);
        let mut s = Scene::new(w, h);
        // One pixel of the screen, in points.
        let p = 1.0 / f.scale.max(0.01);
        s.fill(Rect::new(0.0, 0.0, w, h), BODY);
        s.fill(Rect::new(0.0, 0.0, p, h), LEFT);
        s.fill(Rect::new(w - p, 0.0, p, h), RIGHT);
        s.fill(Rect::new(0.0, 0.0, w, p), TOP);
        s.fill(Rect::new(0.0, h - p, w, p), BOTTOM);
        s
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let size = args
        .iter()
        .position(|a| a == "--size")
        .and_then(|i| Some((args.get(i + 1)?.parse().ok()?, args.get(i + 2)?.parse().ok()?)))
        .unwrap_or((580, 420));
    if let Err(e) = vergence_ui::run(&mut Pattern, size) {
        eprintln!("testpattern: {e}");
        std::process::exit(1);
    }
}
