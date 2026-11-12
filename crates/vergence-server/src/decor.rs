// Title bars drawn by the server (server milestone 1, 2026-11-07).
//
// X11 windows that do not decorate themselves, and Wayland windows that ask for
// server-side decorations (xdg-decoration), get a title bar TITLE_H points tall
// above their content, with maximize and close buttons at the right, and an
// invisible band BORDER points wide around the whole frame for resizing. Sizes are
// in points like everything else, so the bar is the same physical height on every
// monitor; it is drawn with solid colors, which are sharp at any scale. Edges are
// rounded from logical to physical the same way as the window's own position, so
// bar and content meet without a seam.
//
// Every window that draws no frame of its own, ours (the terminal and the settings
// app draw their own title bars) or a server-decorated one, also gets an EDGE-point
// border line and the same resize band (Dave, 2026-11-09: "a small border to make
// resizing the windows easier; it looks weird without one"). A window whose
// bounding box is larger than its geometry is drawing its own frame and shadow
// (GTK's client-side decorations) and is left alone.
//
// The scene is built here, front to back, window by window, so a title bar is
// stacked with its window (behind the windows above it).

use crate::win::Win;
use smithay::{
    backend::renderer::{
        element::{
            AsRenderElements, Id, Kind,
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            solid::SolidColorRenderElement,
            surface::WaylandSurfaceRenderElement,
        },
        gles::GlesRenderer,
        utils::CommitCounter,
    },
    desktop::WindowSurfaceType,
    output::Output,
    reexports::{
        wayland_protocols::xdg::{
            decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
            shell::server::xdg_toplevel,
        },
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{IsAlive, Logical, Physical, Point, Rectangle, Size, Transform},
};
use smithay::backend::allocator::Fourcc;

use crate::{Server, grabs::resize_grab::ResizeEdge};

// The sizes and colors are the desktop's one look (vergence-look), which the
// toolkit's own bars are drawn with too: neither keeps a copy.
/// The title bar's height, in points.
pub const TITLE_H: i32 = vergence_look::TITLE_H as i32;
/// The resize band around a decorated window, in points.
pub const BORDER: i32 = vergence_look::RESIZE_BAND as i32;
/// The border line around a window, in points.
pub const EDGE: i32 = vergence_look::EDGE as i32;
const EDGE_ACTIVE: [f32; 4] = vergence_look::EDGE_FOCUSED.floats();
const EDGE_INACTIVE: [f32; 4] = vergence_look::EDGE_UNFOCUSED.floats();

const BAR_ACTIVE: [f32; 4] = vergence_look::BAR_FOCUSED.floats();
const BAR_INACTIVE: [f32; 4] = vergence_look::BAR_UNFOCUSED.floats();
const CLOSE: [f32; 4] = vergence_look::CLOSE.floats();
const BUTTON: [f32; 4] = vergence_look::BUTTON.floats();
const TEXT_ACTIVE: [f32; 3] = vergence_look::INK_FOCUSED.floats3();
const TEXT_INACTIVE: [f32; 3] = vergence_look::INK_UNFOCUSED.floats3();
/// Space before the title text and between it and the buttons, in points.
const TEXT_PAD: i32 = 8;

smithay::backend::renderer::element::render_elements! {
    pub SceneElement<=GlesRenderer>;
    Surface = WaylandSurfaceRenderElement<GlesRenderer>,
    Solid = SolidColorRenderElement,
    Memory = MemoryRenderBufferRenderElement<GlesRenderer>,
    Moving = crate::shelf::Moving,
}

/// What is under a point of the desktop.
pub enum Hit {
    Surface(Win, WlSurface, Point<f64, Logical>),
    Title(Win),
    Close(Win),
    Maximize(Win),
    Minimize(Win),
    Edge(Win, ResizeEdge),
    /// The body of a native window (native.rs): its input comes in step 4.
    Native(Win),
    /// The shelf's strip (shelf.rs): in front of every window.
    Shelf,
    Nothing,
}

/// The ids of one window's bar parts, for damage tracking, and its title rasters.
pub struct Decor {
    ids: [Id; 11],
    commit: CommitCounter,
    active: bool,
    /// One title raster per output scale (key: the scale's bits).
    titles: std::collections::HashMap<u64, TitleRaster>,
}

/// A title drawn for one scale: what it shows, and the pixels.
struct TitleRaster {
    key: (String, Size<i32, Logical>, bool),
    buffer: MemoryRenderBuffer,
}

impl Decor {
    fn new() -> Self {
        Decor {
            ids: std::array::from_fn(|_| Id::new()),
            commit: CommitCounter::default(),
            active: false,
            titles: Default::default(),
        }
    }
}

/// A decorated window's parts, in logical space.
struct Frame {
    content: Rectangle<i32, Logical>,
    title: Rectangle<i32, Logical>,
    close: Rectangle<i32, Logical>,
    maximize: Rectangle<i32, Logical>,
    /// Left of maximize; drawn and pressed only when the settings name the button.
    minimize: Rectangle<i32, Logical>,
}

impl Frame {
    fn of(content: Rectangle<i32, Logical>) -> Frame {
        let title = Rectangle::new((content.loc.x, content.loc.y - TITLE_H).into(), (content.size.w, TITLE_H).into());
        let close = Rectangle::new((title.loc.x + title.size.w - TITLE_H, title.loc.y).into(), (TITLE_H, TITLE_H).into());
        let maximize = Rectangle::new((close.loc.x - TITLE_H, title.loc.y).into(), (TITLE_H, TITLE_H).into());
        let minimize = Rectangle::new((maximize.loc.x - TITLE_H, title.loc.y).into(), (TITLE_H, TITLE_H).into());
        Frame { content, title, close, maximize, minimize }
    }

    /// Title bar and content.
    fn whole(&self) -> Rectangle<i32, Logical> {
        Rectangle::new(self.title.loc, (self.content.size.w, self.content.size.h + TITLE_H).into())
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
    pub fn has_title(&self, w: &Win) -> bool {
        if let Some(n) = w.native() {
            return !n.own_frame;
        }
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

    fn frame_of(&self, w: &Win) -> Option<Frame> {
        self.has_title(w).then(|| self.space.element_geometry(w).map(Frame::of)).flatten()
    }

    /// The rectangle the server outlines and surrounds with a resize band: the
    /// whole frame of a decorated window, or the geometry of a window that draws
    /// no frame of its own. None for fullscreen, override-redirect and
    /// client-decorated windows.
    pub fn border_of(&self, w: &Win) -> Option<Rectangle<i32, Logical>> {
        if let Some(f) = self.frame_of(w) {
            return Some(f.whole());
        }
        if let Some(x) = w.x11_surface() {
            if x.is_override_redirect() || x.is_fullscreen() {
                return None;
            }
        } else if w.toplevel().is_some_and(|t| t.with_committed_state(|c| c.is_some_and(|s| s.states.contains(xdg_toplevel::State::Fullscreen)))) {
            return None;
        }
        let geo = self.space.element_geometry(w)?;
        let bbox = self.space.element_bbox(w)?;
        (bbox == geo).then_some(geo)
    }

    /// The top-most thing under `pos`: a title bar part, a resize band, or a surface.
    /// True if the settings' window buttons include minimize (they do unless a
    /// layout without it has been chosen): the server's title bars then have it.
    pub fn minimize_button(&self) -> bool {
        match self.settings.current.get(&(crate::settings::WM.to_string(), "button-layout".to_string())) {
            Some(vergence_protocol::bus::SettingValue::Str(layout)) => layout.contains("minimize"),
            _ => true,
        }
    }

    pub fn hit(&self, pos: Point<f64, Logical>) -> Hit {
        if self.shelf_at(pos) {
            return Hit::Shelf;
        }
        for w in self.space.elements().rev() {
            if let Some(f) = self.frame_of(w) {
                if f.close.to_f64().contains(pos) {
                    return Hit::Close(w.clone());
                }
                if f.maximize.to_f64().contains(pos) {
                    return Hit::Maximize(w.clone());
                }
                if self.minimize_button() && f.minimize.to_f64().contains(pos) {
                    return Hit::Minimize(w.clone());
                }
                if f.title.to_f64().contains(pos) {
                    return Hit::Title(w.clone());
                }
            }
            if let Some(b) = self.border_of(w) {
                let whole = b.to_f64();
                let outer = Rectangle::new((b.loc.x - BORDER, b.loc.y - BORDER).into(), (b.size.w + 2 * BORDER, b.size.h + 2 * BORDER).into());
                if outer.to_f64().contains(pos) && !whole.contains(pos) {
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
            if w.native().is_some() {
                // No surfaces: the window's own rectangle (what covers it is above).
                return Hit::Native(w.clone());
            }
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
        let minimize_button = self.minimize_button();
        self.decor.retain(|w, _| w.alive());
        // A window on its way back from the dock is put in its place when it arrives.
        self.shelf_motions_end();
        let windows: Vec<Win> = self.space.elements().rev().cloned().collect();
        // In front of everything: the desktop menu, if open (menu.rs).
        let mut elements = self.menu_elements(renderer, out, scale);
        // Then notifications, in front of every window (notify.rs).
        elements.extend(self.notify_elements(renderer, out, scale));
        // Then the applications popup, if open (launcher.rs).
        elements.extend(self.launcher_elements(renderer, out, scale));
        // Then the shelf's strip, which no window covers (shelf.rs).
        elements.extend(self.shelf_elements(renderer, out, scale));
        // Then windows on their way into the dock or out of it.
        elements.extend(self.shelf_motion_elements(renderer, out, scale));
        for w in windows {
            let frame = self.frame_of(&w);
            let border = self.border_of(&w);
            let reach = border.map(|b| Rectangle::new((b.loc.x - EDGE, b.loc.y - EDGE).into(), (b.size.w + 2 * EDGE, b.size.h + 2 * EDGE).into())).or_else(|| self.space.element_bbox(&w));
            if !reach.is_some_and(|r| r.overlaps(out)) {
                continue;
            }
            if frame.is_some() || border.is_some() {
                let title = self.window_title(&w);
                let active = focused.as_ref().is_some_and(|k| match k {
                    crate::focus::KeyboardFocus::X11(x) => w.x11_surface() == Some(x),
                    crate::focus::KeyboardFocus::Wayland(s) => w.toplevel().is_some_and(|t| t.wl_surface() == s),
                crate::focus::KeyboardFocus::Native(n) => &w == n,
                });
                let d = self.decor.entry(w.clone()).or_insert_with(Decor::new);
                if d.active != active {
                    d.active = active;
                    d.commit.increment();
                }
                let solid = |i: usize, r: Rectangle<f64, Logical>, c: [f32; 4]| {
                    SceneElement::Solid(SolidColorRenderElement::new(d.ids[i].clone(), physical(r, out.loc, scale), d.commit, c, Kind::Unspecified))
                };
                // The border line, just outside the window (ids 6-9).
                if let Some(b) = border {
                    let (r, e) = (b.to_f64(), EDGE as f64);
                    let c = if active { EDGE_ACTIVE } else { EDGE_INACTIVE };
                    elements.push(solid(6, Rectangle::new((r.loc.x - e, r.loc.y - e).into(), (r.size.w + 2.0 * e, e).into()), c));
                    elements.push(solid(7, Rectangle::new((r.loc.x - e, r.loc.y + r.size.h).into(), (r.size.w + 2.0 * e, e).into()), c));
                    elements.push(solid(8, Rectangle::new((r.loc.x - e, r.loc.y).into(), (e, r.size.h).into()), c));
                    elements.push(solid(9, Rectangle::new((r.loc.x + r.size.w, r.loc.y).into(), (e, r.size.h).into()), c));
                }
                if let Some(f) = frame {
                // Front to back: title text, button marks, then the bar.
                let text_x = f.title.loc.x + TEXT_PAD;
                let buttons_from = if minimize_button { f.minimize.loc.x } else { f.maximize.loc.x };
                let logical: Size<i32, Logical> = (buttons_from - TEXT_PAD / 2 - text_x, TITLE_H).into();
                if let Some(font) = self.title_font.as_ref()
                    && logical.w > 0
                    && !title.is_empty()
                {
                    // Pixels exactly as Smithay places `logical` at this scale: drawn 1:1.
                    let (pw, ph) = ((logical.w as f64 * scale).round() as i32, (logical.h as f64 * scale).round() as i32);
                    let key = (title.clone(), logical, active);
                    let raster = d.titles.entry(scale.to_bits()).or_insert_with(|| TitleRaster {
                        key: (String::new(), Size::default(), false),
                        buffer: MemoryRenderBuffer::new(Fourcc::Abgr8888, (1, 1), 1, Transform::Normal, None),
                    });
                    if raster.key != key {
                        let rgba = font.render(&title, pw as usize, ph as usize, scale, if active { TEXT_ACTIVE } else { TEXT_INACTIVE });
                        raster.buffer = MemoryRenderBuffer::from_slice(&rgba, Fourcc::Abgr8888, (pw, ph), 1, Transform::Normal, None);
                        raster.key = key;
                    }
                    let at = (Point::<i32, Logical>::from((text_x, f.title.loc.y)) - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
                    let whole = Rectangle::<f64, Logical>::from_size((pw as f64, ph as f64).into());
                    match MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        at.to_f64(),
                        &raster.buffer,
                        None,
                        Some(whole),
                        Some(logical),
                        Kind::Unspecified,
                    ) {
                        Ok(e) => elements.push(SceneElement::Memory(e)),
                        Err(e) => tracing::warn!("title text: {e:?}"),
                    }
                }
                let m = inset(f.maximize, 7.0);
                let t = 1.5;
                elements.push(solid(0, inset(f.close, 7.0), CLOSE));
                elements.push(solid(1, Rectangle::new(m.loc, (m.size.w, t).into()), BUTTON));
                elements.push(solid(2, Rectangle::new((m.loc.x, m.loc.y + m.size.h - t).into(), (m.size.w, t).into()), BUTTON));
                elements.push(solid(3, Rectangle::new(m.loc, (t, m.size.h).into()), BUTTON));
                elements.push(solid(4, Rectangle::new((m.loc.x + m.size.w - t, m.loc.y).into(), (t, m.size.h).into()), BUTTON));
                if minimize_button {
                    // A line along the foot of its square, as the terminal's bar draws it.
                    let n = inset(f.minimize, 7.0);
                    elements.push(solid(10, Rectangle::new((n.loc.x, n.loc.y + n.size.h - t).into(), (n.size.w, t).into()), BUTTON));
                }
                elements.push(solid(5, f.title.to_f64(), if active { BAR_ACTIVE } else { BAR_INACTIVE }));
                }
            }
            let Some(loc) = self.space.element_location(&w) else { continue };
            // A native window's picture (native.rs), drawn 1:1 at the scale it was
            // drawn for, from the window's top left corner.
            if let Some(n) = w.native() {
                if let Some(p) = n.picture() {
                    let at = (loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
                    let whole = Rectangle::<f64, Logical>::from_size((p.px.w as f64, p.px.h as f64).into());
                    match MemoryRenderBufferRenderElement::from_buffer(renderer, at.to_f64(), &p.buffer, None, Some(whole), Some(p.logical), Kind::Unspecified) {
                        Ok(e) => elements.push(SceneElement::Memory(e)),
                        Err(e) => tracing::warn!("native window picture: {e:?}"),
                    }
                }
                continue;
            }
            let render_loc = (loc - w.geometry().loc - out.loc).to_f64().to_physical(scale).to_i32_round();
            let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = w.render_elements(renderer, render_loc, scale.into(), 1.0);
            elements.extend(surfaces.into_iter().map(SceneElement::Surface));
        }
        // Behind everything: the wallpaper, at this output's own pixels (wallpaper.rs).
        if let Some(mode) = output.current_mode()
            && let Some(e) = self.wallpaper_element(renderer, &output.name(), mode.size, out.size)
        {
            elements.push(SceneElement::Memory(e));
        }
        elements
    }

    /// A window's title, as its app set it.
    pub fn window_title(&self, w: &Win) -> String {
        if let Some(n) = w.native() {
            return n.title();
        }
        if let Some(x) = w.x11_surface() {
            return x.title();
        }
        let Some(t) = w.toplevel() else { return String::new() };
        smithay::wayland::compositor::with_states(t.wl_surface(), |s| {
            s.data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                .and_then(|d| d.lock().unwrap().title.clone())
                .unwrap_or_default()
        })
    }

    /// Close a window as its app is asked to.
    pub fn close_window(&mut self, w: &Win) {
        if let Some(n) = w.native() {
            return self.native_close(n);
        }
        if let Some(x) = w.x11_surface() {
            let _ = x.close();
        } else if let Some(t) = w.toplevel() {
            t.send_close();
        }
    }

    /// Fill the monitor the window is mostly on (title bar included), or restore.
    pub fn toggle_maximize(&mut self, w: &Win) {
        if let Some(old) = self.restore.remove(w) {
            self.set_window_rect(w, old, false);
            self.remember(w);
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
        // A screen with the shelf is filled as far as the strip (shelf.rs).
        let out = self.work_area(out);
        let top = if self.has_title(w) { TITLE_H } else { 0 };
        let rect = Rectangle::new((out.loc.x, out.loc.y + top).into(), (out.size.w, out.size.h - top).into());
        self.restore.insert(w.clone(), geo);
        self.set_window_rect(w, rect, true);
        self.remember(w);
    }

    fn set_window_rect(&mut self, w: &Win, rect: Rectangle<i32, Logical>, maximized: bool) {
        if let Win::Native(n) = w {
            self.native_configure(n, rect.size, maximized);
        } else if let Some(x) = w.x11_surface() {
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
