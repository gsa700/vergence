//! Commands by role (app settings milestone 1): what a native app's menu
//! holds. An app does not send "a menu"; it sends commands, each either a
//! **role** the desktop knows (Copy, New tab, Settings ...) or one of its
//! own (a menu's name, a label, keys). A role's menu, label and keys come
//! from [`ROLES`], here, for every app: the server draws them from this
//! table whatever the app sent, so Copy is called Copy, sits in Edit and is
//! Ctrl+C in every app.
//!
//! One rule bends the keys, the same way in every app that asks for it:
//! a **terminal** (an app whose Ctrl+letter keys belong to the programs it
//! runs) gets Shift added to each Ctrl+letter key ([`keys_for`]): Ctrl+T is
//! Ctrl+Shift+T there. Copy and Paste keep Ctrl+C and Ctrl+V: a terminal
//! copies with them while text is selected and passes them on otherwise.

/// What a command is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// One of the app's own: its menu and label are what it sent.
    Own,
    About,
    Settings,
    Quit,
    NewWindow,
    NewTab,
    Open,
    Save,
    SaveAs,
    Print,
    CloseTab,
    CloseWindow,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Find,
    FindNext,
    Replace,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    FullScreen,
    NextTab,
    PreviousTab,
}

/// Where a role goes, what it is called, its keys. `menu` "" is the menu's
/// own part (About and Settings at the top, Quit at the bottom); `{app}` in a
/// label is the app's name.
pub struct RoleInfo {
    pub role: Role,
    pub code: u16,
    pub menu: &'static str,
    pub label: &'static str,
    pub keys: &'static str,
}

/// Every role, in the order each menu lists them.
pub const ROLES: &[RoleInfo] = &[
    RoleInfo { role: Role::About, code: 1, menu: "", label: "About {app}", keys: "" },
    RoleInfo { role: Role::Settings, code: 2, menu: "", label: "Settings…", keys: "Ctrl+," },
    RoleInfo { role: Role::Quit, code: 3, menu: "", label: "Quit {app}", keys: "Ctrl+Q" },
    RoleInfo { role: Role::NewWindow, code: 10, menu: "File", label: "New window", keys: "Ctrl+N" },
    RoleInfo { role: Role::NewTab, code: 11, menu: "File", label: "New tab", keys: "Ctrl+T" },
    RoleInfo { role: Role::Open, code: 12, menu: "File", label: "Open…", keys: "Ctrl+O" },
    RoleInfo { role: Role::Save, code: 13, menu: "File", label: "Save", keys: "Ctrl+S" },
    RoleInfo { role: Role::SaveAs, code: 14, menu: "File", label: "Save as…", keys: "Ctrl+Shift+S" },
    RoleInfo { role: Role::Print, code: 15, menu: "File", label: "Print…", keys: "Ctrl+P" },
    RoleInfo { role: Role::CloseTab, code: 16, menu: "File", label: "Close tab", keys: "Ctrl+W" },
    RoleInfo { role: Role::CloseWindow, code: 17, menu: "File", label: "Close window", keys: "" },
    RoleInfo { role: Role::Undo, code: 20, menu: "Edit", label: "Undo", keys: "Ctrl+Z" },
    RoleInfo { role: Role::Redo, code: 21, menu: "Edit", label: "Redo", keys: "Ctrl+Shift+Z" },
    RoleInfo { role: Role::Cut, code: 22, menu: "Edit", label: "Cut", keys: "Ctrl+X" },
    RoleInfo { role: Role::Copy, code: 23, menu: "Edit", label: "Copy", keys: "Ctrl+C" },
    RoleInfo { role: Role::Paste, code: 24, menu: "Edit", label: "Paste", keys: "Ctrl+V" },
    RoleInfo { role: Role::SelectAll, code: 25, menu: "Edit", label: "Select all", keys: "Ctrl+A" },
    RoleInfo { role: Role::Find, code: 26, menu: "Edit", label: "Find…", keys: "Ctrl+F" },
    RoleInfo { role: Role::FindNext, code: 27, menu: "Edit", label: "Find next", keys: "Ctrl+G" },
    RoleInfo { role: Role::Replace, code: 28, menu: "Edit", label: "Replace…", keys: "Ctrl+H" },
    RoleInfo { role: Role::ZoomIn, code: 30, menu: "View", label: "Zoom in", keys: "Ctrl+=" },
    RoleInfo { role: Role::ZoomOut, code: 31, menu: "View", label: "Zoom out", keys: "Ctrl+-" },
    RoleInfo { role: Role::ZoomReset, code: 32, menu: "View", label: "Actual size", keys: "Ctrl+0" },
    RoleInfo { role: Role::FullScreen, code: 33, menu: "View", label: "Full screen", keys: "F11" },
    RoleInfo { role: Role::NextTab, code: 40, menu: "Tab", label: "Next tab", keys: "Ctrl+Page Down" },
    RoleInfo { role: Role::PreviousTab, code: 41, menu: "Tab", label: "Previous tab", keys: "Ctrl+Page Up" },
];

/// The menus every app's menu lists first, in this order; an app's own
/// menus follow them in the order it first names them.
pub const MENUS: &[&str] = &["File", "Edit", "View", "Tab"];

impl Role {
    pub fn info(self) -> Option<&'static RoleInfo> {
        ROLES.iter().find(|r| r.role == self)
    }

    pub fn code(self) -> u16 {
        self.info().map_or(0, |i| i.code)
    }

    pub fn from_code(code: u16) -> Option<Role> {
        if code == 0 {
            return Some(Role::Own);
        }
        ROLES.iter().find(|r| r.code == code).map(|r| r.role)
    }
}

/// One command of a window's menu: an id the app chose (unique in the
/// window, not 0), its role, and for its own commands the menu, label and
/// keys (ignored for a role: [`ROLES`] says). `enabled`: it can be chosen
/// now; `checked`: a switch, on or off (None: not a switch); `featured`: one
/// of the app's everyday few, shown on the menu's first panel (the rest are
/// under All commands; Dave, 2026-11-14, after GNOME Terminal's menu).
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub id: u32,
    pub role: Role,
    pub menu: String,
    pub label: String,
    pub keys: String,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub featured: bool,
}

impl Command {
    /// A role's command.
    pub fn role(id: u32, role: Role) -> Command {
        Command {
            id,
            role,
            menu: String::new(),
            label: String::new(),
            keys: String::new(),
            enabled: true,
            checked: None,
            featured: false,
        }
    }

    /// One of the app's own.
    pub fn own(id: u32, menu: &str, label: &str, keys: &str) -> Command {
        Command {
            id,
            role: Role::Own,
            menu: menu.into(),
            label: label.into(),
            keys: keys.into(),
            enabled: true,
            checked: None,
            featured: false,
        }
    }

    pub fn enabled(mut self, on: bool) -> Command {
        self.enabled = on;
        self
    }

    pub fn checked(mut self, on: bool) -> Command {
        self.checked = Some(on);
        self
    }

    /// One of the app's everyday few.
    pub fn featured(mut self) -> Command {
        self.featured = true;
        self
    }

    /// What names it across runs (ids are the app's and may change): a
    /// role by its code, one of its own by menu and label. The desktop keeps
    /// the commands a user added to an app's first panel by this.
    pub fn key(&self) -> String {
        match self.role.info() {
            Some(i) => format!("role:{}", i.code),
            None => format!("{}/{}", self.menu, self.label),
        }
    }

    /// The menu it goes in ("" the menu's own part), as shown.
    pub fn menu_shown(&self) -> &str {
        match self.role.info() {
            Some(i) => i.menu,
            None => &self.menu,
        }
    }

    /// Its label as shown, `{app}` filled in.
    pub fn label_shown(&self, app: &str) -> String {
        match self.role.info() {
            Some(i) => i.label.replace("{app}", app),
            None => self.label.clone(),
        }
    }

    /// Its keys as shown: a role's from the table (Shift added in a
    /// terminal); an app's own keys are dropped if they are a role's (no app
    /// may give Copy's keys to something else).
    pub fn keys_shown(&self, terminal: bool) -> String {
        match self.role.info() {
            Some(i) => keys_for(i.role, i.keys, terminal),
            None if ROLES.iter().any(|r| {
                !r.keys.is_empty() && (keys_for(r.role, r.keys, terminal) == self.keys || r.keys == self.keys)
            }) =>
            {
                String::new()
            }
            None => self.keys.clone(),
        }
    }
}

/// A role's keys in an app: in a terminal, Ctrl+letter becomes
/// Ctrl+Shift+letter, except Copy and Paste.
pub fn keys_for(role: Role, keys: &str, terminal: bool) -> String {
    if !terminal || matches!(role, Role::Copy | Role::Paste) {
        return keys.to_string();
    }
    match keys.strip_prefix("Ctrl+") {
        Some(rest) if rest.len() == 1 && rest.chars().all(|c| c.is_ascii_alphabetic()) => format!("Ctrl+Shift+{rest}"),
        _ => keys.to_string(),
    }
}

/// The most commands a window may send, and the longest label, menu name and keys.
pub const MAX_COMMANDS: usize = 256;
pub const MAX_LABEL: usize = 80;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_has_one_code_and_a_known_menu() {
        let mut codes: Vec<u16> = ROLES.iter().map(|r| r.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), ROLES.len());
        for r in ROLES {
            assert!(r.menu.is_empty() || MENUS.contains(&r.menu), "{}", r.label);
            assert_eq!(Role::from_code(r.code), Some(r.role));
        }
        assert_eq!(Role::from_code(0), Some(Role::Own));
        assert_eq!(Role::from_code(999), None);
    }

    #[test]
    fn a_role_is_the_same_whatever_the_app_sends() {
        let c = Command {
            id: 1,
            role: Role::Copy,
            menu: "Stuff".into(),
            label: "Duplicate".into(),
            keys: "Ctrl+D".into(),
            enabled: true,
            checked: None,
            featured: false,
        };
        assert_eq!(c.key(), "role:23");
        assert_eq!(Command::own(2, "Tab", "Split", "").key(), "Tab/Split");
        assert_eq!(
            (c.menu_shown(), c.label_shown("Terminal").as_str(), c.keys_shown(false).as_str()),
            ("Edit", "Copy", "Ctrl+C")
        );
        assert_eq!(Command::role(2, Role::Quit).label_shown("Terminal"), "Quit Terminal");
    }

    #[test]
    fn a_terminal_shifts_ctrl_letters_but_not_copy_and_paste() {
        assert_eq!(Command::role(1, Role::NewTab).keys_shown(true), "Ctrl+Shift+T");
        assert_eq!(Command::role(1, Role::Copy).keys_shown(true), "Ctrl+C");
        assert_eq!(Command::role(1, Role::Paste).keys_shown(true), "Ctrl+V");
        assert_eq!(Command::role(1, Role::Settings).keys_shown(true), "Ctrl+,");
        assert_eq!(Command::role(1, Role::Redo).keys_shown(true), "Ctrl+Shift+Z");
        assert_eq!(Command::role(1, Role::NewTab).keys_shown(false), "Ctrl+T");
    }

    #[test]
    fn an_app_cannot_take_a_roles_keys() {
        assert_eq!(Command::own(1, "Tab", "Split", "Ctrl+C").keys_shown(false), "");
        assert_eq!(Command::own(1, "Tab", "Split", "Ctrl+Shift+T").keys_shown(true), "");
        assert_eq!(Command::own(1, "Tab", "Split side by side", "Ctrl+Shift+R").keys_shown(true), "Ctrl+Shift+R");
    }
}
