//! The session bus (whitepaper §6.8): the messages between the server and a
//! client, version 1.
//!
//! A client connects to the socket named by `VERGENCE_BUS` (see
//! [`crate::socket_path`]), sends [`ClientMsg::Hello`] first, and gets
//! [`ServerMsg::Welcome`]. Requests carry a `serial` the client picks; the
//! reply to a request carries the same serial. Events the client subscribed
//! to arrive with serial 0.
//!
//! Version 1 has the bus itself and two topics: the monitors
//! ([`Topic::Outputs`]) and the desktop settings ([`Topic::Settings`]); and
//! notifications ([`ClientMsg::Notify`]), whose events go only to the client
//! that made them. Version 2 (2026-11-12, bar milestone 1): the server
//! draws every native window's bar; `Create` lost `own_frame`, and the
//! window's own Move and Resize requests are gone. Version 3 (2026-11-13,
//! look milestone 1): a window may draw under its bar (`UnderBar`), and
//! `Configure` carries the `inset` it draws there; a tab may be marked
//! `alert` (its program runs as the superuser). Version 4 (2026-11-14,
//! window styles milestone 1): a window says its kind, whether it is modal,
//! and its smallest and largest size (`Style`). Version 5 (2026-11-14): a
//! bar may carry the program's own buttons and say where its title starts
//! (`Bar::Tools`), their presses heard as `BarButton`. Version 6
//! (2026-11-14): the server tells a window the wallpaper's color behind it
//! (`Tint`), for a window that mixes a little of it into its grounds.
//! Version 7 (2026-11-14, settings milestone 1): the input devices
//! ([`Topic::Inputs`]): those present and the kinds the machine has had, so
//! the settings app shows a Touchpad or Touchscreen page once there is one.
//! Version 8 (2026-11-14, GPUs milestone 1): each screen says which GPU shows
//! it and which draws it (`OutputInfo::shown_by`, `drawn_by`). Version 9
//! (2026-11-14, settings milestone 2): each screen lists its modes and which
//! is running (`OutputInfo::modes`, `mode`); window memory's apps can be
//! listed and forgotten (`ListPlaces`, `ForgetPlaces`, `Places`). Version 10
//! (2026-11-14, app settings milestone 1): a window's commands for its app
//! menu (`WinRequest::Commands`, `WinEvent::Command`, crate::commands) and
//! its bar style (`WinRequest::BarStyle`). Version 11 (the same evening):
//! a command can be `featured`, one of the app's everyday few on the
//! menu's first panel. Version 12 (tools milestone 1): a window may ask for
//! a size (`WinRequest::Size`); the server decides. Version 13: Force
//! Quit's `ListApps`, `Apps` and `ForceQuit`. Version 14 (editor milestone
//! 1): `Tab::edited`. Version 15 (files milestone 1): `Open`, files and
//! links opened by the server with the applications chosen for their types.
//! Version 16 (files milestone 1, step 8): drag and drop between native
//! windows (`WinRequest::DragStart`/`DragAccept`, `WinEvent::DragOver`/
//! `DragLeave`/`DragDrop`/`DragEnded`), and `WinRequest::Activate`.
//! Version 17 (step 9): `WinKind::Desktop`, the desktop's icons. Version
//! 18: `WinRequest::ContextMenu`, a window's right click answered by the
//! server's menu. Version 19 (dock milestone 1): live tiles. Version 20
//! (backup milestone 1, step 1): `WinRequest::Menu`, a window's own
//! right-click menu by its lines (`window::MenuLine`), submenus included.
//! Version 21: `WinEvent::Configure::text`, a screen's text size factor.
//! Version 22: `OutputInfo::text_ratio`, for the settings app's text slider.
//! Version 23 (overview milestone 1, step 6): `WinRequest::GroundClicked`,
//! the desktop's bare ground clicked (the windows move aside).

use crate::wire::{Decoder, Encoder, Frame, WireError};

/// The protocol version this crate speaks.
pub const VERSION: u32 = 23;

/// Something a client can subscribe to. Values are fixed on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    /// The monitors: their names, places and scales; sent again whenever
    /// one is added, removed, moved or rescaled.
    Outputs,
    /// The desktop settings (appearance, theme, fonts, cursor): all of them
    /// on subscribe, then the ones that changed.
    Settings,
    /// The input devices: on subscribe, then whenever one comes or goes or
    /// a kind is seen for the first time.
    Inputs,
}

impl Topic {
    pub fn to_wire(self) -> u16 {
        match self {
            Topic::Outputs => 1,
            Topic::Settings => 2,
            Topic::Inputs => 3,
        }
    }

    pub fn from_wire(v: u16) -> Option<Topic> {
        match v {
            1 => Some(Topic::Outputs),
            2 => Some(Topic::Settings),
            3 => Some(Topic::Inputs),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Topic::Outputs => "outputs",
            Topic::Settings => "settings",
            Topic::Inputs => "inputs",
        }
    }

    pub fn from_name(s: &str) -> Option<Topic> {
        match s {
            "outputs" => Some(Topic::Outputs),
            "settings" => Some(Topic::Settings),
            "inputs" => Some(Topic::Inputs),
            _ => None,
        }
    }
}

/// What kind of input device something is. Values are fixed on the wire;
/// a kind from a newer server is kept as `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputKind {
    Keyboard,
    Mouse,
    Touchpad,
    Touchscreen,
    Other(u8),
}

impl InputKind {
    pub fn to_wire(self) -> u8 {
        match self {
            InputKind::Keyboard => 1,
            InputKind::Mouse => 2,
            InputKind::Touchpad => 3,
            InputKind::Touchscreen => 4,
            InputKind::Other(v) => v,
        }
    }

    pub fn from_wire(v: u8) -> InputKind {
        match v {
            1 => InputKind::Keyboard,
            2 => InputKind::Mouse,
            3 => InputKind::Touchpad,
            4 => InputKind::Touchscreen,
            v => InputKind::Other(v),
        }
    }

    /// Its name in words and in files ("touchpad").
    pub fn name(self) -> &'static str {
        match self {
            InputKind::Keyboard => "keyboard",
            InputKind::Mouse => "mouse",
            InputKind::Touchpad => "touchpad",
            InputKind::Touchscreen => "touchscreen",
            InputKind::Other(_) => "other",
        }
    }

    pub fn from_name(s: &str) -> Option<InputKind> {
        [InputKind::Keyboard, InputKind::Mouse, InputKind::Touchpad, InputKind::Touchscreen].into_iter().find(|k| k.name() == s)
    }
}

/// An input device present now: its name (the device's own, untrusted) and kind.
#[derive(Debug, Clone, PartialEq)]
pub struct InputInfo {
    pub name: String,
    pub kind: InputKind,
}

/// A setting's value. Tags are fixed on the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    U32(u32),
    I32(i32),
    F64(f64),
    Str(String),
    /// A color, each channel 0 to 1.
    Rgb(f64, f64, f64),
}

/// One setting: a namespace (e.g. "org.freedesktop.appearance"), a key
/// (e.g. "color-scheme") and its value. Namespaces and keys follow what
/// existing toolkits ask for, so the D-Bus edge can pass them on unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    pub namespace: String,
    pub key: String,
    pub value: SettingValue,
}

/// Why the server refused a request. Values are fixed on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// Anything sent before Hello.
    NoHello,
    /// Hello asked for a version the server doesn't speak.
    Version,
    /// A topic number the server doesn't know (from a newer client).
    UnknownTopic,
    /// An opcode the server doesn't know, or a message that didn't decode.
    BadMessage,
    /// Not allowed for this client (§7).
    Denied,
    /// Understood, but this server does not do it yet (native windows,
    /// before native milestone 1's step 3).
    NotYet,
    /// Understood and allowed, but it could not be done (a file no
    /// application opens, a program that would not start); the message says.
    Failed,
    /// A code from a newer server.
    Other(u16),
}

impl ErrorCode {
    pub fn to_wire(self) -> u16 {
        match self {
            ErrorCode::NoHello => 1,
            ErrorCode::Version => 2,
            ErrorCode::UnknownTopic => 3,
            ErrorCode::BadMessage => 4,
            ErrorCode::Denied => 5,
            ErrorCode::NotYet => 6,
            ErrorCode::Failed => 7,
            ErrorCode::Other(v) => v,
        }
    }

    pub fn from_wire(v: u16) -> ErrorCode {
        match v {
            1 => ErrorCode::NoHello,
            2 => ErrorCode::Version,
            3 => ErrorCode::UnknownTopic,
            4 => ErrorCode::BadMessage,
            5 => ErrorCode::Denied,
            6 => ErrorCode::NotYet,
            7 => ErrorCode::Failed,
            v => ErrorCode::Other(v),
        }
    }
}

/// One monitor, as the server shows it. Geometry is in the desktop's
/// logical coordinates (Contract 1's unit is still to come: today these are
/// the server's logical pixels).
#[derive(Debug, Clone, PartialEq)]
pub struct OutputInfo {
    /// The monitor's identity (a hash of its EDID): the same on any port, so
    /// settings kept per monitor follow it.
    pub id: String,
    /// The connector, e.g. "eDP-1", "HDMI-A-1".
    pub name: String,
    /// Make and model from the EDID, e.g. "ASUS VS229".
    pub description: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Pixels per logical pixel on this monitor (calculated from its EDID).
    pub scale: f64,
    /// How the scale was found, in words.
    pub note: String,
    /// The screen's default scale (the sharpest near the calculated one), and
    /// its scale before the size preference (chosen, or the default).
    pub default_scale: f64,
    pub base_scale: f64,
    /// The GPU the screen is connected to, in words ("NVIDIA graphics
    /// (nouveau)"), and the one that draws it (the same, or the main GPU
    /// when the frames are drawn there and copied: GPUs milestone 1).
    pub shown_by: String,
    pub drawn_by: String,
    /// The monitor's modes, each once (largest first, as the monitor lists
    /// them), and the one running: an index into `modes`, or `u32::MAX`.
    pub modes: Vec<ModeInfo>,
    pub mode: u32,
    /// The screen's calculated scale over its base scale (version 22): how
    /// much smaller than calculated its text would be. Its text factor is
    /// this to the power of `[displays] text-strength`.
    pub text_ratio: f64,
}

/// A monitor's mode: its size in pixels, its refresh in millihertz (59940 is
/// 59.94 Hz, kept apart from 60000), and whether the monitor prefers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeInfo {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,
    pub preferred: bool,
}

/// Client to server.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientMsg {
    /// First message on a connection. `app_id` names the application (the
    /// same id its windows carry); the server records it but does not trust
    /// it for permissions (§7.4).
    Hello { version: u32, app_id: String },
    Ping { serial: u32 },
    /// Start receiving a topic's events. The reply is [`ServerMsg::Done`],
    /// followed at once by the topic's current state as an event.
    Subscribe { serial: u32, topic: Topic },
    Unsubscribe { serial: u32, topic: Topic },
    /// The monitors now, as a reply (no subscription).
    GetOutputs { serial: u32 },
    /// Every setting now, as a reply (no subscription).
    GetSettings { serial: u32 },
    /// Show a notification (or replace one of this client's: `replaces`, else
    /// 0). `urgency` 0 low, 1 normal, 2 critical; `timeout_ms` -1 for the
    /// server's choice, 0 to stay until clicked. `actions` are (key, label);
    /// "default" is a click on the notification. Reply: [`ServerMsg::Notified`].
    Notify { serial: u32, replaces: u32, summary: String, body: String, urgency: u8, timeout_ms: i32, actions: Vec<(String, String)> },
    /// Close one of this client's notifications. Reply: Done, or an error.
    CloseNotification { serial: u32, id: u32 },
    /// The apps window memory keeps places for. Reply: [`ServerMsg::Places`].
    ListPlaces { serial: u32 },
    /// Forget every place of an app (its class), on every setup; an empty
    /// `app` forgets all. Reply: Done.
    ForgetPlaces { serial: u32, app: String },
    /// The user's running applications (Force Quit, tools milestone 1).
    /// Reply: [`ServerMsg::Apps`].
    ListApps { serial: u32 },
    /// End an application at once: every process of its windows is sent
    /// SIGKILL (the user's own processes only). Reply: Done, or an error.
    ForceQuit { serial: u32, app: String },
    /// The answer to [`ServerMsg::Alive`]: this client's loop is running.
    Alive { serial: u32 },
    /// Open files (absolute paths) and links with an application: `app`, a
    /// desktop entry id ("firefox.desktop"), or, empty, each one's default
    /// for its type (files milestone 1). The server starts them as the dock
    /// does. Reply: Done, or an error naming what could not be opened.
    Open { serial: u32, items: Vec<String>, app: String },
    /// Put up a live tile on the dock, or change one of this client's with
    /// the same id (dock milestone 1; tile.rs). It stays until removed or
    /// until this connection ends. Reply: Done, or an error saying why the
    /// tile was refused.
    SetTile { serial: u32, tile: crate::tile::Tile },
    /// Take down one of this client's tiles. Reply: Done, or an error.
    RemoveTile { serial: u32, id: String },
    /// Windows (window.rs).
    Window(crate::window::WinRequest),
}

/// A running application, as Force Quit lists it: its key (the dock's),
/// name, icon (`icon_side` square, RGBA; empty if none), whether it is
/// answering, and whether it is being force-quit now (asked to end, killed
/// in a moment if it does not).
#[derive(Debug, Clone, PartialEq)]
pub struct AppInfo {
    pub key: String,
    pub name: String,
    pub icon_side: u16,
    pub icon: Vec<u8>,
    pub responding: bool,
    pub quitting: bool,
}

/// The icon side Apps carries (points; drawn sharp at scale 1, smooth above).
pub const APP_ICON: u16 = 32;
/// The most files or links one Open asks for (more are refused by the server).
pub const MAX_OPEN: usize = 256;
/// The most applications one Apps message lists with icons (a frame is 64 KB).
pub const MAX_APP_ICONS: usize = 12;

/// Server to client.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMsg {
    Welcome { version: u32, client_id: u32 },
    Pong { serial: u32 },
    /// A request that has no other reply succeeded.
    Done { serial: u32 },
    Error { serial: u32, code: ErrorCode, message: String },
    /// The monitors: a reply (serial of the request) or an event (serial 0),
    /// and the setup they make (the set of their EDIDs, hashed): settings kept
    /// per setup (the arrangement) are keyed by it.
    Outputs { serial: u32, outputs: Vec<OutputInfo>, setup: String },
    /// Settings: a reply or the first event after subscribing (`complete`:
    /// every setting), or an event with the ones that changed. A setting
    /// that disappeared is not sent; a later version may add removals.
    Settings { serial: u32, complete: bool, settings: Vec<Setting> },
    /// The id of a notification just shown.
    Notified { serial: u32, id: u32 },
    /// One of this client's notifications closed. `reason`: 1 expired, 2
    /// dismissed by the user, 3 closed by the client.
    NotificationClosed { id: u32, reason: u32 },
    /// The user chose an action on one of this client's notifications.
    ActionInvoked { id: u32, key: String },
    /// The input devices present, and every kind this machine has had
    /// (remembered by the server across sessions): an event (serial 0).
    Inputs { serial: u32, devices: Vec<InputInfo>, seen: Vec<InputKind> },
    /// The apps window memory keeps places for (their class) and how many places each.
    Places { serial: u32, apps: Vec<(String, u32)> },
    /// The user's running applications.
    Apps { serial: u32, apps: Vec<AppInfo> },
    /// Is this client's loop running? Answer with [`ClientMsg::Alive`] (the
    /// toolkit does, from its loop): a client that has not answered for a
    /// few seconds is "not responding" in Force Quit.
    Alive { serial: u32 },
    /// One of this client's tiles: its card opened (a live card may now
    /// publish about once a second) or closed (back to quiet).
    CardOpened { id: String },
    CardClosed { id: String },
    /// A click on one of this client's tiles that names no settings page.
    TileClicked { id: String },
    /// A control on one of this client's cards was used (tile.rs `Control`).
    TileAction { id: String, control: String, value: f64 },
    /// Windows (window.rs).
    Window(crate::window::WinEvent),
}

mod op {
    // Client to server.
    pub const HELLO: u16 = 1;
    pub const PING: u16 = 2;
    pub const SUBSCRIBE: u16 = 3;
    pub const UNSUBSCRIBE: u16 = 4;
    pub const GET_OUTPUTS: u16 = 5;
    pub const GET_SETTINGS: u16 = 6;
    pub const NOTIFY: u16 = 7;
    pub const CLOSE_NOTIFICATION: u16 = 8;
    pub const LIST_PLACES: u16 = 9;
    pub const FORGET_PLACES: u16 = 10;
    pub const LIST_APPS: u16 = 11;
    pub const FORCE_QUIT: u16 = 12;
    pub const ALIVE_REPLY: u16 = 13;
    pub const OPEN: u16 = 14;
    pub const SET_TILE: u16 = 15;
    pub const REMOVE_TILE: u16 = 16;
    // Server to client.
    pub const WELCOME: u16 = 1;
    pub const PONG: u16 = 2;
    pub const DONE: u16 = 3;
    pub const ERROR: u16 = 4;
    pub const OUTPUTS: u16 = 5;
    pub const SETTINGS: u16 = 6;
    pub const NOTIFIED: u16 = 7;
    pub const NOTIFICATION_CLOSED: u16 = 8;
    pub const ACTION_INVOKED: u16 = 9;
    pub const INPUTS: u16 = 10;
    pub const PLACES: u16 = 11;
    pub const APPS: u16 = 12;
    pub const ALIVE: u16 = 13;
    pub const CARD_OPENED: u16 = 14;
    pub const CARD_CLOSED: u16 = 15;
    pub const TILE_CLICKED: u16 = 16;
    pub const TILE_ACTION: u16 = 17;
}

fn put_setting_value(e: &mut Encoder, v: &SettingValue) {
    match v {
        SettingValue::Bool(b) => e.u8(1).bool(*b),
        SettingValue::U32(x) => e.u8(2).u32(*x),
        SettingValue::I32(x) => e.u8(3).i32(*x),
        SettingValue::F64(x) => e.u8(4).f64(*x),
        SettingValue::Str(s) => e.u8(5).str(s),
        SettingValue::Rgb(r, g, b) => e.u8(6).f64(*r).f64(*g).f64(*b),
    };
}

fn get_setting_value(d: &mut Decoder) -> Result<SettingValue, WireError> {
    Ok(match d.u8()? {
        1 => SettingValue::Bool(d.bool()?),
        2 => SettingValue::U32(d.u32()?),
        3 => SettingValue::I32(d.i32()?),
        4 => SettingValue::F64(d.f64()?),
        5 => SettingValue::Str(d.str()?),
        6 => SettingValue::Rgb(d.f64()?, d.f64()?, d.f64()?),
        _ => return Err(WireError::BadValue("setting value")),
    })
}

/// A decode failure that the server answers with an error rather than
/// ending the connection: the frame was well formed but its content wasn't
/// understood (a newer client, most likely).
#[derive(Debug, Clone, PartialEq)]
pub struct Unknown {
    /// The request's serial when it could be read, else 0.
    pub serial: u32,
    pub code: ErrorCode,
    pub error: WireError,
}

fn topic(d: &mut Decoder) -> Result<Topic, (u16, WireError)> {
    let v = d.u16().map_err(|e| (0, e))?;
    Topic::from_wire(v).ok_or((v, WireError::BadValue("topic")))
}

impl ClientMsg {
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let e = match self {
            ClientMsg::Hello { version, app_id } => {
                let mut e = Encoder::frame(op::HELLO);
                e.u32(*version).str(app_id);
                e
            }
            ClientMsg::Ping { serial } => {
                let mut e = Encoder::frame(op::PING);
                e.u32(*serial);
                e
            }
            ClientMsg::Subscribe { serial, topic } | ClientMsg::Unsubscribe { serial, topic } => {
                let opcode = if matches!(self, ClientMsg::Subscribe { .. }) { op::SUBSCRIBE } else { op::UNSUBSCRIBE };
                let mut e = Encoder::frame(opcode);
                e.u32(*serial).u16(topic.to_wire());
                e
            }
            ClientMsg::GetOutputs { serial } => {
                let mut e = Encoder::frame(op::GET_OUTPUTS);
                e.u32(*serial);
                e
            }
            ClientMsg::GetSettings { serial } => {
                let mut e = Encoder::frame(op::GET_SETTINGS);
                e.u32(*serial);
                e
            }
            ClientMsg::Notify { serial, replaces, summary, body, urgency, timeout_ms, actions } => {
                let mut e = Encoder::frame(op::NOTIFY);
                e.u32(*serial).u32(*replaces).str(summary).str(body).u8(*urgency).i32(*timeout_ms).count(actions.len());
                for (k, l) in actions {
                    e.str(k).str(l);
                }
                e
            }
            ClientMsg::CloseNotification { serial, id } => {
                let mut e = Encoder::frame(op::CLOSE_NOTIFICATION);
                e.u32(*serial).u32(*id);
                e
            }
            ClientMsg::ListPlaces { serial } => {
                let mut e = Encoder::frame(op::LIST_PLACES);
                e.u32(*serial);
                e
            }
            ClientMsg::ForgetPlaces { serial, app } => {
                let mut e = Encoder::frame(op::FORGET_PLACES);
                e.u32(*serial).str(app);
                e
            }
            ClientMsg::ListApps { serial } => {
                let mut e = Encoder::frame(op::LIST_APPS);
                e.u32(*serial);
                e
            }
            ClientMsg::ForceQuit { serial, app } => {
                let mut e = Encoder::frame(op::FORCE_QUIT);
                e.u32(*serial).str(app);
                e
            }
            ClientMsg::Open { serial, items, app } => {
                let mut e = Encoder::frame(op::OPEN);
                e.u32(*serial).str(app).count(items.len());
                for i in items {
                    e.str(i);
                }
                e
            }
            ClientMsg::Alive { serial } => {
                let mut e = Encoder::frame(op::ALIVE_REPLY);
                e.u32(*serial);
                e
            }
            ClientMsg::SetTile { serial, tile } => {
                let mut e = Encoder::frame(op::SET_TILE);
                e.u32(*serial);
                tile.encode(&mut e);
                e
            }
            ClientMsg::RemoveTile { serial, id } => {
                let mut e = Encoder::frame(op::REMOVE_TILE);
                e.u32(*serial).str(id);
                e
            }
            // Its descriptors are lost here: send it with encode_with_fds.
            ClientMsg::Window(w) => return w.encode().map(|(bytes, _)| bytes),
        };
        e.finish()
    }

    /// The frame and the descriptors that go beside it (borrowed from the
    /// message, which must outlive the write).
    pub fn encode_with_fds(&self) -> Result<(Vec<u8>, Vec<std::os::fd::RawFd>), WireError> {
        match self {
            ClientMsg::Window(w) => w.encode(),
            m => Ok((m.encode()?, Vec::new())),
        }
    }

    /// As [`ClientMsg::decode`], taking the descriptors a message carries
    /// from `fds`. A window message that does not decode is an error to end
    /// the connection with (its descriptors may be out of step).
    pub fn decode_with_fds(frame: &Frame, fds: &mut dyn crate::window::FdSource) -> Result<ClientMsg, Unknown> {
        if crate::window::is_window_opcode(frame.opcode) {
            return crate::window::WinRequest::decode(frame.opcode, &frame.payload, fds).map(ClientMsg::Window).map_err(|error| Unknown { serial: 0, code: ErrorCode::BadMessage, error });
        }
        ClientMsg::decode(frame)
    }

    pub fn decode(frame: &Frame) -> Result<ClientMsg, Unknown> {
        if crate::window::is_window_opcode(frame.opcode) {
            return ClientMsg::decode_with_fds(frame, &mut crate::window::NoFds);
        }
        let mut d = Decoder::new(&frame.payload);
        let bad = |serial: u32, error: WireError| Unknown { serial, code: ErrorCode::BadMessage, error };
        match frame.opcode {
            op::HELLO => {
                let version = d.u32().map_err(|e| bad(0, e))?;
                let app_id = d.str().map_err(|e| bad(0, e))?;
                Ok(ClientMsg::Hello { version, app_id })
            }
            op::PING => Ok(ClientMsg::Ping { serial: d.u32().map_err(|e| bad(0, e))? }),
            op::SUBSCRIBE | op::UNSUBSCRIBE => {
                let serial = d.u32().map_err(|e| bad(0, e))?;
                let topic = topic(&mut d).map_err(|(v, error)| Unknown {
                    serial,
                    code: if v == 0 && error == WireError::Short { ErrorCode::BadMessage } else { ErrorCode::UnknownTopic },
                    error,
                })?;
                Ok(if frame.opcode == op::SUBSCRIBE {
                    ClientMsg::Subscribe { serial, topic }
                } else {
                    ClientMsg::Unsubscribe { serial, topic }
                })
            }
            op::GET_OUTPUTS => Ok(ClientMsg::GetOutputs { serial: d.u32().map_err(|e| bad(0, e))? }),
            op::GET_SETTINGS => Ok(ClientMsg::GetSettings { serial: d.u32().map_err(|e| bad(0, e))? }),
            op::NOTIFY => {
                let serial = d.u32().map_err(|e| bad(0, e))?;
                let rest = (|| -> Result<ClientMsg, WireError> {
                    let replaces = d.u32()?;
                    let summary = d.str()?;
                    let body = d.str()?;
                    let urgency = d.u8()?;
                    let timeout_ms = d.i32()?;
                    let n = d.count(8)?;
                    let mut actions = Vec::with_capacity(n);
                    for _ in 0..n {
                        actions.push((d.str()?, d.str()?));
                    }
                    Ok(ClientMsg::Notify { serial, replaces, summary, body, urgency, timeout_ms, actions })
                })();
                rest.map_err(|e| bad(serial, e))
            }
            op::CLOSE_NOTIFICATION => {
                let serial = d.u32().map_err(|e| bad(0, e))?;
                let id = d.u32().map_err(|e| bad(serial, e))?;
                Ok(ClientMsg::CloseNotification { serial, id })
            }
            op::LIST_PLACES => Ok(ClientMsg::ListPlaces { serial: d.u32().map_err(|e| bad(0, e))? }),
            op::FORGET_PLACES => {
                let serial = d.u32().map_err(|e| bad(0, e))?;
                let app = d.str().map_err(|e| bad(serial, e))?;
                Ok(ClientMsg::ForgetPlaces { serial, app })
            }
            op::LIST_APPS => Ok(ClientMsg::ListApps { serial: d.u32().map_err(|e| bad(0, e))? }),
            op::FORCE_QUIT => {
                let serial = d.u32().map_err(|e| bad(0, e))?;
                let app = d.str().map_err(|e| bad(serial, e))?;
                Ok(ClientMsg::ForceQuit { serial, app })
            }
            op::ALIVE_REPLY => Ok(ClientMsg::Alive { serial: d.u32().map_err(|e| bad(0, e))? }),
            op::SET_TILE => {
                let serial = d.u32().map_err(|e| bad(0, e))?;
                let tile = crate::tile::Tile::decode(&mut d).map_err(|e| bad(serial, e))?;
                Ok(ClientMsg::SetTile { serial, tile })
            }
            op::REMOVE_TILE => {
                let serial = d.u32().map_err(|e| bad(0, e))?;
                let id = d.str().map_err(|e| bad(serial, e))?;
                Ok(ClientMsg::RemoveTile { serial, id })
            }
            op::OPEN => {
                let serial = d.u32().map_err(|e| bad(0, e))?;
                let rest = (|| -> Result<ClientMsg, WireError> {
                    let app = d.str()?;
                    let n = d.count(4)?;
                    let mut items = Vec::with_capacity(n.min(MAX_OPEN));
                    for _ in 0..n {
                        items.push(d.str()?);
                    }
                    Ok(ClientMsg::Open { serial, items, app })
                })();
                rest.map_err(|e| bad(serial, e))
            }
            other => {
                // Every request starts with its serial; answer with it if present.
                let serial = d.u32().unwrap_or(0);
                Err(bad(serial, WireError::UnknownOpcode(other)))
            }
        }
    }
}

impl ServerMsg {
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let e = match self {
            ServerMsg::Welcome { version, client_id } => {
                let mut e = Encoder::frame(op::WELCOME);
                e.u32(*version).u32(*client_id);
                e
            }
            ServerMsg::Pong { serial } => {
                let mut e = Encoder::frame(op::PONG);
                e.u32(*serial);
                e
            }
            ServerMsg::Done { serial } => {
                let mut e = Encoder::frame(op::DONE);
                e.u32(*serial);
                e
            }
            ServerMsg::Error { serial, code, message } => {
                let mut e = Encoder::frame(op::ERROR);
                e.u32(*serial).u16(code.to_wire()).str(message);
                e
            }
            ServerMsg::Outputs { serial, outputs, setup } => {
                let mut e = Encoder::frame(op::OUTPUTS);
                e.u32(*serial).count(outputs.len());
                for o in outputs {
                    e.str(&o.id).str(&o.name).str(&o.description).i32(o.x).i32(o.y).i32(o.width).i32(o.height).f64(o.scale).str(&o.note).f64(o.default_scale).f64(o.base_scale).str(&o.shown_by).str(&o.drawn_by).count(o.modes.len());
                    for m in &o.modes {
                        e.u32(m.width).u32(m.height).u32(m.refresh_mhz).bool(m.preferred);
                    }
                    e.u32(o.mode).f64(o.text_ratio);
                }
                e.str(setup);
                e
            }
            ServerMsg::Settings { serial, complete, settings } => {
                let mut e = Encoder::frame(op::SETTINGS);
                e.u32(*serial).bool(*complete).count(settings.len());
                for s in settings {
                    e.str(&s.namespace).str(&s.key);
                    put_setting_value(&mut e, &s.value);
                }
                e
            }
            ServerMsg::Notified { serial, id } => {
                let mut e = Encoder::frame(op::NOTIFIED);
                e.u32(*serial).u32(*id);
                e
            }
            ServerMsg::NotificationClosed { id, reason } => {
                let mut e = Encoder::frame(op::NOTIFICATION_CLOSED);
                e.u32(*id).u32(*reason);
                e
            }
            ServerMsg::ActionInvoked { id, key } => {
                let mut e = Encoder::frame(op::ACTION_INVOKED);
                e.u32(*id).str(key);
                e
            }
            ServerMsg::Alive { serial } => {
                let mut e = Encoder::frame(op::ALIVE);
                e.u32(*serial);
                e
            }
            ServerMsg::CardOpened { id } | ServerMsg::CardClosed { id } | ServerMsg::TileClicked { id } => {
                let opcode = match self {
                    ServerMsg::CardOpened { .. } => op::CARD_OPENED,
                    ServerMsg::CardClosed { .. } => op::CARD_CLOSED,
                    _ => op::TILE_CLICKED,
                };
                let mut e = Encoder::frame(opcode);
                e.str(id);
                e
            }
            ServerMsg::TileAction { id, control, value } => {
                let mut e = Encoder::frame(op::TILE_ACTION);
                e.str(id).str(control).f64(*value);
                e
            }
            ServerMsg::Apps { serial, apps } => {
                let mut e = Encoder::frame(op::APPS);
                e.u32(*serial).count(apps.len());
                for a in apps {
                    e.str(&a.key).str(&a.name).u16(a.icon_side).bytes(&a.icon).bool(a.responding).bool(a.quitting);
                }
                e
            }
            ServerMsg::Places { serial, apps } => {
                let mut e = Encoder::frame(op::PLACES);
                e.u32(*serial).count(apps.len());
                for (a, n) in apps {
                    e.str(a).u32(*n);
                }
                e
            }
            ServerMsg::Inputs { serial, devices, seen } => {
                let mut e = Encoder::frame(op::INPUTS);
                e.u32(*serial).count(devices.len());
                for dev in devices {
                    e.str(&dev.name).u8(dev.kind.to_wire());
                }
                e.count(seen.len());
                for k in seen {
                    e.u8(k.to_wire());
                }
                e
            }
            // Its descriptors are lost here: send it with encode_with_fds.
            ServerMsg::Window(w) => return w.encode().map(|(bytes, _)| bytes),
        };
        e.finish()
    }

    /// The frame and the descriptors that go beside it.
    pub fn encode_with_fds(&self) -> Result<(Vec<u8>, Vec<std::os::fd::RawFd>), WireError> {
        match self {
            ServerMsg::Window(w) => w.encode(),
            m => Ok((m.encode()?, Vec::new())),
        }
    }

    /// As [`ServerMsg::decode`], taking the descriptors a message carries from `fds`.
    pub fn decode_with_fds(frame: &Frame, fds: &mut dyn crate::window::FdSource) -> Result<ServerMsg, WireError> {
        if crate::window::is_window_opcode(frame.opcode) {
            return crate::window::WinEvent::decode(frame.opcode, &frame.payload, fds).map(ServerMsg::Window);
        }
        ServerMsg::decode(frame)
    }

    pub fn decode(frame: &Frame) -> Result<ServerMsg, WireError> {
        if crate::window::is_window_opcode(frame.opcode) {
            return ServerMsg::decode_with_fds(frame, &mut crate::window::NoFds);
        }
        let mut d = Decoder::new(&frame.payload);
        Ok(match frame.opcode {
            op::WELCOME => ServerMsg::Welcome { version: d.u32()?, client_id: d.u32()? },
            op::PONG => ServerMsg::Pong { serial: d.u32()? },
            op::DONE => ServerMsg::Done { serial: d.u32()? },
            op::ERROR => ServerMsg::Error { serial: d.u32()?, code: ErrorCode::from_wire(d.u16()?), message: d.str()? },
            op::OUTPUTS => {
                let serial = d.u32()?;
                // Each output: six strings (4 bytes each at least), four i32, three f64, a count and an index.
                let n = d.count(4 + 4 + 4 + 16 + 8 + 4 + 16 + 4 + 4 + 8)?;
                let mut outputs = Vec::with_capacity(n);
                for _ in 0..n {
                    outputs.push(OutputInfo {
                        id: d.str()?,
                        name: d.str()?,
                        description: d.str()?,
                        x: d.i32()?,
                        y: d.i32()?,
                        width: d.i32()?,
                        height: d.i32()?,
                        scale: d.f64()?,
                        note: d.str()?,
                        default_scale: d.f64()?,
                        base_scale: d.f64()?,
                        shown_by: d.str()?,
                        drawn_by: d.str()?,
                        modes: {
                            let n = d.count(13)?;
                            let mut v = Vec::with_capacity(n);
                            for _ in 0..n {
                                v.push(ModeInfo { width: d.u32()?, height: d.u32()?, refresh_mhz: d.u32()?, preferred: d.bool()? });
                            }
                            v
                        },
                        mode: d.u32()?,
                        text_ratio: d.f64()?,
                    });
                }
                let setup = d.str()?;
                ServerMsg::Outputs { serial, outputs, setup }
            }
            op::SETTINGS => {
                let serial = d.u32()?;
                let complete = d.bool()?;
                // Each setting: two strings and a tagged value (at least 4 + 4 + 2 bytes).
                let n = d.count(10)?;
                let mut settings = Vec::with_capacity(n);
                for _ in 0..n {
                    settings.push(Setting { namespace: d.str()?, key: d.str()?, value: get_setting_value(&mut d)? });
                }
                ServerMsg::Settings { serial, complete, settings }
            }
            op::NOTIFIED => ServerMsg::Notified { serial: d.u32()?, id: d.u32()? },
            op::NOTIFICATION_CLOSED => ServerMsg::NotificationClosed { id: d.u32()?, reason: d.u32()? },
            op::ACTION_INVOKED => ServerMsg::ActionInvoked { id: d.u32()?, key: d.str()? },
            op::ALIVE => ServerMsg::Alive { serial: d.u32()? },
            op::CARD_OPENED => ServerMsg::CardOpened { id: d.str()? },
            op::CARD_CLOSED => ServerMsg::CardClosed { id: d.str()? },
            op::TILE_CLICKED => ServerMsg::TileClicked { id: d.str()? },
            op::TILE_ACTION => ServerMsg::TileAction { id: d.str()?, control: d.str()?, value: d.f64()? },
            op::APPS => {
                let serial = d.u32()?;
                let n = d.count(14)?;
                let mut apps = Vec::with_capacity(n);
                for _ in 0..n {
                    let (key, name, icon_side) = (d.str()?, d.str()?, d.u16()?);
                    let icon = d.bytes()?.to_vec();
                    if !icon.is_empty() && icon.len() != icon_side as usize * icon_side as usize * 4 {
                        return Err(WireError::BadValue("app icon"));
                    }
                    let responding = d.bool()?;
                    apps.push(AppInfo { key, name, icon_side, icon, responding, quitting: d.bool()? });
                }
                ServerMsg::Apps { serial, apps }
            }
            op::PLACES => {
                let serial = d.u32()?;
                let n = d.count(8)?;
                let mut apps = Vec::with_capacity(n);
                for _ in 0..n {
                    apps.push((d.str()?, d.u32()?));
                }
                ServerMsg::Places { serial, apps }
            }
            op::INPUTS => {
                let serial = d.u32()?;
                // Each device: a string (4 bytes at least) and its kind.
                let n = d.count(5)?;
                let mut devices = Vec::with_capacity(n);
                for _ in 0..n {
                    devices.push(InputInfo { name: d.str()?, kind: InputKind::from_wire(d.u8()?) });
                }
                let n = d.count(1)?;
                let mut seen = Vec::with_capacity(n);
                for _ in 0..n {
                    seen.push(InputKind::from_wire(d.u8()?));
                }
                ServerMsg::Inputs { serial, devices, seen }
            }
            other => return Err(WireError::UnknownOpcode(other)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::FrameReader;

    fn frame(bytes: &[u8]) -> Frame {
        let mut r = FrameReader::new();
        r.push(bytes);
        r.next_frame().unwrap().unwrap()
    }

    fn outputs() -> Vec<OutputInfo> {
        vec![
            OutputInfo { id: "3f2a9c0e1b7d4a55".into(), name: "eDP-1".into(), description: "14\" 4K".into(), x: 0, y: 0, width: 1170, height: 658, scale: 3.283_333, note: "calculated".into(), default_scale: 3.0, base_scale: 3.0, shown_by: "Intel graphics (i915)".into(), drawn_by: "Intel graphics (i915)".into(), modes: vec![ModeInfo { width: 3840, height: 2160, refresh_mhz: 60000, preferred: true }, ModeInfo { width: 1920, height: 1080, refresh_mhz: 59940, preferred: false }], mode: 0, text_ratio: 1.09 },
            OutputInfo { id: "9b1c22d4e5f60718".into(), name: "HDMI-A-1".into(), description: "ASUS VS229".into(), x: 1170, y: 0, width: 1794, height: 1009, scale: 1.075, note: "set by hand".into(), default_scale: 1.0, base_scale: 1.25, shown_by: "NVIDIA graphics (nouveau)".into(), drawn_by: "Intel graphics (i915)".into(), modes: Vec::new(), mode: u32::MAX, text_ratio: 0.86 },
        ]
    }

    fn sample_tile() -> crate::tile::Tile {
        use crate::tile::{Card, Control, ControlKind, Tile, TileState};
        Tile {
            id: "ups".into(),
            value: "23 m".into(),
            label: "UPS".into(),
            state: TileState::Warning,
            group: "Lab".into(),
            page: String::new(),
            card: Card {
                title: "UPS (rack)".into(),
                big: String::new(),
                rows: vec![("Load".into(), "27 %".into()), ("On".into(), "battery".into())],
                chart: vec![25.0, 26.5, 27.25],
                controls: vec![
                    Control { id: "test".into(), label: "Self-test".into(), kind: ControlKind::Button },
                    Control { id: "beep".into(), label: "Beeper".into(), kind: ControlKind::Switch(true) },
                    Control { id: "x".into(), label: "X".into(), kind: ControlKind::Slider { value: 0.5, min: 0.0, max: 1.0, step: 0.05 } },
                ],
                note: "sample".into(),
            },
        }
    }

    #[test]
    fn a_tile_is_checked_before_it_is_taken() {
        use crate::tile::{Control, ControlKind};
        assert!(sample_tile().check().is_ok());
        assert!(crate::tile::Tile::default().check().is_err(), "no id");
        let mut t = sample_tile();
        t.value = "x".repeat(25);
        assert!(t.check().is_err(), "a value too long for the dock");
        let mut t = sample_tile();
        t.card.chart.push(f32::NAN);
        assert!(t.check().is_err());
        let mut t = sample_tile();
        t.card.controls.push(Control { id: "s".into(), label: String::new(), kind: ControlKind::Slider { value: 0.0, min: 1.0, max: 1.0, step: 0.0 } });
        assert!(t.check().is_err(), "a slider with no range");
        let mut t = sample_tile();
        t.id = "a\nb".into();
        assert!(t.check().is_err());
    }

    #[test]
    fn client_messages_round_trip() {
        for m in [
            ClientMsg::Hello { version: VERSION, app_id: "vergence-settings".into() },
            ClientMsg::Ping { serial: 9 },
            ClientMsg::Subscribe { serial: 10, topic: Topic::Outputs },
            ClientMsg::Unsubscribe { serial: 11, topic: Topic::Outputs },
            ClientMsg::GetOutputs { serial: 12 },
            ClientMsg::GetSettings { serial: 13 },
            ClientMsg::Subscribe { serial: 14, topic: Topic::Settings },
            ClientMsg::Notify {
                serial: 15,
                replaces: 0,
                summary: "Build finished".into(),
                body: "vergence-server, 14 s".into(),
                urgency: 1,
                timeout_ms: -1,
                actions: vec![("default".into(), "Open".into()), ("log".into(), "Show log".into())],
            },
            ClientMsg::CloseNotification { serial: 16, id: 3 },
            ClientMsg::ListPlaces { serial: 17 },
            ClientMsg::ForgetPlaces { serial: 18, app: "vergence-term".into() },
            ClientMsg::ForgetPlaces { serial: 19, app: String::new() },
            ClientMsg::ListApps { serial: 22 },
            ClientMsg::ForceQuit { serial: 23, app: "firefox".into() },
            ClientMsg::Alive { serial: 25 },
            ClientMsg::Open { serial: 26, items: vec!["/home/u/a b.png".into(), "https://example.org".into()], app: String::new() },
            ClientMsg::Open { serial: 27, items: vec![], app: "firefox.desktop".into() },
            ClientMsg::SetTile { serial: 28, tile: crate::tile::Tile::default() },
            ClientMsg::SetTile { serial: 29, tile: sample_tile() },
            ClientMsg::RemoveTile { serial: 30, id: "ups".into() },
        ] {
            assert_eq!(ClientMsg::decode(&frame(&m.encode().unwrap())).unwrap(), m);
        }
    }

    #[test]
    fn server_messages_round_trip() {
        for m in [
            ServerMsg::Welcome { version: VERSION, client_id: 3 },
            ServerMsg::Pong { serial: 9 },
            ServerMsg::Done { serial: 10 },
            ServerMsg::Error { serial: 11, code: ErrorCode::UnknownTopic, message: "no topic 9".into() },
            ServerMsg::Error { serial: 12, code: ErrorCode::Other(77), message: String::new() },
            ServerMsg::Outputs { serial: 0, outputs: outputs(), setup: "ab12".into() },
            ServerMsg::Outputs { serial: 4, outputs: Vec::new(), setup: String::new() },
            ServerMsg::Notified { serial: 15, id: 3 },
            ServerMsg::NotificationClosed { id: 3, reason: 2 },
            ServerMsg::ActionInvoked { id: 3, key: "log".into() },
            ServerMsg::Inputs {
                serial: 0,
                devices: vec![
                    InputInfo { name: "AT Translated Set 2 keyboard".into(), kind: InputKind::Keyboard },
                    InputInfo { name: "ELAN0712:00 04F3:30FD Touchpad".into(), kind: InputKind::Touchpad },
                    InputInfo { name: "a pen".into(), kind: InputKind::Other(9) },
                ],
                seen: vec![InputKind::Keyboard, InputKind::Touchpad, InputKind::Touchscreen],
            },
            ServerMsg::Inputs { serial: 0, devices: Vec::new(), seen: Vec::new() },
            ServerMsg::Places { serial: 20, apps: vec![("vergence-term".into(), 3), ("firefox".into(), 1)] },
            ServerMsg::Places { serial: 21, apps: Vec::new() },
            ServerMsg::Alive { serial: 26 },
            ServerMsg::CardOpened { id: "ups".into() },
            ServerMsg::CardClosed { id: "ups".into() },
            ServerMsg::TileClicked { id: "ups".into() },
            ServerMsg::TileAction { id: "sound".into(), control: "volume".into(), value: 0.66 },
            ServerMsg::Apps {
                serial: 24,
                apps: vec![
                    AppInfo { key: "vergence-calc".into(), name: "Calculator".into(), icon_side: 2, icon: vec![9; 16], responding: true, quitting: false },
                    AppInfo { key: "firefox".into(), name: "Firefox".into(), icon_side: 0, icon: Vec::new(), responding: false, quitting: true },
                ],
            },
            ServerMsg::Settings {
                serial: 0,
                complete: true,
                settings: vec![
                    Setting { namespace: "org.freedesktop.appearance".into(), key: "color-scheme".into(), value: SettingValue::U32(1) },
                    Setting { namespace: "org.freedesktop.appearance".into(), key: "accent-color".into(), value: SettingValue::Rgb(0.2, 0.5, 1.0) },
                    Setting { namespace: "org.gnome.desktop.interface".into(), key: "font-name".into(), value: SettingValue::Str("Sans 10".into()) },
                    Setting { namespace: "n".into(), key: "b".into(), value: SettingValue::Bool(true) },
                    Setting { namespace: "n".into(), key: "i".into(), value: SettingValue::I32(-24) },
                    Setting { namespace: "n".into(), key: "d".into(), value: SettingValue::F64(1.25) },
                ],
            },
        ] {
            assert_eq!(ServerMsg::decode(&frame(&m.encode().unwrap())).unwrap(), m);
        }
    }

    #[test]
    fn unknown_requests_keep_their_serial() {
        // Opcode 99 from a newer client, serial 42.
        let mut e = Encoder::frame(99);
        e.u32(42).str("whatever");
        let u = ClientMsg::decode(&frame(&e.finish().unwrap())).unwrap_err();
        assert_eq!((u.serial, u.code), (42, ErrorCode::BadMessage));
        // Subscribe to topic 9, unknown here.
        let mut e = Encoder::frame(op::SUBSCRIBE);
        e.u32(43).u16(9);
        let u = ClientMsg::decode(&frame(&e.finish().unwrap())).unwrap_err();
        assert_eq!((u.serial, u.code), (43, ErrorCode::UnknownTopic));
    }

    #[test]
    fn newer_fields_at_the_end_are_ignored() {
        let mut e = Encoder::frame(op::PING);
        e.u32(5).str("a field from version 2");
        assert_eq!(ClientMsg::decode(&frame(&e.finish().unwrap())).unwrap(), ClientMsg::Ping { serial: 5 });
    }

    #[test]
    fn a_lying_output_count_is_rejected() {
        let mut e = Encoder::frame(op::OUTPUTS);
        e.u32(0).count(1000);
        assert!(ServerMsg::decode(&frame(&e.finish().unwrap())).is_err());
    }

    #[test]
    fn topics_by_name() {
        assert_eq!(Topic::from_name("outputs"), Some(Topic::Outputs));
        assert_eq!(Topic::from_name(Topic::Outputs.name()), Some(Topic::Outputs));
        assert_eq!(Topic::from_name("settings"), Some(Topic::Settings));
        assert_eq!(Topic::from_name("inputs"), Some(Topic::Inputs));
        assert_eq!(InputKind::from_name("touchpad"), Some(InputKind::Touchpad));
        assert_eq!(InputKind::from_name(InputKind::Touchscreen.name()), Some(InputKind::Touchscreen));
        assert_eq!(Topic::from_name("notifications"), None);
    }
}
