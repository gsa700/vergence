//! The session bus (whitepaper §6.8): the messages between the server and a
//! client, version 1.
//!
//! A client connects to the socket named by `VERGENCE_BUS` (see
//! [`crate::socket_path`]), sends [`ClientMsg::Hello`] first, and gets
//! [`ServerMsg::Welcome`]. Requests carry a `serial` the client picks; the
//! reply to a request carries the same serial. Events the client subscribed
//! to arrive with serial 0.
//!
//! Version 1 has the bus itself and one topic, the monitors
//! ([`Topic::Outputs`]). Settings and notifications come next.

use crate::wire::{Decoder, Encoder, Frame, WireError};

/// The protocol version this crate speaks.
pub const VERSION: u32 = 1;

/// Something a client can subscribe to. Values are fixed on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    /// The monitors: their names, places and scales; sent again whenever
    /// one is added, removed, moved or rescaled.
    Outputs,
}

impl Topic {
    pub fn to_wire(self) -> u16 {
        match self {
            Topic::Outputs => 1,
        }
    }

    pub fn from_wire(v: u16) -> Option<Topic> {
        match v {
            1 => Some(Topic::Outputs),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Topic::Outputs => "outputs",
        }
    }

    pub fn from_name(s: &str) -> Option<Topic> {
        match s {
            "outputs" => Some(Topic::Outputs),
            _ => None,
        }
    }
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
            v => ErrorCode::Other(v),
        }
    }
}

/// One monitor, as the server shows it. Geometry is in the desktop's
/// logical coordinates (Contract 1's unit is still to come: today these are
/// the server's logical pixels).
#[derive(Debug, Clone, PartialEq)]
pub struct OutputInfo {
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
}

/// Server to client.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMsg {
    Welcome { version: u32, client_id: u32 },
    Pong { serial: u32 },
    /// A request that has no other reply succeeded.
    Done { serial: u32 },
    Error { serial: u32, code: ErrorCode, message: String },
    /// The monitors: a reply (serial of the request) or an event (serial 0).
    Outputs { serial: u32, outputs: Vec<OutputInfo> },
}

mod op {
    // Client to server.
    pub const HELLO: u16 = 1;
    pub const PING: u16 = 2;
    pub const SUBSCRIBE: u16 = 3;
    pub const UNSUBSCRIBE: u16 = 4;
    pub const GET_OUTPUTS: u16 = 5;
    // Server to client.
    pub const WELCOME: u16 = 1;
    pub const PONG: u16 = 2;
    pub const DONE: u16 = 3;
    pub const ERROR: u16 = 4;
    pub const OUTPUTS: u16 = 5;
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
        };
        e.finish()
    }

    pub fn decode(frame: &Frame) -> Result<ClientMsg, Unknown> {
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
            ServerMsg::Outputs { serial, outputs } => {
                let mut e = Encoder::frame(op::OUTPUTS);
                e.u32(*serial).count(outputs.len());
                for o in outputs {
                    e.str(&o.name).str(&o.description).i32(o.x).i32(o.y).i32(o.width).i32(o.height).f64(o.scale);
                }
                e
            }
        };
        e.finish()
    }

    pub fn decode(frame: &Frame) -> Result<ServerMsg, WireError> {
        let mut d = Decoder::new(&frame.payload);
        Ok(match frame.opcode {
            op::WELCOME => ServerMsg::Welcome { version: d.u32()?, client_id: d.u32()? },
            op::PONG => ServerMsg::Pong { serial: d.u32()? },
            op::DONE => ServerMsg::Done { serial: d.u32()? },
            op::ERROR => ServerMsg::Error { serial: d.u32()?, code: ErrorCode::from_wire(d.u16()?), message: d.str()? },
            op::OUTPUTS => {
                let serial = d.u32()?;
                // Each output: two strings (4 bytes each at least), four i32, one f64.
                let n = d.count(4 + 4 + 16 + 8)?;
                let mut outputs = Vec::with_capacity(n);
                for _ in 0..n {
                    outputs.push(OutputInfo {
                        name: d.str()?,
                        description: d.str()?,
                        x: d.i32()?,
                        y: d.i32()?,
                        width: d.i32()?,
                        height: d.i32()?,
                        scale: d.f64()?,
                    });
                }
                ServerMsg::Outputs { serial, outputs }
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
            OutputInfo { name: "eDP-1".into(), description: "14\" 4K".into(), x: 0, y: 0, width: 1170, height: 658, scale: 3.283_333 },
            OutputInfo { name: "HDMI-A-1".into(), description: "ASUS VS229".into(), x: 1170, y: 0, width: 1794, height: 1009, scale: 1.075 },
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
            ServerMsg::Outputs { serial: 0, outputs: outputs() },
            ServerMsg::Outputs { serial: 4, outputs: Vec::new() },
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
        assert_eq!(Topic::from_name("settings"), None);
    }
}
