// The Notifications page (Dave, 2026-11-14: "Add a notifications item"):
// do not disturb, how long a card stays, which corner the cards stack in,
// and a test card. desktop.toml [notifications]; the server's notify.rs reads
// the same keys within a second of a change.

use super::*;

pub const SECONDS: [(&str, i64); 5] =
    [("5 seconds", 5), ("10 seconds", 10), ("20 seconds", 20), ("30 seconds", 30), ("1 minute", 60)];
pub const CORNERS: [(&str, &str); 4] = [
    ("Top right", "top-right"),
    ("Top left", "top-left"),
    ("Bottom right", "bottom-right"),
    ("Bottom left", "bottom-left"),
];

/// The apps the server has seen send cards (its notify-apps.toml), newest first.
pub fn read_apps() -> Vec<(String, u64)> {
    let Some(state) = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
    else {
        return Vec::new();
    };
    let text = std::fs::read_to_string(state.join("vergence/notify-apps.toml")).unwrap_or_default();
    parse_apps(&text)
}

fn parse_apps(text: &str) -> Vec<(String, u64)> {
    let mut apps: Vec<(String, u64)> = text
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("apps").and_then(|a| a.as_table()).cloned())
        .map(|a| a.into_iter().filter_map(|(k, v)| v.as_integer().map(|t| (k, t.max(0) as u64))).collect())
        .unwrap_or_default();
    apps.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    apps
}

/// The apps switched off, in lower case.
fn off_list(d: &toml::Table) -> Vec<String> {
    table(d)
        .and_then(|t| t.get("off"))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).map(|s| s.trim().to_lowercase()).collect())
        .unwrap_or_default()
}

fn table(d: &toml::Table) -> Option<&toml::Table> {
    d.get("notifications").and_then(|n| n.as_table())
}

/// Send a card over the session bus, as any app would, on a thread of its
/// own (connecting must not hold up the window), staying connected while
/// the card shows, as an app that wants to hear of a click would.
fn send_test(seconds: u64) {
    std::thread::spawn(move || {
        let Ok(mut bus) = vergence_protocol::Client::connect("vergence-settings") else { return };
        let sent = bus.request(|serial| ClientMsg::Notify {
            serial,
            replaces: 0,
            summary: "A test notification".into(),
            body: "This is how a card from an app looks, and where it shows.".into(),
            urgency: 1,
            timeout_ms: -1,
            actions: Vec::new(),
        });
        if sent.is_ok() {
            std::thread::sleep(Duration::from_secs(seconds + 2));
        }
    });
}

impl App {
    pub fn notify_message(&mut self, m: NotifyMsg) {
        match m {
            NotifyMsg::DoNotDisturb(on) => self.save_table("notifications", |t| {
                t.insert("do-not-disturb".into(), on.into());
            }),
            NotifyMsg::Seconds(i) => {
                let s = SECONDS[i.min(SECONDS.len() - 1)].1;
                self.save_table("notifications", |t| {
                    t.insert("seconds".into(), s.into());
                })
            }
            NotifyMsg::Corner(i) => {
                let c = CORNERS[i.min(CORNERS.len() - 1)].1;
                self.save_table("notifications", |t| {
                    t.insert("corner".into(), c.into());
                })
            }
            NotifyMsg::App(app, on) => {
                let key = app.to_lowercase();
                let mut off = off_list(&self.desktop);
                off.retain(|a| *a != key);
                if !on {
                    off.push(key);
                }
                off.sort();
                self.save_table("notifications", |t| {
                    if off.is_empty() {
                        t.remove("off");
                    } else {
                        t.insert("off".into(), toml::Value::Array(off.into_iter().map(toml::Value::String).collect()));
                    }
                })
            }
            NotifyMsg::Test => {
                let s = table(&self.desktop).and_then(|t| t.get("seconds")).and_then(|v| v.as_integer()).unwrap_or(5);
                send_test(s.clamp(2, 600) as u64);
                // The settings app joins the list the first time; read it again
                // once the server has written it down.
                let mail = self.mail.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(600));
                    mail.send(Message::Notify(NotifyMsg::Reread));
                });
            }
            NotifyMsg::Reread => self.notify_apps = read_apps(),
        }
    }

    pub fn notifications_page(&self) -> Element<Message> {
        let t = table(&self.desktop);
        let dnd = t.and_then(|t| t.get("do-not-disturb")).and_then(|v| v.as_bool()).unwrap_or(false);
        let secs = t.and_then(|t| t.get("seconds")).and_then(|v| v.as_integer()).unwrap_or(5);
        let corner = t.and_then(|t| t.get("corner")).and_then(|v| v.as_str()).unwrap_or("top-right");
        let mut sec_labels: Vec<String> = SECONDS.iter().map(|(l, _)| l.to_string()).collect();
        let sec_at = match SECONDS.iter().position(|(_, s)| *s == secs) {
            Some(i) => i,
            None => {
                // A value set by hand: shown as it is, kept until another is picked.
                sec_labels.push(format!("{secs} seconds"));
                sec_labels.len() - 1
            }
        };
        let corner_at = CORNERS.iter().position(|(_, c)| *c == corner).unwrap_or(0);
        self.page_layout(
            "Notifications",
            "Cards apps show for a moment: a download done, a message come in. A click on a card does what it offers; resting the pointer on one keeps it.",
            vec![
                self.group(None, vec![setting(
                    "Do not disturb",
                    "Apps' cards are not shown. Urgent ones still are, and so is the volume level.",
                    switch(dnd, |on| Message::Notify(NotifyMsg::DoNotDisturb(on))).name("Do not disturb"),
                )]),
                self.group(Some("Cards"), vec![
                    setting(
                        "Show each card for",
                        "When the app leaves it to the desktop. A card with buttons stays at least 10 seconds; urgent ones stay until clicked.",
                        choice(sec_labels, Some(sec_at), |i| Message::Notify(NotifyMsg::Seconds(i))).width(Length::Fixed(160.0)).name("Show each card for"),
                    ),
                    setting(
                        "Where they show",
                        "On the screen the pointer is on, beside the dock. The newest card is nearest the corner.",
                        choice(CORNERS.iter().map(|(l, _)| l.to_string()).collect(), Some(corner_at), |i| Message::Notify(NotifyMsg::Corner(i))).width(Length::Fixed(160.0)).name("Where they show"),
                    ),
                ]),
                self.notify_apps_group(),
                self.group(Some("Try it"), vec![setting(
                    "A test card",
                    if dnd { "Do not disturb is on: the card is held back like any app's." } else { "Shows one now, as an app would." },
                    button(label("Show a test card")).on_press(Message::Notify(NotifyMsg::Test)).name("Show a test card"),
                )]),
                self.status_line(),
            ],
        )
    }
}

impl App {
    /// Each app that has sent a card, with a switch; the name it gives, or its
    /// application entry's name where one matches.
    fn notify_apps_group(&self) -> Element<Message> {
        let off = off_list(&self.desktop);
        if self.notify_apps.is_empty() {
            return self.group(
                Some("Apps"),
                vec![setting(
                    "No app has shown a card yet",
                    "Each app that does is listed here, to turn its cards off.",
                    space(),
                )],
            );
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let rows = self
            .notify_apps
            .iter()
            .map(|(app, t)| {
                let on = !off.contains(&app.to_lowercase());
                let name = self.notify_app_name(app);
                let detail = if on {
                    format!("Last card {}.", ago(now.saturating_sub(*t)))
                } else {
                    "Its cards are not shown.".to_string()
                };
                let app = app.clone();
                setting(
                    &name,
                    &detail,
                    switch(on, move |on| Message::Notify(NotifyMsg::App(app.clone(), on)))
                        .name(&format!("Cards from {name}")),
                )
            })
            .collect();
        self.group(Some("Apps"), rows)
    }

    /// An app's name for people: its application entry's, matched by id or by
    /// name in any case, else as the app gave it.
    fn notify_app_name(&self, app: &str) -> String {
        let low = app.to_lowercase();
        self.apps
            .iter()
            .find(|e| {
                e.id.to_lowercase() == low
                    || e.name.to_lowercase() == low
                    || e.id.to_lowercase().trim_end_matches(".desktop") == low
            })
            .map(|e| e.name.clone())
            .unwrap_or_else(|| app.to_string())
    }
}

/// How long ago, in words ("just now", "5 minutes ago", "3 days ago"); the
/// server writes the time down at most once an hour per app.
fn ago(secs: u64) -> String {
    match secs {
        0..3600 => "within the hour".into(),
        3600..86400 => {
            let h = secs / 3600;
            if h == 1 { "an hour ago".into() } else { format!("{h} hours ago") }
        }
        _ => {
            let d = secs / 86400;
            if d == 1 { "yesterday".into() } else { format!("{d} days ago") }
        }
    }
}

#[derive(Clone, Debug)]
pub enum NotifyMsg {
    DoNotDisturb(bool),
    Seconds(usize),
    Corner(usize),
    /// An app's cards on or off.
    App(String, bool),
    Test,
    /// The apps list read again.
    Reread,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_newest_first() {
        let a = parse_apps("[apps]\nFirefox = 100\nnotify-send = 300\nvergence-settings = 200\n");
        assert_eq!(
            a.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["notify-send", "vergence-settings", "Firefox"]
        );
        assert!(parse_apps("").is_empty());
        assert_eq!(ago(10), "within the hour");
        assert_eq!(ago(7200), "2 hours ago");
        assert_eq!(ago(90000), "yesterday");
    }
}
