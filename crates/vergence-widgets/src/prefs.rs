//! An app's settings window (app settings milestone 1; whitepaper §6.10):
//! one per app, every app's alike. Its rows are the app's, in parts (the
//! app's own, then "On this desktop"); a row that has a desktop default is
//! an **inherit-or-override row**: its first choice says "Desktop default
//! (Dark)", naming what the desktop's setting is now and following it live,
//! and choosing it takes the app's own value away; a row with a value of
//! its own shows "Use the desktop's" under its name.
//!
//! What the window writes is the app's table in desktop.toml (`[terminal]`),
//! a key per row, edited line by line so the rest of the file (other
//! tables, comments) stays as it was; a key that is absent inherits. The app
//! and the server re-read the file within a second, so a change shows at
//! once, and the window re-reads it too, so a default it names follows the
//! settings app.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use crate::{Align, ButtonLook, Element, Length, Padding, Program, Theme, Waker, Window, button, choice, column, label, panel, row, rule, scroll, space, switch};

/// A value as written in desktop.toml.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Number(i64),
    /// A fraction: an opacity.
    Decimal(f32),
    Flag(bool),
}

impl Value {
    /// As it is written after `key = `.
    pub fn written(&self) -> String {
        match self {
            Value::Text(t) => format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\"")),
            Value::Number(n) => n.to_string(),
            Value::Decimal(d) => format!("{d}"),
            Value::Flag(b) => b.to_string(),
        }
    }

    /// As vergence_ui::table_value reads it back (quotes off).
    fn read(&self) -> String {
        match self {
            Value::Text(t) => t.clone(),
            Value::Number(n) => n.to_string(),
            Value::Decimal(d) => format!("{d}"),
            Value::Flag(b) => b.to_string(),
        }
    }
}

/// One choice of a row: what it says, and what it writes.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub label: String,
    pub value: Value,
}

impl Choice {
    pub fn text(label: &str, value: &str) -> Choice {
        Choice { label: label.into(), value: Value::Text(value.into()) }
    }

    pub fn number(label: &str, value: i64) -> Choice {
        Choice { label: label.into(), value: Value::Number(value) }
    }

    pub fn decimal(label: &str, value: f32) -> Choice {
        Choice { label: label.into(), value: Value::Decimal(value) }
    }
}

/// What a row holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    /// A choice. `desktop`: the first choice's words, for a row whose
    /// absent key means a default that is not one of the choices ("Desktop
    /// default (Dark)", following the desktop; "Built-in (Dark)", the app's
    /// own look); else `app_default` is the choice that holds while the key
    /// is absent.
    Choose { choices: Vec<Choice>, desktop: Option<String>, app_default: usize },
    /// On or off; absent is `default`.
    Switch { default: bool },
    /// A button the app answers ([`Rows::button`]): "Forget…".
    Button { label: String, id: u32 },
    /// A percentage typed in (an opacity: Dave, 2026-11-14, rather than a
    /// list in steps of 5), from `min` to 100, written as a fraction; empty
    /// is the default, which `desktop` names ("Desktop default (100)") and
    /// `now` is (where the slider beside it rests while the field is empty).
    Percent { min: u32, desktop: String, now: u32 },
}

/// One row: the key it writes, its name, a line saying what it does.
/// `locked`: something else decides it now (the desktop's Style forcing
/// every app): the row says what, instead of offering a choice that would
/// not take effect.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub key: String,
    pub name: String,
    pub help: String,
    pub control: Control,
    pub locked: Option<String>,
}

impl Row {
    pub fn new(key: &str, name: &str, help: &str, control: Control) -> Row {
        Row { key: key.into(), name: name.into(), help: help.into(), control, locked: None }
    }

    pub fn locked(mut self, why: Option<String>) -> Row {
        self.locked = why;
        self
    }
}

/// A part of the window, under its heading.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub title: String,
    pub rows: Vec<Row>,
}

/// What the app gives its settings window.
pub trait Rows: 'static {
    /// The parts and rows, from desktop.toml's text as it is now (a
    /// default a row names is read from it).
    fn parts(&self, desktop: &str) -> Vec<Part>;

    /// A row's button was pressed.
    fn button(&self, _id: u32) {}
}

/// The window.
pub struct Prefs {
    /// The app's name ("Terminal") and id, its table ("[terminal]"), and
    /// whether its look is dark under "App default".
    pub app: String,
    pub app_id: String,
    pub section: String,
    pub default_dark: bool,
    pub rows: Box<dyn Rows>,
    /// Set while the window is open: the app opens no second one.
    pub open: Rc<Cell<bool>>,
    /// Told after each change it writes (the app shows it at once).
    pub on_write: Option<Box<dyn Fn()>>,
    text: String,
    stamp: Option<SystemTime>,
    alive: Arc<AtomicBool>,
    /// What is typed in a percentage's field, by key, until it is a value.
    drafts: std::collections::HashMap<String, String>,
    /// Its size now, kept when it closes (Dave, 2026-11-14: "at least
    /// remember the size"), per app, in ~/.local/state/vergence/settings-windows.toml.
    size: Option<(f32, f32)>,
}

impl Prefs {
    pub fn new(app: &str, app_id: &str, section: &str, default_dark: bool, rows: Box<dyn Rows>, open: Rc<Cell<bool>>) -> Prefs {
        open.set(true);
        let mut p = Prefs { app: app.into(), app_id: app_id.into(), section: section.into(), default_dark, rows, open, on_write: None, text: String::new(), stamp: None, alive: Arc::new(AtomicBool::new(true)), drafts: Default::default(), size: None };
        p.reread();
        p
    }

    /// desktop.toml again, if it changed. True if it did.
    fn reread(&mut self) -> bool {
        let Some(path) = desktop_path() else { return false };
        let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if stamp == self.stamp && self.stamp.is_some() {
            return false;
        }
        self.stamp = stamp;
        self.text = std::fs::read_to_string(&path).unwrap_or_default();
        true
    }

    /// Writes `key` in the app's table (None: takes it away).
    fn write(&mut self, key: &str, value: Option<&Value>) {
        let Some(path) = desktop_path() else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let new = set_key(&text, &self.section, key, value.map(Value::written).as_deref());
        if new == text {
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let beside = path.with_extension("toml.new");
        if let Err(e) = std::fs::write(&beside, &new).and_then(|_| std::fs::rename(&beside, &path)) {
            eprintln!("{} settings: can't write {}: {e}", self.app_id, path.display());
            return;
        }
        self.text = new;
        self.stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if let Some(f) = &self.on_write {
            f();
        }
    }

    /// Something to call after each change it writes.
    pub fn on_write(mut self, f: impl Fn() + 'static) -> Prefs {
        self.on_write = Some(Box::new(f));
        self
    }

    fn value(&self, key: &str) -> Option<String> {
        vergence_ui::table_value(&self.text, &self.section, key)
    }

    fn row_view(&self, r: &Row, theme: &Theme) -> Element<PrefsMsg> {
        let own = self.value(&r.key);
        let mut left = vec![label(r.name.clone())];
        if !r.help.is_empty() {
            left.push(label(r.help.clone()).size(11.0).dim().wrap().width(Length::FILL));
        }
        if let Some(why) = &r.locked {
            return panel(row(vec![column(left).spacing(2.0).width(Length::FILL), label(why.clone()).size(12.0).dim().wrap().width(Length::Fixed(230.0))]).spacing(16.0).align(Align::Center).width(Length::FILL)).padding(Padding::hv(8.0, 12.0)).width(Length::FILL);
        }
        let control: Element<PrefsMsg> = match &r.control {
            Control::Choose { choices, desktop, app_default } => {
                let same = |c: &Choice, v: &str| match c.value {
                    Value::Decimal(d) => v.parse::<f32>().is_ok_and(|x| (x - d).abs() < 0.001),
                    _ => c.value.read() == v,
                };
                // A value of the app's own that is not among the choices (set by
                // hand: an opacity of 0.97) is shown as itself, last, and kept.
                let mut choices = choices.clone();
                if let Some(v) = own.as_ref().filter(|v| !choices.iter().any(|c| same(c, v))) {
                    let shown = match (v.parse::<f32>(), choices.first().map(|c| &c.value)) {
                        (Ok(x), Some(Value::Decimal(_))) => Choice::decimal(&format!("{:.0}%", x * 100.0), x),
                        (Ok(x), Some(Value::Number(_))) if x.fract() == 0.0 => Choice::number(v, x as i64),
                        _ => Choice::text(v, v),
                    };
                    choices.push(shown);
                }
                let at = own.as_ref().and_then(|v| choices.iter().position(|c| same(c, v)));
                let mut names: Vec<String> = Vec::new();
                let mut values: Vec<Option<Value>> = Vec::new();
                if let Some(d) = desktop {
                    names.push(d.clone());
                    values.push(None);
                }
                for (i, c) in choices.iter().enumerate() {
                    names.push(c.label.clone());
                    // The app's own default writes nothing: absent is that.
                    values.push(if desktop.is_none() && i == *app_default { None } else { Some(c.value.clone()) });
                }
                let selected = match (desktop.is_some(), at) {
                    (true, None) => Some(0),
                    (true, Some(i)) => Some(i + 1),
                    (false, None) => Some(*app_default),
                    (false, Some(i)) => Some(i),
                };
                if let (Some(d), Some(_)) = (desktop, at) {
                    let key = r.key.clone();
                    let back = if d.starts_with("Built-in") { "Use the built-in" } else { "Use the desktop's" };
                    left.push(
                        row(vec![
                            button(label(back).size(11.0).color(theme.accent)).look(ButtonLook::Plain).padding(Padding::hv(0.0, 0.0)).on_press(PrefsMsg::Set(key, None)).name(format!("{}: {}", r.name, back.to_lowercase())),
                            space().width(Length::FILL),
                        ])
                        .width(Length::FILL),
                    );
                }
                let key = r.key.clone();
                choice(names, selected, move |i| PrefsMsg::Set(key.clone(), values.get(i).cloned().flatten())).width(Length::Fixed(230.0)).name(r.name.clone())
            }
            Control::Switch { default } => {
                let on = own.as_deref().map_or(*default, |v| v == "true");
                let (key, default) = (r.key.clone(), *default);
                switch(on, move |b| PrefsMsg::Set(key.clone(), (b != default).then_some(Value::Flag(b)))).name(r.name.clone())
            }
            Control::Button { label: words, id } => button(label(words.clone()).size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(PrefsMsg::Button(*id)).name(words.clone()),
            Control::Percent { min, desktop, now } => {
                let shown = self.drafts.get(&r.key).cloned().unwrap_or_else(|| own.as_deref().and_then(|v| v.parse::<f32>().ok()).map(|x| format!("{:.0}", x * 100.0)).unwrap_or_default());
                if own.is_none() {
                    left.push(label(desktop.clone()).size(11.0).dim());
                }
                if own.is_some() {
                    let key = r.key.clone();
                    left.push(
                        row(vec![
                            button(label("Use the desktop's").size(11.0).color(theme.accent)).look(ButtonLook::Plain).padding(Padding::hv(0.0, 0.0)).on_press(PrefsMsg::Set(key, None)).name(format!("{}: use the desktop's", r.name)),
                            space().width(Length::FILL),
                        ])
                        .width(Length::FILL),
                    );
                }
                let help = format!("{min} to 100");
                let (key, min) = (r.key.clone(), *min);
                // A slider for a quick change (Dave: "could be a slider too"), the
                // field beside it for an exact one; the slider writes when let go.
                let at = shown.parse::<u32>().ok().filter(|n| (min..=100).contains(n)).unwrap_or(*now) as f32;
                let (k1, k2) = (key.clone(), key.clone());
                row(vec![
                    crate::slider(min as f32, 100.0, at, move |v| PrefsMsg::Slid(k1.clone(), v.round() as u32)).step(1.0).on_release(PrefsMsg::Let(k2, min)).width(Length::Fixed(150.0)).name(format!("{} slider", r.name)),
                    crate::field(shown, move |t| PrefsMsg::Typed(key.clone(), t, min)).placeholder(now.to_string()).width(Length::Fixed(64.0)).name(format!("{} ({help})", r.name)),
                    label("%").size(13.0),
                ])
                .spacing(6.0)
                .align(Align::Center)
                .into()
            }
        };
        panel(row(vec![column(left).spacing(2.0).width(Length::FILL), control]).spacing(16.0).align(Align::Center).width(Length::FILL)).padding(Padding::hv(8.0, 12.0)).width(Length::FILL)
    }
}

/// What the window's widgets say.
#[derive(Debug, Clone)]
pub enum PrefsMsg {
    Set(String, Option<Value>),
    Button(u32),
    /// A percentage's field: the key, what is typed, the least allowed.
    Typed(String, String, u32),
    /// Its slider moved, and was let go.
    Slid(String, u32),
    Let(String, u32),
}

impl Program for Prefs {
    type Message = PrefsMsg;

    fn title(&self) -> String {
        // "Terminal Settings"; the settings app's own is not "Settings Settings".
        if self.app == "Settings" { "Settings for the Settings app".into() } else { format!("{} Settings", self.app) }
    }

    fn app_id(&self) -> String {
        self.app_id.clone()
    }

    /// Light, whatever the app's own look, unless the desktop's Style forces
    /// Dark (Dave, 2026-11-14: settings are hard to read dark with small
    /// text); not the app's opacity either: no section of its own.
    fn theme(&self) -> Theme {
        Theme::from_desktop(&self.text, false, "[settings window]")
    }

    /// Its bar light too, unless the desktop's Style forces one.
    fn bar_style(&self) -> Option<vergence_ui::BarStyle> {
        Some(vergence_ui::bar_style_setting(&self.text, "[settings window]", vergence_ui::BarStyle::Light))
    }

    fn view(&self, theme: &Theme) -> Element<PrefsMsg> {
        let mut all: Vec<Element<PrefsMsg>> = Vec::new();
        for part in self.rows.parts(&self.text) {
            let mut inner = Vec::new();
            for (i, r) in part.rows.iter().enumerate() {
                if i > 0 {
                    inner.push(panel(rule()).padding(Padding::hv(0.0, 12.0)).width(Length::FILL));
                }
                inner.push(self.row_view(r, theme));
            }
            let card = panel(column(inner).width(Length::FILL)).color(theme.card).radius(10.0).padding(2.0).width(Length::FILL);
            all.push(column(vec![label(part.title).size(12.0).dim().weight(600), card]).spacing(6.0).width(Length::FILL));
        }
        scroll(panel(column(all).spacing(18.0).width(Length::FILL)).padding(Padding { top: 18.0, bottom: 22.0, left: 20.0, right: 20.0 }).width(Length::FILL)).name(format!("{} Settings", self.app))
    }

    fn update(&mut self, m: PrefsMsg, window: &mut Window) {
        match m {
            PrefsMsg::Set(key, value) => {
                self.drafts.remove(&key);
                self.write(&key, value.as_ref());
            }
            PrefsMsg::Button(id) => self.rows.button(id),
            // Written as soon as it is a value in range; empty is the default.
            PrefsMsg::Typed(key, typed, min) => {
                let digits: String = typed.chars().filter(|c| c.is_ascii_digit()).take(3).collect();
                match percent_of(&digits, min) {
                    Some(v) => self.write(&key, Some(&Value::Decimal(v))),
                    None if digits.is_empty() => self.write(&key, None),
                    None => {}
                }
                self.drafts.insert(key, digits);
            }
            PrefsMsg::Slid(key, n) => {
                self.drafts.insert(key, n.to_string());
            }
            PrefsMsg::Let(key, min) => {
                if let Some(v) = self.drafts.get(&key).and_then(|d| percent_of(d, min)) {
                    self.write(&key, Some(&Value::Decimal(v)));
                }
            }
        }
        window.redraw();
    }

    /// desktop.toml is looked at once a second while the window is open, so
    /// the defaults it names follow the settings app.
    fn waker(&mut self, waker: Waker) {
        let alive = self.alive.clone();
        let _ = std::thread::Builder::new().name("prefs-watch".into()).spawn(move || {
            let mut last = desktop_path().and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
            while alive.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(1));
                let now = desktop_path().and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
                if now != last {
                    last = now;
                    waker.wake();
                }
            }
        });
    }

    fn woken(&mut self, _window: &mut Window) -> bool {
        self.reread()
    }

    fn style(&self) -> vergence_ui::Style {
        // Small enough to be made short (it scrolls); it opens as tall as its rows
        // want, or at the size it was last left at.
        vergence_ui::Style { kind: vergence_ui::WinKind::Dialog, modal: false, min: (520.0, 260.0), max: (900.0, 0.0), alone: false, frosted: false }
    }

    /// It opens at a guess from its rows (the width kept from before), then
    /// fits its height to what it holds (Dave, 2026-11-16: "size to content,
    /// then window space"; the guess came out too tall once the font
    /// changed). The server keeps it on the screen, where a long one scrolls.
    fn opening_size(&self) -> Option<(u32, u32)> {
        let parts = self.rows.parts(&self.text);
        let rows: usize = parts.iter().map(|p| p.rows.len()).sum();
        let tall = (60.0 + parts.len() as f64 * 44.0 + rows as f64 * 56.0).clamp(300.0, 900.0).round() as u32;
        Some((kept_size(&self.app_id).map_or(580, |(w, _)| w), tall))
    }

    fn fit_content(&self) -> bool {
        true
    }

    fn resized(&mut self, width: f32, height: f32) {
        if width > 0.0 && height > 0.0 {
            self.size = Some((width, height));
        }
    }
}

impl Drop for Prefs {
    fn drop(&mut self) {
        self.open.set(false);
        self.alive.store(false, Ordering::Relaxed);
        if let Some((w, h)) = self.size {
            keep_size(&self.app_id, w.round() as u32, h.round() as u32);
        }
    }
}

/// Where settings windows' last sizes are kept (owner only).
fn sizes_path() -> Option<std::path::PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))?;
    Some(state.join("vergence/settings-windows.toml"))
}

/// An app's settings window's last size: a line `app-id = "580x700"`.
fn kept_size(app: &str) -> Option<(u32, u32)> {
    let text = std::fs::read_to_string(sizes_path()?).ok()?;
    sizes_from(&text, app)
}

fn sizes_from(text: &str, app: &str) -> Option<(u32, u32)> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        if k.trim().trim_matches('"') != app {
            return None;
        }
        let (w, h) = v.trim().trim_matches('"').split_once('x')?;
        let (w, h) = (w.parse::<u32>().ok()?, h.parse::<u32>().ok()?);
        ((200..=8000).contains(&w) && (200..=8000).contains(&h)).then_some((w, h))
    })
}

fn keep_size(app: &str, w: u32, h: u32) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let Some(path) = sizes_path() else { return };
    if app.is_empty() || app.contains(['"', '\n', '=']) {
        return;
    }
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = old.lines().filter(|l| l.split_once('=').is_none_or(|(k, _)| k.trim().trim_matches('"') != app)).map(String::from).collect();
    lines.push(format!("\"{app}\" = \"{w}x{h}\""));
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let beside = path.with_extension("toml.new");
    let text = lines.join("\n") + "\n";
    let _ = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&beside).and_then(|mut f| f.write_all(text.as_bytes())).and_then(|_| std::fs::rename(&beside, &path));
}

/// What a percentage's field holds, as a fraction, if it is `min` to 100.
fn percent_of(digits: &str, min: u32) -> Option<f32> {
    let n: u32 = digits.parse().ok()?;
    (min..=100).contains(&n).then(|| n as f32 / 100.0)
}

/// ~/.config/vergence/desktop.toml.
pub fn desktop_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("vergence/desktop.toml"))
}

/// desktop.toml's text with `key` in table `section` ("[terminal]") set to
/// `value` (as written after `= `), or taken away (None). Line by line:
/// every other line, comments included, stays as it was; a missing table is
/// added at the end.
pub fn set_key(text: &str, section: &str, key: &str, value: Option<&str>) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|l| l.trim() == section);
    let Some(start) = start else {
        let Some(v) = value else { return text.to_string() };
        let mut out = text.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("{section}\n{key} = {v}\n"));
        return out;
    };
    let end = lines.iter().enumerate().skip(start + 1).find(|(_, l)| l.trim_start().starts_with('[')).map_or(lines.len(), |(i, _)| i);
    let found = (start + 1..end).find(|&i| lines[i].split_once('=').is_some_and(|(k, _)| k.trim() == key) && !lines[i].trim_start().starts_with('#'));
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    match (found, value) {
        (Some(i), Some(v)) => out[i] = format!("{key} = {v}"),
        (Some(i), None) => {
            out.remove(i);
        }
        (None, Some(v)) => {
            // After the table's last line that is not blank.
            let mut at = end;
            while at > start + 1 && out[at - 1].trim().is_empty() {
                at -= 1;
            }
            out.insert(at, format!("{key} = {v}"));
        }
        (None, None) => return text.to_string(),
    }
    let mut s = out.join("\n");
    if text.ends_with('\n') || text.is_empty() {
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_set_changed_and_taken_away_and_the_rest_stays() {
        let t = "# mine\n[appearance]\ncolor-scheme = \"dark\"\n\n[terminal]\nopacity = 0.97 # see-through\nscheme = \"tango\"\n\n[[touch.panel]]\nname = \"x\"\n";
        let a = set_key(t, "[terminal]", "style", Some("\"light\""));
        assert_eq!(a, "# mine\n[appearance]\ncolor-scheme = \"dark\"\n\n[terminal]\nopacity = 0.97 # see-through\nscheme = \"tango\"\nstyle = \"light\"\n\n[[touch.panel]]\nname = \"x\"\n");
        let b = set_key(&a, "[terminal]", "style", Some("\"dark\""));
        assert!(b.contains("style = \"dark\"") && !b.contains("light"));
        let c = set_key(&b, "[terminal]", "style", None);
        assert_eq!(c, t, "back as it was");
        assert_eq!(set_key(t, "[terminal]", "nothing", None), t);
        // A table that is not there is added; another table's key of the same name is left.
        let d = set_key(t, "[settings]", "opacity", Some("0.9"));
        assert!(d.ends_with("name = \"x\"\n\n[settings]\nopacity = 0.9\n"));
        assert!(d.contains("opacity = 0.97 # see-through"));
        assert_eq!(set_key("", "[terminal]", "style", Some("\"dark\"")), "[terminal]\nstyle = \"dark\"\n");
    }

    #[test]
    fn a_kept_size_is_read_back_for_its_app_only() {
        let t = "\"vergence-term\" = \"600x720\"\n\"vergence-settings\" = \"540x320\"\n\"odd\" = \"9x9\"\n";
        assert_eq!(sizes_from(t, "vergence-term"), Some((600, 720)));
        assert_eq!(sizes_from(t, "vergence-settings"), Some((540, 320)));
        assert_eq!(sizes_from(t, "odd"), None, "too small to be real");
        assert_eq!(sizes_from(t, "firefox"), None);
    }

    #[test]
    fn a_percentage_is_a_value_only_in_its_range() {
        assert_eq!(percent_of("85", 30), Some(0.85));
        assert_eq!(percent_of("100", 30), Some(1.0));
        assert_eq!(percent_of("8", 30), None, "on its way to 85");
        assert_eq!(percent_of("101", 30), None);
        assert_eq!(percent_of("", 30), None);
    }

    #[test]
    fn values_are_written_as_toml_reads_them() {
        assert_eq!(Value::Text("a \"b\"".into()).written(), "\"a \\\"b\\\"\"");
        assert_eq!(Value::Number(10000).written(), "10000");
        assert_eq!(Value::Flag(false).written(), "false");
    }
}
