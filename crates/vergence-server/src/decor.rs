// Title bars drawn by the server (server milestone 1, 2026-11-07; the bar
// itself from vergence-bar since bar milestone 1, 2026-11-12).
//
// X11 windows that do not decorate themselves, Wayland windows that ask for
// server-side decorations (xdg-decoration) and native windows that leave their
// frame to the server get a title bar TITLE_H points tall above their content,
// and an invisible band BORDER points wide around the whole frame for resizing.
// The bar is vergence-bar's, the one the toolkit draws: its buttons where the
// desktop's layout puts them (Gtk/DecorationLayout, the settings app's Windows
// page, followed live), drawn into a picture at each monitor's scale and laid
// out and hit-tested by the same code, so what is drawn and what is pressed
// cannot disagree. Sizes are in points like everything else, so the bar is the
// same physical height on every monitor. Edges are rounded from logical to
// physical the same way as the window's own position, so bar and content meet
// without a seam.
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
    utils::{IsAlive, Logical, Physical, Point, Rectangle, Transform},
};
use smithay::backend::allocator::Fourcc;

use crate::{Server, dispatch::ResizeEdge};

// The sizes and colors are the desktop's one look (vergence-look), which the
// toolkit's own bars are drawn with too: neither keeps a copy.
/// The title bar's height, in points: the one bar's (vergence-bar), the
/// toolkit's too (it was 24 for the server's own until 2026-11-12).
pub const TITLE_H: i32 = vergence_bar::BAR_H as i32;
/// The resize band around a decorated window, in points.
pub const BORDER: i32 = vergence_look::RESIZE_BAND as i32;
/// The border line around a window, in points.
pub const EDGE: i32 = vergence_look::EDGE as i32;
const EDGE_ACTIVE: [f32; 4] = vergence_look::EDGE_FOCUSED.floats();
const EDGE_INACTIVE: [f32; 4] = vergence_look::EDGE_UNFOCUSED.floats();


smithay::backend::renderer::element::render_elements! {
    pub SceneElement<=GlesRenderer>;
    Surface = WaylandSurfaceRenderElement<GlesRenderer>,
    Solid = SolidColorRenderElement,
    Memory = MemoryRenderBufferRenderElement<GlesRenderer>,
    Moving = crate::shelf::Moving,
    /// A native window's GPU picture (native.rs).
    Texture = smithay::backend::renderer::element::texture::TextureRenderElement<smithay::backend::renderer::gles::GlesTexture>,
}

/// What is under a point of the desktop.
pub enum Hit {
    Surface(Win, WlSurface, Point<f64, Logical>),
    Title(Win),
    Close(Win),
    Maximize(Win),
    Minimize(Win),
    Edge(Win, ResizeEdge),
    /// A tab of a native window's bar the server draws (its place in the
    /// row), a tab's close button, the + (bar milestone 1).
    Tab(Win, usize),
    CloseTab(Win, usize),
    Plus(Win),
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
    /// One bar picture per output scale (key: the scale's bits).
    bars: std::collections::HashMap<u64, BarPicture>,
}

/// A bar drawn for one scale: what it shows, and the pixels.
struct BarPicture {
    key: (String, i32, bool, Option<vergence_bar::Part>, String, Vec<String>, usize, Option<u32>),
    buffer: MemoryRenderBuffer,
    /// Its pixels' width and height (the scale's, not the bar's points).
    px: (i32, i32),
}

/// A native window's tabs as the server draws them: titles, the one in
/// front, its accent, and each tab's id (what the program calls it).
struct Tabs {
    titles: Vec<String>,
    shown: usize,
    accent: Option<u32>,
    ids: Vec<u32>,
}

impl Decor {
    fn new() -> Self {
        Decor {
            ids: std::array::from_fn(|_| Id::new()),
            commit: CommitCounter::default(),
            active: false,
            bars: Default::default(),
        }
    }
}

/// A decorated window's parts, in logical space: its content and its bar,
/// laid out by vergence-bar.
struct Frame {
    content: Rectangle<i32, Logical>,
    title: Rectangle<i32, Logical>,
    layout: vergence_bar::Layout,
    /// A native window's tabs, if it described any (else a plain bar).
    tabs: Option<Tabs>,
}

impl Frame {
    fn of(content: Rectangle<i32, Logical>, buttons: &(Vec<vergence_bar::WinButton>, Vec<vergence_bar::WinButton>), tabs: Option<Tabs>) -> Frame {
        let title = Rectangle::new((content.loc.x, content.loc.y - TITLE_H).into(), (content.size.w, TITLE_H).into());
        let layout = match &tabs {
            Some(t) => vergence_bar::Layout::new(content.size.w as f32, t.titles.len(), t.shown, &buttons.0, &buttons.1),
            None => vergence_bar::Layout::plain(content.size.w as f32, &buttons.0, &buttons.1),
        };
        Frame { content, title, layout, tabs }
    }

    /// What of the bar is at `pos`, if anything.
    fn part_at(&self, pos: Point<f64, Logical>) -> Option<vergence_bar::Part> {
        if !self.title.to_f64().contains(pos) {
            return None;
        }
        self.layout.at((pos.x - self.title.loc.x as f64) as f32, (pos.y - self.title.loc.y as f64) as f32)
    }

    /// Title bar and content.
    fn whole(&self) -> Rectangle<i32, Logical> {
        Rectangle::new(self.title.loc, (self.content.size.w, self.content.size.h + TITLE_H).into())
    }
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
        let buttons = self.button_layout();
        self.has_title(w).then(|| self.space.element_geometry(w).map(|g| Frame::of(g, &buttons, tabs_of(w)))).flatten()
    }

    /// The id a native window gave the tab at `index` of its bar.
    pub fn tab_id(&self, w: &Win, index: usize) -> Option<u32> {
        tabs_of(w).and_then(|t| t.ids.get(index).copied())
    }

    /// The window buttons, left and right, as the settings say
    /// (Gtk/DecorationLayout; vergence-bar's default, on the left, if unset).
    pub fn button_layout(&self) -> (Vec<vergence_bar::WinButton>, Vec<vergence_bar::WinButton>) {
        vergence_bar::decoration_layout(&self.button_layout_text())
    }

    fn button_layout_text(&self) -> String {
        match self.settings.current.get(&(crate::settings::WM.to_string(), "button-layout".to_string())) {
            Some(vergence_protocol::bus::SettingValue::Str(layout)) => layout.clone(),
            _ => vergence_bar::DEFAULT_LAYOUT.to_string(),
        }
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

    pub fn hit(&self, pos: Point<f64, Logical>) -> Hit {
        if self.shelf_at(pos) {
            return Hit::Shelf;
        }
        for w in self.space.elements().rev() {
            if let Some(f) = self.frame_of(w) {
                use vergence_bar::{Part, WinButton};
                match f.part_at(pos) {
                    Some(Part::Button(WinButton::Close)) => return Hit::Close(w.clone()),
                    Some(Part::Button(WinButton::Maximize)) => return Hit::Maximize(w.clone()),
                    Some(Part::Button(WinButton::Minimize)) => return Hit::Minimize(w.clone()),
                    Some(Part::Tab(i)) => return Hit::Tab(w.clone(), i),
                    Some(Part::CloseTab(i)) => return Hit::CloseTab(w.clone(), i),
                    Some(Part::Plus) => return Hit::Plus(w.clone()),
                    Some(_) => return Hit::Title(w.clone()),
                    None => {}
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
        let focused = self.keyboard_window();
        // What of a bar the pointer is over, for its hover look (the scene is
        // built again whenever the pointer moves).
        let hovered: Option<(Win, vergence_bar::Part)> = {
            use vergence_bar::{Part, WinButton};
            match self.hit(self.pointer_location()) {
                Hit::Close(w) => Some((w, Part::Button(WinButton::Close))),
                Hit::Maximize(w) => Some((w, Part::Button(WinButton::Maximize))),
                Hit::Minimize(w) => Some((w, Part::Button(WinButton::Minimize))),
                Hit::Tab(w, i) => Some((w, Part::Tab(i))),
                Hit::CloseTab(w, i) => Some((w, Part::CloseTab(i))),
                Hit::Plus(w) => Some((w, Part::Plus)),
                _ => None,
            }
        };
        let layout_text = self.button_layout_text();
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
                let active = focused.as_ref() == Some(&w);
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
                    // The bar, vergence-bar's, drawn once per scale and again when what it shows changes.
                    let hover = hovered.as_ref().filter(|(hw, _)| *hw == w).map(|(_, p)| *p);
                    let width = f.title.size.w;
                    let (titles, shown, accent) = f.tabs.as_ref().map(|t| (t.titles.clone(), t.shown, t.accent)).unwrap_or_default();
                    let key = (title.clone(), width, active, hover, layout_text.clone(), titles.clone(), shown, accent);
                    let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
                    let picture = d.bars.entry(scale.to_bits()).or_insert_with(|| BarPicture {
                        key: (String::new(), -1, false, None, String::new(), Vec::new(), 0, None),
                        buffer: MemoryRenderBuffer::new(Fourcc::Abgr8888, (1, 1), 1, Transform::Normal, None),
                        px: (1, 1),
                    });
                    if picture.key != key && width > 0 {
                        let face = fonts.face("sans-serif", 400, false);
                        let mut scene = vergence_scene::Scene::new(width as f32, TITLE_H as f32);
                        // The tab in front: the desktop's color, or the program's accent (bar milestone 1).
                        let front = match accent {
                            Some(c) => vergence_scene::Color::rgb((c >> 16) as u8, (c >> 8) as u8, c as u8),
                            None => {
                                let c = if active { vergence_look::TAB_FRONT_FOCUSED } else { vergence_look::TAB_FRONT_UNFOCUSED };
                                vergence_scene::Color::rgb(c.0, c.1, c.2)
                            }
                        };
                        let ink = if active { vergence_look::INK_FOCUSED } else { vergence_look::INK_UNFOCUSED };
                        let look = vergence_bar::Look {
                            focused: active,
                            pane: front,
                            pane_ink: vergence_scene::Color::rgb(ink.0, ink.1, ink.2),
                            font: face,
                            font_size: vergence_bar::FONT_SIZE,
                            titles: &titles,
                            title: &title,
                            shown,
                            hover,
                        };
                        vergence_bar::draw(&mut scene, fonts, &f.layout, &look);
                        match vergence_render::Cpu::new().render(&scene, fonts, scale as f32, vergence_scene::Color::rgba(0, 0, 0, 0)) {
                            Ok(p) => {
                                picture.buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
                                picture.px = (p.width as i32, p.height as i32);
                                picture.key = key;
                            }
                            Err(e) => tracing::warn!("title bar: {e}"),
                        }
                    }
                    let at = (f.title.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
                    // All of the picture's pixels (drawn for this scale) into the bar's points:
                    // drawn 1:1 (left out, it was taken as points and shown magnified at
                    // scale 2: kepler, 2026-11-12).
                    let whole = Rectangle::<f64, Logical>::from_size((picture.px.0 as f64, picture.px.1 as f64).into());
                    match MemoryRenderBufferRenderElement::from_buffer(renderer, at.to_f64(), &picture.buffer, None, Some(whole), Some(f.title.size), Kind::Unspecified) {
                        Ok(e) => elements.push(SceneElement::Memory(e)),
                        Err(e) => tracing::warn!("title bar: {e:?}"),
                    }
                }
            }
            let Some(loc) = self.space.element_location(&w) else { continue };
            // A native window's picture (native.rs), drawn 1:1 at the scale it was
            // drawn for, from the window's top left corner.
            if let Some(n) = w.native() {
                if let Some(p) = n.picture() {
                    let at = (loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
                    let whole = Rectangle::<f64, Logical>::from_size((p.px.w as f64, p.px.h as f64).into());
                    match &p.content {
                        crate::native::Content::Memory(buffer) => {
                            match MemoryRenderBufferRenderElement::from_buffer(renderer, at.to_f64(), buffer, None, Some(whole), Some(p.logical), Kind::Unspecified) {
                                Ok(e) => elements.push(SceneElement::Memory(e)),
                                Err(e) => tracing::warn!("native window picture: {e:?}"),
                            }
                        }
                        crate::native::Content::Gpu(g) => {
                            // Imported the first time it is drawn, on whichever monitor that is.
                            let mut g = g.lock().unwrap();
                            if g.texture.is_none() && !g.failed {
                                use smithay::backend::renderer::ImportDma;
                                match renderer.import_dmabuf(&g.dmabuf, None) {
                                    Ok(t) => {
                                        g.texture = Some(smithay::backend::renderer::element::texture::TextureBuffer::from_texture(renderer, t, 1, Transform::Normal, None));
                                    }
                                    Err(e) => {
                                        g.failed = true;
                                        tracing::warn!("native window GPU picture: {e:?}");
                                    }
                                }
                            }
                            if let Some(t) = &g.texture {
                                let e = smithay::backend::renderer::element::texture::TextureRenderElement::from_texture_buffer(at.to_f64(), t, None, Some(whole), Some(p.logical), Kind::Unspecified);
                                elements.push(SceneElement::Texture(e));
                            }
                        }
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

/// A native window's tabs, if it described them (bar milestone 1).
fn tabs_of(w: &Win) -> Option<Tabs> {
    match w.native()?.bar()? {
        vergence_protocol::window::Bar::Tabs { tabs, front, accent, .. } => Some(Tabs {
            shown: tabs.iter().position(|t| t.id == front).unwrap_or(0),
            titles: tabs.iter().map(|t| t.title.clone()).collect(),
            ids: tabs.iter().map(|t| t.id).collect(),
            accent,
        }),
        vergence_protocol::window::Bar::Plain => None,
    }
}
