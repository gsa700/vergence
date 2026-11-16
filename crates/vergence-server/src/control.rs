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
//   restart              exit with status 75 (the launcher starts the new build)
//   quit                 exit
//
// tools: `vctl <command>` on Hubble (printf | nc -U -N).

use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::{fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt}, net::UnixListener},
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
            tracing::info!("control socket {} (VERGENCE_CONTROL=1: any program running as this user can drive the session)", path.display());
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
/// where glibc has no getpeereid (found on Fedora, 2026-11-08).
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

/// The connecting process's id, for `vctl windows` (2026-11-11: the stress
/// test's click tool finds its own terminal by it). Linux: SO_PEERCRED's
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
            // Native windows' meaning (Contract 3, native.rs).
            "tree" => self.native_tree_report(),
            "shelf" => self.shelf_report(),
            "tiles" => self.tiles_report(),
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
            "move" => {
                let (Some(i), Some(x), Some(y)) = (
                    args.first().and_then(|a| a.parse::<usize>().ok()),
                    args.get(1).and_then(|a| a.parse::<i32>().ok()),
                    args.get(2).and_then(|a| a.parse::<i32>().ok()),
                ) else {
                    return "move <i> <x> <y>".into();
                };
                let Some(w) = self.space.elements().nth(i).cloned() else { return format!("no window {i}") };
                self.space.map_element(w.clone(), (x, y), true);
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
            _ => "commands: shot, windows, gpus, shelf, tiles, actions, action <id or name>, session, apps, key <name> (chords: ctrl+v), type <text>, touch, dbus, notes, spawn <cmd>, move <i> <x> <y>, focus <i>, pointer <x> <y>, press|release [left|right|middle], restart, quit".into(),
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
                None => w.toplevel().and_then(|t| smithay::reexports::wayland_server::Resource::client(t.wl_surface())).and_then(|c| c.get_credentials(&self.display_handle).ok()).map(|c| c.pid as u32),
            };
            let pid = pid.map(|p| format!(" pid {p}")).unwrap_or_default();
            out.push(format!("{i} {kind} {title:?} {class:?} {geo:?} on {on:?} title-bar={}{pid} {}", self.has_title(w), self.describe_style(w)));
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
