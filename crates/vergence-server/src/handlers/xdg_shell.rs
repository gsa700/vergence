// Based on Smithay's smallvil example (MIT; see LICENSE-smithay.txt), adapted for Vergence.
use crate::win::Win;
use smithay::{
    desktop::{PopupKind, PopupManager, Space, Window, find_popup_root_surface, get_popup_toplevel_coords},
    input::Seat,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            Resource,
            protocol::{wl_seat, wl_surface::WlSurface},
        },
    },
    utils::Serial,
    wayland::{
        compositor::with_states,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
        },
    },
};

use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;

use crate::{Server, dispatch::ResizeEdge};

impl XdgShellHandler for Server {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Near the top left of the monitor under the pointer, below where a title bar goes.
        let at = self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
        let out = self
            .space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o))
            .find(|g| g.to_f64().contains(at))
            .or_else(|| self.space.outputs().next().and_then(|o| self.space.output_geometry(o)))
            .unwrap_or_default();
        let gap = crate::decor::TITLE_H * 2;
        // The most it may be (xdg-toplevel configure_bounds): the monitor below a title bar.
        let room = crate::place::content_area(out, self.bar_h_in(out)).size;
        surface.with_pending_state(|s| s.bounds = Some(room));
        let window = Win::from(Window::new_wayland_window(surface));
        self.space.map_element(window, (out.loc.x + gap, out.loc.y + gap), false);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        // Remember where it was when it closed (memory.rs).
        let found = self.space.elements().find(|w| w.toplevel() == Some(&surface)).cloned();
        if let Some(w) = found {
            self.remember(&w);
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        let found = self.space.elements().find(|w| w.toplevel() == Some(&surface)).cloned();
        if let Some(w) = found
            && !self.restore.contains_key(&w)
        {
            self.toggle_maximize(&w);
        }
    }

    // Full screen (fullscreen.rs): on the screen it names, else the one it is on.
    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        let found = self.space.elements().find(|w| w.toplevel() == Some(&surface)).cloned();
        let output = output.and_then(|o| smithay::output::Output::from_resource(&o));
        if let Some(w) = found {
            self.enter_fullscreen(&w, output);
        } else {
            surface.send_configure();
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        let found = self.space.elements().find(|w| w.toplevel() == Some(&surface)).cloned();
        if let Some(w) = found {
            self.leave_fullscreen(&w);
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        // The window goes to its icon on the shelf (shelf.rs).
        let found = self.space.elements().find(|w| w.toplevel() == Some(&surface)).cloned();
        if let Some(w) = found {
            self.minimize(&w);
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        let found = self.space.elements().find(|w| w.toplevel() == Some(&surface)).cloned();
        if let Some(w) = found
            && self.restore.contains_key(&w)
        {
            self.toggle_maximize(&w);
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat = Seat::from_resource(&seat).unwrap();
        let wl_surface = surface.wl_surface();
        let window =
            self.space.elements().find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == wl_surface)).cloned();
        // The seat grants it only while a button is held on the window (dispatch.rs);
        // the serial must be that press's, as Smithay's pointer saw it.
        if pressed_with(&seat, wl_surface, serial)
            && let Some(w) = window
        {
            self.grab_request(&w, None);
        } else {
            // Not a pointer's press: a finger's, perhaps (touch.rs).
            self.touch_move_request(wl_surface, serial);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let seat = Seat::from_resource(&seat).unwrap();
        let wl_surface = surface.wl_surface();
        let window =
            self.space.elements().find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == wl_surface)).cloned();
        if pressed_with(&seat, wl_surface, serial)
            && let Some(w) = window
        {
            self.grab_request(&w, Some(ResizeEdge::from(edges)));
        }
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // TODO popup grabs
    }
}

/// Was `serial` a pointer press on this client's surface, still held?
fn pressed_with(seat: &Seat<Server>, surface: &WlSurface, serial: Serial) -> bool {
    let Some(pointer) = seat.get_pointer() else { return false };
    if !pointer.has_grab(serial) {
        return false;
    }
    pointer.grab_start_data().and_then(|d| d.focus).is_some_and(|(focus, _)| focus.id().same_client_as(&surface.id()))
}

/// Should be called on `WlSurface::commit`
pub fn handle_commit(popups: &mut PopupManager, space: &Space<Win>, surface: &WlSurface) {
    // Handle toplevel commits.
    if let Some(window) = space.elements().find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface)).cloned() {
        let initial_configure_sent = with_states(surface, |states| {
            states.data_map.get::<XdgToplevelSurfaceData>().unwrap().lock().unwrap().initial_configure_sent
        });

        if !initial_configure_sent {
            window.toplevel().unwrap().send_configure();
        }
    }

    // Handle popup commits.
    popups.commit(surface);
    if let Some(popup) = popups.find_popup(surface) {
        match popup {
            PopupKind::Xdg(ref xdg) => {
                if !xdg.is_initial_configure_sent() {
                    // NOTE: This should never fail as the initial configure is always
                    // allowed.
                    xdg.send_configure().expect("initial configure failed");
                }
            }
            PopupKind::InputMethod(ref _input_method) => {}
        }
    }
}

impl Server {
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some(window) = self.space.elements().find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == &root)) else {
            return;
        };

        let Some(window_geo) = self.space.element_geometry(window) else { return };
        let parent_offset = get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));

        // Kept on the monitor where the popup is anchored (the menu's button), else
        // the one the window is mostly on. (It used the first monitor: Firefox's
        // menus on one monitor opened on another.)
        let anchor = popup.with_pending_state(|s| s.positioner.anchor_rect);
        let center: smithay::utils::Point<i32, smithay::utils::Logical> = (anchor.size.w / 2, anchor.size.h / 2).into();
        let at = window_geo.loc + parent_offset + anchor.loc + center;
        let geos: Vec<smithay::utils::Rectangle<i32, smithay::utils::Logical>> =
            self.space.outputs().filter_map(|o| self.space.output_geometry(o)).collect();
        let Some(output_geo) = geos.iter().copied().find(|g| g.contains(at)).or_else(|| {
            geos.iter()
                .copied()
                .filter_map(|g| g.intersection(window_geo).map(|i| (i.size.w as i64 * i.size.h as i64, g)))
                .max_by_key(|(a, _)| *a)
                .map(|(_, g)| g)
        }) else {
            return;
        };

        // The target geometry for the positioner should be relative to its parent's geometry, so
        // we will compute that here.
        let mut target = output_geo;
        target.loc -= parent_offset;
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

// GTK's and Qt's modal hint (window styles milestone 1). Smithay keeps the hint
// on the toplevel's data, where style.rs reads it; a change is drawn at once.
impl smithay::wayland::shell::xdg::dialog::XdgDialogHandler for Server {
    fn dialog_hint_changed(
        &mut self,
        toplevel: ToplevelSurface,
        hint: smithay::wayland::shell::xdg::dialog::ToplevelDialogHint,
    ) {
        let _ = toplevel;
        tracing::info!("wayland: a window's dialog hint is now {hint:?}");
        self.redraw_all();
    }
}

// Server-side title bars for apps that ask for them (or leave it to us); an app
// that asks to draw its own gets that.
impl smithay::wayland::shell::xdg::decoration::XdgDecorationHandler for Server {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        set_decoration(&toplevel, DecorationMode::ServerSide);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecorationMode) {
        set_decoration(&toplevel, mode);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        set_decoration(&toplevel, DecorationMode::ServerSide);
    }
}

fn set_decoration(toplevel: &ToplevelSurface, mode: DecorationMode) {
    toplevel.with_pending_state(|s| s.decoration_mode = Some(mode));
    if toplevel.is_initial_configure_sent() {
        toplevel.send_pending_configure();
    }
}
