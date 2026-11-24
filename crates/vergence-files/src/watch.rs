//! The watch: what FileView's window looks at when woken (the preview's keys,
//! searches, counted sizes, thumbnails, the settings, folders handed over,
//! tabs moved here, a disk image mounted, a job's progress) and on its timers
//! (a folder changed on disk, the drive list, rename after a pause). The
//! window itself is in main.rs.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use vergence_ui::Modifiers;
use vergence_widgets::files;
use vergence_widgets::{Input, Program, Window};

use crate::{RENAME_TIMER, Showing, Viewer, desktop_text, home};

/// How often the folder shown is looked at for changes.
pub(crate) const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(2);
pub(crate) const WATCH_TIMER: u64 = 1;

impl Viewer {
    pub(crate) fn on_woken(&mut self, window: &mut Window) -> bool {
        let mut changed = false;
        // The preview's keys (the arrows, Enter) as if pressed here; closed, let go of.
        if let Some(l) = self.peek.clone() {
            let keys = std::mem::take(&mut l.borrow_mut().keys);
            for sym in keys {
                let key = Input::Key { code: 0,
                    sym,
                    text: None,
                    pressed: true,
                    repeat: false,
                    composing: false,
                    modifiers: Modifiers::default(),
                };
                changed |= self.key_first(&key, window).is_some();
            }
            if !l.borrow().open && self.peek.as_ref().is_some_and(|p| Rc::ptr_eq(p, &l)) {
                self.peek = None;
                changed = true;
            }
        }
        if let Some(s) = &self.search
            && (s.count() != self.search_entries.len() || s.done() != self.search_done)
        {
            self.search_entries = s.found();
            self.search_done = s.done();
            changed = true;
        }
        // Folders counted: their sizes into the entries (sorting by size uses
        // them). A folder counted again after it changed counts too.
        for t in &mut self.tabs {
            changed |= t.put_sizes();
        }
        let n = self.sizes.lock().unwrap_or_else(|x| x.into_inner()).len();
        if n != self.sizes_seen {
            // A count that came to no size (a folder that can't be read) shows as such.
            self.sizes_seen = n;
            changed = true;
        }
        let made = self.thumbs.made();
        if made != self.thumbs_made {
            self.thumbs_made = made;
            changed = true;
        }
        let (text, stamp) = desktop_text();
        if stamp != self.stamp {
            (self.desktop, self.stamp) = (text, stamp);
            changed = true;
        }
        // The other windows kept from the last run.
        let kept = std::mem::take(&mut self.shared.borrow_mut().kept);
        for snap in kept {
            window.open_window(vergence_widgets::window(Viewer::from_snap(self.shared.clone(), &snap)));
        }
        let asked: Vec<Vec<PathBuf>> =
            std::mem::take(&mut *self.shared.borrow().inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for paths in asked {
            // The Trash window started again: only forward.
            if self.trash_window {
                window.activate();
                continue;
            }
            // For the window last used, if it is another one: it is woken to take them.
            let other = {
                let s = self.shared.borrow();
                s.last_used
                    .filter(|u| Some(*u) != self.me)
                    .and_then(|u| s.windows.iter().find(|(id, _)| *id == u).map(|(id, w)| (*id, w.clone())))
            };
            match other {
                Some((id, w)) => {
                    self.shared.borrow_mut().opens.push((id, paths));
                    w.wake();
                }
                None => {
                    self.open_paths(&paths);
                    // Handed over by a start the person just made: forward (the server judges).
                    window.activate();
                    changed = true;
                }
            }
        }
        // Folders handed over for this window, and tabs dropped on it.
        let (opens, moved) = {
            let mut s = self.shared.borrow_mut();
            let (mine, rest): (Vec<_>, Vec<_>) =
                std::mem::take(&mut s.opens).into_iter().partition(|(id, _)| Some(*id) == self.me);
            s.opens = rest;
            let (tabs, others): (Vec<_>, Vec<_>) =
                std::mem::take(&mut s.transfers).into_iter().partition(|(into, _, _)| Some(*into) == self.me);
            s.transfers = others;
            (mine, tabs)
        };
        for (_, paths) in opens {
            self.open_paths(&paths);
            window.activate();
            changed = true;
        }
        for (_, to, t) in moved {
            self.take_tab(t, to as usize);
            changed = true;
        }
        if changed {
            self.keep();
        }
        // Folders for windows of their own.
        for p in std::mem::take(&mut self.pending) {
            let mut w = Viewer::new(self.shared.clone());
            w.open_paths(&[p]);
            window.open_window(vergence_widgets::window(w));
        }
        // A disk image mounted: in a tab of its own; or why not.
        let answered = self
            .mounting
            .as_ref()
            .and_then(|(n, a)| a.lock().unwrap_or_else(|e| e.into_inner()).take().map(|r| (n.clone(), r)));
        if let Some((name, r)) = answered {
            self.mounting = None;
            match r {
                // Mounted, and shown under Locations: no tab of its own, since a new tab
                // never appears unless the user asks for it (see docs/files-milestone-3.md).
                Ok(_) => self.notice = None,
                Err(why) => self.notice = Some(format!("“{name}” could not be mounted: {why}")),
            }
            changed = true;
        }
        // Extract To…'s folder chosen.
        if self.extract_to_answered() {
            changed = true;
        }
        // A copy or move running (its ticks) or ended.
        if self.job.is_some() {
            self.job_ended();
            changed = true;
        }
        // A picture decoded: drawn if it is the cursor's.
        if self.cursor_entry().is_some_and(|e| self.picture_of(&e.path).is_some()) {
            changed = true;
        }
        window.wake_after(WATCH_TIMER, WATCH_EVERY);
        changed
    }

    pub(crate) fn on_timer(&mut self, id: u64, window: &mut Window) -> bool {
        if id == RENAME_TIMER {
            // Still the one chosen, and no second click came: its name a field.
            let Some((p, at)) = self.rename_after.take() else { return false };
            let still = self.front_tab().is_some_and(|t| t.chosen == [p.clone()]);
            let latest = self.last_click.as_ref().is_some_and(|(t, q)| *q == p && *t >= at);
            if still && latest && self.naming.is_none() && at.elapsed() >= self.double_gap() {
                self.start_rename(p);
                return true;
            }
            return false;
        }
        if id != WATCH_TIMER {
            return false;
        }
        window.wake_after(WATCH_TIMER, WATCH_EVERY);
        // Where the tabs are scrolled, kept (at most every WATCH_EVERY).
        self.keep();
        // A new drive list from the server: the sidebar's Locations again
        // (and a tab on a drive gone goes home).
        let seen = files::drives_seen();
        if seen != self.drives_seen {
            self.drives_seen = seen;
            *self.ejectable.borrow_mut() = None;
            let drives: Vec<PathBuf> = files::mounted_disks().into_iter().chain(files::network_mounts()).collect();
            // A drive listed before and not now: tabs on it go home.
            let went: Vec<PathBuf> = self.drives_last.iter().filter(|d| !drives.contains(d)).cloned().collect();
            for t in self.tabs.iter_mut().filter(|t| t.dir().is_some_and(|d| went.iter().any(|w| d.starts_with(w)))) {
                t.go(Showing::Folder(home()));
            }
            self.drives_last = drives;
            return true;
        }
        // A drive ejected or pulled out leaves the server's list, and its tabs
        // go home above. One under /media the list does not show as mounted
        // yet is not looked into again here: a look has FreeBSD's automounter
        // mount it (an ejected stick came back that way). Not sent home: a
        // stick just clicked is mounting, and the server sees it at its next
        // look (otherwise clicking a stick sends the user home; see docs/drives-milestone-1.md).
        let mounted: Vec<PathBuf> =
            files::drive_list().into_iter().filter(|d| d.mounted).map(|d| PathBuf::from(d.path)).collect();
        let off = |d: &Path| d.starts_with("/media/") && !mounted.iter().any(|m| d.starts_with(m));
        // The folder changed on disk: read again.
        let Some(t) = self.front_mut() else { return false };
        if t.dir().is_some_and(off) {
            return false;
        }
        let now = t.dir().and_then(|d| std::fs::metadata(d).and_then(|m| m.modified()).ok());
        if t.dir().is_some() && now != t.stamp {
            t.read();
            return true;
        }
        false
    }
}
