//! A window and its input (whitepaper §6.10; toolkit milestone 1, step 2).
//!
//! The first piece of the toolkit proper. A program says what its window
//! looks like at a size ([`App::scene`]) and what it does with input
//! ([`App::input`]); this crate asks the server for the window, learns its
//! size and the exact scale of the monitor it is on, draws the scene at that
//! scale and hands the server the pixels, once for each frame the server
//! asks for and only when something changed.
//!
//! **Input arrives in order and at the place it happened.** Every pointer
//! and key event is given to the program as it is read, a button press with
//! the position the pointer had at that press, never at a later one, and
//! none waits for a frame to be drawn. A program that wants a new picture
//! says so ([`Window::redraw`]); however many events ask, one frame is drawn.
//!
//! **Frames are drawn on the GPU** where there is one and the server takes
//! its buffers (linux-dmabuf): the scene is drawn into memory on the card
//! and the server shows that memory as it is. Otherwise, and for any frame
//! the GPU renderer cannot draw yet, the CPU renderer draws into memory
//! shared with the server (`wl_shm`). `VERGENCE_RENDER=cpu` in the
//! environment keeps to the CPU. There are no widgets yet.
//!
//! **The clipboard** is text for now. [`Window::copy`] puts text on it; the
//! text is written to whoever asks on a thread of its own, so a slow reader
//! never holds the window up. [`Window::paste`] asks for what is there; it
//! arrives as [`Input::Paste`] when the other program has written it, read
//! in pieces between other events, with a limit on its size and on the wait.
//!
//! Sizes and positions a program sees are in reference units (whitepaper
//! Contract 1). It is never told to scale anything: the same scene is drawn
//! sharper on a denser monitor, and drawn again when the window moves to
//! another.
//!
//! **Two roads** (native milestone 1, step 5): in a Vergence session the
//! window opens on the project's own protocol ([`native`]); elsewhere, or if
//! the server will not, over Wayland (the rest of this file). A program
//! cannot tell which: the same calls and the same input either way.

/// The bar, in its own crate since bar milestone 1 so the server draws the same one.
pub use vergence_bar as bar;
pub mod keyboard;
mod native;
pub mod touch;

use std::{
    collections::HashMap,
    io::{self, Read, Write},
    os::fd::AsRawFd,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use vergence_render::{Cpu, Gpu, GpuBuffer, GpuError, Order, Target, pixel_size};
use vergence_scene::{Color, Scene};
use vergence_text::Fonts;
use vergence_wl::{
    Client, Event, Interface, client as wl,
    shm::{ShmFile, read_shared},
};

pub use keyboard::{Modifiers, sym};
/// Where a window asks to be (Contract 2): the server decides. Asked for by
/// [`App::place`] when the window opens and [`Window::ask_place`] later; the
/// native road only (Wayland lets no client place its window).
pub use vergence_protocol::window::Place;

/// One widget of a window's meaning (Contract 3), for the server: its stable
/// id and its nearest ancestor's with a meaning (0: the window), what it is,
/// its name, its value (never a secret's), its states, and its bounds in the
/// window in reference units.
#[derive(Debug, Clone, PartialEq)]
pub struct MeaningNode {
    pub id: u64,
    pub parent: u64,
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    pub states: Vec<String>,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A window's whole meaning: its widgets, the focused one's id (0: none) and
/// the text caret, if a field has one.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MeaningTree {
    pub focus: u64,
    pub caret: Option<(f32, f32, f32, f32)>,
    pub nodes: Vec<MeaningNode>,
}

/// The largest keymap accepted from the server.
const MAX_KEYMAP: usize = 4 << 20;
/// The most text taken from the clipboard, and how long a paste may take.
const MAX_PASTE: usize = 16 << 20;
const PASTE_TIMEOUT: Duration = Duration::from_secs(5);
/// The most kinds of data remembered for one offer, and the longest name of one.
const MAX_KINDS: usize = 64;
const MAX_KIND_NAME: usize = 256;
/// The kinds of text put on the clipboard and looked for on it, best first.
const TEXT_KINDS: [&str; 5] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "TEXT", "STRING"];

/// What a frame is being drawn for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// The window's size, in reference units.
    pub width: f32,
    pub height: f32,
    /// Pixels to the reference unit on the monitor the window is on. For
    /// information only: a scene is the same at every scale.
    pub scale: f32,
    /// Seconds since the window opened, for things that move.
    pub time: f64,
    /// How many frames have been shown before this one.
    pub number: u64,
    /// What drew the frame before this one: "GPU", "CPU", or "" for the first.
    pub drawn_by: &'static str,
    /// The server draws the bar this window would draw ([`App::own_frame`],
    /// on the native road, with its tabs if [`App::tabs_by_server`]): the
    /// scene leaves it out and starts at the top.
    pub bar_by_server: bool,
    /// The strip at the top of the scene that lies under the server's bar,
    /// for a window that draws under it ([`App::under_bar`]; 0 otherwise):
    /// `height` includes it, the content goes below it, and nothing there
    /// takes input (the bar's presses are the server's). Input positions are
    /// in the scene's own points, the strip included.
    pub top: f32,
    /// The wallpaper's color behind the window, for a little of it in its
    /// grounds (window styles milestone 1: the server sends it in dark style
    /// only); None: no tint.
    pub tint: Option<Color>,
    /// Text on this screen is drawn this much larger than its sizes say
    /// (the server's text factor, bus v21; 1 on the Wayland road): sizes are
    /// as on a screen at its calculated scale, so they look the same on
    /// every screen. `vergence_look::text_size` applies it.
    pub text: f32,
    /// The native road (a Vergence session): the server draws what the
    /// desktop draws for every window (its bar, its right-click menus).
    pub native: bool,
}

/// What the bar's description is made of ([`App::bar`]): the native
/// protocol's own types.
pub use vergence_protocol::window::{Bar, BarButton, BarIcon, DROP_COPY, DROP_MOVE, DROP_NONE, MenuEntry as BarMenuEntry, Place as BarPlace, Tab as BarTab};

/// What the user did with the tabs the server drew ([`App::tab`]); each
/// tab named by the id the program gave it in [`App::bar`].
#[derive(Debug, Clone, PartialEq)]
pub enum TabEvent {
    Chosen(u32),
    Close(u32),
    CloseOthers(u32),
    /// The + was pressed.
    New,
    /// Dragged to another place in the row (counted with it taken out first).
    Moved { tab: u32, to: u32 },
    /// Dragged out of the window, or "Move to a new window": a window for it, there.
    Out { tab: u32, place: BarPlace },
    /// Dropped on another window of this program, at `to`.
    Join { tab: u32, into: u32, to: u32 },
    /// One of the program's own tab-menu entries.
    Menu { tab: u32, entry: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
    /// Another button, by its Linux input code.
    Other(u32),
}

/// Something the user did. Positions are in the window, in reference units.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    PointerMoved { x: f32, y: f32 },
    /// The pointer left the window.
    PointerLeft,
    /// A button went down or up, at the place the pointer was then.
    Button { button: Button, pressed: bool, x: f32, y: f32, modifiers: Modifiers },
    /// The wheel or a touchpad: positive `dy` scrolls down, positive `dx` right.
    Scroll { x: f32, y: f32, dx: f32, dy: f32 },
    /// A key: its symbol ([`sym`], or a character's code), the text it
    /// types if it types any, and whether this is a held key repeating.
    /// `composing`: the key went into a dead-key or compose sequence that is
    /// not finished; it types nothing and should not be taken as a command.
    Key { sym: u32, text: Option<String>, pressed: bool, repeat: bool, composing: bool, modifiers: Modifiers },
    /// A finger on the window ([`touch`]): `id` tells it from the others
    /// while it is down. Where it lifted is where it last was.
    Touch { id: i32, phase: touch::Phase, x: f32, y: f32 },
    /// The window gained or lost the keyboard.
    Focus(bool),
    /// The clipboard's text, asked for with [`Window::paste`].
    Paste(String),
}

/// A choice of light or dark as an app's table holds it (app settings
/// milestone 1): absent or "default" is the desktop's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Desktop,
    Dark,
    Light,
}

/// A value of desktop.toml's text: `key` in table `section` (as
/// "[terminal]"), quotes and a trailing comment taken off.
pub fn table_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut within = "";
    let mut found = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            within = line;
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        if within == section && k.trim() == key {
            let v = v.trim();
            let v = match v.strip_prefix('"') {
                Some(rest) => rest.split('"').next().unwrap_or(""),
                None => v.split('#').next().unwrap_or("").trim(),
            };
            found = Some(v.to_string());
        }
    }
    found
}

/// The app's own light or dark for `key` ("style" for its content,
/// "bar-style" for its window bar) in its table.
pub fn app_pick(text: &str, section: &str, key: &str) -> Pick {
    match table_value(text, section, key).as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("dark") => Pick::Dark,
        Some("light") => Pick::Light,
        _ => Pick::Desktop,
    }
}

/// The desktop's Style (`[appearance] color-scheme`): Some(true) light,
/// Some(false) dark, None "App default" (each app its own).
pub fn desktop_pick(text: &str) -> Option<bool> {
    match table_value(text, "[appearance]", "color-scheme").as_deref() {
        Some("light") => Some(true),
        Some("dark") => Some(false),
        _ => None,
    }
}

/// Whether an app's content is light (app settings milestone 1). The
/// desktop's Style is the master switch (Dave, 2026-11-14): Dark or Light
/// forces every app; under "App default" each app is as it wants, its own
/// `style` if it chose one, else its built-in look.
pub fn content_light(text: &str, section: &str, builtin_light: bool) -> bool {
    if let Some(forced) = desktop_pick(text) {
        return forced;
    }
    match app_pick(text, section, "style") {
        Pick::Light => true,
        Pick::Dark => false,
        Pick::Desktop => builtin_light,
    }
}

/// The bar style an app asks the server for: the desktop's while the
/// desktop's Style forces Dark or Light; under "App default" its own
/// `bar-style` if it chose one, else its built-in one (the settings app's
/// light bar).
pub fn bar_style_setting(text: &str, section: &str, builtin: BarStyle) -> BarStyle {
    if desktop_pick(text).is_some() {
        return BarStyle::Desktop;
    }
    match app_pick(text, section, "bar-style") {
        Pick::Light => BarStyle::Light,
        Pick::Dark => BarStyle::Dark,
        Pick::Desktop => builtin,
    }
}

/// How opaque a program's window ground is to be, from desktop.toml's text:
/// `opacity` in the program's own table (`section`, as "[terminal]") if it
/// is there, else `window-opacity` under `[appearance]`, the desktop's word
/// for every program of ours, else 1. From 0.3 (less could not be read
/// on) to 1 (nothing shows through). A program gives its
/// [`App::background`] that much alpha and draws its ground no other way;
/// what it draws on top (text, its bar) stays as it is.
pub fn opacity_setting(text: &str, section: &str) -> f32 {
    let (mut own, mut desktop) = (None, None);
    let mut within = "";
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            within = line;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.split('#').next().unwrap_or("").trim().parse::<f32>().ok().filter(|v| v.is_finite());
        match key.trim() {
            "opacity" if within == section => own = value.or(own),
            "window-opacity" if within == "[appearance]" => desktop = value.or(desktop),
            _ => {}
        }
    }
    own.or(desktop).unwrap_or(1.0).clamp(0.3, 1.0)
}

/// The pointer's shape over the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Default,
    /// A hand, over something that can be pressed.
    Pointer,
    Text,
    Move,
    Grab,
    /// Resizing from these edges ([`Edges`]).
    Resize(Edges),
    /// ↔ between two columns, pulled to change a column's width.
    ColResize,
}

/// Sides of the window, for resizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Edges {
    fn wire(self) -> u32 {
        (self.top as u32 * wl::EDGE_TOP) | (self.bottom as u32 * wl::EDGE_BOTTOM) | (self.left as u32 * wl::EDGE_LEFT) | (self.right as u32 * wl::EDGE_RIGHT)
    }
}

impl Cursor {
    fn wire(self) -> u32 {
        match self {
            Cursor::Default => wl::SHAPE_DEFAULT,
            Cursor::Pointer => wl::SHAPE_POINTER,
            Cursor::Text => wl::SHAPE_TEXT,
            Cursor::Move => wl::SHAPE_MOVE,
            Cursor::Grab => wl::SHAPE_GRAB,
            Cursor::ColResize => wl::SHAPE_COL_RESIZE,
            Cursor::Resize(e) => match (e.top, e.bottom, e.left, e.right) {
                (true, _, true, _) => wl::SHAPE_NW_RESIZE,
                (true, _, _, true) => wl::SHAPE_NE_RESIZE,
                (_, true, true, _) => wl::SHAPE_SW_RESIZE,
                (_, true, _, true) => wl::SHAPE_SE_RESIZE,
                (true, ..) => wl::SHAPE_N_RESIZE,
                (_, true, ..) => wl::SHAPE_S_RESIZE,
                (_, _, true, _) => wl::SHAPE_W_RESIZE,
                (_, _, _, true) => wl::SHAPE_E_RESIZE,
                _ => wl::SHAPE_DEFAULT,
            },
        }
    }
}

enum Command {
    Redraw,
    Close,
    Move,
    Resize(Edges),
    Cursor(Cursor),
    Title(String),
    Maximize(bool),
    Minimize,
    Copy(String),
    Paste,
    Timer(u64, Option<Instant>),
    Place(Place),
    /// Another window of the same program (bar milestone 1, step 4).
    Open(Box<dyn App>),
    /// A size the window would like (the server decides).
    Size(u32, u32),
    /// Start dragging (the native road; files milestone 1, step 8).
    Drag(String, Vec<(String, Vec<u8>)>),
    /// Forward with the keyboard, if the server agrees.
    Activate,
    /// The server's menu with the window's commands, at this point in it.
    ContextMenu(f32, f32),
    Menu(f32, f32, Vec<MenuLine>),
}

/// What a program may ask of its window while it handles input.
#[derive(Default)]
pub struct Window {
    commands: Vec<Command>,
}

impl Window {
    /// The picture has changed: draw a frame.
    pub fn redraw(&mut self) {
        self.commands.push(Command::Redraw);
    }

    pub fn close(&mut self) {
        self.commands.push(Command::Close);
    }

    /// Asks for the window to be this size, in points; the server decides
    /// (within its limits and its screen). Only where the server draws the
    /// bar; elsewhere nothing happens.
    pub fn request_size(&mut self, width: u32, height: u32) {
        self.commands.push(Command::Size(width, height));
    }

    /// Starts dragging, while the pointer's button is held (a row pulled
    /// away): each kind with its data, and the words the ghost shows. On the
    /// native road; elsewhere nothing happens yet.
    pub fn start_drag(&mut self, label: &str, items: Vec<(String, Vec<u8>)>) {
        self.commands.push(Command::Drag(label.to_string(), items));
    }

    /// Asks to come forward with the keyboard: the program was just handed
    /// something to show (a file another start passed on). The server grants
    /// it only for a start the person just made, or while the program has the
    /// keyboard anyway: no stealing focus.
    pub fn activate(&mut self) {
        self.commands.push(Command::Activate);
    }

    /// Asks the server to open its menu here (window points) with the
    /// window's commands: a right click answered in the desktop's menu style.
    /// The commands are sent first if they changed. The native road only.
    pub fn context_menu(&mut self, x: f32, y: f32) {
        self.commands.push(Command::ContextMenu(x, y));
    }

    /// Asks the server to open the window's own right-click menu here
    /// (window points) with these lines (bus v20): a file's menu, the
    /// ground's. A chosen line arrives as the app's command with its id.
    /// The native road only; elsewhere nothing opens yet.
    pub fn menu(&mut self, x: f32, y: f32, lines: Vec<MenuLine>) {
        self.commands.push(Command::Menu(x, y, lines));
    }

    /// Lets the user move the window with the button that was just pressed.
    pub fn start_move(&mut self) {
        self.commands.push(Command::Move);
    }

    /// Lets the user resize the window from these edges with the button
    /// that was just pressed.
    pub fn start_resize(&mut self, edges: Edges) {
        self.commands.push(Command::Resize(edges));
    }

    pub fn set_cursor(&mut self, cursor: Cursor) {
        self.commands.push(Command::Cursor(cursor));
    }

    pub fn set_title(&mut self, title: &str) {
        self.commands.push(Command::Title(title.to_string()));
    }

    pub fn set_maximized(&mut self, maximized: bool) {
        self.commands.push(Command::Maximize(maximized));
    }

    pub fn minimize(&mut self) {
        self.commands.push(Command::Minimize);
    }

    /// Puts text on the clipboard, in answer to the key or button just pressed.
    pub fn copy(&mut self, text: &str) {
        self.commands.push(Command::Copy(text.to_string()));
    }

    /// Asks for the clipboard's text; it arrives as [`Input::Paste`], if
    /// there is any.
    pub fn paste(&mut self) {
        self.commands.push(Command::Paste);
    }

    /// Asks for [`App::timer`] to be called with `id` at `at` (at once if
    /// that has passed). Asking again for the same `id` moves the time;
    /// each request is answered once.
    pub fn wake_at(&mut self, id: u64, at: Instant) {
        self.commands.push(Command::Timer(id, Some(at)));
    }

    /// [`Window::wake_at`] `after` from now.
    pub fn wake_after(&mut self, id: u64, after: Duration) {
        self.wake_at(id, Instant::now() + after);
    }

    /// Forgets the timer `id`, if it has not gone off.
    pub fn cancel_timer(&mut self, id: u64) {
        self.commands.push(Command::Timer(id, None));
    }

    /// Asks to be somewhere else (Contract 2); the server decides. Native
    /// road only.
    pub fn ask_place(&mut self, place: Place) {
        self.commands.push(Command::Place(place));
    }

    /// Opens another window of this program, run by `app` (a tab torn off,
    /// a second terminal window). The windows of a program share its
    /// keyboard, clipboard and waker; the program ends when the last one
    /// closes. Native road only for now: elsewhere the window is not opened
    /// (the Wayland road follows in bar milestone 1, step 4b).
    pub fn open_window(&mut self, app: Box<dyn App>) {
        self.commands.push(Command::Open(app));
    }
}

/// The most timers a window keeps at once; asking for more drops the
/// latest-due.
const MAX_TIMERS: usize = 256;

/// A way for another thread to get the window's attention: [`App::woken`]
/// is then called on the window's thread. However many times it is used
/// before that happens, the program is woken once.
#[derive(Clone)]
pub struct Waker {
    pipe: Arc<std::fs::File>,
    pending: Arc<AtomicBool>,
}

impl Waker {
    pub fn wake(&self) {
        if !self.pending.swap(true, Ordering::SeqCst) {
            let _ = (&*self.pipe).write(&[1]);
        }
    }
}

/// A program with one window.
/// A window's kind, for [`Style`].
pub use vergence_protocol::window::WinKind;
/// An app menu's commands and their roles (app settings milestone 1), and
/// the bar style an app asks for.
pub use vergence_protocol::commands::{Command as AppCommand, Role};
pub use vergence_protocol::window::BarStyle;
pub use vergence_protocol::window::MenuLine;

/// What kind of window an app's is, and its size limits in points (0: none):
/// told to the server on the native road (window styles milestone 1).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Style {
    pub kind: vergence_protocol::window::WinKind,
    pub modal: bool,
    pub min: (f64, f64),
    pub max: (f64, f64),
    /// A window another window opened that is not that window's (a
    /// Properties window, files milestone 2): opened with no parent, so it
    /// goes nowhere with its opener. A dialog so opened is a lone one to the
    /// server: a window of its own with the slim close-only bar, no app button.
    pub alone: bool,
}

pub trait App {
    fn title(&self) -> String;

    /// The program's name to the desktop (window memory, the shelf).
    fn app_id(&self) -> String;

    /// True if the window draws its own bar; false asks the server for one.
    /// On the native road the server draws every bar: the window is told so
    /// by [`Frame::bar_by_server`] and leaves its own out.
    fn own_frame(&self) -> bool {
        false
    }

    /// What is behind the scene. Not opaque: the desktop shows through.
    fn background(&self) -> Color {
        Color::rgb(0, 0, 0)
    }

    /// The window's picture for this frame.
    fn scene(&mut self, frame: &Frame, fonts: &mut Fonts) -> Scene;

    /// Something the user did.
    fn input(&mut self, _input: &Input, _window: &mut Window) {}

    /// Called once, before the window opens, with the means for other
    /// threads to wake it.
    fn waker(&mut self, _waker: Waker) {}

    /// Another thread used the [`Waker`].
    fn woken(&mut self, _window: &mut Window) {}

    /// The time asked for with [`Window::wake_at`] has come.
    fn timer(&mut self, _id: u64, _window: &mut Window) {}

    /// True while the picture changes by itself: a new frame is then drawn
    /// every time the monitor is ready for one.
    fn animating(&self) -> bool {
        false
    }

    /// Called after a frame has been handed to the server, with how long
    /// drawing it took. Return false to close the window.
    fn shown(&mut self, _frame: &Frame, _draw_ms: f64) -> bool {
        true
    }

    /// What this window is among its program's (Contract 2): window memory
    /// keeps a place for each role apart, so a program's Properties window
    /// does not take its main window's place. "main" unless said.
    fn role(&self) -> String {
        "main".into()
    }

    /// The persistence token this window had on an earlier run, if the
    /// program kept one (Contract 2): presented when the window is created,
    /// so window memory puts it back where it was. Empty: a new window.
    fn token(&self) -> String {
        String::new()
    }

    /// The server's token for this window, once it exists: kept by a
    /// program that wants the window back in its place on the next run.
    fn token_given(&mut self, _token: &str) {}

    /// The toolkit's number for this window, as the server's tab events name
    /// another window of the program ([`TabEvent::Join`]'s `into`). Native
    /// road only: called before the window's first frame.
    fn opened(&mut self, _id: u32) {}

    /// The user asked to close the window (its bar's close button, the
    /// server's close). True closes it; false keeps it, and the program may
    /// ask first and close it later with [`Window::close`] (the terminal with
    /// several tabs, settings milestone 2). A close the program makes itself
    /// does not come here.
    fn close_asked(&mut self, _window: &mut Window) -> bool {
        true
    }

    /// The window has closed (the user closed it, or the program did) while
    /// the program runs on. Not called when the program is ended from outside
    /// (a logout): what it keeps of its windows then stays for next time.
    fn closed(&mut self) {}

    /// Where the window asks to be when it opens (Contract 2).
    fn place(&self) -> Place {
        Place::Anywhere
    }

    /// The content size, in reference units, a window opened with
    /// [`Window::open_window`] asks for (a torn-off tab's window: the size of
    /// the one it left). None: the toolkit's own. Window memory still wins.
    fn opening_size(&self) -> Option<(u32, u32)> {
        None
    }

    /// True if the window's bar has tabs the server may draw (bar milestone
    /// 1): on the native road the window then leaves its frame to the server,
    /// describes its tabs with [`App::bar`], hears what was done with them in
    /// [`App::tab`], and draws no bar of its own ([`Frame::bar_by_server`]).
    /// Elsewhere it draws its own bar as before.
    fn tabs_by_server(&self) -> bool {
        false
    }

    /// True if the window draws under its bar (look milestone 1): on the
    /// native road, where the server draws the bar, its scene is taller by
    /// [`Frame::top`] and what it draws in that strip is shown under the bar,
    /// frosted. Elsewhere nothing changes.
    fn under_bar(&self) -> bool {
        false
    }

    /// The window's kind and size limits ([`Style`]); the default is a normal
    /// window with none.
    fn style(&self) -> Style {
        Style::default()
    }

    /// The bar's description, when it has changed since it was last asked:
    /// asked after each frame when the server draws the bar. None: as before.
    fn bar(&mut self) -> Option<Bar> {
        None
    }

    /// Something was done with a tab the server drew.
    fn tab(&mut self, _event: TabEvent, _window: &mut Window) {}

    /// One of the window's own bar buttons ([`Bar::Tools`], the server draws
    /// them) was pressed: the id the window gave it.
    fn bar_button(&mut self, _id: u32, _window: &mut Window) {}

    /// The window's app menu (app settings milestone 1), when it has changed
    /// since it was last asked: whether the app is a terminal (its
    /// Ctrl+letter keys are its programs', so roles' keys take Shift), and
    /// its commands. Asked after each frame on the native road; the server
    /// draws the menu from its app button.
    fn commands(&mut self) -> Option<(bool, Vec<AppCommand>)> {
        None
    }

    /// One of the window's commands was chosen in its app menu: its id.
    fn command(&mut self, _id: u32, _window: &mut Window) {}

    /// A drag is over the window at (x, y) with these kinds: what a drop
    /// would do ([`DROP_NONE`], [`DROP_COPY`], [`DROP_MOVE`]).
    fn drag_over(&mut self, _x: f32, _y: f32, _kinds: &[String], _ctrl: bool) -> u8 {
        DROP_NONE
    }

    /// The drag left the window.
    fn drag_left(&mut self) {}

    /// Dropped here, as this window said it would take it: the data.
    fn dropped(&mut self, _x: f32, _y: f32, _action: u8, _items: &[(String, Vec<u8>)], _window: &mut Window) {}

    /// A drag this window started ended: with what action ([`DROP_NONE`]: nowhere took it).
    fn drag_ended(&mut self, _action: u8, _window: &mut Window) {}

    /// The style the window's bar is to be drawn in, when it has changed
    /// since it was last asked (app settings milestone 1).
    fn bar_style(&mut self) -> Option<BarStyle> {
        None
    }

    /// The window's meaning (Contract 3), when it has changed since it was
    /// last asked: asked after each frame on the native road. None: as before.
    fn meaning(&mut self) -> Option<MeaningTree> {
        None
    }
}

/// Sets, moves or (with no time) forgets the timer `id`.
fn set_timer(timers: &mut Vec<(u64, Instant)>, id: u64, at: Option<Instant>) {
    timers.retain(|t| t.0 != id);
    if let Some(at) = at {
        timers.push((id, at));
        if timers.len() > MAX_TIMERS {
            timers.sort_by_key(|t| t.1);
            timers.truncate(MAX_TIMERS);
        }
    }
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// A picture the server can show: shared memory and the buffer made of it.
struct Buffer {
    file: ShmFile,
    id: u32,
    width: u32,
    height: u32,
    /// Made to be seen through (its format has alpha).
    alpha: bool,
    /// The server is showing it, or may yet: not to be drawn into.
    busy: bool,
}

/// A buffer of GPU memory and the server's name for it.
struct Shared {
    buffer: GpuBuffer,
    id: u32,
    busy: bool,
}

/// GPU buffers kept for a window: one showing, one being drawn, one to spare.
const GPU_BUFFERS: usize = 3;

/// Clipboard text on its way in: the pipe the other program writes to.
struct Paste {
    pipe: std::fs::File,
    data: Vec<u8>,
    started: Instant,
}

/// A held key that repeats, and when it does next.
struct Repeat {
    key: u32,
    next: Instant,
}

use native::Run;

/// One window on the Wayland road (bar milestone 1, step 4b: a program may
/// have several): the program that runs it, its objects and what the road
/// keeps for it. What the windows share (the seat, the keyboard, the
/// clipboard, the GPU, the fonts) is the [`Runner`]'s.
struct WlWin<'a> {
    /// The toolkit's number for it ([`App::opened`]).
    id: u32,
    app: Run<'a>,
    surface: u32,
    xdg_surface: u32,
    toplevel: u32,
    viewport: Option<u32>,
    /// The fractional-scale object (its PreferredScale events name it).
    scale_object: Option<u32>,
    decoration: Option<u32>,
    /// GPU buffers the server has taken, and those it has been asked to take.
    shared: Vec<Shared>,
    asked: Vec<(u32, GpuBuffer)>,
    drawn_by: &'static str,
    started: Instant,
    width: u32,
    height: u32,
    suggested: Option<(u32, u32)>,
    scale120: u32,
    configured: bool,
    dirty: bool,
    frame_callback: Option<u32>,
    frames: u64,
    buffers: Vec<Buffer>,
    closing: bool,
    /// Where the pointer is, if it is over this window.
    pointer_at: Option<(f32, f32)>,
    cursor: Cursor,
    /// The fingers that are down on it, and where each last was.
    fingers: Vec<(i32, f32, f32)>,
    /// The bar is ours to draw: the program leaves its frame to the server
    /// ([`App::own_frame`] false) and the server draws none (no decoration
    /// protocol, as GNOME's; or it answered "client side"). The program's
    /// scene and input are then below [`bar::BAR_H`].
    frame: bool,
    own_frame: bool,
    /// The server draws no border either: ours, and the band inside it
    /// that resizes the window ([`vergence_look::EDGE`], [`vergence_look::RESIZE_BAND`]).
    edges: bool,
    /// The edges the pointer is on, in that band.
    on_edge: Option<Edges>,
    /// What the pointer is over in our bar, and the button pressed there.
    bar_hover: Option<bar::Part>,
    bar_pressed: Option<bar::Part>,
    /// The pointer is on our bar (the program was told it left).
    in_bar: bool,
    /// The server's word: the window is active, maximized (our bar's look and its button).
    active: bool,
    maximized: bool,
    /// The program's timers: its id for each, and when it is due.
    timers: Vec<(u64, Instant)>,
}

impl WlWin<'_> {
    /// Our bar's height, when we draw one.
    fn bar_h(&self) -> f32 {
        if self.frame { bar::BAR_H } else { 0.0 }
    }

    /// A point of the window as the program knows it (below our bar), or
    /// None if it is on our bar.
    fn to_app(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let h = self.bar_h();
        (y >= h).then_some((x, y - h))
    }

    /// The edges of the window (x, y) is within the resizing band of, when
    /// we draw the border; none while maximized.
    fn edge_at(&self, x: f32, y: f32) -> Option<Edges> {
        if !self.edges || self.maximized {
            return None;
        }
        let band = vergence_look::RESIZE_BAND;
        let (w, h) = (self.width as f32, self.height as f32);
        let e = Edges { top: y < band, bottom: y >= h - band, left: x < band, right: x >= w - band };
        (e.top || e.bottom || e.left || e.right).then_some(e)
    }
}

/// The connection and what a program's windows share.
struct Runner<'a> {
    client: Client,
    shm: u32,
    wm_base: u32,
    compositor: u32,
    viewporter: Option<u32>,
    scale_manager: Option<u32>,
    decoration_manager: Option<u32>,
    /// The server offered no decoration protocol: a window leaving its
    /// frame to the server gets none, so we draw it.
    undecorated: bool,
    seat: Option<u32>,
    cursor_shape_manager: Option<u32>,
    pointer: Option<u32>,
    touch: Option<u32>,
    keyboard_object: Option<u32>,
    cursor_device: Option<u32>,
    data_manager: Option<u32>,
    data_device: Option<u32>,
    dmabuf: Option<u32>,

    /// The GPU renderer, while it and the server's taking of its buffers work.
    gpu: Option<Gpu>,
    cpu: Cpu,
    fonts: Fonts,

    windows: Vec<WlWin<'a>>,
    next_window: u32,
    /// The window the pointer is over, the one with the keyboard, and the
    /// window each finger came down on.
    pointer_window: Option<u32>,
    keyboard_window: Option<u32>,
    finger_windows: Vec<(i32, u32)>,

    /// The serial of the pointer's entry (to set its shape) and of the last
    /// button press (to start a move or a resize).
    enter_serial: u32,
    press_serial: u32,
    keyboard: Option<keyboard::Keyboard>,
    /// Repeats a second, and milliseconds before the first.
    repeat_rate: i32,
    repeat_delay: i32,
    repeat: Option<Repeat>,

    /// The serial of the last key or button press (to set the clipboard).
    input_serial: u32,
    /// What is on offer, by the kinds of data each can give; which of them
    /// is the clipboard; and what we put there ourselves.
    offers: HashMap<u32, Vec<String>>,
    selection: Option<u32>,
    copied: Option<(u32, Arc<str>)>,
    paste: Option<Paste>,
    /// The window that asked for the paste on its way in.
    paste_window: u32,
    /// Input made here rather than read from the server, for a window, to give it next.
    inbox: Vec<(u32, Input)>,
    /// Our bar's buttons, left and right ([`bar::desktop_layout`]), and
    /// the flag the settings watcher raises when the layout may have changed.
    buttons: (Vec<bar::WinButton>, Vec<bar::WinButton>),
    /// Light or dark for that bar and the border ([`bar::desktop_light`]).
    light: bool,
    layout_changed: Arc<AtomicBool>,
    /// The pipe other threads wake the program through, its flag, and a
    /// waker to hand each window the program opens.
    wake: std::fs::File,
    wake_pending: Arc<AtomicBool>,
    waker: Waker,
}

/// Whether the server offered no decoration protocol, so that a window
/// leaving its frame to the server would get none: true means we draw it.
fn decoration_manager_offered(offered: &[(Interface, u32, u32)]) -> bool {
    !offered.iter().any(|o| o.0 == Interface::DecorationManager)
}

/// Our bar's title size (the settings app's interface text).
const BAR_FONT_SIZE: f32 = bar::FONT_SIZE;

/// Opens the window and runs until the program's last window is closed.
/// `size` is the size asked for, in reference units; the server may give
/// another. Windows the program opens later ([`Window::open_window`]) run
/// on the same connection.
pub fn run(app: &mut dyn App, size: (u32, u32)) -> io::Result<()> {
    // The native road first, in a Vergence session; it returns before
    // telling the program anything if the window is to be opened over Wayland.
    if native::run(app, size)?.is_some() {
        return Ok(());
    }
    let mut client = Client::connect()?;
    let registry = client.get_registry()?;
    let ready = client.sync()?;

    // What the server offers, of what a window needs.
    let wanted = [
        Interface::Compositor,
        Interface::Shm,
        Interface::WmBase,
        Interface::Viewporter,
        Interface::FractionalScaleManager,
        Interface::DecorationManager,
        Interface::Seat,
        Interface::CursorShapeManager,
        Interface::DataDeviceManager,
        Interface::Dmabuf,
    ];
    let mut offered: Vec<(Interface, u32, u32)> = Vec::new();
    'listing: loop {
        for event in client.dispatch()? {
            match event {
                Event::Global { name, interface, version } => {
                    if let Some(i) = wanted.iter().find(|i| i.global_name() == Some(interface.as_str()))
                        && !offered.iter().any(|o| o.0 == *i)
                    {
                        offered.push((*i, name, version));
                    }
                }
                Event::Done { callback, .. } if callback == ready => break 'listing,
                Event::Error { message, .. } => return Err(other(message)),
                _ => {}
            }
        }
    }
    let bind = |client: &mut Client, interface: Interface| -> io::Result<Option<u32>> {
        match offered.iter().find(|o| o.0 == interface) {
            Some((_, name, version)) => client.bind(registry, *name, interface, *version).map(Some),
            None => Ok(None),
        }
    };
    let undecorated = decoration_manager_offered(&offered);
    let need = |id: Option<u32>, what: &str| id.ok_or_else(|| other(format!("the display server has no {what}")));
    let compositor = need(bind(&mut client, Interface::Compositor)?, "wl_compositor")?;
    let shm = need(bind(&mut client, Interface::Shm)?, "wl_shm")?;
    let wm_base = need(bind(&mut client, Interface::WmBase)?, "xdg_wm_base")?;
    let viewporter = bind(&mut client, Interface::Viewporter)?;
    let scale_manager = bind(&mut client, Interface::FractionalScaleManager)?;
    let decoration_manager = bind(&mut client, Interface::DecorationManager)?;
    let seat = bind(&mut client, Interface::Seat)?;
    let cursor_shape_manager = bind(&mut client, Interface::CursorShapeManager)?;
    let data_manager = bind(&mut client, Interface::DataDeviceManager)?;
    let dmabuf = bind(&mut client, Interface::Dmabuf)?;
    // The GPU renderer, if the server takes GPU buffers, the machine has a
    // GPU that makes them, and nobody asked for the CPU.
    let gpu = dmabuf
        .filter(|_| std::env::var("VERGENCE_RENDER").as_deref() != Ok("cpu"))
        .and_then(|_| Gpu::new().ok())
        .filter(Gpu::shares_buffers);
    let data_device = match (data_manager, seat) {
        (Some(m), Some(s)) => Some(client.get_data_device(m, s)?),
        _ => None,
    };

    let (wake_read, wake_write) = vergence_wl::conn::pipe()?;
    let wake_pending = Arc::new(AtomicBool::new(false));
    let wake_pipe: Arc<std::fs::File> = Arc::new(wake_write.into());
    let waker = Waker { pipe: wake_pipe.clone(), pending: wake_pending.clone() };
    app.waker(waker.clone());
    // The settings file, looked at once a second as the terminal does: a
    // bar of ours follows the button layout the moment it is changed (the
    // settings app's own window included, which stays in front meanwhile).
    let layout_changed = Arc::new(AtomicBool::new(false));
    if !app.own_frame() {
        let (changed, waker) = (layout_changed.clone(), Waker { pipe: wake_pipe, pending: wake_pending.clone() });
        // Vergence's settings and desktop files' times, and GNOME's layout and
        // light or dark when running there.
        let time = |p: Option<std::path::PathBuf>| p.and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
        let stamp = move || (time(bar::settings_path()), time(bar::desktop_path()), bar::host_layout(), bar::host_light());
        std::thread::Builder::new().name("settings".into()).spawn(move || {
            let mut seen = stamp();
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let now = stamp();
                if now != seen {
                    seen = now;
                    changed.store(true, Ordering::SeqCst);
                    waker.wake();
                }
            }
        })?;
    }

    let mut runner = Runner {
        client,
        shm,
        wm_base,
        compositor,
        viewporter,
        scale_manager,
        decoration_manager,
        undecorated,
        seat,
        cursor_shape_manager,
        pointer: None,
        touch: None,
        keyboard_object: None,
        cursor_device: None,
        data_manager,
        data_device,
        dmabuf,
        gpu,
        cpu: Cpu::new(),
        fonts: Fonts::system(),
        windows: Vec::new(),
        next_window: 2,
        pointer_window: None,
        keyboard_window: None,
        finger_windows: Vec::new(),
        enter_serial: 0,
        press_serial: 0,
        keyboard: None,
        repeat_rate: 0,
        repeat_delay: 0,
        repeat: None,
        input_serial: 0,
        offers: HashMap::new(),
        selection: None,
        copied: None,
        paste: None,
        paste_window: 1,
        inbox: Vec::new(),
        buttons: bar::desktop_layout(),
        light: bar::desktop_light(),
        layout_changed,
        wake: wake_read.into(),
        wake_pending,
        waker,
    };
    app.opened(1);
    runner.add_window(1, Run::Borrowed(app), size)?;
    runner.run()
}

impl<'a> Runner<'a> {
    fn index(&self, id: u32) -> Option<usize> {
        self.windows.iter().position(|w| w.id == id)
    }

    /// The window whose object this is (its surface, xdg_surface, toplevel,
    /// decoration or fractional-scale object).
    fn by_object(&self, object: u32) -> Option<usize> {
        self.windows.iter().position(|w| {
            w.surface == object || w.xdg_surface == object || w.toplevel == object || w.decoration == Some(object) || w.scale_object == Some(object)
        })
    }

    fn id_of_surface(&self, surface: u32) -> Option<u32> {
        self.windows.iter().find(|w| w.surface == surface).map(|w| w.id)
    }

    /// Asks the server for a window run by `app`: its surface, role, title,
    /// decoration and scale objects; the first commit has no buffer, it asks
    /// the server to configure the window.
    fn add_window(&mut self, id: u32, mut app: Run<'a>, size: (u32, u32)) -> io::Result<()> {
        let a = app.get();
        let own_frame = a.own_frame();
        // Our bar when the server will draw none (it may still say so later),
        // and our border for any window then.
        let frame = !own_frame && self.undecorated;
        let surface = self.client.create_surface(self.compositor)?;
        let xdg_surface = self.client.get_xdg_surface(self.wm_base, surface)?;
        let toplevel = self.client.get_toplevel(xdg_surface)?;
        self.client.set_title(toplevel, &a.title())?;
        self.client.set_app_id(toplevel, &a.app_id())?;
        let decoration = match self.decoration_manager {
            Some(manager) => {
                let d = self.client.get_decoration(manager, toplevel)?;
                self.client.set_decoration_mode(d, if own_frame { wl::DECORATION_CLIENT } else { wl::DECORATION_SERVER })?;
                Some(d)
            }
            None => None,
        };
        // The exact scale needs both: the server says the scale, and the viewport
        // says the buffer of that many pixels is still the window's size in units.
        let (viewport, scale_object) = match (self.viewporter, self.scale_manager) {
            (Some(v), Some(m)) => {
                let s = self.client.get_fractional_scale(m, surface)?;
                (Some(self.client.get_viewport(v, surface)?), Some(s))
            }
            _ => (None, None),
        };
        self.client.commit(surface)?;
        self.windows.push(WlWin {
            id,
            app,
            surface,
            xdg_surface,
            toplevel,
            viewport,
            scale_object,
            decoration,
            shared: Vec::new(),
            asked: Vec::new(),
            drawn_by: "",
            started: Instant::now(),
            width: size.0.max(1),
            height: size.1.max(1) + if frame { bar::BAR_H as u32 } else { 0 },
            suggested: None,
            scale120: 120,
            configured: false,
            dirty: false,
            frame_callback: None,
            frames: 0,
            buffers: Vec::new(),
            closing: false,
            pointer_at: None,
            cursor: Cursor::Default,
            fingers: Vec::new(),
            frame,
            own_frame,
            edges: self.undecorated,
            on_edge: None,
            bar_hover: None,
            bar_pressed: None,
            in_bar: false,
            active: true,
            maximized: false,
            timers: Vec::new(),
        });
        Ok(())
    }

    /// Another window of the program, opened by one of its windows.
    fn open(&mut self, mut app: Box<dyn App>) -> io::Result<()> {
        let id = self.next_window;
        self.next_window += 1;
        let size = app.opening_size().map_or(native::OPENED_SIZE, |(w, h)| (w.clamp(64, 16_384), h.clamp(48, 16_384)));
        app.opened(id);
        app.waker(self.waker.clone());
        self.add_window(id, Run::Owned(app), size)
    }

    /// A window is gone: its program told, its buffers let go, its objects destroyed.
    fn release(&mut self, mut w: WlWin<'a>) -> io::Result<()> {
        w.app.get().closed();
        for b in &w.buffers {
            self.client.destroy_buffer(b.id)?;
        }
        if let Some(gpu) = self.gpu.as_mut() {
            for s in w.shared {
                self.client.destroy_buffer(s.id)?;
                gpu.release(s.buffer);
            }
            for (_, b) in w.asked {
                gpu.release(b);
            }
        }
        if let Some(d) = w.decoration {
            self.client.destroy_object(d)?;
        }
        self.client.destroy_object(w.toplevel)?;
        self.client.destroy_object(w.xdg_surface)?;
        for o in [w.viewport, w.scale_object].into_iter().flatten() {
            self.client.destroy_object(o)?;
        }
        self.client.destroy_object(w.surface)?;
        if self.pointer_window == Some(w.id) {
            self.pointer_window = None;
        }
        if self.keyboard_window == Some(w.id) {
            self.keyboard_window = None;
            self.repeat = None;
        }
        self.finger_windows.retain(|f| f.1 != w.id);
        self.client.flush()
    }

    fn run(&mut self) -> io::Result<()> {
        loop {
            // Closed windows go; the program ends with its last.
            let mut i = 0;
            while i < self.windows.len() {
                if self.windows[i].closing {
                    let w = self.windows.remove(i);
                    self.release(w)?;
                } else {
                    i += 1;
                }
            }
            if self.windows.is_empty() {
                return Ok(());
            }
            // Wait for the server, but no longer than the next key repeat, the
            // paste's limit or the next timer.
            let now = Instant::now();
            let repeat = self.repeat.as_ref().map(|r| r.next.saturating_duration_since(now));
            let paste = self.paste.as_ref().map(|p| (p.started + PASTE_TIMEOUT).saturating_duration_since(now));
            let timer = self.windows.iter().flat_map(|w| w.timers.iter().map(|t| t.1)).min().map(|t| t.saturating_duration_since(now));
            let wait = [repeat, paste, timer].into_iter().flatten().min();
            let pipe = self.paste.as_ref().map(|p| p.pipe.as_raw_fd());
            let (events, [pipe_ready, woken]) = self.client.dispatch_or([pipe, Some(self.wake.as_raw_fd())], wait)?;
            for event in events {
                self.handle(event)?;
            }
            self.read_paste(pipe_ready);
            if woken {
                // The flag is cleared first: a wake that comes while the program is being told is not lost.
                let mut drain = [0u8; 64];
                let _ = self.wake.read(&mut drain);
                self.wake_pending.store(false, Ordering::SeqCst);
                if self.layout_changed.swap(false, Ordering::SeqCst) {
                    let (buttons, light) = (bar::desktop_layout(), bar::desktop_light());
                    if buttons != self.buttons || light != self.light {
                        self.buttons = buttons;
                        self.light = light;
                        for w in &mut self.windows {
                            w.dirty |= w.frame;
                        }
                    }
                }
                for id in self.windows.iter().map(|w| w.id).collect::<Vec<_>>() {
                    self.with_window(id, |app, window| app.woken(window))?;
                }
            }
            for (id, input) in std::mem::take(&mut self.inbox) {
                self.tell(id, input)?;
            }
            self.repeat_due()?;
            self.timers_due()?;
            for i in 0..self.windows.len() {
                let w = &self.windows[i];
                if w.configured && w.dirty && w.frame_callback.is_none() && !w.closing {
                    self.windows[i].dirty = false;
                    self.draw(i)?;
                }
            }
        }
    }

    /// Gives window `id`'s program one input and does what it asks in return.
    fn tell(&mut self, id: u32, input: Input) -> io::Result<()> {
        self.with_window(id, |app, window| app.input(&input, window))
    }

    /// Calls window `id`'s program with a handle on its window, then does what it asked.
    /// The user asked to close a window: closed if its program agrees
    /// ([`App::close_asked`]).
    fn ask_close(&mut self, id: u32) -> io::Result<()> {
        let mut yes = false;
        self.with_window(id, |app, w| yes = app.close_asked(w))?;
        if yes && let Some(i) = self.index(id) {
            self.windows[i].closing = true;
        }
        Ok(())
    }

    fn with_window(&mut self, id: u32, call: impl FnOnce(&mut dyn App, &mut Window)) -> io::Result<()> {
        let Some(i) = self.index(id) else { return Ok(()) };
        let mut window = Window::default();
        call(self.windows[i].app.get(), &mut window);
        for command in window.commands {
            let Some(i) = self.index(id) else { break };
            let toplevel = self.windows[i].toplevel;
            match command {
                Command::Redraw => self.windows[i].dirty = true,
                Command::Close => self.windows[i].closing = true,
                Command::Move => {
                    if let Some(seat) = self.seat {
                        self.client.start_move(toplevel, seat, self.press_serial)?;
                    }
                }
                Command::Resize(edges) => {
                    if let Some(seat) = self.seat {
                        self.client.start_resize(toplevel, seat, self.press_serial, edges.wire())?;
                    }
                }
                Command::Cursor(cursor) => {
                    let w = &mut self.windows[i];
                    if cursor != w.cursor {
                        w.cursor = cursor;
                        if let (Some(device), Some(_), false) = (self.cursor_device, w.pointer_at, w.in_bar) {
                            self.client.set_cursor_shape(device, self.enter_serial, cursor.wire())?;
                        }
                    }
                }
                Command::Title(title) => self.client.set_title(toplevel, &title)?,
                Command::Maximize(on) => self.client.set_maximized(toplevel, on)?,
                // Wayland has no size request; the user resizes.
                Command::Size(..) => {}
                // Drags under other desktops: Wayland drag and drop, later;
                // activation: xdg-activation, later.
                Command::Drag(..) | Command::Activate | Command::ContextMenu(..) | Command::Menu(..) => {}
                Command::Minimize => self.client.set_minimized(toplevel)?,
                Command::Copy(text) => {
                    if let (Some(manager), Some(device)) = (self.data_manager, self.data_device) {
                        if let Some((old, _)) = self.copied.take() {
                            self.client.destroy_data_source(old)?;
                        }
                        let source = self.client.create_data_source(manager, &TEXT_KINDS)?;
                        self.client.set_selection(device, source, self.input_serial)?;
                        self.copied = Some((source, text.into()));
                    }
                }
                Command::Paste => self.start_paste(id)?,
                Command::Timer(timer, at) => set_timer(&mut self.windows[i].timers, timer, at),
                // Wayland lets no client place its window.
                Command::Place(_) => {}
                Command::Open(app) => self.open(app)?,
            }
        }
        Ok(())
    }

    /// Asks whoever owns the clipboard for its text, for window `id`.
    fn start_paste(&mut self, id: u32) -> io::Result<()> {
        // Our own copy needs no trip through the server (and a program
        // cannot wait on itself).
        if let Some((_, text)) = &self.copied {
            self.inbox.push((id, Input::Paste(text.to_string())));
            return Ok(());
        }
        let Some(offer) = self.selection else { return Ok(()) };
        let Some(kind) = self.offers.get(&offer).and_then(|kinds| TEXT_KINDS.iter().find(|k| kinds.iter().any(|have| have == *k))) else { return Ok(()) };
        let (read_end, write_end) = vergence_wl::conn::pipe()?;
        self.client.receive(offer, kind, write_end.as_raw_fd())?;
        // Sent now, so that our copy of the writing end can be closed: the
        // pipe then ends when the other program has finished writing.
        self.client.flush()?;
        drop(write_end);
        self.paste = Some(Paste { pipe: read_end.into(), data: Vec::new(), started: Instant::now() });
        self.paste_window = id;
        Ok(())
    }

    /// Takes what has arrived of a paste; when it is all here (or too much,
    /// or too late), gives it to the window that asked.
    fn read_paste(&mut self, ready: bool) {
        let Some(paste) = &mut self.paste else { return };
        let mut done = paste.started.elapsed() >= PASTE_TIMEOUT;
        if ready {
            let mut chunk = [0u8; 64 * 1024];
            // The pipe is readable, so this returns at once: some bytes, or the end.
            match paste.pipe.read(&mut chunk) {
                Ok(0) | Err(_) => done = true,
                Ok(n) => {
                    paste.data.extend_from_slice(&chunk[..n]);
                    done |= paste.data.len() >= MAX_PASTE;
                }
            }
        }
        if done && let Some(mut paste) = self.paste.take() {
            paste.data.truncate(MAX_PASTE);
            if !paste.data.is_empty() {
                self.inbox.push((self.paste_window, Input::Paste(String::from_utf8_lossy(&paste.data).into_owned())));
            }
        }
    }

    fn modifiers(&self) -> Modifiers {
        self.keyboard.as_ref().map(keyboard::Keyboard::modifiers).unwrap_or_default()
    }

    fn handle(&mut self, event: Event) -> io::Result<()> {
        match event {
            Event::Ping { serial } => self.client.pong(self.wm_base, serial)?,
            Event::ToplevelConfigure { toplevel, width, height, states } => {
                let Some(i) = self.by_object(toplevel) else { return Ok(()) };
                let w = &mut self.windows[i];
                if width > 0 && height > 0 {
                    w.suggested = Some((width as u32, height as u32));
                }
                let (active, maximized) = (states.contains(&wl::STATE_ACTIVATED), states.contains(&wl::STATE_MAXIMIZED));
                if (w.frame || w.edges) && (active, maximized) != (w.active, w.maximized) {
                    w.dirty = true;
                }
                (w.active, w.maximized) = (active, maximized);
            }
            Event::DecorationMode { decoration, mode } => {
                let Some(i) = self.by_object(decoration) else { return Ok(()) };
                let w = &mut self.windows[i];
                // The server's answer to our request: it may draw no bar after all.
                let frame = !w.own_frame && mode == wl::DECORATION_CLIENT;
                if frame != w.frame {
                    let bar_h = bar::BAR_H as u32;
                    w.height = if frame { w.height + bar_h } else { w.height.saturating_sub(bar_h).max(1) };
                    w.frame = frame;
                    w.edges = frame;
                    w.dirty = true;
                }
            }
            Event::Configure { xdg_surface, serial } => {
                let Some(i) = self.by_object(xdg_surface) else { return Ok(()) };
                self.client.ack_configure(xdg_surface, serial)?;
                let w = &mut self.windows[i];
                if let Some(size) = w.suggested.take() {
                    (w.width, w.height) = size;
                }
                w.configured = true;
                w.dirty = true;
            }
            Event::PreferredScale { object, scale120 } => {
                let Some(i) = self.by_object(object) else { return Ok(()) };
                let w = &mut self.windows[i];
                if w.viewport.is_some() && scale120 > 0 && scale120 != w.scale120 {
                    w.scale120 = scale120;
                    w.dirty = true;
                }
            }
            Event::BufferRelease { buffer } => {
                for w in &mut self.windows {
                    if let Some(b) = w.buffers.iter_mut().find(|b| b.id == buffer) {
                        b.busy = false;
                    }
                    if let Some(b) = w.shared.iter_mut().find(|b| b.id == buffer) {
                        b.busy = false;
                    }
                }
            }
            Event::DmabufCreated { params, buffer } => {
                self.client.destroy_dmabuf_params(params)?;
                match self.windows.iter().position(|w| w.asked.iter().any(|(p, _)| *p == params)) {
                    Some(i) => {
                        let w = &mut self.windows[i];
                        let at = w.asked.iter().position(|(p, _)| *p == params).unwrap_or(0);
                        let (_, made) = w.asked.swap_remove(at);
                        w.shared.push(Shared { buffer: made, id: buffer, busy: false });
                        // A frame drawn in software meanwhile is drawn again on the GPU.
                        w.dirty = true;
                    }
                    None => self.client.destroy_buffer(buffer)?,
                }
            }
            Event::DmabufFailed { params } => {
                // The server does not take this GPU's buffers: software from here on.
                self.client.destroy_dmabuf_params(params)?;
                self.give_up_gpu()?;
            }
            Event::Done { callback, .. } => {
                if let Some(i) = self.windows.iter().position(|w| w.frame_callback == Some(callback)) {
                    self.windows[i].frame_callback = None;
                    if self.windows[i].app.get().animating() {
                        self.windows[i].dirty = true;
                    }
                }
            }
            Event::Close { toplevel } => {
                if let Some(i) = self.by_object(toplevel) {
                    self.ask_close(self.windows[i].id)?;
                }
            }
            Event::Error { object, code, message } => return Err(other(format!("the display server refused: {message} (object {object}, code {code})"))),

            Event::SeatCapabilities { seat, capabilities } => {
                if capabilities & wl::SEAT_POINTER != 0 && self.pointer.is_none() {
                    let pointer = self.client.get_pointer(seat)?;
                    self.pointer = Some(pointer);
                    if let Some(manager) = self.cursor_shape_manager {
                        self.cursor_device = Some(self.client.get_cursor_shape_device(manager, pointer)?);
                    }
                }
                if capabilities & wl::SEAT_TOUCH != 0 && self.touch.is_none() {
                    self.touch = Some(self.client.get_touch(seat)?);
                }
                if capabilities & wl::SEAT_KEYBOARD != 0 && self.keyboard_object.is_none() {
                    self.keyboard_object = Some(self.client.get_keyboard(seat)?);
                }
            }
            Event::PointerEnter { serial, surface, x, y } => {
                let Some(id) = self.id_of_surface(surface) else { return Ok(()) };
                let Some(i) = self.index(id) else { return Ok(()) };
                self.pointer_window = Some(id);
                self.enter_serial = serial;
                self.windows[i].pointer_at = Some((x as f32, y as f32));
                // A pointer that enters has no shape until it is given one.
                if let Some(device) = self.cursor_device {
                    self.client.set_cursor_shape(device, serial, self.windows[i].cursor.wire())?;
                }
                self.windows[i].in_bar = false;
                self.pointer_moved(id, x as f32, y as f32)?;
            }
            Event::PointerLeave { surface, .. } => {
                let Some(id) = self.id_of_surface(surface).or(self.pointer_window) else { return Ok(()) };
                if self.pointer_window == Some(id) {
                    self.pointer_window = None;
                }
                let Some(i) = self.index(id) else { return Ok(()) };
                let w = &mut self.windows[i];
                w.pointer_at = None;
                if w.bar_hover.take().is_some() {
                    w.dirty = true;
                }
                w.bar_pressed = None;
                w.on_edge = None;
                if !std::mem::take(&mut w.in_bar) {
                    self.tell(id, Input::PointerLeft)?;
                }
            }
            Event::PointerMotion { x, y, .. } => {
                if let Some(id) = self.pointer_window {
                    self.pointer_moved(id, x as f32, y as f32)?;
                }
            }
            Event::PointerButton { serial, button, pressed, .. } => {
                if pressed {
                    self.press_serial = serial;
                    self.input_serial = serial;
                }
                let Some(id) = self.pointer_window else { return Ok(()) };
                let Some(i) = self.index(id) else { return Ok(()) };
                let button = match button {
                    0x110 => Button::Left,
                    0x111 => Button::Right,
                    0x112 => Button::Middle,
                    other => Button::Other(other),
                };
                let (x, y) = self.windows[i].pointer_at.unwrap_or_default();
                if let Some(edges) = self.windows[i].on_edge {
                    if pressed && button == Button::Left && let Some(seat) = self.seat {
                        self.client.start_resize(self.windows[i].toplevel, seat, serial, edges.wire())?;
                    }
                    return Ok(());
                }
                match self.windows[i].to_app(x, y) {
                    Some((x, y)) => {
                        let modifiers = self.modifiers();
                        self.tell(id, Input::Button { button, pressed, x, y, modifiers })?;
                    }
                    None if button == Button::Left => self.bar_button(id, x, y, pressed)?,
                    None => {}
                }
            }
            Event::TouchDown { serial, surface, id: finger, x, y, .. } => {
                // A finger's press is as good as a button's for what needs one
                // (the clipboard, moving the window).
                self.press_serial = serial;
                self.input_serial = serial;
                let Some(id) = self.id_of_surface(surface) else { return Ok(()) };
                let Some(i) = self.index(id) else { return Ok(()) };
                let Some((x, y)) = self.windows[i].to_app(x as f32, y as f32) else {
                    // A finger on our bar: a button at once, else the window moves.
                    self.bar_button(id, x as f32, y as f32, true)?;
                    return self.bar_button(id, x as f32, y as f32, false);
                };
                self.finger_windows.retain(|f| f.0 != finger);
                self.finger_windows.push((finger, id));
                let w = &mut self.windows[i];
                w.fingers.retain(|f| f.0 != finger);
                if w.fingers.len() < 16 {
                    w.fingers.push((finger, x, y));
                }
                self.tell(id, Input::Touch { id: finger, phase: touch::Phase::Down, x, y })?;
            }
            Event::TouchMotion { id: finger, x, y, .. } => {
                let Some(id) = self.finger_windows.iter().find(|f| f.0 == finger).map(|f| f.1) else { return Ok(()) };
                let Some(i) = self.index(id) else { return Ok(()) };
                let w = &mut self.windows[i];
                let (x, y) = (x as f32, y as f32 - w.bar_h());
                if let Some(f) = w.fingers.iter_mut().find(|f| f.0 == finger) {
                    (f.1, f.2) = (x, y);
                    self.tell(id, Input::Touch { id: finger, phase: touch::Phase::Moved, x, y })?;
                }
            }
            Event::TouchUp { id: finger, .. } => {
                let Some(pos) = self.finger_windows.iter().position(|f| f.0 == finger) else { return Ok(()) };
                let (_, id) = self.finger_windows.remove(pos);
                let Some(i) = self.index(id) else { return Ok(()) };
                let w = &mut self.windows[i];
                if let Some(index) = w.fingers.iter().position(|f| f.0 == finger) {
                    let (_, x, y) = w.fingers.remove(index);
                    self.tell(id, Input::Touch { id: finger, phase: touch::Phase::Up, x, y })?;
                }
            }
            Event::TouchCancel => {
                self.finger_windows.clear();
                for i in 0..self.windows.len() {
                    let id = self.windows[i].id;
                    for (finger, x, y) in std::mem::take(&mut self.windows[i].fingers) {
                        self.tell(id, Input::Touch { id: finger, phase: touch::Phase::Cancelled, x, y })?;
                    }
                }
            }
            Event::TouchFrame => {}
            Event::PointerAxis { axis, value, .. } => {
                let Some(id) = self.pointer_window else { return Ok(()) };
                let Some(i) = self.index(id) else { return Ok(()) };
                let (x, y) = self.windows[i].pointer_at.unwrap_or_default();
                let Some((x, y)) = self.windows[i].to_app(x, y) else { return Ok(()) };
                let (dx, dy) = if axis == 0 { (0.0, value as f32) } else { (value as f32, 0.0) };
                self.tell(id, Input::Scroll { x, y, dx, dy })?;
            }
            Event::Keymap { format, size } => {
                // The file comes with the event whatever its format: take it either way.
                let file = self.client.take_fd();
                if format == 1
                    && let Some(file) = file
                    && let Ok(text) = read_shared(&file, size as usize, MAX_KEYMAP)
                {
                    self.keyboard = keyboard::Keyboard::from_text(&text);
                }
            }
            Event::RepeatInfo { rate, delay } => (self.repeat_rate, self.repeat_delay) = (rate, delay),
            Event::KeyboardEnter { surface, .. } => {
                let Some(id) = self.id_of_surface(surface) else { return Ok(()) };
                self.keyboard_window = Some(id);
                self.tell(id, Input::Focus(true))?;
            }
            Event::KeyboardLeave { surface, .. } => {
                self.repeat = None;
                if let Some(k) = &mut self.keyboard {
                    k.reset_compose();
                }
                let Some(id) = self.id_of_surface(surface).or(self.keyboard_window) else { return Ok(()) };
                if self.keyboard_window == Some(id) {
                    self.keyboard_window = None;
                }
                self.tell(id, Input::Focus(false))?;
            }
            Event::Modifiers { depressed, latched, locked, group } => {
                if let Some(k) = &mut self.keyboard {
                    k.update(depressed, latched, locked, group);
                }
            }
            Event::Key { serial, key, pressed, .. } => {
                if pressed {
                    self.input_serial = serial;
                    let repeats = self.repeat_rate > 0 && self.keyboard.as_ref().is_some_and(|k| k.repeats(key));
                    self.repeat = repeats.then(|| Repeat { key, next: Instant::now() + Duration::from_millis(self.repeat_delay.max(0) as u64) });
                } else if self.repeat.as_ref().is_some_and(|r| r.key == key) {
                    self.repeat = None;
                }
                self.key(key, pressed, false)?;
            }
            Event::DataOffer { offer } => {
                self.offers.insert(offer, Vec::new());
            }
            Event::OfferKind { offer, mime } => {
                if let Some(kinds) = self.offers.get_mut(&offer)
                    && kinds.len() < MAX_KINDS
                    && mime.len() <= MAX_KIND_NAME
                {
                    kinds.push(mime);
                }
            }
            Event::Selection { offer } => {
                // Offers that are no longer the clipboard's are let go.
                let stale: Vec<u32> = self.offers.keys().copied().filter(|id| Some(*id) != offer).collect();
                for id in stale {
                    self.offers.remove(&id);
                    self.client.destroy_offer(id)?;
                }
                self.selection = offer.filter(|id| self.offers.contains_key(id));
            }
            Event::SourceSend { source, mime } => {
                // The pipe comes with the event: take it whether or not it is wanted.
                let pipe = self.client.take_fd();
                if let (Some(pipe), Some((ours, text))) = (pipe, &self.copied)
                    && *ours == source
                    && TEXT_KINDS.contains(&mime.as_str())
                {
                    // Written on a thread of its own: the reader may be slow, or never read.
                    let text = text.clone();
                    std::thread::spawn(move || {
                        let _ = std::fs::File::from(pipe).write_all(text.as_bytes());
                    });
                }
            }
            Event::SourceCancelled { source } => {
                self.client.destroy_data_source(source)?;
                if self.copied.as_ref().is_some_and(|(ours, _)| *ours == source) {
                    self.copied = None;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn bar_layout(&self, i: usize) -> bar::Layout {
        bar::Layout::plain(self.windows[i].width as f32, &self.buttons.0, &self.buttons.1)
    }

    /// The pointer moved to (x, y) of window `id`: our bar's hover, or the
    /// program's input (it is told the pointer left when it goes onto the bar).
    fn pointer_moved(&mut self, id: u32, x: f32, y: f32) -> io::Result<()> {
        let Some(i) = self.index(id) else { return Ok(()) };
        self.windows[i].pointer_at = Some((x, y));
        // Our border's band: it resizes, before the bar or the program sees the pointer.
        let edge = self.windows[i].edge_at(x, y);
        if edge != self.windows[i].on_edge {
            self.windows[i].on_edge = edge;
            if let Some(device) = self.cursor_device {
                let shape = match edge {
                    Some(e) => Cursor::Resize(e).wire(),
                    None if self.windows[i].to_app(x, y).is_none() => Cursor::Default.wire(),
                    None => self.windows[i].cursor.wire(),
                };
                self.client.set_cursor_shape(device, self.enter_serial, shape)?;
            }
        }
        if edge.is_some() {
            let w = &mut self.windows[i];
            if w.bar_hover.take().is_some() {
                w.dirty = true;
            }
            if !w.in_bar {
                w.in_bar = true;
                self.tell(id, Input::PointerLeft)?;
            }
            return Ok(());
        }
        let hover = if self.windows[i].frame { self.bar_layout(i).at(x, y) } else { None };
        if hover != self.windows[i].bar_hover {
            self.windows[i].bar_hover = hover;
            self.windows[i].dirty = true;
        }
        match self.windows[i].to_app(x, y) {
            Some((x, y)) => {
                if std::mem::take(&mut self.windows[i].in_bar)
                    && let Some(device) = self.cursor_device
                {
                    self.client.set_cursor_shape(device, self.enter_serial, self.windows[i].cursor.wire())?;
                }
                self.tell(id, Input::PointerMoved { x, y })
            }
            None => {
                if !self.windows[i].in_bar {
                    self.windows[i].in_bar = true;
                    if let Some(device) = self.cursor_device {
                        self.client.set_cursor_shape(device, self.enter_serial, Cursor::Default.wire())?;
                    }
                    self.tell(id, Input::PointerLeft)?;
                }
                Ok(())
            }
        }
    }

    /// A press or release on window `id`'s bar: a button does its work when
    /// released over it; a press anywhere else moves the window.
    fn bar_button(&mut self, id: u32, x: f32, y: f32, pressed: bool) -> io::Result<()> {
        let Some(i) = self.index(id) else { return Ok(()) };
        let part = self.bar_layout(i).at(x, y);
        let toplevel = self.windows[i].toplevel;
        if pressed {
            match part {
                Some(bar::Part::Button(_)) => self.windows[i].bar_pressed = part,
                _ => {
                    if let Some(seat) = self.seat {
                        self.client.start_move(toplevel, seat, self.press_serial)?;
                    }
                }
            }
            return Ok(());
        }
        if self.windows[i].bar_pressed.take().is_some_and(|p| Some(p) == part) {
            match part {
                Some(bar::Part::Button(bar::WinButton::Close)) => self.ask_close(self.windows[i].id)?,
                Some(bar::Part::Button(bar::WinButton::Minimize)) => self.client.set_minimized(toplevel)?,
                Some(bar::Part::Button(bar::WinButton::Maximize)) => self.client.set_maximized(toplevel, !self.windows[i].maximized)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// Our bar over window `i`'s scene, which is moved down below it.
    fn add_bar(&mut self, i: usize, scene: &mut Scene, title: &str) {
        scene.translate(0.0, bar::BAR_H);
        scene.height += bar::BAR_H;
        let layout = self.bar_layout(i);
        let font = self.fonts.face("sans-serif", 400, false);
        let w = &self.windows[i];
        let look = bar::Look { bar: vergence_look::bar_look(self.light), frosted: false, focused: w.active, pane: Color::rgb(0, 0, 0), pane_ink: Color::rgb(255, 255, 255), font, font_size: BAR_FONT_SIZE, titles: &[], alerts: &[], edited: &[], title, shown: 0, hover: w.bar_hover, dragged: None, tools: &[], app_icon: None, menu_open: false, root: as_root() };
        bar::draw(scene, &mut self.fonts, &layout, &look);
    }

    /// A key, to the window with the keyboard.
    fn key(&mut self, key: u32, pressed: bool, repeat: bool) -> io::Result<()> {
        let Some(id) = self.keyboard_window else { return Ok(()) };
        let Some(keyboard) = &mut self.keyboard else { return Ok(()) };
        let modifiers = keyboard.modifiers();
        // A key going up types nothing, and is no part of a sequence.
        let (sym, text, composing) = if pressed {
            let p = keyboard.press(key);
            (p.sym, p.text, p.composing)
        } else {
            (keyboard.key(key).0, None, false)
        };
        self.tell(id, Input::Key { sym, text, pressed, repeat, composing, modifiers })
    }

    /// Tells each window's program of every timer of its that is due, the
    /// earliest first. A timer set again while this runs waits for the next turn.
    fn timers_due(&mut self) -> io::Result<()> {
        let now = Instant::now();
        let mut due: Vec<(u64, Instant, u32)> = Vec::new();
        for w in &mut self.windows {
            let id = w.id;
            w.timers.retain(|t| {
                let keep = t.1 > now;
                if !keep {
                    due.push((t.0, t.1, id));
                }
                keep
            });
        }
        due.sort_by_key(|t| t.1);
        for (timer, _, id) in due {
            self.with_window(id, |app, window| app.timer(timer, window))?;
        }
        Ok(())
    }

    /// Repeats the held key as often as is due. A program that falls behind
    /// gets the repeats it missed at once, at most a few: it catches up, it
    /// does not run on after the key is released.
    fn repeat_due(&mut self) -> io::Result<()> {
        let step = Duration::from_secs_f64(1.0 / self.repeat_rate.max(1) as f64);
        for _ in 0..4 {
            let now = Instant::now();
            let Some(r) = self.repeat.as_mut().filter(|r| r.next <= now) else { break };
            r.next = (r.next + step).max(now);
            let key = r.key;
            self.key(key, true, true)?;
        }
        Ok(())
    }

    /// Lets go of the GPU renderer and every window's buffers of it, for good.
    fn give_up_gpu(&mut self) -> io::Result<()> {
        let Some(mut gpu) = self.gpu.take() else { return Ok(()) };
        for w in &mut self.windows {
            for (_, buffer) in w.asked.drain(..) {
                gpu.release(buffer);
            }
            for shared in w.shared.drain(..) {
                self.client.destroy_buffer(shared.id)?;
                gpu.release(shared.buffer);
            }
            w.dirty = true;
        }
        Ok(())
    }

    /// Draws window `i`'s scene on the GPU into a buffer the server has
    /// taken, if there is one free. Ok(None): not this time (draw in software).
    fn draw_on_gpu(&mut self, i: usize, scene: &Scene, scale: f32, background: Color, pw: u32, ph: u32) -> io::Result<Option<u32>> {
        let (Some(gpu), Some(dmabuf)) = (self.gpu.as_mut(), self.dmabuf) else { return Ok(None) };
        let w = &mut self.windows[i];
        // Buffers of another size that the server is done with are let go.
        let mut k = 0;
        while k < w.shared.len() {
            if !w.shared[k].busy && (w.shared[k].buffer.width, w.shared[k].buffer.height) != (pw, ph) {
                let old = w.shared.swap_remove(k);
                self.client.destroy_buffer(old.id)?;
                gpu.release(old.buffer);
            } else {
                k += 1;
            }
        }
        let fits = |b: &GpuBuffer| (b.width, b.height) == (pw, ph);
        let Some(index) = w.shared.iter().position(|s| !s.busy && fits(&s.buffer)) else {
            // None free: ask the server to take more, up to the few a window needs.
            let have = w.shared.iter().filter(|s| fits(&s.buffer)).count() + w.asked.iter().filter(|(_, b)| fits(b)).count();
            let mut failed = false;
            for _ in have..GPU_BUFFERS.min(have + 2) {
                match gpu.buffer(pw, ph) {
                    Ok(b) => {
                        let params = self.client.create_dmabuf(dmabuf, std::os::fd::AsRawFd::as_raw_fd(&b.fd), pw as i32, ph as i32, b.format, b.modifier, b.offset, b.stride)?;
                        w.asked.push((params, b));
                    }
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
            }
            if failed {
                self.give_up_gpu()?;
            }
            return Ok(None);
        };
        match gpu.draw_into(&w.shared[index].buffer, scene, &mut self.fonts, scale, background) {
            Ok(()) => {
                w.shared[index].busy = true;
                Ok(Some(w.shared[index].id))
            }
            // Something this renderer does not draw yet: this frame in software.
            Err(GpuError::Unsupported(_)) => Ok(None),
            Err(_) => {
                self.give_up_gpu()?;
                Ok(None)
            }
        }
    }

    fn draw(&mut self, i: usize) -> io::Result<()> {
        let t = Instant::now();
        let w = &self.windows[i];
        let bar_h = w.bar_h();
        let frame = Frame { width: w.width as f32, height: (w.height as f32 - bar_h).max(1.0), scale: w.scale120 as f32 / 120.0, time: w.started.elapsed().as_secs_f64(), number: w.frames, drawn_by: w.drawn_by, bar_by_server: false, top: 0.0 , tint: None, text: 1.0, native: false };
        let mut scene = self.windows[i].app.get().scene(&frame, &mut self.fonts);
        if self.windows[i].frame {
            let title = self.windows[i].app.get().title();
            self.add_bar(i, &mut scene, &title);
        }
        let w = &self.windows[i];
        if w.edges && !w.maximized {
            // The line the server draws round a window, in the same colors.
            let style = vergence_look::bar_look(self.light);
            let c = if w.active { style.edge } else { style.edge_off };
            scene.stroke(vergence_scene::Rect::new(0.0, 0.0, scene.width, scene.height), 0.0, vergence_look::EDGE, Color::rgb(c.0, c.1, c.2));
        }
        let background = self.windows[i].app.get().background();
        let (pw, ph) = pixel_size(&scene, frame.scale).map_err(other)?;
        if let Some(id) = self.draw_on_gpu(i, &scene, frame.scale, background, pw, ph)? {
            self.windows[i].drawn_by = "GPU";
            return self.show(i, &frame, id, pw, ph, t);
        }
        self.windows[i].drawn_by = "CPU";
        // A buffer of this size that the server is not using; others of another size are let go.
        let mut stale = Vec::new();
        self.windows[i].buffers.retain(|b| {
            let keep = b.busy || (b.width, b.height) == (pw, ph);
            if !keep {
                stale.push(b.id);
            }
            keep
        });
        for id in stale {
            self.client.destroy_buffer(id)?;
        }
        let stride = pw as usize * 4;
        let alpha = background.a != 255;
        let index = match self.windows[i].buffers.iter().position(|b| !b.busy && (b.width, b.height, b.alpha) == (pw, ph, alpha)) {
            Some(k) => k,
            None => {
                let file = ShmFile::new(stride * ph as usize)?;
                let pool = self.client.create_pool(self.shm, file.fd(), file.len() as i32)?;
                let format = if alpha { wl::FORMAT_ARGB8888 } else { wl::FORMAT_XRGB8888 };
                let id = self.client.create_buffer(pool, 0, pw as i32, ph as i32, stride as i32, format)?;
                self.client.destroy_pool(pool)?;
                self.windows[i].buffers.push(Buffer { file, id, width: pw, height: ph, alpha, busy: false });
                self.windows[i].buffers.len() - 1
            }
        };
        let buffer = &mut self.windows[i].buffers[index];
        // Drawn in place, in the server's byte order: no picture of our own, no copy.
        let target = Target { data: buffer.file.bytes(), width: pw, height: ph, order: Order::Bgra };
        self.cpu.render_target(&scene, &mut self.fonts, frame.scale, background, target, 0..ph).map_err(other)?;
        buffer.busy = true;
        let id = buffer.id;
        self.show(i, &frame, id, pw, ph, t)
    }

    /// Hands the server a drawn buffer as window `i`'s next frame.
    fn show(&mut self, i: usize, frame: &Frame, id: u32, pw: u32, ph: u32, t: Instant) -> io::Result<()> {
        let (surface, viewport, width, height) = {
            let w = &self.windows[i];
            (w.surface, w.viewport, w.width, w.height)
        };
        self.client.attach(surface, id)?;
        self.client.damage_buffer(surface, 0, 0, pw as i32, ph as i32)?;
        if let Some(v) = viewport {
            self.client.set_destination(v, width as i32, height as i32)?;
        }
        let callback = self.client.frame(surface)?;
        self.client.commit(surface)?;
        self.client.flush()?;
        let w = &mut self.windows[i];
        w.frame_callback = Some(callback);
        w.frames += 1;
        if !w.app.get().shown(frame, t.elapsed().as_secs_f64() * 1e3) {
            w.closing = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_desktops_dark_or_light_forces_every_app_and_app_default_lets_each_choose() {
        use super::{BarStyle, Pick, app_pick, bar_style_setting, content_light, table_value};
        let forced = "[appearance]\ncolor-scheme = \"dark\"\n[terminal]\nstyle = \"light\" # mine\nbar-style = \"light\"\n";
        assert!(!content_light(forced, "[terminal]", true), "the desktop's Dark wins over the app's light");
        assert_eq!(bar_style_setting(forced, "[terminal]", BarStyle::Light), BarStyle::Desktop);
        let free = "[appearance]\ncolor-scheme = \"default\"\n[terminal]\nstyle = \"light\"\nbar-style = \"dark\"\n";
        assert!(content_light(free, "[terminal]", false), "App default: the app's own choice");
        assert_eq!(bar_style_setting(free, "[terminal]", BarStyle::Desktop), BarStyle::Dark);
        assert!(content_light(free, "[settings]", true), "no choice of its own: its built-in look");
        assert!(!content_light(free, "[other]", false));
        assert_eq!(bar_style_setting(free, "[settings]", BarStyle::Light), BarStyle::Light);
        assert_eq!(bar_style_setting("", "[terminal]", BarStyle::Desktop), BarStyle::Desktop);
        assert_eq!(app_pick("[terminal]\nstyle = default\n", "[terminal]", "style"), Pick::Desktop);
        assert_eq!(table_value("[a]\nk = \"x # y\"\n", "[a]", "k").as_deref(), Some("x # y"));
    }

    #[test]
    fn a_program_s_opacity_is_its_own_word_or_the_desktop_s() {
        use super::opacity_setting as o;
        assert_eq!(o("", "[terminal]"), 1.0);
        assert_eq!(o("[appearance]\nwindow-opacity = 0.9\n", "[terminal]"), 0.9);
        assert_eq!(o("[appearance]\nwindow-opacity = 0.9\n[terminal]\nopacity = 0.8 # mine\n", "[terminal]"), 0.8);
        assert_eq!(o("[terminal]\nopacity = 0.8\n", "[settings]"), 1.0, "another program's word");
        // Kept readable, and nonsense is no word at all.
        assert_eq!(o("[terminal]\nopacity = 0.05\n", "[terminal]"), 0.3);
        assert_eq!(o("[terminal]\nopacity = 7\n", "[terminal]"), 1.0);
        assert_eq!(o("[terminal]\nopacity = glass\n[appearance]\nwindow-opacity = 0.7\n", "[terminal]"), 0.7);
        assert_eq!(o("[shelf]\nopacity = 0.5\n", "[shelf]"), 0.5);
    }

    #[test]
    fn timers_are_set_moved_forgotten_and_bounded() {
        use super::set_timer;
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let mut timers = Vec::new();
        set_timer(&mut timers, 1, Some(t0 + Duration::from_millis(500)));
        set_timer(&mut timers, 2, Some(t0 + Duration::from_millis(100)));
        set_timer(&mut timers, 1, Some(t0 + Duration::from_millis(50)));
        assert_eq!(timers.len(), 2, "asking again moves the timer");
        assert!(timers.contains(&(1, t0 + Duration::from_millis(50))));
        set_timer(&mut timers, 2, None);
        assert_eq!(timers, vec![(1, t0 + Duration::from_millis(50))]);
        for id in 0..(super::MAX_TIMERS as u64 + 10) {
            set_timer(&mut timers, 1000 + id, Some(t0 + Duration::from_millis(id)));
        }
        assert_eq!(timers.len(), super::MAX_TIMERS);
        assert!(!timers.iter().any(|t| t.0 == 1000 + super::MAX_TIMERS as u64 + 9), "the latest-due goes");
    }
}

/// Whether this program runs as the superuser: the bar it draws itself
/// (under another desktop) goes the alarm red, as the server's does.
fn as_root() -> bool {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: geteuid(2) takes nothing and cannot fail.
    unsafe { geteuid() == 0 }
}
