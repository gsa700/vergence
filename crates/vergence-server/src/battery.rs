// The battery's history (vergence-power's history.rs): one line a minute
// while the machine has a battery, kept a week, for the settings app's
// Battery page (Dave, 2026-11-14: "battery users will want it"). On its own
// thread: reading the battery touches /sys or sysctls, never the server's
// frame. Only in the session on the real screens (a nested test server
// would record the same battery twice).

use std::time::Duration;

use vergence_power::history;

use crate::state::Server;

fn record() {
    let Some(path) = history::path() else { return };
    let mut pruned_at = 0u64;
    loop {
        let now = history::now();
        if now.saturating_sub(pruned_at) >= 24 * 3600 {
            pruned_at = now;
            if let Err(e) = history::prune(&path, now)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!("battery: can't trim the history: {e}");
            }
        }
        let p = vergence_power::power();
        if let Some(b) = p.batteries.first()
            && let Some(percent) = b.percent
        {
            let r = history::Record { t: now, percent, watts: b.watts.unwrap_or(0.0) as f32, state: b.state };
            if let Err(e) = history::append(&path, &r) {
                tracing::warn!("battery: can't keep the history: {e}");
            }
        } else if pruned_at == now {
            // No battery: nothing to keep, and no need to look often.
            tracing::info!("battery: none here; no history kept");
            return;
        }
        std::thread::sleep(Duration::from_secs(history::EVERY));
    }
}

impl Server {
    pub fn start_battery_history(&mut self) {
        if !std::env::args().any(|a| a == "--drm") {
            return;
        }
        if let Err(e) = std::thread::Builder::new().name("battery".into()).spawn(record) {
            tracing::warn!("battery: can't start the history: {e}");
        }
    }
}
