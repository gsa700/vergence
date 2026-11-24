//! Edit's window kept for next time (keep.rs writes and reads the file):
//! what a window keeps, writing it soon after changes, a window put back
//! as it was. The window itself is in main.rs.

use vergence_widgets::{TextDoc, Window};

use crate::{DocTab, EditWindow, KEEP_AFTER, KEEP_TIMER, SharedRef, keep, stamp};

impl EditWindow {
    /// What this window keeps for next time (keep.rs).
    pub(crate) fn snap(&self) -> keep::WindowSnap {
        let tabs = self
            .tabs
            .iter()
            .filter(|t| t.path.is_some() || t.modified())
            .map(|t| {
                let d = t.doc.borrow();
                let c = d.caret();
                keep::TabSnap {
                    path: t.path.clone(),
                    untitled: t.untitled,
                    text: t.modified().then(|| d.text()),
                    caret: (c.line, c.col),
                }
            })
            .collect::<Vec<_>>();
        let front =
            self.tabs.iter().filter(|t| t.path.is_some() || t.modified()).position(|t| t.id == self.front).unwrap_or(0);
        keep::WindowSnap { token: self.token.clone(), front, tabs }
    }

    /// Writes what every window keeps, this one's as it is now.
    pub(crate) fn keep_now(&mut self) {
        self.keep_pending = false;
        let Some(me) = self.me else { return };
        let snap = self.snap();
        let mut s = self.shared.borrow_mut();
        match s.snaps.iter_mut().find(|(id, _)| *id == me) {
            Some(slot) => slot.1 = snap,
            None => s.snaps.push((me, snap)),
        }
        let all: Vec<keep::WindowSnap> =
            s.snaps.iter().map(|(_, w)| w.clone()).filter(|w| !w.tabs.is_empty() || !w.token.is_empty()).collect();
        keep::save(&all);
    }

    /// What is kept is written a second from now (once, however many changes).
    pub(crate) fn keep_soon(&mut self, window: &mut Window) {
        if !self.keep_pending {
            self.keep_pending = true;
            window.wake_at(KEEP_TIMER, std::time::Instant::now() + KEEP_AFTER);
        }
    }

    /// A window as it was kept: its token, its files, its unsaved texts.
    pub(crate) fn from_snap(shared: SharedRef, snap: &keep::WindowSnap) -> EditWindow {
        let mut w = EditWindow::new(shared, Vec::new());
        w.tabs.clear();
        w.token = snap.token.clone();
        for t in &snap.tabs {
            let tab = match (&t.text, &t.path) {
                // Unsaved work: its text, marked as not saved.
                (Some(text), path) => {
                    let doc = TextDoc::new(text);
                    doc.borrow_mut().mark_unsaved();
                    let spaces = doc.borrow().buffer().uses_tabs().map(|tabs| !tabs);
                    let lang = crate::tab::language_of(path.as_deref(), &doc);
                    Some(DocTab {
                        id: w.new_tab_id(),
                        doc,
                        path: path.clone(),
                        untitled: t.untitled,
                        read_only: None,
                        bom: false,
                        spaces,
                        disk: path.as_deref().and_then(stamp),
                        changed_on_disk: false,
                        lang,
                        lang_chosen: false,
                    })
                }
                (None, Some(p)) => w.file_tab(p),
                (None, None) => None,
            };
            if let Some(tab) = tab {
                let caret = vergence_widgets::edit::Pos::new(t.caret.0, t.caret.1);
                tab.doc.borrow_mut().set_selection(caret, caret);
                w.tabs.push(tab);
            }
        }
        if w.tabs.is_empty() {
            let t = w.untitled_tab();
            w.tabs.push(t);
        }
        w.front = w.tabs.get(snap.front).or(w.tabs.first()).map_or(0, |t| t.id);
        w.apply_options();
        w
    }
}
