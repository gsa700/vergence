// Based on Smithay's smallvil example (MIT; see LICENSE-smithay.txt), adapted for Vergence.
#![allow(irrefutable_let_patterns)]

mod handlers;

mod dispatch;
mod portal;
mod drag;
mod desktop;
mod exit;
mod keys;
mod input;
mod state;
mod winit;
mod drm;
mod cursor;
mod focus;
mod decor;
mod grid;
#[cfg(test)]
mod geometry_tests;
mod xsettings;
mod control;
mod text;
mod place;
mod memory;
mod wallpaper;
mod menu;
mod appmenu;
mod forcequit;
mod backdrop;
mod bus;
mod dbus;
mod settings;
mod notify;
mod displays;
mod energy;
mod shelf;
mod tiles;
mod lock;
mod session_dialog;
mod greeter;
mod elevate;
mod actions;
mod drawer;
mod applets;
mod session;
mod launcher;
mod touch;
mod pointer;
mod win;
mod native;
mod meaning;
mod sound;
mod switch;
mod thumbs;
mod picker;
mod overview;
mod aside;
mod opened;
mod snap;
mod style;
mod fullscreen;
mod dialogs;
mod tint;
mod inputs;
mod network;
mod battery;
mod gpus;
mod monitors;

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
pub use state::Server;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("vergence-server {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    init_logging();
    // Every socket of the session goes in the runtime directory: refuse one
    // that another user could swap files in (security pass 2026-11-15,
    // finding 6).
    if let Err(why) = check_runtime_dir() {
        tracing::error!("{why}");
        return Err(why.into());
    }
    tighten_own_folders();

    let mut event_loop: EventLoop<Server> = EventLoop::try_new()?;

    let display: Display<Server> = Display::new()?;

    // The login screen (VDM milestone 1, review C3): `--greeter`, as vdm starts
    // it for _vdm. It starts no program and opens nothing to programs.
    if std::env::args().any(|a| a == "--greeter") {
        GREETER.store(true, std::sync::atomic::Ordering::Relaxed);
        tracing::info!("greeter: the login screen; no program is started, no session bus, no X11");
    }
    let mut state = Server::new(&mut event_loop, display);
    state.start_wallpaper();
    if !greeter() {
        state.start_network();
        state.start_applets();
        state.start_lock();
        state.start_battery_history();
    }
    state.start_settings();
    state.start_energy();
    if greeter() {
        state.start_greeter();
    }

    // --drm: the real monitors (from a text console, through libseat);
    // --headless: screens in memory only (`--screen "WxH MMxMM[@HZ]"`, repeated;
    // headless milestone 1); else nested.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--drm") {
        crate::drm::run(&mut event_loop, &mut state)?;
    } else if args.iter().any(|a| a == "--headless") {
        let screens: Vec<String> = args.windows(2).filter(|w| w[0] == "--screen").map(|w| w[1].clone()).collect();
        crate::drm::run_headless(&mut event_loop, &mut state, &screens)?;
    } else {
        crate::winit::init_winit(&mut event_loop, &mut state)?;
    }

    // Set WAYLAND_DISPLAY to our socket name, so child processes connect to Server rather
    // than the host compositor
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name) };

    // Our file dialog for apps not ours, if asked for (portal.rs): the
    // environment that makes GTK and Qt ask the portal.
    crate::portal::export_env();
    // The development control socket (owner-only; control.rs).
    crate::control::listen(&mut state);
    if !greeter() {
        // The session bus (bus.rs); sets VERGENCE_BUS for everything started after it.
        crate::bus::listen(&mut state);
        // The D-Bus edge (dbus.rs); sets DBUS_SESSION_BUS_ADDRESS unless one is set.
        crate::dbus::listen(&mut state);
    }

    // X11 apps through Xwayland, if the settings ask for it (off by default since
    // 2026-11-10: Dave runs no X11-only programs); the first app then starts once
    // it is ready (or failed), so it inherits DISPLAY. Never at the login screen.
    if greeter() {
        tracing::info!("Xwayland: off (the login screen)");
    } else if crate::state::x11_wanted() {
        state.xwayland_started = true;
        state.start_xwayland(spawn_client);
    } else {
        tracing::info!("Xwayland: off ([x11] enabled = true in desktop.toml turns it on)");
        spawn_client();
    }

    event_loop.run(None, &mut state, move |_| {
        // Server is running
    })?;
    // Restart or Shut Down from the log out dialog: the apps have closed;
    // vdm does it (while this session is still registered with it).
    if let Some(p) = state.session_dialog.after_exit.take() {
        match vergence_dm::power::ask(p) {
            Ok(()) => tracing::info!("session: vdm will {}", p.word()),
            Err(e) => tracing::warn!("session: {}: {e}", p.word()),
        }
    }

    if let Some(p) = crate::control::socket_path().filter(|_| state.control_owned) {
        let _ = std::fs::remove_file(p);
    }
    if let Some(p) = state.bus.path.take() {
        let _ = std::fs::remove_file(p);
    }
    if let Some(p) = state.dbus.path.take() {
        let _ = std::fs::remove_file(p);
    }
    if state.exit_code != 0 {
        std::process::exit(state.exit_code);
    }
    Ok(())
}

/// A scale rounded to 120ths, the unit wp-fractional-scale-v1 gives apps: outputs use
/// exactly what apps draw at, so nothing is resampled (server milestone 1, stage 2).
pub fn scale_in_120ths(exact: f64) -> f64 {
    ((exact * 120.0).round() / 120.0).max(1.0 / 120.0)
}

fn init_logging() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }
}

/// The server is the login screen (`--greeter`; VDM milestone 1, review C3).
static GREETER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn greeter() -> bool {
    GREETER.load(std::sync::atomic::Ordering::Relaxed)
}

/// The program `cmd` starts runs in a session of its own, with no
/// controlling terminal (review C6): it cannot open /dev/tty to reach the
/// console the session was started on.
pub fn no_terminal(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    unsafe extern "C" {
        fn setsid() -> i32;
    }
    // SAFETY: setsid is async-signal-safe and touches nothing of the parent's;
    // it cannot fail in a freshly forked child that is not a group leader.
    unsafe {
        cmd.pre_exec(|| {
            setsid();
            Ok(())
        });
    }
}

/// Start an app in the session: stdin from /dev/null (the server's is the console:
/// a .NET app blocked reading it and froze, 2026-11-07), in a session of its own
/// with no controlling terminal (VDM milestone 1, review C6: found on kepler, every
/// app had the console as its controlling terminal and could open /dev/tty, which
/// a lock screen must not allow), reaped when it exits.
pub fn spawn(cmd: &mut std::process::Command) -> std::io::Result<u32> {
    // The login screen starts nothing, whatever asks (review C3).
    if greeter() {
        tracing::warn!("greeter: refused to start {:?}", cmd.get_program());
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "the login screen starts no program"));
    }
    no_terminal(cmd);
    let mut child = cmd.stdin(std::process::Stdio::null()).spawn()?;
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}

/// The first app, once Xwayland is ready: `-c <command>` only, and never on the
/// real monitors (--drm): the session starts with an empty desktop; apps come
/// from the desktop menu (2026-11-08: a foot window opened at every restart, a
/// leftover from the first nested tests). Nested runs still take `-c`.
fn spawn_client() {
    let all: Vec<String> = std::env::args().skip(1).collect();
    if all.iter().any(|a| a == "--drm" || a == "--headless") {
        return;
    }
    let mut args = all.into_iter();
    if let (Some(flag), Some(command)) = (args.next(), args.next())
        && (flag == "-c" || flag == "--command")
    {
        let _ = spawn(&mut std::process::Command::new(command));
    }
}

/// XDG_RUNTIME_DIR must be set and be a real directory (not a link), ours,
/// and closed to everyone else (mode 700 or tighter).
fn check_runtime_dir() -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let dir = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set; start the session with `vergence`")?;
    let m = std::fs::symlink_metadata(&dir).map_err(|e| format!("runtime directory {}: {e}", dir.to_string_lossy()))?;
    match runtime_dir_fault(m.file_type().is_symlink(), m.is_dir(), m.uid(), m.permissions().mode(), crate::control::our_uid()) {
        Some(fault) => Err(format!("runtime directory {} {fault}; refusing to put the session's sockets there", dir.to_string_lossy())),
        None => Ok(()),
    }
}

/// The session's settings and state are the user's alone (CLAUDE.md's
/// owner-only rule; security pass 2026-11-15, finding 12): desktop.toml
/// holds commands the server runs. At every start the two folders are made
/// 0700 and the files directly in them owner-only; links and what is not
/// ours are left alone. Other files the user's programs save are not
/// touched.
fn tighten_own_folders() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else { return };
    let config = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let state = std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".local/state"));
    let us = crate::control::our_uid();
    for dir in [config.join("vergence"), state.join("vergence")] {
        let Ok(m) = std::fs::symlink_metadata(&dir) else { continue };
        if !m.is_dir() || m.uid() != us {
            tracing::warn!("{} is not a folder of ours; left alone", dir.display());
            continue;
        }
        if m.mode() & 0o077 != 0 {
            tracing::info!("{}: mode {:o} made 700", dir.display(), m.mode() & 0o777);
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let Ok(m) = e.path().symlink_metadata() else { continue };
            if m.uid() == us && (m.is_file() || m.is_dir()) && m.mode() & 0o077 != 0 {
                let _ = std::fs::set_permissions(e.path(), std::fs::Permissions::from_mode(m.mode() & 0o700));
            }
        }
    }
    // desktop.toml's commands run with `sh -c`: say so if anyone else could change them.
    let desktop = config.join("vergence/desktop.toml");
    if let Ok(m) = std::fs::metadata(&desktop)
        && (m.uid() != us || m.mode() & 0o022 != 0)
    {
        tracing::warn!("{} can be changed by another user (owner {}, mode {:o}); its commands run in this session", desktop.display(), m.uid(), m.mode() & 0o777);
    }
}

/// What is wrong with a runtime directory, if anything.
fn runtime_dir_fault(link: bool, dir: bool, owner: u32, mode: u32, us: u32) -> Option<String> {
    if link {
        Some("is a link".into())
    } else if !dir {
        Some("is not a directory".into())
    } else if owner != us {
        Some(format!("belongs to user {owner}, not to us ({us})"))
    } else if mode & 0o077 != 0 {
        Some(format!("is open to others (mode {:o})", mode & 0o777))
    } else {
        None
    }
}

#[cfg(test)]
mod runtime_dir_tests {
    use super::runtime_dir_fault;

    #[test]
    fn only_our_own_closed_directory_passes() {
        assert_eq!(runtime_dir_fault(false, true, 1000, 0o40700, 1000), None);
        assert!(runtime_dir_fault(true, false, 1000, 0o120777, 1000).unwrap().contains("link"));
        assert!(runtime_dir_fault(false, false, 1000, 0o100600, 1000).unwrap().contains("not a directory"));
        assert!(runtime_dir_fault(false, true, 1001, 0o40700, 1000).unwrap().contains("user 1001"));
        assert!(runtime_dir_fault(false, true, 1000, 0o40755, 1000).unwrap().contains("755"));
        assert!(runtime_dir_fault(false, true, 1000, 0o41777, 1000).is_some(), "a shared /tmp-like folder");
    }
}
