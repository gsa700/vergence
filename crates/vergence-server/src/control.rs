// The development control socket (server milestone 1, 2026-11-07): lets the
// developer (and Claude over SSH) see and drive a running session on the real
// monitors without a hand at the console.
//
// $XDG_RUNTIME_DIR/vergence-control, a Unix socket in the user's private runtime
// directory (0700), mode 0600, and every connection's peer uid is checked against
// ours (getpeereid / SO_PEERCRED): owner-only, so it is not an unauthenticated control interface
// (whitepaper §7.3). No network socket.
//
// Off unless VERGENCE_CONTROL=1 (security pass 2026-11-08): owner-only still
// means any program running as the user, and these commands bypass every
// mitigation in whitepaper §7.3 (input injection, focus, screen capture, running
// commands). tools/vergence-session turns it on when it runs from a checkout (a
// development session); an installed session (tools/install.sh) has it off unless
// VERGENCE_CONTROL=1 is in the environment or the user created
// ~/.config/vergence/control-socket. One command per connection, one line in (at most LINE_MAX
// bytes), text out; a client gets IO_TIMEOUT to send its line and take the reply:
//
//   shot                 PNG of every monitor as shown -> ~/vergence-shots/
//   windows              the windows: index, kind, title, class, geometry, monitor
//   dbus                 the D-Bus edge: connections, names, every call seen
//   notes                the notifications on screen
//   spawn <command>      run a command (sh -c) inside the session
//   move <i> <x> <y>     put window i's content at logical x, y
//   focus <i>            raise window i and give it the keyboard
//   pointer <x> <y>      move the pointer to logical x, y (as the mouse would)
//   press|release [left|right|middle]   a mouse button, as the mouse would
//   act <i> <node> press|focus|reveal       an action on window i's widget by its
//   act <i> <node> set|choose <value...>     meaning node (`tree` shows the ids;
//                                            Contract 3, bus v24): its serial
//   acted <serial>       that action's answer (ok, why not, or waiting)
//   find <path>          widgets by path: "Calculator > 7", "* > button:Save"
//   focused              the focused widget and the caret, on the screens
//   changes [since]      what changed in the meaning since a number
//   agents               agents' grants, the question showing, requests waiting
//   agent-answer deny|now|session|always|hour   answer it (tests)
//   tile-action <tile> <control> [value]   a control on one of the desktop's own cards, as a press would
//   restart              exit with status 75 (the launcher starts the new build)
//   quit                 exit
//
// tools: `vctl <command>` on Hubble (printf | nc -U -N).

use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        net::UnixListener,
    },
    path::PathBuf,
};

use smithay::reexports::calloop::{Interest, Mode, PostAction, generic::Generic};

use crate::Server;

/// Exit status that asks the launcher to start the server again.
pub const RESTART: i32 = 75;

/// Longest command line accepted.
const LINE_MAX: u64 = 64 * 1024;
/// How long a client gets to send its line, and to take the reply. The socket is
/// served on the server's own thread, so a client that stalls holds the display
/// this long at most (owner-only; a stuck vctl, not an attack).
const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether the commands that stand in for the person before agents (act,
/// agent-answer, the agent tile's controls) are on: only where the server
/// was started with VERGENCE_CONTROL=1 and the person has not turned the
/// socket on for a daily session with the control-socket file, so nothing in a
/// daily session can answer for the person (see docs/meaning-milestone-1.md).
/// Headless tests have no such file.
fn agent_commands() -> bool {
    let file = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|c| c.join("vergence/control-socket"));
    enabled() && !file.is_some_and(|f| f.exists())
}

/// Is the control socket wanted? VERGENCE_CONTROL=1 only.
pub fn enabled() -> bool {
    std::env::var("VERGENCE_CONTROL").is_ok_and(|v| v == "1")
}

pub fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("vergence-control"))
}

pub fn listen(state: &mut Server) {
    if !enabled() {
        return tracing::info!("control: off (VERGENCE_CONTROL=1 turns the development control socket on)");
    }
    let Some(path) = socket_path() else { return tracing::warn!("control: no XDG_RUNTIME_DIR") };
    // A nested server must not take over the session server's socket.
    if crate::bus::socket_is_live(&path) {
        return tracing::warn!("control: {} belongs to a running server; no control socket here", path.display());
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => return tracing::warn!("control: {}: {e}", path.display()),
    };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    let _ = listener.set_nonblocking(true);
    let res = state.loop_handle.insert_source(Generic::new(listener, Interest::READ, Mode::Level), |_, l, state| {
        while let Ok((stream, _)) = l.accept() {
            // Owner only.
            let ours = our_uid();
            match peer_uid(&stream) {
                Some(uid) if uid == ours => {}
                other => {
                    tracing::warn!("control: refused a connection from uid {other:?}");
                    continue;
                }
            }
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
            let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
            let mut line = String::new();
            if BufReader::new((&stream).take(LINE_MAX)).read_line(&mut line).is_err() {
                continue;
            }
            let reply = state.control(line.trim());
            let mut s = &stream;
            let _ = s.write_all(reply.as_bytes());
            let _ = s.write_all(b"\n");
        }
        Ok(PostAction::Continue)
    });
    match res {
        Ok(_) => {
            tracing::info!(
                "control socket {} (VERGENCE_CONTROL=1: any program running as this user can drive the session)",
                path.display()
            );
            state.control_owned = true;
        }
        Err(e) => tracing::warn!("control: {e}"),
    }
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
    #[cfg(not(target_os = "linux"))]
    fn getpeereid(fd: i32, euid: *mut u32, egid: *mut u32) -> i32;
    fn getsockopt(fd: i32, level: i32, name: i32, val: *mut u8, len: *mut u32) -> i32;
}

pub fn our_uid() -> u32 {
    unsafe { libc_getuid() }
}

/// The connecting process's user id: getpeereid on FreeBSD; SO_PEERCRED on Linux,
/// where glibc has no getpeereid.
pub fn peer_uid(stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    #[cfg(not(target_os = "linux"))]
    {
        let (mut uid, mut gid) = (u32::MAX, u32::MAX);
        (unsafe { getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0).then_some(uid)
    }
    #[cfg(target_os = "linux")]
    {
        // struct ucred { pid, uid, gid }; SOL_SOCKET 1, SO_PEERCRED 17.
        let mut buf = [0u8; 12];
        let mut len = buf.len() as u32;
        let ok = unsafe { getsockopt(stream.as_raw_fd(), 1, 17, buf.as_mut_ptr(), &mut len) } == 0;
        (ok && len == 12).then(|| u32::from_ne_bytes(buf[4..8].try_into().unwrap()))
    }
}

/// The connecting process's id, for `vctl windows` (the stress test's click
/// tool finds its own terminal by it). Linux: SO_PEERCRED's
/// struct ucred { pid, uid, gid }. FreeBSD: LOCAL_PEERCRED's struct xucred
/// { u_int version; uid_t uid; short ngroups; gid_t groups[16]; union {
/// void *; pid_t pid; } }, the pid at byte 80 (after 76 bytes, aligned to 8).
pub fn peer_pid(stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    #[cfg(target_os = "linux")]
    let (level, name, size, at) = (1, 17, 12usize, 0usize);
    #[cfg(not(target_os = "linux"))]
    let (level, name, size, at) = (0, 1, 88usize, 80usize); // SOL_LOCAL, LOCAL_PEERCRED
    let mut buf = [0u8; 88];
    let mut len = size as u32;
    // SAFETY: `buf` has room for `len` bytes, and getsockopt writes at most that.
    let ok = unsafe { getsockopt(stream.as_raw_fd(), level, name, buf.as_mut_ptr(), &mut len) } == 0;
    (ok && len as usize >= at + 4).then(|| u32::from_ne_bytes(buf[at..at + 4].try_into().unwrap())).filter(|p| *p != 0)
}

impl Server {
    fn control(&mut self, line: &str) -> String {
        let mut words = line.split_whitespace();
        let cmd = words.next().unwrap_or("");
        let args: Vec<&str> = words.collect();
        // Input made by vctl counts as input: it wakes screens that went off
        // while idle (a test on an idle machine clicked into nothing).
        // Locked, the control socket takes nothing but a picture of the
        // screens (which show only the lock) and the lock's report (threat model T4).
        if self.lock.on && !matches!(cmd, "shot" | "lock") {
            return "locked".into();
        }
        // The elevation prompt is answered by the person at the machine only
        // (elevation review, finding 8): no typing or pressing into it.
        if self.elevating() && matches!(cmd, "pointer" | "press" | "release" | "key" | "type" | "touch") {
            return "an elevation prompt is up".into();
        }
        if matches!(cmd, "pointer" | "press" | "release" | "key" | "type" | "touch") {
            self.energy_activity();
        }
        match cmd {
            "shot" => {
                if self.backend.is_none() {
                    return "shot: only with --drm".into();
                }
                self.shot_pending = self.space.outputs().map(|o| o.name()).collect();
                let dir = shot_dir();
                format!("shot: next frame of each monitor -> {}/<connector>.png", dir.display())
            }
            "windows" => self.describe_windows(),
            "gpus" => self.describe_gpus(),
            // The order of use, most recent first, and the focused window (Alt+Tab's order).
            "used" => self.order_of_use_report(),
            // Alt+Tab's list: what it holds, which row is stepped to, whether it shows.
            "switching" => self.switching_report(),
            // A modifier held down or let go across commands (Alt held while Tab is
            // pressed several times shows Alt+Tab's list): `hold alt`, `let-go alt`.
            "hold" | "let-go" => {
                let code: u32 = match args.first().map(|a| a.to_ascii_lowercase()).as_deref() {
                    Some("alt") => 56,
                    Some("ctrl") | Some("control") => 29,
                    Some("shift") => 42,
                    Some("super") => 125,
                    _ => return "hold|let-go alt|ctrl|shift|super".into(),
                };
                use smithay::backend::input::{InputTime, KeyState};
                let state = if cmd == "hold" { KeyState::Pressed } else { KeyState::Released };
                self.handle_key((code + 8).into(), state, InputTime::now());
                format!("{cmd} {}", args[0])
            }
            // Snapping's choices, what a drag is aimed at, the snapped windows (snap.rs).
            "snap" => self.snap_report(),
            // The overview and the dock's picker as they are (overview milestone 1, tests).
            "overview" => self.overview_report(),
            "picker" => self.picker_report(),
            // Native windows' meaning (Contract 3, native.rs).
            "tree" => self.native_tree_report(),
            // An action on a widget by its meaning node (meaning milestone 1, step 1).
            "act" | "agent-answer" if !agent_commands() => format!("{cmd}: only in a test session (VERGENCE_CONTROL=1 and no ~/.config/vergence/control-socket)"),
            "tile-action" if args.first() == Some(&"agent") && !agent_commands() => "tile-action agent: only in a test session".into(),
            "act" => {
                use vergence_protocol::window::Action;
                let usage = "act <i> <node> press|focus|reveal, or act <i> <node> set|choose <value...>";
                let (Some(i), Some(node), Some(verb)) = (args.first().and_then(|a| a.parse::<usize>().ok()), args.get(1).and_then(|a| a.parse::<u64>().ok()), args.get(2)) else { return usage.into() };
                let value = args.get(3..).map(|v| v.join(" ")).unwrap_or_default();
                let action = match *verb {
                    "press" => Action::Press,
                    "focus" => Action::Focus,
                    "reveal" => Action::Reveal,
                    "set" => Action::Set(value),
                    "choose" if !value.is_empty() => Action::Choose(value),
                    _ => return usage.into(),
                };
                let Some(w) = self.space.elements().nth(i).cloned() else { return format!("no window {i}") };
                match self.send_act(&w, node, action, None) {
                    Some(serial) => format!("act {serial}: sent; `acted {serial}` for the answer"),
                    None => format!("window {i}: not a native window (only they have meaning)"),
                }
            }
            // The server's model of meaning (meaning milestone 1, step 2; meaning.rs).
            "find" => {
                let path = args.join(" ");
                if path.is_empty() {
                    return "find <path>: \"Calculator > 7\", \"Windows > switch:Minimize\", \"* > button:Save\"".into();
                }
                self.meaning_find_report(&path)
            }
            "focused" => self.meaning_focus_report(),
            "agents" => self.agents_report(),
            // Windows drawn from their scenes (scenes.rs).
            "scenes" => self.scenes_report(),
            // The magnifier (zoom.rs): its state, its keys, or a point of
            // interest as the focus or caret would set it.
            "zoom" => {
                use crate::zoom::ZoomKey;
                match args.as_slice() {
                    [] => {}
                    ["in"] => self.zoom_key(ZoomKey::In),
                    ["out"] => self.zoom_key(ZoomKey::Out),
                    ["toggle"] => self.zoom_key(ZoomKey::Toggle),
                    ["lens"] => self.zoom_key(ZoomKey::Lens),
                    [by @ ("focus" | "caret"), x, y] => match (x.parse::<f64>(), y.parse::<f64>()) {
                        (Ok(x), Ok(y)) => self.zoom_follow(if *by == "focus" { "focus" } else { "caret" }, (x, y)),
                        _ => return "zoom focus|caret <x> <y>".into(),
                    },
                    _ => return "zoom [in|out|toggle|lens|focus <x> <y>|caret <x> <y>]".into(),
                }
                self.zoom_report()
            }
            "agent-answer" => self.agent_answer_by_word(args.first().copied().unwrap_or("")),
            "changes" => self.meaning_changes_report(args.first().and_then(|a| a.parse().ok()).unwrap_or(0)),
            "acted" => {
                let Some(serial) = args.first().and_then(|a| a.parse::<u32>().ok()) else { return "acted <serial>".into() };
                match self.act_answer(serial) {
                    None => format!("acted {serial}: no such action"),
                    Some(None) => format!("acted {serial}: waiting"),
                    Some(Some((true, _))) => format!("acted {serial}: ok"),
                    Some(Some((false, why))) => format!("acted {serial}: refused: {why}"),
                }
            }
            "shelf" => self.shelf_report(),
            "tiles" => self.tiles_report(),
            "tile-action" => {
                let (Some(tile), Some(control)) = (args.first(), args.get(1)) else { return "tile-action <tile> <control> [value]".into() };
                let value = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(0.0);
                self.desktop_tile_action(tile, control, value);
                format!("tile-action {tile} {control}: done")
            }
            // The lock: `lock` alone reports; `lock now` asks to lock, as Super+L does.
            "lock" => {
                if args.first() == Some(&"now") {
                    self.lock_request("vctl");
                }
                if self.lock.on { "locked".into() } else { "unlocked".into() }
            }
            // Named actions (actions.rs): `actions` lists them, `action <id or name>` runs one.
            "actions" => self.actions_report(),
            "action" => match self.action_run(&args.join(" ")) {
                Ok(name) => format!("ran {name}"),
                Err(e) => e,
            },
            "session" => self.session_report(),
            // The drive list as the bus topic `Drives` has it (drives.rs).
            "drives" => {
                if self.drives.list.is_empty() {
                    "no drives".into()
                } else {
                    self.drives.list.iter().map(|d| format!("{}\t{:?}\t{}\t{}\t{}\t{}", d.path, d.kind, if d.mounted { "mounted" } else { "not mounted" }, if d.ejectable { "ejectable" } else { "fixed" }, d.device, d.name)).collect::<Vec<_>>().join("\n")
                }
            }
            "apps" => self.launcher_report(),
            // A finger: `touch down|move <x> <y> [finger]`, `touch up [finger]`; alone, a report.
            "touch" => {
                let number = |i: usize| args.get(i).and_then(|a| a.parse::<f64>().ok());
                let time = smithay::backend::input::InputTime::now();
                let finger = |i: usize| smithay::backend::input::TouchSlot::from(args.get(i).and_then(|a| a.parse::<u32>().ok()).or(Some(0)));
                match (args.first().copied(), number(1), number(2)) {
                    (Some("down"), Some(x), Some(y)) => self.touch_down(finger(3), (x, y).into(), time),
                    (Some("move"), Some(x), Some(y)) => self.touch_motion(finger(3), (x, y).into(), time),
                    (Some("up"), ..) => self.touch_up(finger(1), time),
                    (None, ..) => return self.touch_report(),
                    _ => return "touch down|move <x> <y> [finger], touch up [finger]".into(),
                }
                self.touch_frame();
                "touched".into()
            }
            // Keys by name and text by letter, as a US keyboard has them (the kernel's
            // key numbers; what they type is the session's layout's to say).
            "key" | "type" => {
                // Chords: "ctrl+v", "ctrl+shift+c", "alt+Tab": the modifiers
                // held (Control_L, Shift_L, Alt_L, Super_L) while the keys
                // after them are pressed in turn ("alt+Tab+Tab+Tab": Alt held
                // through three Tabs).
                let mut held: Vec<u32> = Vec::new();
                let first = args.first().copied().unwrap_or("");
                let mut names: Vec<&str> = vec![first];
                if cmd == "key" && first.contains('+') && first.len() > 1 {
                    let parts: Vec<&str> = first.split('+').collect();
                    let mods = parts.iter().take_while(|m| matches!(m.to_ascii_lowercase().as_str(), "ctrl" | "control" | "shift" | "alt" | "super")).count().min(parts.len() - 1);
                    for m in &parts[..mods] {
                        held.push(match m.to_ascii_lowercase().as_str() {
                            "ctrl" | "control" => 29,
                            "shift" => 42,
                            "alt" => 56,
                            _ => 125,
                        });
                    }
                    names = parts[mods..].to_vec();
                }
                const ROWS: [(&str, u32); 4] = [("1234567890-=", 2), ("qwertyuiop", 16), ("asdfghjkl", 30), ("zxcvbnm,.", 44)];
                let letter = |c: char| if c == ' ' { Some(57) } else { ROWS.iter().find_map(|(row, first)| row.find(c).map(|i| first + i as u32)) };
                let codes: Vec<u32> = if cmd == "key" {
                    let mut codes = Vec::new();
                    for name in names {
                    codes.extend(match name {
                        "Escape" => vec![1],
                        "Return" => vec![28],
                        "BackSpace" => vec![14],
                        "Tab" => vec![15],
                        "space" => vec![57],
                        "Up" => vec![103],
                        "Down" => vec![108],
                        "Left" => vec![105],
                        "Right" => vec![106],
                        "Page_Up" | "Prior" => vec![104],
                        "Page_Down" | "Next" => vec![109],
                        "Super" => vec![125],
                        // The key above Tab (KEY_GRAVE), for Alt+`.
                        "grave" => vec![41],
                        // The function keys (KEY_F1..F10 are 59..68, F11 and F12 87 and 88).
                        f if f.len() >= 2 && f.starts_with('F') && f[1..].parse::<u32>().is_ok_and(|n| (1..=12).contains(&n)) => {
                            let n: u32 = f[1..].parse().unwrap();
                            vec![if n <= 10 { 58 + n } else { 76 + n }]
                        }
                        // A modifier alone (Ctrl tapped during a drag snaps it, snap.rs).
                        "Control" => vec![29],
                        "Shift" => vec![42],
                        // The volume keys (KEY_MUTE, KEY_VOLUMEDOWN, KEY_VOLUMEUP).
                        "Mute" => vec![113],
                        "VolumeDown" => vec![114],
                        "VolumeUp" => vec![115],
                        other => match other.chars().collect::<Vec<_>>().as_slice() {
                            // One letter or digit, for chords ("ctrl+v").
                            [c] if letter(*c).is_some() => vec![letter(*c).unwrap()],
                            _ => return format!("key: no key {other} (Escape, Return, BackSpace, Tab, grave, space, Up, Down, Left, Right, Super, Mute, VolumeDown, VolumeUp, a letter; chords like ctrl+v, alt+Tab+Tab)"),
                        },
                    });
                    }
                    codes
                } else {
                    let text = args.join(" ");
                    let mut codes = Vec::new();
                    for c in text.chars() {
                        match letter(c) {
                            Some(code) => codes.push(code),
                            None => return format!("type: no key for {c:?} (small letters, digits, space, - = , .)"),
                        }
                    }
                    codes
                };
                use smithay::backend::input::{InputTime, KeyState};
                for m in &held {
                    self.handle_key((m + 8).into(), KeyState::Pressed, InputTime::now());
                }
                for code in &codes {
                    for state in [KeyState::Pressed, KeyState::Released] {
                        self.handle_key((code + 8).into(), state, InputTime::now());
                    }
                }
                for m in held.iter().rev() {
                    self.handle_key((m + 8).into(), KeyState::Released, InputTime::now());
                }
                format!("{} keys", codes.len() + held.len())
            }
            "dbus" => self.dbus_report(),
            "notes" => self.notify_report(),
            "spawn" => {
                let command = line["spawn".len()..].trim();
                if command.is_empty() {
                    return "spawn: no command".into();
                }
                match crate::spawn(std::process::Command::new("sh").args(["-c", command])) {
                    Ok(pid) => format!("spawned pid {pid}"),
                    Err(e) => format!("spawn: {e}"),
                }
            }
            // A window's own picture, as its program drew it (headless
            // milestone 1, step 4): saved as window-<i>.png beside the shots,
            // at its own pixels, to compare with what the screen shows.
            // A window minimized, as its bar's button does (tests: the dock's
            // hollow dots, the pickers' minimized tiles).
            "minimize" => {
                let Some(i) = args.first().and_then(|a| a.parse::<usize>().ok()) else { return "minimize <i>".into() };
                let Some(w) = self.space.elements().nth(i).cloned() else { return format!("no window {i}") };
                self.minimize(&w);
                format!("minimized {i}")
            }
            // A window's picture as the dock's picker and Alt+Tab show it
            // (thumbs.rs), fitted into a square `side` pixels wide.
            "thumb" => {
                if self.backend.is_none() {
                    return "thumb: only with --drm or --headless".into();
                }
                let (Some(i), Some(side)) = (args.first().and_then(|a| a.parse::<usize>().ok()), args.get(1).and_then(|a| a.parse::<i32>().ok()).filter(|s| (8..=2048).contains(s))) else {
                    return "thumb <i> <side 8-2048>".into();
                };
                let Some(w) = self.space.elements().nth(i).cloned() else { return format!("no window {i}") };
                let name = format!("thumb-{i}");
                self.thumb_pending.push((w, side, name.clone()));
                self.redraw_all();
                format!("thumb {i}: {}", shot_dir().join(format!("{name}.png")).display())
            }
            // Screenshots (capture.rs): `capture screen [NAME] | screens | area X Y
            // W H | window I` with `--pointer`, `--no-shadow`, `--to FILE`;
            // the files it will be written to (on the next frame, off the main thread).
            "capture" => {
                use crate::capture::{Dest, Request, What};
                let flag = |f: &str| args.contains(&f);
                let to = args.iter().position(|a| *a == "--to").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from);
                let words: Vec<&str> = args.iter().copied().take_while(|a| !a.starts_with("--")).collect();
                let num = |i: usize| words.get(i).and_then(|v| v.parse::<i32>().ok());
                let what = match words.first().copied() {
                    Some("screen") => What::Screen(words.get(1).map(|s| s.to_string())),
                    Some("screens") => What::Screens,
                    Some("area") => match (num(1), num(2), num(3), num(4)) {
                        (Some(x), Some(y), Some(w), Some(h)) => What::Area(smithay::utils::Rectangle::new((x, y).into(), (w, h).into())),
                        _ => return "capture area X Y W H".into(),
                    },
                    Some("window") => match num(1).and_then(|i| self.space.elements().nth(i as usize).cloned()) {
                        Some(w) => What::Window(w),
                        None => return "capture window I (an index from vctl windows)".into(),
                    },
                    _ => return "capture screen [NAME] | screens | area X Y W H | window I  [--pointer] [--no-shadow] [--to FILE | --clip]".into(),
                };
                let dest = if flag("--clip") { Dest::Clipboard } else { to.map_or(Dest::Folder, Dest::Path) };
                let req = Request { what, dest, pointer: flag("--pointer"), shadow: !flag("--no-shadow") };
                match self.capture_ask(req) {
                    Ok(paths) => paths.iter().map(|p| format!("capture: {}", p.display())).collect::<Vec<_>>().join("\n"),
                    Err(e) => format!("capture: {e}"),
                }
            }
            // What the clipboard holds and from whom (a picture: its size).
            "clipboard" => {
                use smithay::wayland::selection::data_device::current_data_device_selection_userdata;
                match current_data_device_selection_userdata(&self.seat).as_deref() {
                    None => "clipboard: a Wayland program's, or nothing".into(),
                    Some(crate::native::ClipOwner::X11) => "clipboard: an X11 program's".into(),
                    Some(crate::native::ClipOwner::Native(c)) => format!("clipboard: native client {c}'s"),
                    Some(crate::native::ClipOwner::Server(t)) => format!("clipboard: the server's text, {} bytes", t.len()),
                    Some(crate::native::ClipOwner::Picture(png)) => {
                        let size = (png.len() >= 24 && png.starts_with(b"\x89PNG")).then(|| (u32::from_be_bytes(png[16..20].try_into().unwrap()), u32::from_be_bytes(png[20..24].try_into().unwrap())));
                        format!("clipboard: the server's picture, image/png, {} bytes, {:?}", png.len(), size)
                    }
                }
            }
            "picture" => {
                if self.backend.is_none() {
                    return "picture: only with --drm or --headless".into();
                }
                let Some(i) = args.first().and_then(|a| a.parse::<usize>().ok()) else { return "picture <i>".into() };
                let Some(w) = self.space.elements().nth(i).cloned() else { return format!("no window {i}") };
                if w.native().and_then(|n| n.picture()).is_none() {
                    return format!("window {i}: no picture of its own (only native windows have one)");
                }
                let name = format!("window-{i}");
                self.picture_pending.push((w, name.clone()));
                self.redraw_all();
                format!("picture {i}: {}", shot_dir().join(format!("{name}.png")).display())
            }
            // Screens in memory beside the monitors (headless milestone 1):
            // `screen add --fake 2560x1440 597x336 [--hz 60] [--name TEXT]`,
            // `screen add --edid FILE 3840x2160 [--hz 60]`, `screen remove MEM-1`,
            // `screen` (each screen; the ones in memory marked).
            "screen" => {
                if self.backend.is_none() {
                    return "screen: only with --drm".into();
                }
                let usage = "screen [add --fake WxH MMxMM [--hz R] [--name TEXT] | add --edid FILE WxH [--hz R] | remove NAME]";
                let flag = |k: &str| args.iter().position(|a| *a == k).and_then(|i| args.get(i + 1)).copied();
                let pair = |v: &str| -> Option<(u32, u32)> {
                    let (a, b) = v.split_once('x')?;
                    Some((a.parse().ok()?, b.parse().ok()?))
                };
                match args.first().copied() {
                    None => {
                        let mem = crate::drm::memory_screens(self);
                        let mut lines: Vec<String> = self
                            .space
                            .outputs()
                            .map(|o| {
                                let g = self.space.output_geometry(o).unwrap_or_default();
                                let m = if mem.contains(&o.name()) { " (in memory)" } else { "" };
                                format!("{} {:?} {}x{} pt at {},{} scale {:.4}{m}", o.name(), o.physical_properties().model, g.size.w, g.size.h, g.loc.x, g.loc.y, o.current_scale().fractional_scale())
                            })
                            .collect();
                        lines.sort();
                        lines.join("\n")
                    }
                    Some("remove") => match args.get(1) {
                        Some(name) => match crate::drm::remove_memory_screen(self, name) {
                            Ok(()) => format!("removed {name}"),
                            Err(e) => format!("screen: {e}"),
                        },
                        None => usage.into(),
                    },
                    Some("add") => {
                        let hz = flag("--hz").and_then(|v| v.parse::<f64>().ok()).unwrap_or(60.0);
                        let refresh = (hz * 1000.0).round() as u32;
                        let (edid, px) = if let Some(file) = flag("--edid") {
                            let Some(px) = args.iter().skip_while(|a| **a != "--edid").nth(2).and_then(|v| pair(v)) else { return usage.into() };
                            match std::fs::read(file) {
                                Ok(b) => (b, px),
                                Err(e) => return format!("screen: {file}: {e}"),
                            }
                        } else if let Some(px) = flag("--fake").and_then(pair) {
                            let Some(mm) = args.iter().skip_while(|a| **a != "--fake").nth(2).and_then(|v| pair(v)) else { return usage.into() };
                            let name = flag("--name").map_or_else(|| format!("FAKE{}x{}", px.0, px.1), str::to_string);
                            (crate::drm::fake_screen_edid(px, mm, &name), px)
                        } else {
                            return usage.into();
                        };
                        match crate::drm::add_memory_screen(self, edid, px, refresh) {
                            Ok(name) => format!("added {name}"),
                            Err(e) => format!("screen: {e}"),
                        }
                    }
                    _ => usage.into(),
                }
            }
            // A screen's scale for a test, as if chosen but not saved
            // (tools/display-check.sh): `scale DP-1 1.3333`, `scale DP-1 reset`,
            // `scale reset` (all), `scale` (each screen's).
            "scale" => {
                if self.backend.is_none() {
                    return "scale: only with --drm".into();
                }
                match (args.first().copied(), args.get(1).copied()) {
                    (Some("reset"), None) => self.scale_test.clear(),
                    (Some(name), Some("reset")) => {
                        self.scale_test.remove(name);
                    }
                    (Some(name), Some(v)) => {
                        let Some(s) = v.parse::<f64>().ok().filter(|s| s.is_finite() && (0.5..=8.0).contains(s)) else {
                            return "scale: a scale from 0.5 to 8".into();
                        };
                        if !self.space.outputs().any(|o| o.name() == name) {
                            return format!("scale: no screen {name}");
                        }
                        self.scale_test.insert(name.to_string(), s);
                    }
                    (None, _) => {}
                    _ => return "scale [<screen> <scale>|<screen> reset|reset]".into(),
                }
                if !args.is_empty() {
                    crate::drm::displays_changed(self);
                }
                let mut lines: Vec<String> = self
                    .space
                    .outputs()
                    .map(|o| {
                        let test = if self.scale_test.contains_key(&o.name()) { " (test)" } else { "" };
                        format!("{} {:.4}{test}", o.name(), o.current_scale().fractional_scale())
                    })
                    .collect();
                lines.sort();
                lines.join("\n")
            }
            "move" => {
                let (Some(i), Some(x), Some(y)) = (
                    args.first().and_then(|a| a.parse::<usize>().ok()),
                    args.get(1).and_then(|a| a.parse::<i32>().ok()),
                    args.get(2).and_then(|a| a.parse::<i32>().ok()),
                ) else {
                    return "move <i> <x> <y>".into();
                };
                let Some(w) = self.space.elements().nth(i).cloned() else { return format!("no window {i}") };
                self.put(w.clone(), (x, y), true);
                if let Some(xs) = w.x11_surface() {
                    let _ = xs.configure(smithay::utils::Rectangle::new((x, y).into(), xs.geometry().size));
                }
                // Remembered as a move by hand is when it ends (tests of window memory).
                self.remember(&w);
                format!("moved {i} to {x},{y}")
            }
            "focus" => {
                let Some(i) = args.first().and_then(|a| a.parse::<usize>().ok()) else { return "focus <i>".into() };
                let Some(w) = self.space.elements().nth(i).cloned() else { return format!("no window {i}") };
                self.focus_window(&w, smithay::utils::SERIAL_COUNTER.next_serial());
                format!("focused {i}")
            }
            "pointer" => {
                let (Some(x), Some(y)) = (args.first().and_then(|a| a.parse::<f64>().ok()), args.get(1).and_then(|a| a.parse::<f64>().ok())) else {
                    return "pointer <x> <y>".into();
                };
                let t = smithay::backend::input::InputTime::now();
                self.touch_mouse_back();
                self.pointer_to((x, y).into(), t);
                format!("pointer at {x},{y}")
            }
            "press" | "release" => {
                let button = match args.first().copied().unwrap_or("left") {
                    "left" => 0x110,
                    "right" => 0x111,
                    "middle" => 0x112,
                    other => return format!("{cmd}: no button {other}"),
                };
                let state = if cmd == "press" {
                    smithay::backend::input::ButtonState::Pressed
                } else {
                    smithay::backend::input::ButtonState::Released
                };
                let t = smithay::backend::input::InputTime::now();
                self.handle_button(button, state, t);
                format!("{} {:#x}", if cmd == "press" { "pressed" } else { "released" }, button)
            }
            "restart" => {
                self.exit_code = RESTART;
                self.loop_signal.stop();
                "restarting".into()
            }
            "quit" => {
                self.loop_signal.stop();
                "quitting".into()
            }
            _ => "commands: shot, windows, gpus, shelf, tiles, actions, action <id or name>, session, drives, apps, key <name> (chords: ctrl+v), type <text>, touch, dbus, notes, spawn <cmd>, move <i> <x> <y>, scale [<screen> <scale>|reset], screen [add|remove], focus <i>, pointer <x> <y>, press|release [left|right|middle], restart, quit".into(),
        }
    }

    fn order_of_use_report(&self) -> String {
        let focused = self.keyboard_window();
        let mut out = vec![format!("focused: {:?}", focused.as_ref().map(|w| self.window_title(w)))];
        for (i, id) in self.input.seat.order_of_use().iter().enumerate() {
            if let Some(w) = self.input.window(*id) {
                let min = if self.is_minimized(&w) { " (minimized)" } else { "" };
                out.push(format!("{i}: {}{min}", self.window_title(&w)));
            }
        }
        out.join("\n")
    }

    fn describe_windows(&self) -> String {
        let mut out = Vec::new();
        for o in self.space.outputs() {
            let g = self.space.output_geometry(o);
            out.push(format!("output {} scale {:.4} at {:?}", o.name(), o.current_scale().fractional_scale(), g));
        }
        for (i, w) in self.space.elements().enumerate() {
            let geo = self.space.element_geometry(w);
            let on: Vec<String> = self.space.outputs_for_element(w).iter().map(|o| o.name()).collect();
            let (kind, title, class) = match (w.x11_surface(), w.toplevel()) {
                (Some(x), _) => (if x.is_override_redirect() { "x11-or" } else { "x11" }, x.title(), x.class()),
                (None, Some(t)) => {
                    let (title, app) = smithay::wayland::compositor::with_states(t.wl_surface(), |s| {
                        s.data_map
                            .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                            .map(|d| {
                                let d = d.lock().unwrap();
                                (d.title.clone().unwrap_or_default(), d.app_id.clone().unwrap_or_default())
                            })
                            .unwrap_or_default()
                    });
                    ("wayland", title, app)
                }
                _ => match w.native() {
                    Some(n) => ("native", n.title(), n.app_id.clone()),
                    None => ("?", String::new(), String::new()),
                },
            };
            // The window's process (Wayland: the client's credentials; native: its bus connection's).
            let pid = match w.native() {
                Some(n) => self.bus_pid(n.client),
                None => w
                    .toplevel()
                    .and_then(|t| smithay::reexports::wayland_server::Resource::client(t.wl_surface()))
                    .and_then(|c| c.get_credentials(&self.display_handle).ok())
                    .map(|c| c.pid as u32),
            };
            let pid = pid.map(|p| format!(" pid {p}")).unwrap_or_default();
            // A native window's scale and text factor, as told (the display suite).
            let told = w.native().map_or(String::new(), |n| format!(" scale={:.4} text={:.3}", n.scale(), n.text()));
            let scene = w.native().map(|n| format!(" cursor={:?}", n.cursor())).unwrap_or_default()
                + &w.native().and_then(|n| n.scene_report()).map_or(String::new(), |s| format!(" [{s}]"));
            out.push(format!(
                "{i} {kind} {title:?} {class:?} {geo:?} on {on:?} title-bar={} bar-h={}{told}{pid} {}{scene}",
                self.has_title(w),
                if self.has_title(w) { self.bar_h(w) } else { 0 },
                self.describe_style(w)
            ));
        }
        out.join("\n")
    }
}

pub fn shot_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp")).join("vergence-shots")
}

/// Write RGBA pixels as a PNG, off the main thread. A screenshot is the
/// user's screen: the directory is 0700 and the file 0600 (written beside it,
/// then moved into place, so an old world-readable file is replaced too).
pub fn save_png(name: String, w: u32, h: u32, rgba: Vec<u8>) {
    std::thread::spawn(move || {
        let dir = shot_dir();
        let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir);
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        let path = dir.join(format!("{name}.png"));
        let tmp = dir.join(format!("{name}.png.tmp"));
        let res = (|| -> Result<(), Box<dyn std::error::Error>> {
            let file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
            let file = std::io::BufWriter::new(file);
            let mut enc = png::Encoder::new(file, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            enc.write_header()?.write_image_data(&rgba)?;
            std::fs::rename(&tmp, &path)?;
            Ok(())
        })();
        match res {
            Ok(()) => tracing::info!("shot: {}", path.display()),
            Err(e) => tracing::warn!("shot {}: {e}", path.display()),
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_socket_s_peer_is_this_process() {
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        assert_eq!(super::peer_pid(&a), Some(std::process::id()));
        assert_eq!(super::peer_uid(&a), Some(super::our_uid()));
    }
}
