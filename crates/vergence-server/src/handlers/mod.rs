// Based on Smithay's smallvil example (MIT; see LICENSE-smithay.txt), adapted for Vergence.
mod compositor;
mod x11;
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
    type KeyboardFocus = crate::focus::KeyboardFocus;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Server> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: smithay::input::pointer::CursorImageStatus) {
        self.pointer_image.status = image;
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&crate::focus::KeyboardFocus>) {
        use smithay::wayland::seat::WaylandFocus;
        let dh = &self.display_handle;
        let client = focused.and_then(|f| f.wl_surface()).and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client);
    }
}

impl PointerConstraintsHandler for Server {}

// Needed by wp-cursor-shape-v1 (shapes for tablet tools too); no tablets yet.
impl smithay::input::tablet::TabletSeatHandler for Server {
    type ToolFocus = WlSurface;
}

//
// Wl Data Device
//

impl SelectionHandler for Server {
    /// Who a server-side selection is really from (native.rs).
    type SelectionUserData = crate::native::ClipOwner;

    /// A Wayland client put something on the clipboard (or cleared it):
    /// native windows and X11 hear of it.
    fn new_selection(&mut self, ty: smithay::wayland::selection::SelectionTarget, source: Option<smithay::wayland::selection::SelectionSource>, _seat: Seat<Self>) {
        use smithay::wayland::selection::SelectionTarget;
        if ty != SelectionTarget::Clipboard {
            return;
        }
        let kinds = source.map(|s| s.mime_types());
        self.native_clipboard_offer(kinds.clone().unwrap_or_default());
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(e) = xwm.new_selection(SelectionTarget::Clipboard, kinds)
        {
            tracing::warn!("clipboard to X11: {e:?}");
        }
    }

    /// A Wayland client pastes what the server holds for someone: X11's or a
    /// native client's clipboard (before 2026-11-11, X11's never reached it).
    fn send_selection(&mut self, ty: smithay::wayland::selection::SelectionTarget, mime_type: String, fd: std::os::fd::OwnedFd, _seat: Seat<Self>, owner: &crate::native::ClipOwner) {
        use smithay::wayland::selection::SelectionTarget;
        if ty != SelectionTarget::Clipboard {
            return;
        }
        match owner {
            crate::native::ClipOwner::Native(client) => self.native_clipboard_send(*client, mime_type, fd),
            crate::native::ClipOwner::X11 => {
                if let Some(xwm) = self.xwm.as_mut()
                    && let Err(e) = xwm.send_selection(SelectionTarget::Clipboard, mime_type, fd)
                {
                    tracing::warn!("clipboard from X11: {e:?}");
                }
            }
        }
    }
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

//
// GPU buffers (linux-dmabuf; the DRM backend creates the global)
//

impl smithay::wayland::dmabuf::DmabufHandler for Server {
    fn dmabuf_state(&mut self) -> &mut smithay::wayland::dmabuf::DmabufState {
        &mut self.dmabuf.as_mut().expect("dmabuf global without state").0
    }

    fn dmabuf_imported(
        &mut self,
        _global: &smithay::wayland::dmabuf::DmabufGlobal,
        dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
        notifier: smithay::wayland::dmabuf::ImportNotifier,
    ) {
        let ok = self.backend.as_mut().is_some_and(|b| b.import_dmabuf(&dmabuf));
        if ok {
            let _ = notifier.successful::<Server>();
        } else {
            notifier.failed();
        }
    }
}

smithay::delegate_dispatch2!(Server);
