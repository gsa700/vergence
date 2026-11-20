//! Files made here, read back: a classic table, an xref stream with an
//! object stream, an incremental update, damage that needs repair, and
//! random damage that must never panic or hang.

use super::*;

/// A PDF with these objects (numbered from 1, the catalog first), a table
/// with the right offsets, and a trailer.
fn classic(objects: &[&str]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn stream_obj(dict_extra: &str, data: &[u8]) -> String {
    format!("<< /Length {} {} >>\nstream\n{}\nendstream", data.len(), dict_extra, String::from_utf8_lossy(data))
}

fn three_pages() -> Vec<u8> {
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 595 842] /Resources << /Font << /F1 6 0 R >> >> >>",
        "<< /Type /Page /Parent 2 0 R /Contents 7 0 R >>",
        "<< /Type /Page /Parent 2 0 R /Rotate 90 /MediaBox [0 0 612 792] >>",
        "<< /Type /Page /Parent 2 0 R /Rotate -90 /Contents [7 0 R 7 0 R] >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        &stream_obj("", b"BT /F1 12 Tf (Hi) Tj ET"),
    ])
}

#[test]
fn a_classic_file_is_read() {
    let doc = Document::open(three_pages()).expect("opens");
    assert!(!doc.repaired());
    assert_eq!(doc.version().as_deref(), Some("1.7"));
    let pages = doc.pages();
    assert_eq!(pages.len(), 3);
    assert_eq!(pages[0].media_box, [0.0, 0.0, 595.0, 842.0], "inherited");
    assert_eq!(pages[1].media_box, [0.0, 0.0, 612.0, 792.0], "its own");
    assert_eq!((pages[0].rotate, pages[1].rotate, pages[2].rotate), (0, 90, 270));
    let font = pages[0].resources.as_ref().unwrap().get("Font").unwrap().as_dict().unwrap().get("F1").unwrap();
    assert_eq!(doc.resolve(font).as_dict().unwrap().name("BaseFont"), Some(&b"Helvetica"[..]));
    assert_eq!(doc.contents(&pages[0]), b"BT /F1 12 Tf (Hi) Tj ET\n");
    assert_eq!(doc.contents(&pages[2]), b"BT /F1 12 Tf (Hi) Tj ET\nBT /F1 12 Tf (Hi) Tj ET\n");
}

#[test]
fn wrong_offsets_are_found_anyway() {
    // Every offset in the table off by 7: objects are looked up by scanning.
    let mut f = three_pages();
    let at = lexer::rfind(&f, b"xref\n").unwrap();
    let table = String::from_utf8_lossy(&f[at..]).into_owned();
    let shifted: String = table
        .lines()
        .map(|l| {
            if l.len() == 18 && l.ends_with(" n") {
                let off: usize = l[..10].parse().unwrap();
                format!("{:010}{}\n", off + 7, &l[10..])
            } else {
                format!("{l}\n")
            }
        })
        .collect();
    f.truncate(at);
    f.extend_from_slice(shifted.as_bytes());
    let doc = Document::open(f).expect("opens");
    assert_eq!(doc.pages().len(), 3);
}

#[test]
fn no_table_at_all_is_repaired() {
    let mut f = three_pages();
    let at = lexer::rfind(&f, b"xref\n").unwrap();
    f.truncate(at);
    f.extend_from_slice(b"trailer\n<< /Root 1 0 R >>\n%%EOF\n");
    let doc = Document::open(f).expect("repaired");
    assert!(doc.repaired());
    assert_eq!(doc.pages().len(), 3);
    // Not even a trailer: the catalog is found by its /Type.
    let mut g = three_pages();
    let at = lexer::rfind(&g, b"xref\n").unwrap();
    g.truncate(at);
    let doc = Document::open(g).expect("repaired without a trailer");
    assert_eq!(doc.pages().len(), 3);
}

#[test]
fn an_incremental_update_wins() {
    let mut f = three_pages();
    let first_xref = {
        let at = lexer::rfind(&f, b"startxref").unwrap();
        let mut l = Lexer::new(&f, at + 9);
        l.uint().unwrap()
    };
    // Object 4 again, rotated 180, with a new section pointing back.
    let off = f.len();
    f.extend_from_slice(b"4 0 obj\n<< /Type /Page /Parent 2 0 R /Rotate 180 >>\nendobj\n");
    let xref = f.len();
    f.extend_from_slice(format!("xref\n4 1\n{off:010} 00000 n \ntrailer\n<< /Size 8 /Root 1 0 R /Prev {first_xref} >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    let doc = Document::open(f).expect("opens");
    assert!(!doc.repaired());
    let pages = doc.pages();
    assert_eq!(pages.len(), 3);
    assert_eq!(pages[1].rotate, 180, "the newer object 4");
}

#[test]
fn an_xref_stream_and_an_object_stream() {
    // Objects 1-3 in object stream 4; the cross-references in stream 5.
    let inner = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] >>"];
    let mut body = String::new();
    let mut header = String::new();
    for (i, o) in inner.iter().enumerate() {
        header.push_str(&format!("{} {} ", i + 1, body.len()));
        body.push_str(o);
        body.push(' ');
    }
    let objstm = format!("{header}{body}");
    let mut f = b"%PDF-1.5\n".to_vec();
    let off4 = f.len();
    f.extend_from_slice(format!("4 0 obj\n<< /Type /ObjStm /N 3 /First {} /Length {} >>\nstream\n{objstm}\nendstream\nendobj\n", header.len(), objstm.len()).as_bytes());
    let off5 = f.len();
    // W [1 2 1]: type, offset or stream number, generation or index.
    let mut rows: Vec<u8> = vec![0, 0, 0, 255];
    for i in 0..3u8 {
        rows.extend_from_slice(&[2, 0, 4, i]);
    }
    rows.extend_from_slice(&[1, (off4 >> 8) as u8, off4 as u8, 0]);
    rows.extend_from_slice(&[1, (off5 >> 8) as u8, off5 as u8, 0]);
    f.extend_from_slice(format!("5 0 obj\n<< /Type /XRef /Size 6 /W [1 2 1] /Root 1 0 R /Length {} >>\nstream\n", rows.len()).as_bytes());
    f.extend_from_slice(&rows);
    f.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{off5}\n%%EOF\n").as_bytes());
    let doc = Document::open(f.clone()).expect("opens");
    assert!(!doc.repaired());
    let pages = doc.pages();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].media_box, [0.0, 0.0, 100.0, 200.0]);
    // Without its startxref the same file is repaired, object stream members included.
    let at = lexer::rfind(&f, b"startxref").unwrap();
    f.truncate(at);
    let doc = Document::open(f).expect("repaired");
    assert!(doc.repaired());
    assert_eq!(doc.pages().len(), 1);
}

#[test]
fn flate_streams_are_decoded() {
    // Python's zlib.compress(b"BT (compressed) Tj ET", 9).
    let z: [u8; 29] = [0x78, 0xda, 0x73, 0x0a, 0x51, 0xd0, 0x48, 0xce, 0xcf, 0x2d, 0x28, 0x4a, 0x2d, 0x2e, 0x4e, 0x4d, 0xd1, 0x54, 0x08, 0xc9, 0x52, 0x70, 0x0d, 0x01, 0x00, 0x4c, 0x50, 0x06, 0xd4];
    let mut f = b"%PDF-1.4\n".to_vec();
    let o1 = f.len();
    f.extend_from_slice(b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n");
    let o2 = f.len();
    f.extend_from_slice(b"2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n");
    let o3 = f.len();
    f.extend_from_slice(b"3 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R >> endobj\n");
    let o4 = f.len();
    f.extend_from_slice(b"4 0 obj << /Length 5 0 R /Filter /FlateDecode >>\nstream\n");
    f.extend_from_slice(&z);
    f.extend_from_slice(b"\nendstream endobj\n");
    let o5 = f.len();
    f.extend_from_slice(format!("5 0 obj {} endobj\n", z.len()).as_bytes());
    let xref = f.len();
    f.extend_from_slice(format!("xref\n0 6\n0000000000 65535 f \n{o1:010} 00000 n \n{o2:010} 00000 n \n{o3:010} 00000 n \n{o4:010} 00000 n \n{o5:010} 00000 n \ntrailer << /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF").as_bytes());
    let doc = Document::open(f).unwrap();
    let p = &doc.pages()[0];
    assert_eq!(doc.contents(p), b"BT (compressed) Tj ET\n");
}

#[test]
fn cycles_and_lies_do_not_loop() {
    // Pages whose kids point back at themselves, references in a ring.
    let f = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [2 0 R 3 0 R 4 0 R] /Count 999999 >>",
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>",
        "6 0 R",
        "<< /Length 5 0 R >>\nstream\nabc\nendstream",
        "4 0 R",
    ]);
    let doc = Document::open(f).unwrap();
    let pages = doc.pages();
    assert_eq!(pages.len(), 1, "the loop back to the root is cut, and the ring 4 -> 6 -> 4 resolves to nothing");
    assert_eq!(doc.resolve(&Object::Ref(Ref { num: 4, generation: 0 })), Object::Null);
    let _ = doc.contents(&pages[0]);
}

#[test]
fn not_a_pdf_and_no_catalog() {
    assert_eq!(Document::open(b"hello".to_vec()).err(), Some(Error::NotPdf));
    assert_eq!(Document::open(b"%PDF-1.4\n1 0 obj << /A 1 >> endobj\n".to_vec()).err(), Some(Error::NoCatalog));
}

#[test]
fn random_damage_never_panics() {
    let base = three_pages();
    let iters: usize = std::env::var("FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for i in 0..iters {
        let mut f = base.clone();
        match i % 4 {
            0 => {
                for _ in 0..1 + next() % 8 {
                    let k = next() as usize % f.len();
                    f[k] = next() as u8;
                }
            }
            1 => {
                let k = next() as usize % f.len();
                f.truncate(k);
            }
            2 => {
                let k = next() as usize % f.len();
                let n = (next() % 40) as usize;
                f.drain(k..(k + n).min(f.len()));
            }
            _ => {
                let k = next() as usize % f.len();
                let junk: Vec<u8> = (0..next() % 32).map(|_| b"<<>>[]()/0 R obj endobj stream"[next() as usize % 30]).collect();
                f.splice(k..k, junk);
            }
        }
        if let Ok(doc) = Document::open(f) {
            for p in doc.pages().iter().take(10) {
                let _ = doc.contents(p);
            }
        }
    }
}
