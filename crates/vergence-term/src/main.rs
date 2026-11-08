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
// Next: colour schemes, scrollback viewing.
//
// Drawing is in software (iced's tiny-skia): the GPU renderer hung Hubble's
// GPU and froze the desktop (2026-11-08).

mod grid;
mod pty;
mod term;
mod vt;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use iced::keyboard::{self, Key, key::Named};
use iced::widget::canvas::{self, Canvas, Geometry, Program};
use iced::widget::{Space, button, column, container, mouse_area, pane_grid, row, stack, text};
use iced::window::Direction;
use iced::{Background, Border, Color, Element, Event, Length, Padding, Point, Rectangle, Renderer, Size, Subscription, Task, Theme, mouse};

use term::{CellPos, PAD, Term};

/// The title bar's height, in points (the server's bar is 24; tabs want more).
const BAR_H: f32 = 34.0;
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
const TERM_BG: Color = Color::from_rgb(0x1e as f32 / 255.0, 0x1e as f32 / 255.0, 0x2e as f32 / 255.0);
/// The right-click menu's width, in points.
const MENU_W: f32 = 236.0;
const MENU_BG: Color = Color::from_rgb(0.17, 0.18, 0.23);
const ACCENT: Color = Color::from_rgb(0x89 as f32 / 255.0, 0xb4 as f32 / 255.0, 0xfa as f32 / 255.0);

#[derive(Debug, Clone)]
enum Message {
    Output(u64, Vec<u8>),
    Exited(u64),
    Key(Key, keyboard::Modifiers, Option<String>),
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
}

impl App {
    fn new() -> App {
        let mut app = App { terms: HashMap::new(), tabs: Vec::new(), active: 0, next_id: 1, size: initial_size(), focused: true, menu: None };
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
        let cell = term::cell_for(term::font_size());
        let cols = ((area.width - 2.0 * PAD) / cell.width).floor().max(2.0) as usize;
        let rows = ((area.height - 2.0 * PAD) / cell.height).floor().max(1.0) as usize;
        match Term::new(id, cols, rows) {
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
                if let Some(t) = self.terms.get_mut(&id) {
                    t.feed(&bytes);
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
                    // Typing clears the selection, as in other terminals.
                    t.select(None);
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
                let mut refocus = false;
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && let Some(pane) = tab.panes.iter().find(|(_, v)| **v == id).map(|(p, _)| *p)
                    && tab.focus != pane
                {
                    tab.focus = pane;
                    refocus = true;
                }
                if refocus {
                    self.repaint_tab();
                }
                if let Some(t) = self.terms.get_mut(&id) {
                    t.select(sel);
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
                pane_grid::PaneGrid::new(&tab.panes, move |pane, id, _| {
                    let content: Element<'_, Message> = match self.terms.get(id) {
                        Some(t) => Canvas::new(View { term: t, focused: self.focused && pane == focus }).width(Length::Fill).height(Length::Fill).into(),
                        None => Space::new().into(),
                    };
                    pane_grid::Content::new(content)
                })
                .spacing(DIVIDER)
                .on_click(Message::FocusPane)
                .on_resize(8, Message::PaneResized)
                .style(|_: &Theme| pane_grid::Style {
                    hovered_region: pane_grid::Highlight { background: Background::Color(Color { a: 0.2, ..ACCENT }), border: Border::default() },
                    picked_split: pane_grid::Line { color: ACCENT, width: 2.0 },
                    hovered_split: pane_grid::Line { color: ACCENT, width: 2.0 },
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
        let item = |label: &'static str, keys: &'static str, m: Option<Message>| {
            let enabled = m.is_some();
            let c = if enabled { TEXT_ACTIVE } else { TEXT_INACTIVE };
            let mut b = button(row![text(label).size(13).color(c).width(Length::Fill), text(keys).size(11).color(TEXT_INACTIVE)].spacing(12).align_y(iced::alignment::Vertical::Center))
                .width(Length::Fill)
                .padding(Padding::from([5, 10]))
                .style(move |_: &Theme, s| {
                    let bg = (enabled && s == button::Status::Hovered).then_some(Color { a: 0.9, ..ACCENT });
                    rounded(plain(s, bg))
                });
            if let Some(m) = m {
                b = b.on_press(m);
            }
            b
        };
        // A hairline with room around it (the padding outside the coloured line).
        let line = || container(container(Space::new().height(1.0).width(Length::Fill)).style(|_| container::background(Color { a: 0.12, ..Color::WHITE }))).padding(Padding::from([4, 8]));
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
        .style(|_| container::Style {
            background: Some(Background::Color(MENU_BG)),
            border: Border { color: Color { a: 0.15, ..Color::WHITE }, width: 1.0, radius: 8.0.into() },
            ..container::Style::default()
        });
        // Kept inside the window.
        let (mw, mh) = (MENU_W + 10.0, 250.0);
        let x = at.x.min(self.size.width - mw).max(0.0);
        let y = at.y.min(self.size.height - mh).max(0.0);
        let placed = container(menu).padding(Padding { top: y, left: x, right: 0.0, bottom: 0.0 });
        stack![mouse_area(Space::new().width(Length::Fill).height(Length::Fill)).on_press(Message::CloseMenu).on_right_press(Message::CloseMenu), placed].into()
    }

    /// The bar: tabs (the active one in the terminal's colour, joined to it),
    /// +, room to grab, and the window buttons.
    fn title_bar(&self) -> Element<'_, Message> {
        let fg = if self.focused { TEXT_ACTIVE } else { TEXT_INACTIVE };
        let bar_bg = if self.focused { BAR_ACTIVE } else { BAR_INACTIVE };
        let mut tabs = row![].spacing(2).height(Length::Fill).align_y(iced::alignment::Vertical::Bottom);
        for (i, tab) in self.tabs.iter().enumerate() {
            let active = i == self.active;
            let title = tab.panes.get(tab.focus).and_then(|id| self.terms.get(id)).map(Term::title).unwrap_or_default();
            let count = tab.panes.len();
            let label = if count > 1 { format!("{title}  ({count})") } else { title };
            let close = button(text("\u{2715}").size(11).color(fg))
                .padding(Padding::from([2, 6]))
                .on_press(Message::CloseTab(i))
                .style(move |_: &Theme, s| plain(s, if s == button::Status::Hovered { Some(Color { a: 0.18, ..Color::WHITE }) } else { None }));
            let content = row![text(label).size(12).color(fg).width(Length::Fill).wrapping(text::Wrapping::None), close].align_y(iced::alignment::Vertical::Center).spacing(4);
            let tab_button = button(content)
                .width(Length::Fixed(200.0))
                .height(Length::Fixed(BAR_H - 6.0))
                .padding(Padding { top: 0.0, bottom: 0.0, left: 12.0, right: 4.0 })
                .on_press(Message::SelectTab(i))
                .style(move |_: &Theme, s| {
                    let bg = if active {
                        Some(TERM_BG)
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
        let new_tab = button(text("+").size(16).color(fg))
            .padding(Padding::from([2, 10]))
            .on_press(Message::NewTab)
            .style(move |_: &Theme, s| rounded(plain(s, if s == button::Status::Hovered { Some(Color { a: 0.12, ..Color::WHITE }) } else { None })));
        let grab = mouse_area(Space::new().width(Length::Fill).height(Length::Fill)).on_press(Message::Drag).on_double_click(Message::ToggleMaximize);
        let win_button = |label: &'static str, m: Message, danger: bool| {
            button(text(label).size(12).color(fg))
                .width(Length::Fixed(32.0))
                .height(Length::Fixed(24.0))
                .padding(Padding::from([4, 10]))
                .on_press(m)
                .style(move |_: &Theme, s| {
                    let hover = if danger { CLOSE } else { Color { a: 0.15, ..Color::WHITE } };
                    rounded(plain(s, if s == button::Status::Hovered { Some(hover) } else { None }))
                })
        };
        let buttons = row![win_button("\u{2013}", Message::Minimize, false), win_button("\u{25a1}", Message::ToggleMaximize, false), win_button("\u{2715}", Message::CloseWindow, true)]
            .spacing(2)
            .align_y(iced::alignment::Vertical::Center);
        let bar = row![tabs, new_tab, grab, buttons].spacing(4).align_y(iced::alignment::Vertical::Center).padding(Padding { top: 4.0, bottom: 0.0, left: 6.0, right: 6.0 });
        container(bar).height(Length::Fixed(BAR_H)).width(Length::Fill).style(move |_| container::background(bar_bg)).into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs: Vec<Subscription<Message>> = self.terms.keys().map(|id| Subscription::run_with(*id, output)).collect();
        subs.push(iced::event::listen_with(event));
        Subscription::batch(subs)
    }
}

/// A pane on screen: the terminal, and whether it has the keyboard. Mouse:
/// left drag selects (a click clears), right opens the menu.
struct View<'a> {
    term: &'a Term,
    focused: bool,
}

impl Program<Message> for View<'_> {
    /// The selection's anchor while the left button is held.
    type State = Option<CellPos>;

    fn update(&self, state: &mut Self::State, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<Message>> {
        let at = cursor.position_in(bounds);
        let id = self.term.id;
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = at?;
                let cell = self.term.cell_at(p.x, p.y);
                *state = Some(cell);
                Some(canvas::Action::publish(Message::Select(id, None)).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let anchor = (*state)?;
                let p = cursor.position_in(bounds).unwrap_or_else(|| {
                    // Dragging past the pane's edge keeps selecting up to it.
                    let c = cursor.position().unwrap_or(bounds.position());
                    Point::new((c.x - bounds.x).clamp(0.0, bounds.width), (c.y - bounds.y).clamp(0.0, bounds.height))
                });
                let cell = self.term.cell_at(p.x, p.y);
                Some(canvas::Action::publish(Message::Select(id, Some((anchor, cell)))).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.take()?;
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
        if cursor.is_over(bounds) { mouse::Interaction::Text } else { mouse::Interaction::default() }
    }
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

fn event(e: Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    match e {
        Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, text, .. }) => Some(Message::Key(key, modifiers, text.map(|t| t.to_string()))),
        Event::Window(iced::window::Event::Resized(size)) => Some(Message::Resized(size)),
        Event::Window(iced::window::Event::Focused) => Some(Message::Focused(true)),
        Event::Window(iced::window::Event::Unfocused) => Some(Message::Focused(false)),
        _ => None,
    }
}

/// 80 x 24 cells, plus the padding and the title bar.
fn initial_size() -> Size {
    let cell = term::cell_for(term::font_size());
    Size::new(80.0 * cell.width + 2.0 * PAD, 24.0 * cell.height + 2.0 * PAD + BAR_H)
}

fn main() -> iced::Result {
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
        .window(iced::window::Settings { size: initial_size(), decorations: false, ..Default::default() })
        .theme(|_: &App| Theme::CatppuccinMocha)
        .run()
}
