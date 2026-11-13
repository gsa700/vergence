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
//! window's own Move and Resize requests are gone.

use crate::wire::{Decoder, Encoder, Frame, WireError};

/// The protocol version this crate speaks.
pub const VERSION: u32 = 2;

/// Something a client can subscribe to. Values are fixed on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    /// The monitors: their names, places and scales; sent again whenever
    /// one is added, removed, moved or rescaled.
    Outputs,
    /// The desktop settings (appearance, theme, fonts, cursor): all of them
    /// on subscribe, then the ones that changed.
    Settings,
}

impl Topic {
    pub fn to_wire(self) -> u16 {
        match self {
            Topic::Outputs => 1,
            Topic::Settings => 2,
        }
    }

    pub fn from_wire(v: u16) -> Option<Topic> {
        match v {
            1 => Some(Topic::Outputs),
            2 => Some(Topic::Settings),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Topic::Outputs => "outputs",
            Topic::Settings => "settings",
        }
    }

    pub fn from_name(s: &str) -> Option<Topic> {
        match s {
            "outputs" => Some(Topic::Outputs),
            "settings" => Some(Topic::Settings),
            _ => None,
        }
    }
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
    /// Windows (window.rs).
    Window(crate::window::WinRequest),
}

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
                    e.str(&o.id).str(&o.name).str(&o.description).i32(o.x).i32(o.y).i32(o.width).i32(o.height).f64(o.scale).str(&o.note).f64(o.default_scale).f64(o.base_scale);
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
                // Each output: four strings (4 bytes each at least), four i32, one f64.
                let n = d.count(4 + 4 + 4 + 16 + 8 + 4 + 16)?;
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
            OutputInfo { id: "3f2a9c0e1b7d4a55".into(), name: "eDP-1".into(), description: "14\" 4K".into(), x: 0, y: 0, width: 1170, height: 658, scale: 3.283_333, note: "calculated".into(), default_scale: 3.0, base_scale: 3.0 },
            OutputInfo { id: "9b1c22d4e5f60718".into(), name: "HDMI-A-1".into(), description: "ASUS VS229".into(), x: 1170, y: 0, width: 1794, height: 1009, scale: 1.075, note: "set by hand".into(), default_scale: 1.0, base_scale: 1.25 },
        ]
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
        assert_eq!(Topic::from_name("notifications"), None);
    }
}
