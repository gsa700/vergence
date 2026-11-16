// vdm: the Vergence display manager's root daemon (docs/vdm-milestone-1.md).
// It checks a password for the lock screen of the account that asks, locks
// every session before sleep, and runs the login screen (step 4c) and checks
// its logins.
//
//   vdm [--socket PATH] [--service NAME]   the daemon
//   vdm before-sleep                       from the sleep hooks
//   vdm greeter [stop]                     start (keep running) or stop the login screen
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
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt, PermissionsExt};
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
    fn getpwnam_r(name: *const c_char, pwd: *mut c_void, buf: *mut c_char, len: usize, result: *mut *mut c_void) -> c_int;
    fn setsid() -> c_int;
    fn setuid(uid: u32) -> c_int;
    fn setgid(gid: u32) -> c_int;
    fn initgroups(name: *const c_char, gid: u32) -> c_int;
    fn kill(pid: c_int, sig: c_int) -> c_int;
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

/// An account by name: its uid, gid and login shell. struct passwd: pw_name,
/// pw_passwd, then pw_uid and pw_gid (one 8-byte word); Linux: pw_gecos,
/// pw_dir, pw_shell; FreeBSD: pw_change, pw_class, pw_gecos, pw_dir, pw_shell.
fn account(name: &str) -> Option<(u32, u32, String)> {
    let c = std::ffi::CString::new(name).ok()?;
    let mut pwd = [0u64; 16];
    let mut buf = vec![0 as c_char; 4096];
    let mut result: *mut c_void = std::ptr::null_mut();
    // SAFETY: a struct passwd fits in 128 bytes on both; buf and its length.
    let r = unsafe { getpwnam_r(c.as_ptr(), pwd.as_mut_ptr().cast(), buf.as_mut_ptr(), buf.len(), &mut result) };
    if r != 0 || result.is_null() {
        return None;
    }
    let uid = (pwd[2] & 0xffff_ffff) as u32;
    let gid = (pwd[2] >> 32) as u32;
    let shell = pwd[if cfg!(target_os = "linux") { 5 } else { 7 }] as *const c_char;
    let shell = if shell.is_null() {
        String::new()
    } else {
        // SAFETY: pw_shell points into buf, NUL-terminated.
        unsafe { CStr::from_ptr(shell) }.to_string_lossy().into_owned()
    };
    Some((uid, gid, shell))
}

/// Whether an account may log in at the login screen (review I10): not root
/// (nor FreeBSD's toor, uid 0 too), not a system account, with a shell that
/// is a real one.
fn may_log_in(uid: u32, shell: &str) -> bool {
    let real_shell = !shell.is_empty() && !shell.ends_with("/nologin") && !shell.ends_with("/false") && std::path::Path::new(shell).is_file();
    uid != 0 && uid >= paths::MIN_UID && real_shell
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

/// In the child: one check, start to end. Its exit status is what PAM said;
/// the answer to the client (with the pause that follows a "no") is the
/// parent's to send, which alone keeps the pauses. An unlock's Hello is read
/// here; a login's was read by the parent (it had to know the account).
fn check(mut stream: UnixStream, name: &str, service: &str, hello_read: bool) -> c_int {
    if !hello_read {
        match proto::read_to_vdm(&mut stream) {
            Ok(ToVdm::Hello { version: proto::VERSION, service: proto::Service::Unlock }) => {}
            Ok(ToVdm::Hello { .. }) => return 3,
            Ok(_) | Err(ProtoError::Bad(_)) | Err(ProtoError::Io(_)) => return 3,
        }
    }
    let mut relay = Relay { stream };
    let outcome = pam::authenticate(service, name, &mut relay);
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

/// Make a listening socket: its directory root's and readable by all; a
/// socket left by a vdm that ended is replaced, anything else there is not
/// touched.
fn bind(socket: &str, mode: u32) -> UnixListener {
    let path = std::path::Path::new(socket);
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
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    let _ = listener.set_nonblocking(true);
    listener
}

/// `vdm before-sleep`, from the system's sleep hooks: ask the running vdm to
/// lock every session and wait (it answers within a few seconds). Never
/// fails the sleep: a vdm that is not there is said and left.
fn before_sleep() -> ! {
    use std::io::{BufRead, BufReader};
    match UnixStream::connect(paths::CONTROL_SOCKET) {
        Ok(mut s) => {
            let _ = s.set_read_timeout(Some(Duration::from_secs(SLEEP_LIMIT + 5)));
            let _ = s.write_all(b"before-sleep\n");
            let mut line = String::new();
            let _ = BufReader::new(&s).read_line(&mut line);
            eprintln!("vdm before-sleep: {}", line.trim());
        }
        Err(e) => eprintln!("vdm before-sleep: {}: {e}", paths::CONTROL_SOCKET),
    }
    std::process::exit(0)
}

/// `vdm greeter [stop]`: ask the running vdm to start (and keep running) the
/// login screen on its VT, or to stop it (step 4c; at boot from step 7).
fn control_command(line: &str) -> ! {
    use std::io::{BufRead, BufReader};
    match UnixStream::connect(paths::CONTROL_SOCKET) {
        Ok(mut s) => {
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = s.write_all(format!("{line}\n").as_bytes());
            let mut answer = String::new();
            let _ = BufReader::new(&s).read_line(&mut answer);
            println!("vdm: {}", answer.trim());
            std::process::exit(0)
        }
        Err(e) => {
            eprintln!("vdm: {}: {e} (is vdm running? are you root?)", paths::CONTROL_SOCKET);
            std::process::exit(2)
        }
    }
}

/// The login screen as vdm keeps it (step 4c).
struct Greeter {
    /// Wanted: vdm starts it again when it ends.
    on: bool,
    pid: c_int,
    started: std::time::Instant,
    /// Ends within QUICK of starting, in a row: a pause before the next
    /// start doubles; after GIVE_UP of them vdm stops trying (and says so).
    quick: u32,
    next: Option<std::time::Instant>,
}

const QUICK: Duration = Duration::from_secs(30);
const GIVE_UP: u32 = 5;

/// Start the login screen: vergence-server --drm --greeter beside vdm
/// ($prefix/bin), as the greeter account, on its VT (made the one on the
/// screen first, so seatd gives it the seat), in a session of its own with
/// the VT as its terminal, with a home and runtime directory made fresh each
/// time under vdm's run directory (nothing of a previous login screen is
/// kept), its output to STATE_DIR/greeter.log. Every descriptor of vdm's is
/// close-on-exec (Rust opens them so), so it inherits none.
fn start_greeter(uid: u32, gid: u32) -> std::io::Result<c_int> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe()?;
    let server = exe.parent().and_then(|d| d.parent()).map(|p| p.join("bin/vergence-server")).filter(|p| p.is_file()).ok_or_else(|| std::io::Error::other("no vergence-server beside vdm ($prefix/bin)"))?;
    let base = std::path::Path::new(paths::GREETER_SOCKET).parent().ok_or_else(|| std::io::Error::other("no run directory"))?;
    let home = base.join("greeter-home");
    let run = home.join("run");
    if home.symlink_metadata().is_ok() {
        std::fs::remove_dir_all(&home)?;
    }
    for d in [&home, &run] {
        std::fs::DirBuilder::new().mode(0o700).create(d)?;
        std::os::unix::fs::chown(d, Some(uid), Some(gid))?;
    }
    let tty = std::fs::OpenOptions::new().read(true).write(true).open(vergence_dm::vt::tty_path(vergence_dm::vt::GREETER_VT))?;
    let out = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(std::path::Path::new(paths::STATE_DIR).join("greeter.log"))?;
    let name = std::ffi::CString::new(paths::GREETER_USER).map_err(std::io::Error::other)?;
    let mut cmd = std::process::Command::new(&server);
    cmd.args(["--drm", "--greeter"])
        .env_clear()
        .env("HOME", &home)
        .env("XDG_RUNTIME_DIR", &run)
        .env("USER", paths::GREETER_USER)
        .env("LOGNAME", paths::GREETER_USER)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("XDG_SEAT", "seat0")
        .env("XDG_VTNR", vergence_dm::vt::GREETER_VT.to_string())
        .current_dir(&home)
        .stdin(tty)
        .stdout(out.try_clone()?)
        .stderr(out);
    // SAFETY: vdm is single-threaded, so the forked child may call these.
    unsafe {
        cmd.pre_exec(move || {
            if setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            vergence_dm::vt::make_controlling(0)?;
            vergence_dm::vt::activate(0, vergence_dm::vt::GREETER_VT)?;
            if initgroups(name.as_ptr(), gid) != 0 || setgid(gid) != 0 || setuid(uid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // Root is gone for good.
            if setuid(0) == 0 {
                return Err(std::io::Error::other("could still become root"));
            }
            Ok(())
        })
    };
    let child = cmd.spawn()?;
    Ok(child.id() as c_int)
}

/// How long the sessions have to show the lock before the machine sleeps anyway.
const SLEEP_LIMIT: u64 = 5;
/// At most this many servers per account stay registered (the oldest go).
const SESSIONS_PER_UID: usize = 4;

/// Before the machine sleeps (review C4): every registered session is told
/// to lock, and vdm waits until each says the lock is on every screen, or
/// that it cannot lock, or SLEEP_LIMIT passes. Sessions that do not answer
/// are logged and dropped. What happened, for the hook.
fn lock_all(sessions: &mut Vec<(u32, UnixStream)>) -> String {
    let mut waiting: Vec<usize> = Vec::new();
    for (i, (uid, s)) in sessions.iter_mut().enumerate() {
        let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
        if proto::send_from_vdm(s, &FromVdm::LockNow).is_ok() {
            waiting.push(i);
        } else {
            log(&format!("before sleep: {}'s server could not be told", user_name(*uid).unwrap_or_default()));
        }
    }
    let total = waiting.len();
    let (mut shown, mut cannot) = (0, 0);
    let deadline = std::time::Instant::now() + Duration::from_secs(SLEEP_LIMIT);
    let mut dead: Vec<usize> = Vec::new();
    while !waiting.is_empty() {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break;
        }
        let mut fds: Vec<PollFd> = waiting.iter().map(|i| PollFd { fd: sessions[*i].1.as_raw_fd(), events: 1, revents: 0 }).collect();
        // SAFETY: pollfds we own, as many as we pass.
        unsafe { poll(fds.as_mut_ptr(), fds.len() as u64, left.as_millis().min(i32::MAX as u128) as c_int) };
        let ready: Vec<usize> = waiting.iter().zip(&fds).filter(|(_, f)| f.revents != 0).map(|(i, _)| *i).collect();
        for i in ready {
            waiting.retain(|w| *w != i);
            let (uid, s) = &mut sessions[i];
            let _ = s.set_read_timeout(Some(Duration::from_secs(1)));
            let who = user_name(*uid).unwrap_or_default();
            match proto::read_to_vdm(s) {
                Ok(ToVdm::Presented) => shown += 1,
                Ok(ToVdm::CannotLock(why)) => {
                    cannot += 1;
                    log(&format!("before sleep: {who}'s session cannot lock: {why}"));
                }
                _ => dead.push(i),
            }
        }
    }
    for i in &waiting {
        log(&format!("before sleep: {}'s session did not show the lock in time", user_name(sessions[*i].0).unwrap_or_default()));
    }
    dead.sort_unstable();
    for i in dead.into_iter().rev() {
        sessions.remove(i);
    }
    let said = format!("{shown} of {total} sessions locked, {cannot} could not lock, {} did not answer", waiting.len());
    log(&format!("before sleep: {said}"));
    said
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("before-sleep") {
        before_sleep();
    }
    if args.get(1).map(String::as_str) == Some("greeter") {
        let line = if args.get(2).map(String::as_str) == Some("stop") { "greeter stop" } else { "greeter" };
        control_command(line);
    }
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

    let listener = bind(&socket, 0o666);
    let session_listener = bind(paths::SESSION_SOCKET, 0o666);
    let control = bind(paths::CONTROL_SOCKET, 0o600);
    // The login screen's socket: its account's only (threat model T9).
    let greeter_account = account(paths::GREETER_USER);
    let greeter_uid = greeter_account.as_ref().map(|(u, ..)| *u);
    let greeter = greeter_uid.map(|uid| {
        let l = bind(paths::GREETER_SOCKET, 0o600);
        if let Err(e) = std::os::unix::fs::chown(paths::GREETER_SOCKET, Some(uid), None) {
            log(&format!("{}: {e}", paths::GREETER_SOCKET));
        }
        l
    });
    if greeter.is_none() {
        log(&format!("no {} account: no login screen (tools/install.sh makes it)", paths::GREETER_USER));
    }
    let login_service = paths::LOGIN_SERVICE.to_string();
    log(&format!("checking unlocks on {socket} with PAM service {service}; sessions on {}", paths::SESSION_SOCKET));

    let mut pauses = Pauses::load();
    // The check running for each uid, by its child's pid, with the client's
    // connection (the parent answers it when the child ends).
    let mut running: HashMap<c_int, (u32, UnixStream, bool)> = HashMap::new();
    // The servers registered to be locked before sleep, by account.
    let mut sessions: Vec<(u32, UnixStream)> = Vec::new();
    let mut greeter_run = Greeter { on: false, pid: 0, started: std::time::Instant::now(), quick: 0, next: None };
    loop {
        // The login screen, when wanted and not running, once its pause is over.
        if greeter_run.on && greeter_run.pid == 0 && greeter_run.next.is_none_or(|t| std::time::Instant::now() >= t) {
            greeter_run.next = None;
            match greeter_account.as_ref().map(|(u, g, _)| start_greeter(*u, *g)) {
                Some(Ok(pid)) => {
                    greeter_run.pid = pid;
                    greeter_run.started = std::time::Instant::now();
                    log(&format!("login screen started on VT {} (pid {pid})", vergence_dm::vt::GREETER_VT));
                }
                Some(Err(e)) => {
                    log(&format!("login screen: could not start: {e}"));
                    greeter_run.on = false;
                }
                None => greeter_run.on = false,
            }
        }
        // Children that ended: what PAM said for their account.
        loop {
            let mut status: c_int = 0;
            // SAFETY: an out-parameter we own.
            let pid = unsafe { waitpid(-1, &mut status, WNOHANG) };
            if pid <= 0 {
                break;
            }
            if pid == greeter_run.pid {
                greeter_run.pid = 0;
                let ran = greeter_run.started.elapsed();
                greeter_run.quick = if ran < QUICK { greeter_run.quick + 1 } else { 0 };
                log(&format!("login screen ended after {} s (status {status:#x})", ran.as_secs()));
                if greeter_run.on && greeter_run.quick >= GIVE_UP {
                    greeter_run.on = false;
                    log(&format!("login screen: ended {GIVE_UP} times in a row within {} s; not starting it again (see {}/greeter.log)", QUICK.as_secs(), paths::STATE_DIR));
                } else if greeter_run.on {
                    let pause = 1u64 << greeter_run.quick.min(6);
                    greeter_run.next = Some(std::time::Instant::now() + Duration::from_secs(pause));
                }
                continue;
            }
            let Some((uid, mut client, login)) = running.remove(&pid) else { continue };
            let what = if login { "login" } else { "unlock" };
            let code = if status & 0x7f == 0 { (status >> 8) & 0xff } else { -1 };
            let who = user_name(uid).unwrap_or_else(|| uid.to_string());
            match code {
                0 => {
                    log(&format!("{what} for {who}: yes{}", if login { " (starting the session is step 5)" } else { "" }));
                    pauses.succeeded(uid);
                }
                1 => {
                    pauses.failed(uid);
                    log(&format!("{what} for {who}: no (a pause of {} s before the next try)", pauses.wait(uid)));
                }
                2 => log(&format!("{what} for {who}: given up")),
                -1 => log(&format!("{what} for {who}: the check ran out of time")),
                _ => log(&format!("{what} for {who}: PAM could not run (status {code})")),
            }
            // The answer, with the pause it starts, so the lock counts it down at once.
            let _ = client.set_write_timeout(Some(Duration::from_secs(1)));
            let wait = pauses.wait(uid).min(u16::MAX as u64) as u16;
            let _ = proto::send_from_vdm(&mut client, &FromVdm::Done { ok: code == 0, wait });
        }
        let mut fds: Vec<PollFd> = [Some(&listener), Some(&session_listener), Some(&control), greeter.as_ref()].into_iter().flatten().map(|l| PollFd { fd: l.as_raw_fd(), events: 1, revents: 0 }).collect();
        let first_session = fds.len();
        fds.extend(sessions.iter().map(|(_, s)| PollFd { fd: s.as_raw_fd(), events: 1, revents: 0 }));
        // SAFETY: pollfds we own. A check running: look again in 50 ms, so its
        // answer goes out as soon as it ends; none: a second's wait.
        let wait = if running.is_empty() && greeter_run.next.is_none() { 1000 } else { 50 };
        unsafe { poll(fds.as_mut_ptr(), fds.len() as u64, wait) };
        // A registered server that went away (or said something out of turn): dropped.
        let gone: Vec<usize> = fds[first_session..].iter().enumerate().filter(|(_, f)| f.revents != 0).map(|(i, _)| i).collect();
        for i in gone.into_iter().rev() {
            sessions.remove(i);
        }
        // A server registering: the connection is the registration.
        if let Ok((s, _)) = session_listener.accept() {
            let _ = s.set_nonblocking(false);
            match sys::peer_uid(&s) {
                Some(uid) if uid != 0 && uid >= paths::MIN_UID => {
                    if sessions.iter().filter(|(u, _)| *u == uid).count() >= SESSIONS_PER_UID
                        && let Some(oldest) = sessions.iter().position(|(u, _)| *u == uid)
                    {
                        sessions.remove(oldest);
                    }
                    sessions.push((uid, s));
                }
                _ => {}
            }
        }
        // Root's: lock every session before sleeping.
        if let Ok((mut s, _)) = control.accept() {
            let _ = s.set_nonblocking(false);
            if sys::peer_uid(&s) == Some(0) {
                use std::io::{BufRead, BufReader};
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                let _ = BufReader::new(&s).read_line(&mut line);
                let answer = match line.trim() {
                    "before-sleep" => lock_all(&mut sessions),
                    "greeter" if greeter_account.is_none() => format!("no {} account (tools/install.sh makes it)", paths::GREETER_USER),
                    "greeter" => {
                        greeter_run.on = true;
                        greeter_run.quick = 0;
                        if greeter_run.pid == 0 {
                            greeter_run.next = None;
                        }
                        format!("the login screen is on VT {} (Ctrl+Alt+F{0})", vergence_dm::vt::GREETER_VT)
                    }
                    "greeter stop" => {
                        greeter_run.on = false;
                        greeter_run.next = None;
                        if greeter_run.pid > 0 {
                            // SAFETY: our own child; SIGTERM 15.
                            unsafe { kill(greeter_run.pid, 15) };
                        }
                        "the login screen is stopping".to_string()
                    }
                    other => format!("unknown: {other}"),
                };
                let _ = s.write_all(format!("{answer}\n").as_bytes());
            }
        }
        // The login screen asking to log someone in (step 4).
        if let Some(Ok((mut g, _))) = greeter.as_ref().map(|l| l.accept()) {
            let _ = g.set_nonblocking(false);
            let _ = g.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = g.set_write_timeout(Some(IO_LIMIT));
            let request = if sys::peer_uid(&g) == greeter_uid {
                match (proto::read_to_vdm(&mut g), proto::read_to_vdm(&mut g)) {
                    (Ok(ToVdm::Hello { version: proto::VERSION, service: proto::Service::Login }), Ok(ToVdm::Login { user, session })) => Some((user, session)),
                    _ => None,
                }
            } else {
                None
            };
            if let Some((user, _session)) = request {
                match account(&user) {
                    Some((uid, _, shell)) if may_log_in(uid, &shell) => {
                        let wait = pauses.wait(uid);
                        if running.values().any(|(u, ..)| *u == uid) {
                            refuse(g, 1);
                        } else if wait > 0 {
                            log(&format!("login for {user}: refused, {wait} s of its pause left"));
                            refuse(g, wait);
                        } else {
                            let _ = g.set_read_timeout(Some(IO_LIMIT));
                            // SAFETY: vdm is single-threaded; the child only runs check() and exits.
                            let pid = unsafe { fork() };
                            if pid == 0 {
                                drop(listener);
                                drop(session_listener);
                                drop(control);
                                drop(sessions);
                                drop(running);
                                // SAFETY: the child's own limit: SIGALRM ends it if PAM blocks.
                                unsafe { alarm(TIME_LIMIT) };
                                let code = check(g, &user, &login_service, true);
                                // SAFETY: leave without running the parent's exit handlers.
                                unsafe { _exit(code) };
                            }
                            if pid > 0 {
                                running.insert(pid, (uid, g, true));
                            }
                        }
                    }
                    // Root, system accounts, no real shell, no such account:
                    // the same "no" as a wrong password, logged apart.
                    _ => {
                        log(&format!("login for {user}: refused (root, a system account, no login shell, or no such account)"));
                        refuse(g, 0);
                    }
                }
            }
        }
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
        if running.values().any(|(u, ..)| *u == uid) {
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
            drop(session_listener);
            drop(control);
            drop(sessions);
            drop(running);
            drop(greeter);
            // SAFETY: the child's own limit: SIGALRM ends it if PAM blocks.
            unsafe { alarm(TIME_LIMIT) };
            let code = check(stream, &name, &service, false);
            // SAFETY: leave without running the parent's exit handlers.
            unsafe { _exit(code) };
        }
        if pid < 0 {
            log("fork failed");
            continue;
        }
        running.insert(pid, (uid, stream, false));
    }
}
