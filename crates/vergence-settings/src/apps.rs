// The Apps page (app settings milestone 1): every app the desktop knows of
// (ours, those with window places, those with a table of their own in
// desktop.toml), and each one's page. An app of ours has a settings window
// of its own (Ctrl+, in it, or Settings… in its app button), so its page
// points there; an app that is not ours (Firefox) gets here what the desktop
// does with it: its title bar's light or dark (desktop.toml
// `[app."<id>"] bar-style`, which the server reads) and its window places.
// The app menu's Settings… for an app not ours starts this app with
// `--app <id>`.

use super::*;

/// Our own apps: they have settings windows of their own.
const OURS: [(&str, &str); 2] = [("vergence-term", "Terminal"), ("vergence-settings", "Settings")];

#[derive(Debug, Clone)]
pub enum AppsMsg {
    Open(String),
    /// An app's title bar: Some("dark"/"light"), or None for the desktop's.
    Bar(String, Option<String>),
    /// Our Open/Save dialog for apps not ours (desktop.toml `[apps]
    /// own-file-dialog`, read by the server at the start of a session).
    OwnDialog(bool),
    Back,
}

impl App {
    /// The apps to list, by the id desktop.toml and window memory know them
    /// by (lower case), ours first, then by name.
    pub fn apps_known(&self) -> Vec<String> {
        let mut ids: Vec<String> = OURS.iter().map(|(id, _)| id.to_string()).collect();
        let mut others: Vec<String> = Vec::new();
        for (app, _) in self.places.iter().flatten() {
            others.push(app.to_lowercase());
        }
        if let Some(t) = self.desktop.get("app").and_then(|a| a.as_table()) {
            others.extend(t.keys().map(|k| k.to_lowercase()));
        }
        if let Some(a) = &self.app_open {
            others.push(a.clone());
        }
        others.retain(|a| !a.is_empty() && !ids.contains(a));
        others.sort_by_key(|a| self.known_app_name(a).to_lowercase());
        others.dedup();
        ids.extend(others);
        ids
    }

    pub fn known_app_name(&self, id: &str) -> String {
        OURS.iter().find(|(o, _)| *o == id).map(|(_, n)| n.to_string()).unwrap_or_else(|| self.places_name(id))
    }

    fn app_bar(&self, id: &str) -> Option<String> {
        self.desktop
            .get("app")
            .and_then(|a| a.get(id))
            .and_then(|t| t.get("bar-style"))
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    pub fn apps_message(&mut self, m: AppsMsg) {
        match m {
            AppsMsg::Open(id) => {
                self.app_open = Some(id);
                self.open(Page::AppPage);
            }
            AppsMsg::Back => self.open(Page::Apps),
            AppsMsg::OwnDialog(on) => self.save(|d| {
                let apps = d.entry("apps").or_insert_with(|| toml::Value::Table(toml::Table::new()));
                if let Some(t) = apps.as_table_mut() {
                    t.insert("own-file-dialog".into(), on.into());
                }
            }),
            AppsMsg::Bar(id, value) => self.save(|d| {
                let apps = d.entry("app").or_insert_with(|| toml::Value::Table(toml::Table::new()));
                if let Some(apps) = apps.as_table_mut() {
                    match value {
                        Some(v) => {
                            let t = apps.entry(id.clone()).or_insert_with(|| toml::Value::Table(toml::Table::new()));
                            if let Some(t) = t.as_table_mut() {
                                t.insert("bar-style".into(), v.into());
                            }
                        }
                        None => {
                            if let Some(t) = apps.get_mut(&id).and_then(|t| t.as_table_mut()) {
                                t.remove("bar-style");
                                if t.is_empty() {
                                    apps.remove(&id);
                                }
                            }
                        }
                    }
                    if apps.is_empty() {
                        d.remove("app");
                    }
                }
            }),
        }
    }

    pub fn apps_page(&self) -> Element<Message> {
        let rows: Vec<Element<Message>> = self
            .apps_known()
            .into_iter()
            .map(|id| {
                let name = self.known_app_name(&id);
                let what = if OURS.iter().any(|(o, _)| *o == id) {
                    "Settings of its own: Ctrl+, in it"
                } else {
                    "Title bar, window places"
                };
                setting(
                    &name,
                    what,
                    button(label("Open").size(12.0))
                        .padding(Padding::hv(4.0, 12.0))
                        .on_press(Message::Apps(AppsMsg::Open(id.clone())))
                        .name(format!("Open {name}")),
                )
            })
            .collect();
        let own =
            self.desktop.get("apps").and_then(|a| a.get("own-file-dialog")).and_then(|v| v.as_bool()).unwrap_or(false);
        let all = self.group(
            Some("Every app"),
            vec![setting(
                "Vergence's file dialog in other apps",
                "Firefox, GTK and Qt apps open and save files with the desktop's dialog. From the next login; Firefox also needs widget.use-xdg-desktop-portal.file-picker = 1 in about:config.",
                switch(own, |on| Message::Apps(AppsMsg::OwnDialog(on))).name("Vergence's file dialog in other apps"),
            )],
        );
        self.page_layout("Apps", "What the desktop does with each app: its title bar and where its windows go. An app's own settings are in the app.", vec![all, self.group(Some("Apps"), rows)])
    }

    pub fn app_page(&self) -> Element<Message> {
        let Some(id) = self.app_open.clone() else { return self.apps_page() };
        let name = self.known_app_name(&id);
        let back = row(vec![
            button(row(vec![label("‹").size(14.0), label("Apps").size(12.0)]).spacing(4.0).align(Align::Center))
                .look(ButtonLook::Plain)
                .padding(Padding::hv(2.0, 4.0))
                .on_press(Message::Apps(AppsMsg::Back))
                .name("Back to Apps"),
            space().width(Length::FILL),
        ])
        .width(Length::FILL);
        let places = self
            .places
            .as_ref()
            .and_then(|l| l.iter().find(|(a, _)| a.to_lowercase() == id).map(|(a, n)| (a.clone(), *n)));
        let forget: Element<Message> = match &places {
            Some((app, n)) => setting(
                "Window places",
                &format!("Kept for {n} set{} of screens.", if *n == 1 { "" } else { "s" }),
                button(label("Forget…").size(12.0))
                    .padding(Padding::hv(4.0, 12.0))
                    .on_press(Message::Places(PlacesMsg::Ask(app.clone())))
                    .name("Forget window places"),
            ),
            None => setting("Window places", "None kept.", space()),
        };
        if OURS.iter().any(|(o, _)| *o == id) {
            return self.page_layout(
                &name,
                &format!("{name} has a settings window of its own, with its look among them: press Ctrl+, in it, or choose Settings… from its app button (the icon at the end of its title bar)."),
                vec![back, self.group(Some(&name), vec![forget])],
            );
        }
        // The desktop's Style forces every app while it is Dark or Light (Dave, 2026-11-14).
        let scheme = appearance_text(&self.desktop, "color-scheme");
        let bar: Element<Message> = if scheme == "dark" || scheme == "light" {
            setting(
                "Title bar",
                "",
                label(format!(
                    "{} for every app: Appearance > Style",
                    if scheme == "light" { "Light" } else { "Dark" }
                ))
                .size(12.0)
                .dim()
                .wrap()
                .width(Length::Fixed(230.0)),
            )
        } else {
            let own = self.app_bar(&id);
            let names = vec!["Desktop default (Dark)".to_string(), "Dark".into(), "Light".into()];
            let at = match own.as_deref() {
                Some("dark") => 1,
                Some("light") => 2,
                _ => 0,
            };
            let id2 = id.clone();
            setting(
                "Title bar",
                "Its bar's light or dark, while the desktop's Style is App default.",
                choice(names, Some(at), move |i| {
                    Message::Apps(AppsMsg::Bar(
                        id2.clone(),
                        [None, Some("dark".to_string()), Some("light".to_string())][i.min(2)].clone(),
                    ))
                })
                .width(Length::Fixed(230.0))
                .name("Title bar"),
            )
        };
        self.page_layout(
            &name,
            &format!(
                "{name} keeps its own settings and commands in its own menu; these are what the desktop does with it."
            ),
            vec![back, self.group(Some(&name), vec![bar, forget])],
        )
    }
}
