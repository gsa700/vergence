// A window, as the server keeps it (native milestone 1, steps 2 and 3).
//
// Smithay's `Window` knows Wayland and X11 windows only. The server's own
// element, `Win`, holds one of those or, since step 3, a window of the
// project's own protocol (native.rs). Everything that keeps, focuses, moves,
// frames, remembers or draws windows (the space, focus, the grabs, title
// bars, window memory, the dock, the menu, touch) uses `Win`. Wayland and
// X11 windows behave exactly as they did; this file only passes their
// calls on.

use std::{
    borrow::Cow,
    hash::{Hash, Hasher},
    sync::Arc,
};

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

#[derive(Debug, Clone)]
pub enum Win {
    /// A Wayland or X11 window, through Smithay.
    Smithay(Window),
    /// A window of the project's own protocol (native.rs).
    Native(Arc<crate::native::NativeWindow>),
}

// A native window is the same window only as itself (one Arc per window).
impl PartialEq for Win {
    fn eq(&self, other: &Win) -> bool {
        match (self, other) {
            (Win::Smithay(a), Win::Smithay(b)) => a == b,
            (Win::Native(a), Win::Native(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for Win {}

impl Hash for Win {
    fn hash<H: Hasher>(&self, h: &mut H) {
        match self {
            Win::Smithay(w) => {
                0u8.hash(h);
                w.hash(h);
            }
            Win::Native(n) => {
                1u8.hash(h);
                (Arc::as_ptr(n) as usize).hash(h);
            }
        }
    }
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
            Win::Native(_) => None,
        }
    }

    /// The native window behind it, if it is one.
    pub fn native(&self) -> Option<&crate::native::NativeWindow> {
        match self {
            Win::Native(n) => Some(n),
            Win::Smithay(_) => None,
        }
    }

    pub fn geometry(&self) -> Rectangle<i32, Logical> {
        match self {
            Win::Smithay(w) => w.geometry(),
            Win::Native(n) => Rectangle::from_size(n.size()),
        }
    }

    pub fn bbox(&self) -> Rectangle<i32, Logical> {
        match self {
            Win::Smithay(w) => w.bbox(),
            Win::Native(n) => Rectangle::from_size(n.size()),
        }
    }

    pub fn set_activated(&self, active: bool) -> bool {
        match self {
            Win::Smithay(w) => w.set_activated(active),
            Win::Native(n) => n.set_activated(active),
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

    pub fn surface_under<P: Into<Point<f64, Logical>>>(
        &self,
        point: P,
        kind: WindowSurfaceType,
    ) -> Option<(WlSurface, Point<i32, Logical>)> {
        self.smithay().and_then(|w| w.surface_under(point, kind))
    }

    /// Each surface of the window's tree (none for a window without Wayland surfaces).
    pub fn with_surfaces<F: FnMut(&WlSurface, &smithay::wayland::compositor::SurfaceData)>(&self, processor: F) {
        if let Some(w) = self.smithay() {
            w.with_surfaces(processor);
        }
    }

    /// Tells the window's surfaces that a frame has been shown on `output`.
    pub fn send_frame<T, F>(
        &self,
        output: &Output,
        time: T,
        throttle: Option<std::time::Duration>,
        primary_scan_out_output: F,
    ) where
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
            Win::Native(n) => n.user_data(),
        }
    }
}

impl IsAlive for Win {
    fn alive(&self) -> bool {
        match self {
            Win::Smithay(w) => w.alive(),
            Win::Native(n) => n.is_alive(),
        }
    }
}

impl WaylandFocus for Win {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Win::Smithay(w) => w.wl_surface(),
            Win::Native(_) => None,
        }
    }
}

impl SpaceElement for Win {
    fn geometry(&self) -> Rectangle<i32, Logical> {
        match self {
            Win::Smithay(w) => SpaceElement::geometry(w),
            Win::Native(n) => Rectangle::from_size(n.size()),
        }
    }

    fn bbox(&self) -> Rectangle<i32, Logical> {
        match self {
            Win::Smithay(w) => SpaceElement::bbox(w),
            Win::Native(n) => Rectangle::from_size(n.size()),
        }
    }

    fn is_in_input_region(&self, point: &Point<f64, Logical>) -> bool {
        match self {
            Win::Smithay(w) => w.is_in_input_region(point),
            Win::Native(n) => Rectangle::from_size(n.size()).to_f64().contains(*point),
        }
    }

    fn z_index(&self) -> u8 {
        match self {
            Win::Smithay(w) => w.z_index(),
            Win::Native(_) => smithay::desktop::space::RenderZindex::Shell as u8,
        }
    }

    fn set_activate(&self, activated: bool) {
        match self {
            Win::Smithay(w) => w.set_activate(activated),
            Win::Native(n) => {
                n.set_activated(activated);
            }
        }
    }

    fn output_enter(&self, output: &Output, overlap: Rectangle<i32, Logical>) {
        match self {
            Win::Smithay(w) => w.output_enter(output, overlap),
            Win::Native(_) => {}
        }
    }

    fn output_leave(&self, output: &Output) {
        match self {
            Win::Smithay(w) => w.output_leave(output),
            Win::Native(_) => {}
        }
    }

    fn refresh(&self) {
        match self {
            Win::Smithay(w) => w.refresh(),
            Win::Native(_) => {}
        }
    }
}

impl<R> AsRenderElements<R> for Win
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
{
    type RenderElement = WaylandSurfaceRenderElement<R>;

    fn render_elements<C: From<WaylandSurfaceRenderElement<R>>>(
        &self,
        renderer: &mut R,
        location: Point<i32, Physical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<C> {
        match self {
            Win::Smithay(w) => w.render_elements(renderer, location, scale, alpha),
            // Drawn by the scene from its picture (decor.rs), not as a surface.
            Win::Native(_) => Vec::new(),
        }
    }
}
