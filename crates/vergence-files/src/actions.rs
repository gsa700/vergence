//! The window's side of the file operations (ops.rs does the work): paste
//! and its job, Move to the Trash, a name typed (new folder, new file, rename
//! in place), links, archives, the Trash rows' own buttons. The window itself
//! is in main.rs.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use vergence_widgets::files;
use vergence_widgets::{Program, Window};

use crate::{Ask, DOUBLE, Job, Naming, Viewer, cmd, notify, ops};

impl Viewer {
    /// A double click's longest gap: the desktop's (Settings > Mouse), as
    /// the toolkit's widgets use, so a slow double click opens rather than renames.
    pub(crate) fn double_gap(&self) -> std::time::Duration {
        match self.theme().double_click {
            0 => DOUBLE,
            ms => std::time::Duration::from_millis(ms.clamp(100, 2000) as u64),
        }
    }

    /// The visible row whose name is being changed in place, if any.
    pub(crate) fn renaming_row(&self) -> Option<usize> {
        let target = self.naming.as_ref()?.rename.as_ref()?;
        self.visible().iter().position(|e| e.path == *target)
    }

    /// The name of `p` made a field in its row.
    pub(crate) fn start_rename(&mut self, p: PathBuf) {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.naming = Some(Naming { rename: Some(p), text: name, file: false });
    }

    /// A link to the one chosen item beside it ("name link").
    pub(crate) fn make_link(&mut self) {
        let chosen = self.chosen_paths();
        let [one] = chosen.as_slice() else { return };
        let (Some(dir), Some(name)) = (one.parent(), one.file_name()) else { return };
        let to = dir.join(ops::free_name(&format!("{} link", name.to_string_lossy()), |x| {
            dir.join(x).symlink_metadata().is_ok()
        }));
        match std::os::unix::fs::symlink(one, &to) {
            Ok(()) => {
                self.shared.borrow_mut().undo.push(ops::Done::Made(to.clone()));
                self.read_all();
                self.choose_only(to);
            }
            Err(e) => self.notice = Some(format!("The link could not be made: {}", files::plain(&e))),
        }
    }

    /// The chosen items into one archive beside them: "name.tar.gz" for one,
    /// "Archive.tar.gz" for several, by the system's tar (principle 8), on a
    /// worker; the folder's watch shows it when it is there.
    pub(crate) fn compress(&mut self) {
        let chosen = self.chosen_paths();
        let Some(dir) = chosen.first().and_then(|p| p.parent()).map(Path::to_path_buf) else { return };
        if chosen.iter().any(|p| p.parent() != Some(dir.as_path())) {
            self.notice = Some("Items from different folders cannot be compressed together.".into());
            return;
        }
        let base = match chosen.as_slice() {
            [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into()),
            _ => "Archive".into(),
        };
        let name = ops::free_name(&format!("{base}.tar.gz"), |x| dir.join(x).symlink_metadata().is_ok());
        let to = dir.join(&name);
        let names: Vec<std::ffi::OsString> =
            chosen.iter().filter_map(|p| p.file_name().map(|n| n.to_os_string())).collect();
        std::thread::spawn(move || {
            // "--" so a name starting with "-" is a name, not an option.
            let out = std::process::Command::new("tar")
                .arg("-czf")
                .arg(&to)
                .arg("-C")
                .arg(&dir)
                .arg("--")
                .args(&names)
                .output();
            match out {
                Ok(o) if o.status.success() => {}
                Ok(o) => {
                    let _ = std::fs::remove_file(&to);
                    notify(&format!("“{name}” could not be made"), String::from_utf8_lossy(&o.stderr).trim());
                }
                Err(e) => notify(&format!("“{name}” could not be made"), &format!("tar: {e}")),
            }
        });
    }

    /// A Trash row's own button: that row chosen, then put back, or asked
    /// about deleting it forever.
    pub(crate) fn row_tool(&mut self, k: usize, put_back: bool, window: &mut Window) {
        let Some(p) = self.visible().get(k).map(|e| e.path.clone()) else { return };
        if let Some(t) = self.front_mut() {
            t.chosen = vec![p.clone()];
            t.cursor = Some(p.clone());
        }
        if put_back {
            self.run(cmd::RESTORE, window);
        } else {
            self.notice = None;
            self.ask = Some(Ask::Forever(vec![p]));
        }
    }

    /// The name an item is shown by: in the Trash, the name it had.
    pub(crate) fn shown_name(&self, p: &Path) -> String {
        self.front_tab()
            .and_then(|t| t.entries.iter().find(|e| e.path == p))
            .map(|e| e.name.clone())
            .unwrap_or_else(|| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
    }

    pub(crate) fn paste(&mut self) {
        let Some(dir) = self.here() else { return };
        let Some((sources, cut)) = self.shared.borrow().clip.clone() else { return };
        let names = ops::clashes(&sources, &dir);
        if names.is_empty() {
            self.start_paste(sources, dir, cut, ops::Clash::KeepBoth);
        } else {
            self.ask = Some(Ask::Clash { sources, dir, cut, names });
        }
    }

    /// A paste (or Duplicate) on a worker, its progress in the strip.
    pub(crate) fn start_paste(&mut self, sources: Vec<PathBuf>, dir: PathBuf, cut: bool, clash: ops::Clash) {
        if self.job.is_some() {
            self.notice = Some("A copy is still going on.".into());
            return;
        }
        let plan = ops::plan_paste(&sources, &dir, cut, clash);
        if plan.is_empty() {
            return;
        }
        if cut {
            // A cut is used once.
            self.shared.borrow_mut().clip = None;
        }
        let words = format!(
            "{} {} item{}",
            if cut { "Moving" } else { "Copying" },
            plan.len(),
            if plan.len() == 1 { "" } else { "s" }
        );
        let progress = Arc::new(ops::Progress::default());
        let result: Arc<Mutex<Option<(ops::Done, Option<String>)>>> = Arc::default();
        let (p, r, waker) = (progress.clone(), result.clone(), self.waker.clone());
        std::thread::spawn(move || {
            // The window looks again while it runs, and when it ends.
            let ticking = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let (t, w) = (ticking.clone(), waker.clone());
            std::thread::spawn(move || {
                while t.load(std::sync::atomic::Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    if let Some(w) = &w {
                        w.wake();
                    }
                }
            });
            let out = ops::run_paste(&plan, cut, &p);
            *r.lock().unwrap_or_else(|e| e.into_inner()) = Some(out);
            ticking.store(false, std::sync::atomic::Ordering::Relaxed);
            if let Some(w) = waker {
                w.wake();
            }
        });
        self.job = Some(Job { words, progress, result });
    }

    /// A finished job: what it did goes on the undo stack; its error is said.
    pub(crate) fn job_ended(&mut self) -> bool {
        let Some(job) = &self.job else { return false };
        let Some((done, error)) = job.result.lock().unwrap_or_else(|e| e.into_inner()).take() else { return true };
        let made = !matches!(&done, ops::Done::Copied(v) if v.is_empty())
            && !matches!(&done, ops::Done::Moved(v) if v.is_empty());
        if made {
            self.shared.borrow_mut().undo.push(done);
        }
        if let Some(e) = error {
            self.notice = Some(e);
        }
        self.job = None;
        self.read_all();
        true
    }

    pub(crate) fn trash_chosen(&mut self) {
        let paths = self.chosen_paths();
        if paths.is_empty() {
            return;
        }
        // This operation's words replace the last one's.
        self.notice = None;
        if self.in_trash() {
            // In the Trash itself, Delete means for good.
            self.ask = Some(Ask::Forever(paths));
            return;
        }
        let mut done = Vec::new();
        for p in &paths {
            match ops::trash(p) {
                Ok(name) => done.push((p.clone(), name)),
                Err(e) => {
                    self.notice = Some(format!(
                        "“{}” could not be moved to the Trash: {e}",
                        p.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    break;
                }
            }
        }
        if !done.is_empty() {
            // No notice of it: the user knows what they did, it shows in the Trash, and Ctrl+Z puts it back.
            self.shared.borrow_mut().undo.push(ops::Done::Trashed(done));
        }
        self.read_all();
    }

    /// The name strip's Enter: the folder made, or the file renamed.
    pub(crate) fn name_done(&mut self) {
        let Some(n) = self.naming.take() else { return };
        let name = n.text.trim().to_string();
        if name.is_empty() || name == "." || name == ".." {
            return;
        }
        if name.contains('/') {
            self.notice = Some("A name cannot have / in it.".into());
            self.naming = Some(n);
            return;
        }
        let done = match &n.rename {
            Some(from) => {
                let to = from.with_file_name(&name);
                if to == *from {
                    return;
                }
                if to.symlink_metadata().is_ok() {
                    self.notice = Some(format!("Something named “{name}” is already here."));
                    self.naming = Some(n);
                    return;
                }
                std::fs::rename(from, &to).map(|_| (ops::Done::Renamed(from.clone(), to.clone()), to))
            }
            None => {
                let Some(dir) = self.here() else { return };
                let to = dir.join(ops::free_name(&name, |x| dir.join(x).symlink_metadata().is_ok()));
                if n.file {
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&to)
                        .map(|_| (ops::Done::Made(to.clone()), to))
                } else {
                    std::fs::create_dir(&to).map(|_| (ops::Done::NewFolder(to.clone()), to))
                }
            }
        };
        match done {
            Ok((d, to)) => {
                self.shared.borrow_mut().undo.push(d);
                self.read_all();
                if let Some(t) = self.front_mut() {
                    t.chosen = vec![to.clone()];
                    t.cursor = Some(to);
                    t.turn += 1;
                }
            }
            Err(e) => self.notice = Some(format!("It could not be done: {}", files::plain(&e))),
        }
    }
}
