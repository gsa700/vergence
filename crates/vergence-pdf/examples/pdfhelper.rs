//! Draws a page through the confined helper, as Close-Up does:
//! `pdfhelper FILE PAGE SCALE OUT.ppm` (the helper from this target/).

use std::io::Write;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 5 {
        eprintln!("usage: pdfhelper FILE PAGE SCALE OUT.ppm");
        std::process::exit(2);
    }
    let exe = std::env::current_exe().unwrap();
    let helper = exe.parent().unwrap().parent().unwrap().join("vergence-pdf");
    let data = std::fs::read(&a[1]).expect("read");
    let t0 = std::time::Instant::now();
    let mut v = vergence_pdf::helper::Viewer::with_bytes(helper, data).expect("open");
    let page: usize = a[2].parse::<usize>().unwrap().saturating_sub(1);
    let t = v.render(page, a[3].parse().unwrap(), None).expect("render");
    let mut out = std::fs::File::create(&a[4]).unwrap();
    writeln!(out, "P6 {} {} 255", t.width, t.height).unwrap();
    let rgb: Vec<u8> = t.rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    out.write_all(&rgb).unwrap();
    println!("page {} of {}: {} x {} px in {} ms (helper)", page + 1, v.info.pages.len(), t.width, t.height, t0.elapsed().as_millis());
}
