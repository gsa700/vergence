// Full screen. Without this a Firefox video could not go full screen: the server never answered
// Wayland's request, and an X11 window was made full screen but never put back,
// with the dock still drawn over it.
//
// A window made full screen covers the whole of a screen (the one it names,
// else the one it is mostly on), the dock's band included, with no bar; its
// place before is kept and given back when it leaves full screen. While the
// window with the keyboard is full screen on a screen, the dock there is
// neither drawn nor pressed (shelf.rs), as a video or a game wants.

use smithay::{
    output::Output,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{IsAlive, Logical, Rectangle},
};

use crate::{Server, win::Win};

impl Server {
    /// Make a window full screen on `output`, else the screen it is mostly on.
    pub fn enter_fullscreen(&mut self, w: &Win, output: Option<Output>) {
        let Some(geo) = self.space.element_geometry(w) else { return };
        let screen = output.and_then(|o| self.space.output_geometry(&o)).or_else(|| {
            self.space
                .outputs()
                .filter_map(|o| self.space.output_geometry(o))
                .max_by_key(|g| g.intersection(geo).map_or(0, |i| i.size.w as i64 * i.size.h as i64))
        });
        let Some(screen) = screen else { return };
        self.fullscreen.retain(|w, _| w.alive());
        // Its place before: kept from a first request through later ones.
        self.fullscreen.entry(w.clone()).or_insert(geo);
        if let Some(t) = w.toplevel() {
            t.with_pending_state(|s| {
                s.states.set(xdg_toplevel::State::Fullscreen);
                s.size = Some(screen.size);
            });
            t.send_pending_configure();
        } else if let Some(x) = w.x11_surface() {
            let _ = x.set_fullscreen(true);
            let _ = x.configure(screen);
        } else if let Win::Native(n) = w {
            // Filled as it is: off the pixel grid, as a maximized window.
            self.native_configure(n, screen.size, true);
        }
        self.put(w.clone(), screen.loc, true);
        tracing::info!("full screen: {:?} on {:?}", self.window_title(w), screen);
        self.redraw_all();
    }

    /// Give a full-screen window back its place before.
    pub fn leave_fullscreen(&mut self, w: &Win) {
        let Some(before) = self.fullscreen.remove(w) else { return };
        if let Some(t) = w.toplevel() {
            t.with_pending_state(|s| {
                s.states.unset(xdg_toplevel::State::Fullscreen);
                s.size = Some(before.size);
            });
            t.send_pending_configure();
        } else if let Some(x) = w.x11_surface() {
            let _ = x.set_fullscreen(false);
            let _ = x.configure(before);
        } else if let Win::Native(n) = w {
            let maximized = self.restore.contains_key(w);
            self.native_configure(n, before.size, maximized);
        }
        self.put(w.clone(), before.loc, true);
        tracing::info!("full screen: {:?} back to {:?}", self.window_title(w), before);
        self.redraw_all();
    }

    /// The window with the keyboard is full screen on this screen: the dock
    /// there is neither drawn nor pressed.
    pub fn fullscreen_covers(&self, screen: Rectangle<i32, Logical>) -> bool {
        let Some(f) = self.keyboard_window() else { return false };
        self.fullscreen.contains_key(&f)
            && self
                .space
                .element_geometry(&f)
                .is_some_and(|g| g.intersection(screen).is_some_and(|i| i.size.w * 2 >= screen.size.w))
    }
}
