//! What vergence-text finds on this machine, for checking one over SSH:
//! `cargo run --release -p vergence-text --example fonts [text]`.
//! Prints how long the font directories took to index, the faces the generic
//! names resolve to, and which face draws each character of the text.

use std::time::Instant;

use vergence_text::Fonts;

fn main() {
    let text = std::env::args().nth(1).unwrap_or_else(|| "Aé─א漢🙂".to_string());
    let t = Instant::now();
    let mut fonts = Fonts::system();
    let scan = t.elapsed();
    let faces = &fonts.db().faces;
    let files: std::collections::BTreeSet<_> = faces.iter().map(|e| &e.path).collect();
    let families: std::collections::BTreeSet<_> = faces.iter().map(|e| e.info.family.as_str()).collect();
    println!(
        "{} faces in {} files, {} families, indexed in {:.1} ms",
        faces.len(),
        files.len(),
        families.len(),
        scan.as_secs_f64() * 1e3
    );
    println!(
        "{} fixed-pitch, {} variable in weight",
        faces.iter().filter(|e| e.info.monospace).count(),
        faces.iter().filter(|e| e.info.weight_axis.is_some()).count()
    );

    for (family, weight) in [("monospace", 400), ("monospace", 700), ("sans-serif", 400), ("serif", 400)] {
        match fonts.face(family, weight, false) {
            Some(f) => {
                let n = fonts.name(f);
                println!("{family} {weight}: {} {}{}", n.family, n.weight, if n.italic { " italic" } else { "" });
            }
            None => println!("{family} {weight}: nothing"),
        }
    }

    let Some(mono) = fonts.face("monospace", 400, false) else { return };
    for c in text.chars() {
        let t = Instant::now();
        let (face, glyph) = fonts.glyph(mono, c);
        let took = t.elapsed();
        let size = fonts
            .raster(face, glyph, 48.0, 0)
            .map(|b| format!("{}x{} px at 48", b.width, b.height))
            .unwrap_or_else(|| "no outline".into());
        let n = fonts.name(face);
        println!(
            "U+{:04X} {c}: {} {} glyph {glyph} ({size}; found in {:.2} ms)",
            c as u32,
            n.family,
            n.weight,
            took.as_secs_f64() * 1e3
        );
    }
}
