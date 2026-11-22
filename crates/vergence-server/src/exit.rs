// Exit Vergence (2026-11-15, Dave: "We might want to confirm on logout
// though?"): the desktop menu asks first (menu.rs); then every window is
// asked to close as its close button would (close_window: a program with
// work not saved asks about it), and the session ends once they are gone.
// If any is still open after GRACE (a question waiting for its answer), the
// exit stops there and a card says why: nothing is lost by exiting.
// The session's list of running apps (session.rs) is held while they close,
// so the next session starts them again. Ctrl+Alt+Backspace still ends the
// session at once.

use std::time::{Duration, Instant};

use crate::{Server, win::Win};

/// How long the apps have to close before the exit stops.
const GRACE: Duration = Duration::from_secs(8);

impl Server {
    /// The windows an exit waits for: everyone's, the desktop's icons aside.
    fn exit_waits_for(&self) -> Vec<Win> {
        self.space
            .elements()
            .chain(self.shelf.minimized.iter().map(|(w, _)| w))
            .filter(|w| {
                smithay::utils::IsAlive::alive(*w)
                    && !self.is_desktop(w)
                    && self.has_title(w)
                    && !self.belongs_to_parent(w)
            })
            .cloned()
            .collect()
    }

    /// Every app asked to close; the session ends when none is left.
    pub fn begin_exit(&mut self) {
        let windows = self.exit_waits_for();
        if windows.is_empty() {
            return self.loop_signal.stop();
        }
        tracing::info!("exit: asking {} window(s) to close", windows.len());
        // What is running now is what the next session starts again.
        self.session_tick();
        self.resume.exiting = true;
        crate::desktop::STOPPING.store(true, std::sync::atomic::Ordering::Relaxed);
        for w in &windows {
            // Our own windows are told the close is the session's (bus v33):
            // what they keep stays for the next session.
            if let Some(n) = w.native() {
                self.bus_send(
                    n.client,
                    vergence_protocol::bus::ServerMsg::Window(vergence_protocol::window::WinEvent::SessionEnding {
                        window: n.id,
                    }),
                );
            }
            self.close_window(w);
        }
        let started = Instant::now();
        let timer = smithay::reexports::calloop::timer::Timer::from_duration(Duration::from_millis(300));
        let inserted = self.loop_handle.insert_source(timer, move |_, _, state| {
            let left = state.exit_waits_for();
            if left.is_empty() {
                tracing::info!("exit: every app closed; the session ends");
                state.loop_signal.stop();
                return smithay::reexports::calloop::timer::TimeoutAction::Drop;
            }
            if started.elapsed() >= GRACE {
                let names: Vec<String> = left
                    .iter()
                    .map(|w| format!("“{}”", state.window_title(w).trim_end_matches(" •")))
                    .take(3)
                    .collect();
                tracing::info!("exit: stopped; still open: {names:?}");
                state.resume.exiting = false;
                crate::desktop::STOPPING.store(false, std::sync::atomic::Ordering::Relaxed);
                // A restart or shut down waiting for the exit does not happen either.
                state.session_dialog.after_exit = None;
                let req = crate::notify::Request {
                    app: String::new(),
                    replaces: 0,
                    summary: "Vergence is still running".into(),
                    body: format!(
                        "{} did not close (it may be asking about work not saved). Answer it, then exit again.",
                        names.join(", ")
                    ),
                    urgency: 1,
                    timeout_ms: 0,
                    actions: Vec::new(),
                };
                state.notify(crate::notify::Origin::Server, req);
                return smithay::reexports::calloop::timer::TimeoutAction::Drop;
            }
            smithay::reexports::calloop::timer::TimeoutAction::ToDuration(Duration::from_millis(300))
        });
        if let Err(e) = inserted {
            tracing::warn!("exit: {e}; ending now");
            self.loop_signal.stop();
        }
    }
}
