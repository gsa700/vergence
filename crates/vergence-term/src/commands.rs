//! The terminal's commands: the app menu's list and ids, what each command
//! and each line of the right-click menu does, the settings window, text
//! size for a while. The window itself is in main.rs.

use vergence_ui::Window;

use crate::menu::Item;
use crate::panes::Axis;
use crate::session::lock;
use crate::settings::config;
use crate::{Terminal, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP, prefs};

impl Terminal {
    /// The terminal's settings window (Ctrl+, or Settings… in the app menu);
    /// one at a time.
    pub(crate) fn open_settings(&mut self, window: &mut Window) {
        if self.prefs_open.get() {
            return;
        }
        let rows = Box::new(prefs::TermRows::new());
        window.open_window(vergence_widgets::window(vergence_widgets::prefs::Prefs::new(
            "Terminal",
            "vergence-term",
            "[terminal]",
            true,
            rows,
            self.prefs_open.clone(),
        )));
    }

    /// Text size for a while: a step larger or smaller, or (None) as the settings say.
    pub(crate) fn zoom_by(&mut self, step: Option<f32>, window: &mut Window) {
        let z = match step {
            Some(s) => ((self.zoom + s) * 10.0).round() / 10.0,
            None => 1.0,
        };
        let z = z.clamp(ZOOM_MIN, ZOOM_MAX);
        if (z - self.zoom).abs() > 0.001 {
            self.zoom = z;
            self.metrics = None;
            window.redraw();
        }
    }

    /// Divides the active pane's room in two, with a new terminal in the
    /// second half, which the keys then go to.
    /// What a line of the right-click menu, or the same command from the app
    /// menu, does.
    pub(crate) fn do_item(&mut self, item: Item, window: &mut Window) {
        match item {
            Item::Copy => {
                self.copy(window);
            }
            Item::Paste => window.paste(),
            Item::SplitSide => self.split(Axis::Side, window),
            Item::SplitStack => self.split(Axis::Stack, window),
            Item::NewTab => self.new_tab(window),
            Item::ClosePane => self.close_active(window),
            Item::PaneToTab => {
                if let Some(t) = self.take_pane(window) {
                    self.tabs.push(t);
                    self.shown = self.tabs.len() - 1;
                    self.fitted = (0.0, 0.0, 0.0, 0.0);
                    self.retitle(window);
                    self.keep();
                }
            }
            Item::PaneToWindow => {
                // A section of a tab with others; a tab of its own otherwise.
                let moved = match self.take_pane(window) {
                    Some(t) => Some(t),
                    None => self.tabs.get(self.shown).map(|t| t.id).and_then(|id| self.take_tab(id, window)),
                };
                if let Some(t) = moved {
                    // A bar's height down and right of this window, as large.
                    let step = vergence_ui::bar::BAR_H as f64;
                    self.tab_to_window(t, Some(vergence_ui::BarPlace::Parent { x: step, y: step }), window);
                }
            }
        }
    }

    pub(crate) fn on_commands(&mut self) -> Option<(bool, Vec<vergence_ui::AppCommand>)> {
        use vergence_ui::{AppCommand as C, Role};
        let selected = self.front().is_some_and(|s| lock(&s.screen).selection.is_some());
        let several = self.tabs.len() > 1;
        // The app menu holds the terminal's own few (Dave, 2026-11-14: the app
        // button is the app; a pane's and a tab's commands are on their right
        // click); the rest are listed under Keyboard shortcuts.
        let percent = format!("{}%", (self.zoom * 100.0).round());
        let list = vec![
            C::role(cmd::SETTINGS, Role::Settings),
            // Its own About window, as Edit's (Dave, 2026-11-18: the server's
            // card for apps with none was a notification).
            C::role(cmd::ABOUT, Role::About),
            C::role(cmd::NEW_WINDOW, Role::NewWindow).featured(),
            C::role(cmd::NEW_TAB, Role::NewTab).featured(),
            C::role(cmd::ZOOM_OUT, Role::ZoomOut).enabled(self.zoom > ZOOM_MIN + 0.001).featured(),
            C { label: percent, ..C::role(cmd::ZOOM_RESET, Role::ZoomReset).featured() },
            C::role(cmd::ZOOM_IN, Role::ZoomIn).enabled(self.zoom < ZOOM_MAX - 0.001).featured(),
            C::role(cmd::COPY, Role::Copy).enabled(selected),
            C::role(cmd::PASTE, Role::Paste),
            C::role(cmd::CLOSE_TAB, Role::CloseTab),
            C::role(cmd::NEXT_TAB, Role::NextTab).enabled(several),
            C::role(cmd::PREVIOUS_TAB, Role::PreviousTab).enabled(several),
            C::own(cmd::SPLIT_SIDE, "Tab", "Split side by side", "Ctrl+Shift+R"),
            C::own(cmd::SPLIT_STACK, "Tab", "Split top and bottom", "Ctrl+Shift+D"),
            C::own(cmd::PANE_TO_TAB, "Tab", "Move section to a new tab", ""),
            C::own(cmd::PANE_TO_WINDOW, "Tab", "Move to a new window", ""),
        ];
        if self.commands_told.as_ref() == Some(&list) {
            return None;
        }
        self.commands_told = Some(list.clone());
        Some((true, list))
    }

    pub(crate) fn on_command(&mut self, id: u32, window: &mut Window) {
        let item = match id {
            cmd::COPY => Item::Copy,
            cmd::PASTE => Item::Paste,
            cmd::NEW_TAB => Item::NewTab,
            cmd::CLOSE_TAB => Item::ClosePane,
            cmd::SPLIT_SIDE => Item::SplitSide,
            cmd::SPLIT_STACK => Item::SplitStack,
            cmd::PANE_TO_TAB => Item::PaneToTab,
            cmd::PANE_TO_WINDOW => Item::PaneToWindow,
            cmd::NEW_WINDOW => return self.new_window(window),
            cmd::SETTINGS => return self.open_settings(window),
            cmd::ABOUT => {
                let theme = vergence_widgets::Theme::from_desktop(&config("desktop.toml"), true, "[terminal]");
                let about = vergence_widgets::about::About::new(
                    "vergence-term",
                    "Terminal",
                    env!("CARGO_PKG_VERSION"),
                    "A terminal: tabs, and sections side by side or stacked.",
                    theme,
                );
                return window.open_window(vergence_widgets::window(about));
            }
            cmd::ZOOM_IN => return self.zoom_by(Some(ZOOM_STEP), window),
            cmd::ZOOM_OUT => return self.zoom_by(Some(-ZOOM_STEP), window),
            cmd::ZOOM_RESET => return self.zoom_by(None, window),
            cmd::NEXT_TAB | cmd::PREVIOUS_TAB => {
                let n = self.tabs.len().max(1);
                let to = if id == cmd::NEXT_TAB { (self.shown + 1) % n } else { (self.shown + n - 1) % n };
                return self.show_tab(to, window);
            }
            _ => return,
        };
        self.do_item(item, window);
    }
}

/// The app menu's command ids (app settings milestone 1).
/// The command that does what a menu line says (menu::server_lines).
pub(crate) fn command_of(item: Item) -> u32 {
    match item {
        Item::Copy => cmd::COPY,
        Item::Paste => cmd::PASTE,
        Item::SplitSide => cmd::SPLIT_SIDE,
        Item::SplitStack => cmd::SPLIT_STACK,
        Item::NewTab => cmd::NEW_TAB,
        Item::ClosePane => cmd::CLOSE_TAB,
        Item::PaneToTab => cmd::PANE_TO_TAB,
        Item::PaneToWindow => cmd::PANE_TO_WINDOW,
    }
}

mod cmd {
    pub const COPY: u32 = 1;
    pub const PASTE: u32 = 2;
    pub const NEW_TAB: u32 = 3;
    pub const CLOSE_TAB: u32 = 4;
    pub const NEXT_TAB: u32 = 5;
    pub const PREVIOUS_TAB: u32 = 6;
    pub const SPLIT_SIDE: u32 = 7;
    pub const SPLIT_STACK: u32 = 8;
    pub const PANE_TO_TAB: u32 = 9;
    pub const PANE_TO_WINDOW: u32 = 10;
    pub const NEW_WINDOW: u32 = 11;
    pub const ZOOM_IN: u32 = 12;
    pub const ZOOM_OUT: u32 = 13;
    pub const ZOOM_RESET: u32 = 14;
    pub const SETTINGS: u32 = 15;
    pub const ABOUT: u32 = 16;
}
