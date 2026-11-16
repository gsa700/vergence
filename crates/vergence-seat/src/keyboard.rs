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
//! 5. While Alt+Tab is switching (windows milestone 1), every key is its:
//!    Tab and the arrows step, Escape cancels, Alt let go chooses.
//! 6. Super goes down: a tap of Super alone, if nothing else is pressed
//!    before it comes up, opens or closes the apps popup.
//! 7. While the popup is open, every key is its.
//! 8. Alt+Tab (Alt+Shift+Tab backwards) begins switching among every
//!    window; Alt+` (the key above Tab) among the focused application's.
//! 9. Super with an arrow is a window key (snap, maximize, restore);
//!    Super+D shows the desktop (files milestone 1), and again brings the
//!    windows back.
//! 10. Otherwise the window with the keyboard has it.
//!
//! The seat also keeps the order in which windows last had the keyboard
//! ([`Seat::order_of_use`]): the order Alt+Tab steps through.

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
    Tab,
    /// The key above Tab (evdev's KEY_GRAVE, whatever the layout prints on it).
    Grave,
    /// Either Alt key: letting it go ends Alt+Tab's switching.
    Alt,
    Left,
    Right,
    Up,
    Down,
    Space,
    /// The D key (Super+D: show the desktop).
    D,
    /// The L key (Super+L: lock the screen), and Q (Ctrl+Super+Q, the Mac's).
    L,
    Q,
    Other,
}

/// A window key: Super with an arrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKey {
    Left,
    Right,
    Up,
    Down,
}

/// What Alt+Tab's switching is to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// Switching begins: among every window, or (`app`) the focused
    /// application's; the first step is backwards with Shift held.
    Begin { app: bool, back: bool },
    /// One step on (or back).
    Step { back: bool },
    /// Alt was let go: the window stepped to takes the keyboard.
    Choose,
    /// Escape: the keyboard stays where it was.
    Cancel,
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
    /// Ctrl+Alt+Escape: open Force Quit.
    ForceQuit,
    /// Switch to console n.
    Vt(u8),
    /// Close the menu.
    CloseMenu,
    /// Open or close the apps popup.
    Apps,
    /// The popup's: the server gives it the key's symbol and character.
    Popup,
    /// The open app menu's (app settings milestone 1), as the popup's.
    AppMenu,
    /// Alt+Space: open the app menu of the window with the keyboard.
    OpenAppMenu(WinId),
    /// The default output's volume: +5, -5, or 0 to mute or unmute.
    Volume(i8),
    /// Alt+Tab's switching.
    Switch(Switch),
    /// Super with an arrow.
    Window(WindowKey),
    /// Super+D: every window out of the way, or back.
    ShowDesktop,
    /// Kept from every program, with nothing else to do.
    Swallow,
    /// Lock the screen: Super+L, or Ctrl+Super+Q (VDM milestone 1).
    Lock,
    /// The screen is locked: the key is the lock dialog's, and nobody else's.
    Locked,
    /// To the window with the keyboard (None: no window has it; the key is
    /// still fed to Smithay's keyboard so its state stays right).
    Forward(Option<WinId>),
}

#[derive(Debug)]
pub(crate) struct Keyboard {
    pub(crate) focus: Option<WinId>,
    /// Super is down and nothing else has been pressed since.
    pub(crate) super_alone: bool,
    /// Windows by when they last had the keyboard, the most recent first.
    pub(crate) used: Vec<WinId>,
    /// Alt+Tab is switching (among one application's windows if true).
    pub(crate) switching: Option<bool>,
    /// The settings: Super with the arrows, and Alt+`, are the server's.
    pub(crate) window_keys: bool,
    pub(crate) app_switch: bool,
}

impl Default for Keyboard {
    fn default() -> Keyboard {
        Keyboard { focus: None, super_alone: false, used: Vec::new(), switching: None, window_keys: true, app_switch: true }
    }
}

/// At most this many windows are kept in the order of use.
const USED_MAX: usize = 1024;

impl Keyboard {
    /// A window took the keyboard: it is the most recently used.
    pub(crate) fn took(&mut self, w: WinId) {
        self.focus = Some(w);
        self.used.retain(|&x| x != w);
        self.used.insert(0, w);
        self.used.truncate(USED_MAX);
    }
}

impl Seat {
    /// A key went down or up.
    pub fn key(&mut self, key: Key, pressed: bool, mods: Mods) -> KeyOut {
        let is_super = key == Key::Super;
        // Locked (review I6): before any other key of the server's, the
        // session's end and Force Quit among them. Only the volume and
        // console switching pass; everything else is the lock dialog's.
        if self.locked {
            return match key {
                Key::VolumeUp if pressed => KeyOut::Volume(5),
                Key::VolumeDown if pressed => KeyOut::Volume(-5),
                Key::Mute if pressed => KeyOut::Volume(0),
                Key::Vt(n) if pressed && (1..=12).contains(&n) => KeyOut::Vt(n),
                _ => KeyOut::Locked,
            };
        }
        if pressed && mods.logo && !mods.alt && !mods.shift && ((key == Key::L && !mods.ctrl) || (key == Key::Q && mods.ctrl)) {
            self.keyboard.super_alone = false;
            return KeyOut::Lock;
        }
        if pressed {
            if mods.ctrl && mods.alt && key == Key::BackSpace {
                return KeyOut::Quit;
            }
            // Force Quit (tools milestone 1), as macOS has Cmd+Option+Esc: whatever has the keyboard.
            if mods.ctrl && mods.alt && !mods.logo && key == Key::Escape {
                self.menu = None;
                return KeyOut::ForceQuit;
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
        }
        if self.keyboard.switching.is_some() {
            return self.switching_key(key, pressed, mods);
        }
        if pressed {
            self.keyboard.super_alone = is_super;
        } else if is_super && std::mem::take(&mut self.keyboard.super_alone) {
            return KeyOut::Apps;
        }
        if self.menu == Some(Menu::Popup) {
            return if pressed && !is_super { KeyOut::Popup } else { KeyOut::Swallow };
        }
        if self.menu == Some(Menu::AppMenu) {
            return if pressed { KeyOut::AppMenu } else { KeyOut::Swallow };
        }
        // Alt+Space: the app menu, as Windows' system menu (app settings milestone 1).
        if pressed
            && key == Key::Space
            && mods.alt
            && !mods.ctrl
            && !mods.logo
            && let Some(w) = self.keyboard.focus
        {
            return KeyOut::OpenAppMenu(w);
        }
        if pressed && mods.alt && !mods.ctrl && !mods.logo {
            let app = match key {
                Key::Tab => Some(false),
                Key::Grave if self.keyboard.app_switch => Some(true),
                _ => None,
            };
            if let Some(app) = app {
                self.keyboard.switching = Some(app);
                return KeyOut::Switch(Switch::Begin { app, back: mods.shift });
            }
        }
        if pressed && mods.logo && !mods.alt && !mods.ctrl && !mods.shift && key == Key::D {
            return KeyOut::ShowDesktop;
        }
        if pressed && mods.logo && !mods.alt && !mods.ctrl && self.keyboard.window_keys {
            let w = match key {
                Key::Left => Some(WindowKey::Left),
                Key::Right => Some(WindowKey::Right),
                Key::Up => Some(WindowKey::Up),
                Key::Down => Some(WindowKey::Down),
                _ => None,
            };
            if let Some(w) = w {
                return KeyOut::Window(w);
            }
        }
        KeyOut::Forward(self.keyboard.focus)
    }

    /// A key while Alt+Tab is switching: all of them are its.
    fn switching_key(&mut self, key: Key, pressed: bool, mods: Mods) -> KeyOut {
        let app = self.keyboard.switching == Some(true);
        if !pressed {
            // Alt let go (or gone by the time a key came up): the choice is made.
            if key == Key::Alt || !mods.alt {
                self.keyboard.switching = None;
                return KeyOut::Switch(Switch::Choose);
            }
            return KeyOut::Swallow;
        }
        match key {
            Key::Escape => {
                self.keyboard.switching = None;
                KeyOut::Switch(Switch::Cancel)
            }
            Key::Tab if !app => KeyOut::Switch(Switch::Step { back: mods.shift }),
            Key::Grave if app => KeyOut::Switch(Switch::Step { back: mods.shift }),
            Key::Left | Key::Up => KeyOut::Switch(Switch::Step { back: true }),
            Key::Right | Key::Down => KeyOut::Switch(Switch::Step { back: false }),
            _ => KeyOut::Swallow,
        }
    }

    /// Whether Alt+Tab is switching.
    pub fn switching(&self) -> bool {
        self.keyboard.switching.is_some()
    }

    /// The server ended the switching itself (the window list emptied, the
    /// session is ending): the next keys are ordinary again.
    pub fn switch_ended(&mut self) {
        self.keyboard.switching = None;
    }

    /// The settings: whether Super with an arrow, and Alt+`, are the server's.
    pub fn set_window_keys(&mut self, window_keys: bool, app_switch: bool) {
        self.keyboard.window_keys = window_keys;
        self.keyboard.app_switch = app_switch;
    }

    /// Something other than a key or button came between Super's press and
    /// release (a finger on the panel): it is not a tap of Super.
    pub fn interrupt_super(&mut self) {
        self.keyboard.super_alone = false;
    }

    /// The server gave a window the keyboard by itself (a new window, a
    /// click on its dock icon, one brought back from minimized), or took it
    /// from all (None).
    pub fn focus(&mut self, window: Option<WinId>) {
        match window {
            Some(w) => self.keyboard.took(w),
            None => self.keyboard.focus = None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test_util::*;
    use crate::*;
    use keyboard::{Key, KeyOut, Mods, Switch, WindowKey};

    fn none() -> Mods {
        Mods::default()
    }

    #[test]
    fn locked_every_key_is_the_locks_but_volume_and_consoles() {
        let mut seat = Seat::new();
        let ctrl_alt = Mods { ctrl: true, alt: true, ..none() };
        let logo = Mods { logo: true, ..none() };
        assert_eq!(seat.key(Key::L, true, logo), KeyOut::Lock, "Super+L locks");
        assert_eq!(seat.key(Key::Q, true, Mods { ctrl: true, ..logo }), KeyOut::Lock, "Ctrl+Super+Q too");
        assert_eq!(seat.key(Key::L, true, Mods { shift: true, ..logo }), KeyOut::Forward(None), "not with Shift");
        seat.lock();
        assert!(seat.locked());
        assert_eq!(seat.key(Key::BackSpace, true, ctrl_alt), KeyOut::Locked, "no ending the session from the lock");
        assert_eq!(seat.key(Key::Escape, true, ctrl_alt), KeyOut::Locked, "no Force Quit");
        assert_eq!(seat.key(Key::Tab, true, Mods { alt: true, ..none() }), KeyOut::Locked, "no Alt+Tab");
        assert_eq!(seat.key(Key::Super, true, none()), KeyOut::Locked);
        assert_eq!(seat.key(Key::Super, false, none()), KeyOut::Locked, "no launcher on Super's tap");
        assert_eq!(seat.key(Key::Other, true, none()), KeyOut::Locked);
        assert_eq!(seat.key(Key::VolumeUp, true, none()), KeyOut::Volume(5));
        assert_eq!(seat.key(Key::Vt(2), true, ctrl_alt), KeyOut::Vt(2), "consoles still switch (Dave: 2 a)");
        seat.unlock();
        assert_eq!(seat.key(Key::Other, true, none()), KeyOut::Forward(None));
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
    fn a_finger_between_supers_press_and_release_is_not_a_tap() {
        let mut seat = Seat::new();
        seat.key(Key::Super, true, none());
        seat.interrupt_super();
        assert_eq!(seat.key(Key::Super, false, none()), KeyOut::Forward(None));
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
    fn ctrl_alt_escape_is_force_quit_whatever_has_the_keyboard() {
        let mut seat = Seat::new();
        seat.focus(Some(W1));
        let m = Mods { ctrl: true, alt: true, ..none() };
        assert_eq!(seat.key(Key::Escape, true, m), KeyOut::ForceQuit);
        seat.menu_opened(Menu::AppMenu);
        assert_eq!(seat.key(Key::Escape, true, m), KeyOut::ForceQuit);
        assert_eq!(seat.menu(), None);
        assert_eq!(seat.key(Key::Escape, true, Mods { ctrl: true, ..none() }), KeyOut::Forward(Some(W1)));
    }

    #[test]
    fn alt_space_opens_the_app_menu_which_then_takes_every_key() {
        let mut seat = Seat::new();
        assert_eq!(seat.key(Key::Space, true, alt()), KeyOut::Forward(None), "no window: nothing to open");
        seat.focus(Some(W1));
        assert_eq!(seat.key(Key::Space, true, none()), KeyOut::Forward(Some(W1)));
        assert_eq!(seat.key(Key::Space, true, alt()), KeyOut::OpenAppMenu(W1));
        assert_eq!(seat.key(Key::Space, true, Mods { ctrl: true, alt: true, ..none() }), KeyOut::Forward(Some(W1)));
        seat.menu_opened(Menu::AppMenu);
        assert_eq!(seat.key(Key::Other, true, none()), KeyOut::AppMenu);
        assert_eq!(seat.key(Key::Escape, true, none()), KeyOut::AppMenu, "the menu closes itself");
        assert_eq!(seat.key(Key::Other, false, none()), KeyOut::Swallow);
        assert_eq!(seat.key(Key::BackSpace, true, Mods { ctrl: true, alt: true, ..none() }), KeyOut::Quit);
        seat.menu_closed();
        assert_eq!(seat.key(Key::Other, true, none()), KeyOut::Forward(Some(W1)));
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

    fn alt() -> Mods {
        Mods { alt: true, ..none() }
    }

    #[test]
    fn the_order_of_use_puts_the_latest_window_first_and_forgets_gone_ones() {
        let mut seat = Seat::new();
        seat.focus(Some(W1));
        seat.focus(Some(W2));
        seat.button(BTN_LEFT, true, Some(s1()), none(), 0);
        assert_eq!(seat.order_of_use(), &[W1, W2]);
        // Losing the keyboard to the desktop keeps the order.
        seat.focus(None);
        assert_eq!(seat.order_of_use(), &[W1, W2]);
        seat.window_gone(W1);
        assert_eq!(seat.order_of_use(), &[W2]);
    }

    #[test]
    fn a_finger_on_a_window_counts_as_use() {
        let mut seat = Seat::new();
        seat.focus(Some(W1));
        seat.touch_down(0, 10.0, 10.0, Some(s2()), 0);
        assert_eq!(seat.order_of_use(), &[W2, W1]);
    }

    #[test]
    fn alt_tab_steps_while_alt_is_held_and_chooses_when_it_is_let_go() {
        let mut seat = Seat::new();
        seat.focus(Some(W1));
        assert_eq!(seat.key(Key::Alt, true, alt()), KeyOut::Forward(Some(W1)));
        assert_eq!(seat.key(Key::Tab, true, alt()), KeyOut::Switch(Switch::Begin { app: false, back: false }));
        assert!(seat.switching());
        assert_eq!(seat.key(Key::Tab, false, alt()), KeyOut::Swallow);
        assert_eq!(seat.key(Key::Tab, true, alt()), KeyOut::Switch(Switch::Step { back: false }));
        assert_eq!(seat.key(Key::Tab, true, Mods { shift: true, ..alt() }), KeyOut::Switch(Switch::Step { back: true }));
        assert_eq!(seat.key(Key::Left, true, alt()), KeyOut::Switch(Switch::Step { back: true }));
        assert_eq!(seat.key(Key::Down, true, alt()), KeyOut::Switch(Switch::Step { back: false }));
        // Any other key is the switcher's, not the window's.
        assert_eq!(seat.key(Key::Other, true, alt()), KeyOut::Swallow);
        assert_eq!(seat.key(Key::Alt, false, none()), KeyOut::Switch(Switch::Choose));
        assert!(!seat.switching());
        assert_eq!(seat.key(Key::Tab, true, none()), KeyOut::Forward(Some(W1)));
    }

    #[test]
    fn shift_makes_the_first_step_backwards() {
        let mut seat = Seat::new();
        assert_eq!(seat.key(Key::Tab, true, Mods { shift: true, ..alt() }), KeyOut::Switch(Switch::Begin { app: false, back: true }));
    }

    #[test]
    fn escape_cancels_the_switching() {
        let mut seat = Seat::new();
        seat.key(Key::Tab, true, alt());
        assert_eq!(seat.key(Key::Escape, true, alt()), KeyOut::Switch(Switch::Cancel));
        // Alt let go afterwards chooses nothing.
        assert_eq!(seat.key(Key::Alt, false, none()), KeyOut::Forward(None));
    }

    #[test]
    fn alt_grave_switches_among_one_applications_windows_unless_switched_off() {
        let mut seat = Seat::new();
        assert_eq!(seat.key(Key::Grave, true, alt()), KeyOut::Switch(Switch::Begin { app: true, back: false }));
        assert_eq!(seat.key(Key::Grave, true, alt()), KeyOut::Switch(Switch::Step { back: false }));
        // Tab is not a step of Alt+`'s switching.
        assert_eq!(seat.key(Key::Tab, true, alt()), KeyOut::Swallow);
        seat.key(Key::Alt, false, none());
        seat.set_window_keys(true, false);
        assert_eq!(seat.key(Key::Grave, true, alt()), KeyOut::Forward(None));
    }

    #[test]
    fn ctrl_alt_tab_and_super_alt_tab_are_the_programs() {
        let mut seat = Seat::new();
        seat.focus(Some(W1));
        assert_eq!(seat.key(Key::Tab, true, Mods { ctrl: true, ..alt() }), KeyOut::Forward(Some(W1)));
        assert_eq!(seat.key(Key::Tab, true, Mods { logo: true, ..alt() }), KeyOut::Forward(Some(W1)));
    }

    #[test]
    fn the_open_popup_keeps_alt_tab() {
        let mut seat = Seat::new();
        seat.menu_opened(Menu::Popup);
        assert_eq!(seat.key(Key::Tab, true, alt()), KeyOut::Popup);
        assert!(!seat.switching());
    }

    #[test]
    fn the_volume_keys_still_work_while_switching() {
        let mut seat = Seat::new();
        seat.key(Key::Tab, true, alt());
        assert_eq!(seat.key(Key::VolumeUp, true, alt()), KeyOut::Volume(5));
        assert!(seat.switching());
    }

    #[test]
    fn super_with_an_arrow_is_a_window_key_and_not_a_tap() {
        let mut seat = Seat::new();
        seat.focus(Some(W1));
        let logo = Mods { logo: true, ..none() };
        seat.key(Key::Super, true, logo);
        assert_eq!(seat.key(Key::Left, true, logo), KeyOut::Window(WindowKey::Left));
        assert_eq!(seat.key(Key::Up, true, logo), KeyOut::Window(WindowKey::Up));
        assert_eq!(seat.key(Key::Super, false, none()), KeyOut::Forward(Some(W1)));
        // Switched off, they are the program's.
        seat.set_window_keys(false, true);
        assert_eq!(seat.key(Key::Right, true, logo), KeyOut::Forward(Some(W1)));
        // An arrow alone is always the program's.
        assert_eq!(seat.key(Key::Right, true, none()), KeyOut::Forward(Some(W1)));
    }

    #[test]
    fn screens_off_cancels_the_switching() {
        let mut seat = Seat::new();
        seat.key(Key::Tab, true, alt());
        assert!(seat.screens_off().contains(&Out::SwitchCancel));
        assert!(!seat.switching());
    }

    #[test]
    fn a_press_on_a_window_gives_it_the_keyboard() {
        let mut seat = Seat::new();
        seat.button(BTN_LEFT, true, Some(s2()), none(), 0);
        assert_eq!(seat.key(Key::Other, true, none()), KeyOut::Forward(Some(W2)));
    }
}
