// The development control socket (server milestone 1, 2026-11-07): lets the
// developer (and Claude over SSH) see and drive a running session on the real
// monitors without a hand at the console.
//
// $XDG_RUNTIME_DIR/vergence-control, a Unix socket in the user's private runtime
// directory (0700), mode 0600, and every connection's peer uid is checked against
// ours (getpeereid): owner-only, so it is not an unauthenticated control interface
// (whitepaper §7.3). No network socket. One command per connection, one line in,
// text out:
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
    io::{BufRead, BufReader, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::PathBuf,
};

use smithay::reexports::calloop::{Interest, Mode, PostAction, generic::Generic};

use crate::Server;

/// Exit status that asks the launcher to start the server again.
pub const RESTART: i32 = 75;

pub fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("vergence-control"))
}

pub fn listen(state: &mut Server) {
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
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
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
            tracing::info!("control socket {}", path.display());
            state.control_owned = true;
        }
        Err(e) => tracing::warn!("control: {e}"),
    }
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
    fn getpeereid(fd: i32, euid: *mut u32, egid: *mut u32) -> i32;
}

pub fn our_uid() -> u32 {
    unsafe { libc_getuid() }
}

/// The connecting process's user id (getpeereid: FreeBSD, and Linux through libc).
pub fn peer_uid(stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (u32::MAX, u32::MAX);
    (unsafe { getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0).then_some(uid)
}

impl Server {
    fn control(&mut self, line: &str) -> String {
        let mut words = line.split_whitespace();
        let cmd = words.next().unwrap_or("");
        let args: Vec<&str> = words.collect();
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
                format!("{cmd}ed {:#x}", button)
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
            _ => "commands: shot, windows, dbus, notes, spawn <cmd>, move <i> <x> <y>, focus <i>, pointer <x> <y>, press|release [left|right|middle], restart, quit".into(),
        }
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
                _ => ("?", String::new(), String::new()),
            };
            out.push(format!("{i} {kind} {title:?} {class:?} {geo:?} on {on:?} title-bar={}", self.has_title(w)));
        }
        out.join("\n")
    }
}

pub fn shot_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp")).join("vergence-shots")
}

/// Write RGBA pixels as a PNG, off the main thread.
pub fn save_png(name: String, w: u32, h: u32, rgba: Vec<u8>) {
    std::thread::spawn(move || {
        let dir = shot_dir();
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{name}.png"));
        let res = (|| -> Result<(), Box<dyn std::error::Error>> {
            let file = std::io::BufWriter::new(std::fs::File::create(&path)?);
            let mut enc = png::Encoder::new(file, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            enc.write_header()?.write_image_data(&rgba)?;
            Ok(())
        })();
        match res {
            Ok(()) => tracing::info!("shot: {}", path.display()),
            Err(e) => tracing::warn!("shot {}: {e}", path.display()),
        }
    });
}
