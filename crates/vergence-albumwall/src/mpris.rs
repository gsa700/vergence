//! AlbumWall on the session bus as a media player (MPRIS, org.mpris.MediaPlayer2),
//! ported from the Avalonia app's Mpris.cs (AlbumWall milestone 2, step 5): anything
//! that speaks MPRIS (playerctl, a desktop's media controls, the media keys through
//! them) can play, pause, skip, seek and see what plays. The same name keeps one
//! AlbumWall at a time: a second start finds it owned and hands its command (or
//! Raise) to the first over the bus, then exits (CommandLine.cs, SingleInstance.cs).
//!
//! What Mpris.cs learned, kept here:
//! - Pushed, not only pulled: clients read the properties once and then believe only
//!   PropertiesChanged, so every change of what is announced is sent (Position never:
//!   clients run it on from Rate and correct on Seeked, which every jump sends).
//! - mpris:trackid differs per track (a hash of its path: the folders stay off the bus).
//! - Seek past the end is Next; SetPosition for a track no longer playing is ignored.
//!
//! [`handle`] is the protocol, pure (tested without a bus); [`Service`] is the thread
//! that owns the connection and answers from the window's latest [`State`].

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vergence_dbus::client::{self, Client};
use vergence_dbus::{Message, Value, message};

pub const BUS_NAME: &str = "org.mpris.MediaPlayer2.albumwall";
pub const PATH: &str = "/org/mpris/MediaPlayer2";
pub const ROOT: &str = "org.mpris.MediaPlayer2";
pub const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const TRACK_ROOT: &str = "/org/mpris/MediaPlayer2/albumwall/track";

/// What the window tells the bus about itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    pub playing: bool,
    pub has_track: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// `file://` of the album's own cover file; empty when it has none (or only one inside a track).
    pub art_url: String,
    pub length_us: i64,
    pub position_us: i64,
    pub volume: f64,
    /// The playing file's path: only hashed into mpris:trackid.
    pub track_key: String,
}

/// What a call asks of the window.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Raise,
    Quit,
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
    Stop,
    /// To this many seconds into the track playing.
    SeekTo(f64),
}

/// A call's answer: its values, or an error's name and text.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Values(Vec<Value>),
    Error(&'static str, String),
}

const UNKNOWN_METHOD: &str = "org.freedesktop.DBus.Error.UnknownMethod";
const INVALID_ARGS: &str = "org.freedesktop.DBus.Error.InvalidArgs";
const UNKNOWN_PROPERTY: &str = "org.freedesktop.DBus.Error.UnknownProperty";

fn v(x: Value) -> Value {
    Value::Variant(Box::new(x))
}

fn string(s: &str) -> Value {
    Value::Str(s.to_string())
}

fn dict(entries: Vec<(&str, Value)>) -> Value {
    Value::Array(
        "{sv}".into(),
        entries.into_iter().map(|(k, x)| Value::Entry(Box::new(string(k)), Box::new(v(x)))).collect(),
    )
}

fn strings(items: &[&str]) -> Value {
    Value::Array("s".into(), items.iter().map(|s| string(s)).collect())
}

/// An object path for the playing track: only its last element is ours, hex of a
/// hash of the file's path (Mpris.cs TrackId).
pub fn track_id(s: &State) -> String {
    if !s.has_track || s.track_key.is_empty() {
        return format!("{TRACK_ROOT}/none");
    }
    let hex = crate::md5::hex(&crate::md5::digest(s.track_key.as_bytes()));
    format!("{TRACK_ROOT}/{}", &hex[..16])
}

fn status(s: &State) -> &'static str {
    if !s.has_track {
        "Stopped"
    } else if s.playing {
        "Playing"
    } else {
        "Paused"
    }
}

fn metadata(s: &State) -> Value {
    let mut m = vec![("mpris:trackid", Value::Path(track_id(s)))];
    if s.has_track {
        m.push(("xesam:title", string(&s.title)));
        m.push(("xesam:artist", strings(&[&s.artist])));
        m.push(("xesam:album", string(&s.album)));
        m.push(("mpris:length", Value::I64(s.length_us)));
        if !s.art_url.is_empty() {
            m.push(("mpris:artUrl", string(&s.art_url)));
        }
    }
    dict(m)
}

const ROOT_PROPERTIES: [&str; 7] =
    ["CanQuit", "CanRaise", "HasTrackList", "Identity", "DesktopEntry", "SupportedUriSchemes", "SupportedMimeTypes"];
const PLAYER_PROPERTIES: [&str; 13] = [
    "PlaybackStatus",
    "Metadata",
    "Position",
    "Rate",
    "MinimumRate",
    "MaximumRate",
    "Volume",
    "CanGoNext",
    "CanGoPrevious",
    "CanPlay",
    "CanPause",
    "CanSeek",
    "CanControl",
];
/// What moves while it runs, and so what a change is announced for (no Position).
const CHANGING: [&str; 8] =
    ["PlaybackStatus", "Metadata", "Volume", "CanGoNext", "CanGoPrevious", "CanPlay", "CanPause", "CanSeek"];

/// A property's value, or None for a name it does not have.
fn property(name: &str, s: &State) -> Option<Value> {
    Some(match name {
        "CanQuit" | "CanRaise" => Value::Bool(true),
        "HasTrackList" => Value::Bool(false),
        "Identity" => string("AlbumWall"),
        "DesktopEntry" => string("vergence-albumwall"),
        "SupportedUriSchemes" | "SupportedMimeTypes" => strings(&[]),
        "PlaybackStatus" => string(status(s)),
        "Metadata" => metadata(s),
        "Position" => Value::I64(s.position_us),
        "Rate" | "MinimumRate" | "MaximumRate" => Value::Double(1.0),
        "Volume" => Value::Double(s.volume),
        "CanGoNext" | "CanGoPrevious" | "CanPlay" | "CanPause" | "CanControl" => Value::Bool(true),
        // Only for a track whose length is known: nowhere to say "the end" otherwise.
        "CanSeek" => Value::Bool(s.has_track && s.length_us > 0),
        _ => return None,
    })
}

fn all(names: &[&str], s: &State) -> Value {
    dict(names.iter().filter_map(|n| property(n, s).map(|x| (*n, x))).collect())
}

/// PropertiesChanged's arguments for what is announced now.
pub fn changed_args(s: &State) -> Vec<Value> {
    vec![string(PLAYER), all(&CHANGING, s), strings(&[])]
}

/// The answer to a method call on our object, and what it asks of the window.
pub fn handle(call: &Message, s: &State) -> (Answer, Option<Cmd>) {
    let none = || (Answer::Values(vec![]), None);
    let args = call.args().unwrap_or_default();
    let text = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or_default().to_string();
    let member = call.member.as_deref().unwrap_or_default();
    match (call.interface.as_deref().unwrap_or_default(), member) {
        ("org.freedesktop.DBus.Introspectable", "Introspect") => (Answer::Values(vec![string(INTROSPECT)]), None),
        ("org.freedesktop.DBus.Peer", "Ping") => none(),
        (PROPERTIES, "Get") => {
            let (iface, name) = (text(0), text(1));
            let known = match iface.as_str() {
                ROOT => ROOT_PROPERTIES.contains(&name.as_str()),
                PLAYER => PLAYER_PROPERTIES.contains(&name.as_str()),
                _ => false,
            };
            match property(&name, s).filter(|_| known) {
                Some(x) => (Answer::Values(vec![v(x)]), None),
                None => (Answer::Error(UNKNOWN_PROPERTY, format!("no property {iface}.{name}")), None),
            }
        }
        (PROPERTIES, "GetAll") => {
            let names: &[&str] = match text(0).as_str() {
                ROOT => &ROOT_PROPERTIES,
                PLAYER => &PLAYER_PROPERTIES,
                _ => &[],
            };
            (Answer::Values(vec![all(names, s)]), None)
        }
        // Volume and Rate are the only writable ones, and neither is offered:
        // a Set changes nothing (Mpris.cs).
        (PROPERTIES, "Set") => none(),
        (ROOT, "Raise") => (Answer::Values(vec![]), Some(Cmd::Raise)),
        (ROOT, "Quit") => (Answer::Values(vec![]), Some(Cmd::Quit)),
        (PLAYER, "Play") => (Answer::Values(vec![]), Some(Cmd::Play)),
        (PLAYER, "Pause") => (Answer::Values(vec![]), Some(Cmd::Pause)),
        (PLAYER, "PlayPause") => (Answer::Values(vec![]), Some(Cmd::PlayPause)),
        (PLAYER, "Next") => (Answer::Values(vec![]), Some(Cmd::Next)),
        (PLAYER, "Previous") => (Answer::Values(vec![]), Some(Cmd::Previous)),
        (PLAYER, "Stop") => (Answer::Values(vec![]), Some(Cmd::Stop)),
        (PLAYER, "Seek") => {
            let Some(Value::I64(by)) = args.first() else {
                return (Answer::Error(INVALID_ARGS, "Seek(x)".into()), None);
            };
            if !s.has_track || s.length_us <= 0 {
                return none();
            }
            let to = s.position_us.saturating_add(*by);
            if to >= s.length_us {
                return (Answer::Values(vec![]), Some(Cmd::Next));
            }
            (Answer::Values(vec![]), Some(Cmd::SeekTo(to.max(0) as f64 / 1e6)))
        }
        (PLAYER, "SetPosition") => {
            let (Some(Value::Path(track)), Some(Value::I64(at))) = (args.first(), args.get(1)) else {
                return (Answer::Error(INVALID_ARGS, "SetPosition(ox)".into()), None);
            };
            if !s.has_track || *track != track_id(s) || *at < 0 || *at > s.length_us {
                return none();
            }
            (Answer::Values(vec![]), Some(Cmd::SeekTo(*at as f64 / 1e6)))
        }
        // OpenUri is not offered: SupportedUriSchemes is empty.
        (iface, m) => (Answer::Error(UNKNOWN_METHOD, format!("no method {iface}.{m}")), None),
    }
}

/// What the window asks the service to send.
pub enum Out {
    /// What is announced has changed: PropertiesChanged.
    Changed,
    /// The track jumped to this many microseconds: Seeked.
    Seeked(i64),
}

/// The thread that owns the bus connection: answers calls from the latest state,
/// hands commands to the window (waking it), sends what the window asks.
pub struct Service {
    tx: Sender<Out>,
    pub state: Arc<Mutex<State>>,
}

impl Service {
    pub fn start(client: Client, cmds: Sender<Cmd>, wake: impl Fn() + Send + 'static) -> Service {
        let (tx, rx) = channel();
        let state: Arc<Mutex<State>> = Arc::new(Mutex::new(State { volume: 1.0, ..Default::default() }));
        let shared = state.clone();
        let _ = std::thread::Builder::new().name("mpris".into()).spawn(move || serve(client, shared, rx, cmds, wake));
        Service { tx, state }
    }

    pub fn set(&self, s: State) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }

    pub fn send(&self, o: Out) {
        let _ = self.tx.send(o);
    }
}

fn serve(mut c: Client, state: Arc<Mutex<State>>, rx: Receiver<Out>, cmds: Sender<Cmd>, wake: impl Fn()) {
    let now = || state.lock().unwrap_or_else(|e| e.into_inner()).clone();
    loop {
        loop {
            let sent = match rx.try_recv() {
                Ok(Out::Changed) => c.emit(PATH, PROPERTIES, "PropertiesChanged", &changed_args(&now())),
                Ok(Out::Seeked(at)) => c.emit(PATH, PLAYER, "Seeked", &[Value::I64(at)]),
                Err(TryRecvError::Empty) => break,
                // The window is gone: so is the player.
                Err(TryRecvError::Disconnected) => return,
            };
            if sent.is_err() {
                return;
            }
        }
        match c.next_message(Duration::from_millis(50)) {
            Ok(Some(m)) if m.kind == message::METHOD_CALL => {
                if m.path.as_deref() != Some(PATH) {
                    let _ = c.reply_error(&m, "org.freedesktop.DBus.Error.UnknownObject", "no such object");
                    continue;
                }
                let (answer, cmd) = handle(&m, &now());
                let r = match answer {
                    Answer::Values(vals) => c.reply(&m, &vals),
                    Answer::Error(name, text) => c.reply_error(&m, name, &text),
                };
                if r.is_err() {
                    return;
                }
                if let Some(cmd) = cmd {
                    let _ = cmds.send(cmd);
                    wake();
                }
            }
            Ok(_) => {}
            Err(_) => return,
        }
    }
}

/// Who this start is: the first (it owns the name now, and serves), a second (the
/// first has it), or one with no session bus (it runs on its own, as before).
pub enum Start {
    First(Client),
    Second(Client),
    NoBus,
}

pub fn claim() -> Start {
    let Ok(mut c) = Client::session() else { return Start::NoBus };
    match c.request_name(BUS_NAME, client::DO_NOT_QUEUE) {
        Ok(1) | Ok(4) => Start::First(c),
        Ok(_) => Start::Second(c),
        Err(_) => Start::NoBus,
    }
}

/// The command-line flags the running AlbumWall takes (CommandLine.cs), and the
/// MPRIS method each is carried as.
pub const COMMANDS: [(&str, &str, &str, &str); 8] = [
    ("--play-pause", PLAYER, "PlayPause", "Play or pause."),
    ("--play", PLAYER, "Play", "Play, after a pause."),
    ("--pause", PLAYER, "Pause", "Pause."),
    ("--next", PLAYER, "Next", "Next track."),
    ("--previous", PLAYER, "Previous", "Previous track."),
    ("--stop", PLAYER, "Stop", "Stop."),
    ("--show", ROOT, "Raise", "Bring the window forward. Starting the app again while it runs does the same."),
    ("--quit", ROOT, "Quit", "Close the app."),
];

/// The command in the arguments: its flag, interface and method.
pub fn command_of(args: &[String]) -> Option<(&'static str, &'static str, &'static str)> {
    args.iter().find_map(|a| {
        let a = a.to_ascii_lowercase();
        COMMANDS.iter().find(|c| c.0 == a).map(|c| (c.0, c.1, c.2))
    })
}

/// Calls `method` on the running AlbumWall.
pub fn ask(c: &mut Client, interface: &str, method: &str) -> Result<(), String> {
    c.call(Message::method_call(BUS_NAME, PATH, interface, method, &[]), Duration::from_secs(5))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// --help.
pub fn help() -> String {
    let width = COMMANDS.iter().map(|c| c.0.len()).max().unwrap_or(0) + 3;
    let mut out = vec![
        format!("AlbumWall for Vergence {}", env!("CARGO_PKG_VERSION")),
        String::new(),
        "Run with nothing to open the wall, or to bring it forward if it is already open.".into(),
        "Given to the copy that is running (they never start one):".into(),
        String::new(),
    ];
    out.extend(COMMANDS.iter().map(|c| format!("  {:width$}{}", c.0, c.3)));
    out.push(String::new());
    out.push(format!("  {:width$}{}", "--help", "This list."));
    out.push(format!("  {:width$}{}", "--version", "The version."));
    out.join("\n")
}

/// A local file as a `file://` address, its path percent-encoded.
pub fn file_url(path: &std::path::Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

const INTROSPECT: &str = r#"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node>
  <interface name="org.freedesktop.DBus.Introspectable">
    <method name="Introspect"><arg name="xml" type="s" direction="out"/></method>
  </interface>
  <interface name="org.freedesktop.DBus.Peer">
    <method name="Ping"/>
  </interface>
  <interface name="org.freedesktop.DBus.Properties">
    <method name="Get"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
    <method name="GetAll"><arg type="s" direction="in"/><arg type="a{sv}" direction="out"/></method>
    <method name="Set"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/></method>
    <signal name="PropertiesChanged"><arg type="s"/><arg type="a{sv}"/><arg type="as"/></signal>
  </interface>
  <interface name="org.mpris.MediaPlayer2">
    <method name="Raise"/>
    <method name="Quit"/>
    <property name="CanQuit" type="b" access="read"/>
    <property name="CanRaise" type="b" access="read"/>
    <property name="HasTrackList" type="b" access="read"/>
    <property name="Identity" type="s" access="read"/>
    <property name="DesktopEntry" type="s" access="read"/>
    <property name="SupportedUriSchemes" type="as" access="read"/>
    <property name="SupportedMimeTypes" type="as" access="read"/>
  </interface>
  <interface name="org.mpris.MediaPlayer2.Player">
    <method name="Next"/>
    <method name="Previous"/>
    <method name="Pause"/>
    <method name="PlayPause"/>
    <method name="Stop"/>
    <method name="Play"/>
    <method name="Seek"><arg name="Offset" type="x" direction="in"/></method>
    <method name="SetPosition"><arg name="TrackId" type="o" direction="in"/><arg name="Position" type="x" direction="in"/></method>
    <signal name="Seeked"><arg name="Position" type="x"/></signal>
    <property name="PlaybackStatus" type="s" access="read"/>
    <property name="Rate" type="d" access="read"/>
    <property name="Metadata" type="a{sv}" access="read"/>
    <property name="Volume" type="d" access="read"/>
    <property name="Position" type="x" access="read"/>
    <property name="MinimumRate" type="d" access="read"/>
    <property name="MaximumRate" type="d" access="read"/>
    <property name="CanGoNext" type="b" access="read"/>
    <property name="CanGoPrevious" type="b" access="read"/>
    <property name="CanPlay" type="b" access="read"/>
    <property name="CanPause" type="b" access="read"/>
    <property name="CanSeek" type="b" access="read"/>
    <property name="CanControl" type="b" access="read"/>
  </interface>
</node>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn playing() -> State {
        State {
            playing: true,
            has_track: true,
            title: "Time".into(),
            artist: "Pink Floyd".into(),
            album: "The Dark Side of the Moon".into(),
            art_url: "file:///m/a/cover.jpg".into(),
            length_us: 413_000_000,
            position_us: 100_000_000,
            volume: 1.0,
            track_key: "/m/a/04 Time.flac".into(),
        }
    }

    fn call(iface: &str, member: &str, args: &[Value]) -> Message {
        let mut m = Message::method_call(BUS_NAME, PATH, iface, member, args);
        // As it arrives: parsed from its bytes, so args() reads what a peer sent.
        m.serial = 7;
        m.sender = Some(":1.9".into());
        Message::parse(&m.serialize()).unwrap()
    }

    fn values(a: Answer) -> Vec<Value> {
        match a {
            Answer::Values(v) => v,
            Answer::Error(n, t) => panic!("{n}: {t}"),
        }
    }

    #[test]
    fn commands_and_seeking() {
        let s = playing();
        for (m, c) in [
            ("Play", Cmd::Play),
            ("Pause", Cmd::Pause),
            ("PlayPause", Cmd::PlayPause),
            ("Next", Cmd::Next),
            ("Previous", Cmd::Previous),
            ("Stop", Cmd::Stop),
        ] {
            assert_eq!(handle(&call(PLAYER, m, &[]), &s).1, Some(c), "{m}");
        }
        assert_eq!(handle(&call(ROOT, "Raise", &[]), &s).1, Some(Cmd::Raise));
        // Seek: by an offset; past the end is Next; before the start is the start.
        assert_eq!(handle(&call(PLAYER, "Seek", &[Value::I64(5_000_000)]), &s).1, Some(Cmd::SeekTo(105.0)));
        assert_eq!(handle(&call(PLAYER, "Seek", &[Value::I64(400_000_000)]), &s).1, Some(Cmd::Next));
        assert_eq!(handle(&call(PLAYER, "Seek", &[Value::I64(-500_000_000)]), &s).1, Some(Cmd::SeekTo(0.0)));
        // SetPosition: only for the track playing, within it.
        let here = Value::Path(track_id(&s));
        assert_eq!(
            handle(&call(PLAYER, "SetPosition", &[here.clone(), Value::I64(60_000_000)]), &s).1,
            Some(Cmd::SeekTo(60.0))
        );
        let old = Value::Path(format!("{TRACK_ROOT}/0000000000000000"));
        assert_eq!(handle(&call(PLAYER, "SetPosition", &[old, Value::I64(60_000_000)]), &s).1, None);
        assert_eq!(handle(&call(PLAYER, "SetPosition", &[here, Value::I64(500_000_000)]), &s).1, None);
        // Nothing playing: no seek.
        assert_eq!(handle(&call(PLAYER, "Seek", &[Value::I64(1)]), &State::default()).1, None);
        // Unknown methods are errors.
        assert!(matches!(handle(&call(PLAYER, "OpenUri", &[string("x")]), &s).0, Answer::Error(UNKNOWN_METHOD, _)));
    }

    #[test]
    fn properties() {
        let s = playing();
        let get =
            |iface: &str, name: &str, s: &State| handle(&call(PROPERTIES, "Get", &[string(iface), string(name)]), s).0;
        assert_eq!(values(get(PLAYER, "PlaybackStatus", &s)), vec![v(string("Playing"))]);
        assert_eq!(values(get(PLAYER, "PlaybackStatus", &State::default())), vec![v(string("Stopped"))]);
        assert_eq!(values(get(PLAYER, "Position", &s)), vec![v(Value::I64(100_000_000))]);
        assert_eq!(values(get(ROOT, "Identity", &s)), vec![v(string("AlbumWall"))]);
        assert!(matches!(get(ROOT, "PlaybackStatus", &s), Answer::Error(UNKNOWN_PROPERTY, _)));
        let all = values(handle(&call(PROPERTIES, "GetAll", &[string(PLAYER)]), &s).0);
        let Value::Array(sig, items) = &all[0] else { panic!() };
        assert_eq!((sig.as_str(), items.len()), ("{sv}", PLAYER_PROPERTIES.len()));
        let meta = client::lookup(&all[0], "Metadata").unwrap();
        assert_eq!(client::lookup(meta, "xesam:title"), Some(&string("Time")));
        assert_eq!(client::lookup(meta, "mpris:length"), Some(&Value::I64(413_000_000)));
        assert_eq!(client::lookup(meta, "mpris:trackid"), Some(&Value::Path(track_id(&s))));
        // Nothing playing: the track id alone, a valid path.
        let idle = values(get(PLAYER, "Metadata", &State::default()));
        let Value::Variant(m) = &idle[0] else { panic!() };
        let Value::Array(_, items) = &**m else { panic!() };
        assert_eq!(items.len(), 1);
        // The whole introspection data is offered, and the reply survives the wire.
        let x = values(handle(&call("org.freedesktop.DBus.Introspectable", "Introspect", &[]), &s).0);
        assert!(x[0].as_str().is_some_and(|t| t.contains("org.mpris.MediaPlayer2.Player")));
        let mut reply = Message::method_return(&call(PROPERTIES, "GetAll", &[]), ":1.2", &all);
        reply.serial = 8; // numbered as it is sent
        let back = Message::parse(&reply.serialize()).expect("the reply parses");
        assert_eq!(back.args().unwrap(), all);
    }

    #[test]
    fn track_ids_differ_per_track_and_hide_the_path() {
        let a = playing();
        let b = State { track_key: "/m/a/05 The Great Gig.flac".into(), ..playing() };
        assert_ne!(track_id(&a), track_id(&b));
        assert!(!track_id(&a).contains("Time"));
        let last = track_id(&a).rsplit('/').next().unwrap().to_string();
        assert!(last.len() == 16 && last.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(track_id(&State::default()), format!("{TRACK_ROOT}/none"));
    }

    #[test]
    fn announced_changes_leave_position_out() {
        let args = changed_args(&playing());
        assert_eq!(args[0], string(PLAYER));
        assert!(client::lookup(&args[1], "PlaybackStatus").is_some());
        assert!(client::lookup(&args[1], "Position").is_none());
    }

    #[test]
    fn the_command_line() {
        let a = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(command_of(&a(&["--next"])), Some(("--next", PLAYER, "Next")));
        assert_eq!(command_of(&a(&["--SHOW"])), Some(("--show", ROOT, "Raise")));
        assert_eq!(command_of(&a(&["--scan", "/m"])), None);
        assert!(help().contains("--play-pause"));
        assert_eq!(file_url(std::path::Path::new("/m/A b/c#.jpg")), "file:///m/A%20b/c%23.jpg");
    }
}
