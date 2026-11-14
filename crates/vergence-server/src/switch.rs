// Alt+Tab and Alt+` (docs/windows-milestone-1.md, step 1): switching windows
// by the order they were last used.
//
// The seat (vergence-seat keyboard.rs) decides which keys switch and keeps the
// order of use; this takes a snapshot of the windows when switching begins and
// steps through it, so windows opening or closing meanwhile do not move the
// selection under the hand. The windows are the dock's (shelf.rs
// `shelf_items`): real windows, minimized ones included, no menus or tooltips;
// Alt+` keeps to the dock item the focused window belongs to, so it agrees
// with the dock about what one application is. Its list on the screen is step
// 4; until then the switching works without being seen.

use vergence_seat::keyboard::Switch;

use crate::{Server, win::Win};

/// Switching in progress: the windows in the order of use, and the one stepped to.
pub struct Switching {
    list: Vec<Win>,
    at: usize,
}

impl Switching {
    fn step(&mut self, back: bool) {
        let n = self.list.len();
        self.at = if back { (self.at + n - 1) % n } else { (self.at + 1) % n };
    }
}

impl Server {
    pub fn switch_key(&mut self, s: Switch) {
        match s {
            Switch::Begin { app, back } => {
                let list = self.switch_list(app);
                if list.is_empty() {
                    self.input.seat.switch_ended();
                    self.input.switching = None;
                    return;
                }
                // With a window focused it is first in the order: the first step is
                // the one before it. With none (a click on the desktop), the first.
                let focused = self.keyboard_window();
                let mut sw = Switching { list, at: 0 };
                if focused.is_some_and(|f| sw.list[0] == f) {
                    sw.step(back);
                } else if back {
                    sw.at = sw.list.len() - 1;
                }
                self.input.switching = Some(sw);
            }
            Switch::Step { back } => {
                if let Some(sw) = self.input.switching.as_mut() {
                    sw.step(back);
                }
            }
            Switch::Choose => {
                let Some(sw) = self.input.switching.take() else { return };
                let w = sw.list[sw.at].clone();
                if !smithay::utils::IsAlive::alive(&w) {
                    return;
                }
                tracing::info!("switch: {:?}", self.window_title(&w));
                if self.is_minimized(&w) {
                    self.unminimize(&w);
                } else {
                    self.focus_window(&w, smithay::utils::SERIAL_COUNTER.next_serial());
                }
            }
            Switch::Cancel => self.input.switching = None,
        }
    }

    /// The windows to switch among, the most recently used first; those never
    /// used (restored at the session's start, say) after them, as the dock has them.
    fn switch_list(&mut self, app: bool) -> Vec<Win> {
        let items = self.shelf_items();
        let focused = self.keyboard_window();
        let windows: Vec<Win> = if app {
            let Some(f) = focused else { return Vec::new() };
            match items.into_iter().find(|(_, ws)| ws.contains(&f)) {
                Some((_, ws)) => ws,
                None => return Vec::new(),
            }
        } else {
            items.into_iter().flat_map(|(_, ws)| ws).collect()
        };
        let mut list: Vec<Win> = self.input.seat.order_of_use().iter().filter_map(|&id| self.input.window(id)).filter(|w| windows.contains(w)).collect();
        for w in windows {
            if !list.contains(&w) {
                list.push(w);
            }
        }
        list
    }
}
