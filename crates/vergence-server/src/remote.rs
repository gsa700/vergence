//! Vergence Remote's socket (remote milestone 1, step 3;
//! docs/remote-milestone-1.md). Beside the bus, the server listens on
//! `$XDG_RUNTIME_DIR/vergence-remote-<display>` (owner only, the same user
//! only, as the bus); `vergence-remote`, run by SSH for a viewer, connects
//! and speaks Vergence Remote's own protocol (crates/vergence-remote,
//! docs/remote-protocol.md). On this local leg frames go uncompressed:
//! `vergence-remote` compresses them before they go out on SSH, so the
//! compositor's loop never spends its time on that.
//!
//! The screen's stream is a channel of its own, never the agent interface
//! (the agent layer is frozen); only the Dockworker's tile (drm.rs) is
//! shared machinery. Each connection: a thread reading the viewer's
//! messages (handed to the main loop on a channel), a thread writing what
//! the main loop gives it. One frame is in flight at a time: the next is
//! sent after the viewer's Ack, with whatever changed meanwhile merged in
//! (drm.rs keeps it until it is pulled).
//!
//! Same user, no prompt (Dave, 2026-11-23): connecting as the session's
//! own user (SSH with that user's key) is the person; the tile always shows.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;

use smithay::reexports::calloop::channel;
use vergence_remote::proto::{self, Encoding, Frame, Rect, ToHost, ToViewer};

use crate::Server;

/// What the connections' threads tell the main loop.
pub enum Event {
    Opened { conn: u32, writer: mpsc::Sender<Vec<u8>>, program: String },
    Message { conn: u32, msg: ToHost },
    Closed { conn: u32, why: String },
}

#[derive(Default)]
pub struct Remotes {
    conns: Vec<Conn>,
    pub path: Option<PathBuf>,
    /// The accepting thread's go-on flag, while the socket is open.
    open: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// A remote view is being drawn now (drm.rs): behind the curtain it
    /// draws the session, not the lock (decor.rs).
    pub drawing: bool,
    /// A remote viewer's input is being handled now: it passes the curtain
    /// (lock.rs `locked_for_input`).
    pub passing: bool,
}

struct Conn {
    id: u32,
    /// Its remote view (drm.rs), once it said Hello.
    view: Option<u32>,
    writer: mpsc::Sender<Vec<u8>>,
    /// The last frame was acknowledged (or none was sent yet).
    ready: bool,
    seq: u32,
    program: String,
}

/// desktop.toml's `[remote] enabled`: whether Vergence Remote may reach this
/// computer (Privacy > Remote; off by default, Dave 2026-11-23). Read at most
/// once a second.
pub fn enabled() -> bool {
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, bool)>> = std::sync::Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((t, v)) = *cache
        && t.elapsed() < std::time::Duration::from_secs(1)
    {
        return v;
    }
    let text = vergence_config::effective_text_with(crate::wallpaper::desktop_path().as_deref());
    let v = text
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("remote").and_then(|r| r.get("enabled")).and_then(|v| v.as_bool()))
        .unwrap_or(false);
    *cache = Some((std::time::Instant::now(), v));
    v
}

/// The socket opened or closed to follow the setting (the dock's tick, once
/// a second): off, there is no socket at all, and any connection ends.
pub fn follow_setting(state: &mut Server) {
    let on = enabled();
    if on && state.remote.open.is_none() {
        listen(state);
    } else if !on && state.remote.open.is_some() {
        unlisten(state);
    }
}

fn unlisten(state: &mut Server) {
    if let Some(flag) = state.remote.open.take() {
        flag.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    if let Some(path) = state.remote.path.take() {
        // The accepting thread wakes on a connection, sees the flag, and ends.
        let _ = UnixStream::connect(&path);
        let _ = std::fs::remove_file(&path);
    }
    for c in std::mem::take(&mut state.remote.conns) {
        send(&c.writer, &ToViewer::Bye { reason: "Vergence Remote was turned off on this computer".into() });
        if let Some(v) = c.view {
            crate::drm::remote_stop(state, v);
        }
    }
    if state.lock.curtain {
        state.lock.curtain = false;
    }
    tracing::info!("remote: off (Privacy > Remote)");
}

pub fn listen(state: &mut Server) {
    if !enabled() {
        return;
    }
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) else {
        return tracing::warn!("remote: no XDG_RUNTIME_DIR");
    };
    let path = vergence_remote::socket_for(&runtime, &state.socket_name.to_string_lossy());
    if UnixStream::connect(&path).is_ok() {
        return tracing::warn!("remote: {} belongs to a running server", path.display());
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => return tracing::warn!("remote: {}: {e}", path.display()),
    };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    let (tx, rx) = channel::channel::<Event>();
    let res = state.loop_handle.insert_source(rx, |ev, _, state| {
        if let channel::Event::Msg(e) = ev {
            state.remote_event(e);
        }
    });
    if let Err(e) = res {
        return tracing::warn!("remote: {e}");
    }
    let ours = crate::control::our_uid();
    let open = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    state.remote.open = Some(open.clone());
    std::thread::spawn(move || {
        let mut next = 1u32;
        for stream in listener.incoming().flatten() {
            if !open.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            if crate::control::peer_uid(&stream) != Some(ours) {
                tracing::warn!("remote: a connection from another user, refused");
                continue;
            }
            let program = crate::control::peer_pid(&stream)
                .and_then(vergence_dm::elevate::program_path)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "?".into());
            let (conn, tx) = (next, tx.clone());
            next = next.wrapping_add(1).max(1);
            std::thread::spawn(move || serve(conn, stream, program, tx));
        }
    });
    tracing::info!("remote {}", path.display());
    state.remote.path = Some(path);
}

/// One connection's reading side (and its writer thread).
fn serve(conn: u32, stream: UnixStream, program: String, tx: channel::Sender<Event>) {
    let Ok(mut w) = stream.try_clone() else { return };
    let mut r = std::io::BufReader::new(stream);
    if let Err(e) = proto::write_opening(&mut w).map_err(proto::Error::Io).and_then(|_| proto::read_opening(&mut r)) {
        let _ = tx.send(Event::Closed { conn, why: e.to_string() });
        return;
    }
    let (wtx, wrx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        for bytes in wrx {
            if w.write_all(&bytes).and_then(|_| w.flush()).is_err() {
                break;
            }
        }
        let _ = w.shutdown(std::net::Shutdown::Both);
    });
    if tx.send(Event::Opened { conn, writer: wtx, program }).is_err() {
        return;
    }
    loop {
        match ToHost::read(&mut r) {
            Ok(msg) => {
                if tx.send(Event::Message { conn, msg }).is_err() {
                    return;
                }
            }
            Err(e) => {
                let why = match e {
                    proto::Error::Io(ref io) if io.kind() == std::io::ErrorKind::UnexpectedEof => "gone".into(),
                    e => e.to_string(),
                };
                let _ = tx.send(Event::Closed { conn, why });
                return;
            }
        }
    }
}

fn send(writer: &mpsc::Sender<Vec<u8>>, m: &ToViewer) {
    let mut b = Vec::new();
    if m.write(&mut b).is_ok() {
        let _ = writer.send(b);
    }
}

fn host_name() -> String {
    let mut buf = [0u8; 256];
    unsafe extern "C" {
        fn gethostname(name: *mut std::ffi::c_char, len: usize) -> i32;
    }
    // SAFETY: gethostname(3) into a buffer of its length.
    if unsafe { gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return "this computer".into();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

impl Server {
    pub fn remote_event(&mut self, e: Event) {
        match e {
            Event::Opened { conn, writer, program } => {
                tracing::info!("remote: connection {conn} ({program})");
                self.remote.conns.push(Conn { id: conn, view: None, writer, ready: false, seq: 0, program });
            }
            Event::Closed { conn, why } => {
                tracing::info!("remote: connection {conn} closed ({why})");
                self.remote_close(conn);
            }
            Event::Message { conn, msg } => self.remote_message(conn, msg),
        }
    }

    fn remote_message(&mut self, conn: u32, msg: ToHost) {
        let Some(c) = self.remote.conns.iter().position(|c| c.id == conn) else { return };
        match msg {
            ToHost::Hello { name, width, height, screen } if self.remote.conns[c].view.is_none() => {
                let screens: Vec<(String, u32, u32)> = self
                    .space
                    .outputs()
                    .map(|o| {
                        let g = self.space.output_geometry(o).unwrap_or_default();
                        (o.name(), g.size.w.max(0) as u32, g.size.h.max(0) as u32)
                    })
                    .collect();
                // The main screen: the one at the layout's origin, else the first.
                let main = self
                    .space
                    .outputs()
                    .find(|o| self.space.output_geometry(o).is_some_and(|g| g.loc.x == 0 && g.loc.y == 0))
                    .or(self.space.outputs().next())
                    .map(|o| o.name())
                    .unwrap_or_default();
                let source = if screen.is_empty() { main } else { screen };
                let writer = self.remote.conns[c].writer.clone();
                match crate::drm::remote_start(self, &source, (width, height), &name) {
                    Ok(id) => {
                        if let Some(c) = self.remote.conns.iter_mut().find(|c| c.id == conn) {
                            c.view = Some(id);
                            c.ready = true;
                        }
                        tracing::info!("remote: connection {conn} is {name:?}, view {id} of {source}");
                        send(&writer, &ToViewer::Welcome { host: host_name(), screens });
                    }
                    Err(e) => {
                        send(&writer, &ToViewer::Bye { reason: e });
                        self.remote_close(conn);
                    }
                }
            }
            ToHost::Hello { .. } => {}
            ToHost::Resize { width, height } => {
                if let Some(v) = self.remote.conns[c].view {
                    crate::drm::remote_resize(self, v, (width, height));
                }
            }
            ToHost::Ack { seq } => {
                let c = &mut self.remote.conns[c];
                if seq == c.seq {
                    c.ready = true;
                }
            }
            ToHost::Key { .. }
            | ToHost::Pointer { .. }
            | ToHost::Button { .. }
            | ToHost::Scroll { .. }
            | ToHost::Locks { .. } => {
                if let Some(v) = self.remote.conns[c].view {
                    self.remote_input(v, msg);
                }
            }
            // Blank the desk, or lift the curtain (version 2): the desk's
            // monitors locked while this viewer goes on; the answer says
            // whether the desk is blanked now.
            ToHost::Privacy { on } => {
                let writer = self.remote.conns[c].writer.clone();
                if on && !self.lock.on {
                    if !self.lock_curtain() {
                        send(&writer, &ToViewer::Notice { text: "The desk could not be blanked".into() });
                    }
                } else if on {
                    if !self.lock.curtain {
                        send(&writer, &ToViewer::Notice { text: "The desk is locked already".into() });
                    }
                } else {
                    self.lift_curtain();
                }
                send(&writer, &ToViewer::Privacy { on: self.lock.on && self.lock.curtain });
            }
            ToHost::Bye => self.remote_close(conn),
        }
    }

    /// The viewer's keyboard, pointer, buttons and scrolling, as the
    /// machine's own (step 4): the same paths vctl's input takes, so the seat
    /// decides where each goes. Counts as activity (energy.rs). Not while the
    /// server's elevation prompt is up: that is answered at the machine.
    fn remote_input(&mut self, view: u32, msg: ToHost) {
        use smithay::backend::input::InputTime;
        if self.elevating() {
            return;
        }
        self.energy_activity();
        let now = InputTime::now();
        // Behind the curtain the viewer's input passes, one event at a time:
        // the seat's lock set aside around it and set again (the desk's own
        // input meets the lock, lock.rs `locked_for_input`). A lock of any
        // other kind is met: the keys reach its password field.
        let curtain = self.lock.on && self.lock.curtain;
        if curtain {
            self.remote.passing = true;
            self.input.seat.set_locked_quietly(false);
        }
        self.remote_input_now(view, msg, now);
        if curtain {
            self.input.seat.set_locked_quietly(true);
            self.remote.passing = false;
        }
    }

    fn remote_input_now(&mut self, view: u32, msg: ToHost, now: smithay::backend::input::InputTime) {
        use smithay::backend::input::{Axis, AxisSource, ButtonState, KeyState};
        match msg {
            ToHost::Pointer { x, y } => {
                if let Some(p) = crate::drm::remote_to_logical(self, view, x, y) {
                    self.touch_mouse_back();
                    self.pointer_to(p, now);
                }
            }
            // Mouse buttons only (evdev BTN_LEFT .. BTN_TASK).
            ToHost::Button { code, pressed } if (0x110..=0x117).contains(&code) => {
                let state = if pressed { ButtonState::Pressed } else { ButtonState::Released };
                self.handle_button(code, state, now);
            }
            // Keyboard keys only (evdev codes below BTN_MISC), as the server's keycodes (+8).
            ToHost::Key { code, pressed } if code > 0 && code < 0x100 => {
                let state = if pressed { KeyState::Pressed } else { KeyState::Released };
                self.handle_key((code + 8).into(), state, now);
            }
            // The viewer's Caps Lock and Num Lock: this machine's own lock key
            // pressed where they differ (the lights say which are on), so every
            // keyboard state here (the seat's, Smithay's, the apps') follows.
            ToHost::Locks { caps, num } => {
                let Some((num_on, caps_on, _)) = self.input.keys.as_ref().map(|k| k.leds()) else { return };
                for (want, on, code) in [(caps, caps_on, 58u32), (num, num_on, 69u32)] {
                    if want != on {
                        self.handle_key((code + 8).into(), KeyState::Pressed, now);
                        self.handle_key((code + 8).into(), KeyState::Released, now);
                    }
                }
            }
            ToHost::Scroll { dx, dy } => {
                let (dx, dy) = (dx.clamp(-10_000.0, 10_000.0) as f64, dy.clamp(-10_000.0, 10_000.0) as f64);
                let mut frame = smithay::input::pointer::AxisFrame::new(now).source(AxisSource::Continuous);
                if dx != 0.0 {
                    frame = frame.value(Axis::Horizontal, dx);
                }
                if dy != 0.0 {
                    frame = frame.value(Axis::Vertical, dy);
                }
                self.pointer_axis(frame, 2, dx, dy, 0, 0, now);
            }
            _ => {}
        }
    }

    fn remote_close(&mut self, conn: u32) {
        let Some(i) = self.remote.conns.iter().position(|c| c.id == conn) else { return };
        let c = self.remote.conns.remove(i);
        if let Some(v) = c.view {
            crate::drm::remote_stop(self, v);
        }
        // The last viewer gone behind the curtain: the desk stays locked, as
        // any lock (whoever comes back needs the password; no later viewer
        // passes it).
        if self.remote.conns.is_empty() && self.lock.curtain {
            self.lock.curtain = false;
            tracing::info!("remote: the curtain stays as a lock (its viewer left)");
        }
        tracing::info!("remote: connection {} ({}) ended", c.id, c.program);
    }

    /// Each connection that may take a frame gets what changed (a pull of
    /// its view), and one whose view was ended from the desk is told and
    /// closed. Called by the remote views' clock and after End.
    pub fn remote_service(&mut self) {
        let live = crate::drm::remote_ids(self);
        let ended: Vec<u32> =
            self.remote.conns.iter().filter(|c| c.view.is_some_and(|v| !live.contains(&v))).map(|c| c.id).collect();
        for conn in ended {
            if let Some(c) = self.remote.conns.iter().find(|c| c.id == conn) {
                send(&c.writer, &ToViewer::Notice { text: "Ended from the desk".into() });
                send(&c.writer, &ToViewer::Bye { reason: "ended from the desk".into() });
            }
            if let Some(i) = self.remote.conns.iter().position(|c| c.id == conn) {
                let c = self.remote.conns.remove(i);
                tracing::info!("remote: connection {} ended from the desk", c.id);
            }
        }
        let ready: Vec<(u32, u32)> =
            self.remote.conns.iter().filter(|c| c.ready).filter_map(|c| c.view.map(|v| (c.id, v))).collect();
        for (conn, view) in ready {
            let Some(Ok(frame)) = crate::drm::remote_pull(self, view) else { continue };
            if frame.rects.is_empty() {
                continue;
            }
            let Some(c) = self.remote.conns.iter_mut().find(|c| c.id == conn) else { continue };
            c.seq = c.seq.wrapping_add(1);
            c.ready = false;
            let rects = frame
                .rects
                .into_iter()
                .map(|(x, y, w, h, data)| Rect { x, y, w, h, encoding: Encoding::Raw, data })
                .collect();
            let m = ToViewer::Frame(Frame {
                seq: c.seq,
                width: frame.width,
                height: frame.height,
                scale: frame.scale as f32,
                rects,
            });
            send(&c.writer, &m);
        }
    }
}
