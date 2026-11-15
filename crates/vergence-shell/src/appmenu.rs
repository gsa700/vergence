//! The app menu (app settings milestone 1): what opens from a window's app
//! button, for every app. Its skeleton is fixed and the app fills in only
//! the middle, from its commands (vergence_protocol::commands):
//!
//! 1. Search commands… (typing filters every command, submenus included);
//! 2. About <App>, Settings…;
//! 3. the app's menus: File, Edit, View, Tab, then its own;
//! 4. Window ›, the desktop's (layouts, minimize, close);
//! 5. Quit <App>.
//!
//! A part with nothing in it is left out. An app that is not ours (no
//! commands) gets the skeleton with a line saying its own menu holds the
//! rest. Everything here is plain data: the server draws it and carries out
//! what is chosen ([`Action`]).

use vergence_protocol::commands::{Command, MENUS, ROLES, Role};

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
    Item { action: Action, label: String, keys: String, enabled: bool, checked: Option<bool> },
    /// A submenu, by its name.
    Sub(String),
    /// A line of words, not chosen (a foreign app's note).
    Note(String),
    Separator,
}

impl Line {
    /// Whether the keyboard and the pointer can rest on it.
    pub fn selectable(&self) -> bool {
        matches!(self, Line::Item { enabled: true, .. } | Line::Sub(_))
    }

    pub fn height(&self) -> i32 {
        match self {
            Line::Search => SEARCH_H,
            Line::Separator => SEP_H,
            Line::Note(_) => NOTE_H,
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
}

impl Model {
    fn role(&self, role: Role) -> Option<&Command> {
        self.commands.iter().find(|c| c.role == role)
    }

    fn item(&self, c: &Command) -> Line {
        Line::Item { action: Action::Command(c.id), label: c.label_shown(&self.app), keys: c.keys_shown(self.terminal), enabled: c.enabled, checked: c.checked }
    }

    /// A skeleton line: the app's command for the role if it sent one, else the desktop's.
    fn skeleton(&self, role: Role, own: Action) -> Line {
        match self.role(role) {
            Some(c) => self.item(c),
            None => {
                let info = role.info().expect("a role of the table");
                Line::Item { action: own, label: info.label.replace("{app}", &self.app), keys: String::new(), enabled: true, checked: None }
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

    /// The top level.
    pub fn top(&self) -> Vec<Line> {
        let mut lines = vec![Line::Search, self.skeleton(Role::About, Action::About), self.skeleton(Role::Settings, Action::Settings), Line::Separator];
        if self.native {
            lines.extend(self.menus().into_iter().map(Line::Sub));
        } else {
            lines.push(Line::Note(format!("{} keeps its own settings and commands in its own menu.", self.app)));
        }
        lines.push(Line::Separator);
        if !self.window.is_empty() {
            lines.push(Line::Sub("Window".into()));
            lines.push(Line::Separator);
        }
        lines.push(self.skeleton(Role::Quit, Action::Quit));
        tidy(lines)
    }

    /// A submenu's lines: the roles in the table's order, then the app's own
    /// in the order it sent them, a line between the two.
    pub fn sub(&self, name: &str) -> Vec<Line> {
        if name == "Window" {
            return self.window.iter().map(|(a, l)| Line::Item { action: Action::Window(*a), label: l.clone(), keys: String::new(), enabled: true, checked: None }).collect();
        }
        let mut lines: Vec<Line> = ROLES.iter().filter_map(|r| self.commands.iter().find(|c| c.role == r.role && r.menu == name)).map(|c| self.item(c)).collect();
        let own: Vec<Line> = self.commands.iter().filter(|c| c.role == Role::Own && c.menu == name).map(|c| self.item(c)).collect();
        if !lines.is_empty() && !own.is_empty() {
            lines.push(Line::Separator);
        }
        lines.extend(own);
        tidy(lines)
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
        for l in self.top() {
            if matches!(l, Line::Item { .. }) {
                all.push((l, String::new()));
            }
        }
        let mut subs = if self.native { self.menus() } else { Vec::new() };
        if !self.window.is_empty() {
            subs.push("Window".into());
        }
        for m in subs {
            all.extend(self.sub(&m).into_iter().filter(|l| matches!(l, Line::Item { .. })).map(|l| (l, m.clone())));
        }
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
        if self.typed.trim().is_empty() {
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
                Command::role(3, Role::NewTab),
                Command::own(4, "Tab", "Split side by side", "Ctrl+Shift+R"),
                Command::role(5, Role::Settings),
                Command::role(6, Role::NextTab),
                Command::own(7, "Shell", "Clear", ""),
            ],
            terminal: true,
            native: true,
            window: vec![(WindowAct::Snap(Zone::Left), "Left half".into()), (WindowAct::Minimize, "Minimize".into())],
        }
    }

    fn labels(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| match l {
                Line::Search => "[search]".into(),
                Line::Item { label, keys, .. } if keys.is_empty() => label.clone(),
                Line::Item { label, keys, .. } => format!("{label} ({keys})"),
                Line::Sub(n) => format!("{n} >"),
                Line::Note(_) => "[note]".into(),
                Line::Separator => "---".into(),
            })
            .collect()
    }

    #[test]
    fn the_skeleton_holds_the_apps_menus_in_their_order() {
        let m = terminal();
        assert_eq!(labels(&m.top()), ["[search]", "About Terminal", "Settings… (Ctrl+,)", "---", "File >", "Edit >", "Tab >", "Shell >", "---", "Window >", "---", "Quit Terminal"]);
        assert_eq!(labels(&m.sub("Edit")), ["Copy (Ctrl+C)", "Paste (Ctrl+V)"]);
        assert_eq!(labels(&m.sub("Tab")), ["Next tab (Ctrl+Page Down)", "---", "Split side by side (Ctrl+Shift+R)"]);
        assert_eq!(labels(&m.sub("File")), ["New tab (Ctrl+Shift+T)"]);
        assert_eq!(labels(&m.sub("Window")), ["Left half", "Minimize"]);
    }

    #[test]
    fn the_desktops_lines_stand_in_where_the_app_sent_none() {
        let m = terminal();
        let top = m.top();
        assert!(matches!(&top[1], Line::Item { action: Action::About, .. }));
        assert!(matches!(&top[2], Line::Item { action: Action::Command(5), .. }));
        assert!(matches!(top.last(), Some(Line::Item { action: Action::Quit, .. })));
    }

    #[test]
    fn a_foreign_app_gets_the_skeleton_and_a_note() {
        let m = Model { app: "Firefox".into(), commands: vec![], terminal: false, native: false, window: vec![(WindowAct::Minimize, "Minimize".into())] };
        assert_eq!(labels(&m.top()), ["[search]", "About Firefox", "Settings…", "---", "[note]", "---", "Window >", "---", "Quit Firefox"]);
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
        assert_eq!(m.search("left").len(), 1);
        assert!(m.search("   ").is_empty());
    }

    #[test]
    fn keys_move_open_submenus_and_choose() {
        let m = terminal();
        let mut s = State::default();
        assert_eq!(s.key(&m, Key::Down), Did::Moved);
        assert_eq!(s.top, Some(1), "About is first");
        s.key(&m, Key::Down);
        assert_eq!(s.key(&m, Key::Enter), Did::Chosen(Action::Command(5)));
        // Down to Edit: it opens; Right goes in; Down skips the disabled Copy.
        s.key(&m, Key::Down);
        s.key(&m, Key::Down);
        assert_eq!(s.open, Some(5));
        s.key(&m, Key::Right);
        assert_eq!(s.sub, Some(1), "Paste, Copy being disabled");
        assert_eq!(s.key(&m, Key::Enter), Did::Chosen(Action::Command(2)));
        assert_eq!(s.key(&m, Key::Left), Did::Moved);
        assert_eq!(s.sub, None);
        // Up from the top wraps to Quit.
        let mut s = State::default();
        s.key(&m, Key::Up);
        assert_eq!(s.key(&m, Key::Enter), Did::Chosen(Action::Quit));
        assert_eq!(s.key(&m, Key::Escape), Did::Close);
    }

    #[test]
    fn typing_searches_and_enter_takes_the_first_result() {
        let m = terminal();
        let mut s = State::default();
        for c in "new t".chars() {
            s.key(&m, Key::Char(c));
        }
        assert_eq!(labels(&s.shown(&m)), ["[search]", "New tab (Ctrl+Shift+T)"]);
        assert_eq!(s.key(&m, Key::Enter), Did::Chosen(Action::Command(3)));
        // Escape clears what was typed, then closes.
        assert_eq!(s.key(&m, Key::Escape), Did::Moved);
        assert_eq!(s.typed, "");
        assert_eq!(s.key(&m, Key::Escape), Did::Close);
        // Typing while in a submenu leaves it for the search.
        let mut s = State::default();
        s.hover_top(&m, Some(5));
        s.key(&m, Key::Right);
        assert!(s.sub.is_some());
        s.key(&m, Key::Char('q'));
        assert_eq!((s.sub, s.open, s.typed.as_str()), (None, None, "q"));
    }

    #[test]
    fn the_pointer_chooses_and_opens_submenus() {
        let m = terminal();
        let mut s = State::default();
        s.hover_top(&m, Some(6));
        assert_eq!((s.top, s.open), (Some(6), Some(6)));
        assert_eq!(labels(&s.sub_lines(&m)), labels(&m.sub("Tab")));
        s.hover_sub(&m, Some(2));
        assert_eq!(s.sub, Some(2));
        s.hover_sub(&m, Some(1));
        assert_eq!(s.sub, None, "a separator is not chosen");
        // Off the panel, the open submenu stays.
        s.hover_top(&m, None);
        assert_eq!(s.open, Some(6));
        // A separator or the search field is not chosen.
        s.hover_top(&m, Some(3));
        assert_eq!(s.top, Some(6));
    }

    #[test]
    fn rows_stack_from_the_top_with_the_padding() {
        let m = terminal();
        let top = m.top();
        let r = rows((100, 50), &top);
        assert_eq!(r[0], Rect::new(106, 56, WIDTH - 12, SEARCH_H));
        assert_eq!(r[1].y, 56 + SEARCH_H);
        assert_eq!(height(&top), top.iter().map(Line::height).sum::<i32>() + 12);
    }
}
