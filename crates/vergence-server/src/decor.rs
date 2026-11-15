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


smithay::backend::renderer::element::render_elements! {
    pub SceneElement<=GlesRenderer>;
    Surface = WaylandSurfaceRenderElement<GlesRenderer>,
    Solid = SolidColorRenderElement,
    Memory = MemoryRenderBufferRenderElement<GlesRenderer>,
    Moving = crate::shelf::Moving,
    /// A native window's GPU picture (native.rs).
    Texture = smithay::backend::renderer::element::texture::TextureRenderElement<smithay::backend::renderer::gles::GlesTexture>,
    Shaded = crate::backdrop::Shaded,
    Backdrop = crate::backdrop::Backdrop,
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
    /// One of a native window's own bar buttons, by its place.
    Tool(Win, usize),
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
    /// The style it was last drawn in (its border's color follows it).
    light: bool,
    /// One bar picture per output scale (key: the scale's bits).
    bars: std::collections::HashMap<u64, BarPicture>,
    /// The border's four rounded corners per output scale, with the color
    /// they were drawn in (its own pictures: an element appears once a frame).
    corners: std::collections::HashMap<u64, ([u8; 3], [MemoryRenderBuffer; 4])>,
    /// The frost behind a Wayland or X11 window's bar (look milestone 1, step 5).
    backdrop: crate::backdrop::Slot,
    /// The frost of its own top strip, and its picture with rounded corners.
    frost: crate::backdrop::ShadedSlot,
    round: crate::backdrop::ShadedSlot,
    /// The line showing where a dragged tab would land in this bar.
    mark: Id,
    /// The shade over a parent a modal dialog blocks (dialogs.rs).
    dim: Id,
    /// The outline a modal dialog flashes when its parent is pressed.
    flash: [Id; 4],
    /// The ghost of a tab dragged out of this bar: what it shows (title, size,
    /// scale's bits, light) and its pixels.
    ghost: Option<((String, i32, i32, u64, bool), MemoryRenderBuffer, (i32, i32))>,
}

/// A bar drawn for one scale: what it shows, and the pixels.
struct BarPicture {
    /// What it shows: title, width, focused, hover, button layout, tabs, the
    /// one in front, accent; then light, frosted, alerts, frosted front tab, rounded.
    key: (String, i32, bool, Option<vergence_bar::Part>, String, Vec<String>, usize, Option<u32>, (bool, bool, Vec<bool>, bool, bool, Option<usize>)),
    buffer: MemoryRenderBuffer,
    /// Its pixels' width and height (the scale's, not the bar's points).
    px: (i32, i32),
}

/// A native window's tabs as the server draws them: titles, the one in
/// front, its accent, and each tab's id (what the program calls it).
struct Tabs {
    titles: Vec<String>,
    /// Which tabs run as the superuser (look milestone 1).
    alerts: Vec<bool>,
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
            light: false,
            bars: Default::default(),
            corners: Default::default(),
            backdrop: None,
            frost: Default::default(),
            round: Default::default(),
            mark: Id::new(),
            dim: Id::new(),
            flash: std::array::from_fn(|_| Id::new()),
            ghost: None,
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
    fn of(content: Rectangle<i32, Logical>, buttons: &(Vec<vergence_bar::WinButton>, Vec<vergence_bar::WinButton>), tabs: Option<Tabs>, tools: Option<(f32, usize)>, z: vergence_bar::Sizes) -> Frame {
        let h = z.bar_h as i32;
        let title = Rectangle::new((content.loc.x, content.loc.y - h).into(), (content.size.w, h).into());
        let layout = match (&tabs, tools) {
            (Some(t), _) => vergence_bar::Layout::sized(content.size.w as f32, t.titles.len(), t.shown, &buttons.0, &buttons.1, z),
            (None, Some((from, n))) => vergence_bar::Layout::tools_sized(content.size.w as f32, &buttons.0, &buttons.1, z, from, n),
            (None, None) => vergence_bar::Layout::plain_sized(content.size.w as f32, &buttons.0, &buttons.1, z),
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
        Rectangle::new(self.title.loc, (self.content.size.w, self.content.size.h + self.title.size.h).into())
    }
}

/// A logical rectangle on an output, in its physical pixels, corners rounded like
/// window positions are.
/// A color channel from 0-255 to 0.0-1.0.
fn unit(c: u8) -> f32 {
    c as f32 / 255.0
}

fn physical(r: Rectangle<f64, Logical>, output_loc: Point<i32, Logical>, scale: f64) -> Rectangle<i32, Physical> {
    let a = (r.loc - output_loc.to_f64()).to_physical(scale).to_i32_round::<i32>();
    let b = (r.loc + r.size.to_point() - output_loc.to_f64()).to_physical(scale).to_i32_round::<i32>();
    Rectangle::new(a, (b.x - a.x, b.y - a.y).into())
}

/// The border's four corner arcs (top left, top right, bottom left, bottom
/// right) for one scale and color: each RADIUS + EDGE points square, the
/// line EDGE wide on the circle concentric with the window's own corner.
fn corner_pictures(fonts: &mut vergence_text::Fonts, scale: f64, rgb: [u8; 3]) -> Option<[MemoryRenderBuffer; 4]> {
    let e = vergence_look::EDGE;
    let s = vergence_look::RADIUS + e;
    let color = vergence_scene::Color::rgb(rgb[0], rgb[1], rgb[2]);
    let mut out = Vec::with_capacity(4);
    for (right, bottom) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut scene = vergence_scene::Scene::new(s, s);
        let at = |far: bool| if far { s - e / 2.0 - 2.0 * s } else { e / 2.0 };
        scene.stroke(vergence_scene::Rect::new(at(right), at(bottom), 2.0 * s, 2.0 * s), vergence_look::RADIUS + e / 2.0, e, color);
        let p = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, vergence_scene::Color::rgba(0, 0, 0, 0)).ok()?;
        out.push(MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None));
    }
    out.try_into().ok()
}

/// A native window's picture as a GPU texture, with its size in pixels and
/// in points: imported the first time it is asked for, on whichever monitor
/// that is. None for a picture in shared memory or one that failed to import.
fn native_texture(renderer: &mut GlesRenderer, w: &Win) -> Option<(smithay::backend::renderer::element::texture::TextureBuffer<smithay::backend::renderer::gles::GlesTexture>, smithay::backend::renderer::gles::GlesTexture, smithay::utils::Size<i32, smithay::utils::Buffer>, smithay::utils::Size<i32, Logical>)> {
    let n = w.native()?;
    let p = n.picture()?;
    let crate::native::Content::Gpu(g) = &p.content else { return None };
    let mut g = g.lock().unwrap();
    if g.texture.is_none() && !g.failed {
        use smithay::backend::renderer::ImportDma;
        match renderer.import_dmabuf(&g.dmabuf, None) {
            Ok(t) => {
                g.gles = Some(t.clone());
                g.texture = Some(smithay::backend::renderer::element::texture::TextureBuffer::from_texture(renderer, t, 1, Transform::Normal, None));
            }
            Err(e) => {
                g.failed = true;
                tracing::warn!("native window GPU picture: {e:?}");
            }
        }
    }
    Some((g.texture.clone()?, g.gles.clone()?, p.px, p.logical))
}

impl Server {
    /// Does the server draw this window's title bar?
    pub fn has_title(&self, w: &Win) -> bool {
        // Every native window's bar is the server's (bar milestone 1, step 8).
        if w.native().is_some() {
            return true;
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
        let mut buttons = self.button_layout();
        // A dialog or palette has close only, on the side the controls are on, and no tabs.
        if matches!(self.window_style(w).kind, crate::style::Kind::Dialog | crate::style::Kind::Palette) {
            let left = buttons.0.contains(&vergence_bar::WinButton::Close) || buttons.1.is_empty();
            buttons = if left { (vec![vergence_bar::WinButton::Close], vec![]) } else { (vec![], vec![vergence_bar::WinButton::Close]) };
            let z = self.bar_sizes(w);
            return self.has_title(w).then(|| self.space.element_geometry(w).map(|g| Frame::of(g, &buttons, None, None, z))).flatten();
        }
        // No maximize button where the window cannot be maximized (style.rs).
        if !self.maximizable(w) {
            buttons.0.retain(|b| *b != vergence_bar::WinButton::Maximize);
            buttons.1.retain(|b| *b != vergence_bar::WinButton::Maximize);
        }
        let z = self.bar_sizes(w);
        let tools = tools_of(w).map(|(from, t)| (from, t.len()));
        self.has_title(w).then(|| self.space.element_geometry(w).map(|g| Frame::of(g, &buttons, tabs_of(w), tools, z))).flatten()
    }

    /// The size of controls for a screen (look milestone 1, step 8): its own
    /// setting (desktop.toml `[[displays.screen]] controls`), else, on
    /// Automatic, Large where a touch panel is on it and the desktop's
    /// choice (Settings > Appearance) elsewhere.
    pub fn bar_sizes_on(&self, o: &smithay::output::Output) -> vergence_bar::Sizes {
        let desktop = self.look_setting("size");
        let Some(id) = crate::memory::monitor_id_of(o) else { return vergence_bar::Sizes::of(desktop) };
        match self.settings.current.get(&(crate::settings::LOOK.to_string(), format!("size:{id}"))) {
            Some(vergence_protocol::bus::SettingValue::U32(step)) if *step <= 2 => vergence_bar::Sizes::of(*step),
            _ if self.touch_panels.iter().any(|p| self.touch_screen_id(p).as_deref() == Some(id.as_str())) => vergence_bar::LARGE,
            _ => vergence_bar::Sizes::of(desktop),
        }
    }

    /// The size of controls for the screen a point is on (the first screen
    /// when it is on none).
    pub fn bar_sizes_at(&self, p: Point<i32, Logical>) -> vergence_bar::Sizes {
        let on = self.space.outputs().find(|o| self.space.output_geometry(o).is_some_and(|g| g.contains(p))).or_else(|| self.space.outputs().next());
        on.map_or_else(|| vergence_bar::Sizes::of(self.look_setting("size")), |o| self.bar_sizes_on(o))
    }

    /// A window's size of controls: the screen its center is on (window
    /// memory's rule).
    pub fn bar_sizes(&self, w: &Win) -> vergence_bar::Sizes {
        let z = match self.space.element_geometry(w) {
            Some(g) => self.bar_sizes_at((g.loc.x + g.size.w / 2, g.loc.y + g.size.h / 2).into()),
            None => self.bar_sizes_at(self.pointer_location().to_i32_round()),
        };
        // A dialog's bar is slim, a palette's thin (window styles milestone 1):
        // the screen's proportions, scaled to 28 and 22 of the standard 36.
        let k = match self.window_style(w).kind {
            crate::style::Kind::Dialog => 28.0 / 36.0,
            crate::style::Kind::Palette => 22.0 / 36.0,
            _ => return z,
        };
        vergence_bar::Sizes { bar_h: (z.bar_h * k).round(), item_h: (z.item_h * k).round(), button_w: (z.button_w * k).round(), plus_w: 0.0, side: z.side, gap: z.gap }
    }

    /// A window's bar height in points.
    pub fn bar_h(&self, w: &Win) -> i32 {
        self.bar_sizes(w).bar_h as i32
    }

    /// The bar height on the screen that holds `area` (its middle).
    pub fn bar_h_in(&self, area: Rectangle<i32, Logical>) -> i32 {
        self.bar_sizes_at((area.loc.x + area.size.w / 2, area.loc.y + area.size.h / 2).into()).bar_h as i32
    }

    /// A window's bar: where it is on the desktop, and how it is laid out
    /// (tab drags and the tab menu, dispatch.rs).
    /// The line where a dragged tab would land, in the accent (dispatch.rs, tab_mark).
    fn tab_mark_element(&mut self, out: Rectangle<i32, Logical>, scale: f64) -> Option<SceneElement> {
        let (w, before) = self.tab_mark()?;
        let (bar, layout) = self.bar_of(&w)?;
        let x = vergence_bar::gap_x(&layout, before)?;
        let t = layout.tabs.first()?;
        let r = Rectangle::<f64, Logical>::new(
            (bar.loc.x as f64 + (x - vergence_bar::GAP_LINE / 2.0) as f64, bar.loc.y as f64 + t.y as f64 + 2.0).into(),
            (vergence_bar::GAP_LINE as f64, t.h as f64 - 4.0).into(),
        );
        if !r.overlaps(out.to_f64()) {
            return None;
        }
        let a = self.accent();
        let d = self.decor.entry(w).or_insert_with(Decor::new);
        let c = [unit(a.r), unit(a.g), unit(a.b), 1.0];
        Some(SceneElement::Solid(SolidColorRenderElement::new(d.mark.clone(), physical(r, out.loc, scale), d.commit, c, Kind::Unspecified)))
    }

    /// The ghost of a tab being dragged, under the pointer where it was
    /// taken hold of: the tab in the bar's front color, a little see-through.
    fn tab_ghost_element(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64, light: bool) -> Option<SceneElement> {
        let (w, index, at) = self.tab_ghost()?;
        let (_, layout) = self.bar_of(&w)?;
        let tab = *layout.tabs.get(index)?;
        let r = Rectangle::<f64, Logical>::new(at, (tab.w as f64, tab.h as f64).into());
        if !r.overlaps(out.to_f64()) {
            return None;
        }
        let title = tabs_of(&w).and_then(|t| t.titles.get(index).cloned()).unwrap_or_default();
        let style = vergence_look::bar_look(light);
        let key = (title.clone(), tab.w as i32, tab.h as i32, scale.to_bits(), light);
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        let d = self.decor.entry(w.clone()).or_insert_with(Decor::new);
        if d.ghost.as_ref().is_none_or(|(k, ..)| *k != key) {
            let face = fonts.face("sans-serif", 400, false);
            let mut scene = vergence_scene::Scene::new(tab.w, tab.h);
            let (f, i) = (style.front, style.ink);
            vergence_bar::draw_ghost(&mut scene, fonts, tab.w, tab.h, &title, face, vergence_bar::FONT_SIZE, vergence_scene::Color::rgba(f.0, f.1, f.2, 215), vergence_scene::Color::rgb(i.0, i.1, i.2));
            match vergence_render::Cpu::new().render(&scene, fonts, scale as f32, vergence_scene::Color::rgba(0, 0, 0, 0)) {
                Ok(p) => {
                    let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
                    d.ghost = Some((key, buffer, (p.width as i32, p.height as i32)));
                }
                Err(e) => {
                    tracing::warn!("tab ghost: {e}");
                    return None;
                }
            }
        }
        let (_, buffer, px) = d.ghost.as_ref()?;
        let whole = Rectangle::<f64, Logical>::from_size((px.0 as f64, px.1 as f64).into());
        let loc = (at - out.loc.to_f64()).to_physical(scale).to_i32_round::<i32>();
        MemoryRenderBufferRenderElement::from_buffer(renderer, loc.to_f64(), buffer, None, Some(whole), Some((tab.w.round() as i32, tab.h.round() as i32).into()), Kind::Unspecified)
            .ok()
            .map(SceneElement::Memory)
    }

    pub fn bar_of(&self, w: &Win) -> Option<(Rectangle<i32, Logical>, vergence_bar::Layout)> {
        self.frame_of(w).map(|f| (f.title, f.layout))
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

    /// The rounded corners' program, compiled on first use; None if it would
    /// not compile (windows are then square).
    fn round_program(&mut self, renderer: &mut GlesRenderer) -> Option<smithay::backend::renderer::gles::GlesTexProgram> {
        self.round
            .get_or_insert_with(|| {
                use smithay::backend::renderer::gles::{UniformName, UniformType};
                let uniforms = [UniformName::new("size", UniformType::_2f), UniformName::new("radius", UniformType::_1f)];
                match renderer.compile_custom_texture_shader(vergence_render::round::ROUND_SHADER, &uniforms) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        tracing::warn!("corners: the shader did not compile, windows stay square: {e:?}");
                        None
                    }
                }
            })
            .clone()
    }

    /// The frost's program, compiled on first use; None where bars are solid:
    /// Reduce transparency or High contrast on, a software renderer
    /// (llvmpipe, softpipe: the blur would cost more than it gives), or a
    /// shader that would not compile. Only windows that draw under their bar
    /// are frosted (their picture's top is what lies under it); the others
    /// keep a solid bar.
    pub fn frost_program(&mut self, renderer: &mut GlesRenderer) -> Option<smithay::backend::renderer::gles::GlesTexProgram> {
        if self.reduce_transparency() {
            return None;
        }
        let high_contrast = self.settings.current.get(&(crate::settings::APPEARANCE.to_string(), "contrast".to_string())) == Some(&vergence_protocol::bus::SettingValue::U32(1));
        if high_contrast {
            return None;
        }
        self.frost
            .get_or_insert_with(|| {
                use smithay::backend::renderer::gles::{UniformName, UniformType, ffi};
                let name = renderer
                    .with_context(|gl| unsafe {
                        let p = gl.GetString(ffi::RENDERER);
                        if p.is_null() { String::new() } else { std::ffi::CStr::from_ptr(p as *const std::ffi::c_char).to_string_lossy().into_owned() }
                    })
                    .unwrap_or_default();
                if name.contains("llvmpipe") || name.contains("softpipe") {
                    tracing::info!("frost: off, drawing in software ({name})");
                    return None;
                }
                let uniforms = [
                    UniformName::new("texel", UniformType::_2f),
                    UniformName::new("radius", UniformType::_1f),
                    UniformName::new("tint", UniformType::_4f),
                    UniformName::new("ink", UniformType::_3f),
                    UniformName::new("need", UniformType::_1f),
                    UniformName::new("corner", UniformType::_1f),
                    UniformName::new("bottom", UniformType::_1f),
                ];
                match renderer.compile_custom_texture_shader(vergence_render::blur::FROST_SHADER, &uniforms) {
                    Ok(p) => {
                        tracing::info!("frost: on ({name})");
                        Some(p)
                    }
                    Err(e) => {
                        tracing::warn!("frost: the shader did not compile, bars stay solid: {e:?}");
                        None
                    }
                }
            })
            .clone()
    }

    /// Reduce transparency (Settings > Appearance; look milestone 1): bars,
    /// the dock and menus solid.
    pub fn reduce_transparency(&self) -> bool {
        self.settings.current.get(&(crate::settings::LOOK.to_string(), "reduce-transparency".to_string())) == Some(&vergence_protocol::bus::SettingValue::Bool(true))
    }

    /// A number of the desktop's own look (org.vergence.look), or 0.
    pub fn look_setting(&self, key: &str) -> u32 {
        match self.settings.current.get(&(crate::settings::LOOK.to_string(), key.to_string())) {
            Some(vergence_protocol::bus::SettingValue::U32(v)) => *v,
            // Standard Frost and Standard size when unset.
            _ => if key == "frost" || key == "size" { 1 } else { 0 },
        }
    }

    /// Whether the desktop is in its light style (org.freedesktop.appearance
    /// color-scheme 2, from desktop.toml's `color-scheme = "light"`), which
    /// the bars, borders and notification cards follow.
    pub fn light_style(&self) -> bool {
        self.settings.current.get(&(crate::settings::APPEARANCE.to_string(), "color-scheme".to_string())) == Some(&vergence_protocol::bus::SettingValue::U32(2))
    }

    /// The style a window's bar is drawn in: the desktop's, except that with
    /// Style "App default" an app may have a bar of its own style. Built in
    /// for now (Dave, 2026-11-14: the settings app's bar light by default);
    /// the app settings milestone makes it each app's choice.
    pub fn bar_light(&self, w: &Win, desktop_light: bool) -> bool {
        if desktop_light {
            return true;
        }
        let scheme = self.settings.current.get(&(crate::settings::APPEARANCE.to_string(), "color-scheme".to_string()));
        let app_default = matches!(scheme, None | Some(vergence_protocol::bus::SettingValue::U32(0)));
        app_default && w.native().is_some_and(|n| LIGHT_BAR_APPS.contains(&n.app_id.as_str()))
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
                    Some(Part::Tool(i)) => return Hit::Tool(w.clone(), i),
                    Some(_) => return Hit::Title(w.clone()),
                    None => {}
                }
            }
            // A window of a fixed size has no resize band (style.rs).
            if let Some(b) = self.border_of(w).filter(|_| self.resizable(w)) {
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
                Hit::Tool(w, i) => Some((w, Part::Tool(i))),
                _ => None,
            }
        };
        let layout_text = self.button_layout_text();
        // Light or dark (Settings > Appearance > Style; look milestone 1).
        let light = self.light_style();
        let frost = self.frost_program(renderer);
        let round = self.round_program(renderer);
        let mut reinset: Vec<Win> = Vec::new();
        let strength = vergence_look::frost_strength(self.look_setting("frost"));
        let frost_r = vergence_look::frost_radius(self.look_setting("frost"));
        let front_frosted = self.look_setting("front-tab") == 1;
        self.decor.retain(|w, _| w.alive());
        // A window on its way back from the dock is put in its place when it arrives.
        self.shelf_motions_end();
        let windows: Vec<Win> = self.space.elements().rev().cloned().collect();
        // A maximize button rested on opens its layouts (snap.rs).
        self.layouts_tick();
        // Dialogs go where their parents went (dialogs.rs).
        self.dialogs_follow();
        // Native windows told the wallpaper's color behind them (tint.rs).
        self.tint_tick();
        // In front of everything: the desktop menu, if open (menu.rs).
        let mut elements = self.menu_elements(renderer, out, scale);
        // Then notifications, in front of every window (notify.rs).
        elements.extend(self.notify_elements(renderer, out, scale));
        // Then the applications popup, if open (launcher.rs).
        elements.extend(self.launcher_elements(renderer, out, scale));
        // Then Alt+Tab's list, once Alt has been held a moment (switch.rs).
        elements.extend(self.switcher_elements(renderer, out, scale));
        // Then the shelf's strip, which no window covers (shelf.rs).
        elements.extend(self.shelf_elements(renderer, out, scale));
        // Then windows on their way into the dock or out of it.
        elements.extend(self.shelf_motion_elements(renderer, out, scale));
        // Then a dragged tab's ghost, over every window (dispatch.rs, tab_ghost).
        elements.extend(self.tab_ghost_element(renderer, out, scale, light));
        // And the line showing where it would land: over every window too, since
        // a window moved by its only tab covers the bar it is let go on.
        elements.extend(self.tab_mark_element(out, scale));
        // Where a dragged window will go if let go now (snap.rs).
        elements.extend(self.snap_preview_element(renderer, out, scale, light));
        for w in windows {
            // This window's own bar style (bar_light).
            let light = self.bar_light(&w, light);
            let style = vergence_look::bar_look(light);
            // A parent a modal dialog blocks is dimmed, in front of its own bar
            // and content and behind the dialog (dialogs.rs).
            if self.modal_of(&w).is_some()
                && let Some(r) = self.border_of(&w).or_else(|| self.space.element_bbox(&w))
                && r.overlaps(out)
            {
                let top = if self.has_title(&w) { self.bar_h(&w) } else { 0 };
                let all = Rectangle::<f64, Logical>::new((r.loc.x as f64, (r.loc.y - top) as f64).into(), (r.size.w as f64, (r.size.h + top) as f64).into());
                let d = self.decor.entry(w.clone()).or_insert_with(Decor::new);
                let shade = if light { [0.0, 0.0, 0.0, 0.18] } else { [0.0, 0.0, 0.0, 0.35] };
                elements.push(SceneElement::Solid(SolidColorRenderElement::new(d.dim.clone(), physical(all, out.loc, scale), d.commit, shade, Kind::Unspecified)));
            }
            let frame = self.frame_of(&w);
            let border = self.border_of(&w);
            let reach = border.map(|b| Rectangle::new((b.loc.x - EDGE, b.loc.y - EDGE).into(), (b.size.w + 2 * EDGE, b.size.h + 2 * EDGE).into())).or_else(|| self.space.element_bbox(&w));
            if !reach.is_some_and(|r| r.overlaps(out)) {
                continue;
            }
            if frame.is_some() || border.is_some() {
                // A sidebar window's bar carries no title: its page shows its own
                // name (Dave, 2026-11-14: drop the heading over the page).
                let title = if self.window_style(&w).kind == crate::style::Kind::Sidebar { String::new() } else { self.window_title(&w) };
                let active = focused.as_ref() == Some(&w);
                let preview = self.tab_preview(&w);
                let d = self.decor.entry(w.clone()).or_insert_with(Decor::new);
                if d.active != active || d.light != light {
                    d.active = active;
                    d.light = light;
                    d.commit.increment();
                }
                // Rounded corners (look milestone 1, step 7): a native window whose
                // picture is on the GPU and that is not maximized; the others stay
                // square until their pictures can be cut too.
                let rounded = round.is_some() && frame.is_some() && !self.restore.contains_key(&w) && native_texture(renderer, &w).is_some();
                // A modal dialog whose parent was pressed flashes its border in the accent (dialogs.rs).
                let flash = self.flashing(&w).then(|| {
                    let a = self.accent();
                    [a.r as f32 / 255.0, a.g as f32 / 255.0, a.b as f32 / 255.0, 1.0]
                });
                let d = self.decor.get_mut(&w).unwrap();
                let solid = |i: usize, r: Rectangle<f64, Logical>, c: [f32; 4]| {
                    SceneElement::Solid(SolidColorRenderElement::new(d.ids[i].clone(), physical(r, out.loc, scale), d.commit, c, Kind::Unspecified))
                };
                // The border line, just outside the window (ids 6-9).
                if let Some(b) = border {
                    let (r, e) = (b.to_f64(), EDGE as f64);
                    let edge = if active { style.edge } else { style.edge_off };
                    // A modal dialog whose parent was pressed flashes its border in the accent.
                    let c = flash.unwrap_or_else(|| edge.floats());
                    // Rounded: the straight lines stop short of the corners, where
                    // four small arcs (drawn once per scale and color) carry the line round.
                    let s = if rounded { vergence_look::RADIUS as f64 + e } else { 0.0 };
                    let (ox, oy, ow, oh) = (r.loc.x - e, r.loc.y - e, r.size.w + 2.0 * e, r.size.h + 2.0 * e);
                    // Its flash: a bolder outline in the accent just outside the border
                    // (3 points), in front of everything of the window's.
                    if let Some(fc) = flash {
                        let k = 3.0;
                        let (fx, fy, fw, fh) = (ox - k, oy - k, ow + 2.0 * k, oh + 2.0 * k);
                        let sides = [
                            Rectangle::<f64, Logical>::new((fx, fy).into(), (fw, k).into()),
                            Rectangle::new((fx, fy + fh - k).into(), (fw, k).into()),
                            Rectangle::new((fx, fy + k).into(), (k, fh - 2.0 * k).into()),
                            Rectangle::new((fx + fw - k, fy + k).into(), (k, fh - 2.0 * k).into()),
                        ];
                        for (i, r) in sides.into_iter().enumerate() {
                            elements.push(SceneElement::Solid(SolidColorRenderElement::new(d.flash[i].clone(), physical(r, out.loc, scale), d.commit, fc, Kind::Unspecified)));
                        }
                    }
                    elements.push(solid(6, Rectangle::new((ox + s, oy).into(), (ow - 2.0 * s, e).into()), c));
                    elements.push(solid(7, Rectangle::new((ox + s, oy + oh - e).into(), (ow - 2.0 * s, e).into()), c));
                    elements.push(solid(8, Rectangle::new((ox, oy + s).into(), (e, oh - 2.0 * s).into()), c));
                    elements.push(solid(9, Rectangle::new((ox + ow - e, oy + s).into(), (e, oh - 2.0 * s).into()), c));
                    if rounded {
                        let rgb = [edge.0, edge.1, edge.2];
                        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
                        let d = self.decor.get_mut(&w).unwrap();
                        let fresh = d.corners.get(&scale.to_bits()).is_none_or(|(c, _)| *c != rgb);
                        if fresh && let Some(set) = corner_pictures(fonts, scale, rgb) {
                            d.corners.insert(scale.to_bits(), (rgb, set));
                        }
                        if let Some((_, set)) = d.corners.get(&scale.to_bits()) {
                            let side = s.round() as i32;
                            let px = (side as f64 * scale).round() as i32;
                            let whole = Rectangle::<f64, Logical>::from_size((px as f64, px as f64).into());
                            let places = [(ox, oy), (ox + ow - s, oy), (ox, oy + oh - s), (ox + ow - s, oy + oh - s)];
                            for (buffer, (x, y)) in set.iter().zip(places) {
                                let at = (Point::<f64, Logical>::from((x, y)) - out.loc.to_f64()).to_physical(scale);
                                if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(renderer, at, buffer, None, Some(whole), Some((side, side).into()), Kind::Unspecified) {
                                    elements.push(SceneElement::Memory(e));
                                }
                            }
                        }
                    }
                }
                // A window that draws under its bar and moved to a screen with another
                // size of controls: told its new inset after this frame.
                if let (Some(n), Some(f)) = (w.native(), frame.as_ref())
                    && n.inset() > 0
                    && n.inset() != f.title.size.h
                {
                    reinset.push(w.clone());
                }
                let d = self.decor.get_mut(&w).unwrap();
                // A native window that draws under its bar, with its picture on the
                // GPU: the strip at the picture's top is frosted into the bar.
                let under = frost.as_ref().filter(|_| w.native().is_some_and(|n| n.inset() > 0)).and_then(|_| native_texture(renderer, &w));
                // A Wayland or X11 window's bar: frosted over what is behind it.
                let behind = frost.is_some() && w.native().is_none();
                if let Some(f) = frame {
                    let frosted = under.is_some() || behind;
                    // The bar, vergence-bar's, drawn once per scale and again when what it shows changes.
                    let hover = hovered.as_ref().filter(|(hw, _)| *hw == w).map(|(_, p)| *p);
                    let width = f.title.size.w;
                    let (titles, shown, accent) = f.tabs.as_ref().map(|t| (t.titles.clone(), t.shown, t.accent)).unwrap_or_default();
                    let alerts = f.tabs.as_ref().map(|t| t.alerts.clone()).unwrap_or_default();
                    // A tab being dragged (dispatch.rs): left in its place, faint; its
                    // ghost follows the pointer and a line shows where it would land
                    // (Dave, 2026-11-12). The row is not reordered under the drag: a
                    // row drawn with fewer titles than its layout had a blank tab.
                    let dragged = preview.map(|(from, _)| from).filter(|&i| i < titles.len());
                    // The program's own buttons (icons, pressable): part of what the picture shows.
                    let tools: Vec<(vergence_bar::ToolIcon, bool)> = tools_of(&w).map(|(_, t)| t.into_iter().map(|(i, e, _)| (i, e)).collect()).unwrap_or_default();
                    let key = (title.clone(), width, active, hover, format!("{layout_text}{tools:?}"), titles.clone(), shown, accent, (light, frosted, alerts.clone(), front_frosted, rounded, dragged));
                    let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
                    let picture = d.bars.entry(scale.to_bits()).or_insert_with(|| BarPicture {
                        key: (String::new(), -1, false, None, String::new(), Vec::new(), 0, None, (false, false, Vec::new(), false, false, None)),
                        buffer: MemoryRenderBuffer::new(Fourcc::Abgr8888, (1, 1), 1, Transform::Normal, None),
                        px: (1, 1),
                    });
                    if picture.key != key && width > 0 {
                        let face = fonts.face("sans-serif", 400, false);
                        let mut scene = vergence_scene::Scene::new(width as f32, f.title.size.h as f32);
                        // The tab in front: the program's page color (the terminal's
                        // ground: look milestone 1, "front tab should be page color"),
                        // with whichever ink reads on it; else the style's.
                        let (front, ink) = match accent {
                            Some(c) => {
                                let page = vergence_look::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
                                (page, vergence_look::ink_on(page))
                            }
                            None if active => (style.front, style.ink),
                            None => (style.front_off, style.ink_off),
                        };
                        // Tab in front: Frosted (the setting) is a white veil on the bar, its ink the bar's.
                        let (pane, ink) = if front_frosted {
                            (vergence_scene::Color::rgba(255, 255, 255, vergence_look::front_veil(light)), if active { style.ink } else { style.ink_off })
                        } else {
                            (vergence_scene::Color::rgb(front.0, front.1, front.2), ink)
                        };
                        let look = vergence_bar::Look {
                            bar: style,
                            frosted,
                            focused: active,
                            pane,
                            pane_ink: vergence_scene::Color::rgb(ink.0, ink.1, ink.2),
                            font: face,
                            font_size: vergence_bar::FONT_SIZE,
                            titles: &titles,
                            alerts: &alerts,
                            title: &title,
                            shown,
                            hover,
                            dragged,
                            tools: &tools,
                        };
                        // Rounded: the bar's top corners cut to the one radius (its bottom goes on into the window).
                        if rounded {
                            scene.push_clip(vergence_scene::Rect::new(0.0, 0.0, width as f32, f.title.size.h as f32 + vergence_look::RADIUS), vergence_look::RADIUS);
                        }
                        vergence_bar::draw(&mut scene, fonts, &f.layout, &look);
                        if rounded {
                            scene.pop_clip();
                        }
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
                    // Behind it, the frost: the top of the window's own picture, blurred
                    // and tinted (vergence_render::blur), the tint raised where the
                    // bar's ink needs it.
                    if behind && let Some(program) = frost.clone() {
                        let front_alert = alerts.get(shown).copied().unwrap_or(false);
                        let (ground, ink) = match (front_alert, active) {
                            (true, true) => (style.alert_ground, style.ink),
                            (true, false) => (style.alert_ground_off, style.ink_off),
                            (false, true) => (style.ground, style.ink),
                            (false, false) => (style.ground_off, style.ink_off),
                        };
                        let look = crate::backdrop::Look {
                            tint: crate::backdrop::unit(ground),
                            strength,
                            ink: crate::backdrop::unit(ink),
                            need: vergence_look::frost_need(light, active),
                            radius: frost_r * scale as f32,
                            corner: 0.0,
                            bottom: false,
                        };
                        let px = (f.title.size.to_f64().to_physical(scale)).to_i32_round::<i32>();
                        let dst = Rectangle::<i32, Physical>::new(at, px);
                        if let Some(e) = crate::backdrop::element(renderer, &mut d.backdrop, dst, program, &look) {
                            elements.push(SceneElement::Backdrop(e));
                        }
                    }
                    if let (Some(program), Some((_, texture, px, logical))) = (frost.as_ref(), under.as_ref()) {
                        use smithay::backend::renderer::gles::{Uniform, UniformValue};
                        let per_point = px.w as f64 / logical.w.max(1) as f64;
                        let rows = (f.title.size.h as f64 * per_point).round().min(px.h as f64);
                        let strip = Rectangle::<f64, smithay::utils::Buffer>::from_size((px.w as f64, rows).into());
                        let dst = Rectangle::<i32, Physical>::new(at, f.title.size.to_f64().to_physical(scale).to_i32_round());
                        // The frost's tint: the bar's ground, the alarm's while a superuser's tab is in front.
                        let front_alert = alerts.get(shown).copied().unwrap_or(false);
                        let (ground, ink) = match (front_alert, active) {
                            (true, true) => (style.alert_ground, style.ink),
                            (true, false) => (style.alert_ground_off, style.ink_off),
                            (false, true) => (style.ground, style.ink),
                            (false, false) => (style.ground_off, style.ink_off),
                        };
                        let unit = |c: vergence_look::Rgb| (c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0);
                        let (gr, gg, gb) = unit(ground);
                        let (ir, ig, ib) = unit(ink);
                        let uniforms = vec![
                            Uniform::new("texel", UniformValue::_2f(1.0 / px.w.max(1) as f32, 1.0 / px.h.max(1) as f32)),
                            Uniform::new("radius", UniformValue::_1f(frost_r * per_point as f32)),
                            Uniform::new("tint", UniformValue::_4f(gr, gg, gb, strength)),
                            Uniform::new("ink", UniformValue::_3f(ir, ig, ib)),
                            Uniform::new("need", UniformValue::_1f(vergence_look::frost_need(light, active))),
                            Uniform::new("corner", UniformValue::_1f(if rounded { vergence_look::RADIUS * per_point as f32 } else { 0.0 })),
                        ];
                        elements.push(SceneElement::Shaded(crate::backdrop::Shaded::new(&mut d.frost, dst, strip, texture.clone(), program.clone(), uniforms)));
                    }
                }
            }
            let Some(loc) = self.space.element_location(&w) else { continue };
            // A native window's picture (native.rs), drawn 1:1 at the scale it was
            // drawn for, from the window's top left corner.
            if let Some(n) = w.native() {
                if let Some(p) = n.picture() {
                    // A window that draws under its bar starts that much higher.
                    let top = Point::<i32, Logical>::from((loc.x, loc.y - n.inset()));
                    let at = (top - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
                    let whole = Rectangle::<f64, Logical>::from_size((p.px.w as f64, p.px.h as f64).into());
                    match &p.content {
                        crate::native::Content::Memory(buffer) => {
                            match MemoryRenderBufferRenderElement::from_buffer(renderer, at.to_f64(), buffer, None, Some(whole), Some(p.logical), Kind::Unspecified) {
                                Ok(e) => elements.push(SceneElement::Memory(e)),
                                Err(e) => tracing::warn!("native window picture: {e:?}"),
                            }
                        }
                        crate::native::Content::Gpu(_) => {
                            if let Some((t, gles, _, _)) = native_texture(renderer, &w) {
                                // Its bottom corners cut to the one radius, unless maximized.
                                match round.as_ref().filter(|_| !self.restore.contains_key(&w) && self.frame_of(&w).is_some()) {
                                    Some(program) => {
                                        use smithay::backend::renderer::gles::{Uniform, UniformValue};
                                        let per_point = p.px.w as f32 / p.logical.w.max(1) as f32;
                                        let uniforms = vec![
                                            Uniform::new("size", UniformValue::_2f(p.px.w as f32, p.px.h as f32)),
                                            Uniform::new("radius", UniformValue::_1f(vergence_look::RADIUS * per_point)),
                                        ];
                                        let dst = Rectangle::<i32, Physical>::new(at, p.logical.to_f64().to_physical(scale).to_i32_round());
                                        let src = Rectangle::<f64, smithay::utils::Buffer>::from_size((p.px.w as f64, p.px.h as f64).into());
                                        let slot = &mut self.decor.entry(w.clone()).or_insert_with(Decor::new).round;
                                        elements.push(SceneElement::Shaded(crate::backdrop::Shaded::new(slot, dst, src, gles, program.clone(), uniforms)));
                                    }
                                    None => {
                                        let e = smithay::backend::renderer::element::texture::TextureRenderElement::from_texture_buffer(at.to_f64(), &t, None, Some(whole), Some(p.logical), Kind::Unspecified);
                                        elements.push(SceneElement::Texture(e));
                                    }
                                }
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
        for w in reinset {
            if let Some(n) = w.native() {
                self.native_send_configure(n);
            }
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
        // A window whose largest size is smaller than its screen stays as it is
        // (style.rs): no button, and a double-click on its bar does nothing.
        if !self.maximizable(w) {
            return;
        }
        let Some(mut geo) = self.space.element_geometry(w) else { return };
        // A snapped window maximized goes back, later, to where it was before
        // it was snapped (snap.rs).
        if let Some(before) = self.unsnap(w) {
            geo = before;
        }
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
        let top = if self.has_title(w) { self.bar_h_in(out) } else { 0 };
        let rect = Rectangle::new((out.loc.x, out.loc.y + top).into(), (out.size.w, out.size.h - top).into());
        self.restore.insert(w.clone(), geo);
        self.set_window_rect(w, rect, true);
        self.remember(w);
    }

    pub(crate) fn set_window_rect(&mut self, w: &Win, rect: Rectangle<i32, Logical>, maximized: bool) {
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
            alerts: tabs.iter().map(|t| t.alert).collect(),
            ids: tabs.iter().map(|t| t.id).collect(),
            accent,
        }),
        _ => None,
    }
}

/// Apps whose bar is light under Style "App default" (bar_light).
const LIGHT_BAR_APPS: &[&str] = &["vergence-settings"];

/// A native window's own bar buttons (window styles milestone 1): where the
/// title starts, and each button's icon, whether it can be pressed, and id.
fn tools_of(w: &Win) -> Option<(f32, Vec<(vergence_bar::ToolIcon, bool, u32)>)> {
    match w.native()?.bar()? {
        vergence_protocol::window::Bar::Tools { buttons, from } => Some((
            from as f32,
            buttons
                .iter()
                .map(|b| {
                    let icon = match b.icon {
                        vergence_protocol::window::BarIcon::Back => vergence_bar::ToolIcon::Back,
                        vergence_protocol::window::BarIcon::Forward => vergence_bar::ToolIcon::Forward,
                    };
                    (icon, b.enabled, b.id)
                })
                .collect(),
        )),
        _ => None,
    }
}

impl Server {
    /// The id of a native window's own bar button at `index`, if it can be pressed.
    pub fn bar_tool_id(&self, w: &Win, index: usize) -> Option<u32> {
        tools_of(w).and_then(|(_, t)| t.get(index).filter(|(_, enabled, _)| *enabled).map(|(_, _, id)| *id))
    }
}
