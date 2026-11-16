//! The app menu (app settings milestone 1): what opens from a window's app
//! button, for every app. One rule for every app (Dave, 2026-11-14): the
//! app button is the app, a right click is the thing under the pointer (a
//! pane, a tab, a selection), the title bar is the window. So the menu holds
//! only what an app puts in it (its `featured` commands: New window, New
//! tab, Text size), the ones the user added, then the skeleton:
//!
//! 1. Search commands… (only for an app with many commands);
//! 2. the app's commands and the user's (starred); Zoom in, Zoom out and
//!    Actual size together as one Text size row, − 100% +, that leaves the
//!    menu open (a while's size; the lasting one is in the app's settings);
//! 3. Settings…, Keyboard shortcuts › (every command of the app under its
//!    menu's heading with its keys, chosen from there too, a right click
//!    adding one to the menu), About <App>, Quit <App>.
//!
//! An app that is not ours (no commands) gets the skeleton with a line saying
//! its own menu holds the rest. Everything here is plain data: the server
//! draws it and carries out what is chosen ([`Action`]).

use vergence_protocol::commands::{Command, MENUS, ROLES, Role};
use vergence_protocol::window::MenuLine;

use crate::Rect;
use crate::snap::Zone;

/// What a line does when chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// One of the app's commands: the app hears its id.
    Command(u32),
    /// The skeleton's own lines, for an app that did not send them: the
    /// desktop carries them out (its About, its settings window, closing
    /// every window of the app).
    About,
    Settings,
    Quit,
    /// The Window menu's.
    Window(WindowAct),
}

/// What the Window menu does: the layouts menu's (the server's menu.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAct {
    Snap(Zone),
    Maximize,
    Restore,
    Minimize,
    Close,
}

/// One line of a menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// The search field (top level only).
    Search,
    /// `pin`: what names the command across runs, for adding it to the
    /// first panel or taking it off (None: not a command of the app's);
    /// `pinned`: the user added it there.
    Item { action: Action, label: String, keys: String, enabled: bool, checked: Option<bool>, pin: Option<String>, pinned: bool },
    /// A submenu, by its name.
    Sub(String),
    /// A menu's name over its commands, in Keyboard shortcuts.
    Header(String),
    /// Text size: its three commands (smaller, as it was, larger) and what
    /// it is now ("110%", the Actual size command's label as the app sent it).
    Zoom { out: u32, reset: u32, grow: u32, now: String },
    /// A line of words, not chosen (a foreign app's note).
    Note(String),
    Separator,
}

impl Line {
    /// Whether the keyboard and the pointer can rest on it.
    pub fn selectable(&self) -> bool {
        matches!(self, Line::Item { enabled: true, .. } | Line::Sub(_) | Line::Zoom { .. })
    }

    pub fn height(&self) -> i32 {
        match self {
            Line::Search => SEARCH_H,
            Line::Separator => SEP_H,
            Line::Note(_) => NOTE_H,
            Line::Header(_) => HEADER_H,
            _ => ITEM_H,
        }
    }
}

/// Sizes in points.
pub const WIDTH: i32 = 280;
pub const ITEM_H: i32 = 28;
pub const SEARCH_H: i32 = 36;
pub const SEP_H: i32 = 9;
pub const NOTE_H: i32 = 40;
pub const HEADER_H: i32 = 24;
/// The submenu of every command, with its keys.
pub const ALL: &str = "Keyboard shortcuts";
/// An app with more commands than this has a search field at the top (the
/// terminal's 14 do not need one; typing searches anyway).
pub const SMALL: usize = 24;
pub const PAD: i32 = 6;
/// At most this many results while searching.
pub const MAX_FOUND: usize = 12;

/// Everything a window's app menu is made from.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// The app's name, as its About and Quit say it.
    pub app: String,
    pub commands: Vec<Command>,
    /// Its Ctrl+letter keys are its programs' (crate::commands::keys_for).
    pub terminal: bool,
    /// Whether it is one of ours (it speaks the native protocol): if not,
    /// its own menu holds its commands.
    pub native: bool,
    /// The Window menu's lines, as the server offers them for this window.
    pub window: Vec<(WindowAct, String)>,
    /// The commands the user added to the first panel, by their keys
    /// (Command::key), in the order added.
    pub pins: Vec<String>,
    /// A window's own right-click menu (bus v20 `WinRequest::Menu`): these
    /// lines are the whole menu, with no skeleton, search or pins.
    pub given: Option<Vec<MenuLine>>,
}

/// A given line as the menu draws it.
fn given_line(l: &MenuLine) -> Line {
    match l {
        MenuLine::Item { id, label, keys, enabled, checked } => Line::Item { action: Action::Command(*id), label: label.clone(), keys: keys.clone(), enabled: *enabled, checked: *checked, pin: None, pinned: false },
        MenuLine::Separator => Line::Separator,
        MenuLine::Sub { label, .. } => Line::Sub(label.clone()),
    }
}

impl Model {
    fn role(&self, role: Role) -> Option<&Command> {
        self.commands.iter().find(|c| c.role == role)
    }

    fn item(&self, c: &Command) -> Line {
        let key = c.key();
        let pinned = !c.featured && self.pins.contains(&key);
        Line::Item { action: Action::Command(c.id), label: c.label_shown(&self.app), keys: c.keys_shown(self.terminal), enabled: c.enabled, checked: c.checked, pin: Some(key), pinned }
    }

    /// A skeleton line: the app's command for the role if it sent one, else the desktop's.
    fn skeleton(&self, role: Role, own: Action) -> Line {
        match self.role(role) {
            // The skeleton's lines are always there: nothing to add or take off.
            Some(c) => match self.item(c) {
                Line::Item { action, label, keys, enabled, checked, .. } => Line::Item { action, label, keys, enabled, checked, pin: None, pinned: false },
                other => other,
            },
            None => {
                let info = role.info().expect("a role of the table");
                Line::Item { action: own, label: info.label.replace("{app}", &self.app), keys: String::new(), enabled: true, checked: None, pin: None, pinned: false }
            }
        }
    }

    /// The menus the app fills, in their order: the standard ones it uses,
    /// then its own in the order it first names them.
    pub fn menus(&self) -> Vec<String> {
        let mut out: Vec<String> = MENUS.iter().filter(|m| self.commands.iter().any(|c| c.menu_shown() == **m)).map(|m| m.to_string()).collect();
        for c in &self.commands {
            let m = c.menu_shown();
            if !m.is_empty() && !out.iter().any(|o| o == m) {
                out.push(m.to_string());
            }
        }
        out
    }

    /// The commands that go in menus (not the skeleton's About, Settings, Quit).
    fn menu_commands(&self) -> impl Iterator<Item = &Command> {
        self.commands.iter().filter(|c| !c.menu_shown().is_empty())
    }

    /// The first panel's own commands: the app's featured ones in the order it
    /// sent them, then those the user added in the order added.
    pub fn everyday(&self) -> Vec<Line> {
        let mut lines: Vec<Line> = self.menu_commands().filter(|c| c.featured).map(|c| self.item(c)).collect();
        for key in &self.pins {
            if let Some(c) = self.menu_commands().find(|c| !c.featured && c.key() == *key) {
                lines.push(self.item(c));
            }
        }
        lines
    }

    /// The Text size row, if the app put Zoom in and Zoom out in its menu.
    fn zoom(&self) -> Option<Line> {
        let find = |r: Role| self.commands.iter().find(|c| c.role == r);
        let (out, grow) = (find(Role::ZoomOut)?, find(Role::ZoomIn)?);
        if !(out.featured || grow.featured) {
            return None;
        }
        let reset = find(Role::ZoomReset);
        Some(Line::Zoom { out: out.id, reset: reset.map_or(0, |c| c.id), grow: grow.id, now: reset.map(|c| c.label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| "Text size".into()) })
    }

    /// The top level.
    pub fn top(&self) -> Vec<Line> {
        if let Some(given) = &self.given {
            return tidy(given.iter().map(given_line).collect());
        }
        let mut lines = Vec::new();
        if self.native && self.menu_commands().count() > SMALL {
            lines.push(Line::Search);
        }
        if self.native {
            let zoom = self.zoom();
            let zooming = |l: &Line| zoom.is_some() && matches!(l, Line::Item { action: Action::Command(id), .. } if self.commands.iter().any(|c| c.id == *id && matches!(c.role, Role::ZoomIn | Role::ZoomOut | Role::ZoomReset)));
            lines.extend(self.everyday().into_iter().filter(|l| !zooming(l)));
            lines.extend(zoom);
        } else {
            lines.push(Line::Note(format!("{} keeps its own settings and commands in its own menu.", self.app)));
        }
        if !self.window.is_empty() {
            lines.push(Line::Separator);
            lines.push(Line::Sub("Window".into()));
        }
        lines.push(Line::Separator);
        lines.push(self.skeleton(Role::Settings, Action::Settings));
        if self.native && self.menu_commands().next().is_some() {
            lines.push(Line::Sub(ALL.into()));
        }
        lines.push(self.skeleton(Role::About, Action::About));
        lines.push(self.skeleton(Role::Quit, Action::Quit));
        tidy(lines)
    }

    /// One menu's commands: the roles in the table's order, then the app's
    /// own in the order it sent them.
    fn menu(&self, name: &str) -> Vec<Line> {
        let mut lines: Vec<Line> = ROLES.iter().filter_map(|r| self.commands.iter().find(|c| c.role == r.role && r.menu == name)).map(|c| self.item(c)).collect();
        lines.extend(self.commands.iter().filter(|c| c.role == Role::Own && c.menu == name).map(|c| self.item(c)));
        lines
    }

    /// A submenu's lines: All commands (each menu under its heading) or Window.
    pub fn sub(&self, name: &str) -> Vec<Line> {
        if let Some(given) = &self.given {
            return given
                .iter()
                .find_map(|l| match l {
                    MenuLine::Sub { label, lines } if label == name => Some(tidy(lines.iter().map(given_line).collect())),
                    _ => None,
                })
                .unwrap_or_default();
        }
        if name == "Window" {
            return self.window.iter().map(|(a, l)| Line::Item { action: Action::Window(*a), label: l.clone(), keys: String::new(), enabled: true, checked: None, pin: None, pinned: false }).collect();
        }
        if name != ALL {
            return Vec::new();
        }
        let mut lines = Vec::new();
        for m in self.menus() {
            lines.push(Line::Header(m.clone()));
            lines.extend(self.menu(&m));
        }
        lines
    }

    /// Search: every line that can be chosen, in every menu, whose label
    /// holds each typed word (case aside); each labeled with its menu. At
    /// most [`MAX_FOUND`].
    pub fn search(&self, typed: &str) -> Vec<(Line, String)> {
        let words: Vec<String> = typed.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Vec::new();
        }
        let mut all: Vec<(Line, String)> = Vec::new();
        if self.native {
            for m in self.menus() {
                all.extend(self.menu(&m).into_iter().map(|l| (l, m.clone())));
            }
        }
        all.extend(self.sub("Window").into_iter().map(|l| (l, "Window".to_string())));
        all.extend(self.top().into_iter().filter(|l| matches!(l, Line::Item { pin: None, .. })).map(|l| (l, String::new())));
        all.retain(|(l, _)| match l {
            Line::Item { label, enabled: true, .. } => {
                let label = label.to_lowercase();
                words.iter().all(|w| label.contains(w.as_str()))
            }
            _ => false,
        });
        all.truncate(MAX_FOUND);
        all
    }
}

/// No separator first, last or twice.
fn tidy(lines: Vec<Line>) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::with_capacity(lines.len());
    for l in lines {
        if l == Line::Separator && matches!(out.last(), None | Some(Line::Separator) | Some(Line::Search)) {
            continue;
        }
        out.push(l);
    }
    while out.last() == Some(&Line::Separator) {
        out.pop();
    }
    out
}

/// Where each line sits in a panel whose top left is `at`.
pub fn rows(at: (i32, i32), lines: &[Line]) -> Vec<Rect> {
    let mut y = at.1 + PAD;
    lines
        .iter()
        .map(|l| {
            let r = Rect::new(at.0 + PAD, y, WIDTH - 2 * PAD, l.height());
            y += l.height();
            r
        })
        .collect()
}

/// The Text size row's three buttons (smaller, the size now, larger) in a
/// row's rectangle, at its right end; its name is written at its left.
pub fn zoom_parts(r: Rect) -> [Rect; 3] {
    let (b, mid, gap) = (30, 58, 2);
    let right = r.x + r.w - 4;
    let grow = Rect::new(right - b, r.y + 2, b, r.h - 4);
    let now = Rect::new(grow.x - gap - mid, r.y + 2, mid, r.h - 4);
    let out = Rect::new(now.x - gap - b, r.y + 2, b, r.h - 4);
    [out, now, grow]
}

/// A panel's height for its lines.
pub fn height(lines: &[Line]) -> i32 {
    lines.iter().map(Line::height).sum::<i32>() + 2 * PAD
}

/// The open menu: what is typed, which line is chosen, which submenu is
/// open and whether the keyboard is in it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    pub typed: String,
    /// The chosen line of the top level (or of the results while searching).
    pub top: Option<usize>,
    /// The open submenu: the top-level line it hangs from.
    pub open: Option<usize>,
    /// The chosen line of the open submenu, when the keyboard is in it.
    pub sub: Option<usize>,
}

/// A key the menu understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    Back,
    Char(char),
}

/// What a key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Did {
    Nothing,
    Moved,
    Chosen(Action),
    Close,
}

impl State {
    /// The lines the top panel shows now: the top level, or the search field
    /// and what it found.
    pub fn shown(&self, m: &Model) -> Vec<Line> {
        if self.typed.trim().is_empty() || m.given.is_some() {
            m.top()
        } else {
            std::iter::once(Line::Search).chain(m.search(&self.typed).into_iter().map(|(l, _)| l)).collect()
        }
    }

    /// The open submenu's lines.
    pub fn sub_lines(&self, m: &Model) -> Vec<Line> {
        match self.open.and_then(|i| self.shown(m).get(i).cloned()) {
            Some(Line::Sub(name)) => m.sub(&name),
            _ => Vec::new(),
        }
    }

    /// A key.
    pub fn key(&mut self, m: &Model, key: Key) -> Did {
        let lines = self.shown(m);
        if let Some(s) = self.sub {
            let sub = self.sub_lines(m);
            return match key {
                Key::Up | Key::Down => {
                    self.sub = step(&sub, Some(s), key == Key::Down);
                    Did::Moved
                }
                Key::Left | Key::Escape => {
                    self.sub = None;
                    Did::Moved
                }
                Key::Enter | Key::Right => match sub.get(s) {
                    Some(Line::Item { action, enabled: true, .. }) => Did::Chosen(*action),
                    _ => Did::Nothing,
                },
                Key::Char(_) | Key::Back => {
                    self.sub = None;
                    self.open = None;
                    self.key(m, key)
                }
            };
        }
        match key {
            Key::Up | Key::Down => {
                self.top = step(&lines, self.top, key == Key::Down);
                self.open = self.top.filter(|i| matches!(lines.get(*i), Some(Line::Sub(_))));
                Did::Moved
            }
            Key::Left | Key::Right | Key::Enter if matches!(self.top.and_then(|i| lines.get(i)), Some(Line::Zoom { .. })) => match (key, self.top.and_then(|i| lines.get(i))) {
                (Key::Left, Some(Line::Zoom { out, .. })) => Did::Chosen(Action::Command(*out)),
                (Key::Right, Some(Line::Zoom { grow, .. })) => Did::Chosen(Action::Command(*grow)),
                (Key::Enter, Some(Line::Zoom { reset, .. })) if *reset != 0 => Did::Chosen(Action::Command(*reset)),
                _ => Did::Nothing,
            },
            Key::Right | Key::Enter => match self.top.and_then(|i| lines.get(i)) {
                Some(Line::Sub(_)) => {
                    self.open = self.top;
                    let sub = self.sub_lines(m);
                    self.sub = step(&sub, None, true);
                    Did::Moved
                }
                Some(Line::Item { action, enabled: true, .. }) if key == Key::Enter => Did::Chosen(*action),
                _ => Did::Nothing,
            },
            Key::Left => Did::Nothing,
            Key::Escape if !self.typed.is_empty() => {
                self.typed.clear();
                (self.top, self.open) = (None, None);
                Did::Moved
            }
            Key::Escape => Did::Close,
            // A right-click menu has no search.
            Key::Char(_) | Key::Back if m.given.is_some() => Did::Nothing,
            Key::Back => {
                self.typed.pop();
                self.after_typing(m);
                Did::Moved
            }
            Key::Char(c) if !c.is_control() && self.typed.chars().count() < 60 => {
                self.typed.push(c);
                self.after_typing(m);
                Did::Moved
            }
            Key::Char(_) => Did::Nothing,
        }
    }

    /// Searching chooses the first result (Enter takes it).
    fn after_typing(&mut self, m: &Model) {
        let lines = self.shown(m);
        self.open = None;
        self.sub = None;
        self.top = if self.typed.trim().is_empty() { None } else { step(&lines, None, true) };
    }

    /// The pointer is over line `i` of the top panel: it is chosen, and a
    /// submenu there opens.
    pub fn hover_top(&mut self, m: &Model, i: Option<usize>) {
        let lines = self.shown(m);
        match i.filter(|i| lines.get(*i).is_some_and(Line::selectable)) {
            Some(i) => {
                self.top = Some(i);
                self.sub = None;
                self.open = matches!(lines[i], Line::Sub(_)).then_some(i);
            }
            None => {
                if self.open.is_none() {
                    self.top = None;
                }
            }
        }
    }

    /// The pointer is over line `i` of the open submenu.
    pub fn hover_sub(&mut self, m: &Model, i: Option<usize>) {
        let sub = self.sub_lines(m);
        self.sub = i.filter(|i| sub.get(*i).is_some_and(Line::selectable));
    }
}

/// The next line that can be chosen, down or up, wrapping round.
fn step(lines: &[Line], from: Option<usize>, down: bool) -> Option<usize> {
    let n = lines.len();
    if n == 0 {
        return None;
    }
    let mut i = match (from, down) {
        (None, true) => n - 1,
        (None, false) => 0,
        (Some(i), _) => i,
    };
    for _ in 0..n {
        i = if down { (i + 1) % n } else { (i + n - 1) % n };
        if lines[i].selectable() {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal() -> Model {
        Model {
            app: "Terminal".into(),
            commands: vec![
                Command::role(1, Role::Copy).enabled(false),
                Command::role(2, Role::Paste),
                Command::role(3, Role::NewTab).featured(),
                Command::own(4, "Tab", "Split side by side", "Ctrl+Shift+R"),
                Command::role(5, Role::Settings),
                Command::role(6, Role::NextTab),
                Command::own(7, "Shell", "Clear", ""),
                Command::role(8, Role::NewWindow).featured(),
                Command::role(9, Role::ZoomOut).featured(),
                Command { label: "110%".into(), ..Command::role(10, Role::ZoomReset).featured() },
                Command::role(11, Role::ZoomIn).featured(),
            ],
            terminal: true,
            native: true,
            window: Vec::new(),
            pins: Vec::new(),
            given: None,
        }
    }

    fn labels(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| match l {
                Line::Search => "[search]".into(),
                Line::Item { label, keys, pinned, .. } => {
                    let star = if *pinned { " *" } else { "" };
                    if keys.is_empty() { format!("{label}{star}") } else { format!("{label} ({keys}){star}") }
                }
                Line::Sub(n) => format!("{n} >"),
                Line::Header(n) => format!("# {n}"),
                Line::Note(_) => "[note]".into(),
                Line::Zoom { now, .. } => format!("[- {now} +]"),
                Line::Separator => "---".into(),
            })
            .collect()
    }

    /// The terminal with enough commands of its own to have a search field.
    fn big() -> Model {
        let mut m = terminal();
        for i in 0..20 {
            m.commands.push(Command::own(100 + i, "Shell", &format!("Thing {i}"), ""));
        }
        m
    }

    #[test]
    fn a_right_click_menu_is_its_lines_alone() {
        use vergence_protocol::window::MenuLine;
        let mut m = big();
        m.given = Some(vec![
            MenuLine::Separator,
            MenuLine::item(1, "Open", "Enter"),
            MenuLine::Sub { label: "Open With".into(), lines: vec![MenuLine::item(10, "Text Editor", ""), MenuLine::item(11, "Close-Up", "")] },
            MenuLine::Separator,
            MenuLine::Separator,
            MenuLine::item(2, "Rename…", "F2").enabled(false),
            MenuLine::Separator,
        ]);
        assert_eq!(labels(&m.top()), ["Open (Enter)", "Open With >", "---", "Rename… (F2)"]);
        assert_eq!(labels(&m.sub("Open With")), ["Text Editor", "Close-Up"]);
        assert!(m.sub("Keyboard shortcuts").is_empty());
        // No search: typing does nothing; the keys walk past the faint line.
        let mut s = State::default();
        assert_eq!(s.key(&m, Key::Char('x')), Did::Nothing);
        assert_eq!(labels(&s.shown(&m)), labels(&m.top()));
        s.key(&m, Key::Down);
        assert_eq!(s.key(&m, Key::Enter), Did::Chosen(Action::Command(1)));
        s.key(&m, Key::Down);
        s.key(&m, Key::Right);
        s.key(&m, Key::Down);
        assert_eq!(s.key(&m, Key::Enter), Did::Chosen(Action::Command(11)));
        s.key(&m, Key::Left);
        s.key(&m, Key::Down);
        assert_eq!(s.top, Some(0), "wrapped past the faint Rename");
    }

    #[test]
    fn the_menu_is_the_apps_own_few_then_the_skeleton() {
        let m = terminal();
        assert_eq!(labels(&m.top()), ["New tab (Ctrl+Shift+T)", "New window (Ctrl+Shift+N)", "[- 110% +]", "---", "Settings… (Ctrl+,)", "Keyboard shortcuts >", "About Terminal", "Quit Terminal"]);
        assert_eq!(
            labels(&m.sub(ALL)),
            [
                "# File", "New window (Ctrl+Shift+N)", "New tab (Ctrl+Shift+T)", "# Edit", "Copy (Ctrl+C)", "Paste (Ctrl+V)", "# View", "Zoom in (Ctrl+=)", "Zoom out (Ctrl+-)", "Actual size (Ctrl+0)", "# Tab", "Next tab (Ctrl+Page Down)", "Split side by side (Ctrl+Shift+R)", "# Shell", "Clear"
            ]
        );
        assert!(matches!(&big().top()[0], Line::Search), "many commands: a search field");
    }

    #[test]
    fn the_text_size_row_takes_left_right_and_enter() {
        let m = terminal();
        let mut s = State::default();
        s.hover_top(&m, Some(2));
        assert_eq!(s.key(&m, Key::Left), Did::Chosen(Action::Command(9)));
        assert_eq!(s.key(&m, Key::Right), Did::Chosen(Action::Command(11)));
        assert_eq!(s.key(&m, Key::Enter), Did::Chosen(Action::Command(10)));
    }

    #[test]
    fn what_the_user_added_follows_the_apps_own_starred() {
        let mut m = terminal();
        m.pins = vec!["Shell/Clear".into(), "role:11".into(), "gone/away".into(), "Tab/Split side by side".into()];
        let top = labels(&m.top());
        assert_eq!(&top[..5], ["New tab (Ctrl+Shift+T)", "New window (Ctrl+Shift+N)", "Clear *", "Split side by side (Ctrl+Shift+R) *", "[- 110% +]"]);
        assert!(labels(&m.sub(ALL)).contains(&"Clear *".to_string()));
        match &m.top()[2] {
            Line::Item { pin, pinned: true, .. } => assert_eq!(pin.as_deref(), Some("Shell/Clear")),
            l => panic!("{l:?}"),
        }
    }

    #[test]
    fn the_desktops_lines_stand_in_where_the_app_sent_none() {
        let m = terminal();
        let top = m.top();
        let n = top.len();
        assert!(matches!(&top[n - 4], Line::Item { action: Action::Command(5), pin: None, .. }));
        assert!(matches!(&top[n - 2], Line::Item { action: Action::About, .. }));
        assert!(matches!(&top[n - 1], Line::Item { action: Action::Quit, .. }));
    }

    #[test]
    fn a_foreign_app_gets_the_skeleton_and_a_note() {
        let m = Model { app: "Firefox".into(), commands: vec![], terminal: false, native: false, window: Vec::new(), pins: Vec::new(), given: None };
        assert_eq!(labels(&m.top()), ["[note]", "---", "Settings…", "About Firefox", "Quit Firefox"]);
    }

    #[test]
    fn search_finds_commands_in_every_menu_with_their_keys_and_skips_disabled_ones() {
        let m = terminal();
        let found = m.search("split");
        assert_eq!(found.len(), 1);
        assert_eq!(labels(&[found[0].0.clone()]), ["Split side by side (Ctrl+Shift+R)"]);
        assert_eq!(found[0].1, "Tab");
        assert!(m.search("copy").is_empty(), "Copy is disabled");
        assert_eq!(m.search("TAB").len(), 2, "New tab and Next tab, case aside");
        assert_eq!(m.search("quit").len(), 1);
        assert!(m.search("   ").is_empty());
    }

    #[test]
    fn keys_move_open_the_shortcuts_and_choose() {
        let m = terminal();
        let mut s = State::default();
        assert_eq!(s.key(&m, Key::Down), Did::Moved);
        assert_eq!(s.top, Some(0));
        assert_eq!(s.key(&m, Key::Enter), Did::Chosen(Action::Command(3)));
        // Down to Keyboard shortcuts: it opens; Right goes in, past its heading.
        for _ in 0..4 {
            s.key(&m, Key::Down);
        }
        assert_eq!(s.open, Some(5));
        s.key(&m, Key::Right);
        assert_eq!(s.sub, Some(1), "New window, under File");
        assert_eq!(s.key(&m, Key::Enter), Did::Chosen(Action::Command(8)));
        assert_eq!(s.key(&m, Key::Left), Did::Moved);
        assert_eq!(s.sub, None);
        let mut s = State::default();
        s.key(&m, Key::Up);
        assert_eq!(s.key(&m, Key::Enter), Did::Chosen(Action::Quit));
        assert_eq!(s.key(&m, Key::Escape), Did::Close);
    }

    #[test]
    fn typing_searches_even_without_the_field_and_enter_takes_the_first_result() {
        let m = terminal();
        let mut s = State::default();
        for c in "new t".chars() {
            s.key(&m, Key::Char(c));
        }
        assert_eq!(labels(&s.shown(&m)), ["[search]", "New tab (Ctrl+Shift+T)"]);
        assert_eq!(s.key(&m, Key::Enter), Did::Chosen(Action::Command(3)));
        assert_eq!(s.key(&m, Key::Escape), Did::Moved);
        assert_eq!(s.typed, "");
        assert_eq!(s.key(&m, Key::Escape), Did::Close);
    }

    #[test]
    fn the_pointer_chooses_and_opens_the_shortcuts_and_headings_are_not_chosen() {
        let m = terminal();
        let mut s = State::default();
        s.hover_top(&m, Some(5));
        assert_eq!((s.top, s.open), (Some(5), Some(5)));
        assert_eq!(labels(&s.sub_lines(&m)), labels(&m.sub(ALL)));
        s.hover_sub(&m, Some(1));
        assert_eq!(s.sub, Some(1));
        s.hover_sub(&m, Some(0));
        assert_eq!(s.sub, None, "a heading is not chosen");
        s.hover_top(&m, Some(3));
        assert_eq!(s.top, Some(5), "a separator is not chosen");
    }

    #[test]
    fn rows_stack_from_the_top_with_the_padding() {
        let m = terminal();
        let top = m.top();
        let r = rows((100, 50), &top);
        assert_eq!(r[0], Rect::new(106, 56, WIDTH - 12, ITEM_H));
        assert_eq!(r[1].y, 56 + ITEM_H);
        assert_eq!(height(&top), top.iter().map(Line::height).sum::<i32>() + 12);
    }
}
