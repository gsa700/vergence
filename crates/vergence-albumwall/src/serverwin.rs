//! Adding a music server (milestone 2, step 2), as the Avalonia app's
//! ServerSignIn: the server, a user name and a password; signed in on a
//! thread of its own (a server asleep is not waited on in the window); a
//! server with several libraries asks which to add, each a library and a
//! wall of its own. The password makes the token and is forgotten.
//!
//! Plain HTTP to an address outside the local network is warned about
//! before it is added (Dave, 2026-11-26: no warning on the local network;
//! HTTP allowed with a clear warning, HTTPS supported, never required).

use std::sync::{Arc, Mutex};

use vergence_albumwall::subsonic::{Login, Server};
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Program, Theme, Waker, Window, button, checkbox, column, field, label,
    panel, row, space,
};

use crate::prefswin::{Ask, Link, Shared};

/// Signed in: the login, the server's libraries (id, name, ticked), and
/// whether to warn about plain HTTP.
type Signed = (Login, Vec<(String, String, bool)>, bool);

/// What the sign-in thread found.
enum Found {
    /// Signed in: the login, the server's libraries (id, name), whether to warn.
    Ok(Login, Vec<(String, String)>, bool),
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum Msg {
    Address(String),
    User(String),
    Password(String),
    SignIn,
    Tick(usize),
    Add,
    Cancel,
}

pub struct ServerWindow {
    link: Shared,
    app_id: String,
    desktop: String,
    address: String,
    user: String,
    password: String,
    said: Option<String>,
    busy: bool,
    found: Arc<Mutex<Option<Found>>>,
    waker: Option<Waker>,
    /// Signed in: the login, the server's libraries and which are ticked, and the warning.
    signed: Option<Signed>,
}

impl ServerWindow {
    pub fn new(link: Shared, app_id: &str) -> ServerWindow {
        let desktop = vergence_widgets::prefs::desktop_path()
            .map(|p| vergence_config::effective_text_with(Some(&p)))
            .unwrap_or_default();
        ServerWindow {
            link,
            app_id: app_id.into(),
            desktop,
            address: String::new(),
            user: String::new(),
            password: String::new(),
            said: None,
            busy: false,
            found: Arc::default(),
            waker: None,
            signed: None,
        }
    }

    fn sign_in(&mut self) {
        let (address, user, password) =
            (self.address.trim().to_string(), self.user.trim().to_string(), self.password.clone());
        if address.is_empty() || user.is_empty() || password.is_empty() {
            self.said = Some("All three are needed.".into());
            return;
        }
        self.busy = true;
        self.said = Some("Signing in…".into());
        // The password goes to the thread and is dropped there.
        self.password.clear();
        let (found, waker) = (self.found.clone(), self.waker.clone());
        let _ = std::thread::Builder::new().name("sign-in".into()).spawn(move || {
            let tls = crate::tls();
            let f = match Server::sign_in(&address, &user, &password, tls) {
                Ok(s) => {
                    let folders = s.folders().unwrap_or_default();
                    let warn = s.warn_plain();
                    Found::Ok(s.login, folders, warn)
                }
                Err(e) => Found::Failed(match e {
                    vergence_albumwall::subsonic::Error::Unreachable(why) => {
                        format!("{address} is not answering ({why}).")
                    }
                    e => format!("{address} would not sign {user} in: {e}."),
                }),
            };
            drop(password);
            *found.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
            if let Some(w) = &waker {
                w.wake();
            }
        });
    }

    /// The libraries chosen, named, handed to the wall.
    fn add(&mut self, window: &mut Window) {
        let Some((login, folders, _)) = &self.signed else { return };
        let host = login.address.split_once("://").map_or(login.address.as_str(), |(_, r)| r).to_string();
        let chosen: Vec<(String, Login)> = if folders.len() < 2 {
            // Its one library, or a server that does not say: taken whole.
            let folder = folders.first().map(|(id, ..)| id.clone());
            vec![(host, Login { folder, ..login.clone() })]
        } else {
            folders
                .iter()
                .filter(|(.., on)| *on)
                .map(|(id, name, _)| (name.clone(), Login { folder: Some(id.clone()), ..login.clone() }))
                .collect()
        };
        if chosen.is_empty() {
            self.said = Some("Tick at least one.".into());
            return;
        }
        Link::ask(&self.link, Ask::AddServers(chosen));
        window.close();
    }
}

impl Program for ServerWindow {
    type Message = Msg;

    fn title(&self) -> String {
        "Add a Music Server".into()
    }

    fn app_id(&self) -> String {
        self.app_id.clone()
    }

    fn role(&self) -> String {
        "server".into()
    }

    fn theme(&self) -> Theme {
        // A server's address and a user name are not prose: no spelling marks.
        Theme { spell: false, ..crate::prefswin::sheet_theme(&self.desktop) }
    }

    fn bar_style(&self) -> Option<vergence_ui::BarStyle> {
        Some(vergence_ui::BarStyle::Light)
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let dim = |s: &str| label(s.to_string()).size(13.0).dim().wrap().width(Length::FILL);
        let row_of = |name: &str, f: Element<Msg>| {
            row(vec![label(name.to_string()).size(13.0).width(Length::Fixed(90.0)), f])
                .spacing(10.0)
                .align(Align::Center)
                .width(Length::FILL)
        };
        let signed = self.signed.is_some();
        let mut v: Vec<Element<Msg>> = vec![
            label("On a music server").size(18.0).weight(600),
            dim("Navidrome, or any server that speaks Subsonic. Where is the server, and who are you on it?"),
            row_of("Server", {
                let f = field(self.address.clone(), Msg::Address)
                    .placeholder("nas:4533")
                    .width(Length::FILL)
                    .name("Server");
                if signed { f.disabled() } else { f }
            }),
            row_of("User name", {
                let f = field(self.user.clone(), Msg::User).width(Length::FILL).name("User name");
                if signed { f.disabled() } else { f }
            }),
        ];
        if !signed {
            v.push(row_of(
                "Password",
                field(self.password.clone(), Msg::Password)
                    .secret()
                    .on_submit(Msg::SignIn)
                    .width(Length::FILL)
                    .name("Password"),
            ));
            v.push(dim(
                "The password is used once, to sign in, and is not kept. AlbumWall only reads from the server: it plays the files as they are and changes nothing there.",
            ));
        }
        if let Some((_, folders, warn)) = &self.signed {
            if folders.len() >= 2 {
                v.push(label("Which of its libraries?").size(14.0).weight(600));
                v.push(dim("Each one ticked is a library of its own in AlbumWall, with its own wall."));
                for (i, (_, name, on)) in folders.iter().enumerate() {
                    v.push(checkbox(*on, name.clone()).on_press(Msg::Tick(i)));
                }
            }
            if *warn {
                v.push(
                    panel(dim(
                        "This server is reached over plain HTTP, outside your own network: the music and the sign-in token travel unencrypted. If the server offers HTTPS, its https:// address is safer.",
                    ))
                    .color(t.page)
                    .radius(6.0)
                    .padding(Padding::hv(10.0, 12.0))
                    .width(Length::FILL),
                );
            }
        }
        if let Some(s) = &self.said {
            v.push(label(s.clone()).size(13.0).wrap().width(Length::FILL));
        }
        let (go, words) = match &self.signed {
            Some((.., true)) => (Msg::Add, "Add anyway"),
            Some(_) => (Msg::Add, "Add"),
            None => (Msg::SignIn, "Sign in"),
        };
        v.push(
            row(vec![
                space().width(Length::FILL),
                button(label("Cancel").size(13.0))
                    .look(ButtonLook::Normal)
                    .padding(Padding::hv(5.0, 14.0))
                    .on_press(Msg::Cancel),
                button(label(words).size(13.0))
                    .look(ButtonLook::Accent)
                    .padding(Padding::hv(5.0, 14.0))
                    .on_press_if(!self.busy, go)
                    .name(words),
            ])
            .spacing(9.0)
            .width(Length::FILL),
        );
        panel(column(v).spacing(12.0).width(Length::FILL))
            .color(t.page)
            .radius(0.0)
            .padding(Padding::hv(20.0, 22.0))
            .width(Length::FILL)
            .height(Length::FILL)
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        match m {
            Msg::Address(s) => self.address = s,
            Msg::User(s) => self.user = s,
            Msg::Password(s) => self.password = s,
            Msg::SignIn => self.sign_in(),
            Msg::Tick(i) => {
                if let Some((_, folders, _)) = &mut self.signed
                    && let Some(f) = folders.get_mut(i)
                {
                    f.2 = !f.2;
                }
            }
            Msg::Add => self.add(window),
            Msg::Cancel => window.close(),
        }
        window.redraw();
    }

    fn waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    fn woken(&mut self, _window: &mut Window) -> bool {
        let Some(f) = self.found.lock().unwrap_or_else(|e| e.into_inner()).take() else { return false };
        self.busy = false;
        match f {
            Found::Ok(login, folders, warn) => {
                self.said = None;
                self.signed = Some((login, folders.into_iter().map(|(id, name)| (id, name, true)).collect(), warn));
            }
            Found::Failed(why) => self.said = Some(why),
        }
        true
    }

    fn opening_size(&self) -> Option<(u32, u32)> {
        Some((520, 420))
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style {
            kind: vergence_ui::WinKind::Dialog,
            modal: false,
            min: (440.0, 320.0),
            max: (0.0, 0.0),
            alone: false,
            frosted: false,
        }
    }
}
