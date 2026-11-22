//! The type sample (text milestone 1, step 7), drawn into any scene: by
//! vergence-ui's `typesample` example on a screen, and by vergence-render's
//! `vscene type` straight to a PNG with no session.

use vergence_render::text;
use vergence_scene::{Color, Point, Rect, Scene};
use vergence_text::Fonts;

const SIZES: [f32; 8] = [11.0, 12.0, 13.0, 13.5, 14.0, 16.0, 20.0, 28.0];

/// A light half and a dark half (the palette's page and text).
pub fn draw(s: &mut Scene, fonts: &mut Fonts, w: f32, h: f32) {
    let half = (w / 2.0).floor();
    half_page(s, fonts, 0.0, half, h, Color::rgb(255, 255, 255), Color::rgb(29, 29, 31), Color::rgb(110, 110, 115));
    half_page(
        s,
        fonts,
        half,
        w - half,
        h,
        Color::rgb(30, 30, 32),
        Color::rgb(240, 240, 242),
        Color::rgb(150, 150, 156),
    );
}

#[allow(clippy::too_many_arguments)]
fn half_page(s: &mut Scene, fonts: &mut Fonts, x: f32, w: f32, h: f32, page: Color, ink: Color, dim: Color) {
    s.fill(Rect::new(x, 0.0, w, h), page);
    let left = x + 16.0;
    let mut y = 12.0;
    let Some(sans) = fonts.face("sans-serif", 400, false) else { return };
    for size in SIZES {
        y += size * 1.3;
        text(s, fonts, sans, 9.0, dim, Point::new(left, y), &format!("{size}"));
        text(s, fonts, sans, size, ink, Point::new(left + 26.0, y), "AVATAR To Ty Yo r. Wave 0123");
    }
    y += 26.0;
    for (weight, name) in [(400u16, "Regular"), (500, "Medium"), (600, "Semibold"), (700, "Bold")] {
        if let Some(face) = fonts.face("sans-serif", weight, false) {
            text(
                s,
                fonts,
                face,
                13.0,
                ink,
                Point::new(left, y),
                &format!("{name} 13: The quick brown fox jumps over the lazy dog."),
            );
            y += 18.0;
        }
    }
    y += 10.0;
    let para = [
        "Typography is the craft of endowing human language with a",
        "durable visual form. Settings, Edit, FileView and the dock all",
        "draw with this text: menus at 13.5, titles and labels at 13.",
    ];
    for line in para {
        text(s, fonts, sans, 13.0, ink, Point::new(left, y), line);
        y += 18.0;
    }
    y += 10.0;
    if let Some(mono) = fonts.face("monospace", 400, false) {
        for line in ["fn main() { let x = 1 + 1; // == 2 }", "il1| O0 {}[]() <= >= != -> => ..."] {
            text(s, fonts, mono, 12.0, ink, Point::new(left, y), line);
            y += 17.0;
        }
    }
}
