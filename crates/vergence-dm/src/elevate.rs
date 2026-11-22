//! The named actions an elevation prompt may approve (docs/elevation-
//! milestone-1.md): each a fixed thing vdm does with its own code, its
//! arguments checked here, and said in words for the prompt. Nothing a
//! program sends reaches a shell or names a path.

/// One action vdm may carry out after the prompt's yes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Automatic login for this person (VDM milestone 1's rules still apply).
    AutologinOn(String),
    AutologinOff,
    /// The login screen at boot (STATE_DIR/login-screen).
    LoginScreenOn,
    LoginScreenOff,
}

impl Action {
    /// The words a program sent (ToVdm::Act), as an action, or why not.
    pub fn parse(words: &[String]) -> Result<Action, String> {
        let w: Vec<&str> = words.iter().map(String::as_str).collect();
        match w.as_slice() {
            ["autologin.on", user] if valid_name(user) => Ok(Action::AutologinOn(user.to_string())),
            ["autologin.off"] => Ok(Action::AutologinOff),
            ["login-screen.on"] => Ok(Action::LoginScreenOn),
            ["login-screen.off"] => Ok(Action::LoginScreenOff),
            [name, ..] => Err(format!("no such action: {name}")),
            [] => Err("no action".into()),
        }
    }

    /// What the prompt says is asked.
    pub fn describe(&self) -> String {
        match self {
            Action::AutologinOn(u) => {
                format!("Turn on automatic login for {u}: this machine starts into {u}'s session without a password")
            }
            Action::AutologinOff => "Turn off automatic login".into(),
            Action::LoginScreenOn => "Start this machine at the Vergence login screen".into(),
            Action::LoginScreenOff => {
                "Stop starting this machine at the Vergence login screen (log in on a text console instead)".into()
            }
        }
    }

    /// Its name in the log.
    pub fn name(&self) -> &'static str {
        match self {
            Action::AutologinOn(_) => "autologin.on",
            Action::AutologinOff => "autologin.off",
            Action::LoginScreenOn => "login-screen.on",
            Action::LoginScreenOff => "login-screen.off",
        }
    }
}

/// An account's name as accounts are named: letters, digits, '.', '_', '-',
/// not first; at most 64.
fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && !n.starts_with('-')
        && n.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// The program file a process runs (Linux `/proc/<pid>/exe`; FreeBSD the
/// kern.proc.pathname sysctl: CTL_KERN 1, KERN_PROC 14, KERN_PROC_PATHNAME 12).
pub fn program_path(pid: u32) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/exe")).ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        unsafe extern "C" {
            fn sysctl(
                name: *const i32,
                namelen: u32,
                old: *mut u8,
                oldlen: *mut usize,
                new: *const u8,
                newlen: usize,
            ) -> i32;
        }
        let mib = [1i32, 14, 12, pid as i32];
        let mut buf = vec![0u8; 1024];
        let mut len = buf.len();
        // SAFETY: a mib of 4 and a buffer of len bytes, ours.
        if unsafe { sysctl(mib.as_ptr(), 4, buf.as_mut_ptr(), &mut len, std::ptr::null(), 0) } != 0 {
            return None;
        }
        buf.truncate(len);
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        Some(std::path::PathBuf::from(String::from_utf8_lossy(&buf[..end]).into_owned()))
    }
}

/// Whether a program file is one root installed: the file and its
/// directory root's, and neither writable by anyone else.
fn installed_by_root(path: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let ok = |p: &std::path::Path| std::fs::metadata(p).is_ok_and(|m| m.uid() == 0 && m.mode() & 0o022 == 0);
    ok(path) && path.parent().is_some_and(ok)
}

/// Who asks, as the prompt shows it, read from the kernel before anything
/// the program sends is read (elevation review, finding 2: a program's own
/// name is its to choose). One of the desktop's own programs, installed by
/// root, by its name; anything else by its file's name, said to be another
/// program. The words are what the person decides on; this is a hint.
pub fn program_shown(pid: u32) -> String {
    match program_path(pid) {
        Some(p) if installed_by_root(&p) && p.file_name().is_some_and(|n| n == "vergence-settings") => {
            "System Settings".into()
        }
        Some(p) => format!("A program ({})", crate::proto::clean(&p.display().to_string())),
        None => format!("A program (process {pid})"),
    }
}

/// Whether a process runs the installed vergence-server (root's file).
pub fn is_server(pid: u32) -> bool {
    program_path(pid).is_some_and(|p| installed_by_root(&p) && p.file_name().is_some_and(|n| n == "vergence-server"))
}

/// A process's parent (Linux `/proc/<pid>/stat`; FreeBSD ps).
pub fn parent_of(pid: u32) -> Option<u32> {
    if cfg!(target_os = "linux") {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        stat[stat.rfind(')')? + 1..].split_whitespace().nth(1)?.parse().ok()
    } else {
        let o = std::process::Command::new("/bin/ps")
            .args(["-o", "ppid=", "-p", &pid.to_string()])
            .env_clear()
            .output()
            .ok()?;
        String::from_utf8_lossy(&o.stdout).trim().parse().ok()
    }
}

/// Whether `pid` is one of `ancestors` or runs below one of them (at most 64
/// steps up): a program started from the person's own session (finding 3).
pub fn descends_from(pid: u32, ancestors: &[u32]) -> bool {
    let mut p = pid;
    for _ in 0..64 {
        if ancestors.contains(&p) {
            return true;
        }
        match parent_of(p) {
            Some(q) if q > 1 && q != p => p = q,
            _ => return false,
        }
    }
    false
}

/// 128 random bits for a prompt's id (the kernel's random device).
pub fn random_id() -> u128 {
    use std::io::Read;
    let mut b = [0u8; 16];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_err() {
        // No random device: the time's nanoseconds and the pid, not guessable at a glance.
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        return t ^ ((std::process::id() as u128) << 64);
    }
    u128::from_ne_bytes(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn actions_and_their_checks() {
        assert_eq!(Action::parse(&w(&["autologin.on", "derickson"])), Ok(Action::AutologinOn("derickson".into())));
        assert_eq!(Action::parse(&w(&["login-screen.off"])), Ok(Action::LoginScreenOff));
        assert!(Action::parse(&w(&["autologin.on"])).is_err(), "whose");
        assert!(Action::parse(&w(&["autologin.on", "-x"])).is_err());
        assert!(Action::parse(&w(&["autologin.off", "extra"])).is_err());
        assert!(Action::parse(&w(&["rm", "-rf"])).is_err());
        assert!(Action::parse(&[]).is_err());
    }
}

/// A program's side (Settings): ask vdm for an action and wait for the
/// person's answer at the prompt (up to about two minutes: run it in a
/// thread). What vdm said it did, or why not.
pub fn request(words: &[&str]) -> Result<String, String> {
    use crate::proto::{self, FromVdm, Service, ToVdm};
    let mut s = std::os::unix::net::UnixStream::connect(crate::paths::ELEVATE_SOCKET)
        .map_err(|_| "vdm is not running here".to_string())?;
    // The prompt waits 120 s, and a check after it may take as long again.
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(260)));
    proto::send_to_vdm(&mut s, &ToVdm::Hello { version: proto::VERSION, service: Service::Elevate })
        .map_err(|e| e.to_string())?;
    proto::send_to_vdm(&mut s, &ToVdm::Act(words.iter().map(|w| w.to_string()).collect()))
        .map_err(|e| e.to_string())?;
    let mut said = String::new();
    loop {
        match proto::read_from_vdm(&mut s) {
            Ok(FromVdm::Info(t) | FromVdm::Error(t)) => said = t,
            Ok(FromVdm::Done { ok: true, .. }) => return Ok(said),
            Ok(FromVdm::Done { ok: false, .. }) => {
                return Err(if said.is_empty() { "not allowed".into() } else { said });
            }
            Ok(_) => return Err("vdm answered out of turn".into()),
            Err(e) => return Err(format!("vdm: {e}")),
        }
    }
}
