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
use std::io::{Read, Write};
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
    #[cfg(target_os = "freebsd")]
    fn setlogin(name: *const c_char) -> c_int;
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

const WNOHANG: c_int = 1;

/// One line to the system log. Linux: standard error, which the service
/// hands on (runit's vlogger, systemd's journal). FreeBSD: syslog(3) itself
/// (auth.notice), with standard error /dev/null: daemon(8) reading vdm's
/// output waited for every process holding it, a running session's worker
/// among them, so `service vdm restart` hung (kepler, 2026-11-16).
fn log(msg: &str) {
    #[cfg(target_os = "freebsd")]
    {
        unsafe extern "C" {
            fn openlog(ident: *const c_char, option: c_int, facility: c_int);
            fn syslog(priority: c_int, fmt: *const c_char, ...);
        }
        static OPEN: std::sync::Once = std::sync::Once::new();
        // SAFETY: a static NUL-terminated name; LOG_PID 1, LOG_AUTH 4<<3.
        OPEN.call_once(|| unsafe { openlog(c"vdm".as_ptr(), 1, 4 << 3) });
        if let Ok(m) = std::ffi::CString::new(msg.replace('\0', "")) {
            // SAFETY: "%s" with one NUL-terminated string; LOG_AUTH | LOG_NOTICE 5.
            unsafe { syslog((4 << 3) | 5, c"%s".as_ptr(), m.as_ptr()) };
        }
    }
    #[cfg(not(target_os = "freebsd"))]
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

/// An account by name.
#[derive(Clone)]
struct Account {
    name: String,
    uid: u32,
    gid: u32,
    home: String,
    shell: String,
}

/// An account by name: its uid, gid, home and login shell. struct passwd: pw_name,
/// pw_passwd, then pw_uid and pw_gid (one 8-byte word); Linux: pw_gecos,
/// pw_dir, pw_shell; FreeBSD: pw_change, pw_class, pw_gecos, pw_dir, pw_shell.
fn account(name: &str) -> Option<Account> {
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
    let text = |i: usize| {
        let p = pwd[i] as *const c_char;
        if p.is_null() {
            String::new()
        } else {
            // SAFETY: the field points into buf, NUL-terminated.
            unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
        }
    };
    let (dir, shell) = if cfg!(target_os = "linux") { (4, 5) } else { (6, 7) };
    Some(Account { name: name.to_string(), uid, gid, home: text(dir), shell: text(shell) })
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

/// What a login starts, decided by the parent before the worker forks.
struct LoginPlan {
    account: Account,
    /// The session's command (its entry's Exec), run by the login shell.
    exec: String,
    vt: u32,
}

/// In the login worker (step 5): the check, and on a yes the whole session.
/// It tells the parent "Y" on `said` the moment PAM says yes (the parent
/// answers the login screen and stops it), then opens the PAM session,
/// starts the session on its VT and waits for it, ends whatever it left
/// (the reaper, review C6), and closes the PAM session. Its exit status:
/// 0 a session ran; 1 no; 2 given up; 3 PAM broken; 4 the session could not start.
fn login(stream: UnixStream, plan: &LoginPlan, service: &str, said: UnixStream) -> c_int {
    let mut relay = Relay { stream };
    let mut code = 0;
    let outcome = pam::authenticate_then(service, &plan.account.name, &mut relay, &mut |h| {
        // The check's limit does not apply to the session.
        // SAFETY: cancels this process's alarm.
        unsafe { alarm(0) };
        let _ = (&said).write_all(b"Y");
        code = run_session(h, plan);
    });
    match outcome {
        Outcome::Yes => code,
        Outcome::No => 1,
        Outcome::GivenUp => 2,
        Outcome::Broken => 3,
    }
}

/// The runtime directory, where no PAM module made one (review I3): Linux
/// /run/user/<uid>, FreeBSD /var/run/xdg/<user> (pam_xdg's), the user's and
/// 0700; one that is there must be the user's and not a link.
fn runtime_dir(a: &Account) -> std::io::Result<std::path::PathBuf> {
    let dir = if cfg!(target_os = "linux") { std::path::PathBuf::from(format!("/run/user/{}", a.uid)) } else { std::path::PathBuf::from(format!("/var/run/xdg/{}", a.name)) };
    if let Some(parent) = dir.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o755).create(parent)?;
    }
    match dir.symlink_metadata() {
        Ok(m) => {
            use std::os::unix::fs::MetadataExt;
            if !m.is_dir() || m.uid() != a.uid {
                return Err(std::io::Error::other(format!("{} is not {}'s", dir.display(), a.name)));
            }
        }
        Err(_) => {
            std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
            std::os::unix::fs::chown(&dir, Some(a.uid), Some(a.gid))?;
        }
    }
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

/// The session, from the PAM session's opening to its closing.
fn run_session(h: &pam::Handle, plan: &LoginPlan) -> c_int {
    use std::os::unix::process::CommandExt;
    let a = &plan.account;
    let tty = vergence_dm::vt::tty_path(plan.vt);
    // What logind (pam_systemd) needs to know before the session opens (review I4).
    h.set_item(pam::TTY, &tty);
    for kv in ["XDG_SEAT=seat0".to_string(), format!("XDG_VTNR={}", plan.vt), "XDG_SESSION_TYPE=wayland".into(), "XDG_SESSION_CLASS=user".into()] {
        h.putenv(&kv);
    }
    let r = h.setcred(pam::ESTABLISH_CRED);
    if r != pam::SUCCESS {
        log(&format!("session for {}: PAM would not give credentials: {}", a.name, pam::describe(r)));
        return 4;
    }
    let r = h.open_session();
    if r != pam::SUCCESS {
        log(&format!("session for {}: PAM would not open the session: {}", a.name, pam::describe(r)));
        h.setcred(pam::DELETE_CRED);
        return 4;
    }
    if !sys::become_reaper() {
        log("session: could not become the reaper; leftovers may survive the session");
    }
    // The environment from nothing (threat model T17): PAM's, then ours.
    let mut env: Vec<(String, String)> = h.env().into_iter().filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect();
    let has = |env: &Vec<(String, String)>, k: &str| env.iter().any(|(n, _)| n == k);
    let runtime = match env.iter().find(|(k, _)| k == "XDG_RUNTIME_DIR") {
        Some((_, v)) => Ok(std::path::PathBuf::from(v)),
        None => runtime_dir(a),
    };
    let code = match runtime {
        Err(e) => {
            log(&format!("session for {}: no runtime directory: {e}", a.name));
            4
        }
        Ok(runtime) => {
            let set = |env: &mut Vec<(String, String)>, k: &str, v: String| {
                env.retain(|(n, _)| n != k);
                env.push((k.to_string(), v));
            };
            set(&mut env, "HOME", a.home.clone());
            set(&mut env, "USER", a.name.clone());
            set(&mut env, "LOGNAME", a.name.clone());
            set(&mut env, "SHELL", a.shell.clone());
            set(&mut env, "XDG_RUNTIME_DIR", runtime.display().to_string());
            set(&mut env, "XDG_SEAT", "seat0".into());
            set(&mut env, "XDG_VTNR", plan.vt.to_string());
            set(&mut env, "XDG_SESSION_TYPE", "wayland".into());
            set(&mut env, "XDG_SESSION_CLASS", "user".into());
            if !has(&env, "PATH") {
                set(&mut env, "PATH", "/usr/local/bin:/usr/bin:/bin".into());
            }
            if !has(&env, "LANG") {
                set(&mut env, "LANG", "C.UTF-8".into());
            }
            let shell_name = std::path::Path::new(&a.shell).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "sh".into());
            let home = if std::path::Path::new(&a.home).is_dir() { a.home.clone() } else { "/".into() };
            let opened = std::fs::OpenOptions::new().read(true).write(true).open(&tty);
            let null = std::fs::OpenOptions::new().write(true).open("/dev/null");
            match (opened, null) {
                (Ok(tty_file), Ok(null)) => {
                    let name = std::ffi::CString::new(a.name.clone()).unwrap_or_default();
                    let (uid, gid, vt) = (a.uid, a.gid, plan.vt);
                    let mut cmd = std::process::Command::new(&a.shell);
                    // A login shell ("-zsh"), which reads the user's profile, runs the session.
                    cmd.arg0(format!("-{shell_name}"))
                        .args(["-c", &format!("exec {}", plan.exec)])
                        .env_clear()
                        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                        .current_dir(&home)
                        .stdin(tty_file)
                        .stdout(null.try_clone().unwrap_or_else(|_| std::fs::File::open("/dev/null").expect("/dev/null")))
                        .stderr(null);
                    // SAFETY: the worker is single-threaded, so the forked child may call these.
                    unsafe {
                        cmd.pre_exec(move || {
                            if setsid() < 0 {
                                return Err(std::io::Error::last_os_error());
                            }
                            #[cfg(target_os = "freebsd")]
                            if setlogin(name.as_ptr()) != 0 {
                                return Err(std::io::Error::last_os_error());
                            }
                            vergence_dm::vt::make_controlling(0)?;
                            vergence_dm::vt::activate(0, vt)?;
                            if initgroups(name.as_ptr(), gid) != 0 || setgid(gid) != 0 || setuid(uid) != 0 {
                                return Err(std::io::Error::last_os_error());
                            }
                            if setuid(0) == 0 {
                                return Err(std::io::Error::other("could still become root"));
                            }
                            Ok(())
                        })
                    };
                    match cmd.spawn() {
                        Ok(mut child) => {
                            log(&format!("session for {} started on VT {vt} (pid {}): {}", a.name, child.id(), plan.exec));
                            let status = child.wait();
                            log(&format!("session for {} ended ({})", a.name, status.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string())));
                            0
                        }
                        Err(e) => {
                            log(&format!("session for {}: could not start: {e}", a.name));
                            4
                        }
                    }
                }
                (Err(e), _) | (_, Err(e)) => {
                    log(&format!("session for {}: {tty}: {e}", a.name));
                    4
                }
            }
        }
    };
    // Whatever the session left running ends with it (review C6): asked,
    // then, after 3 s, made to.
    end_leftovers(&a.name);
    let r = h.close_session();
    if r != pam::SUCCESS {
        log(&format!("session for {}: closing the PAM session: {}", a.name, pam::describe(r)));
    }
    h.setcred(pam::DELETE_CRED);
    code
}

/// Every process left below the worker: SIGTERM, up to 3 s for them to go,
/// then SIGKILL; each one reaped.
fn end_leftovers(who: &str) {
    let reap = || loop {
        let mut status: c_int = 0;
        // SAFETY: an out-parameter we own.
        if unsafe { waitpid(-1, &mut status, WNOHANG) } <= 0 {
            break;
        }
    };
    reap();
    let asked = sys::signal_descendants(15);
    if asked == 0 {
        return;
    }
    let until = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < until {
        reap();
        if sys::count_descendants() == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let killed = sys::signal_descendants(9);
    std::thread::sleep(Duration::from_millis(200));
    reap();
    log(&format!("session for {who}: {asked} process(es) left at its end were ended{}", if killed > 0 { format!(", {killed} by force") } else { String::new() }));
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
    /// The VT on the screen when `vdm greeter` was asked, to go back to
    /// when it is stopped (kepler, 2026-11-16: a blank screen otherwise).
    came_from: Option<u32>,
    back_to: Option<(u32, std::time::Instant)>,
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

/// A login vdm is running: its worker checks, then runs the session.
struct LoginRun {
    uid: u32,
    name: String,
    /// The login screen's connection, until it is answered.
    client: Option<UnixStream>,
    /// The worker says "Y" here the moment PAM says yes.
    said: UnixStream,
    vt: u32,
    yes: bool,
}

/// At most this many sessions at once (each on a VT of its own).
const MAX_SESSIONS: u32 = 4;

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
    // Sleep, Restart, Shut Down (step 6): anyone may ask; vdm decides by who.
    let power = bind(paths::POWER_SOCKET, 0o666);
    // The login screen's socket: its account's only (threat model T9).
    let greeter_account = account(paths::GREETER_USER);
    let greeter_uid = greeter_account.as_ref().map(|a| a.uid);
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
    // Logins by their worker's pid: checking, then (yes) the running session.
    let mut logins: HashMap<c_int, LoginRun> = HashMap::new();
    let mut greeter_run = Greeter { on: false, pid: 0, started: std::time::Instant::now(), quick: 0, next: None, came_from: None, back_to: None };
    loop {
        // The login screen, when wanted and not running, once its pause is over.
        // A yes: answer the login screen, which then gives way to the session.
        for l in logins.values_mut().filter(|l| !l.yes) {
            let mut b = [0u8; 1];
            if matches!((&l.said).read(&mut b), Ok(1)) && b[0] == b'Y' {
                l.yes = true;
                pauses.succeeded(l.uid);
                log(&format!("login for {}: yes; the session starts on VT {}", l.name, l.vt));
                if let Some(mut c) = l.client.take() {
                    let _ = c.set_write_timeout(Some(Duration::from_secs(1)));
                    let _ = proto::send_from_vdm(&mut c, &FromVdm::Done { ok: true, wait: 0 });
                }
                if greeter_run.pid > 0 {
                    // SAFETY: our own child; SIGTERM 15.
                    unsafe { kill(greeter_run.pid, 15) };
                }
            }
        }
        // After `vdm greeter stop`: the console it was asked from, once the login screen has gone.
        if let Some((vt, at)) = greeter_run.back_to
            && greeter_run.pid == 0
            && std::time::Instant::now() >= at
        {
            greeter_run.back_to = None;
            if let Err(e) = vergence_dm::vt::switch_to(vt) {
                log(&format!("back to VT {vt}: {e}"));
            }
        }
        let session_running = logins.values().any(|l| l.yes);
        if greeter_run.on && greeter_run.pid == 0 && !session_running && greeter_run.next.is_none_or(|t| std::time::Instant::now() >= t) {
            greeter_run.next = None;
            match greeter_account.as_ref().map(|a| start_greeter(a.uid, a.gid)) {
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
                // Stopped for a session: not a failure.
                let for_session = logins.values().any(|l| l.yes);
                greeter_run.quick = if ran < QUICK && !for_session { greeter_run.quick + 1 } else { 0 };
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
            if let Some(mut l) = logins.remove(&pid) {
                let code = if status & 0x7f == 0 { (status >> 8) & 0xff } else { -1 };
                if l.yes {
                    log(&format!("session for {} on VT {}: over (status {code})", l.name, l.vt));
                    // The login screen again, if it is wanted.
                    if greeter_run.on && greeter_run.pid == 0 {
                        greeter_run.quick = 0;
                        greeter_run.next = None;
                    }
                    continue;
                }
                match code {
                    1 => {
                        pauses.failed(l.uid);
                        log(&format!("login for {}: no (a pause of {} s before the next try)", l.name, pauses.wait(l.uid)));
                    }
                    2 => log(&format!("login for {}: given up", l.name)),
                    -1 => log(&format!("login for {}: the check ran out of time", l.name)),
                    _ => log(&format!("login for {}: PAM could not run (status {code})", l.name)),
                }
                if let Some(mut c) = l.client.take() {
                    let _ = c.set_write_timeout(Some(Duration::from_secs(1)));
                    let wait = pauses.wait(l.uid).min(u16::MAX as u64) as u16;
                    let _ = proto::send_from_vdm(&mut c, &FromVdm::Done { ok: false, wait });
                }
                continue;
            }
            let Some((uid, mut client, login)) = running.remove(&pid) else { continue };
            let what = if login { "login" } else { "unlock" };
            let code = if status & 0x7f == 0 { (status >> 8) & 0xff } else { -1 };
            let who = user_name(uid).unwrap_or_else(|| uid.to_string());
            match code {
                0 => {
                    log(&format!("{what} for {who}: yes"));
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
        let mut fds: Vec<PollFd> = [Some(&listener), Some(&session_listener), Some(&control), Some(&power), greeter.as_ref()].into_iter().flatten().map(|l| PollFd { fd: l.as_raw_fd(), events: 1, revents: 0 }).collect();
        let first_session = fds.len();
        fds.extend(sessions.iter().map(|(_, s)| PollFd { fd: s.as_raw_fd(), events: 1, revents: 0 }));
        // SAFETY: pollfds we own. A check running: look again in 50 ms, so its
        // answer goes out as soon as it ends; none: a second's wait.
        let wait = if running.is_empty() && greeter_run.next.is_none() && greeter_run.back_to.is_none() && logins.values().all(|l| l.yes) { 1000 } else { 50 };
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
                    // Not over someone's session: the login screen would take the screen
                    // from it (kepler, 2026-11-16: started over Dave's session by mistake).
                    "greeter" if !greeter_run.on && (!sessions.is_empty() || logins.values().any(|l| l.yes)) => {
                        "not started: someone is logged in (log out first; it comes back by itself after a login screen's session)".to_string()
                    }
                    "greeter" => {
                        if !greeter_run.on {
                            greeter_run.came_from = vergence_dm::vt::active().filter(|v| *v != vergence_dm::vt::GREETER_VT);
                        }
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
                        // Back to the console it was asked from (once it has let go of the seat).
                        if let Some(vt) = greeter_run.came_from.take() {
                            greeter_run.back_to = Some((vt, std::time::Instant::now() + Duration::from_millis(500)));
                        }
                        "the login screen is stopping".to_string()
                    }
                    other => format!("unknown: {other}"),
                };
                let _ = s.write_all(format!("{answer}\n").as_bytes());
            }
        }
        // Sleep, Restart, Shut Down (step 6): for root, the login screen
        // when no one is logged in, and anyone with a session here; restart
        // and shut down not while someone else has one (review M2).
        if let Ok((mut s, _)) = power.accept() {
            use std::io::{BufRead, BufReader};
            let _ = s.set_nonblocking(false);
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            let _ = BufReader::new((&s).take(64)).read_line(&mut line);
            let uid = sys::peer_uid(&s);
            let has_session = |u: u32| sessions.iter().any(|(x, _)| *x == u) || logins.values().any(|l| l.yes && l.uid == u);
            let others = |u: u32| sessions.iter().any(|(x, _)| *x != u) || logins.values().any(|l| l.yes && l.uid != u);
            let who = uid.map(|u| user_name(u).unwrap_or_else(|| u.to_string())).unwrap_or_else(|| "?".into());
            let answer = match (vergence_dm::power::Power::from_word(line.trim()), uid) {
                (None, _) | (_, None) => "no: not understood".to_string(),
                (Some(p), Some(u)) => {
                    let allowed = u == 0 || (Some(u) == greeter_uid && !logins.values().any(|l| l.yes)) || (u >= paths::MIN_UID && has_session(u));
                    if !allowed {
                        log(&format!("power: {} for {who}: refused (no session here)", p.word()));
                        "no: only someone logged in at this machine can do that".to_string()
                    } else if p != vergence_dm::power::Power::Sleep && u != 0 && others(u) {
                        log(&format!("power: {} for {who}: refused (someone else is logged in)", p.word()));
                        "no: someone else is logged in".to_string()
                    } else {
                        let (cmd, args) = p.command();
                        let started = std::process::Command::new(cmd)
                            .args(args)
                            .env_clear()
                            .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn();
                        match started {
                            Ok(_) => {
                                log(&format!("power: {} for {who} ({cmd})", p.word()));
                                "ok".to_string()
                            }
                            Err(e) => {
                                log(&format!("power: {} for {who}: {cmd}: {e}", p.word()));
                                format!("no: {cmd}: {e}")
                            }
                        }
                    }
                }
            };
            let _ = s.write_all(format!("{answer}\n").as_bytes());
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
            if let Some((user, session)) = request {
                match account(&user) {
                    Some(acct) if may_log_in(acct.uid, &acct.shell) => {
                        let uid = acct.uid;
                        let wait = pauses.wait(uid);
                        // The session the login screen chose, by vdm's own reading of the list (review M2).
                        let exec = vergence_dm::sessions::list().get(session as usize).and_then(|e| std::fs::read_to_string(&e.file).ok()).and_then(|t| vergence_dm::sessions::entry_exec(&t));
                        let vt = (0..MAX_SESSIONS).map(|k| vergence_dm::vt::GREETER_VT + 1 + k).find(|vt| !logins.values().any(|l| l.vt == *vt));
                        // One session per person for now: a second would share the
                        // first's runtime directory and its apps' sockets (step 6).
                        let elsewhere = sessions.iter().any(|(u, _)| *u == uid) || logins.values().any(|l| l.uid == uid && l.yes);
                        if elsewhere {
                            log(&format!("login for {user}: refused (already logged in)"));
                            let _ = proto::send_from_vdm(&mut g, &FromVdm::Error("You are already logged in on another console. Log out there first.".into()));
                            refuse(g, 0);
                        } else if running.values().any(|(u, ..)| *u == uid) || logins.values().any(|l| l.uid == uid && !l.yes) {
                            refuse(g, 1);
                        } else if wait > 0 {
                            log(&format!("login for {user}: refused, {wait} s of its pause left"));
                            refuse(g, wait);
                        } else if let (Some(exec), Some(vt)) = (exec, vt) {
                            let _ = g.set_read_timeout(Some(IO_LIMIT));
                            let plan = LoginPlan { account: acct, exec, vt };
                            match UnixStream::pair() {
                                Ok((said_parent, said_child)) => {
                                    // SAFETY: vdm is single-threaded; the child runs login() and exits.
                                    let pid = unsafe { fork() };
                                    if pid == 0 {
                                        drop(listener);
                                        drop(session_listener);
                                        drop(control);
                                        drop(power);
                                        drop(sessions);
                                        drop(running);
                                        drop(said_parent);
                                        // SAFETY: the check's limit (login() cancels it on a yes).
                                        unsafe { alarm(TIME_LIMIT) };
                                        let code = login(g, &plan, &login_service, said_child);
                                        // SAFETY: leave without running the parent's exit handlers.
                                        unsafe { _exit(code) };
                                    }
                                    if pid > 0 {
                                        let _ = said_parent.set_nonblocking(true);
                                        logins.insert(pid, LoginRun { uid, name: user.clone(), client: Some(g), said: said_parent, vt, yes: false });
                                    }
                                }
                                Err(e) => log(&format!("login for {user}: {e}")),
                            }
                        } else {
                            log(&format!("login for {user}: refused (no such session, or {MAX_SESSIONS} sessions running)"));
                            refuse(g, 0);
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
            drop(power);
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
