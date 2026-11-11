// A window, as the server keeps it (native milestone 1, step 2).
//
// Smithay's `Window` knows Wayland and X11 windows only. The server's own
// element, `Win`, holds one of those today and, from step 3, a window of the
// project's own protocol beside them. Everything that keeps, focuses, moves,
// frames, remembers or draws windows (the space, focus, the grabs, title
// bars, window memory, the dock, the menu, touch) uses `Win`. Wayland and
// X11 windows behave exactly as they did; this file only passes their
// calls on.

use std::borrow::Cow;

use smithay::{
    backend::renderer::{
        ImportAll, Renderer,
        element::{AsRenderElements, surface::WaylandSurfaceRenderElement},
    },
    desktop::{Window, WindowSurfaceType, space::SpaceElement},
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Logical, Physical, Point, Rectangle, Scale},
    wayland::{seat::WaylandFocus, shell::xdg::ToplevelSurface},
    xwayland::X11Surface,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Win {
    /// A Wayland or X11 window, through Smithay.
    Smithay(Window),
}

impl From<Window> for Win {
    fn from(w: Window) -> Win {
        Win::Smithay(w)
    }
}

impl Win {
    /// The Smithay window behind it, if it is one.
    pub fn smithay(&self) -> Option<&Window> {
        match self {
            Win::Smithay(w) => Some(w),
        }
    }

    pub fn geometry(&self) -> Rectangle<i32, Logical> {
        match self {
            Win::Smithay(w) => w.geometry(),
        }
    }

    pub fn bbox(&self) -> Rectangle<i32, Logical> {
        match self {
            Win::Smithay(w) => w.bbox(),
        }
    }

    pub fn set_activated(&self, active: bool) -> bool {
        match self {
            Win::Smithay(w) => w.set_activated(active),
        }
    }

    pub fn toplevel(&self) -> Option<&ToplevelSurface> {
        self.smithay().and_then(|w| w.toplevel())
    }

    pub fn x11_surface(&self) -> Option<&X11Surface> {
        self.smithay().and_then(|w| w.x11_surface())
    }

    pub fn is_x11(&self) -> bool {
        self.smithay().is_some_and(|w| w.is_x11())
    }

    pub fn is_wayland(&self) -> bool {
        self.smithay().is_some_and(|w| w.is_wayland())
    }

    pub fn on_commit(&self) {
        if let Some(w) = self.smithay() {
            w.on_commit();
        }
    }

    pub fn surface_under<P: Into<Point<f64, Logical>>>(&self, point: P, kind: WindowSurfaceType) -> Option<(WlSurface, Point<i32, Logical>)> {
        self.smithay().and_then(|w| w.surface_under(point, kind))
    }

    /// Each surface of the window's tree (none for a window without Wayland surfaces).
    pub fn with_surfaces<F: FnMut(&WlSurface, &smithay::wayland::compositor::SurfaceData)>(&self, processor: F) {
        if let Some(w) = self.smithay() {
            w.with_surfaces(processor);
        }
    }

    /// Tells the window's surfaces that a frame has been shown on `output`.
    pub fn send_frame<T, F>(&self, output: &Output, time: T, throttle: Option<std::time::Duration>, primary_scan_out_output: F)
    where
        T: Into<std::time::Duration>,
        F: FnMut(&WlSurface, &smithay::wayland::compositor::SurfaceData) -> Option<Output> + Copy,
    {
        if let Some(w) = self.smithay() {
            w.send_frame(output, time, throttle, primary_scan_out_output);
        }
    }

    pub fn user_data(&self) -> &smithay::utils::user_data::UserDataMap {
        match self {
            Win::Smithay(w) => w.user_data(),
        }
    }
}

impl IsAlive for Win {
    fn alive(&self) -> bool {
        match self {
            Win::Smithay(w) => w.alive(),
        }
    }
}

impl WaylandFocus for Win {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Win::Smithay(w) => w.wl_surface(),
        }
    }
}

impl SpaceElement for Win {
    fn geometry(&self) -> Rectangle<i32, Logical> {
        match self {
            Win::Smithay(w) => SpaceElement::geometry(w),
        }
    }

    fn bbox(&self) -> Rectangle<i32, Logical> {
        match self {
            Win::Smithay(w) => SpaceElement::bbox(w),
        }
    }

    fn is_in_input_region(&self, point: &Point<f64, Logical>) -> bool {
        match self {
            Win::Smithay(w) => w.is_in_input_region(point),
        }
    }

    fn z_index(&self) -> u8 {
        match self {
            Win::Smithay(w) => w.z_index(),
        }
    }

    fn set_activate(&self, activated: bool) {
        match self {
            Win::Smithay(w) => w.set_activate(activated),
        }
    }

    fn output_enter(&self, output: &Output, overlap: Rectangle<i32, Logical>) {
        match self {
            Win::Smithay(w) => w.output_enter(output, overlap),
        }
    }

    fn output_leave(&self, output: &Output) {
        match self {
            Win::Smithay(w) => w.output_leave(output),
        }
    }

    fn refresh(&self) {
        match self {
            Win::Smithay(w) => w.refresh(),
        }
    }
}

impl<R> AsRenderElements<R> for Win
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
{
    type RenderElement = WaylandSurfaceRenderElement<R>;

    fn render_elements<C: From<WaylandSurfaceRenderElement<R>>>(&self, renderer: &mut R, location: Point<i32, Physical>, scale: Scale<f64>, alpha: f32) -> Vec<C> {
        match self {
            Win::Smithay(w) => w.render_elements(renderer, location, scale, alpha),
        }
    }
}
