//! The real helper, confined, on archives made here: ordinary ones, hostile
//! ones (`..`, absolute names, links out, a file written through a link,
//! setuid bits, lies about sizes), and Compress's .zip read back.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use vergence_archive::make::{self, Format, Raw};
use vergence_archive::unpack::{self, Progress};

const HELPER: &str = env!("CARGO_BIN_EXE_vergence-unpack");

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vergence-archive-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn file(name: &str, data: &[u8]) -> Raw {
    Raw { name: name.into(), perm: 0o644, data: data.to_vec(), ..Default::default() }
}

fn dir(name: &str) -> Raw {
    Raw { name: name.into(), kind: 0o040000, perm: 0o755, ..Default::default() }
}

fn symlink(name: &str, target: &str) -> Raw {
    Raw { name: name.into(), kind: 0o120000, perm: 0o777, symlink: Some(target.into()), ..Default::default() }
}

fn run(archive: &Path, into: &Path) -> Result<unpack::Extracted, unpack::Error> {
    let stop = Arc::new(AtomicBool::new(false));
    let listing = unpack::list_with(Path::new(HELPER), archive, &stop)?;
    unpack::extract_with(Path::new(HELPER), archive, into, &listing, &Progress::default())
}

#[test]
fn the_helper_is_confined() {
    assert_eq!(
        unpack::check_confined(Path::new(HELPER)).unwrap(),
        Some(if cfg!(target_os = "linux") { 128 + 31 } else { 4 })
    );
}

#[test]
fn an_ordinary_archive_in_each_format() {
    for format in [Format::TarGz, Format::Zip] {
        let d = scratch(&format!("ordinary-{format:?}"));
        let a = d.join(if format == Format::Zip { "photos.zip" } else { "photos.tar.gz" });
        make::write_raw(
            &a,
            format,
            &[file("one.txt", b"one"), dir("sub"), file("sub/two.txt", b"two"), file("é.txt", b"e")],
        )
        .unwrap();
        let out = run(&a, &d).unwrap();
        assert_eq!(out.made, d.join("photos"), "loose files go in a folder named after it");
        assert_eq!(out.skipped, 0);
        assert_eq!(fs::read(out.made.join("one.txt")).unwrap(), b"one");
        assert_eq!(fs::read(out.made.join("sub/two.txt")).unwrap(), b"two");
        assert_eq!(fs::read(out.made.join("é.txt")).unwrap(), b"e");
        // Again: a free name, nothing replaced.
        assert_eq!(run(&a, &d).unwrap().made, d.join("photos 2"));
        let hidden: Vec<_> = fs::read_dir(&d)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(hidden.is_empty(), "no hidden folder left: {hidden:?}");
    }
}

#[test]
fn one_folder_at_the_top_is_kept_as_it_is() {
    let d = scratch("one-folder");
    let a = d.join("project-1.2.tar.gz");
    make::write_raw(&a, Format::TarGz, &[dir("project-1.2"), file("project-1.2/README", b"hi")]).unwrap();
    let out = run(&a, &d).unwrap();
    assert_eq!(out.made, d.join("project-1.2"));
    assert_eq!(fs::read(out.made.join("README")).unwrap(), b"hi");
}

#[test]
fn hostile_names_and_links_are_skipped() {
    let d = scratch("hostile");
    let outside = d.join("outside");
    fs::create_dir(&outside).unwrap();
    let a = d.join("evil.tar.gz");
    make::write_raw(
        &a,
        Format::TarGz,
        &[
            file("ok.txt", b"ok"),
            file("../escaped.txt", b"x"),
            file("a/../../escaped2.txt", b"x"),
            file(&format!("{}/abs.txt", outside.display()), b"x"),
            file("C:/drive.txt", b"x"),
            file("back\\..\\..\\slash.txt", b"x"),
            // A link out, then a file through it.
            symlink("out", "../outside"),
            file("out/through.txt", b"x"),
            symlink("abs", "/etc"),
            symlink("inside", "ok.txt"),
            symlink("sub/up", "../ok.txt"),
            Raw { name: b"setuid".to_vec(), perm: 0o4755, data: b"s".to_vec(), ..Default::default() },
            Raw { name: b"hard".to_vec(), hardlink: Some(b"ok.txt".to_vec()), ..Default::default() },
            Raw { name: b"hardout".to_vec(), hardlink: Some(b"../outside/x".to_vec()), ..Default::default() },
        ],
    )
    .unwrap();
    let out = run(&a, &d).unwrap();
    assert!(fs::read_dir(&outside).unwrap().next().is_none(), "nothing written outside");
    for n in ["escaped.txt", "escaped2.txt", "slash.txt"] {
        assert!(!d.join(n).exists(), "{n}");
    }
    let m = &out.made;
    assert_eq!(fs::read(m.join("ok.txt")).unwrap(), b"ok");
    // "out/through.txt" was made in a real folder named "out"; the link "out" then could not be.
    assert!(m.join("out").symlink_metadata().unwrap().is_dir());
    assert!(m.join("abs").symlink_metadata().is_err(), "absolute link not made");
    assert_eq!(fs::read_link(m.join("inside")).unwrap(), Path::new("ok.txt"));
    assert_eq!(fs::read(m.join("sub/up")).unwrap(), b"ok");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(fs::metadata(m.join("setuid")).unwrap().permissions().mode() & 0o7777, 0o755);
    assert_eq!(fs::read(m.join("hard")).unwrap(), b"ok");
    assert!(m.join("hardout").symlink_metadata().is_err());
    // ../escaped, a/../../escaped2, the absolute one, C:, the backslashes, out (link), abs, hardout.
    assert_eq!(out.skipped, 8);
}

#[test]
fn a_size_lie_is_stopped_and_leaves_nothing() {
    let d = scratch("lie");
    let a = d.join("lie.tar.gz");
    // The header says 3 bytes; the data is a megabyte and more.
    let big = vec![b'z'; 3 << 20];
    make::write_raw(&a, Format::TarGz, &[file("small", &big)]).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let mut listing = unpack::list_with(Path::new(HELPER), &a, &stop).unwrap();
    assert_eq!(listing.total, 3 << 20);
    listing.total = 3; // as if the listing had been lied to
    let r = unpack::extract_with(Path::new(HELPER), &a, &d, &listing, &Progress::default());
    assert!(matches!(r, Err(unpack::Error::Unreadable(_))), "{r:?}");
    let left: Vec<_> = fs::read_dir(&d).unwrap().filter_map(Result::ok).map(|e| e.file_name()).collect();
    assert_eq!(left, vec![std::ffi::OsString::from("lie.tar.gz")]);
}

#[test]
fn not_an_archive_says_so() {
    let d = scratch("not");
    let a = d.join("text.zip");
    fs::write(&a, b"just some words, not a zip at all").unwrap();
    let r = run(&a, &d);
    assert!(matches!(r, Err(unpack::Error::Unreadable(_))), "{r:?}");
}

#[test]
fn the_caps_ask() {
    let l = unpack::Listing { entries: 10, total: 30 << 30, ..Default::default() };
    assert!(l.ask(1 << 30).unwrap().contains("30.0 GB"));
    let l = unpack::Listing { entries: 200_000, total: 1, ..Default::default() };
    assert!(l.ask(1 << 20).unwrap().contains("200000"));
    let l = unpack::Listing { entries: 1, total: 2 << 30, ..Default::default() };
    assert!(l.ask(1 << 20).is_some(), "2 GB from 1 MB");
    assert!(l.ask(100 << 20).is_none(), "2 GB from 100 MB");
}

#[test]
fn compress_makes_a_zip_that_reads_back() {
    let d = scratch("compress");
    let src = d.join("src");
    fs::create_dir_all(src.join("folder/inner")).unwrap();
    fs::write(src.join("folder/a.txt"), b"alpha").unwrap();
    fs::write(src.join("folder/inner/b.bin"), vec![9u8; 300_000]).unwrap();
    fs::write(src.join("note.md"), b"# note").unwrap();
    fs::write(src.join("folder/café.txt"), b"cafe").unwrap();
    std::os::unix::fs::symlink("a.txt", src.join("folder/link")).unwrap();
    let to = d.join("Archive.zip");
    let items = vec![src.join("folder"), src.join("note.md")];
    let p = Progress::default();
    make::compress(&items, &to, &p).unwrap();
    assert_eq!(p.done.load(std::sync::atomic::Ordering::Relaxed), make::size_of(&items));
    assert!(make::compress(&items, &to, &Progress::default()).is_err(), "never replaces");
    let out = run(&to, &d).unwrap();
    assert_eq!(out.made, d.join("Archive"));
    assert_eq!(fs::read(out.made.join("folder/a.txt")).unwrap(), b"alpha");
    assert_eq!(fs::read(out.made.join("folder/inner/b.bin")).unwrap(), vec![9u8; 300_000]);
    assert_eq!(fs::read(out.made.join("note.md")).unwrap(), b"# note");
    assert_eq!(fs::read(out.made.join("folder/café.txt")).unwrap(), b"cafe");
    assert_eq!(fs::read_link(out.made.join("folder/link")).unwrap(), Path::new("a.txt"));
}

#[test]
fn names_are_cleaned() {
    assert_eq!(unpack::clean(b"./a//b/./c").unwrap(), [b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    assert!(unpack::clean(b"a\0b").is_none());
    assert!(unpack::clean(b"a/../b").is_none());
    assert!(unpack::clean(b"/a").is_none());
    assert!(unpack::clean(b"c:\\x").is_none());
    assert!(unpack::clean(b"./").is_none());
    assert_eq!(unpack::stem(Path::new("x/Photos.TAR.GZ")), "Photos");
    assert_eq!(unpack::stem(Path::new("a.b.zip")), "a.b");
}

/// The review (2026-11-23), finding 1: links were checked against the
/// hidden folder, but only its one folder is placed, so `..` reached beside it.
#[test]
fn a_link_cannot_climb_out_of_the_one_folder() {
    let d = scratch("one-folder-link");
    fs::create_dir(d.join("Documents")).unwrap();
    fs::write(d.join("Documents/private.txt"), b"mine").unwrap();
    let a = d.join("proj.tar.gz");
    make::write_raw(
        &a,
        Format::TarGz,
        &[
            dir("proj"),
            file("proj/file", b"f"),
            symlink("proj/evil", "../Documents"),
            symlink("proj/self", "../proj/file"),
            symlink("proj/fine", "file"),
        ],
    )
    .unwrap();
    let out = run(&a, &d).unwrap();
    assert_eq!(out.made, d.join("proj"));
    assert!(out.made.join("evil").symlink_metadata().is_err(), "a link beside the placed folder not made");
    assert!(out.made.join("self").symlink_metadata().is_err(), "nor one that climbs out and back by name");
    assert_eq!(fs::read(out.made.join("fine")).unwrap(), b"f");
    assert_eq!(out.skipped, 2);
}

/// Finding 2: a link through another link: `a/up -> ..` makes the text's
/// depth not the real one.
#[test]
fn a_chain_of_links_cannot_climb_out() {
    let d = scratch("chain");
    let a = d.join("chain.tar.gz");
    make::write_raw(
        &a,
        Format::TarGz,
        &[dir("a"), symlink("a/up", ".."), symlink("p", "a/up/a/up/a/up/../../../etc/passwd"), symlink("q", "a/up/x")],
    )
    .unwrap();
    let out = run(&a, &d).unwrap();
    let m = &out.made;
    assert_eq!(fs::read_link(m.join("a/up")).unwrap(), Path::new(".."));
    assert!(m.join("p").symlink_metadata().is_err(), "through a link: not made");
    assert!(m.join("q").symlink_metadata().is_err(), "through a link even inside: not made");
    assert_eq!(out.skipped, 2);
    for e in walk(m) {
        if let Ok(real) = e.canonicalize() {
            assert!(real.starts_with(m.canonicalize().unwrap()), "{e:?} leads to {real:?}");
        }
    }
}

fn walk(p: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(p).unwrap().filter_map(Result::ok) {
        let path = e.path();
        out.push(path.clone());
        if e.file_type().unwrap().is_dir() {
            out.extend(walk(&path));
        }
    }
    out
}

/// Finding 7: names that are not UTF-8 stay as bytes, so two of them never
/// become one; finding 6: nothing is made writable by others.
#[test]
fn raw_names_stay_apart_and_nothing_is_writable_by_others() {
    let d = scratch("raw");
    let a = d.join("old.tar.gz");
    make::write_raw(
        &a,
        Format::TarGz,
        &[
            Raw { name: b"caf\x82.txt".to_vec(), perm: 0o644, data: b"one".to_vec(), ..Default::default() },
            Raw { name: b"caf\x8a.txt".to_vec(), perm: 0o777, data: b"two".to_vec(), ..Default::default() },
        ],
    )
    .unwrap();
    let out = run(&a, &d).unwrap();
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    let one = out.made.join(std::ffi::OsStr::from_bytes(b"caf\x82.txt"));
    let two = out.made.join(std::ffi::OsStr::from_bytes(b"caf\x8a.txt"));
    assert_eq!(fs::read(&one).unwrap(), b"one");
    assert_eq!(fs::read(&two).unwrap(), b"two");
    assert_eq!(fs::metadata(&two).unwrap().permissions().mode() & 0o777, 0o755);
}
