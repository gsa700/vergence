// The desktop menu (server milestone 1, 2026-11-07; reshaped 2026-11-15,
// files milestone 1): right-click on the bare desktop, or on the desktop's
// icons, for Open Terminal and the desktop's settings (wallpaper, screens;
// Log Out moved to the launcher's power button, Dave 2026-11-16); a window's right
// click it asks the server to answer (WinRequest::ContextMenu) shows the
// window's commands first (the desktop's icons: New folder, Open, Rename...). Drawn by the server like the
// title bars (solid colors, text rasterized per monitor at its own pixels), sized
// in points so it is the same physical size on every monitor.
//
// Apps of one's own on it: [[menu.item]] entries in ~/.config/vergence/desktop.toml
//
//     [[menu.item]]
//     name = "Terminal"
//     exec = "foot"
//
// (none by default: applications are the dock's and the launcher's). Re-read
// each time the menu opens.

use std::hash::{Hash, Hasher};

use serde::Deserialize;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            },
            gles::GlesRenderer,
        },
    },
    utils::{Logical, Point, Rectangle, Transform},
};

use vergence_scene::{Color, Point as ScenePoint, Rect as SceneRect, Scene};

use crate::{Server, decor::SceneElement};

const WIDTH: i32 = 220;
/// The menus' measurements (vergence-look): rows, separators, the room inside.
const ITEM_H: i32 = vergence_look::MENU_ROW as i32;
const SEP_H: i32 = vergence_look::MENU_SEP as i32;
const PAD: i32 = vergence_look::MENU_PAD as i32;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Item {
    pub name: String,
    pub exec: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Entry {
    App(Item),
    /// One of an application's windows, by its title: the list a click on the
    /// application's icon opens beside the shelf (shelf.rs).
    Window(crate::win::Win, String),
    /// Something to do with an application on the dock, and what the menu calls it.
    Shelf(crate::shelf::Act, String),
    /// Something to do with a tab of a native window's bar (bar milestone 1):
    /// the window, the tab's id, what, and what the menu calls it.
    Tab(crate::win::Win, u32, TabAct, String),
    /// Something to do with a window (windows milestone 1, step 4: the
    /// layouts on its maximize button and its title bar), and what the menu
    /// calls it. Layouts have a small picture before their name.
    Layout(crate::win::Win, LayoutAct, String),
    Separator,
    /// A command of a window's (its context menu, WinRequest::ContextMenu):
    /// the window, the command's id, what the menu calls it.
    Command(crate::win::Win, u32, String),
    /// A program to start (Settings at a page, the Terminal), and its name.
    Run(String, Vec<String>),
}

/// What a window's menu does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutAct {
    Snap(vergence_shell::snap::Zone),
    Maximize,
    /// Back to where it was before it was maximized or snapped.
    Restore,
    Minimize,
    Close,
}

/// The little picture of a layout: a screen 24 x 16 points, the part filled.
const ICON_W: i32 = 24;
const ICON_H: i32 = 16;

/// What a tab's menu does (bar milestone 1): the server's entries, then the program's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabAct {
    Close,
    CloseOthers,
    NewWindow,
    Program(u32),
}

#[derive(Debug, Default, Deserialize)]
struct MenuFile {
    #[serde(default)]
    menu: MenuSection,
}

#[derive(Debug, Default, Deserialize)]
struct MenuSection {
    #[serde(default)]
    item: Vec<Item>,
}

pub struct OpenMenu {
    at: Point<i32, Logical>,
    entries: Vec<Entry>,
    hover: Option<usize>,
    /// Its picture (one, as the app menu's: rounded, the menus' look in
    /// vergence-look), and what it was drawn from.
    picture: Option<(u64, MemoryRenderBuffer)>,
    /// Opened by the pointer resting on a maximize button: it closes when the
    /// pointer has left both (snap.rs, `layouts_tick`).
    pub by_hover: bool,
}

/// The apps for the menu: desktop.toml's, else the usual ones that are installed.
/// The desktop's own entries, after a window's or the Terminal: its settings.
fn desktop_entries() -> Vec<Entry> {
    let settings = |label: &str, page: Option<&str>| {
        let mut argv = vec!["vergence-settings".to_string()];
        if let Some(p) = page {
            argv.extend(["--page".to_string(), p.to_string()]);
        }
        Entry::Run(label.into(), argv)
    };
    vec![Entry::Separator, settings("Change wallpaper…", Some("wallpaper")), settings("Display settings…", Some("screens"))]
}

/// The desktop menu's own lines (Change wallpaper…, Display settings…,
/// the apps desktop.toml names) for the desktop's right-click menu on the
/// bare desktop (bus v20, appmenu.rs): the lines, numbered from `first`,
/// and what each does, carried out here (`Server::menu_run`), never
/// sent to the desktop's program.
pub(crate) fn desktop_own_lines(first: u32) -> (Vec<vergence_protocol::window::MenuLine>, Vec<(u32, Entry)>) {
    use vergence_protocol::window::MenuLine;
    let mut entries: Vec<Entry> = configured_apps().into_iter().map(Entry::App).collect();
    entries.extend(desktop_entries());
    let (mut lines, mut own) = (Vec::new(), Vec::new());
    for e in entries {
        let label = match &e {
            Entry::App(item) => item.name.clone(),
            Entry::Run(label, _) => label.clone(),
            Entry::Separator => {
                if lines.last().is_some_and(|l| *l != MenuLine::Separator) {
                    lines.push(MenuLine::Separator);
                }
                continue;
            }
            _ => continue,
        };
        // A name the menu would refuse (empty, too long, control characters) is left out.
        let label: String = label.chars().filter(|c| !c.is_control()).take(60).collect();
        if label.trim().is_empty() {
            continue;
        }
        let id = first + own.len() as u32;
        lines.push(MenuLine::item(id, &label, ""));
        own.push((id, e));
    }
    (lines, own)
}

/// A menu's panel (vergence-look's menus): rounded, its ground and edge.
pub(crate) fn menu_ground(scene: &mut Scene, w: f32, h: f32, light: bool) {
    let s = vergence_look::shelf_look(light);
    scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), vergence_look::MENU_RADIUS, Color::rgb(s.menu.0, s.menu.1, s.menu.2));
    scene.stroke(SceneRect::new(0.0, 0.0, w, h), vergence_look::MENU_RADIUS, vergence_look::EDGE, Color::rgb(s.edge.0, s.edge.1, s.edge.2));
}

/// A separator line's rule, in its row `r`.
pub(crate) fn menu_separator(scene: &mut Scene, r: SceneRect, light: bool) {
    let s = vergence_look::shelf_look(light);
    let inset = vergence_look::MENU_SEP_INSET;
    scene.fill(SceneRect::new(r.x + inset, r.y + (r.h / 2.0).floor(), r.w - 2.0 * inset, 1.0), Color::rgb(s.separator.0, s.separator.1, s.separator.2));
}

/// Applications desktop.toml's [[menu.item]] puts on the desktop menu (none by default).
fn configured_apps() -> Vec<Item> {
    crate::wallpaper::desktop_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| toml::from_str::<MenuFile>(&t).ok()).map(|f| f.menu.item).unwrap_or_default()
}



impl OpenMenu {
    fn height(&self) -> i32 {
        PAD * 2 + self.entries.iter().map(|e| if *e == Entry::Separator { SEP_H } else { ITEM_H }).sum::<i32>()
    }

    fn rect(&self) -> Rectangle<i32, Logical> {
        Rectangle::new(self.at, (WIDTH, self.height()).into())
    }

    /// Each entry's rectangle.
    fn rows(&self) -> Vec<Rectangle<i32, Logical>> {
        let mut y = self.at.y + PAD;
        self.entries
            .iter()
            .map(|e| {
                let h = if *e == Entry::Separator { SEP_H } else { ITEM_H };
                let r = Rectangle::new((self.at.x + PAD, y).into(), (WIDTH - 2 * PAD, h).into());
                y += h;
                r
            })
            .collect()
    }

    fn entry_at(&self, pos: Point<f64, Logical>) -> Option<usize> {
        self.rows()
            .iter()
            .position(|r| r.to_f64().contains(pos))
            .filter(|i| self.entries[*i] != Entry::Separator)
    }
}

impl Server {
    pub fn menu_is_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Is `pos` on the open menu (dispatch.rs: a press elsewhere closes it)?
    pub fn menu_contains(&self, pos: Point<f64, Logical>) -> bool {
        self.menu.as_ref().is_some_and(|m| m.rect().to_f64().contains(pos))
    }

    /// Open the desktop menu at the pointer, kept on its monitor.
    pub fn open_menu(&mut self, pos: Point<f64, Logical>) {
        // No desktop menu at the login screen (review C3).
        if crate::greeter() {
            return;
        }
        // Applications are the dock's and the launcher's now; the desktop's own
        // things here (2026-11-15). Apps a desktop.toml names still come first.
        let mut entries: Vec<Entry> = configured_apps().into_iter().map(Entry::App).collect();
        if !entries.is_empty() {
            entries.push(Entry::Separator);
        }
        entries.push(Entry::Run("Open Terminal".into(), vec!["vergence-term".into()]));
        entries.extend(desktop_entries());
        self.open_entries(pos, entries);
    }

    /// The menu for a window's right click (WinRequest::ContextMenu): its
    /// commands, grouped by their menus; for the desktop's icons, the
    /// desktop's own things after them.
    pub fn open_context_menu(&mut self, w: &crate::win::Win, pos: Point<f64, Logical>) {
        let (commands, _) = w.native().map(|n| n.commands()).unwrap_or_default();
        let mut entries = Vec::new();
        let mut group: Option<String> = None;
        for c in commands.iter().filter(|c| c.enabled) {
            if group.as_ref().is_some_and(|g| *g != c.menu) {
                entries.push(Entry::Separator);
            }
            group = Some(c.menu.clone());
            // A command that is on says so (Arrange by name).
            let label = c.label_shown("");
            entries.push(Entry::Command(w.clone(), c.id, if c.checked == Some(true) { format!("{label}  •") } else { label }));
        }
        // The desktop's own entries on the bare desktop (its commands all in
        // groups named "Desktop..."), not on its icons' menu.
        if self.is_desktop(w) && commands.iter().all(|c| c.menu.starts_with("Desktop")) {
            entries.extend(desktop_entries());
        }
        if entries.is_empty() {
            return;
        }
        self.open_entries(pos, entries);
    }

    /// A menu of these entries at `pos`, kept on its screen.
    fn open_entries(&mut self, pos: Point<f64, Logical>, entries: Vec<Entry>) {
        let mut m = OpenMenu { at: pos.to_i32_round(), entries, hover: None, picture: None, by_hover: false };
        if let Some(out) = self.output_at_pointer().map(|o| self.work_area(o)) {
            let (w, h) = (WIDTH, m.height());
            m.at.x = m.at.x.min(out.loc.x + out.size.w - w).max(out.loc.x);
            m.at.y = m.at.y.min(out.loc.y + out.size.h - h).max(out.loc.y);
        }
        self.menu = Some(m);
    }

    /// The list of an application's windows, beside its icon's place on the dock
    /// (`slot`) on the side `towards` names, kept on its screen's work area.
    pub fn open_window_list(&mut self, slot: Rectangle<i32, Logical>, towards: vergence_shell::Edge, windows: Vec<(crate::win::Win, String)>) {
        self.open_beside(slot, towards, windows.into_iter().map(|(w, title)| Entry::Window(w, title)).collect());
    }

    /// The menu of a right-click on an application's icon, placed as its list
    /// of windows is.
    /// A tab's menu (bar milestone 1), below the tab: Close, Close the others
    /// and Move to a new window (with more than one tab), then the program's own.
    pub fn open_tab_menu(&mut self, w: &crate::win::Win, tab: u32, slot: Rectangle<i32, Logical>) {
        let Some(vergence_protocol::window::Bar::Tabs { tabs, menu, .. }) = w.native().and_then(|n| n.bar()) else { return };
        let mut entries = vec![Entry::Tab(w.clone(), tab, TabAct::Close, "Close".into())];
        if tabs.len() > 1 {
            entries.push(Entry::Tab(w.clone(), tab, TabAct::CloseOthers, "Close the others".into()));
            entries.push(Entry::Tab(w.clone(), tab, TabAct::NewWindow, "Move to a new window".into()));
        }
        if !menu.is_empty() {
            entries.push(Entry::Separator);
            entries.extend(menu.iter().map(|m| Entry::Tab(w.clone(), tab, TabAct::Program(m.id), m.label.chars().take(60).collect())));
        }
        self.open_beside(slot, vergence_shell::Edge::Bottom, entries);
    }

    pub fn open_shelf_menu(&mut self, slot: Rectangle<i32, Logical>, towards: vergence_shell::Edge, acts: Vec<(crate::shelf::Act, String)>) {
        self.open_beside(slot, towards, acts.into_iter().map(|(act, label)| Entry::Shelf(act, label)).collect());
    }

    fn open_beside(&mut self, slot: Rectangle<i32, Logical>, towards: vergence_shell::Edge, entries: Vec<Entry>) {
        use vergence_shell::Edge;
        if entries.is_empty() {
            return;
        }
        let mut m = OpenMenu { at: slot.loc, entries, hover: None, picture: None, by_hover: false };
        // Beside the icon, on the side `towards` names, a little off the dock.
        let off = vergence_look::SHELF_POPUP_GAP as i32;
        m.at = match towards {
            Edge::Left => (slot.loc.x - off - WIDTH, slot.loc.y),
            Edge::Right => (slot.loc.x + slot.size.w + off, slot.loc.y),
            Edge::Top => (slot.loc.x, slot.loc.y - off - m.height()),
            Edge::Bottom => (slot.loc.x, slot.loc.y + slot.size.h + off),
        }
        .into();
        let middle: Point<i32, Logical> = (slot.loc.x + slot.size.w / 2, slot.loc.y + slot.size.h / 2).into();
        let screen = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).find(|g| g.contains(middle));
        if let Some(area) = screen.map(|g| self.work_area(g)) {
            m.at.x = m.at.x.min(area.loc.x + area.size.w - WIDTH).max(area.loc.x);
            m.at.y = m.at.y.min(area.loc.y + area.size.h - m.height()).max(area.loc.y);
        }
        self.menu = Some(m);
    }

    pub fn close_menu(&mut self) {
        self.menu = None;
    }

    /// A window's menu (windows milestone 1, step 4): the layouts, below its
    /// maximize button (`on_button`) or at the pointer on its title bar, where
    /// Minimize and Close follow them.
    pub fn open_window_menu(&mut self, w: &crate::win::Win, on_button: bool) {
        use vergence_shell::snap::Zone;
        let mut entries: Vec<Entry> = [
            (Zone::Left, "Left half"),
            (Zone::Right, "Right half"),
            (Zone::TopLeft, "Top left"),
            (Zone::TopRight, "Top right"),
            (Zone::BottomLeft, "Bottom left"),
            (Zone::BottomRight, "Bottom right"),
        ]
        .into_iter()
        .map(|(z, name)| Entry::Layout(w.clone(), LayoutAct::Snap(z), name.into()))
        .collect();
        if self.snap.config.top_half {
            entries.push(Entry::Layout(w.clone(), LayoutAct::Snap(Zone::Top), "Top half".into()));
        }
        let placed = self.restore.contains_key(w) || self.snap.snapped.contains_key(w);
        if !self.restore.contains_key(w) && self.maximizable(w) {
            entries.push(Entry::Layout(w.clone(), LayoutAct::Maximize, "Maximize".into()));
        }
        if placed {
            entries.push(Entry::Layout(w.clone(), LayoutAct::Restore, "Back to where it was".into()));
        }
        if !on_button {
            entries.push(Entry::Separator);
            entries.push(Entry::Layout(w.clone(), LayoutAct::Minimize, "Minimize".into()));
            entries.push(Entry::Layout(w.clone(), LayoutAct::Close, "Close".into()));
        }
        let at = self.pointer_location().to_i32_round();
        let slot = if on_button {
            self.bar_of(w)
                .and_then(|(bar, layout)| layout.buttons.iter().find(|(b, _)| *b == vergence_bar::WinButton::Maximize).map(|(_, r)| Rectangle::new((bar.loc.x + r.x as i32, bar.loc.y + r.y as i32).into(), (r.w as i32, r.h as i32).into())))
                .unwrap_or(Rectangle::new(at, (1, 1).into()))
        } else {
            Rectangle::new(at, (1, 1).into())
        };
        self.open_beside(slot, vergence_shell::Edge::Bottom, entries);
    }

    /// Is `pos` on the open menu or within `pad` points of it?
    pub fn menu_near(&self, pos: Point<f64, Logical>, pad: i32) -> bool {
        self.menu.as_ref().is_some_and(|m| {
            let r = m.rect();
            Rectangle::<i32, Logical>::new((r.loc.x - pad, r.loc.y - pad).into(), (r.size.w + 2 * pad, r.size.h + 2 * pad).into()).to_f64().contains(pos)
        })
    }

    /// Whether the open menu was opened by the pointer resting on a maximize button.
    pub fn menu_by_hover(&self) -> bool {
        self.menu.as_ref().is_some_and(|m| m.by_hover)
    }

    pub fn set_menu_by_hover(&mut self) {
        if let Some(m) = self.menu.as_mut() {
            m.by_hover = true;
        }
    }

    /// Pointer motion while the menu is open: the entry under it lights up.
    pub fn menu_hover(&mut self, pos: Point<f64, Logical>) {
        if let Some(m) = self.menu.as_mut() {
            let h = m.entry_at(pos);
            if h != m.hover {
                m.hover = h;
            }
        }
    }

    /// A press while the menu is open: run the entry under it, or just close.
    pub fn menu_press(&mut self, pos: Point<f64, Logical>) {
        let Some(m) = self.menu.take() else { return };
        let Some(i) = m.entry_at(pos) else { return };
        let e = m.entries[i].clone();
        self.menu_run(&e);
    }

    /// Carries out a menu entry.
    pub(crate) fn menu_run(&mut self, e: &Entry) {
        match e {
            Entry::App(item) => {
                tracing::info!("menu: {} ({})", item.name, item.exec);
                if let Err(e) = crate::spawn(std::process::Command::new("sh").args(["-c", &item.exec])) {
                    tracing::warn!("menu: {}: {e}", item.exec);
                }
            }
            Entry::Window(w, title) => {
                tracing::info!("shelf: to window {title:?}");
                if smithay::utils::IsAlive::alive(w) {
                    self.shelf_to_window(&w.clone());
                }
            }
            Entry::Shelf(act, label) => {
                tracing::info!("shelf: {label}");
                // Opening an application from the launcher's menu closes the launcher, as a click on it does.
                if matches!(act, crate::shelf::Act::Start(_)) {
                    self.launcher_close();
                }
                self.shelf_act(act.clone());
            }
            Entry::Tab(w, tab, act, label) => {
                tracing::info!("tab menu: {label}");
                let (w, tab, act) = (w.clone(), *tab, *act);
                self.tab_act(&w, tab, act);
            }
            Entry::Layout(w, act, label) => {
                tracing::info!("window menu: {label}");
                let (w, act) = (w.clone(), *act);
                if smithay::utils::IsAlive::alive(&w) {
                    self.layout_act(&w, act);
                }
            }
            Entry::Command(w, id, label) => {
                tracing::info!("context menu: {label}");
                if let crate::win::Win::Native(n) = w {
                    self.bus_send(n.client, vergence_protocol::bus::ServerMsg::Window(vergence_protocol::window::WinEvent::Command { window: n.id, id: *id }));
                }
            }
            Entry::Run(label, argv) => {
                tracing::info!("menu: {label}");
                if let Some((p, rest)) = argv.split_first() {
                    let mut c = std::process::Command::new(p);
                    c.args(rest);
                    // The Terminal opens in the Desktop folder.
                    if p == "vergence-term" && let Some(d) = crate::desktop::desktop_dir().filter(|d| d.is_dir()) {
                        c.current_dir(d);
                    }
                    if let Err(e) = crate::spawn(&mut c) {
                        tracing::warn!("menu: {p}: {e}");
                    }
                }
            }
            Entry::Separator => {}
        }
    }

    /// The menu's elements for an output (empty if closed or elsewhere):
    /// one picture, drawn again when what it shows changes.
    pub fn menu_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        // Light or dark (look milestone 1): the dock's and menus' look.
        let light = self.light_style();
        let accent = self.accent();
        let Some(m) = self.menu.as_ref() else { return Vec::new() };
        let rect = m.rect();
        if !rect.overlaps(out) {
            return Vec::new();
        }
        let label = |e: &Entry| match e {
            Entry::App(item) => item.name.clone(),
            Entry::Window(_, title) => title.clone(),
            Entry::Shelf(_, label) => label.clone(),
            Entry::Tab(_, _, _, label) => label.clone(),
            Entry::Layout(_, _, label) => label.clone(),
            Entry::Command(_, _, label) => label.clone(),
            Entry::Run(label, _) => label.clone(),
            Entry::Separator => String::new(),
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for e in &m.entries {
            label(e).hash(&mut hasher);
            matches!(e, Entry::Separator).hash(&mut hasher);
        }
        (m.hover, light, scale.to_bits(), accent.r, accent.g, accent.b, rect.size.w, rect.size.h).hash(&mut hasher);
        let stamp = hasher.finish();
        if m.picture.as_ref().map(|(k, _)| *k) != Some(stamp) {
            let look = vergence_look::shelf_look(light);
            let color = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
            let (w, h) = (rect.size.w as f32, rect.size.h as f32);
            let mut scene = Scene::new(w, h);
            menu_ground(&mut scene, w, h, light);
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            let face = fonts.face("sans-serif", 400, false);
            let local: Vec<SceneRect> = m.rows().iter().map(|r| SceneRect::new((r.loc.x - rect.loc.x) as f32, (r.loc.y - rect.loc.y) as f32, r.size.w as f32, r.size.h as f32)).collect();
            for (i, e) in m.entries.iter().enumerate() {
                let r = local[i];
                if *e == Entry::Separator {
                    menu_separator(&mut scene, r, light);
                    continue;
                }
                if m.hover == Some(i) {
                    scene.fill_rounded(r, vergence_look::MENU_ROW_RADIUS, color(look.hover));
                }
                // A layout's picture: a small screen, its part filled in the accent.
                let zone = match e {
                    Entry::Layout(_, LayoutAct::Snap(z), _) => Some(Some(*z)),
                    Entry::Layout(_, LayoutAct::Maximize, _) => Some(None),
                    _ => None,
                };
                let mut indent = 0.0;
                if let Some(zone) = zone {
                    let (x, y) = (r.x as i32 + 8, r.y as i32 + (r.h as i32 - ICON_H) / 2);
                    let screen = vergence_shell::Rect::new(x, y, ICON_W, ICON_H);
                    let part = match zone {
                        Some(z) => vergence_shell::snap::zone_rect(z, screen, true).0,
                        None => screen,
                    };
                    scene.stroke(SceneRect::new(x as f32 + 0.5, y as f32 + 0.5, ICON_W as f32 - 1.0, ICON_H as f32 - 1.0), 1.0, 1.0, color(look.menu_ink));
                    scene.fill(SceneRect::new(part.x as f32 + 2.0, part.y as f32 + 2.0, (part.w - 3).max(1) as f32, (part.h - 3).max(1) as f32), Color::rgb(accent.r, accent.g, accent.b));
                    indent = 8.0 + ICON_W as f32;
                }
                if let Some(face) = face {
                    let size = vergence_look::MENU_TEXT;
                    let mt = fonts.metrics(face, size);
                    scene.push_clip(SceneRect::new(r.x + 10.0 + indent, r.y, r.w - 20.0 - indent, r.h), 0.0);
                    vergence_render::text(&mut scene, fonts, face, size, color(look.menu_ink), ScenePoint::new(r.x + 10.0 + indent, (r.y + (r.h - mt.ascent - mt.descent) / 2.0 + mt.ascent).round()), &label(e));
                    scene.pop_clip();
                }
            }
            match vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) {
                Ok(p) => {
                    let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
                    if let Some(m) = self.menu.as_mut() {
                        m.picture = Some((stamp, buffer));
                    }
                }
                Err(e) => {
                    tracing::warn!("menu: could not be drawn: {e}");
                    return Vec::new();
                }
            }
        }
        let Some((_, buffer)) = self.menu.as_ref().and_then(|m| m.picture.as_ref()) else { return Vec::new() };
        let a = (rect.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let b = (rect.loc + rect.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
        match MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(rect.size), Kind::Unspecified) {
            Ok(el) => vec![SceneElement::Memory(el)],
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vergence_protocol::window::MenuLine;

    #[test]
    fn the_desktop_menu_keeps_its_own_lines() {
        let first = vergence_protocol::window::MENU_SERVER_IDS;
        let (lines, own) = desktop_own_lines(first);
        assert!(matches!(own.last().map(|(_, e)| e), Some(Entry::Run(l, _)) if l == "Display settings…"), "the screens' settings last");
        assert!(!lines.iter().any(|l| matches!(l, MenuLine::Item { label, .. } if label.contains("Log Out") || label == "Settings…")), "Log Out is the launcher's");
        assert!(own.iter().enumerate().all(|(i, (id, _))| *id == first + i as u32), "numbered from the server's ids");
        assert_eq!(lines.iter().filter(|l| matches!(l, MenuLine::Item { .. })).count(), own.len());
        assert_ne!(lines.first(), Some(&MenuLine::Separator), "no separator first");
    }
}
