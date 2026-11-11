// Based on Smithay's smallvil example (MIT; see LICENSE-smithay.txt), adapted for Vergence.
//! Move grab is the state of a composer during which the client window is being dragged around.
//!
//! eg. Usually whenever a user clicks on the app's titlebar and starts dragging, the compositors
//! enters a MoveSurfaceGrab state.

use crate::win::Win;
use crate::Server;
use smithay::{
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
        GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData, MotionEvent, PointerGrab,
        PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
};

pub struct MoveSurfaceGrab {
    pub start_data: PointerGrabStartData<Server>,
    pub window: Win,
    pub initial_window_location: Point<i32, Logical>,
    /// A maximized window: it leaves maximized only once the pointer has
    /// really moved, so a double-click on its title bar (whose first press
    /// starts this grab) restores it instead of being undone by the second.
    pub unmaximize_on_drag: bool,
}

/// How far the pointer moves before a press on a maximized window's title
/// bar becomes a drag that takes it out of maximized.
const DRAG: f64 = 6.0;

impl PointerGrab<Server> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut Server,
        handle: &mut PointerInnerHandle<'_, Server>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus
        handle.motion(data, None, event);

        let delta = event.location - self.start_data.location;
        if self.unmaximize_on_drag {
            if delta.x.hypot(delta.y) < DRAG {
                return;
            }
            // Out of maximized: back to its size, under the pointer, and dragged on from there.
            self.unmaximize_on_drag = false;
            if let Some(old) = data.restore.get(&self.window).copied() {
                data.toggle_maximize(&self.window);
                let x = (event.location.x - old.size.w as f64 / 2.0).round() as i32;
                let y = event.location.y.round() as i32 + crate::decor::TITLE_H / 2;
                data.space.map_element(self.window.clone(), (x, y), true);
                if let Some(xs) = self.window.x11_surface() {
                    let _ = xs.configure(smithay::utils::Rectangle::new((x, y).into(), old.size));
                }
                self.initial_window_location = (x, y).into();
                self.start_data.location = event.location;
            }
            return;
        }
        let new_location = self.initial_window_location.to_f64() + delta;
        let loc = new_location.to_i32_round();
        data.space.map_element(self.window.clone(), loc, true);
        // X11 apps place their menus from where X thinks the window is.
        if let Some(x) = self.window.x11_surface() {
            let _ = x.configure(smithay::utils::Rectangle::new(loc, x.geometry().size));
        }
    }

    fn relative_motion(
        &mut self,
        data: &mut Server,
        handle: &mut PointerInnerHandle<'_, Server>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut Server,
        handle: &mut PointerInnerHandle<'_, Server>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        // The button is a button code as defined in the
        // Linux kernel's linux/input-event-codes.h header file, e.g. BTN_LEFT.
        const BTN_LEFT: u32 = 0x110;

        if !handle.current_pressed().contains(&BTN_LEFT) {
            // No more buttons are pressed, release the grab.
            handle.unset_grab(self, data, event.serial, event.time, true);
            data.remember(&self.window);
        }
    }

    fn axis(
        &mut self,
        data: &mut Server,
        handle: &mut PointerInnerHandle<'_, Server>,
        details: AxisFrame,
    ) {
        handle.axis(data, details)
    }

    fn frame(&mut self, data: &mut Server, handle: &mut PointerInnerHandle<'_, Server>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut Server,
        handle: &mut PointerInnerHandle<'_, Server>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event)
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut Server,
        handle: &mut PointerInnerHandle<'_, Server>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event)
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut Server,
        handle: &mut PointerInnerHandle<'_, Server>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event)
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut Server,
        handle: &mut PointerInnerHandle<'_, Server>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event)
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut Server,
        handle: &mut PointerInnerHandle<'_, Server>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event)
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut Server,
        handle: &mut PointerInnerHandle<'_, Server>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event)
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut Server,
        handle: &mut PointerInnerHandle<'_, Server>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event)
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut Server,
        handle: &mut PointerInnerHandle<'_, Server>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event)
    }

    fn start_data(&self) -> &PointerGrabStartData<Server> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut Server) {}
}
