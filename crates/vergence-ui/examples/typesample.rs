//! A type sample for text milestone 1's checks (docs/text-milestone-1.md,
//! step 7): Inter and JetBrains Mono at the sizes the desktop uses, kerning
//! pairs, a paragraph, and the same on a light and a dark half, all fixed,
//! so shots before and after a change can be put side by side.
//!
//!     cargo build --release -p vergence-ui --example typesample
//!     vctl spawn ~/src/vergence/target/release/examples/typesample --size 960 600

use vergence_scene::Scene;
use vergence_text::Fonts;
use vergence_ui::{App, Frame};

#[path = "../../vergence-render/examples/shared/typesample.rs"]
mod typesample;

struct Sample;

impl App for Sample {
    fn title(&self) -> String {
        "Type sample".into()
    }

    fn app_id(&self) -> String {
        "vergence-typesample".into()
    }

    fn scene(&mut self, f: &Frame, fonts: &mut Fonts) -> Scene {
        let (w, h) = (f.width, f.height);
        let mut s = Scene::new(w, h);
        typesample::draw(&mut s, fonts, w, h);
        s
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let size = args
        .iter()
        .position(|a| a == "--size")
        .and_then(|i| Some((args.get(i + 1)?.parse().ok()?, args.get(i + 2)?.parse().ok()?)))
        .unwrap_or((960, 600));
    if let Err(e) = vergence_ui::run(&mut Sample, size) {
        eprintln!("typesample: {e}");
        std::process::exit(1);
    }
}
