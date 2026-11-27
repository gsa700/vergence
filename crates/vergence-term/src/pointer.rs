//! The mouse: selecting text, the scrollbar's thumb, the lines between
//! panes, the right-click menu, and mouse reports to a program that asked
//! for them. The window itself is in main.rs.

use std::time::Instant;

use vergence_scene::Rect;
use vergence_ui::{Button, Cursor, Edges, Input, Modifiers, Window, sym};

use crate::commands::command_of;
use crate::grid::MouseMode;
use crate::menu::{self, Menu};
use crate::panes::Axis;
use crate::scrollbar::Scrollbar;
use crate::select::{Selection, Unit};
use crate::session::lock;
use crate::{DOUBLE_CLICK, Held, PAD, Terminal, mouse};

impl Terminal {
    /// The left button went down on the active pane's text: a selection
    /// starts (by characters, then words, then lines for presses on the
    /// same cell in quick succession), or with Shift the one there is
    /// reaches to here.
    fn press_text(&mut self, x: f32, y: f32, m: Modifiers, window: &mut Window) {
        let (Some(cell), Some((_, area)), Some(session)) = (self.cell(), self.active(), self.front()) else { return };
        let mut guard = lock(&session.screen);
        let s = &mut *guard;
        // A program that asked for the mouse has it (report()); Shift takes it
        // back, and scrolled back the text under the pointer is ours.
        if s.grid.mouse != MouseMode::Off && !m.shift && s.scroll == 0 {
            return;
        }
        let pos = s.cell_at(x, y, area, cell);
        let now = Instant::now();
        let nth = match self.presses {
            Some((then, at, n)) if at == pos && now.duration_since(then) < DOUBLE_CLICK => n % 3 + 1,
            _ => 1,
        };
        if nth == 1 && m.shift && s.selection.is_some() {
            s.drag_selection(pos);
        } else {
            s.selection = Some(Selection::start(&s.grid, [Unit::Cell, Unit::Word, Unit::Line][nth - 1], pos));
        }
        drop(guard);
        self.presses = Some((now, pos, nth));
        self.held = Some(Held::Text);
        window.redraw();
    }

    /// The active pane's selected text, if there is any.
    fn selected_text(&self) -> Option<String> {
        let s = lock(&self.front()?.screen);
        s.selection.and_then(|sel| sel.text(&s.grid))
    }

    /// Copies what is selected in the active pane; false if nothing is.
    pub(crate) fn copy(&mut self, window: &mut Window) -> bool {
        match self.selected_text() {
            Some(text) => {
                window.copy(&text);
                true
            }
            None => false,
        }
    }

    /// Input while the menu is open is the menu's: a press on a line does
    /// what it says, a press anywhere else or Escape closes it. True if the
    /// input was used up here.
    pub(crate) fn menu_input(&mut self, input: &Input, window: &mut Window) -> bool {
        let Some(menu) = &self.menu else { return false };
        match input {
            Input::PointerMoved { x, y } => {
                let over = menu.at(*x, *y);
                window.set_cursor(Cursor::Default);
                if over != self.menu_hover {
                    self.menu_hover = over;
                    window.redraw();
                }
                true
            }
            Input::Button { button, pressed: true, x, y, .. } => {
                let chosen = if *button == Button::Left { menu.at(*x, *y) } else { None };
                // A press on the menu's own ground, between its lines, leaves it open.
                if chosen.is_none() && *button == Button::Left && menu.contains(*x, *y) {
                    return true;
                }
                self.menu = None;
                self.menu_hover = None;
                window.redraw();
                if let Some(item) = chosen {
                    self.do_item(item, window);
                }
                true
            }
            Input::Button { .. } | Input::Scroll { .. } => true,
            Input::Key { sym: sym::ESCAPE, pressed: true, .. } | Input::Focus(false) => {
                self.menu = None;
                self.menu_hover = None;
                window.redraw();
                matches!(input, Input::Key { .. })
            }
            _ => false,
        }
    }

    /// The scrollbar of a pane of the front tab, if it has one now.
    fn scrollbar(&self, id: u64, area: Rect) -> Option<Scrollbar> {
        lock(&self.session(id)?.screen).scrollbar(area)
    }

    /// The held thumb follows the pointer, and the active pane's view with it.
    pub(crate) fn drag_thumb(&mut self, y: f32, grab: f32, window: &mut Window) {
        let (Some((_, area)), Some(session)) = (self.active(), self.front()) else { return };
        let mut s = lock(&session.screen);
        if let Some(bar) = s.scrollbar(area) {
            let to = bar.scroll_for(y - grab);
            if to != s.scroll {
                s.scroll = to;
                s.clamp_scroll();
                window.redraw();
            }
        }
    }

    /// Tells the program in a pane of the front tab about a mouse event at
    /// a point of the window, if it asked for such events. True if the
    /// event was the program's.
    pub(crate) fn report(&mut self, id: u64, area: Rect, action: mouse::Action, x: f32, y: f32, m: Modifiers) -> bool {
        let Some(cell) = self.cell() else { return false };
        let last = self.reported_cell;
        let Some(session) = self.session_mut(id) else { return false };
        let s = lock(&session.screen);
        if s.grid.mouse == MouseMode::Off || s.scroll > 0 {
            // Scrolled back, what is under the pointer is not the program's screen.
            return false;
        }
        let at = s.screen_cell(x, y, area, cell);
        if matches!(action, mouse::Action::Motion(_)) && last == Some((id, at)) {
            return true;
        }
        let bytes = mouse::bytes(s.grid.mouse, s.grid.mouse_sgr, action, at, m);
        drop(s);
        if let Some(bytes) = bytes {
            session.pty.write(&bytes);
        }
        self.reported_cell = Some((id, at));
        true
    }

    /// The pointer moved with the button held on the active pane's text.
    pub(crate) fn drag_text(&mut self, x: f32, y: f32, window: &mut Window) {
        let (Some(cell), Some((_, area)), Some(session)) = (self.cell(), self.active(), self.front()) else { return };
        let mut s = lock(&session.screen);
        let pos = s.cell_at(x, y, area, cell);
        let bottom = area.y + PAD + s.grid.rows as f32 * cell.1;
        // Past the top or bottom edge the view scrolls for as long as it is held there (scene()).
        let edge = if y < area.y + PAD {
            Some((true, pos.0))
        } else if y >= bottom {
            Some((false, pos.0))
        } else {
            None
        };
        if s.drag_selection(pos) || edge.is_some() {
            window.redraw();
        }
        drop(s);
        self.edge = edge;
    }

    /// The pointer moved with nothing held: what it is over, the shape it
    /// takes, and word of it to a program that asked to hear.
    pub(crate) fn hover(&mut self, x: f32, y: f32, window: &mut Window) {
        let part = self.bar_at(x, y);
        let pane = if part.is_none() && y >= self.bar_h { self.pane_at(x, y) } else { None };
        self.over = pane.map(|(id, _)| id);
        if pane.is_none() && self.bar_near.take().is_some() {
            window.redraw();
        }
        let divider = if part.is_none() { self.areas().1.into_iter().find(|d| d.holds(x, y)) } else { None };
        let cursor = if let Some(d) = divider {
            Cursor::Resize(if d.axis == Axis::Side {
                Edges { right: true, ..Edges::default() }
            } else {
                Edges { bottom: true, ..Edges::default() }
            })
        } else if let Some((id, area)) = pane {
            // The scrollbar's edge and a program that has the mouse get the arrow; text, the text bar.
            let on_bar = self.scrollbar(id, area).is_some_and(|bar| bar.holds(x, y));
            // The bar widens under the pointer, as the toolkit's do (Dave's wish, 2026-11-27).
            let near = on_bar.then_some(id);
            if near != self.bar_near {
                self.bar_near = near;
                window.redraw();
            }
            let theirs = !on_bar
                && !self.shift
                && self.report(id, area, mouse::Action::Motion(None), x, y, Modifiers::default());
            if on_bar || theirs { Cursor::Default } else { Cursor::Text }
        } else {
            Cursor::Default
        };
        window.set_cursor(cursor);
        if part != self.hover {
            self.hover = part;
            window.redraw();
        }
    }

    /// A button went down below the bar.
    pub(crate) fn press(&mut self, button: Button, x: f32, y: f32, m: Modifiers, window: &mut Window) {
        // The line between two panes: the left button takes hold of it.
        if button == Button::Left
            && let Some(d) = self.areas().1.into_iter().find(|d| d.holds(x, y))
        {
            self.held = Some(Held::Divider(d.path));
            return;
        }
        let Some((id, area)) = self.pane_at(x, y) else { return };
        // A press on a pane makes it the one the keys go to.
        if let Some(tab) = self.tabs.get_mut(self.shown)
            && tab.active != id
        {
            tab.active = id;
            self.retitle(window);
            window.redraw();
        }
        // The scrollbar: a press takes hold of the thumb, which comes to the pointer if it was not under it.
        if button == Button::Left
            && let Some(bar) = self.scrollbar(id, area).filter(|bar| bar.holds(x, y))
        {
            let grab = bar.grab(y);
            self.held = Some(Held::Thumb(grab));
            return self.drag_thumb(y, grab, window);
        }
        if !m.shift && self.report(id, area, mouse::Action::Press(button), x, y, m) {
            self.held = Some(Held::Reported(button));
            return;
        }
        match button {
            Button::Left => self.press_text(x, y, m, window),
            // The server draws it where it draws the bar (the native road), as
            // every window's right-click menu; a chosen line comes back as the
            // command (Self::command). Elsewhere, drawn here.
            Button::Right if !self.own_bar => {
                window.menu(x, y, menu::server_lines(self.selected_text().is_some(), command_of));
            }
            Button::Right => {
                self.menu = Some(Menu::new(x, y, (self.width, self.height), self.selected_text().is_some()));
                self.menu_hover = None;
                window.redraw();
            }
            _ => {}
        }
    }
}
