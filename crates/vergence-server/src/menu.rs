// The desktop menu (server milestone 1, 2026-11-07): right-click on the bare
// desktop for a list of apps and "Exit Vergence". Drawn by the server like the
// title bars (solid colors, text rasterized per monitor at its own pixels), sized
// in points so it is the same physical size on every monitor.
//
// The apps: [[menu.item]] entries in ~/.config/vergence/desktop.toml
//
//     [[menu.item]]
//     name = "Terminal"
//     exec = "foot"
//
// or, without any, the usual apps that are installed (Vergence's terminal,
// settings, browser, files, AlbumWall). Re-read each time the menu opens.

use std::collections::HashMap;

use serde::Deserialize;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Id, Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::SolidColorRenderElement,
            },
            gles::GlesRenderer,
            utils::CommitCounter,
        },
    },
    utils::{Logical, Point, Rectangle, Size, Transform},
};

use crate::{Server, decor::SceneElement};

const WIDTH: i32 = 220;
const ITEM_H: i32 = 26;
const SEP_H: i32 = 9;
const PAD: i32 = 4;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Item {
    pub name: String,
    pub exec: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Entry {
    App(Item),
    /// One of an application's windows, by its title: the list a click on the
    /// application's icon opens beside the shelf (shelf.rs).
    Window(crate::win::Win, String),
    /// Something to do with an application on the dock, and what the menu calls it.
    Shelf(crate::shelf::Act, String),
    /// Something to do with a tab of a native window's bar (bar milestone 1):
    /// the window, the tab's id, what, and what the menu calls it.
    Tab(crate::win::Win, u32, TabAct, String),
    Separator,
    Exit,
}

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
    /// Damage ids: background, edge, hover, separators..., then the edge's
    /// other three sides.
    ids: Vec<Id>,
    commit: CommitCounter,
    /// Label rasters per (entry, scale bits, light).
    labels: HashMap<(usize, u64, bool), MemoryRenderBuffer>,
    /// The style it was last drawn in (its solid parts' damage follows it).
    light: bool,
    /// The frost behind it (look milestone 1, step 5).
    backdrop: crate::backdrop::Slot,
}

/// The apps for the menu: desktop.toml's, else the usual ones that are installed.
fn apps() -> Vec<Item> {
    let configured = crate::wallpaper::desktop_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| toml::from_str::<MenuFile>(&t).ok())
        .map(|f| f.menu.item)
        .unwrap_or_default();
    if !configured.is_empty() {
        return configured;
    }
    let here = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    let mut out = Vec::new();
    let mut add = |name: &str, exec: Option<String>| {
        if let Some(exec) = exec {
            out.push(Item { name: name.into(), exec });
        }
    };
    // Our own terminal and settings app, beside the server (a checkout's
    // target/release, or the installed bin directory), else on PATH; foot only if
    // the terminal isn't there at all (Dave, 2026-11-08: "no more dirty foot").
    let beside = |name: &str| here.as_ref().map(|d| d.join(name)).filter(|p| p.exists()).map(|p| p.display().to_string()).or_else(|| on_path(name));
    add("Terminal", beside("vergence-term").or_else(|| on_path("foot")));
    add("Settings", beside("vergence-settings"));
    add("Firefox", on_path("firefox"));
    add("Files", on_path("thunar"));
    let albumwall = home.join(".local/share/albumwall/AlbumWall");
    add("AlbumWall", albumwall.exists().then(|| albumwall.display().to_string()));
    out
}

/// A command's name if it is on PATH.
fn on_path(cmd: &str) -> Option<String> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .any(|d| std::path::Path::new(d).join(cmd).is_file())
        .then(|| cmd.to_string())
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
        let mut entries: Vec<Entry> = apps().into_iter().map(Entry::App).collect();
        if !entries.is_empty() {
            entries.push(Entry::Separator);
        }
        entries.push(Entry::Exit);
        let ids = (0..6 + entries.len()).map(|_| Id::new()).collect();
        let mut m = OpenMenu {
            at: pos.to_i32_round(),
            entries,
            hover: None,
            ids,
            commit: CommitCounter::default(),
            labels: HashMap::new(),
            light: false,
            backdrop: None,
        };
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
        let ids = (0..6 + entries.len()).map(|_| Id::new()).collect();
        let mut m = OpenMenu { at: slot.loc, entries, hover: None, ids, commit: CommitCounter::default(), labels: HashMap::new(), light: false, backdrop: None };
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

    /// Pointer motion while the menu is open: the entry under it lights up.
    pub fn menu_hover(&mut self, pos: Point<f64, Logical>) {
        if let Some(m) = self.menu.as_mut() {
            let h = m.entry_at(pos);
            if h != m.hover {
                m.hover = h;
                m.commit.increment();
            }
        }
    }

    /// A press while the menu is open: run the entry under it, or just close.
    pub fn menu_press(&mut self, pos: Point<f64, Logical>) {
        let Some(m) = self.menu.take() else { return };
        let Some(i) = m.entry_at(pos) else { return };
        match &m.entries[i] {
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
            Entry::Exit => {
                tracing::info!("menu: exit");
                self.loop_signal.stop();
            }
            Entry::Separator => {}
        }
    }

    /// The menu's elements for an output, front to back (empty if closed or elsewhere).
    pub fn menu_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        // Light or dark (look milestone 1): the dock's and menus' look.
        let light = self.light_style();
        let look = vergence_look::shelf_look(light);
        let frost = self.frost_program(renderer);
        let strength = vergence_look::frost_strength(self.look_setting("frost"));
        let Some(m) = self.menu.as_mut() else { return Vec::new() };
        if m.light != light {
            m.light = light;
            m.commit.increment();
        }
        if !m.rect().overlaps(out) {
            return Vec::new();
        }
        let phys = |r: Rectangle<i32, Logical>| {
            let a = (r.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            let b = (r.loc + r.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            Rectangle::new(a, (b.x - a.x, b.y - a.y).into())
        };
        let mut els = Vec::new();
        let rows = m.rows();
        // Labels (front).
        let font = self.title_font.as_ref();
        for (i, e) in m.entries.iter().enumerate() {
            let label = match e {
                Entry::App(item) => item.name.clone(),
                Entry::Window(_, title) => title.clone(),
                Entry::Shelf(_, label) => label.clone(),
                Entry::Tab(_, _, _, label) => label.clone(),
                Entry::Exit => "Exit Vergence".into(),
                Entry::Separator => continue,
            };
            let Some(font) = font else { continue };
            let r = rows[i];
            let text = Rectangle::new((r.loc.x + 8, r.loc.y).into(), (r.size.w - 16, r.size.h).into());
            let (pw, ph) = ((text.size.w as f64 * scale).round() as i32, (text.size.h as f64 * scale).round() as i32);
            let buf = m.labels.entry((i, scale.to_bits(), light)).or_insert_with(|| {
                let rgba = font.render(&label, pw as usize, ph as usize, scale, look.menu_ink.floats3());
                MemoryRenderBuffer::from_slice(&rgba, Fourcc::Abgr8888, (pw, ph), 1, Transform::Normal, None)
            });
            let at = (text.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>().to_f64();
            let src = Rectangle::<f64, Logical>::from_size((pw as f64, ph as f64).into());
            let size: Size<i32, Logical> = text.size;
            if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(renderer, at, buf, None, Some(src), Some(size), Kind::Unspecified) {
                els.push(SceneElement::Memory(el));
            }
        }
        // Hover, separators, edge and background (back).
        if let Some(h) = m.hover {
            els.push(SceneElement::Solid(SolidColorRenderElement::new(m.ids[2].clone(), phys(rows[h]), m.commit, look.hover.floats(), Kind::Unspecified)));
        }
        for (i, e) in m.entries.iter().enumerate() {
            if *e == Entry::Separator {
                let r = rows[i];
                let line = Rectangle::new((r.loc.x + 6, r.loc.y + SEP_H / 2).into(), (r.size.w - 12, 1).into());
                els.push(SceneElement::Solid(SolidColorRenderElement::new(m.ids[3 + i].clone(), phys(line), m.commit, look.separator.floats(), Kind::Unspecified)));
            }
        }
        let rect = m.rect();
        let inner = Rectangle::new((rect.loc.x + 1, rect.loc.y + 1).into(), (rect.size.w - 2, rect.size.h - 2).into());
        // Its ground: the frost of what is behind it, or solid.
        match frost {
            Some(program) => {
                let fl = crate::backdrop::Look {
                    tint: crate::backdrop::unit(look.menu),
                    strength,
                    ink: crate::backdrop::unit(look.menu_ink),
                    need: 5.0,
                    radius: vergence_look::FROST_RADIUS * scale as f32,
                    corner: 0.0,
                    bottom: false,
                };
                if let Some(e) = crate::backdrop::element(renderer, &mut m.backdrop, phys(inner), program, &fl) {
                    els.push(SceneElement::Backdrop(e));
                }
            }
            None => els.push(SceneElement::Solid(SolidColorRenderElement::new(m.ids[0].clone(), phys(inner), m.commit, look.menu.floats(), Kind::Unspecified))),
        }
        // The edge: four lines round it, not a rectangle behind the ground (the
        // frost would show that instead of what is behind the menu).
        let (x, y, w, h) = (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
        let sides = [Rectangle::new((x, y).into(), (w, 1).into()), Rectangle::new((x, y + h - 1).into(), (w, 1).into()), Rectangle::new((x, y + 1).into(), (1, h - 2).into()), Rectangle::new((x + w - 1, y + 1).into(), (1, h - 2).into())];
        let n = m.ids.len();
        for (i, side) in sides.into_iter().enumerate() {
            let id = if i == 0 { m.ids[1].clone() } else { m.ids[n - 4 + i].clone() };
            els.push(SceneElement::Solid(SolidColorRenderElement::new(id, phys(side), m.commit, look.edge.floats(), Kind::Unspecified)));
        }
        els
    }
}
