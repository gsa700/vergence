//! Extracting an archive (archive milestone 1): the helper reads it,
//! confined; this side makes the files, and trusts nothing it is told but
//! the stream's framing.
//!
//! The rules (docs/archive-milestone-1.md, "How it stays safe"):
//! - Everything is made in a new hidden folder beside where it goes, and
//!   moved into place at the end; a stop or an error removes it.
//! - A name with `..`, an absolute name or a drive letter is skipped;
//!   backslashes count as separators.
//! - Links are made last, after every file and folder, so nothing is
//!   ever written through one. A symbolic link is made only when its target
//!   stays inside the folder that will be placed and passes through none of
//!   the archive's other links (it is resolved once more after placing);
//!   a hard link only to a file made here.
//! - Names are kept as bytes: two names that are not UTF-8 never become one.
//! - No more entries, and no more bytes, than the listing said, counted on
//!   this side too (a compromised helper is not trusted to stop).
//! - Setuid, setgid and sticky bits are dropped, and nothing is writable by
//!   others; owners are never set; devices and pipes are not made.
//! - The listing's sizes are the most it may write (plus a little), so an
//!   archive that lies about its sizes is stopped.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crate::stream::{self, Header, Kind, Message};

/// Past these, the person is asked first (Dave, 2026-11-23: the plan's
/// lean): the total unpacked, the number of entries, and the ratio of the
/// unpacked size to the archive's once past [`RATIO_FROM`].
pub const ASK_BYTES: u64 = 20 << 30;
pub const ASK_ENTRIES: u64 = 100_000;
pub const ASK_RATIO: u64 = 1_000;
pub const RATIO_FROM: u64 = 1 << 30;
/// What an extract may write past the listing's total (its sizes are
/// what it said; a little room for rounding is all).
const SLACK: u64 = 1 << 20;
/// The longest name part, and the deepest path, made.
const MAX_PART: usize = 255;
const MAX_DEPTH: usize = 128;

#[derive(Debug)]
pub enum Error {
    /// The helper program is not installed.
    NoHelper,
    NeedsPassword,
    /// The archive could not be read: why, in words.
    Unreadable(String),
    Stopped,
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoHelper => write!(f, "the archive helper (vergence-unpack) is not installed"),
            Error::NeedsPassword => write!(f, "this archive needs a password"),
            Error::Unreadable(why) => write!(f, "it could not be read: {why}"),
            Error::Stopped => write!(f, "stopped"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

/// What an archive holds, from its headers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Listing {
    pub entries: u64,
    /// The files' sizes added up.
    pub total: u64,
    /// Some file's size is not said (then the total is not the most).
    pub unknown: bool,
    /// The first name parts, and whether each is a folder (something is
    /// under it, or it is one).
    pub tops: Vec<(String, bool)>,
}

impl Listing {
    /// The one folder everything is in, if that is how it is made.
    pub fn one_folder(&self) -> Option<&str> {
        match self.tops.as_slice() {
            [(name, true)] => Some(name),
            _ => None,
        }
    }

    /// Why to ask before extracting (an archive `archive_size` bytes long),
    /// in words for the question, or None.
    pub fn ask(&self, archive_size: u64) -> Option<String> {
        if self.total > ASK_BYTES {
            return Some(format!("This archive would unpack to {}.", size_words(self.total)));
        }
        if self.entries > ASK_ENTRIES {
            return Some(format!("This archive holds {} items.", self.entries));
        }
        if self.total > RATIO_FROM && self.total / archive_size.max(1) > ASK_RATIO {
            return Some(format!(
                "This archive of {} would unpack to {}.",
                size_words(archive_size),
                size_words(self.total)
            ));
        }
        None
    }
}

fn size_words(n: u64) -> String {
    let gb = n as f64 / (1u64 << 30) as f64;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", n as f64 / (1u64 << 20) as f64) }
}

/// What an extract made.
#[derive(Debug, Clone, PartialEq)]
pub struct Extracted {
    /// The folder now beside the archive (or where it was asked to go).
    pub made: PathBuf,
    /// Entries not made, by the rules.
    pub skipped: u64,
}

/// Where the helper is: beside this program, in its libexec, or /usr/local's.
pub fn helper() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [
        dir.join("vergence-unpack"),
        dir.parent()?.join("libexec/vergence/vergence-unpack"),
        PathBuf::from("/usr/local/libexec/vergence/vergence-unpack"),
        PathBuf::from("/usr/libexec/vergence/vergence-unpack"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// A running helper, killed when `stop` is set (its pipe then ends, so a
/// read in progress returns). The child is shared with the watchdog under a
/// lock, and reaped under it, so a kill never reaches a process that has
/// already been reaped (whose pid may be someone else's by then: the
/// review, 2026-11-23). No timeout of its own: the helper can block on
/// nothing but its pipe, and its processor time is limited.
struct Running {
    child: Arc<std::sync::Mutex<Option<std::process::Child>>>,
    out: io::BufReader<std::process::ChildStdout>,
    alive: Arc<AtomicBool>,
}

impl Running {
    fn start(helper: &Path, archive: &Path, args: &[String], stop: Arc<AtomicBool>) -> Result<Running, Error> {
        let file = vergence_types::open_regular(archive)?;
        let mut child = Command::new(helper)
            .args(args)
            .stdin(Stdio::from(file))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let out = io::BufReader::with_capacity(1 << 20, child.stdout.take().ok_or(Error::NoHelper)?);
        let child = Arc::new(std::sync::Mutex::new(Some(child)));
        let alive = Arc::new(AtomicBool::new(true));
        let (c, a) = (child.clone(), alive.clone());
        std::thread::spawn(move || {
            while a.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
                if stop.load(Ordering::Relaxed) {
                    if let Some(ch) = c.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                        let _ = ch.kill();
                    }
                    return;
                }
            }
        });
        Ok(Running { child, out, alive })
    }

    fn next(&mut self) -> io::Result<Message> {
        stream::read_message(&mut self.out)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        if let Some(mut ch) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = ch.kill();
            let _ = ch.wait();
        }
    }
}

/// An archive's listing, from its headers (the helper reads it, confined).
pub fn list(archive: &Path, stop: &Arc<AtomicBool>) -> Result<Listing, Error> {
    list_with(&helper().ok_or(Error::NoHelper)?, archive, stop)
}

pub fn list_with(helper: &Path, archive: &Path, stop: &Arc<AtomicBool>) -> Result<Listing, Error> {
    let mut run = Running::start(helper, archive, &["--list".into()], stop.clone())?;
    let mut l = Listing::default();
    loop {
        let m = run.next();
        if stop.load(Ordering::Relaxed) {
            return Err(Error::Stopped);
        }
        match m? {
            Message::Entry(h) => {
                l.entries += 1;
                match h.size {
                    Some(n) if h.kind == Kind::File => l.total = l.total.saturating_add(n),
                    None => l.unknown = true,
                    _ => {}
                }
                if let Some(parts) = clean(&h.name) {
                    let folder = parts.len() > 1 || h.kind == Kind::Dir;
                    let top = String::from_utf8_lossy(&parts[0]).into_owned();
                    if let Some(t) = l.tops.iter_mut().find(|(n, _)| *n == top) {
                        t.1 |= folder;
                    } else if l.tops.len() < 2 {
                        // Two are enough to know it is not one folder.
                        l.tops.push((top, folder));
                    }
                }
            }
            Message::Data(_) => {}
            Message::NeedsPassword => return Err(Error::NeedsPassword),
            Message::Failed(why) => return Err(Error::Unreadable(why)),
            Message::End => return Ok(l),
        }
    }
}

/// A name's parts, as bytes (a name need not be UTF-8: an old Windows zip's
/// are not, and two such names must not become one).
pub type Parts = Vec<Vec<u8>>;

/// A name's parts, cleaned, or None when it may not be made: an absolute
/// name, a drive letter, `..`, a NUL, a part too long, too deep, or nothing.
/// Backslashes count as separators.
pub fn clean(name: &[u8]) -> Option<Parts> {
    let name: Vec<u8> = name.iter().map(|&b| if b == b'\\' { b'/' } else { b }).collect();
    if name.first() == Some(&b'/') || name.contains(&0) {
        return None;
    }
    let parts: Parts = name.split(|&b| b == b'/').filter(|p| !p.is_empty() && *p != b".").map(<[u8]>::to_vec).collect();
    let drive = parts.first().is_some_and(|p| p.len() == 2 && p[1] == b':' && p[0].is_ascii_alphabetic());
    if parts.is_empty() || drive || parts.len() > MAX_DEPTH || parts.iter().any(|p| p == b".." || p.len() > MAX_PART) {
        return None;
    }
    Some(parts)
}

fn path_of(root: &Path, parts: &[Vec<u8>]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    parts.iter().fold(root.to_path_buf(), |a, p| a.join(std::ffi::OsStr::from_bytes(p)))
}

/// A symbolic link's target, when it stays inside the folder that will be
/// placed: the link at `at` (its parts from that folder), the target as the
/// archive has it, and every symbolic link this archive makes (`links`,
/// from the same folder). Relative only; `..` may climb no higher than the
/// placed folder; and the target may not pass *through* another of the
/// archive's links (it may end at one, which is checked by itself), since
/// a link in the middle makes the text's depth not the real one (the
/// review: `a/up -> ..` then `p -> a/up/a/up/../../etc`).
fn link_inside(at: &[Vec<u8>], target: &[u8], links: &HashSet<Parts>) -> Option<Vec<u8>> {
    if target.is_empty() || target[0] == b'/' || target.contains(&b'\\') || target.contains(&0) {
        return None;
    }
    let mut stack: Parts = at[..at.len().checked_sub(1)?].to_vec();
    let comps: Vec<&[u8]> = target.split(|&b| b == b'/').filter(|c| !c.is_empty() && *c != b".").collect();
    for (i, c) in comps.iter().enumerate() {
        if *c == b".." {
            stack.pop()?;
        } else {
            stack.push(c.to_vec());
            if i + 1 < comps.len() && links.contains(&stack) {
                return None;
            }
        }
        if stack.len() > MAX_DEPTH {
            return None;
        }
    }
    Some(target.to_vec())
}

/// Whether every folder from `root` down to `parts`' parent is a real
/// folder (none a link), making the missing ones.
fn real_parents(root: &Path, parts: &[Vec<u8>]) -> bool {
    let mut at = root.to_path_buf();
    use std::os::unix::ffi::OsStrExt;
    for p in &parts[..parts.len().saturating_sub(1)] {
        at.push(std::ffi::OsStr::from_bytes(p));
        match at.symlink_metadata() {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return false,
            Err(_) => {
                if fs::create_dir(&at).is_err() {
                    return false;
                }
            }
        }
    }
    true
}

/// A folder's name in `dir` not taken yet: itself, then "name 2", "name 3".
pub fn folder_name(name: &str, dir: &Path) -> String {
    let taken = |n: &str| dir.join(n).symlink_metadata().is_ok();
    if !taken(name) {
        return name.to_string();
    }
    (2..10_000).map(|n| format!("{name} {n}")).find(|n| !taken(n)).unwrap_or_else(|| format!("{name} copy"))
}

/// The folder name an archive's contents get: its name without the
/// archive's extension ("photos.tar.gz" is "photos").
pub fn stem(archive: &Path) -> String {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = name.to_lowercase();
    for ext in [
        ".tar.gz", ".tar.xz", ".tar.zst", ".tar.bz2", ".tar.lz4", ".tgz", ".txz", ".tzst", ".tbz2", ".tar", ".zip",
        ".7z", ".rar", ".jar", ".apk", ".deb", ".rpm", ".cpio",
    ] {
        if lower.ends_with(ext) && lower.len() > ext.len() {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ if name.is_empty() => "Archive".into(),
        _ => format!("{name} contents"),
    }
}

/// How far an extract is: bytes written, of the listing's total; set
/// `stop` to stop it.
#[derive(Debug, Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub stop: Arc<AtomicBool>,
}

/// Extracts `archive` into `into`, as `listing` (from [`list`]) said it
/// is: one folder at the top is made as it is, anything else goes in a
/// folder named after the archive. It writes no more than the listing's
/// total (or, when some size was not said, no more than the cap), and no
/// more entries than it listed.
pub fn extract(archive: &Path, into: &Path, listing: &Listing, progress: &Progress) -> Result<Extracted, Error> {
    extract_with(&helper().ok_or(Error::NoHelper)?, archive, into, listing, progress)
}

pub fn extract_with(
    helper: &Path,
    archive: &Path,
    into: &Path,
    listing: &Listing,
    progress: &Progress,
) -> Result<Extracted, Error> {
    // The most it may write: what it said, or past the cap only if every
    // size was said (the person was asked about the total).
    let most = if listing.unknown { listing.total.max(ASK_BYTES) } else { listing.total }.saturating_add(SLACK);
    let hidden = hidden_folder(into)?;
    let made = (|| {
        let w = write_all(helper, archive, &hidden, listing.entries, most, progress)?;
        let made = place(&hidden, into, archive, w.top.as_deref())?;
        Ok(Extracted { made: made.clone(), skipped: w.skipped + recheck_links(&made, &w.links) })
    })();
    if made.is_err() {
        let _ = fs::remove_dir_all(&hidden);
    }
    made
}

/// A new hidden folder in `into`, made here (not one already there).
fn hidden_folder(into: &Path) -> Result<PathBuf, Error> {
    for n in 0..1000 {
        let p = into.join(format!(".vergence-unpack-{}-{n}", std::process::id()));
        match fs::create_dir(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(Error::Io(io::Error::other("no hidden folder could be made")))
}

/// The hidden folder's contents moved into place: its one folder `top` as
/// it is (decided before any link was made, and links were checked against
/// it), or the whole of it under the archive's name. The name is taken
/// first (an empty folder made here), then the move replaces that empty
/// folder, so nothing someone else made meanwhile is ever replaced.
fn place(hidden: &Path, into: &Path, archive: &Path, top: Option<&[u8]>) -> Result<PathBuf, Error> {
    let (from, name) = match top {
        Some(t) => (path_of(hidden, &[t.to_vec()]), String::from_utf8_lossy(t).into_owned()),
        None => (hidden.to_path_buf(), stem(archive)),
    };
    for _ in 0..100 {
        let to = into.join(folder_name(&name, into));
        match fs::create_dir(&to) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
        if let Err(e) = fs::rename(&from, &to) {
            let _ = fs::remove_dir(&to);
            return Err(e.into());
        }
        if from != hidden {
            let _ = fs::remove_dir(hidden);
        }
        return Ok(to);
    }
    Err(Error::Io(io::Error::other("no free name")))
}

/// After placing, each symbolic link made is resolved once more: one that
/// leads outside the placed folder is removed (a second line behind
/// `link_inside`). A link to nothing stays, as the archive had it. How many
/// were removed.
fn recheck_links(made: &Path, links: &[Parts]) -> u64 {
    let Ok(root) = made.canonicalize() else { return 0 };
    let mut removed = 0;
    for parts in links {
        let p = path_of(made, parts);
        if let Ok(real) = p.canonicalize()
            && !real.starts_with(&root)
        {
            let _ = fs::remove_file(&p);
            removed += 1;
        }
    }
    removed
}

/// What [`write_all`] made: how many were skipped, the one folder at the
/// top (what will be placed, if that is how it is made), and the symbolic
/// links made (their parts from what is placed).
struct Written {
    skipped: u64,
    top: Option<Vec<u8>>,
    links: Vec<Parts>,
}

/// The helper's entries made in `root`. How many were skipped, the one
/// folder at the top (if that is how it is made: it is what will be
/// placed), and the symbolic links made (their parts from what is placed).
fn write_all(
    helper: &Path,
    archive: &Path,
    root: &Path,
    entries: u64,
    most: u64,
    p: &Progress,
) -> Result<Written, Error> {
    let most_entries = entries.saturating_add(16);
    let args = vec![format!("--max-bytes={most}"), format!("--max-entries={most_entries}")];
    let mut run = Running::start(helper, archive, &args, p.stop.clone())?;
    let (mut skipped, mut written, mut seen) = (0u64, 0u64, 0u64);
    let mut files: HashSet<Parts> = HashSet::new();
    let mut links: Vec<(Parts, Header)> = Vec::new();
    // The file being written, if the last entry is one being made.
    let mut open: Option<(fs::File, PathBuf, Header)> = None;
    let finish = |open: &mut Option<(fs::File, PathBuf, Header)>| -> io::Result<()> {
        if let Some((f, path, h)) = open.take() {
            // A time the file system refuses is no reason to fail the extract.
            let _ = f.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(h.mtime.max(0) as u64));
            drop(f);
            use std::os::unix::fs::PermissionsExt;
            // Never writable by others: 0o755 at most, the owner's read and write always.
            fs::set_permissions(&path, fs::Permissions::from_mode((h.perm & 0o755) | 0o600))?;
        }
        Ok(())
    };
    loop {
        let m = run.next();
        if p.stop.load(Ordering::Relaxed) {
            return Err(Error::Stopped);
        }
        match m? {
            Message::Data(d) => {
                // Every byte counts, written or not: a helper that lies cannot
                // make this side read without end.
                written += d.len() as u64;
                if written > most {
                    return Err(Error::Unreadable("larger than it says".into()));
                }
                if let Some((f, _, _)) = &mut open {
                    f.write_all(&d)?;
                    p.done.fetch_add(d.len() as u64, Ordering::Relaxed);
                }
            }
            Message::Entry(h) => {
                finish(&mut open)?;
                seen += 1;
                if seen > most_entries {
                    return Err(Error::Unreadable("more entries than it says".into()));
                }
                let Some(parts) = clean(&h.name) else {
                    skipped += 1;
                    continue;
                };
                match h.kind {
                    Kind::Dir => {
                        let mut inside = parts.clone();
                        inside.push(Vec::new());
                        if !real_parents(root, &inside) {
                            skipped += 1;
                        }
                    }
                    Kind::File => {
                        if !real_parents(root, &parts) {
                            skipped += 1;
                            continue;
                        }
                        let path = path_of(root, &parts);
                        // The same name again (a tar may hold a file twice):
                        // the later one wins, if the first was a file made here.
                        if files.contains(&parts) {
                            fs::remove_file(&path)?;
                        }
                        match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                            Ok(f) => {
                                files.insert(parts);
                                open = Some((f, path, h));
                            }
                            Err(_) => skipped += 1,
                        }
                    }
                    Kind::Symlink | Kind::Hardlink => links.push((parts, h)),
                    Kind::Other => skipped += 1,
                }
            }
            Message::NeedsPassword => return Err(Error::NeedsPassword),
            Message::Failed(why) => return Err(Error::Unreadable(why)),
            Message::End => break,
        }
    }
    finish(&mut open)?;
    // What will be placed, decided before any link is made: one folder at
    // the top (everything made, links included, under one name that is a
    // real folder), or the whole hidden folder.
    let mut tops: HashSet<Vec<u8>> = links.iter().map(|(parts, _)| parts[0].clone()).collect();
    use std::os::unix::ffi::OsStrExt;
    for e in fs::read_dir(root)?.filter_map(Result::ok) {
        tops.insert(e.file_name().as_bytes().to_vec());
    }
    let top = match tops.into_iter().collect::<Vec<_>>().as_slice() {
        [one] if path_of(root, std::slice::from_ref(one)).symlink_metadata().is_ok_and(|m| m.is_dir()) => {
            Some(one.clone())
        }
        _ => None,
    };
    let base = usize::from(top.is_some());
    let symlinks: HashSet<Parts> =
        links.iter().filter(|(_, h)| h.kind == Kind::Symlink).map(|(parts, _)| parts[base..].to_vec()).collect();
    // Hard links first (to files made here), then symbolic links: nothing
    // is written after a link is made.
    links.sort_by_key(|(_, h)| h.kind != Kind::Hardlink);
    let mut made = Vec::new();
    for (parts, h) in links {
        let path = path_of(root, &parts);
        let ok = match h.kind {
            Kind::Hardlink => clean(&h.link).filter(|t| files.contains(t)).is_some_and(|t| {
                real_parents(root, &parts) && real_parents(root, &t) && fs::hard_link(path_of(root, &t), &path).is_ok()
            }),
            _ => {
                let from_top = &parts[base..];
                !from_top.is_empty()
                    && link_inside(from_top, &h.link, &symlinks).is_some_and(|t| {
                        real_parents(root, &parts)
                            && std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(&t), &path).is_ok()
                    })
                    && {
                        made.push(from_top.to_vec());
                        true
                    }
            }
        };
        if !ok {
            skipped += 1;
        }
    }
    Ok(Written { skipped, top, links: made })
}

/// The helper's own check that it is confined: 4 is confined.
pub fn check_confined(helper: &Path) -> io::Result<Option<i32>> {
    let out = Command::new(helper).arg("--check-confined").stdin(Stdio::null()).output()?;
    use std::os::unix::process::ExitStatusExt;
    Ok(out.status.code().or_else(|| out.status.signal().map(|s| 128 + s)))
}
