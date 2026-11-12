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
use vergence_shell::{Edge, Rect, launcher::{Grid, Step, search, step}};

use crate::{Server, decor::SceneElement, shelf::Act};

/// The popup's size in points (less on a screen too small for it), the
/// room around its contents, the search field's height, and a cell.
const WIDTH: i32 = 536;
const HEIGHT: i32 = 452;
const PAD: i32 = 12;
const FIELD_H: i32 = 34;
const CELL_W: i32 = 128;
const CELL_H: i32 = 84;
/// An application's icon in the grid, in points.
const ICON: i32 = 32;

#[derive(Default)]
pub struct Launcher {
    open: Option<Open>,
    picture: Option<(u64, MemoryRenderBuffer)>,
}

struct Open {
    at: Rectangle<i32, Logical>,
    typed: String,
    selected: usize,
    scroll: i32,
}

impl Open {
    fn field(&self) -> Rect {
        Rect::new(self.at.loc.x + PAD, self.at.loc.y + PAD, self.at.size.w - 2 * PAD, FIELD_H)
    }

    fn grid(&self) -> Grid {
        let top = PAD + FIELD_H + PAD;
        let mut g = Grid::new(Rect::new(self.at.loc.x + PAD, self.at.loc.y + top, self.at.size.w - 2 * PAD, self.at.size.h - top - PAD), CELL_W, CELL_H);
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
        if self.launcher.open.is_some() {
            return self.launcher_close();
        }
        // Beside the dock's head, on the windows' side; with no dock, in the
        // middle of the screen the pointer is on.
        let screen = self.shelf_screen().or_else(|| self.output_at_pointer());
        let Some(screen) = screen else { return };
        let area = self.work_area(screen);
        let (w, h) = (WIDTH.min(area.size.w), HEIGHT.min(area.size.h));
        let gap = vergence_look::SHELF_POPUP_GAP as i32;
        let (x, y) = match self.shelf.dock {
            Some(d) => match self.shelf_side(d) {
                Edge::Left => (d.loc.x - gap - w, d.loc.y),
                Edge::Right => (d.loc.x + d.size.w + gap, d.loc.y),
                Edge::Top => (d.loc.x, d.loc.y - gap - h),
                Edge::Bottom => (d.loc.x, d.loc.y + d.size.h + gap),
            },
            None => (area.loc.x + (area.size.w - w) / 2, area.loc.y + (area.size.h - h) / 2),
        };
        let x = x.min(area.loc.x + area.size.w - w).max(area.loc.x);
        let y = y.min(area.loc.y + area.size.h - h).max(area.loc.y);
        tracing::info!("apps: opened");
        self.close_menu();
        self.launcher.open = Some(Open { at: Rectangle::new((x, y).into(), (w, h).into()), typed: String::new(), selected: 0, scroll: 0 });
        self.redraw_all();
    }

    pub fn launcher_close(&mut self) {
        if self.launcher.open.take().is_some() {
            tracing::info!("apps: closed");
            self.redraw_all();
        }
    }

    /// The applications shown now, as places in the shelf's list.
    fn launcher_found(&self) -> Vec<usize> {
        self.launcher.open.as_ref().map(|o| search(&self.shelf.apps, &o.typed)).unwrap_or_default()
    }

    fn launcher_start(&mut self, index: usize) {
        let found = self.launcher_found();
        let Some(id) = found.get(index).map(|i| self.shelf.apps[*i].id.clone()) else { return };
        self.launcher_close();
        self.shelf_act(Act::Start(id));
    }

    /// A key pressed while the popup is open: a keysym, and the character it types if any.
    pub fn launcher_key(&mut self, sym: u32, typed: Option<char>) {
        let count = self.launcher_found().len();
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
            (key::KEY_Escape, _) => return self.launcher_close(),
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
        let count = self.launcher_found().len();
        let Some(open) = self.launcher.open.as_ref() else { return };
        if !open.at.to_f64().contains(at) {
            return self.launcher_close();
        }
        if let Some(index) = open.grid().at(count, at.x.floor() as i32, at.y.floor() as i32) {
            self.launcher_start(index);
        }
    }

    /// The pointer moved: the application under it is the selected one.
    pub fn launcher_hover(&mut self, at: Point<f64, Logical>) {
        if self.launcher.open.is_none() {
            return;
        }
        let count = self.launcher_found().len();
        let Some(open) = self.launcher.open.as_mut() else { return };
        if let Some(index) = open.grid().at(count, at.x.floor() as i32, at.y.floor() as i32)
            && index != open.selected
        {
            open.selected = index;
            self.redraw_all();
        }
    }

    /// The wheel, in points: the grid scrolls.
    pub fn launcher_scroll(&mut self, amount: f64) {
        let count = self.launcher_found().len();
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
        let names: Vec<&str> = found.iter().take(12).map(|i| self.shelf.apps[*i].name.as_str()).collect();
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
            cells.push((index, cell, name, icon));
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (&typed, selected, grid.scroll, scale.to_bits(), at.size.w, at.size.h, found.len()).hash(&mut hasher);
        for (index, _, name, icon) in &cells {
            (index, name, icon.is_some()).hash(&mut hasher);
        }
        let stamp = hasher.finish();
        if self.launcher.picture.as_ref().map(|(k, _)| *k) != Some(stamp) {
            let (w, h) = (at.size.w as f32, at.size.h as f32);
            // Everything below is in the popup's own points.
            let local = |r: Rect| SceneRect::new((r.x - at.loc.x) as f32, (r.y - at.loc.y) as f32, r.w as f32, r.h as f32);
            let mut scene = Scene::new(w, h);
            scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, color(vergence_look::MENU));
            scene.stroke(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, vergence_look::EDGE, color(vergence_look::SHELF_EDGE));
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            let face = fonts.face("sans-serif", 400, false);
            let bold = fonts.face("sans-serif", 700, false).or(face);
            // The search field: what has been typed and a caret, or what to do.
            let f = local(field);
            scene.fill_rounded(f, vergence_look::RADIUS, color(vergence_look::SHELF));
            scene.stroke(f, vergence_look::RADIUS, vergence_look::EDGE, color(vergence_look::ACCENT));
            if let Some(face) = face {
                let size = 14.0;
                let m = fonts.metrics(face, size);
                let base = (f.y + (f.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round();
                scene.push_clip(SceneRect::new(f.x + 8.0, f.y, f.w - 16.0, f.h), 0.0);
                if typed.is_empty() {
                    vergence_render::text(&mut scene, fonts, face, size, color(vergence_look::MENU_INK_OFF), ScenePoint::new(f.x + 14.0, base), "Type to search");
                    scene.fill(SceneRect::new(f.x + 10.0, f.y + 8.0, 1.0, f.h - 16.0), color(vergence_look::MENU_INK));
                } else {
                    let (_, width) = fonts.layout(face, size, &typed);
                    vergence_render::text(&mut scene, fonts, face, size, color(vergence_look::MENU_INK), ScenePoint::new(f.x + 10.0, base), &typed);
                    scene.fill(SceneRect::new(f.x + 11.0 + width, f.y + 8.0, 1.0, f.h - 16.0), color(vergence_look::MENU_INK));
                }
                scene.pop_clip();
            }
            // The grid, cut off at its area's edges.
            scene.push_clip(local(grid.area), 0.0);
            if found.is_empty()
                && let Some(face) = face
            {
                let g = local(grid.area);
                vergence_render::text(&mut scene, fonts, face, 14.0, color(vergence_look::MENU_INK_OFF), ScenePoint::new(g.x + 2.0, g.y + 24.0), "No application of that name");
            }
            for (index, cell, name, icon) in &cells {
                let c = local(*cell);
                if *index == selected {
                    scene.fill_rounded(SceneRect::new(c.x + 2.0, c.y + 2.0, c.w - 4.0, c.h - 4.0), vergence_look::RADIUS, color(vergence_look::SHELF_TILE));
                }
                let square = SceneRect::new(c.x + (c.w - ICON as f32) / 2.0, c.y + 12.0, ICON as f32, ICON as f32);
                match icon {
                    Some(icon) => {
                        let (iw, ih) = (icon.width as f32 / scale as f32, icon.height as f32 / scale as f32);
                        let picture = scene.image(Image { width: icon.width, height: icon.height, rgba: icon.rgba.clone() });
                        scene.draw_image(picture, SceneRect::new(square.x + (square.w - iw) / 2.0, square.y + (square.h - ih) / 2.0, iw, ih), 1.0);
                    }
                    None => {
                        scene.fill_rounded(square, vergence_look::RADIUS, color(if *index == selected { vergence_look::SHELF } else { vergence_look::SHELF_TILE }));
                        let letter: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
                        if let Some(face) = bold {
                            let size = 18.0;
                            let m = fonts.metrics(face, size);
                            let (_, width) = fonts.layout(face, size, &letter);
                            vergence_render::text(&mut scene, fonts, face, size, color(vergence_look::INK_FOCUSED), ScenePoint::new(square.x + (square.w - width) / 2.0, square.y + ((square.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round()), &letter);
                        }
                    }
                }
                // The name under it: in the middle, or from the left and cut off if it is too long.
                if let Some(face) = face {
                    let size = 12.5;
                    let m = fonts.metrics(face, size);
                    let (_, width) = fonts.layout(face, size, name);
                    let room = c.w - 12.0;
                    scene.push_clip(SceneRect::new(c.x + 6.0, c.y, room, c.h), 0.0);
                    let x = if width < room { c.x + 6.0 + (room - width) / 2.0 } else { c.x + 6.0 };
                    vergence_render::text(&mut scene, fonts, face, size, color(vergence_look::MENU_INK), ScenePoint::new(x.round(), (square.y + square.h + 8.0 + m.ascent).round()), name);
                    scene.pop_clip();
                }
            }
            scene.pop_clip();
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
        match MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(at.size), Kind::Unspecified) {
            Ok(el) => vec![SceneElement::Memory(el)],
            Err(_) => Vec::new(),
        }
    }
}
