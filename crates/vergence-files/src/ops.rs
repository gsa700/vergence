//! File Viewer's file operations (files milestone 1, step 7): the
//! freedesktop Trash, names that do not clash, copying a tree with progress
//! and a way to stop, moving (a rename where it can be, a copy and a delete
//! where it cannot), and what undo needs to put things back. Plain functions
//! on paths; the window (main.rs) runs the long ones on a worker.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The freedesktop Trash in the user's data folder: its `files` and `info`.
pub fn trash_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("Trash"))
}

/// A path %-escaped as the Trash's info files write it (the spec's URL
/// path form: everything but unreserved characters and `/`).
pub fn escape(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::new();
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The reverse of [`escape`].
pub fn unescape(s: &str) -> PathBuf {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (b.get(i + 1).and_then(|c| hex(*c)), b.get(i + 2).and_then(|c| hex(*c)))
        {
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(out))
}

/// A name in `dir` not taken yet, from `name`: itself, then "name 2.ext",
/// "name 3.ext", and so on (the extension kept; a dot file's whole name is
/// its stem). `taken` says whether a name is in use.
pub fn free_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..10_000).map(|n| format!("{stem} {n}{ext}")).find(|n| !taken(n)).unwrap_or_else(|| format!("{stem} copy{ext}"))
}

/// The name a copy gets in its own folder: "name copy.ext", then "name copy 2.ext".
pub fn copy_name(name: &str, dir: &Path) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    free_name(&format!("{stem} copy{ext}"), |n| dir.join(n).symlink_metadata().is_ok())
}

/// One item in the Trash: where its info says it came from, its name there, when.
#[derive(Debug, Clone, PartialEq)]
pub struct Trashed {
    pub name: String,
    pub from: PathBuf,
    pub when: String,
}

/// Moves `path` to the Trash: an info file first (made new, so two never
/// share a name), then the file itself renamed in; across file systems it is
/// copied in and the original removed. Returns the name it has there.
pub fn trash(path: &Path) -> io::Result<String> {
    let dir = trash_dir().ok_or_else(|| io::Error::other("no Trash folder (no home)"))?;
    let (files, info) = (dir.join("files"), dir.join("info"));
    fs::create_dir_all(&files)?;
    fs::create_dir_all(&info)?;
    let name = path.file_name().ok_or_else(|| io::Error::other("no name"))?.to_string_lossy().into_owned();
    let path = std::path::absolute(path)?;
    for _ in 0..50 {
        let n = free_name(&name, |n| files.join(n).symlink_metadata().is_ok() || info.join(format!("{n}.trashinfo")).exists());
        let info_path = info.join(format!("{n}.trashinfo"));
        let text = format!("[Trash Info]\nPath={}\nDeletionDate={}\n", escape(&path), vergence_widgets::files::local_stamp(std::time::SystemTime::now()));
        let made = fs::OpenOptions::new().write(true).create_new(true).open(&info_path);
        let mut f = match made {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        f.write_all(text.as_bytes())?;
        drop(f);
        let dest = files.join(&n);
        let moved = match fs::rename(&path, &dest) {
            Ok(()) => Ok(()),
            Err(e) if e.raw_os_error() == Some(18) => copy_tree(&path, &dest, &Progress::default()).and_then(|_| remove_tree(&path)),
            Err(e) => Err(e),
        };
        if let Err(e) = moved {
            let _ = fs::remove_file(&info_path);
            return Err(e);
        }
        return Ok(n);
    }
    Err(io::Error::other("no free name in the Trash"))
}

/// What is in the Trash, newest first.
pub fn trashed() -> Vec<Trashed> {
    let Some(dir) = trash_dir() else { return Vec::new() };
    let mut out: Vec<Trashed> = fs::read_dir(dir.join("info"))
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let file = e.file_name().to_string_lossy().into_owned();
                    let name = file.strip_suffix(".trashinfo")?.to_string();
                    let text = fs::read_to_string(e.path()).ok()?;
                    let get = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)).map(str::trim).map(str::to_string);
                    Some(Trashed { name, from: unescape(&get("Path=")?), when: get("DeletionDate=").unwrap_or_default() })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.when.cmp(&a.when));
    out
}

/// Puts a trashed item back where it came from (a free name there if that
/// one is taken); its info file goes. Returns where it went.
pub fn restore(name: &str) -> io::Result<PathBuf> {
    let dir = trash_dir().ok_or_else(|| io::Error::other("no Trash folder"))?;
    let item = trashed().into_iter().find(|t| t.name == name).ok_or_else(|| io::Error::other("not in the Trash"))?;
    let parent = item.from.parent().ok_or_else(|| io::Error::other("no folder to go back to"))?.to_path_buf();
    fs::create_dir_all(&parent)?;
    let want = item.from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string());
    let to = parent.join(free_name(&want, |n| parent.join(n).symlink_metadata().is_ok()));
    move_item(&dir.join("files").join(name), &to, &Progress::default())?;
    let _ = fs::remove_file(dir.join("info").join(format!("{name}.trashinfo")));
    Ok(to)
}

/// Empties the Trash: every file and its info.
pub fn empty_trash() -> io::Result<()> {
    let dir = trash_dir().ok_or_else(|| io::Error::other("no Trash folder"))?;
    for sub in ["files", "info"] {
        if let Ok(rd) = fs::read_dir(dir.join(sub)) {
            for e in rd.flatten() {
                remove_tree(&e.path())?;
            }
        }
    }
    Ok(())
}

/// How a long job is doing: bytes done of all, the file now, and a flag to stop.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub now: Mutex<String>,
    pub stop: AtomicBool,
}

/// The bytes a tree holds (files only; links not followed).
pub fn tree_size(p: &Path) -> u64 {
    let Ok(m) = p.symlink_metadata() else { return 0 };
    if m.is_dir() {
        fs::read_dir(p).map(|rd| rd.flatten().map(|e| tree_size(&e.path())).sum()).unwrap_or(0)
    } else {
        m.len()
    }
}

fn stopped() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "stopped")
}

/// Copies a file, a folder with all in it, or a link (as a link), to `to`,
/// which must not exist. Progress counts bytes; `stop` ends it between
/// pieces (what was copied so far stays, to be removed by the caller).
pub fn copy_tree(from: &Path, to: &Path, progress: &Progress) -> io::Result<()> {
    if progress.stop.load(Ordering::Relaxed) {
        return Err(stopped());
    }
    let m = from.symlink_metadata()?;
    if m.file_type().is_symlink() {
        return std::os::unix::fs::symlink(fs::read_link(from)?, to);
    }
    if m.is_dir() {
        if to.starts_with(from) {
            return Err(io::Error::other("a folder cannot be copied into itself"));
        }
        fs::create_dir(to)?;
        for e in fs::read_dir(from)?.flatten() {
            copy_tree(&e.path(), &to.join(e.file_name()), progress)?;
        }
        let _ = fs::set_permissions(to, m.permissions());
        return Ok(());
    }
    if let Some(n) = from.file_name() {
        *progress.now.lock().unwrap_or_else(|e| e.into_inner()) = n.to_string_lossy().into_owned();
    }
    if !m.is_file() {
        return Err(io::Error::other("only files, folders and links are copied (not a pipe, a device or a socket)"));
    }
    let mut src = vergence_types::open_regular(from)?;
    let mut dst = fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if progress.stop.load(Ordering::Relaxed) {
            drop(dst);
            let _ = fs::remove_file(to);
            return Err(stopped());
        }
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])?;
        progress.done.fetch_add(n as u64, Ordering::Relaxed);
    }
    dst.set_permissions(m.permissions())?;
    if let Ok(t) = m.modified() {
        let _ = dst.set_modified(t);
    }
    Ok(())
}

/// Removes a file, a link, or a folder and all in it.
pub fn remove_tree(p: &Path) -> io::Result<()> {
    let m = p.symlink_metadata()?;
    if m.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }
}

/// Moves `from` to `to` (which must not exist): renamed if both are on one
/// file system, else copied and the original removed.
pub fn move_item(from: &Path, to: &Path, progress: &Progress) -> io::Result<()> {
    if to.starts_with(from) && from != to {
        return Err(io::Error::other("a folder cannot be moved into itself"));
    }
    match fs::rename(from, to) {
        Ok(()) => {
            progress.done.fetch_add(tree_size(to), Ordering::Relaxed);
            Ok(())
        }
        Err(e) if e.raw_os_error() == Some(18) => {
            copy_tree(from, to, progress)?;
            remove_tree(from)
        }
        Err(e) => Err(e),
    }
}

/// What a name clash in a paste becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clash {
    Replace,
    KeepBoth,
    Skip,
}

/// A paste's plan: each source and where it goes, by the clash answer.
/// Pasting a copy into its own folder makes "name copy".
pub fn plan_paste(sources: &[PathBuf], dir: &Path, cut: bool, clash: Clash) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    let mut taken: Vec<PathBuf> = Vec::new();
    for s in sources {
        let Some(name) = s.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        let same_folder = s.parent() == Some(dir);
        if same_folder && cut {
            continue;
        }
        let target = dir.join(&name);
        let to = if same_folder {
            dir.join(copy_name(&name, dir))
        } else if target.symlink_metadata().is_ok() || taken.contains(&target) {
            match clash {
                Clash::Skip => continue,
                Clash::Replace => target,
                Clash::KeepBoth => dir.join(free_name(&name, |n| dir.join(n).symlink_metadata().is_ok() || taken.contains(&dir.join(n)))),
            }
        } else {
            target
        };
        taken.push(to.clone());
        out.push((s.clone(), to));
    }
    out
}

/// The names in `dir` a paste of `sources` would clash with (a copy into
/// its own folder never does: it is named "copy").
pub fn clashes(sources: &[PathBuf], dir: &Path) -> Vec<String> {
    sources
        .iter()
        .filter(|s| s.parent() != Some(dir))
        .filter_map(|s| s.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|n| dir.join(n).symlink_metadata().is_ok())
        .collect()
}

/// A job's outcome, for undo: what was made or moved.
#[derive(Debug, Clone, PartialEq)]
pub enum Done {
    /// Copies made (undo puts them in the Trash).
    Copied(Vec<PathBuf>),
    /// Items moved: (from, to) (undo moves them back).
    Moved(Vec<(PathBuf, PathBuf)>),
    /// Items put in the Trash: (where it was, its name in the Trash).
    Trashed(Vec<(PathBuf, String)>),
    Renamed(PathBuf, PathBuf),
    NewFolder(PathBuf),
    /// Something new made from the menu (a file, a link, an archive): undo
    /// puts it in the Trash.
    Made(PathBuf),
}

/// Runs a paste's plan: each copied or moved, a replaced target first put
/// in the Trash (so Replace can be undone too). Stops at the first error.
pub fn run_paste(plan: &[(PathBuf, PathBuf)], cut: bool, progress: &Arc<Progress>) -> (Done, Option<String>) {
    let total: u64 = plan.iter().map(|(s, _)| tree_size(s)).sum();
    progress.total.store(total.max(1), Ordering::Relaxed);
    let (mut copied, mut moved) = (Vec::new(), Vec::new());
    let mut error = None;
    for (from, to) in plan {
        if to.symlink_metadata().is_ok()
            && let Err(e) = trash(to)
        {
            error = Some(format!("“{}” could not be replaced: {e}", to.display()));
            break;
        }
        let r = if cut { move_item(from, to, progress) } else { copy_tree(from, to, progress) };
        match r {
            Ok(()) if cut => moved.push((from.clone(), to.clone())),
            Ok(()) => copied.push(to.clone()),
            Err(e) => {
                // A copy stopped or failed halfway: what it made goes.
                if !cut && to.symlink_metadata().is_ok() {
                    let _ = remove_tree(to);
                }
                error = Some(if e.kind() == io::ErrorKind::Interrupted { "Stopped.".into() } else { format!("“{}”: {e}", from.file_name().unwrap_or_default().to_string_lossy()) });
                break;
            }
        }
    }
    (if cut { Done::Moved(moved) } else { Done::Copied(copied) }, error)
}

/// Undoes a job. Returns what is undone in words, or why not.
pub fn undo(d: &Done) -> Result<String, String> {
    let p = Progress::default();
    match d {
        Done::Copied(made) => {
            for m in made {
                trash(m).map_err(|e| e.to_string())?;
            }
            Ok(format!("The copies ({}) are in the Trash.", made.len()))
        }
        Done::Moved(moves) => {
            for (from, to) in moves.iter().rev() {
                move_item(to, from, &p).map_err(|e| e.to_string())?;
            }
            Ok(format!("Moved back ({}).", moves.len()))
        }
        Done::Trashed(items) => {
            for (_, name) in items {
                restore(name).map_err(|e| e.to_string())?;
            }
            Ok(format!("Back from the Trash ({}).", items.len()))
        }
        Done::Renamed(from, to) => {
            if from.symlink_metadata().is_ok() {
                return Err(format!("“{}” is there again.", from.display()));
            }
            fs::rename(to, from).map_err(|e| e.to_string())?;
            Ok("The name is back.".into())
        }
        Done::Made(p) => {
            trash(p).map_err(|e| e.to_string())?;
            Ok(format!("“{}” is in the Trash.", p.file_name().unwrap_or_default().to_string_lossy()))
        }
        Done::NewFolder(p) => {
            fs::remove_dir(p).map_err(|_| "The new folder is not empty any more.".to_string())?;
            Ok("The new folder is gone.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vergence-ops-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn names_do_not_clash() {
        let taken = ["a.txt", "a 2.txt", "b", ".rc"];
        let t = |n: &str| taken.contains(&n);
        assert_eq!(free_name("c.txt", t), "c.txt");
        assert_eq!(free_name("a.txt", t), "a 3.txt");
        assert_eq!(free_name("b", t), "b 2");
        assert_eq!(free_name(".rc", t), ".rc 2", "a dot file's name is its stem");
        assert_eq!(unescape(&escape(Path::new("/home/u/a b%#é.txt"))), PathBuf::from("/home/u/a b%#é.txt"));
        assert_eq!(escape(Path::new("/x/a b")), "/x/a%20b");
    }

    #[test]
    fn a_tree_copies_moves_and_stops() {
        let d = tmp("copy");
        let src = d.join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("one.txt"), "hello").unwrap();
        fs::write(src.join("sub/two.txt"), "world!").unwrap();
        std::os::unix::fs::symlink("one.txt", src.join("link")).unwrap();
        let p = Progress::default();
        copy_tree(&src, &d.join("copy"), &p).unwrap();
        assert_eq!(fs::read_to_string(d.join("copy/sub/two.txt")).unwrap(), "world!");
        assert_eq!(fs::read_link(d.join("copy/link")).unwrap(), PathBuf::from("one.txt"), "a link stays a link");
        assert_eq!(p.done.load(Ordering::Relaxed), 11);
        assert!(copy_tree(&src, &src.join("sub/inside"), &p).is_err(), "not into itself");
        move_item(&d.join("copy"), &d.join("moved"), &p).unwrap();
        assert!(d.join("moved/one.txt").exists() && !d.join("copy").exists());
        let stop = Progress::default();
        stop.stop.store(true, Ordering::Relaxed);
        assert!(copy_tree(&src, &d.join("never"), &stop).is_err());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_paste_plans_clashes_and_copies_in_place() {
        let d = tmp("paste");
        let (a, b) = (d.join("a"), d.join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "1").unwrap();
        fs::write(b.join("x.txt"), "2").unwrap();
        let src = vec![a.join("x.txt")];
        assert_eq!(clashes(&src, &b), vec!["x.txt"]);
        assert_eq!(plan_paste(&src, &b, false, Clash::KeepBoth), vec![(a.join("x.txt"), b.join("x 2.txt"))]);
        assert!(plan_paste(&src, &b, false, Clash::Skip).is_empty());
        assert_eq!(plan_paste(&src, &a, false, Clash::Skip), vec![(a.join("x.txt"), a.join("x copy.txt"))], "a copy in its own folder");
        assert!(plan_paste(&src, &a, true, Clash::Skip).is_empty(), "a cut pasted where it is: nothing");
        let progress = Arc::new(Progress::default());
        let (done, err) = run_paste(&plan_paste(&src, &b, false, Clash::KeepBoth), false, &progress);
        assert_eq!(err, None);
        assert_eq!(done, Done::Copied(vec![b.join("x 2.txt")]));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_trash_takes_gives_back_and_undoes() {
        let d = tmp("trash");
        let data = d.join("data");
        // SAFETY: the tests that touch XDG_DATA_HOME run in this one test only.
        unsafe { std::env::set_var("XDG_DATA_HOME", &data) };
        let f = d.join("doc.txt");
        fs::write(&f, "keep me").unwrap();
        let name = trash(&f).unwrap();
        assert!(!f.exists());
        let t = trashed();
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].name.as_str(), t[0].from.as_path()), (name.as_str(), f.as_path()));
        assert!(fs::read_to_string(data.join("Trash/info").join(format!("{name}.trashinfo"))).unwrap().starts_with("[Trash Info]\nPath=/"));
        // A second file of the same name gets its own name in the Trash.
        fs::write(&f, "second").unwrap();
        let name2 = trash(&f).unwrap();
        assert_ne!(name, name2);
        assert_eq!(restore(&name).unwrap(), f);
        assert_eq!(fs::read_to_string(&f).unwrap(), "keep me");
        // Undo of a trashing restores; of a rename renames back.
        let undone = undo(&Done::Trashed(vec![(f.clone(), name2.clone())]));
        assert!(undone.is_ok());
        assert!(d.join("doc 2.txt").exists(), "back beside the first, under a free name");
        fs::rename(&f, d.join("renamed.txt")).unwrap();
        undo(&Done::Renamed(f.clone(), d.join("renamed.txt"))).unwrap();
        assert!(f.exists());
        empty_trash().unwrap();
        assert!(trashed().is_empty());
        unsafe { std::env::remove_var("XDG_DATA_HOME") };
        fs::remove_dir_all(&d).unwrap();
    }
}
