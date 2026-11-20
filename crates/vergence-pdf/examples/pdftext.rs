//! Prints a page's text layer as the viewer joins it: `pdftext FILE PAGE`.

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 3 {
        eprintln!("usage: pdftext FILE PAGE");
        std::process::exit(2);
    }
    let doc = vergence_pdf::Document::open(std::fs::read(&a[1]).expect("read")).expect("open");
    let pages = doc.pages();
    let i = a[2].parse::<usize>().unwrap_or(1).saturating_sub(1);
    let t0 = std::time::Instant::now();
    let chars = vergence_pdf::page_text(&doc, &pages[i]);
    let took = t0.elapsed();
    println!("{}", vergence_pdf::text::join(&chars).0);
    eprintln!("page {} of {}: {} characters in {} ms", i + 1, pages.len(), chars.len(), took.as_millis());
}
