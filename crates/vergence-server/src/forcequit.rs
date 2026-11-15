// Force Quit (tools milestone 1; Dave: "they call it force quit on the mac
// ... very simple as we said, term for the hard stuff"): the server lists
// the user's running applications as the dock knows them, with their icons,
// and ends one at once on request, by SIGKILL to every process behind its
// windows (a native window's bus connection, a Wayland client's
// credentials, an X11 window's _NET_WM_PID). Only the user's own clients
// reach the server, so only the user's own processes are ever named; the
// server itself and process 1 are never sent anything. The Force Quit app
// (vergence-quit) is only the window; Ctrl+Alt+Escape opens it.

use std::collections::BTreeSet;

use vergence_protocol::bus::{APP_ICON, AppInfo, MAX_APP_ICONS};

use crate::{Server, win::Win};

unsafe extern "C" {
    /// POSIX kill(2), in every libc the server links.
    fn kill(pid: i32, sig: i32) -> i32;
}

const SIGKILL: i32 = 9;

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
                responding: true,
            });
        }
        out
    }

    /// The process behind a window, if known.
    fn window_pid(&self, w: &Win) -> Option<u32> {
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
            let r = unsafe { kill(*p as i32, SIGKILL) };
            tracing::info!("force quit: {} pid {p} {}", item.name, if r == 0 { "killed" } else { "could not be killed" });
        }
        Ok(pids.len())
    }
}
