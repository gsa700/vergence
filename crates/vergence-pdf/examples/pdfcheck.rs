//! PDF milestone 1, step 9: the corpus checks.
//!
//! `render` draws sample pages of every PDF under the folders given (the
//! first, the middle and the last, up to --pages) into DIR as PPM files,
//! with an index (index.tsv: key, page, milliseconds, width, height, file,
//! scale; pages over 16M pixels are drawn smaller)
//! and a timing summary. A panic is caught and counted (there must be none).
//! Run it once and keep DIR as the reference pictures; after a change, run it
//! again into a new folder and `compare` the two: every page whose pixels
//! changed is listed, to be looked at.
//!
//! `time` only draws (nothing written) and reports the timing: kepler's
//! check is a typical page in well under 100 ms.
//!
//! The corpus and every picture drawn from it stay on the dev boxes, never
//! in the repository. pdf.js's pictures for the by-eye pass come from
//! tools/pdfcheck/pdfjs-render.mjs, reading the same index.
//!
//!     cargo run --release -p vergence-pdf --example pdfcheck -- render ~/pdf-corpus --out ~/pdfcheck/ref [--scale 1.5] [--pages 3]
//!     cargo run --release -p vergence-pdf --example pdfcheck -- time ~/pdf-corpus [--scale 1.5] [--pages 3]
//!     cargo run --release -p vergence-pdf --example pdfcheck -- compare ~/pdfcheck/ref ~/pdfcheck/new

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use vergence_pdf::{Document, render_page};

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

/// A short name for a file, from its path below the corpus folder (FNV-1a).
fn key(rel: &Path) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in rel.to_string_lossy().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The pages to draw, numbered from 1: the first, the last, then spread between.
fn sample(count: usize, want: usize) -> Vec<usize> {
    let mut v: Vec<usize> = match (count, want) {
        (0, _) | (_, 0) => Vec::new(),
        (_, 1) => vec![1],
        _ => (0..want).map(|i| 1 + i * (count - 1) / (want - 1)).collect(),
    };
    v.dedup();
    v
}

fn arg<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn write_ppm(path: &Path, c: &vergence_raster::Canvas) -> std::io::Result<()> {
    let mut out = format!("P6 {} {} 255\n", c.width, c.height).into_bytes();
    out.reserve(c.width * c.height * 3);
    // The ground is white and opaque, so premultiplied is plain RGB here.
    for px in c.data.chunks_exact(4) {
        out.extend_from_slice(&px[..3]);
    }
    std::fs::write(path, out)
}

fn read_ppm(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let data = std::fs::read(path).ok()?;
    let mut fields = Vec::new();
    let mut i = 0;
    while fields.len() < 4 {
        while data.get(i)?.is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while !data.get(i)?.is_ascii_whitespace() {
            i += 1;
        }
        fields.push(std::str::from_utf8(&data[start..i]).ok()?.to_string());
    }
    let (w, h): (usize, usize) = (fields[1].parse().ok()?, fields[2].parse().ok()?);
    let body = data.get(i + 1..i + 1 + w * h * 3)?.to_vec();
    Some((w, h, body))
}

fn render(args: &[String], out: Option<&Path>) {
    let scale: f64 = arg(args, "--scale", 1.5);
    let want: usize = arg(args, "--pages", 3);
    let skip_value = |i: usize| i > 0 && matches!(args[i - 1].as_str(), "--out" | "--scale" | "--pages");
    let roots: Vec<&String> = args.iter().enumerate().filter(|(i, a)| !a.starts_with("--") && !skip_value(*i)).map(|(_, a)| a).collect();
    let mut files = Vec::new();
    for r in &roots {
        let before = files.len();
        walk(Path::new(r), &mut files);
        for f in &mut files[before..] {
            *f = PathBuf::from(r).join(f.strip_prefix(r).unwrap_or(f));
        }
    }
    files.sort();
    let mut index = out.map(|d| {
        std::fs::create_dir_all(d).expect("the output folder");
        std::fs::File::create(d.join("index.tsv")).expect("index.tsv")
    });
    let (mut times, mut panics, mut failed, mut locked) = (Vec::new(), 0, 0, 0);
    let mut slowest: Vec<(f64, String)> = Vec::new();
    let started = Instant::now();
    for f in &files {
        let rel = roots.iter().find_map(|r| f.strip_prefix(r).ok()).unwrap_or(f);
        let k = key(rel);
        let Ok(bytes) = std::fs::read(f) else { continue };
        let outcome = std::panic::catch_unwind(|| {
            let Ok(doc) = Document::open(bytes) else { return Err(()) };
            if doc.needs_password() {
                return Ok(None);
            }
            let pages = doc.pages();
            let mut drawn = Vec::new();
            for n in sample(pages.len(), want) {
                // Huge scans (75-inch pages) at most 16M pixels, as Close-Up's untiled view.
                let (w, h) = vergence_pdf::page_size(&pages[n - 1]);
                let s = scale.min((16_777_216.0 / (w * h).max(1.0)).sqrt());
                let at = Instant::now();
                let c = render_page(&doc, &pages[n - 1], s);
                drawn.push((n, s, at.elapsed().as_secs_f64() * 1000.0, c));
            }
            Ok(Some(drawn))
        });
        match outcome {
            Err(_) => {
                panics += 1;
                println!("PANIC  {}", f.display());
            }
            Ok(Err(())) => failed += 1,
            Ok(Ok(None)) => locked += 1,
            Ok(Ok(Some(drawn))) => {
                for (n, s, ms, c) in drawn {
                    let Some(c) = c else {
                        println!("not drawn  page {n}  {}", f.display());
                        continue;
                    };
                    times.push(ms);
                    slowest.push((ms, format!("page {n}  {}", rel.display())));
                    if let (Some(d), Some(ix)) = (out, index.as_mut()) {
                        write_ppm(&d.join(format!("{k}-p{n}.ppm")), &c).expect("a picture");
                        writeln!(ix, "{k}\t{n}\t{ms:.1}\t{}\t{}\t{}\t{s}", c.width, c.height, f.display()).expect("index.tsv");
                    }
                }
                slowest.sort_by(|a, b| b.0.total_cmp(&a.0));
                slowest.truncate(10);
            }
        }
    }
    times.sort_by(f64::total_cmp);
    let at = |q: f64| times.get(((times.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0.0);
    println!("\n{} files, {} pages drawn at {scale}x in {:.1} s", files.len(), times.len(), started.elapsed().as_secs_f64());
    println!("not opened {failed}  need a password {locked}  PANICS {panics}");
    println!("ms per page: median {:.1}  90% {:.1}  99% {:.1}  max {:.1}", at(0.5), at(0.9), at(0.99), at(1.0));
    println!("pages under 100 ms: {:.1}%", 100.0 * times.iter().filter(|t| **t < 100.0).count() as f64 / times.len().max(1) as f64);
    println!("slowest:");
    for (ms, what) in &slowest {
        println!("  {ms:.0} ms  {what}");
    }
}

fn compare(reference: &Path, new: &Path) {
    let (mut same, mut changed, mut missing) = (0, Vec::new(), 0);
    let mut names: Vec<_> = std::fs::read_dir(reference).expect("the reference folder").flatten().map(|e| e.file_name()).collect();
    names.sort();
    for name in names.iter().filter(|n| n.to_string_lossy().ends_with(".ppm")) {
        let (Some(a), Some(b)) = (read_ppm(&reference.join(name)), read_ppm(&new.join(name))) else {
            missing += 1;
            println!("missing  {}", name.to_string_lossy());
            continue;
        };
        if a.0 != b.0 || a.1 != b.1 {
            changed.push((f64::INFINITY, name.to_string_lossy().into_owned()));
            continue;
        }
        // Pixels whose color moved by more than a rounding step.
        let moved = a.2.chunks_exact(3).zip(b.2.chunks_exact(3)).filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 2)).count();
        if moved == 0 {
            same += 1;
        } else {
            changed.push((100.0 * moved as f64 / (a.0 * a.1) as f64, name.to_string_lossy().into_owned()));
        }
    }
    changed.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (pct, name) in &changed {
        if pct.is_infinite() {
            println!("size changed  {name}");
        } else {
            println!("{pct:7.3}% of pixels  {name}");
        }
    }
    println!("\n{same} pages the same, {} changed, {missing} missing", changed.len());
    if !changed.is_empty() || missing > 0 {
        std::process::exit(1);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("render") => {
            let out: PathBuf = arg(&args, "--out", PathBuf::new());
            assert!(!out.as_os_str().is_empty(), "render needs --out DIR");
            render(&args[1..], Some(&out));
        }
        Some("time") => render(&args[1..], None),
        Some("compare") if args.len() == 3 => compare(Path::new(&args[1]), Path::new(&args[2])),
        _ => eprintln!("pdfcheck render FOLDER... --out DIR [--scale S] [--pages N] | time FOLDER... | compare REF NEW"),
    }
}
