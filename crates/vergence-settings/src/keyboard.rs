//! The Keyboard page (Page::Keyboard): key repeat and spelling
//! (desktop.toml [keyboard], [text]).
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

/// Key repeat as GNOME's defaults have it (the server's too): after 500 ms, 30 a second.
pub(crate) const REPEAT_DELAY: i64 = 500;
pub(crate) const REPEAT_RATE: i64 = 30;

pub(crate) fn keyboard_int(d: &toml::Table, k: &str, default: i64) -> i64 {
    d.get("keyboard").and_then(|a| a.get(k)).and_then(|v| v.as_integer()).unwrap_or(default)
}

impl App {
    fn save_keyboard(&mut self, f: impl FnOnce(&mut toml::Table)) {
        self.save_table("keyboard", f);
    }

    pub(crate) fn keyboard(&self) -> Element<Message> {
        let on = self.desktop.get("keyboard").and_then(|a| a.get("repeat")).and_then(|v| v.as_bool()).unwrap_or(true);
        let value = |s: String| label(s).width(Length::Fixed(72.0)).align(Align::End);
        let delay = row(vec![
            slider(150.0, 1000.0, self.repeat_delay, Message::RepeatDelay)
                .step(10.0)
                .on_release(Message::RepeatDone)
                .width(Length::Fixed(220.0))
                .name("Delay"),
            value(format!("{:.0} ms", self.repeat_delay)),
        ])
        .spacing(8.0)
        .align(Align::Center);
        let rate = row(vec![
            slider(5.0, 60.0, self.repeat_rate, Message::RepeatRate)
                .step(1.0)
                .on_release(Message::RepeatDone)
                .width(Length::Fixed(220.0))
                .name("Speed"),
            value(format!("{:.0} a second", self.repeat_rate)),
        ])
        .spacing(8.0)
        .align(Align::Center);
        let mut rows = vec![setting(
            "Key repeat",
            "Holding a key down types it again and again.",
            switch(on, Message::Repeat).name("Key repeat"),
        )];
        if on {
            rows.push(setting(
                "Delay",
                "How long a key is held before it repeats. Longer: fewer doubled letters.",
                delay,
            ));
            rows.push(setting("Speed", "How fast it repeats once it starts.", rate));
        }
        self.page_layout(
            "Keyboard",
            "Key repeat and spelling for every app. Changes apply at once.",
            vec![
                self.group(Some("Key repeat"), rows),
                self.group(Some("Try it"), vec![setting("Test", "Hold a key down here.", field(&self.key_test, Message::KeyTest).placeholder("Type here").width(Length::Fixed(260.0)).name("Test"))]),
                self.group(Some("Spelling"), vec![setting(
                    "Check spelling",
                    "In every Vergence app: words spelled wrong are underlined in red, and a right click on one gives suggestions. Each app's settings can choose otherwise. American English.",
                    switch(self.desktop.get("text").and_then(|a| a.get("spellcheck")).and_then(|v| v.as_bool()).unwrap_or(true), Message::Spellcheck).name("Check spelling"),
                )]),
                self.advanced("keyboard", self.devices_of(InputKind::Keyboard)),
                self.status_line(),
            ],
        )
    }

    /// This page's messages, as main.rs's handle routes them here.
    pub(crate) fn keyboard_message(&mut self, m: Message) {
        match m {
            Message::Spellcheck(on) => self.save_table("text", |t| {
                t.insert("spellcheck".into(), on.into());
            }),
            Message::Repeat(on) => self.save_keyboard(|t| {
                t.insert("repeat".into(), on.into());
            }),
            Message::RepeatDelay(v) => self.repeat_delay = v,
            Message::RepeatRate(v) => self.repeat_rate = v,
            Message::RepeatDone => {
                let (delay, rate) = (self.repeat_delay.round() as i64, self.repeat_rate.round() as i64);
                self.save_keyboard(|t| {
                    t.insert("repeat-delay".into(), delay.into());
                    t.insert("repeat-rate".into(), rate.into());
                });
            }
            Message::KeyTest(t) => self.key_test = t,
            _ => {}
        }
    }
}
