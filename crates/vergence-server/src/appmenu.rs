// The app menu (app settings milestone 1): what a window's app button opens,
// for every app, native or not. Its lines and their behavior are
// vergence-shell's appmenu (plain data, tested there); here it is opened
// under the button, drawn, given the pointer and the keys, and what is
// chosen is carried out: an app's command goes to the app, the skeleton's
// own lines (About, Settings…, Quit for an app that sent none) and the
// Window menu are the desktop's. A right click on one of the app's commands
// adds it to the first panel or takes it off (Dave, 2026-11-14): kept per
// app in ~/.local/state/vergence/app-menu.toml (owner only), by the
// command's key (its role, or its menu and label), not its id.

use std::hash::{Hash, Hasher};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{Kind, memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement}},
            gles::GlesRenderer,
        },
    },
    input::keyboard::keysyms as key,
    utils::{Logical, Point, Rectangle, Transform},
};
use vergence_scene::{Color, Point as ScenePoint, Rect as SceneRect, Scene};
use vergence_shell::appmenu::{self, Action, Did, Line, Model, State, WindowAct};

use vergence_protocol::window::{self, MenuLine};

use crate::{Server, decor::SceneElement, win::Win};

#[derive(Default)]
pub struct AppMenu {
    open: Option<Open>,
    /// A press in the search field, held: moving selects (a drag).
    field_drag: bool,
    /// The two panels' pictures (top, submenu), each with what it was drawn from.
    pictures: [Option<(u64, MemoryRenderBuffer)>; 2],
}

struct Open {
    /// The window it is of (None: the launcher, which is the desktop's).
    win: Option<Win>,
    /// The app, as its pins are kept (its windows' class).
    app_key: String,
    model: Model,
    state: State,
    /// The top panel's top left corner, and whether submenus open to its left
    /// (the button was in the window's right half: the menu opens toward the
    /// middle).
    at: Point<i32, Logical>,
    leftward: bool,
    /// The screen it is on: panels are kept inside it.
    screen: Rectangle<i32, Logical>,
    /// A window's right-click menu (bus v20), not its app button's: the
    /// button is not lit and nothing is pinned.
    context: bool,
    /// The desktop's own lines in its right-click menu, by id, and what each
    /// does (menu.rs): carried out here, not sent to the desktop's program.
    own: Vec<(u32, crate::menu::Entry)>,
    /// The launcher (launcher.rs): the desktop's own menu of applications.
    launcher: bool,
    /// Where its bottom stays (the launcher over a dock at the bottom: it
    /// grows upward as what it shows changes).
    bottom: Option<i32>,
}

impl Open {
    /// The top panel's top left: `at`, or, held by its bottom (the launcher
    /// over a dock at the bottom), as high as its lines are now.
    fn origin(&self) -> Point<i32, Logical> {
        match self.bottom {
            Some(b) => (self.at.x, (b - appmenu::height(&self.top_lines())).max(self.screen.loc.y)).into(),
            None => self.at,
        }
    }

    fn top_lines(&self) -> Vec<Line> {
        self.state.shown(&self.model)
    }

    fn top_rect(&self) -> Rectangle<i32, Logical> {
        Rectangle::new(self.origin(), (appmenu::WIDTH, appmenu::height(&self.top_lines())).into())
    }

    /// The open submenu's panel, beside the line it hangs from, kept on the screen.
    fn sub_rect(&self) -> Option<Rectangle<i32, Logical>> {
        let i = self.state.open?;
        let lines = self.top_lines();
        let row = *appmenu::rows((self.origin().x, self.origin().y), &lines).get(i)?;
        let sub = self.state.sub_lines(&self.model);
        if sub.is_empty() {
            return None;
        }
        let h = appmenu::height(&sub);
        let mut x = if self.leftward { self.origin().x - appmenu::WIDTH + 4 } else { self.origin().x + appmenu::WIDTH - 4 };
        if x < self.screen.loc.x || x + appmenu::WIDTH > self.screen.loc.x + self.screen.size.w {
            x = if self.leftward { self.origin().x + appmenu::WIDTH - 4 } else { self.origin().x - appmenu::WIDTH + 4 };
        }
        let y = (row.y - appmenu::PAD).min(self.screen.loc.y + self.screen.size.h - h).max(self.screen.loc.y);
        Some(Rectangle::new((x, y).into(), (appmenu::WIDTH, h).into()))
    }

    /// Which line of which panel is under `pos`: (submenu?, line).
    fn line_at(&self, pos: Point<f64, Logical>) -> Option<(bool, Option<usize>)> {
        let find = |at: Point<i32, Logical>, lines: &[Line]| appmenu::rows((at.x, at.y), lines).iter().position(|r| (r.x as f64..(r.x + r.w) as f64).contains(&pos.x) && (r.y as f64..(r.y + r.h) as f64).contains(&pos.y));
        if let Some(s) = self.sub_rect()
            && s.to_f64().contains(pos)
        {
            return Some((true, find(s.loc, &self.state.sub_lines(&self.model))));
        }
        self.top_rect().to_f64().contains(pos).then(|| (false, find(self.origin(), &self.top_lines())))
    }
}

impl Server {
    pub fn app_menu_is_open(&self) -> bool {
        self.app_menu.open.is_some()
    }

    /// The window whose app menu is open (its button stays lit).
    pub fn app_menu_window(&self) -> Option<&Win> {
        self.app_menu.open.as_ref().filter(|o| !o.context).and_then(|o| o.win.as_ref())
    }

    pub fn app_menu_contains(&self, pos: Point<f64, Logical>) -> bool {
        self.app_menu.open.as_ref().is_some_and(|o| o.line_at(pos).is_some())
    }

    pub fn app_menu_close(&mut self) {
        if self.app_menu.open.take().is_some() {
            self.app_menu.pictures = [None, None];
            self.redraw_all();
        }
    }

    /// The app button was pressed, or Alt+Space: open the window's app menu
    /// under its button (closed if it was already open).
    pub fn app_menu_toggle(&mut self, w: &Win, by_key: bool) {
        if self.app_menu.open.as_ref().is_some_and(|o| o.win.as_ref() == Some(w)) {
            return self.app_menu_close();
        }
        self.close_menu();
        self.launcher_close();
        let app_key = self.app_key(w);
        let mut model = self.app_model(w);
        model.pins = pins_of(&app_key);
        let Some((bar, layout)) = self.bar_of(w) else { return };
        let button = layout.buttons.iter().find(|(b, _)| *b == vergence_bar::WinButton::AppMenu).map(|(_, r)| *r);
        let (bx, bw, by) = match button {
            Some(r) => (bar.loc.x + r.x as i32, r.w as i32, bar.loc.y + bar.size.h),
            None => (bar.loc.x, 0, bar.loc.y + bar.size.h),
        };
        let leftward = bx + bw / 2 > bar.loc.x + bar.size.w / 2;
        let screen = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).find(|g| g.contains((bx, by))).or_else(|| self.output_at_pointer()).unwrap_or(bar);
        let mut state = State::default();
        let lines = state.shown(&model);
        let h = appmenu::height(&lines);
        let mut x = if leftward { bx + bw - appmenu::WIDTH } else { bx };
        x = x.min(screen.loc.x + screen.size.w - appmenu::WIDTH).max(screen.loc.x);
        let y = (by + 2).min(screen.loc.y + screen.size.h - h).max(screen.loc.y);
        // Opened by the keyboard, the first line is chosen, so Enter and the arrows work at once.
        if by_key {
            state.key(&model, appmenu::Key::Down);
        }
        tracing::info!("app menu: opened for {:?} ({} commands)", model.app, model.commands.len());
        self.app_menu.open = Some(Open { win: Some(w.clone()), app_key, model, state, at: (x, y).into(), leftward, screen, context: false, own: Vec::new(), launcher: false, bottom: None });
        self.app_menu.pictures = [None, None];
        self.redraw_all();
    }

    /// A window's own right-click menu (bus v20 `WinRequest::Menu`): its
    /// lines at `pos`, drawn and worked as the app menu is (faint lines,
    /// keys, submenus, the arrows), kept on the screen; submenus open to the
    /// right unless the menu is in the screen's right part.
    pub fn app_menu_lines(&mut self, w: &Win, pos: Point<f64, Logical>, lines: Vec<vergence_protocol::window::MenuLine>) {
        // Nothing to show, or not over the window any more (a stale request).
        if lines.is_empty() || crate::greeter() {
            return;
        }
        self.close_menu();
        self.launcher_close();
        // The desktop's own lines where its menu marks them; elsewhere the mark is dropped.
        let mut own = Vec::new();
        let mut lines = lines;
        if let Some(i) = lines.iter().position(|l| matches!(l, MenuLine::Item { id, .. } if *id == window::MENU_DESKTOP_LINES)) {
            lines.remove(i);
            if self.is_desktop(w) {
                let (mine, acts) = crate::menu::desktop_own_lines(window::MENU_SERVER_IDS);
                own = acts;
                lines.splice(i..i, mine);
            }
            lines.retain(|l| !matches!(l, MenuLine::Item { id, .. } if *id == window::MENU_DESKTOP_LINES));
            // No separator first, last or twice running.
            let mut tidy: Vec<MenuLine> = Vec::new();
            for l in lines {
                if l == MenuLine::Separator && tidy.last().is_none_or(|p| *p == MenuLine::Separator) {
                    continue;
                }
                tidy.push(l);
            }
            if tidy.last() == Some(&MenuLine::Separator) {
                tidy.pop();
            }
            tidy.truncate(window::MAX_MENU_LINES);
            lines = tidy;
        }
        if lines.is_empty() {
            return;
        }
        let model = Model { app: String::new(), commands: Vec::new(), terminal: false, native: true, window: Vec::new(), pins: Vec::new(), given: Some(lines), searchable: false };
        let state = State::default();
        let at = pos.to_i32_round::<i32>();
        let screen = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).find(|g| g.contains(at)).or_else(|| self.output_at_pointer()).unwrap_or(Rectangle::new(at, (appmenu::WIDTH, 1).into()));
        let h = appmenu::height(&state.shown(&model));
        let x = at.x.min(screen.loc.x + screen.size.w - appmenu::WIDTH).max(screen.loc.x);
        // Below the pointer, or above it if there is no room below.
        let y = if at.y + h <= screen.loc.y + screen.size.h { at.y } else { (at.y - h).max(screen.loc.y) };
        let leftward = x + appmenu::WIDTH * 2 > screen.loc.x + screen.size.w;
        tracing::info!("right-click menu: {} lines", model.given.as_ref().map_or(0, Vec::len));
        self.app_menu.open = Some(Open { win: Some(w.clone()), app_key: String::new(), model, state, at: (x, y).into(), leftward, screen, context: true, own, launcher: false, bottom: None });
        self.app_menu.pictures = [None, None];
        self.redraw_all();
    }

    /// The launcher (menus milestone 1, Dave 2026-11-16: a text menu pinned
    /// to the dock's button, as a start menu): these lines with a search
    /// field, at `at` (its top left, kept on `screen`), every line carried
    /// out here (`own`).
    pub(crate) fn app_menu_launcher(&mut self, at: Point<i32, Logical>, bottom: Option<i32>, screen: Rectangle<i32, Logical>, lines: Vec<MenuLine>, own: Vec<(u32, crate::menu::Entry)>) {
        if lines.is_empty() || crate::greeter() {
            return;
        }
        self.close_menu();
        let model = Model { app: String::new(), commands: Vec::new(), terminal: false, native: true, window: Vec::new(), pins: Vec::new(), given: Some(lines), searchable: true };
        let state = State::default();
        let x = at.x.min(screen.loc.x + screen.size.w - appmenu::WIDTH).max(screen.loc.x);
        let leftward = x + appmenu::WIDTH * 2 > screen.loc.x + screen.size.w;
        self.app_menu.open = Some(Open { win: None, app_key: String::new(), model, state, at: (x, at.y).into(), leftward, screen, context: true, own, launcher: true, bottom });
        self.app_menu.pictures = [None, None];
        self.redraw_all();
    }

    /// Whether the open menu is the launcher.
    pub fn app_menu_is_launcher(&self) -> bool {
        self.app_menu.open.as_ref().is_some_and(|o| o.launcher)
    }

    /// The launcher's height for these lines (its search field included), to place it.
    pub fn launcher_height(lines: &[MenuLine]) -> i32 {
        let model = Model { app: String::new(), commands: Vec::new(), terminal: false, native: true, window: Vec::new(), pins: Vec::new(), given: Some(lines.to_vec()), searchable: true };
        appmenu::height(&model.top())
    }

    /// The app a window belongs to, as pins and the settings app name it.
    fn app_key(&mut self, w: &Win) -> String {
        self.window_identity(w).map(|i| if i.class.is_empty() { i.instance } else { i.class }).unwrap_or_default()
    }

    /// What a window's app menu is made from: its app's name (the dock's),
    /// its commands, and the Window menu's lines for it now.
    fn app_model(&mut self, w: &Win) -> Model {
        let items = self.shelf_items();
        let app = items.iter().find(|(_, ws)| ws.contains(w)).map(|(i, _)| i.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| self.window_title(w));
        let (commands, terminal) = w.native().map(|n| n.commands()).unwrap_or_default();
        // No Window menu (Dave, 2026-11-14): the window's layouts are on its
        // title bar (right click, long press) and its maximize button, and
        // Super with the arrows; the app menu is the app's.
        let window: Vec<(WindowAct, String)> = Vec::new();
        Model { app, commands, terminal, native: w.native().is_some(), window, pins: Vec::new(), given: None, searchable: false }
    }

    /// A window sent new commands: an open menu of it shows them (the Text
    /// size row's percentage after a press).
    pub fn app_menu_refresh(&mut self, w: &Win) {
        let Some(o) = self.app_menu.open.as_mut().filter(|o| o.win.as_ref() == Some(w)) else { return };
        if let Some(n) = w.native() {
            (o.model.commands, o.model.terminal) = n.commands();
            self.redraw_all();
        }
    }

    /// Ctrl+V in the search field: the clipboard's text, when it comes.
    fn app_menu_paste_request(&mut self) {
        self.server_paste(|st, text| {
            if let Some(o) = st.app_menu.open.as_mut() {
                o.state.paste(&o.model, &text);
                st.redraw_all();
            }
        });
    }

    /// A button let go: a drag in the search field ends.
    pub fn app_menu_release(&mut self) {
        self.app_menu.field_drag = false;
    }

    /// The byte of the search text nearest `pos`, if `pos` is in the field.
    fn app_menu_field_index(&mut self, pos: Point<f64, Logical>) -> Option<usize> {
        let o = self.app_menu.open.as_ref()?;
        let lines = o.top_lines();
        let i = lines.iter().position(|l| matches!(l, Line::Search))?;
        let at = o.origin();
        let r = appmenu::rows((at.x, at.y), &lines)[i];
        let inside_y = (r.y as f64..(r.y + r.h) as f64).contains(&pos.y);
        if !self.app_menu.field_drag && !(inside_y && (r.x as f64..(r.x + r.w) as f64).contains(&pos.x)) {
            return None;
        }
        let typed = o.state.typed.clone();
        // The text starts 28 points in (app_menu_draw), at 13 points.
        let x = (pos.x - r.x as f64 - 28.0) as f32;
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        let face = fonts.face("sans-serif", 400, false)?;
        let mut best = (0usize, f32::MAX);
        let mut ends: Vec<usize> = typed.char_indices().map(|(b, _)| b).collect();
        ends.push(typed.len());
        for b in ends {
            let w = fonts.layout(face, 13.0, &typed[..b]).1;
            let d = (w - x).abs();
            if d < best.1 {
                best = (b, d);
            }
        }
        Some(best.0)
    }

    /// The pointer moved: the line under it is chosen, a submenu there opens.
    pub fn app_menu_hover(&mut self, pos: Point<f64, Logical>) {
        if self.app_menu.field_drag {
            if let Some(at) = self.app_menu_field_index(pos)
                && let Some(o) = self.app_menu.open.as_mut()
            {
                o.state.place(at, true);
                self.redraw_all();
            }
            return;
        }
        let Some(o) = self.app_menu.open.as_mut() else { return };
        let before = o.state.clone();
        match o.line_at(pos) {
            Some((true, i)) => o.state.hover_sub(&o.model, i),
            Some((false, i)) => o.state.hover_top(&o.model, i),
            None => {}
        }
        if o.state != before {
            self.redraw_all();
        }
    }

    pub fn app_menu_press(&mut self, pos: Point<f64, Logical>, button: u32) {
        // In the search field: the caret goes there (Shift: the selection to
        // there), and holding the button and moving selects.
        if button == vergence_seat::BTN_LEFT && let Some(at) = self.app_menu_field_index(pos) {
            let shift = self.input.keys.as_ref().is_some_and(|k| k.mods().shift);
            if let Some(o) = self.app_menu.open.as_mut() {
                o.state.place(at, shift);
            }
            self.app_menu.field_drag = true;
            self.redraw_all();
            return;
        }
        self.app_menu_hover(pos);
        if button == vergence_seat::BTN_RIGHT {
            return self.app_menu_pin(pos);
        }
        let Some(o) = self.app_menu.open.as_ref() else { return };
        let action = match o.line_at(pos) {
            Some((true, Some(i))) => match o.state.sub_lines(&o.model).get(i) {
                Some(Line::Item { action, enabled: true, .. }) => Some(*action),
                _ => None,
            },
            Some((false, Some(i))) => match o.top_lines().get(i) {
                Some(Line::Item { action, enabled: true, .. }) => Some(*action),
                // The Text size row: whichever of its buttons is under the pointer.
                Some(Line::Zoom { out, reset, grow, .. }) => {
                    let row = { let at = o.origin(); appmenu::rows((at.x, at.y), &o.top_lines()) }[i];
                    let inside = |r: vergence_shell::Rect| (r.x as f64..(r.x + r.w) as f64).contains(&pos.x);
                    let [a, b, c] = appmenu::zoom_parts(row);
                    [(a, *out), (b, *reset), (c, *grow)].into_iter().find(|(r, id)| *id != 0 && inside(*r)).map(|(_, id)| Action::Command(id))
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(a) = action {
            self.app_menu_do(a);
        }
    }

    /// A right click on one of the app's commands: onto the first panel, or
    /// off it. The menu stays open and shows the change.
    fn app_menu_pin(&mut self, pos: Point<f64, Logical>) {
        let Some(o) = self.app_menu.open.as_mut() else { return };
        let line = match o.line_at(pos) {
            Some((true, Some(i))) => o.state.sub_lines(&o.model).get(i).cloned(),
            Some((false, Some(i))) => o.top_lines().get(i).cloned(),
            _ => None,
        };
        let Some(Line::Item { pin: Some(key), .. }) = line else { return };
        // The app's own everyday few are its to choose: not added, not taken off.
        if o.model.commands.iter().any(|c| c.featured && c.key() == key) {
            return;
        }
        if let Some(at) = o.model.pins.iter().position(|k| *k == key) {
            o.model.pins.remove(at);
        } else {
            o.model.pins.push(key.clone());
        }
        tracing::info!("app menu: {:?} pins now {:?}", o.app_key, o.model.pins);
        keep_pins(&o.app_key, &o.model.pins);
        // Lines moved: what was chosen may be elsewhere now.
        o.state.top = None;
        if o.state.sub.is_none() {
            o.state.open = None;
        }
        self.redraw_all();
    }

    /// A key while the menu is open (the seat gives it every key).
    pub fn app_menu_key(&mut self, sym: u32, typed: Option<char>, mods: vergence_seat::keyboard::Mods) {
        // The search field's clipboard keys (a field like any other).
        if mods.ctrl && !mods.alt {
            match sym {
                key::KEY_c | key::KEY_C | key::KEY_x | key::KEY_X => {
                    let cut = matches!(sym, key::KEY_x | key::KEY_X);
                    let Some(o) = self.app_menu.open.as_mut() else { return };
                    let text = if cut { o.state.cut(&o.model) } else { o.state.selected() };
                    if let Some(t) = text {
                        self.server_copy(t);
                        self.redraw_all();
                    }
                    return;
                }
                key::KEY_v | key::KEY_V => return self.app_menu_paste_request(),
                _ => {}
            }
        }
        let Some(o) = self.app_menu.open.as_mut() else { return };
        use appmenu::Key as K;
        let k = match sym {
            key::KEY_Up => K::Up,
            key::KEY_Down => K::Down,
            key::KEY_Left if mods.shift => K::SelectLeft,
            key::KEY_Right if mods.shift => K::SelectRight,
            key::KEY_Left => K::Left,
            key::KEY_Right => K::Right,
            key::KEY_Home | key::KEY_KP_Home if mods.shift => K::SelectHome,
            key::KEY_End | key::KEY_KP_End if mods.shift => K::SelectEnd,
            key::KEY_Home | key::KEY_KP_Home => K::Home,
            key::KEY_End | key::KEY_KP_End => K::End,
            key::KEY_Delete | key::KEY_KP_Delete => K::Delete,
            key::KEY_Return | key::KEY_KP_Enter => K::Enter,
            key::KEY_Escape => K::Escape,
            key::KEY_BackSpace if mods.ctrl => K::BackWord,
            key::KEY_BackSpace => K::Back,
            key::KEY_a | key::KEY_A if mods.ctrl && !mods.alt => K::SelectAll,
            _ if mods.ctrl || mods.alt || mods.logo => return,
            _ => match typed {
                Some(c) if !c.is_control() => K::Char(c),
                _ => return,
            },
        };
        match o.state.key(&o.model, k) {
            Did::Nothing => {}
            Did::Moved => self.redraw_all(),
            Did::Close => self.app_menu_close(),
            Did::Chosen(a) => self.app_menu_do(a),
        }
    }

    /// Carry out what was chosen; the menu closes first, except for the Text
    /// size row's buttons, which may be pressed again and again.
    fn app_menu_do(&mut self, action: Action) {
        if let (Action::Command(id), Some(o)) = (action, self.app_menu.open.as_ref())
            && o.model.commands.iter().any(|c| c.id == id && matches!(c.role, vergence_protocol::commands::Role::ZoomIn | vergence_protocol::commands::Role::ZoomOut | vergence_protocol::commands::Role::ZoomReset))
        {
            if let Some(n) = o.win.as_ref().and_then(|w| w.native()) {
                self.bus_send(n.client, vergence_protocol::bus::ServerMsg::Window(vergence_protocol::window::WinEvent::Command { window: n.id, id }));
            }
            return;
        }
        let Some(o) = self.app_menu.open.take() else { return };
        self.app_menu.pictures = [None, None];
        self.redraw_all();
        tracing::info!("app menu: {action:?} for {:?}", o.model.app);
        // One of the desktop's own lines: done here, the program told nothing.
        if let Action::Command(id) = action
            && let Some((_, e)) = o.own.iter().find(|(i, _)| *i == id)
        {
            let e = e.clone();
            self.menu_run(&e);
            return;
        }
        let Some(w) = o.win else { return };
        if !smithay::utils::IsAlive::alive(&w) {
            return;
        }
        match action {
            Action::Command(id) => {
                if let Some(n) = w.native() {
                    self.bus_send(n.client, vergence_protocol::bus::ServerMsg::Window(vergence_protocol::window::WinEvent::Command { window: n.id, id }));
                }
            }
            Action::About => self.app_about(&w, &o.model.app),
            Action::Settings => {
                // The desktop's part of the app's settings, in the settings app
                // (app settings milestone 1, step 6).
                // Lower case, as desktop.toml's [app."<id>"] tables are named.
                let id = o.app_key.to_lowercase();
                if let Err(e) = crate::spawn(std::process::Command::new("vergence-settings").args(["--app", &id])) {
                    tracing::warn!("app menu: vergence-settings --app {id}: {e}");
                }
            }
            Action::Quit => {
                // Every window of the app, asked politely as its close button asks.
                let items = self.shelf_items();
                let theirs = items.into_iter().find(|(_, ws)| ws.contains(&w)).map(|(_, ws)| ws).unwrap_or_else(|| vec![w.clone()]);
                for t in theirs {
                    self.close_window(&t);
                }
            }
            Action::Window(a) => {
                let act = match a {
                    WindowAct::Snap(z) => crate::menu::LayoutAct::Snap(z),
                    WindowAct::Maximize => crate::menu::LayoutAct::Maximize,
                    WindowAct::Restore => crate::menu::LayoutAct::Restore,
                    WindowAct::Minimize => crate::menu::LayoutAct::Minimize,
                    WindowAct::Close => crate::menu::LayoutAct::Close,
                };
                self.layout_act(&w, act);
            }
        }
    }

    /// About an app that has no About of its own: a card with what its
    /// desktop entry says of it.
    fn app_about(&mut self, w: &Win, name: &str) {
        let items = self.shelf_items();
        let key = items.iter().find(|(_, ws)| ws.contains(w)).map(|(i, _)| i.key.clone()).unwrap_or_default();
        let entry = self.shelf.apps.iter().find(|a| a.id == key).cloned();
        let body = match &entry {
            Some(e) => [e.generic_name.as_str(), e.comment.as_str()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(". "),
            None => String::new(),
        };
        let req = crate::notify::Request { app: String::new(), replaces: 0, summary: name.to_string(), body, urgency: 1, timeout_ms: -1, actions: Vec::new() };
        self.notify(crate::notify::Origin::Server, req);
    }

    /// The open menu's panels on `out`, front to back: the submenu over the top panel.
    pub fn app_menu_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        let Some(o) = self.app_menu.open.as_ref() else { return Vec::new() };
        let light = self.light_style();
        let mut panels: Vec<(usize, Rectangle<i32, Logical>, Vec<Line>, Option<usize>, String, bool)> = Vec::new();
        let field = (o.state.caret, o.state.anchor);
        if let Some(r) = o.sub_rect() {
            panels.push((1, r, o.state.sub_lines(&o.model), o.state.sub, String::new(), true));
        }
        let top = o.top_lines();
        let found_menus: Vec<String> = if o.state.typed.trim().is_empty() { Vec::new() } else { o.model.search(&o.state.typed).into_iter().map(|(_, m)| m).collect() };
        panels.push((0, o.top_rect(), top, o.state.top, o.state.typed.clone(), o.state.sub.is_none()));
        let open_sub = o.state.open;
        let mut elements = Vec::new();
        for (slot, rect, lines, chosen, typed, keyboard_here) in panels {
            if !rect.overlaps(out) {
                continue;
            }
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (format!("{lines:?}"), chosen, &typed, field, keyboard_here, open_sub, scale.to_bits(), light, rect.size.w, rect.size.h).hash(&mut hasher);
            let stamp = hasher.finish();
            if self.app_menu.pictures[slot].as_ref().map(|(k, _)| *k) != Some(stamp) {
                let menus = if slot == 0 { found_menus.clone() } else { Vec::new() };
                match self.app_menu_draw(rect, &lines, chosen, (&typed, field), slot == 0 && !typed.trim().is_empty(), &menus, open_sub.filter(|_| slot == 0), light, scale) {
                    Some(b) => self.app_menu.pictures[slot] = Some((stamp, b)),
                    None => continue,
                }
            }
            let Some((_, buffer)) = &self.app_menu.pictures[slot] else { continue };
            let a = (rect.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            let b = (rect.loc + rect.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
            if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(rect.size), Kind::Unspecified) {
                elements.push(SceneElement::Memory(el));
            }
        }
        elements
    }

    /// One panel's picture.
    #[allow(clippy::too_many_arguments)]
    fn app_menu_draw(&mut self, rect: Rectangle<i32, Logical>, lines: &[Line], chosen: Option<usize>, (typed, (caret, anchor)): (&str, (usize, usize)), results: bool, menus: &[String], open_sub: Option<usize>, light: bool, scale: f64) -> Option<MemoryRenderBuffer> {
        // The launcher finds applications; an app's menu, its commands.
        let placeholder = if self.app_menu_is_launcher() { "Search applications…" } else { "Search commands…" };
        let accent = self.accent();
        let s = vergence_look::shelf_look(light);
        let color = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
        let (w, h) = (rect.size.w as f32, rect.size.h as f32);
        let mut scene = Scene::new(w, h);
        crate::menu::menu_ground(&mut scene, w, h, light);
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        let face = fonts.face("sans-serif", 400, false);
        let rows = appmenu::rows((0, 0), lines);
        let gutter = if lines.iter().any(|l| matches!(l, Line::Item { checked: Some(_), .. })) { 18.0 } else { 0.0 };
        for (i, (line, r)) in lines.iter().zip(&rows).enumerate() {
            let r = SceneRect::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32);
            let lit = chosen == Some(i) || (open_sub == Some(i) && matches!(line, Line::Sub(_)));
            match line {
                Line::Separator => {
                    crate::menu::menu_separator(&mut scene, r, light);
                }
                Line::Search => {
                    let f = SceneRect::new(r.x, r.y + 2.0, r.w, r.h - 6.0);
                    scene.fill_rounded(f, vergence_look::MENU_ROW_RADIUS, color(s.field));
                    // The menu has the keyboard, so the field always shows it
                    // has: an outline in the accent, and a caret (Dave: it
                    // looked as if it could not be typed in until typed in).
                    scene.stroke(f, vergence_look::MENU_ROW_RADIUS, vergence_look::EDGE, Color::rgb(accent.r, accent.g, accent.b));
                    // A magnifier.
                    let (cx, cy) = (f.x + 14.0, f.y + f.h / 2.0 - 1.0);
                    scene.stroke(SceneRect::new(cx - 4.5, cy - 4.5, 9.0, 9.0), 4.5, 1.25, color(s.menu_ink_off));
                    scene.line(ScenePoint::new(cx + 3.2, cy + 3.2), ScenePoint::new(cx + 7.0, cy + 7.0), 1.25, color(s.menu_ink_off));
                    if let Some(face) = face {
                        let size = 13.0;
                        let m = fonts.metrics(face, size);
                        let base = (f.y + (f.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round();
                        scene.push_clip(SceneRect::new(f.x + 26.0, f.y, f.w - 34.0, f.h), 0.0);
                        if typed.is_empty() {
                            scene.fill(SceneRect::new(f.x + 28.0, f.y + 7.0, 1.0, f.h - 14.0), color(s.menu_ink));
                            vergence_render::text(&mut scene, fonts, face, size, color(s.menu_ink_off), ScenePoint::new(f.x + 32.0, base), placeholder);
                        } else {
                            // The selection lit, the text, the caret where it is.
                            let x_of = |fonts: &mut vergence_text::Fonts, b: usize| fonts.layout(face, size, &typed[..b.min(typed.len())]).1;
                            let (a, b) = (caret.min(anchor).min(typed.len()), caret.max(anchor).min(typed.len()));
                            if a != b {
                                let (xa, xb) = (x_of(fonts, a), x_of(fonts, b));
                                scene.fill(SceneRect::new(f.x + 28.0 + xa, f.y + 5.0, xb - xa, f.h - 10.0), Color::rgba(accent.r, accent.g, accent.b, 110));
                            }
                            vergence_render::text(&mut scene, fonts, face, size, color(s.menu_ink), ScenePoint::new(f.x + 28.0, base), typed);
                            let xc = x_of(fonts, caret);
                            scene.fill(SceneRect::new(f.x + 28.0 + xc, f.y + 7.0, 1.0, f.h - 14.0), color(s.menu_ink));
                        }
                        scene.pop_clip();
                    }
                }
                Line::Note(text) => {
                    if let Some(face) = face {
                        let size = 12.0;
                        let m = fonts.metrics(face, size);
                        let room = r.w - 24.0;
                        let mut out: Vec<String> = Vec::new();
                        let mut cur = String::new();
                        for word in text.split_whitespace() {
                            let tried = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
                            if fonts.layout(face, size, &tried).1 <= room || cur.is_empty() {
                                cur = tried;
                            } else {
                                out.push(std::mem::take(&mut cur));
                                cur = word.to_string();
                            }
                        }
                        out.push(cur);
                        let line_h = (m.ascent + m.descent + m.line_gap).ceil();
                        for (k, l) in out.iter().take(2).enumerate() {
                            vergence_render::text(&mut scene, fonts, face, size, color(s.menu_ink_off), ScenePoint::new(r.x + 12.0, (r.y + 6.0 + m.ascent + k as f32 * line_h).round()), l);
                        }
                    }
                }
                Line::Zoom { now, .. } => {
                    if lit {
                        scene.fill_rounded(r, vergence_look::MENU_ROW_RADIUS, color(s.hover));
                    }
                    let parts = appmenu::zoom_parts(vergence_shell::Rect::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32));
                    for (k, p) in parts.iter().enumerate() {
                        let p = SceneRect::new(p.x as f32, p.y as f32, p.w as f32, p.h as f32);
                        if k != 1 {
                            scene.fill_rounded(p, vergence_look::MENU_ROW_RADIUS, color(s.field));
                            let (cx, cy) = (p.x + p.w / 2.0, p.y + p.h / 2.0);
                            scene.line(ScenePoint::new(cx - 4.5, cy), ScenePoint::new(cx + 4.5, cy), 1.5, color(s.menu_ink));
                            if k == 2 {
                                scene.line(ScenePoint::new(cx, cy - 4.5), ScenePoint::new(cx, cy + 4.5), 1.5, color(s.menu_ink));
                            }
                        }
                    }
                    if let Some(face) = face {
                        let size = 13.0;
                        let m = fonts.metrics(face, size);
                        let base = (r.y + (r.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round();
                        vergence_render::text(&mut scene, fonts, face, size, color(s.menu_ink), ScenePoint::new(r.x + 12.0 + gutter, base), "Text size");
                        let mid = parts[1];
                        let (_, nw) = fonts.layout(face, 12.0, now);
                        vergence_render::text(&mut scene, fonts, face, 12.0, color(s.menu_ink), ScenePoint::new(mid.x as f32 + (mid.w as f32 - nw) / 2.0, base), now);
                    }
                }
                Line::Header(name) => {
                    if let Some(face) = face {
                        let size = 11.5;
                        let m = fonts.metrics(face, size);
                        let base = (r.y + r.h - 6.0 - m.descent).round();
                        vergence_render::text(&mut scene, fonts, face, size, color(s.menu_ink_off), ScenePoint::new(r.x + 12.0, base), name);
                    }
                }
                Line::Item { label, keys, enabled, checked, pinned, .. } => {
                    if lit && *enabled {
                        scene.fill_rounded(r, vergence_look::MENU_ROW_RADIUS, color(s.hover));
                    }
                    let ink = if *enabled { s.menu_ink } else { s.menu_ink_off };
                    if let Some(face) = face {
                        let size = 13.0;
                        let m = fonts.metrics(face, size);
                        let base = (r.y + (r.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round();
                        if *checked == Some(true) {
                            let (cx, cy) = (r.x + 12.0, r.y + r.h / 2.0);
                            scene.line(ScenePoint::new(cx - 3.5, cy), ScenePoint::new(cx - 1.0, cy + 3.0), 1.5, color(ink));
                            scene.line(ScenePoint::new(cx - 1.0, cy + 3.0), ScenePoint::new(cx + 4.5, cy - 4.0), 1.5, color(ink));
                        }
                        let (_, kw) = if keys.is_empty() { (Vec::new(), 0.0) } else { fonts.layout(face, 12.0, keys) };
                        let left = r.x + 12.0 + gutter;
                        scene.push_clip(SceneRect::new(left, r.y, (r.w - 24.0 - gutter - kw - 8.0).max(0.0), r.h), 0.0);
                        let (_, lw) = fonts.layout(face, size, label);
                        vergence_render::text(&mut scene, fonts, face, size, color(ink), ScenePoint::new(left, base), label);
                        // While searching, each result says which menu it is in.
                        if results && let Some(menu) = menus.get(i.wrapping_sub(1)).filter(|m| !m.is_empty()) {
                            vergence_render::text(&mut scene, fonts, face, 11.5, color(s.menu_ink_off), ScenePoint::new(left + lw + 8.0, base), menu);
                        }
                        scene.pop_clip();
                        if !keys.is_empty() {
                            vergence_render::text(&mut scene, fonts, face, 12.0, color(s.menu_ink_off), ScenePoint::new(r.x + r.w - 12.0 - kw, base), keys);
                        }
                        // Added by the user: a small star at the line's end (before its keys).
                        if *pinned {
                            let cx = r.x + r.w - 12.0 - kw - if keys.is_empty() { 4.0 } else { 14.0 };
                            star(&mut scene, cx, r.y + r.h / 2.0, 4.5, color(vergence_look::ACCENT));
                        }
                    }
                }
                Line::Sub(name) => {
                    if lit {
                        scene.fill_rounded(r, vergence_look::MENU_ROW_RADIUS, color(s.hover));
                    }
                    if let Some(face) = face {
                        let size = 13.0;
                        let m = fonts.metrics(face, size);
                        let base = (r.y + (r.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round();
                        vergence_render::text(&mut scene, fonts, face, size, color(s.menu_ink), ScenePoint::new(r.x + 12.0 + gutter, base), name);
                    }
                    let (cx, cy) = (r.x + r.w - 14.0, r.y + r.h / 2.0);
                    scene.line(ScenePoint::new(cx - 2.0, cy - 4.0), ScenePoint::new(cx + 2.0, cy), 1.25, color(s.menu_ink));
                    scene.line(ScenePoint::new(cx + 2.0, cy), ScenePoint::new(cx - 2.0, cy + 4.0), 1.25, color(s.menu_ink));
                }
            }
        }
        match vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) {
            Ok(p) => Some(MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None)),
            Err(e) => {
                tracing::warn!("app menu: could not be drawn: {e}");
                None
            }
        }
    }
}

/// A five-pointed star's outline, `r` from its middle to a point.
fn star(scene: &mut Scene, cx: f32, cy: f32, r: f32, ink: Color) {
    let at = |k: usize| {
        let a = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
        let d = if k % 2 == 0 { r } else { r * 0.45 };
        ScenePoint::new(cx + d * a.cos(), cy + d * a.sin())
    };
    for k in 0..10 {
        scene.line(at(k), at(k + 1), 1.2, ink);
    }
}

/// Where the commands users added to apps' first panels are kept.
fn pins_path() -> Option<std::path::PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))?;
    Some(state.join("vergence/app-menu.toml"))
}

/// The commands the user added to an app's first panel.
fn pins_of(app: &str) -> Vec<String> {
    let table = pins_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| t.parse::<toml::Table>().ok()).unwrap_or_default();
    table.get(app).and_then(|a| a.get("pins")).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str()).take(64).map(String::from).collect()).unwrap_or_default()
}

/// Keeps an app's pins (the file's other apps as they were).
fn keep_pins(app: &str, pins: &[String]) {
    use std::{io::Write, os::unix::fs::{DirBuilderExt, OpenOptionsExt}};
    let Some(path) = pins_path() else { return };
    if app.is_empty() {
        return;
    }
    let mut table = std::fs::read_to_string(&path).ok().and_then(|t| t.parse::<toml::Table>().ok()).unwrap_or_default();
    if pins.is_empty() {
        table.remove(app);
    } else {
        let mut t = toml::Table::new();
        t.insert("pins".into(), toml::Value::Array(pins.iter().map(|p| toml::Value::String(p.clone())).collect()));
        table.insert(app.to_string(), toml::Value::Table(t));
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir);
    }
    let text = toml::to_string(&table).unwrap_or_default();
    let beside = path.with_extension("toml.new");
    let written = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&beside).and_then(|mut f| f.write_all(text.as_bytes())).and_then(|_| std::fs::rename(&beside, &path));
    if let Err(e) = written {
        tracing::warn!("app menu: can't keep {app}'s commands: {e}");
    }
}
