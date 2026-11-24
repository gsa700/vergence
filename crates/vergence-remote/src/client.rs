//! The viewer's side of a connection, with no platform code (remote
//! milestone 1, step 4: the portable core every viewer shares; a shell per
//! platform draws [`Shared::picture`] and feeds input to [`Session::send`]).
//!
//! A session runs a command that reaches a host's `vergence-remote` (`ssh
//! <host> vergence-remote`; `vergence-remote` itself for a session on this
//! machine), says Hello, and keeps a picture up to date from the frames: each
//! applied (checked, whole or not at all: proto.rs), acknowledged, and the
//! changed rectangles noted for the shell, which is woken to draw them.

use std::io::{BufReader, BufWriter, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::proto::{self, ToHost, ToViewer};

/// Where a session is.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Connecting,
    Live,
    /// It ended: why, in words ("ended from the desk", "ssh: no route").
    Ended(String),
}

/// What the session's thread keeps for the shell, under a lock.
#[derive(Debug)]
pub struct Shared {
    pub state: State,
    /// The host's name and screens, from its Welcome.
    pub host: String,
    pub screens: Vec<(String, u32, u32)>,
    /// RGBA, `size` pixels, as the host drew it.
    pub picture: Vec<u8>,
    pub size: (u32, u32),
    pub scale: f32,
    /// Rectangles changed since the shell last took them (x, y, w, h); a
    /// new size makes the whole picture changed.
    pub dirty: Vec<(u32, u32, u32, u32)>,
    pub frames: u64,
    /// The last notice from the host, to show the person.
    pub notice: Option<String>,
    /// The host's desk is blanked (version 2).
    pub privacy: bool,
}

pub struct Session {
    pub shared: Arc<Mutex<Shared>>,
    tx: mpsc::Sender<ToHost>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Session {
    /// A session through `command` (the program and its arguments), named
    /// `name` on the host's tile, for a view of `size` pixels at this
    /// machine's `scale`; `wake` is called from the session's thread when
    /// there is something to draw.
    pub fn connect(
        command: &[String],
        name: &str,
        size: (u32, u32),
        scale: f32,
        wake: impl Fn() + Send + 'static,
    ) -> Session {
        let shared = Arc::new(Mutex::new(Shared {
            state: State::Connecting,
            host: String::new(),
            screens: Vec::new(),
            picture: Vec::new(),
            size: (0, 0),
            scale: 1.0,
            dirty: Vec::new(),
            frames: 0,
            notice: None,
            privacy: false,
        }));
        let (tx, rx) = mpsc::channel::<ToHost>();
        let child: Arc<Mutex<Option<Child>>> = Arc::default();
        let (s, c, name, command) = (shared.clone(), child.clone(), name.to_string(), command.to_vec());
        std::thread::spawn(move || {
            let why = run(&command, &name, size, scale, &s, &c, rx, &wake);
            if let Ok(mut sh) = s.lock()
                && !matches!(sh.state, State::Ended(_))
            {
                sh.state = State::Ended(why);
            }
            wake();
        });
        Session { shared, tx, child }
    }

    /// Input, a resize or Bye to the host (dropped once the session ended).
    pub fn send(&self, m: ToHost) {
        let _ = self.tx.send(m);
    }

    /// Ends the session: Bye, then the command is stopped.
    pub fn close(&self) {
        self.send(ToHost::Bye);
        if let Some(mut ch) = self.child.lock().ok().and_then(|mut c| c.take()) {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let _ = ch.kill();
                let _ = ch.wait();
            });
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

/// The session's whole life; what ended it, in words.
#[allow(clippy::too_many_arguments)]
fn run(
    command: &[String],
    name: &str,
    size: (u32, u32),
    scale: f32,
    shared: &Arc<Mutex<Shared>>,
    child_slot: &Arc<Mutex<Option<Child>>>,
    rx: mpsc::Receiver<ToHost>,
    wake: &impl Fn(),
) -> String {
    let Some((prog, args)) = command.split_first() else { return "no command".into() };
    let mut child =
        match Command::new(prog).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        {
            Ok(c) => c,
            Err(e) => return format!("{prog}: {e}"),
        };
    let (Some(stdin), Some(stdout), stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        return "no pipes".into();
    };
    // What the command says on its error output (ssh's "Permission denied"),
    // the last line kept for when the stream ends without a word.
    let said: Arc<Mutex<String>> = Arc::default();
    if let Some(err) = stderr {
        let said = said.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                if !line.trim().is_empty() {
                    *said.lock().unwrap_or_else(|e| e.into_inner()) = line.trim().to_string();
                }
            }
        });
    }
    if let Ok(mut c) = child_slot.lock() {
        *c = Some(child);
    }
    let mut w = BufWriter::new(stdin);
    let mut r = BufReader::with_capacity(1 << 20, stdout);
    let ended = |e: proto::Error| -> String {
        let s = said.lock().map(|s| s.clone()).unwrap_or_default();
        if !s.is_empty() { s } else { e.to_string() }
    };
    if let Err(e) = proto::write_opening(&mut w).map_err(proto::Error::Io).and_then(|_| proto::read_opening(&mut r)) {
        return ended(e);
    }
    let hello = ToHost::Hello {
        name: name.to_string(),
        width: size.0,
        height: size.1,
        screen: String::new(),
        scale: scale.clamp(0.5, 4.0),
    };
    if hello.write(&mut w).and_then(|_| w.flush()).is_err() {
        return "the connection closed".into();
    }
    // The writer: the shell's messages and the reader's Acks, in order.
    let (ack_tx, ack_rx) = mpsc::channel::<ToHost>();
    std::thread::spawn(move || {
        loop {
            let m = match ack_rx.try_recv() {
                Ok(m) => m,
                Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => match rx.recv_timeout(std::time::Duration::from_millis(5)) {
                    Ok(m) => m,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                },
            };
            let bye = m == ToHost::Bye;
            if m.write(&mut w).and_then(|_| w.flush()).is_err() || bye {
                break;
            }
        }
    });
    loop {
        let m = match ToViewer::read(&mut r) {
            Ok(m) => m,
            Err(e) => return ended(e),
        };
        match m {
            ToViewer::Welcome { host, screens } => {
                if let Ok(mut s) = shared.lock() {
                    s.host = host;
                    s.screens = screens;
                    s.state = State::Live;
                }
                wake();
            }
            ToViewer::Frame(f) => {
                let applied = {
                    let Ok(mut s) = shared.lock() else { return "the picture's lock".into() };
                    let Shared { picture, size, .. } = &mut *s;
                    let before = *size;
                    match f.apply(picture, size) {
                        Ok(()) => {
                            if *size != before {
                                s.dirty = vec![(0, 0, f.width, f.height)];
                            } else {
                                s.dirty.extend(f.rects.iter().map(|r| (r.x, r.y, r.w, r.h)));
                            }
                            s.scale = f.scale;
                            s.frames += 1;
                            s.state = State::Live;
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                };
                if let Err(e) = applied {
                    return e.to_string();
                }
                let _ = ack_tx.send(ToHost::Ack { seq: f.seq });
                wake();
            }
            ToViewer::Notice { text } => {
                if let Ok(mut s) = shared.lock() {
                    s.notice = Some(text);
                }
                wake();
            }
            ToViewer::Privacy { on } => {
                if let Ok(mut s) = shared.lock() {
                    s.privacy = on;
                }
                wake();
            }
            ToViewer::Bye { reason } => return reason,
        }
    }
}

/// The command that reaches `host`'s Vergence Remote: `ssh` with no
/// questions it could not answer here (a password prompt has no terminal),
/// or this machine's own `vergence-remote` for "localhost".
pub fn command_for(host: &str) -> Vec<String> {
    if host == "localhost" || host.is_empty() {
        return vec!["vergence-remote".into()];
    }
    vec![
        "ssh".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "--".into(),
        host.into(),
        "vergence-remote".into(),
    ]
}
