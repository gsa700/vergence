//! Draws one page of a PDF to a PPM (P6) to look at (PDF milestone 1,
//! step 3: everything but text, which is step 4).
//!
//!     cargo run --release -p vergence-pdf --example pdfrender -- file.pdf [page] [scale] [out.ppm]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let file = args.first().expect("a PDF file");
    let page: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
    let scale: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let out = args.get(3).cloned().unwrap_or_else(|| "page.ppm".into());
    let doc = vergence_pdf::Document::open(std::fs::read(file).expect("read")).expect("open");
    let pages = doc.pages();
    let p = pages.get(page - 1).expect("no such page");
    let t = std::time::Instant::now();
    let c = vergence_pdf::render_page(&doc, p, scale).expect("drawn");
    eprintln!(
        "page {page} of {}: {} x {} px in {:.0} ms",
        pages.len(),
        c.width,
        c.height,
        t.elapsed().as_secs_f64() * 1000.0
    );
    let mut ppm = format!("P6 {} {} 255\n", c.width, c.height).into_bytes();
    for y in 0..c.height {
        for x in 0..c.width {
            ppm.extend_from_slice(&c.pixel(x, y)[..3]);
        }
    }
    std::fs::write(&out, ppm).expect("write");
}
