//! The keyboard: the server's own keys first, then the focused window.
//!
//! The server reads each key's symbol from its own xkb state (xkbcommon,
//! used directly; Dave, 2026-11-12) and names the few the seat cares about
//! ([`Key`]); everything else is [`Key::Other`]. The seat never reads a
//! keymap, so nothing here can run inside Smithay's keyboard lock.
//!
//! The filter, in order, for a key going down:
//!
//! 1. Ctrl+Alt+Backspace ends the session.
//! 2. Escape closes the desktop menu or an icon's menu.
//! 3. The volume keys are the default output's, never a program's.
//! 4. Ctrl+Alt+F1..F12 (the consoles' keys) switch consoles.
//! 5. Super goes down: a tap of Super alone, if nothing else is pressed
//!    before it comes up, opens or closes the apps popup.
//! 6. While the popup is open, every key is its.
//! 7. Otherwise the window with the keyboard has it.

use crate::{Menu, Seat, WinId};

/// The keys the seat tells apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    BackSpace,
    Escape,
    /// Either Super (logo) key.
    Super,
    VolumeUp,
    VolumeDown,
    Mute,
    /// The key for console n (1 to 12): XF86Switch_VT_n.
    Vt(u8),
    Other,
}

/// The modifiers held, from the server's xkb state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

/// What a key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOut {
    /// End the session.
    Quit,
    /// Switch to console n.
    Vt(u8),
    /// Close the menu.
    CloseMenu,
    /// Open or close the apps popup.
    Apps,
    /// The popup's: the server gives it the key's symbol and character.
    Popup,
    /// The default output's volume: +5, -5, or 0 to mute or unmute.
    Volume(i8),
    /// Kept from every program, with nothing else to do.
    Swallow,
    /// To the window with the keyboard (None: no window has it; the key is
    /// still fed to Smithay's keyboard so its state stays right).
    Forward(Option<WinId>),
}

#[derive(Debug, Default)]
pub(crate) struct Keyboard {
    pub(crate) focus: Option<WinId>,
    /// Super is down and nothing else has been pressed since.
    pub(crate) super_alone: bool,
}

impl Seat {
    /// A key went down or up.
    pub fn key(&mut self, key: Key, pressed: bool, mods: Mods) -> KeyOut {
        let is_super = key == Key::Super;
        if pressed {
            if mods.ctrl && mods.alt && key == Key::BackSpace {
                return KeyOut::Quit;
            }
            if self.menu == Some(Menu::Menu) && key == Key::Escape {
                self.menu = None;
                return KeyOut::CloseMenu;
            }
            match key {
                Key::VolumeUp => return KeyOut::Volume(5),
                Key::VolumeDown => return KeyOut::Volume(-5),
                Key::Mute => return KeyOut::Volume(0),
                Key::Vt(n) if (1..=12).contains(&n) => return KeyOut::Vt(n),
                _ => {}
            }
            self.keyboard.super_alone = is_super;
        } else if is_super && std::mem::take(&mut self.keyboard.super_alone) {
            return KeyOut::Apps;
        }
        if self.menu == Some(Menu::Popup) {
            return if pressed && !is_super { KeyOut::Popup } else { KeyOut::Swallow };
        }
        KeyOut::Forward(self.keyboard.focus)
    }

    /// The server gave a window the keyboard by itself (a new window, a
    /// click on its dock icon, one brought back from minimized), or took it
    /// from all (None).
    pub fn focus(&mut self, window: Option<WinId>) {
        self.keyboard.focus = window;
    }
}

#[cfg(test)]
mod tests {
    use crate::test_util::*;
    use crate::*;
    use keyboard::{Key, KeyOut, Mods};

    fn none() -> Mods {
        Mods::default()
    }

    #[test]
    fn ctrl_alt_backspace_ends_the_session_whatever_has_the_keyboard() {
        let mut seat = Seat::new();
        seat.menu_opened(Menu::Popup);
        let m = Mods { ctrl: true, alt: true, ..none() };
        assert_eq!(seat.key(Key::BackSpace, true, m), KeyOut::Quit);
        assert_eq!(seat.key(Key::BackSpace, true, none()), KeyOut::Popup);
    }

    #[test]
    fn the_volume_and_console_keys_are_never_a_programs() {
        let mut seat = Seat::new();
        seat.focus(Some(W1));
        assert_eq!(seat.key(Key::VolumeUp, true, none()), KeyOut::Volume(5));
        assert_eq!(seat.key(Key::VolumeDown, true, none()), KeyOut::Volume(-5));
        assert_eq!(seat.key(Key::Mute, true, none()), KeyOut::Volume(0));
        assert_eq!(seat.key(Key::Vt(3), true, none()), KeyOut::Vt(3));
        // Their releases go on to the window, which never saw the press: harmless,
        // and Smithay's keyboard state stays right.
        assert_eq!(seat.key(Key::VolumeUp, false, none()), KeyOut::Forward(Some(W1)));
    }

    #[test]
    fn super_tapped_alone_opens_the_popup() {
        let mut seat = Seat::new();
        assert_eq!(seat.key(Key::Super, true, none()), KeyOut::Forward(None));
        assert_eq!(seat.key(Key::Super, false, Mods { logo: true, ..none() }), KeyOut::Apps);
    }

    #[test]
    fn super_with_another_key_or_a_button_is_not_a_tap() {
        let mut seat = Seat::new();
        seat.key(Key::Super, true, none());
        seat.key(Key::Other, true, Mods { logo: true, ..none() });
        assert_eq!(seat.key(Key::Super, false, none()), KeyOut::Forward(None));
        seat.key(Key::Super, true, none());
        seat.button(BTN_LEFT, true, Some(s1()), Mods { logo: true, ..none() }, 0);
        assert_eq!(seat.key(Key::Super, false, none()), KeyOut::Forward(Some(W1)));
    }

    #[test]
    fn the_open_popup_has_every_key() {
        let mut seat = Seat::new();
        seat.focus(Some(W1));
        seat.menu_opened(Menu::Popup);
        assert_eq!(seat.key(Key::Other, true, none()), KeyOut::Popup);
        assert_eq!(seat.key(Key::Escape, true, none()), KeyOut::Popup);
        assert_eq!(seat.key(Key::Other, false, none()), KeyOut::Swallow);
        // Super tapped again closes it.
        seat.key(Key::Super, true, none());
        assert_eq!(seat.key(Key::Super, false, none()), KeyOut::Apps);
    }

    #[test]
    fn escape_closes_a_menu_and_other_keys_still_reach_the_window() {
        let mut seat = Seat::new();
        seat.focus(Some(W2));
        seat.menu_opened(Menu::Menu);
        assert_eq!(seat.key(Key::Other, true, none()), KeyOut::Forward(Some(W2)));
        assert_eq!(seat.key(Key::Escape, true, none()), KeyOut::CloseMenu);
        assert_eq!(seat.menu(), None);
        assert_eq!(seat.key(Key::Escape, true, none()), KeyOut::Forward(Some(W2)));
    }

    #[test]
    fn a_press_on_a_window_gives_it_the_keyboard() {
        let mut seat = Seat::new();
        seat.button(BTN_LEFT, true, Some(s2()), none(), 0);
        assert_eq!(seat.key(Key::Other, true, none()), KeyOut::Forward(Some(W2)));
    }
}
