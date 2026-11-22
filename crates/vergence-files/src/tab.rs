//! FileView's tabs: what a tab shows and reads (Tab), its history (Back,
//! Forward, Up, a typed path), the window's tabs, and folders' sizes counted
//! by a worker. The window itself (Viewer, Shared, the Program wiring) is in
//! main.rs.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use vergence_ui::TabEvent;
use vergence_widgets::files::{self, Entry};
use vergence_widgets::{Waker, Window};

use crate::{Viewer, cmd, home, ops};

/// What a tab shows: a folder, or the recent files.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Showing {
    Folder(PathBuf),
    Recent,
}

/// One folder in a tab, with its way back and forward.
pub(crate) struct Tab {
    pub(crate) id: u32,
    pub(crate) showing: Showing,
    pub(crate) back: Vec<Showing>,
    pub(crate) forward: Vec<Showing>,
    pub(crate) entries: Vec<Entry>,
    pub(crate) problem: Option<String>,
    /// The folder's modification time when read (changes are looked for).
    pub(crate) stamp: Option<SystemTime>,
    /// What is chosen (by path, so it survives sorting), and the one the arrows move from.
    pub(crate) chosen: Vec<PathBuf>,
    pub(crate) cursor: Option<PathBuf>,
    /// Bumped when the cursor moves, so its row is scrolled to.
    pub(crate) turn: u64,
    /// The window's counted folder sizes, put into the entries as they are read.
    pub(crate) sizes: Sizes,
    /// In the Trash: where each item came from (its path in the Trash to
    /// the path it had).
    pub(crate) from: std::collections::HashMap<PathBuf, PathBuf>,
    /// How far the folder is scrolled (points, as the toolkit last told),
    /// where to scroll it at the next layout, and where each folder of its
    /// history was left (files milestone 3, step 2: Back and Forward bring a
    /// folder back at its place, for the session).
    pub(crate) scroll: f32,
    pub(crate) want: Option<f32>,
    pub(crate) places: std::collections::HashMap<Showing, f32>,
}

impl Tab {
    pub(crate) fn new(id: u32, showing: Showing, sizes: Sizes) -> Tab {
        let mut t = Tab {
            id,
            showing,
            back: Vec::new(),
            forward: Vec::new(),
            entries: Vec::new(),
            problem: None,
            stamp: None,
            chosen: Vec::new(),
            cursor: None,
            turn: 0,
            sizes,
            from: Default::default(),
            scroll: 0.0,
            want: None,
            places: Default::default(),
        };
        t.read();
        t
    }

    /// Leaving the folder shown: its place remembered.
    pub(crate) fn leave(&mut self) {
        self.places.insert(self.showing.clone(), self.scroll);
    }

    /// Arrived at the folder shown: at its place if it was here before, else the top.
    pub(crate) fn arrive(&mut self) {
        self.scroll = self.places.get(&self.showing).copied().unwrap_or(0.0);
        self.want = Some(self.scroll);
    }

    /// Counted sizes into the folders' entries, where the folder has not
    /// changed since; true if any entry's size changed.
    pub(crate) fn put_sizes(&mut self) -> bool {
        let all = self.sizes.lock().unwrap_or_else(|x| x.into_inner());
        let mut changed = false;
        for e in self.entries.iter_mut().filter(|e| e.dir) {
            if let Some((when, Some(n))) = all.get(&e.path)
                && *when == e.changed
                && e.size != *n
            {
                e.size = *n;
                changed = true;
            }
        }
        changed
    }

    pub(crate) fn dir(&self) -> Option<&Path> {
        match &self.showing {
            Showing::Folder(p) => Some(p),
            Showing::Recent => None,
        }
    }

    pub(crate) fn name(&self) -> String {
        match &self.showing {
            Showing::Recent => "Recent".into(),
            Showing::Folder(p) if Some(p.as_path()) == files::home().as_deref() => "Home".into(),
            Showing::Folder(p) if ops::trash_dir().is_some_and(|t| t.join("files") == *p) => "Trash".into(),
            Showing::Folder(p) if p.as_os_str() == "/" => "Computer".into(),
            Showing::Folder(p) => {
                p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
            }
        }
    }

    pub(crate) fn read(&mut self) {
        self.problem = None;
        self.entries = match &self.showing {
            Showing::Folder(dir) => {
                self.stamp = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
                let mut v = files::read_folder(dir).unwrap_or_else(|why| {
                    self.problem = Some(why);
                    Vec::new()
                });
                self.from = match ops::trash_dir() {
                    Some(t) if t.join("files") == *dir => as_trashed(&mut v, &t),
                    _ => Default::default(),
                };
                v
            }
            Showing::Recent => files::recent(),
        };
        // Folders already counted keep their sizes (the Size sort uses them).
        self.put_sizes();
        // What was chosen and is gone is chosen no more.
        let here: std::collections::HashSet<&PathBuf> = self.entries.iter().map(|e| &e.path).collect();
        self.chosen.retain(|p| here.contains(p));
        if self.cursor.as_ref().is_some_and(|c| !here.contains(c)) {
            self.cursor = None;
        }
    }

    pub(crate) fn go(&mut self, to: Showing) {
        if to != self.showing {
            self.leave();
            self.back.push(std::mem::replace(&mut self.showing, to));
            self.forward.clear();
            self.arrive();
        }
        self.chosen.clear();
        self.cursor = None;
        self.read();
    }
}

/// The Trash's entries as its list shows them (Dave, 2026-11-18: "just a
/// list of the files and a restore or remove forever button"): each by the
/// name it had (two of a name have names of their own in the Trash), its
/// time the moment it was trashed (its info file's, written then), and
/// where it came from, returned by its path in the Trash.
pub(crate) fn as_trashed(entries: &mut [Entry], trash: &Path) -> std::collections::HashMap<PathBuf, PathBuf> {
    let info = Some(trash.join("info"));
    let all: std::collections::HashMap<String, ops::Trashed> =
        ops::trashed_in(trash).into_iter().map(|t| (t.name.clone(), t)).collect();
    let mut from = std::collections::HashMap::new();
    for e in entries.iter_mut() {
        let Some(t) = all.get(&e.name) else { continue };
        if let Some(i) = &info
            && let Ok(when) = std::fs::metadata(i.join(format!("{}.trashinfo", t.name))).and_then(|m| m.modified())
        {
            e.changed = Some(when);
        }
        if let Some(n) = t.from.file_name() {
            e.name = n.to_string_lossy().into_owned();
            // By the name it had, else what its bytes said.
            e.mime = files::kind_of(e.dir, &e.name).or(e.mime);
        }
        from.insert(e.path.clone(), t.from.clone());
    }
    from
}

/// Folders' sizes as counted: by path, the folder's own time then and the
/// size (None: too much to count).
pub(crate) type Sizes = Arc<Mutex<std::collections::HashMap<PathBuf, (Option<SystemTime>, Option<u64>)>>>;

/// The most entries counted for one folder's size (more: not shown).
pub(crate) const COUNT_MOST: u32 = 400_000;

/// Everything in a folder, on its own file system only (as `du -x`), not
/// following links; None past [`COUNT_MOST`] entries.
pub(crate) fn count_tree(p: &Path, dev: u64, budget: &mut u32) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    let Ok(m) = p.symlink_metadata() else { return Some(0) };
    if m.dev() != dev {
        return Some(0);
    }
    if !m.is_dir() {
        return Some(m.len());
    }
    let mut total = 0u64;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            *budget = budget.checked_sub(1)?;
            total += count_tree(&e.path(), dev, budget)?;
        }
    }
    Some(total)
}

/// The worker that counts folders' sizes, one at a time, waking the window
/// with each.
pub(crate) fn size_worker(
    sizes: Sizes,
    wake: Arc<Mutex<Option<Waker>>>,
) -> std::sync::mpsc::Sender<(PathBuf, Option<SystemTime>)> {
    use std::os::unix::fs::MetadataExt;
    let (tx, rx) = std::sync::mpsc::channel::<(PathBuf, Option<SystemTime>)>();
    let _ = std::thread::Builder::new().name("folder-sizes".into()).spawn(move || {
        for (p, when) in rx {
            let Ok(m) = p.symlink_metadata() else { continue };
            let mut budget = COUNT_MOST;
            let n = count_tree(&p, m.dev(), &mut budget);
            sizes.lock().unwrap_or_else(|x| x.into_inner()).insert(p, (when, n));
            if let Some(w) = wake.lock().unwrap_or_else(|x| x.into_inner()).as_ref() {
                w.wake();
            }
        }
    });
    tx
}

impl Viewer {
    pub(crate) fn new_tab(&mut self, showing: Showing) {
        let id = {
            let mut s = self.shared.borrow_mut();
            s.next_tab += 1;
            s.next_tab
        };
        self.tabs.push(Tab::new(id, showing, self.sizes.clone()));
        self.front = id;
        self.clear_narrow();
    }

    /// Takes a tab out of this window (moved to another, or closed).
    pub(crate) fn remove_tab(&mut self, id: u32) -> Option<Tab> {
        let i = self.tabs.iter().position(|t| t.id == id)?;
        let t = self.tabs.remove(i);
        if self.front == id {
            self.front = self.tabs.get(i.min(self.tabs.len().saturating_sub(1))).map_or(0, |t| t.id);
            self.tab_changed();
        }
        Some(t)
    }

    /// Closes a tab; the window with it, if it was the last.
    pub(crate) fn close_tab(&mut self, id: u32, window: &mut Window) {
        self.remove_tab(id);
        if self.tabs.is_empty() {
            window.close();
        }
    }

    /// Another tab in front: the window's own state for a folder (the
    /// narrowing, a search, the preview) starts again, and the folder is read
    /// again if it changed while behind.
    pub(crate) fn tab_changed(&mut self) {
        self.clear_narrow();
        self.close_peek();
        self.path_edit = None;
        if let Some(t) = self.front_mut() {
            // Back at the place this tab was scrolled to.
            t.want = Some(t.scroll);
            let now = t.dir().and_then(|d| std::fs::metadata(d).and_then(|m| m.modified()).ok());
            if t.dir().is_none() || now != t.stamp {
                t.read();
            }
        }
    }

    /// The typed path gone to: a folder, or a file's folder with the file
    /// chosen; "~" is home. Not there: said, and the field stays to mend it.
    pub(crate) fn path_go(&mut self) {
        let Some(typed) = self.path_edit.clone() else { return };
        let typed = typed.trim();
        let p = match typed.strip_prefix('~') {
            Some(rest) => home().join(rest.trim_start_matches('/')),
            None => PathBuf::from(typed),
        };
        if !p.is_absolute() || !p.exists() {
            self.notice = Some(format!("There is no folder or file at {typed}."));
            return;
        }
        self.path_edit = None;
        if p.is_dir() {
            self.go(Showing::Folder(p));
        } else if let Some(dir) = p.parent() {
            self.go(Showing::Folder(dir.to_path_buf()));
            self.choose_only(p.clone());
        }
    }

    /// A tab from another window: this window's folder sizes from now on.
    pub(crate) fn take_tab(&mut self, mut t: Tab, at: usize) {
        t.sizes = self.sizes.clone();
        self.front = t.id;
        self.tabs.insert(at.min(self.tabs.len()), t);
        self.tab_changed();
    }

    pub(crate) fn front_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == self.front).or(self.tabs.first())
    }

    pub(crate) fn front_mut(&mut self) -> Option<&mut Tab> {
        let id = self.front_tab()?.id;
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub(crate) fn go(&mut self, to: Showing) {
        self.clear_narrow();
        if let Some(t) = self.front_mut() {
            t.go(to);
        }
    }

    pub(crate) fn back(&mut self) {
        if let Some(t) = self.front_mut()
            && let Some(s) = t.back.pop()
        {
            t.leave();
            t.forward.push(std::mem::replace(&mut t.showing, s));
            t.arrive();
            t.read();
        }
        self.clear_narrow();
    }

    pub(crate) fn forward(&mut self) {
        if let Some(t) = self.front_mut()
            && let Some(s) = t.forward.pop()
        {
            t.leave();
            t.back.push(std::mem::replace(&mut t.showing, s));
            t.arrive();
            t.read();
        }
        self.clear_narrow();
    }

    pub(crate) fn up(&mut self) {
        if self.in_trash() {
            return;
        }
        let up = self.front_tab().and_then(|t| t.dir()).and_then(Path::parent).map(Path::to_path_buf);
        if let Some(p) = up {
            let came_from = self.front_tab().and_then(|t| t.dir().map(Path::to_path_buf));
            self.go(Showing::Folder(p));
            // The folder come up from is chosen.
            if let (Some(c), Some(t)) = (came_from, self.front_mut()) {
                t.chosen = vec![c.clone()];
                t.cursor = Some(c);
                t.turn += 1;
            }
        }
    }

    /// A folder's size if counted (asked for if not, unless it is where
    /// counting does not belong: a network mount, the Trash, the system's
    /// own trees).
    pub(crate) fn folder_size(&self, e: &Entry) -> Option<u64> {
        if let Some((when, n)) = self.sizes.lock().unwrap_or_else(|x| x.into_inner()).get(&e.path)
            && *when == e.changed
        {
            return *n;
        }
        // Asked once per folder and time (the mount table read only then).
        if !self.size_asked.borrow_mut().insert((e.path.clone(), e.changed)) {
            return None;
        }
        let skip = self.in_trash()
            || ["/proc", "/sys", "/dev", "/run"].iter().any(|r| e.path.starts_with(r))
            || files::network_mounts().iter().any(|m| e.path.starts_with(m));
        if !skip {
            let _ = self.size_queue.send((e.path.clone(), e.changed));
        }
        None
    }

    /// The folder shown, if it is one (not Recent).
    pub(crate) fn here(&self) -> Option<PathBuf> {
        self.front_tab().and_then(|t| t.dir().map(Path::to_path_buf))
    }

    /// Whether the folder shown is the Trash's.
    pub(crate) fn in_trash(&self) -> bool {
        let trash = ops::trash_dir().map(|d| d.join("files"));
        trash.is_some() && self.here() == trash
    }

    /// Every tab read again (after an operation).
    pub(crate) fn read_all(&mut self) {
        for t in &mut self.tabs {
            t.read();
        }
    }

    pub(crate) fn on_tab(&mut self, event: TabEvent, window: &mut Window) {
        // A name being changed is kept before the tabs change.
        if self.renaming_row().is_some() {
            self.name_done();
        }
        match event {
            TabEvent::Chosen(id) => {
                self.front = id;
                self.tab_changed();
            }
            TabEvent::Close(id) => self.close_tab(id, window),
            TabEvent::CloseOthers(id) => {
                self.tabs.retain(|t| t.id == id);
                self.front = id;
                self.tab_changed();
            }
            TabEvent::New => self.run(cmd::NEW_TAB, window),
            TabEvent::Moved { tab, to } => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == tab) {
                    let t = self.tabs.remove(i);
                    self.tabs.insert((to as usize).min(self.tabs.len()), t);
                }
            }
            // Out of the row: a window of its own, its folder, history and choice with it.
            TabEvent::Out { tab, .. } => {
                if self.tabs.len() > 1
                    && let Some(t) = self.remove_tab(tab)
                {
                    let mut w = Viewer::new(self.shared.clone());
                    w.take_tab(t, 0);
                    window.open_window(vergence_widgets::window(w));
                }
            }
            TabEvent::Join { tab, into, to } => {
                if let Some(t) = self.remove_tab(tab) {
                    let waker = {
                        let mut s = self.shared.borrow_mut();
                        s.transfers.push((into, to, t));
                        s.windows.iter().find(|(id, _)| *id == into).map(|(_, w)| w.clone())
                    };
                    if let Some(w) = waker {
                        w.wake();
                    }
                    if self.tabs.is_empty() {
                        window.close();
                    }
                }
            }
            TabEvent::Menu { .. } => {}
        }
        self.keep();
        window.redraw();
    }
}
