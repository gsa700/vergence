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
//   [energy.battery]          the same two times while the machine runs on its battery
//   screens-off = 5           (settings milestone 2, 2026-11-14); a key left out, or the
//   sleep = 20                whole table, means the same as on power
//
// On battery or on power comes from vergence-power, read on a thread every 30 s
// (/sys or sysctls, never on the server's frame); the matching pair applies
// from the current idle clock, so pulling the plug can turn the screens off at
// once if the battery's time has already passed.
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

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

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
    /// [energy.battery]: None for a key left out (the same as on power).
    pub battery_screens_off: Option<Duration>,
    pub battery_sleep: Option<Duration>,
    /// [lock] (VDM step 8): lock when the screens go off (default on), after
    /// this long with no input (zero: not on its own), before sleep (default on).
    pub lock_on_screens_off: bool,
    /// After the screens went off by their timeout, how long before the
    /// lock comes on (10 s, 30 s, 1 minute; see docs/apps-roadmap.md): a
    /// touch within it brings them back with no password. A lock asked for,
    /// and the lock before sleep, are immediate whatever this says.
    pub lock_grace: Duration,
    pub lock_after: Duration,
    pub lock_before_sleep: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            screens_off: Duration::ZERO,
            sleep: Duration::ZERO,
            sleep_command: String::new(),
            battery_screens_off: None,
            battery_sleep: None,
            lock_on_screens_off: true,
            lock_grace: Duration::ZERO,
            lock_after: Duration::ZERO,
            lock_before_sleep: true,
        }
    }
}

impl Config {
    /// The screens-off and sleep times that apply on battery or on power.
    pub fn times(&self, on_battery: bool) -> (Duration, Duration) {
        if on_battery {
            (self.battery_screens_off.unwrap_or(self.screens_off), self.battery_sleep.unwrap_or(self.sleep))
        } else {
            (self.screens_off, self.sleep)
        }
    }
}

pub struct Energy {
    pub config: Config,
    pub last_input: Instant,
    /// Every monitor is off; the next input turns them on.
    pub screens_off: bool,
    /// Sleep was requested in this idle stretch (once per stretch).
    pub slept: bool,
    /// On battery, as the power thread last read it.
    pub on_battery: Arc<AtomicBool>,
    /// What energy_tick last used (to log a change once).
    pub was_on_battery: bool,
}

impl Energy {
    pub fn new() -> Self {
        Energy {
            config: load(),
            last_input: Instant::now(),
            screens_off: false,
            slept: false,
            on_battery: Arc::new(AtomicBool::new(false)),
            was_on_battery: false,
        }
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
    let table = text
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("energy").and_then(|e| e.as_table()).cloned())
        .unwrap_or_default();
    let read = |t: &toml::Table, k: &str| {
        t.get(k)
            .and_then(|v| v.as_integer().or_else(|| v.as_float().map(|f| f.round() as i64)))
            .map(|v| Duration::from_secs(v.clamp(0, 24 * 60) as u64 * 60))
    };
    let battery = table.get("battery").and_then(|b| b.as_table()).cloned().unwrap_or_default();
    let lock = text
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("lock").and_then(|e| e.as_table()).cloned())
        .unwrap_or_default();
    let on = |k: &str| lock.get(k).and_then(|v| v.as_bool()).unwrap_or(true);
    Config {
        lock_on_screens_off: on("screens-off"),
        lock_grace: Duration::from_secs(
            lock.get("grace").and_then(|v| v.as_integer()).unwrap_or(0).clamp(0, 300) as u64
        ),
        lock_after: read(&lock, "after").unwrap_or_default(),
        lock_before_sleep: on("before-sleep"),
        screens_off: read(&table, "screens-off").unwrap_or_default(),
        sleep: read(&table, "sleep").unwrap_or_default(),
        sleep_command: table
            .get("sleep-command")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        battery_screens_off: read(&battery, "screens-off"),
        battery_sleep: read(&battery, "sleep"),
    }
}

/// Read battery or mains every 30 s; a machine with no battery is read once.
fn follow_power(flag: Arc<AtomicBool>) {
    loop {
        let p = vergence_power::power();
        flag.store(p.on_battery(), Ordering::Relaxed);
        if p.batteries.is_empty() && p.on_mains.is_none_or(|m| m) {
            return;
        }
        std::thread::sleep(Duration::from_secs(30));
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
        let flag = self.energy.on_battery.clone();
        if let Err(e) = std::thread::Builder::new().name("energy-power".into()).spawn(move || follow_power(flag)) {
            tracing::warn!("energy: can't follow the power: {e}");
        }
        log_config(&self.energy.config);
    }

    /// desktop.toml changed: new times apply from the current idle clock.
    pub fn energy_reload(&mut self) {
        let new = load();
        if new != self.energy.config {
            log_config(&new);
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
        // Only the session on the real monitors: a headless one (tests, its
        // screens in memory) must never turn screens off, lock, or put the
        // machine to sleep (headless milestone 1).
        if !std::env::args().any(|a| a == "--drm") {
            return;
        }
        let idle = self.energy.last_input.elapsed();
        let c = self.energy.config.clone();
        let battery = self.energy.on_battery.load(Ordering::Relaxed);
        if battery != self.energy.was_on_battery {
            self.energy.was_on_battery = battery;
            let (off, sleep) = c.times(battery);
            tracing::info!(
                "energy: on {}; screens off after {}, sleep after {}",
                if battery { "battery" } else { "power" },
                minutes(off),
                minutes(sleep)
            );
        }
        let (screens_off, sleep) = c.times(battery);
        // [lock] after: locked after this long with no input, screens on or off.
        if !c.lock_after.is_zero() && idle >= c.lock_after && !self.lock.on {
            self.lock_for_idle("no input for the lock's time");
        }
        // The grace after the screens went off has passed with no input: locked.
        if c.lock_on_screens_off
            && !c.lock_grace.is_zero()
            && self.energy.screens_off
            && !screens_off.is_zero()
            && idle >= screens_off + c.lock_grace
            && !self.lock.on
        {
            self.lock_for_idle("the screens went off, and the grace passed");
        }
        if !screens_off.is_zero() && !self.energy.screens_off && idle >= screens_off {
            tracing::info!("energy: no input for {}; screens off", minutes(idle));
            // Locked first (VDM milestone 1): the screens come back to the lock;
            // or, with a grace set, once it has passed (below).
            if c.lock_on_screens_off && c.lock_grace.is_zero() {
                self.lock_for_idle("the screens went off");
            }
            self.energy.screens_off = true;
            crate::drm::screens_off(self);
            self.input_screens_off();
        }
        // The login screen does not put the machine to sleep itself (vdm, step 6).
        if !sleep.is_zero() && !self.energy.slept && idle >= sleep && !crate::greeter() {
            if c.lock_before_sleep {
                self.lock_for_idle("sleep");
            }
            self.energy.slept = true;
            let cmd = sleep_command(&c).to_string();
            tracing::info!("energy: no input for {}; sleep: {cmd}", minutes(idle));
            if !self.energy.screens_off {
                self.energy.screens_off = true;
                crate::drm::screens_off(self);
                self.input_screens_off();
            }
            // The command may block until the machine wakes (zzz does); its
            // result is only logged, the wake comes through input like any other.
            // In a session of its own, with no terminal (review C6; elevation
            // review, finding 5: started from the console's session, it could
            // register with vdm as if it were the server).
            std::thread::spawn(move || {
                let mut c = std::process::Command::new("sh");
                c.args(["-c", &cmd]).stdin(std::process::Stdio::null());
                crate::no_terminal(&mut c);
                let res = c.output();
                match res {
                    Ok(o) if o.status.success() => tracing::info!("energy: back from sleep ({cmd})"),
                    Ok(o) => {
                        tracing::warn!("energy: {cmd}: {} {}", o.status, String::from_utf8_lossy(&o.stderr).trim())
                    }
                    Err(e) => tracing::warn!("energy: {cmd}: {e}"),
                }
            });
        }
    }
}

fn log_config(c: &Config) {
    let (boff, bsleep) = c.times(true);
    tracing::info!(
        "energy: on power screens off after {}, sleep after {}; on battery {}, {} ({})",
        minutes(c.screens_off),
        minutes(c.sleep),
        minutes(boff),
        minutes(bsleep),
        sleep_command(c)
    );
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

    #[test]
    fn battery_times_fall_back_to_power() {
        let c = parse("[energy]\nscreens-off = 15\nsleep = 60\n");
        assert_eq!(c.times(true), c.times(false));
        let c = parse("[energy]\nscreens-off = 15\nsleep = 60\n[energy.battery]\nscreens-off = 5\n");
        assert_eq!(c.times(false), (Duration::from_secs(900), Duration::from_secs(3600)));
        assert_eq!(c.times(true), (Duration::from_secs(300), Duration::from_secs(3600)));
        let c = parse("[energy]\nsleep = 60\n[energy.battery]\nsleep = 0\n");
        assert_eq!(c.times(true).1, Duration::ZERO);
    }

    #[test]
    fn on_battery_from_mains_or_discharging() {
        use vergence_power::{Battery, Power, State};
        let bat = |state| Battery { state, ..Default::default() };
        assert!((Power { on_mains: Some(false), batteries: vec![] }).on_battery());
        assert!(!(Power { on_mains: Some(true), batteries: vec![bat(State::Discharging)] }).on_battery());
        assert!((Power { on_mains: None, batteries: vec![bat(State::Discharging)] }).on_battery());
        assert!(!(Power { on_mains: None, batteries: vec![bat(State::Charging)] }).on_battery());
        assert!(!(Power::default()).on_battery());
    }
}
