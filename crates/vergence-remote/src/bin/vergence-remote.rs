//! `vergence-remote`: the host's side of Vergence Remote, run by SSH for a
//! viewer (`ssh <host> vergence-remote`). Its standard input and output are
//! the channel to the viewer; it connects to the running session's remote
//! socket (crates/vergence-server/src/remote.rs) and relays between the two
//! in Vergence Remote's protocol (docs/remote-protocol.md), compressing
//! frames on their way out (the server sends them uncompressed on the local
//! leg, so its loop never spends time on that).
//!
//! It checks both sides' messages as it relays (each is decoded and written
//! again), so nothing the viewer sends reaches the server unchecked.
//!
//! With no session of this user's running, and Vergence Remote on in their
//! settings, it starts one with no monitors (remote milestone 1, step 6;
//! Dave: "log in remotely and have no active session on the server side.
//! Like ssh can do, but with the vergence desktop"): `vergence-session
//! --monitorless`, at the viewer's size and scale, in a session of its own
//! so it stays when the viewer goes (his ham club's remote station: "the
//! apps need to stay running"). SSH has already said who the person is.

use std::io::{BufReader, BufWriter, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use vergence_remote::proto::{self, Rect, ToHost, ToViewer};

/// Bye to a viewer that has had the opening already.
fn bye(out: &mut impl Write, why: &str) -> ExitCode {
    let _ = ToViewer::Bye { reason: why.to_string() }.write(out);
    let _ = out.flush();
    eprintln!("vergence-remote: {why}");
    ExitCode::from(1)
}

/// Vergence Remote is on in this person's settings (Privacy > Remote; off by
/// default): the same switch the running server listens by.
fn enabled() -> bool {
    vergence_config::effective()
        .get("remote")
        .and_then(|r| r.get("enabled"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
}

/// A session with no monitors started for this viewer, `px` pixels at
/// `scale`, then its remote socket once it answers.
fn start_session(px: (u32, u32), scale: f32) -> Result<std::path::PathBuf, String> {
    // vergence-session beside this program (installed together), never one
    // found elsewhere, and only one that knows --monitorless: an older one
    // ignores the option and starts on the real monitors (2026-11-23, on
    // Hubble beside a running session: six failed tries and its log rotated
    // away). VERGENCE_SESSION for the tests.
    let session = match std::env::var_os("VERGENCE_SESSION") {
        Some(s) => std::path::PathBuf::from(s),
        None => std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join("vergence-session")))
            .ok_or("no vergence-session beside vergence-remote")?,
    };
    let knows = std::fs::read_to_string(&session).is_ok_and(|t| t.contains("--monitorless)"));
    if !knows {
        return Err(format!("{} cannot start a session with no monitors (an older Vergence?)", session.display()));
    }
    // A nominal physical size for that scale (its monitor identity); the
    // scale itself is given outright.
    let mm = |p: u32| ((p as f32 * 25.4 / (96.0 * scale)).round() as u32).max(1);
    let spec = format!("{}x{} {}x{}@60 scale={:.4}", px.0, px.1, mm(px.0), mm(px.1), scale);
    let mut cmd = Command::new(&session);
    cmd.arg("--monitorless").arg(&spec).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // Its own runtime directory, owner-only (not the SSH login's: that goes
    // when the login does, and the session stays). One that is not ours (made
    // first by another account, or a link) is refused, as vergence-session
    // and the server refuse one.
    let run = vergence_remote::monitorless_dir();
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        let _ = std::fs::DirBuilder::new().mode(0o700).create(&run);
        let m = std::fs::symlink_metadata(&run).map_err(|e| format!("{}: {e}", run.display()))?;
        let ours = m.is_dir() && !m.file_type().is_symlink() && m.uid() == vergence_remote::uid();
        if !ours || std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700)).is_err() {
            return Err(format!("{} is not this account's own folder", run.display()));
        }
    }
    cmd.env("XDG_RUNTIME_DIR", &run).env_remove("WAYLAND_DISPLAY").env_remove("DISPLAY");
    // Its own session (no controlling terminal, out of SSH's process group),
    // so the SSH connection ending does not end it.
    unsafe {
        cmd.pre_exec(|| {
            unsafe extern "C" {
                fn setsid() -> i32;
            }
            setsid();
            Ok(())
        });
    }
    cmd.spawn().map_err(|e| format!("{}: {e}", session.display()))?;
    eprintln!("vergence-remote: started a session with no monitors ({spec})");
    let until = Instant::now() + Duration::from_secs(30);
    while Instant::now() < until {
        if let Ok(p) = vergence_remote::find_socket() {
            return Ok(p);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err("the session started here did not answer within 30 seconds".into())
}

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("vergence-remote, protocol version {}", proto::VERSION);
        return ExitCode::SUCCESS;
    }
    // The viewer first: its Hello says the size and scale a session started
    // for it should have.
    let mut viewer_r = BufReader::new(std::io::stdin());
    let mut viewer_w = BufWriter::with_capacity(1 << 20, std::io::stdout());
    if proto::write_opening(&mut viewer_w).and_then(|_| viewer_w.flush()).is_err() {
        return ExitCode::from(1);
    }
    if let Err(e) = proto::read_opening(&mut viewer_r) {
        eprintln!("vergence-remote: the viewer: {e}");
        return ExitCode::from(1);
    }
    let hello = match ToHost::read(&mut viewer_r) {
        Ok(h @ ToHost::Hello { .. }) => h,
        Ok(_) => return bye(&mut viewer_w, "the viewer did not begin with Hello"),
        Err(e) => return bye(&mut viewer_w, &format!("the viewer: {e}")),
    };
    let path = match vergence_remote::live_sockets().len() {
        1 => vergence_remote::find_socket(),
        0 if !enabled() => Err("Vergence Remote is off on this computer (Settings, Privacy, Remote)".into()),
        0 => {
            let ToHost::Hello { width, height, scale, .. } = hello else { unreachable!() };
            start_session((width, height), scale)
        }
        _ => vergence_remote::find_socket(),
    };
    let path = match path {
        Ok(p) => p,
        Err(e) => return bye(&mut viewer_w, &e),
    };
    let local = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) => return bye(&mut viewer_w, &format!("{}: {e}", path.display())),
    };
    let Ok(local_w) = local.try_clone() else { return bye(&mut viewer_w, "no second handle on the socket") };
    let mut local_r = BufReader::with_capacity(1 << 20, local);
    let mut local_w = BufWriter::new(local_w);
    if let Err(e) = proto::write_opening(&mut local_w)
        .map_err(proto::Error::Io)
        .and_then(|_| proto::read_opening(&mut local_r))
        .and_then(|_| hello.write(&mut local_w).and_then(|_| local_w.flush()).map_err(proto::Error::Io))
    {
        return bye(&mut viewer_w, &format!("the session: {e}"));
    }
    // Viewer to session, on a thread of its own.
    let up = std::thread::spawn(move || {
        while let Ok(m) = ToHost::read(&mut viewer_r) {
            let bye = m == ToHost::Bye;
            if m.write(&mut local_w).and_then(|_| local_w.flush()).is_err() || bye {
                break;
            }
        }
        let _ = local_w.flush();
        if let Ok(s) = local_w.into_inner() {
            let _ = s.shutdown(std::net::Shutdown::Write);
        }
    });
    // Session to viewer: frames compressed on the way.
    let code = loop {
        let m = match ToViewer::read(&mut local_r) {
            Ok(m) => m,
            Err(_) => break ExitCode::SUCCESS,
        };
        let m = match m {
            ToViewer::Frame(mut f) => {
                f.rects = f.rects.into_iter().map(|r| Rect::new(r.x, r.y, r.w, r.h, &r.data)).collect();
                ToViewer::Frame(f)
            }
            other => other,
        };
        let bye = matches!(m, ToViewer::Bye { .. });
        if m.write(&mut viewer_w).and_then(|_| viewer_w.flush()).is_err() {
            break ExitCode::from(1);
        }
        if bye {
            break ExitCode::SUCCESS;
        }
    };
    drop(up);
    code
}
