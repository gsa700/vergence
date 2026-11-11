// The shelf (whitepaper §6.9; docs/shelf-milestone-1.md, step 1).
//
// A dock on the right edge of the primary screen, drawn by the server: a
// rounded panel as tall as what it holds, floating a gap in from the edge
// (Dave, 2026-11-10: "like a dock, rounded with the same radius as all the
// other elements"). It stands in a column that runs the height of the screen
// (called the strip below, as vergence-shell calls it), and a screen with the
// shelf has a WORK AREA, the screen less that column: maximise,
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
//     hide = false     # read; does nothing until step 6
//
// The screen is the primary one ([displays] primary, else the first in
// arrangement order): vergence_profile::primary, the settings app's rule.

use serde::Deserialize;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{Kind, memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement}},
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

use smithay::{desktop::Window, reexports::calloop::channel::{Event, channel}, utils::IsAlive};
use vergence_scene::{Color, Image, Point as ScenePoint, Rect as SceneRect, Scene};
use vergence_shell::{Rect, desktop::Entry, running};

use crate::{Server, decor::SceneElement};

/// An icon as pixels: its size, and four bytes a pixel (red, green, blue,
/// alpha, not multiplied in), as a scene wants a picture.
pub struct Icon {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
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
    pub hide: bool,
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
    /// The applications this machine has (vergence-shell), read again when a
    /// directory of desktop entries changes.
    apps: Vec<Entry>,
    apps_stamp: Vec<Option<SystemTime>>,
    /// The windows that get an item, in the order they were first seen.
    order: Vec<Window>,
    /// Icons by name and pixel size, and the thread that decodes them.
    icons: HashMap<(String, u32), Asked>,
    decoder: Option<std::sync::mpsc::Sender<(String, String, u32)>>,
    /// The fonts a letter tile is written with, looked for when first needed.
    fonts: Option<vergence_text::Fonts>,
    /// Where the shelf's column was last reported in the log, so a move is said once.
    said: Option<Rectangle<i32, Logical>>,
    /// The dock as it was last drawn or pressed: what a point is tested against.
    dock: Option<Rectangle<i32, Logical>>,
    /// The item the pointer is over, and whether it has rested there long
    /// enough for the application's name to be shown beside it.
    hover: Option<usize>,
    named: bool,
    /// The items' names as last drawn, and the name's picture for a scale.
    names: Vec<String>,
    label: Option<((String, u64), MemoryRenderBuffer, i32)>,
    /// Minimised windows, each with the place it goes back to. They are on no
    /// screen (the server's space does not hold them); their application's icon
    /// on the shelf is where they are.
    pub minimised: Vec<(Window, Point<i32, Logical>)>,
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

    /// The applications with a window open, in the order they started, and each
    /// item's windows.
    pub fn shelf_items(&mut self) -> Vec<(running::Item, Vec<Window>)> {
        // The windows that are someone's (not menus or tooltips), oldest first;
        // the minimised ones with them.
        self.shelf.minimised.retain(|(w, _)| w.alive());
        let open: Vec<Window> = self.space.elements().cloned().chain(self.shelf.minimised.iter().map(|(w, _)| w.clone())).collect();
        self.shelf.order.retain(|w| w.alive() && open.contains(w));
        for w in open {
            if !self.shelf.order.contains(&w) {
                self.shelf.order.push(w);
            }
        }
        let focused = self.seat.get_keyboard().and_then(|k| k.current_focus());
        let order = self.shelf.order.clone();
        let mut windows: Vec<Window> = Vec::new();
        let mut classes: Vec<(String, bool)> = Vec::new();
        for w in order {
            let Some(id) = self.window_identity(&w) else { continue };
            let active = focused.as_ref().is_some_and(|k| match k {
                crate::focus::KeyboardFocus::X11(x) => w.x11_surface() == Some(x),
                crate::focus::KeyboardFocus::Wayland(s) => w.toplevel().is_some_and(|t| t.wl_surface() == s),
            });
            classes.push((if id.class.is_empty() { id.instance } else { id.class }, active));
            windows.push(w);
        }
        running::items(&self.shelf.apps, &classes).into_iter().map(|item| {
            let theirs = item.windows.iter().map(|i| windows[*i].clone()).collect();
            (item, theirs)
        }).collect()
    }

    /// The icon `name` at `pixels`, if it has been decoded; asked for if not.
    fn shelf_icon(&mut self, name: &str, pixels: u32) -> Option<Arc<Icon>> {
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
        let (config, primary) = load();
        if config != self.shelf.config || primary != self.shelf.primary {
            self.shelf.config = config;
            self.shelf.primary = primary;
            self.shelf_moved();
        }
    }

    /// The screen the shelf is on: the primary one.
    pub fn shelf_screen(&self) -> Option<Rectangle<i32, Logical>> {
        let outputs = self.bus_outputs();
        let ids: Vec<String> = outputs.iter().map(|o| o.id.clone()).collect();
        let primary = vergence_profile::primary(self.shelf.primary.as_deref(), &ids)?;
        let o = outputs.iter().find(|o| o.id == primary)?;
        Some(Rectangle::new((o.x, o.y).into(), (o.width, o.height).into()))
    }

    /// The column the shelf stands in, if a screen has it: what windows keep out of.
    pub fn shelf_rect(&self) -> Option<Rectangle<i32, Logical>> {
        vergence_shell::strip(rect(self.shelf_screen()?)).map(rectangle)
    }

    /// The dock itself, as tall as the apps button's place and `items` under it.
    fn shelf_dock(&mut self, items: usize) -> Option<Rectangle<i32, Logical>> {
        let dock = self.shelf_screen().and_then(|s| vergence_shell::dock(rect(s), items)).map(rectangle);
        self.shelf.dock = dock;
        dock
    }

    /// What a screen leaves for windows: all of it, less the strip if the shelf is on it.
    pub fn work_area(&self, screen: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        if self.shelf_screen() == Some(screen) { rectangle(vergence_shell::work_area(rect(screen))) } else { screen }
    }

    /// A window's place moved clear of the strip, if the strip would cover part of it.
    pub fn clear_of_shelf(&self, window: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        match self.shelf_screen() {
            Some(screen) => rectangle(vergence_shell::clear_of_strip(rect(window), rect(screen))),
            None => window,
        }
    }

    /// True for a point on the dock: the windows behind it get nothing of the pointer.
    pub fn shelf_at(&self, at: Point<f64, Logical>) -> bool {
        self.shelf.dock.is_some_and(|r| r.to_f64().contains(at))
    }

    /// The pointer moved. Over an item of the dock, and resting there half a
    /// second, the application's name appears beside its icon (a dock that
    /// stands upright has no room for names).
    pub fn shelf_hover(&mut self, at: Point<f64, Logical>) {
        let over = self.shelf.dock.and_then(|d| running::item_at(rect(d), self.shelf.names.len(), at.x.floor() as i32, at.y.floor() as i32));
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

    /// Minimise a window: it leaves the screen and stays as its application's
    /// icon, and the keyboard goes to the window that is then in front.
    pub fn minimise(&mut self, w: &Window) {
        if w.x11_surface().is_some_and(|x| x.is_override_redirect()) || self.is_minimised(w) {
            return;
        }
        let Some(at) = self.space.element_location(w) else { return };
        tracing::info!("shelf: minimised {:?}", self.window_title(w));
        self.shelf.minimised.push((w.clone(), at));
        self.space.unmap_elem(w);
        w.set_activated(false);
        if let Some(t) = w.toplevel() {
            t.send_pending_configure();
        }
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        let front = self.space.elements().next_back().cloned();
        match front {
            Some(front) => self.focus_window(&front, serial),
            None => {
                if let Some(k) = self.seat.get_keyboard() {
                    k.set_focus(self, None, serial);
                }
            }
        }
        self.redraw_all();
    }

    pub fn is_minimised(&self, w: &Window) -> bool {
        self.shelf.minimised.iter().any(|(m, _)| m == w)
    }

    /// Bring a minimised window back to where it was, in front, with the keyboard.
    pub fn unminimise(&mut self, w: &Window) {
        let Some(index) = self.shelf.minimised.iter().position(|(m, _)| m == w) else { return };
        let (w, mut at) = self.shelf.minimised.remove(index);
        if !w.alive() {
            return;
        }
        // Its screen may have gone meanwhile: then onto the one under the pointer.
        let size = w.geometry().size;
        let centre: Point<i32, Logical> = (at.x + size.w / 2, at.y + size.h / 2).into();
        let on_a_screen = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).any(|g| g.contains(centre));
        if !on_a_screen && let Some(out) = self.output_at_pointer() {
            let area = self.work_area(out);
            at = (area.loc.x + 40, area.loc.y + 40).into();
        }
        tracing::info!("shelf: back {:?}", self.window_title(&w));
        self.space.map_element(w.clone(), at, true);
        self.focus_window(&w, smithay::utils::SERIAL_COUNTER.next_serial());
        self.redraw_all();
    }

    /// To a window from the shelf: back if it is minimised, else forward.
    pub fn shelf_to_window(&mut self, w: &Window) {
        if self.is_minimised(w) {
            self.unminimise(w);
        } else {
            self.focus_window(w, smithay::utils::SERIAL_COUNTER.next_serial());
        }
    }

    /// A press on the dock. On an application with one window: that window
    /// comes forward and has the keyboard, or comes back if it was minimised,
    /// or, if it is the one that has the keyboard already, is minimised (Dave,
    /// 2026-11-09). With several: a list of them opens beside the icon. True if
    /// a list opened (the press's release is then the list's, not a window's).
    pub fn shelf_press(&mut self, at: Point<f64, Logical>) -> bool {
        let items = self.shelf_items();
        // The name beside an icon goes when the icon is pressed.
        self.shelf.named = false;
        let Some(strip) = self.shelf_dock(items.len()) else { return false };
        let Some(index) = running::item_at(rect(strip), items.len(), at.x.floor() as i32, at.y.floor() as i32) else { return false };
        let (item, windows) = &items[index];
        match windows.as_slice() {
            [] => false,
            [only] => {
                tracing::info!("shelf: to {}", item.name);
                let only = only.clone();
                if item.focused && !self.is_minimised(&only) {
                    self.minimise(&only);
                } else {
                    self.shelf_to_window(&only);
                }
                false
            }
            several => {
                let titled: Vec<(Window, String)> = several
                    .iter()
                    .map(|w| {
                        let title = self.window_title(w);
                        (w.clone(), if title.is_empty() { item.name.clone() } else { title })
                    })
                    .collect();
                let top = running::slot(rect(strip), index).map_or(strip.loc.y, |s| s.y);
                self.open_window_list((strip.loc.x, top).into(), titled);
                true
            }
        }
    }

    /// The screens or the settings changed: say where the strip is now, and
    /// bring maximised windows on its screen inside the work area.
    pub fn shelf_moved(&mut self) {
        let now = self.shelf_rect();
        if now == self.shelf.said {
            return;
        }
        self.shelf.said = now;
        match now {
            Some(r) => tracing::info!("shelf: its column at {},{} {}x{} points", r.loc.x, r.loc.y, r.size.w, r.size.h),
            None => tracing::info!("shelf: no screen for it"),
        }
        // A maximised window fills its screen's work area, which has just changed.
        let maximised: Vec<_> = self.restore.keys().cloned().collect();
        for w in maximised {
            self.toggle_maximize(&w);
            self.toggle_maximize(&w);
        }
    }

    /// For `vctl shelf`.
    pub fn shelf_report(&mut self) -> String {
        match (self.shelf_rect(), self.shelf_screen()) {
            (Some(r), Some(s)) => {
                let w = self.work_area(s);
                let items = self.shelf_items();
                let dock = self.shelf_dock(items.len()).unwrap_or(r);
                let mut out = format!(
                    "dock {},{} {}x{} in column {},{} {}x{} on screen {},{} {}x{}; work area {},{} {}x{}; hide {} (not yet acted on)",
                    dock.loc.x, dock.loc.y, dock.size.w, dock.size.h, r.loc.x, r.loc.y, r.size.w, r.size.h, s.loc.x, s.loc.y, s.size.w, s.size.h, w.loc.x, w.loc.y, w.size.w, w.size.h, self.shelf.config.hide
                );
                let r = dock;
                for (i, (item, windows)) in items.iter().enumerate() {
                    let place = running::slot(rect(r), i).map_or("no room".to_string(), |s| format!("{},{}", s.x, s.y));
                    let away = windows.iter().filter(|w| self.is_minimised(w)).count();
                    out.push_str(&format!("\n{i} {:?} \"{}\" icon \"{}\" windows {}{}{} at {place}", item.key, item.name, item.icon, windows.len(), if away > 0 { format!(" ({away} minimised)") } else { String::new() }, if item.focused { " focused" } else { "" }));
                }
                out
            }
            _ => "no shelf: no screen wide enough is connected".into(),
        }
    }

    /// The strip's picture on `out`, if it is on that screen.
    pub fn shelf_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        let colour = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
        // What the dock shows: each item, its mark, and its icon once that has come.
        let items = self.shelf_items();
        let Some(strip) = self.shelf_dock(items.len()).filter(|r| r.overlaps(out)) else { return Vec::new() };
        let pixels = (running::ICON as f64 * scale).round() as u32;
        let icons: Vec<Option<Arc<Icon>>> = items.iter().map(|(item, _)| self.shelf_icon(&item.icon, pixels)).collect();
        let accent = match self.settings.current.get(&(crate::settings::APPEARANCE.to_string(), "accent-color".to_string())) {
            Some(vergence_protocol::bus::SettingValue::Rgb(r, g, b)) => Color::rgb((r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8),
            _ => colour(vergence_look::ACCENT),
        };
        // An application all of whose windows are minimised is there, but faint.
        let away: Vec<bool> = items.iter().map(|(_, windows)| windows.iter().all(|w| self.is_minimised(w))).collect();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for (((item, windows), icon), away) in items.iter().zip(&icons).zip(&away) {
            (&item.key, &item.name, windows.len(), item.focused, icon.is_some(), away).hash(&mut hasher);
        }
        (accent.r, accent.g, accent.b).hash(&mut hasher);
        let key = (strip.size.h, scale.to_bits(), hasher.finish());
        if self.shelf.picture.as_ref().map(|(k, _)| *k) != Some(key) {
            // The dock as a scene, in points: a rounded panel with a line around it,
            // on nothing (the desktop shows at its corners).
            let (w, h) = (strip.size.w as f32, strip.size.h as f32);
            let mut scene = Scene::new(w, h);
            scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, colour(vergence_look::SHELF));
            scene.stroke(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, vergence_look::EDGE, colour(vergence_look::SHELF_EDGE));
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            let face = fonts.face("sans-serif", 700, false).or_else(|| fonts.face("sans-serif", 400, false));
            let local = Rect::new(0, 0, strip.size.w, strip.size.h);
            for (index, ((item, _), icon)) in items.iter().zip(&icons).enumerate() {
                let Some(slot) = running::slot(local, index) else { break };
                let at = running::icon_in(slot);
                let square = SceneRect::new(at.x as f32, at.y as f32, at.w as f32, at.h as f32);
                // The mark, on the windows' side: short and grey for an application that
                // is running, long and in the accent for the one that has the keyboard.
                let (mark_h, mark) = if item.focused { (18.0, accent) } else { (6.0, colour(vergence_look::INK_UNFOCUSED)) };
                let faint = away[index];
                if !faint {
                    scene.fill_rounded(SceneRect::new(3.0, slot.y as f32 + (slot.h as f32 - mark_h) / 2.0, 3.0, mark_h), 1.5, mark);
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
                        scene.fill_rounded(square, vergence_look::RADIUS, colour(vergence_look::SHELF_TILE));
                        let letter: String = item.name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
                        if let Some(face) = face {
                            let size = 18.0;
                            let m = fonts.metrics(face, size);
                            let (_, width) = fonts.layout(face, size, &letter);
                            let origin = ScenePoint::new(square.x + (square.w - width) / 2.0, square.y + ((square.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round());
                            vergence_render::text(&mut scene, fonts, face, size, colour(vergence_look::INK_FOCUSED), origin, &letter);
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
        // The name of the application the pointer rests on, to the left of its icon.
        let mut elements = Vec::new();
        let named = self.shelf.hover.filter(|_| self.shelf.named).and_then(|i| Some((running::slot(rect(strip), i)?, self.shelf.names.get(i)?.clone())));
        if let Some((slot, name)) = named {
            let key = (name.clone(), scale.to_bits());
            if self.shelf.label.as_ref().map(|(k, _, _)| k) != Some(&key) {
                let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
                if let Some(face) = fonts.face("sans-serif", 400, false) {
                    let size = 13.5;
                    let m = fonts.metrics(face, size);
                    let (_, width) = fonts.layout(face, size, &name);
                    let (w, h) = ((width + 20.0).ceil().min(480.0), 28.0);
                    let mut scene = Scene::new(w, h);
                    scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, colour(vergence_look::MENU));
                    scene.stroke(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, vergence_look::EDGE, colour(vergence_look::SHELF_EDGE));
                    scene.push_clip(SceneRect::new(8.0, 0.0, w - 16.0, h), 0.0);
                    vergence_render::text(&mut scene, fonts, face, size, colour(vergence_look::MENU_INK), ScenePoint::new(10.0, ((h - (m.ascent + m.descent)) / 2.0 + m.ascent).round()), &name);
                    scene.pop_clip();
                    if let Ok(p) = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) {
                        let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
                        self.shelf.label = Some((key, buffer, w as i32));
                    }
                }
            }
            if let Some((_, buffer, w)) = &self.shelf.label {
                let area: Rectangle<i32, Logical> = Rectangle::new(((strip.loc.x - 8 - w).max(out.loc.x), slot.y + (slot.h - 28) / 2).into(), (*w, 28).into());
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
        elements
    }
}
