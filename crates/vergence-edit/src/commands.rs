//! Edit's commands: their numbers, what each does, the app menu's list
//! of them, and the keys that run them. The window itself is in main.rs.

use vergence_ui::{AppCommand as C, Role};
use vergence_widgets::{Input, Program, Window};

use crate::{APP_ID, EditWindow, Picking, ZOOM_MAX, ZOOM_MIN};

pub(crate) mod cmd {
    pub const NEW_WINDOW: u32 = 1;
    pub const NEW_TAB: u32 = 2;
    pub const OPEN: u32 = 3;
    pub const SAVE: u32 = 4;
    pub const SAVE_AS: u32 = 5;
    pub const CLOSE_TAB: u32 = 6;
    pub const UNDO: u32 = 10;
    pub const REDO: u32 = 11;
    pub const CUT: u32 = 12;
    pub const COPY: u32 = 13;
    pub const PASTE: u32 = 14;
    pub const SELECT_ALL: u32 = 15;
    pub const FIND: u32 = 16;
    pub const FIND_NEXT: u32 = 17;
    pub const FIND_PREV: u32 = 18;
    pub const REPLACE: u32 = 19;
    pub const ZOOM_IN: u32 = 20;
    pub const ZOOM_OUT: u32 = 21;
    pub const ZOOM_RESET: u32 = 22;
    pub const LINE_NUMBERS: u32 = 23;
    pub const WRAP: u32 = 24;
    pub const NEXT_TAB: u32 = 30;
    pub const PREV_TAB: u32 = 31;
    pub const GO_TO_LINE: u32 = 32;
    pub const SETTINGS: u32 = 40;
    pub const ABOUT: u32 = 41;
}

impl EditWindow {
    /// The menu's commands, as the app's own keys too.
    pub(crate) fn run(&mut self, id: u32, window: &mut Window) {
        let doc = self.doc();
        match id {
            cmd::NEW_WINDOW => {
                window.open_window(vergence_widgets::window(EditWindow::new(self.shared.clone(), Vec::new())))
            }
            cmd::NEW_TAB => {
                let t = self.untitled_tab();
                self.front = t.id;
                self.tabs.push(t);
                self.apply_options();
            }
            cmd::OPEN => self.pick(Picking::Open, window),
            cmd::SAVE => self.save_or_ask(self.front, window),
            cmd::SAVE_AS => self.pick(Picking::SaveAs { tab: self.front, then_close: false }, window),
            cmd::CLOSE_TAB => self.close_tab(self.front, window),
            cmd::UNDO => {
                if let Some(d) = doc {
                    d.borrow_mut().undo();
                }
            }
            cmd::REDO => {
                if let Some(d) = doc {
                    d.borrow_mut().redo();
                }
            }
            cmd::CUT | cmd::COPY => {
                if let Some(d) = doc {
                    let text = d.borrow().selected_text();
                    if !text.is_empty() {
                        window.copy(&text);
                        if id == cmd::CUT {
                            d.borrow_mut().insert("");
                        }
                    }
                }
            }
            cmd::PASTE => window.paste(),
            cmd::SELECT_ALL => {
                if let Some(d) = doc {
                    d.borrow_mut().select_all();
                }
            }
            cmd::ZOOM_IN => self.set_zoom(self.zoom * 1.1),
            cmd::ZOOM_OUT => self.set_zoom(self.zoom / 1.1),
            cmd::ZOOM_RESET => self.set_zoom(1.0),
            cmd::LINE_NUMBERS => self.toggle_setting("line-numbers", true),
            cmd::WRAP => self.toggle_setting("wrap", true),
            cmd::FIND => self.open_find(false),
            cmd::GO_TO_LINE => {
                self.goto = Some(String::new());
                self.focus_turn += 1;
            }
            cmd::REPLACE => self.open_find(true),
            cmd::FIND_NEXT | cmd::FIND_PREV => {
                if self.find.is_none() {
                    self.open_find(false);
                }
                self.find_step(id == cmd::FIND_PREV);
            }
            cmd::NEXT_TAB => self.step_tab(1),
            cmd::PREV_TAB => self.step_tab(-1),
            cmd::SETTINGS => self.open_settings(window),
            cmd::ABOUT => {
                let about = vergence_widgets::about::About::new(
                    APP_ID,
                    "Edit",
                    env!("CARGO_PKG_VERSION"),
                    "A plain-text editor: tabs, line numbers, wrapping, undo; files saved whole or not at all.",
                    self.theme(),
                );
                window.open_window(vergence_widgets::window(about));
            }
            _ => {}
        }
    }

    pub(crate) fn on_commands(&self) -> Option<(bool, Vec<C>)> {
        let (undo, redo, selected, editable) = match self.front_tab() {
            Some(t) => {
                let d = t.doc.borrow();
                let (a, b) = d.selection();
                (d.can_undo(), d.can_redo(), a != b, t.read_only.is_none())
            }
            None => (false, false, false, false),
        };
        Some((
            false,
            vec![
                C::role(cmd::NEW_TAB, Role::NewTab).featured(),
                C::role(cmd::NEW_WINDOW, Role::NewWindow),
                C::role(cmd::OPEN, Role::Open).featured(),
                C::role(cmd::SAVE, Role::Save).enabled(editable).featured(),
                C::role(cmd::SAVE_AS, Role::SaveAs),
                C::role(cmd::CLOSE_TAB, Role::CloseTab),
                C::role(cmd::UNDO, Role::Undo).enabled(undo && editable),
                C::role(cmd::REDO, Role::Redo).enabled(redo && editable),
                C::role(cmd::CUT, Role::Cut).enabled(selected && editable),
                C::role(cmd::COPY, Role::Copy).enabled(selected),
                C::role(cmd::PASTE, Role::Paste).enabled(editable),
                C::role(cmd::SELECT_ALL, Role::SelectAll),
                C::role(cmd::FIND, Role::Find).featured(),
                C::role(cmd::FIND_NEXT, Role::FindNext),
                C::own(cmd::FIND_PREV, "Edit", "Find previous", "Ctrl+Shift+G"),
                C::role(cmd::REPLACE, Role::Replace).enabled(editable),
                C::own(cmd::GO_TO_LINE, "Edit", "Go to line…", "Ctrl+L"),
                // The app menu's "− 100% +" Text size row, as the terminal's
                // (see docs/app-settings-milestone-1.md): for now, this window; the lasting
                // size is the editor's own settings window's.
                C::role(cmd::ZOOM_OUT, Role::ZoomOut).enabled(self.zoom > ZOOM_MIN + 0.001).featured(),
                C {
                    label: format!("{}%", (self.zoom * 100.0).round()),
                    ..C::role(cmd::ZOOM_RESET, Role::ZoomReset).featured()
                },
                C::role(cmd::ZOOM_IN, Role::ZoomIn).enabled(self.zoom < ZOOM_MAX - 0.001).featured(),
                C::own(cmd::LINE_NUMBERS, "View", "Line numbers", "")
                    .checked(self.flag("line-numbers", true))
                    .featured(),
                C::own(cmd::WRAP, "View", "Wrap lines", "").checked(self.flag("wrap", true)),
                C::role(cmd::NEXT_TAB, Role::NextTab),
                C::role(cmd::PREV_TAB, Role::PreviousTab),
                C::role(cmd::SETTINGS, Role::Settings),
                C::role(cmd::ABOUT, Role::About),
            ],
        ))
    }

    pub(crate) fn on_input(&mut self, input: &Input, window: &mut Window) -> bool {
        const PAGE_UP: u32 = 0xff55;
        const PAGE_DOWN: u32 = 0xff56;
        const ESCAPE: u32 = 0xff1b;
        let Input::Key { sym, pressed: true, modifiers: m, .. } = input else { return false };
        if *sym == ESCAPE && self.goto.is_some() {
            self.goto = None;
            self.focus_turn += 1;
            return true;
        }
        if *sym == ESCAPE && self.find.is_some() && !self.doc().is_some_and(|d| d.borrow().completing()) {
            self.close_find();
            return true;
        }
        if !(m.ctrl || m.logo) || m.alt {
            return false;
        }
        let letter = char::from_u32(*sym).map(|c| c.to_ascii_lowercase());
        let id = match (letter, *sym) {
            (Some('s'), _) if m.shift => cmd::SAVE_AS,
            (Some('s'), _) => cmd::SAVE,
            (Some('n'), _) => cmd::NEW_WINDOW,
            (Some('o'), _) => cmd::OPEN,
            (Some('t'), _) => cmd::NEW_TAB,
            (Some('w'), _) => cmd::CLOSE_TAB,
            (Some('=' | '+'), _) => cmd::ZOOM_IN,
            (Some('-'), _) => cmd::ZOOM_OUT,
            (Some('0'), _) => cmd::ZOOM_RESET,
            (Some(','), _) => cmd::SETTINGS,
            (Some('f'), _) => cmd::FIND,
            (Some('l'), _) => cmd::GO_TO_LINE,
            (Some('h'), _) => cmd::REPLACE,
            (Some('g'), _) if m.shift => cmd::FIND_PREV,
            (Some('g'), _) => cmd::FIND_NEXT,
            (_, PAGE_DOWN) => cmd::NEXT_TAB,
            (_, PAGE_UP) => cmd::PREV_TAB,
            _ => return false,
        };
        self.run(id, window);
        self.keep_soon(window);
        true
    }
}
