// X11 apps through rootless Xwayland (server milestone 1). Adapted from Smithay's
// anvil (MIT; see LICENSE-smithay.txt), cut down: X windows live in the same Space
// as Wayland windows, are placed centered on the monitor under the pointer, and
// move/resize through the seat (dispatch.rs). Scale: one client scale for all of
// Xwayland (VERGENCE_XWAYLAND_SCALE, default 1) until per-monitor X scaling lands.

use crate::win::Win;
use std::os::unix::io::OwnedFd;

use smithay::{
    desktop::Window,
    utils::{Logical, Rectangle},
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
    dispatch::ResizeEdge,
};

impl XWaylandShellHandler for Server {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl Server {
    pub fn window_for_x11(&self, x: &X11Surface) -> Option<Win> {
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
        // Where memory had it on this setup (memory.rs), else on a monitor and
        // fitting it (place.rs).
        let asked_size = window.geometry().size;
        let w = Win::from(Window::new_x11_window(window.clone()));
        let remembered = self.remembered_place(&w);
        let rect = remembered.map(|(r, _)| r).unwrap_or_else(|| self.place_x11(&window));
        let loc = rect.loc;
        let _ = window.configure(rect);
        tracing::info!(
            "X11 window {:?} {:?} at {:?} ({}, asked {:?})",
            window.title(),
            window.class(),
            rect,
            if remembered.is_some() { "remembered" } else { "placed" },
            asked_size
        );
        self.mapped_at.retain(|w, (t, _)| smithay::utils::IsAlive::alive(w) && t.elapsed().as_secs() < 10);
        // The app's own restore (a move just after mapping) only when memory had nothing.
        if remembered.is_none() {
            self.mapped_at.insert(w.clone(), (std::time::Instant::now(), asked_size));
        }
        self.space.map_element(w.clone(), loc, true);
        if remembered.is_some_and(|(_, max)| max) {
            self.toggle_maximize(&w);
        }
        self.window_opened(&w);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        // Menus, tooltips: they place themselves.
        let loc = window.geometry().loc;
        self.space.map_element(Win::from(Window::new_x11_window(window)), loc, true);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window) {
            if !window.is_override_redirect() {
                self.remember(&w);
            }
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
        let mut geo = window.geometry();
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        let Some(mapped) = self.window_for_x11(&window) else {
            // Not mapped yet: as asked; place_x11 decides when it maps.
            if let Some(x) = x {
                geo.loc.x = x;
            }
            if let Some(y) = y {
                geo.loc.y = y;
            }
            let _ = window.configure(geo);
            return;
        };
        // Mapped: from where the server has it, not X's last report (X may not have
        // caught up with our configure; AlbumWall snapped back to 0,0), and never
        // larger than the monitor it is mostly on (AlbumWall restored a saved
        // 1222 x 922 onto the 14"'s 1170 x 658).
        if let Some(g) = self.space.element_geometry(&mapped) {
            geo.loc = g.loc;
        }
        if let Some(room) = self.room_for(&mapped) {
            geo.size = crate::place::fit(geo.size, room, window.min_size());
        }
        // A move just after mapping is the app restoring where it was (Spindle's
        // WindowPlacement sets the position ~60 ms after the window opens): honored
        // if the window then lies wholly on a monitor. Later moves are the server's.
        // On that monitor it gets back the size it asked for when it opened (fitted
        // to the monitor under the pointer then), capped to the new monitor.
        let fresh = self.mapped_at.get(&mapped).copied().filter(|(t, _)| t.elapsed() < std::time::Duration::from_millis(1500));
        if let Some((_, asked_size)) = fresh
            && (x.is_some() || y.is_some())
        {
            let titled = self.has_title(&mapped);
            let loc: smithay::utils::Point<i32, Logical> = (x.unwrap_or(geo.loc.x), y.unwrap_or(geo.loc.y)).into();
            let target = [Rectangle::new(loc, asked_size), Rectangle::new(loc, geo.size)]
                .into_iter()
                .find(|r| self.fits_a_monitor(*r, titled));
            if let Some(r) = target {
                tracing::info!("X11 {:?} restores its place {:?}", window.title(), r);
                geo = r;
                self.space.map_element(mapped.clone(), geo.loc, false);
            }
        }
        let _ = window.configure(geo);
    }

    fn configure_notify(&mut self, _xwm: XwmId, window: X11Surface, geometry: Rectangle<i32, Logical>, _above: Option<u32>) {
        if let Some(w) = self.window_for_x11(&window) {
            self.space.map_element(w.clone(), geometry.loc, false);
            // Its place now, unless a drag is still going (recorded when it ends).
            if self.input.seat.grab().is_none() && !window.is_override_redirect() {
                self.remember(&w);
            }
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window)
            && !self.restore.contains_key(&w)
        {
            self.toggle_maximize(&w);
        }
    }

    fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window) {
            self.minimize(&w);
        }
    }

    fn unminimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let found = self.shelf.minimized.iter().map(|(w, _)| w).find(|w| w.x11_surface() == Some(&window)).cloned();
        if let Some(w) = found {
            self.unminimize(&w);
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
        if let Some(w) = self.window_for_x11(&window) {
            self.enter_fullscreen(&w, None);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        match self.window_for_x11(&window) {
            Some(w) => self.leave_fullscreen(&w),
            None => {
                let _ = window.set_fullscreen(false);
            }
        }
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
        self.grab_request(&w, Some(edges));
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32) {
        tracing::info!("X11 move request {:?}: button {button}", window.title());
        if let Some(w) = self.window_for_x11(&window) {
            self.grab_request(&w, None);
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
        if selection != SelectionTarget::Clipboard {
            return;
        }
        // A native client's clipboard (native.rs), else a Wayland client's.
        let native = match current_data_device_selection_userdata(&self.seat).as_deref() {
            Some(crate::native::ClipOwner::Native(c)) => Some(*c),
            _ => None,
        };
        if let Some(c) = native {
            self.native_clipboard_send(c, mime_type, fd);
        } else if let Err(e) = request_data_device_client_selection(&self.seat, mime_type, fd) {
            tracing::warn!("X11 clipboard request: {e:?}");
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        if selection == SelectionTarget::Clipboard {
            set_data_device_selection(&self.display_handle, &self.seat, mime_types.clone(), crate::native::ClipOwner::X11);
            self.native_clipboard_offer(mime_types);
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        // Only X11's own: a native client's clipboard stays when X11 lets go of its copy.
        let x11 = current_data_device_selection_userdata(&self.seat).is_some_and(|o| *o == crate::native::ClipOwner::X11);
        if selection == SelectionTarget::Clipboard && x11 {
            clear_data_device_selection(&self.display_handle, &self.seat);
            self.native_clipboard_offer(Vec::new());
        }
    }

    fn disconnected(&mut self, _xwm: XwmId) {
        tracing::warn!("Xwayland's window manager connection closed");
        self.xwm = None;
    }
}
