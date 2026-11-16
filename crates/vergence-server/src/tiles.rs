//! Live tiles (dock milestone 1, step 2): the tiles the dock shows after
//! its applications, and the card that opens beside one the pointer rests
//! on. The server draws both from values (vergence_protocol::tile); a
//! click goes to the tile's page in the settings app. The desktop's own
//! clock is the first tile; programs publish theirs over the bus from
//! step 3.

use std::hash::{Hash, Hasher};

use smithay::{
    backend::renderer::{
        element::{Kind, memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement}},
        gles::GlesRenderer,
    },
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::{Logical, Point, Rectangle, Transform},
};
use smithay::backend::allocator::Fourcc;
use vergence_protocol::tile::{Tile, TileState};
use vergence_scene::{Color, Point as ScenePoint, Rect as SceneRect, Scene};
use vergence_shell::{Edge, Rect, tiles};

use crate::{Server, decor::SceneElement};

/// Who published a tile: the desktop itself, or a client of the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Desktop,
    #[allow(dead_code)] // step 3
    Client(u32),
}

#[derive(Debug, Clone)]
pub struct Placed {
    pub owner: Owner,
    pub tile: Tile,
}

/// The card that is open: its tile, where it was put, and its picture.
struct Open {
    id: String,
    rect: Rectangle<i32, Logical>,
    title: Rectangle<i32, Logical>,
    picture: Option<((u64, u64, bool), MemoryRenderBuffer)>,
}

#[derive(Default)]
pub struct Tiles {
    pub list: Vec<Placed>,
    /// The tile the pointer is over (by id) and the card that is open.
    hover: Option<String>,
    card: Option<Open>,
    /// The minute the clock last showed.
    minute: Option<i64>,
}

/// How long the pointer rests on a tile before its card opens, and how long
/// it may be away from both before the card closes.
const REST: std::time::Duration = std::time::Duration::from_millis(250);
const AWAY: std::time::Duration = std::time::Duration::from_millis(200);
const CARD_W: i32 = 280;
/// From the dock to a card, in points.
const OFF: i32 = 8;

fn rect(r: Rectangle<i32, Logical>) -> Rect {
    Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h)
}

/// A tile's colors by its state: its usual ink, or the warning's or the problem's.
pub fn state_ink(state: TileState, light: bool, usual: vergence_look::Rgb) -> Color {
    let (r, g, b) = match (state, light) {
        (TileState::Fine, _) => (usual.0, usual.1, usual.2),
        (TileState::Warning, false) => (0xf0, 0xb4, 0x4c),
        (TileState::Warning, true) => (0xb2, 0x6a, 0x00),
        (TileState::Problem, false) => (0xff, 0x8a, 0x80),
        (TileState::Problem, true) => (0xc6, 0x28, 0x28),
    };
    Color::rgb(r, g, b)
}

impl Server {
    /// Once a second (with the dock's tick): the clock, when its minute changes.
    pub fn tiles_tick(&mut self) {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        if self.tiles.minute == Some(secs / 60) {
            return;
        }
        self.tiles.minute = Some(secs / 60);
        let Some((local, utc)) = tiles::now_at(secs) else { return };
        self.tile_set(Owner::Desktop, tiles::clock(local, utc));
    }

    /// Put up a tile, or change one its owner already has.
    pub fn tile_set(&mut self, owner: Owner, tile: Tile) {
        match self.tiles.list.iter_mut().find(|p| p.owner == owner && p.tile.id == tile.id) {
            Some(p) if p.tile == tile => return,
            Some(p) => p.tile = tile,
            None => {
                tracing::info!("tiles: {} from {owner:?}", tile.id);
                self.tiles.list.push(Placed { owner, tile });
            }
        }
        if let Some(open) = self.tiles.card.as_mut() {
            open.picture = None;
        }
        self.redraw_all();
    }

    /// The tiles the dock shows, in order.
    pub fn tiles_on_dock(&self) -> Vec<Tile> {
        self.tiles.list.iter().map(|p| p.tile.clone()).collect()
    }

    /// Which tile is at a point on the dock, if any (the dock as last drawn).
    pub fn tile_under(&self, at: Point<f64, Logical>) -> Option<usize> {
        let dock = self.shelf.dock?;
        let upright = self.shelf.config.placement().upright();
        tiles::tile_at(rect(dock), self.shelf.names.len(), self.tiles.list.len(), upright, at.x.floor() as i32, at.y.floor() as i32)
    }

    /// Whether the open card is at a point.
    pub fn card_at(&self, at: Point<f64, Logical>) -> bool {
        self.tiles.card.as_ref().is_some_and(|c| c.rect.to_f64().contains(at))
    }

    /// The pointer moved (shelf.rs passes it on): resting on a tile opens its
    /// card; away from the tile and its card a moment, the card closes.
    pub fn tiles_hover(&mut self, at: Point<f64, Logical>, busy: bool) {
        let over = if busy { None } else { self.tile_under(at).and_then(|i| self.tiles.list.get(i)).map(|p| p.tile.id.clone()) };
        if self.card_at(at) || (over.is_some() && over == self.tiles.card.as_ref().map(|c| c.id.clone())) {
            self.tiles.hover = over;
            return;
        }
        if over == self.tiles.hover {
            return;
        }
        self.tiles.hover = over.clone();
        match over {
            Some(id) => {
                let _ = self.loop_handle.insert_source(Timer::from_duration(REST), move |_, _, st| {
                    if st.tiles.hover.as_ref() == Some(&id) {
                        st.card_open(&id);
                    }
                    TimeoutAction::Drop
                });
            }
            None if self.tiles.card.is_some() => {
                let _ = self.loop_handle.insert_source(Timer::from_duration(AWAY), |_, _, st| {
                    let at = st.pointer_location();
                    let on_tile = st.tile_under(at).and_then(|i| st.tiles.list.get(i)).map(|p| &p.tile.id) == st.tiles.card.as_ref().map(|c| &c.id);
                    if !st.card_at(at) && !on_tile {
                        st.card_close();
                    }
                    TimeoutAction::Drop
                });
            }
            None => {}
        }
    }

    /// Open a tile's card (closing any other).
    pub fn card_open(&mut self, id: &str) {
        if self.tiles.card.as_ref().is_some_and(|c| c.id == id) {
            return;
        }
        tracing::info!("tiles: card {id} open");
        self.tiles.card = Some(Open { id: id.to_string(), rect: Rectangle::default(), title: Rectangle::default(), picture: None });
        self.redraw_all();
    }

    pub fn card_close(&mut self) {
        if let Some(c) = self.tiles.card.take() {
            tracing::info!("tiles: card {} closed", c.id);
            self.redraw_all();
        }
    }

    /// A click on a tile, or on its card's title: to where the tile lives.
    pub fn tile_go(&mut self, index: usize) {
        let Some(p) = self.tiles.list.get(index).cloned() else { return };
        self.card_close();
        if p.tile.page.is_empty() {
            // Step 3: the owner hears of the click.
            tracing::info!("tiles: {} clicked (owner {:?})", p.tile.id, p.owner);
            return;
        }
        tracing::info!("tiles: {} opens the settings page {:?}", p.tile.id, p.tile.page);
        if let Err(e) = crate::spawn(std::process::Command::new("vergence-settings").args(["--page", &p.tile.page])) {
            tracing::warn!("tiles: the settings app could not be started: {e}");
        }
    }

    /// A press on the open card: its title goes where the tile does.
    pub fn card_press(&mut self, at: Point<f64, Logical>) {
        let Some(c) = self.tiles.card.as_ref() else { return };
        if c.title.to_f64().contains(at)
            && let Some(i) = self.tiles.list.iter().position(|p| p.tile.id == c.id)
        {
            self.tile_go(i);
        }
    }

    /// The open card, beside its tile on the side the windows are, kept on
    /// the dock's screen.
    pub fn card_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        let Some(id) = self.tiles.card.as_ref().map(|c| c.id.clone()) else { return Vec::new() };
        let Some(index) = self.tiles.list.iter().position(|p| p.tile.id == id) else {
            self.tiles.card = None;
            return Vec::new();
        };
        let Some(dock) = self.shelf.dock else { return Vec::new() };
        let upright = self.shelf.config.placement().upright();
        let Some(slot) = tiles::slot(rect(dock), self.shelf.names.len(), index, upright) else { return Vec::new() };
        let screen = self.shelf_screen().unwrap_or(out);
        if !screen.overlaps(out) {
            return Vec::new();
        }
        let tile = self.tiles.list[index].tile.clone();
        let light = self.light_style();
        let accent = self.accent();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        tile.hash(&mut hasher);
        let key = (hasher.finish(), scale.to_bits(), light);
        let s = vergence_look::shelf_look(light);
        let ink = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
        let side = self.shelf_side(dock);
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        let (Some(face), Some(bold)) = (fonts.face("sans-serif", 400, false), fonts.face("sans-serif", 600, false).or_else(|| fonts.face("sans-serif", 700, false))) else { return Vec::new() };
        // Its height from what it holds.
        let card = &tile.card;
        let pad = 12.0;
        let mut h = pad + 18.0;
        if !card.big.is_empty() {
            h += 40.0;
        }
        h += card.rows.len() as f32 * 20.0;
        if card.chart.len() > 1 {
            h += 44.0;
        }
        if !card.note.is_empty() {
            h += 18.0;
        }
        h += pad - 2.0;
        let (w, h) = (CARD_W as f32, h.ceil());
        let (wi, hi) = (CARD_W, h as i32);
        let middle = (slot.y + slot.h / 2 - hi / 2, slot.x + slot.w / 2 - wi / 2);
        let (x, y) = match side {
            Edge::Left => (dock.loc.x - OFF - wi, middle.0),
            Edge::Right => (dock.loc.x + dock.size.w + OFF, middle.0),
            Edge::Top => (middle.1, dock.loc.y - OFF - hi),
            Edge::Bottom => (middle.1, dock.loc.y + dock.size.h + OFF),
        };
        let (x, y) = (x.min(screen.loc.x + screen.size.w - wi).max(screen.loc.x), y.min(screen.loc.y + screen.size.h - hi).max(screen.loc.y));
        let area: Rectangle<i32, Logical> = Rectangle::new((x, y).into(), (wi, hi).into());
        let open = self.tiles.card.as_mut().expect("open");
        open.rect = area;
        open.title = Rectangle::new((x, y).into(), (wi, (pad + 18.0) as i32).into());
        if open.picture.as_ref().map(|(k, _)| *k) != Some(key) {
            let mut scene = Scene::new(w, h);
            scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, ink(s.menu));
            scene.stroke(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, vergence_look::EDGE, ink(s.edge));
            let baseline = |fonts: &vergence_text::Fonts, top: f32, line: f32, size: f32| {
                let m = fonts.metrics(face, size);
                (top + (line - (m.ascent + m.descent)) / 2.0 + m.ascent).round()
            };
            let mut top = pad;
            let title = if tile.page.is_empty() { card.title.clone() } else { format!("{}  \u{203a}", card.title) };
            vergence_render::text(&mut scene, fonts, bold, 12.5, ink(s.menu_ink_off), ScenePoint::new(14.0, baseline(fonts, top, 18.0, 12.5)), &title);
            top += 18.0;
            if !card.big.is_empty() {
                vergence_render::text(&mut scene, fonts, face, 32.0, state_ink(tile.state, light, s.menu_ink), ScenePoint::new(13.0, baseline(fonts, top, 40.0, 32.0)), &card.big);
                top += 40.0;
            }
            for (name, value) in &card.rows {
                let b = baseline(fonts, top, 20.0, 13.0);
                vergence_render::text(&mut scene, fonts, face, 13.0, ink(s.menu_ink), ScenePoint::new(14.0, b), name);
                let vw = fonts.layout(face, 13.0, value).1;
                vergence_render::text(&mut scene, fonts, face, 13.0, ink(s.menu_ink), ScenePoint::new(w - 14.0 - vw, b), value);
                top += 20.0;
            }
            if card.chart.len() > 1 {
                let (cx, cy, cw, ch) = (14.0, top + 4.0, w - 28.0, 36.0);
                let max = card.chart.iter().cloned().fold(f32::MIN, f32::max).max(1e-6) * 1.1;
                let n = card.chart.len() - 1;
                let pt = |i: usize, v: f32| ScenePoint::new(cx + cw * i as f32 / n as f32, cy + ch - (v / max).clamp(0.0, 1.0) * ch);
                for i in 0..n {
                    scene.line(pt(i, card.chart[i]), pt(i + 1, card.chart[i + 1]), 1.5, accent);
                }
                top += 44.0;
            }
            if !card.note.is_empty() {
                vergence_render::text(&mut scene, fonts, face, 11.0, ink(s.menu_ink_off), ScenePoint::new(14.0, baseline(fonts, top, 18.0, 11.0)), &card.note);
            }
            let Ok(p) = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) else { return Vec::new() };
            let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
            self.tiles.card.as_mut().expect("open").picture = Some((key, buffer));
        }
        let Some((_, buffer)) = self.tiles.card.as_ref().and_then(|c| c.picture.as_ref()) else { return Vec::new() };
        let a = (area.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let b = (area.loc + area.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(area.size), Kind::Unspecified)
            .ok()
            .map(SceneElement::Memory)
            .into_iter()
            .collect()
    }
}
