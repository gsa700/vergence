//! The machine list (remote milestone 1, step 4; question 3: our own list,
//! with the hosts of `~/.ssh/config` offered when adding one). Saved in
//! `~/.config/vergence/remote-machines`, one SSH destination a line (as
//! `ssh` takes it: a name from the ssh config, `user@host`, an address).
//! Choosing one opens its window; each machine is a window of its own.

use std::path::PathBuf;

use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Program, Theme, Window, button, column, field, label, panel, row,
    scroll, space,
};

const APP_ID: &str = "vergence-remote-viewer";

#[derive(Debug, Clone)]
pub enum Msg {
    Open(String),
    Remove(String),
    Typed(String),
    Add,
    Offer(String),
}

pub struct Machines {
    list: Vec<String>,
    typed: String,
    /// Hosts from ~/.ssh/config not on the list yet.
    offered: Vec<String>,
    notice: Option<String>,
}

fn list_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("vergence").join("remote-machines"))
}

/// The hosts named in ~/.ssh/config (`Host` lines), patterns left out.
fn ssh_hosts() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(PathBuf::from(home).join(".ssh/config")) else { return Vec::new() };
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("Host ").or_else(|| line.strip_prefix("host ")) else { continue };
        for h in rest.split_whitespace() {
            if !h.contains(['*', '?', '!']) && !out.iter().any(|o| o == h) {
                out.push(h.to_string());
            }
        }
    }
    out
}

/// An SSH destination worth keeping: no spaces, no option-like start, no
/// shell's characters (it goes to ssh after `--`, never to a shell).
fn sane(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && !host.starts_with('-')
        && host.chars().all(|c| c.is_ascii_alphanumeric() || "@.-_:[]".contains(c))
}

impl Machines {
    pub fn new() -> Machines {
        let list: Vec<String> = list_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| t.lines().map(str::trim).filter(|l| sane(l)).map(str::to_string).collect())
            .unwrap_or_default();
        let mut m = Machines { list, typed: String::new(), offered: Vec::new(), notice: None };
        m.offer();
        m
    }

    fn offer(&mut self) {
        self.offered = ssh_hosts().into_iter().filter(|h| sane(h) && !self.list.contains(h)).collect();
    }

    fn save(&mut self) {
        let Some(p) = list_path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text: String = self.list.iter().map(|h| format!("{h}\n")).collect();
        if let Err(e) = std::fs::write(&p, text) {
            self.notice = Some(format!("The list could not be saved: {e}"));
        }
    }

    fn add(&mut self, host: &str) {
        let host = host.trim();
        if !sane(host) {
            self.notice = Some("That is not a name ssh takes (a host, user@host or an address).".into());
            return;
        }
        if !self.list.iter().any(|h| h == host) {
            self.list.push(host.to_string());
            self.save();
        }
        self.notice = None;
        self.offer();
    }
}

impl Program for Machines {
    type Message = Msg;

    fn title(&self) -> String {
        "Vergence Remote".into()
    }

    fn app_id(&self) -> String {
        APP_ID.into()
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let mut items: Vec<Element<Msg>> = vec![
            label("Use another Vergence computer as if you were sitting at it, over SSH with your own keys.")
                .size(12.0)
                .dim()
                .wrap()
                .width(Length::FILL),
        ];
        if self.list.is_empty() {
            items.push(label("No computers yet: add one below.").size(13.0).width(Length::FILL));
        }
        for h in &self.list {
            items.push(
                row(vec![
                    button(label(h.clone()).weight(600).width(Length::FILL))
                        .look(ButtonLook::Plain)
                        .padding(Padding::hv(8.0, 10.0))
                        .on_press(Msg::Open(h.clone()))
                        .width(Length::FILL)
                        .name(format!("Open {h}")),
                    button(label("Remove").size(11.0))
                        .look(ButtonLook::Plain)
                        .padding(Padding::hv(4.0, 8.0))
                        .on_press(Msg::Remove(h.clone()))
                        .name(format!("Remove {h}")),
                ])
                .spacing(6.0)
                .align(Align::Center)
                .width(Length::FILL),
            );
        }
        items.push(space().height(Length::Fixed(6.0)));
        items.push(
            row(vec![
                field(self.typed.clone(), Msg::Typed).on_submit(Msg::Add).width(Length::FILL).name("Computer"),
                button(label("Add")).look(ButtonLook::Accent).padding(Padding::hv(4.0, 12.0)).on_press(Msg::Add),
            ])
            .spacing(6.0)
            .align(Align::Center)
            .width(Length::FILL),
        );
        if !self.offered.is_empty() {
            items.push(label("From your SSH settings:").size(11.0).dim());
            let offers: Vec<Element<Msg>> = self
                .offered
                .iter()
                .take(12)
                .map(|h| {
                    button(label(format!("+ {h}")).size(12.0))
                        .look(ButtonLook::Plain)
                        .padding(Padding::hv(3.0, 8.0))
                        .on_press(Msg::Offer(h.clone()))
                        .name(format!("Add {h}"))
                })
                .collect();
            items.push(column(offers).spacing(2.0));
        }
        if let Some(n) = &self.notice {
            items.push(label(n.clone()).size(12.0).color(t.accent).wrap().width(Length::FILL));
        }
        // The margins as the other tools have them: a panel's padding, the
        // bar's height above (t.top).
        panel(scroll(column(items).spacing(8.0).width(Length::FILL)))
            .padding(Padding { top: 14.0 + t.top, bottom: 14.0, left: 14.0, right: 14.0 })
            .radius(0.0)
            .width(Length::FILL)
            .height(Length::FILL)
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        match m {
            Msg::Open(host) => window.open_window(Box::new(crate::session::Viewer::new(&host))),
            Msg::Remove(host) => {
                self.list.retain(|h| *h != host);
                self.save();
                self.offer();
            }
            Msg::Typed(s) => self.typed = s,
            Msg::Add => {
                let t = std::mem::take(&mut self.typed);
                self.add(&t);
            }
            Msg::Offer(h) => self.add(&h),
        }
    }
}
