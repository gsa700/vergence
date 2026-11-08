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
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::SERIAL_COUNTER,
};

use crate::state::Server;

/// What a key does before any app sees it.
enum KeyAction {
    Quit,
    Vt(i32),
}

impl Server {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
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
                    |_, mods, handle| {
                        let sym = handle.modified_sym().raw();
                        if event.state() == smithay::backend::input::KeyState::Pressed {
                            if mods.ctrl && mods.alt && sym == smithay::input::keyboard::keysyms::KEY_BackSpace {
                                return FilterResult::Intercept(KeyAction::Quit);
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
                let serial = SERIAL_COUNTER.next_serial();
                let under = self.surface_under(pos);
                // Over the bare background no client owns the pointer: the arrow again.
                if under.is_none() {
                    self.pointer_image.status = smithay::input::pointer::CursorImageStatus::default_named();
                }
                pointer.motion(self, under, &MotionEvent { location: pos, serial, time: Event::time(&event) });
                pointer.frame(self);
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
                let pointer = self.seat.get_pointer().unwrap();
                let keyboard = self.seat.get_keyboard().unwrap();

                let serial = SERIAL_COUNTER.next_serial();

                let button = event.button_code();

                let button_state = event.state();

                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    if let Some((window, _loc)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(w, l)| (w.clone(), l))
                    {
                        self.space.raise_element(&window, true);
                        keyboard.set_focus(
                            self,
                            Some(window.toplevel().unwrap().wl_surface().clone()),
                            serial,
                        );
                        self.space.elements().for_each(|window| {
                            window.toplevel().unwrap().send_pending_configure();
                        });
                    } else {
                        self.space.elements().for_each(|window| {
                            window.set_activated(false);
                            window.toplevel().unwrap().send_pending_configure();
                        });
                        keyboard.set_focus(self, Option::<WlSurface>::None, serial);
                    }
                };

                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state: button_state,
                        serial,
                        time: event.time(),
                    },
                );
                pointer.frame(self);
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
