//! Vergence Remote (remote milestone 1, docs/remote-milestone-1.md): using a
//! Vergence machine from another as if sitting at it, over SSH.
//!
//! This crate is the protocol both sides speak (proto.rs) and the
//! compression of frames' rectangles (lz.rs, our own, in LZ4's published
//! block format). It has no platform code, so every viewer (on Vergence,
//! the Mac, Windows) and the host's program share it. The host's server
//! draws each frame at the viewer's size and scale and sends only what
//! changed ("Frames, not display lists" in the plan).
//!
//! Start reading: proto.rs's module comment (the stream), then
//! [`proto::Frame::apply`] (where the viewer meets untrusted input).

pub mod lz;
pub mod proto;

/// The host's socket for Vergence Remote, beside the session's bus: the
/// server listens on it, `vergence-remote` connects to it.
pub fn socket_for(runtime: &std::path::Path, display: &str) -> std::path::PathBuf {
    runtime.join(format!("vergence-remote-{display}"))
}

/// The running session's remote socket, for `vergence-remote` run over SSH
/// (where neither WAYLAND_DISPLAY nor, often, XDG_RUNTIME_DIR is set): in
/// `$XDG_RUNTIME_DIR`, or the usual runtime folders for this user, the
/// `vergence-remote-*` sockets something answers on. More than one (two
/// sessions) is an error the person sees.
pub fn find_socket() -> Result<std::path::PathBuf, String> {
    let uid = unsafe {
        unsafe extern "C" {
            fn getuid() -> u32;
        }
        getuid()
    };
    // XDG_RUNTIME_DIR when it is set (a login that has it, a test session);
    // else the usual places for this user.
    let dirs: Vec<std::path::PathBuf> = match std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        Some(d) => vec![d.into()],
        None => {
            let mut v: Vec<std::path::PathBuf> = vec![format!("/run/user/{uid}").into(), format!("/tmp/xdg-{uid}").into()];
            if let Ok(user) = std::env::var("USER") {
                v.push(format!("/var/run/xdg/{user}").into());
            }
            v
        }
    };
    let mut live = Vec::new();
    for d in dirs {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("vergence-remote-")
                && std::os::unix::net::UnixStream::connect(e.path()).is_ok()
                && !live.contains(&e.path())
            {
                live.push(e.path());
            }
        }
    }
    match live.len() {
        0 => Err("no Vergence session is running for this user here".into()),
        1 => Ok(live.remove(0)),
        _ => Err("more than one Vergence session is running for this user here".into()),
    }
}
