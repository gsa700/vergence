//! Reading a library in the background (step 2): the scan runs on a thread
//! of its own, with Hymie's helper; the window hears its progress and its
//! end through a shared state and the toolkit's waker.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use vergence_albumwall::index::{self, Index};
use vergence_albumwall::library::Album;
use vergence_albumwall::scan::{self, Progress, Unreachable};
use vergence_albumwall::subsonic::{self, Listing, Server};
use vergence_widgets::Waker;

/// What a finished read found.
pub enum Outcome {
    Albums {
        albums: Vec<Album>,
        files: usize,
    },
    /// It could not be reached; `held` tracks the index remembers under it
    /// (music last time: a sleeping drive, not a wrong folder).
    Unreachable {
        why: Unreachable,
        held: usize,
    },
}

#[derive(Default)]
pub struct State {
    pub progress: Option<Progress>,
    pub outcome: Option<Outcome>,
    pub running: bool,
}

/// One library being read, or read.
#[derive(Default, Clone)]
pub struct Reader {
    pub state: Arc<Mutex<State>>,
    cancel: Arc<AtomicBool>,
}

impl Reader {
    /// Reads `root` on a thread of its own (`rescan`: every file opened, the
    /// index not trusted). Nothing if a read is under way. A music server
    /// (`server`) is read from what is kept of it first, then asked.
    pub fn start(&mut self, root: PathBuf, rescan: bool, waker: Option<Waker>, server: Option<Server>) {
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.running {
                return;
            }
            s.running = true;
            s.progress = None;
        }
        self.cancel = Arc::new(AtomicBool::new(false));
        let (state, cancel) = (self.state.clone(), self.cancel.clone());
        std::thread::Builder::new()
            .name("scan".into())
            .spawn(move || {
                if let Some(srv) = server {
                    read_server(&root, srv, &state, &cancel, waker.as_ref());
                    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                    s.running = false;
                    drop(s);
                    if let Some(w) = &waker {
                        w.wake();
                    }
                    return;
                }
                let index_path = index::path_for(&root);
                let mut idx = index_path.as_deref().map(Index::load).unwrap_or_default();
                let held = idx.tracks.keys().filter(|p| p.starts_with(&root)).count();
                let outcome = match vergence_hymie::helper::Scanner::new() {
                    Err(e) => Outcome::Unreachable { why: Unreachable::Failed(e.to_string()), held },
                    Ok(mut scanner) => {
                        let wake = waker.clone();
                        let progress_state = state.clone();
                        match scan::scan(&root, &mut idx, !rescan, held > 0 && !rescan, &mut scanner, &cancel, |p| {
                            progress_state.lock().unwrap_or_else(|e| e.into_inner()).progress = Some(p);
                            if let Some(w) = &wake {
                                w.wake();
                            }
                        }) {
                            Ok(found) => {
                                if let Some(p) = &index_path {
                                    let _ = idx.save(p);
                                }
                                Outcome::Albums { albums: found.albums, files: found.files }
                            }
                            Err(why) => Outcome::Unreachable { why, held },
                        }
                    }
                };
                {
                    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                    s.outcome = Some(outcome);
                    s.running = false;
                }
                if let Some(w) = &waker {
                    w.wake();
                }
            })
            .ok();
    }

    /// A finished read's outcome, taken (once).
    pub fn take(&self) -> Option<Outcome> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).outcome.take()
    }

    pub fn progress(&self) -> Option<Progress> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).progress
    }

    pub fn running(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).running
    }

    /// Stops a read under way between files (another library chosen).
    pub fn stop(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A music server's library: what is kept of it shown at once, then the
/// server asked (every album and song, the covers not yet kept); its answer
/// replaces what was shown and is kept. A server that does not answer leaves
/// what was kept on the wall (`held`: its songs), as a sleeping drive does.
fn read_server(
    root: &std::path::Path,
    srv: Server,
    state: &Arc<Mutex<State>>,
    cancel: &AtomicBool,
    waker: Option<&Waker>,
) {
    let wake = || {
        if let Some(w) = waker {
            w.wake();
        }
    };
    let Some((list_path, cover_dir)) = subsonic::cache_paths(root) else { return };
    let kept = Listing::load(&list_path);
    let held = kept.as_ref().map_or(0, |l| l.songs.len());
    if let Some(l) = &kept {
        let albums = subsonic::albums(root, l, &cover_dir);
        let files = l.songs.len();
        state.lock().unwrap_or_else(|e| e.into_inner()).outcome = Some(Outcome::Albums { albums, files });
        wake();
    }
    let progress = |songs: usize| {
        state.lock().unwrap_or_else(|e| e.into_inner()).progress =
            Some(Progress { done: songs, total: songs, opened: songs });
        wake();
    };
    match srv.listing(cancel, progress) {
        Ok(l) => {
            srv.fetch_covers(&l, &cover_dir, cancel, |done, of| {
                state.lock().unwrap_or_else(|e| e.into_inner()).progress =
                    Some(Progress { done, total: of, opened: done });
                wake();
            });
            let _ = l.save(&list_path);
            let albums = subsonic::albums(root, &l, &cover_dir);
            let files = l.songs.len();
            state.lock().unwrap_or_else(|e| e.into_inner()).outcome = Some(Outcome::Albums { albums, files });
        }
        Err(e) => {
            let why = Unreachable::Failed(e.to_string());
            state.lock().unwrap_or_else(|e| e.into_inner()).outcome = Some(Outcome::Unreachable { why, held });
        }
    }
}
