// A new window and the keyboard (2026-11-14; found running windows milestone
// 1, step 1 on kepler, and by Dave the day before: a window opened from the
// launcher waited for a click before it could be typed in).
//
// A new window takes the keyboard when it is one the person just asked for:
//
// 1. nothing has the keyboard (there is nothing to take it from);
// 2. it is the focused program's own (a new terminal window, a dialog, a tab
//    torn off): the person is working in that program;
// 3. its application was just started from the dock, the launcher or session
//    restore (shelf.rs `launched`, the first window of each start, within 15
//    seconds), and nothing has been typed into another
//    window since: a slow program that opens while the person types
//    elsewhere (a password, say) does not take the keys.
//
// Otherwise it opens behind the focused window, and its dock icon asks for
// attention (shelf.rs `attention`) until it has had the keyboard.

use crate::{Server, win::Win};

/// How long after being started an application's first window still counts
/// as asked for (the dock's starting mark lasts as long).
const LAUNCHED: std::time::Duration = std::time::Duration::from_secs(15);

impl Server {
    /// A window has just been put on the screen for the first time.
    pub fn window_opened(&mut self, w: &Win) {
        // Window memory had it snapped: into its part first (snap.rs).
        self.snap_recalled(w);
        // A dialog opens over its parent (dialogs.rs).
        self.place_dialog(w);
        let typed_at = self.input.typed_at;
        let focused = self.keyboard_window();
        let same = match &focused {
            Some(f) => self.same_program(f, w),
            None => false,
        };
        let items = self.shelf_items();
        let key = items.iter().find(|(_, ws)| ws.contains(w)).map(|(i, _)| i.key.clone());
        let started_at = key.as_ref().and_then(|key| self.shelf.launched.iter().find(|(k, since)| k == key && since.elapsed() < LAUNCHED).map(|(_, since)| *since));
        let take = takes_keyboard(focused.is_some(), same, started_at, typed_at);
        // The launch is used up by its first window.
        if let Some(key) = key {
            self.shelf.launched.retain(|(k, _)| *k != key);
        }
        if take {
            self.focus_window(w, smithay::utils::SERIAL_COUNTER.next_serial());
        } else {
            tracing::info!("opened behind the focused window, asking for attention: {:?}", self.window_title(w));
            self.shelf.attention.push(w.clone());
        }
    }

    /// Two windows of one program: the same identity (class, else instance), as
    /// the dock groups them.
    pub fn same_program(&mut self, a: &Win, b: &Win) -> bool {
        let key = |id: vergence_profile::windows::WindowId| if id.class.is_empty() { id.instance } else { id.class };
        match (self.window_identity(a), self.window_identity(b)) {
            (Some(x), Some(y)) => {
                let (x, y) = (key(x), key(y));
                !x.is_empty() && x == y
            }
            _ => false,
        }
    }
}

/// The rule, apart from the windows: does a new window take the keyboard?
/// `started_at`: when its application was started by the person, if it was
/// and is still starting; `typed_at`: when a key last went into a window.
fn takes_keyboard(something_focused: bool, same_program: bool, started_at: Option<std::time::Instant>, typed_at: Option<std::time::Instant>) -> bool {
    !something_focused || same_program || started_at.is_some_and(|since| typed_at.is_none_or(|t| t < since))
}

#[cfg(test)]
mod tests {
    use super::takes_keyboard;
    use std::time::{Duration, Instant};

    #[test]
    fn with_nothing_focused_a_new_window_takes_the_keyboard() {
        assert!(takes_keyboard(false, false, None, Some(Instant::now())));
    }

    #[test]
    fn the_focused_programs_own_new_window_takes_it() {
        assert!(takes_keyboard(true, true, None, Some(Instant::now())));
    }

    #[test]
    fn a_window_appearing_by_itself_does_not_take_it() {
        assert!(!takes_keyboard(true, false, None, None));
    }

    #[test]
    fn a_just_started_application_takes_it_unless_typing_went_elsewhere_since() {
        let started = Instant::now();
        assert!(takes_keyboard(true, false, Some(started), None));
        assert!(takes_keyboard(true, false, Some(started), Some(started - Duration::from_secs(1))));
        // A password typed into another window while the program loaded keeps the keyboard there.
        assert!(!takes_keyboard(true, false, Some(started), Some(started + Duration::from_secs(1))));
    }
}
