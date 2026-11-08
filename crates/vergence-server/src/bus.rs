// The session bus, core side (whitepaper §6.8; bus milestone 1, 2026-11-08).
//
// $XDG_RUNTIME_DIR/vergence-bus-<wayland socket>, a Unix socket in the user's
// private runtime directory, mode 0600, every connection's peer uid checked
// against ours. The server exports its path as VERGENCE_BUS to everything it
// starts. Messages and wire format: crates/vergence-protocol (bus.rs, wire.rs).
//
// Each connection is a calloop source: readable -> read every whole frame and
// answer it. Replies and events go into the connection's outbox and are
// written at once; what the socket won't take yet waits for a write source
// that is removed as soon as the outbox is empty. A client that lets more than
// OUTBOX_MAX pile up is cut off (shutdown; its read source then sees EOF and
// cleans up), so one stuck client can't grow the server's memory.

use std::{
    collections::{HashMap, HashSet},
    io::{ErrorKind, Read, Write},
    os::unix::{fs::PermissionsExt, net::{UnixListener, UnixStream}},
    path::PathBuf,
};

use smithay::reexports::calloop::{Interest, Mode, PostAction, RegistrationToken, generic::Generic};
use vergence_protocol::{
    bus::{ClientMsg, ErrorCode, OutputInfo, ServerMsg, Topic, VERSION},
    wire::FrameReader,
};

use crate::Server;

/// Most bytes a client may leave unread before it is disconnected.
const OUTBOX_MAX: usize = 1 << 20;

#[derive(Default)]
pub struct Bus {
    clients: HashMap<u32, Conn>,
    next_id: u32,
    /// The monitors as last sent, so unchanged layouts aren't re-sent.
    last_outputs: Option<Vec<OutputInfo>>,
    pub path: Option<PathBuf>,
}

struct Conn {
    stream: UnixStream,
    reader: FrameReader,
    /// Set by Hello: the app id the client gave (recorded, not trusted, §7.4).
    app_id: Option<String>,
    uid: u32,
    topics: HashSet<Topic>,
    outbox: Vec<u8>,
    write_source: Option<RegistrationToken>,
    /// Shut down after a protocol error or overflow; its read source removes it.
    closing: bool,
}

/// Is something answering on this socket path (a live server)?
pub fn socket_is_live(path: &std::path::Path) -> bool {
    UnixStream::connect(path).is_ok()
}

pub fn listen(state: &mut Server) {
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) else {
        return tracing::warn!("bus: no XDG_RUNTIME_DIR");
    };
    let path = vergence_protocol::bus_socket_for(&runtime, &state.socket_name.to_string_lossy());
    if socket_is_live(&path) {
        return tracing::warn!("bus: {} belongs to a running server; no bus here", path.display());
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => return tracing::warn!("bus: {}: {e}", path.display()),
    };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    let _ = listener.set_nonblocking(true);
    let res = state.loop_handle.insert_source(Generic::new(listener, Interest::READ, Mode::Level), |_, l, state| {
        while let Ok((stream, _)) = l.accept() {
            state.bus_accept(stream);
        }
        Ok(PostAction::Continue)
    });
    match res {
        Ok(_) => {
            tracing::info!("bus {}", path.display());
            // Everything the server starts from now on finds the bus.
            unsafe { std::env::set_var(vergence_protocol::BUS_ENV, &path) };
            state.bus.path = Some(path);
        }
        Err(e) => tracing::warn!("bus: {e}"),
    }
}

impl Server {
    fn bus_accept(&mut self, stream: UnixStream) {
        let ours = crate::control::our_uid();
        let uid = match crate::control::peer_uid(&stream) {
            Some(uid) if uid == ours => uid,
            other => return tracing::warn!("bus: refused a connection from uid {other:?}"),
        };
        if stream.set_nonblocking(true).is_err() {
            return;
        }
        let Ok(write_half) = stream.try_clone() else { return };
        self.bus.next_id = self.bus.next_id.wrapping_add(1).max(1);
        let id = self.bus.next_id;
        let res = self.loop_handle.insert_source(Generic::new(stream, Interest::READ, Mode::Level), move |_, s, state| {
            Ok(if state.bus_readable(id, s) { PostAction::Continue } else { PostAction::Remove })
        });
        if let Err(e) = res {
            return tracing::warn!("bus: {e}");
        }
        self.bus.clients.insert(
            id,
            Conn {
                stream: write_half,
                reader: FrameReader::new(),
                app_id: None,
                uid,
                topics: HashSet::new(),
                outbox: Vec::new(),
                write_source: None,
                closing: false,
            },
        );
        tracing::debug!("bus: client {id} connected");
    }

    /// Read what has arrived and answer it; false when the connection is
    /// finished (its source is then removed).
    fn bus_readable(&mut self, id: u32, mut stream: &UnixStream) -> bool {
        let mut buf = [0u8; 8192];
        let mut eof = false;
        loop {
            match stream.read(&mut buf) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => {
                    let Some(c) = self.bus.clients.get_mut(&id) else { return false };
                    c.reader.push(&buf[..n]);
                    if c.reader.pending() > OUTBOX_MAX {
                        tracing::warn!("bus: client {id} sent an oversized burst");
                        eof = true;
                        break;
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => {
                    eof = true;
                    break;
                }
            }
        }
        // Answer every whole frame, even when the client has already hung up.
        loop {
            let Some(c) = self.bus.clients.get_mut(&id) else { return false };
            if c.closing {
                break;
            }
            match c.reader.next_frame() {
                Ok(Some(frame)) => match ClientMsg::decode(&frame) {
                    Ok(m) => self.bus_handle(id, m),
                    Err(u) => {
                        let hello = self.bus.clients.get(&id).is_some_and(|c| c.app_id.is_some());
                        let code = if hello { u.code } else { ErrorCode::NoHello };
                        self.bus_send(id, ServerMsg::Error { serial: u.serial, code, message: u.error.to_string() });
                        if !hello {
                            self.bus_close(id);
                        }
                    }
                },
                Ok(None) => break,
                Err(e) => {
                    tracing::warn!("bus: client {id}: {e}; disconnecting");
                    eof = true;
                    break;
                }
            }
        }
        if eof {
            self.bus_forget(id);
            return false;
        }
        true
    }

    fn bus_handle(&mut self, id: u32, m: ClientMsg) {
        let Some(c) = self.bus.clients.get_mut(&id) else { return };
        let hello = c.app_id.is_some();
        match m {
            ClientMsg::Hello { version, app_id } => {
                if hello {
                    return self.bus_send(id, ServerMsg::Error { serial: 0, code: ErrorCode::BadMessage, message: "second Hello".into() });
                }
                if version != VERSION {
                    self.bus_send(id, ServerMsg::Error {
                        serial: 0,
                        code: ErrorCode::Version,
                        message: format!("this server speaks version {VERSION}"),
                    });
                    return self.bus_close(id);
                }
                tracing::info!("bus: client {id} is {app_id:?} (uid {})", c.uid);
                c.app_id = Some(app_id);
                self.bus_send(id, ServerMsg::Welcome { version: VERSION, client_id: id });
            }
            _ if !hello => {
                self.bus_send(id, ServerMsg::Error { serial: 0, code: ErrorCode::NoHello, message: "say Hello first".into() });
                self.bus_close(id);
            }
            ClientMsg::Ping { serial } => self.bus_send(id, ServerMsg::Pong { serial }),
            ClientMsg::GetOutputs { serial } => {
                let outputs = self.bus_outputs();
                self.bus_send(id, ServerMsg::Outputs { serial, outputs, setup: self.memory.setup.clone() });
            }
            ClientMsg::Notify { serial, replaces, summary, body, urgency, timeout_ms, actions } => {
                let app = c.app_id.clone().unwrap_or_default();
                let req = crate::notify::Request { app, replaces, summary, body, urgency, timeout_ms, actions };
                let nid = self.notify(crate::notify::Origin::Bus(id), req);
                self.bus_send(id, ServerMsg::Notified { serial, id: nid });
            }
            ClientMsg::CloseNotification { serial, id: nid } => {
                if self.notify_close(crate::notify::Origin::Bus(id), nid) {
                    self.bus_send(id, ServerMsg::Done { serial });
                } else {
                    self.bus_send(id, ServerMsg::Error { serial, code: ErrorCode::Denied, message: format!("no notification {nid} of yours") });
                }
            }
            ClientMsg::GetSettings { serial } => {
                let settings = crate::settings::all(&self.settings.current);
                self.bus_send(id, ServerMsg::Settings { serial, complete: true, settings });
            }
            ClientMsg::Subscribe { serial, topic } => {
                c.topics.insert(topic);
                self.bus_send(id, ServerMsg::Done { serial });
                // The topic's state now, so the client never has to ask separately.
                match topic {
                    Topic::Outputs => {
                        let outputs = self.bus_outputs();
                        self.bus_send(id, ServerMsg::Outputs { serial: 0, outputs, setup: self.memory.setup.clone() });
                    }
                    Topic::Settings => {
                        let settings = crate::settings::all(&self.settings.current);
                        self.bus_send(id, ServerMsg::Settings { serial: 0, complete: true, settings });
                    }
                }
            }
            ClientMsg::Unsubscribe { serial, topic } => {
                c.topics.remove(&topic);
                self.bus_send(id, ServerMsg::Done { serial });
            }
        }
    }

    /// Queue a message for a client and write what the socket takes now.
    fn bus_send(&mut self, id: u32, m: ServerMsg) {
        let bytes = match m.encode() {
            Ok(b) => b,
            Err(e) => return tracing::warn!("bus: can't encode {m:?}: {e}"),
        };
        let Some(c) = self.bus.clients.get_mut(&id) else { return };
        if c.closing {
            return;
        }
        if c.outbox.len() + bytes.len() > OUTBOX_MAX {
            tracing::warn!("bus: client {id} ({:?}) isn't reading; disconnecting", c.app_id);
            return self.bus_close(id);
        }
        c.outbox.extend_from_slice(&bytes);
        if !self.bus_flush(id) {
            self.bus_watch_writable(id);
        }
    }

    /// Write as much of the outbox as the socket takes; true when it is empty.
    fn bus_flush(&mut self, id: u32) -> bool {
        let Some(c) = self.bus.clients.get_mut(&id) else { return true };
        while !c.outbox.is_empty() {
            match (&c.stream).write(&c.outbox) {
                Ok(0) => break,
                Ok(n) => {
                    c.outbox.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    // The client is gone; its read source sees EOF and cleans up.
                    c.outbox.clear();
                    c.closing = true;
                    break;
                }
            }
        }
        c.outbox.is_empty()
    }

    fn bus_watch_writable(&mut self, id: u32) {
        let Some(c) = self.bus.clients.get_mut(&id) else { return };
        if c.write_source.is_some() {
            return;
        }
        let Ok(half) = c.stream.try_clone() else { return self.bus_close(id) };
        let res = self.loop_handle.insert_source(Generic::new(half, Interest::WRITE, Mode::Level), move |_, _, state| {
            if state.bus_flush(id) {
                if let Some(c) = state.bus.clients.get_mut(&id) {
                    c.write_source = None;
                }
                Ok(PostAction::Remove)
            } else {
                Ok(PostAction::Continue)
            }
        });
        match res {
            Ok(token) => c.write_source = Some(token),
            Err(e) => {
                tracing::warn!("bus: {e}");
                self.bus_close(id);
            }
        }
    }

    /// End a connection after its last queued message: shut it down so its
    /// read source sees EOF and removes it.
    fn bus_close(&mut self, id: u32) {
        let _ = self.bus_flush(id);
        if let Some(c) = self.bus.clients.get_mut(&id) {
            c.closing = true;
            let _ = c.stream.shutdown(std::net::Shutdown::Both);
        }
    }

    /// The connection is finished: drop its state and its write source.
    fn bus_forget(&mut self, id: u32) {
        if let Some(c) = self.bus.clients.remove(&id) {
            if let Some(t) = c.write_source {
                self.loop_handle.remove(t);
            }
            tracing::debug!("bus: client {id} ({:?}) left", c.app_id);
        }
    }

    /// The monitors as the bus describes them, left to right.
    fn bus_outputs(&self) -> Vec<OutputInfo> {
        let mut out: Vec<OutputInfo> = self
            .space
            .outputs()
            .filter_map(|o| {
                let g = self.space.output_geometry(o)?;
                Some(OutputInfo {
                    id: self.monitor_id(&o.name()),
                    name: o.name(),
                    description: o.physical_properties().model,
                    x: g.loc.x,
                    y: g.loc.y,
                    width: g.size.w,
                    height: g.size.h,
                    scale: o.current_scale().fractional_scale(),
                    note: self.scale_notes.get(&o.name()).map(|s| s.how.clone()).unwrap_or_default(),
                    default_scale: self.scale_notes.get(&o.name()).map(|s| s.default).unwrap_or(1.0),
                    base_scale: self.scale_notes.get(&o.name()).map(|s| s.base).unwrap_or(1.0),
                })
            })
            .collect();
        out.sort_by_key(|o| (o.x, o.y));
        out
    }

    pub fn bus_notification_closed(&mut self, client: u32, id: u32, reason: u32) {
        self.bus_send(client, ServerMsg::NotificationClosed { id, reason });
    }

    pub fn bus_action_invoked(&mut self, client: u32, id: u32, key: &str) {
        self.bus_send(client, ServerMsg::ActionInvoked { id, key: key.into() });
    }

    /// Settings changed (settings.rs): subscribers get the changed ones.
    pub fn bus_settings_changed(&mut self, changed: &[vergence_protocol::bus::Setting]) {
        let ids: Vec<u32> = self.bus.clients.iter().filter(|(_, c)| c.topics.contains(&Topic::Settings)).map(|(id, _)| *id).collect();
        for id in ids {
            self.bus_send(id, ServerMsg::Settings { serial: 0, complete: false, settings: changed.to_vec() });
        }
    }

    /// Call after monitors were added, removed, moved or rescaled: subscribers
    /// get the new list (once, and only if it changed).
    pub fn bus_outputs_changed(&mut self) {
        let outputs = self.bus_outputs();
        if self.bus.last_outputs.as_ref() == Some(&outputs) {
            return;
        }
        self.bus.last_outputs = Some(outputs.clone());
        let ids: Vec<u32> = self.bus.clients.iter().filter(|(_, c)| c.topics.contains(&Topic::Outputs)).map(|(id, _)| *id).collect();
        for id in ids {
            self.bus_send(id, ServerMsg::Outputs { serial: 0, outputs: outputs.clone(), setup: self.memory.setup.clone() });
        }
    }
}
