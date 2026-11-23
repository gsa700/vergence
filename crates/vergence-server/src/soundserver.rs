// The session's sound server on Linux: PipeWire, with WirePlumber and its
// PulseAudio side (pipewire-pulse). Most Linux programs play through the
// PulseAudio protocol (AlbumWall, games, browsers that prefer it), and a
// system without systemd has no user service to start one (Void: a fresh
// install with Vergence had no sound server at all, so AlbumWall was silent,
// 2026-11-22). Vergence's own sound needs none (vergence-sound: ALSA, or the
// PulseAudio protocol when a server runs), so this is for other programs.
//
// Started here, by the server, right after the D-Bus edge is up and before
// the session's apps are restored, so they find it; stopped when the server
// ends, since a PipeWire that outlives its session bus loses its outputs
// (WirePlumber's device reservation goes with the bus). Only for a session
// on the real monitors, never at the login screen or in a headless test
// session (it would take the sound card from the person's session). Off with
// desktop.toml `[sound] pipewire = false`; nothing is started when a sound
// server already runs for the user or PipeWire is not installed. FreeBSD
// keeps OSS (PipeWire there is the user's choice). For tests,
// VERGENCE_SOUND_SERVER=on starts it in any session (a headless one too) and
// =off never.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The server is ending: the keepers start nothing more.
static STOPPING: AtomicBool = AtomicBool::new(false);
/// The processes started, by pid, to stop at the end.
static RUNNING: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// PipeWire and its two companions, if this session should have them.
pub fn start() {
    let forced = std::env::var("VERGENCE_SOUND_SERVER").unwrap_or_default();
    if !cfg!(target_os = "linux") || crate::greeter() || forced == "off" {
        return;
    }
    if forced != "on" && !std::env::args().any(|a| a == "--drm") {
        return;
    }
    let text = vergence_config::effective_text_with(crate::wallpaper::desktop_path().as_deref());
    if !wanted(&text) {
        return tracing::info!("sound server: off ([sound] pipewire = false)");
    }
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from) else { return };
    if answers(&dir.join("pipewire-0")) || answers(&dir.join("pulse/native")) {
        return tracing::info!("sound server: one already runs for this user; not starting PipeWire");
    }
    if which("pipewire").is_none() {
        return tracing::info!("sound server: PipeWire is not installed; programs that need one get no sound");
    }
    std::thread::spawn(move || {
        keep("pipewire");
        // The companions once PipeWire answers, unless its own configuration
        // starts them (Void's documented setup links them into
        // pipewire.conf.d, and PipeWire then runs them itself).
        let up = Instant::now();
        while !answers(&dir.join("pipewire-0")) {
            if up.elapsed() > Duration::from_secs(10) || STOPPING.load(Ordering::Relaxed) {
                return tracing::warn!("sound server: PipeWire did not come up");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        std::thread::sleep(Duration::from_millis(1500));
        for program in ["wireplumber", "pipewire-pulse"] {
            if running_for_me(program) {
                tracing::info!("sound server: {program} runs already (PipeWire's configuration starts it)");
            } else if which(program).is_some() {
                keep(program);
            } else {
                tracing::warn!("sound server: {program} is not installed");
            }
        }
    });
}

/// Every process started here, stopped (the server is ending).
pub fn stop() {
    STOPPING.store(true, Ordering::Relaxed);
    let pids = std::mem::take(&mut *RUNNING.lock().unwrap_or_else(|e| e.into_inner()));
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    for pid in pids.into_iter().rev() {
        // SAFETY: kill only sends a signal; the pids are our own children.
        unsafe { kill(pid as i32, 15) };
    }
}

/// `[sound] pipewire`, true unless set false.
fn wanted(text: &str) -> bool {
    text.parse::<toml::Table>().ok().and_then(|t| t.get("sound")?.get("pipewire")?.as_bool()).unwrap_or(true)
}

/// A program kept running on a thread of its own: started again when it
/// exits, at most five times a minute (as the desktop's icons are).
fn keep(program: &'static str) {
    std::thread::spawn(move || {
        let mut starts: Vec<Instant> = Vec::new();
        while !STOPPING.load(Ordering::Relaxed) {
            starts.retain(|t| t.elapsed() < Duration::from_secs(60));
            if starts.len() >= 5 {
                return tracing::warn!("sound server: {program} exited five times in a minute; not started again");
            }
            starts.push(Instant::now());
            let mut cmd = std::process::Command::new(program);
            ends_with_us(&mut cmd);
            let mut child = match crate::spawn_child(&mut cmd) {
                Ok(c) => c,
                Err(e) => return tracing::warn!("sound server: {program} could not be started: {e}"),
            };
            let pid = child.id();
            RUNNING.lock().unwrap_or_else(|e| e.into_inner()).push(pid);
            tracing::info!("sound server: {program} started (pid {pid})");
            let status = child.wait();
            RUNNING.lock().unwrap_or_else(|e| e.into_inner()).retain(|p| *p != pid);
            if STOPPING.load(Ordering::Relaxed) {
                return;
            }
            tracing::info!("sound server: {program} exited ({status:?}); starting it again");
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

/// The child is sent SIGTERM by the kernel when the server ends, however it
/// ends (a crash, or a test session killed, never reaches [`stop`]). Linux's
/// parent-death signal follows the thread that started the child; the keeper
/// thread lives as long as its child, so it holds.
fn ends_with_us(cmd: &mut std::process::Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        unsafe extern "C" {
            fn prctl(option: i32, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> i32;
        }
        const PR_SET_PDEATHSIG: i32 = 1;
        // SAFETY: prctl is async-signal-safe and changes only the new child.
        unsafe {
            cmd.pre_exec(|| {
                prctl(PR_SET_PDEATHSIG, 15, 0, 0, 0);
                Ok(())
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = cmd;
}

/// Something listens on this socket (a file left by a crash does not count).
fn answers(path: &std::path::Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

/// The program in PATH.
fn which(program: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH")?.to_str()?.split(':').map(|d| std::path::Path::new(d).join(program)).find(|p| p.is_file())
}

/// A process of this name runs as this user (Linux's /proc).
fn running_for_me(program: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    let me = unsafe { getuid() };
    let Ok(dir) = std::fs::read_dir("/proc") else { return false };
    dir.flatten().any(|e| {
        e.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
            && e.metadata().is_ok_and(|m| m.uid() == me)
            && std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| c.trim_end() == program)
    })
}

unsafe extern "C" {
    fn getuid() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_unless_switched_off() {
        assert!(wanted(""));
        assert!(wanted("[sound]\nvolume-step = 5\n"));
        assert!(!wanted("[sound]\npipewire = false\n"));
        assert!(wanted("[sound]\npipewire = true\n"));
    }

    #[test]
    fn a_file_is_not_a_listener() {
        let dir = std::env::temp_dir().join(format!("vergence-sound-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stale = dir.join("pipewire-0");
        std::fs::write(&stale, b"").unwrap();
        assert!(!answers(&stale));
        let live = dir.join("live");
        let _l = std::os::unix::net::UnixListener::bind(&live).unwrap();
        assert!(answers(&live));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
