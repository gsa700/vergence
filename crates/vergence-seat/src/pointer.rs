//! The pointer: focus, the implicit grab, presses on the server's parts,
//! and the grabs they begin.
//!
//! What a press does, in order (the server's if-chain until 2026-11-12):
//!
//! 1. A grab in progress has it: a move or resize ends at the release of
//!    its button; a drag and drop is Smithay's.
//! 2. A second button while one is held goes where the first went.
//! 3. A menu or the popup open: a press on it is its own; anywhere else it
//!    closes it and goes no further (its release neither).
//! 4. A press on a window, its frame or content, raises it and gives it the
//!    keyboard. Then: Alt+left drag moves, Alt+right drag resizes any
//!    window; the title bar moves (a second press within 400 ms maximizes
//!    or restores instead); a border resizes; the buttons act at the press.
//! 5. Otherwise the press, and later its release, go to what is under it:
//!    content, the dock, a notification, the bare desktop (a left press
//!    there takes the keyboard from every window).

use crate::{BTN_LEFT, BTN_RIGHT, Grab, Menu, Out, Part, Seat, Target, WinId, keyboard::Mods};

/// Two presses on one title bar this close together are a double-click.
pub const DOUBLE_MS: u64 = 400;

#[derive(Debug, Default)]
pub(crate) struct Pointer {
    /// The target the pointer is on, or held on.
    pub(crate) focus: Option<Target>,
    /// Buttons held, in the order pressed: while any is, focus stays.
    pub(crate) held: Vec<u32>,
    /// Buttons whose release goes nowhere (their press closed a menu,
    /// pressed a frame button, landed on nothing, or came during a grab).
    pub(crate) swallow: Vec<u32>,
    /// The last press on a title bar, for a double-click.
    last_title: Option<(WinId, u64)>,
}

impl Pointer {
    /// The pointer is on `under` now: leave and enter if that changed.
    pub(crate) fn refocus(&mut self, under: Option<Target>, out: &mut Vec<Out>) {
        if self.focus == under {
            return;
        }
        if let Some(old) = self.focus.take() {
            out.push(Out::Leave(old));
        }
        if let Some(new) = under {
            out.push(Out::Enter(new));
        }
        self.focus = under;
    }

    fn swallow(&mut self, button: u32) {
        if !self.swallow.contains(&button) {
            self.swallow.push(button);
        }
    }
}

impl Seat {
    /// The pointer moved; `under` is what is under it now.
    pub fn motion(&mut self, under: Option<Target>) -> Vec<Out> {
        let mut out = Vec::new();
        match self.grab {
            Some(Grab::Edge) => out.push(Out::ToEdge),
            Some(Grab::Move { .. } | Grab::Resize { .. }) => out.push(Out::GrabMotion),
            None if !self.pointer.held.is_empty() => {
                // The implicit grab: the press's target follows the pointer anywhere.
                if let Some(t) = self.pointer.focus {
                    out.push(Out::Motion(t));
                }
            }
            None => {
                if self.pointer.focus == under {
                    if let Some(t) = under {
                        out.push(Out::Motion(t));
                    }
                } else {
                    self.pointer.refocus(under, &mut out);
                }
            }
        }
        out
    }

    /// A button went down or up; `under` is what is under the pointer,
    /// `now_ms` a clock in milliseconds (for the title bar's double-click).
    pub fn button(&mut self, button: u32, pressed: bool, under: Option<Target>, mods: Mods, now_ms: u64) -> Vec<Out> {
        let mut out = Vec::new();
        // A button with Super held is not a tap of Super.
        self.keyboard.super_alone = false;
        match self.grab {
            Some(Grab::Edge) => {
                out.push(Out::ToEdge);
                return out;
            }
            Some(Grab::Move { window, button: b } | Grab::Resize { window, button: b, .. }) => {
                if !pressed && button == b {
                    self.grab = None;
                    out.push(Out::GrabEnd(window));
                    self.pointer.refocus(under, &mut out);
                } else if pressed {
                    self.pointer.swallow(button);
                } else {
                    self.pointer.swallow.retain(|x| *x != button);
                }
                return out;
            }
            None => {}
        }
        if pressed {
            self.press(button, under, mods, now_ms, &mut out);
        } else {
            self.release(button, under, &mut out);
        }
        out
    }

    fn press(&mut self, button: u32, under: Option<Target>, mods: Mods, now_ms: u64, out: &mut Vec<Out>) {
        let p = &mut self.pointer;
        // A press already swallowed (a stuck button sent twice): still swallowed.
        if p.swallow.contains(&button) || p.held.contains(&button) {
            return;
        }
        if !p.held.is_empty() {
            match p.focus {
                Some(t) => {
                    out.push(Out::Press(t, button));
                    p.held.push(button);
                }
                None => p.swallow(button),
            }
            return;
        }
        if let Some(menu) = self.menu {
            let own = Target::Server(match menu {
                Menu::Menu => Part::Menu,
                Menu::Popup => Part::Popup,
            });
            if under == Some(own) {
                p.refocus(under, out);
                out.push(Out::Press(own, button));
                p.held.push(button);
            } else {
                self.menu = None;
                out.push(Out::CloseMenu);
                p.swallow(button);
            }
            return;
        }
        p.refocus(under, out);
        let Some(t) = under else {
            p.swallow(button);
            return;
        };
        if let Some(w) = t.window() {
            out.push(Out::Focus(w));
            self.keyboard.focus = Some(w);
        }
        let left = button == BTN_LEFT;
        let p = &mut self.pointer;
        match t {
            Target::Surface(w, _) | Target::Native(w) if mods.alt && (left || button == BTN_RIGHT) => {
                if left {
                    self.begin(Grab::Move { window: w, button }, out);
                } else {
                    self.begin(Grab::Resize { window: w, edges: None, button }, out);
                }
            }
            Target::Server(Part::Title(w)) if left => {
                let double = p.last_title.is_some_and(|(lw, at)| lw == w && now_ms.saturating_sub(at) < DOUBLE_MS);
                if double {
                    p.last_title = None;
                    out.push(Out::Maximize(w));
                    p.swallow(button);
                } else {
                    p.last_title = Some((w, now_ms));
                    self.begin(Grab::Move { window: w, button }, out);
                }
            }
            Target::Server(Part::Edge(w, e)) if left => self.begin(Grab::Resize { window: w, edges: Some(e), button }, out),
            Target::Server(Part::Close(w)) if left => {
                out.push(Out::Close(w));
                p.swallow(button);
            }
            Target::Server(Part::Maximize(w)) if left => {
                out.push(Out::Maximize(w));
                p.swallow(button);
            }
            Target::Server(Part::Minimize(w)) if left => {
                out.push(Out::Minimize(w));
                p.swallow(button);
            }
            // The frame's other buttons do nothing.
            Target::Server(Part::Title(_) | Part::Edge(..) | Part::Close(_) | Part::Maximize(_) | Part::Minimize(_)) => p.swallow(button),
            Target::Server(Part::Desktop) => {
                if left {
                    out.push(Out::Unfocus);
                    self.keyboard.focus = None;
                }
                out.push(Out::Press(t, button));
                self.pointer.held.push(button);
            }
            _ => {
                out.push(Out::Press(t, button));
                p.held.push(button);
            }
        }
    }

    fn release(&mut self, button: u32, under: Option<Target>, out: &mut Vec<Out>) {
        let p = &mut self.pointer;
        if let Some(i) = p.swallow.iter().position(|b| *b == button) {
            p.swallow.remove(i);
            return;
        }
        let Some(i) = p.held.iter().position(|b| *b == button) else { return };
        p.held.remove(i);
        if let Some(t) = p.focus {
            out.push(Out::Release(t, button));
        }
        if p.held.is_empty() {
            p.refocus(under, out);
        }
    }

    /// A move or resize begins: the pointer leaves what it was on.
    fn begin(&mut self, grab: Grab, out: &mut Vec<Out>) {
        if let Some(t) = self.pointer.focus.take() {
            out.push(Out::Leave(t));
        }
        out.push(match grab {
            Grab::Move { window, .. } => Out::MoveBegin(window),
            Grab::Resize { window, edges, .. } => Out::ResizeBegin(window, edges),
            Grab::Edge => Out::ToEdge,
        });
        self.grab = Some(grab);
    }

    /// A program asked to be moved (`resize` None) or resized with the press
    /// it was given (a Wayland or X11 window's own title bar, our toolkit's
    /// bar and edges). Granted only while a button is held on that window:
    /// the held button becomes the grab's. Empty when refused.
    pub fn grab_from_press(&mut self, window: WinId, resize: Option<crate::Edges>) -> Vec<Out> {
        let mut out = Vec::new();
        if self.grab.is_some() || self.pointer.focus.and_then(|t| t.window()) != Some(window) {
            return out;
        }
        let Some(&button) = self.pointer.held.first() else { return out };
        // The other held buttons' releases go nowhere now.
        for b in std::mem::take(&mut self.pointer.held).into_iter().skip(1) {
            self.pointer.swallow(b);
        }
        self.begin(
            match resize {
                None => Grab::Move { window, button },
                Some(e) => Grab::Resize { window, edges: Some(e), button },
            },
            &mut out,
        );
        out
    }

    /// Forget a held button without a release (a long press on the dock is
    /// its menu, not a click; touch.rs in the server until step 4).
    pub fn abandon(&mut self, button: u32) {
        self.pointer.held.retain(|b| *b != button);
    }

    /// A wheel turned (or a finger scrolled): who scrolls.
    pub fn axis(&mut self, under: Option<Target>) -> Vec<Out> {
        let mut out = Vec::new();
        match self.grab {
            Some(Grab::Edge) => out.push(Out::ToEdge),
            Some(_) => {}
            None if self.menu == Some(Menu::Popup) => out.push(Out::Axis(Target::Server(Part::Popup))),
            None => {
                if self.pointer.held.is_empty() {
                    self.pointer.refocus(under, &mut out);
                }
                if let Some(t) = self.pointer.focus {
                    out.push(Out::Axis(t));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::test_util::*;
    use crate::*;

    fn alt() -> keyboard::Mods {
        keyboard::Mods { alt: true, ..Default::default() }
    }

    #[test]
    fn enter_and_leave_come_once_each() {
        let mut seat = Seat::new();
        assert_eq!(seat.motion(Some(s1())), vec![Out::Enter(s1())]);
        assert_eq!(seat.motion(Some(s1())), vec![Out::Motion(s1())]);
        assert_eq!(seat.motion(Some(s2())), vec![Out::Leave(s1()), Out::Enter(s2())]);
        assert_eq!(seat.motion(None), vec![Out::Leave(s2())]);
        assert!(seat.motion(None).is_empty());
    }

    #[test]
    fn a_held_button_keeps_the_target_and_its_release_goes_there() {
        let mut seat = Seat::new();
        seat.motion(Some(s1()));
        let out = seat.button(BTN_LEFT, true, Some(s1()), Default::default(), 0);
        assert_eq!(out, vec![Out::Focus(W1), Out::Press(s1(), BTN_LEFT)]);
        // Dragged over another window: still the first one's.
        assert_eq!(seat.motion(Some(s2())), vec![Out::Motion(s1())]);
        // A second button goes there too.
        assert_eq!(seat.button(BTN_RIGHT, true, Some(s2()), Default::default(), 1), vec![Out::Press(s1(), BTN_RIGHT)]);
        assert_eq!(seat.button(BTN_LEFT, false, Some(s2()), Default::default(), 2), vec![Out::Release(s1(), BTN_LEFT)]);
        // The last release lets go: the pointer is on what is under it.
        assert_eq!(seat.button(BTN_RIGHT, false, Some(s2()), Default::default(), 3), vec![Out::Release(s1(), BTN_RIGHT), Out::Leave(s1()), Out::Enter(s2())]);
    }

    #[test]
    fn alt_drag_moves_and_resizes_any_window() {
        let mut seat = Seat::new();
        seat.motion(Some(Target::Native(W1)));
        let out = seat.button(BTN_LEFT, true, Some(Target::Native(W1)), alt(), 0);
        assert_eq!(out, vec![Out::Focus(W1), Out::Leave(Target::Native(W1)), Out::MoveBegin(W1)]);
        assert_eq!(seat.motion(Some(s2())), vec![Out::GrabMotion]);
        assert_eq!(seat.button(BTN_LEFT, false, Some(s2()), alt(), 1), vec![Out::GrabEnd(W1), Out::Enter(s2())]);
        let out = seat.button(BTN_RIGHT, true, Some(s2()), alt(), 2);
        assert_eq!(out, vec![Out::Focus(W2), Out::Leave(s2()), Out::ResizeBegin(W2, None)]);
    }

    #[test]
    fn another_button_during_a_move_goes_nowhere() {
        let mut seat = Seat::new();
        seat.button(BTN_LEFT, true, Some(Target::Server(Part::Title(W1))), Default::default(), 0);
        assert!(seat.button(BTN_RIGHT, true, Some(s2()), Default::default(), 1).is_empty());
        assert!(seat.button(BTN_RIGHT, false, Some(s2()), Default::default(), 2).is_empty());
        assert_eq!(seat.button(BTN_LEFT, false, None, Default::default(), 3), vec![Out::GrabEnd(W1)]);
    }

    #[test]
    fn a_title_bar_moves_and_a_double_click_maximizes() {
        let mut seat = Seat::new();
        let title = Some(Target::Server(Part::Title(W1)));
        seat.motion(title);
        let out = seat.button(BTN_LEFT, true, title, Default::default(), 1000);
        assert!(out.contains(&Out::MoveBegin(W1)));
        seat.button(BTN_LEFT, false, title, Default::default(), 1100);
        let out = seat.button(BTN_LEFT, true, title, Default::default(), 1200);
        assert_eq!(out, vec![Out::Focus(W1), Out::Maximize(W1)]);
        assert!(seat.button(BTN_LEFT, false, title, Default::default(), 1250).is_empty());
        // A third press is a move again, not another double-click.
        assert!(seat.button(BTN_LEFT, true, title, Default::default(), 1300).contains(&Out::MoveBegin(W1)));
    }

    #[test]
    fn presses_too_far_apart_or_on_two_windows_are_not_a_double_click() {
        let mut seat = Seat::new();
        let t1 = Some(Target::Server(Part::Title(W1)));
        let t2 = Some(Target::Server(Part::Title(W2)));
        seat.button(BTN_LEFT, true, t1, Default::default(), 0);
        seat.button(BTN_LEFT, false, t1, Default::default(), 10);
        assert!(seat.button(BTN_LEFT, true, t1, Default::default(), 500).contains(&Out::MoveBegin(W1)));
        seat.button(BTN_LEFT, false, t1, Default::default(), 510);
        assert!(seat.button(BTN_LEFT, true, t2, Default::default(), 520).contains(&Out::MoveBegin(W2)));
    }

    #[test]
    fn the_frame_buttons_act_at_the_press_and_swallow_the_release() {
        let mut seat = Seat::new();
        let close = Some(Target::Server(Part::Close(W1)));
        assert_eq!(seat.button(BTN_LEFT, true, close, Default::default(), 0), vec![Out::Enter(close.unwrap()), Out::Focus(W1), Out::Close(W1)]);
        assert!(seat.button(BTN_LEFT, false, close, Default::default(), 1).is_empty());
        let edge = Some(Target::Server(Part::Edge(W2, Edges::LEFT)));
        assert!(seat.button(BTN_LEFT, true, edge, Default::default(), 2).contains(&Out::ResizeBegin(W2, Some(Edges::LEFT))));
    }

    #[test]
    fn a_menu_takes_its_clicks_and_a_click_outside_closes_it() {
        let mut seat = Seat::new();
        seat.menu_opened(Menu::Menu);
        let menu = Some(Target::Server(Part::Menu));
        assert_eq!(seat.button(BTN_LEFT, true, menu, Default::default(), 0), vec![Out::Enter(menu.unwrap()), Out::Press(menu.unwrap(), BTN_LEFT)]);
        assert_eq!(seat.button(BTN_LEFT, false, menu, Default::default(), 1), vec![Out::Release(menu.unwrap(), BTN_LEFT)]);
        // Outside: closed, and the window under it hears nothing of it.
        assert_eq!(seat.button(BTN_LEFT, true, Some(s1()), Default::default(), 2), vec![Out::CloseMenu]);
        assert_eq!(seat.menu(), None);
        assert!(seat.button(BTN_LEFT, false, Some(s1()), Default::default(), 3).is_empty());
        assert_eq!(seat.keyboard_focus(), None);
    }

    #[test]
    fn a_left_press_on_the_desktop_takes_the_keyboard_from_every_window() {
        let mut seat = Seat::new();
        seat.button(BTN_LEFT, true, Some(s1()), Default::default(), 0);
        seat.button(BTN_LEFT, false, Some(s1()), Default::default(), 1);
        assert_eq!(seat.keyboard_focus(), Some(W1));
        let desk = Target::Server(Part::Desktop);
        let out = seat.button(BTN_LEFT, true, Some(desk), Default::default(), 2);
        assert!(out.contains(&Out::Unfocus) && out.contains(&Out::Press(desk, BTN_LEFT)));
        assert_eq!(seat.keyboard_focus(), None);
        // The right button there keeps the focus (the desktop menu opens).
        seat.button(BTN_LEFT, false, Some(desk), Default::default(), 3);
        seat.button(BTN_LEFT, true, Some(s1()), Default::default(), 4);
        seat.button(BTN_LEFT, false, Some(s1()), Default::default(), 5);
        let out = seat.button(BTN_RIGHT, true, Some(desk), Default::default(), 6);
        assert!(!out.contains(&Out::Unfocus));
        assert_eq!(seat.keyboard_focus(), Some(W1));
    }

    #[test]
    fn a_press_on_the_dock_is_released_on_the_dock() {
        let mut seat = Seat::new();
        let dock = Target::Server(Part::Dock);
        seat.button(BTN_LEFT, true, Some(dock), Default::default(), 0);
        // Dragged off the dock: the dock still hears where it went and the release.
        assert_eq!(seat.motion(Some(s1())), vec![Out::Motion(dock)]);
        assert_eq!(seat.button(BTN_LEFT, false, Some(s1()), Default::default(), 1), vec![Out::Release(dock, BTN_LEFT), Out::Leave(dock), Out::Enter(s1())]);
    }

    #[test]
    fn the_wheel_scrolls_the_popup_while_it_is_open() {
        let mut seat = Seat::new();
        seat.motion(Some(s1()));
        assert_eq!(seat.axis(Some(s1())), vec![Out::Axis(s1())]);
        seat.menu_opened(Menu::Popup);
        assert_eq!(seat.axis(Some(s1())), vec![Out::Axis(Target::Server(Part::Popup))]);
    }

    #[test]
    fn a_program_may_move_itself_only_with_a_press_held_on_it() {
        let mut seat = Seat::new();
        let bar = Target::Native(W1);
        // Nothing held: refused.
        seat.motion(Some(bar));
        assert!(seat.grab_from_press(W1, None).is_empty());
        seat.button(BTN_LEFT, true, Some(bar), Default::default(), 0);
        // Another window's request: refused.
        assert!(seat.grab_from_press(W2, None).is_empty());
        assert_eq!(seat.grab_from_press(W1, Some(Edges::BOTTOM)), vec![Out::Leave(bar), Out::ResizeBegin(W1, Some(Edges::BOTTOM))]);
        assert_eq!(seat.motion(Some(s2())), vec![Out::GrabMotion]);
        // The press's release ends it; the window never hears it.
        assert_eq!(seat.button(BTN_LEFT, false, Some(s2()), Default::default(), 1), vec![Out::GrabEnd(W1), Out::Enter(s2())]);
    }

    #[test]
    fn a_release_with_no_press_goes_nowhere() {
        let mut seat = Seat::new();
        assert!(seat.button(BTN_LEFT, false, Some(s1()), Default::default(), 0).is_empty());
        // A press on nothing (between screens): its release neither.
        seat.button(BTN_MIDDLE, true, None, Default::default(), 1);
        assert!(seat.button(BTN_MIDDLE, false, Some(s1()), Default::default(), 2).is_empty());
    }
}
