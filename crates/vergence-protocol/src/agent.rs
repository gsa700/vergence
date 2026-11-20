//! The agent interface (Contract 3, whitepaper §6.5; meaning milestone 1,
//! step 3; bus v26): an agent working for the person reads the windows'
//! meaning and acts on widgets by name, on the session bus, as any client.
//! It says who it is first ([`AgentRequest::Hello`]: a name the person will
//! see); the server may refuse it (agents are off until the person allows
//! them, step 4). Bounds in replies are on the screens, in logical points.
//! A secret field is a role and a name, never a value, and is never set.
//!
//! Opcodes 200 to 299, both ways (window.rs has 100 to 199).

use crate::window::{Action, Node, finite, get_action, get_nodes, put_action, put_nodes};
use crate::wire::{Decoder, Encoder, WireError};

/// Is this opcode the agent interface's?
pub fn is_agent_opcode(op: u16) -> bool {
    (200..300).contains(&op)
}

/// A rectangle on the screens: x, y, width, height (logical points).
pub type Rect4 = (f64, f64, f64, f64);

/// The most windows, found widgets and changes one reply carries.
pub const MAX_LISTED: usize = 512;
/// The longest agent name, and path.
pub const MAX_NAME: usize = 40;
pub const MAX_PATH: usize = 1024;

/// A window as an agent sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentWindow {
    /// The bus client and its window id (native windows); 0 and 0 for
    /// others, which an agent can only see, not read or act on.
    pub client: u32,
    pub window: u32,
    pub title: String,
    pub app_id: String,
    /// "native", "wayland" or "x11".
    pub kind: String,
    /// It has the keyboard.
    pub keyboard: bool,
    /// It has published its meaning (only native windows can).
    pub meaning: bool,
    pub at: Rect4,
    /// The screen it is mostly on (its name).
    pub screen: String,
}

/// A widget found, with its window.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentFound {
    pub client: u32,
    pub window: u32,
    pub title: String,
    /// Its bounds are on the screens.
    pub node: Node,
    pub screen: String,
}

/// What changed in the meaning, numbered (the server's log).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentChange {
    /// The keyboard went to this window (0, 0: one that is not native).
    Window { client: u32, window: u32 },
    Focus { client: u32, window: u32, node: u64 },
    Caret { client: u32, window: u32, at: Option<Rect4> },
    Tree { client: u32, window: u32 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentRequest {
    /// Who the agent is. Reply: Done, or an error (agents are off).
    Hello { serial: u32, name: String },
    /// Every window. Reply: [`AgentEvent::Windows`].
    Windows { serial: u32 },
    /// A native window's tree. Reply: [`AgentEvent::Tree`].
    Tree { serial: u32, client: u32, window: u32 },
    /// Widgets by path ("Calculator > 7", "* > button:Save"; the server's
    /// meaning.rs says how paths go). Reply: [`AgentEvent::Found`].
    Find { serial: u32, path: String },
    /// The focused widget and the caret. Reply: [`AgentEvent::Focused`].
    Focused { serial: u32 },
    /// An action on a widget. Reply: [`AgentEvent::Acted`] once the window answers.
    Act { serial: u32, client: u32, window: u32, node: u64, action: Action },
    /// The changes after number `since`. Reply: [`AgentEvent::Changes`].
    Changes { serial: u32, since: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    Windows { serial: u32, windows: Vec<AgentWindow> },
    Tree { serial: u32, client: u32, window: u32, title: String, focus: u64, caret: Option<Rect4>, nodes: Vec<Node> },
    Found { serial: u32, found: Vec<AgentFound> },
    /// Boxed: a found widget is large, and every bus message would carry its size.
    Focused { serial: u32, found: Option<Box<AgentFound>>, caret: Option<Rect4> },
    Acted { serial: u32, ok: bool, why: String },
    /// `latest`: the number to ask from next time.
    Changes { serial: u32, latest: u64, changes: Vec<(u64, AgentChange)> },
}

mod op {
    // Agent to server.
    pub const HELLO: u16 = 200;
    pub const WINDOWS: u16 = 201;
    pub const TREE: u16 = 202;
    pub const FIND: u16 = 203;
    pub const FOCUSED: u16 = 204;
    pub const ACT: u16 = 205;
    pub const CHANGES: u16 = 206;
    // Server to agent.
    pub const E_WINDOWS: u16 = 200;
    pub const E_TREE: u16 = 201;
    pub const E_FOUND: u16 = 202;
    pub const E_FOCUSED: u16 = 203;
    pub const E_ACTED: u16 = 204;
    pub const E_CHANGES: u16 = 205;
}

fn put_rect(e: &mut Encoder, r: &Rect4) {
    e.f64(r.0).f64(r.1).f64(r.2).f64(r.3);
}

fn get_rect(d: &mut Decoder) -> Result<Rect4, WireError> {
    Ok((finite(d.f64()?, "rect")?, finite(d.f64()?, "rect")?, finite(d.f64()?, "rect")?, finite(d.f64()?, "rect")?))
}

fn put_opt_rect(e: &mut Encoder, r: &Option<Rect4>) {
    match r {
        Some(r) => {
            e.bool(true);
            put_rect(e, r);
        }
        None => {
            e.bool(false);
        }
    }
}

fn get_opt_rect(d: &mut Decoder) -> Result<Option<Rect4>, WireError> {
    Ok(if d.bool()? { Some(get_rect(d)?) } else { None })
}

fn put_found(e: &mut Encoder, f: &AgentFound) {
    e.u32(f.client).u32(f.window).str(&f.title).str(&f.screen);
    put_nodes(e, std::slice::from_ref(&f.node));
}

fn get_found(d: &mut Decoder) -> Result<AgentFound, WireError> {
    let (client, window, title, screen) = (d.u32()?, d.u32()?, d.str()?, d.str()?);
    let node = get_nodes(d)?.into_iter().next().ok_or(WireError::BadValue("found node"))?;
    Ok(AgentFound { client, window, title, node, screen })
}

fn listed(d: &mut Decoder, min: usize) -> Result<usize, WireError> {
    let n = d.count(min)?;
    if n > MAX_LISTED {
        return Err(WireError::TooLong(n));
    }
    Ok(n)
}

impl AgentRequest {
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let e = match self {
            AgentRequest::Hello { serial, name } => {
                let mut e = Encoder::frame(op::HELLO);
                e.u32(*serial).str(name);
                e
            }
            AgentRequest::Windows { serial } => {
                let mut e = Encoder::frame(op::WINDOWS);
                e.u32(*serial);
                e
            }
            AgentRequest::Tree { serial, client, window } => {
                let mut e = Encoder::frame(op::TREE);
                e.u32(*serial).u32(*client).u32(*window);
                e
            }
            AgentRequest::Find { serial, path } => {
                let mut e = Encoder::frame(op::FIND);
                e.u32(*serial).str(path);
                e
            }
            AgentRequest::Focused { serial } => {
                let mut e = Encoder::frame(op::FOCUSED);
                e.u32(*serial);
                e
            }
            AgentRequest::Act { serial, client, window, node, action } => {
                let mut e = Encoder::frame(op::ACT);
                e.u32(*serial).u32(*client).u32(*window).u64(*node);
                put_action(&mut e, action);
                e
            }
            AgentRequest::Changes { serial, since } => {
                let mut e = Encoder::frame(op::CHANGES);
                e.u32(*serial).u64(*since);
                e
            }
        };
        e.finish()
    }

    /// The serial first, so an error can name it even when the rest is bad.
    pub fn decode(opcode: u16, payload: &[u8]) -> Result<AgentRequest, (u32, WireError)> {
        let mut d = Decoder::new(payload);
        let serial = d.u32().map_err(|e| (0, e))?;
        let rest = (|| -> Result<AgentRequest, WireError> {
            Ok(match opcode {
                op::HELLO => {
                    let name = d.str()?;
                    if name.chars().count() > MAX_NAME {
                        return Err(WireError::TooLong(name.len()));
                    }
                    AgentRequest::Hello { serial, name }
                }
                op::WINDOWS => AgentRequest::Windows { serial },
                op::TREE => AgentRequest::Tree { serial, client: d.u32()?, window: d.u32()? },
                op::FIND => {
                    let path = d.str()?;
                    if path.len() > MAX_PATH {
                        return Err(WireError::TooLong(path.len()));
                    }
                    AgentRequest::Find { serial, path }
                }
                op::FOCUSED => AgentRequest::Focused { serial },
                op::ACT => {
                    let (client, window, node) = (d.u32()?, d.u32()?, d.u64()?);
                    AgentRequest::Act { serial, client, window, node, action: get_action(&mut d)? }
                }
                op::CHANGES => AgentRequest::Changes { serial, since: d.u64()? },
                other => return Err(WireError::UnknownOpcode(other)),
            })
        })();
        rest.map_err(|e| (serial, e))
    }

    pub fn serial(&self) -> u32 {
        match self {
            AgentRequest::Hello { serial, .. }
            | AgentRequest::Windows { serial }
            | AgentRequest::Tree { serial, .. }
            | AgentRequest::Find { serial, .. }
            | AgentRequest::Focused { serial }
            | AgentRequest::Act { serial, .. }
            | AgentRequest::Changes { serial, .. } => *serial,
        }
    }
}

impl AgentEvent {
    /// The serial of the request it answers.
    pub fn serial(&self) -> u32 {
        match self {
            AgentEvent::Windows { serial, .. }
            | AgentEvent::Tree { serial, .. }
            | AgentEvent::Found { serial, .. }
            | AgentEvent::Focused { serial, .. }
            | AgentEvent::Acted { serial, .. }
            | AgentEvent::Changes { serial, .. } => *serial,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let e = match self {
            AgentEvent::Windows { serial, windows } => {
                let mut e = Encoder::frame(op::E_WINDOWS);
                e.u32(*serial).count(windows.len());
                for w in windows {
                    e.u32(w.client).u32(w.window).str(&w.title).str(&w.app_id).str(&w.kind).bool(w.keyboard).bool(w.meaning);
                    put_rect(&mut e, &w.at);
                    e.str(&w.screen);
                }
                e
            }
            AgentEvent::Tree { serial, client, window, title, focus, caret, nodes } => {
                let mut e = Encoder::frame(op::E_TREE);
                e.u32(*serial).u32(*client).u32(*window).str(title).u64(*focus);
                put_opt_rect(&mut e, caret);
                put_nodes(&mut e, nodes);
                e
            }
            AgentEvent::Found { serial, found } => {
                let mut e = Encoder::frame(op::E_FOUND);
                e.u32(*serial).count(found.len());
                for f in found {
                    put_found(&mut e, f);
                }
                e
            }
            AgentEvent::Focused { serial, found, caret } => {
                let mut e = Encoder::frame(op::E_FOCUSED);
                e.u32(*serial);
                match found {
                    Some(f) => {
                        e.bool(true);
                        put_found(&mut e, f);
                    }
                    None => {
                        e.bool(false);
                    }
                }
                put_opt_rect(&mut e, caret);
                e
            }
            AgentEvent::Acted { serial, ok, why } => {
                let mut e = Encoder::frame(op::E_ACTED);
                e.u32(*serial).bool(*ok).str(why);
                e
            }
            AgentEvent::Changes { serial, latest, changes } => {
                let mut e = Encoder::frame(op::E_CHANGES);
                e.u32(*serial).u64(*latest).count(changes.len());
                for (n, c) in changes {
                    e.u64(*n);
                    match c {
                        AgentChange::Window { client, window } => e.u8(0).u32(*client).u32(*window),
                        AgentChange::Focus { client, window, node } => e.u8(1).u32(*client).u32(*window).u64(*node),
                        AgentChange::Caret { client, window, at } => {
                            e.u8(2).u32(*client).u32(*window);
                            put_opt_rect(&mut e, at);
                            &mut e
                        }
                        AgentChange::Tree { client, window } => e.u8(3).u32(*client).u32(*window),
                    };
                }
                e
            }
        };
        e.finish()
    }

    pub fn decode(opcode: u16, payload: &[u8]) -> Result<AgentEvent, WireError> {
        let mut d = Decoder::new(payload);
        let serial = d.u32()?;
        Ok(match opcode {
            op::E_WINDOWS => {
                let n = listed(&mut d, 8 + 12 + 2 + 32 + 4)?;
                let mut windows = Vec::with_capacity(n);
                for _ in 0..n {
                    let (client, window, title, app_id, kind, keyboard, meaning) = (d.u32()?, d.u32()?, d.str()?, d.str()?, d.str()?, d.bool()?, d.bool()?);
                    let at = get_rect(&mut d)?;
                    windows.push(AgentWindow { client, window, title, app_id, kind, keyboard, meaning, at, screen: d.str()? });
                }
                AgentEvent::Windows { serial, windows }
            }
            op::E_TREE => {
                let (client, window, title, focus) = (d.u32()?, d.u32()?, d.str()?, d.u64()?);
                let caret = get_opt_rect(&mut d)?;
                AgentEvent::Tree { serial, client, window, title, focus, caret, nodes: get_nodes(&mut d)? }
            }
            op::E_FOUND => {
                let n = listed(&mut d, 8 + 8 + 4)?;
                let found = (0..n).map(|_| get_found(&mut d)).collect::<Result<_, _>>()?;
                AgentEvent::Found { serial, found }
            }
            op::E_FOCUSED => {
                let found = if d.bool()? { Some(Box::new(get_found(&mut d)?)) } else { None };
                AgentEvent::Focused { serial, found, caret: get_opt_rect(&mut d)? }
            }
            op::E_ACTED => AgentEvent::Acted { serial, ok: d.bool()?, why: d.str()? },
            op::E_CHANGES => {
                let latest = d.u64()?;
                let n = listed(&mut d, 8 + 9)?;
                let mut changes = Vec::with_capacity(n);
                for _ in 0..n {
                    let seq = d.u64()?;
                    let c = match d.u8()? {
                        0 => AgentChange::Window { client: d.u32()?, window: d.u32()? },
                        1 => AgentChange::Focus { client: d.u32()?, window: d.u32()?, node: d.u64()? },
                        2 => {
                            let (client, window) = (d.u32()?, d.u32()?);
                            AgentChange::Caret { client, window, at: get_opt_rect(&mut d)? }
                        }
                        3 => AgentChange::Tree { client: d.u32()?, window: d.u32()? },
                        _ => return Err(WireError::BadValue("change")),
                    };
                    changes.push((seq, c));
                }
                AgentEvent::Changes { serial, latest, changes }
            }
            other => return Err(WireError::UnknownOpcode(other)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> Node {
        Node { id: 9, parent: 0, role: "choice".into(), name: "Font".into(), value: Some("Inter".into()), states: vec!["focused".into()], x: 100.0, y: 40.0, w: 80.0, h: 20.0, items: vec!["Inter".into(), "Mono".into()] }
    }

    #[test]
    fn requests_round_trip() {
        for r in [
            AgentRequest::Hello { serial: 1, name: "Claude".into() },
            AgentRequest::Windows { serial: 2 },
            AgentRequest::Tree { serial: 3, client: 4, window: 1 },
            AgentRequest::Find { serial: 4, path: "Calculator > button:7".into() },
            AgentRequest::Focused { serial: 5 },
            AgentRequest::Act { serial: 6, client: 4, window: 1, node: 1 << 50, action: Action::Choose("Mono".into()) },
            AgentRequest::Changes { serial: 7, since: 123 },
        ] {
            let bytes = r.encode().unwrap();
            let (len, opcode) = (u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize, u16::from_le_bytes(bytes[4..6].try_into().unwrap()));
            assert!(is_agent_opcode(opcode));
            assert_eq!(AgentRequest::decode(opcode, &bytes[6..4 + len]).unwrap(), r);
        }
    }

    #[test]
    fn events_round_trip() {
        let f = AgentFound { client: 4, window: 1, title: "Windows".into(), node: node(), screen: "DP-1".into() };
        for ev in [
            AgentEvent::Windows { serial: 1, windows: vec![AgentWindow { client: 4, window: 1, title: "Windows".into(), app_id: "vergence-settings".into(), kind: "native".into(), keyboard: true, meaning: true, at: (10.0, 20.0, 800.0, 600.0), screen: "DP-1".into() }] },
            AgentEvent::Tree { serial: 2, client: 4, window: 1, title: "Windows".into(), focus: 9, caret: Some((1.0, 2.0, 1.0, 14.0)), nodes: vec![node()] },
            AgentEvent::Found { serial: 3, found: vec![f.clone(), f.clone()] },
            AgentEvent::Focused { serial: 4, found: Some(Box::new(f)), caret: None },
            AgentEvent::Focused { serial: 5, found: None, caret: None },
            AgentEvent::Acted { serial: 6, ok: false, why: "it is off".into() },
            AgentEvent::Changes {
                serial: 7,
                latest: 40,
                changes: vec![
                    (37, AgentChange::Window { client: 4, window: 1 }),
                    (38, AgentChange::Focus { client: 4, window: 1, node: 9 }),
                    (39, AgentChange::Caret { client: 4, window: 1, at: Some((5.0, 6.0, 1.0, 14.0)) }),
                    (40, AgentChange::Tree { client: 4, window: 1 }),
                ],
            },
        ] {
            let bytes = ev.encode().unwrap();
            let (len, opcode) = (u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize, u16::from_le_bytes(bytes[4..6].try_into().unwrap()));
            assert_eq!(AgentEvent::decode(opcode, &bytes[6..4 + len]).unwrap(), ev);
        }
    }

    #[test]
    fn damage_never_panics() {
        let ev = AgentEvent::Tree { serial: 2, client: 4, window: 1, title: "W".into(), focus: 9, caret: None, nodes: vec![node(); 3] }.encode().unwrap();
        let mut seed = 7u64;
        for i in 0..2000 {
            let mut b = ev[6..].to_vec();
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let at = (seed >> 33) as usize % b.len();
            b[at] = (seed >> 13) as u8;
            if i % 3 == 0 {
                b.truncate(at);
            }
            let _ = AgentEvent::decode(201, &b);
            let _ = AgentRequest::decode(200 + (i % 7) as u16, &b);
        }
    }
}
