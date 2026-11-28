//! vergence-web: WPE WebKit behind a platform of our own on Vergence's
//! protocol (docs/web-milestone-1.md).
//!
//! Milestone 1 shows one page in a plain window (`vergence-web-view`); the
//! service process that places web views inside other programs' windows,
//! over an engine-neutral protocol, is milestone 2 (the plan's "component's
//! shape"). Nothing here names WebKit to the window code but this crate.
//!
//! Start reading: platform.rs (our WPE display, view and toplevel), then
//! ffi.rs (the calls into WebKit), then bin/vergence-web-view.rs.
//!
//! Rule: WebKit is loaded at run time, never linked (ffi.rs); every call
//! into it happens on the thread that runs GLib's main loop.

mod ffi;
mod platform;

use std::ffi::{CStr, CString, c_int, c_uint};

pub use ffi::LIB_NAME;
use ffi::{GPointer, wpe_enum};
use platform::lib;
pub use platform::{Frame, GpuFrame, Host, Platform, on_gpu};

/// One web view and its WPE view.
pub struct WebView {
    web: GPointer,
    view: GPointer,
    /// The keyboard's modifiers, as WPE counts them.
    modifiers: u32,
    pointer: (f64, f64),
}

/// A pointer button in Vergence's terms (evdev's BTN_LEFT, BTN_RIGHT,
/// BTN_MIDDLE) as WPE numbers them (GDK's: 1 primary, 2 middle, 3 secondary).
pub fn wpe_button(evdev: u32) -> Option<u32> {
    match evdev {
        0x110 => Some(1),
        0x111 => Some(3),
        0x112 => Some(2),
        0x113 => Some(8),
        0x114 => Some(9),
        _ => None,
    }
}

/// xkb's modifier masks (Shift, Lock, Control, Mod1 = Alt, Mod4 = Super) as WPE's.
pub fn wpe_modifiers(xkb: u32) -> u32 {
    let mut m = 0;
    if xkb & 1 != 0 {
        m |= wpe_enum::MODIFIER_SHIFT;
    }
    if xkb & 2 != 0 {
        m |= wpe_enum::MODIFIER_CAPS_LOCK;
    }
    if xkb & 4 != 0 {
        m |= wpe_enum::MODIFIER_CONTROL;
    }
    if xkb & 8 != 0 {
        m |= wpe_enum::MODIFIER_ALT;
    }
    if xkb & 64 != 0 {
        m |= wpe_enum::MODIFIER_META;
    }
    m
}

/// A wheel's turn as WPE's scroll steps: one notch is one step, exactly, as
/// everywhere in Vergence (smooth scrolling is off in WebKit's settings);
/// down and right are negative, as WPE's Wayland platform sends them.
pub fn wheel_steps(v120: i32) -> f64 {
    -(v120 as f64) / 120.0
}

/// The size of a frame in pixels for a window of `w` by `h` points at `scale`.
pub fn pixels(w: f64, h: f64, scale: f64) -> (u32, u32) {
    ((w * scale).round().max(1.0) as u32, (h * scale).round().max(1.0) as u32)
}

impl WebView {
    fn new(web: GPointer) -> WebView {
        // SAFETY: a WebKitWebView just made on our display.
        let view = unsafe { (lib().webkit_web_view_get_wpe_view)(web) };
        WebView { web, view, modifiers: 0, pointer: (0.0, 0.0) }
    }

    pub fn load(&self, uri: &str) {
        let Ok(u) = CString::new(uri) else { return };
        // SAFETY: the view and a C string.
        unsafe { (lib().webkit_web_view_load_uri)(self.web, u.as_ptr()) };
    }

    /// The page's title, if it has one.
    pub fn title(&self) -> Option<String> {
        // SAFETY: the view's own string, read at once.
        let t = unsafe { (lib().webkit_web_view_get_title)(self.web) };
        (!t.is_null()).then(|| unsafe { CStr::from_ptr(t) }.to_string_lossy().into_owned())
    }

    /// The window's size in points and its scale, from the server.
    pub fn configure(&self, width: f64, height: f64, scale: f64) {
        let lib = lib();
        let (w, h) = (width.round().max(1.0) as c_int, height.round().max(1.0) as c_int);
        // SAFETY: our view and its toplevel, when it has one.
        unsafe {
            let top = (lib.wpe_view_get_toplevel)(self.view);
            if !top.is_null() {
                (lib.wpe_toplevel_scale_changed)(top, scale);
                (lib.wpe_toplevel_resized)(top, w, h);
            }
            (lib.wpe_view_resized)(self.view, w, h);
        }
    }

    /// The server showed the last frame.
    pub fn frame_done(&self) {
        platform::frame_done(self.view);
    }

    pub fn focus(&self, focused: bool) {
        let lib = lib();
        // SAFETY: our view.
        unsafe { if focused { (lib.wpe_view_focus_in)(self.view) } else { (lib.wpe_view_focus_out)(self.view) } }
    }

    fn send(&self, event: GPointer) {
        if event.is_null() {
            return;
        }
        let lib = lib();
        // SAFETY: an event made for our view; WPE takes its own reference.
        unsafe {
            (lib.wpe_view_event)(self.view, event);
            (lib.wpe_event_unref)(event);
        }
    }

    pub fn pointer_enter(&mut self, time: u32, x: f64, y: f64) {
        self.pointer = (x, y);
        // SAFETY: WPE's event constructor with our view.
        let e = unsafe {
            (lib().wpe_event_pointer_move_new)(
                wpe_enum::EVENT_POINTER_ENTER,
                self.view,
                wpe_enum::INPUT_SOURCE_MOUSE,
                time,
                self.modifiers,
                x,
                y,
                0.0,
                0.0,
            )
        };
        self.send(e);
    }

    pub fn pointer_leave(&mut self, time: u32) {
        let (x, y) = self.pointer;
        // SAFETY: as above.
        let e = unsafe {
            (lib().wpe_event_pointer_move_new)(
                wpe_enum::EVENT_POINTER_LEAVE,
                self.view,
                wpe_enum::INPUT_SOURCE_MOUSE,
                time,
                self.modifiers,
                x,
                y,
                0.0,
                0.0,
            )
        };
        self.send(e);
    }

    pub fn pointer_motion(&mut self, time: u32, x: f64, y: f64) {
        let (dx, dy) = (x - self.pointer.0, y - self.pointer.1);
        self.pointer = (x, y);
        // SAFETY: as above.
        let e = unsafe {
            (lib().wpe_event_pointer_move_new)(
                wpe_enum::EVENT_POINTER_MOVE,
                self.view,
                wpe_enum::INPUT_SOURCE_MOUSE,
                time,
                self.modifiers,
                x,
                y,
                dx,
                dy,
            )
        };
        self.send(e);
    }

    pub fn pointer_button(&mut self, time: u32, evdev: u32, pressed: bool) {
        let Some(button) = wpe_button(evdev) else { return };
        let bit = wpe_enum::MODIFIER_BUTTON1 << (button.min(5) - 1);
        let (x, y) = self.pointer;
        let kind = if pressed { wpe_enum::EVENT_POINTER_DOWN } else { wpe_enum::EVENT_POINTER_UP };
        // Pressed: the button counts as held in this event; released: no longer.
        let mods = if pressed { self.modifiers | bit } else { self.modifiers & !bit };
        // SAFETY: as above.
        let e = unsafe {
            (lib().wpe_event_pointer_button_new)(
                kind,
                self.view,
                wpe_enum::INPUT_SOURCE_MOUSE,
                time,
                mods,
                button as c_uint,
                x,
                y,
                if pressed { 1 } else { 0 },
            )
        };
        self.modifiers = mods;
        self.send(e);
    }

    /// Scrolling: a wheel by notches (`v120`), a touchpad by `dx`, `dy` points.
    pub fn scroll(&mut self, time: u32, source: u8, dx: f64, dy: f64, v120x: i32, v120y: i32) {
        let (x, y) = self.pointer;
        let wheel = source == 0 || source == 3;
        let (sx, sy, precise, src) = if wheel {
            (wheel_steps(v120x), wheel_steps(v120y), 0, wpe_enum::INPUT_SOURCE_MOUSE)
        } else {
            (-dx, -dy, 1, wpe_enum::INPUT_SOURCE_TOUCHPAD)
        };
        if sx == 0.0 && sy == 0.0 {
            return;
        }
        // SAFETY: as above.
        let e = unsafe { (lib().wpe_event_scroll_new)(self.view, src, time, self.modifiers, sx, sy, precise, 0, x, y) };
        self.send(e);
    }

    pub fn modifiers(&mut self, depressed: u32, locked: u32) {
        let buttons = self.modifiers & !0xff;
        self.modifiers = buttons | wpe_modifiers(depressed | (locked & 2));
    }

    /// A key, by its evdev code (xkb's keycode is 8 more).
    pub fn key(&mut self, time: u32, evdev: u32, pressed: bool) {
        let lib = lib();
        let keycode = evdev + 8;
        let mut keyval: c_uint = 0;
        // SAFETY: the display's keymap (WPE's own xkb one) and out-pointers.
        unsafe {
            let keymap = platform_keymap();
            if !keymap.is_null() {
                (lib.wpe_keymap_translate_keyboard_state)(
                    keymap,
                    keycode,
                    self.modifiers,
                    0,
                    &mut keyval,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
            }
        }
        let kind = if pressed { wpe_enum::EVENT_KEYBOARD_KEY_DOWN } else { wpe_enum::EVENT_KEYBOARD_KEY_UP };
        // SAFETY: as above.
        let e = unsafe {
            (lib.wpe_event_keyboard_new)(
                kind,
                self.view,
                wpe_enum::INPUT_SOURCE_KEYBOARD,
                time,
                self.modifiers,
                keycode,
                keyval,
            )
        };
        self.send(e);
    }
}

fn platform_keymap() -> GPointer {
    // SAFETY: our own display vfunc; it takes no display state.
    unsafe { platform::display_keymap() }
}

/// Runs GLib's main loop; `on_fd(true)` is called whenever `fd` (the
/// server's socket) is readable, and `on_fd(false)` once at the start for
/// what was read already (a request's wait queues the events it passed).
/// It returns false to stop the loop.
pub fn run(fd: std::os::fd::RawFd, mut on_fd: Box<dyn FnMut(bool) -> bool>) {
    let lib = lib();
    /// The callback and the loop it may stop.
    type OnFd = Option<(Box<dyn FnMut(bool) -> bool>, GPointer)>;
    thread_local! {
        static ON_FD: std::cell::RefCell<OnFd> = const { std::cell::RefCell::new(None) };
    }
    unsafe extern "C" fn ready(_: c_int, cond: c_uint, _: GPointer) -> ffi::Gboolean {
        let keep = ON_FD.with(|f| {
            let mut f = f.borrow_mut();
            let Some((cb, lp)) = f.as_mut() else { return false };
            let keep = cond & (ffi::G_IO_HUP | ffi::G_IO_ERR) == 0 && cb(true);
            if !keep {
                // SAFETY: the loop made below.
                unsafe { (platform::lib().g_main_loop_quit)(*lp) };
            }
            keep
        });
        keep as ffi::Gboolean
    }
    // SAFETY: GLib's default context; our callback lives in ON_FD until the loop ends.
    if !on_fd(false) {
        return;
    }
    unsafe {
        let lp = (lib.g_main_loop_new)(std::ptr::null_mut(), 0);
        ON_FD.with(|f| *f.borrow_mut() = Some((on_fd, lp)));
        (lib.g_unix_fd_add)(fd, ffi::G_IO_IN | ffi::G_IO_HUP | ffi::G_IO_ERR, ready, std::ptr::null_mut());
        (lib.g_main_loop_run)(lp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons() {
        assert_eq!(wpe_button(0x110), Some(1));
        assert_eq!(wpe_button(0x111), Some(3));
        assert_eq!(wpe_button(0x112), Some(2));
        assert_eq!(wpe_button(0x200), None);
    }

    #[test]
    fn modifiers() {
        assert_eq!(wpe_modifiers(1 | 4), wpe_enum::MODIFIER_SHIFT | wpe_enum::MODIFIER_CONTROL);
        assert_eq!(wpe_modifiers(8 | 64), wpe_enum::MODIFIER_ALT | wpe_enum::MODIFIER_META);
        assert_eq!(wpe_modifiers(0), 0);
    }

    #[test]
    fn a_notch_is_one_step() {
        assert_eq!(wheel_steps(120), -1.0);
        assert_eq!(wheel_steps(-240), 2.0);
    }

    #[test]
    fn frame_pixels() {
        assert_eq!(pixels(800.0, 600.0, 1.0), (800, 600));
        assert_eq!(pixels(800.0, 600.0, 1.3333), (1067, 800));
    }
}
