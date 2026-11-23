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
//!   ever written through one; a symbolic link is made only when it points
//!   inside, a hard link only to a file made here.
//! - Setuid, setgid and sticky bits are dropped; owners are never set;
//!   devices and pipes are not made.
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
/// A helper that says nothing for this long is stopped.
const STALL: Duration = Duration::from_secs(120);
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

/// A running helper, stopped when it stalls or `stop` is set (its pipe
/// then ends, so a read in progress returns).
struct Running {
    child: std::process::Child,
    out: io::BufReader<std::process::ChildStdout>,
    alive: Arc<AtomicBool>,
    heard: Arc<AtomicU64>,
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
        let (alive, heard) = (Arc::new(AtomicBool::new(true)), Arc::new(AtomicU64::new(0)));
        let pid = child.id();
        let (a, h) = (alive.clone(), heard.clone());
        std::thread::spawn(move || {
            let (mut last, mut quiet) = (u64::MAX, Duration::ZERO);
            let tick = Duration::from_millis(100);
            while a.load(Ordering::Relaxed) {
                std::thread::sleep(tick);
                let now = h.load(Ordering::Relaxed);
                quiet = if now == last { quiet + tick } else { Duration::ZERO };
                last = now;
                if stop.load(Ordering::Relaxed) || quiet >= STALL {
                    kill(pid);
                    return;
                }
            }
        });
        Ok(Running { child, out, alive, heard })
    }

    fn next(&mut self) -> io::Result<Message> {
        let m = stream::read_message(&mut self.out);
        self.heard.fetch_add(1, Ordering::Relaxed);
        m
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn kill(pid: u32) {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    // SAFETY: kill(2) with our own child's pid and SIGKILL (9); the child is
    // reaped by its Running, so the pid is not reused meanwhile.
    unsafe { kill(pid as i32, 9) };
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
                    if let Some(t) = l.tops.iter_mut().find(|(n, _)| *n == parts[0]) {
                        t.1 |= folder;
                    } else if l.tops.len() < 2 {
                        // Two are enough to know it is not one folder.
                        l.tops.push((parts[0].clone(), folder));
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

/// A name's parts, cleaned, or None when it may not be made: an absolute
/// name, a drive letter, `..`, a part too long, too deep, or nothing.
pub fn clean(name: &[u8]) -> Option<Vec<String>> {
    let text = String::from_utf8_lossy(name).replace('\\', "/");
    if text.starts_with('/') {
        return None;
    }
    let parts: Vec<String> = text.split('/').filter(|p| !p.is_empty() && *p != ".").map(str::to_string).collect();
    let drive =
        parts.first().is_some_and(|p| p.len() == 2 && p.ends_with(':') && p.as_bytes()[0].is_ascii_alphabetic());
    if parts.is_empty()
        || drive
        || parts.len() > MAX_DEPTH
        || parts.iter().any(|p| p == ".." || p.len() > MAX_PART || p.contains('\0'))
    {
        return None;
    }
    Some(parts)
}

/// A symbolic link's target, when it stays inside: the link at `at` (its
/// parts), the target as the archive has it. Relative only; `..` may climb
/// no higher than the top.
fn link_inside(at: &[String], target: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(target).into_owned();
    if text.is_empty() || text.starts_with('/') || text.contains('\\') || text.contains('\0') {
        return None;
    }
    let mut depth = at.len().checked_sub(1)?;
    for p in text.split('/') {
        match p {
            "" | "." => {}
            ".." => depth = depth.checked_sub(1)?,
            _ => depth += 1,
        }
    }
    Some(text)
}

/// Whether every folder from `root` down to `parts`' parent is a real
/// folder (none a link), making the missing ones.
fn real_parents(root: &Path, parts: &[String]) -> bool {
    let mut at = root.to_path_buf();
    for p in &parts[..parts.len() - 1] {
        at.push(p);
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
/// total (or, when some size was not said, no more than the cap).
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
    let most = if listing.unknown { listing.total.max(ASK_BYTES) } else { listing.total } + SLACK;
    let hidden = hidden_folder(into)?;
    let made = (|| {
        let skipped = write_all(helper, archive, &hidden, listing.entries, most, progress)?;
        Ok(Extracted { made: place(&hidden, into, archive)?, skipped })
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

/// The hidden folder's contents moved into place: its one folder as it is,
/// or the whole of it under the archive's name. The name is taken first
/// (an empty folder made here), then the move replaces that empty folder,
/// so nothing someone else made meanwhile is ever replaced.
fn place(hidden: &Path, into: &Path, archive: &Path) -> Result<PathBuf, Error> {
    let inside: Vec<_> = fs::read_dir(hidden)?.filter_map(Result::ok).collect();
    let (from, name) = match inside.as_slice() {
        [one] if one.path().symlink_metadata().is_ok_and(|m| m.is_dir()) => {
            (one.path(), one.file_name().to_string_lossy().into_owned())
        }
        _ => (hidden.to_path_buf(), stem(archive)),
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

/// The helper's entries made in `root`. How many were skipped.
fn write_all(helper: &Path, archive: &Path, root: &Path, entries: u64, most: u64, p: &Progress) -> Result<u64, Error> {
    let args = vec![format!("--max-bytes={most}"), format!("--max-entries={}", entries.saturating_add(16))];
    let mut run = Running::start(helper, archive, &args, p.stop.clone())?;
    let (mut skipped, mut written) = (0u64, 0u64);
    let mut files: HashSet<Vec<String>> = HashSet::new();
    let mut links: Vec<(Vec<String>, Header)> = Vec::new();
    // The file being written, if the last entry is one being made.
    let mut open: Option<(fs::File, PathBuf, Header)> = None;
    let finish = |open: &mut Option<(fs::File, PathBuf, Header)>| -> io::Result<()> {
        if let Some((f, path, h)) = open.take() {
            f.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(h.mtime.max(0) as u64))?;
            drop(f);
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode((h.perm & 0o777) | 0o600))?;
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
                if let Some((f, _, _)) = &mut open {
                    written += d.len() as u64;
                    if written > most {
                        return Err(Error::Unreadable("larger than it says".into()));
                    }
                    f.write_all(&d)?;
                    p.done.fetch_add(d.len() as u64, Ordering::Relaxed);
                }
            }
            Message::Entry(h) => {
                finish(&mut open)?;
                let Some(parts) = clean(&h.name) else {
                    skipped += 1;
                    continue;
                };
                match h.kind {
                    Kind::Dir => {
                        if !real_parents(root, &[parts.clone(), vec![String::new()]].concat()) {
                            skipped += 1;
                        }
                    }
                    Kind::File => {
                        if !real_parents(root, &parts) {
                            skipped += 1;
                            continue;
                        }
                        let path = parts.iter().fold(root.to_path_buf(), |a, p| a.join(p));
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
    // Hard links first (to files made here), then symbolic links: nothing
    // is written after a link is made.
    links.sort_by_key(|(_, h)| h.kind != Kind::Hardlink);
    for (parts, h) in links {
        let path = parts.iter().fold(root.to_path_buf(), |a, p| a.join(p));
        let ok = match h.kind {
            Kind::Hardlink => clean(&h.link).filter(|t| files.contains(t)).is_some_and(|t| {
                let target = t.iter().fold(root.to_path_buf(), |a, p| a.join(p));
                real_parents(root, &parts) && real_parents(root, &t) && fs::hard_link(&target, &path).is_ok()
            }),
            _ => link_inside(&parts, &h.link)
                .is_some_and(|t| real_parents(root, &parts) && std::os::unix::fs::symlink(t, &path).is_ok()),
        };
        if !ok {
            skipped += 1;
        }
    }
    Ok(skipped)
}

/// The helper's own check that it is confined: 4 is confined.
pub fn check_confined(helper: &Path) -> io::Result<Option<i32>> {
    let out = Command::new(helper).arg("--check-confined").stdin(Stdio::null()).output()?;
    use std::os::unix::process::ExitStatusExt;
    Ok(out.status.code().or_else(|| out.status.signal().map(|s| 128 + s)))
}
