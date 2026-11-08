// Based on Smithay's smallvil example (MIT; see LICENSE-smithay.txt), adapted for Vergence.
mod compositor;
mod xdg_shell;

use crate::Server;

//
// Wl Seat
//

use smithay::input::dnd::{DnDGrab, DndGrabHandler, GrabType, Source};
use smithay::input::pointer::Focus;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::Serial;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::pointer_constraints::PointerConstraintsHandler;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{
    DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus,
};

impl SeatHandler for Server {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Server> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: smithay::input::pointer::CursorImageStatus) {}

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client);
    }
}

impl PointerConstraintsHandler for Server {}

//
// Wl Data Device
//

impl SelectionHandler for Server {
    type SelectionUserData = ();
}

impl DataDeviceHandler for Server {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl DndGrabHandler for Server {}
impl WaylandDndGrabHandler for Server {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        _icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        match type_ {
            GrabType::Pointer => {
                let ptr = seat.get_pointer().unwrap();
                let start_data = ptr.grab_start_data().unwrap();

                // create a dnd grab to start the operation
                let grab = DnDGrab::new_pointer(&self.display_handle, start_data, source, seat);
                ptr.set_grab(self, grab, serial, Focus::Keep);
            }
            GrabType::Touch => {
                // smallvil lacks touch handling
                source.cancel();
            }
        }
    }
}

//
// Wl Output & Xdg Output
//

impl OutputHandler for Server {}

//
// Fractional scale (server milestone 1b)
//

/// A surface that asks for a fractional scale is told the scale of the output its
/// window is on (the first output for now; per output and per frame in 1c).
impl smithay::wayland::fractional_scale::FractionalScaleHandler for Server {
    fn new_fractional_scale(&mut self, surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) {
        let Some(scale) = self.space.outputs().next().map(|o| o.current_scale().fractional_scale()) else { return };
        smithay::wayland::compositor::with_states(&surface, |states| {
            smithay::wayland::fractional_scale::with_fractional_scale(states, |fs| fs.set_preferred_scale(scale));
        });
    }
}

smithay::delegate_dispatch2!(Server);
