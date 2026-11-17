// The applications popup (shelf milestone 1, step 5): opened by the button
// at the head of the dock or by the Super key tapped alone, it shows every
// installed application in a grid under a search field. While it is open
// the keyboard is its own: typing filters, the arrows move, Enter starts the
// selected application, Escape or a click elsewhere closes it. Which
// applications a word finds, the grid and the arrows are vergence-shell's
// (launcher.rs there); this file draws and routes input.
//
// Drawn like the dock: a scene in points, the CPU renderer, one picture
// kept until what it shows changes.

use std::hash::{Hash, Hasher};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{Kind, memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement}},
            gles::GlesRenderer,
        },
    },
    input::keyboard::keysyms as key,
    utils::{Logical, Point, Rectangle, Transform},
};
use vergence_scene::{Color, Image, Point as ScenePoint, Rect as SceneRect, Scene};
use vergence_shell::{Rect, launcher::{Grid, Step, step}};

use crate::{Server, decor::SceneElement, shelf::Act};

/// The launcher's size (shelf milestone 2): about two thirds of its
/// screen, held between these, in points; the room around its contents,
/// the search field's and the tabs' heights, a tab's widest, and a cell.
const MIN_W: i32 = 560;
const MIN_H: i32 = 420;
const MAX_W: i32 = 1040;
const MAX_H: i32 = 760;
const PAD: i32 = 16;
const FIELD_H: i32 = 40;
const TABS_H: i32 = 30;
const TAB_W: i32 = 124;
const CELL_W: i32 = 132;
const CELL_H: i32 = 120;
/// An application's icon in the grid, in points.
const ICON: i32 = 64;
/// The row of Vergence's own applications at the top (2026-11-15): its
/// height (a heading, small icons, a name), a cell's widest, an icon.
const OWN_H: i32 = 96;
const OWN_CELL: i32 = 96;
const OWN_ICON: i32 = 40;
const OWN_HEAD: i32 = 20;
/// The footer (Dave, 2026-11-16: the launcher is where the session ends,
/// as a start menu): the person's name, then Lock and the power button,
/// which opens the session dialog (Log Out, Sleep, Restart, Shut Down).
const FOOT_H: i32 = 46;
const FOOT_BUTTON: i32 = 36;

/// A button of the footer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Foot {
    Lock,
    Power,
}

#[derive(Default)]
pub struct Launcher {
    open: Option<Open>,
    /// A mouse press on an application (dock milestone 1, step 6): where,
    /// and which (its place in the shelf's list). Let go there, it starts;
    /// moved 6 points, the launcher closes and it is dragged to the dock.
    pick: Option<(Point<f64, Logical>, usize)>,
    picture: Option<(u64, MemoryRenderBuffer)>,
    /// The frost behind it (look milestone 1, step 5).
    backdrop: crate::backdrop::Slot,
}

struct Open {
    at: Rectangle<i32, Logical>,
    typed: String,
    selected: usize,
    scroll: i32,
    /// The tab chosen (an index into vergence-shell's TABS); None: All.
    tab: Option<usize>,
    /// The tabs shown, after All: those that have an application.
    tabs: Vec<usize>,
    /// Vergence's own applications, for the row at the top (places in the shelf's list).
    own: Vec<usize>,
    /// The row's application under the pointer.
    own_hover: Option<usize>,
    /// A finger down on it: where it came down, the scroll then, and
    /// whether it has moved far enough to be a drag (a tap starts or
    /// chooses when the finger lifts; a drag scrolls).
    held: Option<(Point<f64, Logical>, i32, bool)>,
    /// The footer's button under the pointer, and the person's name in it.
    foot_hover: Option<Foot>,
    person: String,
}

impl Open {
    fn field(&self) -> Rect {
        Rect::new(self.at.loc.x + PAD, self.at.loc.y + PAD, self.at.size.w - 2 * PAD, FIELD_H)
    }

    /// Whether the row of Vergence's apps shows: while nothing is typed.
    fn own_shown(&self) -> bool {
        self.typed.is_empty() && !self.own.is_empty()
    }

    /// The room the row takes, if it shows.
    fn own_room(&self) -> i32 {
        if self.own_shown() { OWN_H + 8 } else { 0 }
    }

    /// The row's cells: (place in `own`, rectangle), as many as fit.
    fn own_rects(&self) -> Vec<(usize, Rect)> {
        if !self.own_shown() {
            return Vec::new();
        }
        let width = self.at.size.w - 2 * PAD;
        let w = (width / self.own.len() as i32).min(OWN_CELL).max(60);
        let fit = (width / w) as usize;
        let y = self.at.loc.y + PAD + FIELD_H + 10 + OWN_HEAD;
        (0..self.own.len().min(fit)).map(|i| (i, Rect::new(self.at.loc.x + PAD + i as i32 * w, y, w, OWN_H - OWN_HEAD))).collect()
    }

    /// The row's application at a point.
    fn own_at(&self, x: i32, y: i32) -> Option<usize> {
        self.own_rects().into_iter().find(|(_, r)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom()).map(|(i, _)| self.own[i])
    }

    /// The tabs' row: All, then each of `tabs`, as (tab, rectangle).
    fn tab_rects(&self) -> Vec<(Option<usize>, Rect)> {
        let y = self.at.loc.y + PAD + FIELD_H + 10 + self.own_room();
        let n = (self.tabs.len() + 1) as i32;
        let w = ((self.at.size.w - 2 * PAD) / n.max(1)).min(TAB_W);
        std::iter::once(None).chain(self.tabs.iter().map(|t| Some(*t))).enumerate().map(|(i, t)| (t, Rect::new(self.at.loc.x + PAD + i as i32 * w, y, w - 4, TABS_H))).collect()
    }

    /// The footer's buttons, at its right: Lock, then the power button last.
    fn foot_rects(&self) -> [(Foot, Rect); 2] {
        let y = self.at.loc.y + self.at.size.h - PAD - FOOT_BUTTON;
        let right = self.at.loc.x + self.at.size.w - PAD;
        [(Foot::Lock, Rect::new(right - 2 * FOOT_BUTTON - 6, y, FOOT_BUTTON, FOOT_BUTTON)), (Foot::Power, Rect::new(right - FOOT_BUTTON, y, FOOT_BUTTON, FOOT_BUTTON))]
    }

    fn foot_at(&self, x: i32, y: i32) -> Option<Foot> {
        self.foot_rects().into_iter().find(|(_, r)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom()).map(|(f, _)| f)
    }

    fn grid(&self) -> Grid {
        let top = PAD + FIELD_H + 10 + self.own_room() + TABS_H + 12;
        let mut g = Grid::new(Rect::new(self.at.loc.x + PAD, self.at.loc.y + top, self.at.size.w - 2 * PAD, self.at.size.h - top - PAD - FOOT_H), CELL_W, CELL_H);
        g.scroll = self.scroll;
        g
    }
}

impl Server {
    pub fn launcher_is_open(&self) -> bool {
        self.launcher.open.is_some()
    }

    /// Is `at` on the open popup (dispatch.rs: a press elsewhere closes it)?
    pub fn launcher_contains(&self, at: Point<f64, Logical>) -> bool {
        self.launcher.open.as_ref().is_some_and(|o| o.at.to_f64().contains(at))
    }

    pub fn launcher_toggle(&mut self) {
        // No launcher at the login screen (review C3).
        if crate::greeter() {
            return;
        }
        if self.launcher.open.is_some() {
            return self.launcher_close();
        }
        // In the middle of the screen the pointer is on (shelf milestone 2,
        // Dave's answer: it opens where you are looking), about two thirds of it.
        let Some(screen) = self.output_at_pointer().or_else(|| self.shelf_screen()) else { return };
        let area = self.work_area(screen);
        let w = (area.size.w * 2 / 3).clamp(MIN_W, MAX_W).min(area.size.w);
        let h = (area.size.h * 2 / 3).clamp(MIN_H, MAX_H).min(area.size.h);
        let (x, y) = (area.loc.x + (area.size.w - w) / 2, area.loc.y + (area.size.h - h) / 2);
        let all = vergence_shell::launcher::search(&self.shelf.apps, "");
        let tabs = vergence_shell::launcher::tabs(&self.shelf.apps, &all).into_iter().map(|(t, _)| t).collect();
        let own = vergence_shell::launcher::own(&self.shelf.apps);
        tracing::info!("apps: opened");
        self.close_menu();
        self.launcher.open = Some(Open { at: Rectangle::new((x, y).into(), (w, h).into()), typed: String::new(), selected: 0, scroll: 0, tab: None, tabs, own, own_hover: None, held: None, foot_hover: None, person: person_name() });
        self.redraw_all();
    }

    /// Escape during a drag out of the launcher (shelf.rs): the press is forgotten.
    pub fn launcher_pick_clear(&mut self) {
        self.launcher.pick = None;
    }

    pub fn launcher_close(&mut self) {
        if self.launcher.open.take().is_some() {
            tracing::info!("apps: closed");
            self.redraw_all();
        }
    }

    /// The applications shown now, as places in the shelf's list.
    fn launcher_found(&self) -> Vec<usize> {
        self.launcher.open.as_ref().map(|o| vergence_shell::launcher::search_in(&self.shelf.apps, &o.typed, o.tab)).unwrap_or_default()
    }

    /// The named actions found by what was typed (dock milestone 1, step 8),
    /// shown after the applications; none while nothing is typed or a tab
    /// other than All is chosen.
    fn launcher_actions(&self) -> Vec<vergence_shell::actions::Action> {
        let Some(open) = self.launcher.open.as_ref() else { return Vec::new() };
        if open.typed.trim().is_empty() || open.tab.is_some() {
            return Vec::new();
        }
        let all = self.actions_all();
        vergence_shell::actions::search(&all, &open.typed).into_iter().map(|i| all[i].clone()).collect()
    }

    /// How many cells the grid has: the applications found, then the actions.
    fn launcher_count(&self) -> usize {
        self.launcher_found().len() + self.launcher_actions().len()
    }

    fn launcher_start(&mut self, index: usize) {
        let found = self.launcher_found();
        // Past the applications: an action, run by name.
        if index >= found.len() {
            let Some(a) = self.launcher_actions().get(index - found.len()).cloned() else { return };
            self.launcher_close();
            if let Err(e) = self.action_run(&a.id) {
                tracing::warn!("apps: {e}");
            }
            return;
        }
        let Some(id) = found.get(index).map(|i| self.shelf.apps[*i].id.clone()) else { return };
        self.launcher_close();
        self.shelf_act(Act::Start(id));
    }

    /// A key pressed while the popup is open: a keysym, and the character it types if any.
    pub fn launcher_key(&mut self, sym: u32, typed: Option<char>) {
        let count = self.launcher_count();
        let Some(open) = self.launcher.open.as_mut() else { return };
        let columns = open.grid().columns;
        let arrow = match sym {
            key::KEY_Left => Some(Step::Left),
            key::KEY_Right => Some(Step::Right),
            key::KEY_Up => Some(Step::Up),
            key::KEY_Down => Some(Step::Down),
            _ => None,
        };
        match (sym, arrow) {
            // Escape clears what was typed, then closes.
            (key::KEY_Escape, _) if !open.typed.is_empty() => {
                open.typed.clear();
                (open.selected, open.scroll) = (0, 0);
            }
            (key::KEY_Escape, _) => return self.launcher_close(),
            // Page Up and Page Down (with Ctrl or without) go through the tabs.
            (key::KEY_Page_Up | key::KEY_Page_Down, _) => {
                let order: Vec<Option<usize>> = std::iter::once(None).chain(open.tabs.iter().map(|t| Some(*t))).collect();
                let at = order.iter().position(|t| *t == open.tab).unwrap_or(0);
                let next = if sym == key::KEY_Page_Down { (at + 1).min(order.len() - 1) } else { at.saturating_sub(1) };
                open.tab = order[next];
                (open.selected, open.scroll) = (0, 0);
            }
            (key::KEY_Return | key::KEY_KP_Enter, _) => {
                let selected = open.selected;
                return self.launcher_start(selected);
            }
            (_, Some(arrow)) => {
                open.selected = step(open.selected, count, columns, arrow);
                open.scroll = open.grid().showing(open.selected, count).scroll;
            }
            (key::KEY_BackSpace, _) => {
                open.typed.pop();
                (open.selected, open.scroll) = (0, 0);
            }
            _ => match typed.filter(|c| !c.is_control()) {
                Some(c) if open.typed.chars().count() < 64 => {
                    open.typed.push(c);
                    (open.selected, open.scroll) = (0, 0);
                }
                _ => return,
            },
        }
        self.redraw_all();
    }

    /// A press while the popup is open: on an application, it is started; on
    /// the popup's own ground, nothing; anywhere else, the popup closes.
    pub fn launcher_press(&mut self, at: Point<f64, Logical>) {
        let Some(open) = self.launcher.open.as_mut() else { return };
        if !open.at.to_f64().contains(at) {
            return self.launcher_close();
        }
        // A finger: nothing yet; lifted where it came down it is a tap, moved it scrolls.
        if self.touch.fingers_last {
            open.held = Some((at, open.scroll, false));
            return;
        }
        // On an application: a click when let go, a drag to the dock when moved.
        if let Some(app) = self.launcher_app_at(at) {
            self.launcher.pick = Some((at, app));
            return;
        }
        self.launcher_activate(at);
    }

    /// The application (its place in the shelf's list) under a point of the launcher.
    fn launcher_app_at(&self, at: Point<f64, Logical>) -> Option<usize> {
        let open = self.launcher.open.as_ref()?;
        let (x, y) = (at.x.floor() as i32, at.y.floor() as i32);
        if let Some(app) = open.own_at(x, y) {
            return Some(app);
        }
        let found = self.launcher_found();
        open.grid().at(found.len(), x, y).and_then(|i| found.get(i).copied())
    }

    /// A finger lifted from the launcher: a tap if it did not move.
    pub fn launcher_release(&mut self, at: Point<f64, Logical>) {
        // An application dragged out of the launcher: pinned if let go on the dock.
        if self.shelf_lifting_new() {
            self.launcher.pick = None;
            return self.shelf_drop_lifted();
        }
        if let Some((from, _)) = self.launcher.pick.take() {
            return self.launcher_activate(from);
        }
        let Some(open) = self.launcher.open.as_mut() else { return };
        if let Some((from, _, moved)) = open.held.take()
            && !moved
        {
            self.launcher_activate(from);
        }
        let _ = at;
    }

    /// A click (or a tap) at `at` on the launcher: a tab is chosen, an
    /// application started.
    fn launcher_activate(&mut self, at: Point<f64, Logical>) {
        let count = self.launcher_count();
        let Some(open) = self.launcher.open.as_ref() else { return };
        let (x, y) = (at.x.floor() as i32, at.y.floor() as i32);
        if let Some(app) = open.own_at(x, y) {
            let id = self.shelf.apps[app].id.clone();
            self.launcher_close();
            return self.shelf_act(Act::Start(id));
        }
        // The footer: the screen locked, or the session dialog asked.
        match open.foot_at(x, y) {
            Some(Foot::Lock) => {
                self.launcher_close();
                self.lock_request("the launcher's Lock button");
                return;
            }
            Some(Foot::Power) => {
                self.launcher_close();
                return self.session_dialog_open();
            }
            None => {}
        }
        if let Some((tab, _)) = open.tab_rects().into_iter().find(|(_, r)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom()) {
            let open = self.launcher.open.as_mut().unwrap();
            if open.tab != tab {
                (open.tab, open.selected, open.scroll) = (tab, 0, 0);
                self.redraw_all();
            }
            return;
        }
        if let Some(index) = open.grid().at(count, at.x.floor() as i32, at.y.floor() as i32) {
            self.launcher_start(index);
        }
    }

    /// A right press (or a finger held) on an application: its menu, the
    /// dock's own (open it, pin it to the dock or unpin it). Elsewhere on the
    /// popup, nothing.
    pub fn launcher_menu(&mut self, at: Point<f64, Logical>) {
        let found = self.launcher_found();
        let Some(open) = self.launcher.open.as_ref() else { return };
        let grid = open.grid();
        let (x, y) = (at.x.floor() as i32, at.y.floor() as i32);
        let (entry, cell, index) = if let Some(app) = open.own_at(x, y) {
            let r = open.own_rects().into_iter().find(|(_, r)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom()).map_or(Rect::new(x, y, 1, 1), |(_, r)| r);
            (self.shelf.apps[app].clone(), r, None)
        } else {
            let Some(index) = grid.at(found.len(), x, y) else { return };
            let Some(entry) = found.get(index).map(|i| self.shelf.apps[*i].clone()) else { return };
            (entry, grid.cell(index), Some(index))
        };
        let pinned = self.shelf.config.pinned.contains(&entry.id);
        let acts = vec![
            (Act::Start(entry.id.clone()), "Open".to_string()),
            if pinned { (Act::Unpin(entry.id.clone()), "Unpin from the dock".to_string()) } else { (Act::Pin(entry.id.clone()), "Pin to the dock".to_string()) },
        ];
        if let Some(open) = self.launcher.open.as_mut()
            && let Some(index) = index
        {
            open.selected = index;
        }
        self.open_shelf_menu(Rectangle::new((cell.x, cell.y).into(), (cell.w, cell.h).into()), vergence_shell::Edge::Right, acts);
        self.redraw_all();
    }

    /// The pointer moved: the application under it is the selected one.
    pub fn launcher_hover(&mut self, at: Point<f64, Logical>) {
        // A press on an application that moves: the launcher makes way and
        // the application goes with the pointer, to be let go on the dock.
        if let Some((from, app)) = self.launcher.pick {
            if self.shelf_lifting_new() {
                return self.shelf_lift_at(at);
            }
            if (at.x - from.x).abs() > 6.0 || (at.y - from.y).abs() > 6.0 {
                let Some(e) = self.shelf.apps.get(app).cloned() else { return };
                self.launcher_close();
                return self.shelf_lift_new(e.id, e.name, e.icon, at);
            }
            return;
        }
        if self.launcher.open.is_none() {
            return;
        }
        let count = self.launcher_count();
        let Some(open) = self.launcher.open.as_mut() else { return };
        // A finger held: past a few points it drags the grid with it.
        if let Some((from, scroll, moved)) = open.held {
            let dy = from.y - at.y;
            if moved || dy.abs() > 6.0 {
                let to = (scroll + dy.round() as i32).clamp(0, open.grid().max_scroll(count));
                open.held = Some((from, scroll, true));
                if to != open.scroll {
                    open.scroll = to;
                    self.redraw_all();
                }
            }
            return;
        }
        let hover = open.own_at(at.x.floor() as i32, at.y.floor() as i32);
        let mut changed = hover != open.own_hover;
        open.own_hover = hover;
        let foot = open.foot_at(at.x.floor() as i32, at.y.floor() as i32);
        changed |= foot != open.foot_hover;
        open.foot_hover = foot;
        if let Some(index) = open.grid().at(count, at.x.floor() as i32, at.y.floor() as i32)
            && index != open.selected
        {
            open.selected = index;
            changed = true;
        }
        if changed {
            self.redraw_all();
        }
    }

    /// The wheel, in points: the grid scrolls.
    pub fn launcher_scroll(&mut self, amount: f64) {
        let count = self.launcher_count();
        let Some(open) = self.launcher.open.as_mut() else { return };
        let to = (open.scroll + (amount * 3.0).round() as i32).clamp(0, open.grid().max_scroll(count));
        if to != open.scroll {
            open.scroll = to;
            self.redraw_all();
        }
    }

    /// For `vctl apps`.
    pub fn launcher_report(&self) -> String {
        let Some(open) = self.launcher.open.as_ref() else { return "closed".into() };
        let found = self.launcher_found();
        let actions = self.launcher_actions();
        let names: Vec<&str> = found.iter().take(12).map(|i| self.shelf.apps[*i].name.as_str()).chain(actions.iter().map(|a| a.name.as_str())).collect();
        format!(
            "open at {},{} {}x{}; typed {:?}; {} found; selected {} {:?}; scroll {}\n{}",
            open.at.loc.x, open.at.loc.y, open.at.size.w, open.at.size.h, open.typed, found.len(), open.selected,
            found.get(open.selected).map(|i| self.shelf.apps[*i].name.as_str()).unwrap_or(""), open.scroll, names.join(", ")
        )
    }

    /// The popup's picture on `out`, if it is open and on that screen.
    pub fn launcher_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        let Some(open) = self.launcher.open.as_ref() else {
            self.launcher.picture = None;
            return Vec::new();
        };
        let at = open.at;
        if !at.overlaps(out) {
            return Vec::new();
        }
        let color = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
        let (typed, selected) = (open.typed.clone(), open.selected);
        let (grid, field) = (open.grid(), open.field());
        let (tab, tab_rects) = (open.tab, open.tab_rects());
        let (own_rects, own_hover, own_list) = (open.own_rects(), open.own_hover, open.own.clone());
        let (foot_rects, foot_hover, person) = (open.foot_rects(), open.foot_hover, open.person.clone());
        let own_pixels = (OWN_ICON as f64 * scale).round() as u32;
        let own_cells: Vec<(Rect, String, Option<_>, bool)> = own_rects
            .iter()
            .map(|(i, r)| {
                let app = own_list[*i];
                let (name, icon) = (self.shelf.apps[app].name.clone(), self.shelf.apps[app].icon.clone());
                (*r, name, self.shelf_icon(&icon, own_pixels), own_hover == Some(app))
            })
            .collect();
        let found = self.launcher_found();
        // The cells that show, each with its application and its icon once that has come.
        let pixels = (ICON as f64 * scale).round() as u32;
        let mut cells = Vec::new();
        for (index, app) in found.iter().enumerate() {
            let cell = grid.cell(index);
            if cell.bottom() <= grid.area.y || cell.y >= grid.area.bottom() {
                continue;
            }
            let (name, icon) = (self.shelf.apps[*app].name.clone(), self.shelf.apps[*app].icon.clone());
            let icon = self.shelf_icon(&icon, pixels);
            cells.push((index, cell, name, icon, false));
        }
        // After them, the named actions found (step 8): a mark for an icon.
        let actions = self.launcher_actions();
        for (k, a) in actions.iter().enumerate() {
            let index = found.len() + k;
            let cell = grid.cell(index);
            if cell.bottom() <= grid.area.y || cell.y >= grid.area.bottom() {
                continue;
            }
            cells.push((index, cell, a.name.clone(), None, true));
        }
        // Light or dark (look milestone 1).
        let light = self.light_style();
        let s = vergence_look::shelf_look(light);
        // With the frost on, the popup's ground is the frost, drawn under its picture.
        let frost = self.frost_program(renderer);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (&typed, selected, grid.scroll, scale.to_bits(), at.size.w, at.size.h, found.len(), light, frost.is_some(), tab, tab_rects.len()).hash(&mut hasher);
        (foot_hover, &person).hash(&mut hasher);
        let counts = vergence_shell::launcher::tabs(&self.shelf.apps, &found);
        for (index, _, name, icon, action) in &cells {
            (index, name, icon.is_some(), action).hash(&mut hasher);
        }
        for (r, name, icon, hover) in &own_cells {
            (r.x, name, icon.is_some(), hover).hash(&mut hasher);
        }
        let stamp = hasher.finish();
        if self.launcher.picture.as_ref().map(|(k, _)| *k) != Some(stamp) {
            let (w, h) = (at.size.w as f32, at.size.h as f32);
            // Everything below is in the popup's own points.
            let local = |r: Rect| SceneRect::new((r.x - at.loc.x) as f32, (r.y - at.loc.y) as f32, r.w as f32, r.h as f32);
            let mut scene = Scene::new(w, h);
            if frost.is_none() {
                scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, color(s.menu));
            }
            scene.stroke(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, vergence_look::EDGE, color(s.edge));
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            let face = fonts.face("sans-serif", 400, false);
            let bold = fonts.face("sans-serif", 700, false).or(face);
            // The search field: what has been typed and a caret, or what to do.
            let f = local(field);
            scene.fill_rounded(f, vergence_look::RADIUS, color(s.field));
            scene.stroke(f, vergence_look::RADIUS, vergence_look::EDGE, color(vergence_look::ACCENT));
            if let Some(face) = face {
                let size = 14.0;
                let m = fonts.metrics(face, size);
                let base = (f.y + (f.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round();
                scene.push_clip(SceneRect::new(f.x + 8.0, f.y, f.w - 16.0, f.h), 0.0);
                if typed.is_empty() {
                    vergence_render::text(&mut scene, fonts, face, size, color(s.menu_ink_off), ScenePoint::new(f.x + 14.0, base), "Type to search");
                    scene.fill(SceneRect::new(f.x + 10.0, f.y + 8.0, 1.0, f.h - 16.0), color(s.menu_ink));
                } else {
                    let (_, width) = fonts.layout(face, size, &typed);
                    vergence_render::text(&mut scene, fonts, face, size, color(s.menu_ink), ScenePoint::new(f.x + 10.0, base), &typed);
                    scene.fill(SceneRect::new(f.x + 11.0 + width, f.y + 8.0, 1.0, f.h - 16.0), color(s.menu_ink));
                }
                scene.pop_clip();
            }
            // Vergence's own applications, a row under the search field while
            // nothing is typed: small icons, one line of name.
            if let (Some(face), Some((first, _, _, _))) = (face, own_cells.first()) {
                let head = local(Rect::new(first.x, first.y - OWN_HEAD, 200, OWN_HEAD));
                let m = fonts.metrics(face, 11.5);
                vergence_render::text(&mut scene, fonts, face, 11.5, color(s.menu_ink_off), ScenePoint::new(head.x + 6.0, (head.y + m.ascent).round()), "Vergence");
                for (r, name, icon, hover) in &own_cells {
                    let c = local(*r);
                    if *hover {
                        scene.fill_rounded(SceneRect::new(c.x + 2.0, c.y, c.w - 4.0, c.h), vergence_look::RADIUS, color(s.hover));
                    }
                    let square = SceneRect::new(c.x + (c.w - OWN_ICON as f32) / 2.0, c.y + 6.0, OWN_ICON as f32, OWN_ICON as f32);
                    match icon {
                        Some(icon) => {
                            let (iw, ih) = (icon.width as f32 / scale as f32, icon.height as f32 / scale as f32);
                            let picture = scene.image(Image { width: icon.width, height: icon.height, rgba: icon.rgba.clone() });
                            scene.draw_image(picture, SceneRect::new(square.x + (square.w - iw) / 2.0, square.y + (square.h - ih) / 2.0, iw, ih), 1.0);
                        }
                        None => {
                            scene.fill_rounded(square, vergence_look::RADIUS, color(s.tile));
                        }
                    }
                    let size = 11.5;
                    let m = fonts.metrics(face, size);
                    let room = c.w - 8.0;
                    let mut label = name.clone();
                    if fonts.layout(face, size, &label).1 > room {
                        while !label.is_empty() && fonts.layout(face, size, &format!("{label}…")).1 > room {
                            label.pop();
                        }
                        label.push('…');
                    }
                    let (_, width) = fonts.layout(face, size, &label);
                    vergence_render::text(&mut scene, fonts, face, size, color(s.menu_ink), ScenePoint::new((c.x + (c.w - width) / 2.0).round(), (square.y + square.h + 6.0 + m.ascent).round()), &label);
                }
                // A faint line under the row.
                let y = c_bottom(&own_cells) - at.loc.y as f32 + 6.0;
                scene.fill(SceneRect::new(PAD as f32, y, w - 2.0 * PAD as f32, 1.0), color(s.edge));
            }
            // The tabs: All and those that have an application; the chosen one shaded.
            // While something is typed, each says how many it holds of the finds.
            if let Some(face) = face {
                let size = 12.5;
                let m = fonts.metrics(face, size);
                for (t, r) in &tab_rects {
                    let r = local(*r);
                    let chosen = *t == tab;
                    if chosen {
                        scene.fill_rounded(r, vergence_look::RADIUS, color(s.hover));
                    }
                    let name = t.map_or("All", |t| vergence_shell::launcher::TABS[t]);
                    let label = match (t, typed.is_empty()) {
                        (Some(t), false) => format!("{name} {}", counts.iter().find(|(c, _)| c == t).map_or(0, |(_, n)| *n)),
                        (None, false) => format!("{name} {}", found.len()),
                        _ => name.to_string(),
                    };
                    let (_, width) = fonts.layout(face, size, &label);
                    scene.push_clip(r, 0.0);
                    let ink = if chosen || typed.is_empty() { s.menu_ink } else { s.menu_ink_off };
                    vergence_render::text(&mut scene, fonts, face, size, color(ink), ScenePoint::new((r.x + (r.w - width).max(8.0) / 2.0).round(), (r.y + (r.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round()), &label);
                    scene.pop_clip();
                }
            }
            // The grid, cut off at its area's edges.
            scene.push_clip(local(grid.area), 0.0);
            if found.is_empty()
                && actions.is_empty()
                && let Some(face) = face
            {
                let g = local(grid.area);
                vergence_render::text(&mut scene, fonts, face, 14.0, color(s.menu_ink_off), ScenePoint::new(g.x + 2.0, g.y + 24.0), "No application of that name");
            }
            for (index, cell, name, icon, action) in &cells {
                let c = local(*cell);
                if *index == selected {
                    scene.fill_rounded(SceneRect::new(c.x + 2.0, c.y + 2.0, c.w - 4.0, c.h - 4.0), vergence_look::RADIUS, color(s.hover));
                }
                let square = SceneRect::new(c.x + (c.w - ICON as f32) / 2.0, c.y + 12.0, ICON as f32, ICON as f32);
                match icon {
                    Some(icon) => {
                        let (iw, ih) = (icon.width as f32 / scale as f32, icon.height as f32 / scale as f32);
                        let picture = scene.image(Image { width: icon.width, height: icon.height, rgba: icon.rgba.clone() });
                        scene.draw_image(picture, SceneRect::new(square.x + (square.w - iw) / 2.0, square.y + (square.h - ih) / 2.0, iw, ih), 1.0);
                    }
                    None => {
                        scene.fill_rounded(square, vergence_look::RADIUS, color(if *index == selected { s.field } else { s.tile }));
                        let letter: String = if *action { "\u{203a}".into() } else { name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into()) };
                        if let Some(face) = bold {
                            let size = 18.0;
                            let m = fonts.metrics(face, size);
                            let (_, width) = fonts.layout(face, size, &letter);
                            vergence_render::text(&mut scene, fonts, face, size, color(s.tile_ink), ScenePoint::new(square.x + (square.w - width) / 2.0, square.y + ((square.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round()), &letter);
                        }
                    }
                }
                // The name under it, in the middle: two lines at most, the second
                // ending in an ellipsis if even that is too little.
                if let Some(face) = face {
                    let size = 12.5;
                    let m = fonts.metrics(face, size);
                    let room = c.w - 12.0;
                    let fits = |fonts: &mut vergence_text::Fonts, t: &str| fonts.layout(face, size, t).1 <= room;
                    let mut lines: Vec<String> = Vec::new();
                    let mut line = String::new();
                    for word in name.split_whitespace() {
                        let tried = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                        if fits(fonts, &tried) || line.is_empty() {
                            line = tried;
                        } else {
                            lines.push(std::mem::take(&mut line));
                            line = word.to_string();
                        }
                    }
                    if !line.is_empty() {
                        lines.push(line);
                    }
                    if lines.len() > 2 {
                        let rest = lines[1..].join(" ");
                        lines.truncate(1);
                        lines.push(rest);
                    }
                    if let Some(last) = lines.last_mut()
                        && !fits(fonts, last)
                    {
                        while !last.is_empty() && !fits(fonts, &format!("{last}…")) {
                            last.pop();
                        }
                        last.push('…');
                    }
                    scene.push_clip(SceneRect::new(c.x + 6.0, c.y, room, c.h), 0.0);
                    let line_h = (m.ascent + m.descent + m.line_gap).ceil();
                    for (i, l) in lines.iter().enumerate() {
                        let (_, width) = fonts.layout(face, size, l);
                        let x = c.x + 6.0 + ((room - width) / 2.0).max(0.0);
                        vergence_render::text(&mut scene, fonts, face, size, color(s.menu_ink), ScenePoint::new(x.round(), (square.y + square.h + 8.0 + m.ascent + i as f32 * line_h).round()), l);
                    }
                    scene.pop_clip();
                }
            }
            scene.pop_clip();
            // The footer: a line over it, the person, Lock and the power button.
            let fy = h - PAD as f32 - FOOT_H as f32 + 2.0;
            scene.fill(SceneRect::new(PAD as f32, fy, w - 2.0 * PAD as f32, 1.0), color(s.edge));
            let mid = h - PAD as f32 - FOOT_BUTTON as f32 / 2.0;
            if let Some(face) = face {
                let initial: String = person.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                let d = 30.0;
                let disc = SceneRect::new(PAD as f32 + 4.0, mid - d / 2.0, d, d);
                scene.fill_rounded(disc, d / 2.0, color(vergence_look::ACCENT));
                let b = bold.unwrap_or(face);
                let m = fonts.metrics(b, 13.0);
                let iw = fonts.layout(b, 13.0, &initial).1;
                vergence_render::text(&mut scene, fonts, b, 13.0, Color::rgb(0xff, 0xff, 0xff), ScenePoint::new((disc.x + (d - iw) / 2.0).round(), (mid - (m.ascent + m.descent) / 2.0 + m.ascent).round()), &initial);
                let m = fonts.metrics(face, 13.5);
                vergence_render::text(&mut scene, fonts, face, 13.5, color(s.menu_ink), ScenePoint::new(disc.x + d + 10.0, (mid - (m.ascent + m.descent) / 2.0 + m.ascent).round()), &person);
            }
            for (f, r) in &foot_rects {
                let r = local(*r);
                if foot_hover == Some(*f) {
                    scene.fill_rounded(r, vergence_look::RADIUS, color(s.hover));
                }
                let ink = color(s.menu_ink);
                let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                match f {
                    Foot::Lock => {
                        scene.stroke(SceneRect::new(cx - 7.0, cy - 1.0, 14.0, 10.0), 2.0, 1.8, ink);
                        // The shackle: a half ring over the body.
                        arc(&mut scene, cx, cy - 4.0, 4.5, std::f32::consts::PI, 2.0 * std::f32::consts::PI, ink);
                        scene.line(ScenePoint::new(cx - 4.5, cy - 4.0), ScenePoint::new(cx - 4.5, cy - 1.0), 1.8, ink);
                        scene.line(ScenePoint::new(cx + 4.5, cy - 4.0), ScenePoint::new(cx + 4.5, cy - 1.0), 1.8, ink);
                    }
                    Foot::Power => {
                        // A ring open at the top, a bar through the gap.
                        let gap = 0.75;
                        arc(&mut scene, cx, cy + 1.0, 7.5, -std::f32::consts::FRAC_PI_2 + gap, 1.5 * std::f32::consts::PI - gap, ink);
                        scene.line(ScenePoint::new(cx, cy - 8.0), ScenePoint::new(cx, cy + 0.5), 1.8, ink);
                    }
                }
            }
            match vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) {
                Ok(p) => {
                    let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
                    self.launcher.picture = Some((stamp, buffer));
                }
                Err(e) => {
                    tracing::warn!("apps: the popup could not be drawn: {e}");
                    return Vec::new();
                }
            }
        }
        let Some((_, buffer)) = &self.launcher.picture else { return Vec::new() };
        let a = (at.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let b = (at.loc + at.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
        let mut elements = Vec::new();
        if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(at.size), Kind::Unspecified) {
            elements.push(SceneElement::Memory(el));
        }
        if let Some(program) = frost {
            let look = crate::backdrop::Look {
                tint: crate::backdrop::unit(s.menu),
                strength: vergence_look::frost_strength(self.look_setting("frost")),
                ink: crate::backdrop::unit(s.menu_ink),
                need: 5.0,
                radius: vergence_look::frost_radius(self.look_setting("frost")) * scale as f32,
                corner: vergence_look::RADIUS * scale as f32,
                bottom: true,
            };
            let dst = Rectangle::<i32, smithay::utils::Physical>::new(a, (b.x - a.x, b.y - a.y).into());
            let mut slot = self.launcher.backdrop.take();
            if let Some(e) = crate::backdrop::element(renderer, &mut slot, dst, program, &look) {
                elements.push(SceneElement::Backdrop(e));
            }
            self.launcher.backdrop = slot;
        }
        elements
    }
}

/// The bottom of the row's cells, in global points.
fn c_bottom<T>(cells: &[(Rect, String, Option<T>, bool)]) -> f32 {
    cells.iter().map(|(r, ..)| r.bottom()).max().unwrap_or(0) as f32
}

/// An arc of a circle at (cx, cy), from angle `a` to `b` (radians, y down),
/// as short lines.
fn arc(scene: &mut Scene, cx: f32, cy: f32, r: f32, a: f32, b: f32, ink: Color) {
    let steps = 24;
    let at = |t: f32| ScenePoint::new(cx + r * t.cos(), cy + r * t.sin());
    for i in 0..steps {
        let (t0, t1) = (a + (b - a) * i as f32 / steps as f32, a + (b - a) * (i + 1) as f32 / steps as f32);
        scene.line(at(t0), at(t1), 1.8, ink);
    }
}

/// The person's name for the footer: the first part of their full name in
/// the password file, else their login.
fn person_name() -> String {
    let login = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_default();
    let full = std::fs::read_to_string("/etc/passwd").ok().and_then(|t| {
        t.lines().map(|l| l.split(':').collect::<Vec<_>>()).find(|f| f.len() > 4 && f[0] == login).map(|f| f[4].split(',').next().unwrap_or("").trim().to_string())
    });
    let name = full.filter(|n| !n.is_empty()).unwrap_or(login);
    name.chars().filter(|c| !c.is_control()).take(40).collect()
}
