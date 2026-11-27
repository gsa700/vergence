// Now Playing for the whole desktop (AlbumWall milestone 2, step 6;
// docs/albumwall-milestone-2.md). The server is the session bus (dbus.rs), so it
// sees every MPRIS player come and go (org.mpris.MediaPlayer2.*: AlbumWall,
// Firefox playing a video, mpv) and hears what each one announces. It shows the
// one most recently playing as a Dockworker tile and card (the cover, the track,
// previous / play or pause / next, a seek bar), and the media keys and the
// media.* actions go to it, whichever window has the keyboard.
//
// The edge asks each player as the pseudo-connection ":1.0" (dbus.rs
// dbus_edge_call; the replies come back by serial to nowplaying_answer) and
// hears PropertiesChanged and Seeked as they are broadcast (no match rule of
// its own). Its picture: the player's mpris:artUrl when it is a local file,
// decoded in vergence-decode's confined helper on a thread, never here; an
// address on the network is not fetched.
//
// Costs nothing while nobody looks (Dockworkers principle 7): the position is
// asked for once a second only while the card is open and something plays.
//
// When DBUS_SESSION_BUS_ADDRESS named another bus at start (dbus.rs header),
// players register there and the edge sees none: Now Playing stays away.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use smithay::reexports::calloop::{
    channel,
    timer::{TimeoutAction, Timer},
};
use vergence_dbus::client::lookup;
use vergence_dbus::{Message, Value, message};
use vergence_protocol::tile::{Card, Control, ControlKind, Icon, Picture, Tile, TileState};
use vergence_seat::keyboard::Media;

use crate::Server;
use crate::tiles::Owner;

pub const PREFIX: &str = "org.mpris.MediaPlayer2.";
pub const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
pub const TILE: &str = "nowplaying";
/// The picture's largest side, in pixels (a card's picture may be up to 256).
const ART: u32 = 256;

/// One player on the bus, as it last said.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Player {
    /// Its well-known name (org.mpris.MediaPlayer2.albumwall) and the connection owning it.
    pub name: String,
    pub owner: u32,
    pub identity: String,
    /// Its application's icon in the theme (from its MPRIS DesktopEntry: the
    /// desktop entry's Icon, else the entry's own name, as icons usually are).
    pub icon: String,
    /// Playing, Paused or Stopped.
    pub status: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art_url: String,
    pub length_us: i64,
    pub track_id: String,
    pub position_us: i64,
    /// When the position was last heard (it runs on from there while playing).
    pub position_at: Option<Instant>,
    pub rate: f64,
    pub can_seek: bool,
    /// Bumped whenever it starts playing (or comes): the most recent one is Now Playing.
    pub order: u64,
}

impl Player {
    fn playing(&self) -> bool {
        self.status == "Playing"
    }

    /// Where it is now: the last heard position, run on by the clock while playing.
    pub fn position(&self, now: Instant) -> i64 {
        let ran = match (self.playing(), self.position_at) {
            (true, Some(at)) => (now.saturating_duration_since(at).as_secs_f64() * self.rate.max(0.0) * 1e6) as i64,
            _ => 0,
        };
        let at = self.position_us.saturating_add(ran).max(0);
        if self.length_us > 0 { at.min(self.length_us) } else { at }
    }
}

/// A number as MPRIS players send it (x, t, i, u or d).
fn number(v: &Value) -> Option<i64> {
    Some(match v {
        Value::I64(x) => *x,
        Value::U64(x) => *x as i64,
        Value::I32(x) => *x as i64,
        Value::U32(x) => *x as i64,
        Value::Double(x) => *x as i64,
        _ => return None,
    })
}

fn text(v: &Value) -> String {
    match v {
        Value::Str(s) | Value::Path(s) => s.chars().filter(|c| !c.is_control()).take(200).collect(),
        // xesam:artist is a list: the names, with commas.
        Value::Array(_, items) => items.iter().map(text).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", "),
        Value::Variant(inner) => text(inner),
        _ => String::new(),
    }
}

/// A dictionary of org.mpris.MediaPlayer2.Player's properties applied to `p`
/// (GetAll's answer, PropertiesChanged's changes). True if it started playing.
pub fn apply(p: &mut Player, props: &Value, now: Instant) -> bool {
    let was = p.playing();
    if let Some(s) = lookup(props, "PlaybackStatus") {
        // Where it got to under the old status (running while it played),
        // then the new one: a pause holds the place it reached.
        p.position_us = p.position(now);
        p.position_at = Some(now);
        p.status = text(s);
    }
    if let Some(r) = lookup(props, "Rate").and_then(|v| if let Value::Double(d) = v { Some(*d) } else { None }) {
        p.rate = r;
    }
    if let Some(Value::Bool(b)) = lookup(props, "CanSeek") {
        p.can_seek = *b;
    }
    if let Some(x) = lookup(props, "Position").and_then(number) {
        (p.position_us, p.position_at) = (x, Some(now));
    }
    if let Some(m) = lookup(props, "Metadata") {
        let get = |k: &str| lookup(m, k).map(text).unwrap_or_default();
        let id = get("mpris:trackid");
        if id != p.track_id {
            // Another track: it starts from its beginning unless told otherwise.
            (p.position_us, p.position_at) = (0, Some(now));
        }
        p.track_id = id;
        p.title = get("xesam:title");
        p.artist = get("xesam:artist");
        p.album = get("xesam:album");
        p.art_url = get("mpris:artUrl");
        p.length_us = lookup(m, "mpris:length").and_then(number).unwrap_or(0).max(0);
    }
    !was && p.playing()
}

/// The player Now Playing: the most recent of those playing, else the most recent.
pub fn current(players: &[Player]) -> Option<&Player> {
    players.iter().filter(|p| p.playing()).max_by_key(|p| p.order).or_else(|| players.iter().max_by_key(|p| p.order))
}

/// "3:07", "1:02:03".
pub fn clock(us: i64) -> String {
    let s = (us.max(0) / 1_000_000) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "\u{2026}" }
}

/// The tile for what plays (None: nothing playing or paused, the tile goes:
/// quiet when nothing plays). The media keys still reach a stopped player.
pub fn tile(players: &[Player], art: Option<&Picture>, now: Instant) -> Option<Tile> {
    let p = current(players).filter(|p| p.status == "Playing" || p.status == "Paused")?;
    let title = if p.title.is_empty() { p.identity.clone() } else { p.title.clone() };
    let mut rows = Vec::new();
    for (k, v) in [("Artist", &p.artist), ("Album", &p.album), ("Player", &p.identity)] {
        if !v.is_empty() {
            rows.push((k.to_string(), short(v, 60)));
        }
    }
    // AlbumWall's own transport buttons, in one row (Dave, 2026-11-27).
    let icon =
        |id: &str, label: &str, i: Icon| Control { id: id.into(), label: label.into(), kind: ControlKind::Icon(i) };
    let mut controls = vec![
        icon("previous", "Previous", Icon::Previous),
        if p.playing() { icon("playpause", "Pause", Icon::Pause) } else { icon("playpause", "Play", Icon::Play) },
        icon("next", "Next", Icon::Next),
    ];
    if p.can_seek && p.length_us > 0 {
        let at = p.position(now);
        controls.push(Control {
            id: "seek".into(),
            label: format!("{} of {}", clock(at), clock(p.length_us)),
            kind: ControlKind::Slider {
                value: at as f64 / 1e6,
                min: 0.0,
                max: (p.length_us as f64 / 1e6).max(1.0),
                step: 1.0,
            },
        });
    }
    let others: Vec<String> = players
        .iter()
        .filter(|o| o.name != p.name)
        .map(|o| format!("{} ({})", o.identity, o.status.to_lowercase()))
        .collect();
    Some(Tile {
        id: TILE.into(),
        value: short(&title, 24),
        label: short(if p.playing() { "Playing" } else { &p.status }, 24),
        state: TileState::Fine,
        group: "System".into(),
        page: String::new(),
        // On the dock, the player's own icon with play or pause over its
        // corner: a title cannot be read at that size (Dave, 2026-11-27).
        icon: p.icon.clone(),
        badge: Some(if p.playing() { Icon::Play } else { Icon::Pause }),
        card: Card {
            title: short(&title, 200),
            rows,
            controls,
            note: if others.is_empty() { String::new() } else { short(&format!("Also: {}", others.join(", ")), 200) },
            picture: art.cloned(),
            ..Card::default()
        },
    })
}

/// What a call the edge made was for.
pub enum Asked {
    All(String),
    Identity(String),
    DesktopEntry(String),
    Position(String),
}

#[derive(Default)]
pub struct NowPlaying {
    pub players: Vec<Player>,
    order: u64,
    /// Calls the edge made, by serial: what for, and which connection must answer.
    pending: HashMap<u32, (Asked, u32)>,
    /// The picture for an artUrl (None while it decodes, or when it cannot be).
    art: Option<(String, Option<Picture>)>,
    art_tx: Option<channel::Sender<(String, Option<Picture>)>>,
}

/// A local file named by an artUrl, decoded in the confined helper and fitted
/// into ART × ART.
fn decode_art(url: &str) -> Option<Picture> {
    let path = url.strip_prefix("file://")?;
    let path = std::path::PathBuf::from(percent_decode(path)?);
    if !path.is_absolute() || std::fs::metadata(&path).ok()?.len() > 32 << 20 {
        return None;
    }
    let p = vergence_decode::decode_file(&path, Some((ART, ART))).ok()?;
    let img = image::RgbaImage::from_raw(p.width, p.height, p.frames.into_iter().next()?.rgba)?;
    let k = (ART as f32 / img.width().max(img.height()) as f32).min(1.0);
    let (w, h) = (((img.width() as f32 * k).round() as u32).max(1), ((img.height() as f32 * k).round() as u32).max(1));
    let img = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    Picture::new(w as u16, h as u16, img.into_raw(), "the cover".into())
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

impl Server {
    /// The picture's channel and the once-a-second look while the card is open.
    pub fn nowplaying_start(&mut self) {
        let (tx, rx) = channel::channel::<(String, Option<Picture>)>();
        let _ = self.loop_handle.insert_source(rx, |e, _, st| {
            if let channel::Event::Msg((url, pic)) = e
                && st.nowplaying.art.as_ref().is_some_and(|(u, _)| *u == url)
            {
                st.nowplaying.art = Some((url, pic));
                st.nowplaying_show();
            }
        });
        self.nowplaying.art_tx = Some(tx);
        let _ = self.loop_handle.insert_source(Timer::from_duration(Duration::from_secs(1)), |_, _, st| {
            st.nowplaying_tick();
            TimeoutAction::ToDuration(Duration::from_secs(1))
        });
    }

    /// An org.mpris.MediaPlayer2.* name changed hands (dbus.rs): a player came or went.
    pub fn nowplaying_owner(&mut self, name: &str, new: Option<u32>) {
        self.nowplaying.players.retain(|p| p.name != name);
        if let Some(owner) = new {
            self.nowplaying.order += 1;
            let order = self.nowplaying.order;
            self.nowplaying.players.push(Player {
                name: name.into(),
                owner,
                status: "Stopped".into(),
                rate: 1.0,
                order,
                ..Default::default()
            });
            self.nowplaying_ask(name, PROPERTIES, "GetAll", &[Value::Str(PLAYER.into())], Asked::All(name.into()));
            self.nowplaying_ask(
                name,
                PROPERTIES,
                "Get",
                &[Value::Str(ROOT.into()), Value::Str("Identity".into())],
                Asked::Identity(name.into()),
            );
            self.nowplaying_ask(
                name,
                PROPERTIES,
                "Get",
                &[Value::Str(ROOT.into()), Value::Str("DesktopEntry".into())],
                Asked::DesktopEntry(name.into()),
            );
        }
        self.nowplaying_show();
    }

    fn nowplaying_ask(&mut self, name: &str, iface: &str, member: &str, args: &[Value], asked: Asked) {
        let Some(owner) = self.nowplaying.players.iter().find(|p| p.name == name).map(|p| p.owner) else { return };
        if let Some(serial) = self.dbus_edge_call(name, PATH, iface, member, args) {
            self.nowplaying.pending.insert(serial, (asked, owner));
        }
    }

    /// A reply to a call the edge made (dbus.rs routes it here by serial).
    /// False if it was none of ours.
    pub fn nowplaying_answer(&mut self, from: u32, m: &Message) -> bool {
        let Some(serial) = m.reply_serial else { return false };
        let Some((asked, owner)) = self.nowplaying.pending.remove(&serial) else { return false };
        if owner != from || m.kind != message::METHOD_RETURN {
            return true;
        }
        let args = m.args().unwrap_or_default();
        let now = Instant::now();
        let first = |a: &[Value]| a.first().map(|v| if let Value::Variant(x) = v { (**x).clone() } else { v.clone() });
        match asked {
            Asked::All(name) => {
                if let (Some(p), Some(dict)) =
                    (self.nowplaying.players.iter_mut().find(|p| p.name == name), args.first())
                {
                    apply(p, dict, now);
                }
            }
            Asked::Identity(name) => {
                if let (Some(p), Some(v)) = (self.nowplaying.players.iter_mut().find(|p| p.name == name), first(&args))
                {
                    p.identity = text(&v);
                }
            }
            Asked::DesktopEntry(name) => {
                let entry = first(&args).map(|v| text(&v)).unwrap_or_default();
                // Its entry's icon (the dock knows the machine's applications),
                // else the entry's name; only a name in the theme, never a path.
                let icon = self
                    .shelf
                    .apps
                    .iter()
                    .find(|e| e.id == entry)
                    .map(|e| e.icon.clone())
                    .unwrap_or_else(|| entry.clone());
                if let Some(p) = self.nowplaying.players.iter_mut().find(|p| p.name == name) {
                    p.icon =
                        if icon.contains('/') || icon.chars().any(char::is_control) { String::new() } else { icon };
                }
            }
            Asked::Position(name) => {
                if let (Some(p), Some(x)) = (
                    self.nowplaying.players.iter_mut().find(|p| p.name == name),
                    first(&args).as_ref().and_then(number),
                ) {
                    (p.position_us, p.position_at) = (x, Some(now));
                }
            }
        }
        self.nowplaying_show();
        true
    }

    /// PropertiesChanged or Seeked from connection `from` on /org/mpris/MediaPlayer2.
    pub fn nowplaying_signal(&mut self, from: u32, m: &Message) {
        let now = Instant::now();
        let args = m.args().unwrap_or_default();
        let mut stale = Vec::new();
        for p in self.nowplaying.players.iter_mut().filter(|p| p.owner == from) {
            let mut started = false;
            match (m.interface.as_deref(), m.member.as_deref()) {
                (Some(PROPERTIES), Some("PropertiesChanged"))
                    if args.first().and_then(Value::as_str) == Some(PLAYER) =>
                {
                    if let Some(dict) = args.get(1) {
                        started |= apply(p, dict, now);
                    }
                    // Named but not sent: asked for again.
                    if args.get(2).is_some_and(|v| matches!(v, Value::Array(_, items) if !items.is_empty())) {
                        stale.push(p.name.clone());
                    }
                }
                (Some(PLAYER), Some("Seeked")) => {
                    if let Some(x) = args.first().and_then(number) {
                        (p.position_us, p.position_at) = (x, Some(now));
                    }
                }
                _ => {}
            }
            if started {
                self.nowplaying.order += 1;
                p.order = self.nowplaying.order;
            }
        }
        for name in stale {
            self.nowplaying_ask(&name, PROPERTIES, "GetAll", &[Value::Str(PLAYER.into())], Asked::All(name.clone()));
        }
        self.nowplaying_show();
    }

    /// The tile as it is now, and the picture for it.
    fn nowplaying_show(&mut self) {
        let url = current(&self.nowplaying.players).map(|p| p.art_url.clone()).unwrap_or_default();
        if !url.is_empty() && self.nowplaying.art.as_ref().map(|(u, _)| u.as_str()) != Some(url.as_str()) {
            self.nowplaying.art = Some((url.clone(), None));
            if let Some(tx) = self.nowplaying.art_tx.clone() {
                let url = url.clone();
                let _ = std::thread::Builder::new().name("nowplaying-art".into()).spawn(move || {
                    let pic = decode_art(&url);
                    let _ = tx.send((url, pic));
                });
            }
        }
        let art = self.nowplaying.art.as_ref().filter(|(u, _)| *u == url).and_then(|(_, p)| p.as_ref());
        match tile(&self.nowplaying.players, art, Instant::now()) {
            Some(t) => self.tile_set(Owner::Desktop, t),
            None => {
                self.tile_remove(Owner::Desktop, TILE);
            }
        }
    }

    /// Once a second: while its card is open and something plays, the position
    /// asked for and the bar moved on.
    fn nowplaying_tick(&mut self) {
        let open = self.tiles.open_card() == Some(TILE);
        let Some(p) = current(&self.nowplaying.players).filter(|p| open && p.playing()).map(|p| p.name.clone()) else {
            return;
        };
        self.nowplaying_ask(
            &p,
            PROPERTIES,
            "Get",
            &[Value::Str(PLAYER.into()), Value::Str("Position".into())],
            Asked::Position(p.clone()),
        );
        self.nowplaying_show();
    }

    /// A media key or a media.* action: for the player Now Playing.
    pub fn media_key(&mut self, m: Media) {
        let Some(name) = current(&self.nowplaying.players).map(|p| p.name.clone()) else {
            return tracing::info!("nowplaying: {m:?}, but no player is on the bus");
        };
        let method = match m {
            Media::PlayPause => "PlayPause",
            Media::Pause => "Pause",
            Media::Next => "Next",
            Media::Previous => "Previous",
            Media::Stop => "Stop",
        };
        tracing::info!("nowplaying: {method} to {name}");
        let _ = self.dbus_edge_call(&name, PATH, PLAYER, method, &[]);
    }

    /// A control on the Now Playing card.
    pub fn nowplaying_action(&mut self, control: &str, value: f64) {
        match control {
            "previous" => self.media_key(Media::Previous),
            "playpause" => self.media_key(Media::PlayPause),
            "next" => self.media_key(Media::Next),
            "seek" => {
                let Some(p) = current(&self.nowplaying.players) else { return };
                if !p.can_seek || p.track_id.is_empty() {
                    return;
                }
                let (name, track) = (p.name.clone(), p.track_id.clone());
                let at = (value.max(0.0) * 1e6) as i64;
                let _ = self.dbus_edge_call(&name, PATH, PLAYER, "SetPosition", &[Value::Path(track), Value::I64(at)]);
                if let Some(p) = self.nowplaying.players.iter_mut().find(|p| p.name == name) {
                    (p.position_us, p.position_at) = (at, Some(Instant::now()));
                }
                self.nowplaying_show();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(entries: Vec<(&str, Value)>) -> Value {
        Value::Array(
            "{sv}".into(),
            entries
                .into_iter()
                .map(|(k, v)| Value::Entry(Box::new(Value::Str(k.into())), Box::new(Value::Variant(Box::new(v)))))
                .collect(),
        )
    }

    fn albumwall_playing() -> Value {
        dict(vec![
            ("PlaybackStatus", Value::Str("Playing".into())),
            ("CanSeek", Value::Bool(true)),
            ("Rate", Value::Double(1.0)),
            (
                "Metadata",
                dict(vec![
                    ("mpris:trackid", Value::Path("/org/mpris/MediaPlayer2/albumwall/track/abc".into())),
                    ("xesam:title", Value::Str("Time".into())),
                    ("xesam:artist", Value::Array("s".into(), vec![Value::Str("Pink Floyd".into())])),
                    ("xesam:album", Value::Str("The Dark Side of the Moon".into())),
                    ("mpris:length", Value::I64(413_000_000)),
                    ("mpris:artUrl", Value::Str("file:///m/cover.jpg".into())),
                ]),
            ),
        ])
    }

    #[test]
    fn a_player_as_it_says() {
        let t0 = Instant::now();
        let mut p = Player { rate: 1.0, ..Default::default() };
        assert!(apply(&mut p, &albumwall_playing(), t0), "it started playing");
        assert_eq!((p.title.as_str(), p.artist.as_str(), p.length_us), ("Time", "Pink Floyd", 413_000_000));
        // The position runs on while playing, and holds still once paused.
        let later = t0 + Duration::from_secs(10);
        assert!((p.position(later) - 10_000_000).abs() < 1_000);
        apply(&mut p, &dict(vec![("PlaybackStatus", Value::Str("Paused".into()))]), later);
        assert!((p.position(later + Duration::from_secs(30)) - 10_000_000).abs() < 1_000);
        // Another track starts from its beginning.
        let next = dict(vec![(
            "Metadata",
            dict(vec![("mpris:trackid", Value::Path("/t/2".into())), ("xesam:title", Value::Str("Breathe".into()))]),
        )]);
        apply(&mut p, &next, later);
        assert_eq!((p.title.as_str(), p.position_us), ("Breathe", 0));
        // Odd number types (some players send u or d for the length).
        let odd = dict(vec![(
            "Metadata",
            dict(vec![("mpris:length", Value::U64(5)), ("mpris:trackid", Value::Path("/t/2".into()))]),
        )]);
        apply(&mut p, &odd, later);
        assert_eq!(p.length_us, 5);
    }

    #[test]
    fn the_most_recent_player_playing_is_now_playing() {
        let a = Player { name: "a".into(), status: "Playing".into(), order: 1, ..Default::default() };
        let b = Player { name: "b".into(), status: "Paused".into(), order: 2, ..Default::default() };
        let c = Player { name: "c".into(), status: "Playing".into(), order: 3, ..Default::default() };
        assert_eq!(current(&[a.clone(), b.clone()]).unwrap().name, "a");
        assert_eq!(current(&[a.clone(), b.clone(), c]).unwrap().name, "c");
        assert_eq!(current(&[b]).unwrap().name, "b", "none playing: the most recent");
        assert!(current(&[]).is_none());
    }

    #[test]
    fn the_tile_and_its_card() {
        let t0 = Instant::now();
        let mut p = Player {
            name: "org.mpris.MediaPlayer2.albumwall".into(),
            identity: "AlbumWall".into(),
            rate: 1.0,
            order: 2,
            ..Default::default()
        };
        apply(&mut p, &albumwall_playing(), t0);
        let other = Player {
            name: "x".into(),
            identity: "Firefox".into(),
            status: "Paused".into(),
            order: 1,
            ..Default::default()
        };
        let t = tile(&[p.clone(), other], None, t0).unwrap();
        assert!(t.check().is_ok(), "{:?}", t.check());
        assert_eq!((t.value.as_str(), t.label.as_str()), ("Time", "Playing"));
        assert_eq!(t.card.rows[0], ("Artist".to_string(), "Pink Floyd".to_string()));
        let ids: Vec<&str> = t.card.controls.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["previous", "playpause", "next", "seek"]);
        assert_eq!(t.card.controls[1].label, "Pause");
        assert_eq!(t.card.controls[3].label, "0:00 of 6:53");
        assert_eq!(t.card.note, "Also: Firefox (paused)");
        assert_eq!(t.badge, Some(Icon::Play));
        // A long title fits the dock.
        p.title = "A very long title that will not fit on the dock at all".into();
        let t = tile(&[p], None, t0).unwrap();
        assert!(t.check().is_ok() && t.value.ends_with('\u{2026}'));
        assert!(tile(&[], None, t0).is_none());
        let stopped = Player { status: "Stopped".into(), ..Default::default() };
        assert!(tile(&[stopped], None, t0).is_none(), "quiet when nothing plays");
    }

    #[test]
    fn art_addresses() {
        assert_eq!(percent_decode("/m/A%20b/c.jpg").as_deref(), Some("/m/A b/c.jpg"));
        assert_eq!(percent_decode("/bad%zz"), None);
        assert!(decode_art("https://example.com/cover.jpg").is_none(), "nothing fetched");
        assert!(decode_art("file://relative/x.jpg").is_none());
        assert_eq!(clock(413_000_000), "6:53");
        assert_eq!(clock(3_723_000_000), "1:02:03");
    }
}
