//! `vergd watch`: print the setup, then one block per change.
//!
//! One X connection listens for RandR events. A single plug or mode change
//! makes the server send a burst of events (screen, output, CRTC, and more as
//! the desktop reacts), so events are collected until the server has been
//! quiet for [`SETTLE`], then the setup is read again and compared with the
//! last one. Only real differences are printed.
//!
//! FreeBSD only: Xorg does not re-probe outputs when a monitor is plugged in
//! (libudev-devd passes input devices, not DRM hot-plug; found on Hubble
//! 2026-11-06). The kernel does tell devd, so a thread reads devd's pipe and,
//! on a DRM HOTPLUG event, makes X re-probe with RRGetScreenResources; X then
//! sends the usual RandR events to the main connection.

use std::time::{Duration, Instant};

use x11rb::connection::Connection as _;
use x11rb::protocol::randr::{ConnectionExt as _, NotifyMask};

use crate::x11::{self, Setup};
use crate::{key_line, print_setup};

/// How long the server must be quiet before a burst counts as finished.
const SETTLE: Duration = Duration::from_millis(400);

pub fn watch() -> Result<(), Box<dyn std::error::Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;
    conn.randr_select_input(
        root,
        NotifyMask::SCREEN_CHANGE
            | NotifyMask::OUTPUT_CHANGE
            | NotifyMask::CRTC_CHANGE
            | NotifyMask::OUTPUT_PROPERTY,
    )?
    .check()?;

    #[cfg(target_os = "freebsd")]
    devd::spawn();

    let mut last = x11::read_setup()?;
    println!("{} watching", stamp());
    print_setup(&last);

    loop {
        conn.wait_for_event()?;
        // Collect the rest of the burst.
        let mut quiet_since = Instant::now();
        while quiet_since.elapsed() < SETTLE {
            if conn.poll_for_event()?.is_some() {
                quiet_since = Instant::now();
            } else {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let now = x11::read_setup()?;
        let changes = diff(&last, &now);
        if !changes.is_empty() {
            for c in &changes {
                println!("{} {c}", stamp());
            }
            println!("{} {}", stamp(), key_line(&now));
        }
        last = now;
    }
}

/// One line per output that changed, in X's output order.
fn diff(old: &Setup, new: &Setup) -> Vec<String> {
    let mut out = Vec::new();
    for n in &new.outputs {
        let o = old.outputs.iter().find(|o| o.name == n.name);
        let was = o.map(|o| o.summary()).unwrap_or_default();
        let is = n.summary();
        if was != is {
            out.push(format!("{:<9} {is}", n.name));
        }
    }
    for o in &old.outputs {
        if !new.outputs.iter().any(|n| n.name == o.name) {
            out.push(format!("{:<9} gone", o.name));
        }
    }
    out
}

/// Local time as HH:MM:SS.
fn stamp() -> String {
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return "--:--:--".into();
    }
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

#[cfg(target_os = "freebsd")]
mod devd {
    //! Listen to devd for DRM connector hot-plug and make X re-probe.

    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    use x11rb::connection::Connection as _;
    use x11rb::protocol::randr::ConnectionExt as _;

    /// devd's stream socket: one event per line, readable by any user.
    const PIPE: &str = "/var/run/devd.pipe";

    pub fn spawn() {
        std::thread::spawn(|| {
            if let Err(e) = run() {
                eprintln!("vergd: devd listener stopped ({e}); plugs are only seen when X re-probes");
            }
        });
    }

    fn run() -> Result<(), Box<dyn std::error::Error>> {
        let stream = UnixStream::connect(PIPE).map_err(|e| format!("{PIPE}: {e}"))?;
        let (conn, screen_num) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen_num].root;
        for line in BufReader::new(stream).lines() {
            let line = line?;
            if is_drm_hotplug(&line) {
                // The full (probing) request: X asks the driver for every
                // output's state and sends RandR events if anything changed.
                conn.randr_get_screen_resources(root)?.reply()?;
            }
        }
        Err("devd closed the pipe".into())
    }

    /// `!system=DRM subsystem=CONNECTOR type=HOTPLUG cdev=dri/card0 connector=115`
    pub fn is_drm_hotplug(line: &str) -> bool {
        let Some(rest) = line.strip_prefix('!') else { return false };
        rest.split_whitespace().any(|w| w == "system=DRM")
            && rest.split_whitespace().any(|w| w == "type=HOTPLUG")
    }

    #[cfg(test)]
    mod tests {
        use super::is_drm_hotplug;

        #[test]
        fn matches_hubble_event() {
            assert!(is_drm_hotplug(
                "!system=DRM subsystem=CONNECTOR type=HOTPLUG cdev=dri/card0 connector=115"
            ));
            assert!(!is_drm_hotplug("!system=USB subsystem=DEVICE type=ATTACH"));
            assert!(!is_drm_hotplug("+ugen0.2 at bus=0"));
        }
    }
}
