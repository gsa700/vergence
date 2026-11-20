//! A test card for the rasterizer, written as a PPM (P6) to look at:
//! curves, both fill rules, joins, caps, dashes, a clip, a stretched pen.
//!
//!     cargo run --release -p vergence-raster --example rastercard -- card.ppm

use vergence_raster::{Canvas, Cap, Clip, FillRule, Join, Path, Stroke, Transform};

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "card.ppm".into());
    let (w, h) = (640, 400);
    let mut c = Canvas::new(w, h).expect("canvas");
    c.clear([1.0, 1.0, 1.0, 1.0]);
    let id = Transform::IDENTITY;
    let ink = [0.1, 0.1, 0.12, 1.0];
    // Fill rules: a star drawn once, filled both ways.
    let star = |cx: f64, cy: f64| {
        let mut p = Path::new();
        for i in 0..5 {
            let a = -std::f64::consts::FRAC_PI_2 + i as f64 * 4.0 * std::f64::consts::PI / 5.0;
            let (x, y) = (cx + 60.0 * a.cos(), cy + 60.0 * a.sin());
            if i == 0 { p.move_to(x, y) } else { p.line_to(x, y) };
        }
        p.close();
        p
    };
    c.fill(&star(80.0, 90.0), &id, FillRule::NonZero, [0.85, 0.2, 0.2, 1.0], None);
    c.fill(&star(220.0, 90.0), &id, FillRule::EvenOdd, [0.2, 0.4, 0.85, 1.0], None);
    // Curves: a circle and a ring, clipped to a band.
    let mut ring = Path::new();
    ring.circle(380.0, 90.0, 60.0).circle(380.0, 90.0, 30.0);
    let mut band = Path::new();
    band.rect(320.0, 50.0, 120.0, 70.0);
    let clip = Clip::full(w, h).intersect(&band, &id, FillRule::NonZero, w, h);
    c.fill(&ring, &id, FillRule::EvenOdd, [0.2, 0.6, 0.3, 1.0], Some(&clip));
    // A stretched pen: a circle stroked under a 3 x 1 scale.
    let mut small = Path::new();
    small.circle(0.0, 0.0, 15.0);
    c.stroke(&small, &Transform::scale(3.0, 1.0).then(&Transform::translate(540.0, 90.0)), &Stroke { width: 4.0, ..Default::default() }, ink, None);
    // Joins and caps.
    let zig = |x: f64| {
        let mut p = Path::new();
        p.move_to(x, 330.0).line_to(x + 40.0, 220.0).line_to(x + 80.0, 330.0);
        p
    };
    for (i, (join, cap)) in [(Join::Miter, Cap::Butt), (Join::Round, Cap::Round), (Join::Bevel, Cap::Square)].iter().enumerate() {
        let s = Stroke { width: 18.0, join: *join, cap: *cap, ..Default::default() };
        c.stroke(&zig(30.0 + i as f64 * 120.0), &id, &s, ink, None);
    }
    // Dashes, a hairline, and a dotted line of zero-length dashes with round caps.
    let mut line = Path::new();
    line.move_to(400.0, 230.0).line_to(620.0, 230.0);
    c.stroke(&line, &id, &Stroke { width: 6.0, dash: vec![20.0, 10.0], ..Default::default() }, ink, None);
    let mut hair = Path::new();
    hair.move_to(400.0, 260.0).line_to(620.0, 300.0);
    c.stroke(&hair, &id, &Stroke { width: 0.0, ..Default::default() }, ink, None);
    let mut dots = Path::new();
    dots.move_to(400.0, 330.0).line_to(620.0, 330.0);
    c.stroke(&dots, &id, &Stroke { width: 10.0, cap: Cap::Round, dash: vec![0.0, 20.0], ..Default::default() }, [0.85, 0.2, 0.2, 1.0], None);
    let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
    for y in 0..h {
        for x in 0..w {
            ppm.extend_from_slice(&c.pixel(x, y)[..3]);
        }
    }
    std::fs::write(&out, ppm).expect("write");
    println!("wrote {out}");
}
