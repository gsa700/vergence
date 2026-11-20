//! Reads every PDF under the folders given (PDF milestone 1, step 9's
//! corpus): opens each, walks its pages, decodes their content streams, and
//! reports what it found. A panic is caught and counted (there must be
//! none). The corpus itself stays on the dev boxes, never in the repository.
//!
//!     cargo run --release -p vergence-pdf --example pdfcorpus -- ~/pdf-corpus [--list]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vergence_pdf::{Document, Error};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pdf")) {
            out.push(p);
        }
    }
}

#[derive(Default)]
struct Totals {
    files: usize,
    opened: usize,
    not_pdf: usize,
    no_catalog: usize,
    panics: usize,
    repaired: usize,
    encrypted: usize,
    no_pages: usize,
    no_pages_plain: usize,
    locked: usize,
    revisions: std::collections::BTreeMap<i64, usize>,
    bad_plain: usize,
    pages_plain: usize,
    pages: usize,
    empty_pages: usize,
    bad_streams: usize,
    slowest: Vec<(Duration, PathBuf)>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let list = args.iter().any(|a| a == "--list");
    let render = args.iter().any(|a| a == "--render");
    let mut files = Vec::new();
    for a in args.iter().filter(|a| !a.starts_with("--")) {
        walk(Path::new(a), &mut files);
    }
    files.sort();
    let mut t = Totals { files: files.len(), ..Default::default() };
    let started = Instant::now();
    for f in &files {
        let Ok(bytes) = std::fs::read(f) else { continue };
        let at = Instant::now();
        let outcome = std::panic::catch_unwind(|| {
            let doc = Document::open(bytes)?;
            let pages = doc.pages();
            let mut empty = 0;
            let mut bad = 0;
            for p in &pages {
                let c = doc.contents(p);
                if c.iter().all(|b| b.is_ascii_whitespace()) {
                    empty += 1;
                }
                // Each content stream's own decode.
                if let vergence_pdf::Object::Stream(s) = doc.get_in(&p.dict, "Contents")
                    && !doc.stream(&s).ok
                {
                    bad += 1;
                }
            }
            // The first page drawn at 1x (step 3), when asked.
            if render && let Some(p) = pages.first() {
                let _ = vergence_pdf::render_page(&doc, p, 1.0);
            }
            Ok::<_, Error>((doc.repaired(), doc.encrypted(), pages.len(), empty, bad, doc.needs_password(), doc.encryption_revision()))
        });
        let took = at.elapsed();
        t.slowest.push((took, f.clone()));
        t.slowest.sort_by_key(|s| std::cmp::Reverse(s.0));
        t.slowest.truncate(5);
        match outcome {
            Err(_) => {
                t.panics += 1;
                println!("PANIC  {}", f.display());
            }
            Ok(Err(Error::NotPdf)) => {
                t.not_pdf += 1;
                if list {
                    println!("not a PDF  {}", f.display());
                }
            }
            Ok(Err(Error::NoCatalog)) => {
                t.no_catalog += 1;
                println!("no catalog  {}", f.display());
            }
            Ok(Ok((repaired, encrypted, pages, empty, bad, locked, revision))) => {
                t.locked += locked as usize;
                if let Some(r) = revision {
                    *t.revisions.entry(r).or_default() += 1;
                }
                if locked {
                    println!("needs a password  {}", f.display());
                }
                t.opened += 1;
                t.repaired += repaired as usize;
                t.encrypted += encrypted as usize;
                t.pages += pages;
                t.empty_pages += empty;
                t.bad_streams += bad;
                if !encrypted {
                    t.bad_plain += bad;
                    t.pages_plain += pages;
                }
                if pages == 0 {
                    t.no_pages += 1;
                    if !encrypted {
                        t.no_pages_plain += 1;
                        println!("no pages (not encrypted)  {}", f.display());
                    }
                }
                if list && (repaired || bad > 0) {
                    println!("{}{}  {}", if repaired { "repaired " } else { "" }, if bad > 0 { format!("{bad} bad streams") } else { String::new() }, f.display());
                }
            }
        }
    }
    println!("\n{} files in {:.1} s", t.files, started.elapsed().as_secs_f64());
    println!("opened {}  (repaired {}, encrypted {}, with no pages {})", t.opened, t.repaired, t.encrypted, t.no_pages);
    println!("not PDF {}  no catalog {}  PANICS {}", t.not_pdf, t.no_catalog, t.panics);
    println!("pages {}  (empty content {}; content streams that did not decode cleanly {})", t.pages, t.empty_pages, t.bad_streams);
    println!("encrypted by revision {:?}; need a password {}", t.revisions, t.locked);
    println!("not encrypted: pages {}, files with no pages {}, content streams not clean {}", t.pages_plain, t.no_pages_plain, t.bad_plain);
    println!("slowest:");
    for (d, p) in &t.slowest {
        println!("  {:.0} ms  {}", d.as_secs_f64() * 1000.0, p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    }
}
