//! The widgets so far, in a window: `cargo run --example vwidgets`.
//! `VERGENCE_UI_TREE=1` prints what each widget means.

use vergence_widgets::{Align, ButtonLook, Element, Length, Padding, Program, Theme, Window, button, column, label, panel, row, rule, run, space};

#[derive(Debug, Clone)]
enum Msg {
    Count(i32),
    Side(bool),
    Dark(bool),
}

struct Demo {
    count: i32,
    left: bool,
    dark: bool,
}

fn desktop() -> String {
    let dir = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config")));
    dir.and_then(|d| std::fs::read_to_string(d.join("vergence/desktop.toml")).ok()).unwrap_or_default()
}

impl Program for Demo {
    type Message = Msg;

    fn title(&self) -> String {
        "Widgets".into()
    }

    fn app_id(&self) -> String {
        "vwidgets".into()
    }

    fn theme(&self) -> Theme {
        let mut t = Theme::from_desktop(&desktop(), self.dark, "[widgets]");
        if t.dark != self.dark {
            t = Theme { opacity: t.opacity, ..Theme::new(self.dark, t.accent, t.high_contrast) };
        }
        t
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let side = |left: bool, name: &str| button(label(name)).look(if self.left == left { ButtonLook::Chosen } else { ButtonLook::Normal }).on_press(Msg::Side(left));
        let setting = |title: &str, note: &str, control: Element<Msg>| {
            row(vec![column(vec![label(title), label(note).dim().size(11.0).wrap()]).spacing(2.0).width(Length::FILL), control]).spacing(16.0).align(Align::Center).width(Length::FILL)
        };
        let card = panel(
            column(vec![
                setting("Count", &format!("Pressed so far: {}.", self.count), row(vec![button(label("\u{2212}")).name("Less").on_press(Msg::Count(-1)), button(label("+")).name("More").on_press(Msg::Count(1))]).spacing(6.0)),
                rule(),
                setting("Controls", "Left, as on a Mac; right, as on Windows.", row(vec![side(true, "Left"), side(false, "Right")]).spacing(6.0)),
                rule(),
                setting("Look", "Dark or light, for this window only.", button(label(if self.dark { "Light" } else { "Dark" })).look(ButtonLook::Accent).on_press(Msg::Dark(!self.dark))),
                rule(),
                setting("Off", "A button with nothing to do.", button(label("Nothing"))),
            ])
            .spacing(10.0)
            .width(Length::FILL),
        )
        .color(t.card)
        .padding(Padding::all(14.0))
        .width(Length::FILL);
        panel(
            column(vec![
                label("Widgets").size(20.0).weight(600),
                label("Tab moves between the controls; Enter or Space presses. A finger works as the pointer does.").dim().wrap().width(Length::FILL),
                space().height(Length::Fixed(6.0)),
                card,
            ])
            .spacing(8.0)
            .width(Length::FILL)
            .max_width(560.0),
        )
        .padding(Padding::all(24.0))
        .align_x(Align::Center)
        .width(Length::FILL)
        .height(Length::FILL)
    }

    fn update(&mut self, m: Msg, _: &mut Window) {
        match m {
            Msg::Count(d) => self.count += d,
            Msg::Side(left) => self.left = left,
            Msg::Dark(d) => self.dark = d,
        }
    }
}

fn main() -> std::io::Result<()> {
    run(Demo { count: 0, left: true, dark: true }, (640, 420))
}
