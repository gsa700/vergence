//! Lists a page's fonts (and those of the forms it draws, one level down):
//! resource name, subtype, base font, encoding, and how the program is
//! embedded (FontFile, FontFile2, FontFile3 and its subtype) or that it is
//! not. A tool for font bugs (PDF milestone 1, step 9).
//!
//!     cargo run --release -p vergence-pdf --example pdffonts -- FILE PAGE [DUMP-DIR]
//!
//! With a folder, each embedded program is also written there as
//! `<resource name>.bin`, to look into.

use vergence_pdf::Document;
use vergence_pdf::object::{Dict, Object};

fn show(doc: &Document, label: &str, font: &Object, dump: Option<&str>) {
    let Some(f) = doc.resolve(font).as_dict().cloned() else { return };
    let name = |d: &Dict, k: &str| {
        doc.get_in(d, k).as_name().map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_else(|| "-".into())
    };
    let mut desc = doc.get_in(&f, "FontDescriptor").as_dict().cloned();
    let mut detail = String::new();
    if let Some(Object::Array(kids)) = Some(doc.get_in(&f, "DescendantFonts"))
        && let Some(kid) = kids.first().and_then(|k| doc.resolve(k).as_dict().cloned())
    {
        detail = format!(" descendant {} {}", name(&kid, "Subtype"), name(&kid, "BaseFont"));
        desc = doc.get_in(&kid, "FontDescriptor").as_dict().cloned();
    }
    let embedded = desc.as_ref().map_or("no descriptor".to_string(), |d| {
        for k in ["FontFile", "FontFile2", "FontFile3"] {
            if let Object::Stream(s) = doc.get_in(d, k) {
                let sub =
                    s.dict.name("Subtype").map(|n| format!(" /{}", String::from_utf8_lossy(n))).unwrap_or_default();
                let data = doc.stream(&s).data;
                if let Some(dir) = dump {
                    let _ = std::fs::write(format!("{dir}/{}.bin", label.replace('/', "_")), &data);
                }
                return format!("{k}{sub}, {} bytes", data.len());
            }
        }
        "not embedded".into()
    });
    let enc = match doc.get_in(&f, "Encoding") {
        Object::Name(n) => String::from_utf8_lossy(&n).into_owned(),
        Object::Dict(_) => "dictionary".into(),
        Object::Null => "-".into(),
        _ => "other".into(),
    };
    println!("{label}: {} {}{detail}, encoding {enc}, {embedded}", name(&f, "Subtype"), name(&f, "BaseFont"));
}

fn fonts(doc: &Document, res: &Dict, prefix: &str, depth: usize, dump: Option<&str>) {
    if let Some(fs) = doc.get_in(res, "Font").as_dict() {
        for (k, v) in &fs.0 {
            show(doc, &format!("{prefix}{}", String::from_utf8_lossy(k)), v, dump);
        }
    }
    if depth == 0 {
        return;
    }
    if let Some(xs) = doc.get_in(res, "XObject").as_dict() {
        for (k, v) in &xs.0 {
            if let Object::Stream(s) = doc.resolve(v)
                && s.dict.name("Subtype") == Some(b"Form".as_slice())
                && let Some(r) = doc.get_in(&s.dict, "Resources").as_dict()
            {
                fonts(doc, r, &format!("{prefix}{}/", String::from_utf8_lossy(k)), depth - 1, dump);
            }
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 3 && a.len() != 4 {
        eprintln!("usage: pdffonts FILE PAGE [DUMP-DIR]");
        std::process::exit(2);
    }
    let doc = Document::open(std::fs::read(&a[1]).expect("read")).expect("open");
    let pages = doc.pages();
    let i = a[2].parse::<usize>().unwrap_or(1).saturating_sub(1);
    let Some(page) = pages.get(i) else { return };
    if let Some(r) = &page.resources {
        fonts(&doc, r, "", 2, a.get(3).map(String::as_str));
    }
}
