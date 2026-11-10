// Based on Smithay's smallvil example (MIT; see LICENSE-smithay.txt), adapted for Vergence.
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::FilterResult,
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    utils::SERIAL_COUNTER,
};

use crate::{focus::KeyboardFocus, grabs::resize_grab::ResizeEdge, state::Server};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// What a key does before any app sees it.
enum KeyAction {
    Quit,
    Vt(i32),
    CloseMenu,
}

impl Server {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        // The idle clock (energy.rs). A key or button that only woke the screens
        // goes no further; pointer motion does, so the pointer is where the hand put it.
        if self.energy_activity() && matches!(event, InputEvent::Keyboard { .. } | InputEvent::PointerButton { .. }) {
            return;
        }
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time(&event);

                // Ctrl+Alt+Backspace quits; Ctrl+Alt+Fn switches consoles (stage 2).
                let action = self.seat.get_keyboard().unwrap().input::<KeyAction, _>(
                    self,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |st, mods, handle| {
                        let sym = handle.modified_sym().raw();
                        if event.state() == smithay::backend::input::KeyState::Pressed {
                            if mods.ctrl && mods.alt && sym == smithay::input::keyboard::keysyms::KEY_BackSpace {
                                return FilterResult::Intercept(KeyAction::Quit);
                            }
                            if st.menu_is_open() && sym == smithay::input::keyboard::keysyms::KEY_Escape {
                                return FilterResult::Intercept(KeyAction::CloseMenu);
                            }
                            let vt1 = smithay::input::keyboard::keysyms::KEY_XF86Switch_VT_1;
                            if (vt1..vt1 + 12).contains(&sym) {
                                return FilterResult::Intercept(KeyAction::Vt((sym - vt1 + 1) as i32));
                            }
                        }
                        FilterResult::Forward
                    },
                );
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
                    None => {}
                }
            }
            InputEvent::PointerMotion { event, .. } => {
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

                let serial = SERIAL_COUNTER.next_serial();

                let pointer = self.seat.get_pointer().unwrap();

                let under = self.surface_under(pos);

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerButton { event, .. } => {
                self.handle_button(event.button_code(), event.state(), Event::time(&event));
            }
            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();

                let horizontal_amount = event
                    .amount(Axis::Horizontal)
                    .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.);
                let vertical_amount = event
                    .amount(Axis::Vertical)
                    .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.);
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

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

                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }
}

impl Server {
    /// Raise a window and give it the keyboard (X11 windows: X focus and stacking too).
    pub fn focus_window(&mut self, window: &smithay::desktop::Window, serial: smithay::utils::Serial) {
        self.space.raise_element(window, true);
        let focus = match window.x11_surface() {
            Some(x) => {
                if let Some(wm) = self.xwm.as_mut() {
                    let _ = wm.raise_window(x);
                }
                KeyboardFocus::X11(x.clone())
            }
            None => KeyboardFocus::Wayland(window.toplevel().unwrap().wl_surface().clone()),
        };
        for w in self.space.elements() {
            w.set_activated(w == window);
            if let Some(t) = w.toplevel() {
                t.send_pending_configure();
            }
        }
        if let Some(k) = self.seat.get_keyboard() {
            k.set_focus(self, Some(focus), serial);
        }
    }

    /// The edges to resize from: the window's nearest third in each direction.
    fn nearest_edges(&self, window: &smithay::desktop::Window, at: smithay::utils::Point<f64, smithay::utils::Logical>) -> ResizeEdge {
        let Some(g) = self.space.element_geometry(window) else { return ResizeEdge::BOTTOM_RIGHT };
        let (fx, fy) = ((at.x - g.loc.x as f64) / g.size.w.max(1) as f64, (at.y - g.loc.y as f64) / g.size.h.max(1) as f64);
        let mut e = ResizeEdge::empty();
        if fx < 1.0 / 3.0 {
            e |= ResizeEdge::LEFT;
        } else if fx > 2.0 / 3.0 {
            e |= ResizeEdge::RIGHT;
        }
        if fy < 1.0 / 3.0 {
            e |= ResizeEdge::TOP;
        } else if fy > 2.0 / 3.0 {
            e |= ResizeEdge::BOTTOM;
        }
        if e.is_empty() { ResizeEdge::BOTTOM_RIGHT } else { e }
    }
}

impl Server {
    /// The pointer at `pos` (logical): focus, shape, and motion to the app or grab.
    /// The mouse and the control socket's `pointer` both come here.
    pub fn pointer_to(&mut self, pos: smithay::utils::Point<f64, smithay::utils::Logical>, time: smithay::backend::input::InputTime) {
        let pointer = self.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        self.menu_hover(pos);
        self.notify_hover(pos);
        let under = self.surface_under(pos);
        pointer.motion(self, under, &MotionEvent { location: pos, serial, time });
        pointer.frame(self);
    }

    /// A button pressed or released: title bars, Alt+drag, focus, then the app.
    /// The mouse and the control socket's `press`/`release` both come here.
    pub fn handle_button(&mut self, button: u32, button_state: ButtonState, time: smithay::backend::input::InputTime) {
        let pointer = self.seat.get_pointer().unwrap();
        let keyboard = self.seat.get_keyboard().unwrap();

        let serial = SERIAL_COUNTER.next_serial();

        // The desktop menu takes the clicks while it is open (and the release of the
        // click that opened or used it).
        if button_state == ButtonState::Released && self.menu_swallow == Some(button) {
            self.menu_swallow = None;
            return;
        }
        if button_state == ButtonState::Pressed && self.menu_is_open() {
            let at = pointer.current_location();
            self.menu_press(at);
            self.menu_swallow = Some(button);
            return;
        }
        // Notifications take clicks on themselves (notify.rs).
        if button_state == ButtonState::Released && self.notes.swallow == Some(button) {
            self.notes.swallow = None;
            return;
        }
        if button_state == ButtonState::Pressed && self.notify_at(pointer.current_location()) {
            let at = pointer.current_location();
            self.notify_press(at);
            self.notes.swallow = Some(button);
            return;
        }

        // (window, Some(edges) = resize / None = move)
        let mut grab_with: Option<(smithay::desktop::Window, Option<ResizeEdge>)> = None;
        if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
            use crate::decor::Hit;
            let at = pointer.current_location();
            let hit = self.hit(at);
            let left = button == BTN_LEFT;
            match &hit {
                Hit::Surface(w, ..) | Hit::Title(w) | Hit::Close(w) | Hit::Maximize(w) | Hit::Edge(w, _) => {
                    self.focus_window(&w.clone(), serial)
                }
                Hit::Nothing => {}
            }
            match hit {
                // Alt+left drag moves, Alt+right drag resizes any window.
                Hit::Surface(w, ..) if keyboard.modifier_state().alt && (left || button == BTN_RIGHT) => {
                    let edges = (!left).then(|| self.nearest_edges(&w, at));
                    grab_with = Some((w, edges));
                }
                Hit::Surface(..) => {}
                Hit::Title(w) if left => {
                    let now = std::time::Instant::now();
                    let double = self
                        .last_title_press
                        .as_ref()
                        .is_some_and(|(lw, t)| *lw == w && now.duration_since(*t).as_millis() < 400);
                    if double {
                        self.last_title_press = None;
                        self.toggle_maximize(&w);
                    } else {
                        self.last_title_press = Some((w.clone(), now));
                        grab_with = Some((w, None));
                    }
                }
                Hit::Close(w) if left => self.close_window(&w),
                Hit::Maximize(w) if left => self.toggle_maximize(&w),
                Hit::Edge(w, e) if left => grab_with = Some((w, Some(e))),
                Hit::Title(_) | Hit::Close(_) | Hit::Maximize(_) | Hit::Edge(..) => {}
                // Right-click on the bare desktop: the desktop menu.
                Hit::Nothing if button == BTN_RIGHT => {
                    self.open_menu(at);
                    self.menu_swallow = Some(button);
                    return;
                }
                Hit::Nothing => {
                self.space.elements().for_each(|window| {
                    window.set_activated(false);
                    if let Some(t) = window.toplevel() {
                        t.send_pending_configure();
                    }
                });
                keyboard.set_focus(self, Option::<KeyboardFocus>::None, serial);
                }
            }
        };

        if let Some((window, resize)) = grab_with {
            // The press goes to the grab, not the app. Moving a maximised window
            // takes it out of maximised (back to its size, under the pointer).
            let start = smithay::input::pointer::GrabStartData { focus: None, button, location: pointer.current_location() };
            match resize {
                Some(edges) => self.start_resize(window, edges, Some(start)),
                None => {
                    if let Some(old) = self.restore.get(&window).copied() {
                        self.toggle_maximize(&window);
                        let x = (start.location.x - old.size.w as f64 / 2.0).round() as i32;
                        let y = start.location.y.round() as i32 + crate::decor::TITLE_H / 2;
                        self.space.map_element(window.clone(), (x, y), true);
                        if let Some(xs) = window.x11_surface() {
                            let _ = xs.configure(smithay::utils::Rectangle::new((x, y).into(), old.size));
                        }
                    }
                    self.start_move(window, Some(start));
                }
            }
            return;
        }

        pointer.button(
            self,
            &ButtonEvent {
                button,
                state: button_state,
                serial,
                time: time,
            },
        );
        pointer.frame(self);
    }
}
