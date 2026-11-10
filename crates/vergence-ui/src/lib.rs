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
//! For now the pixels are drawn by the CPU renderer into memory shared with
//! the server (`wl_shm`); the GPU renderer and its buffers are step 3 of
//! the milestone. The clipboard comes next. There are no widgets yet.
//!
//! Sizes and positions a program sees are in reference units (whitepaper
//! Contract 1). It is never told to scale anything: the same scene is drawn
//! sharper on a denser monitor, and drawn again when the window moves to
//! another.

pub mod keyboard;

use std::{
    io,
    time::{Duration, Instant},
};

use vergence_render::{Cpu, Pixmap, pixel_size};
use vergence_scene::{Color, Scene};
use vergence_text::Fonts;
use vergence_wl::{
    Client, Event, Interface, client as wl,
    shm::{ShmFile, read_shared},
};

pub use keyboard::{Modifiers, sym};

/// The largest keymap accepted from the server.
const MAX_KEYMAP: usize = 4 << 20;

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
    Key { sym: u32, text: Option<String>, pressed: bool, repeat: bool, modifiers: Modifiers },
    /// The window gained or lost the keyboard.
    Focus(bool),
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
    /// The server is showing it, or may yet: not to be drawn into.
    busy: bool,
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
    keyboard_object: Option<u32>,
    cursor_device: Option<u32>,

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
    pixmap: Option<Pixmap>,
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
        keyboard_object: None,
        cursor_device: None,
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
        pixmap: None,
        closing: false,
        pointer_at: None,
        enter_serial: 0,
        press_serial: 0,
        cursor: Cursor::Default,
        keyboard: None,
        repeat_rate: 0,
        repeat_delay: 0,
        repeat: None,
    };
    runner.run(app)
}

impl Runner {
    fn run(&mut self, app: &mut dyn App) -> io::Result<()> {
        while !self.closing {
            // Wait for the server, but no longer than the next key repeat is due.
            let wait = self.repeat.as_ref().map(|r| r.next.saturating_duration_since(Instant::now()));
            for event in self.client.dispatch_within(wait)? {
                self.handle(event, app)?;
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
        let mut window = Window::default();
        app.input(&input, &mut window);
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
            }
        }
        Ok(())
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
                self.tell(app, Input::Focus(false))?;
            }
            Event::Modifiers { depressed, latched, locked, group } => {
                if let Some(k) = &mut self.keyboard {
                    k.update(depressed, latched, locked, group);
                }
            }
            Event::Key { key, pressed, .. } => {
                if pressed {
                    let repeats = self.repeat_rate > 0 && self.keyboard.as_ref().is_some_and(|k| k.repeats(key));
                    self.repeat = repeats.then(|| Repeat { key, next: Instant::now() + Duration::from_millis(self.repeat_delay.max(0) as u64) });
                } else if self.repeat.as_ref().is_some_and(|r| r.key == key) {
                    self.repeat = None;
                }
                self.key(app, key, pressed, false)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn key(&mut self, app: &mut dyn App, key: u32, pressed: bool, repeat: bool) -> io::Result<()> {
        let Some(keyboard) = &self.keyboard else { return Ok(()) };
        let (sym, text) = keyboard.key(key);
        let modifiers = keyboard.modifiers();
        // A key going up types nothing.
        self.tell(app, Input::Key { sym, text: text.filter(|_| pressed), pressed, repeat, modifiers })
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

    fn draw(&mut self, app: &mut dyn App) -> io::Result<()> {
        let t = Instant::now();
        let frame = Frame { width: self.width as f32, height: self.height as f32, scale: self.scale120 as f32 / 120.0, time: self.started.elapsed().as_secs_f64(), number: self.frames };
        let scene = app.scene(&frame, &mut self.fonts);
        let background = app.background();
        let (pw, ph) = pixel_size(&scene, frame.scale).map_err(other)?;
        if self.pixmap.as_ref().is_none_or(|p| (p.width, p.height) != (pw, ph)) {
            self.pixmap = Some(Pixmap::new(pw, ph));
        }
        let picture = self.pixmap.as_mut().expect("just made");
        self.cpu.render_rows(&scene, &mut self.fonts, frame.scale, background, picture, 0..ph).map_err(other)?;

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
        let index = match self.buffers.iter().position(|b| !b.busy && (b.width, b.height) == (pw, ph)) {
            Some(i) => i,
            None => {
                let file = ShmFile::new(stride * ph as usize)?;
                let pool = self.client.create_pool(self.shm, file.fd(), file.len() as i32)?;
                let format = if background.a == 255 { wl::FORMAT_XRGB8888 } else { wl::FORMAT_ARGB8888 };
                let id = self.client.create_buffer(pool, 0, pw as i32, ph as i32, stride as i32, format)?;
                self.client.destroy_pool(pool)?;
                self.buffers.push(Buffer { file, id, width: pw, height: ph, busy: false });
                self.buffers.len() - 1
            }
        };
        let buffer = &mut self.buffers[index];
        // The renderer's bytes are red, green, blue, alpha; the server's format has blue first.
        for (to, from) in buffer.file.bytes().chunks_exact_mut(4).zip(picture.data.chunks_exact(4)) {
            to.copy_from_slice(&[from[2], from[1], from[0], from[3]]);
        }
        buffer.busy = true;
        let id = buffer.id;
        self.client.attach(self.surface, id)?;
        self.client.damage_buffer(self.surface, 0, 0, pw as i32, ph as i32)?;
        if let Some(v) = self.viewport {
            self.client.set_destination(v, self.width as i32, self.height as i32)?;
        }
        self.frame_callback = Some(self.client.frame(self.surface)?);
        self.client.commit(self.surface)?;
        self.client.flush()?;
        self.frames += 1;
        if !app.shown(&frame, t.elapsed().as_secs_f64() * 1e3) {
            self.closing = true;
        }
        Ok(())
    }
}
