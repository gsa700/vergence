// One terminal program for all its windows (bar milestone 1, step 6;
// docs/bar-milestone-1.md, "Windows that stay where they were").
//
// - **One process.** In a Vergence session the first `vergence-term` listens
//   on a socket of its own (owner-only, in the runtime directory); starting
//   it again asks that one for a new window and exits. Joining a tab to
//   another window, tearing one off, moving a section into its own window:
//   all inside the one process, on either road (the Wayland road too since
//   bar milestone 1, step 4b). Not for a terminal started to be measured
//   (--tabs, --panes, VTERM_FRAMES), which wants a process of its own.
//   Windows are kept and restored on the native road only: by their
//   tokens, which another desktop does not give.
// - **What it keeps.** Each window's persistence token (the server's, so
//   window memory puts the window back where it was) and its tabs' working
//   directories, in ~/.local/state/vergence/terminal-windows (0600), written
//   whenever they change, so that a logout, which ends the program from
//   outside, finds it current. A window the user closes leaves it. At the
//   next start the terminal opens them again, each with its token and tabs.
// - **No titles, no scrollback, no commands** are kept: only where the
//   windows were (the server's) and where their shells stood.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::{fs::PermissionsExt, net::{UnixListener, UnixStream}},
    path::PathBuf,
    sync::{Arc, Mutex},
};

/// What one window keeps: its token, and its tabs' working directories in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowState {
    pub token: String,
    pub tabs: Vec<String>,
}

/// The most windows and tabs a state file is read with (a bad file opens no
/// more than this), and the longest directory kept.
const MAX_WINDOWS: usize = 64;
const MAX_TABS: usize = 64;
const MAX_PATH: usize = 4096;

pub fn state_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("vergence/terminal-windows"))
}

/// The windows a state file keeps: "window <token>" lines, each followed by
/// "tab <directory>" lines; anything else is passed over (a broken file
/// opens no more than it can make sense of).
pub fn parse(text: &str) -> Vec<WindowState> {
    let mut windows: Vec<WindowState> = Vec::new();
    for line in text.lines() {
        if let Some(token) = line.strip_prefix("window ") {
            if windows.len() >= MAX_WINDOWS {
                break;
            }
            let token = token.trim();
            if !token.is_empty() && token.len() <= 64 {
                windows.push(WindowState { token: token.to_string(), tabs: Vec::new() });
            }
        } else if let Some(dir) = line.strip_prefix("tab ")
            && let Some(w) = windows.last_mut()
            && w.tabs.len() < MAX_TABS
            && dir.len() <= MAX_PATH
        {
            w.tabs.push(dir.to_string());
        }
    }
    windows
}

pub fn render(windows: &[WindowState]) -> String {
    let mut out = String::from("# vergence-term's windows: each one's persistence token (the server remembers\n# where it was by it) and its tabs' working directories. No titles or text.\n");
    for w in windows {
        out += &format!("window {}\n", w.token);
        for dir in w.tabs.iter().filter(|d| !d.contains('\n')) {
            out += &format!("tab {dir}\n");
        }
    }
    out
}

pub fn load() -> Vec<WindowState> {
    state_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse(&t)).unwrap_or_default()
}

/// Written whole, owner-only, by rename, so a logout midway leaves the old one.
pub fn save(windows: &[WindowState]) {
    let Some(p) = state_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("tmp");
    let res = std::fs::write(&tmp, render(windows))
        .and_then(|_| std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)))
        .and_then(|_| std::fs::rename(&tmp, &p));
    if let Err(e) = res {
        eprintln!("vergence-term: {}: {e}", p.display());
    }
}

/// The program's windows, kept in step with the state file. Each window has
/// a key of its own here (the order it opened in this run).
#[derive(Default)]
pub struct Program {
    pub windows: Vec<(u64, WindowState)>,
    next_key: u64,
    written: Vec<WindowState>,
    /// Windows to open again at start, after the first.
    pub restore: Vec<WindowState>,
}

impl Program {
    pub fn add(&mut self) -> u64 {
        self.next_key += 1;
        self.windows.push((self.next_key, WindowState::default()));
        self.next_key
    }

    /// A window's token and tabs now; written if anything changed. A window
    /// with no token yet is not written (it has no place to come back to).
    pub fn update(&mut self, key: u64, state: WindowState) {
        if let Some(w) = self.windows.iter_mut().find(|(k, _)| *k == key) {
            w.1 = state;
        }
        self.write();
    }

    /// The user closed the window: it is not opened again.
    pub fn remove(&mut self, key: u64) {
        self.windows.retain(|(k, _)| *k != key);
        self.write();
    }

    fn write(&mut self) {
        let now: Vec<WindowState> = self.windows.iter().map(|(_, w)| w.clone()).filter(|w| !w.token.is_empty()).collect();
        if now != self.written {
            // Tests keep to themselves: no file of the user's is written.
            #[cfg(not(test))]
            save(&now);
            self.written = now;
        }
    }
}

/// The program's socket: where a second start finds the first.
fn socket_path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
    Some(dir.join("vergence-term.sock"))
}

/// A request from a second start: open a window here, running this (empty: the shell).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenRequest {
    pub cwd: String,
    pub run: Vec<String>,
}

fn encode(r: &OpenRequest) -> String {
    let mut fields = vec!["open".to_string(), r.cwd.clone()];
    fields.extend(r.run.iter().cloned());
    fields.iter().map(|f| f.replace(['\t', '\n'], " ")).collect::<Vec<_>>().join("\t") + "\n"
}

fn decode(line: &str) -> Option<OpenRequest> {
    let mut f = line.trim_end_matches('\n').split('\t');
    if f.next()? != "open" {
        return None;
    }
    let cwd = f.next().unwrap_or("").chars().take(MAX_PATH).collect();
    let run: Vec<String> = f.take(256).map(str::to_string).collect();
    Some(OpenRequest { cwd, run })
}

/// Ask a running terminal to open a window: true if one said yes.
pub fn hand_over(r: &OpenRequest) -> bool {
    let Some(path) = socket_path() else { return false };
    let Ok(mut s) = UnixStream::connect(&path) else { return false };
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
    if s.write_all(encode(r).as_bytes()).is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

/// Become the program others hand over to: requests go to `inbox`, and
/// `wake` is called for each. False if the socket could not be made.
pub fn listen(inbox: Arc<Mutex<Vec<OpenRequest>>>, wake: impl Fn() + Send + 'static) -> bool {
    let Some(path) = socket_path() else { return false };
    // A socket nobody answers on is left from a terminal that went away.
    if UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let Ok(listener) = UnixListener::bind(&path) else { return false };
    // Only its owner may ask it anything (the runtime directory is the owner's too).
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    std::thread::Builder::new()
        .name("handover".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                let mut reader = BufReader::new(&stream);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                if reader.read_line(&mut line).is_ok()
                    && line.len() <= 64 * 1024
                    && let Some(r) = decode(&line)
                {
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(r);
                    let _ = (&stream).write_all(b"ok\n");
                    wake();
                }
            }
        })
        .is_ok()
}

/// The working directory of process `pid` (a tab's shell), if it can be read.
pub fn cwd_of(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok().map(|p| p.to_string_lossy().into_owned())
    }
    #[cfg(not(target_os = "linux"))]
    {
        // FreeBSD: procstat's file list, whose "cwd" line names it.
        let out = std::process::Command::new("procstat").args(["-h", "-f", &pid.to_string()]).output().ok()?;
        String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.get(2) == Some(&"cwd")).then(|| f.last().map(|p| p.to_string())).flatten()
        })
    }
}

/// The command name of process `pid` ("ssh", "make"), if it can be read:
/// Linux's /proc/<pid>/comm; elsewhere (FreeBSD) `ps -o comm=`.
pub fn name_of(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    #[cfg(not(target_os = "linux"))]
    let name = {
        let out = std::process::Command::new("ps").args(["-o", "comm=", "-p", &pid.to_string()]).stderr(std::process::Stdio::null()).output().ok()?;
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let name = name.trim();
    (!name.is_empty()).then(|| name.chars().take(40).collect())
}

/// The effective user of process `pid`, if it can be read: Linux's
/// /proc/<pid>/status (its Uid line: real, effective, saved, filesystem);
/// elsewhere (FreeBSD) `ps -o uid=`, which prints the effective user.
pub fn euid_of(pid: u32) -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        status.lines().find_map(|l| l.strip_prefix("Uid:")).and_then(|ids| ids.split_whitespace().nth(1)?.parse().ok())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out = std::process::Command::new("ps").args(["-o", "uid=", "-p", &pid.to_string()]).stderr(std::process::Stdio::null()).output().ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
}

/// Whether any process of process group `group` runs as the superuser (a
/// script's group holds the sudo it runs, though the script leads it):
/// Linux's /proc/<pid>/stat for each process's group; elsewhere (FreeBSD)
/// one `ps` listing of every process's group and effective user.
pub fn group_has_root(group: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        let Ok(dir) = std::fs::read_dir("/proc") else { return false };
        dir.flatten().filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok()).any(|pid| {
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { return false };
            // After the name in parentheses: state, parent, group, ...
            let pgrp = stat.rfind(')').and_then(|i| stat[i + 1..].split_whitespace().nth(2)?.parse::<u32>().ok());
            pgrp == Some(group) && euid_of(pid) == Some(0)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let Ok(out) = std::process::Command::new("ps").args(["-ax", "-o", "pgid=", "-o", "uid="]).stderr(std::process::Stdio::null()).output() else { return false };
        String::from_utf8_lossy(&out.stdout).lines().any(|l| {
            let mut f = l.split_whitespace();
            f.next().and_then(|g| g.parse::<u32>().ok()) == Some(group) && f.next() == Some("0")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The red tab's question, answered on each platform: this test's own
    /// process is the user running it; a process that does not exist, none.
    #[test]
    fn the_effective_user_of_a_process_is_read() {
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        assert_eq!(euid_of(std::process::id()), Some(unsafe { geteuid() }));
        assert_eq!(euid_of(999_999_999), None);
        assert!(name_of(std::process::id()).is_some_and(|n| !n.is_empty()));
        assert_eq!(name_of(999_999_999), None);
    }

    /// This test's own process group has a superuser in it only if the
    /// tests run as root.
    #[test]
    fn a_process_group_is_searched_for_the_superuser() {
        unsafe extern "C" {
            fn geteuid() -> u32;
            fn getpgrp() -> i32;
        }
        let (me, group) = unsafe { (geteuid(), getpgrp() as u32) };
        assert_eq!(group_has_root(group), me == 0);
        assert!(!group_has_root(999_999_999));
    }

    #[test]
    fn the_state_file_round_trips_and_a_bad_one_opens_nothing() {
        let w = vec![WindowState { token: "a".repeat(32), tabs: vec!["/home/d".into(), "/tmp".into()] }, WindowState { token: "b".repeat(32), tabs: vec![] }];
        assert_eq!(parse(&render(&w)), w);
        assert!(parse("not this format\n[[[").is_empty());
        assert!(parse("tab /x\nwindow \n").is_empty(), "a tab with no window, a window with no token: skipped");
        let many: String = (0..100).map(|i| format!("window t{i}\n")).collect();
        assert_eq!(parse(&many).len(), MAX_WINDOWS);
        assert_eq!(parse("window aa\ntab /with spaces/ok\n")[0].tabs, ["/with spaces/ok"]);
    }

    #[test]
    fn program_writes_only_windows_with_tokens_and_only_on_change() {
        let mut p = Program::default();
        let a = p.add();
        let b = p.add();
        p.update(a, WindowState { token: String::new(), tabs: vec!["/".into()] });
        assert!(p.written.is_empty(), "no token, nothing to write");
        p.update(b, WindowState { token: "t".repeat(32), tabs: vec!["/tmp".into()] });
        assert_eq!(p.written.len(), 1);
        p.remove(b);
        assert!(p.written.is_empty());
    }

    #[test]
    fn a_handover_request_survives_the_line() {
        let r = OpenRequest { cwd: "/home/d/src".into(), run: vec!["htop".into(), "-d".into(), "5".into()] };
        assert_eq!(decode(&encode(&r)), Some(r));
        assert_eq!(decode("close\t/x\n"), None);
        let tabbed = OpenRequest { cwd: "/a\tb".into(), run: vec![] };
        assert_eq!(decode(&encode(&tabbed)).unwrap().cwd, "/a b");
    }
}
