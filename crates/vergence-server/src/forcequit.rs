// Force Quit (tools milestone 1; Dave: "they call it force quit on the mac
// ... very simple as we said, term for the hard stuff"): the server lists
// the user's running applications as the dock knows them, with their icons,
// and ends one on request: SIGTERM to every process behind its windows (a
// native window's bus connection, a Wayland client's credentials, an X11
// window's _NET_WM_PID), so a program that still can flushes its files and
// lets its children go, then two seconds later SIGKILL to any of them still
// there (Dave, 2026-11-14: try gently "just before killing"). Only the user's own clients
// reach the server, so only the user's own processes are ever named; the
// server itself and process 1 are never sent anything. The Force Quit app
// (vergence-quit) is only the window; Ctrl+Alt+Escape opens it.
//
// "Not responding": while Force Quit asks for the list (every two seconds),
// each app's clients are asked whether they are alive (a native client by
// the bus's Alive, which the toolkit answers from its loop; a Wayland client
// by xdg_wm_base's ping); one asked more than NOT_RESPONDING ago and still
// silent is not responding. X11 clients are taken as answering.

use std::collections::BTreeSet;

use vergence_protocol::bus::{APP_ICON, AppInfo, MAX_APP_ICONS};

use crate::{Server, win::Win};

unsafe extern "C" {
    /// POSIX kill(2), in every libc the server links.
    fn kill(pid: i32, sig: i32) -> i32;
}

const SIGKILL: i32 = 9;
const SIGTERM: i32 = 15;
/// How long a client may leave "are you alive?" unanswered before it is not responding.
const NOT_RESPONDING: std::time::Duration = std::time::Duration::from_secs(3);
/// How long a program asked to end has before it is killed.
const GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether a process is still there (kill with signal 0 asks only that).
fn alive(pid: u32) -> bool {
    // SAFETY: kill(2) with signal 0 sends nothing; no memory is passed.
    unsafe { kill(pid as i32, 0) == 0 }
}

impl Server {
    /// The user's running applications: those of the dock's items that have windows.
    pub fn running_apps(&mut self) -> Vec<AppInfo> {
        let items = self.shelf_items();
        let mut out = Vec::new();
        for (item, windows) in items {
            if windows.is_empty() {
                continue;
            }
            let icon = (out.len() < MAX_APP_ICONS && !item.icon.is_empty())
                .then(|| self.shelf_icon(&item.icon, APP_ICON as u32))
                .flatten()
                .filter(|i| i.width == APP_ICON as u32 && i.height == APP_ICON as u32 && i.rgba.len() == (APP_ICON as usize).pow(2) * 4);
            out.push(AppInfo {
                key: item.key.clone(),
                name: item.name.clone(),
                icon_side: if icon.is_some() { APP_ICON } else { 0 },
                icon: icon.map(|i| i.rgba.clone()).unwrap_or_default(),
                responding: windows.iter().all(|w| self.answering(w)),
                quitting: self.quitting.contains_key(&item.key),
            });
        }
        out
    }

    /// Whether a window's client answers: asked now if nothing is pending,
    /// silent if what was asked is older than NOT_RESPONDING.
    fn answering(&mut self, w: &Win) -> bool {
        let now = std::time::Instant::now();
        if let Some(n) = w.native() {
            let client = n.client;
            return match self.alive_asked.get(&client) {
                Some(since) => now.duration_since(*since) < NOT_RESPONDING,
                None => {
                    let serial: u32 = smithay::utils::SERIAL_COUNTER.next_serial().into();
                    self.bus_send(client, vergence_protocol::bus::ServerMsg::Alive { serial });
                    self.alive_asked.insert(client, now);
                    true
                }
            };
        }
        let Some(t) = w.toplevel() else { return true };
        let Some(id) = smithay::reexports::wayland_server::Resource::client(t.wl_surface()).map(|c| c.id()) else { return true };
        match t.client().send_ping(smithay::utils::SERIAL_COUNTER.next_serial()) {
            // Sent: the last one was answered.
            Ok(()) => {
                self.pinged.insert(id, now);
                true
            }
            Err(smithay::wayland::shell::PingError::PingAlreadyPending(_)) => self.pinged.get(&id).is_none_or(|since| now.duration_since(*since) < NOT_RESPONDING),
            Err(_) => true,
        }
    }

    /// The process behind a window, if known.
    pub(crate) fn window_pid(&self, w: &Win) -> Option<u32> {
        if let Some(n) = w.native() {
            return self.bus_pid(n.client);
        }
        if let Some(x) = w.x11_surface() {
            return x.pid();
        }
        let t = w.toplevel()?;
        let client = smithay::reexports::wayland_server::Resource::client(t.wl_surface())?;
        client.get_credentials(&self.display_handle).ok().map(|c| c.pid as u32)
    }

    /// Ends an application at once (its dock key): SIGKILL to each process
    /// behind its windows. Err if it has none the server can name.
    pub fn force_quit(&mut self, key: &str) -> Result<usize, String> {
        let items = self.shelf_items();
        let Some((item, windows)) = items.into_iter().find(|(i, ws)| i.key == key && !ws.is_empty()) else {
            return Err(format!("{key} is not running"));
        };
        let own = std::process::id();
        let pids: BTreeSet<u32> = windows.iter().filter_map(|w| self.window_pid(w)).filter(|p| *p > 1 && *p != own).collect();
        if pids.is_empty() {
            return Err(format!("no process known for {}", item.name));
        }
        for p in &pids {
            // SAFETY: kill(2) with a pid and a signal number; no memory is passed.
            let r = unsafe { kill(*p as i32, SIGTERM) };
            tracing::info!("force quit: {} pid {p} {}", item.name, if r == 0 { "asked to end" } else { "could not be signalled" });
        }
        let pids: Vec<u32> = pids.into_iter().collect();
        self.quitting.insert(key.to_string(), pids.clone());
        let (key, name) = (key.to_string(), item.name.clone());
        let (fallback, fallback_key) = (pids.clone(), key.clone());
        let timer = smithay::reexports::calloop::timer::Timer::from_duration(GRACE);
        let inserted = self.loop_handle.insert_source(timer, move |_, _, state| {
            for p in &pids {
                if alive(*p) {
                    // SAFETY: as above.
                    let r = unsafe { kill(*p as i32, SIGKILL) };
                    tracing::info!("force quit: {name} pid {p} still there after {GRACE:?}: {}", if r == 0 { "killed" } else { "could not be killed" });
                } else {
                    tracing::info!("force quit: {name} pid {p} ended when asked");
                }
            }
            state.quitting.remove(&key);
            smithay::reexports::calloop::timer::TimeoutAction::Drop
        });
        if let Err(e) = inserted {
            tracing::warn!("force quit: no timer ({e:?}); killing at once");
            for p in &fallback {
                // SAFETY: as above.
                unsafe { kill(*p as i32, SIGKILL) };
            }
            self.quitting.remove(&fallback_key);
        }
        Ok(self.quitting.values().map(Vec::len).sum())
    }
}
