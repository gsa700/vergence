// Window places (settings milestone 2, step 6): the apps window memory keeps
// places for, read from the server over the bus (`ListPlaces`, never its
// file), each with Forget, and Forget all; each asks first in a dialog. The
// dock's "Forget window places..." starts this app with `--page windows
// --forget <app>`, which asks the same question once the list has come.

use super::*;

/// The apps with saved places, from the server, mailed back.
pub fn fetch(mail: Mail) {
    std::thread::spawn(move || {
        let Ok(mut bus) = vergence_protocol::Client::connect("vergence-settings") else { return };
        if let Ok(ServerMsg::Places { apps, .. }) = bus.request(|serial| ClientMsg::ListPlaces { serial }) {
            mail.send(Message::Places(PlacesMsg::List(apps)));
        }
    });
}

/// Forget an app's places (all apps' when empty), then read the list again.
fn forget(mail: Mail, app: String) {
    std::thread::spawn(move || {
        let Ok(mut bus) = vergence_protocol::Client::connect("vergence-settings") else { return };
        let _ = bus.request(|serial| ClientMsg::ForgetPlaces { serial, app: app.clone() });
        if let Ok(ServerMsg::Places { apps, .. }) = bus.request(|serial| ClientMsg::ListPlaces { serial }) {
            mail.send(Message::Places(PlacesMsg::List(apps)));
        }
    });
}

#[derive(Debug, Clone)]
pub enum PlacesMsg {
    /// The list from the server: (app, places kept).
    List(Vec<(String, u32)>),
    /// Forget pressed for an app ("" for all): ask first.
    Ask(String),
    /// The dialog said Forget.
    Forget(String),
}

impl App {
    pub fn places_message(&mut self, m: PlacesMsg) {
        match m {
            PlacesMsg::List(mut apps) => {
                apps.sort_by_key(|(a, _)| self.places_name(a).to_lowercase());
                self.places = Some(apps);
                // Started from the dock: ask now that the list is here, if the
                // app still has places.
                if let Some(app) = self.places_from_dock.take()
                    && self.places.as_ref().is_some_and(|l| l.iter().any(|(a, _)| *a == app))
                {
                    self.places_ask = Some(app);
                }
            }
            PlacesMsg::Ask(app) => self.places_ask = Some(app),
            PlacesMsg::Forget(app) => forget(self.mail.clone(), app),
        }
    }

    /// An app's name for people: its application entry's (by id, its last
    /// part, or its name, in any case), else as window memory has it.
    pub fn places_name(&self, app: &str) -> String {
        let low = app.to_lowercase();
        self.apps
            .iter()
            .find(|e| {
                let id = e.id.trim_end_matches(".desktop").to_lowercase();
                id == low || id.rsplit('.').next() == Some(low.as_str()) || e.name.to_lowercase() == low
            })
            .map(|e| e.name.clone())
            .unwrap_or_else(|| app.to_string())
    }

    /// Forget pressed (or asked for from the dock): the question, modal.
    pub fn places_dialog(&mut self, window: &mut Window) {
        if let Some(app) = self.places_ask.take() {
            let name = self.places_name(&app);
            window.open_window(vergence_widgets::window(ForgetDialog {
                mail: self.mail.clone(),
                app,
                name,
                theme: Program::theme(self),
            }));
        }
    }

    pub fn places_group(&self) -> Element<Message> {
        let heading = Some("Window places");
        let Some(list) = &self.places else {
            return self.group(heading, vec![setting("Reading the list...", "", space())]);
        };
        if list.is_empty() {
            return self.group(
                heading,
                vec![setting(
                    "No window places are kept",
                    "Where each app's windows were is remembered when they move or close, per screen setup.",
                    space(),
                )],
            );
        }
        let mut rows: Vec<Element<Message>> = list
            .iter()
            .map(|(app, n)| {
                let name = self.places_name(app);
                let detail = if *n == 1 {
                    "1 place kept.".to_string()
                } else {
                    format!("{n} places kept, across screen setups.")
                };
                setting(
                    &name,
                    &detail,
                    button(label("Forget"))
                        .on_press(Message::Places(PlacesMsg::Ask(app.clone())))
                        .name(&format!("Forget {name}'s window places")),
                )
            })
            .collect();
        rows.push(setting(
            "Every app",
            "Windows open in the default place until they are moved again. Open windows stay where they are.",
            button(label("Forget all"))
                .on_press(Message::Places(PlacesMsg::Ask(String::new())))
                .name("Forget all window places"),
        ));
        self.group(heading, rows)
    }
}

/// "Forget where Terminal's windows go?", with Cancel and Forget.
pub struct ForgetDialog {
    pub mail: Mail,
    /// The app's name in window memory; empty for every app.
    pub app: String,
    /// Its name for people.
    pub name: String,
    pub theme: Theme,
}

#[derive(Debug, Clone)]
pub enum ForgetMsg {
    Forget,
    Cancel,
}

impl ForgetDialog {
    fn question(&self) -> String {
        if self.app.is_empty() {
            "Forget where every app's windows go?".into()
        } else {
            format!("Forget where {}'s windows go?", self.name)
        }
    }
}

impl Program for ForgetDialog {
    type Message = ForgetMsg;
    fn title(&self) -> String {
        "Forget window places?".into()
    }
    fn app_id(&self) -> String {
        "vergence-settings".into()
    }
    fn theme(&self) -> Theme {
        self.theme.clone()
    }
    fn view(&self, _: &Theme) -> Element<ForgetMsg> {
        let detail = if self.app.is_empty() {
            "Every app's windows open in the default place next time."
        } else {
            "They open in the default place next time."
        };
        panel(
            column(vec![
                label(self.question()).size(15.0).weight(600).wrap().width(Length::FILL),
                label(detail).dim().wrap().width(Length::FILL),
                row(vec![
                    space().width(Length::FILL),
                    button(label("Cancel").size(12.0))
                        .padding(Padding::hv(4.0, 12.0))
                        .on_press(ForgetMsg::Cancel)
                        .name("Cancel"),
                    button(label("Forget").size(12.0))
                        .look(ButtonLook::Accent)
                        .padding(Padding::hv(4.0, 14.0))
                        .on_press(ForgetMsg::Forget)
                        .name("Forget"),
                ])
                .spacing(8.0)
                .width(Length::FILL),
            ])
            .spacing(10.0)
            .width(Length::FILL),
        )
        .padding(20.0)
        .radius(0.0)
        .width(Length::FILL)
        .height(Length::FILL)
        .into()
    }
    fn update(&mut self, m: ForgetMsg, window: &mut Window) {
        if matches!(m, ForgetMsg::Forget) {
            self.mail.send(Message::Places(PlacesMsg::Forget(self.app.clone())));
        }
        window.close();
    }
    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style {
            kind: vergence_ui::WinKind::Dialog,
            modal: true,
            min: (400.0, 150.0),
            max: (0.0, 0.0),
            alone: false,
            frosted: false,
        }
    }
    fn scale(&mut self, _: f32) -> bool {
        false
    }
}
