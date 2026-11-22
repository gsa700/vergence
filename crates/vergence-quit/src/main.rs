//! Vergence's Force Quit (tools milestone 1, docs/tools-milestone-1.md: named
//! as on the Mac, and kept very simple; the terminal is for anything harder): the
//! user's running applications, with their icons, and one of them ended at
//! once after a question. The list and the ending are the server's
//! (forcequit.rs) over the session bus; this is only the window. Opened from
//! the launcher, or by Ctrl+Alt+Escape anywhere.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use vergence_protocol::bus::{AppInfo, ClientMsg, ServerMsg};
use vergence_scene::Image;
use vergence_ui::{AppCommand as C, BarStyle, Role};
use vergence_widgets::{
    Align, ButtonLook, Element, Input, Length, Padding, Program, Theme, Waker, Window, button, column, label, panel,
    picture, row, scroll, space,
};

const APP_ID: &str = "vergence-quit";

#[derive(Debug, Clone)]
enum Msg {
    Pick(String),
    Ask,
    Cancel,
    Quit,
}

/// What the bus thread mails back.
enum Mail {
    Apps(Vec<AppInfo>),
    Failed(String),
}

struct ForceQuit {
    apps: Vec<(AppInfo, Option<Arc<Image>>)>,
    picked: Option<String>,
    /// The question is showing ("Force Calculator to quit?").
    asking: bool,
    status: String,
    inbox: Receiver<Mail>,
    outbox: Sender<String>,
    mail: Sender<Mail>,
    desktop: String,
}

impl ForceQuit {
    fn new() -> ForceQuit {
        let (mail, inbox) = channel();
        let (outbox, _) = channel();
        let desktop =
            vergence_widgets::prefs::desktop_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        ForceQuit { apps: Vec::new(), picked: None, asking: false, status: String::new(), inbox, outbox, mail, desktop }
    }

    fn picked_name(&self) -> Option<String> {
        let key = self.picked.as_ref()?;
        self.apps.iter().find(|(a, _)| &a.key == key).map(|(a, _)| a.name.clone())
    }
}

/// The bus: the list every two seconds, and an app ended when asked.
fn bus_thread(mail: Sender<Mail>, quits: Receiver<String>, waker: Waker) {
    std::thread::spawn(move || {
        let Ok(mut bus) = vergence_protocol::Client::connect(APP_ID) else {
            let _ = mail.send(Mail::Failed("Force Quit works in a Vergence session.".into()));
            waker.wake();
            return;
        };
        loop {
            while let Ok(key) = quits.try_recv() {
                if let Ok(ServerMsg::Error { message, .. }) =
                    bus.request(|serial| ClientMsg::ForceQuit { serial, app: key.clone() })
                {
                    let _ = mail.send(Mail::Failed(message));
                }
            }
            match bus.request(|serial| ClientMsg::ListApps { serial }) {
                Ok(ServerMsg::Apps { apps, .. }) => {
                    if mail.send(Mail::Apps(apps)).is_err() {
                        return;
                    }
                    waker.wake();
                }
                Ok(_) => {}
                Err(_) => return,
            }
            // Asked to end one: at once; else look again in two seconds.
            if let Ok(key) = quits.recv_timeout(Duration::from_secs(2)) {
                if let Ok(ServerMsg::Error { message, .. }) =
                    bus.request(|serial| ClientMsg::ForceQuit { serial, app: key.clone() })
                {
                    let _ = mail.send(Mail::Failed(message));
                }
                std::thread::sleep(Duration::from_millis(300));
            }
        }
    });
}

impl Program for ForceQuit {
    type Message = Msg;

    fn title(&self) -> String {
        "Force Quit".into()
    }

    fn app_id(&self) -> String {
        APP_ID.into()
    }

    fn theme(&self) -> Theme {
        Theme::from_desktop(&self.desktop, false, "[force quit]")
    }

    fn bar_style(&self) -> Option<BarStyle> {
        Some(vergence_ui::bar_style_setting(&self.desktop, "[force quit]", BarStyle::Light))
    }

    fn waker(&mut self, waker: Waker) {
        let (quits_tx, quits_rx) = channel();
        self.outbox = quits_tx;
        bus_thread(self.mail.clone(), quits_rx, waker);
    }

    fn woken(&mut self, _window: &mut Window) -> bool {
        let mut any = false;
        while let Ok(m) = self.inbox.try_recv() {
            any = true;
            match m {
                Mail::Apps(apps) => {
                    self.apps = apps
                        .into_iter()
                        .filter(|a| a.key != APP_ID)
                        .map(|a| {
                            let side = a.icon_side as u32;
                            let img = (!a.icon.is_empty())
                                .then(|| Arc::new(Image { width: side, height: side, rgba: a.icon.clone() }));
                            (a, img)
                        })
                        .collect();
                    // What was picked and has gone is no longer picked.
                    if let Some(p) = &self.picked
                        && !self.apps.iter().any(|(a, _)| &a.key == p)
                    {
                        self.picked = None;
                        self.asking = false;
                    }
                }
                Mail::Failed(why) => self.status = why,
            }
        }
        any
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let intro = label("If an application has stopped answering, choose it and press Force Quit.")
            .size(12.0)
            .dim()
            .wrap()
            .width(Length::FILL);
        let mut rows: Vec<Element<Msg>> = Vec::new();
        for (a, img) in &self.apps {
            let icon: Element<Msg> = match img {
                Some(i) => picture(i.clone()),
                None => panel(
                    label(a.name.chars().next().unwrap_or('?').to_uppercase().to_string())
                        .weight(700)
                        .align(Align::Center)
                        .width(Length::FILL),
                )
                .color(t.card)
                .radius(6.0)
                .width(Length::Fixed(32.0))
                .height(Length::Fixed(32.0)),
            };
            let mut words = vec![label(a.name.clone())];
            if a.quitting {
                words.push(label("Quitting…").size(11.0).dim());
            } else if !a.responding {
                words.push(label("Not responding").size(11.0).color(vergence_scene::Color::rgb(0xc7, 0x4d, 0x47)));
            }
            let picked = self.picked.as_deref() == Some(a.key.as_str());
            rows.push(
                button(
                    row(vec![icon, column(words).spacing(1.0).width(Length::FILL)])
                        .spacing(10.0)
                        .align(Align::Center)
                        .width(Length::FILL),
                )
                .look(if picked { ButtonLook::Accent } else { ButtonLook::Plain })
                .padding(Padding::hv(6.0, 8.0))
                .on_press(Msg::Pick(a.key.clone()))
                .width(Length::FILL)
                .name(if a.responding {
                    a.name.clone()
                } else {
                    format!("{} (not responding)", a.name)
                }),
            );
        }
        if self.apps.is_empty() {
            rows.push(label(if self.status.is_empty() { "No applications are running." } else { "" }).dim());
        }
        let list = panel(scroll(column(rows).spacing(2.0).width(Length::FILL)))
            .color(t.card)
            .radius(10.0)
            .padding(4.0)
            .width(Length::FILL)
            .height(Length::FILL);
        let bottom: Element<Msg> = match (self.asking, self.picked_name()) {
            (true, Some(name)) => column(vec![
                label(format!("Force {name} to quit?")).weight(600),
                label("Anything not saved in it is lost.").size(12.0).dim(),
                row(vec![
                    space().width(Length::FILL),
                    button(label("Cancel").size(12.0))
                        .padding(Padding::hv(4.0, 12.0))
                        .on_press(Msg::Cancel)
                        .name("Cancel"),
                    button(label("Force Quit").size(12.0))
                        .look(ButtonLook::Accent)
                        .padding(Padding::hv(4.0, 14.0))
                        .on_press(Msg::Quit)
                        .name("Force Quit"),
                ])
                .spacing(8.0)
                .align(Align::Center)
                .width(Length::FILL),
            ])
            .spacing(4.0)
            .width(Length::FILL),
            _ => {
                let quit = button(label("Force Quit").size(12.0))
                    .look(ButtonLook::Accent)
                    .padding(Padding::hv(4.0, 14.0))
                    .name("Force Quit");
                row(vec![
                    label(self.status.clone()).size(11.0).dim().wrap().width(Length::FILL),
                    if self.picked.is_some() { quit.on_press(Msg::Ask) } else { quit },
                ])
                .spacing(8.0)
                .align(Align::Center)
                .width(Length::FILL)
            }
        };
        panel(column(vec![intro, list, bottom]).spacing(10.0).width(Length::FILL).height(Length::FILL))
            .padding(Padding { top: 14.0 + t.top, bottom: 14.0, left: 14.0, right: 14.0 })
            .radius(0.0)
            .width(Length::FILL)
            .height(Length::FILL)
            .into()
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        match m {
            Msg::Pick(key) => {
                self.picked = Some(key);
                self.asking = false;
                self.status.clear();
            }
            Msg::Ask => self.asking = self.picked.is_some(),
            Msg::Cancel => self.asking = false,
            Msg::Quit => {
                if let Some(key) = self.picked.take() {
                    let _ = self.outbox.send(key);
                }
                self.asking = false;
            }
        }
        window.redraw();
    }

    /// Escape takes the question back, or closes the window.
    fn input(&mut self, input: &Input, window: &mut Window) -> bool {
        if let Input::Key { sym: 0xff1b, pressed: true, .. } = input {
            if self.asking {
                self.asking = false;
            } else {
                window.close();
            }
            return true;
        }
        false
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        Some((false, vec![C::role(1, Role::About)]))
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        if id == 1 {
            let about = vergence_widgets::about::About::new(
                APP_ID,
                "Force Quit",
                env!("CARGO_PKG_VERSION"),
                "Ends an application that has stopped answering, at once. Ctrl+Alt+Escape opens it anywhere.",
                self.theme(),
            );
            window.open_window(vergence_widgets::window(about));
        }
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { min: (340.0, 320.0), ..Default::default() }
    }
}

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("vergence-quit {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if let Err(e) = vergence_widgets::run(ForceQuit::new(), (400, 440)) {
        eprintln!("vergence-quit: {e}");
        std::process::exit(1);
    }
}
