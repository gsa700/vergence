// Input from the devices (seat milestone 1): the idle clock first; then keys,
// through our own xkb state (keys.rs) and the seat's key filter; keyboard
// focus, which the seat is told of; touch, put on its panel's screen
// (touch.rs); and the pointer, kept on the screens and handed to the seat
// (dispatch.rs).
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, Event, InputBackend, InputEvent,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::FilterResult,
        pointer::AxisFrame,
    },
    utils::SERIAL_COUNTER,
};

use crate::{focus::KeyboardFocus, state::Server};


impl Server {
    /// A key went down or up: the keyboard, and the control socket's `key` and
    /// `type`, both come here. Our own xkb state says what the key is (keys.rs),
    /// the seat says whose it is (vergence-seat's key filter), and then:
    /// Smithay's keyboard is fed it in every case, so its state stays right,
    /// and delivers it only to a Wayland or X11 window that has the keyboard;
    /// a native window has it from us over the bus.
    pub fn handle_key(&mut self, code: smithay::input::keyboard::Keycode, state: smithay::backend::input::KeyState, time: smithay::backend::input::InputTime) {
        use vergence_seat::keyboard::KeyOut;
        let pressed = state == smithay::backend::input::KeyState::Pressed;
        // The seat knows which menu is open (the popup may have opened from the keyboard).
        self.dispatch_sync();
        self.keys_ready();
        let Some(keys) = self.input.keys.as_mut() else { return };
        let key = keys.feed(code, pressed);
        let mods = keys.mods();
        // The log out dialog takes every key while it is open (session_dialog.rs).
        if self.session_dialog.open && !self.lock.on {
            if let Some(k) = self.seat.get_keyboard() {
                k.input::<(), _>(self, code, state, SERIAL_COUNTER.next_serial(), time, |_, _, _| FilterResult::Intercept(()));
            }
            return self.session_dialog_key(key.sym, pressed);
        }
        // Escape during a drag on the dock (an icon lifted, the dock dragged or
        // "Move dock"): everything goes back, and no program hears the key.
        if pressed && !self.lock.on && self.shelf_busy() && crate::keys::seat_key(key.sym, code.raw()) == vergence_seat::keyboard::Key::Escape {
            if let Some(k) = self.seat.get_keyboard() {
                k.input::<(), _>(self, code, state, SERIAL_COUNTER.next_serial(), time, |_, _, _| FilterResult::Intercept(()));
            }
            self.shelf_cancel();
            return;
        }
        let out = self.input.seat.key(crate::keys::seat_key(key.sym, code.raw()), pressed, mods);
        let native = match out {
            KeyOut::Forward(Some(id)) => self.input.window(id).filter(|w| w.native().is_some()),
            _ => None,
        };
        let to_smithay = matches!(out, KeyOut::Forward(_)) && native.is_none();
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(k) = self.seat.get_keyboard() {
            k.input::<(), _>(self, code, state, serial, time, |_, _, _| if to_smithay { FilterResult::Forward } else { FilterResult::Intercept(()) });
        }
        // Ctrl pressed while a window is moved: with the "ctrl" setting it may now
        // snap, and the preview shows without waiting for the pointer to move.
        if key.mods_changed && mods.ctrl && self.is_moving() {
            let at = self.pointer_location();
            self.snap_track(at);
        }
        // Locked, no window is told the modifiers either (review I6).
        if key.mods_changed
            && !self.lock.on
            && let Some(n) = self.input.keyboard_native.clone()
        {
            self.native_modifiers(&n);
        }
        match out {
            KeyOut::Forward(target) => {
                if pressed && target.is_some() {
                    self.input.typed_at = Some(std::time::Instant::now());
                }
                if let Some(w) = native {
                    self.native_key(&w, code.raw(), pressed, serial.into(), time.millis());
                }
            }
            KeyOut::Quit => self.loop_signal.stop(),
            KeyOut::ForceQuit => {
                self.app_menu_close();
                self.close_menu();
                self.launcher_close();
                if let Err(e) = crate::spawn(&mut std::process::Command::new("vergence-quit")) {
                    tracing::warn!("force quit: vergence-quit: {e}");
                }
            }
            KeyOut::CloseMenu => self.close_menu(),
            KeyOut::Vt(n) => {
                if let Some(session) = self.session.as_mut() {
                    use smithay::backend::session::Session;
                    if let Err(e) = session.change_vt(n as i32) {
                        tracing::warn!("console {n}: {e:?}");
                    }
                }
            }
            KeyOut::Apps => self.launcher_toggle(),
            KeyOut::Popup => self.launcher_key(key.sym, key.typed),
            KeyOut::AppMenu => self.app_menu_key(key.sym, key.typed),
            KeyOut::OpenAppMenu(id) => {
                if let Some(w) = self.input.window(id) {
                    self.app_menu_toggle(&w, true);
                }
            }
            KeyOut::Volume(step) => self.volume_key(step),
            KeyOut::Switch(s) => self.switch_key(s),
            // Super with an arrow (snap.rs).
            KeyOut::Window(k) => self.window_key(k),
            KeyOut::ShowDesktop => self.show_desktop(),
            // The lock screen (lock.rs).
            KeyOut::Lock => {
                self.lock_request("Super+L");
            }
            KeyOut::Locked if crate::greeter() => self.greeter_key(key.sym, key.typed, pressed),
            KeyOut::Locked => self.lock_key(key.sym, key.typed, pressed),
            KeyOut::Swallow => {}
        }
    }

    /// Our xkb state, from Smithay's keymap, before the first key (not inside
    /// any keyboard callback: this runs at the top of handle_key and focus_window).
    pub fn keys_ready(&mut self) {
        if self.input.keys.is_some() {
            return;
        }
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        let text = keyboard.with_xkb_state(self, |ctx| ctx.xkb().lock().unwrap().keymap_as_string());
        self.input.keys = crate::keys::Keys::from_text(text);
        if self.input.keys.is_none() {
            tracing::error!("keyboard: the keymap could not be read back");
        }
    }

    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        // The idle clock (energy.rs). A key or button that only woke the screens
        // goes no further; pointer motion does, so the pointer is where the hand put it.
        // Only what a person does counts: a device coming or going is not input
        // (2026-11-11, Hubble: the 14" monitor's USB touchscreen dropped off when the
        // screens went off and came back ~14 s later, waking them every 10 min all night).
        let by_hand = !matches!(event, InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. } | InputEvent::TouchCancel { .. } | InputEvent::Special(_));
        if by_hand && self.energy_activity() && matches!(event, InputEvent::Keyboard { .. } | InputEvent::PointerButton { .. } | InputEvent::TouchDown { .. }) {
            return;
        }
        match event {
            InputEvent::Keyboard { event, .. } => self.handle_key(event.key_code(), event.state(), Event::time(&event)),
            // Touch (touch.rs): where on its panel a finger is, put on the panel's screen.
            InputEvent::TouchDown { event, .. } => {
                use smithay::backend::input::{Device, TouchEvent};
                if let Some(screen) = self.touch_screen(&event.device().name()) {
                    let at = event.position_transformed(screen.size) + screen.loc.to_f64();
                    self.touch_down(event.slot(), at, Event::time(&event));
                }
            }
            InputEvent::TouchMotion { event, .. } => {
                use smithay::backend::input::{Device, TouchEvent};
                if let Some(screen) = self.touch_screen(&event.device().name()) {
                    let at = event.position_transformed(screen.size) + screen.loc.to_f64();
                    self.touch_motion(event.slot(), at, Event::time(&event));
                }
            }
            InputEvent::TouchUp { event, .. } => {
                use smithay::backend::input::TouchEvent;
                self.touch_up(event.slot(), Event::time(&event));
            }
            InputEvent::TouchCancel { event, .. } => self.touch_cancel(Event::time(&event)),
            InputEvent::TouchFrame { .. } => self.touch_frame(),
            InputEvent::PointerMotion { event, .. } => {
                // A mouse or touchpad on the real monitors: moved by its delta, kept on the screens.
                use smithay::backend::input::PointerMotionEvent;
                self.touch_mouse_back();
                let pos = self.on_screens(self.pointer_location() + event.delta());
                self.pointer_to(pos, Event::time(&event));
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                self.touch_mouse_back();
                // On the real monitors, an absolute device (a drawing tablet, a touch
                // panel left in mouse mode, a VM's mouse) covers one screen: the one
                // desktop.toml ties it to, else the primary, as a touch panel does
                // (touch.rs). It went through the nested mapping below until
                // 2026-11-12 and landed nowhere useful (kepler's panel in mouse mode).
                if self.backend.is_some() {
                    use smithay::backend::input::Device;
                    if let Some(screen) = self.touch_screen(&event.device().name()) {
                        let pos = event.position_transformed(screen.size) + screen.loc.to_f64();
                        self.pointer_to(pos, Event::time(&event));
                    }
                    return;
                }
                // Nested: the position in the host window, mapped through the output it is
                // over (1c): its left edge in logical space plus the offset at its scale.
                let w = self.window_px;
                let px = event.position_transformed((w.w, w.h).into());
                let pos = self
                    .regions
                    .iter()
                    .find(|r| px.x >= r.window.loc.x && px.x < r.window.loc.x + r.window.size.w)
                    .or(self.regions.last())
                    .map(|r| {
                        smithay::utils::Point::<f64, smithay::utils::Logical>::from((
                            r.logical_x + (px.x - r.window.loc.x) / r.scale,
                            px.y / r.scale,
                        ))
                    })
                    .unwrap_or(px);

                // The same path as the mouse on real monitors: hover effects and
                // native windows too (native.rs).
                self.pointer_to(pos, Event::time(&event));
            }
            InputEvent::PointerButton { event, .. } => {
                self.touch_mouse_back();
                self.handle_button(event.button_code(), event.state(), Event::time(&event));
            }
            InputEvent::PointerAxis { event, .. } => {
                // A wheel notch or a touchpad's scroll, each axis scaled by the
                // lines a notch moves (pointer.rs). A wheel that gives no smooth
                // amount moves 15 a notch (libinput's v120 counts 120 a notch).
                let source = event.source();
                // Only a touchpad scrolls with fingers (inputs.rs).
                if source == AxisSource::Finger {
                    self.inputs_touchpad_used();
                }
                let k = self.scroll_factor(source);
                let amount = |axis| k * event.amount(axis).unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0);
                let notches = |axis| event.amount_v120(axis).map(|v| v * k);
                let mut frame = AxisFrame::new(event.time()).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    if amount(axis) != 0.0 {
                        frame = frame.value(axis, amount(axis));
                        if let Some(n) = notches(axis) {
                            frame = frame.v120(axis, n as i32);
                        }
                    }
                    // Fingers lifted off the touchpad: that axis's scroll ends (kinetic scrolling stops there).
                    if source == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                        frame = frame.stop(axis);
                    }
                }
                // Who scrolls is the seat's: a Wayland window gets the frame, a
                // native one the same amounts over the bus, the open popup its grid (dispatch.rs).
                let wire = match source {
                    AxisSource::Wheel => 0,
                    AxisSource::Finger => 1,
                    AxisSource::Continuous => 2,
                    AxisSource::WheelTilt => 3,
                };
                let v120 = |axis| notches(axis).unwrap_or(0.0) as i32;
                self.pointer_axis(frame, wire, amount(Axis::Horizontal), amount(Axis::Vertical), v120(Axis::Horizontal), v120(Axis::Vertical), Event::time(&event));
            }
            _ => {}
        }
    }
}

impl Server {
    /// Raise a window and give it the keyboard (X11 windows: X focus and
    /// stacking too). A Wayland or X11 window has it through Smithay's
    /// keyboard; a native window from us (native.rs), Smithay's focus being
    /// none then. The seat is told either way.
    pub fn focus_window(&mut self, window: &crate::win::Win, serial: smithay::utils::Serial) {
        // Locked, no window takes the keyboard (review I6); the lock gives it back.
        if self.lock.on {
            return;
        }
        // A parent a modal dialog blocks gives the keyboard to the dialog (dialogs.rs).
        if let Some(modal) = self.modal_of(window)
            && modal != *window
        {
            self.space.raise_element(window, true);
            return self.focus_window(&modal, serial);
        }
        // The desktop's icons have the keyboard but stay under every window (desktop.rs).
        if !self.is_desktop(window) {
            self.space.raise_element(window, true);
        }
        // Its dialogs and palettes come forward with it.
        for c in self.children_of(window) {
            if self.space.element_geometry(&c).is_some() {
                self.space.raise_element(&c, false);
            }
        }
        self.keys_ready();
        let focus = match (window.x11_surface(), window.toplevel()) {
            (Some(x), _) => {
                if let Some(wm) = self.xwm.as_mut() {
                    let _ = wm.raise_window(x);
                }
                Some(KeyboardFocus::X11(x.clone()))
            }
            (None, Some(t)) => Some(KeyboardFocus::Wayland(t.wl_surface().clone())),
            (None, None) => None,
        };
        for w in self.space.elements() {
            w.set_activated(w == window);
            if let Some(t) = w.toplevel() {
                t.send_pending_configure();
            }
        }
        if let Some(k) = self.seat.get_keyboard() {
            k.set_focus(self, focus, serial);
        }
        let native = window.native().is_some().then(|| window.clone());
        if self.input.keyboard_native != native {
            if let Some(old) = self.input.keyboard_native.take() {
                self.native_keyboard_leave(&old);
            }
            if let Some(new) = native {
                self.native_keyboard_enter(&new, serial.into());
                self.native_modifiers(&new);
                self.input.keyboard_native = Some(new);
            }
        }
        let id = self.win_id(window);
        self.input.seat.focus(Some(id));
        // Palettes: only their application's, on top (dialogs.rs).
        self.palettes_follow_focus(Some(window));
        // It has had the keyboard: its dock icon need ask for nothing (opened.rs).
        self.shelf.attention.retain(|w| w != window);
    }

    /// No window has the keyboard (a press on the bare desktop, the last
    /// window minimized).
    pub fn unfocus_all(&mut self) {
        self.space.elements().for_each(|w| {
            w.set_activated(false);
            if let Some(t) = w.toplevel() {
                t.send_pending_configure();
            }
        });
        if let Some(k) = self.seat.get_keyboard() {
            k.set_focus(self, Option::<KeyboardFocus>::None, SERIAL_COUNTER.next_serial());
        }
        if let Some(old) = self.input.keyboard_native.take() {
            self.native_keyboard_leave(&old);
        }
        self.input.seat.focus(None);
        self.palettes_follow_focus(None);
    }

    /// A point kept on the screens (the pointer cannot leave the desktop).
    fn on_screens(&self, mut pos: smithay::utils::Point<f64, smithay::utils::Logical>) -> smithay::utils::Point<f64, smithay::utils::Logical> {
        let all = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).reduce(|a, b| a.merge(b));
        if let Some(b) = all {
            pos.x = pos.x.clamp(b.loc.x as f64, (b.loc.x + b.size.w) as f64 - 1.0);
            pos.y = pos.y.clamp(b.loc.y as f64, (b.loc.y + b.size.h) as f64 - 1.0);
        }
        pos
    }

    /// The window that has the keyboard, any kind.
    pub fn keyboard_window(&self) -> Option<crate::win::Win> {
        self.input.seat.keyboard_focus().and_then(|id| self.input.window(id)).filter(|w| smithay::utils::IsAlive::alive(w))
    }
}
