// Based on Smithay's smallvil example (MIT; see LICENSE-smithay.txt), adapted for Vergence.
use crate::{Server, state::ClientState};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    reexports::wayland_server::{
        Client,
        protocol::{wl_buffer, wl_surface::WlSurface},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState, get_parent, is_sync_subsurface,
        },
        shm::{ShmHandler, ShmState},
    },
};

use super::xdg_shell;
use smithay::wayland::seat::WaylandFocus;

impl CompositorHandler for Server {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // Xwayland's client is created by Smithay with its own client data.
        if let Some(x) = client.get_data::<smithay::xwayland::XWaylandClientData>() {
            return &x.compositor_state;
        }
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            let found = self.space.elements().find(|w| w.wl_surface().as_deref() == Some(&root)).cloned();
            if let Some(window) = found {
                window.on_commit();
                // A new Wayland window is placed once it has a size (place.rs).
                if window.toplevel().is_some() {
                    self.place_wayland(&window);
                }
            }
        };

        xdg_shell::handle_commit(&mut self.popups, &self.space, surface);
        self.resize_commit(surface);
    }
}

impl BufferHandler for Server {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Server {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
