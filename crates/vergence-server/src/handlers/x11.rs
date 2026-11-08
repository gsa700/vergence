// X11 apps through rootless Xwayland (server milestone 1). Adapted from Smithay's
// anvil (MIT; see LICENSE-smithay.txt), cut down: X windows live in the same Space
// as Wayland windows, are placed centred on the monitor under the pointer, and
// move/resize with the server's own grabs. Scale: one client scale for all of
// Xwayland (VERGENCE_XWAYLAND_SCALE, default 1) until per-monitor X scaling lands.

use std::os::unix::io::OwnedFd;

use smithay::{
    desktop::Window,
    input::pointer::{Focus, GrabStartData},
    utils::{Logical, Rectangle, SERIAL_COUNTER},
    wayland::{
        selection::{
            SelectionTarget,
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata,
                request_data_device_client_selection, set_data_device_selection,
            },
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XwmHandler,
        xwm::{Reorder, ResizeEdge as X11ResizeEdge, XwmId},
    },
};

use crate::{
    Server,
    focus::KeyboardFocus,
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab, resize_grab::ResizeEdge},
};

impl XWaylandShellHandler for Server {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl Server {
    pub fn window_for_x11(&self, x: &X11Surface) -> Option<Window> {
        self.space.elements().find(|w| w.x11_surface() == Some(x)).cloned()
    }
}

impl XwmHandler for Server {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().unwrap()
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Err(e) = window.set_mapped(true) {
            return tracing::warn!("X11 map: {e}");
        }
        // On a monitor and fitting it (place.rs).
        let rect = self.place_x11(&window);
        let loc = rect.loc;
        let _ = window.configure(rect);
        tracing::info!("X11 window {:?} {:?} at {:?}", window.title(), window.class(), rect);
        let w = Window::new_x11_window(window);
        self.space.map_element(w, loc, true);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        // Menus, tooltips: they place themselves.
        let loc = window.geometry().loc;
        self.space.map_element(Window::new_x11_window(window), loc, true);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window) {
            self.space.unmap_elem(&w);
        }
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        tracing::debug!("X11 configure request {:?}: x {x:?} y {y:?} w {w:?} h {h:?}", window.title());
        // Size as asked; position only before it is mapped (the server places windows).
        // Once mapped, from where the server has it: X may not have caught up with
        // our last configure yet (AlbumWall snapped back to 0,0 on its first resize).
        let mut geo = window.geometry();
        if let Some(w) = self.window_for_x11(&window)
            && let Some(g) = self.space.element_geometry(&w)
        {
            geo.loc = g.loc;
            // Never larger than the monitor it is mostly on (AlbumWall restored a
            // saved 1222 x 922 after being fitted to the 14"'s 1170 x 658).
            if let Some(room) = self.room_for(&w) {
                geo.size = crate::place::fit(geo.size, room, window.min_size());
            }
        }
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        if self.window_for_x11(&window).is_none() {
            if let Some(x) = x {
                geo.loc.x = x;
            }
            if let Some(y) = y {
                geo.loc.y = y;
            }
        }
        let _ = window.configure(geo);
    }

    fn configure_notify(&mut self, _xwm: XwmId, window: X11Surface, geometry: Rectangle<i32, Logical>, _above: Option<u32>) {
        if let Some(w) = self.window_for_x11(&window) {
            self.space.map_element(w, geometry.loc, false);
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window)
            && !self.restore.contains_key(&w)
        {
            self.toggle_maximize(&w);
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window)
            && self.restore.contains_key(&w)
        {
            self.toggle_maximize(&w);
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let (Some(w), Some(o)) = (self.window_for_x11(&window), self.output_at_pointer()) {
            let _ = window.set_fullscreen(true);
            let _ = window.configure(o);
            self.space.map_element(w, o.loc, true);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _ = window.set_fullscreen(false);
    }

    fn resize_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32, edge: X11ResizeEdge) {
        tracing::info!("X11 resize request {:?}: button {button} edge {edge:?}", window.title());
        let Some(w) = self.window_for_x11(&window) else { return };
        let edges = match edge {
            X11ResizeEdge::Top => ResizeEdge::TOP,
            X11ResizeEdge::Bottom => ResizeEdge::BOTTOM,
            X11ResizeEdge::Left => ResizeEdge::LEFT,
            X11ResizeEdge::Right => ResizeEdge::RIGHT,
            X11ResizeEdge::TopLeft => ResizeEdge::TOP_LEFT,
            X11ResizeEdge::TopRight => ResizeEdge::TOP_RIGHT,
            X11ResizeEdge::BottomLeft => ResizeEdge::BOTTOM_LEFT,
            X11ResizeEdge::BottomRight => ResizeEdge::BOTTOM_RIGHT,
        };
        self.start_resize(w, edges, None);
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32) {
        tracing::info!("X11 move request {:?}: button {button}", window.title());
        if let Some(w) = self.window_for_x11(&window) {
            self.start_move(w, None);
        }
    }

    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        // Only the focused X window reads the clipboard.
        matches!(
            self.seat.get_keyboard().and_then(|k| k.current_focus()),
            Some(KeyboardFocus::X11(x)) if x.xwm_id() == Some(xwm)
        )
    }

    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        if selection == SelectionTarget::Clipboard
            && let Err(e) = request_data_device_client_selection(&self.seat, mime_type, fd)
        {
            tracing::warn!("X11 clipboard request: {e:?}");
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        if selection == SelectionTarget::Clipboard {
            set_data_device_selection(&self.display_handle, &self.seat, mime_types, ());
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        if selection == SelectionTarget::Clipboard && current_data_device_selection_userdata(&self.seat).is_some() {
            clear_data_device_selection(&self.display_handle, &self.seat);
        }
    }

    fn disconnected(&mut self, _xwm: XwmId) {
        tracing::warn!("Xwayland's window manager connection closed");
        self.xwm = None;
    }
}

impl Server {
    /// Move a window with the pointer (an app's request, or Alt+left drag).
    /// `start`: the grab's start (Alt+drag), else the click the app was given.
    pub fn start_move(&mut self, window: Window, start: Option<GrabStartData<Server>>) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let Some(start_data) = start.or_else(|| pointer.grab_start_data()) else { return };
        let Some(initial_window_location) = self.space.element_location(&window) else { return };
        let grab = MoveSurfaceGrab { start_data, window, initial_window_location };
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }

    /// Resize a window with the pointer (an app's request, or Alt+right drag).
    pub fn start_resize(&mut self, window: Window, edges: ResizeEdge, start: Option<GrabStartData<Server>>) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let Some(start_data) = start.or_else(|| pointer.grab_start_data()) else { return };
        let Some(loc) = self.space.element_location(&window) else { return };
        let size = window.geometry().size;
        let grab = ResizeSurfaceGrab::start(start_data, window, edges, Rectangle::new(loc, size));
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }
}
