//! Properties (files milestone 2, step 3): a file's or folder's own window,
//! Windows-style (Dave, 2026-11-16), opened by Get Info. Tabs inside it:
//!
//! - **General**: the mark, the name (changed here, undone with Ctrl+Z in
//!   File Viewer), its kind, where it is, its size (a folder's counted on a
//!   worker, with how many items), created, changed and opened, a link's
//!   target, and what opens it, which can be changed for every file of the
//!   type (Vergence's own mimeapps list, as `vergence-open --default`).
//! - **Permissions**: owner and group; read and write for the owner, the
//!   group and others; "Allow running as a program" for a file. Changed
//!   only by the owner (or root); a folder that can be read can be entered.
//! - Later **Earlier Versions** (backup milestone 1, step 4).
//!
//! Changes are made at once (no OK or Apply): the Mac's way, the window
//! closed by its bar.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use vergence_widgets::files::{self, Entry};
use vergence_widgets::{Align, ButtonLook, Element, Length, Padding, Program, Theme, Waker, Window, button, choice, column, field, label, panel, row, rule, space, switch};

use crate::{SharedRef, ops};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Permissions,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Page(Page),
    NameTyped(String),
    NameDone,
    OpensWith(usize),
    /// Who (0 owner, 1 group, 2 others) and what: 0 read and write, 1 read only, 2 no access.
    Access(usize, usize),
    Runs(bool),
}

/// A folder's count: its size and how many items, or None while counting
/// (or if it was too big to count).
type Count = Arc<Mutex<Option<Option<(u64, u64)>>>>;

pub struct Properties {
    path: PathBuf,
    shared: SharedRef,
    theme: Theme,
    page: Page,
    name: String,
    problem: Option<String>,
    count: Count,
    /// The applications for its type (desktop ids and names), and the default.
    apps: Vec<(String, String)>,
    default: Option<usize>,
    mime: String,
}

impl Properties {
    pub fn new(path: PathBuf, shared: SharedRef, theme: Theme) -> Properties {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string_lossy().into_owned());
        let mime = vergence_types::of_file(&path).to_string();
        let (apps, default) = if path.is_dir() {
            (Vec::new(), None)
        } else {
            let entries = vergence_shell::apps::installed();
            let lists = vergence_types::lists();
            let ids = vergence_shell::open::apps_for(&mime, &entries, &lists);
            let named: Vec<(String, String)> = ids
                .into_iter()
                .take(24)
                .map(|id| {
                    let n = entries.iter().find(|e| vergence_shell::open::desktop_id(&e.id) == id).map(|e| e.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| id.trim_end_matches(".desktop").to_string());
                    (id, n)
                })
                .collect();
            let known: Vec<String> = entries.iter().map(|e| vergence_shell::open::desktop_id(&e.id)).collect();
            let apps: Vec<vergence_types::App> = entries.iter().zip(&known).filter(|(e, _)| !e.hidden).map(|(e, id)| vergence_types::App { id, types: &e.mime_types }).collect();
            let def = vergence_types::default_for(&mime, &apps, &lists);
            let default = def.and_then(|d| named.iter().position(|(id, _)| *id == d));
            (named, default)
        };
        Properties { path, shared, theme, page: Page::General, name, problem: None, count: Arc::default(), apps, default, mime }
    }

    fn general(&self) -> Vec<Element<Msg>> {
        let mut out = Vec::new();
        let m = self.path.symlink_metadata();
        let entry = Entry::of(self.path.clone());
        let kind = entry.as_ref().map(|e| e.kind()).unwrap_or(files::Kind::Other);
        out.push(
            row(vec![
                files::kind_icon_sized(kind, 2.6).width(Length::Fixed(56.0)).height(Length::Fixed(44.0)),
                field(self.name.clone(), Msg::NameTyped).on_submit(Msg::NameDone).width(Length::FILL).name("Name"),
            ])
            .spacing(10.0)
            .align(Align::Center)
            .width(Length::FILL),
        );
        if let Some(p) = &self.problem {
            out.push(label(p.clone()).size(11.0).color(vergence_scene::Color { r: 214, g: 64, b: 64, a: 255 }).wrap().width(Length::FILL));
        }
        out.push(rule());
        let Ok(m) = m else {
            out.push(fact("", "It is not there any more.".into()));
            return out;
        };
        let kind_words = if m.file_type().is_symlink() {
            "Link".to_string()
        } else {
            entry.as_ref().map(files::kind_words).unwrap_or_default()
        };
        out.push(fact("Kind", kind_words));
        if let Some(dir) = self.path.parent() {
            out.push(fact("Where", files::tilde(dir)));
        }
        if m.file_type().is_symlink() {
            out.push(fact("Points to", std::fs::read_link(&self.path).map(|t| t.to_string_lossy().into_owned()).unwrap_or_default()));
        }
        if m.is_dir() {
            let (size, items) = match *self.count.lock().unwrap_or_else(|e| e.into_inner()) {
                None => ("Counting…".to_string(), None),
                Some(None) => ("Too much to count".to_string(), None),
                Some(Some((bytes, items))) => (with_bytes(bytes), Some(format!("{} item{}", grouped(items), if items == 1 { "" } else { "s" }))),
            };
            out.push(fact("Size", size));
            if let Some(i) = items {
                out.push(fact("Contains", i));
            }
        } else {
            out.push(fact("Size", with_bytes(m.len())));
        }
        out.push(rule());
        let time = |t: std::io::Result<SystemTime>| t.ok().map(files::when).unwrap_or_else(|| "—".into());
        out.push(fact("Created", time(m.created())));
        out.push(fact("Changed", time(m.modified())));
        out.push(fact("Opened", time(m.accessed())));
        if !m.is_dir() {
            out.push(rule());
            let names: Vec<String> = self.apps.iter().map(|(_, n)| n.clone()).collect();
            let opens: Element<Msg> = if names.is_empty() { label("Nothing installed opens this kind of file.").size(12.0).dim().wrap().width(Length::FILL) } else { choice(names, self.default, Msg::OpensWith).name("Opens with") };
            out.push(row(vec![label("Opens with").size(12.0).dim().width(Length::Fixed(LABEL_W)), opens]).spacing(8.0).align(Align::Center).width(Length::FILL));
            if !self.apps.is_empty() {
                out.push(label(format!("Changing it changes it for every {} file.", vergence_types::describe(&self.mime))).size(11.0).dim().wrap().width(Length::FILL));
            }
        }
        out
    }

    fn permissions(&self, t: &Theme) -> Vec<Element<Msg>> {
        let mut out = Vec::new();
        let Ok(m) = self.path.symlink_metadata() else {
            out.push(fact("", "It is not there any more.".into()));
            return out;
        };
        let mine = m.uid() == uid() || uid() == 0;
        out.push(fact("Owner", owner_name(m.uid())));
        out.push(fact("Group", group_name(m.gid())));
        out.push(rule());
        let mode = m.permissions().mode();
        let options = vec!["Read and write".to_string(), "Read only".to_string(), "No access".to_string()];
        for (who, name, shift) in [(0usize, "Owner can", 6u32), (1, "Group can", 3), (2, "Others can", 0)] {
            let bits = mode >> shift & 7;
            let now = match (bits & 4 != 0, bits & 2 != 0) {
                (true, true) => 0,
                (true, false) => 1,
                (false, _) => 2,
            };
            let c: Element<Msg> = if mine { choice(options.clone(), Some(now), move |k| Msg::Access(who, k)).name(format!("{name} access")) } else { label(options[now].clone()).size(12.0) };
            out.push(row(vec![label(name).size(12.0).dim().width(Length::Fixed(LABEL_W)), c]).spacing(8.0).align(Align::Center).width(Length::FILL));
        }
        if m.is_file() {
            out.push(rule());
            let runs = mode & 0o100 != 0;
            let s: Element<Msg> = if mine { switch(runs, Msg::Runs).name("Allow running as a program") } else { label(if runs { "Yes" } else { "No" }).size(12.0) };
            out.push(row(vec![label("Allow running as a program").size(12.0).width(Length::FILL), s]).spacing(8.0).align(Align::Center).width(Length::FILL));
        }
        if !mine {
            out.push(label(format!("Only {} can change these.", owner_name(m.uid()))).size(11.0).color(t.dim).wrap().width(Length::FILL));
        }
        out
    }

    fn set_mode(&mut self, f: impl Fn(u32) -> u32) {
        let Ok(m) = self.path.metadata() else { return };
        let mode = f(m.permissions().mode());
        if let Err(e) = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(mode)) {
            self.problem = Some(format!("It could not be changed: {}", files::plain(&e)));
        }
    }
}

const LABEL_W: f32 = 90.0;

/// A line of facts: what, then its value.
fn fact(what: &str, value: String) -> Element<Msg> {
    row(vec![label(what.to_string()).size(12.0).dim().width(Length::Fixed(LABEL_W)), label(value).size(12.0).wrap().width(Length::FILL)]).spacing(8.0).width(Length::FILL)
}

/// "4.8 MB (4,812,345 bytes)".
fn with_bytes(n: u64) -> String {
    if n < 1000 { files::size_words(n) } else { format!("{} ({} bytes)", files::size_words(n), grouped(n)) }
}

/// A number with commas between thousands.
fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn uid() -> u32 {
    std::env::var_os("HOME").and_then(|h| std::fs::metadata(h).ok()).map_or(u32::MAX, |m| m.uid())
}

/// A name from a colon-separated account file (/etc/passwd, /etc/group) by
/// its number; the number if it is not there.
fn name_in(file: &str, id: u32) -> String {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|t| {
            t.lines().find_map(|l| {
                let mut f = l.split(':');
                let (name, _, n) = (f.next()?, f.next()?, f.next()?);
                (n.parse::<u32>().ok()? == id).then(|| name.to_string())
            })
        })
        .unwrap_or_else(|| id.to_string())
}

pub fn owner_name(id: u32) -> String {
    name_in("/etc/passwd", id)
}

fn group_name(id: u32) -> String {
    name_in("/etc/group", id)
}

/// Everything in a folder (on its file system, links not followed): bytes
/// and items; None past a few hundred thousand items.
fn count(p: &Path, dev: u64, budget: &mut u32, items: &mut u64) -> Option<u64> {
    let Ok(m) = p.symlink_metadata() else { return Some(0) };
    if m.dev() != dev {
        return Some(0);
    }
    if !m.is_dir() {
        return Some(m.len());
    }
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            *budget = budget.checked_sub(1)?;
            *items += 1;
            total += count(&e.path(), dev, budget, items)?;
        }
    }
    Some(total)
}

impl Program for Properties {
    type Message = Msg;

    fn title(&self) -> String {
        format!("{} Properties", self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.to_string_lossy().into_owned()))
    }

    fn app_id(&self) -> String {
        crate::APP_ID.into()
    }

    fn theme(&self) -> Theme {
        // A file's name is not prose: no spelling marks.
        Theme { spell: false, ..self.theme.clone() }
    }

    fn opening_size(&self) -> Option<(u32, u32)> {
        Some((420, 470))
    }

    /// Its own place in window memory, apart from FileView's windows (Dave,
    /// 2026-11-18: it no longer kept its size or place).
    fn role(&self) -> String {
        "properties".into()
    }

    fn style(&self) -> vergence_ui::Style {
        // A window of its own with only a close button (Dave, 2026-11-16: no
        // app menu or settings on a Properties window).
        vergence_ui::Style { kind: vergence_ui::WinKind::Dialog, alone: true, min: (360.0, 380.0), ..Default::default() }
    }

    fn waker(&mut self, waker: Waker) {
        // A folder's size, counted on a worker.
        if let Ok(m) = self.path.symlink_metadata()
            && m.is_dir()
        {
            let (p, dev, out) = (self.path.clone(), m.dev(), self.count.clone());
            std::thread::spawn(move || {
                let (mut budget, mut items) = (crate::COUNT_MOST, 0u64);
                let bytes = count(&p, dev, &mut budget, &mut items);
                *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(bytes.map(|b| (b, items)));
                waker.wake();
            });
        }
    }

    fn woken(&mut self, _window: &mut Window) -> bool {
        true
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let tab = |text: &str, p: Page| -> Element<Msg> { button(label(text).size(12.0)).look(if self.page == p { ButtonLook::Chosen } else { ButtonLook::Plain }).padding(Padding::hv(4.0, 14.0)).on_press(Msg::Page(p)).name(text) };
        let tabs = row(vec![tab("General", Page::General), tab("Permissions", Page::Permissions), space().width(Length::FILL)]).spacing(4.0).width(Length::FILL);
        let body = match self.page {
            Page::General => self.general(),
            Page::Permissions => self.permissions(t),
        };
        let card = panel(column(body).spacing(10.0).width(Length::FILL)).color(t.card).radius(8.0).padding(14.0).width(Length::FILL);
        panel(column(vec![tabs, card]).spacing(10.0).width(Length::FILL)).padding(Padding { top: t.top + 12.0, bottom: 14.0, left: 14.0, right: 14.0 }).radius(0.0).width(Length::FILL).height(Length::FILL)
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        self.problem = None;
        match m {
            Msg::Page(p) => self.page = p,
            Msg::NameTyped(s) => self.name = s,
            Msg::NameDone => {
                let name = self.name.trim().to_string();
                let from = self.path.clone();
                if name.is_empty() || name == "." || name == ".." || name.contains('/') {
                    self.problem = Some("That cannot be a name.".into());
                } else if from.file_name().is_some_and(|n| n.to_string_lossy() != name) {
                    let to = from.with_file_name(&name);
                    if to.symlink_metadata().is_ok() {
                        self.problem = Some(format!("Something named “{name}” is already there."));
                    } else {
                        match std::fs::rename(&from, &to) {
                            Ok(()) => {
                                self.shared.borrow_mut().undo.push(ops::Done::Renamed(from, to.clone()));
                                self.path = to;
                                window.set_title(&self.title());
                            }
                            Err(e) => self.problem = Some(format!("It could not be renamed: {}", files::plain(&e))),
                        }
                    }
                }
            }
            Msg::OpensWith(k) => {
                if let Some((id, _)) = self.apps.get(k) {
                    match vergence_types::choose_default(&self.mime, id) {
                        Ok(()) => self.default = Some(k),
                        Err(e) => self.problem = Some(format!("It could not be changed: {e}")),
                    }
                }
            }
            Msg::Access(who, k) => {
                let shift = [6u32, 3, 0][who.min(2)];
                let dir = self.path.is_dir();
                self.set_mode(|mode| {
                    let mut bits = mode >> shift & 7;
                    bits &= !6;
                    bits |= [6, 4, 0][k.min(2)];
                    // A folder that can be read can be entered; one that cannot, cannot.
                    if dir {
                        bits = if k < 2 { bits | 1 } else { bits & !1 };
                    }
                    (mode & !(7 << shift)) | (bits << shift)
                });
            }
            Msg::Runs(on) => {
                // Running follows reading: whoever may read it may run it.
                self.set_mode(|mode| if on { mode | ((mode & 0o444) >> 2) } else { mode & !0o111 });
            }
        }
        window.redraw();
    }
}
