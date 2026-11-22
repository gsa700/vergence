//! FileView's keys and choosing: the keyboard (`on_key_first`), clicks, the
//! arrows moving the choice. The window itself is in main.rs.

use std::path::PathBuf;
use std::time::Instant;

use vergence_widgets::files::Entry;
use vergence_widgets::{Input, Window};

use crate::{PANE_W, ViewKind, Viewer, cmd};

impl Viewer {
    pub(crate) fn cursor_entry(&self) -> Option<Entry> {
        let t = self.front_tab()?;
        let c = t.cursor.as_ref()?;
        t.entries.iter().find(|e| &e.path == c).cloned()
    }

    /// A click on visible row `k`, with the modifiers of its press.
    pub(crate) fn click(&mut self, k: usize) {
        let visible: Vec<PathBuf> = self.visible().iter().map(|e| e.path.clone()).collect();
        let Some(p) = visible.get(k).cloned() else { return };
        let now = Instant::now();
        let gap = self.double_gap();
        let double = self.last_click.as_ref().is_some_and(|(t, q)| *q == p && now.duration_since(*t) < gap);
        self.last_click = Some((now, p.clone()));
        let mods = self.mods;
        let Some(t) = self.front_mut() else { return };
        if mods.ctrl {
            match t.chosen.iter().position(|c| *c == p) {
                Some(i) => {
                    t.chosen.remove(i);
                }
                None => t.chosen.push(p.clone()),
            }
            t.cursor = Some(p);
        } else if mods.shift {
            let from = t.cursor.as_ref().and_then(|c| visible.iter().position(|v| v == c)).unwrap_or(k);
            let (a, b) = (from.min(k), from.max(k));
            t.chosen = visible[a..=b].to_vec();
        } else {
            t.chosen = vec![p.clone()];
            t.cursor = Some(p);
            if double {
                self.last_click = None;
                self.rename_after = None;
                self.open_chosen();
            }
        }
    }

    /// The arrows: the cursor moves by `by` among the visible rows (Shift: the choice grows).
    pub(crate) fn move_cursor(&mut self, by: isize) {
        let visible: Vec<PathBuf> = self.visible().iter().map(|e| e.path.clone()).collect();
        if visible.is_empty() {
            return;
        }
        let shift = self.mods.shift;
        let Some(t) = self.front_mut() else { return };
        let at = t.cursor.as_ref().and_then(|c| visible.iter().position(|v| v == c));
        let k = match at {
            None if by > 0 => 0,
            None => visible.len() - 1,
            Some(i) => (i as isize + by).clamp(0, visible.len() as isize - 1) as usize,
        };
        let p = visible[k].clone();
        if shift {
            if !t.chosen.contains(&p) {
                t.chosen.push(p.clone());
            }
        } else {
            t.chosen = vec![p.clone()];
        }
        t.cursor = Some(p);
        t.turn += 1;
    }

    /// Icons per row in the icon view, for Up and Down.
    fn per_row(&self) -> isize {
        let room = self.width - 160.0 - 24.0 - if self.pane { PANE_W + 8.0 } else { 0.0 };
        ((room + 6.0) / 106.0).floor().max(1.0) as isize
    }

    /// This item alone chosen, the arrows starting from it.
    pub(crate) fn choose_only(&mut self, p: PathBuf) {
        if let Some(t) = self.front_mut() {
            t.chosen = vec![p.clone()];
            t.cursor = Some(p);
            t.turn += 1;
        }
    }

    pub(crate) fn chosen_paths(&self) -> Vec<PathBuf> {
        self.front_tab().map(|t| t.chosen.clone()).unwrap_or_default()
    }

    pub(crate) fn on_key_first(&mut self, input: &Input, window: &mut Window) -> Option<bool> {
        const UP: u32 = 0xff52;
        const DOWN: u32 = 0xff54;
        const LEFT: u32 = 0xff51;
        const RIGHT: u32 = 0xff53;
        const RETURN: u32 = 0xff0d;
        const ESCAPE: u32 = 0xff1b;
        const BACKSPACE: u32 = 0xff08;
        const SPACE: u32 = 0x20;
        const DELETE: u32 = 0xffff;
        const F2: u32 = 0xffbf;
        let Input::Key { sym, pressed: true, modifiers: m, .. } = input else { return None };
        self.mods = *m;
        // A name being typed: its field has the keys (Escape cancels it).
        if self.naming.is_some() {
            if *sym == ESCAPE {
                self.naming = None;
                window.redraw();
                return Some(true);
            }
            return None;
        }
        // The path being typed: Escape lets go; the field has the other keys.
        if self.path_edit.is_some() {
            if *sym == ESCAPE {
                self.path_edit = None;
                window.redraw();
                return Some(true);
            }
            return None;
        }
        let typing = !self.narrow.is_empty();
        match *sym {
            s if m.ctrl && (s == 0x6c || s == 0x4c) && !self.trash_window => {
                self.path_edit =
                    Some(self.front_tab().and_then(|t| t.dir()).map(|d| d.display().to_string()).unwrap_or_default());
            }
            ESCAPE if self.ask.is_some() => self.ask = None,
            DELETE if m.shift && !typing => self.run(cmd::DELETE, window),
            DELETE if !typing => self.run(cmd::TRASH, window),
            F2 => self.run(cmd::RENAME, window),
            s if m.ctrl && m.shift && (s == 0x6e || s == 0x4e) => self.run(cmd::NEW_FOLDER, window),
            s if m.ctrl && (s == 0x78 || s == 0x58) && !typing => self.run(cmd::CUT, window),
            s if m.ctrl && (s == 0x63 || s == 0x43) && !typing => self.run(cmd::COPY, window),
            s if m.ctrl && (s == 0x76 || s == 0x56) && !typing => self.run(cmd::PASTE, window),
            s if m.ctrl && (s == 0x64 || s == 0x44) => self.run(cmd::DUPLICATE, window),
            s if m.ctrl && (s == 0x7a || s == 0x5a) && !typing => self.run(cmd::UNDO, window),
            UP if m.alt => self.run(cmd::UP, window),
            LEFT if m.alt => self.run(cmd::BACK, window),
            RIGHT if m.alt => self.run(cmd::FORWARD, window),
            DOWN if m.alt || m.ctrl => self.run(cmd::OPEN, window),
            UP => {
                self.move_cursor(if self.shown_view() == ViewKind::Icons { -self.per_row() } else { -1 });
                self.want_picture();
            }
            DOWN => {
                self.move_cursor(if self.shown_view() == ViewKind::Icons { self.per_row() } else { 1 });
                self.want_picture();
            }
            LEFT if self.shown_view() == ViewKind::Icons || self.peek_open() => {
                self.move_cursor(-1);
                self.want_picture();
            }
            RIGHT if self.shown_view() == ViewKind::Icons || self.peek_open() => {
                self.move_cursor(1);
                self.want_picture();
            }
            RETURN => {
                self.close_peek();
                self.run(cmd::OPEN, window)
            }
            ESCAPE if self.peek_open() => self.close_peek(),
            ESCAPE if typing => self.narrow.clear(),
            SPACE if !typing => self.run(cmd::PREVIEW, window),
            BACKSPACE if !typing => self.run(cmd::UP, window),
            s if m.ctrl && (s == 0x61 || s == 0x41) && !typing => self.run(cmd::SELECT_ALL, window),
            s if m.ctrl && !m.shift && (s == 0x6e || s == 0x4e) => self.run(cmd::NEW_WINDOW, window),
            s if m.ctrl && (s == 0x77 || s == 0x57) => self.run(cmd::CLOSE_TAB, window),
            s if m.ctrl && (s == 0x74 || s == 0x54) => self.run(cmd::NEW_TAB, window),
            0xff56 if m.ctrl => self.run(cmd::NEXT_TAB, window),
            0xff55 if m.ctrl => self.run(cmd::PREV_TAB, window),
            s if m.ctrl && (s == 0x68 || s == 0x48) => self.run(cmd::HIDDEN, window),
            s if m.ctrl && (s == 0x69 || s == 0x49) && !typing => self.run(cmd::INFO, window),
            s if m.ctrl && s == 0x2c => self.run(cmd::SETTINGS, window),
            s if m.ctrl && (s == 0x31) => self.run(cmd::LIST, window),
            s if m.ctrl && (s == 0x32) => self.run(cmd::ICONS, window),
            s if m.ctrl && (s == 0x72 || s == 0x52) => self.run(cmd::RELOAD, window),
            _ => return None,
        }
        window.redraw();
        Some(true)
    }
}
