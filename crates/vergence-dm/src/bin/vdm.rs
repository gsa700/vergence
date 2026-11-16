// vdm: the Vergence display manager's root daemon (docs/vdm-milestone-1.md).
// In this first step it does one thing: it checks a password for the lock
// screen of the account that asks, and nothing else.
//
//   vdm [--socket PATH] [--service NAME]
//
// It runs as root, started by the init system (it always runs: question 3).
// The unlock socket (paths::UNLOCK_SOCKET) takes any local user; vdm learns
// who connected from the kernel (getpeereid / SO_PEERCRED), never from the
// request, and checks only that account (threat model T7). Root and system
// accounts are refused (review I10). After a wrong answer an account waits
// before it may try again, longer each time, kept across restarts (T1, T8,
// review M1); one check at a time per account.
//
// Each check runs in a child of its own (review C5), with a time limit: a PAM
// module that blocks (a security key waiting for a touch, a network lookup)
// holds only that child, which is killed when the limit passes. The child
// speaks the protocol (vergence_dm::proto) with the client, runs PAM with our
// own service file (/etc/pam.d/vdm-unlock; vdm refuses to start without it,
// review I2), and exits with what PAM said. Passwords live in wiped,
// locked pages (vergence_dm::secret) and leave with the child.
//
// Every refusal, failure and success goes to standard error, which the
// service sends to the system log (user and time, never what was typed).

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use vergence_dm::pam::{self, Conversation, Outcome};
use vergence_dm::proto::{self, FromVdm, ProtoError, ToVdm};
use vergence_dm::secret::Secret;
use vergence_dm::{paths, sys};

/// How long one check may take, PAM's waits included.
const TIME_LIMIT: c_uint = 120;
/// How long the client has for each message.
const IO_LIMIT: Duration = Duration::from_secs(90);
/// The longest pause after failures, in seconds.
const MAX_PAUSE: u64 = 300;

unsafe extern "C" {
    fn fork() -> c_int;
    fn _exit(status: c_int) -> !;
    fn alarm(secs: c_uint) -> c_uint;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn poll(fds: *mut PollFd, n: u64, timeout: c_int) -> c_int;
    fn getpwuid_r(uid: u32, pwd: *mut c_void, buf: *mut c_char, len: usize, result: *mut *mut c_void) -> c_int;
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

const WNOHANG: c_int = 1;

/// One line to standard error, which the service hands to the system log
/// (syslog or vlogger add the time, the name and the pid).
fn log(msg: &str) {
    eprintln!("{msg}");
}

/// An account's name by its uid (the first field of struct passwd on both
/// Linux and FreeBSD, whose structs differ after it).
fn user_name(uid: u32) -> Option<String> {
    let mut pwd = [0u64; 16];
    let mut buf = vec![0 as c_char; 4096];
    let mut result: *mut c_void = std::ptr::null_mut();
    // SAFETY: a struct passwd fits in 128 bytes on both; buf and its length.
    let r = unsafe { getpwuid_r(uid, pwd.as_mut_ptr().cast(), buf.as_mut_ptr(), buf.len(), &mut result) };
    if r != 0 || result.is_null() {
        return None;
    }
    let name = pwd[0] as *const c_char;
    if name.is_null() {
        return None;
    }
    // SAFETY: pw_name points into buf, NUL-terminated.
    Some(unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned())
}

/// The pause after `failures` wrong answers in a row: 1, 2, 4 ... seconds, at most MAX_PAUSE.
fn pause_after(failures: u32) -> u64 {
    if failures == 0 { 0 } else { (1u64 << (failures - 1).min(16)).min(MAX_PAUSE) }
}

/// Failures in a row per uid and until when it must wait (unix seconds),
/// kept in STATE_DIR/pause so a restart does not forget them.
struct Pauses {
    by_uid: HashMap<u32, (u32, u64)>,
}

impl Pauses {
    fn path() -> std::path::PathBuf {
        std::path::Path::new(paths::STATE_DIR).join("pause")
    }

    fn load() -> Pauses {
        let text = std::fs::read_to_string(Self::path()).unwrap_or_default();
        let by_uid = text
            .lines()
            .filter_map(|l| {
                let mut f = l.split_whitespace().map(|x| x.parse::<u64>().ok());
                Some((f.next()?? as u32, (f.next()?? as u32, f.next()??)))
            })
            .collect();
        Pauses { by_uid }
    }

    fn save(&self) {
        let dir = std::path::Path::new(paths::STATE_DIR);
        let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir);
        let text: String = self.by_uid.iter().map(|(u, (n, t))| format!("{u} {n} {t}\n")).collect();
        let beside = dir.join("pause.new");
        let written = std::fs::File::create(&beside).and_then(|mut f| {
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            f.write_all(text.as_bytes())?;
            f.sync_all()
        });
        if written.and_then(|_| std::fs::rename(&beside, Self::path())).is_err() {
            log("the pause file could not be written");
        }
    }

    /// Seconds this uid must still wait, 0 if none.
    fn wait(&self, uid: u32) -> u64 {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.by_uid.get(&uid).map_or(0, |(_, until)| until.saturating_sub(now))
    }

    fn failed(&mut self, uid: u32) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let e = self.by_uid.entry(uid).or_insert((0, 0));
        e.0 = e.0.saturating_add(1);
        e.1 = now + pause_after(e.0);
        self.save();
    }

    fn succeeded(&mut self, uid: u32) {
        if self.by_uid.remove(&uid).is_some() {
            self.save();
        }
    }
}

/// The child's conversation: PAM's messages to the client, its answers back.
struct Relay {
    stream: UnixStream,
}

impl Conversation for Relay {
    fn prompt(&mut self, echo: bool, text: &str) -> Option<Secret> {
        proto::send_from_vdm(&mut self.stream, &FromVdm::Prompt { echo, text: text.into() }).ok()?;
        match proto::read_to_vdm(&mut self.stream) {
            Ok(ToVdm::Answer(s)) => Some(s),
            _ => None,
        }
    }

    fn info(&mut self, text: &str) {
        let _ = proto::send_from_vdm(&mut self.stream, &FromVdm::Info(text.into()));
    }

    fn error(&mut self, text: &str) {
        let _ = proto::send_from_vdm(&mut self.stream, &FromVdm::Error(text.into()));
    }
}

/// In the child: one check, start to end. Its exit status is what PAM said.
fn check(mut stream: UnixStream, name: &str, service: &str) -> c_int {
    match proto::read_to_vdm(&mut stream) {
        Ok(ToVdm::Hello { version: proto::VERSION, service: proto::Service::Unlock }) => {}
        Ok(ToVdm::Hello { .. }) => return 3,
        Ok(_) | Err(ProtoError::Bad(_)) | Err(ProtoError::Io(_)) => return 3,
    }
    let mut relay = Relay { stream };
    let outcome = pam::authenticate(service, name, &mut relay);
    let _ = proto::send_from_vdm(&mut relay.stream, &FromVdm::Done { ok: outcome == Outcome::Yes, wait: 0 });
    match outcome {
        Outcome::Yes => 0,
        Outcome::No => 1,
        Outcome::GivenUp => 2,
        Outcome::Broken => 3,
    }
}

fn refuse(mut stream: UnixStream, wait: u64) {
    let _ = proto::send_from_vdm(&mut stream, &FromVdm::Done { ok: false, wait: wait.min(u16::MAX as u64) as u16 });
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let opt = |name: &str, default: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| default.to_string());
    let socket = opt("--socket", paths::UNLOCK_SOCKET);
    let service = opt("--service", paths::UNLOCK_SERVICE);
    if sys::euid() != 0 {
        eprintln!("vdm: must run as root");
        std::process::exit(1);
    }
    // Our own PAM file, or nothing: without it PAM would use "other", which
    // differs between systems (review I2).
    if !std::path::Path::new(paths::PAM_DIR).join(&service).is_file() {
        log(&format!("{}/{service} is missing; not starting (tools/install.sh puts it there)", paths::PAM_DIR));
        std::process::exit(1);
    }
    sys::no_core_files();
    sys::no_debugger();

    // The socket: its directory root's and readable by all; a socket left by
    // a vdm that ended is replaced, anything else there is not touched.
    let path = std::path::Path::new(&socket);
    if let Some(dir) = path.parent() {
        let _ = std::fs::DirBuilder::new().recursive(true).mode(0o755).create(dir);
    }
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_socket() => {
            let _ = std::fs::remove_file(path);
        }
        Ok(_) => {
            log(&format!("{socket} is there and is not a socket; not starting"));
            std::process::exit(1);
        }
        Err(_) => {}
    }
    let listener = match UnixListener::bind(path) {
        Ok(l) => l,
        Err(e) => {
            log(&format!("{socket}: {e}"));
            std::process::exit(1);
        }
    };
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666));
    let _ = listener.set_nonblocking(true);
    log(&format!("checking unlocks on {socket} with PAM service {service}"));

    let mut pauses = Pauses::load();
    // The check running for each uid: its child's pid.
    let mut running: HashMap<c_int, u32> = HashMap::new();
    loop {
        // Children that ended: what PAM said for their account.
        loop {
            let mut status: c_int = 0;
            // SAFETY: an out-parameter we own.
            let pid = unsafe { waitpid(-1, &mut status, WNOHANG) };
            if pid <= 0 {
                break;
            }
            let Some(uid) = running.remove(&pid) else { continue };
            let code = if status & 0x7f == 0 { (status >> 8) & 0xff } else { -1 };
            let who = user_name(uid).unwrap_or_else(|| uid.to_string());
            match code {
                0 => {
                    log(&format!("unlock for {who}: yes"));
                    pauses.succeeded(uid);
                }
                1 => {
                    pauses.failed(uid);
                    log(&format!("unlock for {who}: no (a pause of {} s before the next try)", pauses.wait(uid)));
                }
                2 => log(&format!("unlock for {who}: given up")),
                -1 => log(&format!("unlock for {who}: the check ran out of time")),
                _ => log(&format!("unlock for {who}: PAM could not run (status {code})")),
            }
        }
        let mut fds = [PollFd { fd: listener.as_raw_fd(), events: 1, revents: 0 }];
        // SAFETY: one pollfd we own; a second's wait so children are reaped.
        unsafe { poll(fds.as_mut_ptr(), 1, 1000) };
        let stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(_) => continue,
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(IO_LIMIT));
        let _ = stream.set_write_timeout(Some(IO_LIMIT));
        let Some(uid) = sys::peer_uid(&stream) else { continue };
        if uid == 0 || uid < paths::MIN_UID {
            log(&format!("unlock asked for uid {uid}: refused (root and system accounts)"));
            refuse(stream, 0);
            continue;
        }
        let Some(name) = user_name(uid) else {
            log(&format!("unlock asked for uid {uid}: refused (no such account)"));
            refuse(stream, 0);
            continue;
        };
        if running.values().any(|u| *u == uid) {
            refuse(stream, 1);
            continue;
        }
        let wait = pauses.wait(uid);
        if wait > 0 {
            log(&format!("unlock for {name}: refused, {wait} s of its pause left"));
            refuse(stream, wait);
            continue;
        }
        // SAFETY: vdm is single-threaded; the child only runs check() and exits.
        let pid = unsafe { fork() };
        if pid == 0 {
            drop(listener);
            // SAFETY: the child's own limit: SIGALRM ends it if PAM blocks.
            unsafe { alarm(TIME_LIMIT) };
            let code = check(stream, &name, &service);
            // SAFETY: leave without running the parent's exit handlers.
            unsafe { _exit(code) };
        }
        if pid < 0 {
            log("fork failed");
            continue;
        }
        running.insert(pid, uid);
        drop(stream);
    }
}
