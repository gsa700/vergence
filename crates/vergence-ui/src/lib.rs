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

pub mod bar;
pub mod keyboard;
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
}

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
pub trait App {
    fn title(&self) -> String;

    /// The program's name to the desktop (window memory, the shelf).
    fn app_id(&self) -> String;

    /// True if the window draws its own bar; false asks the server for one.
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

struct Runner {
    client: Client,
    shm: u32,
    wm_base: u32,
    surface: u32,
    xdg_surface: u32,
    toplevel: u32,
    viewport: Option<u32>,
    exact_scale: bool,
    seat: Option<u32>,
    cursor_shape_manager: Option<u32>,
    pointer: Option<u32>,
    touch: Option<u32>,
    /// The fingers that are down, and where each last was.
    fingers: Vec<(i32, f32, f32)>,
    keyboard_object: Option<u32>,
    cursor_device: Option<u32>,
    data_manager: Option<u32>,
    data_device: Option<u32>,
    dmabuf: Option<u32>,

    /// The GPU renderer, while it and the server's taking of its buffers work.
    gpu: Option<Gpu>,
    /// GPU buffers the server has taken, and those it has been asked to take.
    shared: Vec<Shared>,
    asked: Vec<(u32, GpuBuffer)>,
    drawn_by: &'static str,

    cpu: Cpu,
    fonts: Fonts,
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

    /// Where the pointer is, if it is over the window.
    pointer_at: Option<(f32, f32)>,
    /// The serial of the pointer's entry (to set its shape) and of the last
    /// button press (to start a move or a resize).
    enter_serial: u32,
    press_serial: u32,
    cursor: Cursor,
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
    /// Input made here rather than read from the server, to give the program next.
    inbox: Vec<Input>,
    /// The pipe other threads wake the window through, and its flag.
    wake: std::fs::File,
    wake_pending: Arc<AtomicBool>,
}

/// Opens the window and runs until it is closed. `size` is the size asked
/// for, in reference units; the server may give another.
pub fn run(app: &mut dyn App, size: (u32, u32)) -> io::Result<()> {
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

    let surface = client.create_surface(compositor)?;
    let xdg_surface = client.get_xdg_surface(wm_base, surface)?;
    let toplevel = client.get_toplevel(xdg_surface)?;
    client.set_title(toplevel, &app.title())?;
    client.set_app_id(toplevel, &app.app_id())?;
    if let Some(manager) = decoration_manager {
        let decoration = client.get_decoration(manager, toplevel)?;
        client.set_decoration_mode(decoration, if app.own_frame() { wl::DECORATION_CLIENT } else { wl::DECORATION_SERVER })?;
    }
    // The exact scale needs both: the server says the scale, and the viewport
    // says the buffer of that many pixels is still the window's size in units.
    let (viewport, exact_scale) = match (viewporter, scale_manager) {
        (Some(v), Some(m)) => {
            client.get_fractional_scale(m, surface)?;
            (Some(client.get_viewport(v, surface)?), true)
        }
        _ => (None, false),
    };
    // The first commit has no buffer: it asks the server to configure the window.
    client.commit(surface)?;

    let (wake_read, wake_write) = vergence_wl::conn::pipe()?;
    let wake_pending = Arc::new(AtomicBool::new(false));
    app.waker(Waker { pipe: Arc::new(wake_write.into()), pending: wake_pending.clone() });

    let mut runner = Runner {
        client,
        shm,
        wm_base,
        surface,
        xdg_surface,
        toplevel,
        viewport,
        exact_scale,
        seat,
        cursor_shape_manager,
        pointer: None,
        touch: None,
        fingers: Vec::new(),
        keyboard_object: None,
        cursor_device: None,
        data_manager,
        data_device,
        dmabuf,
        gpu,
        shared: Vec::new(),
        asked: Vec::new(),
        drawn_by: "",
        cpu: Cpu::new(),
        fonts: Fonts::system(),
        started: Instant::now(),
        width: size.0.max(1),
        height: size.1.max(1),
        suggested: None,
        scale120: 120,
        configured: false,
        dirty: false,
        frame_callback: None,
        frames: 0,
        buffers: Vec::new(),
        closing: false,
        pointer_at: None,
        enter_serial: 0,
        press_serial: 0,
        cursor: Cursor::Default,
        keyboard: None,
        repeat_rate: 0,
        repeat_delay: 0,
        repeat: None,
        input_serial: 0,
        offers: HashMap::new(),
        selection: None,
        copied: None,
        paste: None,
        inbox: Vec::new(),
        wake: wake_read.into(),
        wake_pending,
    };
    runner.run(app)
}

impl Runner {
    fn run(&mut self, app: &mut dyn App) -> io::Result<()> {
        while !self.closing {
            // Wait for the server, but no longer than the next key repeat is due.
            let now = Instant::now();
            let repeat = self.repeat.as_ref().map(|r| r.next.saturating_duration_since(now));
            let paste = self.paste.as_ref().map(|p| (p.started + PASTE_TIMEOUT).saturating_duration_since(now));
            let wait = match (repeat, paste) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let pipe = self.paste.as_ref().map(|p| p.pipe.as_raw_fd());
            let (events, [pipe_ready, woken]) = self.client.dispatch_or([pipe, Some(self.wake.as_raw_fd())], wait)?;
            for event in events {
                self.handle(event, app)?;
            }
            self.read_paste(pipe_ready);
            if woken {
                // The flag is cleared first: a wake that comes while the program is being told is not lost.
                let mut drain = [0u8; 64];
                let _ = self.wake.read(&mut drain);
                self.wake_pending.store(false, Ordering::SeqCst);
                self.with_window(app, |app, window| app.woken(window))?;
            }
            for input in std::mem::take(&mut self.inbox) {
                self.tell(app, input)?;
            }
            self.repeat_due(app)?;
            if self.configured && self.dirty && self.frame_callback.is_none() && !self.closing {
                self.dirty = false;
                self.draw(app)?;
            }
        }
        Ok(())
    }

    /// Gives the program one input and does what it asks in return.
    fn tell(&mut self, app: &mut dyn App, input: Input) -> io::Result<()> {
        self.with_window(app, |app, window| app.input(&input, window))
    }

    /// Calls the program with a handle on its window, then does what it asked.
    fn with_window(&mut self, app: &mut dyn App, call: impl FnOnce(&mut dyn App, &mut Window)) -> io::Result<()> {
        let mut window = Window::default();
        call(app, &mut window);
        for command in window.commands {
            match command {
                Command::Redraw => self.dirty = true,
                Command::Close => self.closing = true,
                Command::Move => {
                    if let Some(seat) = self.seat {
                        self.client.start_move(self.toplevel, seat, self.press_serial)?;
                    }
                }
                Command::Resize(edges) => {
                    if let Some(seat) = self.seat {
                        self.client.start_resize(self.toplevel, seat, self.press_serial, edges.wire())?;
                    }
                }
                Command::Cursor(cursor) => {
                    if cursor != self.cursor {
                        self.cursor = cursor;
                        if let (Some(device), Some(_)) = (self.cursor_device, self.pointer_at) {
                            self.client.set_cursor_shape(device, self.enter_serial, cursor.wire())?;
                        }
                    }
                }
                Command::Title(title) => self.client.set_title(self.toplevel, &title)?,
                Command::Maximize(on) => self.client.set_maximized(self.toplevel, on)?,
                Command::Minimize => self.client.set_minimized(self.toplevel)?,
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
                Command::Paste => self.start_paste()?,
            }
        }
        Ok(())
    }

    /// Asks whoever owns the clipboard for its text.
    fn start_paste(&mut self) -> io::Result<()> {
        // Our own copy needs no trip through the server (and a program
        // cannot wait on itself).
        if let Some((_, text)) = &self.copied {
            self.inbox.push(Input::Paste(text.to_string()));
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
        Ok(())
    }

    /// Takes what has arrived of a paste; when it is all here (or too much,
    /// or too late), gives it to the program.
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
                self.inbox.push(Input::Paste(String::from_utf8_lossy(&paste.data).into_owned()));
            }
        }
    }

    fn modifiers(&self) -> Modifiers {
        self.keyboard.as_ref().map(keyboard::Keyboard::modifiers).unwrap_or_default()
    }

    fn handle(&mut self, event: Event, app: &mut dyn App) -> io::Result<()> {
        match event {
            Event::Ping { serial } => self.client.pong(self.wm_base, serial)?,
            Event::ToplevelConfigure { width, height, .. } => {
                if width > 0 && height > 0 {
                    self.suggested = Some((width as u32, height as u32));
                }
            }
            Event::Configure { serial, .. } => {
                self.client.ack_configure(self.xdg_surface, serial)?;
                if let Some(size) = self.suggested.take() {
                    (self.width, self.height) = size;
                }
                self.configured = true;
                self.dirty = true;
            }
            Event::PreferredScale { scale120, .. } => {
                if self.exact_scale && scale120 > 0 && scale120 != self.scale120 {
                    self.scale120 = scale120;
                    self.dirty = true;
                }
            }
            Event::BufferRelease { buffer } => {
                if let Some(b) = self.buffers.iter_mut().find(|b| b.id == buffer) {
                    b.busy = false;
                }
                if let Some(b) = self.shared.iter_mut().find(|b| b.id == buffer) {
                    b.busy = false;
                }
            }
            Event::DmabufCreated { params, buffer } => {
                self.client.destroy_dmabuf_params(params)?;
                if let Some(i) = self.asked.iter().position(|(p, _)| *p == params) {
                    let (_, made) = self.asked.swap_remove(i);
                    self.shared.push(Shared { buffer: made, id: buffer, busy: false });
                    // A frame drawn in software meanwhile is drawn again on the GPU.
                    self.dirty = true;
                } else {
                    self.client.destroy_buffer(buffer)?;
                }
            }
            Event::DmabufFailed { params } => {
                // The server does not take this GPU's buffers: software from here on.
                self.client.destroy_dmabuf_params(params)?;
                self.give_up_gpu()?;
            }
            Event::Done { callback, .. } if Some(callback) == self.frame_callback => {
                self.frame_callback = None;
                if app.animating() {
                    self.dirty = true;
                }
            }
            Event::Close { .. } => self.closing = true,
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
            Event::PointerEnter { serial, x, y, .. } => {
                self.enter_serial = serial;
                self.pointer_at = Some((x as f32, y as f32));
                // A pointer that enters has no shape until it is given one.
                if let Some(device) = self.cursor_device {
                    self.client.set_cursor_shape(device, serial, self.cursor.wire())?;
                }
                self.tell(app, Input::PointerMoved { x: x as f32, y: y as f32 })?;
            }
            Event::PointerLeave { .. } => {
                self.pointer_at = None;
                self.tell(app, Input::PointerLeft)?;
            }
            Event::PointerMotion { x, y, .. } => {
                self.pointer_at = Some((x as f32, y as f32));
                self.tell(app, Input::PointerMoved { x: x as f32, y: y as f32 })?;
            }
            Event::PointerButton { serial, button, pressed, .. } => {
                if pressed {
                    self.press_serial = serial;
                    self.input_serial = serial;
                }
                let button = match button {
                    0x110 => Button::Left,
                    0x111 => Button::Right,
                    0x112 => Button::Middle,
                    other => Button::Other(other),
                };
                let (x, y) = self.pointer_at.unwrap_or_default();
                let modifiers = self.modifiers();
                self.tell(app, Input::Button { button, pressed, x, y, modifiers })?;
            }
            Event::TouchDown { serial, id, x, y, .. } => {
                // A finger's press is as good as a button's for what needs one
                // (the clipboard, moving the window).
                self.press_serial = serial;
                self.input_serial = serial;
                let (x, y) = (x as f32, y as f32);
                self.fingers.retain(|f| f.0 != id);
                if self.fingers.len() < 16 {
                    self.fingers.push((id, x, y));
                }
                self.tell(app, Input::Touch { id, phase: touch::Phase::Down, x, y })?;
            }
            Event::TouchMotion { id, x, y, .. } => {
                let (x, y) = (x as f32, y as f32);
                if let Some(finger) = self.fingers.iter_mut().find(|f| f.0 == id) {
                    (finger.1, finger.2) = (x, y);
                    self.tell(app, Input::Touch { id, phase: touch::Phase::Moved, x, y })?;
                }
            }
            Event::TouchUp { id, .. } => {
                if let Some(index) = self.fingers.iter().position(|f| f.0 == id) {
                    let (_, x, y) = self.fingers.remove(index);
                    self.tell(app, Input::Touch { id, phase: touch::Phase::Up, x, y })?;
                }
            }
            Event::TouchCancel => {
                for (id, x, y) in std::mem::take(&mut self.fingers) {
                    self.tell(app, Input::Touch { id, phase: touch::Phase::Cancelled, x, y })?;
                }
            }
            Event::TouchFrame => {}
            Event::PointerAxis { axis, value, .. } => {
                let (x, y) = self.pointer_at.unwrap_or_default();
                let (dx, dy) = if axis == 0 { (0.0, value as f32) } else { (value as f32, 0.0) };
                self.tell(app, Input::Scroll { x, y, dx, dy })?;
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
            Event::KeyboardEnter { .. } => self.tell(app, Input::Focus(true))?,
            Event::KeyboardLeave { .. } => {
                self.repeat = None;
                if let Some(k) = &mut self.keyboard {
                    k.reset_compose();
                }
                self.tell(app, Input::Focus(false))?;
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
                self.key(app, key, pressed, false)?;
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

    fn key(&mut self, app: &mut dyn App, key: u32, pressed: bool, repeat: bool) -> io::Result<()> {
        let Some(keyboard) = &mut self.keyboard else { return Ok(()) };
        let modifiers = keyboard.modifiers();
        // A key going up types nothing, and is no part of a sequence.
        let (sym, text, composing) = if pressed {
            let p = keyboard.press(key);
            (p.sym, p.text, p.composing)
        } else {
            (keyboard.key(key).0, None, false)
        };
        self.tell(app, Input::Key { sym, text, pressed, repeat, composing, modifiers })
    }

    /// Repeats the held key as often as is due. A program that falls behind
    /// gets the repeats it missed at once, at most a few: it catches up, it
    /// does not run on after the key is released.
    fn repeat_due(&mut self, app: &mut dyn App) -> io::Result<()> {
        let step = Duration::from_secs_f64(1.0 / self.repeat_rate.max(1) as f64);
        for _ in 0..4 {
            let now = Instant::now();
            let Some(r) = self.repeat.as_mut().filter(|r| r.next <= now) else { break };
            r.next = (r.next + step).max(now);
            let key = r.key;
            self.key(app, key, true, true)?;
        }
        Ok(())
    }

    /// Lets go of the GPU renderer and its buffers, for good.
    fn give_up_gpu(&mut self) -> io::Result<()> {
        let Some(mut gpu) = self.gpu.take() else { return Ok(()) };
        for (_, buffer) in self.asked.drain(..) {
            gpu.release(buffer);
        }
        for shared in self.shared.drain(..) {
            self.client.destroy_buffer(shared.id)?;
            gpu.release(shared.buffer);
        }
        self.dirty = true;
        Ok(())
    }

    /// Draws the scene on the GPU into a buffer the server has taken, if
    /// there is one free. Ok(None): not this time (draw in software).
    fn draw_on_gpu(&mut self, scene: &Scene, scale: f32, background: Color, pw: u32, ph: u32) -> io::Result<Option<u32>> {
        let (Some(gpu), Some(dmabuf)) = (self.gpu.as_mut(), self.dmabuf) else { return Ok(None) };
        // Buffers of another size that the server is done with are let go.
        let mut i = 0;
        while i < self.shared.len() {
            if !self.shared[i].busy && (self.shared[i].buffer.width, self.shared[i].buffer.height) != (pw, ph) {
                let old = self.shared.swap_remove(i);
                self.client.destroy_buffer(old.id)?;
                gpu.release(old.buffer);
            } else {
                i += 1;
            }
        }
        let fits = |b: &GpuBuffer| (b.width, b.height) == (pw, ph);
        let Some(index) = self.shared.iter().position(|s| !s.busy && fits(&s.buffer)) else {
            // None free: ask the server to take more, up to the few a window needs.
            let have = self.shared.iter().filter(|s| fits(&s.buffer)).count() + self.asked.iter().filter(|(_, b)| fits(b)).count();
            for _ in have..GPU_BUFFERS.min(have + 2) {
                match gpu.buffer(pw, ph) {
                    Ok(b) => {
                        let params = self.client.create_dmabuf(dmabuf, std::os::fd::AsRawFd::as_raw_fd(&b.fd), pw as i32, ph as i32, b.format, b.modifier, b.offset, b.stride)?;
                        self.asked.push((params, b));
                    }
                    Err(_) => {
                        self.give_up_gpu()?;
                        break;
                    }
                }
            }
            return Ok(None);
        };
        match gpu.draw_into(&self.shared[index].buffer, scene, &mut self.fonts, scale, background) {
            Ok(()) => {
                self.shared[index].busy = true;
                Ok(Some(self.shared[index].id))
            }
            // Something this renderer does not draw yet: this frame in software.
            Err(GpuError::Unsupported(_)) => Ok(None),
            Err(_) => {
                self.give_up_gpu()?;
                Ok(None)
            }
        }
    }

    fn draw(&mut self, app: &mut dyn App) -> io::Result<()> {
        let t = Instant::now();
        let frame = Frame { width: self.width as f32, height: self.height as f32, scale: self.scale120 as f32 / 120.0, time: self.started.elapsed().as_secs_f64(), number: self.frames, drawn_by: self.drawn_by };
        let scene = app.scene(&frame, &mut self.fonts);
        let background = app.background();
        let (pw, ph) = pixel_size(&scene, frame.scale).map_err(other)?;
        if let Some(id) = self.draw_on_gpu(&scene, frame.scale, background, pw, ph)? {
            self.drawn_by = "GPU";
            return self.show(app, &frame, id, pw, ph, t);
        }
        self.drawn_by = "CPU";
        // A buffer of this size that the server is not using; others of another size are let go.
        let mut stale = Vec::new();
        self.buffers.retain(|b| {
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
        let index = match self.buffers.iter().position(|b| !b.busy && (b.width, b.height, b.alpha) == (pw, ph, alpha)) {
            Some(i) => i,
            None => {
                let file = ShmFile::new(stride * ph as usize)?;
                let pool = self.client.create_pool(self.shm, file.fd(), file.len() as i32)?;
                let format = if alpha { wl::FORMAT_ARGB8888 } else { wl::FORMAT_XRGB8888 };
                let id = self.client.create_buffer(pool, 0, pw as i32, ph as i32, stride as i32, format)?;
                self.client.destroy_pool(pool)?;
                self.buffers.push(Buffer { file, id, width: pw, height: ph, alpha, busy: false });
                self.buffers.len() - 1
            }
        };
        let buffer = &mut self.buffers[index];
        // Drawn in place, in the server's byte order: no picture of our own, no copy.
        let target = Target { data: buffer.file.bytes(), width: pw, height: ph, order: Order::Bgra };
        self.cpu.render_target(&scene, &mut self.fonts, frame.scale, background, target, 0..ph).map_err(other)?;
        buffer.busy = true;
        let id = buffer.id;
        self.show(app, &frame, id, pw, ph, t)
    }

    /// Hands the server a drawn buffer as the window's next frame.
    fn show(&mut self, app: &mut dyn App, frame: &Frame, id: u32, pw: u32, ph: u32, t: Instant) -> io::Result<()> {
        self.client.attach(self.surface, id)?;
        self.client.damage_buffer(self.surface, 0, 0, pw as i32, ph as i32)?;
        if let Some(v) = self.viewport {
            self.client.set_destination(v, self.width as i32, self.height as i32)?;
        }
        self.frame_callback = Some(self.client.frame(self.surface)?);
        self.client.commit(self.surface)?;
        self.client.flush()?;
        self.frames += 1;
        if !app.shown(frame, t.elapsed().as_secs_f64() * 1e3) {
            self.closing = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
}
