// Search in subfolders (files milestone 1, step 10; docs/files-milestone-1.md
// §5): the names below a folder that have the words in them, found by a
// worker thread walking down from it, the results coming as they are found.
// It stays on the folder's own disk (no wandering into other mounts, /proc
// or a network share mounted below), does not follow links to folders,
// leaves hidden names out unless they are shown, and stops at MAX results.
// No index (as Spotlight keeps): a walk each time.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vergence_widgets::files::Entry;

/// The most results a search gathers.
pub const MAX: usize = 5000;

pub struct Search {
    found: Arc<Mutex<Vec<Entry>>>,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}

impl Search {
    /// Starts walking down from `root`; `wake` is called as results come
    /// (at most five times a second) and when it is done.
    pub fn start(root: &Path, words: &str, hidden: bool, wake: impl Fn() + Send + 'static) -> Search {
        let (found, stop, done) = (Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (f, s, d, from, w) = (found.clone(), stop.clone(), done.clone(), root.to_path_buf(), words.to_lowercase());
        std::thread::spawn(move || {
            walk(&from, &w, hidden, &s, |batch| {
                f.lock().unwrap_or_else(|e| e.into_inner()).extend(batch);
                wake();
            });
            d.store(true, Ordering::Release);
            wake();
        });
        Search { found, stop, done }
    }

    /// What has been found so far.
    pub fn found(&self) -> Vec<Entry> {
        self.found.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn count(&self) -> usize {
        self.found.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// The walk: breadth first (what is near shows first), handing batches
/// of matches to `give`.
pub fn walk(root: &Path, words: &str, hidden: bool, stop: &AtomicBool, mut give: impl FnMut(Vec<Entry>)) {
    use std::os::unix::fs::MetadataExt;
    let Ok(top) = std::fs::metadata(root) else { return };
    let dev = top.dev();
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    let (mut batch, mut total, mut last) = (Vec::new(), 0usize, Instant::now());
    while let Some(dir) = queue.pop_front() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for item in rd.flatten() {
            if stop.load(Ordering::Acquire) {
                return;
            }
            let name = item.file_name().to_string_lossy().into_owned();
            if !hidden && name.starts_with('.') {
                continue;
            }
            let path = item.path();
            // The item itself, links not followed: a link to a folder is not walked into.
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            if meta.is_dir() && meta.dev() == dev {
                queue.push_back(path.clone());
            }
            if name.to_lowercase().contains(words)
                && let Some(e) = Entry::of(path)
            {
                batch.push(e);
                total += 1;
                if total >= MAX {
                    give(std::mem::take(&mut batch));
                    return;
                }
            }
            if !batch.is_empty() && last.elapsed() >= Duration::from_millis(200) {
                give(std::mem::take(&mut batch));
                last = Instant::now();
            }
        }
    }
    if !batch.is_empty() {
        give(batch);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    #[test]
    fn the_walk_finds_names_below_leaves_hidden_out_and_does_not_follow_links() {
        let dir = std::env::temp_dir().join(format!("vsearch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::create_dir_all(dir.join(".hid")).unwrap();
        for f in ["Report.txt", "a/report-2.md", "a/b/old REPORT.pdf", "a/other.txt", ".hid/report.txt"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        std::os::unix::fs::symlink(dir.join("a"), dir.join("link")).unwrap();
        let mut got = Vec::new();
        super::walk(&dir, "report", false, &AtomicBool::new(false), |b| got.extend(b.into_iter().map(|e| e.name)));
        got.sort();
        assert_eq!(got, vec!["Report.txt", "old REPORT.pdf", "report-2.md"], "case not looked at; hidden left out; the link not walked");
        let mut with_hidden = 0;
        super::walk(&dir, "report", true, &AtomicBool::new(false), |b| with_hidden += b.len());
        assert_eq!(with_hidden, 4);
        let mut none = 0;
        super::walk(&dir, "report", false, &AtomicBool::new(true), |b| none += b.len());
        assert_eq!(none, 0, "stopped at once");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
