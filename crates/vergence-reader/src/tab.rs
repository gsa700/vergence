//! Reader's tabs: a document in a tab (Tab, Content), read by a worker
//! (`load`), its place when it opens, the window's tabs (opening, moving,
//! closing, unlocking), files read again when changed, and what wakes the
//! window (`on_woken`). The window itself (Viewer, Shared, the Program
//! wiring) is in main.rs.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use vergence_ui::TabEvent;
use vergence_widgets::{Waker, Window};

use crate::{Shared, Viewer, WATCH_EVERY, WATCH_TIMER, cmd, keep, pdf};

/// What a file turned out to be, from the worker that read it.
pub(crate) enum Loaded {
    Pdf(pdf::Pdf),
    /// A PDF that needs its password, open in its helper.
    Locked(vergence_pdf::helper::Viewer),
    /// Why it cannot be shown here, and whether another app might.
    Problem {
        why: String,
        elsewhere: bool,
    },
}

/// A tab: one document, and the pages gone to by links, the outline,
/// find and the page field (back and forward through them).
pub(crate) struct Tab {
    pub(crate) id: u32,
    pub(crate) path: PathBuf,
    pub(crate) content: Content,
    pub(crate) back: Vec<usize>,
    pub(crate) forward: Vec<usize>,
    /// The file as last read (modified time, length): changed on disk, it
    /// is read again.
    pub(crate) stamp: Option<(SystemTime, u64)>,
    /// A document read again: its view to put back (zoom, view, turn,
    /// night, page) once it is open.
    pub(crate) keep: Option<(pdf::Mode, pdf::View, u16, bool, usize)>,
    /// Its place kept from the last run (the window came back after a
    /// restart), gone to once it is open; None: opened by the user, it
    /// starts at the first page.
    pub(crate) kept: Option<keep::Place>,
}

/// A file's modified time and length, to see that it changed.
fn stamp_of(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

fn new_tab(id: u32, path: PathBuf) -> Tab {
    let stamp = stamp_of(&path);
    Tab { id, path, content: Content::Loading, back: Vec::new(), forward: Vec::new(), stamp, keep: None, kept: None }
}

pub(crate) enum Content {
    Loading,
    Pdf(pdf::Pdf),
    /// The helper (taken while a password is tried), the password typed, and
    /// whether the last one was wrong.
    Locked {
        viewer: Option<vergence_pdf::helper::Viewer>,
        password: String,
        wrong: bool,
    },
    Problem {
        why: String,
        elsewhere: bool,
    },
}

impl Tab {
    pub(crate) fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }

    /// The document's own title (its Info /Title) when it has a real one,
    /// else the file's name.
    pub(crate) fn title(&self) -> String {
        if let Content::Pdf(d) = &self.content
            && let Some(meta) = d.shared.meta.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
            && let Some((_, t)) = meta.iter().find(|(k, _)| k == "Title")
            && title_is_real(t)
        {
            return t.clone();
        }
        self.name()
    }
}

/// Whether a document's Title says something: not empty, not "untitled",
/// not a file's name (as many programs write: "Microsoft Word - a.docx").
pub(crate) fn title_is_real(t: &str) -> bool {
    let low = t.trim().to_lowercase();
    let file_like =
        [".pdf", ".doc", ".docx", ".odt", ".rtf", ".txt", ".tex", ".indd", ".ps", ".xls", ".xlsx", ".ppt", ".pptx"]
            .iter()
            .any(|e| low.ends_with(e));
    low.chars().count() >= 3
        && !["untitled", "title", "document", "(anonymous)"].contains(&low.as_str())
        && !low.starts_with("microsoft word - ")
        && !file_like
}

/// Reads a document in a worker: opened in its helper, or why not.
pub(crate) fn load(path: PathBuf, id: u32, shared: &Shared, waker: Option<Waker>) {
    let out = shared.loaded.clone();
    std::thread::spawn(move || {
        let mime = vergence_types::of_file(&path);
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let loaded = if mime == "application/pdf" {
            match pdf::open(&path, waker.clone()) {
                Ok(pdf::Opened::Ready(d)) => Loaded::Pdf(d),
                Ok(pdf::Opened::Locked(v)) => Loaded::Locked(v),
                Err(why) => {
                    Loaded::Problem { why: format!("“{name}” could not be shown: {why}"), elsewhere: false }
                }
            }
        } else if mime == vergence_types::FOLDER {
            Loaded::Problem { why: "This is a folder.".into(), elsewhere: true }
        } else {
            Loaded::Problem {
                why: format!("“{name}” is {} ({mime}), not a PDF document.", vergence_types::describe(mime)),
                elsewhere: true,
            }
        };
        out.lock().unwrap_or_else(|e| e.into_inner()).push((id, loaded));
        if let Some(w) = waker {
            w.wake();
        }
    });
}

/// A document as it opens (session milestone 1; Dave, 2026-11-21: after
/// a restart "bring it back JUST like it was ... scrolled to where they
/// were", but "if I close it by choice and reopen it sometime later, I
/// think it should open at the first page"). Kept from the last run: its
/// zoom and place, gone to once placed (unless `[session] places` is
/// false). Opened by the user: the first page, at the zoom it was last read at.
fn with_place(mut d: pdf::Pdf, path: &Path, kept: Option<&keep::Place>) -> pdf::Pdf {
    if let Some((_, mode)) = pdf::places::load(path) {
        d.mode = mode;
    }
    if let Some(k) = kept
        && vergence_widgets::session::places()
    {
        if let Some(mode) = pdf::places::parse_mode(&k.mode) {
            d.mode = mode;
        }
        d.restore = Some((k.page.min(d.pages.len() - 1), k.within));
    }
    d
}

impl Viewer {
    /// Files to show, each in a tab of this window (one already open is
    /// brought to the front).
    pub(crate) fn take(&mut self, paths: &[PathBuf]) {
        self.open_paths(paths);
    }

    /// Takes a tab out of this window (to another, or one of its own).
    fn remove_tab(&mut self, id: u32) -> Option<Tab> {
        let i = self.tabs.iter().position(|t| t.id == id)?;
        let t = self.tabs.remove(i);
        if self.front == id {
            self.front = self.tabs.get(i.min(self.tabs.len().saturating_sub(1))).map_or(0, |t| t.id);
        }
        Some(t)
    }

    /// Closes a tab; the window with it, if it was the last.
    pub(crate) fn close_tab(&mut self, id: u32, window: &mut Window) {
        self.remove_tab(id);
        if self.tabs.is_empty() {
            window.close();
        }
        self.tab_changed();
    }

    /// Another document in front: the search goes on in it, the page field
    /// starts again.
    fn tab_changed(&mut self) {
        self.page_field = None;
        self.pending_uri = None;
        if let Some(q) = self.finding.clone()
            && let Some(d) = self.pdf_mut()
            && d.shared.search.lock().unwrap_or_else(|e| e.into_inner()).query != q.trim()
        {
            d.start_search(&q);
        }
    }

    /// Files changed on disk since they were read: read again, the view
    /// kept (as Edit does).
    pub(crate) fn watch(&mut self) {
        let r = self.area.get();
        let waker = self.waker.clone();
        let shared = self.shared.clone();
        for t in self.tabs.iter_mut() {
            if matches!(t.content, Content::Loading) {
                continue;
            }
            let now = stamp_of(&t.path);
            if now.is_none() || now == t.stamp {
                continue;
            }
            t.stamp = now;
            if let Content::Pdf(d) = &t.content {
                t.keep = Some((d.mode, d.view, d.turn, d.night, d.current(r)));
            }
            t.content = Content::Loading;
            load(t.path.clone(), t.id, &shared.borrow(), waker.clone());
        }
    }

    fn next_id(&self) -> u32 {
        let mut s = self.shared.borrow_mut();
        s.next_tab += 1;
        s.next_tab
    }

    /// A file shown once the window can be woken (its `waker` reads it).
    pub(crate) fn queue(&mut self, p: PathBuf) {
        let id = self.next_id();
        self.tabs.push(new_tab(id, p));
        self.front = id;
    }

    fn open_paths(&mut self, paths: &[PathBuf]) {
        for p in paths {
            let p = std::path::absolute(p).unwrap_or_else(|_| p.clone());
            // Already open: brought to the front.
            if let Some(t) = self.tabs.iter().find(|t| t.path == p) {
                self.front = t.id;
                continue;
            }
            let id = self.next_id();
            load(p.clone(), id, &self.shared.borrow(), self.waker.clone());
            self.tabs.push(new_tab(id, p));
            self.front = id;
        }
    }

    pub(crate) fn front_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == self.front).or(self.tabs.first())
    }

    pub(crate) fn front_mut(&mut self) -> Option<&mut Tab> {
        let id = self.front_tab()?.id;
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// The front document, if it is open.
    pub(crate) fn pdf_mut(&mut self) -> Option<&mut pdf::Pdf> {
        match &mut self.front_mut()?.content {
            Content::Pdf(d) => Some(d),
            _ => None,
        }
    }

    pub(crate) fn is_pdf(&self) -> bool {
        self.front_tab().is_some_and(|t| matches!(t.content, Content::Pdf(_)))
    }

    /// Tries the typed password on a locked document; opens it if it is right.
    pub(crate) fn unlock(&mut self) {
        let waker = self.waker.clone();
        let Some(t) = self.front_mut() else { return };
        let Content::Locked { viewer, password, wrong } = &mut t.content else { return };
        let typed = std::mem::take(password);
        let Some(mut v) = viewer.take() else { return };
        match v.unlock(typed.as_bytes()) {
            Ok(true) => {
                t.content = match pdf::start(v, waker) {
                    Ok(d) => Content::Pdf(with_place(d, &t.path, t.kept.take().as_ref())),
                    Err(why) => Content::Problem { why, elsewhere: false },
                }
            }
            Ok(false) => {
                *viewer = Some(v);
                *wrong = true;
            }
            Err(e) => t.content = Content::Problem { why: e.to_string(), elsewhere: false },
        }
    }

    pub(crate) fn on_tab(&mut self, event: TabEvent, window: &mut Window) {
        match event {
            TabEvent::Chosen(id) => {
                self.front = id;
                self.tab_changed();
            }
            TabEvent::Close(id) => self.close_tab(id, window),
            TabEvent::CloseOthers(id) => {
                self.tabs.retain(|t| t.id == id);
                self.front = id;
            }
            TabEvent::New => self.run(cmd::OPEN, window),
            TabEvent::Moved { tab, to } => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == tab) {
                    let t = self.tabs.remove(i);
                    self.tabs.insert((to as usize).min(self.tabs.len()), t);
                }
            }
            // Out of the row: a window of its own, its view with it.
            TabEvent::Out { tab, .. } => {
                if self.tabs.len() > 1
                    && let Some(t) = self.remove_tab(tab)
                {
                    let mut w = Viewer::new(self.shared.clone());
                    w.front = t.id;
                    w.tabs.push(t);
                    window.open_window(vergence_widgets::window(w));
                    self.tab_changed();
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

    pub(crate) fn on_woken(&mut self, window: &mut Window) -> bool {
        // The other windows kept from the last run.
        let pending = std::mem::take(&mut self.shared.borrow_mut().pending);
        for snap in pending {
            let w = Viewer::from_snap(self.shared.clone(), &snap);
            window.open_window(vergence_widgets::window(w));
        }
        // Woken by the drawing (its place changed) or a page drawn: look again.
        let mut changed = self.reread() || self.is_pdf();
        let asked: Vec<Vec<PathBuf>> =
            std::mem::take(&mut *self.shared.borrow().inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for paths in asked {
            self.take(&paths);
            // Handed over by a start (or the Open dialog): forward, if the server agrees.
            window.activate();
            changed = true;
        }
        // Tabs dropped on this window.
        let moved: Vec<(u32, u32, Tab)> = {
            let mut s = self.shared.borrow_mut();
            let (mine, rest): (Vec<_>, Vec<_>) =
                std::mem::take(&mut s.transfers).into_iter().partition(|(into, _, _)| Some(*into) == self.me);
            s.transfers = rest;
            mine
        };
        for (_, to, t) in moved {
            if let (Content::Pdf(d), Some(w)) = (&t.content, &self.waker) {
                d.shared.set_waker(w.clone());
            }
            self.front = t.id;
            self.tabs.insert((to as usize).min(self.tabs.len()), t);
            self.tab_changed();
            changed = true;
        }
        if !self.watching {
            self.watching = true;
            window.wake_after(WATCH_TIMER, WATCH_EVERY);
        }
        let done: Vec<(u32, Loaded)> = {
            let shared = self.shared.borrow();
            let mut all = shared.loaded.lock().unwrap_or_else(|e| e.into_inner());
            let (mine, rest): (Vec<_>, Vec<_>) =
                all.drain(..).partition(|(id, _)| self.tabs.iter().any(|t| t.id == *id));
            *all = rest;
            mine
        };
        for (id, loaded) in done {
            if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                t.content = match loaded {
                    Loaded::Pdf(d) => Content::Pdf(with_place(d, &t.path, t.kept.take().as_ref())),
                    Loaded::Locked(v) => Content::Locked { viewer: Some(v), password: String::new(), wrong: false },
                    Loaded::Problem { why, elsewhere } => Content::Problem { why, elsewhere },
                };
                changed = true;
            }
        }
        // A document read again: its view put back.
        let r = self.area.get();
        if r.w > 1.0 {
            for t in self.tabs.iter_mut() {
                if let (Content::Pdf(d), Some((mode, view, turn, night, page))) = (&mut t.content, t.keep) {
                    d.restore = None;
                    d.mode = mode;
                    d.view = view;
                    d.night = night;
                    d.set_turn(turn, r);
                    d.go_to(page, r);
                    t.keep = None;
                    changed = true;
                }
            }
        }
        // A remembered page, gone to once the view has its place.
        if r.w > 1.0
            && let Some(d) = self.pdf_mut()
            && let Some((page, within)) = d.restore.take()
        {
            d.go_to_place(page, within, r);
            changed = true;
        }
        // Documents opened, moved or closed: kept for the next start.
        self.keep();
        changed
    }
}
