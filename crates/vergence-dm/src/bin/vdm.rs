// vdm: the Vergence display manager's root daemon (docs/vdm-milestone-1.md).
// It checks a password for the lock screen of the account that asks, locks
// every session before sleep, and runs the login screen (step 4c) and checks
// its logins.
//
//   vdm [--socket PATH] [--service NAME]   the daemon
//   vdm before-sleep                       from the sleep hooks
//   vdm greeter [stop]                     start (keep running) or stop the login screen
//   vdm autologin on [USER] | off          automatic login at boot (root; the rules: below)
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
const MAX_PAUSE: u64 = 60;
/// Wrong answers in a row that cost nothing beyond PAM's own delay (typos).
const FREE_TRIES: u32 = 3;
/// Failures are forgotten after this long without another, in seconds.
const FORGET_AFTER: u64 = 15 * 60;

unsafe extern "C" {
    fn fork() -> c_int;
    fn _exit(status: c_int) -> !;
    fn alarm(secs: c_uint) -> c_uint;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn poll(fds: *mut PollFd, n: u64, timeout: c_int) -> c_int;
    fn getpwuid_r(uid: u32, pwd: *mut c_void, buf: *mut c_char, len: usize, result: *mut *mut c_void) -> c_int;
    fn getpwnam_r(
        name: *const c_char,
        pwd: *mut c_void,
        buf: *mut c_char,
        len: usize,
        result: *mut *mut c_void,
    ) -> c_int;
    fn setsid() -> c_int;
    fn setuid(uid: u32) -> c_int;
    fn setgid(gid: u32) -> c_int;
    fn kill(pid: c_int, sig: c_int) -> c_int;
    /// SIG_DFL 0, SIG_IGN 1, or a handler's address.
    fn signal(sig: c_int, handler: usize) -> usize;
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
/// among them, so `service vdm restart` hung.
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
    let real_shell = !shell.is_empty()
        && !shell.ends_with("/nologin")
        && !shell.ends_with("/false")
        && std::path::Path::new(shell).is_file();
    uid != 0 && uid >= paths::MIN_UID && real_shell
}

/// The people who may log in here: accounts that are not root nor system
/// accounts, with a real login shell (may_log_in), from every source the
/// system names accounts by (getpwent: files, and LDAP, sssd or NIS where
/// set up; finding 8: /etc/passwd alone missed those).
fn people() -> Vec<String> {
    unsafe extern "C" {
        fn setpwent();
        fn getpwent() -> *const c_void;
        fn endpwent();
    }
    let mut out = Vec::new();
    // SAFETY: vdm is single-threaded; each struct passwd is read before the
    // next call (layout as in account()).
    unsafe {
        setpwent();
        loop {
            let p = getpwent() as *const u64;
            if p.is_null() || out.len() > 10_000 {
                break;
            }
            let name = *p as *const c_char;
            let uid = (*p.add(2) & 0xffff_ffff) as u32;
            let shell = *p.add(if cfg!(target_os = "linux") { 5 } else { 7 }) as *const c_char;
            if name.is_null() || shell.is_null() {
                continue;
            }
            let shell = CStr::from_ptr(shell).to_string_lossy().into_owned();
            if may_log_in(uid, &shell) {
                let n = CStr::from_ptr(name).to_string_lossy().into_owned();
                if !out.contains(&n) {
                    out.push(n);
                }
            }
        }
        endpwent();
    }
    out
}

/// Whether automatic login may be on (answer 4 a in docs/vdm-milestone-1.md,
/// review M6): only where
/// exactly one person can log in, and nothing on the machine is encrypted.
/// That person's name, or why not.
fn autologin_allowed() -> Result<String, String> {
    let enc = vergence_dm::encryption::found()
        .map_err(|why| format!("it cannot be told whether storage here is encrypted: {why}"))?;
    if !enc.is_empty() {
        return Err(format!(
            "storage here is encrypted ({}): automatic login would unlock at power on what the encryption keeps",
            enc.join(", ")
        ));
    }
    match people().as_slice() {
        [one] => Ok(one.clone()),
        [] => Err("no one can log in here".into()),
        _ => Err("more than one person can log in here".into()),
    }
}

/// autologin_allowed, worked out at most once a minute for the Login page
/// (finding 3: anyone asking ran `zfs get` as root on vdm's main loop).
fn allowed_cached(cache: &mut Option<(std::time::Instant, Result<String, String>)>) -> Result<String, String> {
    if let Some((at, r)) = cache.as_ref()
        && at.elapsed() < Duration::from_secs(60)
    {
        return r.clone();
    }
    let r = autologin_allowed();
    *cache = Some((std::time::Instant::now(), r.clone()));
    r
}

/// The pause after `failures` wrong answers in a row: none for the first
/// FREE_TRIES (a typo, a lock key left on), then 5, 15, 30 seconds, then
/// MAX_PAUSE. Gentler than it was (1, 2, 4 ... 300 s from the first): a
/// keyboard light gone wrong could cost six tries and a 256 s wait. A guesser
/// at the console still gets about one try a minute (see docs/vdm-milestone-1.md).
fn pause_after(failures: u32) -> u64 {
    match failures.saturating_sub(FREE_TRIES) {
        0 => 0,
        1 => 5,
        2 => 15,
        3 => 30,
        _ => MAX_PAUSE,
    }
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
        self.failed_at(uid, now);
        self.save();
    }

    /// A failure at `now`: counted on from the last one, unless that was
    /// FORGET_AFTER ago or more (then it is the first again).
    fn failed_at(&mut self, uid: u32, now: u64) {
        let e = self.by_uid.entry(uid).or_insert((0, 0));
        if e.0 > 0 && now >= e.1.saturating_add(FORGET_AFTER) {
            e.0 = 0;
        }
        e.0 = e.0.saturating_add(1);
        e.1 = now + pause_after(e.0);
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
    /// An answer was given: giving up after it counts as a wrong one, so the
    /// pause cannot be skipped by answering the password and cancelling at
    /// a second prompt (finding 10).
    answered: bool,
}

impl Conversation for Relay {
    fn prompt(&mut self, echo: bool, text: &str) -> Option<Secret> {
        proto::send_from_vdm(&mut self.stream, &FromVdm::Prompt { echo, text: text.into() }).ok()?;
        match proto::read_to_vdm(&mut self.stream) {
            Ok(ToVdm::Answer(s)) => {
                self.answered = true;
                Some(s)
            }
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
    let mut relay = Relay { stream, answered: false };
    let outcome = pam::authenticate(service, name, &mut relay);
    match outcome {
        Outcome::Yes => 0,
        Outcome::No => 1,
        Outcome::GivenUp if relay.answered => 1,
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
    let mut relay = Relay { stream, answered: false };
    let mut code = 0;
    // The session's terminal before the check; an expired password changed
    // before the yes (findings 6, 10).
    let opts = pam::Options { tty: Some(vergence_dm::vt::tty_path(plan.vt)), change_expired: true };
    let outcome = pam::authenticate_then(service, &plan.account.name, &mut relay, &opts, &mut |h| {
        // The check's limit does not apply to the session.
        // SAFETY: cancels this process's alarm.
        unsafe { alarm(0) };
        let _ = (&said).write_all(b"Y");
        code = run_session(h, plan);
    });
    match outcome {
        Outcome::Yes => code,
        Outcome::No => 1,
        Outcome::GivenUp if relay.answered => 1,
        Outcome::GivenUp => 2,
        Outcome::Broken => 3,
    }
}

/// The runtime directory, where no PAM module made one (review I3): Linux
/// `/run/user/<uid>`, FreeBSD `/var/run/xdg/<user>` (pam_xdg's), the user's and
/// 0700; one that is there must be the user's and not a link.
fn runtime_dir(a: &Account) -> std::io::Result<std::path::PathBuf> {
    let dir = if cfg!(target_os = "linux") {
        std::path::PathBuf::from(format!("/run/user/{}", a.uid))
    } else {
        std::path::PathBuf::from(format!("/var/run/xdg/{}", a.name))
    };
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
    for kv in [
        "XDG_SEAT=seat0".to_string(),
        format!("XDG_VTNR={}", plan.vt),
        "XDG_SESSION_TYPE=wayland".into(),
        "XDG_SESSION_CLASS=user".into(),
    ] {
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
    let mut env: Vec<(String, String)> =
        h.env().into_iter().filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect();
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
            let shell_name = std::path::Path::new(&a.shell)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "sh".into());
            let home = if std::path::Path::new(&a.home).is_dir() { a.home.clone() } else { "/".into() };
            let opened = vergence_dm::vt::open(plan.vt);
            let null = std::fs::OpenOptions::new().write(true).open("/dev/null");
            match (opened, null) {
                (Ok(tty_file), Ok(null)) => {
                    #[cfg(target_os = "freebsd")]
                    let name = std::ffi::CString::new(a.name.clone()).unwrap_or_default();
                    let (uid, gid, vt) = (a.uid, a.gid, plan.vt);
                    let groups = sys::groups_of(&a.name, gid);
                    let mut cmd = std::process::Command::new(&a.shell);
                    // A login shell ("-zsh"), which reads the user's profile, runs the session.
                    cmd.arg0(format!("-{shell_name}"))
                        .args(["-c", &format!("exec {}", plan.exec)])
                        .env_clear()
                        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                        .current_dir(&home)
                        .stdin(tty_file)
                        .stdout(
                            null.try_clone().unwrap_or_else(|_| std::fs::File::open("/dev/null").expect("/dev/null")),
                        )
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
                            drop_to(uid, gid, &groups)
                        })
                    };
                    match cmd.spawn() {
                        Ok(child) => {
                            log(&format!(
                                "session for {} started on VT {vt} (pid {}): {}",
                                a.name,
                                child.id(),
                                plan.exec
                            ));
                            let status = wait_reaping(child.id());
                            log(&format!(
                                "session for {} ended ({})",
                                a.name,
                                status.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string())
                            ));
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

/// Wait for the worker's own child `pid`, reaping on the way every orphan the
/// reaper role (sys::become_reaper) hands this worker: a process whose parent
/// ended before it did (an app started from a shell, a crashed helper) would
/// otherwise stay a zombie until the session ends.
fn wait_reaping(pid: u32) -> std::io::Result<std::process::ExitStatus> {
    use std::os::unix::process::ExitStatusExt;
    loop {
        let mut status: c_int = 0;
        // SAFETY: an out-parameter we own.
        let got = unsafe { waitpid(-1, &mut status, 0) };
        if got == pid as c_int {
            return Ok(std::process::ExitStatus::from_raw(status));
        }
        if got < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() != std::io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
        // Any other pid: an orphan, now gone.
    }
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
    // SIGKILL until none is left (on Linux the tree is read and then
    // signalled, so one that forks in a loop needs more than one pass).
    let mut killed = 0;
    for _ in 0..10 {
        let n = sys::signal_descendants(9);
        killed += n;
        std::thread::sleep(Duration::from_millis(100));
        reap();
        if n == 0 || sys::count_descendants() == 0 {
            break;
        }
    }
    log(&format!(
        "session for {who}: {asked} process(es) left at its end were ended{}",
        if killed > 0 { format!(", {killed} by force") } else { String::new() }
    ));
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
    /// when it is stopped (a blank screen otherwise).
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
fn start_greeter(uid: u32, gid: u32, close: &[c_int]) -> std::io::Result<c_int> {
    let server = SERVER
        .get()
        .cloned()
        .flatten()
        .filter(|p| p.is_file())
        .ok_or_else(|| std::io::Error::other("no vergence-server beside vdm ($prefix/bin)"))?;
    let base = std::path::Path::new(paths::GREETER_SOCKET)
        .parent()
        .ok_or_else(|| std::io::Error::other("no run directory"))?;
    let home = base.join("greeter-home");
    let run = home.join("run");
    // Nothing of a previous login screen runs on (security review
    // 2026-11-16, finding 2): every process of the greeter account is ended
    // before a new one starts, by a child that becomes that account and
    // signals all of its processes.
    end_account_processes(uid, gid);
    if home.symlink_metadata().is_ok() {
        std::fs::remove_dir_all(&home)?;
    }
    // Both made while root owns them, the home handed over last (finding 1:
    // with the home already _vdm's, a leftover could swap `run` for a link
    // and have root's chown give it any file). Neither is followed if a link.
    for d in [&home, &run] {
        std::fs::DirBuilder::new().mode(0o700).create(d)?;
    }
    for d in [&run, &home] {
        chown_no_follow(d, uid, gid)?;
    }
    // What the login screen keeps from one start to the next (its text size):
    // its own small directory under vdm's state, the one thing that lasts.
    let keep = std::path::Path::new(paths::GREETER_KEEP);
    if keep.symlink_metadata().is_err() {
        std::fs::DirBuilder::new().mode(0o700).create(keep)?;
    }
    chown_no_follow(keep, uid, gid)?;
    let tty = vergence_dm::vt::open(vergence_dm::vt::GREETER_VT)?;
    let out = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(std::path::Path::new(paths::STATE_DIR).join("greeter.log"))?;
    // A worker holds the login screen's PAM session (logind's seat goes only
    // to a process in a session, review I4) and starts the server in it; vdm
    // knows the worker, which passes SIGTERM on to the server.
    // SAFETY: vdm is single-threaded; the worker runs greeter_worker and exits.
    let pid = unsafe { fork() };
    if pid < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if pid > 0 {
        return Ok(pid);
    }
    // vdm's sockets and connections are not the worker's to hold (finding 10:
    // a server vdm dropped never saw its connection close).
    sys::close_fds(close, &[]);
    reset_sigterm();
    let code = greeter_worker(uid, gid, &server, &home, &run, tty, out);
    // SAFETY: leave without running the parent's exit handlers.
    unsafe { _exit(code) }
}

/// In a forked child before exec: become `uid`/`gid` with `groups` (read
/// before the fork: no name lookups after it, finding 10), and make sure
/// root cannot be had back.
fn drop_to(uid: u32, gid: u32, groups: &[u32]) -> std::io::Result<()> {
    unsafe extern "C" {
        #[cfg(target_os = "linux")]
        fn setgroups(n: usize, list: *const u32) -> c_int;
        #[cfg(not(target_os = "linux"))]
        fn setgroups(n: c_int, list: *const u32) -> c_int;
    }
    // SAFETY: a list and its length, ours.
    let r = unsafe { setgroups(groups.len() as _, groups.as_ptr()) };
    // SAFETY: plain id changes.
    if r != 0 || unsafe { setgid(gid) } != 0 || unsafe { setuid(uid) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: as above; must fail now.
    if unsafe { setuid(0) } == 0 {
        return Err(std::io::Error::other("could still become root"));
    }
    Ok(())
}

/// In a forked child: files it writes at most `bytes`; SIGXFSZ ignored, so a
/// write past it fails (EFBIG) instead of ending the program. RLIMIT_FSIZE 1
/// and SIGXFSZ 25 on both.
fn limit_file_size(bytes: u64) {
    unsafe extern "C" {
        fn setrlimit(resource: c_int, rlim: *const [u64; 2]) -> c_int;
    }
    // SAFETY: a limit we own; SIG_IGN is 1.
    unsafe {
        setrlimit(1, &[bytes, bytes]);
        signal(25, 1);
    }
}

/// chown a directory without following a link: opened O_DIRECTORY|O_NOFOLLOW
/// and changed through the descriptor (fchown), so a name swapped for a link
/// fails instead of handing over what it points to.
fn chown_no_follow(path: &std::path::Path, uid: u32, gid: u32) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    const FLAGS: i32 = 0o200000 | 0o400000; // O_DIRECTORY | O_NOFOLLOW
    #[cfg(not(target_os = "linux"))]
    const FLAGS: i32 = 0x0002_0000 | 0x0100; // O_DIRECTORY | O_NOFOLLOW
    unsafe extern "C" {
        fn fchown(fd: c_int, uid: u32, gid: u32) -> c_int;
    }
    let d = std::fs::OpenOptions::new().read(true).custom_flags(FLAGS).open(path)?;
    if !d.metadata()?.is_dir() {
        return Err(std::io::Error::other(format!("{} is not a directory", path.display())));
    }
    // SAFETY: the descriptor we just opened.
    if unsafe { fchown(d.as_raw_fd(), uid, gid) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// End every process of an account (the greeter's): a child becomes it and
/// sends SIGKILL to all it may signal (kill(-1), which spares the sender),
/// then vdm waits for that child.
fn end_account_processes(uid: u32, gid: u32) {
    // SAFETY: vdm is single-threaded; the child only changes ids, signals and exits.
    let pid = unsafe { fork() };
    if pid == 0 {
        // SAFETY: async-signal-safe calls only, then _exit.
        unsafe {
            if setgid(gid) == 0 && setuid(uid) == 0 && setuid(0) != 0 {
                kill(-1, 9);
            }
            _exit(0);
        }
    }
    if pid > 0 {
        let mut status: c_int = 0;
        // SAFETY: our own child.
        unsafe { waitpid(pid, &mut status, 0) };
    }
}

/// In a forked worker: SIGTERM ends it again (vdm's own handler, which only
/// notes a stop for the main loop, is inherited by a fork; finding 10).
fn reset_sigterm() {
    // SAFETY: SIG_DFL is 0.
    unsafe { signal(15, 0) };
}

/// vdm was asked to stop (SIGTERM from its service): it stops the login
/// screen first, so a restarted vdm never finds an old one still on its VT.
/// Sessions it started run on (their workers end them when they end).
static STOPPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn note_stop(_sig: c_int) {
    STOPPING.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// The server the login screen's worker started, for SIGTERM to reach it.
static GREETER_SERVER: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// A SIGTERM that came before the server was started (finding 10).
static GREETER_TERM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn pass_term_on(_sig: c_int) {
    GREETER_TERM.store(true, std::sync::atomic::Ordering::Relaxed);
    let pid = GREETER_SERVER.load(std::sync::atomic::Ordering::Relaxed);
    if pid > 0 {
        // SAFETY: kill is async-signal-safe.
        unsafe { kill(pid, 15) };
    }
}

/// In the login screen's worker: its PAM session (when /etc/pam.d/vdm-greeter
/// is there), the server in it, and the session closed when the server ends.
fn greeter_worker(
    uid: u32,
    gid: u32,
    server: &std::path::Path,
    home: &std::path::Path,
    run: &std::path::Path,
    tty: std::fs::File,
    out: std::fs::File,
) -> c_int {
    // SAFETY: a handler that only touches atomics and calls kill.
    unsafe { signal(15, pass_term_on as extern "C" fn(c_int) as usize) };
    // Everything the login screen starts stays below this worker, and is
    // ended with it (finding 2).
    sys::become_reaper();
    let mut io = Some((tty, out));
    let mut code = 3;
    let tty_path = vergence_dm::vt::tty_path(vergence_dm::vt::GREETER_VT);
    let with_pam = paths::pam_service_exists(paths::GREETER_SERVICE);
    let mut run_it = |env: Vec<(String, String)>| -> c_int {
        let Some((tty, out)) = io.take() else { return 3 };
        match spawn_greeter_server(uid, gid, server, home, run, tty, out, env) {
            Ok(child) => {
                GREETER_SERVER.store(child.id() as i32, std::sync::atomic::Ordering::Relaxed);
                if GREETER_TERM.load(std::sync::atomic::Ordering::Relaxed) {
                    // SAFETY: our own child.
                    unsafe { kill(child.id() as c_int, 15) };
                }
                let _ = wait_reaping(child.id());
                end_leftovers("the login screen");
                0
            }
            Err(e) => {
                log(&format!("login screen: could not start the server: {e}"));
                3
            }
        }
    };
    if with_pam {
        let started = pam::session_only(paths::GREETER_SERVICE, paths::GREETER_USER, false, &mut |h| {
            h.set_item(pam::TTY, &tty_path);
            for kv in [
                "XDG_SEAT=seat0".to_string(),
                format!("XDG_VTNR={}", vergence_dm::vt::GREETER_VT),
                "XDG_SESSION_TYPE=wayland".into(),
                "XDG_SESSION_CLASS=greeter".into(),
            ] {
                h.putenv(&kv);
            }
            let r = h.open_session();
            if r != pam::SUCCESS {
                log(&format!("login screen: PAM would not open its session: {}", pam::describe(r)));
            }
            let env = h
                .env()
                .into_iter()
                .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                .collect();
            code = run_it(env);
            if r == pam::SUCCESS {
                h.close_session();
            }
        });
        if let Err(e) = started {
            log(&format!("login screen: {e}; running without its session"));
            code = run_it(Vec::new());
        }
    } else {
        code = run_it(Vec::new());
    }
    code
}

/// The login screen's server: as the greeter account, on its VT, in a
/// session of its own with the VT as its terminal, the environment from
/// nothing (PAM's session's, then ours).
/// The last session's main screen (ToVdm::MainScreen), for the login
/// screen to start on: STATE_DIR/main-screen, root's, a monitor id only.
fn main_screen_file() -> std::path::PathBuf {
    std::path::Path::new(paths::STATE_DIR).join("main-screen")
}

fn main_screen() -> Option<String> {
    std::fs::read_to_string(main_screen_file()).ok().map(|t| t.trim().to_string()).filter(|t| proto::is_monitor_id(t))
}

/// O_NOFOLLOW: a link planted where the file is written is refused, not followed.
const fn libc_o_nofollow() -> i32 {
    #[cfg(target_os = "linux")]
    return 0o400000;
    #[cfg(not(target_os = "linux"))]
    return 0x0100;
}

/// Kept for the next login screen (written beside, moved into place).
fn keep_main_screen(id: &str) {
    use std::io::Write;
    if !proto::is_monitor_id(id) || main_screen().as_deref() == Some(id) {
        return;
    }
    let path = main_screen_file();
    let beside = path.with_extension("new");
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .custom_flags(libc_o_nofollow())
        .open(&beside)
        .and_then(|mut f| f.write_all(format!("{id}\n").as_bytes()))
        .and_then(|_| std::fs::rename(&beside, &path));
    if let Err(e) = written {
        log(&format!("main screen: could not keep it: {e}"));
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_greeter_server(
    uid: u32,
    gid: u32,
    server: &std::path::Path,
    home: &std::path::Path,
    run: &std::path::Path,
    tty: std::fs::File,
    out: std::fs::File,
    pam_env: Vec<(String, String)>,
) -> std::io::Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    let groups = sys::groups_of(paths::GREETER_USER, gid);
    // logind's runtime directory where it made one; ours otherwise.
    let runtime = pam_env
        .iter()
        .find(|(k, _)| k == "XDG_RUNTIME_DIR")
        .map(|(_, v)| std::path::PathBuf::from(v))
        .unwrap_or_else(|| run.to_path_buf());
    let mut cmd = std::process::Command::new(server);
    cmd.args(["--drm", "--greeter"])
        .env_clear()
        .envs(pam_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .env("HOME", home)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("VDM_KEEP", paths::GREETER_KEEP)
        .env("USER", paths::GREETER_USER)
        .env("LOGNAME", paths::GREETER_USER)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("XDG_SEAT", "seat0")
        .env("XDG_VTNR", vergence_dm::vt::GREETER_VT.to_string())
        .current_dir(home)
        .stdin(tty)
        .stdout(out.try_clone()?)
        .stderr(out);
    // SAFETY: the worker is single-threaded, so the forked child may call these.
    unsafe {
        cmd.pre_exec(move || {
            if setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            vergence_dm::vt::make_controlling(0)?;
            vergence_dm::vt::activate(0, vergence_dm::vt::GREETER_VT)?;
            // Its log (and any file it makes) at most 16 MB, the write past
            // that failing rather than ending it (finding 10: the log grew
            // unbounded in root's state directory).
            limit_file_size(16 << 20);
            drop_to(uid, gid, &groups)
        })
    };
    // The screen to start on: the last session's main screen.
    if let Some(id) = main_screen() {
        cmd.env("VDM_MAIN_SCREEN", id);
    }
    // The machine's keyboard layout (step 6), which xkbcommon takes from these.
    if let Some(k) = vergence_dm::keyboard::system() {
        for (name, v) in
            [("XKB_DEFAULT_LAYOUT", k.layout), ("XKB_DEFAULT_VARIANT", k.variant), ("XKB_DEFAULT_OPTIONS", k.options)]
        {
            if !v.is_empty() {
                cmd.env(name, v);
            }
        }
    }
    cmd.spawn()
}

/// An elevation request waiting for its prompt (docs/elevation-milestone-1.md).
struct Elevation {
    id: u128,
    uid: u32,
    name: String,
    action: vergence_dm::elevate::Action,
    /// The asking program's name, from the kernel's pid.
    who: String,
    /// The asking program, told the outcome at the end.
    program: UnixStream,
    /// The PAM check running for it: its worker and the server's connection.
    checking: Option<(c_int, UnixStream)>,
    since: std::time::Instant,
}

/// How long a prompt waits for its answer.
const ELEVATION_LIMIT: Duration = Duration::from_secs(120);

/// Whether an account is an administrator (decision 1 a in
/// docs/elevation-milestone-1.md): in the system's
/// administrators' group (wheel). struct group: gr_name, gr_passwd, gr_gid
/// at byte 16, on both.
fn is_admin(name: &str, gid: u32) -> bool {
    unsafe extern "C" {
        fn getgrnam(name: *const c_char) -> *const u8;
    }
    let Ok(g) = std::ffi::CString::new(paths::ADMIN_GROUP) else { return false };
    // SAFETY: vdm is single-threaded; the struct is read at once.
    let p = unsafe { getgrnam(g.as_ptr()) };
    if p.is_null() {
        return false;
    }
    // SAFETY: gr_gid at offset 16 of the struct getgrnam gave.
    let admin = unsafe { std::ptr::read_unaligned(p.add(16) as *const u32) };
    sys::groups_of(name, gid).contains(&admin)
}

/// An approved action, done by vdm's own code: what it did, or why not.
/// `asker`: the account that asked (a picture is read as them).
fn carry_out(action: &vergence_dm::elevate::Action, asker: Option<&Account>) -> Result<String, String> {
    use vergence_dm::elevate::Action;
    match action {
        Action::LoginWallpaper(path) => {
            let a = asker.ok_or("no such account")?;
            let groups = sys::groups_of(&a.name, a.gid);
            let bytes = read_as(a.uid, a.gid, &groups, path)?;
            let ext = picture_kind(&bytes).ok_or("not a PNG, JPEG, WebP or SVG picture")?;
            put_login_wallpaper(&bytes, ext).map(|_| "the login screen shows the new picture".into())
        }
        Action::LoginWallpaperDefault => {
            for n in paths::LOGIN_WALLPAPER_NAMES {
                match std::fs::remove_file(std::path::Path::new(paths::LOOK_DIR).join(n)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            Ok("the login screen shows Vergence's own picture".into())
        }
        Action::AutologinOn(user) => match autologin_allowed() {
            Ok(one) if &one == user => std::fs::write(paths::autologin_file(), format!("{user}\n"))
                .map(|_| format!("automatic login is on for {user}"))
                .map_err(|e| e.to_string()),
            Ok(one) => Err(format!("only {one} can log in here")),
            Err(why) => Err(why),
        },
        Action::AutologinOff => match std::fs::remove_file(paths::autologin_file()) {
            Ok(()) => Ok("automatic login is off".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok("automatic login was off".into()),
            Err(e) => Err(e.to_string()),
        },
        Action::LoginScreenOn => std::fs::write(paths::login_screen_file(), b"")
            .map(|_| "the login screen starts at boot".into())
            .map_err(|e| e.to_string()),
        Action::LoginScreenOff => match std::fs::remove_file(paths::login_screen_file()) {
            Ok(()) => Ok("no login screen at boot".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok("there was no login screen at boot".into()),
            Err(e) => Err(e.to_string()),
        },
    }
}

/// The most a login screen picture may be (an 8K JPEG is about 20 MB).
const LOGIN_WALLPAPER_MAX: usize = 40 * 1024 * 1024;

/// A file's bytes, read by a child that has become `uid` (with its groups),
/// so root reads nothing the person could not: the path is theirs to name.
/// Only a plain file; opened without waiting (a FIFO named in its place does
/// not hang vdm); at most LOGIN_WALLPAPER_MAX; five seconds at most.
fn read_as(uid: u32, gid: u32, groups: &[u32], path: &str) -> Result<Vec<u8>, String> {
    use std::io::{Read, Write};
    use std::os::unix::fs::OpenOptionsExt;
    #[cfg(target_os = "linux")]
    const O_NONBLOCK: i32 = 0o4000;
    #[cfg(not(target_os = "linux"))]
    const O_NONBLOCK: i32 = 0x4;
    let (mut ours, mut theirs) = UnixStream::pair().map_err(|e| e.to_string())?;
    // SAFETY: vdm is single-threaded; the child changes ids, reads one file,
    // writes it to its end of the pair and exits.
    let pid = unsafe { fork() };
    if pid < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if pid == 0 {
        drop(ours);
        reset_sigterm();
        let code = (|| -> Result<(), ()> {
            drop_to(uid, gid, groups).map_err(|_| ())?;
            let mut f = std::fs::OpenOptions::new().read(true).custom_flags(O_NONBLOCK).open(path).map_err(|_| ())?;
            if !f.metadata().map_err(|_| ())?.is_file() {
                return Err(());
            }
            let mut buf = Vec::new();
            Read::by_ref(&mut f).take(LOGIN_WALLPAPER_MAX as u64 + 1).read_to_end(&mut buf).map_err(|_| ())?;
            theirs.write_all(&buf).map_err(|_| ())
        })();
        // SAFETY: the child's end.
        unsafe { _exit(if code.is_ok() { 0 } else { 1 }) };
    }
    drop(theirs);
    let _ = ours.set_read_timeout(Some(Duration::from_secs(5)));
    let mut bytes = Vec::new();
    let read = Read::by_ref(&mut ours).take(LOGIN_WALLPAPER_MAX as u64 + 1).read_to_end(&mut bytes);
    let mut status: c_int = 0;
    if read.is_err() {
        // SAFETY: our own child, stuck: ended.
        unsafe { kill(pid, 9) };
    }
    // SAFETY: our own child.
    unsafe { waitpid(pid, &mut status, 0) };
    if read.is_err() {
        return Err("the picture took too long to read".into());
    }
    if status != 0 {
        return Err("the picture could not be read as you (is it a file you may read?)".into());
    }
    if bytes.len() > LOGIN_WALLPAPER_MAX {
        return Err("the picture is larger than 40 MB".into());
    }
    Ok(bytes)
}

/// What kind of picture bytes are, by their first bytes only (root never
/// decodes them; the login screen does, as its own account, in the decode
/// helper): the file name it is kept under.
fn picture_kind(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("login-wallpaper.png")
    } else if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("login-wallpaper.jpg")
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("login-wallpaper.webp")
    } else {
        let head = String::from_utf8_lossy(&b[..b.len().min(4096)]);
        let t = head.trim_start_matches('\u{feff}').trim_start();
        ((t.starts_with("<?xml") || t.starts_with("<svg") || t.starts_with("<!--")) && head.contains("<svg"))
            .then_some("login-wallpaper.svg")
    }
}

/// Keep the login screen's picture: LOOK_DIR made root's (0755, not followed
/// if a link), the bytes written beside the final name and renamed over it
/// (0644), any picture of another kind removed.
fn put_login_wallpaper(bytes: &[u8], name: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;
    let dir = std::path::Path::new(paths::LOOK_DIR);
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_dir() && m.uid() == 0 => {}
        Ok(_) => return Err(format!("{} is not root's directory", dir.display())),
        Err(_) => std::fs::DirBuilder::new().mode(0o755).create(dir).map_err(|e| e.to_string())?,
    }
    let tmp = dir.join(format!(".{name}.new"));
    let _ = std::fs::remove_file(&tmp);
    let mut f =
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o644).open(&tmp).map_err(|e| e.to_string())?;
    f.write_all(bytes).and_then(|_| f.sync_all()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join(name)).map_err(|e| e.to_string())?;
    for other in paths::LOGIN_WALLPAPER_NAMES.iter().filter(|n| **n != name) {
        let _ = std::fs::remove_file(dir.join(other));
    }
    Ok(())
}

/// Tell the asking program how it ended.
fn finish(mut program: UnixStream, outcome: Result<String, String>) {
    let _ = program.set_write_timeout(Some(Duration::from_secs(1)));
    let ok = outcome.is_ok();
    let text = match outcome {
        Ok(t) | Err(t) => t,
    };
    let _ = proto::send_from_vdm(&mut program, &if ok { FromVdm::Info(text) } else { FromVdm::Error(text) });
    let _ = proto::send_from_vdm(&mut program, &FromVdm::Done { ok, wait: 0 });
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
        let mut fds: Vec<PollFd> =
            waiting.iter().map(|i| PollFd { fd: sessions[*i].1.as_raw_fd(), events: 1, revents: 0 }).collect();
        // SAFETY: pollfds we own, as many as we pass.
        unsafe { poll(fds.as_mut_ptr(), fds.len() as u64, left.as_millis().min(i32::MAX as u128) as c_int) };
        let ready: Vec<usize> = waiting.iter().zip(&fds).filter(|(_, f)| f.revents != 0).map(|(i, _)| *i).collect();
        for i in ready {
            waiting.retain(|w| *w != i);
            let (uid, s) = &mut sessions[i];
            let who = user_name(*uid).unwrap_or_default();
            // The whole answer by the deadline, however slowly it comes (finding 3).
            match proto::read_to_vdm(&mut sys::Deadline { stream: &*s, until: deadline }) {
                Ok(ToVdm::Presented) => shown += 1,
                Ok(ToVdm::CannotLock(why)) => {
                    cannot += 1;
                    log(&format!("before sleep: {who}'s session cannot lock: {why}"));
                }
                // Its main screen, said just then: kept; its answer is still to come.
                Ok(ToVdm::MainScreen(id)) => {
                    keep_main_screen(&id);
                    waiting.push(i);
                }
                _ => dead.push(i),
            }
        }
    }
    for i in &waiting {
        log(&format!(
            "before sleep: {}'s session did not show the lock in time",
            user_name(sessions[*i].0).unwrap_or_default()
        ));
    }
    dead.sort_unstable();
    for i in dead.into_iter().rev() {
        sessions.remove(i);
    }
    let said = format!("{shown} of {total} sessions locked, {cannot} could not lock, {} did not answer", waiting.len());
    log(&format!("before sleep: {said}"));
    said
}

/// The display server the login screen runs, beside vdm ($prefix/bin), found
/// once at the start: an install that replaces vdm's file while it runs (a
/// session up, its restart left for later) leaves the running vdm unable to
/// name its own file on FreeBSD, and every login screen after the next
/// log out failed to start.
static SERVER: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();

fn main() {
    let _ = SERVER.set(
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().and_then(|d| d.parent()).map(|p| p.join("bin/vergence-server"))),
    );
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("before-sleep") {
        before_sleep();
    }
    if args.get(1).map(String::as_str) == Some("autologin") {
        let line = match (args.get(2).map(String::as_str), args.get(3)) {
            (Some("on"), Some(u)) => format!("autologin on {u}"),
            (Some("on"), None) => "autologin on".to_string(),
            (Some("off"), _) => "autologin off".to_string(),
            _ => {
                eprintln!("usage: vdm autologin on [USER] | off");
                std::process::exit(2)
            }
        };
        control_command(&line);
    }
    // The login screen at boot, on or off (root): the file vdm reads when it
    // starts (paths::login_screen_file); what `tools/install.sh --vdm` did,
    // for a package's admin step.
    if args.get(1).map(String::as_str) == Some("login-screen") {
        let file = paths::login_screen_file();
        let r = match args.get(2).map(String::as_str) {
            Some("on") => std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(paths::STATE_DIR)
                .and_then(|_| std::fs::write(&file, b"")),
            Some("off") => std::fs::remove_file(&file)
                .or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }),
            // vdm's state directory is root's alone: an ordinary account
            // can't see the file, so it is told so, not "off".
            Some("status") | None => match std::fs::metadata(&file) {
                Ok(_) => {
                    println!("login screen at boot: on");
                    std::process::exit(0)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("login screen at boot: off");
                    std::process::exit(0)
                }
                Err(e) => {
                    eprintln!("vdm: {}: {e} (as root: sudo vdm login-screen status)", file.display());
                    std::process::exit(1)
                }
            },
            _ => {
                eprintln!("usage: vdm login-screen on | off | status");
                std::process::exit(2)
            }
        };
        match r {
            Ok(()) => {
                println!("login screen at boot: {} (from vdm's next start)", if file.exists() { "on" } else { "off" });
                std::process::exit(0)
            }
            Err(e) => {
                eprintln!("vdm: {}: {e} (as root: sudo vdm login-screen ...)", file.display());
                std::process::exit(1)
            }
        }
    }
    if args.get(1).map(String::as_str) == Some("greeter") {
        let line = if args.get(2).map(String::as_str) == Some("stop") { "greeter stop" } else { "greeter" };
        control_command(line);
    }
    let opt = |name: &str, default: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };
    let socket = opt("--socket", paths::UNLOCK_SOCKET);
    let service = opt("--service", paths::UNLOCK_SERVICE);
    if sys::euid() != 0 {
        eprintln!("vdm: must run as root");
        std::process::exit(1);
    }
    // Our own PAM file, or nothing: without it PAM would use "other", which
    // differs between systems (review I2).
    if !paths::pam_service_exists(&service) {
        log(&format!(
            "{}/{service} is missing; not starting (the package or tools/install.sh puts it there)",
            paths::PAM_DIRS.join(" or ")
        ));
        std::process::exit(1);
    }
    sys::no_core_files();
    sys::no_debugger();
    // FreeBSD: vdm writes to syslog itself (log()); its standard output and
    // error, daemon(8)'s pipe, go to /dev/null, so no session or login screen
    // started from it holds that pipe: daemon waited for them to close it
    // before a restart could finish.
    #[cfg(target_os = "freebsd")]
    if let Ok(null) = std::fs::OpenOptions::new().write(true).open("/dev/null") {
        unsafe extern "C" {
            fn dup2(from: c_int, to: c_int) -> c_int;
        }
        // SAFETY: our own descriptors; /dev/null stays open in 1 and 2.
        unsafe {
            dup2(null.as_raw_fd(), 1);
            dup2(null.as_raw_fd(), 2);
        }
    }

    let listener = bind(&socket, 0o666);
    let session_listener = bind(paths::SESSION_SOCKET, 0o666);
    let control = bind(paths::CONTROL_SOCKET, 0o600);
    // Sleep, Restart, Shut Down (step 6): anyone may ask; vdm decides by who.
    let power = bind(paths::POWER_SOCKET, 0o666);
    // Elevation (docs/elevation-milestone-1.md): anyone may ask; vdm decides.
    let elevate = bind(paths::ELEVATE_SOCKET, 0o666);
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
    // Logins by their worker's pid: checking, then (yes) the running session
    // (made above, with an automatic login's).
    // The login screen at boot, where the machine is set up for it (step 7).
    let at_boot = paths::login_screen_file().is_file() && greeter_account.is_some();
    if at_boot {
        log(&format!("the login screen starts with vdm ({} is there)", paths::login_screen_file().display()));
    }
    // At boot (the machine up less than 3 minutes): at once, so the text
    // console shows only for a moment. Otherwise (vdm
    // restarted under running servers) after 6 s: they have registered by
    // then, and the login screen waits for them.
    let first = (at_boot && sys::uptime() >= 180).then(|| std::time::Instant::now() + Duration::from_secs(6));
    {
        // SAFETY: a handler that only sets an atomic; SIGTERM 15.
        unsafe { signal(15, note_stop as extern "C" fn(c_int) as usize) };
    }
    // Automatic login at boot (step 8): only at boot, only if turned on, and
    // only while the rules still hold (checked now, every boot: M6).
    let mut logins: HashMap<c_int, LoginRun> = HashMap::new();
    // Once per boot (finding 8: a vdm restarted soon after boot logged in
    // again): a mark in the run directory, which boot clears.
    let autologin_done = std::path::Path::new(paths::GREETER_SOCKET).with_file_name("autologin-done");
    if at_boot
        && sys::uptime() < 180
        && !autologin_done.exists()
        && std::fs::write(&autologin_done, b"").is_ok()
        && let Ok(wanted) = std::fs::read_to_string(paths::autologin_file())
    {
        let wanted = wanted.trim().to_string();
        match autologin_allowed() {
            Ok(user) if user == wanted => {
                let exec = vergence_dm::sessions::list()
                    .first()
                    .and_then(|e| std::fs::read_to_string(&e.file).ok())
                    .and_then(|t| vergence_dm::sessions::entry_exec(&t));
                match (account(&user), exec, UnixStream::pair()) {
                    (Some(acct), Some(exec), Ok((said, _other))) => {
                        let uid = acct.uid;
                        let vt = vergence_dm::vt::GREETER_VT + 1;
                        let plan = LoginPlan { account: acct, exec, vt };
                        // SAFETY: vdm is single-threaded; the worker runs the session and exits.
                        let pid = unsafe { fork() };
                        if pid == 0 {
                            reset_sigterm();
                            let mut code = 4;
                            let started = pam::session_only(paths::AUTOLOGIN_SERVICE, &user, true, &mut |h| {
                                code = run_session(h, &plan)
                            });
                            if let Err(e) = &started {
                                log(&format!("automatic login for {user}: {e}"));
                            }
                            // SAFETY: leave without running the parent's exit handlers.
                            unsafe { _exit(if started.is_ok() { code } else { 3 }) };
                        }
                        if pid > 0 {
                            log(&format!("automatic login for {user} on VT {vt}"));
                            logins.insert(pid, LoginRun { uid, name: user.clone(), client: None, said, vt, yes: true });
                        }
                    }
                    _ => log(&format!("automatic login for {user}: no account or no session to start")),
                }
            }
            Ok(user) => {
                log(&format!("automatic login not done: it is set for {wanted}, but {user} is the one who can log in"))
            }
            Err(why) => log(&format!("automatic login not done: {why}")),
        }
    }
    let mut allowed_cache: Option<(std::time::Instant, Result<String, String>)> = None;
    // Elevation requests waiting for their prompt's answer, one per person.
    let mut elevations: Vec<Elevation> = Vec::new();
    // When a person last said no to a prompt (or let it run out), and how
    // many times in a row: a new request waits 10 s, then 20, 40 ... up to
    // 5 minutes (elevation review, finding 4: a program asking again and
    // again); a yes clears it.
    let mut declined: HashMap<u32, (std::time::Instant, u32)> = HashMap::new();
    // Each registered server's pid, by its connection's descriptor (an
    // asker must run below one of its person's: finding 3).
    let mut session_pids: HashMap<c_int, u32> = HashMap::new();
    let mut greeter_run = Greeter {
        on: at_boot,
        pid: 0,
        started: std::time::Instant::now(),
        quick: 0,
        next: first,
        came_from: None,
        back_to: None,
    };
    // The power key, held from the system while a session is registered
    // (menus milestone 1: it opens the session dialog); one a vdm before
    // this one never gave back, given back first.
    vergence_dm::powerkey::recover();
    let mut power_hold: Option<vergence_dm::powerkey::Hold> = None;
    // FreeBSD: devd's pipe, for the power button the server cannot see, and
    // what of a line has come; tried again every 10 s while it is not there.
    let mut devd: Option<(UnixStream, Vec<u8>)> = None;
    let mut devd_next = std::time::Instant::now();
    loop {
        if STOPPING.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(h) = power_hold.take() {
                vergence_dm::powerkey::release(h);
            }
            if greeter_run.pid > 0 {
                // SAFETY: our own child (the worker passes it on to the server).
                unsafe { kill(greeter_run.pid, 15) };
                let until = std::time::Instant::now() + Duration::from_secs(3);
                while std::time::Instant::now() < until {
                    let mut status: c_int = 0;
                    // SAFETY: an out-parameter we own.
                    if unsafe { waitpid(greeter_run.pid, &mut status, WNOHANG) } == greeter_run.pid {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            // Its sockets go with it (finding 9: a lock checking for them
            // would have locked with no one to open it).
            for p in [
                socket.as_str(),
                paths::SESSION_SOCKET,
                paths::CONTROL_SOCKET,
                paths::POWER_SOCKET,
                paths::ELEVATE_SOCKET,
                paths::GREETER_SOCKET,
            ] {
                let _ = std::fs::remove_file(p);
            }
            log("stopping (asked to); the login screen stopped, sessions run on");
            std::process::exit(0);
        }
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
        // Never over a session: one of vdm's, or a server registered from a
        // console (whose end then brings the login screen).
        let session_running = logins.values().any(|l| l.yes) || !sessions.is_empty();
        if greeter_run.on
            && greeter_run.pid == 0
            && !session_running
            && greeter_run.next.is_none_or(|t| std::time::Instant::now() >= t)
        {
            greeter_run.next = None;
            let mut ours: Vec<c_int> = [
                Some(&listener),
                Some(&session_listener),
                Some(&control),
                Some(&power),
                Some(&elevate),
                greeter.as_ref(),
            ]
            .into_iter()
            .flatten()
            .map(|l| l.as_raw_fd())
            .collect();
            ours.extend(sessions.iter().map(|(_, s)| s.as_raw_fd()));
            ours.extend(running.values().map(|(_, s, _)| s.as_raw_fd()));
            ours.extend(
                logins
                    .values()
                    .flat_map(|l| [Some(l.said.as_raw_fd()), l.client.as_ref().map(|c| c.as_raw_fd())])
                    .flatten(),
            );
            ours.extend(
                elevations
                    .iter()
                    .flat_map(|e| [Some(e.program.as_raw_fd()), e.checking.as_ref().map(|(_, s)| s.as_raw_fd())])
                    .flatten(),
            );
            match greeter_account.as_ref().map(|a| start_greeter(a.uid, a.gid, &ours)) {
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
                    log(&format!(
                        "login screen: ended {GIVE_UP} times in a row within {} s; not starting it again (see {}/greeter.log)",
                        QUICK.as_secs(),
                        paths::STATE_DIR
                    ));
                } else if greeter_run.on {
                    let pause = 1u64 << greeter_run.quick.min(6);
                    greeter_run.next = Some(std::time::Instant::now() + Duration::from_secs(pause));
                }
                continue;
            }
            if let Some(i) = elevations.iter().position(|e| e.checking.as_ref().is_some_and(|(p, _)| *p == pid)) {
                let code = if status & 0x7f == 0 { (status >> 8) & 0xff } else { -1 };
                let (_, mut server) = elevations[i].checking.take().expect("checking");
                let _ = server.set_write_timeout(Some(Duration::from_secs(1)));
                let uid = elevations[i].uid;
                if code == 0 {
                    pauses.succeeded(uid);
                    declined.remove(&uid);
                    let e = elevations.remove(i);
                    let _ = proto::send_from_vdm(&mut server, &FromVdm::Done { ok: true, wait: 0 });
                    let outcome = carry_out(&e.action, account(&e.name).as_ref());
                    log(&format!(
                        "elevation: {} by {} through {}: {}",
                        e.action.name(),
                        e.name,
                        e.who,
                        match &outcome {
                            Ok(m) => m.clone(),
                            Err(m) => format!("failed: {m}"),
                        }
                    ));
                    finish(e.program, outcome);
                } else {
                    if code == 1 {
                        pauses.failed(uid);
                    }
                    let wait = pauses.wait(uid);
                    log(&format!(
                        "elevation: {} by {}: {}",
                        elevations[i].action.name(),
                        elevations[i].name,
                        if code == 1 {
                            format!("wrong password (a pause of {wait} s)")
                        } else {
                            format!("not checked (status {code})")
                        }
                    ));
                    let _ = proto::send_from_vdm(
                        &mut server,
                        &FromVdm::Done { ok: false, wait: wait.min(u16::MAX as u64) as u16 },
                    );
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
                        log(&format!(
                            "login for {}: no (a pause of {} s before the next try)",
                            l.name,
                            pauses.wait(l.uid)
                        ));
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
        // The power key: the session's while one is registered, else the system's.
        if !sessions.is_empty() && power_hold.is_none() {
            let (h, done) = vergence_dm::powerkey::hold();
            log(&format!(
                "power key: the session's ({})",
                if done.is_empty() { "nothing to hold it from" } else { &done }
            ));
            power_hold = Some(h);
        } else if sessions.is_empty()
            && let Some(h) = power_hold.take()
        {
            vergence_dm::powerkey::release(h);
            log("power key: the system's again");
        }
        if cfg!(target_os = "freebsd") && devd.is_none() && std::time::Instant::now() >= devd_next {
            devd_next = std::time::Instant::now() + Duration::from_secs(10);
            if let Ok(s) = UnixStream::connect(vergence_dm::powerkey::DEVD_PIPE)
                && s.set_nonblocking(true).is_ok()
            {
                devd = Some((s, Vec::new()));
            }
        }
        let mut fds: Vec<PollFd> =
            [Some(&listener), Some(&session_listener), Some(&control), Some(&power), Some(&elevate), greeter.as_ref()]
                .into_iter()
                .flatten()
                .map(|l| PollFd { fd: l.as_raw_fd(), events: 1, revents: 0 })
                .collect();
        if let Some((d, _)) = &devd {
            fds.push(PollFd { fd: d.as_raw_fd(), events: 1, revents: 0 });
        }
        let first_session = fds.len();
        fds.extend(sessions.iter().map(|(_, s)| PollFd { fd: s.as_raw_fd(), events: 1, revents: 0 }));
        // SAFETY: pollfds we own. A check running: look again in 50 ms, so its
        // answer goes out as soon as it ends; none: a second's wait.
        let wait = if running.is_empty()
            && greeter_run.next.is_none()
            && greeter_run.back_to.is_none()
            && logins.values().all(|l| l.yes)
        {
            1000
        } else {
            50
        };
        unsafe { poll(fds.as_mut_ptr(), fds.len() as u64, wait) };
        // devd's lines: the power button pressed, while the key is the
        // session's, goes to every registered server (FreeBSD).
        if let Some((d, pending)) = devd.as_mut() {
            let mut buf = [0u8; 4096];
            let mut gone = false;
            loop {
                match d.read(&mut buf) {
                    Ok(0) => {
                        gone = true;
                        break;
                    }
                    Ok(n) => pending.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        gone = true;
                        break;
                    }
                }
            }
            let mut pressed = false;
            while let Some(i) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=i).collect();
                pressed |= vergence_dm::powerkey::is_power_button(&String::from_utf8_lossy(&line));
            }
            // A line that never ends is not kept for ever.
            if pending.len() > 16 * 1024 {
                pending.clear();
            }
            if pressed && power_hold.is_some() {
                log("power key: pressed; the session asks");
                for (_, s) in sessions.iter_mut() {
                    let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
                    let _ = proto::send_from_vdm(s, &FromVdm::PowerKey);
                }
            }
            if gone {
                devd = None;
            }
        }
        // A registered server that went away (or said something out of turn):
        // dropped. Its main screen is the one thing it may say unasked.
        let gone: Vec<usize> = fds[first_session..]
            .iter()
            .enumerate()
            .filter(|(_, f)| f.revents != 0)
            .map(|(i, _)| i)
            .filter(|&i| {
                let s = &sessions[i].1;
                let until = std::time::Instant::now() + Duration::from_secs(1);
                match proto::read_to_vdm(&mut sys::Deadline { stream: s, until }) {
                    Ok(ToVdm::MainScreen(id)) => {
                        keep_main_screen(&id);
                        false
                    }
                    _ => true,
                }
            })
            .collect();
        for i in gone.into_iter().rev() {
            sessions.remove(i);
        }
        // A server registering: the connection is the registration.
        // Only a server on one of the machine's consoles counts as someone at
        // the machine (finding 4: an ssh login registering got Sleep,
        // Restart and Shut Down, and kept the login screen away).
        if let Ok((s, _)) = session_listener.accept() {
            let _ = s.set_nonblocking(false);
            let console = sys::peer_pid(&s).is_some_and(sys::on_console);
            match sys::peer_uid(&s) {
                Some(uid) if uid != 0 && uid >= paths::MIN_UID && !console => {
                    log(&format!(
                        "session registration from {} refused: not on a console of this machine",
                        user_name(uid).unwrap_or_else(|| uid.to_string())
                    ));
                }
                Some(uid) if uid != 0 && uid >= paths::MIN_UID => {
                    if sessions.iter().filter(|(u, _)| *u == uid).count() >= SESSIONS_PER_UID
                        && let Some(oldest) = sessions.iter().position(|(u, _)| *u == uid)
                    {
                        sessions.remove(oldest);
                    }
                    if let Some(p) = sys::peer_pid(&s) {
                        session_pids.insert(s.as_raw_fd(), p);
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
                    // from it (it once started over a running session by mistake).
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
                    cmd if cmd.starts_with("autologin on") => {
                        let asked = cmd.strip_prefix("autologin on").map(str::trim).filter(|u| !u.is_empty());
                        match autologin_allowed() {
                            Ok(user) if asked.is_none_or(|a| a == user) => match std::fs::write(paths::autologin_file(), format!("{user}\n")) {
                                Ok(()) => {
                                    log(&format!("automatic login turned on for {user}"));
                                    format!("automatic login is on for {user} (at boot; the rules are checked again each time)")
                                }
                                Err(e) => format!("not turned on: {e}"),
                            },
                            Ok(user) => format!("not turned on: only {user} can log in here"),
                            Err(why) => format!("not turned on: {why}"),
                        }
                    }
                    "autologin off" => {
                        let _ = std::fs::remove_file(paths::autologin_file());
                        log("automatic login turned off");
                        "automatic login is off".to_string()
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
            let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
            let mut line = String::new();
            // One short line within a second, or nothing (finding 3).
            let _ = BufReader::new(
                sys::Deadline { stream: &s, until: std::time::Instant::now() + Duration::from_secs(1) }.take(64),
            )
            .read_line(&mut line);
            let uid = sys::peer_uid(&s);
            let has_session =
                |u: u32| sessions.iter().any(|(x, _)| *x == u) || logins.values().any(|l| l.yes && l.uid == u);
            let others = |u: u32| sessions.iter().any(|(x, _)| *x != u) || logins.values().any(|l| l.yes && l.uid != u);
            let who = uid.map(|u| user_name(u).unwrap_or_else(|| u.to_string())).unwrap_or_else(|| "?".into());
            let answer = match line.trim() {
                // What the settings app's Login page shows (anyone may ask).
                "login-state" => {
                    let auto = std::fs::read_to_string(paths::autologin_file())
                        .ok()
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty());
                    format!(
                        "login-screen={}\tautologin={}\tallowed={}",
                        if paths::login_screen_file().is_file() { "on" } else { "off" },
                        auto.as_deref().unwrap_or("-"),
                        match allowed_cached(&mut allowed_cache) {
                            Ok(u) => u,
                            Err(why) => format!("no: {why}"),
                        }
                    )
                }
                word => match (vergence_dm::power::Power::from_word(word), uid) {
                    (None, _) | (_, None) => "no: not understood".to_string(),
                    (Some(p), Some(u)) => {
                        let allowed = u == 0
                            || (Some(u) == greeter_uid && !logins.values().any(|l| l.yes))
                            || (u >= paths::MIN_UID && has_session(u));
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
                },
            };
            let _ = s.write_all(format!("{answer}\n").as_bytes());
        }
        // Elevation prompts that waited too long: over.
        let now = std::time::Instant::now();
        while let Some(i) =
            elevations.iter().position(|e| e.checking.is_none() && now.duration_since(e.since) > ELEVATION_LIMIT)
        {
            let e = elevations.remove(i);
            log(&format!(
                "elevation: {} by {} through {}: no answer in {} s",
                e.action.name(),
                e.name,
                e.who,
                ELEVATION_LIMIT.as_secs()
            ));
            let n = declined.get(&e.uid).map_or(0, |d| d.1);
            declined.insert(e.uid, (std::time::Instant::now(), n + 1));
            for (_, st) in sessions.iter_mut().filter(|(u, _)| *u == e.uid) {
                let _ = st.set_write_timeout(Some(Duration::from_secs(1)));
                let _ = proto::send_from_vdm(st, &FromVdm::Withdrawn(e.id));
            }
            finish(e.program, Err("no answer at the prompt".into()));
        }
        // A request whose program went away, or whose person has no session
        // left, is over (finding 7): it is not carried out for no one.
        while let Some(i) = elevations.iter().position(|e| {
            e.checking.is_none() && (sys::hung_up(&e.program) || !sessions.iter().any(|(u, _)| *u == e.uid))
        }) {
            let e = elevations.remove(i);
            log(&format!(
                "elevation: {} by {} through {}: withdrawn (the program or the session went away)",
                e.action.name(),
                e.name,
                e.who
            ));
            for (_, st) in sessions.iter_mut().filter(|(u, _)| *u == e.uid) {
                let _ = st.set_write_timeout(Some(Duration::from_secs(1)));
                let _ = proto::send_from_vdm(st, &FromVdm::Withdrawn(e.id));
            }
        }
        // Elevation (docs/elevation-milestone-1.md): a program asking for a
        // named action, or the person's server answering or declining.
        if let Ok((mut c, _)) = elevate.accept() {
            let _ = c.set_nonblocking(false);
            let _ = c.set_write_timeout(Some(Duration::from_secs(1)));
            let until = std::time::Instant::now() + Duration::from_secs(1);
            let (uid, pid) = (sys::peer_uid(&c), sys::peer_pid(&c));
            // Who asks, from the kernel, before anything it sends is read (finding 2).
            let who = pid.map(vergence_dm::elevate::program_shown).unwrap_or_default();
            let first = {
                let mut r = sys::Deadline { stream: &c, until };
                match (proto::read_to_vdm(&mut r), proto::read_to_vdm(&mut r)) {
                    (Ok(ToVdm::Hello { version: proto::VERSION, service: proto::Service::Elevate }), Ok(m)) => Some(m),
                    _ => None,
                }
            };
            let refuse_with = |mut c: UnixStream, why: &str| {
                let _ = proto::send_from_vdm(&mut c, &FromVdm::Error(why.into()));
                let _ = proto::send_from_vdm(&mut c, &FromVdm::Done { ok: false, wait: 0 });
            };
            match (first, uid, pid) {
                (Some(ToVdm::Act(words)), Some(uid), Some(pid)) => {
                    let acct = user_name(uid).and_then(|n| account(&n));
                    let action = vergence_dm::elevate::Action::parse(&words);
                    // Started from the person's own session at the machine: below
                    // its server, or below the worker vdm runs it in (finding 3).
                    let ancestors: Vec<u32> = sessions
                        .iter()
                        .filter(|(u, _)| *u == uid)
                        .filter_map(|(_, st)| session_pids.get(&st.as_raw_fd()).copied())
                        .chain(logins.iter().filter(|(_, l)| l.uid == uid && l.yes).map(|(p, _)| *p as u32))
                        .collect();
                    let at_console = !ancestors.is_empty() && vergence_dm::elevate::descends_from(pid, &ancestors);
                    let pam_file = paths::pam_service_exists(paths::ELEVATE_SERVICE);
                    let wait_declined = declined.get(&uid).map_or(0, |(t, n)| {
                        (10u64 << (n.saturating_sub(1)).min(5)).min(300).saturating_sub(t.elapsed().as_secs())
                    });
                    match (acct, action) {
                        (_, Err(why)) => refuse_with(c, &why),
                        (None, _) => refuse_with(c, "no such account"),
                        (Some(a), Ok(_)) if !may_log_in(a.uid, &a.shell) || !at_console => {
                            refuse_with(c, "only someone logged in at this machine can ask")
                        }
                        (Some(a), Ok(_)) if !is_admin(&a.name, a.gid) => {
                            log(&format!("elevation: refused for {} (not in {})", a.name, paths::ADMIN_GROUP));
                            refuse_with(
                                c,
                                &format!("only an administrator (the {} group) can do that", paths::ADMIN_GROUP),
                            );
                        }
                        (Some(_), Ok(_)) if elevations.iter().any(|e| e.uid == uid) => {
                            refuse_with(c, "another request is waiting at the prompt")
                        }
                        (Some(_), Ok(_)) if wait_declined > 0 => {
                            refuse_with(c, &format!("the last request was declined; wait {wait_declined} s"))
                        }
                        (Some(_), Ok(_)) if !pam_file => refuse_with(
                            c,
                            &format!(
                                "{}/{} is missing (the package or tools/install.sh writes it)",
                                paths::PAM_DIRS[0],
                                paths::ELEVATE_SERVICE
                            ),
                        ),
                        (Some(_), Ok(_)) if pauses.wait(uid) > 0 => {
                            refuse_with(c, &format!("wait {} s after a wrong password", pauses.wait(uid)))
                        }
                        (Some(a), Ok(action)) => {
                            let id = vergence_dm::elevate::random_id();
                            let what = action.describe();
                            // Every server of the person's (finding 5: not only the last to register).
                            let mut sent = false;
                            for (_, st) in sessions.iter_mut().filter(|(u, _)| *u == uid) {
                                let _ = st.set_write_timeout(Some(Duration::from_secs(1)));
                                sent |= proto::send_from_vdm(
                                    st,
                                    &FromVdm::Ask { id, what: what.clone(), who: who.clone() },
                                )
                                .is_ok();
                            }
                            if sent {
                                log(&format!("elevation: {} asks for {} ({})", who, action.name(), a.name));
                                elevations.push(Elevation {
                                    id,
                                    uid,
                                    name: a.name,
                                    action,
                                    who,
                                    program: c,
                                    checking: None,
                                    since: std::time::Instant::now(),
                                });
                            } else {
                                refuse_with(c, "the prompt could not be shown");
                            }
                        }
                    }
                }
                // The person's server answers prompt `id`: its PAM check, in a worker.
                (Some(ToVdm::Answering(id)), Some(uid), Some(pid))
                    if sys::on_console(pid) && vergence_dm::elevate::is_server(pid) =>
                {
                    let elevation_fds: Vec<c_int> = elevations
                        .iter()
                        .flat_map(|e| [Some(e.program.as_raw_fd()), e.checking.as_ref().map(|(_, st)| st.as_raw_fd())])
                        .flatten()
                        .collect();
                    match elevations.iter_mut().find(|e| e.id == id && e.uid == uid && e.checking.is_none()) {
                        Some(e) if pauses.wait(uid) == 0 => {
                            let _ = c.set_read_timeout(Some(IO_LIMIT));
                            let _ = c.set_write_timeout(Some(IO_LIMIT));
                            let name = e.name.clone();
                            if let Ok(theirs) = c.try_clone() {
                                // vdm's sockets and connections, closed in the worker (finding 6).
                                let mut ours: Vec<c_int> = [
                                    Some(&listener),
                                    Some(&session_listener),
                                    Some(&control),
                                    Some(&power),
                                    Some(&elevate),
                                    greeter.as_ref(),
                                ]
                                .into_iter()
                                .flatten()
                                .map(|l| l.as_raw_fd())
                                .collect();
                                ours.extend(sessions.iter().map(|(_, st)| st.as_raw_fd()));
                                ours.extend(running.values().map(|(_, st, _)| st.as_raw_fd()));
                                ours.extend(
                                    logins
                                        .values()
                                        .flat_map(|l| {
                                            [Some(l.said.as_raw_fd()), l.client.as_ref().map(|x| x.as_raw_fd())]
                                        })
                                        .flatten(),
                                );
                                ours.extend(&elevation_fds);
                                ours.push(c.as_raw_fd());
                                // SAFETY: vdm is single-threaded; the child runs check() and exits.
                                let pid = unsafe { fork() };
                                if pid == 0 {
                                    sys::close_fds(&ours, &[theirs.as_raw_fd()]);
                                    reset_sigterm();
                                    // SAFETY: the check's own limit.
                                    unsafe { alarm(TIME_LIMIT) };
                                    let code = check(theirs, &name, paths::ELEVATE_SERVICE, true);
                                    // SAFETY: leave without running the parent's exit handlers.
                                    unsafe { _exit(code) };
                                }
                                drop(theirs);
                                if pid > 0 {
                                    e.checking = Some((pid, c));
                                }
                            }
                        }
                        Some(_) => {
                            let w = pauses.wait(uid).min(u16::MAX as u64) as u16;
                            let _ = proto::send_from_vdm(&mut c, &FromVdm::Done { ok: false, wait: w });
                        }
                        None => {
                            let _ = proto::send_from_vdm(&mut c, &FromVdm::Done { ok: false, wait: 0 });
                        }
                    }
                }
                (Some(ToVdm::Decline(id)), Some(uid), Some(pid))
                    if sys::on_console(pid) && vergence_dm::elevate::is_server(pid) =>
                {
                    if let Some(i) = elevations.iter().position(|e| e.id == id && e.uid == uid && e.checking.is_none())
                    {
                        let e = elevations.remove(i);
                        log(&format!("elevation: {} by {} through {}: declined", e.action.name(), e.name, e.who));
                        let n = declined.get(&uid).map_or(0, |d| d.1);
                        declined.insert(uid, (std::time::Instant::now(), n + 1));
                        finish(e.program, Err("declined at the prompt".into()));
                    }
                }
                _ => {}
            }
        }
        // The login screen asking to log someone in (step 4).
        let greeter_fd = greeter.as_ref().map_or(-1, |l| l.as_raw_fd());
        if let Some(Ok((mut g, _))) = greeter.as_ref().map(|l| l.accept()) {
            let _ = g.set_nonblocking(false);
            let _ = g.set_write_timeout(Some(IO_LIMIT));
            // Only the running login screen is heard (finding 2), its request
            // whole within a second (finding 3).
            let until = std::time::Instant::now() + Duration::from_secs(1);
            let request = if sys::peer_uid(&g) == greeter_uid && greeter_run.pid > 0 {
                let mut r = sys::Deadline { stream: &g, until };
                match (proto::read_to_vdm(&mut r), proto::read_to_vdm(&mut r)) {
                    (
                        Ok(ToVdm::Hello { version: proto::VERSION, service: proto::Service::Login }),
                        Ok(ToVdm::Login { user, session }),
                    ) => Some((user, session)),
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
                        let exec = vergence_dm::sessions::list()
                            .get(session as usize)
                            .and_then(|e| std::fs::read_to_string(&e.file).ok())
                            .and_then(|t| vergence_dm::sessions::entry_exec(&t));
                        let vt = (0..MAX_SESSIONS)
                            .map(|k| vergence_dm::vt::GREETER_VT + 1 + k)
                            .find(|vt| !logins.values().any(|l| l.vt == *vt));
                        // One session per person for now: a second would share the
                        // first's runtime directory and its apps' sockets (step 6).
                        let elsewhere =
                            sessions.iter().any(|(u, _)| *u == uid) || logins.values().any(|l| l.uid == uid && l.yes);
                        if elsewhere {
                            log(&format!("login for {user}: refused (already logged in)"));
                            let _ = proto::send_from_vdm(
                                &mut g,
                                &FromVdm::Error(
                                    "You are already logged in on another console. Log out there first.".into(),
                                ),
                            );
                            refuse(g, 0);
                        } else if running.values().any(|(u, ..)| *u == uid)
                            || logins.values().any(|l| l.uid == uid && !l.yes)
                        {
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
                                        reset_sigterm();
                                        drop(listener);
                                        drop(session_listener);
                                        drop(control);
                                        drop(power);
                                        drop(elevate);
                                        drop(sessions);
                                        drop(running);
                                        drop(logins);
                                        drop(elevations);
                                        drop(said_parent);
                                        // The greeter listener, borrowed here, by its number (finding 10).
                                        sys::close_fds(&[greeter_fd], &[]);
                                        // SAFETY: the check's limit (login() cancels it on a yes).
                                        unsafe { alarm(TIME_LIMIT) };
                                        let code = login(g, &plan, &login_service, said_child);
                                        // SAFETY: leave without running the parent's exit handlers.
                                        unsafe { _exit(code) };
                                    }
                                    if pid > 0 {
                                        let _ = said_parent.set_nonblocking(true);
                                        logins.insert(
                                            pid,
                                            LoginRun {
                                                uid,
                                                name: user.clone(),
                                                client: Some(g),
                                                said: said_parent,
                                                vt,
                                                yes: false,
                                            },
                                        );
                                    }
                                }
                                Err(e) => log(&format!("login for {user}: {e}")),
                            }
                        } else {
                            log(&format!(
                                "login for {user}: refused (no such session, or {MAX_SESSIONS} sessions running)"
                            ));
                            refuse(g, 0);
                        }
                    }
                    // Root, system accounts, no real shell, no such account:
                    // the same "no" as a wrong password, logged apart.
                    _ => {
                        log(&format!(
                            "login for {user}: refused (root, a system account, no login shell, or no such account)"
                        ));
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
        // One check at a time per account, a login's included (finding 10).
        if running.values().any(|(u, ..)| *u == uid) || logins.values().any(|l| l.uid == uid && !l.yes) {
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
            reset_sigterm();
            drop(control);
            drop(power);
            drop(elevate);
            drop(sessions);
            drop(running);
            drop(greeter);
            drop(logins);
            drop(elevations);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The login screen's picture is told by its first bytes only.
    #[test]
    fn picture_kinds_by_their_first_bytes() {
        assert_eq!(picture_kind(b"\x89PNG\r\n\x1a\n...."), Some("login-wallpaper.png"));
        assert_eq!(picture_kind(&[0xff, 0xd8, 0xff, 0xe0, 0, 0]), Some("login-wallpaper.jpg"));
        assert_eq!(picture_kind(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Some("login-wallpaper.webp"));
        assert_eq!(picture_kind(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), Some("login-wallpaper.svg"));
        assert_eq!(picture_kind(b"\xef\xbb\xbf<?xml version='1.0'?>\n<svg/>"), Some("login-wallpaper.svg"));
        assert_eq!(picture_kind(b"#!/bin/sh\nrm -rf /"), None);
        assert_eq!(picture_kind(b"<html><body>no</body></html>"), None);
        assert_eq!(picture_kind(b""), None);
    }

    /// A process orphaned while the worker waits is reaped on the way, not
    /// left a zombie until the session ends (two of Firefox's processes
    /// were seen under a session's worker). This binary has no other
    /// tests, so the test process may become a reaper itself.
    #[test]
    fn pauses_are_gentle_then_firm_and_forgotten() {
        assert_eq!((1..=8).map(pause_after).collect::<Vec<_>>(), vec![0, 0, 0, 5, 15, 30, 60, 60]);
        let mut p = Pauses { by_uid: HashMap::new() };
        for _ in 0..4 {
            p.failed_at(1000, 10_000);
        }
        assert_eq!(p.by_uid[&1000], (4, 10_005));
        // A quarter of an hour after the last pause ended: the count starts again.
        p.failed_at(1000, 10_005 + FORGET_AFTER);
        assert_eq!(p.by_uid[&1000].0, 1);
        assert_eq!(p.by_uid[&1000].1, 10_005 + FORGET_AFTER);
    }

    #[test]
    // Both children are reaped by waitpid, not by Child::wait.
    #[allow(clippy::zombie_processes)]
    fn orphans_are_reaped_while_waiting() {
        if !sys::become_reaper() {
            eprintln!("no reaper role on this system; skipped");
            return;
        }
        // The shell leaves a background sleep behind and exits: the sleep is
        // handed to us and ends while we still wait for `main`.
        std::process::Command::new("/bin/sh").args(["-c", "sleep 0.2 & exit 0"]).spawn().expect("sh");
        let main = std::process::Command::new("/bin/sh").args(["-c", "sleep 1; exit 3"]).spawn().expect("sh");
        let status = wait_reaping(main.id()).expect("wait");
        assert_eq!(status.code(), Some(3));
        // Nothing left to reap: no zombie below us, and nothing running.
        let mut st: c_int = 0;
        // SAFETY: an out-parameter we own.
        let left = unsafe { waitpid(-1, &mut st, WNOHANG) };
        assert_eq!(left, -1, "a child was still there to reap");
    }
}
