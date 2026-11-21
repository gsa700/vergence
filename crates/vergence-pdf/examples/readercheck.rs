//! Reader's requests over a corpus (Reader milestone 1, step 7): each PDF
//! below the folders opened through the confined helper as Reader opens
//! it, then asked for what Reader asks: what it says about itself (`M`),
//! its outline, the first page's text and links, and the first page drawn
//! at thumbnail size. Counts what opened, what is locked, what failed and
//! how (a refusal is the helper saying no; a crash or hang is a bug), and
//! the slowest files.
//!
//!     readercheck FOLDER... [--max N]
//!
//! The helper is the one in this target/ (`cargo build --release -p
//! vergence-pdf` builds both).

use std::path::{Path, PathBuf};
use std::time::Instant;

fn pdfs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            pdfs(&p, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            out.push(p);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let max = args.iter().position(|a| a == "--max").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let folders: Vec<&String> = args.iter().filter(|a| !a.starts_with("--") && a.parse::<usize>().is_err()).collect();
    if folders.is_empty() {
        eprintln!("usage: readercheck FOLDER... [--max N]");
        std::process::exit(2);
    }
    let mut files = Vec::new();
    for f in &folders {
        pdfs(Path::new(f), &mut files);
    }
    files.truncate(max);
    let exe = std::env::current_exe().unwrap();
    let helper = exe.parent().unwrap().parent().unwrap().join("vergence-pdf");
    let (mut opened, mut locked, mut titled, mut with_outline) = (0, 0, 0, 0);
    let mut problems: Vec<(PathBuf, String)> = Vec::new();
    let mut slowest: Vec<(u128, PathBuf)> = Vec::new();
    let started = Instant::now();
    for (i, path) in files.iter().enumerate() {
        let t0 = Instant::now();
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                problems.push((path.clone(), format!("read: {e}")));
                continue;
            }
        };
        let mut v = match vergence_pdf::helper::Viewer::with_bytes(helper.clone(), data) {
            Ok(v) => v,
            Err(e) => {
                problems.push((path.clone(), format!("open: {e}")));
                continue;
            }
        };
        if v.info.needs_password {
            locked += 1;
            continue;
        }
        let mut what = Vec::new();
        match v.metadata() {
            Ok(m) => {
                if m.iter().any(|(k, t)| k == "Title" && !t.trim().is_empty()) {
                    titled += 1;
                }
            }
            Err(e) => what.push(format!("metadata: {e}")),
        }
        match v.outline() {
            Ok(o) => with_outline += !o.is_empty() as usize,
            Err(e) => what.push(format!("outline: {e}")),
        }
        if !v.info.pages.is_empty() {
            if let Err(e) = v.text(0) {
                what.push(format!("text: {e}"));
            }
            if let Err(e) = v.links(0) {
                what.push(format!("links: {e}"));
            }
            let (w, _) = v.info.pages[0];
            if let Err(e) = v.render(0, (132.0 / w.max(1.0)).min(4.0), None) {
                what.push(format!("thumbnail: {e}"));
            }
        }
        if what.is_empty() {
            opened += 1;
        } else {
            problems.push((path.clone(), what.join("; ")));
        }
        slowest.push((t0.elapsed().as_millis(), path.clone()));
        if (i + 1) % 100 == 0 {
            eprintln!("{} of {}", i + 1, files.len());
        }
    }
    slowest.sort_by(|a, b| b.0.cmp(&a.0));
    println!("{} files in {:.0} s: {opened} opened and read, {locked} locked, {} with problems; {titled} with a title, {with_outline} with an outline", files.len(), started.elapsed().as_secs_f64(), problems.len());
    let crashed = problems.iter().filter(|(_, w)| w.contains("the PDF reader stopped")).count();
    println!("helper crashes or hangs: {crashed}");
    for (p, w) in problems.iter().take(40) {
        println!("  {}: {w}", p.display());
    }
    println!("slowest:");
    for (ms, p) in slowest.iter().take(5) {
        println!("  {ms} ms {}", p.display());
    }
    if crashed > 0 {
        std::process::exit(1);
    }
}
