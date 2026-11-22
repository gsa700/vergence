//! The Energy page (Page::Energy): screens off and sleep after idle, on
//! power and on battery, and the sleep command (desktop.toml [energy]).
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

// ---- Energy: desktop.toml [energy] (the server's energy.rs reads the same keys) ----
const OFF_CHOICES: [(&str, i64); 11] = [
    ("Never", 0),
    ("1 minute", 1),
    ("2 minutes", 2),
    ("5 minutes", 5),
    ("10 minutes", 10),
    ("15 minutes", 15),
    ("20 minutes", 20),
    ("30 minutes", 30),
    ("45 minutes", 45),
    ("1 hour", 60),
    ("2 hours", 120),
];
const SLEEP_CHOICES: [(&str, i64); 9] = [
    ("Never", 0),
    ("10 minutes", 10),
    ("15 minutes", 15),
    ("20 minutes", 20),
    ("30 minutes", 30),
    ("45 minutes", 45),
    ("1 hour", 60),
    ("2 hours", 120),
    ("3 hours", 180),
];

fn energy_int(d: &toml::Table, k: &str, default: i64) -> i64 {
    d.get("energy").and_then(|a| a.get(k)).and_then(|v| v.as_integer()).unwrap_or(default)
}

/// [energy.battery]'s value, else the on-power one (absent means the same).
fn energy_battery_int(d: &toml::Table, k: &str) -> i64 {
    d.get("energy")
        .and_then(|a| a.get("battery"))
        .and_then(|b| b.get(k))
        .and_then(|v| v.as_integer())
        .unwrap_or_else(|| energy_int(d, k, 0))
}

pub(crate) fn energy_cmd(d: &toml::Table) -> String {
    d.get("energy").and_then(|a| a.get("sleep-command")).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// The labels for a minutes setting, and the current one; a value set by hand
/// that is not among the choices is shown as "N minutes" and kept.
fn minute_choices(choices: &[(&str, i64)], current: i64) -> (Vec<String>, String) {
    let mut list: Vec<String> = choices.iter().map(|(l, _)| l.to_string()).collect();
    let label = match choices.iter().find(|(_, m)| *m == current) {
        Some((l, _)) => l.to_string(),
        None => {
            let l = format!("{current} minutes");
            list.push(l.clone());
            l
        }
    };
    (list, label)
}

fn minutes_of(choices: &[(&str, i64)], label: &str) -> i64 {
    choices
        .iter()
        .find(|(l, _)| *l == label)
        .map(|(_, m)| *m)
        .or_else(|| label.split_whitespace().next().and_then(|n| n.parse().ok()))
        .unwrap_or(0)
}

impl App {
    /// One key of [energy.battery]; the other stays absent (the same as on
    /// power) until it is chosen too.
    fn save_energy_battery(&mut self, key: &str, minutes: i64) {
        self.save_table("energy", |t| {
            if !t.get("battery").is_some_and(|b| b.is_table()) {
                t.insert("battery".into(), toml::Value::Table(toml::Table::new()));
            }
            if let Some(b) = t.get_mut("battery").and_then(|b| b.as_table_mut()) {
                b.insert(key.into(), minutes.into());
            }
        })
    }

    pub(crate) fn energy(&self) -> Element<Message> {
        let off = energy_int(&self.desktop, "screens-off", 0);
        let sleep = energy_int(&self.desktop, "sleep", 0);
        let (off_list, off_now) = minute_choices(&OFF_CHOICES, off);
        let (sleep_list, sleep_now) = minute_choices(&SLEEP_CHOICES, sleep);
        let default_cmd = if Path::new("/run/systemd/system").is_dir() { "systemctl suspend" } else { "sudo -n zzz" };
        let detail = format!("Empty: {default_cmd}. zzz needs root: a sudoers line for it, without a password.");
        let minutes = |list: Vec<String>, now: &str, f: fn(String) -> Message, name: &str| {
            let at = list.iter().position(|l| l == now);
            let options = list.clone();
            choice(options, at, move |i| f(list[i].clone())).width(Length::Fixed(160.0)).name(name)
        };
        let mut groups = if self.has_battery() {
            // Two pairs where there is a battery; the one in use says so.
            let on_battery = self.battery.power.on_battery();
            let (boff_list, boff_now) = minute_choices(&OFF_CHOICES, energy_battery_int(&self.desktop, "screens-off"));
            let (bsleep_list, bsleep_now) = minute_choices(&SLEEP_CHOICES, energy_battery_int(&self.desktop, "sleep"));
            vec![
                self.group(
                    Some(if on_battery { "On battery (now)" } else { "On battery" }),
                    vec![
                        setting(
                            "Turn the screens off",
                            "After no input. Every monitor is switched off; apps keep running.",
                            minutes(
                                boff_list,
                                &boff_now,
                                Message::BatteryScreensOff,
                                "On battery: turn the screens off",
                            ),
                        ),
                        setting(
                            "Put the machine to sleep",
                            "Suspend to memory. The screens go off first.",
                            minutes(
                                bsleep_list,
                                &bsleep_now,
                                Message::BatterySleepAfter,
                                "On battery: put the machine to sleep",
                            ),
                        ),
                    ],
                ),
                self.group(
                    Some(if on_battery { "On power" } else { "On power (now)" }),
                    vec![
                        setting(
                            "Turn the screens off",
                            "After no input, plugged in.",
                            minutes(off_list, &off_now, Message::ScreensOff, "On power: turn the screens off"),
                        ),
                        setting(
                            "Put the machine to sleep",
                            "Suspend to memory. The screens go off first.",
                            minutes(sleep_list, &sleep_now, Message::SleepAfter, "On power: put the machine to sleep"),
                        ),
                    ],
                ),
            ]
        } else {
            vec![self.group(
                Some("After no input"),
                vec![
                    setting(
                        "Turn the screens off",
                        "Every monitor is switched off; apps keep running.",
                        minutes(off_list, &off_now, Message::ScreensOff, "Turn the screens off"),
                    ),
                    setting(
                        "Put the machine to sleep",
                        "Suspend to memory. The screens go off first.",
                        minutes(sleep_list, &sleep_now, Message::SleepAfter, "Put the machine to sleep"),
                    ),
                ],
            )]
        };
        self.page_layout(
            "Energy",
            "When nothing has been touched for a while: the screens go dark, then the machine sleeps. Any key or a nudge of the mouse brings everything back.",
            {
                groups.extend([
                self.group(Some("How it sleeps"), vec![setting(
                    "Sleep command",
                    &detail,
                    field(&self.sleep_cmd, Message::SleepCommand).placeholder(default_cmd).on_submit(Message::SleepCommandSubmit).width(Length::Fixed(260.0)).name("Sleep command"),
                )]),
                self.group(None, vec![setting("Apps", "Apps cannot yet ask to keep the screens on while they play or present; that comes with the session bus.", space())]),
                self.status_line(),
                ]);
                groups
            },
        )
    }

    /// This page's messages, as main.rs's handle routes them here.
    pub(crate) fn energy_message(&mut self, m: Message) {
        match m {
            Message::ScreensOff(label) => {
                let m = minutes_of(&OFF_CHOICES, &label);
                self.save_table("energy", |t| {
                    t.insert("screens-off".into(), m.into());
                })
            }
            Message::SleepAfter(label) => {
                let m = minutes_of(&SLEEP_CHOICES, &label);
                self.save_table("energy", |t| {
                    t.insert("sleep".into(), m.into());
                })
            }
            Message::BatteryScreensOff(label) => {
                let m = minutes_of(&OFF_CHOICES, &label);
                self.save_energy_battery("screens-off", m)
            }
            Message::BatterySleepAfter(label) => {
                let m = minutes_of(&SLEEP_CHOICES, &label);
                self.save_energy_battery("sleep", m)
            }
            Message::SleepCommand(s) => self.sleep_cmd = s,
            Message::SleepCommandSubmit => {
                let v = self.sleep_cmd.trim().to_string();
                self.save_table("energy", |t| {
                    t.insert("sleep-command".into(), v.into());
                })
            }
            _ => {}
        }
    }
}
