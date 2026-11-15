// The shelf (whitepaper §6.9; docs/shelf-milestone-1.md, step 1).
//
// A dock on the right edge of the primary screen, drawn by the server: a
// rounded panel as tall as what it holds, floating a gap in from the edge
// (Dave, 2026-11-10: "like a dock, rounded with the same radius as all the
// other elements"). It stands in a column that runs the height of the screen
// (called the strip below, as vergence-shell calls it), and a screen with the
// shelf has a WORK AREA, the screen less that column: maximize,
// default placement and fitting, the desktop menu and notification cards keep
// inside it, and a remembered window the strip would cover is moved clear.
// Where the strip is and what it leaves is vergence-shell's arithmetic; this
// file knows which screen, draws, and (in later steps) routes input.
//
// The strip is drawn with the project's own drawing code: a scene
// (vergence-scene) rendered by vergence-render's CPU renderer at each
// screen's exact pixels, kept until something in it changes. It is the first
// thing the server draws that way; title bars, the menu and notifications
// follow when the server's compositing moves onto the renderer.
//
// ~/.config/vergence/desktop.toml:
//
//     [shelf]
//     hide = "never"   # "windows" (or true): at an edge, the dock slides
//                      # away while a window covers where it stands;
//                      # "always": away until the pointer comes. Away, a
//                      # sliver stays (as LXDE's panel does) and windows have
//                      # the whole screen; the pointer on the sliver, or a
//                      # press on it, brings it out (step 6)
//     pinned = ["firefox", "vergence-term"]   # entry ids, in their order
//     opacity = 1.0    # the panel's, 0.3 to 1
//     place = "edge"   # or "free", with x, y and upright; edge = "right",
//                      # "left", "top", "bottom"; screen = "<id>"; gap = 1;
//                      # keep-clear = true
//
// The screen is the primary one ([displays] primary, else the first in
// arrangement order): vergence_profile::primary, the settings app's rule.

use crate::win::Win;
use serde::Deserialize;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                AsRenderElements, Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                surface::WaylandSurfaceRenderElement,
                utils::{Relocate, RelocateRenderElement, RescaleRenderElement},
            },
            gles::GlesRenderer,
        },
    },
    utils::{Logical, Point, Rectangle, Transform},
};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::Path,
    sync::Arc,
    time::SystemTime,
};

use smithay::{reexports::calloop::channel::{Event, channel}, utils::IsAlive};
use vergence_scene::{Color, Image, Point as ScenePoint, Rect as SceneRect, Scene};
use vergence_shell::{Edge, Place, Placement, Rect, desktop::Entry, running};

use crate::{Server, decor::SceneElement};

/// An icon as pixels: its size, and four bytes a pixel (red, green, blue,
/// alpha, not multiplied in), as a scene wants a picture.
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// An icon the strip has asked for: on its way, or as it came back (None:
/// there is no file for it, or the file could not be used; a letter stands in).
enum Asked {
    Pending,
    Done(Option<Arc<Icon>>),
}

/// Files larger than this, and pictures wider or taller, are not icons.
const MAX_ICON_FILE: u64 = 8 * 1024 * 1024;
const MAX_ICON_SIDE: u32 = 4096;

/// A file as an icon about `pixels` across. Run on the shelf's own thread:
/// decoding a picture is work on a file the server did not write, and is
/// kept off the thread that owns the screen and the input.
fn decode(path: &Path, pixels: u32) -> Option<Icon> {
    if std::fs::metadata(path).ok()?.len() > MAX_ICON_FILE {
        return None;
    }
    let pixels = pixels.clamp(8, 512);
    if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        let data = std::fs::read(path).ok()?;
        let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
        let size = tree.size();
        let scale = pixels as f32 / size.width().max(size.height()).max(1.0);
        let (w, h) = (((size.width() * scale).round() as u32).clamp(1, pixels), ((size.height() * scale).round() as u32).clamp(1, pixels));
        let mut canvas = resvg::tiny_skia::Pixmap::new(w, h)?;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut canvas.as_mut());
        // The drawing comes with alpha multiplied in; a scene's pictures do not.
        let rgba = canvas.pixels().iter().flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        }).collect();
        return Some(Icon { width: w, height: h, rgba });
    }
    let reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    if w == 0 || h == 0 || w > MAX_ICON_SIDE || h > MAX_ICON_SIDE {
        return None;
    }
    let picture = image::open(path).ok()?.into_rgba8();
    let scale = pixels as f32 / w.max(h) as f32;
    let (nw, nh) = (((w as f32 * scale).round() as u32).clamp(1, pixels), ((h as f32 * scale).round() as u32).clamp(1, pixels));
    let fitted = if (nw, nh) == (w, h) { picture } else { image::imageops::resize(&picture, nw, nh, image::imageops::FilterType::CatmullRom) };
    Some(Icon { width: nw, height: nh, rgba: fitted.into_raw() })
}

/// When each directory that can hold desktop entries was last changed.
fn apps_stamp() -> Vec<Option<SystemTime>> {
    vergence_shell::apps::data_dirs().iter().map(|d| std::fs::metadata(d.join("applications")).and_then(|m| m.modified()).ok()).collect()
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ShelfConfig {
    /// false or "never"; true or "windows": out of the way while a window
    /// covers it; "always": away until the pointer comes to it.
    pub hide: HideSetting,
    /// "edge" (along an edge, the default) or "free" (where it was put).
    pub place: Option<String>,
    /// "right" (the default), "left", "top" or "bottom".
    pub edge: Option<String>,
    /// The screen it is on, by its id; else the primary one.
    pub screen: Option<String>,
    /// A floating dock's top-left corner from its screen's, in points, and
    /// whether it stands upright (the default) or lies level.
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub upright: Option<bool>,
    /// The room between the dock and its edge and the windows, in points (6).
    pub gap: Option<i32>,
    /// At an edge: windows keep out of the dock's column (true).
    #[serde(rename = "keep-clear")]
    pub keep_clear: Option<bool>,
    /// How opaque the dock's panel is, 0.3 to 1 (1): less, and the desktop
    /// shows through it. Its icons and marks stay as they are.
    pub opacity: Option<f32>,
    /// Pinned applications, by desktop entry id, in their order on the dock.
    #[serde(default)]
    pub pinned: Vec<String>,
}

/// How `hide` is written: a word, or true and false (true is "windows").
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum HideSetting {
    Flag(bool),
    Word(String),
}

impl Default for HideSetting {
    fn default() -> Self {
        HideSetting::Flag(false)
    }
}

/// When the dock gets out of the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hide {
    Never,
    /// While a window covers where it stands (Dave, 2026-11-10: "visible
    /// unless a window came over it ... several desktops do this").
    Windows,
    /// Always, until the pointer comes to its sliver.
    Always,
}

impl HideSetting {
    pub fn mode(&self) -> Hide {
        match self {
            HideSetting::Flag(true) => Hide::Windows,
            HideSetting::Flag(false) => Hide::Never,
            HideSetting::Word(w) => match w.trim().to_ascii_lowercase().as_str() {
                "windows" | "dodge" | "true" => Hide::Windows,
                "always" => Hide::Always,
                _ => Hide::Never,
            },
        }
    }
}

/// What the menu of an application's icon can do.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Pin(String),
    Unpin(String),
    /// Start the application (again): the entry's id.
    Start(String),
    Close(Vec<Win>),
    /// Ask about forgetting an app's window places (its name in window
    /// memory): the settings app opens at its Windows page with the question.
    ForgetPlaces(String),
}

/// How long a started application's icon says it is starting, if no window
/// of it appears.
const STARTING: std::time::Duration = std::time::Duration::from_secs(15);

impl ShelfConfig {
    /// The dock's place as vergence-shell reckons with it.
    pub fn placement(&self) -> Placement {
        let place = match self.place.as_deref() {
            Some("free") => Place::Free { x: self.x.unwrap_or(0), y: self.y.unwrap_or(0), upright: self.upright.unwrap_or(true) },
            _ => Place::Edge(match self.edge.as_deref() {
                Some("left") => Edge::Left,
                Some("top") => Edge::Top,
                Some("bottom") => Edge::Bottom,
                _ => Edge::Right,
            }),
        };
        Placement { place, gap: self.gap.unwrap_or(vergence_shell::GAP), keep_clear: self.keep_clear.unwrap_or(true) }
    }
}

/// Write the dock's place into desktop.toml's [shelf] table, the rest of the
/// file kept (as the settings app keeps it when it writes): the file is read,
/// the keys set, and the whole written beside it and moved into its place,
/// readable by its owner only.
fn save_place(screen: Option<&str>, place: Place) -> std::io::Result<()> {
    save(|shelf| set_place(shelf, screen, place))
}

/// Write the pinned applications there, the same way.
fn save_pinned(pinned: &[String]) -> std::io::Result<()> {
    save(|shelf| {
        shelf.insert("pinned".into(), toml::Value::Array(pinned.iter().map(|p| p.as_str().into()).collect()));
    })
}

fn set_place(shelf: &mut toml::Table, screen: Option<&str>, place: Place) {
    for key in ["x", "y", "upright", "edge"] {
        shelf.remove(key);
    }
    match place {
        Place::Edge(edge) => {
            shelf.insert("place".into(), "edge".into());
            shelf.insert("edge".into(), match edge { Edge::Left => "left", Edge::Right => "right", Edge::Top => "top", Edge::Bottom => "bottom" }.into());
        }
        Place::Free { x, y, upright } => {
            shelf.insert("place".into(), "free".into());
            shelf.insert("x".into(), (x as i64).into());
            shelf.insert("y".into(), (y as i64).into());
            shelf.insert("upright".into(), upright.into());
        }
    }
    match screen {
        Some(id) => shelf.insert("screen".into(), id.into()),
        None => shelf.remove("screen"),
    };
}

fn save(change: impl FnOnce(&mut toml::Table)) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let Some(path) = crate::wallpaper::desktop_path() else { return Ok(()) };
    let mut table: toml::Table = std::fs::read_to_string(&path).ok().and_then(|t| t.parse().ok()).unwrap_or_default();
    let shelf = table.entry("shelf").or_insert_with(|| toml::Value::Table(Default::default()));
    let Some(shelf) = shelf.as_table_mut() else { return Ok(()) };
    change(shelf);
    let text = toml::to_string(&table).map_err(std::io::Error::other)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let beside = path.with_extension("toml.new");
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&beside)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&beside, &path)
}

/// A press on the dock that has not been let go: where it began, where the
/// dock was then, and whether the pointer has moved far enough since for it
/// to be a drag and not a click.
#[derive(Clone, Copy)]
pub struct Held {
    start: Point<f64, Logical>,
    dock: Rectangle<i32, Logical>,
    dragging: bool,
}

#[derive(Debug, Default, Deserialize)]
struct DesktopFile {
    #[serde(default)]
    shelf: ShelfConfig,
    #[serde(default)]
    displays: Primary,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Primary {
    primary: Option<String>,
}

/// What desktop.toml says about the shelf: its own table, and which screen is primary.
pub fn load() -> (ShelfConfig, Option<String>) {
    crate::wallpaper::desktop_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| match toml::from_str::<DesktopFile>(&t) {
            Ok(f) => (f.shelf, f.displays.primary),
            Err(e) => {
                tracing::warn!("desktop.toml [shelf]: {e}");
                Default::default()
            }
        })
        .unwrap_or_default()
}

#[derive(Default)]
pub struct Shelf {
    pub config: ShelfConfig,
    /// `[displays] primary`, as recorded (it may not be connected).
    pub primary: Option<String>,
    /// The strip's picture for a height in points, a scale and what it shows,
    /// kept until one of them changes.
    picture: Option<((i32, u64, u64), MemoryRenderBuffer)>,
    /// The frost behind the dock (look milestone 1, step 5): its element's
    /// id, the texture its capture goes into (kept while its size holds).
    backdrop: crate::backdrop::Slot,
    /// The applications this machine has (vergence-shell), read again when a
    /// directory of desktop entries changes.
    pub apps: Vec<Entry>,
    apps_stamp: Vec<Option<SystemTime>>,
    /// The windows that get an item, in the order they were first seen.
    order: Vec<Win>,
    /// Icons by name and pixel size, and the thread that decodes them.
    icons: HashMap<(String, u32), Asked>,
    decoder: Option<std::sync::mpsc::Sender<(String, String, u32)>>,
    /// The fonts a letter tile is written with, looked for when first needed.
    pub fonts: Option<vergence_text::Fonts>,
    /// Where the shelf's column was last reported in the log, so a move is said once.
    said: Option<Rectangle<i32, Logical>>,
    /// The dock as it was last drawn or pressed: what a point is tested against.
    pub dock: Option<Rectangle<i32, Logical>>,
    /// A press on the dock that is still down (and the button that made it),
    /// and, while it is being dragged, where the dock is.
    pub held: Option<(u32, Held)>,
    dragged: Option<Rectangle<i32, Logical>>,
    /// The item the pointer is over, and whether it has rested there long
    /// enough for the application's name to be shown beside it.
    hover: Option<usize>,
    named: bool,
    /// The items' names as last drawn, and the name's picture for a scale.
    names: Vec<String>,
    label: Option<((String, u64, bool), MemoryRenderBuffer, i32)>,
    /// Minimized windows, each with the place it goes back to. They are on no
    /// screen (the server's space does not hold them); their application's icon
    /// on the shelf is where they are.
    pub minimized: Vec<(Win, Point<i32, Logical>)>,
    /// Minimized windows shown on their way into their icon, or out of it.
    motions: Vec<Motion>,
    /// Applications started from the dock that have shown no window yet
    /// (entry id, when started, how many windows it had then).
    starting: Vec<(String, std::time::Instant, usize)>,
    /// Windows that opened behind the focused one (opened.rs): their icon asks
    /// for attention until they have had the keyboard.
    pub attention: Vec<Win>,
    /// Applications the person started (dock, launcher, session restore) and
    /// when, for opened.rs: the first window of each takes the keyboard. Kept
    /// apart from `starting`, which ends when the dock counts a new window,
    /// and a Wayland window is counted before it is first shown.
    pub launched: Vec<(String, std::time::Instant)>,
    /// With `hide`: the dock is out (shown); where it is sliding to and
    /// since when; since when the pointer has been away from it.
    pub out: bool,
    slide: Option<(std::time::Instant, bool)>,
    away: Option<std::time::Instant>,
    /// Brought out by the pointer or a press on its sliver: out until the
    /// pointer has been away from it a while.
    peek: bool,
}

/// A window drawn smaller, somewhere else: on its way into the dock or out.
pub type Moving = RelocateRenderElement<RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>>;

/// With `hide`: how long the dock takes to slide, and how long the pointer
/// is away from it before it goes.
const SLIDE: std::time::Duration = std::time::Duration::from_millis(160);
/// What stays of a hidden dock at its edge, in points.
const SLIVER: i32 = 3;
const AWAY: std::time::Duration = std::time::Duration::from_millis(700);

/// How long a window takes into its icon or out of it.
const MOTION: std::time::Duration = std::time::Duration::from_millis(220);

/// A window on its way between its place and its application's icon. It is
/// in the list of minimized windows throughout: going in, it is already
/// minimized and only its picture moves; coming back, it is put in its place
/// when the picture arrives.
struct Motion {
    window: Win,
    /// Its place on the desktop: where it left, or where it is going back to.
    place: Rectangle<i32, Logical>,
    start: std::time::Instant,
    back: bool,
}

impl Shelf {
    /// The shelf as desktop.toml has it, with nothing drawn yet.
    pub fn load() -> Shelf {
        let (config, primary) = load();
        Shelf { config, primary, apps: vergence_shell::apps::installed(), apps_stamp: apps_stamp(), ..Default::default() }
    }
}

fn rect(r: Rectangle<i32, Logical>) -> Rect {
    Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h)
}

fn rectangle(r: Rect) -> Rectangle<i32, Logical> {
    Rectangle::new((r.x, r.y).into(), (r.w, r.h).into())
}

impl Server {
    /// Once a second: the applications are read again if a directory of desktop
    /// entries changed (something was installed or removed).
    pub fn shelf_tick(&mut self) {
        let stamp = apps_stamp();
        if stamp != self.shelf.apps_stamp {
            self.shelf.apps_stamp = stamp;
            self.shelf.apps = vergence_shell::apps::installed();
            tracing::info!("shelf: {} applications installed", self.shelf.apps.len());
            self.redraw_all();
        }
    }

    /// The dock's applications: the pinned ones, then the others with a window
    /// open in the order they started; and each item's windows.
    pub fn shelf_items(&mut self) -> Vec<(running::Item, Vec<Win>)> {
        // The windows that are someone's (not menus or tooltips), oldest first;
        // the minimized ones with them.
        self.shelf.minimized.retain(|(w, _)| w.alive());
        let open: Vec<Win> = self.space.elements().cloned().chain(self.shelf.minimized.iter().map(|(w, _)| w.clone())).collect();
        self.shelf.order.retain(|w| w.alive() && open.contains(w));
        for w in open {
            if !self.shelf.order.contains(&w) {
                self.shelf.order.push(w);
            }
        }
        let focused = self.keyboard_window();
        let order = self.shelf.order.clone();
        let mut windows: Vec<Win> = Vec::new();
        let mut classes: Vec<(String, bool)> = Vec::new();
        for w in order {
            // A dialog or palette is its parent's, not a window of its own (dialogs.rs).
            if self.belongs_to_parent(&w) {
                continue;
            }
            let Some(id) = self.window_identity(&w) else { continue };
            let active = focused.as_ref() == Some(&w);
            classes.push((if id.class.is_empty() { id.instance } else { id.class }, active));
            windows.push(w);
        }
        let items: Vec<(running::Item, Vec<Win>)> = running::items(&self.shelf.apps, &self.shelf.config.pinned, &classes).into_iter().map(|item| {
            let theirs = item.windows.iter().map(|i| windows[*i].clone()).collect();
            (item, theirs)
        }).collect();
        // An application is starting until a window more of it is there, or it has had its time.
        let now = std::time::Instant::now();
        self.shelf.starting.retain(|(key, since, had)| now.duration_since(*since) < STARTING && items.iter().find(|(i, _)| i.key == *key).map_or(0, |(_, w)| w.len()) <= *had);
        items
    }

    /// The icon `name` at `pixels`, if it has been decoded; asked for if not.
    pub fn shelf_icon(&mut self, name: &str, pixels: u32) -> Option<Arc<Icon>> {
        if name.is_empty() {
            return None;
        }
        let key = (name.to_string(), pixels);
        match self.shelf.icons.get(&key) {
            Some(Asked::Done(icon)) => return icon.clone(),
            Some(Asked::Pending) => return None,
            None => {}
        }
        if self.shelf.decoder.is_none() {
            // One thread finds and decodes icons, one at a time; what it makes comes
            // back through the event loop. A file that never finishes decoding costs
            // that thread, not the session: its item keeps its letter.
            let (jobs, work) = std::sync::mpsc::channel::<(String, String, u32)>();
            let (done, results) = channel::<(String, u32, Option<Icon>)>();
            let spawned = std::thread::Builder::new().name("shelf-icons".into()).spawn(move || {
                for (theme, name, pixels) in work {
                    let icon = vergence_shell::icons::find(&theme, &name, pixels).and_then(|file| decode(&file, pixels));
                    if done.send((name, pixels, icon)).is_err() {
                        break;
                    }
                }
            });
            let listening = self.loop_handle.insert_source(results, |event, _, state| {
                if let Event::Msg((name, pixels, icon)) = event {
                    state.shelf.icons.insert((name, pixels), Asked::Done(icon.map(Arc::new)));
                    state.redraw_all();
                }
            });
            if spawned.is_ok() && listening.is_ok() {
                self.shelf.decoder = Some(jobs);
            }
        }
        let theme = match self.settings.current.get(&(crate::settings::INTERFACE.to_string(), "icon-theme".to_string())) {
            Some(vergence_protocol::bus::SettingValue::Str(t)) => t.clone(),
            _ => String::new(),
        };
        let sent = self.shelf.decoder.as_ref().is_some_and(|d: &std::sync::mpsc::Sender<(String, String, u32)>| d.send((theme, name.to_string(), pixels)).is_ok());
        self.shelf.icons.insert(key, if sent { Asked::Pending } else { Asked::Done(None) });
        None
    }

    /// Read the shelf's settings again (at start, and when desktop.toml changes).
    pub fn shelf_reload(&mut self) {
        self.touch_reload();
        let (config, primary) = load();
        if config != self.shelf.config || primary != self.shelf.primary {
            self.shelf.config = config;
            self.shelf.primary = primary;
            self.shelf_moved();
            // A change that moves nothing (its opacity) still shows at once: it
            // waited for the next frame something else asked for (Hubble,
            // 2026-11-13: "the slider does nothing").
            self.redraw_all();
        }
    }

    /// The screen the shelf is on: the one its settings name, if it is
    /// connected, else the primary one.
    pub fn shelf_screen(&self) -> Option<Rectangle<i32, Logical>> {
        let outputs = self.bus_outputs();
        let ids: Vec<String> = outputs.iter().map(|o| o.id.clone()).collect();
        let named = self.shelf.config.screen.as_deref().filter(|id| ids.iter().any(|i| i == id));
        let id = named.or_else(|| vergence_profile::primary(self.shelf.primary.as_deref(), &ids))?;
        let o = outputs.iter().find(|o| o.id == id)?;
        Some(Rectangle::new((o.x, o.y).into(), (o.width, o.height).into()))
    }

    /// The band the dock stands in at an edge, if windows are to keep out of it.
    pub fn shelf_rect(&self) -> Option<Rectangle<i32, Logical>> {
        let at = self.shelf_placement();
        vergence_shell::column(rect(self.shelf_screen()?), &at).filter(|_| at.keep_clear).map(rectangle)
    }

    /// The dock itself, as long as the apps button's place and `items` after
    /// it; while it is being dragged, where the pointer has taken it.
    fn shelf_dock(&mut self, items: usize) -> Option<Rectangle<i32, Logical>> {
        let at = self.shelf.config.placement();
        let mut dock = self.shelf.dragged.or_else(|| self.shelf_screen().and_then(|s| vergence_shell::dock(rect(s), &at, items)).map(rectangle));
        // Hidden: moved out past its edge as far as it is slid away.
        // Out or away as the mode says, judged where the dock stands when out.
        if self.shelf_hides()
            && let Some(full) = dock
        {
            let want = self.shelf_want_out(full);
            let going = self.shelf.slide.map_or(self.shelf.out, |s| s.1);
            if want != going {
                self.shelf_slide(want);
            }
        }
        // Away, as LXDE's panel hides: a sliver of it stays at the edge to be found.
        if self.shelf_hides()
            && let (Some(d), Place::Edge(edge)) = (dock.as_mut(), at.place)
        {
            let shown = self.shelf_shown();
            let thickness = if at.upright() { d.size.w } else { d.size.h };
            let off = ((1.0 - shown) * (thickness + at.gap - SLIVER) as f32).round() as i32;
            match edge {
                Edge::Right => d.loc.x += off,
                Edge::Left => d.loc.x -= off,
                Edge::Bottom => d.loc.y += off,
                Edge::Top => d.loc.y -= off,
            }
        }
        self.shelf.dock = dock;
        dock
    }

    /// Where the dock is as windows reckon with it: with `hide`, at an edge,
    /// they have the whole screen.
    fn shelf_placement(&self) -> Placement {
        let mut at = self.shelf.config.placement();
        if self.shelf.config.hide.mode() != Hide::Never && matches!(at.place, Place::Edge(_)) {
            at.keep_clear = false;
        }
        at
    }

    /// The dock hides itself now: `hide` is set, it is at an edge, and it is
    /// not being held or dragged.
    fn shelf_hides(&self) -> bool {
        self.shelf.config.hide.mode() != Hide::Never && matches!(self.shelf.config.placement().place, Place::Edge(_)) && self.shelf.held.is_none() && self.shelf.dragged.is_none()
    }

    /// Whether the dock should be out now, standing at `full` when it is.
    fn shelf_want_out(&self, full: Rectangle<i32, Logical>) -> bool {
        if self.shelf.peek || self.menu.is_some() || self.launcher_is_open() {
            return true;
        }
        match self.shelf.config.hide.mode() {
            Hide::Never => true,
            Hide::Always => false,
            // Out unless a window on the desktop overlaps where it stands.
            Hide::Windows => !self.space.elements().any(|w| self.space.element_geometry(w).is_some_and(|g| g.overlaps(full))),
        }
    }

    /// How far out the dock is, 0 (away) to 1 (shown).
    fn shelf_shown(&self) -> f32 {
        match self.shelf.slide {
            Some((since, out)) => {
                let t = (since.elapsed().as_secs_f32() / SLIDE.as_secs_f32()).min(1.0);
                if out { t } else { 1.0 - t }
            }
            None => self.shelf.out as u8 as f32,
        }
    }

    /// Slide the dock out (or away), frame by frame unless motion is not wanted.
    fn shelf_slide(&mut self, out: bool) {
        if self.shelf.out == out && self.shelf.slide.is_none() {
            return;
        }
        self.shelf.away = None;
        if !self.motion_wanted() {
            self.shelf.out = out;
            self.shelf.slide = None;
            self.redraw_all();
            return;
        }
        self.shelf.slide = Some((std::time::Instant::now(), out));
        let tick = smithay::reexports::calloop::timer::Timer::from_duration(std::time::Duration::from_millis(16));
        let _ = self.loop_handle.insert_source(tick, move |_, _, state| {
            state.redraw_all();
            match state.shelf.slide {
                Some((since, to)) if to == out && since.elapsed() < SLIDE => smithay::reexports::calloop::timer::TimeoutAction::ToDuration(std::time::Duration::from_millis(16)),
                Some((_, to)) if to == out => {
                    state.shelf.slide = None;
                    state.shelf.out = out;
                    smithay::reexports::calloop::timer::TimeoutAction::Drop
                }
                _ => smithay::reexports::calloop::timer::TimeoutAction::Drop,
            }
        });
    }

    /// With `hide`: the pointer on the hidden dock's sliver brings it out;
    /// away from it for a while, it goes again.
    fn shelf_hide_follow(&mut self, at: Point<f64, Logical>) {
        if !self.shelf_hides() {
            if self.shelf.slide.is_some() || !self.shelf.out {
                self.shelf.slide = None;
                self.shelf.out = true;
            }
            return;
        }
        let going_out = self.shelf.slide.map_or(self.shelf.out, |s| s.1);
        let on_dock = self.shelf.dock.is_some_and(|d| d.to_f64().contains(at));
        if !going_out {
            // The sliver touched: out at once, as LXDE's panel comes.
            if on_dock {
                self.shelf.peek = true;
                self.shelf_slide(true);
            }
            return;
        }
        if !self.shelf.peek {
            // Out because nothing covers it: it stays until something does.
            return;
        }
        // Brought out by the pointer: near it (or its menu open) it stays.
        let margin = 24.0;
        let near = self.shelf.dock.is_some_and(|d| {
            let d = d.to_f64();
            at.x >= d.loc.x - margin && at.x <= d.loc.x + d.size.w + margin && at.y >= d.loc.y - margin && at.y <= d.loc.y + d.size.h + margin
        });
        if near || self.menu.is_some() || self.launcher_is_open() {
            self.shelf.away = None;
        } else if self.shelf.away.is_none() {
            self.shelf.away = Some(std::time::Instant::now());
            let wait = smithay::reexports::calloop::timer::Timer::from_duration(AWAY);
            let _ = self.loop_handle.insert_source(wait, |_, _, state| {
                if state.shelf.away.is_some_and(|t| t.elapsed() >= AWAY - std::time::Duration::from_millis(20)) && state.menu.is_none() && !state.launcher_is_open() {
                    // Back to what the mode says (the next frame decides).
                    state.shelf.peek = false;
                    state.shelf.away = None;
                    state.redraw_all();
                }
                smithay::reexports::calloop::timer::TimeoutAction::Drop
            });
        }
    }

    /// What a screen leaves for windows: all of it, less the dock's band if the
    /// shelf is on it, at an edge, and windows are to keep clear.
    pub fn work_area(&self, screen: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        if self.shelf_screen() == Some(screen) { rectangle(vergence_shell::work_area(rect(screen), &self.shelf_placement())) } else { screen }
    }

    /// A window's place moved clear of the dock's band, if the band would cover part of it.
    pub fn clear_of_shelf(&self, window: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        match self.shelf_screen() {
            Some(screen) => rectangle(vergence_shell::clear_of_dock(rect(window), rect(screen), &self.shelf_placement())),
            None => window,
        }
    }

    /// True for a point on the dock: the windows behind it get nothing of the pointer.
    pub fn shelf_at(&self, at: Point<f64, Logical>) -> bool {
        let covered = self.shelf_screen().is_some_and(|s| self.fullscreen_covers(s));
        !covered && self.shelf.dock.is_some_and(|r| r.to_f64().contains(at))
    }

    /// The pointer moved. Over an item of the dock, and resting there half a
    /// second, the application's name appears beside its icon (a dock that
    /// stands upright has no room for names).
    pub fn shelf_hover(&mut self, at: Point<f64, Logical>) {
        self.shelf_hide_follow(at);
        // A press on the dock that moves far enough is a drag: the dock goes with
        // the pointer until it is let go.
        if let Some((_, held)) = self.shelf.held.as_mut() {
            let (dx, dy) = (at.x - held.start.x, at.y - held.start.y);
            held.dragging |= dx.abs() > 6.0 || dy.abs() > 6.0;
            if held.dragging {
                let to = Rectangle::new((held.dock.loc.x + dx.round() as i32, held.dock.loc.y + dy.round() as i32).into(), held.dock.size);
                if self.shelf.dragged != Some(to) {
                    self.shelf.dragged = Some(to);
                    self.shelf.named = false;
                    self.redraw_all();
                }
                return;
            }
        }
        let upright = self.shelf.config.placement().upright();
        let over = self.shelf.dock.and_then(|d| running::item_at(rect(d), self.shelf.names.len(), upright, at.x.floor() as i32, at.y.floor() as i32));
        if over == self.shelf.hover {
            return;
        }
        self.shelf.hover = over;
        if std::mem::take(&mut self.shelf.named) {
            self.redraw_all();
        }
        if over.is_some() {
            let rest = smithay::reexports::calloop::timer::Timer::from_duration(std::time::Duration::from_millis(500));
            let _ = self.loop_handle.insert_source(rest, move |_, _, state| {
                if state.shelf.hover == over && !state.shelf.named {
                    state.shelf.named = true;
                    state.redraw_all();
                }
                smithay::reexports::calloop::timer::TimeoutAction::Drop
            });
        }
    }

    /// Minimize a window: it leaves the screen and stays as its application's
    /// icon, and the keyboard goes to the window that is then in front.
    pub fn minimize(&mut self, w: &Win) {
        if w.x11_surface().is_some_and(|x| x.is_override_redirect()) || self.is_minimized(w) {
            return;
        }
        let Some(at) = self.space.element_location(w) else { return };
        // Its dialogs and palettes go with it (dialogs.rs).
        for c in self.children_of(w) {
            self.minimize(&c);
        }
        tracing::info!("shelf: minimized {:?}", self.window_title(w));
        self.shelf.minimized.push((w.clone(), at));
        if self.motion_wanted() {
            self.shelf.motions.push(Motion { window: w.clone(), place: Rectangle::new(at, w.geometry().size), start: std::time::Instant::now(), back: false });
        }
        self.space.unmap_elem(w);
        w.set_activated(false);
        if let Some(t) = w.toplevel() {
            t.send_pending_configure();
        }
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        let front = self.space.elements().next_back().cloned();
        match front {
            Some(front) => self.focus_window(&front, serial),
            None => self.unfocus_all(),
        }
        self.redraw_all();
    }

    pub fn is_minimized(&self, w: &Win) -> bool {
        self.shelf.minimized.iter().any(|(m, _)| m == w)
    }

    /// Motion on the desktop is wanted (the settings' reduce-motion is off).
    fn motion_wanted(&self) -> bool {
        !matches!(self.settings.current.get(&(crate::settings::INTERFACE.to_string(), "enable-animations".to_string())), Some(vergence_protocol::bus::SettingValue::Bool(false)))
    }

    /// Bring a minimized window back to where it was, in front, with the
    /// keyboard: at once, or after its picture has come out of its icon.
    pub fn unminimize(&mut self, w: &Win) {
        // Its dialogs and palettes come back with it (dialogs.rs).
        for c in self.children_of(w) {
            if self.is_minimized(&c) {
                self.unminimize_now(&c);
            }
        }
        let Some(&(_, at)) = self.shelf.minimized.iter().find(|(m, _)| m == w) else { return };
        if !self.motion_wanted() || !w.alive() {
            self.shelf.motions.retain(|m| m.window != *w);
            return self.unminimize_now(w);
        }
        // Already on its way back: nothing more to do. On its way in: it turns round.
        if self.shelf.motions.iter().any(|m| m.window == *w && m.back) {
            return;
        }
        let now = std::time::Instant::now();
        let mut start = now;
        if let Some(i) = self.shelf.motions.iter().position(|m| m.window == *w) {
            // As far out as it had come in.
            let gone = now.duration_since(self.shelf.motions.remove(i).start).min(MOTION);
            start = now - (MOTION - gone);
        }
        self.shelf.motions.push(Motion { window: w.clone(), place: Rectangle::new(at, w.geometry().size), start, back: true });
        self.redraw_all();
    }

    /// Motions that have run their time are done with; a window that was
    /// coming back is put in its place.
    pub fn shelf_motions_end(&mut self) {
        if self.shelf.motions.is_empty() {
            return;
        }
        let now = std::time::Instant::now();
        let (done, going): (Vec<Motion>, Vec<Motion>) = std::mem::take(&mut self.shelf.motions).into_iter().partition(|m| !m.window.alive() || now.duration_since(m.start) >= MOTION);
        self.shelf.motions = going;
        for m in done {
            if m.back {
                self.unminimize_now(&m.window);
            }
        }
        self.redraw_all();
    }

    /// The pictures of the windows on their way, as far as each has come:
    /// smaller and fainter towards its application's icon on the dock.
    pub fn shelf_motion_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        if self.shelf.motions.is_empty() {
            return Vec::new();
        }
        let items = self.shelf_items();
        let upright = self.shelf.config.placement().upright();
        let Some(dock) = self.shelf_dock(items.len()) else { return Vec::new() };
        let now = std::time::Instant::now();
        let mut elements = Vec::new();
        for m in &self.shelf.motions {
            let Some(index) = items.iter().position(|(_, windows)| windows.contains(&m.window)) else { continue };
            let icon = running::slot(rect(dock), index, upright).map_or(dock, |s| rectangle(running::icon_in(s)));
            let t = (now.duration_since(m.start).as_secs_f64() / MOTION.as_secs_f64()).clamp(0.0, 1.0);
            // 0 at its place, 1 in the icon; slow away from its place, quick into the icon.
            let t = if m.back { 1.0 - t } else { t };
            let e = t * t;
            let (w, h) = (m.place.size.w.max(1) as f64, m.place.size.h.max(1) as f64);
            let small = (icon.size.w as f64 / w).min(icon.size.h as f64 / h).min(1.0);
            let factor = 1.0 + (small - 1.0) * e;
            let from = (m.place.loc.x as f64 + w / 2.0, m.place.loc.y as f64 + h / 2.0);
            let to = (icon.loc.x as f64 + icon.size.w as f64 / 2.0, icon.loc.y as f64 + icon.size.h as f64 / 2.0);
            let corner: Point<f64, Logical> = (from.0 + (to.0 - from.0) * e - w * factor / 2.0, from.1 + (to.1 - from.1) * e - h * factor / 2.0).into();
            // Drawn at its own place, made smaller about its corner, then moved.
            let origin = (m.place.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            let render_loc = (m.place.loc - m.window.geometry().loc - out.loc).to_f64().to_physical(scale).to_i32_round();
            let shift = (corner - m.place.loc.to_f64()).to_physical(scale).to_i32_round::<i32>();
            let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = m.window.render_elements(renderer, render_loc, scale.into(), (1.0 - 0.7 * e) as f32);
            elements.extend(surfaces.into_iter().map(|s| SceneElement::Moving(RelocateRenderElement::from_element(RescaleRenderElement::from_element(s, origin, factor), shift, Relocate::Relative))));
        }
        elements
    }

    fn unminimize_now(&mut self, w: &Win) {
        let Some(index) = self.shelf.minimized.iter().position(|(m, _)| m == w) else { return };
        let (w, mut at) = self.shelf.minimized.remove(index);
        if !w.alive() {
            return;
        }
        // Its screen may have gone meanwhile: then onto the one under the pointer.
        let size = w.geometry().size;
        let center: Point<i32, Logical> = (at.x + size.w / 2, at.y + size.h / 2).into();
        let on_a_screen = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).any(|g| g.contains(center));
        if !on_a_screen && let Some(out) = self.output_at_pointer() {
            let area = self.work_area(out);
            at = (area.loc.x + 40, area.loc.y + 40).into();
        }
        tracing::info!("shelf: back {:?}", self.window_title(&w));
        self.space.map_element(w.clone(), at, true);
        self.focus_window(&w, smithay::utils::SERIAL_COUNTER.next_serial());
        self.redraw_all();
    }

    /// To a window from the shelf: back if it is minimized, else forward.
    pub fn shelf_to_window(&mut self, w: &Win) {
        if self.is_minimized(w) {
            self.unminimize(w);
        } else {
            self.focus_window(w, smithay::utils::SERIAL_COUNTER.next_serial());
        }
    }

    /// A press on the dock. Nothing happens yet: let go where it was pressed
    /// it is a click ([`Server::shelf_release`]); moved first, it drags the dock.
    pub fn shelf_press(&mut self, at: Point<f64, Logical>, button: u32) {
        self.shelf.named = false;
        // A press on a hidden dock's sliver (a finger has no hover) brings it
        // out, and does nothing else.
        if self.shelf_hides() && self.shelf_shown() < 1.0 {
            self.shelf.peek = true;
            self.shelf_slide(true);
            return;
        }
        if let Some(dock) = self.shelf.dock {
            self.shelf.held = Some((button, Held { start: at, dock, dragging: false }));
        }
    }

    /// The press on the dock was let go. After a drag: the dock settles along
    /// the edge it was dropped near, of whichever screen that is, or floats
    /// where it was dropped, and its place is written to the settings. After
    /// a click on an application with one window: that window comes forward
    /// and has the keyboard, or comes back if it was minimized, or, if it is
    /// the one that has the keyboard already, is minimized (Dave, 2026-11-09).
    /// With several: a list of them opens beside the icon.
    pub fn shelf_release(&mut self, at: Point<f64, Logical>) {
        let Some((_, held)) = self.shelf.held.take() else { return };
        if held.dragging {
            let dropped = self.shelf.dragged.take().unwrap_or(held.dock);
            let outputs = self.bus_outputs();
            let screens: Vec<Rect> = outputs.iter().map(|o| Rect::new(o.x, o.y, o.width, o.height)).collect();
            let upright = self.shelf.config.placement().upright();
            if let Some((screen, place)) = vergence_shell::settle(rect(dropped), upright, &screens) {
                let id = outputs[screen].id.clone();
                tracing::info!("shelf: dropped at {},{}: {place:?} on screen {id}", dropped.loc.x, dropped.loc.y);
                let c = &mut self.shelf.config;
                match place {
                    Place::Edge(edge) => {
                        c.place = Some("edge".into());
                        c.edge = Some(match edge { Edge::Left => "left", Edge::Right => "right", Edge::Top => "top", Edge::Bottom => "bottom" }.into());
                    }
                    Place::Free { x, y, upright } => {
                        c.place = Some("free".into());
                        (c.x, c.y, c.upright) = (Some(x), Some(y), Some(upright));
                    }
                }
                c.screen = Some(id.clone());
                if let Err(e) = save_place(Some(&id), place) {
                    tracing::warn!("shelf: its place could not be written: {e}");
                }
            }
            self.shelf.picture = None;
            self.shelf_moved();
            self.redraw_all();
            return;
        }
        let items = self.shelf_items();
        let Some(strip) = self.shelf_dock(items.len()) else { return };
        let upright = self.shelf.config.placement().upright();
        let Some(index) = running::item_at(rect(strip), items.len(), upright, at.x.floor() as i32, at.y.floor() as i32) else {
            // The dock's head is the apps button.
            let head = Rectangle::new(strip.loc, (running::SLOT, running::SLOT).into());
            if head.to_f64().contains(at) {
                self.launcher_toggle();
            }
            return;
        };
        let (item, windows) = &items[index];
        match windows.as_slice() {
            [] => self.shelf_act(Act::Start(item.key.clone())),
            [only] => {
                tracing::info!("shelf: to {}", item.name);
                let only = only.clone();
                if item.focused && !self.is_minimized(&only) {
                    self.minimize(&only);
                } else {
                    self.shelf_to_window(&only);
                }
            }
            several => {
                let titled: Vec<(Win, String)> = several
                    .iter()
                    .map(|w| {
                        let title = self.window_title(w);
                        (w.clone(), if title.is_empty() { item.name.clone() } else { title })
                    })
                    .collect();
                let slot = running::slot(rect(strip), index, upright).map_or(strip, rectangle);
                let towards = self.shelf_side(strip);
                self.open_window_list(slot, towards, titled);
            }
        }
    }

    /// The applications that are running and could be started again: those
    /// with a window and a desktop entry that needs no terminal, by entry id.
    pub fn shelf_running(&mut self) -> Vec<String> {
        let items = self.shelf_items();
        items
            .iter()
            .filter(|(item, windows)| !windows.is_empty() && self.shelf.apps.iter().any(|e| e.id == item.key && !e.terminal && !e.exec.is_empty()))
            .map(|(item, _)| item.key.clone())
            .collect()
    }

    /// Nothing rests on the dock any more (a finger that tapped it has gone).
    pub fn shelf_unhover(&mut self) {
        if self.shelf.hover.take().is_some() || self.shelf.named {
            self.shelf.named = false;
            self.redraw_all();
        }
    }

    /// A right press on the dock: on an application's icon, its menu (pin or
    /// unpin, a new window, close).
    pub fn shelf_menu(&mut self, at: Point<f64, Logical>) {
        self.shelf.named = false;
        let items = self.shelf_items();
        let Some(dock) = self.shelf_dock(items.len()) else { return };
        let upright = self.shelf.config.placement().upright();
        let Some(index) = running::item_at(rect(dock), items.len(), upright, at.x.floor() as i32, at.y.floor() as i32) else { return };
        let (item, windows) = &items[index];
        // Only an application with a desktop entry can be started or pinned.
        let known = self.shelf.apps.iter().any(|e| e.id == item.key);
        let mut acts = Vec::new();
        if known {
            acts.push((Act::Start(item.key.clone()), if windows.is_empty() { "Open" } else { "New window" }.to_string()));
            acts.push(if item.pinned { (Act::Unpin(item.key.clone()), "Unpin".to_string()) } else { (Act::Pin(item.key.clone()), "Pin".to_string()) });
        }
        // Only where window memory has places for it (settings milestone 2).
        if let Some(app) = self.shelf_places_name(&item.key, windows) {
            acts.push((Act::ForgetPlaces(app), "Forget window places...".to_string()));
        }
        match windows.len() {
            0 => {}
            1 => acts.push((Act::Close(windows.clone()), "Close".to_string())),
            n => acts.push((Act::Close(windows.clone()), format!("Close all {n} windows"))),
        }
        let slot = running::slot(rect(dock), index, upright).map_or(dock, rectangle);
        let towards = self.shelf_side(dock);
        self.open_shelf_menu(slot, towards, acts);
    }

    /// Do what an entry of an icon's menu says (or a click on a pinned
    /// application that is not running: start it).
    pub fn shelf_act(&mut self, act: Act) {
        match act {
            Act::Pin(key) => {
                if !self.shelf.config.pinned.contains(&key) {
                    self.shelf.config.pinned.push(key);
                }
                self.shelf_save_pinned();
            }
            Act::Unpin(key) => {
                self.shelf.config.pinned.retain(|p| *p != key);
                self.shelf_save_pinned();
            }
            Act::Start(key) => {
                let Some(entry) = self.shelf.apps.iter().find(|e| e.id == key).cloned() else { return };
                if entry.exec.is_empty() {
                    return;
                }
                // The entry's own arguments, with no shell between; one that
                // wants a terminal runs in ours (`vergence-term -e`).
                let mut command = if entry.terminal {
                    let mut c = std::process::Command::new("vergence-term");
                    c.arg("-e").args(&entry.exec);
                    c
                } else {
                    let mut c = std::process::Command::new(&entry.exec[0]);
                    c.args(&entry.exec[1..]);
                    c
                };
                match crate::spawn(&mut command) {
                    Ok(pid) => {
                        tracing::info!("shelf: started {} (pid {pid})", entry.name);
                        let had = self.shelf_items().iter().find(|(i, _)| i.key == key).map_or(0, |(_, w)| w.len());
                        self.shelf.starting.retain(|(k, ..)| *k != key);
                        self.shelf.launched.retain(|(k, since)| *k != key && since.elapsed() < STARTING);
                        self.shelf.launched.push((key.clone(), std::time::Instant::now()));
                        self.shelf.starting.push((key, std::time::Instant::now(), had));
                    }
                    Err(e) => tracing::warn!("shelf: {} could not be started: {e}", entry.name),
                }
            }
            Act::Close(windows) => {
                for w in windows.iter().filter(|w| w.alive()) {
                    self.close_window(w);
                }
            }
            Act::ForgetPlaces(app) => {
                if let Err(e) = crate::spawn(std::process::Command::new("vergence-settings").args(["--page", "windows", "--forget", &app])) {
                    tracing::warn!("shelf: the settings app could not be started: {e}");
                }
            }
        }
        self.redraw_all();
    }

    /// The name window memory keeps an icon's places under, if it keeps any:
    /// its windows' (class, else instance), else its entry's id ("org.gnome.Foo"
    /// tried whole and as "foo").
    fn shelf_places_name(&mut self, key: &str, windows: &[Win]) -> Option<String> {
        let known: Vec<String> = self.places_apps().into_iter().map(|(a, _)| a).collect();
        let mut names: Vec<String> = windows.iter().filter_map(|w| self.window_identity(w)).map(|id| id.app_name()).collect();
        let id = key.trim_end_matches(".desktop").to_lowercase();
        names.push(id.clone());
        if let Some(last) = id.rsplit('.').next() {
            names.push(last.to_string());
        }
        names.into_iter().find(|n| known.contains(n))
    }

    fn shelf_save_pinned(&mut self) {
        if let Err(e) = save_pinned(&self.shelf.config.pinned) {
            tracing::warn!("shelf: the pinned applications could not be written: {e}");
        }
    }

    /// The side of the dock its names and lists are on (its marks are on the
    /// other, the outside edge): the one facing
    /// the middle of its screen (the windows' side, at an edge).
    pub fn shelf_side(&self, dock: Rectangle<i32, Logical>) -> Edge {
        let screen = self.shelf_screen().unwrap_or(dock);
        let (dx, dy) = (dock.loc.x + dock.size.w / 2 - (screen.loc.x + screen.size.w / 2), dock.loc.y + dock.size.h / 2 - (screen.loc.y + screen.size.h / 2));
        match (self.shelf.config.placement().upright(), dx > 0, dy > 0) {
            (true, true, _) => Edge::Left,
            (true, false, _) => Edge::Right,
            (false, _, true) => Edge::Top,
            (false, _, false) => Edge::Bottom,
        }
    }

    /// The screens or the settings changed: say where the strip is now, and
    /// bring maximized windows on its screen inside the work area.
    pub fn shelf_moved(&mut self) {
        // What windows are left with on the shelf's screen is what can have changed.
        let now = self.shelf_screen().map(|s| self.work_area(s));
        if now == self.shelf.said {
            return;
        }
        self.shelf.said = now;
        match now {
            Some(r) => tracing::info!("shelf: {:?}; its screen's work area {},{} {}x{} points", self.shelf.config.placement().place, r.loc.x, r.loc.y, r.size.w, r.size.h),
            None => tracing::info!("shelf: no screen for it"),
        }
        // A maximized window fills its screen's work area, which has just changed.
        let maximized: Vec<_> = self.restore.keys().cloned().collect();
        for w in maximized {
            self.toggle_maximize(&w);
            self.toggle_maximize(&w);
        }
    }

    /// For `vctl shelf`.
    pub fn shelf_report(&mut self) -> String {
        let Some(s) = self.shelf_screen() else { return "no shelf: no screen is connected".into() };
        let at = self.shelf.config.placement();
        let w = self.work_area(s);
        let items = self.shelf_items();
        let hidden = if self.shelf_hides() && self.shelf_shown() <= 0.0 { " (away now)" } else { "" };
        let Some(dock) = self.shelf_dock(items.len()).or_else(|| vergence_shell::dock(rect(s), &at, items.len()).map(rectangle)) else {
            return "no shelf: its screen is too small for it".into();
        };
        let mut out = format!(
            "dock {},{} {}x{} {:?} gap {} keep-clear {} on screen {},{} {}x{}; work area {},{} {}x{}; hide {}{hidden}",
            dock.loc.x, dock.loc.y, dock.size.w, dock.size.h, at.place, at.gap, at.keep_clear, s.loc.x, s.loc.y, s.size.w, s.size.h, w.loc.x, w.loc.y, w.size.w, w.size.h, format!("{:?}", self.shelf.config.hide.mode()).to_lowercase()
        );
        for (i, (item, windows)) in items.iter().enumerate() {
            let place = running::slot(rect(dock), i, at.upright()).map_or("no room".to_string(), |s| format!("{},{}", s.x, s.y));
            let away = windows.iter().filter(|w| self.is_minimized(w)).count();
            out.push_str(&format!("\n{i} {:?} \"{}\" icon \"{}\" windows {}{}{} at {place}", item.key, item.name, item.icon, windows.len(), if away > 0 { format!(" ({away} minimized)") } else { String::new() }, if item.focused { " focused" } else { "" }));
        }
        out
    }

    /// The dock's opacity setting, 0.3 to 1 (1 if unset).
    fn shelf_opacity(&self) -> f32 {
        self.shelf.config.opacity.filter(|o| o.is_finite()).unwrap_or(1.0).clamp(0.3, 1.0)
    }

    /// The accent the settings name (Appearance), or the default.
    pub fn accent(&self) -> Color {
        match self.settings.current.get(&(crate::settings::APPEARANCE.to_string(), "accent-color".to_string())) {
            Some(vergence_protocol::bus::SettingValue::Rgb(r, g, b)) => Color::rgb((r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8),
            _ => {
                let a = vergence_look::ACCENT;
                Color::rgb(a.0, a.1, a.2)
            }
        }
    }

    /// The strip's picture on `out`, if it is on that screen.
    pub fn shelf_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        // A full-screen window in front on this screen: no dock over it (fullscreen.rs).
        if self.fullscreen_covers(out) {
            return Vec::new();
        }
        let color = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
        // Light or dark (look milestone 1).
        let light = self.light_style();
        let s = vergence_look::shelf_look(light);
        // What the dock shows: each item, its mark, and its icon once that has come.
        let items = self.shelf_items();
        let Some(strip) = self.shelf_dock(items.len()).filter(|r| r.overlaps(out)) else { return Vec::new() };
        let pixels = (running::ICON as f64 * scale).round() as u32;
        let icons: Vec<Option<Arc<Icon>>> = items.iter().map(|(item, _)| self.shelf_icon(&item.icon, pixels)).collect();
        let accent = self.accent();
        // An application all of whose windows are minimized is there, but faint.
        let away: Vec<bool> = items.iter().map(|(_, windows)| !windows.is_empty() && windows.iter().all(|w| self.is_minimized(w))).collect();
        // One that is starting says so with a mark that grows and shrinks, ten
        // steps a second (a still one where motion is not wanted).
        let moving = self.motion_wanted();
        // One whose window opened behind the focused one (opened.rs): its long
        // mark blinks, twice a second (steady where motion is not wanted).
        self.shelf.attention.retain(|w| w.alive());
        let asking: Vec<Option<bool>> = items
            .iter()
            .map(|(_, windows)| windows.iter().any(|w| self.shelf.attention.contains(w)).then(|| !moving || (self.start_time.elapsed().as_millis() / 500) % 2 == 0))
            .collect();
        let starting: Vec<Option<u8>> = items
            .iter()
            .map(|(item, _)| {
                let (_, since, _) = self.shelf.starting.iter().find(|(key, ..)| *key == item.key)?;
                Some(if moving { ((since.elapsed().as_millis() / 100) % 10) as u8 } else { 5 })
            })
            .collect();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for (((item, windows), icon), away) in items.iter().zip(&icons).zip(&away) {
            (&item.key, &item.name, windows.len(), item.focused, icon.is_some(), away).hash(&mut hasher);
        }
        starting.hash(&mut hasher);
        asking.hash(&mut hasher);
        // The frost behind the dock (look milestone 1, step 5): its ground is then
        // the frost, drawn under the picture; Reduce transparency (or no frost
        // on this renderer) leaves the dock as its own setting says, solid if that.
        let frost = self.frost_program(renderer);
        let glass = if frost.is_some() { 0 } else if self.reduce_transparency() { 255 } else { (self.shelf_opacity() * 255.0).round() as u8 };
        glass.hash(&mut hasher);
        light.hash(&mut hasher);
        (accent.r, accent.g, accent.b).hash(&mut hasher);
        // Level or upright, and which side faces the windows (its marks are on the other).
        let upright = self.shelf.config.placement().upright();
        let side = self.shelf_side(strip);
        (strip.size.w, upright, side as u8).hash(&mut hasher);
        let key = (strip.size.h, scale.to_bits(), hasher.finish());
        if self.shelf.picture.as_ref().map(|(k, _)| *k) != Some(key) {
            // The dock as a scene, in points: a rounded panel with a line around it,
            // on nothing (the desktop shows at its corners).
            let (w, h) = (strip.size.w as f32, strip.size.h as f32);
            let mut scene = Scene::new(w, h);
            let panel = s.dock;
            scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, Color::rgba(panel.0, panel.1, panel.2, glass));
            scene.stroke(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, vergence_look::EDGE, color(s.dock_edge));
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            let face = fonts.face("sans-serif", 700, false).or_else(|| fonts.face("sans-serif", 400, false));
            // The apps button at its head: nine dots.
            for i in 0..9 {
                let (column, row) = ((i % 3) as f32, (i / 3) as f32);
                scene.fill_rounded(SceneRect::new(14.0 + column * 8.0, 14.0 + row * 8.0, 4.0, 4.0), 1.0, color(s.mark));
            }
            let local = Rect::new(0, 0, strip.size.w, strip.size.h);
            for (index, ((item, windows), icon)) in items.iter().zip(&icons).enumerate() {
                let Some(slot) = running::slot(local, index, upright) else { break };
                let at = running::icon_in(slot);
                let square = SceneRect::new(at.x as f32, at.y as f32, at.w as f32, at.h as f32);
                // The mark: short and gray for an application that
                // is running, long and in the accent for the one that has the keyboard.
                let (long, mark) = match starting[index] {
                    // From 6 points to 18 and back in a second.
                    Some(step) => (6.0 + 12.0 * (1.0 - (step as f32 - 5.0).abs() / 5.0), accent),
                    None if asking[index] == Some(true) => (18.0, accent),
                    None if asking[index] == Some(false) => (6.0, color(s.mark)),
                    None if item.focused => (18.0, accent),
                    None => (6.0, color(s.mark)),
                };
                let faint = away[index];
                // No mark for one that is neither running nor starting.
                if !faint && (!windows.is_empty() || starting[index].is_some()) {
                    let (down, along) = (slot.y as f32 + (slot.h as f32 - long) / 2.0, slot.x as f32 + (slot.w as f32 - long) / 2.0);
                    // On the dock's outside edge, away from the windows (Dave, 2026-11-10).
                    let outside = match side {
                        Edge::Left => Edge::Right,
                        Edge::Right => Edge::Left,
                        Edge::Top => Edge::Bottom,
                        Edge::Bottom => Edge::Top,
                    };
                    let place = match outside {
                        Edge::Left => SceneRect::new(3.0, down, 3.0, long),
                        Edge::Right => SceneRect::new(w - 6.0, down, 3.0, long),
                        Edge::Top => SceneRect::new(along, 3.0, long, 3.0),
                        Edge::Bottom => SceneRect::new(along, h - 6.0, long, 3.0),
                    };
                    scene.fill_rounded(place, 1.5, mark);
                }
                match icon {
                    Some(icon) => {
                        // In the middle of its square, at its own shape.
                        let (iw, ih) = (icon.width as f32 / scale as f32, icon.height as f32 / scale as f32);
                        let picture = scene.image(Image { width: icon.width, height: icon.height, rgba: icon.rgba.clone() });
                        scene.draw_image(picture, SceneRect::new(square.x + (square.w - iw) / 2.0, square.y + (square.h - ih) / 2.0, iw, ih), if faint { 0.45 } else { 1.0 });
                    }
                    None => {
                        // No icon (yet, or at all): a tile with the name's first letter.
                        scene.fill_rounded(square, vergence_look::RADIUS, color(s.tile));
                        let letter: String = item.name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
                        if let Some(face) = face {
                            let size = 18.0;
                            let m = fonts.metrics(face, size);
                            let (_, width) = fonts.layout(face, size, &letter);
                            let origin = ScenePoint::new(square.x + (square.w - width) / 2.0, square.y + ((square.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round());
                            vergence_render::text(&mut scene, fonts, face, size, color(s.tile_ink), origin, &letter);
                        }
                    }
                }
            }
            let buffer = match vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) {
                Ok(p) => MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None),
                Err(e) => {
                    tracing::warn!("shelf: its picture could not be drawn: {e}");
                    return Vec::new();
                }
            };
            self.shelf.picture = Some((key, buffer));
        }
        self.shelf.names = items.iter().map(|(item, _)| item.name.clone()).collect();
        // The name of the application the pointer rests on, beside its icon on the
        // side the windows are.
        let mut elements = Vec::new();
        let named = self.shelf.hover.filter(|_| self.shelf.named && !self.menu_is_open() && self.shelf.dragged.is_none()).and_then(|i| Some((running::slot(rect(strip), i, upright)?, self.shelf.names.get(i)?.clone())));
        if let Some((slot, name)) = named {
            let key = (name.clone(), scale.to_bits(), light);
            if self.shelf.label.as_ref().map(|(k, _, _)| k) != Some(&key) {
                let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
                if let Some(face) = fonts.face("sans-serif", 400, false) {
                    let size = 13.5;
                    let m = fonts.metrics(face, size);
                    let (_, width) = fonts.layout(face, size, &name);
                    let (w, h) = ((width + 20.0).ceil().min(480.0), 28.0);
                    let mut scene = Scene::new(w, h);
                    scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, color(s.menu));
                    scene.stroke(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, vergence_look::EDGE, color(s.edge));
                    scene.push_clip(SceneRect::new(8.0, 0.0, w - 16.0, h), 0.0);
                    vergence_render::text(&mut scene, fonts, face, size, color(s.menu_ink), ScenePoint::new(10.0, ((h - (m.ascent + m.descent)) / 2.0 + m.ascent).round()), &name);
                    scene.pop_clip();
                    if let Ok(p) = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) {
                        let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
                        self.shelf.label = Some((key, buffer, w as i32));
                    }
                }
            }
            if let Some((_, buffer, w)) = &self.shelf.label {
                let (middle, across) = (slot.y + (slot.h - 28) / 2, slot.x + (slot.w - w) / 2);
                let (x, y) = match side {
                    Edge::Left => (strip.loc.x - 8 - w, middle),
                    Edge::Right => (strip.loc.x + strip.size.w + 8, middle),
                    Edge::Top => (across, strip.loc.y - 8 - 28),
                    Edge::Bottom => (across, strip.loc.y + strip.size.h + 8),
                };
                // Kept on its screen.
                let (x, y) = (x.min(out.loc.x + out.size.w - w).max(out.loc.x), y.min(out.loc.y + out.size.h - 28).max(out.loc.y));
                let area: Rectangle<i32, Logical> = Rectangle::new((x, y).into(), (*w, 28).into());
                let a = (area.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
                let b = (area.loc + area.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
                let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
                if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(area.size), Kind::Unspecified) {
                    elements.push(SceneElement::Memory(el));
                }
            }
        }
        let Some((_, buffer)) = &self.shelf.picture else { return elements };
        // Placed on whole pixels, as everything the server draws is.
        let a = (strip.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let b = (strip.loc + strip.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
        if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(strip.size), Kind::Unspecified) {
            elements.push(SceneElement::Memory(el));
        }
        // Behind it, the frost of what is behind the dock.
        if let Some(program) = frost {
            let dst = Rectangle::<i32, smithay::utils::Physical>::new(a, (b.x - a.x, b.y - a.y).into());
            let s = vergence_look::shelf_look(self.light_style());
            // The dock's own opacity (Settings > Dock) is the frost's strength, at
            // Standard as it is, scaled by the Frost step: it had been left out
            // since the frost came (Dave, 2026-11-13: "the slider does nothing").
            let step = vergence_look::frost_strength(self.look_setting("frost")) / vergence_look::FROST_STRENGTH;
            let look = crate::backdrop::Look {
                tint: crate::backdrop::unit(s.dock),
                strength: (self.shelf_opacity() * step).min(1.0),
                ink: crate::backdrop::unit(s.mark),
                // The marks (the apps button's dots, the running marks) are what
                // must read on it: WCAG's 3:1 for marks that are not text. The
                // icons stand on tiles of their own. (3.6 kept a light dock over
                // a dark wallpaper all but solid.)
                need: 3.0,
                radius: vergence_look::frost_radius(self.look_setting("frost")) * scale as f32,
                corner: vergence_look::RADIUS * scale as f32,
                bottom: true,
            };
            let mut slot = self.shelf.backdrop.take();
            if let Some(e) = crate::backdrop::element(renderer, &mut slot, dst, program, &look) {
                elements.push(SceneElement::Backdrop(e));
            }
            self.shelf.backdrop = slot;
        }
        elements
    }
}
