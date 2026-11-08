// Title bars drawn by the server (server milestone 1, 2026-11-07).
//
// X11 windows that do not decorate themselves, and Wayland windows that ask for
// server-side decorations (xdg-decoration), get a title bar TITLE_H points tall
// above their content, with maximise and close buttons at the right, and an
// invisible band BORDER points wide around the whole frame for resizing. Sizes are
// in points like everything else, so the bar is the same physical height on every
// monitor; it is drawn with solid colours, which are sharp at any scale. Edges are
// rounded from logical to physical the same way as the window's own position, so
// bar and content meet without a seam. Title text: later (needs text rendering).
//
// The scene is built here, front to back, window by window, so a title bar is
// stacked with its window (behind the windows above it).

use smithay::{
    backend::renderer::{
        element::{
            AsRenderElements, Id, Kind,
            solid::SolidColorRenderElement,
            surface::WaylandSurfaceRenderElement,
        },
        gles::GlesRenderer,
        utils::CommitCounter,
    },
    desktop::{Window, WindowSurfaceType},
    output::Output,
    reexports::{
        wayland_protocols::xdg::{
            decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
            shell::server::xdg_toplevel,
        },
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{IsAlive, Logical, Physical, Point, Rectangle},
};

use crate::{Server, grabs::resize_grab::ResizeEdge};

/// The title bar's height, in points.
pub const TITLE_H: i32 = 24;
/// The resize band around a decorated window, in points.
pub const BORDER: i32 = 6;

const BAR_ACTIVE: [f32; 4] = [0.20, 0.24, 0.32, 1.0];
const BAR_INACTIVE: [f32; 4] = [0.22, 0.22, 0.22, 1.0];
const CLOSE: [f32; 4] = [0.78, 0.30, 0.28, 1.0];
const BUTTON: [f32; 4] = [0.72, 0.72, 0.72, 1.0];

smithay::backend::renderer::element::render_elements! {
    pub SceneElement<=GlesRenderer>;
    Surface = WaylandSurfaceRenderElement<GlesRenderer>,
    Solid = SolidColorRenderElement,
}

/// What is under a point of the desktop.
pub enum Hit {
    Surface(Window, WlSurface, Point<f64, Logical>),
    Title(Window),
    Close(Window),
    Maximize(Window),
    Edge(Window, ResizeEdge),
    Nothing,
}

/// The ids of one window's bar parts, for damage tracking.
pub struct Decor {
    ids: [Id; 6],
    commit: CommitCounter,
    active: bool,
}

impl Decor {
    fn new() -> Self {
        Decor { ids: std::array::from_fn(|_| Id::new()), commit: CommitCounter::default(), active: false }
    }
}

/// A decorated window's parts, in logical space.
struct Frame {
    content: Rectangle<i32, Logical>,
    title: Rectangle<i32, Logical>,
    close: Rectangle<i32, Logical>,
    maximize: Rectangle<i32, Logical>,
}

impl Frame {
    fn of(content: Rectangle<i32, Logical>) -> Frame {
        let title = Rectangle::new((content.loc.x, content.loc.y - TITLE_H).into(), (content.size.w, TITLE_H).into());
        let close = Rectangle::new((title.loc.x + title.size.w - TITLE_H, title.loc.y).into(), (TITLE_H, TITLE_H).into());
        let maximize = Rectangle::new((close.loc.x - TITLE_H, title.loc.y).into(), (TITLE_H, TITLE_H).into());
        Frame { content, title, close, maximize }
    }

    /// Title bar and content.
    fn whole(&self) -> Rectangle<i32, Logical> {
        Rectangle::new(self.title.loc, (self.content.size.w, self.content.size.h + TITLE_H).into())
    }

    fn outer(&self) -> Rectangle<i32, Logical> {
        let w = self.whole();
        Rectangle::new((w.loc.x - BORDER, w.loc.y - BORDER).into(), (w.size.w + 2 * BORDER, w.size.h + 2 * BORDER).into())
    }
}

/// Smaller by `by` points on every side.
fn inset(r: Rectangle<i32, Logical>, by: f64) -> Rectangle<f64, Logical> {
    let r = r.to_f64();
    Rectangle::new((r.loc.x + by, r.loc.y + by).into(), ((r.size.w - 2.0 * by).max(0.0), (r.size.h - 2.0 * by).max(0.0)).into())
}

/// A logical rectangle on an output, in its physical pixels, corners rounded like
/// window positions are.
fn physical(r: Rectangle<f64, Logical>, output_loc: Point<i32, Logical>, scale: f64) -> Rectangle<i32, Physical> {
    let a = (r.loc - output_loc.to_f64()).to_physical(scale).to_i32_round::<i32>();
    let b = (r.loc + r.size.to_point() - output_loc.to_f64()).to_physical(scale).to_i32_round::<i32>();
    Rectangle::new(a, (b.x - a.x, b.y - a.y).into())
}

impl Server {
    /// Does the server draw this window's title bar?
    pub fn has_title(&self, w: &Window) -> bool {
        if let Some(x) = w.x11_surface() {
            return !x.is_override_redirect() && !x.is_decorated() && !x.is_fullscreen();
        }
        w.toplevel().is_some_and(|t| {
            let check = |s: &smithay::wayland::shell::xdg::ToplevelState| {
                s.decoration_mode == Some(DecorationMode::ServerSide) && !s.states.contains(xdg_toplevel::State::Fullscreen)
            };
            // What the app has acknowledged; before its first commit, what we offered.
            t.with_committed_state(|c| c.map(check)).unwrap_or_else(|| t.with_pending_state(|p| check(p)))
        })
    }

    fn frame_of(&self, w: &Window) -> Option<Frame> {
        self.has_title(w).then(|| self.space.element_geometry(w).map(Frame::of)).flatten()
    }

    /// The top-most thing under `pos`: a title bar part, a resize band, or a surface.
    pub fn hit(&self, pos: Point<f64, Logical>) -> Hit {
        for w in self.space.elements().rev() {
            if let Some(f) = self.frame_of(w) {
                if f.close.to_f64().contains(pos) {
                    return Hit::Close(w.clone());
                }
                if f.maximize.to_f64().contains(pos) {
                    return Hit::Maximize(w.clone());
                }
                if f.title.to_f64().contains(pos) {
                    return Hit::Title(w.clone());
                }
                let whole = f.whole().to_f64();
                if f.outer().to_f64().contains(pos) && !whole.contains(pos) {
                    let mut e = ResizeEdge::empty();
                    if pos.x < whole.loc.x {
                        e |= ResizeEdge::LEFT;
                    } else if pos.x >= whole.loc.x + whole.size.w {
                        e |= ResizeEdge::RIGHT;
                    }
                    if pos.y < whole.loc.y {
                        e |= ResizeEdge::TOP;
                    } else if pos.y >= whole.loc.y + whole.size.h {
                        e |= ResizeEdge::BOTTOM;
                    }
                    return Hit::Edge(w.clone(), e);
                }
            }
            let Some(bbox) = self.space.element_bbox(w) else { continue };
            if !bbox.to_f64().contains(pos) {
                continue;
            }
            let Some(loc) = self.space.element_location(w) else { continue };
            let render_loc = loc - w.geometry().loc;
            if let Some((s, p)) = w.surface_under(pos - render_loc.to_f64(), WindowSurfaceType::ALL) {
                return Hit::Surface(w.clone(), s, (p + render_loc).to_f64());
            }
        }
        Hit::Nothing
    }

    /// Everything on `output`, front to back: each window with its title bar.
    pub fn scene(&mut self, renderer: &mut GlesRenderer, output: &Output, scale: f64) -> Vec<SceneElement> {
        let Some(out) = self.space.output_geometry(output) else { return Vec::new() };
        let focused = self.seat.get_keyboard().and_then(|k| k.current_focus());
        self.decor.retain(|w, _| w.alive());
        let windows: Vec<Window> = self.space.elements().rev().cloned().collect();
        let mut elements = Vec::new();
        for w in windows {
            let frame = self.frame_of(&w);
            let reach = frame.as_ref().map(|f| f.whole()).or_else(|| self.space.element_bbox(&w));
            if !reach.is_some_and(|r| r.overlaps(out)) {
                continue;
            }
            if let Some(f) = frame {
                let active = focused.as_ref().is_some_and(|k| match k {
                    crate::focus::KeyboardFocus::X11(x) => w.x11_surface() == Some(x),
                    crate::focus::KeyboardFocus::Wayland(s) => w.toplevel().is_some_and(|t| t.wl_surface() == s),
                });
                let d = self.decor.entry(w.clone()).or_insert_with(Decor::new);
                if d.active != active {
                    d.active = active;
                    d.commit.increment();
                }
                let solid = |i: usize, r: Rectangle<f64, Logical>, c: [f32; 4]| {
                    SceneElement::Solid(SolidColorRenderElement::new(d.ids[i].clone(), physical(r, out.loc, scale), d.commit, c, Kind::Unspecified))
                };
                // Front to back: button marks, then the bar.
                let m = inset(f.maximize, 7.0);
                let t = 1.5;
                elements.push(solid(0, inset(f.close, 7.0), CLOSE));
                elements.push(solid(1, Rectangle::new(m.loc, (m.size.w, t).into()), BUTTON));
                elements.push(solid(2, Rectangle::new((m.loc.x, m.loc.y + m.size.h - t).into(), (m.size.w, t).into()), BUTTON));
                elements.push(solid(3, Rectangle::new(m.loc, (t, m.size.h).into()), BUTTON));
                elements.push(solid(4, Rectangle::new((m.loc.x + m.size.w - t, m.loc.y).into(), (t, m.size.h).into()), BUTTON));
                elements.push(solid(5, f.title.to_f64(), if active { BAR_ACTIVE } else { BAR_INACTIVE }));
            }
            let Some(loc) = self.space.element_location(&w) else { continue };
            let render_loc = (loc - w.geometry().loc - out.loc).to_f64().to_physical(scale).to_i32_round();
            let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = w.render_elements(renderer, render_loc, scale.into(), 1.0);
            elements.extend(surfaces.into_iter().map(SceneElement::Surface));
        }
        elements
    }

    /// Close a window as its app is asked to.
    pub fn close_window(&self, w: &Window) {
        if let Some(x) = w.x11_surface() {
            let _ = x.close();
        } else if let Some(t) = w.toplevel() {
            t.send_close();
        }
    }

    /// Fill the monitor the window is mostly on (title bar included), or restore.
    pub fn toggle_maximize(&mut self, w: &Window) {
        if let Some(old) = self.restore.remove(w) {
            self.set_window_rect(w, old, false);
            return;
        }
        let Some(geo) = self.space.element_geometry(w) else { return };
        let out = self
            .space
            .outputs()
            .filter_map(|o| {
                let g = self.space.output_geometry(o)?;
                let i = g.intersection(geo)?;
                Some((i.size.w as i64 * i.size.h as i64, g))
            })
            .max_by_key(|(a, _)| *a)
            .map(|(_, g)| g)
            .or_else(|| self.space.outputs().next().and_then(|o| self.space.output_geometry(o)));
        let Some(out) = out else { return };
        let top = if self.has_title(w) { TITLE_H } else { 0 };
        let rect = Rectangle::new((out.loc.x, out.loc.y + top).into(), (out.size.w, out.size.h - top).into());
        self.restore.insert(w.clone(), geo);
        self.set_window_rect(w, rect, true);
    }

    fn set_window_rect(&mut self, w: &Window, rect: Rectangle<i32, Logical>, maximized: bool) {
        if let Some(x) = w.x11_surface() {
            let _ = x.set_maximized(maximized);
            let _ = x.configure(rect);
        } else if let Some(t) = w.toplevel() {
            t.with_pending_state(|s| {
                if maximized {
                    s.states.set(xdg_toplevel::State::Maximized);
                } else {
                    s.states.unset(xdg_toplevel::State::Maximized);
                }
                s.size = Some(rect.size);
            });
            t.send_pending_configure();
        }
        self.space.map_element(w.clone(), rect.loc, true);
    }
}

/// The pointer shape for a resize band.
pub fn edge_icon(e: ResizeEdge) -> smithay::input::pointer::CursorIcon {
    use smithay::input::pointer::CursorIcon as C;
    match e {
        ResizeEdge::TOP_LEFT => C::NwResize,
        ResizeEdge::TOP_RIGHT => C::NeResize,
        ResizeEdge::BOTTOM_LEFT => C::SwResize,
        ResizeEdge::BOTTOM_RIGHT => C::SeResize,
        ResizeEdge::TOP => C::NResize,
        ResizeEdge::BOTTOM => C::SResize,
        ResizeEdge::LEFT => C::WResize,
        _ => C::EResize,
    }
}
