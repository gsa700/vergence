// Vergence's terminal. Step 1 (2026-11-08): our own terminal core (vt.rs
// parser, grid.rs screen, pty.rs) drawn with iced at the screen's own pixels in
// the desktop's monospace font. Step 2: tabs in the title bar (Dave: "I prefer
// the tabs in the titlebar ala Firefox"): the window draws its own bar (the
// server leaves its own off for windows that ask), in the server's colours,
// with the tabs, a + button and the window buttons; the bar moves the window,
// a double-click maximises, the edges resize. Next: split panes and the
// right-click menu, then colour schemes.

mod grid;
mod pty;
mod term;
mod vt;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Canvas, Space, button, column, container, mouse_area, row, stack, text};
use iced::window::Direction;
use iced::{Background, Border, Color, Element, Event, Length, Padding, Size, Subscription, Task, Theme, mouse};

use term::{PAD, Tab};

/// The title bar's height, in points (the server's bar is 24; tabs want more).
const BAR_H: f32 = 34.0;
/// The invisible border that resizes the window, in points.
const EDGE: f32 = 5.0;

// The server's title bar colours (vergence-server decor.rs), so our bar
// matches the windows it decorates.
const BAR_ACTIVE: Color = Color::from_rgb(0.20, 0.24, 0.32);
const BAR_INACTIVE: Color = Color::from_rgb(0.22, 0.22, 0.22);
const TEXT_ACTIVE: Color = Color::from_rgb(0.92, 0.92, 0.92);
const TEXT_INACTIVE: Color = Color::from_rgb(0.62, 0.62, 0.62);
const CLOSE: Color = Color::from_rgb(0.78, 0.30, 0.28);
/// The terminal's background: the active tab is drawn in it, joined to the text below.
const TERM_BG: Color = Color::from_rgb(0x1e as f32 / 255.0, 0x1e as f32 / 255.0, 0x2e as f32 / 255.0);

#[derive(Debug, Clone)]
enum Message {
    Output(u64, Vec<u8>),
    Exited(u64),
    Key(Key, keyboard::Modifiers, Option<String>),
    Resized(Size),
    Focused(bool),
    NewTab,
    SelectTab(usize),
    CloseTab(u64),
    Drag,
    ToggleMaximize,
    Minimize,
    CloseWindow,
    ResizeEdge(Direction),
}

/// Each tab's shell output, handed from the tab's creation to its subscription
/// (iced builds streams from plain functions).
static READERS: OnceLock<Mutex<HashMap<u64, std::fs::File>>> = OnceLock::new();

fn readers() -> std::sync::MutexGuard<'static, HashMap<u64, std::fs::File>> {
    READERS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap()
}

struct App {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    /// The window's size (points) and whether it has the keyboard.
    size: Size,
    focused: bool,
}

impl App {
    fn new() -> App {
        let mut app = App { tabs: Vec::new(), active: 0, next_id: 1, size: initial_size(), focused: true };
        app.open_tab();
        app
    }

    /// The terminal area: the window minus the title bar.
    fn body(&self) -> Size {
        Size::new(self.size.width, (self.size.height - BAR_H).max(1.0))
    }

    fn open_tab(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        let cell = term::cell_for(term::font_size());
        let body = self.body();
        let cols = ((body.width - 2.0 * PAD) / cell.width).floor().max(2.0) as usize;
        let rows = ((body.height - 2.0 * PAD) / cell.height).floor().max(1.0) as usize;
        match Tab::new(id, cols, rows) {
            Ok((tab, reader)) => {
                readers().insert(id, reader);
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
            }
            Err(e) => eprintln!("vergence-term: cannot start a shell: {e}"),
        }
    }

    /// Remove a tab; the window closes with its last tab.
    fn remove_tab(&mut self, id: u64) -> Task<Message> {
        let Some(i) = self.tabs.iter().position(|t| t.id == id) else { return Task::none() };
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            return iced::exit();
        }
        if self.active >= i && self.active > 0 {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len() - 1);
        Task::none()
    }

    fn update(&mut self, m: Message) -> Task<Message> {
        match m {
            Message::Output(id, bytes) => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                    t.feed(&bytes);
                }
            }
            Message::Exited(id) => return self.remove_tab(id),
            Message::Key(key, mods, txt) => {
                // The terminal's own shortcuts (Ctrl+Shift so the shell keeps Ctrl+T etc.).
                let cs = mods.control() && mods.shift();
                match &key {
                    Key::Character(c) if cs && c.eq_ignore_ascii_case("t") => {
                        self.open_tab();
                        return Task::none();
                    }
                    Key::Character(c) if cs && c.eq_ignore_ascii_case("w") => {
                        let id = self.tabs.get(self.active).map(|t| t.id);
                        return id.map(|id| self.remove_tab(id)).unwrap_or_else(Task::none);
                    }
                    Key::Named(Named::PageUp) if mods.control() && !mods.shift() => {
                        self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
                        return Task::none();
                    }
                    Key::Named(Named::PageDown) if mods.control() && !mods.shift() => {
                        self.active = (self.active + 1) % self.tabs.len();
                        return Task::none();
                    }
                    Key::Named(Named::Tab) if mods.control() => {
                        let n = self.tabs.len();
                        self.active = if mods.shift() { (self.active + n - 1) % n } else { (self.active + 1) % n };
                        return Task::none();
                    }
                    _ => {}
                }
                if let Some(t) = self.tabs.get_mut(self.active)
                    && let Some(bytes) = term::key_bytes(&key, mods, txt.as_deref(), t.grid.app_cursor)
                {
                    t.pty.write(&bytes);
                }
            }
            Message::Resized(size) => {
                self.size = size;
                let body = self.body();
                for t in &mut self.tabs {
                    t.fit(body);
                }
            }
            Message::Focused(f) => self.focused = f,
            Message::NewTab => self.open_tab(),
            Message::SelectTab(i) => self.active = i.min(self.tabs.len().saturating_sub(1)),
            Message::CloseTab(id) => return self.remove_tab(id),
            Message::Drag => return iced::window::latest().and_then(iced::window::drag),
            Message::ToggleMaximize => return iced::window::latest().and_then(iced::window::toggle_maximize),
            Message::Minimize => return iced::window::latest().and_then(|id| iced::window::minimize(id, true)),
            Message::CloseWindow => return iced::exit(),
            Message::ResizeEdge(d) => return iced::window::latest().and_then(move |id| iced::window::drag_resize(id, d)),
        }
        Task::none()
    }

    fn title(&self) -> String {
        self.tabs.get(self.active).map(Tab::title).unwrap_or_else(|| "Terminal".into())
    }

    fn view(&self) -> Element<'_, Message> {
        let body: Element<'_, Message> = match self.tabs.get(self.active) {
            Some(t) => Canvas::new(t).width(Length::Fill).height(Length::Fill).into(),
            None => Space::new().into(),
        };
        let window = column![self.title_bar(), body];
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
        stack![window, edges].into()
    }

    /// The bar: tabs (the active one in the terminal's colour, joined to it),
    /// +, room to grab, and the window buttons.
    fn title_bar(&self) -> Element<'_, Message> {
        let fg = if self.focused { TEXT_ACTIVE } else { TEXT_INACTIVE };
        let bar_bg = if self.focused { BAR_ACTIVE } else { BAR_INACTIVE };
        let mut tabs = row![].spacing(2).height(Length::Fill).align_y(iced::alignment::Vertical::Bottom);
        for (i, t) in self.tabs.iter().enumerate() {
            let active = i == self.active;
            let close = button(text("\u{2715}").size(11).color(fg))
                .padding(Padding::from([2, 6]))
                .on_press(Message::CloseTab(t.id))
                .style(move |_: &Theme, s| plain(s, if s == button::Status::Hovered { Some(Color { a: 0.18, ..Color::WHITE }) } else { None }));
            let label = row![text(t.title()).size(12).color(fg).width(Length::Fill).wrapping(text::Wrapping::None), close]
                .align_y(iced::alignment::Vertical::Center)
                .spacing(4);
            let tab = button(label)
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
            tabs = tabs.push(tab);
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
        let mut subs: Vec<Subscription<Message>> = self.tabs.iter().map(|t| Subscription::run_with(t.id, output)).collect();
        subs.push(iced::event::listen_with(event));
        Subscription::batch(subs)
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

/// A tab's shell output as messages, read on a thread of its own.
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
    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        // (No app_id: iced sets it on Linux only; the server names windows without
        // one by their program, vergence-term.)
        .window(iced::window::Settings { size: initial_size(), decorations: false, ..Default::default() })
        .theme(|_: &App| Theme::CatppuccinMocha)
        .run()
}
