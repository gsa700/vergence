// Privacy > Agents (meaning milestone 1, step 4, part 2): whether agents
// may ask to use apps at all (desktop.toml [agents] enabled, on by default),
// how long a timed grant lasts ([agents] timed-minutes), and every grant
// the person gave, read from the server over the bus (`ListGrants`), each
// with Take back, and Take back all (`RevokeGrants`). Taking back asks
// nothing first: the agent only asks again next time.

use super::*;

/// The lengths a timed grant can have, with their names.
const TIMED_CHOICES: [(&str, i64); 7] = [
    ("15 minutes", 15),
    ("30 minutes", 30),
    ("An hour", 60),
    ("2 hours", 120),
    ("4 hours", 240),
    ("8 hours", 480),
    ("A day", 1440),
];

/// desktop.toml's `[agents]`: on, and the timed length in minutes.
pub fn agents_settings(d: &toml::Table) -> (bool, i64) {
    let t = d.get("agents").and_then(|a| a.as_table());
    let on = t.and_then(|t| t.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(true);
    let minutes = t.and_then(|t| t.get("timed-minutes")).and_then(|v| v.as_integer()).unwrap_or(60);
    (on, minutes)
}

/// The grants, from the server, mailed back.
pub fn fetch(mail: Mail) {
    std::thread::spawn(move || {
        let Ok(mut bus) = vergence_protocol::Client::connect("vergence-settings") else { return };
        if let Ok(ServerMsg::Grants { grants, .. }) = bus.request(|serial| ClientMsg::ListGrants { serial }) {
            mail.send(Message::Privacy(PrivacyMsg::List(grants)));
        }
    });
}

/// Take grants back (an agent's for an app; all when everything is empty), then read again.
fn revoke(mail: Mail, agent: String, program: String, app: String) {
    std::thread::spawn(move || {
        let Ok(mut bus) = vergence_protocol::Client::connect("vergence-settings") else { return };
        let _ = bus.request(|serial| ClientMsg::RevokeGrants {
            serial,
            agent: agent.clone(),
            program: program.clone(),
            app: app.clone(),
        });
        if let Ok(ServerMsg::Grants { grants, .. }) = bus.request(|serial| ClientMsg::ListGrants { serial }) {
            mail.send(Message::Privacy(PrivacyMsg::List(grants)));
        }
    });
}

#[derive(Debug, Clone)]
pub enum PrivacyMsg {
    List(Vec<vergence_protocol::bus::Grant>),
    /// Take back: agent, its program and app (all empty for all).
    Revoke(String, String, String),
    Enabled(bool),
    Timed(i64),
}

/// How a grant reads: "For 42 more minutes", "Until you log out".
fn span_words(g: &vergence_protocol::bus::Grant) -> String {
    let base = match g.span.as_str() {
        "timed" if g.minutes_left == 1 => "For 1 more minute".to_string(),
        "timed" => format!("For {} more minutes", g.minutes_left),
        "session" => "Until you log out".into(),
        "always" => "Always".into(),
        "never" => "Never: it is not asked about".into(),
        _ => "Just now, while it stays connected".into(),
    };
    if g.working { format!("{base}. Working now.") } else { format!("{base}.") }
}

impl App {
    pub fn privacy_message(&mut self, m: PrivacyMsg) {
        match m {
            PrivacyMsg::List(g) => self.grants = Some(g),
            PrivacyMsg::Revoke(agent, program, app) => revoke(self.mail.clone(), agent, program, app),
            PrivacyMsg::Enabled(on) => self.save_table("agents", |t| {
                t.insert("enabled".into(), on.into());
            }),
            PrivacyMsg::Timed(m) => self.save_table("agents", |t| {
                t.insert("timed-minutes".into(), m.into());
            }),
        }
    }

    pub fn agents_page(&self) -> Element<Message> {
        let (on, minutes) = agents_settings(&self.desktop);
        let names: Vec<String> = TIMED_CHOICES.iter().map(|(n, _)| n.to_string()).collect();
        let at = TIMED_CHOICES.iter().position(|(_, m)| *m == minutes);
        let grants_group = match &self.grants {
            None => self.group(Some("What agents may use"), vec![setting("Reading the list...", "", space())]),
            Some(list) if list.is_empty() => self.group(
                Some("What agents may use"),
                vec![setting(
                    "Nothing yet",
                    "An agent asks the first time it wants to use an app; what you answer is kept here.",
                    space(),
                )],
            ),
            Some(list) => {
                let mut rows: Vec<Element<Message>> = list
                    .iter()
                    .map(|g| {
                        let title = format!("\u{201c}{}\u{201d} and {}", g.agent, g.shown);
                        let control = button(label("Take back"))
                            .on_press(Message::Privacy(PrivacyMsg::Revoke(
                                g.agent.clone(),
                                g.program.clone(),
                                g.app.clone(),
                            )))
                            .name(format!("Take back {}'s use of {}", g.agent, g.shown));
                        let detail = format!(
                            "{} Program: {}.",
                            span_words(g),
                            if g.program.is_empty() { "not known" } else { &g.program }
                        );
                        setting(&title, &detail, control)
                    })
                    .collect();
                rows.push(setting(
                    "Every agent",
                    "Each asks again the next time it wants an app.",
                    button(label("Take back all"))
                        .on_press(Message::Privacy(PrivacyMsg::Revoke(String::new(), String::new(), String::new())))
                        .name("Take back every agent's use of every app"),
                ));
                self.group(Some("What agents may use"), rows)
            }
        };
        self.page_layout(
            "Agents",
            "An agent (an assistant working for you) can read your apps' windows and press, type and choose in them, as you would. It asks you first for each app, in a question the desktop draws, and while it works a tile says so and a ring shows where it acts. It never sees or types into a secret field. Everything it does is logged in ~/.local/state/vergence/agents.log.",
            vec![
                self.group(Some("Agents"), vec![
                    setting("Agents may ask", if on { "When one wants an app you are asked; nothing is allowed until you say." } else { "Off: no agent can read or use anything, whatever you allowed before." }, switch(on, |on| Message::Privacy(PrivacyMsg::Enabled(on))).name("Agents may ask")),
                    setting("A timed answer lasts", "How long the timed answer in an agent's question lasts.", choice(names.clone(), at, move |i| Message::Privacy(PrivacyMsg::Timed(TIMED_CHOICES[i].1))).width(Length::Fixed(160.0)).name("A timed answer lasts")),
                ]),
                grants_group,
            ],
        )
    }
}
