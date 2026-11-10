// Energy: what happens when nobody has touched the machine for a while
// (desktop.toml [energy], the settings app's Energy page, 2026-11-09):
//
//   [energy]
//   screens-off = 15          minutes without input before every monitor is turned off (0: never)
//   sleep = 60                minutes without input before the machine is put to sleep (0: never)
//   sleep-command = "..."     what puts it to sleep; empty: `systemctl suspend` where systemd
//                             runs the machine, else `sudo -n zzz` (FreeBSD, Void: zzz needs root;
//                             a sudoers line `user ALL=(root) NOPASSWD: /usr/bin/zzz` does it)
//
// Idle is counted from the last key, button, pointer or wheel event (input.rs).
// Screens off means each monitor's CRTC is disabled (DPMS off, through the DRM
// compositor's clear), so drawing stops: no VBlank comes, the watchdog keeps
// quiet, the GPU idles. The next input brings every monitor back with a full
// frame; the key or button that woke them is swallowed, pointer motion is not.
// Before the machine sleeps the screens are turned off the same way, so waking
// draws everything afresh. Apps cannot ask to keep the screens on yet: that
// arrives with the session bus's inhibit (whitepaper §6.8).
//
// Both files are re-read within a second of a change (settings.rs), so the
// settings app's choices apply live. Nothing here runs in a nested session.

use std::time::{Duration, Instant};

use smithay::{
    backend::session::Session,
    reexports::calloop::timer::{TimeoutAction, Timer},
};

use crate::state::Server;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Zero: never.
    pub screens_off: Duration,
    pub sleep: Duration,
    /// Empty: the platform's default command.
    pub sleep_command: String,
}

impl Default for Config {
    fn default() -> Self {
        Config { screens_off: Duration::ZERO, sleep: Duration::ZERO, sleep_command: String::new() }
    }
}

pub struct Energy {
    pub config: Config,
    pub last_input: Instant,
    /// Every monitor is off; the next input turns them on.
    pub screens_off: bool,
    /// Sleep was requested in this idle stretch (once per stretch).
    pub slept: bool,
}

impl Energy {
    pub fn new() -> Self {
        Energy { config: load(), last_input: Instant::now(), screens_off: false, slept: false }
    }
}

/// [energy] from desktop.toml (missing file or table: everything off).
pub fn load() -> Config {
    crate::wallpaper::desktop_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse(&t))
        .unwrap_or_default()
}

pub fn parse(text: &str) -> Config {
    let table = text.parse::<toml::Table>().ok().and_then(|t| t.get("energy").and_then(|e| e.as_table()).cloned()).unwrap_or_default();
    let minutes = |k: &str| {
        let v = table.get(k).and_then(|v| v.as_integer().or_else(|| v.as_float().map(|f| f.round() as i64))).unwrap_or(0);
        Duration::from_secs(v.clamp(0, 24 * 60) as u64 * 60)
    };
    Config {
        screens_off: minutes("screens-off"),
        sleep: minutes("sleep"),
        sleep_command: table.get("sleep-command").and_then(|v| v.as_str()).map(|s| s.trim().to_string()).unwrap_or_default(),
    }
}

/// The command that puts this machine to sleep when none is configured.
pub fn default_sleep_command() -> &'static str {
    if std::path::Path::new("/run/systemd/system").is_dir() { "systemctl suspend" } else { "sudo -n zzz" }
}

impl Server {
    /// Check the idle clock every second (the DRM session only).
    pub fn start_energy(&mut self) {
        let res = self.loop_handle.insert_source(Timer::from_duration(Duration::from_secs(1)), |_, _, state| {
            state.energy_tick();
            TimeoutAction::ToDuration(Duration::from_secs(1))
        });
        if let Err(e) = res {
            tracing::warn!("energy: {e}");
        }
        let c = &self.energy.config;
        tracing::info!("energy: screens off after {}, sleep after {} ({})", minutes(c.screens_off), minutes(c.sleep), sleep_command(c));
    }

    /// desktop.toml changed: new times apply from the current idle clock.
    pub fn energy_reload(&mut self) {
        let new = load();
        if new != self.energy.config {
            tracing::info!("energy: screens off after {}, sleep after {} ({})", minutes(new.screens_off), minutes(new.sleep), sleep_command(&new));
            self.energy.config = new;
        }
    }

    /// Input arrived. Returns true when it woke the screens (the caller swallows a
    /// key or button that only did that).
    pub fn energy_activity(&mut self) -> bool {
        self.energy.last_input = Instant::now();
        self.energy.slept = false;
        if !self.energy.screens_off {
            return false;
        }
        self.energy.screens_off = false;
        tracing::info!("energy: input; screens on");
        crate::drm::screens_on(self);
        true
    }

    fn energy_tick(&mut self) {
        if self.backend.is_none() || self.session.as_ref().is_some_and(|s| !s.is_active()) {
            return;
        }
        let idle = self.energy.last_input.elapsed();
        let c = self.energy.config.clone();
        if !c.screens_off.is_zero() && !self.energy.screens_off && idle >= c.screens_off {
            tracing::info!("energy: no input for {}; screens off", minutes(idle));
            self.energy.screens_off = true;
            crate::drm::screens_off(self);
        }
        if !c.sleep.is_zero() && !self.energy.slept && idle >= c.sleep {
            self.energy.slept = true;
            let cmd = sleep_command(&c).to_string();
            tracing::info!("energy: no input for {}; sleep: {cmd}", minutes(idle));
            if !self.energy.screens_off {
                self.energy.screens_off = true;
                crate::drm::screens_off(self);
            }
            // The command may block until the machine wakes (zzz does); its
            // result is only logged, the wake comes through input like any other.
            std::thread::spawn(move || match std::process::Command::new("sh").args(["-c", &cmd]).stdin(std::process::Stdio::null()).output() {
                Ok(o) if o.status.success() => tracing::info!("energy: back from sleep ({cmd})"),
                Ok(o) => tracing::warn!("energy: {cmd}: {} {}", o.status, String::from_utf8_lossy(&o.stderr).trim()),
                Err(e) => tracing::warn!("energy: {cmd}: {e}"),
            });
        }
    }
}

fn sleep_command(c: &Config) -> &str {
    if c.sleep_command.is_empty() { default_sleep_command() } else { &c.sleep_command }
}

fn minutes(d: Duration) -> String {
    if d.is_zero() {
        return "never".into();
    }
    let m = d.as_secs() / 60;
    if m >= 60 && m % 60 == 0 { format!("{} h", m / 60) } else { format!("{m} min") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn energy_table() {
        let c = parse("[energy]\nscreens-off = 15\nsleep = 90\nsleep-command = \" systemctl suspend \"\n");
        assert_eq!(c.screens_off, Duration::from_secs(15 * 60));
        assert_eq!(c.sleep, Duration::from_secs(90 * 60));
        assert_eq!(c.sleep_command, "systemctl suspend");
        assert_eq!(minutes(c.sleep), "90 min");
        assert_eq!(minutes(Duration::from_secs(7200)), "2 h");
    }

    #[test]
    fn missing_or_odd_values_mean_never() {
        assert_eq!(parse(""), Config::default());
        assert_eq!(parse("[wallpaper]\nmode = \"fill\"\n"), Config::default());
        let c = parse("[energy]\nscreens-off = -5\nsleep = 100000\n");
        assert_eq!(c.screens_off, Duration::ZERO);
        assert_eq!(c.sleep, Duration::from_secs(24 * 3600));
        assert_eq!(minutes(Duration::ZERO), "never");
        assert!(!default_sleep_command().is_empty());
    }
}
