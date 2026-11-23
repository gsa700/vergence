//! Archives in FileView (archive milestone 1, docs/archive-milestone-1.md):
//! extracting (a double click, Extract Here, Extract To…) and Compress to
//! .zip, each a job on a worker with its progress in the strip, as a copy
//! is. The work is vergence-archive's: an archive is read only in its
//! confined helper, and the files are made by its rules.
//!
//! An extract lists every archive first, so the caps are asked about
//! before anything is written ("Extract anyway"), and the progress has
//! its total.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use vergence_archive::{make, unpack};
use vergence_widgets::{Program, Window};

use crate::{APP_ID, Ask, Job, JobResult, Viewer, ops};

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Why an archive could not be extracted, in words for the strip.
fn said(archive: &Path, e: &unpack::Error) -> String {
    let n = name_of(archive);
    match e {
        unpack::Error::Stopped => "Stopped.".into(),
        unpack::Error::NeedsPassword => {
            format!("“{n}” needs a password. Archives with a password cannot be opened yet.")
        }
        e => format!("“{n}” could not be extracted: {e}."),
    }
}

/// The job's two progresses kept as one while `running`: the archive's
/// bytes shown in the strip, the strip's Stop passed to the archive.
fn mirror(strip: Arc<ops::Progress>, work: Arc<unpack::Progress>, running: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        while running.load(Ordering::Relaxed) {
            strip.done.store(work.done.load(Ordering::Relaxed), Ordering::Relaxed);
            if strip.stop.load(Ordering::Relaxed) {
                work.stop.store(true, Ordering::Relaxed);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}

/// `work` run on a worker; the window looks again while it runs (its
/// ticks) and when it ends.
fn on_worker(waker: Option<vergence_widgets::Waker>, work: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        let ticking = Arc::new(AtomicBool::new(true));
        let (t, w) = (ticking.clone(), waker.clone());
        std::thread::spawn(move || {
            while t.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if let Some(w) = &w {
                    w.wake();
                }
            }
        });
        work();
        ticking.store(false, Ordering::Relaxed);
        if let Some(w) = waker {
            w.wake();
        }
    });
}

impl Viewer {
    /// The chosen items its menu may extract.
    pub(crate) fn chosen_archives(&self) -> Vec<PathBuf> {
        self.chosen_paths()
            .into_iter()
            .filter(|p| !p.is_dir() && vergence_widgets::files::can_extract(vergence_types::of_file(p)))
            .collect()
    }

    /// Extracts `archives`, each beside itself or all into `into`, on a
    /// worker. `agreed`: the person has said Extract anyway, so the caps
    /// are not asked about again.
    pub(crate) fn start_extract(&mut self, archives: Vec<PathBuf>, into: Option<PathBuf>, agreed: bool) {
        if archives.is_empty() {
            return;
        }
        if self.job.is_some() {
            self.notice = Some("A copy is still going on.".into());
            return;
        }
        self.notice = None;
        let words = match archives.as_slice() {
            [one] => format!("Extracting “{}”", name_of(one)),
            many => format!("Extracting {} archives", many.len()),
        };
        let progress = Arc::new(ops::Progress::default());
        let result: JobResult = Arc::default();
        let ask: Arc<Mutex<Option<Ask>>> = Arc::default();
        let (p, r, q) = (progress.clone(), result.clone(), ask.clone());
        on_worker(self.waker.clone(), move || {
            let work = Arc::new(unpack::Progress::default());
            let running = Arc::new(AtomicBool::new(true));
            mirror(p.clone(), work.clone(), running.clone());
            let out = extract_all(&archives, into.as_deref(), agreed, &p, &work, &q);
            running.store(false, Ordering::Relaxed);
            *r.lock().unwrap_or_else(|e| e.into_inner()) = Some(out);
        });
        self.job = Some(Job { words, progress, result, ask });
    }

    /// Extract To…: a folder chosen in the file dialog, then the chosen
    /// archives extracted into it (the answer comes back through
    /// `extract_to`, as a mount's does).
    pub(crate) fn extract_to_dialog(&mut self, window: &mut Window) {
        let archives = self.chosen_archives();
        if archives.is_empty() {
            return;
        }
        let (slot, waker) = (self.extract_to.clone(), self.waker.clone());
        let here = self.here();
        let mut d = vergence_widgets::filedialog::FileDialog::choose_folder(
            APP_ID,
            "Extract to",
            self.theme(),
            move |chosen| {
                if let Some(dir) = chosen.into_iter().next() {
                    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some((archives, dir));
                    if let Some(w) = &waker {
                        w.wake();
                    }
                }
            },
        );
        if let Some(dir) = here {
            d = d.folder(&dir);
        }
        window.open_window(vergence_widgets::window(d));
    }

    /// The folder Extract To… chose, once the dialog answers.
    pub(crate) fn extract_to_answered(&mut self) -> bool {
        let Some((archives, dir)) = self.extract_to.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return false;
        };
        self.start_extract(archives, Some(dir), false);
        true
    }

    /// Compress: one .zip of the chosen items beside them ("Archive.zip"
    /// for several, the item's name for one), on a worker with progress;
    /// undone by putting it in the Trash.
    pub(crate) fn compress(&mut self) {
        let chosen = self.chosen_paths();
        let Some(dir) = chosen.first().and_then(|p| p.parent()).map(Path::to_path_buf) else { return };
        if chosen.iter().any(|p| p.parent() != Some(dir.as_path())) {
            self.notice = Some("Items from different folders cannot be compressed together.".into());
            return;
        }
        if self.job.is_some() {
            self.notice = Some("A copy is still going on.".into());
            return;
        }
        self.notice = None;
        let base = match chosen.as_slice() {
            [one] => name_of(one),
            _ => "Archive".into(),
        };
        let name = ops::free_name(&format!("{base}.zip"), |x| dir.join(x).symlink_metadata().is_ok());
        let to = dir.join(&name);
        let words = format!("Compressing into “{name}”");
        let progress = Arc::new(ops::Progress::default());
        let result: JobResult = Arc::default();
        let (p, r) = (progress.clone(), result.clone());
        on_worker(self.waker.clone(), move || {
            p.total.store(make::size_of(&chosen).max(1), Ordering::Relaxed);
            let work = Arc::new(unpack::Progress::default());
            let running = Arc::new(AtomicBool::new(true));
            mirror(p.clone(), work.clone(), running.clone());
            let out = match make::compress(&chosen, &to, &work) {
                Ok(()) => (ops::Done::Made(to), None),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    (ops::Done::Extracted(Vec::new()), Some("Stopped.".into()))
                }
                Err(e) => (ops::Done::Extracted(Vec::new()), Some(format!("“{name}” could not be made: {e}."))),
            };
            running.store(false, Ordering::Relaxed);
            *r.lock().unwrap_or_else(|e| e.into_inner()) = Some(out);
        });
        self.job = Some(Job { words, progress, result, ask: Arc::default() });
    }
}

/// The worker's whole extract: every listing first (the caps asked about
/// through `ask` before anything is written, unless `agreed`), then each
/// archive in turn. What was made, and what to say.
fn extract_all(
    archives: &[PathBuf],
    into: Option<&Path>,
    agreed: bool,
    strip: &ops::Progress,
    work: &Arc<unpack::Progress>,
    ask: &Mutex<Option<Ask>>,
) -> (ops::Done, Option<String>) {
    let mut listed = Vec::new();
    for a in archives {
        match unpack::list(a, &work.stop) {
            Ok(l) => listed.push(l),
            Err(e) => return (ops::Done::Extracted(Vec::new()), Some(said(a, &e))),
        }
    }
    if !agreed {
        let why = archives.iter().zip(&listed).find_map(|(a, l)| {
            let size = a.metadata().map(|m| m.len()).unwrap_or(0);
            l.ask(size).map(|w| if archives.len() > 1 { format!("“{}”: {w}", name_of(a)) } else { w })
        });
        if let Some(why) = why {
            *ask.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(Ask::Extract { archives: archives.to_vec(), into: into.map(Path::to_path_buf), why });
            return (ops::Done::Extracted(Vec::new()), None);
        }
    }
    strip.total.store(listed.iter().map(|l| l.total).sum::<u64>().max(1), Ordering::Relaxed);
    let (mut made, mut skipped) = (Vec::new(), 0u64);
    for (a, l) in archives.iter().zip(&listed) {
        *strip.now.lock().unwrap_or_else(|e| e.into_inner()) = name_of(a);
        let dir = match into.map(Path::to_path_buf).or_else(|| a.parent().map(Path::to_path_buf)) {
            Some(d) => d,
            None => continue,
        };
        match unpack::extract(a, &dir, l, work) {
            Ok(x) => {
                skipped += x.skipped;
                made.push(x.made);
            }
            Err(e) => return (ops::Done::Extracted(made), Some(said(a, &e))),
        }
    }
    let note = (skipped > 0).then(|| {
        format!(
            "{skipped} item{} left out: names or links that would reach outside, or devices.",
            if skipped == 1 { " was" } else { "s were" }
        )
    });
    (ops::Done::Extracted(made), note)
}
