// The server's own keyboard state (seat milestone 1, step 3; Dave, 2026-11-12:
// xkbcommon used directly).
//
// Smithay's keyboard keeps an xkb state for Wayland and X11 windows; this is a
// second one from the same keymap and the same keys, for everything else: the
// seat's key filter (which keys are the server's), the apps popup's typing, and
// native windows' keys and modifiers (native.rs). It is read and changed only
// in handle_key, never inside one of Smithay's keyboard callbacks, which run
// holding the keyboard's lock (2026-11-11: reading the keymap there froze the
// server).
//
// The keymap is Smithay's own, taken once as text before the first key, so the
// two states cannot disagree about what a key is.

use xkbcommon::xkb;

pub struct Keys {
    state: xkb::State,
    /// The keymap as xkb text (for native clients, NUL-terminated there).
    pub text: String,
    /// The modifiers as last told: depressed, latched, locked, layout.
    told: (u32, u32, u32, u32),
}

/// What one key did.
pub struct Pressed {
    /// The key's symbol with the modifiers held as it went down (or up).
    pub sym: u32,
    /// The character it types, if one.
    pub typed: Option<char>,
    /// The modifiers changed (native windows are told).
    pub mods_changed: bool,
}

impl Keys {
    pub fn from_text(text: String) -> Option<Keys> {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_string(&ctx, text.clone(), xkb::KEYMAP_FORMAT_TEXT_V1, xkb::KEYMAP_COMPILE_NO_FLAGS)?;
        let state = xkb::State::new(&keymap);
        Some(Keys { state, text, told: (0, 0, 0, 0) })
    }

    /// A key (xkb's code: evdev's plus 8) went down or up.
    pub fn feed(&mut self, code: xkb::Keycode, pressed: bool) -> Pressed {
        let sym = self.state.key_get_one_sym(code).raw();
        let typed = if pressed { self.state.key_get_utf8(code).chars().next().filter(|c| !c.is_control()) } else { None };
        self.state.update_key(code, if pressed { xkb::KeyDirection::Down } else { xkb::KeyDirection::Up });
        let now = self.serialized();
        let mods_changed = now != self.told;
        self.told = now;
        Pressed { sym, typed, mods_changed }
    }

    /// Depressed, latched, locked modifiers and the effective layout, as Wayland tells them.
    pub fn serialized(&self) -> (u32, u32, u32, u32) {
        (
            self.state.serialize_mods(xkb::STATE_MODS_DEPRESSED),
            self.state.serialize_mods(xkb::STATE_MODS_LATCHED),
            self.state.serialize_mods(xkb::STATE_MODS_LOCKED),
            self.state.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE),
        )
    }

    /// The modifiers in effect, for the seat.
    pub fn mods(&self) -> vergence_seat::keyboard::Mods {
        let on = |name: &str| self.state.mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE);
        vergence_seat::keyboard::Mods { ctrl: on(xkb::MOD_NAME_CTRL), alt: on(xkb::MOD_NAME_ALT), shift: on(xkb::MOD_NAME_SHIFT), logo: on(xkb::MOD_NAME_LOGO) }
    }
}

/// The key above Tab, by position (evdev's KEY_GRAVE, 41, plus xkb's 8):
/// Alt+` means that key whatever the layout prints on it.
const GRAVE_CODE: u32 = 41 + 8;

/// The seat's name for a key, from its symbol (and, for the key above Tab,
/// its code: xkb's, evdev's plus 8).
pub fn seat_key(sym: u32, code: u32) -> vergence_seat::keyboard::Key {
    use vergence_seat::keyboard::Key;
    use xkb::keysyms as k;
    if code == GRAVE_CODE {
        return Key::Grave;
    }
    match sym {
        k::KEY_BackSpace => Key::BackSpace,
        k::KEY_Escape => Key::Escape,
        k::KEY_Super_L | k::KEY_Super_R => Key::Super,
        k::KEY_XF86AudioRaiseVolume => Key::VolumeUp,
        k::KEY_XF86AudioLowerVolume => Key::VolumeDown,
        k::KEY_XF86AudioMute => Key::Mute,
        // Shift+Tab is ISO_Left_Tab in most keymaps.
        k::KEY_Tab | k::KEY_ISO_Left_Tab => Key::Tab,
        k::KEY_Alt_L | k::KEY_Alt_R => Key::Alt,
        k::KEY_Left => Key::Left,
        k::KEY_Right => Key::Right,
        k::KEY_Up => Key::Up,
        k::KEY_Down => Key::Down,
        k::KEY_space => Key::Space,
        k::KEY_d | k::KEY_D => Key::D,
        k::KEY_l | k::KEY_L => Key::L,
        k::KEY_q | k::KEY_Q => Key::Q,
        s if (k::KEY_XF86Switch_VT_1..k::KEY_XF86Switch_VT_1 + 12).contains(&s) => Key::Vt((s - k::KEY_XF86Switch_VT_1 + 1) as u8),
        _ => Key::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::seat_key;
    use vergence_seat::keyboard::Key;
    use xkbcommon::xkb::keysyms as k;

    /// Every letter the seat decides on reaches it as that letter, in either
    /// case (Super+L once came through as Other: VDM milestone 1, 2026-11-16).
    #[test]
    fn the_seats_letters_reach_it() {
        for (syms, key) in [((k::KEY_d, k::KEY_D), Key::D), ((k::KEY_l, k::KEY_L), Key::L), ((k::KEY_q, k::KEY_Q), Key::Q)] {
            assert_eq!(seat_key(syms.0, 0), key);
            assert_eq!(seat_key(syms.1, 0), key);
        }
        assert_eq!(seat_key(k::KEY_a, 0), Key::Other);
    }
}
