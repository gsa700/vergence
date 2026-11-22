//! The Mouse, Touchpad and Touchscreen pages (Page::Mouse, Touchpad,
//! Touchscreen; desktop.toml [mouse], [touchpad], [touch]).
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

impl App {
    fn pointer_table(&self, table: &str) -> Option<&toml::Table> {
        self.desktop.get(table).and_then(|v| v.as_table())
    }

    fn pointer_float(&self, table: &'static str, key: &'static str, default: f32) -> f32 {
        if let Some(v) = self.sliding.get(&(table, key)) {
            return *v;
        }
        self.pointer_table(table)
            .and_then(|t| t.get(key))
            .and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64)))
            .map_or(default, |v| v as f32)
    }

    fn pointer_int(&self, table: &'static str, key: &'static str, default: i64) -> i64 {
        if let Some(v) = self.sliding.get(&(table, key)) {
            return v.round() as i64;
        }
        self.pointer_table(table).and_then(|t| t.get(key)).and_then(|v| v.as_integer()).unwrap_or(default)
    }

    fn pointer_flag(&self, table: &str, key: &str, default: bool) -> bool {
        self.pointer_table(table).and_then(|t| t.get(key)).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    fn pointer_word(&self, table: &str, key: &str) -> String {
        self.pointer_table(table).and_then(|t| t.get(key)).and_then(|v| v.as_str()).unwrap_or("").to_string()
    }

    /// A pointer speed slider for desktop.toml's `table` (mouse or touchpad).
    fn speed_row(&self, table: &'static str) -> Element<Message> {
        {
            let v = self.pointer_float(table, "speed", 0.0).clamp(-1.0, 1.0);
            let words = if v < -0.05 {
                "slower"
            } else if v > 0.05 {
                "faster"
            } else {
                "standard"
            };
            row(vec![
                slider(-1.0, 1.0, v, move |v| Message::PointerSlide(table, "speed", v))
                    .step(0.05)
                    .marks(vec![0.0])
                    .on_release(Message::PointerSave(table, "speed"))
                    .width(Length::Fixed(220.0))
                    .name("Pointer speed"),
                label(words).width(Length::Fixed(72.0)).align(Align::End),
            ])
            .spacing(8.0)
            .align(Align::Center)
        }
    }

    /// Lines a notch or a stroke scrolls, for `table`.
    fn lines_row(&self, table: &'static str) -> Element<Message> {
        {
            let n = self.pointer_int(table, "scroll-lines", 3).clamp(1, 10);
            row(vec![
                slider(1.0, 10.0, n as f32, move |v| Message::PointerSlide(table, "scroll-lines", v))
                    .step(1.0)
                    .marks(vec![3.0])
                    .on_release(Message::PointerSave(table, "scroll-lines"))
                    .width(Length::Fixed(220.0))
                    .name("Scrolling speed"),
                label(if n == 1 { "1 line".to_string() } else { format!("{n} lines") })
                    .width(Length::Fixed(72.0))
                    .align(Align::End),
            ])
            .spacing(8.0)
            .align(Align::Center)
        }
    }

    /// The devices of a kind present now, by name, for an input page's Advanced part.
    pub(crate) fn devices_of(&self, kind: InputKind) -> Vec<Element<Message>> {
        let mut names: Vec<&str> = self.inputs.iter().filter(|d| d.kind == kind).map(|d| d.name.as_str()).collect();
        names.dedup();
        if names.is_empty() {
            return vec![setting("None connected now", "", space())];
        }
        names.into_iter().map(|n| setting(&short_name(n, 80), "", space())).collect()
    }

    /// Whether the machine has had a device of this kind (the server remembers).
    pub(crate) fn has_had(&self, kind: InputKind) -> bool {
        self.inputs_seen.contains(&kind) || self.inputs.iter().any(|d| d.kind == kind && kind != InputKind::Touchpad)
    }

    pub(crate) fn mouse_page(&self) -> Element<Message> {
        let dc = self.pointer_int("mouse", "double-click", 400).clamp(200, 900);
        let tested = match self.click_test.1 {
            0 => "Double-click the button to try it.".to_string(),
            1 => "One double-click so far.".to_string(),
            n => format!("{n} double-clicks so far."),
        };
        let mouse = vec![
            setting("Pointer speed", "How far the pointer goes for a move of the hand.", self.speed_row("mouse")),
            setting(
                "Acceleration",
                "On: a fast move takes the pointer farther. Off: always the same distance, for precise work and games.",
                switch(self.pointer_flag("mouse", "acceleration", true), |on| {
                    Message::PointerFlag("mouse", "acceleration", on)
                })
                .name("Acceleration"),
            ),
            setting(
                "Scrolling speed",
                "How many lines a notch of the wheel moves, in every app.",
                self.lines_row("mouse"),
            ),
            setting(
                "Natural scrolling",
                "The content follows the wheel, as on a Mac.",
                switch(self.pointer_flag("mouse", "natural-scroll", false), |on| {
                    Message::PointerFlag("mouse", "natural-scroll", on)
                })
                .name("Natural scrolling (mouse)"),
            ),
            setting(
                "Primary button",
                "Right, for the left hand.",
                choice(
                    vec!["Left".into(), "Right".into()],
                    Some(self.pointer_flag("mouse", "left-handed", false) as usize),
                    Message::PrimaryButton,
                )
                .width(Length::Fixed(120.0))
                .name("Primary button"),
            ),
            setting(
                "Middle click",
                "Pressing both buttons together is a middle click, for mice without one.",
                switch(self.pointer_flag("mouse", "middle-click", false), |on| {
                    Message::PointerFlag("mouse", "middle-click", on)
                })
                .name("Middle click"),
            ),
            setting(
                "Double-click speed",
                &format!("How quickly the two clicks must come: {dc} ms. {tested}"),
                row(vec![
                    slider(200.0, 900.0, dc as f32, |v| Message::PointerSlide("mouse", "double-click", v))
                        .step(10.0)
                        .marks(vec![400.0])
                        .on_release(Message::PointerSave("mouse", "double-click"))
                        .width(Length::Fixed(160.0))
                        .name("Double-click speed"),
                    button(label("Try it")).on_press(Message::ClickTest).name("Double-click test"),
                ])
                .spacing(8.0)
                .align(Align::Center),
            ),
        ];
        self.page_layout(
            "Mouse",
            "How the pointer moves and scrolls. Changes apply at once, to every mouse plugged in.",
            vec![
                self.group(None, mouse),
                self.advanced("mouse", self.devices_of(InputKind::Mouse)),
                self.status_line(),
            ],
        )
    }

    pub(crate) fn touchpad_page(&self) -> Element<Message> {
        let pad_scroll = (self.pointer_word("touchpad", "scroll") == "edge") as usize;
        let pad_click = (self.pointer_word("touchpad", "click") == "fingers") as usize;
        let pad = vec![
            setting("Pointer speed", "", self.speed_row("touchpad")),
            setting(
                "Tap to click",
                "A tap is a click; two fingers, a right click.",
                switch(self.pointer_flag("touchpad", "tap-to-click", true), |on| {
                    Message::PointerFlag("touchpad", "tap-to-click", on)
                })
                .name("Tap to click"),
            ),
            setting(
                "Tap and drag",
                "Tap, then touch again and move: a drag.",
                switch(self.pointer_flag("touchpad", "tap-and-drag", true), |on| {
                    Message::PointerFlag("touchpad", "tap-and-drag", on)
                })
                .name("Tap and drag"),
            ),
            setting(
                "Natural scrolling",
                "The content follows the fingers, as on a phone.",
                switch(self.pointer_flag("touchpad", "natural-scroll", true), |on| {
                    Message::PointerFlag("touchpad", "natural-scroll", on)
                })
                .name("Natural scrolling (touchpad)"),
            ),
            setting(
                "Scrolling",
                "",
                choice(vec!["Two fingers".into(), "Along the edge".into()], Some(pad_scroll), Message::PadScroll)
                    .width(Length::Fixed(160.0))
                    .name("Scrolling"),
            ),
            setting("Scrolling speed", "", self.lines_row("touchpad")),
            setting(
                "Right click",
                "On the pad's bottom right corner, or with two fingers anywhere.",
                choice(vec!["Bottom right corner".into(), "Two fingers".into()], Some(pad_click), Message::PadClick)
                    .width(Length::Fixed(180.0))
                    .name("Right click"),
            ),
            setting(
                "Off while typing",
                "A palm on the pad does nothing while keys are being typed.",
                switch(self.pointer_flag("touchpad", "disable-while-typing", true), |on| {
                    Message::PointerFlag("touchpad", "disable-while-typing", on)
                })
                .name("Off while typing"),
            ),
        ];
        let mut items = vec![self.group(None, pad)];
        if !self.inputs.iter().any(|d| d.kind == InputKind::Touchpad) {
            items.push(label("No touchpad is connected now; these apply when one is.").size(11.0).dim());
        }
        items.push(self.advanced("touchpad", self.devices_of(InputKind::Touchpad)));
        items.push(self.status_line());
        self.page_layout("Touchpad", "Tapping, scrolling and clicking with the pad. Changes apply at once.", items)
    }

    /// Which screen each touch panel is on (touch.rs in the server: a panel
    /// says where on itself it was touched, not which monitor it is glued
    /// to), and what a finger does.
    pub(crate) fn touchscreen_page(&self) -> Element<Message> {
        let panels: Vec<&str> = {
            let mut v: Vec<&str> =
                self.inputs.iter().filter(|d| d.kind == InputKind::Touchscreen).map(|d| d.name.as_str()).collect();
            v.dedup();
            v
        };
        let tied = |name: &str| -> Option<String> {
            self.desktop
                .get("touch")?
                .get("panel")?
                .as_array()?
                .iter()
                .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))?
                .get("screen")?
                .as_str()
                .map(str::to_string)
        };
        let mut items = Vec::new();
        if panels.is_empty() {
            items.push(self.group(
                None,
                vec![setting("No touch screen is connected now", "Its settings come back when it is.", space())],
            ));
        }
        for name in &panels {
            let mut names = vec!["The main screen".to_string()];
            names.extend(self.outputs.iter().map(screen_label));
            let at = tied(name).and_then(|id| self.outputs.iter().position(|o| o.id == id)).map_or(0, |i| i + 1);
            let ids: Vec<String> = self.outputs.iter().map(|o| o.id.clone()).collect();
            let panel_name = name.to_string();
            let control: Element<Message> = if self.outputs.len() > 1 {
                choice(names, Some(at), move |i| {
                    Message::PanelScreen(panel_name.clone(), if i == 0 { None } else { ids.get(i - 1).cloned() })
                })
                .width(Length::Fixed(200.0))
                .name("Screen")
            } else {
                label(names.get(1).cloned().unwrap_or_else(|| names[0].clone())).dim()
            };
            items.push(self.group(
                Some(&short_name(name, 60)),
                vec![setting("On the screen", "The screen this panel is glued to: a touch lands there.", control)],
            ));
        }
        items.push(self.group(
            Some("Fingers"),
            vec![
                setting("Tap", "On a window: what the program makes of it. Elsewhere (the dock, a menu, a title bar, the desktop): a click.", space()),
                setting("Touch and hold", "Half a second still on a dock icon or the desktop: its menu, as the right button.", space()),
                setting(
                    "Size of controls",
                    "A screen with a touch panel shows Large window bars and buttons unless its own setting says otherwise.",
                    button(label("Screens").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(Message::Go(if self.outputs.len() > 1 { Page::Overview } else { Page::Screen(0) })).name("Open Screens"),
                ),
            ],
        ));
        items.push(self.advanced("touchscreen", self.devices_of(InputKind::Touchscreen)));
        items.push(self.status_line());
        self.page_layout("Touchscreen", "Touch panels: which screen each is on, and what a finger does.", items)
    }

    /// This page's messages, as main.rs's handle routes them here.
    pub(crate) fn mouse_message(&mut self, m: Message) {
        match m {
            Message::PanelScreen(name, screen) => self.save(|d| {
                let touch = d.entry("touch").or_insert_with(|| toml::Value::Table(toml::Table::new()));
                let Some(touch) = touch.as_table_mut() else { return };
                let panels = touch.entry("panel").or_insert_with(|| toml::Value::Array(Vec::new()));
                let Some(panels) = panels.as_array_mut() else { return };
                panels.retain(|p| p.get("name").and_then(|n| n.as_str()) != Some(name.as_str()));
                if let Some(id) = screen {
                    let mut t = toml::Table::new();
                    t.insert("name".into(), name.clone().into());
                    t.insert("screen".into(), id.into());
                    panels.push(toml::Value::Table(t));
                }
            }),
            Message::PointerSlide(table, key, v) => {
                self.sliding.insert((table, key), v);
            }
            Message::PointerSave(table, key) => {
                if let Some(v) = self.sliding.remove(&(table, key)) {
                    let value: toml::Value = if key == "speed" {
                        ((v as f64 * 100.0).round() / 100.0).into()
                    } else {
                        (v.round() as i64).into()
                    };
                    self.save_table(table, |t| {
                        t.insert(key.into(), value);
                    });
                }
            }
            Message::PointerFlag(table, key, on) => self.save_table(table, |t| {
                t.insert(key.into(), on.into());
            }),
            Message::PrimaryButton(i) => self.save_table("mouse", |t| {
                t.insert("left-handed".into(), (i == 1).into());
            }),
            Message::PadScroll(i) => self.save_table("touchpad", |t| {
                t.insert("scroll".into(), if i == 1 { "edge" } else { "two-finger" }.into());
            }),
            Message::PadClick(i) => self.save_table("touchpad", |t| {
                t.insert("click".into(), if i == 1 { "fingers" } else { "areas" }.into());
            }),
            Message::ClickTest => {
                let limit = Duration::from_millis(self.pointer_int("mouse", "double-click", 400) as u64);
                let now = Instant::now();
                match self.click_test.0 {
                    Some(at) if now.duration_since(at) <= limit => {
                        self.click_test = (None, self.click_test.1 + 1);
                    }
                    _ => self.click_test.0 = Some(now),
                }
            }
            _ => {}
        }
    }
}
