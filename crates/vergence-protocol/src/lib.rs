//! Vergence's native protocol (whitepaper §6): the message types and wire
//! format the server and its clients share. The session bus (§6.8) is the
//! first part; windows, placement (Contract 2) and accessibility (Contract 3)
//! join it on the same connection later.
//!
//! Our own code, with no dependencies (§8, "Our own parts").

pub mod agent;
pub mod bus;
pub mod commands;
pub mod sock;
pub mod window;
pub mod wire;
pub mod shm;
pub mod tile;

use std::{
    os::unix::net::UnixStream,
    path::PathBuf,
};

use bus::{ClientMsg, ServerMsg, VERSION};
use wire::{FrameReader, WireError};

/// The environment variable that names the bus socket, set by the server for
/// everything it starts (like `WAYLAND_DISPLAY`).
pub const BUS_ENV: &str = "VERGENCE_BUS";

/// The bus socket for a server whose Wayland socket is `wayland_name`:
/// `$XDG_RUNTIME_DIR/vergence-bus-<wayland_name>`, so a nested server and the
/// session's own never share one.
pub fn bus_socket_for(runtime_dir: &std::path::Path, wayland_name: &str) -> PathBuf {
    runtime_dir.join(format!("vergence-bus-{wayland_name}"))
}

/// Where a client finds the bus: `$VERGENCE_BUS` if set (a path, or a name
/// inside `$XDG_RUNTIME_DIR`), else derived from `$WAYLAND_DISPLAY`.
pub fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    if let Some(v) = std::env::var_os(BUS_ENV) {
        let p = PathBuf::from(v);
        return if p.is_absolute() { Some(p) } else { runtime.map(|r| r.join(p)) };
    }
    let wayland = std::env::var("WAYLAND_DISPLAY").ok()?;
    Some(bus_socket_for(&runtime?, &wayland))
}

#[derive(Debug)]
pub enum ClientError {
    Io(std::io::Error),
    Wire(WireError),
    /// The server said no (its error code and message).
    Refused(bus::ErrorCode, String),
    /// The server answered with something that doesn't fit the request.
    Unexpected(ServerMsg),
    /// No bus socket could be found from the environment.
    NoSocket,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Io(e) => write!(f, "{e}"),
            ClientError::Wire(e) => write!(f, "protocol: {e}"),
            ClientError::Refused(code, msg) => write!(f, "refused ({code:?}): {msg}"),
            ClientError::Unexpected(m) => write!(f, "unexpected reply {m:?}"),
            ClientError::NoSocket => write!(f, "no bus: set {BUS_ENV} or WAYLAND_DISPLAY and XDG_RUNTIME_DIR"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<std::io::Error> for ClientError {
    fn from(e: std::io::Error) -> Self {
        ClientError::Io(e)
    }
}

impl From<WireError> for ClientError {
    fn from(e: WireError) -> Self {
        ClientError::Wire(e)
    }
}

/// A simple blocking bus client, for tools and tests. Events that arrive
/// while waiting for a reply are kept and returned by [`Client::next_event`].
pub struct Client {
    stream: UnixStream,
    reader: FrameReader,
    /// Descriptors that came with what has been read, not yet claimed.
    fds: std::collections::VecDeque<std::os::fd::OwnedFd>,
    serial: u32,
    events: std::collections::VecDeque<ServerMsg>,
    pub client_id: u32,
}

impl Client {
    /// Connect to the bus named by the environment and say hello.
    pub fn connect(app_id: &str) -> Result<Client, ClientError> {
        let path = socket_path().ok_or(ClientError::NoSocket)?;
        Self::connect_to(&path, app_id)
    }

    pub fn connect_to(path: &std::path::Path, app_id: &str) -> Result<Client, ClientError> {
        let stream = UnixStream::connect(path)?;
        let mut c = Client { stream, reader: FrameReader::new(), fds: Default::default(), serial: 0, events: Default::default(), client_id: 0 };
        c.send(&ClientMsg::Hello { version: VERSION, app_id: app_id.into() })?;
        match c.read()? {
            ServerMsg::Welcome { client_id, .. } => c.client_id = client_id,
            ServerMsg::Error { code, message, .. } => return Err(ClientError::Refused(code, message)),
            other => return Err(ClientError::Unexpected(other)),
        }
        Ok(c)
    }

    pub fn next_serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1).max(1);
        self.serial
    }

    pub fn send(&mut self, m: &ClientMsg) -> Result<(), ClientError> {
        let (bytes, fds) = m.encode_with_fds()?;
        sock::send_all(&self.stream, &bytes, &fds)?;
        Ok(())
    }

    fn read(&mut self) -> Result<ServerMsg, ClientError> {
        loop {
            if let Some(f) = self.reader.next_frame()? {
                return Ok(ServerMsg::decode_with_fds(&f, &mut self.fds)?);
            }
            let mut buf = [0u8; 4096];
            let n = sock::recv_with_fds(&self.stream, &mut buf, &mut self.fds, 64)?;
            if n == 0 {
                return Err(ClientError::Io(std::io::ErrorKind::UnexpectedEof.into()));
            }
            self.reader.push(&buf[..n]);
        }
    }

    /// Send a request and wait for the reply with its serial.
    pub fn request(&mut self, make: impl FnOnce(u32) -> ClientMsg) -> Result<ServerMsg, ClientError> {
        let serial = self.next_serial();
        self.send(&make(serial))?;
        loop {
            let m = self.read()?;
            match reply_serial(&m) {
                Some(s) if s == serial => {
                    return match m {
                        ServerMsg::Error { code, message, .. } => Err(ClientError::Refused(code, message)),
                        m => Ok(m),
                    };
                }
                _ => self.events.push_back(m),
            }
        }
    }

    /// The next event (serial 0), waiting for one if none is queued.
    pub fn next_event(&mut self) -> Result<ServerMsg, ClientError> {
        if let Some(m) = self.events.pop_front() {
            return Ok(m);
        }
        self.read()
    }

    // For a client with an event loop of its own (the toolkit's native road):
    // poll `raw_fd`, call `read_ready` when it is readable, then take what
    // `try_event` has, none of which blocks.

    /// The connection's descriptor, to poll.
    pub fn raw_fd(&self) -> std::os::fd::RawFd {
        std::os::fd::AsRawFd::as_raw_fd(&self.stream)
    }

    /// Whether messages are already here, read and not yet taken (poll would
    /// not say so).
    pub fn has_buffered(&mut self) -> bool {
        !self.events.is_empty() || self.reader.has_frame()
    }

    /// One read, after poll said the connection is readable. Ok(false): the
    /// server closed it.
    pub fn read_ready(&mut self) -> Result<bool, ClientError> {
        let mut buf = [0u8; 16 * 1024];
        let n = sock::recv_with_fds(&self.stream, &mut buf, &mut self.fds, 64)?;
        self.reader.push(&buf[..n]);
        Ok(n > 0)
    }

    /// The next message already read, if there is one; never waits.
    pub fn try_event(&mut self) -> Result<Option<ServerMsg>, ClientError> {
        if let Some(m) = self.events.pop_front() {
            return Ok(Some(m));
        }
        match self.reader.next_frame()? {
            Some(f) => Ok(Some(ServerMsg::decode_with_fds(&f, &mut self.fds)?)),
            None => Ok(None),
        }
    }
}

/// The serial a server message answers, if it is a reply (events have 0).
pub fn reply_serial(m: &ServerMsg) -> Option<u32> {
    let s = match m {
        ServerMsg::Welcome { .. } | ServerMsg::NotificationClosed { .. } | ServerMsg::ActionInvoked { .. } | ServerMsg::Alive { .. } => return None,
        ServerMsg::CardOpened { .. } | ServerMsg::CardClosed { .. } | ServerMsg::TileClicked { .. } | ServerMsg::TileAction { .. } => return None,
        ServerMsg::Pong { serial } | ServerMsg::Done { serial } | ServerMsg::Notified { serial, .. } => *serial,
        ServerMsg::Error { serial, .. } | ServerMsg::Outputs { serial, .. } | ServerMsg::Settings { serial, .. } | ServerMsg::Inputs { serial, .. } | ServerMsg::Places { serial, .. } | ServerMsg::Apps { serial, .. } | ServerMsg::Grants { serial, .. } => *serial,
        ServerMsg::Window(w) => return w.reply_serial(),
        ServerMsg::Agent(a) => a.serial(),
    };
    (s != 0).then_some(s)
}
