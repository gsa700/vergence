// Keyboard focus: a Wayland surface, or an X11 window through Xwayland (server
// milestone 1, Xwayland). An X window needs more than wl_keyboard.enter on its
// surface: X input focus or WM_TAKE_FOCUS, as its ICCCM input model says, which
// Smithay's X11Surface does. Pointer focus stays a WlSurface: X11Surface only
// forwards pointer events to its surface.

use std::borrow::Cow;

use smithay::{
    backend::input::{InputTime, KeyState},
    input::{
        Seat,
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
    },
    reexports::wayland_server::{Resource, backend::ObjectId, protocol::wl_surface::WlSurface},
    utils::{IsAlive, Serial},
    wayland::seat::WaylandFocus,
    xwayland::X11Surface,
};

use crate::Server;

#[derive(Debug, Clone, PartialEq)]
pub enum KeyboardFocus {
    Wayland(WlSurface),
    X11(X11Surface),
}

impl From<WlSurface> for KeyboardFocus {
    fn from(s: WlSurface) -> Self {
        KeyboardFocus::Wayland(s)
    }
}

impl IsAlive for KeyboardFocus {
    fn alive(&self) -> bool {
        match self {
            KeyboardFocus::Wayland(s) => s.is_alive(),
            KeyboardFocus::X11(x) => x.alive(),
        }
    }
}

impl WaylandFocus for KeyboardFocus {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            KeyboardFocus::Wayland(s) => Some(Cow::Borrowed(s)),
            KeyboardFocus::X11(x) => x.wl_surface().map(Cow::Owned),
        }
    }

    fn same_client_as(&self, object_id: &ObjectId) -> bool {
        match self {
            KeyboardFocus::Wayland(s) => s.same_client_as(object_id),
            KeyboardFocus::X11(x) => x.same_client_as(object_id),
        }
    }
}

impl KeyboardTarget<Server> for KeyboardFocus {
    fn enter(&self, seat: &Seat<Server>, data: &mut Server, keys: Vec<KeysymHandle<'_>>, serial: Serial) {
        match self {
            KeyboardFocus::Wayland(s) => KeyboardTarget::enter(s, seat, data, keys, serial),
            KeyboardFocus::X11(x) => KeyboardTarget::enter(x, seat, data, keys, serial),
        }
    }

    fn leave(&self, seat: &Seat<Server>, data: &mut Server, serial: Serial) {
        match self {
            KeyboardFocus::Wayland(s) => KeyboardTarget::leave(s, seat, data, serial),
            KeyboardFocus::X11(x) => KeyboardTarget::leave(x, seat, data, serial),
        }
    }

    fn key(
        &self,
        seat: &Seat<Server>,
        data: &mut Server,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: InputTime,
    ) {
        match self {
            KeyboardFocus::Wayland(s) => KeyboardTarget::key(s, seat, data, key, state, serial, time),
            KeyboardFocus::X11(x) => KeyboardTarget::key(x, seat, data, key, state, serial, time),
        }
    }

    fn modifiers(&self, seat: &Seat<Server>, data: &mut Server, modifiers: ModifiersState, serial: Serial) {
        match self {
            KeyboardFocus::Wayland(s) => KeyboardTarget::modifiers(s, seat, data, modifiers, serial),
            KeyboardFocus::X11(x) => KeyboardTarget::modifiers(x, seat, data, modifiers, serial),
        }
    }
}
