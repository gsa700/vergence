// Session restore (whitepaper §6.9; Dave, 2026-11-10): the applications
// running when a session ends are started again when the next one begins,
// and window memory puts their windows back where they were. What an
// application had open is its own to restore (Firefox does).
//
// The server keeps the list of running applications in
// ~/.local/state/vergence/session.toml, written whenever it changes, so it
// survives a crash or a power cut as well as a logout:
//
//     running = ["firefox", "vergence-term"]   # desktop entry ids
//     pending = false    # restored, and not yet lasted a minute
//     strikes = 0        # sessions in a row that ended while pending
//
// Applications are started from their desktop entries, never from a
// remembered command line: what was started from a shell is not restored,
// nor an application with no entry, nor one that wants a terminal.
//
// Not restoring: `[session] restore = false` in desktop.toml; `vergence
// --clean` (VERGENCE_CLEAN) for one start; and by itself after two sessions
// in a row that ended within a minute of their restore (vergence-shell's
// session.rs), so an application that takes the session down as it starts
// cannot do so for ever. Only a session on the real monitors (--drm)
// restores or records: a nested test run does neither.

use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::Server;

/// How long after the session's start the applications are started: the X
/// server for X11 applications is up by then.
const WAIT: Duration = Duration::from_secs(3);
/// How long a restored application counts as running before it has shown a window.
const HOLD: Duration = Duration::from_secs(20);

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Record {
    running: Vec<String>,
    pending: bool,
    strikes: u32,
}

#[derive(Default)]
pub struct Session {
    record: Record,
    /// The start has been dealt with (restored, or not).
    begun: bool,
    /// This run neither restores nor records (not on the real monitors).
    off: bool,
    /// When this session restored, and what.
    restored: Option<(Instant, Vec<String>)>,
    /// What the start came to, for `vctl session`.
    said: String,
    /// An exit is closing the apps (exit.rs): the list stays as it was, so
    /// they are started again at the next session's start.
    pub exiting: bool,
    /// The user's autostart entries have been started (vergence-shell autostart.rs).
    autostarted: bool,
}

fn path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("vergence/session.toml"))
}

fn read() -> Record {
    path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

/// Written beside and moved into place, readable by its owner only.
fn write(record: &Record) -> std::io::Result<()> {
    use std::{
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    };
    let Some(path) = path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let text = toml::to_string(record).map_err(std::io::Error::other)?;
    let beside = path.with_extension("toml.new");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&beside)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&beside, &path)
}

/// `[session] restore` in desktop.toml: true unless it says false.
fn wanted() -> bool {
    crate::wallpaper::desktop_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("session")?.get("restore")?.as_bool())
        .unwrap_or(true)
}

impl Server {
    /// Start the user's autostart entries, each as the dock starts an application.
    fn autostart(&mut self) {
        let Some(dir) = vergence_shell::autostart::dir() else { return };
        for e in vergence_shell::autostart::entries(&dir) {
            tracing::info!("session: autostart {} ({})", e.name, e.id);
            if let Err(err) = crate::spawn(std::process::Command::new(&e.exec[0]).args(&e.exec[1..])) {
                tracing::warn!("session: autostart {}: {err}", e.id);
            }
        }
    }

    /// Once a second: at the session's start, the applications of the last
    /// one are started; after that, the list of what is running is kept.
    pub fn session_tick(&mut self) {
        // The login screen restores nothing and starts nothing (review C3).
        if crate::greeter() {
            return;
        }
        // Once, at the session's start, on the real monitors: what the user
        // put in ~/.config/autostart (dock milestone 1: lab tiles, say),
        // whether or not the last session is restored.
        if !self.resume.autostarted && self.start_time.elapsed() >= WAIT {
            self.resume.autostarted = true;
            if std::env::args().any(|a| a == "--drm") {
                self.autostart();
            }
        }
        if self.resume.off || self.resume.exiting {
            return;
        }
        if !self.resume.begun {
            if self.start_time.elapsed() < WAIT {
                return;
            }
            self.resume.begun = true;
            if !std::env::args().any(|a| a == "--drm") {
                self.resume.off = true;
                self.resume.said = "off: not a session on the real monitors".into();
                return;
            }
            self.session_begin();
            return;
        }
        // A session that has lasted is no longer held against restoring.
        if self.resume.record.pending
            && self
                .resume
                .restored
                .as_ref()
                .is_none_or(|(at, _)| at.elapsed().as_secs() >= vergence_shell::session::SOON)
        {
            self.resume.record.pending = false;
            self.resume.record.strikes = 0;
            self.session_write();
        }
        // What is running now; what was restored counts until it has had time to show a window.
        let mut running = self.shelf_running();
        if let Some((at, ids)) = &self.resume.restored
            && at.elapsed() < HOLD
        {
            for id in ids {
                if !running.contains(id) {
                    running.push(id.clone());
                }
            }
        }
        if running != self.resume.record.running {
            self.resume.record.running = running;
            self.session_write();
        }
    }

    fn session_begin(&mut self) {
        // The desktop's icons (desktop.rs) first, under whatever is restored.
        self.desktop_start();
        // Drives plugged in are mounted (vergence-media: udisks2 on Linux),
        // unless desktop.toml `[media] automount = false`.
        let text = crate::wallpaper::desktop_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        let automount =
            text.parse::<toml::Table>().ok().and_then(|t| t.get("media")?.get("automount")?.as_bool()).unwrap_or(true);
        if automount {
            std::thread::spawn(|| vergence_media::automount(|said| tracing::info!("{said}")));
        }
        let last = read();
        let (restore, strikes) = vergence_shell::session::at_start(last.pending, last.strikes);
        let clean = std::env::var_os("VERGENCE_CLEAN").is_some_and(|v| !v.is_empty());
        let said = if last.running.is_empty() {
            "nothing was running".to_string()
        } else if !restore {
            format!("a clean start: the last {} sessions ended soon after restoring", vergence_shell::session::STRIKES)
        } else if clean {
            "a clean start was asked for".to_string()
        } else if !wanted() {
            "restore is switched off".to_string()
        } else {
            for id in &last.running {
                self.shelf_act(crate::shelf::Act::Start(id.clone()));
            }
            self.resume.restored = Some((Instant::now(), last.running.clone()));
            format!("restored {}", last.running.join(", "))
        };
        tracing::info!("session: {said}");
        let restored = self.resume.restored.is_some();
        self.resume.record = Record {
            running: if restored { last.running } else { Vec::new() },
            pending: restored,
            strikes: if restored { strikes } else { 0 },
        };
        self.resume.said = said;
        self.session_write();
    }

    fn session_write(&mut self) {
        if let Err(e) = write(&self.resume.record) {
            tracing::warn!("session: the list of running applications could not be written: {e}");
        }
    }

    /// For `vctl session`.
    pub fn session_report(&self) -> String {
        let r = &self.resume.record;
        format!(
            "at start: {}\nrunning: {}\npending {} strikes {}",
            if self.resume.begun { self.resume.said.as_str() } else { "not yet" },
            r.running.join(", "),
            r.pending,
            r.strikes
        )
    }
}
