// The session bus's D-Bus edge (whitepaper §6.8; bus milestone 1 step 2).
//
// Existing apps expect a D-Bus session bus. The server provides one, written
// by the project (crates/vergence-dbus for the wire protocol), at
// $XDG_RUNTIME_DIR/vergence-dbus-<wayland socket>, exported as
// DBUS_SESSION_BUS_ADDRESS to everything it starts.
//
// Step 2a, this file: a working minimal bus that logs. It authenticates
// (EXTERNAL, peer uid checked), implements org.freedesktop.DBus (Hello,
// names with queueing, match rules, the usual queries), routes method calls
// and replies to names a connection owns, delivers signals by match rule
// (no eavesdropping), and answers calls to names nobody owns with
// ServiceUnknown, never starting anything. Every call is counted per
// destination and method: `vctl dbus` shows the table, which decides what the
// edge translates into core services first (whitepaper §10, question 13).
//
// Step 2c: the edge itself provides org.freedesktop.portal.Desktop's Settings
// interface (ReadAll, Read, ReadOne, SettingChanged, version 2) from the
// server's settings service (settings.rs), as the pseudo-connection ":1.0".
// FileChooser and OpenURI (files milestone 1) are in portal.rs. Other
// portal interfaces answer UnknownMethod, so apps fall back as before.
//
// Step 3: org.freedesktop.Notifications (same pseudo-connection) translated
// into the server's own notifications (notify.rs). NotificationClosed and
// ActionInvoked go only to the connection that made the notification.
//
// VERGENCE_DBUS=0 turns the edge off. Otherwise it takes over
// DBUS_SESSION_BUS_ADDRESS when none is set, or always with VERGENCE_DBUS=1.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    io::{ErrorKind, Read, Write},
    os::unix::{fs::PermissionsExt, net::{UnixListener, UnixStream}},
    path::PathBuf,
};

use smithay::reexports::calloop::{Interest, Mode, PostAction, RegistrationToken, channel, generic::Generic};
use vergence_dbus::{
    BUS_NAME, BUS_PATH, Message, Value,
    auth::{Auth, Step},
    matchrule::MatchRule,
    message::{self, METHOD_CALL, NO_REPLY_EXPECTED, SIGNAL},
};

use crate::Server;

/// Most bytes queued for a client before it is cut off.
const OUTBOX_MAX: usize = 32 << 20;
/// Most match rules per connection.
const MAX_RULES: usize = 512;

const E_SERVICE_UNKNOWN: &str = "org.freedesktop.DBus.Error.ServiceUnknown";
const E_NAME_HAS_NO_OWNER: &str = "org.freedesktop.DBus.Error.NameHasNoOwner";
const E_UNKNOWN_METHOD: &str = "org.freedesktop.DBus.Error.UnknownMethod";
const E_INVALID_ARGS: &str = "org.freedesktop.DBus.Error.InvalidArgs";
const E_MATCH_INVALID: &str = "org.freedesktop.DBus.Error.MatchRuleInvalid";
const E_MATCH_NOT_FOUND: &str = "org.freedesktop.DBus.Error.MatchRuleNotFound";
const E_ACCESS_DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";
const E_LIMITS: &str = "org.freedesktop.DBus.Error.LimitsExceeded";

/// The services the edge provides itself, as the pseudo-connection 0.
const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_SETTINGS: &str = "org.freedesktop.portal.Settings";
const NOTIFY_NAME: &str = "org.freedesktop.Notifications";
const NOTIFY_PATH: &str = "/org/freedesktop/Notifications";
const EDGE_UNIQUE: &str = ":1.0";
const EDGE: u32 = 0;

// RequestName flags and replies.
const ALLOW_REPLACEMENT: u32 = 1;
const REPLACE_EXISTING: u32 = 2;
const DO_NOT_QUEUE: u32 = 4;

#[derive(Default)]
pub struct DBus {
    conns: HashMap<u32, Conn>,
    next_id: u32,
    /// Well-known names: the owner first, then the queue (connection, flags).
    names: HashMap<String, VecDeque<(u32, u32)>>,
    guid: String,
    serial: u32,
    pub path: Option<PathBuf>,
    /// Calls seen: "destination interface.member" -> (count, answered by an owner).
    pub stats: BTreeMap<String, (u64, bool)>,
    /// Program names, looked up off the main thread (ps takes a few ms).
    names_tx: Option<channel::Sender<(u32, String)>>,
    /// Settings already read through the portal, to log each one once.
    reads: HashSet<(String, String)>,
}

struct Conn {
    stream: UnixStream,
    auth: Option<Auth>,
    inbuf: Vec<u8>,
    /// ":1.N" once Hello is done.
    unique: Option<String>,
    uid: u32,
    pid: Option<u32>,
    /// The program, for the log (from the pid).
    program: String,
    rules: Vec<MatchRule>,
    outbox: Vec<u8>,
    write_source: Option<RegistrationToken>,
    closing: bool,
}

pub fn listen(state: &mut Server) {
    let mode = std::env::var("VERGENCE_DBUS").unwrap_or_default();
    if mode == "0" {
        return tracing::info!("dbus: edge off (VERGENCE_DBUS=0)");
    }
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) else {
        return tracing::warn!("dbus: no XDG_RUNTIME_DIR");
    };
    let path = runtime.join(format!("vergence-dbus-{}", state.socket_name.to_string_lossy()));
    if crate::bus::socket_is_live(&path) {
        return tracing::warn!("dbus: {} belongs to a running server; no edge here", path.display());
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => return tracing::warn!("dbus: {}: {e}", path.display()),
    };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    let _ = listener.set_nonblocking(true);
    let res = state.loop_handle.insert_source(Generic::new(listener, Interest::READ, Mode::Level), |_, l, state| {
        while let Ok((stream, _)) = l.accept() {
            state.dbus_accept(stream);
        }
        Ok(PostAction::Continue)
    });
    if let Err(e) = res {
        return tracing::warn!("dbus: {e}");
    }
    let (tx, rx) = channel::channel::<(u32, String)>();
    let res = state.loop_handle.insert_source(rx, |event, _, state| {
        if let channel::Event::Msg((id, program)) = event {
            state.dbus_program_known(id, program);
        }
    });
    if let Err(e) = res {
        tracing::warn!("dbus: {e}");
    }
    state.dbus.names_tx = Some(tx);
    state.dbus.guid = make_guid();
    let existing = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().filter(|a| !a.is_empty());
    if existing.is_none() || mode == "1" {
        unsafe { std::env::set_var("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", path.display())) };
        tracing::info!("dbus: edge {} is the session bus", path.display());
    } else {
        tracing::info!("dbus: edge {} (the session bus stays {:?}; VERGENCE_DBUS=1 takes over)", path.display(), existing);
    }
    state.dbus.path = Some(path);
}

/// 32 hex digits, different for every server start.
fn make_guid() -> String {
    let mut b = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut b);
    }
    vergence_dbus::auth::encode_hex(&b)
}

/// The connecting process's pid: LOCAL_PEERCRED on FreeBSD, SO_PEERCRED on Linux.
fn peer_pid(stream: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn getsockopt(fd: i32, level: i32, name: i32, val: *mut u8, len: *mut u32) -> i32;
    }
    #[cfg(target_os = "freebsd")]
    {
        // struct xucred: version u32, uid u32, ngroups i16, groups [u32; 16], then
        // (8-aligned) a union holding cr_pid: 88 bytes on 64-bit.
        let mut buf = [0u8; 88];
        let mut len = buf.len() as u32;
        let ok = unsafe { getsockopt(stream.as_raw_fd(), 0, 1, buf.as_mut_ptr(), &mut len) } == 0;
        (ok && len as usize >= 84).then(|| i32::from_ne_bytes(buf[80..84].try_into().unwrap()) as u32).filter(|p| *p > 0)
    }
    #[cfg(target_os = "linux")]
    {
        // struct ucred { pid, uid, gid }; SOL_SOCKET 1, SO_PEERCRED 17.
        let mut buf = [0u8; 12];
        let mut len = buf.len() as u32;
        let ok = unsafe { getsockopt(stream.as_raw_fd(), 1, 17, buf.as_mut_ptr(), &mut len) } == 0;
        (ok && len == 12).then(|| i32::from_ne_bytes(buf[0..4].try_into().unwrap()) as u32).filter(|p| *p > 0)
    }
    #[cfg(not(any(target_os = "freebsd", target_os = "linux")))]
    {
        let _ = stream;
        None
    }
}

/// The program name for a pid, for the log.
fn program_of(pid: Option<u32>) -> String {
    let Some(pid) = pid else { return "?".into() };
    #[cfg(target_os = "linux")]
    if let Ok(s) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        return s.trim().to_string();
    }
    std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "?".into())
}

impl Server {
    fn dbus_accept(&mut self, stream: UnixStream) {
        let ours = crate::control::our_uid();
        let uid = match crate::control::peer_uid(&stream) {
            Some(uid) if uid == ours => uid,
            other => return tracing::warn!("dbus: refused a connection from uid {other:?}"),
        };
        let pid = peer_pid(&stream);
        if stream.set_nonblocking(true).is_err() {
            return;
        }
        let Ok(write_half) = stream.try_clone() else { return };
        self.dbus.next_id = self.dbus.next_id.wrapping_add(1).max(1);
        let id = self.dbus.next_id;
        let res = self.loop_handle.insert_source(Generic::new(stream, Interest::READ, Mode::Level), move |_, s, state| {
            Ok(if state.dbus_readable(id, s) { PostAction::Continue } else { PostAction::Remove })
        });
        if let Err(e) = res {
            return tracing::warn!("dbus: {e}");
        }
        let guid = self.dbus.guid.clone();
        if let Some(tx) = self.dbus.names_tx.clone() {
            std::thread::spawn(move || {
                let _ = tx.send((id, program_of(pid)));
            });
        }
        self.dbus.conns.insert(
            id,
            Conn {
                stream: write_half,
                auth: Some(Auth::new(uid, &guid)),
                inbuf: Vec::new(),
                unique: None,
                uid,
                pid,
                program: String::new(),
                rules: Vec::new(),
                outbox: Vec::new(),
                write_source: None,
                closing: false,
            },
        );
    }

    fn dbus_readable(&mut self, id: u32, mut stream: &UnixStream) -> bool {
        let mut buf = [0u8; 16384];
        let mut eof = false;
        loop {
            match stream.read(&mut buf) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => self.dbus_bytes(id, &buf[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => {
                    eof = true;
                    break;
                }
            }
            if self.dbus.conns.get(&id).is_none_or(|c| c.closing) {
                break;
            }
        }
        if eof || self.dbus.conns.get(&id).is_none_or(|c| c.closing) {
            self.dbus_forget(id);
            return false;
        }
        true
    }

    fn dbus_bytes(&mut self, id: u32, data: &[u8]) {
        let Some(c) = self.dbus.conns.get_mut(&id) else { return };
        if let Some(auth) = c.auth.as_mut() {
            match auth.feed(data) {
                Step::Continue(reply) => {
                    self.dbus_send_raw(id, &reply);
                    return;
                }
                Step::Done(reply, rest) => {
                    c.auth = None;
                    c.inbuf = rest;
                    self.dbus_send_raw(id, &reply);
                }
                Step::Fail => {
                    tracing::warn!("dbus: authentication failed for pid {:?}", c.pid);
                    c.closing = true;
                    return;
                }
            }
        } else {
            if c.inbuf.len() + data.len() > message::MAX_MESSAGE * 2 {
                tracing::warn!("dbus: {} sent too much at once", c.program);
                c.closing = true;
                return;
            }
            c.inbuf.extend_from_slice(data);
        }
        loop {
            let Some(c) = self.dbus.conns.get_mut(&id) else { return };
            if c.closing {
                return;
            }
            let total = match message::frame_len(&c.inbuf) {
                Ok(Some(t)) if c.inbuf.len() >= t => t,
                Ok(_) => return,
                Err(e) => {
                    tracing::warn!("dbus: {} sent a bad message ({e}); disconnecting", c.program);
                    c.closing = true;
                    return;
                }
            };
            let bytes: Vec<u8> = c.inbuf.drain(..total).collect();
            match Message::parse(&bytes) {
                Ok(m) => self.dbus_message(id, m),
                Err(e) => {
                    tracing::warn!("dbus: {} sent a bad message ({e}); disconnecting", c.program);
                    c.closing = true;
                    return;
                }
            }
        }
    }

    fn dbus_message(&mut self, id: u32, mut m: Message) {
        let Some(c) = self.dbus.conns.get(&id) else { return };
        let to_bus = m.destination.as_deref() == Some(BUS_NAME);
        let Some(unique) = c.unique.clone() else {
            if to_bus && m.kind == METHOD_CALL && m.member.as_deref() == Some("Hello") {
                return self.dbus_hello(id, &m);
            }
            tracing::warn!("dbus: {} sent {} before Hello; disconnecting", c.program, m.method());
            if let Some(c) = self.dbus.conns.get_mut(&id) {
                c.closing = true;
            }
            return;
        };
        // The bus says who sent it; a client can't claim another name.
        m.sender = Some(unique.clone());

        if m.kind == METHOD_CALL {
            let dest = m.destination.clone().unwrap_or_default();
            let owner = if to_bus { None } else { self.dbus_owner(&dest) };
            let key = format!("{dest} {}", m.method());
            let entry = self.dbus.stats.entry(key).or_insert((0, false));
            entry.0 += 1;
            entry.1 |= to_bus || owner.is_some();
            // Each destination and method is logged the first time; `vctl dbus` counts the rest.
            if !to_bus && entry.0 == 1 {
                let program = &self.dbus_who(id);
                match owner {
                    Some(_) => tracing::info!("dbus: {program} {unique} -> {dest} {} {}", m.method(), m.path.as_deref().unwrap_or("")),
                    None => tracing::info!("dbus: {program} {unique} -> {dest} {} {} (nobody owns it)", m.method(), m.path.as_deref().unwrap_or("")),
                }
            }
        }

        if to_bus {
            if m.kind == METHOD_CALL {
                self.dbus_driver(id, &m);
            }
            return;
        }
        match m.destination.clone() {
            Some(dest) => match self.dbus_owner(&dest) {
                Some(EDGE) => {
                    if m.kind == METHOD_CALL {
                        if m.path.as_deref() == Some(NOTIFY_PATH) {
                            self.dbus_notifications(id, &m);
                        } else {
                            self.dbus_portal(id, &m);
                        }
                    }
                }
                Some(to) => self.dbus_deliver(to, &m),
                None if m.kind == METHOD_CALL && m.flags & NO_REPLY_EXPECTED == 0 => {
                    let text = format!("The name {dest} is not provided by this session (Vergence starts nothing on demand)");
                    self.dbus_reply(id, Message::error(&m, BUS_NAME, E_SERVICE_UNKNOWN, &text));
                }
                None => {}
            },
            None if m.kind == SIGNAL => self.dbus_broadcast(&m),
            None => {
                if m.kind == METHOD_CALL && m.flags & NO_REPLY_EXPECTED == 0 {
                    self.dbus_reply(id, Message::error(&m, BUS_NAME, E_UNKNOWN_METHOD, "method calls need a destination"));
                }
            }
        }
    }

    fn dbus_hello(&mut self, id: u32, m: &Message) {
        let unique = format!(":1.{id}");
        let Some(c) = self.dbus.conns.get_mut(&id) else { return };
        c.unique = Some(unique.clone());
        if !c.program.is_empty() {
            tracing::info!("dbus: {} (pid {:?}, uid {}) is {unique}", c.program, c.pid, c.uid);
        }
        let mut call = m.clone();
        call.sender = Some(unique.clone());
        self.dbus_reply(id, Message::method_return(&call, BUS_NAME, &[Value::Str(unique.clone())]));
        self.dbus_name_acquired(id, &unique);
        self.dbus_owner_changed(&unique, "", &unique);
    }

    /// The program for logs, or its pid while the name is being looked up.
    fn dbus_who(&self, id: u32) -> String {
        match self.dbus.conns.get(&id) {
            Some(c) if !c.program.is_empty() => c.program.clone(),
            Some(c) => format!("pid {}", c.pid.map(|p| p.to_string()).unwrap_or_else(|| "?".into())),
            None => "?".into(),
        }
    }

    /// The program name arrived from the lookup thread.
    fn dbus_program_known(&mut self, id: u32, program: String) {
        let Some(c) = self.dbus.conns.get_mut(&id) else { return };
        if let Some(u) = &c.unique {
            tracing::info!("dbus: {program} (pid {:?}, uid {}) is {u}", c.pid, c.uid);
        }
        c.program = program;
    }

    /// The connection that owns `name` (unique or well-known); EDGE for the
    /// services the edge provides itself.
    fn dbus_owner(&self, name: &str) -> Option<u32> {
        if name == PORTAL_NAME || name == NOTIFY_NAME || name == EDGE_UNIQUE {
            return Some(EDGE);
        }
        if let Some(n) = name.strip_prefix(":1.") {
            let id: u32 = n.parse().ok()?;
            return self.dbus.conns.get(&id).filter(|c| c.unique.is_some()).map(|_| id);
        }
        self.dbus.names.get(name).and_then(|q| q.front()).map(|(id, _)| *id)
    }

    fn dbus_unique(&self, id: u32) -> String {
        if id == EDGE {
            return EDGE_UNIQUE.into();
        }
        self.dbus.conns.get(&id).and_then(|c| c.unique.clone()).unwrap_or_default()
    }

    /// Every name connection `id` owns, its unique name first.
    fn dbus_names_of(&self, id: u32) -> Vec<String> {
        if id == EDGE {
            return vec![EDGE_UNIQUE.into(), PORTAL_NAME.into(), NOTIFY_NAME.into()];
        }
        let mut out = vec![self.dbus_unique(id)];
        out.extend(self.dbus.names.iter().filter(|(_, q)| q.front().is_some_and(|(o, _)| *o == id)).map(|(n, _)| n.clone()));
        out
    }

    fn dbus_next_serial(&mut self) -> u32 {
        self.dbus.serial = self.dbus.serial.wrapping_add(1).max(1);
        self.dbus.serial
    }

    /// A message from the bus itself to connection `to`.
    fn dbus_reply(&mut self, to: u32, mut m: Message) {
        m.serial = self.dbus_next_serial();
        let bytes = m.serialize();
        self.dbus_send_raw(to, &bytes);
    }

    fn dbus_deliver(&mut self, to: u32, m: &Message) {
        let bytes = m.serialize();
        self.dbus_send_raw(to, &bytes);
    }

    /// A signal without a destination: to every connection with a matching rule.
    fn dbus_broadcast(&mut self, m: &Message) {
        let sender_names = match m.sender.as_deref() {
            Some(BUS_NAME) => vec![BUS_NAME.to_string()],
            Some(s) => self.dbus_owner(s).map(|id| self.dbus_names_of(id)).unwrap_or_else(|| vec![s.to_string()]),
            None => Vec::new(),
        };
        let targets: Vec<u32> = self
            .dbus
            .conns
            .iter()
            .filter(|(_, c)| c.unique.is_some() && c.rules.iter().any(|r| r.matches(m, &sender_names)))
            .map(|(id, _)| *id)
            .collect();
        if targets.is_empty() {
            return;
        }
        let bytes = m.serialize();
        for t in targets {
            self.dbus_send_raw(t, &bytes);
        }
    }

    fn dbus_bus_signal(&mut self, member: &str, args: &[Value], to: Option<u32>) {
        let mut s = Message::signal(BUS_NAME, BUS_PATH, BUS_NAME, member, args);
        s.serial = self.dbus_next_serial();
        match to {
            Some(id) => {
                s.destination = Some(self.dbus_unique(id));
                let bytes = s.serialize();
                self.dbus_send_raw(id, &bytes);
            }
            None => self.dbus_broadcast(&s),
        }
    }

    fn dbus_name_acquired(&mut self, id: u32, name: &str) {
        self.dbus_bus_signal("NameAcquired", &[Value::Str(name.into())], Some(id));
    }

    fn dbus_owner_changed(&mut self, name: &str, old: &str, new: &str) {
        self.dbus_bus_signal("NameOwnerChanged", &[Value::Str(name.into()), Value::Str(old.into()), Value::Str(new.into())], None);
    }

    fn dbus_driver(&mut self, id: u32, m: &Message) {
        let args = m.args().unwrap_or_default();
        let s = |i: usize| args.get(i).and_then(Value::as_str).map(str::to_owned);
        let u = |i: usize| args.get(i).and_then(Value::as_u32);
        let iface = m.interface.as_deref().unwrap_or(BUS_NAME);
        let member = m.member.as_deref().unwrap_or("");
        let ok = |vals: &[Value]| Message::method_return(m, BUS_NAME, vals);
        let err = |name: &str, text: &str| Message::error(m, BUS_NAME, name, text);
        let reply = match (iface, member) {
            (BUS_NAME, "Hello") => err(E_ACCESS_DENIED, "already said Hello"),
            (BUS_NAME, "RequestName") => match (s(0), u(1)) {
                (Some(name), Some(flags)) => match self.dbus_request_name(id, &name, flags) {
                    Ok(r) => ok(&[Value::U32(r)]),
                    Err(text) => err(E_INVALID_ARGS, &text),
                },
                _ => err(E_INVALID_ARGS, "RequestName(su)"),
            },
            (BUS_NAME, "ReleaseName") => match s(0) {
                Some(name) => ok(&[Value::U32(self.dbus_release_name(id, &name, true))]),
                None => err(E_INVALID_ARGS, "ReleaseName(s)"),
            },
            (BUS_NAME, "GetNameOwner") => match s(0) {
                Some(n) if n == BUS_NAME => ok(&[Value::Str(BUS_NAME.into())]),
                Some(n) => match self.dbus_owner(&n) {
                    Some(o) => ok(&[Value::Str(self.dbus_unique(o))]),
                    None => err(E_NAME_HAS_NO_OWNER, &format!("{n} has no owner")),
                },
                None => err(E_INVALID_ARGS, "GetNameOwner(s)"),
            },
            (BUS_NAME, "NameHasOwner") => match s(0) {
                Some(n) => ok(&[Value::Bool(n == BUS_NAME || self.dbus_owner(&n).is_some())]),
                None => err(E_INVALID_ARGS, "NameHasOwner(s)"),
            },
            (BUS_NAME, "ListNames") => {
                let mut names = vec![BUS_NAME.to_string(), EDGE_UNIQUE.to_string(), PORTAL_NAME.to_string(), NOTIFY_NAME.to_string()];
                names.extend(self.dbus.conns.values().filter_map(|c| c.unique.clone()));
                names.extend(self.dbus.names.keys().cloned());
                ok(&[Value::strings(names)])
            }
            (BUS_NAME, "ListActivatableNames") => ok(&[Value::strings([BUS_NAME])]),
            (BUS_NAME, "ListQueuedOwners") => match s(0) {
                Some(n) => match self.dbus.names.get(&n) {
                    Some(q) => {
                        let list: Vec<String> = q.iter().map(|(o, _)| self.dbus_unique(*o)).collect();
                        ok(&[Value::strings(list)])
                    }
                    None => err(E_NAME_HAS_NO_OWNER, &format!("{n} has no owner")),
                },
                None => err(E_INVALID_ARGS, "ListQueuedOwners(s)"),
            },
            (BUS_NAME, "AddMatch") => match s(0).as_deref().map(MatchRule::parse) {
                Some(Some(rule)) => {
                    let c = self.dbus.conns.get_mut(&id).unwrap();
                    if c.rules.len() >= MAX_RULES {
                        err(E_LIMITS, "too many match rules")
                    } else {
                        c.rules.push(rule);
                        ok(&[])
                    }
                }
                Some(None) => err(E_MATCH_INVALID, "match rule not understood"),
                None => err(E_INVALID_ARGS, "AddMatch(s)"),
            },
            (BUS_NAME, "RemoveMatch") => match s(0) {
                Some(text) => {
                    let c = self.dbus.conns.get_mut(&id).unwrap();
                    match c.rules.iter().position(|r| r.text == text) {
                        Some(i) => {
                            c.rules.remove(i);
                            ok(&[])
                        }
                        None => err(E_MATCH_NOT_FOUND, "no such match rule"),
                    }
                }
                None => err(E_INVALID_ARGS, "RemoveMatch(s)"),
            },
            (BUS_NAME, "GetConnectionUnixUser") | (BUS_NAME, "GetConnectionUnixProcessID") | (BUS_NAME, "GetConnectionCredentials") => {
                match s(0).and_then(|n| self.dbus_owner(&n)).and_then(|o| self.dbus.conns.get(&o)) {
                    Some(c) => match member {
                        "GetConnectionUnixUser" => ok(&[Value::U32(c.uid)]),
                        "GetConnectionUnixProcessID" => match c.pid {
                            Some(p) => ok(&[Value::U32(p)]),
                            None => err("org.freedesktop.DBus.Error.UnixProcessIdUnknown", "pid unknown"),
                        },
                        _ => {
                            let mut d = vec![entry("UnixUserID", Value::U32(c.uid))];
                            if let Some(p) = c.pid {
                                d.push(entry("ProcessID", Value::U32(p)));
                            }
                            ok(&[Value::Array("{sv}".into(), d)])
                        }
                    },
                    None => err(E_NAME_HAS_NO_OWNER, "no such connection"),
                }
            }
            (BUS_NAME, "GetId") => ok(&[Value::Str(self.dbus.guid.clone())]),
            (BUS_NAME, "StartServiceByName") => match s(0) {
                Some(n) if self.dbus_owner(&n).is_some() => ok(&[Value::U32(2)]),
                Some(n) => err(E_SERVICE_UNKNOWN, &format!("{n}: Vergence starts nothing on demand")),
                None => err(E_INVALID_ARGS, "StartServiceByName(su)"),
            },
            // Accepted and ignored: nothing is ever activated (only logged).
            (BUS_NAME, "UpdateActivationEnvironment") | (BUS_NAME, "ReloadConfig") => ok(&[]),
            ("org.freedesktop.DBus.Peer", "Ping") => ok(&[]),
            ("org.freedesktop.DBus.Peer", "GetMachineId") => ok(&[Value::Str(machine_id())]),
            ("org.freedesktop.DBus.Introspectable", "Introspect") => ok(&[Value::Str(INTROSPECTION.into())]),
            ("org.freedesktop.DBus.Properties", "GetAll") => {
                ok(&[Value::Array("{sv}".into(), vec![entry("Features", Value::strings(Vec::<String>::new())), entry("Interfaces", Value::strings(Vec::<String>::new()))])])
            }
            ("org.freedesktop.DBus.Properties", "Get") => match s(1).as_deref() {
                Some("Features") | Some("Interfaces") => ok(&[Value::Variant(Box::new(Value::strings(Vec::<String>::new())))]),
                _ => err(E_INVALID_ARGS, "no such property"),
            },
            _ => err(E_UNKNOWN_METHOD, &format!("{iface}.{member} is not provided by this bus")),
        };
        if m.flags & NO_REPLY_EXPECTED == 0 {
            self.dbus_reply(id, reply);
        }
    }

    /// RequestName: 1 primary owner, 2 queued, 3 exists, 4 already owner.
    fn dbus_request_name(&mut self, id: u32, name: &str, flags: u32) -> Result<u32, String> {
        if name.starts_with(':') || name == BUS_NAME || name == PORTAL_NAME || name == NOTIFY_NAME || !vergence_dbus::valid_bus_name(name) {
            return Err(format!("{name} can't be requested"));
        }
        let me = self.dbus_unique(id);
        let q = self.dbus.names.entry(name.to_string()).or_default();
        let Some(&(owner, owner_flags)) = q.front() else {
            q.push_back((id, flags));
            self.dbus_name_acquired(id, name);
            self.dbus_owner_changed(name, "", &me);
            tracing::info!("dbus: {me} owns {name}");
            return Ok(1);
        };
        if owner == id {
            q[0].1 = flags;
            return Ok(4);
        }
        if flags & REPLACE_EXISTING != 0 && owner_flags & ALLOW_REPLACEMENT != 0 {
            q.retain(|(o, _)| *o != id);
            q.pop_front();
            q.push_front((id, flags));
            if owner_flags & DO_NOT_QUEUE == 0 {
                q.insert(1, (owner, owner_flags));
            }
            let old = self.dbus_unique(owner);
            self.dbus_bus_signal("NameLost", &[Value::Str(name.into())], Some(owner));
            self.dbus_name_acquired(id, name);
            self.dbus_owner_changed(name, &old, &me);
            tracing::info!("dbus: {me} took {name} from {old}");
            return Ok(1);
        }
        if flags & DO_NOT_QUEUE != 0 {
            return Ok(3);
        }
        if !q.iter().any(|(o, _)| *o == id) {
            q.push_back((id, flags));
        }
        Ok(2)
    }

    /// ReleaseName: 1 released, 2 no such name, 3 not owner. `tell` sends
    /// NameLost (not when the connection is gone).
    fn dbus_release_name(&mut self, id: u32, name: &str, tell: bool) -> u32 {
        let Some(q) = self.dbus.names.get_mut(name) else { return 2 };
        let Some(pos) = q.iter().position(|(o, _)| *o == id) else { return 3 };
        q.remove(pos);
        if pos != 0 {
            return 1;
        }
        let next = q.front().map(|(o, _)| *o);
        if next.is_none() {
            self.dbus.names.remove(name);
        }
        let old = self.dbus_unique(id);
        if tell {
            self.dbus_bus_signal("NameLost", &[Value::Str(name.into())], Some(id));
        }
        match next {
            Some(n) => {
                let new = self.dbus_unique(n);
                self.dbus_name_acquired(n, name);
                self.dbus_owner_changed(name, &old, &new);
            }
            None => self.dbus_owner_changed(name, &old, ""),
        }
        1
    }

    fn dbus_send_raw(&mut self, id: u32, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let Some(c) = self.dbus.conns.get_mut(&id) else { return };
        if c.closing {
            return;
        }
        if c.outbox.len() + bytes.len() > OUTBOX_MAX {
            tracing::warn!("dbus: {} isn't reading; disconnecting", c.program);
            c.closing = true;
            let _ = c.stream.shutdown(std::net::Shutdown::Both);
            return;
        }
        c.outbox.extend_from_slice(bytes);
        if !self.dbus_flush(id) {
            self.dbus_watch_writable(id);
        }
    }

    fn dbus_flush(&mut self, id: u32) -> bool {
        let Some(c) = self.dbus.conns.get_mut(&id) else { return true };
        while !c.outbox.is_empty() {
            match (&c.stream).write(&c.outbox) {
                Ok(0) => break,
                Ok(n) => {
                    c.outbox.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    c.outbox.clear();
                    c.closing = true;
                    let _ = c.stream.shutdown(std::net::Shutdown::Both);
                    break;
                }
            }
        }
        c.outbox.is_empty()
    }

    fn dbus_watch_writable(&mut self, id: u32) {
        let Some(c) = self.dbus.conns.get_mut(&id) else { return };
        if c.write_source.is_some() {
            return;
        }
        let Ok(half) = c.stream.try_clone() else { return };
        let res = self.loop_handle.insert_source(Generic::new(half, Interest::WRITE, Mode::Level), move |_, _, state| {
            if state.dbus_flush(id) {
                if let Some(c) = state.dbus.conns.get_mut(&id) {
                    c.write_source = None;
                }
                Ok(PostAction::Remove)
            } else {
                Ok(PostAction::Continue)
            }
        });
        if let Ok(t) = res {
            c.write_source = Some(t);
        }
    }

    /// The connection is gone: release its names, tell the others.
    fn dbus_forget(&mut self, id: u32) {
        let Some(c) = self.dbus.conns.get(&id) else { return };
        let unique = c.unique.clone();
        let program = c.program.clone();
        let owned: Vec<String> = self.dbus.names.iter().filter(|(_, q)| q.iter().any(|(o, _)| *o == id)).map(|(n, _)| n.clone()).collect();
        for n in owned {
            self.dbus_release_name(id, &n, false);
        }
        if let Some(c) = self.dbus.conns.remove(&id)
            && let Some(t) = c.write_source
        {
            self.loop_handle.remove(t);
        }
        if let Some(u) = unique {
            self.dbus_owner_changed(&u, &u, "");
            tracing::info!("dbus: {program} {u} left");
        }
    }

    /// `vctl dbus`: the connections, and every call seen.
    pub fn dbus_report(&self) -> String {
        let mut out = Vec::new();
        match &self.dbus.path {
            Some(p) => out.push(format!("edge {}", p.display())),
            None => return "dbus: edge off".into(),
        }
        let mut conns: Vec<_> = self.dbus.conns.iter().collect();
        conns.sort_by_key(|(id, _)| **id);
        for (id, c) in conns {
            let names: Vec<String> = self.dbus_names_of(*id).into_iter().skip(1).collect();
            out.push(format!(
                "conn {} {} pid {:?} owns {:?} rules {}",
                c.unique.as_deref().unwrap_or("(no Hello)"),
                c.program,
                c.pid,
                names,
                c.rules.len()
            ));
        }
        out.push("calls (count, destination method, answered):".into());
        let mut stats: Vec<_> = self.dbus.stats.iter().collect();
        stats.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
        for (k, (n, answered)) in stats {
            out.push(format!("{n:6} {k} {}", if *answered { "" } else { "  <- nobody" }));
        }
        out.join("\n")
    }
}

impl Server {
    /// A call to the edge's own portal object.
    fn dbus_portal(&mut self, id: u32, m: &Message) {
        let args = m.args().unwrap_or_default();
        let s = |i: usize| args.get(i).and_then(Value::as_str).map(str::to_owned);
        let iface = m.interface.as_deref().unwrap_or("");
        let member = m.member.as_deref().unwrap_or("");
        let on_portal = m.path.as_deref() == Some(PORTAL_PATH);
        let ok = |vals: &[Value]| Message::method_return(m, EDGE_UNIQUE, vals);
        let err = |name: &str, text: &str| Message::error(m, EDGE_UNIQUE, name, text);
        let not_found = |ns: &str, key: &str| err("org.freedesktop.portal.Error.NotFound", &format!("{ns} {key}: no such setting"));
        let reply = match (iface, member) {
            ("org.freedesktop.DBus.Peer", "Ping") => ok(&[]),
            ("org.freedesktop.DBus.Peer", "GetMachineId") => ok(&[Value::Str(machine_id())]),
            ("org.freedesktop.DBus.Introspectable", "Introspect") => {
                ok(&[Value::Str(if on_portal { PORTAL_INTROSPECTION } else { EMPTY_INTROSPECTION }.into())])
            }
            // A portal Request object (portal.rs): the caller gives up.
            (crate::portal::REQUEST, "Close") if m.path.as_deref().is_some_and(|p| p.starts_with(crate::portal::REQUEST_PREFIX)) => {
                self.portal_close(m.path.as_deref().unwrap_or_default());
                ok(&[])
            }
            (_, _) if !on_portal => err(E_UNKNOWN_METHOD, "no such object"),
            (crate::portal::FILE_CHOOSER, "OpenFile" | "SaveFile" | "SaveFiles") => {
                let unique = self.dbus.conns.get(&id).and_then(|c| c.unique.clone()).unwrap_or_default();
                match self.portal_choose(id, &unique, member, &args) {
                    Ok(handle) => ok(&[Value::Path(handle)]),
                    Err(e) => err("org.freedesktop.portal.Error.Failed", &e),
                }
            }
            (crate::portal::OPEN_URI, "OpenURI") => {
                let unique = self.dbus.conns.get(&id).and_then(|c| c.unique.clone()).unwrap_or_default();
                match self.portal_open_uri(id, &unique, &args) {
                    Ok(handle) => ok(&[Value::Path(handle)]),
                    Err(e) => err(E_INVALID_ARGS, &e),
                }
            }
            (crate::portal::OPEN_URI, "OpenFile" | "OpenDirectory") => err("org.freedesktop.portal.Error.NotAllowed", "files passed by descriptor are not taken by Vergence's bus yet; use OpenURI"),
            (PORTAL_SETTINGS, "ReadOne") | (PORTAL_SETTINGS, "Read") => match (s(0), s(1)) {
                (Some(ns), Some(key)) => {
                    self.dbus_log_read(id, &ns, &key);
                    match self.settings.current.get(&(ns.clone(), key.clone())) {
                        // Read (deprecated) wraps the value in a second variant.
                        Some(v) if member == "Read" => ok(&[Value::Variant(Box::new(Value::Variant(Box::new(to_dbus(v)))))]),
                        Some(v) => ok(&[Value::Variant(Box::new(to_dbus(v)))]),
                        None => not_found(&ns, &key),
                    }
                }
                _ => err(E_INVALID_ARGS, "ReadOne(ss)"),
            },
            (PORTAL_SETTINGS, "ReadAll") => {
                let patterns: Vec<String> = match args.first() {
                    Some(Value::Array(_, items)) => items.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
                    _ => Vec::new(),
                };
                self.dbus_log_read(id, &patterns.join(","), "*");
                let mut by_ns: BTreeMap<String, Vec<Value>> = BTreeMap::new();
                for ((ns, key), v) in &self.settings.current {
                    if namespace_matches(&patterns, ns) {
                        by_ns.entry(ns.clone()).or_default().push(entry(key, to_dbus(v)));
                    }
                }
                let outer = by_ns
                    .into_iter()
                    .map(|(ns, entries)| Value::Entry(Box::new(Value::Str(ns)), Box::new(Value::Array("{sv}".into(), entries))))
                    .collect();
                ok(&[Value::Array("{sa{sv}}".into(), outer)])
            }
            ("org.freedesktop.DBus.Properties", "Get") => match (s(0).as_deref(), s(1).as_deref()) {
                (Some(PORTAL_SETTINGS), Some("version")) => ok(&[Value::Variant(Box::new(Value::U32(2)))]),
                (Some(crate::portal::FILE_CHOOSER), Some("version")) => ok(&[Value::Variant(Box::new(Value::U32(3)))]),
                (Some(crate::portal::OPEN_URI), Some("version")) => ok(&[Value::Variant(Box::new(Value::U32(3)))]),
                _ => err(E_INVALID_ARGS, "no such interface or property"),
            },
            ("org.freedesktop.DBus.Properties", "GetAll") => match s(0).as_deref() {
                Some(PORTAL_SETTINGS) => ok(&[Value::Array("{sv}".into(), vec![entry("version", Value::U32(2))])]),
                Some(crate::portal::FILE_CHOOSER) | Some(crate::portal::OPEN_URI) => ok(&[Value::Array("{sv}".into(), vec![entry("version", Value::U32(3))])]),
                _ => err(E_INVALID_ARGS, "no such interface"),
            },
            _ => err(E_UNKNOWN_METHOD, &format!("{iface}.{member} is not provided (yet) by Vergence")),
        };
        if m.flags & NO_REPLY_EXPECTED == 0 {
            self.dbus_reply(id, reply);
        }
    }

    /// org.freedesktop.Notifications, translated into the server's notifications.
    fn dbus_notifications(&mut self, id: u32, m: &Message) {
        let args = m.args().unwrap_or_default();
        let iface = m.interface.as_deref().unwrap_or("");
        let member = m.member.as_deref().unwrap_or("");
        let ok = |vals: &[Value]| Message::method_return(m, EDGE_UNIQUE, vals);
        let err = |name: &str, text: &str| Message::error(m, EDGE_UNIQUE, name, text);
        let reply = match (iface, member) {
            ("org.freedesktop.DBus.Peer", "Ping") => ok(&[]),
            ("org.freedesktop.DBus.Introspectable", "Introspect") => ok(&[Value::Str(NOTIFY_INTROSPECTION.into())]),
            (NOTIFY_NAME, "GetCapabilities") | ("", "GetCapabilities") => ok(&[Value::strings(["body", "actions", "persistence"])]),
            (NOTIFY_NAME, "GetServerInformation") | ("", "GetServerInformation") => ok(&[
                Value::Str("Vergence".into()),
                Value::Str("Vergence".into()),
                Value::Str(env!("CARGO_PKG_VERSION").into()),
                Value::Str("1.2".into()),
            ]),
            (NOTIFY_NAME, "Notify") | ("", "Notify") => match parse_notify(&args) {
                Some(req) => {
                    let nid = self.notify(crate::notify::Origin::DBus(id), req);
                    ok(&[Value::U32(nid)])
                }
                None => err(E_INVALID_ARGS, "Notify(susssasa{sv}i)"),
            },
            (NOTIFY_NAME, "CloseNotification") | ("", "CloseNotification") => match args.first().and_then(Value::as_u32) {
                Some(nid) => {
                    if self.notify_close(crate::notify::Origin::DBus(id), nid) {
                        ok(&[])
                    } else {
                        err(E_INVALID_ARGS, &format!("no notification {nid} of yours"))
                    }
                }
                None => err(E_INVALID_ARGS, "CloseNotification(u)"),
            },
            _ => err(E_UNKNOWN_METHOD, &format!("{iface}.{member} is not provided by Vergence")),
        };
        if m.flags & NO_REPLY_EXPECTED == 0 {
            self.dbus_reply(id, reply);
        }
    }

    /// A connection's process, if its credentials gave one.
    pub fn dbus_conn_pid(&self, conn: u32) -> Option<u32> {
        self.dbus.conns.get(&conn).and_then(|c| c.pid)
    }

    /// A portal Request's Response signal, to the connection that made it only.
    pub fn dbus_request_response(&mut self, conn: u32, handle: &str, response: u32, results: Value) {
        let Some(dest) = self.dbus.conns.get(&conn).and_then(|c| c.unique.clone()) else { return };
        let mut sig = Message::signal(EDGE_UNIQUE, handle, crate::portal::REQUEST, "Response", &[Value::U32(response), results]);
        sig.destination = Some(dest);
        sig.serial = self.dbus_next_serial();
        let bytes = sig.serialize();
        self.dbus_send_raw(conn, &bytes);
    }

    /// A signal from the notification service to one connection only.
    fn dbus_notify_signal(&mut self, conn: u32, member: &str, args: &[Value]) {
        let Some(dest) = self.dbus.conns.get(&conn).and_then(|c| c.unique.clone()) else { return };
        let mut sig = Message::signal(EDGE_UNIQUE, NOTIFY_PATH, NOTIFY_NAME, member, args);
        sig.destination = Some(dest);
        sig.serial = self.dbus_next_serial();
        let bytes = sig.serialize();
        self.dbus_send_raw(conn, &bytes);
    }

    pub fn dbus_notification_closed(&mut self, conn: u32, id: u32, reason: u32) {
        self.dbus_notify_signal(conn, "NotificationClosed", &[Value::U32(id), Value::U32(reason)]);
    }

    pub fn dbus_action_invoked(&mut self, conn: u32, id: u32, key: &str) {
        self.dbus_notify_signal(conn, "ActionInvoked", &[Value::U32(id), Value::Str(key.into())]);
    }

    fn dbus_log_read(&mut self, id: u32, ns: &str, key: &str) {
        if self.dbus.reads.insert((ns.to_string(), key.to_string())) {
            let program = self.dbus_who(id);
            tracing::info!("dbus: portal: {program} reads {ns} {key}");
        }
    }

    /// Settings changed (settings.rs): the portal's SettingChanged signal.
    pub fn dbus_settings_changed(&mut self, changed: &[vergence_protocol::bus::Setting]) {
        if self.dbus.path.is_none() {
            return;
        }
        for c in changed {
            let args = [Value::Str(c.namespace.clone()), Value::Str(c.key.clone()), Value::Variant(Box::new(to_dbus(&c.value)))];
            let mut sig = Message::signal(EDGE_UNIQUE, PORTAL_PATH, PORTAL_SETTINGS, "SettingChanged", &args);
            sig.serial = self.dbus_next_serial();
            self.dbus_broadcast(&sig);
        }
    }
}

/// Notify(app_name s, replaces_id u, app_icon s, summary s, body s,
/// actions as, hints a{sv}, expire_timeout i).
fn parse_notify(args: &[Value]) -> Option<crate::notify::Request> {
    let s = |i: usize| args.get(i).and_then(Value::as_str).map(str::to_owned);
    let actions = match args.get(5)? {
        Value::Array(_, items) => {
            let flat: Vec<String> = items.iter().filter_map(Value::as_str).map(str::to_owned).collect();
            flat.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0].clone(), c[1].clone())).collect()
        }
        _ => return None,
    };
    let mut urgency = 1;
    if let Some(Value::Array(_, hints)) = args.get(6) {
        for h in hints {
            if let Value::Entry(k, v) = h
                && k.as_str() == Some("urgency")
                && let Value::Variant(v) = &**v
            {
                urgency = match **v {
                    Value::Byte(b) => b,
                    Value::U32(u) => u.min(2) as u8,
                    Value::I32(i) => i.clamp(0, 2) as u8,
                    _ => 1,
                };
            }
        }
    }
    let timeout_ms = match args.get(7)? {
        Value::I32(t) => *t,
        _ => return None,
    };
    Some(crate::notify::Request {
        app: s(0)?,
        replaces: args.get(1)?.as_u32()?,
        summary: s(3)?,
        body: s(4)?,
        urgency,
        timeout_ms,
        actions,
    })
}

const NOTIFY_INTROSPECTION: &str = r#"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node>
  <interface name="org.freedesktop.Notifications">
    <method name="GetCapabilities"><arg type="as" direction="out"/></method>
    <method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/><arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method>
    <method name="CloseNotification"><arg type="u" direction="in"/></method>
    <method name="GetServerInformation"><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/></method>
    <signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
    <signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
  </interface>
</node>
"#;

/// ReadAll's namespace patterns: empty means all; "org.gnome.*" a prefix.
fn namespace_matches(patterns: &[String], ns: &str) -> bool {
    patterns.is_empty()
        || patterns.iter().any(|p| match p.strip_suffix('*') {
            Some(prefix) => ns.starts_with(prefix),
            None => p == ns || p.is_empty(),
        })
}

fn to_dbus(v: &vergence_protocol::bus::SettingValue) -> Value {
    use vergence_protocol::bus::SettingValue as S;
    match v {
        S::Bool(b) => Value::Bool(*b),
        S::U32(x) => Value::U32(*x),
        S::I32(x) => Value::I32(*x),
        S::F64(x) => Value::Double(*x),
        S::Str(s) => Value::Str(s.clone()),
        S::Rgb(r, g, b) => Value::Struct(vec![Value::Double(*r), Value::Double(*g), Value::Double(*b)]),
    }
}

const EMPTY_INTROSPECTION: &str = "<node/>";

const PORTAL_INTROSPECTION: &str = r#"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node>
  <interface name="org.freedesktop.portal.Settings">
    <method name="ReadAll"><arg type="as" name="namespaces" direction="in"/><arg type="a{sa{sv}}" name="value" direction="out"/></method>
    <method name="Read"><arg type="s" name="namespace" direction="in"/><arg type="s" name="key" direction="in"/><arg type="v" name="value" direction="out"/></method>
    <method name="ReadOne"><arg type="s" name="namespace" direction="in"/><arg type="s" name="key" direction="in"/><arg type="v" name="value" direction="out"/></method>
    <signal name="SettingChanged"><arg type="s" name="namespace"/><arg type="s" name="key"/><arg type="v" name="value"/></signal>
    <property name="version" type="u" access="read"/>
  </interface>
  <interface name="org.freedesktop.portal.FileChooser">
    <method name="OpenFile"><arg type="s" name="parent_window" direction="in"/><arg type="s" name="title" direction="in"/><arg type="a{sv}" name="options" direction="in"/><arg type="o" name="handle" direction="out"/></method>
    <method name="SaveFile"><arg type="s" name="parent_window" direction="in"/><arg type="s" name="title" direction="in"/><arg type="a{sv}" name="options" direction="in"/><arg type="o" name="handle" direction="out"/></method>
    <method name="SaveFiles"><arg type="s" name="parent_window" direction="in"/><arg type="s" name="title" direction="in"/><arg type="a{sv}" name="options" direction="in"/><arg type="o" name="handle" direction="out"/></method>
    <property name="version" type="u" access="read"/>
  </interface>
  <interface name="org.freedesktop.portal.OpenURI">
    <method name="OpenURI"><arg type="s" name="parent_window" direction="in"/><arg type="s" name="uri" direction="in"/><arg type="a{sv}" name="options" direction="in"/><arg type="o" name="handle" direction="out"/></method>
    <property name="version" type="u" access="read"/>
  </interface>
</node>
"#;

fn entry(k: &str, v: Value) -> Value {
    Value::Entry(Box::new(Value::Str(k.into())), Box::new(Value::Variant(Box::new(v))))
}

fn machine_id() -> String {
    for p in ["/etc/machine-id", "/var/lib/dbus/machine-id", "/var/db/dbus/machine-id"] {
        if let Ok(s) = std::fs::read_to_string(p) {
            let s = s.trim();
            if s.len() == 32 {
                return s.to_string();
            }
        }
    }
    "00000000000000000000000000000000".into()
}

const INTROSPECTION: &str = r#"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node>
  <interface name="org.freedesktop.DBus">
    <method name="Hello"><arg direction="out" type="s"/></method>
    <method name="RequestName"><arg direction="in" type="s"/><arg direction="in" type="u"/><arg direction="out" type="u"/></method>
    <method name="ReleaseName"><arg direction="in" type="s"/><arg direction="out" type="u"/></method>
    <method name="GetNameOwner"><arg direction="in" type="s"/><arg direction="out" type="s"/></method>
    <method name="NameHasOwner"><arg direction="in" type="s"/><arg direction="out" type="b"/></method>
    <method name="ListNames"><arg direction="out" type="as"/></method>
    <method name="ListActivatableNames"><arg direction="out" type="as"/></method>
    <method name="AddMatch"><arg direction="in" type="s"/></method>
    <method name="RemoveMatch"><arg direction="in" type="s"/></method>
    <method name="GetId"><arg direction="out" type="s"/></method>
    <signal name="NameOwnerChanged"><arg type="s"/><arg type="s"/><arg type="s"/></signal>
    <signal name="NameLost"><arg type="s"/></signal>
    <signal name="NameAcquired"><arg type="s"/></signal>
  </interface>
  <interface name="org.freedesktop.DBus.Peer"><method name="Ping"/><method name="GetMachineId"><arg direction="out" type="s"/></method></interface>
  <interface name="org.freedesktop.DBus.Introspectable"><method name="Introspect"><arg direction="out" type="s"/></method></interface>
</node>
"#;
