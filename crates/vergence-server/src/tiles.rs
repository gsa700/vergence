//! Live tiles (dock milestone 1, step 2): the tiles the dock shows after
//! its applications, and the card that opens beside one the pointer rests
//! on. The server draws both from values (vergence_protocol::tile); a
//! click goes to the tile's page in the settings app. The desktop's own
//! clock is the first tile; programs publish theirs over the bus from
//! step 3.

use std::hash::{Hash, Hasher};

use smithay::backend::allocator::Fourcc;
use smithay::{
    backend::renderer::{
        element::{
            Kind,
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
        },
        gles::GlesRenderer,
    },
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::{Logical, Point, Rectangle, Transform},
};
use vergence_protocol::tile::{ControlKind, Tile, TileState};
use vergence_scene::{Color, Point as ScenePoint, Rect as SceneRect, Scene};
use vergence_shell::{Edge, Rect, tiles};

use crate::{Server, decor::SceneElement};

/// desktop.toml `[clock]`: a 24-hour clock and the date under it, both on
/// unless set off (the settings app's Date & Time page).
fn clock_look() -> tiles::ClockLook {
    let text = vergence_config::effective_text_with(crate::wallpaper::desktop_path().as_deref());
    let table = text.parse::<toml::Table>().ok();
    let get = |k: &str| {
        table.as_ref().and_then(|t| t.get("clock")).and_then(|c| c.get(k)).and_then(|v| v.as_bool()).unwrap_or(true)
    };
    tiles::ClockLook { h24: get("24-hour"), date: get("date") }
}

/// Who published a tile: the desktop itself, or a client of the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Desktop,
    Client(u32),
}

#[derive(Debug, Clone)]
pub struct Placed {
    pub owner: Owner,
    pub tile: Tile,
}

impl Placed {
    /// The tile's key on this desktop: its id, after its owner's (two
    /// programs may both name a tile "status").
    pub fn key(&self) -> String {
        match self.owner {
            Owner::Desktop => self.tile.id.clone(),
            Owner::Client(c) => format!("{c}:{}", self.tile.id),
        }
    }
}

/// What stands in one of the dock's tile places: the drawer's tile (first,
/// when the drawer holds any), or a tile by its place in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Drawer,
    Tile(usize),
}

/// What the dock draws in a tile place.
#[derive(Debug, Clone, PartialEq, Hash)]
pub enum Shown {
    /// The drawer: how many it holds, and the worst state among them.
    Drawer(usize, TileState),
    Tile(Box<Tile>),
}

/// The card that is open: its tile (by key), where it was put, and its picture.
struct Open {
    id: String,
    rect: Rectangle<i32, Logical>,
    title: Rectangle<i32, Logical>,
    picture: Option<((u64, u64, bool), MemoryRenderBuffer)>,
    /// Where each of its controls is (by its place in the card's list), and
    /// the slider being dragged, if one is.
    controls: Vec<(usize, Rectangle<i32, Logical>)>,
    sliding: Option<usize>,
}

#[derive(Default)]
pub struct Tiles {
    pub list: Vec<Placed>,
    /// The tile the pointer is over (by id) and the card that is open.
    pub hover: Option<String>,
    card: Option<Open>,
    /// The wheel's travel over the Sound tile not yet a whole step.
    wheel: f64,
    /// The minute the clock last showed.
    minute: Option<i64>,
    /// Whether the network card is open, for the reading thread (applets.rs).
    pub live: crate::applets::Live,
    /// The drawer, when it is open, and a tile being dragged (drawer.rs).
    pub drawer: Option<crate::drawer::DrawerOpen>,
    pub lift: Option<crate::drawer::TileLift>,
    /// A press on a tile in the drawer that may yet become a drag.
    pub pending: Option<crate::drawer::Pending>,
    /// Every applet this desktop has seen, by tile id: its name and who
    /// supplies it (read from and written to applets.toml once known).
    known: Option<std::collections::BTreeMap<String, (String, String)>>,
    /// The frost behind the open card and behind the open drawer (look milestone 1).
    pub card_frost: crate::backdrop::Slot,
    pub drawer_frost: crate::backdrop::Slot,
}

impl Tiles {
    /// The tile whose card is open (by its key), if any.
    pub fn open_card(&self) -> Option<&str> {
        self.card.as_ref().map(|c| c.id.as_str())
    }
}

/// ~/.local/state/vergence/applets.toml: every applet seen, for the settings
/// app's Dock page (its Applets list, each with a switch).
fn applets_path() -> Option<std::path::PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))?;
    Some(state.join("vergence/applets.toml"))
}

/// `[applets.<id>] name, from`.
pub fn parse_applets(text: &str) -> std::collections::BTreeMap<String, (String, String)> {
    let Ok(t) = text.parse::<toml::Table>() else { return Default::default() };
    let Some(a) = t.get("applets").and_then(|a| a.as_table()) else { return Default::default() };
    a.iter()
        .filter_map(|(id, v)| {
            let v = v.as_table()?;
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            Some((id.clone(), (s("name"), s("from"))))
        })
        .collect()
}

/// Written beside and moved into place, readable by its owner only.
fn write_applets(known: &std::collections::BTreeMap<String, (String, String)>) -> std::io::Result<()> {
    use std::{
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    };
    let Some(path) = applets_path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let mut all = toml::Table::new();
    for (id, (name, from)) in known {
        let mut t = toml::Table::new();
        t.insert("name".into(), name.clone().into());
        t.insert("from".into(), from.clone().into());
        all.insert(id.clone(), toml::Value::Table(t));
    }
    let mut table = toml::Table::new();
    table.insert("applets".into(), toml::Value::Table(all));
    let text = toml::to_string(&table).map_err(std::io::Error::other)?;
    let beside = path.with_extension("toml.new");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&beside)?;
    file.write_all(text.as_bytes())?;
    std::fs::rename(beside, path)
}

/// How long the pointer rests on a tile before its card opens, and how long
/// it may be away from both before the card closes.
const REST: std::time::Duration = std::time::Duration::from_millis(250);
const AWAY: std::time::Duration = std::time::Duration::from_millis(200);
const CARD_W: i32 = 280;
/// The tallest a card's picture is drawn, in points.
const PICTURE_H: f32 = 160.0;
/// From the dock to a card, in points.
const OFF: i32 = 8;
/// The height of a control's row in a card, and where a slider's track starts.
const CONTROL_H: f32 = 30.0;
/// A row of icon buttons (AlbumWall's transport: 36 across, 44 for play or pause).
const ICON_ROW: f32 = 52.0;
const ICON: f32 = 36.0;
const ICON_BIG: f32 = 44.0;
const ICON_GAP: f32 = 14.0;

/// Where each control stands in a card `w` wide, from the controls' top, and
/// their height: a row each, but a run of icon buttons shares one row, its
/// round buttons centered in it.
fn control_places(controls: &[vergence_protocol::tile::Control], w: f32) -> (Vec<SceneRect>, f32) {
    use vergence_protocol::tile::Icon;
    let icon = |k: &vergence_protocol::tile::Control| matches!(k.kind, ControlKind::Icon(_));
    let mut out = vec![SceneRect::new(0.0, 0.0, 0.0, 0.0); controls.len()];
    let (mut top, mut i) = (0.0, 0);
    while i < controls.len() {
        if !icon(&controls[i]) {
            out[i] = SceneRect::new(0.0, top, w, CONTROL_H);
            top += CONTROL_H;
            i += 1;
            continue;
        }
        let run: Vec<usize> = (i..controls.len()).take_while(|j| icon(&controls[*j])).collect();
        let size = |j: usize| match controls[j].kind {
            ControlKind::Icon(Icon::Play | Icon::Pause) => ICON_BIG,
            _ => ICON,
        };
        let total: f32 = run.iter().map(|j| size(*j)).sum::<f32>() + ICON_GAP * (run.len() - 1) as f32;
        let mut x = ((w - total) / 2.0).round();
        for &j in &run {
            let s = size(j);
            out[j] = SceneRect::new(x, top + ((ICON_ROW - s) / 2.0).round(), s, s);
            x += s + ICON_GAP;
        }
        top += ICON_ROW;
        i += run.len();
    }
    (out, top)
}
const TRACK_X: f32 = 92.0;

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
        // The agent tile goes when its agent rests (agents.rs).
        self.agents_tick();
        // Vergence Remote's socket follows Privacy > Remote (remote.rs).
        crate::remote::follow_setting(self);
        // The remote tile's "since", once a minute is plenty; each second is cheap.
        if crate::drm::remote_count(self) > 0 {
            crate::drm::remote_tile(self);
        }
        self.acts_timeout();
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        if self.tiles.minute == Some(secs / 60) {
            return;
        }
        self.tiles.minute = Some(secs / 60);
        let Some((local, utc)) = tiles::now_at(secs) else { return };
        self.tile_set(Owner::Desktop, tiles::clock(local, utc, clock_look()));
    }

    /// desktop.toml changed (settings.rs): the clock is drawn again at the
    /// next tick, as its `[clock]` table now says.
    pub fn tiles_reload(&mut self) {
        self.tiles.minute = None;
    }

    /// Put up a tile, or change one its owner already has.
    pub fn tile_set(&mut self, owner: Owner, tile: Tile) {
        match self.tiles.list.iter_mut().find(|p| p.owner == owner && p.tile.id == tile.id) {
            Some(p) if p.tile == tile => return,
            Some(p) => p.tile = tile,
            None => {
                tracing::info!("tiles: {} from {owner:?}", tile.id);
                self.applet_seen(owner, &tile);
                self.tiles.list.push(Placed { owner, tile });
                // The desktop's in a fixed order, programs' after them in the
                // order they came, the clock last (the end of the dock).
                self.tiles.list.sort_by_key(|p| match (p.owner, p.tile.id.as_str()) {
                    (Owner::Desktop, "sound") => 0,
                    (Owner::Desktop, "network") => 1,
                    (Owner::Desktop, "power") => 2,
                    (Owner::Desktop, "nowplaying") => 3,
                    (Owner::Desktop, "clock") => 9,
                    (Owner::Desktop, _) => 3,
                    (Owner::Client(_), _) => 5,
                });
                // A card open on a tile that moved keeps its tile (by key).
            }
        }
        if let Some(open) = self.tiles.card.as_mut() {
            open.picture = None;
        }
        // No redraw_all: it throws away every screen's damage history, so the
        // network tile's reading every 2 s redrew whole screens (on a 4K
        // screen, a 22 ms frame every 2 s). The dock's and the
        // drawer's pictures are keyed by the tiles they show, and the card's
        // was dropped above, so the next frame draws just what changed.
    }

    /// A tile a client of the bus put up (or changed), if it may.
    pub fn tile_from_client(&mut self, client: u32, tile: Tile) -> Result<(), String> {
        tile.check()?;
        let owner = Owner::Client(client);
        let mine = self.tiles.list.iter().filter(|p| p.owner == owner).count();
        let known = self.tiles.list.iter().any(|p| p.owner == owner && p.tile.id == tile.id);
        if !known && mine >= vergence_protocol::tile::MAX_TILES {
            return Err(format!("no more than {} tiles", vergence_protocol::tile::MAX_TILES));
        }
        self.tile_set(owner, tile);
        Ok(())
    }

    /// Take down one of an owner's tiles; whether it had it.
    pub fn tile_remove(&mut self, owner: Owner, id: &str) -> bool {
        let before = self.tiles.list.len();
        self.tiles.list.retain(|p| !(p.owner == owner && p.tile.id == id));
        let gone = self.tiles.list.len() != before;
        if gone {
            tracing::info!("tiles: {id} from {owner:?} taken down");
            self.tiles_after_removal();
        }
        gone
    }

    /// A client of the bus left: its tiles go with it.
    pub fn tiles_forget(&mut self, client: u32) {
        let before = self.tiles.list.len();
        self.tiles.list.retain(|p| p.owner != Owner::Client(client));
        if self.tiles.list.len() != before {
            tracing::info!("tiles: client {client}'s tiles taken down");
            self.tiles_after_removal();
        }
    }

    /// After tiles went: a card left without its tile closes; the dock is
    /// shorter.
    fn tiles_after_removal(&mut self) {
        if let Some(c) = self.tiles.card.as_ref()
            && !self.tiles.list.iter().any(|p| p.key() == c.id)
        {
            self.tiles.card = None;
        }
        self.redraw_all();
    }

    /// Tell a tile's owner something about it, if the owner is a client.
    fn tile_tell(&mut self, key: &str, make: impl FnOnce(String) -> vergence_protocol::bus::ServerMsg) {
        if let Some((Owner::Client(client), id)) =
            self.tiles.list.iter().find(|p| p.key() == key).map(|p| (p.owner, p.tile.id.clone()))
        {
            self.bus_send(client, make(id));
        }
    }

    /// For `vctl tiles`: each tile, its owner, what it shows, and its card's state.
    pub fn tiles_report(&self) -> String {
        let mut out: Vec<String> = self
            .tiles
            .list
            .iter()
            .map(|p| {
                format!(
                    "{} {:?} {:?} {:?} {:?} group {:?} page {:?} rows {} chart {} controls {} picture {} title {:?}",
                    p.key(),
                    p.owner,
                    p.tile.value,
                    p.tile.label,
                    p.tile.state,
                    p.tile.group,
                    p.tile.page,
                    p.tile.card.rows.len(),
                    p.tile.card.chart.len(),
                    p.tile.card.controls.len(),
                    p.tile.card.picture.as_ref().map_or("none".to_string(), |x| format!("{}x{}", x.width, x.height)),
                    p.tile.card.title
                )
            })
            .collect();
        out.push(format!(
            "card: {}",
            self.tiles.card.as_ref().map_or("none".to_string(), |c| format!(
                "{} at {},{} {}x{}",
                c.id, c.rect.loc.x, c.rect.loc.y, c.rect.size.w, c.rect.size.h
            ))
        ));
        out.join("\n")
    }

    /// An applet's first appearance this session: written down for the
    /// settings app if it is new (its name: the card's title, else its label).
    fn applet_seen(&mut self, owner: Owner, tile: &Tile) {
        // The desktop's by fixed names (the clock's card title is the date).
        let name = match (owner, tile.id.as_str()) {
            (Owner::Desktop, "clock") => "Clock".to_string(),
            (Owner::Desktop, "sound") => "Sound".to_string(),
            (Owner::Desktop, "network") => "Network".to_string(),
            (Owner::Desktop, "power") => "Battery".to_string(),
            (Owner::Desktop, "nowplaying") => "Now Playing".to_string(),
            _ if !tile.card.title.is_empty() => tile.card.title.clone(),
            _ => tile.label.clone(),
        };
        let from = match owner {
            Owner::Desktop => "the desktop".to_string(),
            Owner::Client(c) => {
                Some(self.bus_app_id(c)).filter(|a| !a.is_empty()).unwrap_or_else(|| "a program".into())
            }
        };
        let known = self.tiles.known.get_or_insert_with(|| {
            applets_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse_applets(&t)).unwrap_or_default()
        });
        if known.get(&tile.id) == Some(&(name.clone(), from.clone())) {
            return;
        }
        known.insert(tile.id.clone(), (name, from));
        if let Err(e) = write_applets(known) {
            tracing::warn!("tiles: applets.toml could not be written: {e}");
        }
    }

    /// Whether an applet is switched on (desktop.toml `[shelf]` applets-off).
    pub fn tile_visible(&self, p: &Placed) -> bool {
        !self.shelf.config.applets_off.as_ref().is_some_and(|off| off.contains(&p.tile.id))
    }

    /// Whether a tile stands on the dock (else it is in the drawer):
    /// desktop.toml `[shelf]` tiles, and trouble coming out (`[shelf]`
    /// trouble-out, on unless set off).
    pub fn tile_docked(&self, p: &Placed) -> bool {
        let c = &self.shelf.config;
        tiles::on_dock(c.tiles.as_deref(), &p.tile.id, p.tile.state, c.trouble_out.unwrap_or(true))
    }

    /// The dock's tile places in order: the drawer's tile when it holds any,
    /// then the tiles that stand on the dock.
    pub fn dock_places(&self) -> Vec<Place> {
        let mut out = Vec::new();
        if self.tiles.list.iter().any(|p| self.tile_visible(p) && !self.tile_docked(p)) {
            out.push(Place::Drawer);
        }
        // In the order of [shelf] tiles (the user's, by dragging; else the
        // desktop's own order); a tile out only for its trouble comes first,
        // beside the drawer it came out of.
        let chosen: Vec<String> = match &self.shelf.config.tiles {
            Some(d) => d.clone(),
            None => tiles::DOCKED.iter().map(|s| s.to_string()).collect(),
        };
        let mut docked: Vec<usize> = (0..self.tiles.list.len())
            .filter(|i| self.tile_visible(&self.tiles.list[*i]) && self.tile_docked(&self.tiles.list[*i]))
            .collect();
        docked.sort_by_key(|i| chosen.iter().position(|c| *c == self.tiles.list[*i].tile.id).map_or(-1, |p| p as i64));
        out.extend(docked.into_iter().map(Place::Tile));
        out
    }

    /// What the dock draws in its tile places.
    pub fn tiles_on_dock(&self) -> Vec<Shown> {
        self.dock_places()
            .into_iter()
            .map(|place| match place {
                Place::Drawer => {
                    let inside: Vec<&Placed> =
                        self.tiles.list.iter().filter(|p| self.tile_visible(p) && !self.tile_docked(p)).collect();
                    let worst = inside.iter().map(|p| p.tile.state).max_by_key(|s| *s as u8).unwrap_or_default();
                    Shown::Drawer(inside.len(), worst)
                }
                Place::Tile(i) => Shown::Tile(Box::new(self.tiles.list[i].tile.clone())),
            })
            .collect()
    }

    /// Which tile place is at a point on the dock, if any (the dock as last drawn).
    pub fn tile_under(&self, at: Point<f64, Logical>) -> Option<Place> {
        let dock = self.shelf.dock?;
        let upright = self.shelf.config.placement().upright();
        let places = self.dock_places();
        tiles::tile_at(
            rect(dock),
            self.shelf.names.len(),
            places.len(),
            upright,
            at.x.floor() as i32,
            at.y.floor() as i32,
        )
        .map(|i| places[i])
    }

    /// The key of the tile at a point, on the dock or in the open drawer.
    fn key_under(&self, at: Point<f64, Logical>) -> Option<String> {
        let i = match self.tile_under(at) {
            Some(Place::Tile(i)) => Some(i),
            Some(Place::Drawer) => None,
            None => self.drawer_cell_at(at),
        }?;
        self.tiles.list.get(i).map(|p| p.key())
    }

    /// The mouse wheel over the dock or an open card (see docs/apps-roadmap.md,
    /// polish batch 2): over
    /// the desktop's Sound tile, or its open card, each notch (a touchpad's
    /// travel, 15 points a step) is a volume key's 5%; up louder.
    pub fn tile_scroll(&mut self, at: Point<f64, Logical>, dy: f64, v120y: i32) {
        let over_sound = self.key_under(at).as_deref() == Some("sound")
            || self.tiles.card.as_ref().is_some_and(|c| c.id == "sound" && c.rect.to_f64().contains(at));
        if !over_sound {
            return;
        }
        let travel = if v120y != 0 { v120y as f64 / 120.0 } else { dy / 15.0 };
        self.tiles.wheel += travel;
        while self.tiles.wheel <= -1.0 {
            self.tiles.wheel += 1.0;
            self.volume_key(5);
        }
        while self.tiles.wheel >= 1.0 {
            self.tiles.wheel -= 1.0;
            self.volume_key(-5);
        }
    }

    /// Whether the open card is at a point.
    pub fn card_at(&self, at: Point<f64, Logical>) -> bool {
        self.tiles.card.as_ref().is_some_and(|c| c.rect.to_f64().contains(at))
    }

    /// The pointer moved (shelf.rs passes it on): resting on a tile opens its
    /// card; away from the tile and its card a moment, the card closes.
    pub fn tiles_hover(&mut self, at: Point<f64, Logical>, busy: bool) {
        // A slider held: it follows the pointer, wherever the pointer goes.
        if let Some(i) = self.tiles.card.as_ref().and_then(|c| c.sliding) {
            self.card_slide(i, at);
            return;
        }
        // A tile dragged out of the drawer or off the dock (drawer.rs).
        if self.tiles.lift.is_some() || self.tiles.pending.is_some() {
            self.tile_lift_follow(at);
            return;
        }
        // A press held on the dock (a click on its way, a drag): nothing
        // changes; after a click the card does not open again under the
        // pointer that is still there.
        if busy {
            return;
        }
        self.drawer_hover(at);
        let over = self.key_under(at);
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
                    // Not if a press or a drag began meanwhile.
                    let busy = st.tiles.lift.is_some() || st.tiles.pending.is_some() || st.shelf.held.is_some();
                    if st.tiles.hover.as_ref() == Some(&id) && !busy {
                        st.card_open(&id);
                    }
                    TimeoutAction::Drop
                });
            }
            None if self.tiles.card.is_some() => {
                let _ = self.loop_handle.insert_source(Timer::from_duration(AWAY), |_, _, st| {
                    let at = st.pointer_location();
                    let on_tile = st.key_under(at) == st.tiles.card.as_ref().map(|c| c.id.clone());
                    let sliding = st.tiles.card.as_ref().is_some_and(|c| c.sliding.is_some());
                    if !st.card_at(at) && !on_tile && !sliding {
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
        self.card_close();
        tracing::info!("tiles: card {id} open");
        self.tiles.card = Some(Open {
            id: id.to_string(),
            rect: Rectangle::default(),
            title: Rectangle::default(),
            picture: None,
            controls: Vec::new(),
            sliding: None,
        });
        self.tiles.live.set(id == "network");
        self.tile_tell(id, |id| vergence_protocol::bus::ServerMsg::CardOpened { id });
        self.redraw_all();
    }

    pub fn card_close(&mut self) {
        self.tiles.live.set(false);
        if let Some(c) = self.tiles.card.take() {
            tracing::info!("tiles: card {} closed", c.id);
            self.tile_tell(&c.id, |id| vergence_protocol::bus::ServerMsg::CardClosed { id });
            self.redraw_all();
        }
    }

    /// A finger has no hover (whitepaper §6.9): its first tap on a tile opens
    /// the tile's card; a tap on a tile whose card is open, or any click,
    /// goes where the tile lives. Whether the card was opened instead.
    pub fn tile_card_first(&mut self, index: usize) -> bool {
        if !self.touch.fingers_last {
            return false;
        }
        let Some(key) = self.tiles.list.get(index).map(|p| p.key()) else { return false };
        if self.tiles.card.as_ref().is_some_and(|c| c.id == key) {
            return false;
        }
        self.card_open(&key);
        true
    }

    /// A click or tap on a tile on the dock (shelf.rs).
    pub fn tile_tap_or_go(&mut self, index: usize) {
        if !self.tile_card_first(index) {
            self.tile_go(index);
        }
    }

    /// A click on a tile, or on its card's title: to where the tile lives.
    pub fn tile_go(&mut self, index: usize) {
        let Some(p) = self.tiles.list.get(index).cloned() else { return };
        self.card_close();
        if p.tile.page.is_empty() {
            tracing::info!("tiles: {} clicked (owner {:?})", p.tile.id, p.owner);
            self.tile_tell(&p.key(), |id| vergence_protocol::bus::ServerMsg::TileClicked { id });
            return;
        }
        tracing::info!("tiles: {} opens the settings page {:?}", p.tile.id, p.tile.page);
        if let Err(e) = crate::spawn(std::process::Command::new("vergence-settings").args(["--page", &p.tile.page])) {
            tracing::warn!("tiles: the settings app could not be started: {e}");
        }
    }

    /// A press on the open card: its title goes where the tile does; a
    /// button is pressed, a switch turns over, a slider takes the value
    /// under the pointer and follows it until the press ends.
    pub fn card_press(&mut self, at: Point<f64, Logical>) {
        let Some(c) = self.tiles.card.as_ref() else { return };
        let Some(index) = self.tiles.list.iter().position(|p| p.key() == c.id) else { return };
        if c.title.to_f64().contains(at) {
            return self.tile_go(index);
        }
        let Some(&(i, _)) = c.controls.iter().find(|(_, r)| r.to_f64().contains(at)) else { return };
        let Some(control) = self.tiles.list[index].tile.card.controls.get(i).cloned() else { return };
        match control.kind {
            ControlKind::Button | ControlKind::Icon(_) => self.card_act(index, i, 1.0),
            ControlKind::Switch(on) => self.card_act(index, i, if on { 0.0 } else { 1.0 }),
            ControlKind::Slider { .. } => {
                if let Some(c) = self.tiles.card.as_mut() {
                    c.sliding = Some(i);
                }
                self.card_slide(i, at);
            }
        }
    }

    /// The press on a card ended (a slider let go).
    pub fn card_release(&mut self) {
        if let Some(c) = self.tiles.card.as_mut() {
            c.sliding = None;
        }
    }

    /// A slider held at `at`: the value there, to its step.
    fn card_slide(&mut self, i: usize, at: Point<f64, Logical>) {
        let Some(c) = self.tiles.card.as_ref() else { return };
        let Some(index) = self.tiles.list.iter().position(|p| p.key() == c.id) else { return };
        let Some(&(_, r)) = c.controls.iter().find(|(j, _)| *j == i) else { return };
        let Some(ControlKind::Slider { value, min, max, step }) =
            self.tiles.list[index].tile.card.controls.get(i).map(|k| k.kind)
        else {
            return;
        };
        // The track as drawn: from 14 + TRACK_X to 14 short of the card's edge,
        // the knob's middle 7 in from each end.
        let (x0, x1) = (r.loc.x as f64 + 14.0 + TRACK_X as f64 + 7.0, (r.loc.x + r.size.w) as f64 - 14.0 - 7.0);
        let f = ((at.x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0);
        let mut v = min + f * (max - min);
        if step > 0.0 {
            v = (min + ((v - min) / step).round() * step).clamp(min, max);
        }
        if v != value {
            self.card_act(index, i, v);
        }
    }

    /// A control was used: shown at once as the card's new state, and told
    /// to the tile's owner (the desktop carries out its own: applets.rs).
    pub fn card_act(&mut self, index: usize, i: usize, value: f64) {
        let Some(p) = self.tiles.list.get_mut(index) else { return };
        let Some(control) = p.tile.card.controls.get_mut(i) else { return };
        control.kind = match control.kind {
            ControlKind::Switch(_) => ControlKind::Switch(value >= 0.5),
            ControlKind::Slider { min, max, step, .. } => ControlKind::Slider { value, min, max, step },
            k => k,
        };
        let (owner, tile, name) = (p.owner, p.tile.id.clone(), control.id.clone());
        if let Some(c) = self.tiles.card.as_mut() {
            c.picture = None;
        }
        self.redraw_all();
        tracing::info!("tiles: {tile} {name} = {value}");
        match owner {
            Owner::Desktop => self.desktop_tile_action(&tile, &name, value),
            Owner::Client(client) => {
                self.bus_send(client, vergence_protocol::bus::ServerMsg::TileAction { id: tile, control: name, value })
            }
        }
    }

    /// The open card, beside its tile on the side the windows are, kept on
    /// the dock's screen.
    pub fn card_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        out: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Vec<SceneElement> {
        let Some(id) = self.tiles.card.as_ref().map(|c| c.id.clone()) else { return Vec::new() };
        let Some(index) = self.tiles.list.iter().position(|p| p.key() == id) else {
            self.tiles.card = None;
            return Vec::new();
        };
        let Some(dock) = self.shelf.dock else { return Vec::new() };
        let upright = self.shelf.config.placement().upright();
        // Beside its tile on the dock, or beside the drawer at its cell.
        let (slot, beside) = match self.dock_places().iter().position(|p| *p == Place::Tile(index)) {
            Some(at) => match tiles::slot(rect(dock), self.shelf.names.len(), at, upright) {
                Some(slot) => (slot, dock),
                None => return Vec::new(),
            },
            // In the drawer, which is closed: the card goes too.
            None => match self.drawer_cell_of(index) {
                Some(cell) => cell,
                None => {
                    self.card_close();
                    return Vec::new();
                }
            },
        };
        let screen = self.shelf_screen().unwrap_or(out);
        if !screen.overlaps(out) {
            return Vec::new();
        }
        let tile = self.tiles.list[index].tile.clone();
        let light = self.light_style();
        let accent = self.accent();
        // Frosted as the menus are (look milestone 1); solid without the frost.
        let frost = self.frost_program(renderer);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        tile.hash(&mut hasher);
        frost.is_some().hash(&mut hasher);
        // With no frost, the ground is the dock's own see-through (solid with Reduce transparency).
        let ground = vergence_look::shelf_look(light).menu;
        let alpha = if frost.is_some() || self.reduce_transparency() {
            255
        } else {
            (self.shelf_opacity() * 255.0).round() as u8
        };
        let glass = Color::rgba(ground.0, ground.1, ground.2, alpha);
        alpha.hash(&mut hasher);
        let key = (hasher.finish(), scale.to_bits(), light);
        let s = vergence_look::shelf_look(light);
        let ink = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
        let side = self.shelf_side(dock);
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        let (Some(face), Some(bold)) = (
            fonts.face("sans-serif", 400, false),
            fonts.face("sans-serif", 600, false).or_else(|| fonts.face("sans-serif", 700, false)),
        ) else {
            return Vec::new();
        };
        // Its height from what it holds.
        let card = &tile.card;
        let pad = 12.0;
        // A card with a picture leads with its subject (Now Playing's track):
        // its title larger, in the words' own ink.
        let (title_size, title_h) = if card.picture.is_some() { (15.0, 22.0) } else { (12.5, 18.0) };
        let mut h = pad + title_h;
        // A picture under the title: fitted into the card's width and at most
        // PICTURE_H tall, its shape kept.
        let pic = card.picture.as_ref().map(|p| {
            let k = ((CARD_W as f32 - 28.0) / p.width as f32).min(PICTURE_H / p.height as f32);
            (p, (p.width as f32 * k).round(), (p.height as f32 * k).round())
        });
        if let Some((_, _, ph)) = pic {
            h += ph + 8.0;
        }
        if !card.big.is_empty() {
            h += 40.0;
        }
        h += card.rows.len() as f32 * 20.0;
        if card.chart.len() > 1 {
            h += 44.0;
        }
        let controls_top = h;
        let (places, controls_h) = control_places(&card.controls, CARD_W as f32);
        h += controls_h;
        if !card.controls.is_empty() {
            h += 4.0;
        }
        if !card.note.is_empty() {
            h += 18.0;
        }
        h += pad - 2.0;
        let (w, h) = (CARD_W as f32, h.ceil());
        let (wi, hi) = (CARD_W, h as i32);
        let middle = (slot.y + slot.h / 2 - hi / 2, slot.x + slot.w / 2 - wi / 2);
        let (x, y) = match side {
            Edge::Left => (beside.loc.x - OFF - wi, middle.0),
            Edge::Right => (beside.loc.x + beside.size.w + OFF, middle.0),
            Edge::Top => (middle.1, beside.loc.y - OFF - hi),
            Edge::Bottom => (middle.1, beside.loc.y + beside.size.h + OFF),
        };
        let (x, y) = (
            x.min(screen.loc.x + screen.size.w - wi).max(screen.loc.x),
            y.min(screen.loc.y + screen.size.h - hi).max(screen.loc.y),
        );
        let area: Rectangle<i32, Logical> = Rectangle::new((x, y).into(), (wi, hi).into());
        let open = self.tiles.card.as_mut().expect("open");
        open.rect = area;
        open.title = Rectangle::new((x, y).into(), (wi, (pad + title_h) as i32).into());
        open.controls = places
            .iter()
            .enumerate()
            // A row across the whole card (a press just past a slider's end still
            // takes it); an icon button, its round button.
            .map(|(i, r)| {
                (
                    i,
                    Rectangle::new(
                        (x + r.x as i32, y + (controls_top + r.y) as i32).into(),
                        (r.w as i32, r.h as i32).into(),
                    ),
                )
            })
            .collect();
        if open.picture.as_ref().map(|(k, _)| *k) != Some(key) {
            let mut scene = Scene::new(w, h);
            if frost.is_none() {
                scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, glass);
            }
            scene.stroke(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, vergence_look::EDGE, ink(s.edge));
            let baseline = |fonts: &vergence_text::Fonts, top: f32, line: f32, size: f32| {
                let m = fonts.metrics(face, size);
                (top + (line - (m.ascent + m.descent)) / 2.0 + m.ascent).round()
            };
            let mut top = pad;
            // Words kept within `room` points, an ellipsis where they are cut (a
            // long video title ran past the card's edge: Dave's screenshot).
            let fit = |fonts: &mut vergence_text::Fonts, f, size: f32, text: &str, room: f32| -> String {
                if fonts.layout(f, size, text).1 <= room {
                    return text.to_string();
                }
                let mut chars: Vec<char> = text.chars().collect();
                while !chars.is_empty() {
                    chars.pop();
                    let t: String = chars.iter().collect::<String>().trim_end().to_string() + "\u{2026}";
                    if fonts.layout(f, size, &t).1 <= room {
                        return t;
                    }
                }
                String::new()
            };
            let title = if tile.page.is_empty() { card.title.clone() } else { format!("{}  \u{203a}", card.title) };
            let title = fit(fonts, bold, title_size, &title, w - 28.0);
            let title_ink = if card.picture.is_some() { s.menu_ink } else { s.menu_ink_off };
            vergence_render::text(
                &mut scene,
                fonts,
                bold,
                title_size,
                ink(title_ink),
                ScenePoint::new(14.0, baseline(fonts, top, title_h, title_size)),
                &title,
            );
            top += title_h;
            if let Some((p, pw, ph)) = pic {
                let image = scene.image(vergence_scene::Image {
                    width: p.width as u32,
                    height: p.height as u32,
                    rgba: (*p.rgba).clone(),
                });
                scene.draw_image(image, SceneRect::new(((w - pw) / 2.0).round(), top + 4.0, pw, ph), 1.0);
                top += ph + 8.0;
            }
            if !card.big.is_empty() {
                vergence_render::text(
                    &mut scene,
                    fonts,
                    face,
                    32.0,
                    state_ink(tile.state, light, s.menu_ink),
                    ScenePoint::new(13.0, baseline(fonts, top, 40.0, 32.0)),
                    &card.big,
                );
                top += 40.0;
            }
            for (name, value) in &card.rows {
                let b = baseline(fonts, top, 20.0, 13.0);
                vergence_render::text(&mut scene, fonts, face, 13.0, ink(s.menu_ink), ScenePoint::new(14.0, b), name);
                let nw = fonts.layout(face, 13.0, name).1;
                let value = &fit(fonts, face, 13.0, value, w - 28.0 - nw - 12.0);
                let vw = fonts.layout(face, 13.0, value).1;
                vergence_render::text(
                    &mut scene,
                    fonts,
                    face,
                    13.0,
                    ink(s.menu_ink),
                    ScenePoint::new(w - 14.0 - vw, b),
                    value,
                );
                top += 20.0;
            }
            if card.chart.len() > 1 {
                let (cx, cy, cw, ch) = (14.0, top + 4.0, w - 28.0, 36.0);
                // Scaled to the values' own range, with a little room above and below.
                let lo = card.chart.iter().cloned().fold(f32::MAX, f32::min);
                let hi = card.chart.iter().cloned().fold(f32::MIN, f32::max);
                let room = ((hi - lo) * 0.1).max(hi.abs().max(1.0) * 0.01);
                let (lo, span) = (lo - room, hi - lo + 2.0 * room);
                let n = card.chart.len() - 1;
                let pt = |i: usize, v: f32| {
                    ScenePoint::new(cx + cw * i as f32 / n as f32, cy + ch - ((v - lo) / span).clamp(0.0, 1.0) * ch)
                };
                for i in 0..n {
                    scene.line(pt(i, card.chart[i]), pt(i + 1, card.chart[i + 1]), 1.5, accent);
                }
                top += 44.0;
            }
            // The controls, one row each: a button across the card; a label
            // with a switch at the end; a label with a slider's track after it;
            // a run of icon buttons, round, side by side.
            let controls_y = top;
            for (k, place) in card.controls.iter().zip(&places) {
                let top = controls_y + place.y;
                let mid = top + CONTROL_H / 2.0;
                match k.kind {
                    ControlKind::Icon(icon) => {
                        use vergence_render::transport::Glyph;
                        let r = SceneRect::new(place.x, top, place.w, place.h);
                        scene.fill_rounded(r, place.w / 2.0, ink(s.hover));
                        let g = match icon {
                            vergence_protocol::tile::Icon::Previous => Glyph::Previous,
                            vergence_protocol::tile::Icon::Play => Glyph::Play,
                            vergence_protocol::tile::Icon::Pause => Glyph::Pause,
                            vergence_protocol::tile::Icon::Next => Glyph::Next,
                            vergence_protocol::tile::Icon::Stop => Glyph::Stop,
                        };
                        vergence_render::transport::draw(&mut scene, g, r, ink(s.menu_ink));
                    }
                    ControlKind::Button => {
                        let r = SceneRect::new(14.0, top + 3.0, w - 28.0, CONTROL_H - 6.0);
                        scene.fill_rounded(r, vergence_look::RADIUS, ink(s.hover));
                        let lw = fonts.layout(face, 13.0, &k.label).1;
                        vergence_render::text(
                            &mut scene,
                            fonts,
                            face,
                            13.0,
                            ink(s.menu_ink),
                            ScenePoint::new((w - lw) / 2.0, baseline(fonts, top, CONTROL_H, 13.0)),
                            &k.label,
                        );
                    }
                    ControlKind::Switch(on) => {
                        vergence_render::text(
                            &mut scene,
                            fonts,
                            face,
                            13.0,
                            ink(s.menu_ink),
                            ScenePoint::new(14.0, baseline(fonts, top, CONTROL_H, 13.0)),
                            &k.label,
                        );
                        let pill = SceneRect::new(w - 14.0 - 36.0, mid - 10.0, 36.0, 20.0);
                        scene.fill_rounded(pill, 10.0, if on { accent } else { ink(s.field) });
                        let knob = SceneRect::new(if on { pill.x + 18.0 } else { pill.x + 2.0 }, mid - 8.0, 16.0, 16.0);
                        scene.fill_rounded(knob, 8.0, Color::rgb(0xff, 0xff, 0xff));
                    }
                    ControlKind::Slider { value, min, max, .. } => {
                        vergence_render::text(
                            &mut scene,
                            fonts,
                            face,
                            13.0,
                            ink(s.menu_ink),
                            ScenePoint::new(14.0, baseline(fonts, top, CONTROL_H, 13.0)),
                            &k.label,
                        );
                        let (x0, x1) = (14.0 + TRACK_X, w - 14.0);
                        let f = ((value - min) / (max - min)).clamp(0.0, 1.0) as f32;
                        let at = x0 + 7.0 + (x1 - x0 - 14.0) * f;
                        scene.fill_rounded(SceneRect::new(x0, mid - 2.0, x1 - x0, 4.0), 2.0, ink(s.field));
                        scene.fill_rounded(SceneRect::new(x0, mid - 2.0, at - x0, 4.0), 2.0, accent);
                        scene.fill_rounded(
                            SceneRect::new(at - 7.0, mid - 7.0, 14.0, 14.0),
                            7.0,
                            Color::rgb(0xff, 0xff, 0xff),
                        );
                        scene.stroke(SceneRect::new(at - 7.0, mid - 7.0, 14.0, 14.0), 7.0, 1.0, ink(s.edge));
                    }
                }
            }
            top = controls_y + controls_h;
            if !card.controls.is_empty() {
                top += 4.0;
            }
            if !card.note.is_empty() {
                let note = fit(fonts, face, 11.0, &card.note, w - 28.0);
                vergence_render::text(
                    &mut scene,
                    fonts,
                    face,
                    11.0,
                    // The rows' own ink: the dimmer one was unreadable on the frost (Dave, 2026-11-27).
                    ink(s.menu_ink),
                    ScenePoint::new(14.0, baseline(fonts, top, 18.0, 11.0)),
                    &note,
                );
            }
            let Ok(p) = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) else {
                return Vec::new();
            };
            let buffer = MemoryRenderBuffer::from_slice(
                &p.data,
                Fourcc::Abgr8888,
                (p.width as i32, p.height as i32),
                1,
                Transform::Normal,
                None,
            );
            self.tiles.card.as_mut().expect("open").picture = Some((key, buffer));
        }
        let Some((_, buffer)) = self.tiles.card.as_ref().and_then(|c| c.picture.as_ref()) else { return Vec::new() };
        let a = (area.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let b = (area.loc + area.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
        let mut out: Vec<SceneElement> = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            a.to_f64(),
            buffer,
            None,
            Some(src),
            Some(area.size),
            Kind::Unspecified,
        )
        .ok()
        .map(SceneElement::Memory)
        .into_iter()
        .collect();
        if let Some(program) = frost {
            let dst = Rectangle::<i32, smithay::utils::Physical>::new(a, (b.x - a.x, b.y - a.y).into());
            let mut slot = self.tiles.card_frost.take();
            if let Some(e) = crate::backdrop::element(renderer, &mut slot, dst, program, &self.panel_frost(scale)) {
                out.push(SceneElement::Backdrop(e));
            }
            self.tiles.card_frost = slot;
        }
        out
    }

    /// The frost under a card or the drawer: as a menu's, with the panel's
    /// corners, as see-through as the dock itself (its opacity, Settings > Dock,
    /// scaled by the Frost step, as the dock's own frost: Dave, 2026-11-27). The
    /// words keep their contrast however clear it is (`need`).
    pub fn panel_frost(&self, scale: f64) -> crate::backdrop::Look {
        let look = vergence_look::shelf_look(self.light_style());
        let step = vergence_look::frost_strength(self.look_setting("frost")) / vergence_look::FROST_STRENGTH;
        crate::backdrop::Look {
            tint: crate::backdrop::unit(look.menu),
            strength: (self.shelf_opacity() * step).min(1.0),
            ink: crate::backdrop::unit(look.menu_ink),
            need: 5.0,
            radius: vergence_look::frost_radius(self.look_setting("frost")) * scale as f32,
            corner: vergence_look::RADIUS * scale as f32,
            bottom: false,
            cap: false,
        }
    }
}

#[cfg(test)]
mod control_place_tests {
    use super::*;
    use vergence_protocol::tile::{Control, Icon};

    fn c(kind: ControlKind) -> Control {
        Control { id: "x".into(), label: "x".into(), kind }
    }

    /// Now Playing's controls: three icon buttons share a row, centered, play
    /// the largest; the seek bar a row of its own under them.
    #[test]
    fn a_run_of_icon_buttons_shares_one_row() {
        let controls = [
            c(ControlKind::Icon(Icon::Previous)),
            c(ControlKind::Icon(Icon::Pause)),
            c(ControlKind::Icon(Icon::Next)),
            c(ControlKind::Slider { value: 0.0, min: 0.0, max: 1.0, step: 0.0 }),
        ];
        let (p, h) = control_places(&controls, 280.0);
        assert_eq!(h, ICON_ROW + CONTROL_H);
        assert_eq!((p[0].w, p[1].w, p[2].w), (ICON, ICON_BIG, ICON));
        assert!(p[0].y == p[2].y && p[1].y < p[0].y, "centered in their row");
        let (left, right) = (p[0].x, 280.0 - (p[2].x + p[2].w));
        assert!((left - right).abs() <= 1.0, "the run centered: {left} {right}");
        assert!(p[0].x + p[0].w < p[1].x && p[1].x + p[1].w < p[2].x, "side by side, apart");
        assert_eq!((p[3].x, p[3].y, p[3].w), (0.0, ICON_ROW, 280.0));
        // Ordinary controls: a row each, across the card.
        let (p, h) = control_places(&[c(ControlKind::Button), c(ControlKind::Switch(true))], 280.0);
        assert_eq!((h, p[1].y, p[0].w), (2.0 * CONTROL_H, CONTROL_H, 280.0));
    }
}
