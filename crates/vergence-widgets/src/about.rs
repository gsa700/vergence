//! An app's About window (tools milestone 1): its name, version, what it
//! is and its license, the same in every app of ours. The About role of the
//! app menu opens it; a dialog over the app's window.

use crate::{Align, ButtonLook, Element, Length, Padding, Program, Theme, Window, button, column, label, panel, row, space};

pub struct About {
    pub app_id: String,
    pub name: String,
    pub version: String,
    /// A sentence or two: what the app is.
    pub what: String,
    pub theme: Theme,
}

impl About {
    pub fn new(app_id: &str, name: &str, version: &str, what: &str, theme: Theme) -> About {
        About { app_id: app_id.into(), name: name.into(), version: version.into(), what: what.into(), theme }
    }
}

#[derive(Debug, Clone)]
pub struct Close;

impl Program for About {
    type Message = Close;

    fn title(&self) -> String {
        format!("About {}", self.name)
    }

    fn app_id(&self) -> String {
        self.app_id.clone()
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    fn view(&self, _: &Theme) -> Element<Close> {
        panel(
            column(vec![
                label(self.name.clone()).size(20.0).weight(700),
                label(format!("Version {}", self.version)).size(12.0).dim(),
                label(self.what.clone()).wrap().width(Length::FILL),
                label("Part of Vergence. Apache-2.0 OR GPL-2.0-or-later.").size(11.0).dim().wrap().width(Length::FILL),
                row(vec![space().width(Length::FILL), button(label("Close").size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(4.0, 14.0)).on_press(Close).name("Close")]).align(Align::Center).width(Length::FILL),
            ])
            .spacing(8.0)
            .width(Length::FILL),
        )
        .padding(20.0)
        .radius(0.0)
        .width(Length::FILL)
        .height(Length::FILL)
        .into()
    }

    fn update(&mut self, _: Close, window: &mut Window) {
        window.close();
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { kind: vergence_ui::WinKind::Dialog, modal: false, min: (380.0, 210.0), max: (0.0, 0.0), alone: false }
    }
}
