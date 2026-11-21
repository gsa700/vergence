// Settings > Screenshots (screenshots milestone 1, step 8): desktop.toml
// [screenshots], which the server's capture.rs reads at each capture, and
// the programs allowed to take screenshots through the desktop portal
// (~/.local/state/vergence/screenshot-programs, which the server reads at
// each request, so taking one back here holds at once).

use super::*;

/// The folders offered, with their names.
const FOLDERS: [(&str, &str); 3] = [("Pictures \u{203a} Screenshots", "~/Pictures/Screenshots"), ("Desktop", "~/Desktop"), ("Downloads \u{203a} Screenshots", "~/Downloads/Screenshots")];

#[derive(Debug, Clone)]
pub enum ShotsMsg {
    EveryScreen(bool),
    Folder(usize),
    PerDay(bool),
    Thumbnail(bool),
    /// Take a program's permission back (its executable); empty: all.
    TakeBack(String),
}

fn table(d: &toml::Table) -> Option<&toml::Table> {
    d.get("screenshots").and_then(|m| m.as_table())
}

fn flag(d: &toml::Table, key: &str, default: bool) -> bool {
    table(d).and_then(|t| t.get(key)).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn grants_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))?;
    Some(base.join("vergence/screenshot-programs"))
}

/// The programs allowed: (executable, name), as the server keeps them.
pub fn read_grants() -> Vec<(String, String)> {
    let text = grants_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let mut v: Vec<(String, String)> = text
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            match (f.next(), f.next(), f.next()) {
                (Some("allow"), Some(p), name) if p.starts_with('/') => Some((p.to_string(), name.unwrap_or("").to_string())),
                _ => None,
            }
        })
        .collect();
    v.sort();
    v
}

fn write_grants(g: &[(String, String)]) {
    use std::os::unix::fs::OpenOptionsExt;
    let Some(p) = grants_path() else { return };
    let text: String = g.iter().map(|(p, n)| format!("allow\t{p}\t{n}\n")).collect();
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&p) {
        use std::io::Write;
        let _ = f.write_all(text.as_bytes());
    }
}

impl App {
    pub fn shots_message(&mut self, m: ShotsMsg) {
        if let ShotsMsg::TakeBack(program) = m {
            let kept: Vec<(String, String)> = if program.is_empty() { Vec::new() } else { read_grants().into_iter().filter(|(p, _)| *p != program).collect() };
            write_grants(&kept);
            self.shot_grants = kept;
            return;
        }
        self.save_table("screenshots", |t| match m {
            ShotsMsg::EveryScreen(on) => {
                t.insert("every-screen".into(), on.into());
            }
            ShotsMsg::Folder(i) => {
                if let Some((_, f)) = FOLDERS.get(i) {
                    t.insert("folder".into(), (*f).into());
                }
            }
            ShotsMsg::PerDay(on) => {
                t.insert("per-day".into(), on.into());
            }
            ShotsMsg::Thumbnail(on) => {
                t.insert("thumbnail".into(), on.into());
            }
            ShotsMsg::TakeBack(_) => {}
        });
    }

    pub fn screenshots_page(&self) -> Element<Message> {
        let d = &self.desktop;
        let folder = table(d).and_then(|t| t.get("folder")).and_then(|v| v.as_str()).unwrap_or("~/Pictures/Screenshots");
        let mut names: Vec<String> = FOLDERS.iter().map(|(n, _)| n.to_string()).collect();
        let at = match FOLDERS.iter().position(|(_, f)| *f == folder) {
            Some(i) => i,
            None => {
                // Set by hand: shown as it is, kept until another is picked.
                names.push(folder.to_string());
                names.len() - 1
            }
        };
        let every = flag(d, "every-screen", false);
        let programs: Vec<Element<Message>> = if self.shot_grants.is_empty() {
            vec![label("None yet. A program that asks (Firefox's screenshots, OBS, a color picker) is asked about first.").size(12.0).dim().wrap().width(Length::FILL)]
        } else {
            let mut rows: Vec<Element<Message>> = self
                .shot_grants
                .iter()
                .map(|(p, n)| {
                    let shown = if n.is_empty() { p.rsplit('/').next().unwrap_or(p).to_string() } else { n.clone() };
                    setting(&shown, p, button(label("Take back")).on_press(Message::Shots(ShotsMsg::TakeBack(p.clone()))).name(format!("Take back {shown}'s screenshots")))
                })
                .collect();
            rows.push(setting("", "Each asks again the next time.", button(label("Take back all")).on_press(Message::Shots(ShotsMsg::TakeBack(String::new()))).name("Take back every program's screenshots")));
            rows
        };
        self.page_layout(
            "Screenshots",
            "Super+Shift+3 takes the screen, Super+Shift+4 an area (Space: a window), Super+Shift+5 opens the screenshot bar; add Ctrl to copy to the clipboard instead. The Screenshot app opens the bar too.",
            vec![
                self.group(None, vec![
                    setting("Super+Shift+3 takes", "The screen the pointer is on, or every screen, each in a file of its own.", choice(vec!["The screen under the pointer".into(), "Every screen".into()], Some(every as usize), |i| Message::Shots(ShotsMsg::EveryScreen(i == 1))).width(Length::Fixed(220.0)).name("Super+Shift+3 takes")),
                    setting("Save to", "Where screenshots go.", choice(names, Some(at), |i| Message::Shots(ShotsMsg::Folder(i))).width(Length::Fixed(220.0)).name("Save screenshots to")),
                    setting("A folder for each day", "Inside it, named by the date (2026-11-21).", switch(flag(d, "per-day", true), |on| Message::Shots(ShotsMsg::PerDay(on))).name("A folder for each day")),
                    setting("Show the thumbnail", "A small picture of each screenshot at the bottom right: click to open it, drag it into an app, flick it away.", switch(flag(d, "thumbnail", true), |on| Message::Shots(ShotsMsg::Thumbnail(on))).name("Show the thumbnail")),
                ]),
                self.group(Some("Programs that may take screenshots"), programs),
            ],
        )
    }
}
