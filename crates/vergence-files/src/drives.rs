//! Drives and mounting in FileView's window: a disk's free space (`df -kP`),
//! whether the folder shown can be ejected, a disk image mounted. Eject itself
//! is a command (commands.rs); the drive list's changes are watched in
//! watch.rs. The window itself is in main.rs.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use vergence_widgets::files;

use crate::Viewer;

/// The free bytes of the file system `dir` is on, from `df -kP` (as
/// vergence-sysinfo reads file systems, on Linux and FreeBSD alike).
pub(crate) fn df_free(dir: &Path) -> Option<u64> {
    let out = std::process::Command::new("df").arg("-kP").arg(dir).stderr(std::process::Stdio::null()).output().ok()?;
    parse_df_free(&String::from_utf8_lossy(&out.stdout))
}

/// `df -kP`'s second line: Filesystem, 1024-blocks, Used, Available, ... ;
/// the Available column in bytes.
pub(crate) fn parse_df_free(text: &str) -> Option<u64> {
    let line = text.lines().nth(1)?;
    let kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(kb * 1024)
}

impl Viewer {
    /// Mounts a disk image on a worker, as a USB drive is (see docs/files-milestone-3.md);
    /// once mounted it shows under Locations, with no tab of its own (watch.rs).
    pub(crate) fn mount_image(&mut self, p: PathBuf) {
        if self.mounting.is_some() {
            return;
        }
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let answer: Arc<Mutex<Option<Result<PathBuf, String>>>> = Arc::default();
        let (a, waker) = (answer.clone(), self.thumb_waker.clone());
        std::thread::spawn(move || {
            let r = files::mount_image(&p);
            *a.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            if let Some(w) = waker.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                w.wake();
            }
        });
        self.notice = Some(format!("Mounting “{name}”…"));
        self.mounting = Some((name, answer));
    }

    /// The free space of the disk the front folder is on, once read (a
    /// worker runs `df -kP` for each folder the first time it is shown).
    pub(crate) fn free_here(&self) -> Option<u64> {
        let dir = self.here()?;
        if let Some(n) = self.free.lock().unwrap_or_else(|e| e.into_inner()).get(&dir) {
            return *n;
        }
        if self.free_asked.borrow_mut().insert(dir.clone()) {
            let (free, waker) = (self.free.clone(), self.thumb_waker.clone());
            std::thread::spawn(move || {
                let n = df_free(&dir);
                free.lock().unwrap_or_else(|e| e.into_inner()).insert(dir, n);
                if let Some(w) = waker.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                    w.wake();
                }
            });
        }
        None
    }

    /// Whether the folder shown can be ejected (asked of the mount table
    /// once per folder).
    pub(crate) fn here_ejectable(&self) -> bool {
        let Some(here) = self.here() else { return false };
        if let Some((p, e)) = self.ejectable.borrow().as_ref()
            && *p == here
        {
            return *e;
        }
        let e = files::ejectable(&here);
        *self.ejectable.borrow_mut() = Some((here, e));
        e
    }
}
