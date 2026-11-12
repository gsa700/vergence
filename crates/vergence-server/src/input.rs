// Input from the devices: the idle clock, keys (the key filter, until the seat
// has it in step 3), touch (touch.rs) and the pointer, which goes to the seat
// (dispatch.rs). Begun from Smithay's smallvil example (MIT; see
// LICENSE-smithay.txt); the pointer part is ours since seat milestone 1.
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


/// What a key does before any app sees it.
enum KeyAction {
    Quit,
    Vt(i32),
    CloseMenu,
    /// Super was tapped alone: open or close the applications popup.
    Apps,
    /// A key for the open popup: its keysym and the character it types.
    Popup(u32, Option<char>),
    /// Kept from the apps, with nothing else to do.
    Nothing,
    /// A volume key (sound milestone 1, step 4): +5, -5, or 0 to mute or unmute.
    Volume(i8),
}

impl Server {
    /// A key went down or up: the keyboard, and the control socket's `key` and
    /// `type`, both come here.
    pub fn handle_key(&mut self, code: smithay::input::keyboard::Keycode, state: smithay::backend::input::KeyState, time: smithay::backend::input::InputTime) {
        use smithay::input::keyboard::keysyms;
        let serial = SERIAL_COUNTER.next_serial();
        let pressed = state == smithay::backend::input::KeyState::Pressed;
        // Ctrl+Alt+Backspace quits; Ctrl+Alt+Fn switches consoles (stage 2).
        let action = self.seat.get_keyboard().unwrap().input::<KeyAction, _>(self, code, state, serial, time, |st, mods, handle| {
            let sym = handle.modified_sym().raw();
            let is_super = sym == keysyms::KEY_Super_L || sym == keysyms::KEY_Super_R;
            if pressed {
                if mods.ctrl && mods.alt && sym == keysyms::KEY_BackSpace {
                    return FilterResult::Intercept(KeyAction::Quit);
                }
                if st.menu_is_open() && sym == keysyms::KEY_Escape {
                    return FilterResult::Intercept(KeyAction::CloseMenu);
                }
                // The volume keys, for the default output (sound.rs), never the apps'.
                match sym {
                    keysyms::KEY_XF86AudioRaiseVolume => return FilterResult::Intercept(KeyAction::Volume(5)),
                    keysyms::KEY_XF86AudioLowerVolume => return FilterResult::Intercept(KeyAction::Volume(-5)),
                    keysyms::KEY_XF86AudioMute => return FilterResult::Intercept(KeyAction::Volume(0)),
                    _ => {}
                }
                let vt1 = keysyms::KEY_XF86Switch_VT_1;
                if (vt1..vt1 + 12).contains(&sym) {
                    return FilterResult::Intercept(KeyAction::Vt((sym - vt1 + 1) as i32));
                }
                // Super alone, until anything else is pressed.
                st.launcher.super_alone = is_super;
            } else if is_super && std::mem::take(&mut st.launcher.super_alone) {
                // Super was tapped: the applications popup (launcher.rs).
                return FilterResult::Intercept(KeyAction::Apps);
            }
            // While the popup is open the keyboard is its own.
            if st.launcher_is_open() {
                return FilterResult::Intercept(if pressed && !is_super { KeyAction::Popup(sym, handle.modified_sym().key_char()) } else { KeyAction::Nothing });
            }
            FilterResult::Forward
        });
        match action {
            Some(KeyAction::Quit) => self.loop_signal.stop(),
            Some(KeyAction::CloseMenu) => self.close_menu(),
            Some(KeyAction::Vt(n)) => {
                if let Some(session) = self.session.as_mut() {
                    use smithay::backend::session::Session;
                    if let Err(e) = session.change_vt(n) {
                        tracing::warn!("console {n}: {e:?}");
                    }
                }
            }
            Some(KeyAction::Apps) => self.launcher_toggle(),
            Some(KeyAction::Popup(sym, typed)) => self.launcher_key(sym, typed),
            Some(KeyAction::Volume(step)) => self.volume_key(step),
            Some(KeyAction::Nothing) | None => {}
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
                self.touch_mouse_back();
                // A mouse on the real monitors (stage 2): moved by its delta, kept on the outputs.
                use smithay::backend::input::PointerMotionEvent;
                let pointer = self.seat.get_pointer().unwrap();
                let mut pos = pointer.current_location() + event.delta();
                let mut bounds: Option<smithay::utils::Rectangle<i32, smithay::utils::Logical>> = None;
                for o in self.space.outputs() {
                    if let Some(g) = self.space.output_geometry(o) {
                        bounds = Some(bounds.map_or(g, |b| b.merge(g)));
                    }
                }
                if let Some(b) = bounds {
                    pos.x = pos.x.clamp(b.loc.x as f64, (b.loc.x + b.size.w) as f64 - 1.0);
                    pos.y = pos.y.clamp(b.loc.y as f64, (b.loc.y + b.size.h) as f64 - 1.0);
                }
                let _ = pointer;
                self.pointer_to(pos, Event::time(&event));
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                self.touch_mouse_back();
                // The position in the host window, mapped through the output it is over
                // (1c): its left edge in logical space plus the offset at its scale.
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
                let source = event.source();
                // The lines a notch moves (pointer.rs): every amount scaled alike.
                let k = self.scroll_factor(source);

                let horizontal_amount = k * event
                    .amount(Axis::Horizontal)
                    .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.);
                let vertical_amount = k * event
                    .amount(Axis::Vertical)
                    .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.);
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal).map(|v| v * k);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical).map(|v| v * k);

                let mut frame = AxisFrame::new(event.time()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                // Who scrolls is the seat's: a window (Wayland through the frame,
                // native with the same amounts over the bus) or the open popup (dispatch.rs).
                let wire_source = match source {
                    AxisSource::Wheel => 0,
                    AxisSource::Finger => 1,
                    AxisSource::Continuous => 2,
                    AxisSource::WheelTilt => 3,
                };
                let (v120x, v120y) = (horizontal_amount_discrete.unwrap_or(0.0) as i32, vertical_amount_discrete.unwrap_or(0.0) as i32);
                self.pointer_axis(frame, wire_source, horizontal_amount, vertical_amount, v120x, v120y, Event::time(&event));
            }
            _ => {}
        }
    }
}

impl Server {
    /// Raise a window and give it the keyboard (X11 windows: X focus and stacking too).
    pub fn focus_window(&mut self, window: &crate::win::Win, serial: smithay::utils::Serial) {
        self.space.raise_element(window, true);
        // A native window (native.rs) takes the keyboard as a focus of its own
        // kind (focus.rs): its keys go over the bus. Its keymap is read now,
        // before set_focus, whose callback must not touch the keyboard.
        if window.native().is_some() {
            self.native_prepare_keymap();
        }
        let focus = match (window.x11_surface(), window.toplevel()) {
            (Some(x), _) => {
                if let Some(wm) = self.xwm.as_mut() {
                    let _ = wm.raise_window(x);
                }
                Some(KeyboardFocus::X11(x.clone()))
            }
            (None, Some(t)) => Some(KeyboardFocus::Wayland(t.wl_surface().clone())),
            (None, None) => window.native().map(|_| KeyboardFocus::Native(window.clone())),
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
    }
}
