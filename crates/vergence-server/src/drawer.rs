//! The drawer (dock milestone 1, step 5): every tile that does not stand on
//! the dock, behind a tile of its own that shows how many it holds and the
//! color of the worst state among them. A click slides it out beside the
//! dock as a grid (vergence-shell `drawer_layout`): tiles needing attention
//! first, then by group. Each tile there has its card and its click, as on
//! the dock. It closes half a second after the pointer leaves it, its tile
//! and the card. Tiles move between the dock and the drawer by dragging:
//! off the dock onto the drawer's tile (or into the open drawer), or out of
//! the drawer onto the dock; desktop.toml [shelf] tiles keeps the choice.

use std::hash::{Hash, Hasher};

use smithay::{
    backend::allocator::Fourcc,
    backend::renderer::{
        element::{Kind, memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement}},
        gles::GlesRenderer,
    },
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::{Logical, Point, Rectangle, Transform},
};
use vergence_protocol::tile::TileState;
use vergence_scene::{Color, Point as ScenePoint, Rect as SceneRect, Scene};
use vergence_shell::{Edge, Rect, tiles};

use crate::{Server, decor::SceneElement, tiles::Place};

/// The open drawer: where it stands, each tile's cell (by its place in the
/// list), the headings, and its picture.
pub struct DrawerOpen {
    rect: Rectangle<i32, Logical>,
    cells: Vec<(usize, Rectangle<i32, Logical>)>,
    picture: Option<(u64, MemoryRenderBuffer)>,
}

/// A press on a tile in the drawer, not yet a drag.
#[derive(Clone, Debug)]
pub struct Pending {
    index: usize,
    start: Point<f64, Logical>,
}

/// A tile being dragged: which (by key), whether it came out of the drawer,
/// and where the pointer is.
#[derive(Clone, Debug)]
pub struct TileLift {
    key: String,
    from_drawer: bool,
    at: Point<f64, Logical>,
}

/// How long the pointer may be away from the drawer before it closes.
const AWAY: std::time::Duration = std::time::Duration::from_millis(500);
/// From the dock to the drawer, in points.
const OFF: i32 = 8;

fn rect(r: Rectangle<i32, Logical>) -> Rect {
    Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h)
}

impl Server {
    /// The tiles in the drawer, by their place in the list.
    fn drawer_tiles(&self) -> Vec<usize> {
        (0..self.tiles.list.len()).filter(|i| !self.tile_docked(&self.tiles.list[*i])).collect()
    }

    /// The drawer tile's place on the dock, if the dock shows one.
    fn drawer_slot(&self) -> Option<Rect> {
        let dock = self.shelf.dock?;
        let at = self.dock_places().iter().position(|p| *p == Place::Drawer)?;
        tiles::slot(rect(dock), self.shelf.names.len(), at, self.shelf.config.placement().upright())
    }

    /// A click on the drawer's tile: out, or back in.
    pub fn drawer_toggle(&mut self) {
        if self.tiles.drawer.is_some() {
            self.drawer_close();
        } else {
            tracing::info!("drawer: open");
            self.card_close();
            self.tiles.drawer = Some(DrawerOpen { rect: Rectangle::default(), cells: Vec::new(), picture: None });
            self.drawer_place();
            self.redraw_all();
        }
    }

    pub fn drawer_close(&mut self) {
        if self.tiles.drawer.take().is_some() {
            tracing::info!("drawer: closed");
            self.card_close();
            self.redraw_all();
        }
    }

    /// Where the drawer stands and where each of its tiles is: beside its
    /// tile on the side the windows are, kept on the dock's screen.
    fn drawer_place(&mut self) {
        let inside = self.drawer_tiles();
        if inside.is_empty() {
            self.tiles.drawer = None;
            return;
        }
        let (Some(dock), Some(slot), Some(screen)) = (self.shelf.dock, self.drawer_slot(), self.shelf_screen()) else { return };
        let what: Vec<(String, TileState)> = inside.iter().map(|i| (self.tiles.list[*i].tile.group.clone(), self.tiles.list[*i].tile.state)).collect();
        let (_, cells, h) = tiles::drawer_layout(&what);
        let w = tiles::DRAWER_W;
        let (mid_y, mid_x) = (slot.y + slot.h / 2 - h / 2, slot.x + slot.w / 2 - w / 2);
        let (x, y) = match self.shelf_side(dock) {
            Edge::Left => (dock.loc.x - OFF - w, mid_y),
            Edge::Right => (dock.loc.x + dock.size.w + OFF, mid_y),
            Edge::Top => (mid_x, dock.loc.y - OFF - h),
            Edge::Bottom => (mid_x, dock.loc.y + dock.size.h + OFF),
        };
        let (x, y) = (x.min(screen.loc.x + screen.size.w - w).max(screen.loc.x), y.min(screen.loc.y + screen.size.h - h).max(screen.loc.y));
        if let Some(d) = self.tiles.drawer.as_mut() {
            d.rect = Rectangle::new((x, y).into(), (w, h).into());
            d.cells = cells.into_iter().map(|(k, c)| (inside[k], Rectangle::new((x + c.x, y + c.y).into(), (c.w, c.h).into()))).collect();
        }
    }

    /// Whether the open drawer is at a point.
    pub fn drawer_at(&self, at: Point<f64, Logical>) -> bool {
        self.tiles.drawer.as_ref().is_some_and(|d| d.rect.to_f64().contains(at))
    }

    /// The tile (by its place in the list) whose cell in the open drawer is at a point.
    pub fn drawer_cell_at(&self, at: Point<f64, Logical>) -> Option<usize> {
        self.tiles.drawer.as_ref()?.cells.iter().find(|(_, r)| r.to_f64().contains(at)).map(|(i, _)| *i)
    }

    /// A tile's cell in the open drawer (for its card), and the drawer's rectangle.
    pub fn drawer_cell_of(&self, index: usize) -> Option<(Rect, Rectangle<i32, Logical>)> {
        let d = self.tiles.drawer.as_ref()?;
        d.cells.iter().find(|(i, _)| *i == index).map(|(_, r)| (rect(*r), d.rect))
    }

    /// The pointer moved with the drawer open: away from it, its tile and
    /// the card for half a second, it closes (not while a tile is dragged).
    pub fn drawer_hover(&mut self, at: Point<f64, Logical>) {
        if self.tiles.drawer.is_none() || self.drawer_keeps(at) {
            return;
        }
        let _ = self.loop_handle.insert_source(Timer::from_duration(AWAY), |_, _, st| {
            let at = st.pointer_location();
            if !st.drawer_keeps(at) && st.tiles.lift.is_none() {
                st.drawer_close();
            }
            TimeoutAction::Drop
        });
    }

    fn drawer_keeps(&self, at: Point<f64, Logical>) -> bool {
        self.drawer_at(at) || self.card_at(at) || self.tile_under(at) == Some(Place::Drawer)
    }

    /// A press in the drawer: on a tile, a click or the start of a drag.
    pub fn drawer_press(&mut self, at: Point<f64, Logical>) {
        if let Some(index) = self.drawer_cell_at(at) {
            self.tiles.pending = Some(Pending { index, start: at });
        }
    }

    /// The press in the drawer ended: a click goes where the tile does; a
    /// drag lets the tile go where it is.
    pub fn drawer_release(&mut self, at: Point<f64, Logical>) {
        if self.tiles.lift.is_some() {
            return self.tile_drop(at);
        }
        if let Some(p) = self.tiles.pending.take() {
            self.drawer_close();
            self.tile_go(p.index);
        }
    }

    /// A tile is pressed and the pointer moves: past 6 points it is lifted
    /// (out of the drawer, or, from shelf.rs, off the dock) and follows the pointer.
    pub fn tile_lift_follow(&mut self, at: Point<f64, Logical>) {
        if let Some(p) = self.tiles.pending.clone() {
            if (at.x - p.start.x).abs() <= 6.0 && (at.y - p.start.y).abs() <= 6.0 {
                return;
            }
            self.tiles.pending = None;
            self.tile_lift(p.index, true, at);
        }
        if let Some(l) = self.tiles.lift.as_mut() {
            l.at = at;
            self.redraw_all();
        }
    }

    /// Lift a tile (by its place in the list) to drag it.
    pub fn tile_lift(&mut self, index: usize, from_drawer: bool, at: Point<f64, Logical>) {
        let Some(p) = self.tiles.list.get(index) else { return };
        tracing::info!("drawer: {} lifted {}", p.tile.id, if from_drawer { "out of the drawer" } else { "off the dock" });
        self.tiles.lift = Some(TileLift { key: p.key(), from_drawer, at });
        self.tiles.hover = None;
        self.card_close();
        self.redraw_all();
    }

    /// A lifted tile let go: out of the drawer onto the dock, it stands on
    /// the dock from now on; off the dock onto the drawer's tile or into the
    /// open drawer, it goes into the drawer. Anywhere else, nothing changes.
    pub fn tile_drop(&mut self, at: Point<f64, Logical>) {
        let Some(lift) = self.tiles.lift.take() else { return };
        let Some(p) = self.tiles.list.iter().find(|p| p.key() == lift.key).cloned() else { return };
        let on_dock = self.shelf_at(at);
        let into_drawer = self.drawer_at(at) || self.tile_under(at) == Some(Place::Drawer);
        let mut docked: Vec<String> = match &self.shelf.config.tiles {
            Some(d) => d.clone(),
            None => tiles::DOCKED.iter().map(|s| s.to_string()).collect(),
        };
        let changed = if lift.from_drawer && on_dock && !into_drawer {
            if !docked.contains(&p.tile.id) {
                docked.push(p.tile.id.clone());
            }
            true
        } else if !lift.from_drawer && into_drawer {
            docked.retain(|d| *d != p.tile.id);
            true
        } else {
            false
        };
        if changed {
            tracing::info!("drawer: {} now {}", p.tile.id, if docked.contains(&p.tile.id) { "on the dock" } else { "in the drawer" });
            self.shelf.config.tiles = Some(docked.clone());
            if let Err(e) = crate::shelf::save_tiles(&docked) {
                tracing::warn!("drawer: the dock's tiles could not be written: {e}");
            }
            if self.tiles.drawer.is_some() {
                self.drawer_place();
                if let Some(d) = self.tiles.drawer.as_mut() {
                    d.picture = None;
                }
            }
        }
        self.redraw_all();
    }

    /// Escape during a tile's drag (shelf.rs): it goes back.
    pub fn tile_lift_cancel(&mut self) {
        self.tiles.pending = None;
        if self.tiles.lift.take().is_some() {
            self.redraw_all();
        }
    }

    /// The open drawer, and a lifted tile under the pointer in front of it.
    pub fn drawer_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        let mut elements = Vec::new();
        if let Some(e) = self.tile_ghost(renderer, out, scale) {
            elements.push(e);
        }
        if self.tiles.drawer.is_none() {
            return elements;
        }
        // Placed again each frame: tiles come, go and change their state.
        self.drawer_place();
        let Some(d) = self.tiles.drawer.as_ref() else { return elements };
        if !d.rect.overlaps(out) {
            return elements;
        }
        let area = d.rect;
        let inside = self.drawer_tiles();
        let what: Vec<(String, TileState)> = inside.iter().map(|i| (self.tiles.list[*i].tile.group.clone(), self.tiles.list[*i].tile.state)).collect();
        let (headings, cells, _) = tiles::drawer_layout(&what);
        let light = self.light_style();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for i in &inside {
            self.tiles.list[*i].tile.hash(&mut hasher);
        }
        (scale.to_bits(), light, area.size.w, area.size.h).hash(&mut hasher);
        let key = hasher.finish();
        if d.picture.as_ref().map(|(k, _)| *k) != Some(key) {
            let s = vergence_look::shelf_look(light);
            let ink = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            let (Some(face), Some(bold)) = (fonts.face("sans-serif", 400, false), fonts.face("sans-serif", 700, false).or_else(|| fonts.face("sans-serif", 600, false))) else { return elements };
            let (w, h) = (area.size.w as f32, area.size.h as f32);
            let mut scene = Scene::new(w, h);
            scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, ink(s.menu));
            scene.stroke(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, vergence_look::EDGE, ink(s.edge));
            for (name, y) in &headings {
                let m = fonts.metrics(bold, 10.5);
                vergence_render::text(&mut scene, fonts, bold, 10.5, ink(s.menu_ink_off), ScenePoint::new(12.0, *y as f32 + m.ascent), &name.to_uppercase());
            }
            for (k, c) in &cells {
                let t = &self.tiles.list[inside[*k]].tile;
                let cell = SceneRect::new(c.x as f32, c.y as f32, c.w as f32, c.h as f32);
                scene.fill_rounded(cell, vergence_look::RADIUS, ink(s.field));
                let color = crate::tiles::state_ink(t.state, light, s.menu_ink);
                let (vm, lm) = (fonts.metrics(bold, 13.0), fonts.metrics(face, 9.5));
                let (vh, lh) = (vm.ascent + vm.descent, lm.ascent + lm.descent);
                let top = cell.y + (cell.h - vh - lh) / 2.0;
                let vw = fonts.layout(bold, 13.0, &t.value).1;
                vergence_render::text(&mut scene, fonts, bold, 13.0, color, ScenePoint::new(cell.x + (cell.w - vw) / 2.0, (top + vm.ascent).round()), &t.value);
                let lw = fonts.layout(face, 9.5, &t.label).1;
                let dim = if t.state == TileState::Fine { ink(s.menu_ink_off) } else { color };
                vergence_render::text(&mut scene, fonts, face, 9.5, dim, ScenePoint::new(cell.x + (cell.w - lw) / 2.0, (top + vh + lm.ascent).round()), &t.label);
            }
            let Ok(p) = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) else { return elements };
            let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
            if let Some(d) = self.tiles.drawer.as_mut() {
                d.picture = Some((key, buffer));
            }
        }
        let Some((_, buffer)) = self.tiles.drawer.as_ref().and_then(|d| d.picture.as_ref()) else { return elements };
        if let Some(e) = place(renderer, buffer, area, out, scale) {
            elements.push(e);
        }
        elements
    }

    /// A lifted tile, drawn as on the dock, its middle under the pointer.
    fn tile_ghost(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Option<SceneElement> {
        let lift = self.tiles.lift.clone()?;
        if !out.to_f64().contains(lift.at) {
            return None;
        }
        let t = self.tiles.list.iter().find(|p| p.key() == lift.key)?.tile.clone();
        let light = self.light_style();
        let s = vergence_look::shelf_look(light);
        let ink = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        let (face, bold) = (fonts.face("sans-serif", 400, false)?, fonts.face("sans-serif", 700, false)?);
        let side = vergence_shell::running::SLOT as f32;
        let mut scene = Scene::new(side, side);
        scene.fill_rounded(SceneRect::new(0.0, 0.0, side, side), vergence_look::RADIUS, ink(s.menu));
        scene.stroke(SceneRect::new(0.0, 0.0, side, side), vergence_look::RADIUS, vergence_look::EDGE, ink(s.edge));
        let color = crate::tiles::state_ink(t.state, light, s.menu_ink);
        let (vm, lm) = (fonts.metrics(bold, 13.0), fonts.metrics(face, 9.5));
        let top = (side - (vm.ascent + vm.descent) - (lm.ascent + lm.descent)) / 2.0;
        let vw = fonts.layout(bold, 13.0, &t.value).1;
        vergence_render::text(&mut scene, fonts, bold, 13.0, color, ScenePoint::new((side - vw) / 2.0, (top + vm.ascent).round()), &t.value);
        let lw = fonts.layout(face, 9.5, &t.label).1;
        vergence_render::text(&mut scene, fonts, face, 9.5, ink(s.menu_ink_off), ScenePoint::new((side - lw) / 2.0, (top + vm.ascent + vm.descent + lm.ascent).round()), &t.label);
        let p = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)).ok()?;
        let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
        let half = vergence_shell::running::SLOT / 2;
        let area = Rectangle::new((lift.at.x.round() as i32 - half, lift.at.y.round() as i32 - half).into(), (half * 2, half * 2).into());
        place(renderer, &buffer, area, out, scale)
    }
}

/// A picture placed on whole pixels of an output.
fn place(renderer: &mut GlesRenderer, buffer: &MemoryRenderBuffer, area: Rectangle<i32, Logical>, out: Rectangle<i32, Logical>, scale: f64) -> Option<SceneElement> {
    let a = (area.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
    let b = (area.loc + area.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
    let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
    MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(area.size), Kind::Unspecified).ok().map(SceneElement::Memory)
}
