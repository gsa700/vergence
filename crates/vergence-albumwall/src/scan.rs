//! Scanning a folder library (the Avalonia app's LibraryScanner.Scan;
//! docs/albumwall-milestone-1.md, step 2).
//!
//! **The files on disk are the truth.** Listed first, read second: walking
//! the tree takes a fraction of a second for 18,000 files where reading
//! them takes minutes on a cold disk, and knowing the total first is what
//! lets the wait be shown as progress (a bare "scanning…" was reported as
//! a hang). With the index trusted, a file whose path, size and time it
//! knows is not opened; without (Rescan), every file is, and the index is
//! rebuilt from what was read.
//!
//! **A library that cannot be reached is said so, not read as empty**: a
//! folder that is not there, or that is empty while it held music last
//! time (an unmounted share is an empty folder), or that went away during
//! the scan. Concluding that every album was deleted would empty the wall
//! and prune the index, and the next time the share is there the whole
//! first scan is paid again. A file listed and then not readable is not
//! remembered as unreadable (a share that drops for a moment would leave
//! albums missing until their files changed): it is simply not seen.
//!
//! Every file's bytes are read by Hymie's helper (helper::Scanner), only
//! their ends; pictures beside the tracks are measured there too.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use vergence_hymie::helper::Scanner;

use crate::index::{Index, Kept, KeptArt};
use crate::library::{self, Album, Art, Seen, TrackFacts};

/// How far a scan has come: files gone through of the total, albums so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    /// Files actually opened so far (none on an unchanged library with the index trusted).
    pub opened: usize,
}

/// Why a library could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreachable {
    /// Its folder is not there.
    Missing,
    /// Its folder is empty, and held music last time.
    Empty,
    /// It could not be listed, or went away while it was read.
    Failed(String),
}

/// What a scan found.
pub struct Found {
    pub albums: Vec<Album>,
    /// Files listed, files opened.
    pub files: usize,
    pub opened: usize,
}

/// The files of `root` with their size and modified time (nanoseconds),
/// music only, in a stable order. Folders reached through a link are not
/// followed (a link back up would never end); hidden ones are skipped.
pub fn list(root: &Path) -> std::io::Result<Vec<(PathBuf, u64, i64)>> {
    let mut out = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(dir) = folders.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            // The library's own folder must be listed; a folder inside it
            // that refuses us is passed over (as a file that cannot be read).
            Err(e) if dir == root => return Err(e),
            Err(_) => continue,
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                folders.push(path);
            } else if library::is_audio(&path) {
                let Ok(meta) = std::fs::metadata(&path) else { continue };
                if meta.is_file() {
                    out.push((path, meta.len(), modified_of(&meta)));
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

fn modified_of(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos().min(i64::MAX as u128) as i64)
}

/// Scans `root`: its albums in the wall's order, the index updated with
/// what was read. `trust`: files the index knows are not opened.
/// `expect_music`: the library held music last time (an empty folder is
/// then an absent one). `cancel` stops it between files; what was read is
/// still kept in the index.
pub fn scan(
    root: &Path,
    index: &mut Index,
    trust: bool,
    expect_music: bool,
    scanner: &mut Scanner,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<Found, Unreachable> {
    if !root.is_dir() {
        return Err(Unreachable::Missing);
    }
    let files = list(root).map_err(|e| Unreachable::Failed(e.to_string()))?;
    if files.is_empty() && (expect_music || (trust && index.has_files_under(root))) {
        return Err(Unreachable::Empty);
    }
    let total = files.len();
    progress(Progress { done: 0, total, opened: 0 });
    let mut seen: Vec<Seen> = Vec::with_capacity(total);
    let mut opened = 0;
    let mut gaps = false;
    for (n, (path, size, modified)) in files.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let facts = match index.find(&path, size, modified).filter(|_| trust) {
            Some(f) => f.clone(),
            None => {
                opened += 1;
                match scanner.facts(&path) {
                    Ok(f) => TrackFacts::from_hymie(&path, size, &f),
                    // Not readable now (gone, the share dropped): not a fact about it.
                    Err(vergence_hymie::Error::Io(_)) => {
                        if !root.is_dir() {
                            return Err(Unreachable::Failed("it went away while it was read".into()));
                        }
                        gaps = true;
                        continue;
                    }
                    // Opened and found not to be music Hymie plays: remembered.
                    Err(_) => TrackFacts::unreadable(),
                }
            }
        };
        index.tracks.insert(path.clone(), Kept { size, modified, facts: facts.clone() });
        seen.push(Seen { path, size, modified, facts });
        if n % 25 == 0 {
            progress(Progress { done: n + 1, total, opened });
        }
    }
    if gaps && seen.is_empty() {
        return Err(Unreachable::Failed("not one file could be read".into()));
    }
    // A complete scan forgets what is no longer under the library; one cut
    // short, or with gaps, concludes nothing about what is gone.
    if !gaps && seen.len() == total {
        let here: std::collections::HashSet<&Path> = seen.iter().map(|s| s.path.as_path()).collect();
        index.tracks.retain(|p, _| !p.starts_with(root) || here.contains(p.as_path()));
    }
    let mut measured: HashMap<PathBuf, KeptArt> = HashMap::new();
    let albums = library::group(&seen, |folder, names| {
        let (path, size, modified) = sidecar(folder, names)?;
        let (width, height) = match index.find_art(&path, size, modified).filter(|_| trust) {
            Some(wh) => wh,
            None => scanner.image_size(&path).unwrap_or((0, 0)),
        };
        measured.insert(path.clone(), KeptArt { size, modified, width, height });
        Some(Art { path, embedded: false, width, height, size, modified })
    });
    index.art.retain(|p, _| !p.starts_with(root));
    index.art.extend(measured);
    progress(Progress { done: total, total, opened });
    Ok(Found { albums, files: total, opened })
}

/// The first of `names` in `folder`, in the order given, whatever its case
/// (Windows-era libraries write Folder.jpg): the exact name looked for
/// first, so a library spelled our way never lists a folder.
fn sidecar(folder: &Path, names: &[&str]) -> Option<(PathBuf, u64, i64)> {
    let found = |p: PathBuf| {
        let meta = std::fs::metadata(&p).ok().filter(|m| m.is_file())?;
        Some((p, meta.len(), modified_of(&meta)))
    };
    for name in names {
        if let Some(f) = found(folder.join(name)) {
            return Some(f);
        }
    }
    let present: Vec<String> = std::fs::read_dir(folder)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    for name in names {
        if let Some(p) = present.iter().find(|p| p.eq_ignore_ascii_case(name)) {
            return found(folder.join(p));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("albumwall-scan-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn listing_finds_music_only_and_skips_hidden_folders() {
        let root = scratch("list");
        std::fs::create_dir_all(root.join("Toto/1982 - IV")).unwrap();
        std::fs::create_dir_all(root.join(".Trash/x")).unwrap();
        for f in ["Toto/1982 - IV/01.flac", "Toto/1982 - IV/cover.jpg", "Toto/notes.txt", ".Trash/x/02.flac"] {
            std::fs::write(root.join(f), b"x").unwrap();
        }
        let files: Vec<PathBuf> = list(&root).unwrap().into_iter().map(|f| f.0).collect();
        assert_eq!(files, [root.join("Toto/1982 - IV/01.flac")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn sidecars_are_found_whatever_their_case() {
        let root = scratch("side");
        std::fs::write(root.join("Folder.JPG"), b"x").unwrap();
        let (p, ..) = sidecar(&root, &library::ART_NAMES).unwrap();
        assert_eq!(p, root.join("Folder.JPG"));
        assert!(sidecar(&root, &library::BACK_NAMES).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_or_emptied_library_is_unreachable_not_empty() {
        let mut s = Scanner::with_program(Path::new("/nonexistent"));
        let no = AtomicBool::new(false);
        let mut index = Index::default();
        let root = scratch("gone");
        let missing = root.join("not-here");
        assert_eq!(scan(&missing, &mut index, true, false, &mut s, &no, |_| {}).err(), Some(Unreachable::Missing));
        // Empty, and it held music last time: absent, until a Rescan says otherwise.
        assert_eq!(scan(&root, &mut index, true, true, &mut s, &no, |_| {}).err(), Some(Unreachable::Empty));
        index.tracks.insert(root.join("a.flac"), Kept { size: 1, modified: 1, facts: TrackFacts::unreadable() });
        assert_eq!(scan(&root, &mut index, true, false, &mut s, &no, |_| {}).err(), Some(Unreachable::Empty));
        let found = scan(&root, &mut index, false, false, &mut s, &no, |_| {}).unwrap();
        assert!(found.albums.is_empty());
        assert!(!index.has_files_under(&root), "an honest rescan forgets what is gone");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
