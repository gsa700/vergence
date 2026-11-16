//! The lock screen (VDM milestone 1, step 1; docs/vdm-milestone-1.md). The
//! server is the lock: no locker program whose crash could leave the
//! desktop open. While locked:
//!
//! - every key but the volume's and console switching, every press, motion,
//!   wheel and finger is the lock's (the seat's lock state, checked first,
//!   review I6); no window has the keyboard or the pointer, and no program's
//!   own cursor is drawn;
//! - each screen shows only the wallpaper, dimmed, and the lock (decor.rs's
//!   scene asks [`Server::lock_elements`] and nothing else), so no window,
//!   notification or menu is ever drawn, nor in a `vctl shot`;
//! - the password is checked by vdm, the one part that runs as root, over
//!   its unlock socket (vergence-dm's protocol), in a thread so the screen
//!   never waits; PAM's prompts and messages are shown as they come; the
//!   field takes typed keys only (no paste: Dave, 2026-11-16).
//!
//! It never shows a lock that is not one (threat model T18, review C2): it
//! locks only when vdm's socket is there and the session cannot fall back to
//! a shell (started by `exec vergence` or by a display manager); otherwise it
//! says why and stays unlocked. A marker in the runtime directory says the
//! session is locked: the session script does not start a server again after
//! one that ended locked, and a server that finds the marker starts locked
//! (review C1).

use std::hash::{Hash, Hasher};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Id, Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::SolidColorRenderElement,
            },
            gles::GlesRenderer,
            utils::CommitCounter,
        },
    },
    input::pointer::CursorImageStatus,
    output::Output,
    reexports::calloop::channel::{self, Event},
    utils::{Logical, Rectangle, SERIAL_COUNTER, Transform},
};
use vergence_dm::proto::{self, FromVdm, Service, ToVdm};
use vergence_dm::secret::Secret;
use vergence_scene::{Color, Point as ScenePoint, Rect as SceneRect, Scene};

use crate::{Server, decor::SceneElement, win::Win};

/// What the thread talking to vdm heard.
pub(crate) enum Heard {
    Prompt { echo: bool, text: String },
    Info(String),
    Error(String),
    Done { ok: bool, wait: u16 },
    /// vdm could not be reached, or the connection broke.
    Gone(String),
    /// vdm, on the session socket: the machine is about to sleep (step 2).
    LockNow,
    /// vdm, on the session socket: show the elevation prompt (elevate.rs).
    Ask { id: u32, what: String, who: String },
    /// vdm: that prompt is over.
    Withdrawn(u32),
}

/// What the server answers vdm before sleep.
enum Reply {
    Presented,
    Cannot(String),
}

/// One conversation with vdm in progress.
struct Talk {
    answers: std::sync::mpsc::Sender<Option<Secret>>,
    /// vdm has asked and waits for an answer.
    asked: bool,
}

#[derive(Default)]
pub struct Lock {
    pub on: bool,
    field: Option<Secret>,
    /// PAM's prompt (shown above the field) and whether what is typed shows.
    prompt: String,
    echo: bool,
    /// The last message: PAM's, or ours, and whether it is an error.
    message: Option<(String, bool)>,
    talk: Option<Talk>,
    /// Enter was pressed before vdm asked: the answer goes when it does.
    submit: bool,
    /// An answer is with vdm.
    waiting: bool,
    pause_until: Option<Instant>,
    /// The window that had the keyboard when it locked.
    before: Option<Win>,
    heard: Option<channel::Sender<Heard>>,
    /// Idle could not lock and has said so once this session.
    told_idle: bool,
    /// The answer to vdm's LockNow, sent by the session thread.
    replies: Option<std::sync::mpsc::Sender<Reply>>,
    /// vdm waits to hear the lock is on every screen.
    awaiting: bool,
    /// Since it locked: the screens a frame was queued for, and those it was
    /// shown on (their VBlank came); when all have, the lock is presented.
    queued: std::collections::HashSet<String>,
    shown: std::collections::HashSet<String>,
    pub presented: bool,
    dim: Option<(Id, CommitCounter)>,
    picture: Option<(u64, MemoryRenderBuffer, i32, i32)>,
}

/// The marker that says the session is locked, in the runtime directory.
fn marker() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|d| std::path::PathBuf::from(d).join("vergence-locked"))
}

/// The process names a session may run under without a shell above it
/// (review C2): login(1) after `exec vergence`, vdm, and display managers
/// that start sessions themselves (names cut at 15, as the system keeps them).
/// dbus-run-session is passed through to its own parent, which must be one
/// of these (security review 2026-11-16, finding 5: typed in a logged-in
/// shell, it returned to that shell when the server crashed).
const SAFE_PARENTS: [&str; 6] = ["login", "vdm", "gdm-wayland-ses", "gdm-x-session", "greetd", "sddm-helper"];
const PASS_THROUGH: [&str; 1] = ["dbus-run-sessio"];

fn ps(field: &str, pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps").args(["-o", &format!("{field}="), "-p", &pid.to_string()]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Whether nothing but a login or a display manager is above the session
/// script: a server that ends then ends the session, never returning to a
/// shell that is logged in. The server's parent is the session script; its
/// parent is what is asked about.
fn no_shell_above() -> Result<(), String> {
    let script = std::os::unix::process::parent_id();
    let mut above = ps("ppid", script).and_then(|p| p.parse::<u32>().ok()).ok_or("the session's parent is not known")?;
    for _ in 0..4 {
        let name = ps("comm", above).unwrap_or_default();
        let name = name.rsplit('/').next().unwrap_or(&name).trim_start_matches('-').to_string();
        if SAFE_PARENTS.contains(&name.as_str()) {
            return Ok(());
        }
        if !PASS_THROUGH.contains(&name.as_str()) {
            return Err(format!("the session was started from {name}, which a crash would return to, logged in"));
        }
        above = ps("ppid", above).and_then(|p| p.parse::<u32>().ok()).ok_or("the session's parent is not known")?;
    }
    Err("the session's parents could not be followed".into())
}

/// The session's standing connection to vdm (step 2, review C4): vdm learns
/// the account from the kernel; when the machine is about to sleep it says
/// LockNow, and this answers once the lock is on every screen. Reconnects
/// when vdm restarts.
fn session(heard: channel::Sender<Heard>, replies: std::sync::mpsc::Receiver<Reply>) {
    loop {
        let Ok(mut s) = UnixStream::connect(vergence_dm::paths::SESSION_SOCKET) else {
            std::thread::sleep(Duration::from_secs(5));
            continue;
        };
        tracing::info!("lock: registered with vdm for sleep");
        loop {
            match proto::read_from_vdm(&mut s) {
                Ok(FromVdm::LockNow) => {
                    while replies.try_recv().is_ok() {}
                    if heard.send(Heard::LockNow).is_err() {
                        return;
                    }
                    let msg = match replies.recv_timeout(Duration::from_secs(6)) {
                        Ok(Reply::Presented) => ToVdm::Presented,
                        Ok(Reply::Cannot(why)) => ToVdm::CannotLock(why),
                        Err(_) => continue,
                    };
                    if proto::send_to_vdm(&mut s, &msg).is_err() {
                        break;
                    }
                }
                Ok(FromVdm::Ask { id, what, who }) => {
                    if heard.send(Heard::Ask { id, what, who }).is_err() {
                        return;
                    }
                }
                Ok(FromVdm::Withdrawn(id)) => {
                    if heard.send(Heard::Withdrawn(id)).is_err() {
                        return;
                    }
                }
                Ok(_) | Err(_) => break,
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn talk(heard: channel::Sender<Heard>, answers: std::sync::mpsc::Receiver<Option<Secret>>) {
    converse(heard, answers, vergence_dm::paths::UNLOCK_SOCKET, vec![ToVdm::Hello { version: proto::VERSION, service: Service::Unlock }]);
}

/// One PAM conversation with vdm on `socket`, opened with `first` (the lock's
/// Hello; the login screen's Hello and Login): PAM's prompts and messages go
/// to the main loop through `heard`, the answers come back through `answers`.
pub(crate) fn converse(heard: channel::Sender<Heard>, answers: std::sync::mpsc::Receiver<Option<Secret>>, socket: &str, first: Vec<ToVdm>) {
    let mut s = match UnixStream::connect(socket) {
        Ok(s) => s,
        Err(e) => {
            let _ = heard.send(Heard::Gone(format!("vdm: {e}")));
            return;
        }
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(150)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
    for m in &first {
        if proto::send_to_vdm(&mut s, m).is_err() {
            let _ = heard.send(Heard::Gone("vdm closed the connection".into()));
            return;
        }
    }
    loop {
        match proto::read_from_vdm(&mut s) {
            Ok(FromVdm::Prompt { echo, text }) => {
                if heard.send(Heard::Prompt { echo, text }).is_err() {
                    return;
                }
                match answers.recv() {
                    Ok(Some(secret)) => {
                        if proto::send_to_vdm(&mut s, &ToVdm::Answer(secret)).is_err() {
                            let _ = heard.send(Heard::Gone("vdm closed the connection".into()));
                            return;
                        }
                    }
                    _ => {
                        let _ = proto::send_to_vdm(&mut s, &ToVdm::Cancel);
                        return;
                    }
                }
            }
            Ok(FromVdm::Info(t)) => {
                let _ = heard.send(Heard::Info(t));
            }
            Ok(FromVdm::Error(t)) => {
                let _ = heard.send(Heard::Error(t));
            }
            Ok(FromVdm::Done { ok, wait }) => {
                let _ = heard.send(Heard::Done { ok, wait });
                return;
            }
            // Only on the session socket; here it breaks the protocol.
            Ok(FromVdm::LockNow | FromVdm::Ask { .. } | FromVdm::Withdrawn(_)) => {
                let _ = heard.send(Heard::Gone("vdm: a session's message on the unlock socket".into()));
                return;
            }
            Err(e) => {
                let _ = heard.send(Heard::Gone(format!("vdm: {e}")));
                return;
            }
        }
    }
}

impl Server {
    /// At the start: the channel the conversation threads report on, and a
    /// session that ended locked starts locked (review C1).
    pub fn start_lock(&mut self) {
        let (tx, rx) = channel::channel::<Heard>();
        let res = self.loop_handle.insert_source(rx, |event, _, st| {
            if let Event::Msg(h) = event {
                st.lock_heard(h);
            }
        });
        if let Err(e) = res {
            tracing::warn!("lock: {e}");
        }
        // Registered with vdm, to be locked before the machine sleeps (step 2).
        let (rtx, rrx) = std::sync::mpsc::channel();
        self.lock.replies = Some(rtx);
        let heard = tx.clone();
        if std::env::args().any(|a| a == "--drm")
            && let Err(e) = std::thread::Builder::new().name("lock-session".into()).spawn(move || session(heard, rrx))
        {
            tracing::warn!("lock: {e}");
        }
        self.lock.heard = Some(tx);
        // Programs may not attach a debugger to the session's server (threat
        // model T6) nor find a password in a core file (review I1).
        if std::env::args().any(|a| a == "--drm") {
            vergence_dm::sys::no_core_files();
            if !vergence_dm::sys::no_debugger() {
                tracing::warn!("lock: could not refuse debuggers");
            }
            if marker().is_some_and(|m| m.exists()) {
                tracing::warn!("lock: the session was locked when its server ended; it starts locked");
                self.lock_now();
            }
        }
    }

    /// Why the screen could not be locked now, if it could not.
    fn lock_blocker(&self) -> Option<String> {
        use std::os::unix::fs::FileTypeExt;
        // vdm answering, not only its socket's file (finding 9: a stopped vdm
        // left the file, and a lock no one could open).
        let vdm = std::fs::symlink_metadata(vergence_dm::paths::UNLOCK_SOCKET).is_ok_and(|m| m.file_type().is_socket()) && std::os::unix::net::UnixStream::connect(vergence_dm::paths::POWER_SOCKET).is_ok();
        if !vdm {
            return Some("vdm is not running, so nothing could check the password".into());
        }
        no_shell_above().err().map(|why| format!("{why}. Start Vergence with `exec vergence`, or from the login screen"))
    }

    /// Asked to lock (Super+L, the named action, idle, sleep): it locks, or
    /// says plainly why it cannot and leaves the screen visibly unlocked.
    pub fn lock_request(&mut self, why: &str) -> bool {
        if self.lock.on {
            return true;
        }
        // Nothing to lock at the login screen.
        if crate::greeter() {
            return false;
        }
        if let Some(reason) = self.lock_blocker() {
            tracing::warn!("lock: not locked ({why}): {reason}");
            let req = crate::notify::Request {
                app: "Vergence".into(),
                replaces: 0,
                summary: "The screen cannot be locked".into(),
                body: reason,
                urgency: 1,
                timeout_ms: -1,
                actions: Vec::new(),
            };
            self.notify(crate::notify::Origin::Server, req);
            return false;
        }
        tracing::info!("lock: locked ({why})");
        self.lock_now();
        true
    }

    /// Idle long enough for the screens to go off, or going to sleep: locked
    /// if it can be; if not, said once (not at every idle stretch).
    pub fn lock_for_idle(&mut self, why: &str) {
        if self.lock.on || crate::greeter() {
            return;
        }
        if self.lock_blocker().is_some() {
            if !std::mem::replace(&mut self.lock.told_idle, true) {
                self.lock_request(why);
            }
            return;
        }
        self.lock_request(why);
    }

    /// Back on the session's console (review I7): if it was away longer than
    /// the screens' idle time, it is locked before the first frame is drawn.
    pub fn lock_if_idle(&mut self) {
        let battery = self.energy.on_battery.load(std::sync::atomic::Ordering::Relaxed);
        let (off, _) = self.energy.config.times(battery);
        let after = self.energy.config.lock_after;
        let idle = self.energy.last_input.elapsed();
        if (self.energy.config.lock_on_screens_off && !off.is_zero() && idle >= off) || (!after.is_zero() && idle >= after) {
            self.lock_for_idle("back after the idle time");
        }
    }

    /// vdm says the machine is about to sleep: lock (or say why not), and
    /// answer once the lock is on every screen.
    fn lock_for_sleep(&mut self) {
        let reply = |st: &mut Server, r: Reply| {
            if let Some(tx) = st.lock.replies.as_ref() {
                let _ = tx.send(r);
            }
        };
        // [lock] before-sleep = false: the person chose a machine that wakes
        // into the session; vdm is told so (it logs it), not shown a lock.
        if !self.lock.on && !self.energy.config.lock_before_sleep {
            return reply(self, Reply::Cannot("lock before sleep is off in the settings".into()));
        }
        if !self.lock.on {
            if let Some(why) = self.lock_blocker() {
                self.lock_request("sleep");
                return reply(self, Reply::Cannot(why));
            }
            tracing::info!("lock: locked (the machine is going to sleep)");
            self.lock_now();
        }
        // Screens off: their CRTCs are cleared, nothing of the session is on
        // them; already shown everywhere: nothing more to wait for.
        if self.energy.screens_off || self.lock.presented {
            return reply(self, Reply::Presented);
        }
        self.lock.awaiting = true;
        self.redraw_all();
    }

    /// drm.rs: a frame for a screen was queued (it shows at that screen's VBlank).
    pub fn lock_frame_queued(&mut self, screen: String) {
        if self.lock.on && !self.lock.presented {
            self.lock.queued.insert(screen);
        }
    }

    /// drm.rs: a screen's VBlank: a frame queued since it locked is on it.
    /// When every screen shows the lock, it is presented (vdm is told, if it waits).
    pub fn lock_frame_shown(&mut self, screen: &str, screens: usize) {
        if !self.lock.on || self.lock.presented || !self.lock.queued.contains(screen) {
            return;
        }
        self.lock.shown.insert(screen.to_string());
        if self.lock.shown.len() >= screens {
            self.lock.presented = true;
            tracing::info!("lock: shown on every screen");
            if std::mem::take(&mut self.lock.awaiting)
                && let Some(tx) = self.lock.replies.as_ref()
            {
                let _ = tx.send(Reply::Presented);
            }
        }
    }

    fn lock_now(&mut self) {
        self.lock.on = true;
        self.lock.presented = false;
        self.lock.queued.clear();
        self.lock.shown.clear();
        if let Some(m) = marker() {
            let _ = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&m).and_then(|mut f| f.write_all(b"locked\n"));
        }
        self.lock.field = Secret::new();
        self.lock.prompt = "Password".into();
        self.lock.echo = false;
        self.lock.message = None;
        self.lock.submit = false;
        self.lock.waiting = false;
        // Nothing of the session's stays open over it or takes input under it.
        let out = self.input.seat.lock();
        let pos = self.pointer_location();
        self.apply(out, pos, smithay::backend::input::InputTime::now(), &crate::dispatch::Came::Nothing);
        self.close_menu();
        self.app_menu_close();
        self.launcher_close();
        self.card_close();
        self.drawer_close();
        self.shelf_cancel();
        // No window keeps the keyboard; no program's own cursor is drawn.
        self.lock.before = self.keyboard_window();
        if let Some(k) = self.seat.get_keyboard() {
            k.set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        if let Some(old) = self.input.keyboard_native.take() {
            self.native_keyboard_leave(&old);
        }
        self.pointer_image.status = CursorImageStatus::default_named();
        self.redraw_all();
    }

    fn unlock_now(&mut self) {
        tracing::info!("lock: unlocked");
        self.lock.on = false;
        self.lock.talk = None;
        self.lock.field = None;
        self.lock.picture = None;
        if let Some(m) = marker() {
            let _ = std::fs::remove_file(m);
        }
        let out = self.input.seat.unlock();
        let pos = self.pointer_location();
        self.apply(out, pos, smithay::backend::input::InputTime::now(), &crate::dispatch::Came::Nothing);
        if let Some(w) = self.lock.before.take().filter(smithay::utils::IsAlive::alive) {
            self.focus_window(&w, SERIAL_COUNTER.next_serial());
        }
        self.redraw_all();
    }

    /// A key while locked: typed into the field, Backspace, Escape (clears),
    /// Enter (answers). Only keys pressed at the machine reach here.
    pub fn lock_key(&mut self, sym: u32, typed: Option<char>, pressed: bool) {
        use xkbcommon::xkb::keysyms as k;
        if !pressed || !self.lock.on {
            return;
        }
        // During vdm's pause the field takes nothing: it is greyed and counts down.
        if self.lock_pause_left() > 0 {
            return;
        }
        // A key is the first sign of someone: the conversation with vdm starts.
        if self.lock.talk.is_none() && !self.lock.waiting {
            self.lock_talk();
        }
        match sym {
            k::KEY_Return | k::KEY_KP_Enter => {
                self.lock.submit = true;
                self.lock_answer();
            }
            k::KEY_BackSpace => {
                if let Some(f) = self.lock.field.as_mut() {
                    f.pop_char();
                }
            }
            k::KEY_Escape => {
                if let Some(f) = self.lock.field.as_mut() {
                    f.clear();
                }
            }
            _ => {
                if let (Some(c), Some(f)) = (typed.filter(|c| !c.is_control()), self.lock.field.as_mut()) {
                    let mut b = [0u8; 4];
                    f.push_str(c.encode_utf8(&mut b));
                }
            }
        }
        self.redraw_all();
    }

    /// Seconds left of vdm's pause after a wrong answer, 0 if none.
    fn lock_pause_left(&self) -> u64 {
        self.lock.pause_until.map_or(0, |u| {
            let left = u.saturating_duration_since(Instant::now());
            left.as_secs() + u64::from(left.subsec_nanos() > 0)
        })
    }

    /// Once a second (settings.rs's tick): the pause counts down on the
    /// screen, and when it is over the dialog asks again.
    pub fn lock_tick(&mut self) {
        if !self.lock.on || self.lock.pause_until.is_none() {
            return;
        }
        if self.lock_pause_left() == 0 {
            self.lock.pause_until = None;
            self.lock.message = None;
        }
        self.redraw_all();
    }

    /// Start a conversation with vdm, in a thread of its own.
    fn lock_talk(&mut self) {
        let Some(heard) = self.lock.heard.clone() else { return };
        let (tx, rx) = std::sync::mpsc::channel();
        self.lock.talk = Some(Talk { answers: tx, asked: false });
        if let Err(e) = std::thread::Builder::new().name("lock".into()).spawn(move || talk(heard, rx)) {
            tracing::warn!("lock: {e}");
            self.lock.talk = None;
        }
    }

    /// Send what is typed, if vdm has asked and Enter was pressed.
    fn lock_answer(&mut self) {
        if let Some(until) = self.lock.pause_until {
            let left = until.saturating_duration_since(Instant::now()).as_secs();
            if left > 0 {
                self.lock.message = Some((format!("Try again in {left} s"), true));
                self.lock.submit = false;
                return;
            }
            self.lock.pause_until = None;
        }
        let Some(t) = self.lock.talk.as_mut() else { return };
        if !t.asked || !self.lock.submit {
            return;
        }
        // The answer goes; a new, empty field takes its place.
        let Some(answer) = std::mem::replace(&mut self.lock.field, Secret::new()) else { return };
        if t.answers.send(Some(answer)).is_ok() {
            t.asked = false;
            self.lock.submit = false;
            self.lock.waiting = true;
            self.lock.message = None;
        }
    }

    fn lock_heard(&mut self, h: Heard) {
        if let Heard::LockNow = h {
            return self.lock_for_sleep();
        }
        if matches!(h, Heard::Ask { .. } | Heard::Withdrawn(_)) {
            return self.elevate_from_vdm(h);
        }
        if !self.lock.on {
            return;
        }
        match h {
            Heard::Prompt { echo, text } => {
                self.lock.prompt = text.trim().trim_end_matches(':').to_string();
                self.lock.echo = echo;
                self.lock.waiting = false;
                if let Some(t) = self.lock.talk.as_mut() {
                    t.asked = true;
                }
                self.lock_answer();
            }
            Heard::Info(t) => self.lock.message = Some((t, false)),
            Heard::Error(t) => self.lock.message = Some((t, true)),
            Heard::LockNow | Heard::Ask { .. } | Heard::Withdrawn(_) => {}
            Heard::Done { ok: true, .. } => return self.unlock_now(),
            Heard::Done { ok: false, wait } => {
                self.lock.talk = None;
                self.lock.waiting = false;
                if let Some(f) = self.lock.field.as_mut() {
                    f.clear();
                }
                self.lock.prompt = "Password".into();
                self.lock.echo = false;
                if wait > 0 {
                    self.lock.pause_until = Some(Instant::now() + Duration::from_secs(wait as u64));
                    self.lock.message = Some((format!("Try again in {wait} s"), true));
                } else {
                    self.lock.message = Some(("That was not right. Try again.".into(), true));
                }
            }
            // vdm went away (review I8): still locked; the next key tries again.
            Heard::Gone(why) => {
                tracing::warn!("lock: {why}");
                self.lock.talk = None;
                self.lock.waiting = false;
                self.lock.message = Some(("Waiting for vdm, which checks the password…".into(), true));
            }
        }
        self.redraw_all();
    }

    /// While locked, all a screen shows: the lock (on the screen the
    /// pointer is on), a dimming over the wallpaper, the wallpaper.
    pub fn lock_elements(&mut self, renderer: &mut GlesRenderer, output: &Output, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        let mut elements = Vec::new();
        let here = out.to_f64().contains(self.pointer_location());
        if here && let Some(e) = self.lock_panel(renderer, out, scale) {
            elements.push(e);
        }
        let (id, commit) = self.lock.dim.get_or_insert_with(|| (Id::new(), CommitCounter::default())).clone();
        let phys = Rectangle::<i32, smithay::utils::Physical>::from_size(out.size.to_f64().to_physical(scale).to_i32_round());
        elements.push(SceneElement::Solid(SolidColorRenderElement::new(id, phys, commit, [0.0, 0.0, 0.0, 0.55], Kind::Unspecified)));
        if let Some(mode) = output.current_mode()
            && let Some(e) = self.wallpaper_element(renderer, &output.name(), mode.size, out.size)
        {
            elements.push(SceneElement::Memory(e));
        }
        elements
    }

    fn lock_panel(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Option<SceneElement> {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let (local, _) = vergence_shell::tiles::now_at(secs)?;
        let time = format!("{:02}:{:02}", local.hour, local.min);
        let user = std::env::var("USER").unwrap_or_default();
        let dots = self.lock.field.as_ref().map_or(0, |f| f.chars());
        let paused = self.lock_pause_left();
        let shown = if self.lock.echo { self.lock.field.as_ref().map(|f| String::from_utf8_lossy(f.as_bytes()).into_owned()) } else { None };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&time, &user, dots, &shown, &self.lock.prompt, &self.lock.message, self.lock.waiting, paused, scale.to_bits()).hash(&mut h);
        let key = h.finish();
        if self.lock.picture.as_ref().map(|(k, ..)| *k) != Some(key) {
            let s = vergence_look::shelf_look(false);
            let ink = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            let face = fonts.face("sans-serif", 400, false)?;
            let bold = fonts.face("sans-serif", 600, false).unwrap_or(face);
            let (w, hgt) = (380.0f32, 250.0f32);
            let mut scene = Scene::new(w, hgt);
            scene.fill_rounded(SceneRect::new(0.0, 0.0, w, hgt), vergence_look::RADIUS * 2.0, Color::rgba(s.menu.0, s.menu.1, s.menu.2, 235));
            scene.stroke(SceneRect::new(0.0, 0.0, w, hgt), vergence_look::RADIUS * 2.0, vergence_look::EDGE, ink(s.edge));
            let centered = |fonts: &mut vergence_text::Fonts, scene: &mut Scene, f, size: f32, color, y: f32, t: &str| {
                let m = fonts.metrics(f, size);
                let tw = fonts.layout(f, size, t).1;
                vergence_render::text(scene, fonts, f, size, color, ScenePoint::new(((w - tw) / 2.0).round(), (y + m.ascent).round()), t);
            };
            centered(fonts, &mut scene, face, 52.0, ink(s.menu_ink), 18.0, &time);
            centered(fonts, &mut scene, bold, 15.0, ink(s.menu_ink), 92.0, &user);
            centered(fonts, &mut scene, face, 12.0, ink(s.menu_ink_off), 118.0, &self.lock.prompt);
            // The field: dots for what is typed (or the text, where PAM asks with echo).
            let field = SceneRect::new(40.0, 140.0, w - 80.0, 34.0);
            // Greyed during the pause: no accent, faint, nothing typed shows.
            if paused > 0 {
                scene.fill_rounded(field, vergence_look::RADIUS, Color::rgba(s.field.0, s.field.1, s.field.2, 110));
                scene.stroke(field, vergence_look::RADIUS, vergence_look::EDGE, ink(s.edge));
            } else {
                scene.fill_rounded(field, vergence_look::RADIUS, ink(s.field));
                scene.stroke(field, vergence_look::RADIUS, vergence_look::EDGE, color_accent());
            }
            match &shown {
                Some(t) => {
                    let m = fonts.metrics(face, 14.0);
                    vergence_render::text(&mut scene, fonts, face, 14.0, ink(s.menu_ink), ScenePoint::new(field.x + 12.0, (field.y + (field.h - m.ascent - m.descent) / 2.0 + m.ascent).round()), t);
                }
                None => {
                    for i in 0..dots.min(24) {
                        scene.fill_rounded(SceneRect::new(field.x + 14.0 + i as f32 * 12.0, field.y + field.h / 2.0 - 4.0, 8.0, 8.0), 4.0, ink(s.menu_ink));
                    }
                }
            }
            let (msg, bad) = match (&self.lock.message, self.lock.waiting) {
                _ if paused > 0 => (format!("Try again in {paused} s"), true),
                (_, true) => ("Checking…".to_string(), false),
                (Some((m, bad)), _) => (m.clone(), *bad),
                (None, _) => ("Locked. Type your password and press Enter.".to_string(), false),
            };
            let color = if bad { Color::rgb(0xff, 0x8a, 0x80) } else { ink(s.menu_ink_off) };
            centered(fonts, &mut scene, face, 12.0, color, 196.0, &msg);
            let p = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)).ok()?;
            let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
            self.lock.picture = Some((key, buffer, w as i32, hgt as i32));
        }
        let (_, buffer, w, hgt) = self.lock.picture.as_ref()?;
        let at = (out.loc.x + (out.size.w - w) / 2, out.loc.y + (out.size.h - hgt) / 2);
        let area: Rectangle<i32, Logical> = Rectangle::new(at.into(), (*w, *hgt).into());
        let a = (area.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let b = (area.loc + area.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(area.size), Kind::Unspecified).ok().map(SceneElement::Memory)
    }
}

fn color_accent() -> Color {
    let a = vergence_look::ACCENT;
    Color::rgb(a.0, a.1, a.2)
}
