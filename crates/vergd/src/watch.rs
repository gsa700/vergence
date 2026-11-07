//! `vergd watch`: print the setup, then one block per change; since
//! milestone 2 also remember and restore each setup's arrangement.
//!
//! One X connection listens for RandR events. A single plug or mode change
//! makes the server send a burst of events (screen, output, CRTC, and more as
//! the desktop reacts), so events are collected until the server has been
//! quiet for [`SETTLE`], then the setup is read again and compared with the
//! last one.
//!
//! Remembering (decisions in docs/milestone-2.md):
//! - Key unchanged, arrangement changed: the user (or the desktop) changed
//!   it; learn it.
//! - Key changed to a known setup: wait a little longer ([`PLUG_SETTLE`]) so
//!   the desktop (Xfce) has reacted first, then apply the saved arrangement.
//!   vergd acts last.
//! - Key changed to an unknown setup: leave it as X and the desktop made it,
//!   and learn it.
//! - For [`GUARD`] after an apply, a change on the same setup is the desktop
//!   reacting late, not the user: put the profile back (at most
//!   [`MAX_REAPPLY`] times) instead of learning it.
//!
//! FreeBSD only: Xorg does not re-probe outputs when a monitor is plugged in
//! (libudev-devd passes input devices, not DRM hot-plug; found on Hubble
//! 2026-11-06). The kernel does tell devd, so a thread reads devd's pipe and,
//! on a DRM HOTPLUG event, makes X re-probe with RRGetScreenResources; X then
//! sends the usual RandR events to the main connection.

use std::time::{Duration, Instant};

use x11rb::connection::Connection as _;
use x11rb::protocol::randr::{ConnectionExt as _, NotifyMask};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

use vergence_profile::arrangement::{same_arrangement, Profile, Store};

use crate::apply::{self, Outcome};
use crate::windows::WindowMemory;
use crate::x11::{self, Setup};
use crate::{key_line, now_string, print_setup, short, stamp};

/// How long the server must be quiet before a burst counts as finished.
const SETTLE: Duration = Duration::from_millis(400);
/// After a plug or unplug, how long to wait for quiet before applying, so
/// the desktop's own reaction (Xfce: about a second) comes first.
const PLUG_SETTLE: Duration = Duration::from_millis(2000);
/// After an apply, how long changes count as the desktop reacting.
const GUARD: Duration = Duration::from_secs(6);
const MAX_REAPPLY: u32 = 2;
/// After a plug, extra quiet before windows are put back.
const WINDOW_SETTLE: Duration = Duration::from_millis(800);

/// Wait until the server has sent nothing for `quiet`.
fn drain(conn: &RustConnection, quiet: Duration) -> Result<(), Box<dyn std::error::Error>> {
    let mut since = Instant::now();
    while since.elapsed() < quiet {
        if conn.poll_for_event()?.is_some() {
            since = Instant::now();
        } else {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    Ok(())
}

struct Memory {
    store: Store,
    observe_only: bool,
    guard_until: Option<Instant>,
    reapplied: u32,
    /// Until when a change counts as the desktop finishing a new setup
    /// (only used for the log wording).
    new_setup_until: Option<Instant>,
}

impl Memory {
    fn learn(&mut self, setup: &Setup, why: &str) -> Result<(), Box<dyn std::error::Error>> {
        let key = setup.key();
        let arr = setup.arrangement();
        if let Some(p) = self.store.get(&key) {
            if same_arrangement(&p.outputs, &arr) {
                return Ok(());
            }
        }
        self.store.put(Profile { key: key.clone(), updated: now_string(), outputs: arr.clone() });
        crate::store::save(&self.store)?;
        println!("{} learned {} for {}:", stamp(), why, short(&key));
        for o in &arr {
            println!("{}   {}", stamp(), o.describe());
        }
        Ok(())
    }

    /// Apply the saved profile for this setup; returns true if it changed
    /// anything.
    fn restore(&mut self, setup: &Setup) -> Result<bool, Box<dyn std::error::Error>> {
        let key = setup.key();
        let Some(p) = self.store.get(&key).cloned() else { return Ok(false) };
        match apply::apply(&p.outputs, false)? {
            Outcome::AlreadyThere => {
                println!("{} known setup {}: already as saved", stamp(), short(&key));
                Ok(false)
            }
            Outcome::Applied(steps) => {
                println!("{} applied profile {}:", stamp(), short(&key));
                for s in steps {
                    println!("{}   {s}", stamp());
                }
                self.guard_until = Some(Instant::now() + GUARD);
                Ok(true)
            }
            Outcome::Refused(why) => {
                println!("{} NOT applying profile {}: {why}", stamp(), short(&key));
                Ok(false)
            }
            Outcome::Reverted(why) => {
                println!("{} applying profile {} FAILED: {why}", stamp(), short(&key));
                self.guard_until = Some(Instant::now() + GUARD);
                Ok(true)
            }
        }
    }

    /// Decide what a settled change means. `plugged` = the key changed.
    fn react(&mut self, conn: &RustConnection, root: u32, now: Setup, plugged: bool)
        -> Result<Setup, Box<dyn std::error::Error>> {
        if self.observe_only || now.edids().is_empty() {
            return Ok(now);
        }
        // The file is the truth: `vergd forget` and hand edits may have
        // changed it since the last change. A file that no longer parses
        // stops vergd rather than being overwritten.
        self.store = crate::store::load()?;
        let key = now.key();
        let known = self.store.get(&key).is_some();
        let guarded = self.guard_until.is_some_and(|t| Instant::now() < t);

        if plugged {
            self.reapplied = 0;
            self.new_setup_until = None;
            if !known {
                self.learn(&now, "new setup")?;
                self.new_setup_until = Some(Instant::now() + GUARD);
                return Ok(now);
            }
            // Let the desktop react first, then act last.
            drain(conn, PLUG_SETTLE)?;
            let before = now;
            let now = x11::read_setup_on(conn, root)?;
            if now.key() != key {
                // Plugged again while waiting. Keep the setup we reacted to:
                // the main loop sees the new one differs and handles it as a
                // plug (adopting it here would hide the plug).
                return Ok(before);
            }
            if self.restore(&now)? {
                return Ok(x11::read_setup_on(conn, root)?);
            }
            return Ok(now);
        }

        // Same setup, different arrangement.
        if guarded && known {
            if self.reapplied < MAX_REAPPLY {
                self.reapplied += 1;
                println!("{} change right after an apply: the desktop reacting, putting the profile back", stamp());
                if self.restore(&now)? {
                    return Ok(x11::read_setup_on(conn, root)?);
                }
                return Ok(now);
            }
            println!("{} still changing after {MAX_REAPPLY} re-applies; leaving it and learning it", stamp());
        }
        // vergd cannot see who made a change. Right after a new setup it is
        // almost always the desktop (Xfce mirrors a new monitor ~1 s after
        // the plug); otherwise say nothing about who.
        let why = if self.new_setup_until.is_some_and(|t| Instant::now() < t) {
            "the desktop's change to the new setup"
        } else {
            "an arrangement change"
        };
        self.learn(&now, why)?;
        Ok(now)
    }
}

pub fn watch(observe_only: bool, no_windows: bool, no_xsettings: bool) -> Result<(), Box<dyn std::error::Error>> {
    let (conn, root) = x11::connect()?;
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

    let store = if observe_only { Store::default() } else { crate::store::load()? };
    let mut mem =
        Memory { store, observe_only, guard_until: None, reapplied: 0, new_setup_until: None };
    let mut wm = WindowMemory::new(&conn, root, !observe_only && !no_windows)?;
    // Milestone 4: vergd is the XSETTINGS manager. A failure here is
    // reported and vergd carries on without it.
    let mut xs = if observe_only || no_xsettings {
        None
    } else {
        match crate::xsettings::Manager::start(&conn, root) {
            Ok(m) => Some(m),
            Err(e) => {
                println!("{} XSETTINGS manager not started: {e}", stamp());
                None
            }
        }
    };

    let mut last = x11::read_setup_on(&conn, root)?;
    println!(
        "{} watching{}",
        stamp(),
        if observe_only {
            " (observe only)"
        } else if no_windows {
            " (monitors only, no window memory); profiles in ~/.config/vergence/"
        } else {
            "; profiles and window memory in ~/.config/vergence/"
        }
    );
    print_setup(&last);
    // At start: a known setup is restored, an unknown one learned. Windows
    // already open are tracked, not moved.
    last = mem.react(&conn, root, last, true)?;
    wm.mark_unstable();

    // Events are collected until the server has been quiet for SETTLE, then
    // handled by kind: RandR (monitors) and everything else (windows).
    let mut randr_seen = false;
    let mut windows_seen = false;
    let mut last_event: Option<Instant> = None;
    loop {
        while let Some(ev) = conn.poll_for_event()? {
            match ev {
                Event::RandrScreenChangeNotify(_) | Event::RandrNotify(_) => randr_seen = true,
                Event::Error(_) => {} // a window vanished while we looked at it
                _ => windows_seen = true,
            }
            last_event = Some(Instant::now());
        }

        if last_event.is_some_and(|t| t.elapsed() >= SETTLE) {
            last_event = None;
            if randr_seen {
                randr_seen = false;
                let now = x11::read_setup_on(&conn, root)?;
                let changes = diff(&last, &now);
                if !changes.is_empty() {
                    for c in &changes {
                        println!("{} {c}", stamp());
                    }
                    println!("{} {}", stamp(), key_line(&now));
                    let plugged = now.key() != last.key();
                    last = mem.react(&conn, root, now, plugged)?;
                    wm.mark_unstable();
                    if wm.enabled {
                        // Let the monitors and the window manager settle,
                        // then put the windows back for this setup. Also
                        // when only the arrangement changed (a monitor
                        // moved, no plug): positions are kept relative to
                        // each monitor, so windows follow their monitor;
                        // the window manager leaves them where they were
                        // on screen, i.e. on whatever monitor is there now.
                        drain(&conn, WINDOW_SETTLE)?;
                        let cur = x11::read_setup_on(&conn, root)?;
                        if cur.key() == last.key() {
                            last = cur;
                            wm.restore_all(&conn, &last)?;
                        }
                    }
                    // A monitor plugged or unplugged while vergd waited above
                    // (its events were drained): handle it now, as a plug,
                    // against the setup vergd last acted on.
                    if x11::read_setup_on(&conn, root)?.key() != last.key() {
                        randr_seen = true;
                        last_event = Some(Instant::now());
                    }
                    // drain() swallowed window events; look again.
                    windows_seen = true;
                }
            }
            if windows_seen {
                windows_seen = false;
                wm.on_window_events(&conn)?;
            }
        }

        wm.tick(&conn, &last)?;
        if let Some(m) = xs.as_mut() {
            m.tick(&conn)?;
        }
        std::thread::sleep(Duration::from_millis(20));
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
    if out.is_empty() && old.primary != new.primary {
        out.push("primary output changed".into());
    }
    out
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
