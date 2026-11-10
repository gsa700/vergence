//! Keys into characters.
//!
//! The server sends the keyboard's map as xkb text and then bare key
//! numbers; turning a key into a symbol and the text it types, under the
//! modifiers held, is xkbcommon's work (the system library, borrowed,
//! whitepaper §8). It is called through declarations of our own, not a
//! crate. Dead keys and compose sequences are not handled yet.

use std::ffi::{CString, c_char, c_void};

#[link(name = "xkbcommon")]
unsafe extern "C" {
    fn xkb_context_new(flags: i32) -> *mut c_void;
    fn xkb_context_unref(context: *mut c_void);
    fn xkb_keymap_new_from_string(context: *mut c_void, string: *const c_char, format: i32, flags: i32) -> *mut c_void;
    fn xkb_keymap_new_from_names(context: *mut c_void, names: *const c_void, flags: i32) -> *mut c_void;
    fn xkb_keymap_unref(keymap: *mut c_void);
    fn xkb_keymap_key_repeats(keymap: *mut c_void, key: u32) -> i32;
    fn xkb_state_new(keymap: *mut c_void) -> *mut c_void;
    fn xkb_state_unref(state: *mut c_void);
    fn xkb_state_update_mask(state: *mut c_void, depressed: u32, latched: u32, locked: u32, depressed_layout: u32, latched_layout: u32, locked_layout: u32) -> i32;
    fn xkb_state_key_get_one_sym(state: *mut c_void, key: u32) -> u32;
    fn xkb_state_key_get_utf8(state: *mut c_void, key: u32, buffer: *mut c_char, size: usize) -> i32;
    fn xkb_state_mod_name_is_active(state: *mut c_void, name: *const c_char, kind: i32) -> i32;
}

const KEYMAP_FORMAT_TEXT_V1: i32 = 1;
const STATE_MODS_EFFECTIVE: i32 = 1 << 3;
/// xkb numbers keys 8 above the kernel's codes, which the server sends.
const EVDEV_OFFSET: u32 = 8;

/// Key symbols a program is likely to ask about (X11's numbers, which xkb uses).
pub mod sym {
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const RETURN: u32 = 0xff0d;
    pub const ESCAPE: u32 = 0xff1b;
    pub const HOME: u32 = 0xff50;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const END: u32 = 0xff57;
    pub const DELETE: u32 = 0xffff;
}

/// The modifier keys held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// The Super (logo) key.
    pub logo: bool,
}

pub struct Keyboard {
    context: *mut c_void,
    keymap: *mut c_void,
    state: *mut c_void,
}

impl Keyboard {
    /// From the map the server sent. None if it is not a map xkb accepts.
    pub fn from_text(text: &[u8]) -> Option<Keyboard> {
        // The text ends at its first zero byte, as the C library reads it.
        let text = CString::new(&text[..text.iter().position(|b| *b == 0).unwrap_or(text.len())]).ok()?;
        Keyboard::build(|context| {
            // SAFETY: a live context and a valid C string.
            unsafe { xkb_keymap_new_from_string(context, text.as_ptr(), KEYMAP_FORMAT_TEXT_V1, 0) }
        })
    }

    /// The system's default layout, for tests and for a server that sends no map.
    pub fn default_layout() -> Option<Keyboard> {
        // SAFETY: a live context; no names means the defaults.
        Keyboard::build(|context| unsafe { xkb_keymap_new_from_names(context, std::ptr::null(), 0) })
    }

    fn build(keymap: impl FnOnce(*mut c_void) -> *mut c_void) -> Option<Keyboard> {
        // SAFETY: plain constructors; every result is checked and released on failure.
        unsafe {
            let context = xkb_context_new(0);
            if context.is_null() {
                return None;
            }
            let keymap = keymap(context);
            if keymap.is_null() {
                xkb_context_unref(context);
                return None;
            }
            let state = xkb_state_new(keymap);
            if state.is_null() {
                xkb_keymap_unref(keymap);
                xkb_context_unref(context);
                return None;
            }
            Some(Keyboard { context, keymap, state })
        }
    }

    /// The modifier and layout state the server reported.
    pub fn update(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        // SAFETY: a live state.
        unsafe { xkb_state_update_mask(self.state, depressed, latched, locked, 0, 0, group) };
    }

    pub fn modifiers(&self) -> Modifiers {
        // SAFETY: a live state and valid C strings.
        let held = |name: &std::ffi::CStr| unsafe { xkb_state_mod_name_is_active(self.state, name.as_ptr(), STATE_MODS_EFFECTIVE) } > 0;
        Modifiers { shift: held(c"Shift"), ctrl: held(c"Control"), alt: held(c"Mod1"), logo: held(c"Mod4") }
    }

    /// What a key (the kernel's code, as the server sends it) is under the
    /// present modifiers: its symbol, and the text it types, if it types any.
    pub fn key(&self, key: u32) -> (u32, Option<String>) {
        let code = key.saturating_add(EVDEV_OFFSET);
        let mut buffer = [0 as c_char; 64];
        // SAFETY: a live state; the buffer's true size is given, and the library ends what it writes with a zero.
        let (symbol, len) = unsafe { (xkb_state_key_get_one_sym(self.state, code), xkb_state_key_get_utf8(self.state, code, buffer.as_mut_ptr(), buffer.len())) };
        let len = (len.max(0) as usize).min(buffer.len() - 1);
        let bytes: Vec<u8> = buffer[..len].iter().map(|b| *b as u8).collect();
        // Text is what would be typed: Enter, Escape and Ctrl+letter are keys, not text.
        let text = String::from_utf8(bytes).ok().filter(|t| !t.is_empty() && !t.chars().any(char::is_control));
        (symbol, text)
    }

    /// Whether holding the key repeats it.
    pub fn repeats(&self, key: u32) -> bool {
        // SAFETY: a live keymap.
        unsafe { xkb_keymap_key_repeats(self.keymap, key.saturating_add(EVDEV_OFFSET)) > 0 }
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        // SAFETY: the three objects made in build(), released once, in reverse order.
        unsafe {
            xkb_state_unref(self.state);
            xkb_keymap_unref(self.keymap);
            xkb_context_unref(self.context);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The kernel's codes for a few keys.
    const KEY_A: u32 = 30;
    const KEY_1: u32 = 2;
    const KEY_ENTER: u32 = 28;
    const KEY_ESC: u32 = 1;
    const KEY_LEFTSHIFT: u32 = 42;
    const KEY_LEFT: u32 = 105;

    fn keyboard() -> Keyboard {
        Keyboard::default_layout().expect("the system has a default keyboard layout")
    }

    #[test]
    fn keys_type_what_the_layout_says() {
        let mut k = keyboard();
        assert_eq!(k.key(KEY_A), ('a' as u32, Some("a".into())));
        assert_eq!(k.key(KEY_1), ('1' as u32, Some("1".into())));
        assert_eq!(k.modifiers(), Modifiers::default());
        // Shift held: xkb's first modifier bit.
        k.update(1, 0, 0, 0);
        assert!(k.modifiers().shift && !k.modifiers().ctrl);
        assert_eq!(k.key(KEY_A), ('A' as u32, Some("A".into())));
        assert_eq!(k.key(KEY_1).1, Some("!".into()));
        // Control held (the third bit): a key, not text.
        k.update(4, 0, 0, 0);
        assert!(k.modifiers().ctrl);
        assert_eq!(k.key(KEY_A).1, None);
        // Caps Lock locked (the second bit): letters, not digits.
        k.update(0, 0, 2, 0);
        assert_eq!(k.key(KEY_A).1, Some("A".into()));
        assert_eq!(k.key(KEY_1).1, Some("1".into()));
    }

    #[test]
    fn keys_that_are_not_text() {
        let k = keyboard();
        assert_eq!(k.key(KEY_ENTER), (sym::RETURN, None));
        assert_eq!(k.key(KEY_ESC), (sym::ESCAPE, None));
        assert_eq!(k.key(KEY_LEFT), (sym::LEFT, None));
        assert_eq!(k.key(KEY_LEFTSHIFT).1, None);
        assert!(k.repeats(KEY_A) && k.repeats(KEY_LEFT));
        assert!(!k.repeats(KEY_LEFTSHIFT), "a modifier does not repeat");
        // A key number no keyboard has.
        assert_eq!(k.key(60_000).1, None);
        assert_eq!(k.key(u32::MAX).1, None);
    }

    #[test]
    fn a_map_that_is_not_one_is_refused() {
        assert!(Keyboard::from_text(b"this is not a keymap").is_none());
        assert!(Keyboard::from_text(b"").is_none());
        assert!(Keyboard::from_text(&[0xff, 0xfe, 0, 1, 2]).is_none());
    }
}
