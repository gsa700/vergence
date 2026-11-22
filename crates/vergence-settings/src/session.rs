//! The Session page (Page::Session): what comes back when Vergence starts
//! (desktop.toml [session]); keep.rs keeps the settings app's own page.
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

/// Settings > Session's "Remember the page after closing": the names
/// shown and the minutes (`[session] settings-keep`; -1 until the next session).
const KEEP_CHOICES: [(&str, i64); 5] = [
    ("Never", 0),
    ("10 minutes", 10),
    ("30 minutes", 30),
    ("1 hour", 60),
    ("Until the next session", vergence_widgets::session::UNTIL_NEXT_SESSION),
];

impl App {
    /// The Session page (session milestone 1; Dave, 2026-11-21): desktop.toml
    /// `[session]` restore (moved here from the Dock page), places and
    /// settings-keep. Window places themselves are on the Windows page.
    pub(crate) fn session_page(&self) -> Element<Message> {
        let s = self.desktop.get("session");
        let restore = s.and_then(|d| d.get("restore")).and_then(|v| v.as_bool()).unwrap_or(true);
        let places = s.and_then(|d| d.get("places")).and_then(|v| v.as_bool()).unwrap_or(true);
        let keep = s
            .and_then(|d| d.get("settings-keep"))
            .and_then(|v| v.as_integer())
            .unwrap_or(vergence_widgets::session::SETTINGS_KEEP);
        let keep_at = KEEP_CHOICES.iter().position(|(_, m)| *m == keep);
        let mut start = vec![setting(
            "When Vergence starts",
            "Reopen the apps that were open when the last session ended, with their windows and documents, or start with none open. Apps then also forget the windows they kept.",
            choice(
                vec!["Reopen the apps that were open".into(), "Start with no apps open".into()],
                Some(if restore { 0 } else { 1 }),
                Message::Restore,
            )
            .width(Length::Fixed(260.0))
            .name("When Vergence starts"),
        )];
        if restore {
            start.push(setting(
                "Return to where I was in documents",
                "After a restart, a logout or a power cut, documents come back scrolled to where they were. A document you close yourself opens at its start next time.",
                switch(places, Message::SessionPlaces).name("Return to where I was in documents"),
            ));
        }
        let keep_names: Vec<String> = KEEP_CHOICES.iter().map(|(n, _)| n.to_string()).collect();
        self.page_layout(
            "Session",
            "What comes back when Vergence starts, and when you open an app again.",
            vec![
                self.group(Some("Starting"), start),
                self.group(Some("Settings"), vec![setting(
                    "Remember the page after closing",
                    "Settings opens again on the page and at the place you left, for this long. A new session always starts at the first page.",
                    choice(keep_names, keep_at, Message::SettingsKeep).width(Length::Fixed(220.0)).name("Remember the page after closing"),
                )]),
                self.group(Some("Windows"), vec![setting(
                    "Window places",
                    "Where each app's windows open is remembered for each set of screens. They can be forgotten on the Windows page.",
                    button(label("Windows")).on_press(Message::Go(Page::Windows)).name("Open the Windows page"),
                )]),
            ],
        )
    }

    /// This page's messages, as main.rs's handle routes them here.
    pub(crate) fn session_message(&mut self, m: Message) {
        match m {
            Message::SessionPlaces(on) => self.save(|d| {
                let t = d.entry("session").or_insert_with(|| toml::Value::Table(toml::Table::new()));
                if let Some(t) = t.as_table_mut() {
                    t.insert("places".into(), on.into());
                }
            }),
            Message::SettingsKeep(i) => self.save(|d| {
                let t = d.entry("session").or_insert_with(|| toml::Value::Table(toml::Table::new()));
                if let Some(t) = t.as_table_mut() {
                    t.insert("settings-keep".into(), KEEP_CHOICES[i.min(KEEP_CHOICES.len() - 1)].1.into());
                }
            }),
            Message::Restore(i) => self.save(|d| {
                let t = d.entry("session").or_insert_with(|| toml::Value::Table(toml::Table::new()));
                if let Some(t) = t.as_table_mut() {
                    t.insert("restore".into(), (i == 0).into());
                }
            }),
            _ => {}
        }
    }
}
