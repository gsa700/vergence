//! Touch: a finger on a window is the window's; the first finger anywhere
//! else is the pointer with its left button held (Dave, 2026-11-10).
//!
//! - A finger on a window's content raises it, gives it the keyboard, and
//!   is that window's from its first contact to its last, wherever it
//!   moves (Wayland's rule, and ours for native windows).
//! - The first finger on the dock, a menu, the popup, a notification, a
//!   frame the server draws or the bare desktop acts as the pointer with
//!   its left button held: a tap is a click, a drag a drag. Other fingers
//!   there do nothing. While a menu or the popup is open, every first
//!   finger is the pointer (so a tap outside closes it).
//! - A pointing finger held still ([`STILL`]) until the server's timer
//!   fires ([`Seat::touch_long_press`], after half a second) on the dock or
//!   the desktop is the right button there.
//! - A window that asks to be moved from a finger's press (our toolkit's
//!   bar does) goes with that finger.
//!
//! Where a finger "meant" when it came down a little off a button (the
//! server's REACH) is the server's: it passes the target it decided on.

use crate::{BTN_LEFT, Grab, Menu, Out, Part, Seat, Slot, Target, WinId, keyboard::Mods};

/// How far a long press may wander, in points.
pub const STILL: f64 = 10.0;

#[derive(Debug, Default)]
pub(crate) struct Touch {
    /// The finger acting as the pointer.
    pointing: Option<Slot>,
    /// Where it came down, while it has not wandered: a long press if it stays.
    resting: Option<(f64, f64)>,
    /// Which press this is, for its timer.
    presses: u64,
    /// The fingers on windows, each with its target for good.
    on_windows: Vec<(Slot, Target)>,
    /// A window going with a finger: the finger, the window, where it came down.
    moving: Option<(Slot, WinId, f64, f64)>,
}

impl Touch {
    pub(crate) fn window_gone(&mut self, window: WinId, _out: &mut [Out]) {
        self.on_windows.retain(|(_, t)| t.window() != Some(window));
        if self.moving.is_some_and(|(_, w, ..)| w == window) {
            self.moving = None;
        }
    }

    /// Every finger let go (the panel took them back, or the screens went off).
    pub(crate) fn cancel_into(&mut self, out: &mut Vec<Out>) {
        for (slot, t) in self.on_windows.drain(..) {
            out.push(Out::TouchCancel(t, slot));
        }
        self.pointing = None;
        self.resting = None;
        self.moving = None;
    }
}

impl Seat {
    /// A finger came down at (x, y); `under` is what the server decided it meant.
    pub fn touch_down(&mut self, slot: Slot, x: f64, y: f64, under: Option<Target>, now_ms: u64) -> Vec<Out> {
        // Locked: every finger is the lock's.
        let under = if self.locked { Some(Target::Server(Part::Lock)) } else { under };
        let mut out = Vec::new();
        self.keyboard.super_alone = false;
        let ours = self.menu.is_some() || matches!(under, Some(Target::Server(Part::Note | Part::Card | Part::Drawer | Part::Session)));
        if self.touch.on_windows.iter().any(|(s, _)| *s == slot) || self.touch.pointing == Some(slot) {
            // A slot already down (the panel lost an up): ignore the repeat.
            return out;
        }
        if let Some(t) = under.filter(|t| t.is_content() && !ours) {
            let w = t.window().expect("content has a window");
            out.push(Out::Focus(w));
            self.keyboard.took(w);
            self.touch.on_windows.push((slot, t));
            out.push(Out::TouchDown(t, slot));
            return out;
        }
        if self.touch.pointing.is_some() || !self.touch.on_windows.is_empty() {
            return out;
        }
        self.touch.pointing = Some(slot);
        out.push(Out::PointerTo { x, y });
        out.extend(self.motion(under));
        out.extend(self.button(BTN_LEFT, true, under, Mods::default(), now_ms));
        self.touch.presses += 1;
        self.touch.resting = (!ours && matches!(under, Some(Target::Server(Part::Dock | Part::Desktop | Part::Title(_))))).then_some((x, y));
        out.push(Out::LongPressTimer(self.touch.presses));
        out
    }

    /// A finger moved to (x, y).
    pub fn touch_motion(&mut self, slot: Slot, x: f64, y: f64, under: Option<Target>) -> Vec<Out> {
        // Locked: every finger is the lock's.
        let under = if self.locked { Some(Target::Server(Part::Lock)) } else { under };
        let mut out = Vec::new();
        if self.touch.pointing == Some(slot) {
            if self.touch.resting.is_some_and(|(rx, ry)| (x - rx).abs() > STILL || (y - ry).abs() > STILL) {
                self.touch.resting = None;
            }
            out.push(Out::PointerTo { x, y });
            out.extend(self.motion(under));
        } else if let Some((_, window, dx0, dy0)) = self.touch.moving.filter(|m| m.0 == slot) {
            // The window goes with the finger; its program hears no more of it.
            out.push(Out::FingerMove { window, dx: x - dx0, dy: y - dy0 });
        } else if let Some((_, t)) = self.touch.on_windows.iter().find(|(s, _)| *s == slot) {
            out.push(Out::TouchMotion(*t, slot));
        }
        out
    }

    /// A finger lifted.
    pub fn touch_up(&mut self, slot: Slot, under: Option<Target>, now_ms: u64) -> Vec<Out> {
        let mut out = Vec::new();
        if let Some((_, window, ..)) = self.touch.moving.take_if(|m| m.0 == slot) {
            out.push(Out::FingerMoveEnd(window));
        }
        if self.touch.pointing == Some(slot) {
            self.touch.pointing = None;
            self.touch.resting = None;
            out.extend(self.button(BTN_LEFT, false, under, Mods::default(), now_ms));
            out.push(Out::Unhover);
        } else if let Some(i) = self.touch.on_windows.iter().position(|(s, _)| *s == slot) {
            let (_, t) = self.touch.on_windows.remove(i);
            out.push(Out::TouchUp(t, slot));
        }
        out
    }

    /// The long-press timer `press` fired. What it acts on is what the finger
    /// pressed: a resting finger has not left it.
    pub fn touch_long_press(&mut self, press: u64) -> Vec<Out> {
        let mut out = Vec::new();
        if press != self.touch.presses || self.touch.pointing.is_none() || self.touch.resting.take().is_none() {
            return out;
        }
        // A finger on a title bar began moving its window at once: the bar is what it pressed.
        let under = match self.grab {
            Some(Grab::Move { window, button: BTN_LEFT }) => Some(Target::Server(Part::Title(window))),
            _ => self.pointer.focus.filter(|_| self.pointer.held.contains(&BTN_LEFT)),
        };
        match under {
            Some(Target::Server(Part::Dock)) => {
                // The press the dock was holding is not a click any more: no release goes to it.
                self.pointer.held.retain(|b| *b != BTN_LEFT);
                out.push(Out::DockMenu);
                self.menu = Some(Menu::Menu);
            }
            Some(Target::Server(Part::Desktop)) => {
                out.extend(self.button(BTN_LEFT, false, under, Mods::default(), 0));
                out.push(Out::DesktopMenu);
                self.menu = Some(Menu::Menu);
            }
            // On a title bar: the window's menu. The move the press began ends
            // where it is (the finger rested), and its lifting is no release.
            Some(Target::Server(Part::Title(w))) => {
                if matches!(self.grab, Some(Grab::Move { window, .. }) if window == w) {
                    self.grab = None;
                    out.push(Out::GrabEnd(w));
                }
                self.pointer.held.retain(|b| *b != BTN_LEFT);
                out.push(Out::WindowMenu { window: w, on_button: false });
                self.menu = Some(Menu::Menu);
            }
            _ => return out,
        }
        // The finger has said what it had to: its lifting does nothing more.
        self.touch.pointing = None;
        out
    }

    /// A window asked to be moved from the press of `slot` (the server
    /// matched the request's serial to the finger): it goes with the finger.
    /// False if that finger is not on that window.
    pub fn touch_move_request(&mut self, slot: Slot, window: WinId, x: f64, y: f64) -> bool {
        if self.touch.on_windows.iter().any(|(s, t)| *s == slot && t.window() == Some(window)) {
            self.touch.moving = Some((slot, window, x, y));
            true
        } else {
            false
        }
    }

    /// The panel took its touches back (a palm, a gesture of the system's).
    pub fn touch_cancel(&mut self, under: Option<Target>, now_ms: u64) -> Vec<Out> {
        let mut out = Vec::new();
        if self.touch.pointing.is_some() {
            out.extend(self.button(BTN_LEFT, false, under, Mods::default(), now_ms));
            out.push(Out::Unhover);
        }
        self.touch.cancel_into(&mut out);
        out
    }

    /// Fingers down now: (as the pointer, on windows), for `vctl touch`.
    pub fn touch_counts(&self) -> (usize, usize) {
        (self.touch.pointing.iter().count(), self.touch.on_windows.len())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_util::*;
    use crate::*;

    #[test]
    fn a_finger_on_a_window_stays_its_wherever_it_goes() {
        let mut seat = Seat::new();
        assert_eq!(seat.touch_down(0, 5.0, 5.0, Some(s1()), 0), vec![Out::Focus(W1), Out::TouchDown(s1(), 0)]);
        assert_eq!(seat.touch_motion(0, 500.0, 5.0, Some(s2())), vec![Out::TouchMotion(s1(), 0)]);
        assert_eq!(seat.touch_up(0, Some(s2()), 1), vec![Out::TouchUp(s1(), 0)]);
        assert_eq!(seat.keyboard_focus(), Some(W1));
    }

    #[test]
    fn the_first_finger_elsewhere_is_the_left_button() {
        let mut seat = Seat::new();
        let dock = Target::Server(Part::Dock);
        let out = seat.touch_down(3, 1900.0, 500.0, Some(dock), 0);
        assert_eq!(out, vec![Out::PointerTo { x: 1900.0, y: 500.0 }, Out::Enter(dock), Out::Press(dock, BTN_LEFT), Out::LongPressTimer(1)]);
        // A second finger there does nothing.
        assert!(seat.touch_down(4, 1900.0, 520.0, Some(dock), 1).is_empty());
        let out = seat.touch_up(3, Some(dock), 2);
        assert_eq!(out, vec![Out::Release(dock, BTN_LEFT), Out::Unhover]);
        assert!(seat.touch_up(4, Some(dock), 3).is_empty());
    }

    #[test]
    fn a_tap_outside_an_open_menu_closes_it_even_on_a_window() {
        let mut seat = Seat::new();
        seat.menu_opened(Menu::Menu);
        let out = seat.touch_down(0, 5.0, 5.0, Some(s1()), 0);
        assert!(out.contains(&Out::CloseMenu));
        assert!(!out.iter().any(|o| matches!(o, Out::TouchDown(..))));
        // Its lifting goes nowhere.
        assert_eq!(seat.touch_up(0, Some(s1()), 1), vec![Out::Unhover]);
    }

    #[test]
    fn a_long_still_press_is_the_right_button_on_the_dock_and_desktop() {
        let mut seat = Seat::new();
        let dock = Target::Server(Part::Dock);
        seat.touch_down(0, 1900.0, 500.0, Some(dock), 0);
        seat.touch_motion(0, 1904.0, 503.0, Some(dock));
        assert_eq!(seat.touch_long_press(1), vec![Out::DockMenu]);
        // The lift is not a click on the dock.
        assert!(seat.touch_up(0, Some(dock), 600).is_empty());
        assert_eq!(seat.menu(), Some(Menu::Menu));
        seat.menu_closed();

        let desk = Target::Server(Part::Desktop);
        seat.touch_down(1, 300.0, 300.0, Some(desk), 1000);
        let out = seat.touch_long_press(2);
        assert_eq!(out, vec![Out::Release(desk, BTN_LEFT), Out::DesktopMenu]);
    }

    #[test]
    fn a_long_still_press_on_a_title_bar_is_the_windows_menu_not_a_move() {
        let mut seat = Seat::new();
        let bar = Target::Server(Part::Title(W1));
        let out = seat.touch_down(0, 300.0, 20.0, Some(bar), 0);
        assert!(out.contains(&Out::MoveBegin(W1)));
        let out = seat.touch_long_press(1);
        assert_eq!(out, vec![Out::GrabEnd(W1), Out::WindowMenu { window: W1, on_button: false }]);
        assert_eq!(seat.grab(), None);
        assert_eq!(seat.menu(), Some(Menu::Menu));
        assert!(seat.touch_up(0, Some(bar), 900).iter().all(|o| !matches!(o, Out::Release(..))));
    }

    #[test]
    fn a_finger_that_wanders_or_an_old_timer_is_no_long_press() {
        let mut seat = Seat::new();
        let desk = Target::Server(Part::Desktop);
        seat.touch_down(0, 300.0, 300.0, Some(desk), 0);
        seat.touch_motion(0, 330.0, 300.0, Some(desk));
        assert!(seat.touch_long_press(1).is_empty());
        seat.touch_up(0, Some(desk), 100);
        seat.touch_down(0, 300.0, 300.0, Some(desk), 200);
        assert!(seat.touch_long_press(1).is_empty());
        // A finger that moves along a title bar is a move, not the window's menu
        // (a still one is: windows milestone 1, step 4).
        seat.touch_up(0, Some(desk), 300);
        let bar = Target::Server(Part::Title(W1));
        seat.touch_down(0, 300.0, 300.0, Some(bar), 400);
        seat.touch_motion(0, 340.0, 310.0, Some(bar));
        assert!(seat.touch_long_press(3).is_empty());
        assert!(matches!(seat.grab(), Some(Grab::Move { window: W1, .. })));
    }

    #[test]
    fn a_window_goes_with_the_finger_that_asked() {
        let mut seat = Seat::new();
        seat.touch_down(0, 100.0, 20.0, Some(Target::Native(W1)), 0);
        assert!(!seat.touch_move_request(0, W2, 100.0, 20.0));
        assert!(seat.touch_move_request(0, W1, 100.0, 20.0));
        assert_eq!(seat.touch_motion(0, 150.0, 60.0, None), vec![Out::FingerMove { window: W1, dx: 50.0, dy: 40.0 }]);
        assert_eq!(seat.touch_up(0, None, 1), vec![Out::FingerMoveEnd(W1), Out::TouchUp(Target::Native(W1), 0)]);
    }

    #[test]
    fn a_cancel_lets_every_finger_go() {
        let mut seat = Seat::new();
        seat.touch_down(0, 5.0, 5.0, Some(s1()), 0);
        seat.touch_down(1, 6.0, 6.0, Some(s2()), 0);
        let out = seat.touch_cancel(None, 1);
        assert_eq!(out, vec![Out::TouchCancel(s1(), 0), Out::TouchCancel(s2(), 1)]);
        assert_eq!(seat.touch_counts(), (0, 0));
    }
}
