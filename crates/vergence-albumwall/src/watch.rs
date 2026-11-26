//! Watching a folder library for changes (the Avalonia app's
//! LibraryWatcher; docs/albumwall-milestone-1.md, step 2): a change to a
//! music file, a cover or the folders, and after 3 seconds of quiet the
//! library is read again (with the index trusted, so only what changed is
//! opened).
//!
//! On Linux, inotify: one watch per folder, added as folders appear. A
//! network share does not tell of changes made elsewhere (inotify sees only
//! this machine's), so a share is looked at again every 15 minutes instead,
//! as the Avalonia app does; so is any library on a system with no watcher
//! here (FreeBSD for now: kqueue needs a descriptor per folder), and also
//! when the window comes to the front (Dave, 2026-11-25: "the lean is fine").

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How long changes must stop before the library is read again.
pub const QUIET: Duration = Duration::from_secs(3);
/// How often a library no watcher covers is looked at again.
pub const POLL: Duration = Duration::from_secs(15 * 60);

/// A library's watcher: `changed` is set (and the waker called) after a
/// burst of changes has settled. Dropped, it stops.
pub struct Watcher {
    changed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Watcher {
    /// Whether the library changed since this was last asked.
    pub fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::Relaxed)
    }

    /// A watcher for `root`, telling `wake` when it changed; None where
    /// none can watch it (a network share, a system without inotify, too
    /// many folders): the caller looks again every [`POLL`].
    pub fn start(root: &Path, wake: impl Fn() + Send + 'static) -> Option<Watcher> {
        if is_network(root) {
            return None;
        }
        imp::start(root, wake)
    }
}

/// Whether `path` is on a network file system (NFS, SMB/CIFS, a FUSE mount
/// such as sshfs), where changes made on other machines are not told.
pub fn is_network(path: &Path) -> bool {
    imp::fs_type(path).is_some_and(|t| matches!(t, 0x6969 | 0xFF53_4D42 | 0xFE53_4D42 | 0x6573_5546 | 0x517B))
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::time::Instant;

    unsafe extern "C" {
        fn inotify_init1(flags: i32) -> i32;
        fn inotify_add_watch(fd: i32, path: *const std::ffi::c_char, mask: u32) -> i32;
        fn close(fd: i32) -> i32;
        fn poll(fds: *mut PollFd, n: u64, timeout: i32) -> i32;
        fn statfs(path: *const std::ffi::c_char, buf: *mut u8) -> i32;
    }

    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }

    const IN_CLOEXEC: i32 = 0o2000000;
    const IN_CLOSE_WRITE: u32 = 0x8;
    const IN_ATTRIB: u32 = 0x4;
    const IN_MOVED_FROM: u32 = 0x40;
    const IN_MOVED_TO: u32 = 0x80;
    const IN_CREATE: u32 = 0x100;
    const IN_DELETE: u32 = 0x200;
    const IN_DELETE_SELF: u32 = 0x400;
    const IN_MOVE_SELF: u32 = 0x800;
    const IN_Q_OVERFLOW: u32 = 0x4000;
    const IN_ISDIR: u32 = 0x4000_0000;
    const MASK: u32 = IN_CLOSE_WRITE
        | IN_ATTRIB
        | IN_MOVED_FROM
        | IN_MOVED_TO
        | IN_CREATE
        | IN_DELETE
        | IN_DELETE_SELF
        | IN_MOVE_SELF;
    /// More folders than this and the library is looked at every POLL instead.
    const MOST_FOLDERS: usize = 50_000;

    pub fn fs_type(path: &Path) -> Option<i64> {
        let c = CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut buf = [0u8; 256];
        // SAFETY: statfs(2) fills at most sizeof(struct statfs) (120 bytes on
        // x86_64 and aarch64) of our 256; f_type is its first field, a long.
        (unsafe { statfs(c.as_ptr(), buf.as_mut_ptr()) } == 0)
            .then(|| i64::from_ne_bytes(buf[..8].try_into().unwrap_or([0; 8])))
    }

    fn add(fd: i32, dir: &Path, watched: &mut std::collections::HashMap<i32, PathBuf>) -> bool {
        let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else { return true };
        // SAFETY: inotify_add_watch(2) on our descriptor with a valid C string.
        let wd = unsafe { inotify_add_watch(fd, c.as_ptr(), MASK) };
        if wd < 0 {
            return false;
        }
        watched.insert(wd, dir.to_path_buf());
        true
    }

    /// `dir` and every folder under it watched (hidden ones skipped, links
    /// not followed, as the scan does). False if the watches ran out.
    fn add_tree(fd: i32, dir: &Path, watched: &mut std::collections::HashMap<i32, PathBuf>) -> bool {
        let mut todo = vec![dir.to_path_buf()];
        while let Some(d) = todo.pop() {
            if watched.len() >= MOST_FOLDERS || !add(fd, &d, watched) {
                return false;
            }
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    todo.push(e.path());
                }
            }
        }
        true
    }

    pub fn start(root: &Path, wake: impl Fn() + Send + 'static) -> Option<Watcher> {
        // SAFETY: inotify_init1(2), no memory passed.
        let fd = unsafe { inotify_init1(IN_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        let mut watched = std::collections::HashMap::new();
        if !add_tree(fd, root, &mut watched) {
            // SAFETY: our own descriptor, closed once.
            unsafe { close(fd) };
            return None;
        }
        let changed = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (c, s) = (changed.clone(), stop.clone());
        let spawned = std::thread::Builder::new().name("watch".into()).spawn(move || {
            use std::io::Read;
            use std::os::fd::FromRawFd;
            // SAFETY: the descriptor is ours and handed to this File alone,
            // which closes it when the thread ends.
            let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
            let mut buf = vec![0u8; 64 << 10];
            let mut pending: Option<Instant> = None;
            while !s.load(Ordering::Relaxed) {
                let mut p = PollFd { fd, events: 1, revents: 0 };
                // SAFETY: poll(2) on one pollfd of ours, a second at most.
                let ready = unsafe { poll(&mut p, 1, 1000) };
                if ready > 0 {
                    let n = match file.read(&mut buf) {
                        Ok(n) if n > 0 => n as isize,
                        _ => continue,
                    };
                    let mut at = 0usize;
                    while at + 16 <= n as usize {
                        let field = |i: usize| u32::from_ne_bytes(buf[at + i..at + i + 4].try_into().unwrap_or([0; 4]));
                        let (wd, mask, len) = (field(0) as i32, field(4), field(12) as usize);
                        let name_bytes = &buf[at + 16..(at + 16 + len).min(n as usize)];
                        let name_end = name_bytes.iter().position(|b| *b == 0).unwrap_or(name_bytes.len());
                        let name = std::ffi::OsStr::from_bytes(&name_bytes[..name_end]);
                        at += 16 + len;
                        let path = watched.get(&wd).map(|d| d.join(name));
                        let relevant = mask & IN_Q_OVERFLOW != 0
                            || mask & (IN_DELETE_SELF | IN_MOVE_SELF) != 0
                            || mask & IN_ISDIR != 0
                            || path.as_deref().is_some_and(crate::library::is_library_file);
                        if mask & IN_ISDIR != 0
                            && mask & (IN_CREATE | IN_MOVED_TO) != 0
                            && let Some(p) = &path
                            && !name.to_string_lossy().starts_with('.')
                        {
                            add_tree(fd, p, &mut watched);
                        }
                        if relevant {
                            pending = Some(Instant::now());
                        }
                    }
                }
                if pending.is_some_and(|t| t.elapsed() >= QUIET) {
                    pending = None;
                    c.store(true, Ordering::Relaxed);
                    wake();
                }
            }
        });
        if spawned.is_err() {
            // SAFETY: the thread never started; the descriptor is still ours.
            unsafe { close(fd) };
            return None;
        }
        Some(Watcher { changed, stop })
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::*;

    pub fn fs_type(_path: &Path) -> Option<i64> {
        None
    }

    pub fn start(_root: &Path, _wake: impl Fn() + Send + 'static) -> Option<Watcher> {
        None
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_new_track_is_told_after_the_quiet() {
        let root = std::env::temp_dir().join(format!("albumwall-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Toto/1982 - IV")).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let w = Watcher::start(&root, move || {
            let _ = tx.send(());
        })
        .expect("a local folder is watched");
        // Not a library file: nothing.
        std::fs::write(root.join("Toto/notes.txt"), b"x").unwrap();
        assert!(rx.recv_timeout(QUIET + Duration::from_secs(2)).is_err());
        assert!(!w.take_changed());
        // A track in a folder made after the watch began.
        let started = Instant::now();
        std::fs::create_dir_all(root.join("Toto/1984 - Isolation")).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(root.join("Toto/1984 - Isolation/01.flac"), b"x").unwrap();
        rx.recv_timeout(QUIET + Duration::from_secs(5)).expect("told");
        assert!(started.elapsed() >= QUIET);
        assert!(w.take_changed());
        drop(w);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
