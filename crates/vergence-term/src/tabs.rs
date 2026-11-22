//! The window's tabs and panes: opening, splitting, closing and showing
//! them, moving tabs and sections between windows of the process, the
//! server's tab events, and where each pane is. The window itself is in
//! main.rs.

use std::sync::atomic::Ordering;
use std::time::Instant;

use vergence_scene::Rect;
use vergence_ui::Window;
use vergence_ui::bar::{self, Layout, Part};

use crate::panes::{Axis, Divider, Node};
use crate::session::{Session, Tab, lock};
use crate::{NEXT_PANE, NEXT_TAB, Terminal, program};

impl Terminal {
    /// Tell the program what this window keeps (its token, its tabs' working
    /// directories), for the state file (program.rs).
    pub(crate) fn keep(&mut self) {
        let tabs =
            self.tabs.iter().filter_map(|t| t.session(t.active)).filter_map(|s| program::cwd_of(s.pty.pid())).collect();
        self.program.borrow_mut().update(self.key, program::WindowState { token: self.token.clone(), tabs });
    }

    /// Another window of this process: sharing its program, opened for a
    /// later start's request or to restore one kept from the last run.
    fn window_for(
        &self,
        present: String,
        start_dirs: Vec<String>,
        start_cwd: Option<String>,
        run: Option<Vec<String>>,
    ) -> Terminal {
        let mut t = Terminal::new();
        t.program = self.program.clone();
        t.key = t.program.borrow_mut().add();
        t.inbox = self.inbox.clone();
        t.moves = self.moves.clone();
        t.wids = self.wids.clone();
        t.open_with = start_dirs.len().max(1);
        t.present = present;
        t.start_dirs = start_dirs;
        t.start_cwd = start_cwd;
        t.run = run;
        t
    }

    /// Windows asked for by later starts, and windows from the last run still
    /// to open: whichever window hears first opens them.
    pub(crate) fn open_asked(&mut self, window: &mut Window) {
        let asked: Vec<program::OpenRequest> =
            std::mem::take(&mut *self.inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for r in asked {
            let run = (!r.run.is_empty()).then_some(r.run);
            let cwd = (!r.cwd.is_empty()).then_some(r.cwd);
            window.open_window(Box::new(self.window_for(String::new(), Vec::new(), cwd, run)));
        }
        let restore = std::mem::take(&mut self.program.borrow_mut().restore);
        for w in restore {
            window.open_window(Box::new(self.window_for(w.token, w.tabs, None, None)));
        }
    }

    /// A window of its own for `tab` (taken from this one), asking to be at `place`.
    pub(crate) fn tab_to_window(&mut self, tab: Tab, place: Option<vergence_ui::BarPlace>, window: &mut Window) {
        let mut t = self.window_for(String::new(), Vec::new(), None, None);
        t.moved = vec![tab];
        t.ask = place;
        // As large as the window it came from (Dave, 2026-11-12).
        t.ask_size =
            (self.width > 0.0 && self.height > 0.0).then(|| (self.width.round() as u32, self.height.round() as u32));
        window.open_window(Box::new(t));
    }

    /// Takes the tab with this id out of the window (the last one stays: a
    /// window keeps a tab).
    pub(crate) fn take_tab(&mut self, id: u32, window: &mut Window) -> Option<Tab> {
        if self.tabs.len() < 2 {
            return None;
        }
        let i = self.tabs.iter().position(|t| t.id == id)?;
        let front = self.tabs.get(self.shown).map(|t| t.id);
        let tab = self.tabs.remove(i);
        self.shown =
            front.and_then(|f| self.tabs.iter().position(|t| t.id == f)).unwrap_or_else(|| i.min(self.tabs.len() - 1));
        self.held = None;
        self.retitle(window);
        self.keep();
        window.redraw();
        Some(tab)
    }

    /// The active section (pane) of the front tab, out of its tab: its
    /// terminal keeps running. None if it is the tab's only one.
    pub(crate) fn take_pane(&mut self, window: &mut Window) -> Option<Tab> {
        let tab = self.tabs.get_mut(self.shown)?;
        if tab.panes.len() < 2 {
            return None;
        }
        let id = tab.active;
        let leaves = tab.tree.leaves();
        if !tab.tree.remove(id) {
            return None;
        }
        let i = tab.panes.iter().position(|(own, _)| *own == id)?;
        let (_, session) = tab.panes.remove(i);
        let at = leaves.iter().position(|l| *l == id).unwrap_or(0);
        tab.active = leaves[if at > 0 { at - 1 } else { 1 }];
        self.held = None;
        window.redraw();
        Some(Tab {
            id: NEXT_TAB.fetch_add(1, Ordering::SeqCst),
            tree: Node::Leaf(id),
            panes: vec![(id, session)],
            active: id,
        })
    }

    /// Tabs other windows of the process sent here (a tab joined to this window).
    pub(crate) fn take_moves(&mut self, window: &mut Window) {
        let mine: Vec<(u64, Tab, usize)> = {
            let mut moves = self.moves.borrow_mut();
            let (mine, rest): (Vec<_>, Vec<_>) = moves.drain(..).partition(|(k, _, _)| *k == self.key);
            *moves = rest;
            mine
        };
        for (_, tab, to) in mine {
            let to = to.min(self.tabs.len());
            self.tabs.insert(to, tab);
            self.shown = to;
            self.fitted = (0.0, 0.0, 0.0, 0.0);
            self.retitle(window);
            self.keep();
            window.redraw();
        }
    }

    fn layout(&self) -> Layout {
        Layout::new(self.width, self.tabs.len(), self.shown, &self.settings.left, &self.settings.right)
    }

    /// What of our own bar is at a point; nothing when the server draws the bar.
    pub(crate) fn bar_at(&self, x: f32, y: f32) -> Option<Part> {
        if self.own_bar { self.layout().at(x, y) } else { None }
    }

    /// The room under the bar, which the front tab's panes share.
    pub(crate) fn room(&self) -> Rect {
        Rect::new(0.0, self.bar_h, self.width, (self.height - self.bar_h).max(0.0))
    }

    /// Where the front tab's panes and the lines between them are.
    pub(crate) fn areas(&self) -> (Vec<(u64, Rect)>, Vec<Divider>) {
        self.tabs.get(self.shown).map(|tab| tab.tree.layout(self.room())).unwrap_or_default()
    }

    /// The pane under a point, and its room.
    pub(crate) fn pane_at(&self, x: f32, y: f32) -> Option<(u64, Rect)> {
        self.areas().0.into_iter().find(|(_, r)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
    }

    /// The front tab's active pane, and its room.
    pub(crate) fn active(&self) -> Option<(u64, Rect)> {
        let active = self.tabs.get(self.shown)?.active;
        self.areas().0.into_iter().find(|(id, _)| *id == active)
    }

    /// A terminal of the front tab.
    pub(crate) fn session(&self, id: u64) -> Option<&Session> {
        self.tabs.get(self.shown)?.session(id)
    }

    pub(crate) fn session_mut(&mut self, id: u64) -> Option<&mut Session> {
        self.tabs.get_mut(self.shown)?.session_mut(id)
    }

    /// The terminal the keys go to.
    pub(crate) fn front(&self) -> Option<&Session> {
        let tab = self.tabs.get(self.shown)?;
        tab.session(tab.active)
    }

    pub(crate) fn front_mut(&mut self) -> Option<&mut Session> {
        let tab = self.tabs.get_mut(self.shown)?;
        tab.session_mut(tab.active)
    }

    /// A cell's width and height, once the fonts are known.
    pub(crate) fn cell(&self) -> Option<(f32, f32)> {
        self.metrics.as_ref().map(|mt| (mt.cell_w, mt.cell_h))
    }

    /// Starts a terminal and gives it a number.
    pub(crate) fn start(&mut self, cols: usize, rows: usize) -> Option<(u64, Session)> {
        let waker = self.waker.clone()?;
        // A restored tab starts where its shell stood; then where the request said.
        let cwd = if self.start_dirs.is_empty() { self.start_cwd.clone() } else { Some(self.start_dirs.remove(0)) };
        // `-e`'s program runs in the first terminal only; the others get the shell.
        match Session::start(cols, rows, waker, self.run.take(), cwd.as_deref()) {
            Ok(session) => Some((NEXT_PANE.fetch_add(1, Ordering::SeqCst), session)),
            Err(e) => {
                eprintln!("vergence-term: could not start the program: {e}");
                None
            }
        }
    }

    /// A new window of this terminal, with a shell of its own.
    pub(crate) fn new_window(&mut self, window: &mut Window) {
        window.open_window(Box::new(self.window_for(String::new(), Vec::new(), None, None)));
    }

    pub(crate) fn new_tab(&mut self, window: &mut Window) {
        if let Some((id, session)) = self.start(self.fits.0, self.fits.1) {
            self.tabs.push(Tab {
                id: NEXT_TAB.fetch_add(1, Ordering::SeqCst),
                tree: Node::Leaf(id),
                panes: vec![(id, session)],
                active: id,
            });
            self.shown = self.tabs.len() - 1;
            self.retitle(window);
        }
        self.keep();
        window.redraw();
    }

    pub(crate) fn split(&mut self, axis: Axis, window: &mut Window) {
        self.split_active(axis);
        self.retitle(window);
        window.redraw();
    }

    pub(crate) fn split_active(&mut self, axis: Axis) {
        let Some(active) = self.tabs.get(self.shown).map(|tab| tab.active) else { return };
        // The new terminal starts at the size of the one it divides; the next frame fits both.
        let size = self.session(active).map(|s| {
            let s = lock(&s.screen);
            (s.grid.cols, s.grid.rows)
        });
        let Some((id, session)) = self.start(size.map_or(self.fits.0, |s| s.0), size.map_or(self.fits.1, |s| s.1))
        else {
            return;
        };
        let tab = &mut self.tabs[self.shown];
        if tab.tree.split(active, axis, id) {
            tab.panes.push((id, session));
            tab.active = id;
        }
    }

    /// Closes a tab (its programs are ended); the window with the last one.
    pub(crate) fn close_tab(&mut self, index: usize, window: &mut Window) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        self.held = None;
        if !self.tabs.is_empty() {
            self.keep();
        }
        if self.tabs.is_empty() {
            window.close();
            return;
        }
        if self.shown > index || self.shown >= self.tabs.len() {
            self.shown = self.shown.saturating_sub(1);
        }
        self.retitle(window);
        window.redraw();
    }

    /// Closes a pane (its program is ended): its neighbor gets its room.
    /// The tab goes with its last pane.
    pub(crate) fn close_pane(&mut self, index: usize, id: u64, window: &mut Window) {
        let Some(tab) = self.tabs.get_mut(index) else { return };
        if tab.panes.len() <= 1 {
            return self.close_tab(index, window);
        }
        let leaves = tab.tree.leaves();
        if !tab.tree.remove(id) {
            return;
        }
        tab.panes.retain(|(own, _)| *own != id);
        if tab.active == id {
            // The keys go to the pane that came before it, or the first.
            let at = leaves.iter().position(|l| *l == id).unwrap_or(0);
            tab.active = leaves[if at > 0 { at - 1 } else { 1 }];
        }
        self.held = None;
        self.retitle(window);
        window.redraw();
    }

    pub(crate) fn close_active(&mut self, window: &mut Window) {
        if let Some(active) = self.tabs.get(self.shown).map(|tab| tab.active) {
            self.close_pane(self.shown, active, window);
        }
    }

    pub(crate) fn show_tab(&mut self, index: usize, window: &mut Window) {
        if index < self.tabs.len() && index != self.shown {
            if self.measuring {
                self.asked = Some((index, self.frame_now, Instant::now()));
                eprintln!(
                    "press for tab {index} at {:.3}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0.0, |d| d.as_secs_f64() % 1000.0)
                );
            }
            self.shown = index;
            self.held = None;
            self.retitle(window);
            window.redraw();
        }
    }

    /// The window is called what its front tab is.
    pub(crate) fn retitle(&self, window: &mut Window) {
        if let Some(tab) = self.tabs.get(self.shown) {
            window.set_title(&tab.label());
        }
    }

    pub(crate) fn on_tab(&mut self, event: vergence_ui::TabEvent, window: &mut Window) {
        use vergence_ui::TabEvent;
        let index = |id: u32| self.tabs.iter().position(|t| t.id == id);
        match event {
            TabEvent::Chosen(id) => {
                if let Some(i) = index(id) {
                    self.show_tab(i, window);
                }
            }
            TabEvent::Close(id) => {
                if let Some(i) = index(id) {
                    self.close_tab(i, window);
                }
            }
            TabEvent::CloseOthers(id) => {
                while let Some(i) = self.tabs.iter().position(|t| t.id != id) {
                    self.close_tab(i, window);
                }
            }
            TabEvent::New => self.new_tab(window),
            TabEvent::Moved { tab, to } => {
                if let Some(from) = index(tab) {
                    let front = self.tabs.get(self.shown).map(|t| t.id);
                    bar::reorder(&mut self.tabs, from, to as usize);
                    self.keep();
                    if let Some(f) = front.and_then(|f| self.tabs.iter().position(|t| t.id == f)) {
                        self.shown = f;
                    }
                    window.redraw();
                }
            }
            // Dragged out (or "Move to a new window"): a window of its own, where it was dropped.
            TabEvent::Out { tab, place } => {
                if let Some(t) = self.take_tab(tab, window) {
                    self.tab_to_window(t, Some(place), window);
                }
            }
            // Dropped on another window of this process: it goes there.
            TabEvent::Join { tab, into, to } => {
                let target = self.wids.borrow().iter().find(|(w, _)| *w == into).map(|(_, k)| *k);
                // A window's only tab may go too: the window closes behind it.
                let last = || self.tabs.len() == 1 && self.tabs[0].id == tab;
                let take = if target.is_some() && last() {
                    self.held = None;
                    window.close();
                    self.tabs.pop()
                } else if target.is_some() {
                    self.take_tab(tab, window)
                } else {
                    None
                };
                if let Some(key) = target
                    && let Some(t) = take
                {
                    self.moves.borrow_mut().push((key, t, to as usize));
                    if let Some(w) = &self.waker {
                        w.wake();
                    }
                }
            }
            // The terminal adds no entries of its own to a tab's menu yet.
            TabEvent::Menu { .. } => {}
        }
    }
}
