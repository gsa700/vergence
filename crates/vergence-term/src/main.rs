// Vergence's terminal.
//
// Step 1 (2026-11-08): our own terminal core (vt.rs parser, grid.rs screen,
// pty.rs) drawn at the screen's own pixels in the desktop's monospace font.
// Step 2: tabs in the title bar (Dave: "I prefer the tabs in the titlebar ala
// Firefox"): the window draws its own bar in the server's colours, with the
// tabs, a + button and the window buttons; the bar moves the window, a
// double-click maximises, the edges resize.
// Step 3: split panes (iced's pane grid; Dave liked COSMIC Terminal's "ability
// to split"), selection with copy and paste, and the right-click menu.
// Step 4: mouse reporting (xterm modes 9/1000/1002/1003, SGR 1006): when a
// program asks for the mouse (htop, vim, mc), clicks, drags and the wheel go
// to it; Shift keeps selecting and the right-click menu. The wheel on the
// alternate screen sends arrow keys when the program didn't ask (less, man).
// Step 5: scrollback viewing: the wheel on the shell's screen, Shift+PageUp/
// PageDown/Home/End; typing returns to the live screen; "N lines back" shows.
// A selection dragged past the top or bottom edge scrolls the view; the
// scrollbar (in the right padding) shows where the view is and can be dragged.
// Step 6: colour schemes from the desktop's settings (look.rs): dark or light
// (Catppuccin Mocha / Latte), high contrast, the accent for the cursor,
// selection and highlights, and the font size, all live within a second.
// Next: the font family, a scheme picker of its own if wanted.
//
// Drawing is in software (iced's tiny-skia): the GPU renderer hung Hubble's
// GPU and froze the desktop (2026-11-08).

#[cfg(test)]
mod fuzz;
mod grid;
mod look;
mod pty;
mod term;
mod vt;

use std::collections::HashMap;
use std::time::Duration;

use iced::time::Instant;
use std::sync::{Mutex, OnceLock};

use iced::keyboard::{self, Key, key::Named};
use iced::widget::canvas::{self, Canvas, Geometry, Program};
use iced::widget::{Space, button, column, container, mouse_area, pane_grid, row, stack, text};
use iced::window::Direction;
use iced::{Background, Border, Color, Element, Event, Length, Padding, Point, Rectangle, Renderer, Size, Subscription, Task, Theme, mouse};

use grid::MouseMode;
use term::{CellPos, MouseAction, MouseButton, PAD, Term};

/// The title bar's height, in points (the server's bar is 24; tabs want more).
const BAR_H: f32 = 34.0;
/// The tabs' height: they sit on the bar's bottom edge, joined to the
/// terminal; everything else in the bar is centred on the tabs' middle.
const TAB_H: f32 = 28.0;
/// A tab's width, in points; longer titles end in an ellipsis.
const TAB_W: f32 = 200.0;
/// The window buttons' and the + button's icons: one box size, so they line up.
const ICON: f32 = 10.0;
/// How far the tab titles are lifted from where iced centres them: it centres
/// the line box, and Noto Sans' capitals then sit about a point low (measured
/// from a screenshot at scale 2.25, 2026-11-08). The icons stay exactly centred.
const TEXT_LIFT: f32 = 1.0;
/// The invisible border that resizes the window, in points.
const EDGE: f32 = 5.0;
/// The gap between panes (the divider), in points.
const DIVIDER: f32 = 2.0;

// The server's title bar colours (vergence-server decor.rs), so our bar
// matches the windows it decorates.
const BAR_ACTIVE: Color = Color::from_rgb(0.20, 0.24, 0.32);
const BAR_INACTIVE: Color = Color::from_rgb(0.22, 0.22, 0.22);
const TEXT_ACTIVE: Color = Color::from_rgb(0.92, 0.92, 0.92);
const TEXT_INACTIVE: Color = Color::from_rgb(0.62, 0.62, 0.62);
const CLOSE: Color = Color::from_rgb(0.78, 0.30, 0.28);
/// The terminal's background: the active tab is drawn in it, joined to the text below.
/// The right-click menu's width, in points.
const MENU_W: f32 = 236.0;
const MENU_BG: Color = Color::from_rgb(0.17, 0.18, 0.23);
/// The menu with the light scheme (secondary surfaces follow the terminal's scheme).
const MENU_BG_LIGHT: Color = Color::from_rgb(0.97, 0.97, 0.98);
const MENU_TEXT_LIGHT: Color = Color::from_rgb(0.12, 0.12, 0.18);
const MENU_DIM_LIGHT: Color = Color::from_rgb(0.50, 0.51, 0.58);

#[derive(Debug, Clone)]
enum Message {
    Output(u64, Vec<u8>),
    Exited(u64),
    Key(Key, keyboard::Modifiers, Option<String>),
    Modifiers(keyboard::Modifiers),
    /// The desktop's look changed (desktop.toml, settings.toml).
    Look(look::Look),
    /// Bytes for a terminal's program from the mouse (reporting, or the wheel as arrows).
    Mouse(u64, Vec<u8>),
    /// Scroll a terminal's view by lines (positive: back into the scrollback).
    Scroll(u64, isize),
    /// The scrollbar: the view goes this many lines back.
    ScrollTo(u64, usize),
    /// A selection dragged past the top (positive lines) or bottom edge: the
    /// view scrolls and the selection's end follows, at this column.
    DragScroll(u64, isize, usize),
    Resized(Size),
    Focused(bool),
    NewTab,
    SelectTab(usize),
    CloseTab(usize),
    FocusPane(pane_grid::Pane),
    PaneResized(pane_grid::ResizeEvent),
    Split(pane_grid::Axis),
    ClosePane,
    Select(u64, Option<(CellPos, CellPos)>),
    /// The right-click menu for a terminal, at the click (window points).
    Menu(u64, Point),
    CloseMenu,
    Copy,
    Paste,
    Pasted(Option<String>),
    Drag,
    ToggleMaximize,
    Minimize,
    CloseWindow,
    ResizeEdge(Direction),
}

/// Each terminal's shell output, handed from its creation to its subscription
/// (iced builds streams from plain functions).
static READERS: OnceLock<Mutex<HashMap<u64, std::fs::File>>> = OnceLock::new();

fn readers() -> std::sync::MutexGuard<'static, HashMap<u64, std::fs::File>> {
    READERS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap()
}

/// A tab: its panes (each a terminal, by id) and the one with the keyboard.
struct Tab {
    panes: pane_grid::State<u64>,
    focus: pane_grid::Pane,
}

struct App {
    terms: HashMap<u64, Term>,
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    /// The window's size (points) and whether it has the keyboard.
    size: Size,
    focused: bool,
    /// The right-click menu: where (window points) and for which terminal.
    menu: Option<(Point, u64)>,
    /// The modifier keys held now (Shift-click selects even when a program has the mouse).
    mods: keyboard::Modifiers,
    /// The desktop's look: colours, accent, font size.
    look: look::Look,
}

impl App {
    fn new() -> App {
        let mut app = App { terms: HashMap::new(), tabs: Vec::new(), active: 0, next_id: 1, size: Size::ZERO, focused: true, menu: None, mods: keyboard::Modifiers::empty(), look: look::Look::load() };
        app.size = initial_size(&app.look);
        if let Some(id) = app.spawn(app.body()) {
            let (panes, focus) = pane_grid::State::new(id);
            app.tabs.push(Tab { panes, focus });
        }
        app
    }

    /// The terminal area: the window minus the title bar.
    fn body(&self) -> Size {
        Size::new(self.size.width, (self.size.height - BAR_H).max(1.0))
    }

    /// A new terminal sized for an area; its id.
    fn spawn(&mut self, area: Size) -> Option<u64> {
        let id = self.next_id;
        self.next_id += 1;
        let cell = term::cell_for(&self.look);
        let cols = ((area.width - 2.0 * PAD) / cell.width).floor().max(2.0) as usize;
        let rows = ((area.height - 2.0 * PAD) / cell.height).floor().max(1.0) as usize;
        match Term::new(id, cols, rows, &self.look) {
            Ok((t, reader)) => {
                readers().insert(id, reader);
                self.terms.insert(id, t);
                Some(id)
            }
            Err(e) => {
                eprintln!("vergence-term: cannot start a shell: {e}");
                None
            }
        }
    }

    fn open_tab(&mut self) {
        if let Some(id) = self.spawn(self.body()) {
            let (panes, focus) = pane_grid::State::new(id);
            self.tabs.push(Tab { panes, focus });
            self.active = self.tabs.len() - 1;
        }
    }

    /// The terminal with the keyboard.
    fn focused_term(&self) -> Option<u64> {
        let tab = self.tabs.get(self.active)?;
        tab.panes.get(tab.focus).copied()
    }

    /// Fit every pane's terminal of a tab to its region.
    fn fit_tab(&mut self, t: usize) {
        let Some(tab) = self.tabs.get(t) else { return };
        let body = self.body();
        let regions = tab.panes.layout().pane_regions(DIVIDER, 0.0, body);
        for (pane, r) in regions {
            if let Some(id) = tab.panes.get(pane)
                && let Some(term) = self.terms.get_mut(id)
            {
                term.fit(r.size());
            }
        }
    }

    fn fit_all(&mut self) {
        for t in 0..self.tabs.len() {
            self.fit_tab(t);
        }
    }

    /// Redraw a tab's panes (the focus changed).
    fn repaint_tab(&mut self) {
        if let Some(tab) = self.tabs.get(self.active) {
            for (_, id) in tab.panes.iter() {
                if let Some(t) = self.terms.get(id) {
                    t.cache.clear();
                }
            }
        }
    }

    /// A terminal ended (its shell exited, or its pane or tab was closed): its
    /// pane closes; the tab with its last pane; the window with its last tab.
    fn remove_term(&mut self, id: u64) -> Task<Message> {
        self.terms.remove(&id);
        let Some(t) = self.tabs.iter().position(|tab| tab.panes.iter().any(|(_, v)| *v == id)) else { return Task::none() };
        let tab = &mut self.tabs[t];
        let pane = tab.panes.iter().find(|(_, v)| **v == id).map(|(p, _)| *p);
        if tab.panes.len() > 1 {
            if let Some(p) = pane
                && let Some((_, sibling)) = tab.panes.close(p)
                && tab.focus == p
            {
                tab.focus = sibling;
            }
            self.fit_tab(t);
            self.repaint_tab();
            return Task::none();
        }
        self.tabs.remove(t);
        if self.tabs.is_empty() {
            return iced::exit();
        }
        if self.active >= t && self.active > 0 {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len() - 1);
        Task::none()
    }

    fn close_tab(&mut self, t: usize) -> Task<Message> {
        let Some(tab) = self.tabs.get(t) else { return Task::none() };
        let ids: Vec<u64> = tab.panes.iter().map(|(_, v)| *v).collect();
        let mut task = Task::none();
        for id in ids {
            task = self.remove_term(id);
        }
        task
    }

    /// Give a terminal's pane the keyboard (a click in it).
    fn focus_term(&mut self, id: u64) {
        if let Some(tab) = self.tabs.get_mut(self.active)
            && let Some(pane) = tab.panes.iter().find(|(_, v)| **v == id).map(|(p, _)| *p)
            && tab.focus != pane
        {
            tab.focus = pane;
        } else {
            return;
        }
        self.repaint_tab();
    }

    fn split(&mut self, axis: pane_grid::Axis) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let focus = tab.focus;
        // The new pane starts at half the focused one; fitted right after.
        let half = Size::new(self.body().width / 2.0, self.body().height);
        let Some(id) = self.spawn(half) else { return };
        let tab = &mut self.tabs[self.active];
        if let Some((pane, _)) = tab.panes.split(axis, focus, id) {
            tab.focus = pane;
        }
        let a = self.active;
        self.fit_tab(a);
        self.repaint_tab();
    }

    fn update(&mut self, m: Message) -> Task<Message> {
        match m {
            Message::Output(id, bytes) => {
                if let Some(t) = self.terms.get_mut(&id)
                    && let Some(copied) = t.feed(&bytes)
                {
                    return iced::clipboard::write(copied);
                }
            }
            Message::Exited(id) => return self.remove_term(id),
            Message::Key(key, mods, txt) => {
                if self.menu.is_some() && key == Key::Named(Named::Escape) {
                    self.menu = None;
                    return Task::none();
                }
                // The terminal's own shortcuts: Ctrl+Shift, so the shell keeps Ctrl+T etc.
                let cs = mods.control() && mods.shift();
                let is = |c: &str| matches!(&key, Key::Character(k) if k.eq_ignore_ascii_case(c));
                if cs && is("t") {
                    self.open_tab();
                    return Task::none();
                }
                if cs && is("w") {
                    return self.update(Message::ClosePane);
                }
                if cs && is("r") {
                    self.split(pane_grid::Axis::Vertical);
                    return Task::none();
                }
                if cs && is("d") {
                    self.split(pane_grid::Axis::Horizontal);
                    return Task::none();
                }
                if cs && is("c") {
                    return self.update(Message::Copy);
                }
                if cs && is("v") {
                    return self.update(Message::Paste);
                }
                let n = self.tabs.len().max(1);
                match &key {
                    Key::Named(Named::PageUp) if mods.control() && !mods.shift() => {
                        self.active = (self.active + n - 1) % n;
                        return Task::none();
                    }
                    Key::Named(Named::PageDown) if mods.control() && !mods.shift() => {
                        self.active = (self.active + 1) % n;
                        return Task::none();
                    }
                    // The scrollback: Shift+PageUp/PageDown a page, Shift+Home/End the ends
                    // (on the alternate screen the keys go to the program).
                    Key::Named(n @ (Named::PageUp | Named::PageDown | Named::Home | Named::End))
                        if mods.shift() && !mods.control() && !mods.alt()
                            && let Some(t) = self.focused_term().and_then(|id| self.terms.get_mut(&id))
                            && !t.grid.alt =>
                    {
                        let lines = match n {
                            Named::PageUp => t.page(),
                            Named::PageDown => -t.page(),
                            Named::Home => isize::MAX,
                            _ => isize::MIN,
                        };
                        t.scroll_by(lines);
                        return Task::none();
                    }
                    Key::Named(Named::Tab) if mods.control() => {
                        self.active = if mods.shift() { (self.active + n - 1) % n } else { (self.active + 1) % n };
                        return Task::none();
                    }
                    _ => {}
                }
                if let Some(id) = self.focused_term()
                    && let Some(t) = self.terms.get_mut(&id)
                    && let Some(bytes) = term::key_bytes(&key, mods, txt.as_deref(), t.grid.app_cursor)
                {
                    // Typing clears the selection and goes back to the live screen, as in other terminals.
                    t.select(None);
                    t.scroll_to_bottom();
                    t.pty.write(&bytes);
                }
            }
            Message::Resized(size) => {
                self.size = size;
                self.fit_all();
            }
            Message::Focused(f) => {
                self.focused = f;
                self.repaint_tab();
            }
            Message::NewTab => {
                self.menu = None;
                self.open_tab();
            }
            Message::SelectTab(i) => self.active = i.min(self.tabs.len().saturating_sub(1)),
            Message::CloseTab(i) => return self.close_tab(i),
            Message::FocusPane(p) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.focus = p;
                }
                self.repaint_tab();
            }
            Message::PaneResized(e) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.panes.resize(e.split, e.ratio);
                }
                let a = self.active;
                self.fit_tab(a);
            }
            Message::Split(axis) => {
                self.menu = None;
                self.split(axis);
            }
            Message::ClosePane => {
                let target = self.menu.map(|(_, id)| id).or_else(|| self.focused_term());
                self.menu = None;
                if let Some(id) = target {
                    return self.remove_term(id);
                }
            }
            Message::Select(id, sel) => {
                // A click in a pane gives it the keyboard as well.
                self.focus_term(id);
                if let Some(t) = self.terms.get_mut(&id) {
                    t.select(sel);
                }
            }
            Message::Modifiers(m) => self.mods = m,
            Message::Look(l) => {
                for t in self.terms.values_mut() {
                    t.set_look(&l);
                }
                self.look = l;
                // A new font size changes how many cells fit.
                self.fit_all();
            }
            Message::Scroll(id, lines) => {
                if let Some(t) = self.terms.get_mut(&id) {
                    t.scroll_by(lines);
                }
            }
            Message::ScrollTo(id, lines) => {
                if let Some(t) = self.terms.get_mut(&id) {
                    t.scroll_to(lines);
                }
            }
            Message::DragScroll(id, lines, col) => {
                if let Some(t) = self.terms.get_mut(&id) {
                    t.scroll_by(lines);
                    t.drag_to_edge(col, lines > 0);
                }
            }
            Message::Mouse(id, bytes) => {
                self.focus_term(id);
                if let Some(t) = self.terms.get_mut(&id) {
                    t.select(None);
                    t.pty.write(&bytes);
                }
            }
            Message::Menu(id, at) => self.menu = Some((at, id)),
            Message::CloseMenu => self.menu = None,
            Message::Copy => {
                let target = self.menu.map(|(_, id)| id).or_else(|| self.focused_term());
                self.menu = None;
                if let Some(text) = target.and_then(|id| self.terms.get(&id)).and_then(Term::selected_text) {
                    return iced::clipboard::write(text);
                }
            }
            Message::Paste => {
                self.menu = None;
                return iced::clipboard::read().map(Message::Pasted);
            }
            Message::Pasted(text) => {
                if let Some(text) = text
                    && let Some(id) = self.focused_term()
                    && let Some(t) = self.terms.get_mut(&id)
                {
                    t.paste(&text);
                }
            }
            Message::Drag => return iced::window::latest().and_then(iced::window::drag),
            Message::ToggleMaximize => return iced::window::latest().and_then(iced::window::toggle_maximize),
            Message::Minimize => return iced::window::latest().and_then(|id| iced::window::minimize(id, true)),
            Message::CloseWindow => return iced::exit(),
            Message::ResizeEdge(d) => return iced::window::latest().and_then(move |id| iced::window::drag_resize(id, d)),
        }
        Task::none()
    }

    fn title(&self) -> String {
        self.focused_term().and_then(|id| self.terms.get(&id)).map(Term::title).unwrap_or_else(|| "Terminal".into())
    }

    fn view(&self) -> Element<'_, Message> {
        let body: Element<'_, Message> = match self.tabs.get(self.active) {
            Some(tab) => {
                let focus = tab.focus;
                let accent = self.look.palette().accent;
                pane_grid::PaneGrid::new(&tab.panes, move |pane, id, _| {
                    let content: Element<'_, Message> = match self.terms.get(id) {
                        Some(t) => Canvas::new(View { term: t, focused: self.focused && pane == focus, mods: self.mods }).width(Length::Fill).height(Length::Fill).into(),
                        None => Space::new().into(),
                    };
                    pane_grid::Content::new(content)
                })
                .spacing(DIVIDER)
                .on_click(Message::FocusPane)
                .on_resize(8, Message::PaneResized)
                .style(move |_: &Theme| pane_grid::Style {
                    hovered_region: pane_grid::Highlight { background: Background::Color(Color { a: 0.2, ..accent }), border: Border::default() },
                    picked_split: pane_grid::Line { color: accent, width: 2.0 },
                    hovered_split: pane_grid::Line { color: accent, width: 2.0 },
                })
                .into()
            }
            None => Space::new().into(),
        };
        let window = container(column![self.title_bar(), body]).style(|_| container::background(BAR_INACTIVE));
        // The edges and corners resize the window (it draws its own frame).
        let edge = |w: Length, h: Length, d: Direction, cursor: mouse::Interaction| {
            mouse_area(Space::new().width(w).height(h)).on_press(Message::ResizeEdge(d)).interaction(cursor)
        };
        let e = Length::Fixed(EDGE);
        let edges = column![
            row![edge(e, e, Direction::NorthWest, mouse::Interaction::ResizingDiagonallyDown), edge(Length::Fill, e, Direction::North, mouse::Interaction::ResizingVertically), edge(e, e, Direction::NorthEast, mouse::Interaction::ResizingDiagonallyUp)],
            row![edge(e, Length::Fill, Direction::West, mouse::Interaction::ResizingHorizontally), Space::new().width(Length::Fill).height(Length::Fill), edge(e, Length::Fill, Direction::East, mouse::Interaction::ResizingHorizontally)],
            row![edge(e, e, Direction::SouthWest, mouse::Interaction::ResizingDiagonallyUp), edge(Length::Fill, e, Direction::South, mouse::Interaction::ResizingVertically), edge(e, e, Direction::SouthEast, mouse::Interaction::ResizingDiagonallyDown)],
        ];
        let mut layers = stack![window, edges];
        if let Some((at, id)) = self.menu {
            layers = layers.push(self.context_menu(at, id));
        }
        layers.into()
    }

    /// The right-click menu at `at`: a click outside it closes it.
    fn context_menu(&self, at: Point, id: u64) -> Element<'_, Message> {
        let has_selection = self.terms.get(&id).and_then(Term::selected_text).is_some();
        let dark = self.look.dark;
        let accent = self.look.palette().accent;
        let (ui, ui_size) = (self.look.ui_font(), self.look.ui_size);
        let (menu_bg, text_on, text_off, edge) =
            if dark { (MENU_BG, TEXT_ACTIVE, TEXT_INACTIVE, Color { a: 0.15, ..Color::WHITE }) } else { (MENU_BG_LIGHT, MENU_TEXT_LIGHT, MENU_DIM_LIGHT, Color { a: 0.18, ..Color::BLACK }) };
        let item = |label: &'static str, keys: &'static str, m: Option<Message>| {
            let enabled = m.is_some();
            let c = if enabled { text_on } else { text_off };
            // Hovered: the accent behind white text (readable on either scheme).
            let mut b = button(row![text(label).font(ui).size(ui_size).width(Length::Fill), text(keys).font(ui).size(ui_size - 2.0)].spacing(12).align_y(iced::alignment::Vertical::Center))
                .width(Length::Fill)
                .padding(Padding::from([5, 10]))
                .style(move |_: &Theme, s| {
                    let hovered = enabled && s == button::Status::Hovered;
                    let mut st = rounded(plain(s, hovered.then_some(Color { a: 0.9, ..accent })));
                    st.text_color = if hovered { Color::WHITE } else { c };
                    st
                });
            if let Some(m) = m {
                b = b.on_press(m);
            }
            b
        };
        // A hairline with room around it (the padding outside the coloured line).
        let hairline = if dark { Color { a: 0.12, ..Color::WHITE } } else { Color { a: 0.12, ..Color::BLACK } };
        let line = || container(container(Space::new().height(1.0).width(Length::Fill)).style(move |_| container::background(hairline))).padding(Padding::from([4, 8]));
        // As wide as its longest line ("Split top and bottom" and its keys), no
        // wider: the separators fill the column, so the column sets the width.
        let menu = container(column![
            item("Copy", "Ctrl+Shift+C", has_selection.then_some(Message::Copy)),
            item("Paste", "Ctrl+Shift+V", Some(Message::Paste)),
            line(),
            item("Split side by side", "Ctrl+Shift+R", Some(Message::Split(pane_grid::Axis::Vertical))),
            item("Split top and bottom", "Ctrl+Shift+D", Some(Message::Split(pane_grid::Axis::Horizontal))),
            line(),
            item("New tab", "Ctrl+Shift+T", Some(Message::NewTab)),
            item("Close pane", "Ctrl+Shift+W", Some(Message::ClosePane)),
        ]
        .width(Length::Fixed(MENU_W)))
        .padding(4)
        .style(move |_| container::Style {
            background: Some(Background::Color(menu_bg)),
            border: Border { color: edge, width: 1.0, radius: 8.0.into() },
            ..container::Style::default()
        });
        // Kept inside the window.
        let (mw, mh) = (MENU_W + 10.0, 250.0);
        let x = at.x.min(self.size.width - mw).max(0.0);
        let y = at.y.min(self.size.height - mh).max(0.0);
        let placed = container(menu).padding(Padding { top: y, left: x, right: 0.0, bottom: 0.0 });
        stack![mouse_area(Space::new().width(Length::Fill).height(Length::Fill)).on_press(Message::CloseMenu).on_right_press(Message::CloseMenu), placed].into()
    }

    /// The bar: tabs (the active one in the terminal's colours, joined to it),
    /// +, room to grab, and the window buttons where the desktop's layout
    /// (Gtk/DecorationLayout) puts them. Tabs sit on the bar's bottom edge;
    /// every other item is centred on the tabs' middle line, and the icons
    /// share one box size, so the whole bar lines up.
    fn title_bar(&self) -> Element<'_, Message> {
        let fg = if self.focused { TEXT_ACTIVE } else { TEXT_INACTIVE };
        let bar_bg = if self.focused { BAR_ACTIVE } else { BAR_INACTIVE };
        let pal = self.look.palette();
        let (ui, ui_size) = (self.look.ui_font(), self.look.ui_size);
        let center = iced::alignment::Vertical::Center;
        let mut tabs = row![].spacing(2).height(Length::Fixed(TAB_H)).align_y(center);
        for (i, tab) in self.tabs.iter().enumerate() {
            let active = i == self.active;
            let title = tab.panes.get(tab.focus).and_then(|id| self.terms.get(id)).map(Term::title).unwrap_or_default();
            let count = tab.panes.len();
            let label = if count > 1 { format!("{title}  ({count})") } else { title.clone() };
            let tab_fg = if active { pal.fg } else { fg };
            let close = button(icon(Glyph::Close, tab_fg, 8.0))
                .width(Length::Fixed(20.0))
                .height(Length::Fixed(20.0))
                .padding(0)
                .on_press(Message::CloseTab(i))
                .style(move |_: &Theme, s| rounded(plain(s, if s == button::Status::Hovered { Some(Color { a: 0.18, ..tab_fg }) } else { None })));
            // The pane count stays visible: only the title is shortened.
            let room = TAB_W - 12.0 - 4.0 - 6.0 - 20.0;
            let label = if count > 1 {
                let suffix = format!("  ({count})");
                format!("{}{suffix}", fit_label(&title, room - text_width(&suffix, ui_size), ui_size))
            } else {
                fit_label(&label, room, ui_size)
            };
            let name = text(label).font(ui).size(ui_size).color(tab_fg).width(Length::Fill).height(Length::Fill).align_y(center).wrapping(text::Wrapping::None);
            let name = container(name).width(Length::Fill).height(Length::Fill).clip(true).padding(Padding { bottom: 2.0 * TEXT_LIFT, ..Padding::ZERO });
            let content = row![name, close].height(Length::Fill).align_y(center).spacing(6);
            let tab_button = button(content)
                .width(Length::Fixed(TAB_W))
                .height(Length::Fixed(TAB_H))
                .padding(Padding { top: 0.0, bottom: 0.0, left: 12.0, right: 4.0 })
                .on_press(Message::SelectTab(i))
                .style(move |_: &Theme, s| {
                    let bg = if active {
                        Some(pal.bg)
                    } else if s == button::Status::Hovered {
                        Some(Color { a: 0.10, ..Color::WHITE })
                    } else {
                        None
                    };
                    let mut st = plain(s, bg);
                    st.border = Border { radius: iced::border::Radius { top_left: 7.0, top_right: 7.0, bottom_left: 0.0, bottom_right: 0.0 }, ..Border::default() };
                    st
                });
            tabs = tabs.push(tab_button);
        }
        let new_tab = button(icon(Glyph::Plus, fg, ICON))
            .width(Length::Fixed(28.0))
            .height(Length::Fixed(24.0))
            .padding(0)
            .on_press(Message::NewTab)
            .style(move |_: &Theme, s| rounded(plain(s, if s == button::Status::Hovered { Some(Color { a: 0.12, ..Color::WHITE }) } else { None })));
        let grab = mouse_area(Space::new().width(Length::Fill).height(Length::Fill)).on_press(Message::Drag).on_double_click(Message::ToggleMaximize);
        let win_button = |b: look::WinButton| -> Element<'_, Message> {
            let (glyph, m, danger) = match b {
                look::WinButton::Minimize => (Glyph::Minimize, Message::Minimize, false),
                look::WinButton::Maximize => (Glyph::Maximize, Message::ToggleMaximize, false),
                look::WinButton::Close => (Glyph::Close, Message::CloseWindow, true),
            };
            button(icon(glyph, fg, ICON))
                .width(Length::Fixed(32.0))
                .height(Length::Fixed(24.0))
                .padding(0)
                .on_press(m)
                .style(move |_: &Theme, s| {
                    let hover = if danger { CLOSE } else { Color { a: 0.15, ..Color::WHITE } };
                    rounded(plain(s, if s == button::Status::Hovered { Some(hover) } else { None }))
                })
                .into()
        };
        let side = |list: &[look::WinButton]| row(list.iter().map(|b| win_button(*b))).spacing(2).align_y(center);
        let bar = row![side(&self.look.buttons_left), tabs, new_tab, grab, side(&self.look.buttons_right)].spacing(4).height(Length::Fixed(TAB_H)).align_y(center);
        container(bar)
            .padding(Padding { top: BAR_H - TAB_H, bottom: 0.0, left: 6.0, right: 6.0 })
            .height(Length::Fixed(BAR_H))
            .width(Length::Fill)
            .style(move |_| container::background(bar_bg))
            .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs: Vec<Subscription<Message>> = self.terms.keys().map(|id| Subscription::run_with(*id, output)).collect();
        subs.push(iced::event::listen_with(event));
        subs.push(Subscription::run(watch_look));
        Subscription::batch(subs)
    }
}

/// A pane on screen: the terminal, whether it has the keyboard, and the
/// modifier keys held. Mouse: left drag selects (a click clears), right opens
/// the menu, unless the program asked for the mouse (then it gets the events;
/// Shift still selects and opens the menu).
struct View<'a> {
    term: &'a Term,
    focused: bool,
    mods: keyboard::Modifiers,
}

/// A pane's mouse state between events.
#[derive(Default)]
struct Pointer {
    /// The selection's anchor while the left button is held.
    anchor: Option<CellPos>,
    /// The button whose press went to the program, until its release.
    held: Option<MouseButton>,
    /// The cell of the last report (motion is reported per cell).
    last: Option<CellPos>,
    /// Wheel movement not yet a whole step (touchpads scroll in pixels).
    wheel: f32,
    /// The scrollbar's thumb is being dragged: where in it it was grabbed.
    thumb: Option<f32>,
    /// A selection is being dragged past the pane's edge: when the view last scrolled.
    edge_tick: Option<Instant>,
}

/// While a selection is held past the pane's edge, the view scrolls this often.
const EDGE_TICK: Duration = Duration::from_millis(50);

fn button_of(b: mouse::Button) -> Option<MouseButton> {
    match b {
        mouse::Button::Left => Some(MouseButton::Left),
        mouse::Button::Middle => Some(MouseButton::Middle),
        mouse::Button::Right => Some(MouseButton::Right),
        _ => None,
    }
}

impl View<'_> {
    /// The program has the mouse (Shift takes it back for selecting).
    fn reporting(&self) -> bool {
        self.term.grid.mouse != MouseMode::Off && !self.mods.shift()
    }

    fn report(&self, action: MouseAction, cell: CellPos) -> Option<Vec<u8>> {
        let g = &self.term.grid;
        term::mouse_bytes(g.mouse, g.mouse_sgr, action, cell, self.mods)
    }

    /// The point under the pointer in the pane; past the pane's edge, the nearest one.
    fn point_clamped(bounds: Rectangle, cursor: mouse::Cursor) -> Point {
        cursor.position_in(bounds).unwrap_or_else(|| {
            let c = cursor.position().unwrap_or(bounds.position());
            Point::new((c.x - bounds.x).clamp(0.0, bounds.width), (c.y - bounds.y).clamp(0.0, bounds.height))
        })
    }

    /// The screen cell under the pointer (mouse reporting).
    fn cell_clamped(&self, bounds: Rectangle, cursor: mouse::Cursor) -> CellPos {
        let p = Self::point_clamped(bounds, cursor);
        self.term.cell_at(p.x, p.y)
    }

    /// The cell under the pointer as the selection counts it (absolute rows).
    fn text_cell_clamped(&self, bounds: Rectangle, cursor: mouse::Cursor) -> CellPos {
        let p = Self::point_clamped(bounds, cursor);
        self.term.text_cell_at(p.x, p.y)
    }
}

impl Program<Message> for View<'_> {
    type State = Pointer;

    fn update(&self, state: &mut Self::State, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<Message>> {
        let at = cursor.position_in(bounds);
        let id = self.term.id;
        let send = |bytes: Option<Vec<u8>>| Some(match bytes {
            Some(b) => canvas::Action::publish(Message::Mouse(id, b)).and_capture(),
            None => canvas::Action::capture(),
        });
        match event {
            // The scrollbar: a press on the thumb grabs it; elsewhere on the track the thumb jumps there.
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if let Some(p) = at
                    && p.x >= bounds.width - term::SCROLLBAR_GRAB
                    && let Some((_, thumb)) = self.term.scrollbar(bounds.size()) =>
            {
                let grab = if (thumb.y..thumb.y + thumb.height).contains(&p.y) { p.y - thumb.y } else { thumb.height / 2.0 };
                state.thumb = Some(grab);
                let to = self.term.scroll_for_thumb(bounds.size(), p.y - grab)?;
                Some(canvas::Action::publish(Message::ScrollTo(id, to)).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if let Some(grab) = state.thumb => {
                let y = cursor.position().map(|c| c.y - bounds.y)?;
                let to = self.term.scroll_for_thumb(bounds.size(), y - grab)?;
                (to != self.term.scroll).then(|| canvas::Action::publish(Message::ScrollTo(id, to)).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.thumb.is_some() => {
                state.thumb = None;
                Some(canvas::Action::capture())
            }
            // A selection held past the top or bottom edge: scroll every EDGE_TICK,
            // faster the further out the pointer is.
            Event::Window(iced::window::Event::RedrawRequested(now)) if state.anchor.is_some() && state.edge_tick.is_some() => {
                let c = cursor.position()?;
                let (above, below) = (bounds.y - c.y, c.y - (bounds.y + bounds.height));
                let speed = |d: f32| 1 + ((d / self.term.cell.height) as isize).min(9);
                let lines = if above > 0.0 {
                    speed(above)
                } else if below > 0.0 {
                    -speed(below)
                } else {
                    state.edge_tick = None;
                    return None;
                };
                let last = state.edge_tick?;
                if now.duration_since(last) < EDGE_TICK {
                    return Some(canvas::Action::request_redraw_at(last + EDGE_TICK));
                }
                state.edge_tick = Some(*now);
                let col = self.term.cell_at((c.x - bounds.x).clamp(0.0, bounds.width), PAD).0;
                // (Publishing redraws, so the next tick comes on the next RedrawRequested.)
                Some(canvas::Action::publish(Message::DragScroll(id, lines, col)))
            }
            Event::Mouse(mouse::Event::ButtonPressed(b)) if self.reporting() && state.held.is_none() => {
                let p = at?;
                let btn = button_of(*b)?;
                let cell = self.term.cell_at(p.x, p.y);
                state.held = Some(btn);
                state.last = Some(cell);
                send(self.report(MouseAction::Press(btn), cell))
            }
            Event::Mouse(mouse::Event::ButtonReleased(b)) if state.held.is_some() && state.held == button_of(*b) => {
                let btn = state.held.take()?;
                let cell = self.cell_clamped(bounds, cursor);
                state.last = Some(cell);
                send(self.report(MouseAction::Release(btn), cell))
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.anchor.is_none() && (state.held.is_some() || (self.reporting() && at.is_some())) => {
                let cell = self.cell_clamped(bounds, cursor);
                if state.last == Some(cell) {
                    return None;
                }
                state.last = Some(cell);
                self.report(MouseAction::Motion(state.held), cell).map(|b| canvas::Action::publish(Message::Mouse(id, b)).and_capture())
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let p = at?;
                // One step per notch (no smooth scrolling); pixels add up to a cell's height.
                state.wheel += match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / self.term.cell.height,
                };
                let steps = state.wheel.trunc();
                state.wheel -= steps;
                if steps == 0.0 {
                    return None;
                }
                let (up, n) = (steps > 0.0, steps.abs() as usize);
                if self.reporting() {
                    let cell = self.term.cell_at(p.x, p.y);
                    let one = self.report(if up { MouseAction::WheelUp } else { MouseAction::WheelDown }, cell)?;
                    return send(Some(one.repeat(n)));
                }
                if self.term.grid.alt {
                    return send(Some(term::wheel_keys(up, self.term.grid.app_cursor, 3 * n)));
                }
                // The shell's screen: the view scrolls through the scrollback, 3 lines a notch.
                let lines = 3 * n as isize;
                Some(canvas::Action::publish(Message::Scroll(id, if up { lines } else { -lines })).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = at?;
                let cell = self.term.text_cell_at(p.x, p.y);
                state.anchor = Some(cell);
                Some(canvas::Action::publish(Message::Select(id, None)).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let anchor = state.anchor?;
                // Past the top or bottom edge: the view starts scrolling (RedrawRequested above).
                let outside = cursor.position().is_some_and(|c| c.y < bounds.y || c.y > bounds.y + bounds.height);
                if outside && state.edge_tick.is_none() {
                    state.edge_tick = Some(Instant::now() - EDGE_TICK);
                } else if !outside {
                    state.edge_tick = None;
                }
                let cell = self.text_cell_clamped(bounds, cursor);
                Some(canvas::Action::publish(Message::Select(id, Some((anchor, cell)))).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.edge_tick = None;
                state.anchor.take()?;
                Some(canvas::Action::capture())
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                at?;
                let p = cursor.position()?;
                Some(canvas::Action::publish(Message::Menu(id, p)).and_capture())
            }
            _ => None,
        }
    }

    fn draw(&self, _state: &Self::State, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        self.term.draw(renderer, bounds, self.focused)
    }

    fn mouse_interaction(&self, _state: &Self::State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        // An arrow while the program has the mouse, as in xterm; else the text beam.
        let on_bar = cursor.position_in(bounds).is_some_and(|p| p.x >= bounds.width - term::SCROLLBAR_GRAB) && self.term.scrollbar(bounds.size()).is_some();
        if !cursor.is_over(bounds) || self.reporting() || on_bar { mouse::Interaction::default() } else { mouse::Interaction::Text }
    }
}

/// The bar's icons, drawn as lines (font glyphs for these differ in size and
/// height from font to font, so they never lined up).
#[derive(Debug, Clone, Copy)]
enum Glyph {
    Minimize,
    Maximize,
    Close,
    Plus,
}

struct Icon {
    glyph: Glyph,
    color: Color,
    size: f32,
}

impl Program<Message> for Icon {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let s = self.size;
        let (x, y) = ((bounds.width - s) / 2.0, (bounds.height - s) / 2.0);
        let stroke = canvas::Stroke::default().with_color(self.color).with_width(1.25).with_line_cap(canvas::LineCap::Round);
        let line = |frame: &mut canvas::Frame, a: (f32, f32), b: (f32, f32)| frame.stroke(&canvas::Path::line(Point::new(x + a.0, y + a.1), Point::new(x + b.0, y + b.1)), stroke);
        match self.glyph {
            // On the box's bottom edge, as GNOME and Windows draw it.
            Glyph::Minimize => line(&mut frame, (0.0, s), (s, s)),
            Glyph::Maximize => frame.stroke(&canvas::Path::rectangle(Point::new(x, y), Size::new(s, s)), stroke),
            Glyph::Close => {
                line(&mut frame, (0.0, 0.0), (s, s));
                line(&mut frame, (s, 0.0), (0.0, s));
            }
            Glyph::Plus => {
                line(&mut frame, (s / 2.0, 0.0), (s / 2.0, s));
                line(&mut frame, (0.0, s / 2.0), (s, s / 2.0));
            }
        }
        vec![frame.into_geometry()]
    }
}

/// An icon filling its button, drawn centred at `size` points.
fn icon<'a>(glyph: Glyph, color: Color, size: f32) -> Element<'a, Message> {
    Canvas::new(Icon { glyph, color, size }).width(Length::Fill).height(Length::Fill).into()
}

/// A flat button: no frame, an optional background.
fn plain(_s: button::Status, bg: Option<Color>) -> button::Style {
    button::Style { background: bg.map(Background::Color), text_color: TEXT_ACTIVE, border: Border::default(), ..button::Style::default() }
}

fn rounded(mut s: button::Style) -> button::Style {
    s.border = Border { radius: 5.0.into(), ..Border::default() };
    s
}

/// A terminal's shell output as messages, read on a thread of its own.
fn output(id: &u64) -> impl iced::futures::Stream<Item = Message> + use<> {
    let id = *id;
    iced::stream::channel(64, move |tx: iced::futures::channel::mpsc::Sender<Message>| async move {
        let reader = readers().remove(&id);
        if let Some(r) = reader {
            std::thread::spawn(move || {
                use iced::futures::SinkExt;
                let mut tx2 = tx.clone();
                pty::read_loop(r, |bytes| iced::futures::executor::block_on(tx2.send(Message::Output(id, bytes))).is_ok());
                let mut tx3 = tx;
                let _ = iced::futures::executor::block_on(tx3.send(Message::Exited(id)));
            });
        }
        std::future::pending::<()>().await;
    })
}

/// The desktop's look as messages, from a thread that watches its files.
fn watch_look() -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(4, |tx: iced::futures::channel::mpsc::Sender<Message>| async move {
        std::thread::spawn(move || {
            use iced::futures::SinkExt;
            let mut tx = tx;
            look::watch(look::Look::load(), |l| iced::futures::executor::block_on(tx.send(Message::Look(l))).is_ok());
        });
        std::future::pending::<()>().await;
    })
}

fn event(e: Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    match e {
        Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, text, .. }) => Some(Message::Key(key, modifiers, text.map(|t| t.to_string()))),
        Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => Some(Message::Modifiers(m)),
        Event::Window(iced::window::Event::Resized(size)) => Some(Message::Resized(size)),
        Event::Window(iced::window::Event::Focused) => Some(Message::Focused(true)),
        Event::Window(iced::window::Event::Unfocused) => Some(Message::Focused(false)),
        _ => None,
    }
}

/// Shortens a tab title to fit `width` points, ending it in an ellipsis.
/// iced 0.14 has no ellipsis, so widths are estimated per character (Noto
/// Sans averages about 0.55 em; narrow letters count less, wide ones more).
/// The tab also clips its text, so a low estimate never draws past the tab.
fn fit_label(label: &str, width: f32, size: f32) -> String {
    let w = |c: char| char_width(c, size);
    if text_width(label, size) <= width {
        return label.to_string();
    }
    let room = width - w('…');
    let mut used = 0.0;
    let mut out = String::new();
    for c in label.chars() {
        used += w(c);
        if used > room {
            break;
        }
        out.push(c);
    }
    format!("{}…", out.trim_end())
}

fn text_width(s: &str, size: f32) -> f32 {
    s.chars().map(|c| char_width(c, size)).sum()
}

fn char_width(c: char, size: f32) -> f32 {
    {
        let em = match c {
            'i' | 'l' | 'j' | 't' | 'f' | 'r' | 'I' | '.' | ',' | ':' | ';' | '\'' | '|' | '!' | ' ' => 0.30,
            'm' | 'w' | 'M' | 'W' | '@' => 0.85,
            c if c.is_uppercase() || c.is_ascii_digit() => 0.62,
            c if c.is_ascii() => 0.53,
            _ => 1.0,
        };
        em * size
    }
}

/// 80 x 24 cells, plus the padding and the title bar.
fn initial_size(look: &look::Look) -> Size {
    let cell = term::cell_for(look);
    Size::new(80.0 * cell.width + 2.0 * PAD, 24.0 * cell.height + 2.0 * PAD + BAR_H)
}

fn main() -> iced::Result {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("vergence-term {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // Software drawing (iced's tiny-skia renderer) unless told otherwise: on
    // Hubble (Intel UHD 630, FreeBSD 15.1) the GPU renderer hung the GPU again
    // and again ("GPU HANG ... in vergence-term"), and each reset froze the whole
    // desktop (2026-11-08). Vulkan doesn't present on vergence-server yet either.
    // A terminal is text: the CPU draws it easily. ICED_BACKEND=wgpu to try the GPU.
    if std::env::var_os("ICED_BACKEND").is_none() {
        unsafe { std::env::set_var("ICED_BACKEND", "tiny-skia") };
    }
    if std::env::var_os("WGPU_BACKEND").is_none() {
        unsafe { std::env::set_var("WGPU_BACKEND", "gl") };
    }
    // (No app_id: iced sets it on Linux only; the server names windows without
    // one by their program, vergence-term.)
    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .window(iced::window::Settings { size: initial_size(&look::Look::load()), decorations: false, ..Default::default() })
        .theme(|_: &App| Theme::CatppuccinMocha)
        .run()
}
