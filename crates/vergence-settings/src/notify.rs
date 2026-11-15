// The Notifications page (Dave, 2026-11-14: "Add a notifications item"):
// do not disturb, how long a card stays, which corner the cards stack in,
// and a test card. desktop.toml [notifications]; the server's notify.rs reads
// the same keys within a second of a change.

use super::*;

pub const SECONDS: [(&str, i64); 5] = [("5 seconds", 5), ("10 seconds", 10), ("20 seconds", 20), ("30 seconds", 30), ("1 minute", 60)];
pub const CORNERS: [(&str, &str); 4] = [("Top right", "top-right"), ("Top left", "top-left"), ("Bottom right", "bottom-right"), ("Bottom left", "bottom-left")];

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
            NotifyMsg::Test => {
                let s = table(&self.desktop).and_then(|t| t.get("seconds")).and_then(|v| v.as_integer()).unwrap_or(5);
                send_test(s.clamp(2, 600) as u64);
            }
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

#[derive(Clone, Debug)]
pub enum NotifyMsg {
    DoNotDisturb(bool),
    Seconds(usize),
    Corner(usize),
    Test,
}
