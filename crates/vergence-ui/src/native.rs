//! The toolkit's native road (native milestone 1, step 5).
//!
//! In a Vergence session (`VERGENCE_BUS` names the server's bus) a window is
//! opened on the project's own protocol, on the connection that carries the
//! session bus, instead of over Wayland. Nothing in a program changes: the
//! same [`App`], [`Window`], [`Input`] and [`Frame`], the same keyboard and
//! touch handling, timers, waker and clipboard behavior as the Wayland road
//! in lib.rs, which this follows event for event. Elsewhere (another
//! desktop), or when the server refuses the window, [`run`] returns
//! `Ok(None)` before the program has been told anything, and lib.rs opens
//! the window over Wayland. `VERGENCE_UI_WAYLAND=1` keeps to Wayland, to
//! compare the two.
//!
//! The differences from Wayland, all in the protocol (vergence-protocol's
//! window.rs): one configure carries the size in reference units and the
//! monitor's exact scale; shared memory is copied by the server when it
//! reads the attach, so one buffer serves every frame; a GPU buffer is free
//! again once a later picture has been attached and a frame has followed;
//! cursor shapes and resize edges use the same numbers as Wayland's.
//!
//! A program may have several windows (bar milestone 1, step 4:
//! [`Window::open_window`]). They share the connection, the keyboard and its
//! repeat, the clipboard, the GPU and the fonts, and the waker (a wake goes
//! to every window); each has its own picture, size and scale, frames,
//! pointer, fingers, cursor and timers. The first window is created before
//! anything else, since it decides between this road and Wayland; the others
//! as the program asks. The program ends when its last window closes.

use std::{
    io::{self, Read, Write},
    os::fd::AsRawFd,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use vergence_protocol::{
    Client, ClientError,
    bus::{ClientMsg, ServerMsg},
    shm::{ShmFile, read_shared},
    sock,
    window::{Buffer, Fd, MAX_NODES, MAX_STATES, Node, Plane, SHM_ARGB8888, SHM_XRGB8888, WinEvent, WinRequest},
};
use vergence_render::{Cpu, Gpu, GpuBuffer, GpuError, Order, Target, pixel_size};
use vergence_scene::{Color, Scene};
use vergence_text::Fonts;

use crate::{
    TabEvent,
    App, Button, Command, Cursor, Frame, GPU_BUFFERS, Input, MAX_KEYMAP, MeaningTree, MAX_PASTE, Modifiers, PASTE_TIMEOUT, Paste, Repeat, TEXT_KINDS, Waker, Window, keyboard, other, set_timer,
    touch,
};

/// A GPU buffer and whether the server may still be reading it.
struct Slot {
    buffer: GpuBuffer,
    busy: bool,
}

/// The program, borrowed for its first window, or a window it opened later.
pub(crate) enum Run<'a> {
    Borrowed(&'a mut dyn App),
    Owned(Box<dyn App>),
}

impl Run<'_> {
    pub(crate) fn get(&mut self) -> &mut dyn App {
        match self {
            Run::Borrowed(a) => &mut **a,
            Run::Owned(a) => a.as_mut(),
        }
    }
}

/// One window: the program that runs it and what the road keeps for it.
struct Win<'a> {
    id: u32,
    app: Run<'a>,
    /// Created by the server (the first is before the loop; the others when
    /// their `Created` comes).
    created: bool,
    /// The window that opened it (its parent to the server); 0 for none.
    opened_by: u32,
    slots: Vec<Slot>,
    /// The slot attached last: the one the server shows until the next.
    showing: Option<usize>,
    drawn_by: &'static str,
    /// Shared memory for frames drawn in software: copied by the server at
    /// attach, so one serves every frame (its size, and whether it has alpha).
    shm: Option<(ShmFile, u32, u32, bool)>,
    started: Instant,
    width: u32,
    height: u32,
    scale: f64,
    configured: bool,
    dirty: bool,
    /// A picture is out and its frame event has not come yet.
    frame_pending: bool,
    frames: u64,
    closing: bool,
    pointer_at: Option<(f32, f32)>,
    cursor: Cursor,
    fingers: Vec<(i32, f32, f32)>,
    timers: Vec<(u64, Instant)>,
    /// The server draws this window's tabbed bar (App::tabs_by_server).
    bar_by_server: bool,
    /// The strip under the server's bar it draws (look milestone 1), points.
    inset: f32,
    /// The wallpaper's color behind it, as the server last said (Tint).
    tint: Option<Color>,
}

/// The connection and what a program's windows share.
struct Native<'a> {
    client: Client,
    gpu: Option<Gpu>,
    cpu: Cpu,
    fonts: Fonts,
    windows: Vec<Win<'a>>,
    next_window: u32,
    /// Windows asked for and not yet answered: the request's serial, the window.
    creating: Vec<(u32, u32)>,
    /// The window with the keyboard.
    focus: Option<u32>,
    /// The serial of the last key or button press, any window's (to copy).
    input_serial: u32,
    keyboard: Option<keyboard::Keyboard>,
    repeat_rate: i32,
    repeat_delay: i32,
    repeat: Option<Repeat>,

    /// What the clipboard holds, as the server last offered it.
    clip_kinds: Vec<String>,
    /// What we put there, and whether the offer echoing our copy is yet to come.
    copied: Option<Arc<str>>,
    own_offer_due: bool,
    /// The paste asked for (its serial and the window that asked), and the text on its way in.
    paste_serial: Option<u32>,
    paste_window: u32,
    paste: Option<Paste>,
    /// Input made here rather than read from the server, for a window next.
    inbox: Vec<(u32, Input)>,
    wake: std::fs::File,
    wake_pending: Arc<AtomicBool>,
    waker: Waker,
}

fn client_error(e: ClientError) -> io::Error {
    match e {
        ClientError::Io(e) => e,
        other => io::Error::other(other.to_string()),
    }
}

/// Opens the window natively and runs until the program's last window is
/// closed. Ok(None): not in a Vergence session, or the server would not open
/// a window this way; the program has not been called, and the caller opens
/// it over Wayland.
pub(crate) fn run(app: &mut dyn App, size: (u32, u32)) -> io::Result<Option<()>> {
    if std::env::var("VERGENCE_UI_WAYLAND").as_deref() == Ok("1") || std::env::var_os(vergence_protocol::BUS_ENV).is_none() {
        return Ok(None);
    }
    let Ok(mut client) = Client::connect(&app.app_id()) else { return Ok(None) };
    let (bar_by_server, request) = create_request(app, 1, 0, size);
    let created = client.request(|serial| {
        let mut r = request.clone();
        if let WinRequest::Create { serial: s, .. } = &mut r {
            *s = serial;
        }
        ClientMsg::Window(r)
    });
    let token = match created {
        Ok(ServerMsg::Window(WinEvent::Created { token, .. })) => token,
        // A server without native windows says "not yet", or something else:
        // Wayland it is.
        _ => return Ok(None),
    };
    app.token_given(&token);
    app.opened(1);
    let gpu = (std::env::var("VERGENCE_RENDER").as_deref() != Ok("cpu")).then(|| Gpu::new().ok()).flatten().filter(Gpu::shares_buffers);
    let (wake_read, wake_write) = sock::pipe()?;
    let wake_pending = Arc::new(AtomicBool::new(false));
    let waker = Waker { pipe: Arc::new(wake_write.into()), pending: wake_pending.clone() };
    app.waker(waker.clone());
    // On the native road the server draws every window's bar, whether or not
    // the app would draw one elsewhere (`bar_by_server` says only that): an app
    // that asks to draw under it, or describes it, is heard (2026-11-14: the
    // settings app, which draws no frame, was neither).
    let under_bar = app.under_bar();
    let app_style = app.style();
    let mut first = Win::new(1, Run::Borrowed(app), size, bar_by_server);
    first.created = true;
    if under_bar {
        client.send(&ClientMsg::Window(WinRequest::UnderBar { window: 1 })).map_err(client_error)?;
    }
    if let Some(r) = style_request(app_style, 1) {
        client.send(&ClientMsg::Window(r)).map_err(client_error)?;
    }
    let mut n = Native {
        client,
        gpu,
        cpu: Cpu::new(),
        fonts: Fonts::system(),
        windows: vec![first],
        next_window: 2,
        creating: Vec::new(),
        focus: None,
        input_serial: 0,
        keyboard: None,
        repeat_rate: 0,
        repeat_delay: 0,
        repeat: None,
        clip_kinds: Vec::new(),
        copied: None,
        own_offer_due: false,
        paste_serial: None,
        paste_window: 0,
        paste: None,
        inbox: Vec::new(),
        wake: wake_read.into(),
        wake_pending,
        waker,
    };
    let result = n.run();
    for w in std::mem::take(&mut n.windows) {
        let _ = n.client.send(&ClientMsg::Window(WinRequest::Destroy { window: w.id }));
    }
    result.map(Some)
}

/// The Style request for an app's window, if it says anything a normal window
/// without limits would not.
fn style_request(s: crate::Style, window: u32) -> Option<WinRequest> {
    (s != crate::Style::default()).then_some(WinRequest::Style { window, kind: s.kind, modal: s.modal, min: s.min, max: s.max })
}

/// The Create request for a window run by `app` (its serial filled in by the
/// sender), and whether the server draws a bar the app would otherwise draw.
fn create_request(app: &dyn App, window: u32, serial: u32, size: (u32, u32)) -> (bool, WinRequest) {
    // The server draws every native window's bar (bar milestone 1, step 8):
    // an app that draws its own elsewhere leaves it out here.
    let bar_by_server = app.own_frame();
    let request = WinRequest::Create {
        serial,
        window,
        role: "main".into(),
        token: app.token(),
        title: app.title(),
        width: size.0.max(1) as f64,
        height: size.1.max(1) as f64,
        parent: 0,
        place: app.place(),
    };
    (bar_by_server, request)
}

impl<'a> Win<'a> {
    fn new(id: u32, app: Run<'a>, size: (u32, u32), bar_by_server: bool) -> Win<'a> {
        Win {
            id,
            app,
            created: false,
            opened_by: 0,
            slots: Vec::new(),
            showing: None,
            drawn_by: "",
            shm: None,
            started: Instant::now(),
            width: size.0.max(1),
            height: size.1.max(1),
            scale: 1.0,
            configured: false,
            dirty: false,
            frame_pending: false,
            frames: 0,
            closing: false,
            pointer_at: None,
            cursor: Cursor::Default,
            fingers: Vec::new(),
            timers: Vec::new(),
            bar_by_server,
            inset: 0.0,
            tint: None,
        }
    }
}

/// A new window's size before the server says (the first window's is the program's).
pub(crate) const OPENED_SIZE: (u32, u32) = (820, 520);

impl<'a> Native<'a> {
    fn send(&mut self, r: WinRequest) -> io::Result<()> {
        self.client.send(&ClientMsg::Window(r)).map_err(client_error)
    }

    fn index(&self, id: u32) -> Option<usize> {
        self.windows.iter().position(|w| w.id == id)
    }

    fn run(&mut self) -> io::Result<()> {
        loop {
            // Closed windows go; the program ends with its last.
            let mut i = 0;
            while i < self.windows.len() {
                if self.windows[i].closing {
                    let w = self.windows.remove(i);
                    self.release_window(w)?;
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
            let wait = if self.client.has_buffered() { Some(Duration::ZERO) } else { [repeat, paste, timer].into_iter().flatten().min() };
            let pipe = self.paste.as_ref().map(|p| p.pipe.as_raw_fd());
            let ready = sock::poll_readable(&[Some(self.client.raw_fd()), pipe, Some(self.wake.as_raw_fd())], wait)?;
            if ready[0] && !self.client.read_ready().map_err(client_error)? {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the display server closed the connection"));
            }
            while let Some(m) = self.client.try_event().map_err(client_error)? {
                self.handle(m)?;
            }
            self.read_paste(ready[1]);
            if ready[2] {
                // The flag is cleared first: a wake that comes while the program is being told is not lost.
                let mut drain = [0u8; 64];
                let _ = self.wake.read(&mut drain);
                self.wake_pending.store(false, Ordering::SeqCst);
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
                if w.created && w.configured && w.dirty && !w.frame_pending && !w.closing {
                    self.windows[i].dirty = false;
                    self.draw(i)?;
                }
            }
        }
    }

    /// A window is gone: its program told, its GPU buffers let go and the server told.
    fn release_window(&mut self, mut w: Win<'a>) -> io::Result<()> {
        w.app.get().closed();
        if let Some(gpu) = self.gpu.as_mut() {
            for s in w.slots {
                gpu.release(s.buffer);
            }
        }
        if self.focus == Some(w.id) {
            self.focus = None;
            self.repeat = None;
        }
        self.send(WinRequest::Destroy { window: w.id })
    }

    fn tell(&mut self, id: u32, input: Input) -> io::Result<()> {
        self.with_window(id, |app, window| app.input(&input, window))
    }

    fn with_window(&mut self, id: u32, call: impl FnOnce(&mut dyn App, &mut Window)) -> io::Result<()> {
        let Some(i) = self.index(id) else { return Ok(()) };
        let mut window = Window::default();
        call(self.windows[i].app.get(), &mut window);
        for command in window.commands {
            let Some(i) = self.index(id) else { break };
            match command {
                Command::Redraw => self.windows[i].dirty = true,
                Command::Close => self.windows[i].closing = true,
                // The server's bar and edges move and resize a native window.
                Command::Move | Command::Resize(_) => {}
                Command::Cursor(cursor) => {
                    let w = &mut self.windows[i];
                    if cursor != w.cursor {
                        w.cursor = cursor;
                        if w.pointer_at.is_some() {
                            self.send(WinRequest::Cursor { window: id, shape: cursor.wire() as u16 })?;
                        }
                    }
                }
                Command::Title(title) => self.send(WinRequest::Title { window: id, title })?,
                Command::Maximize(on) => self.send(WinRequest::Maximize { window: id, on })?,
                Command::Size(w, h) => self.send(WinRequest::Size { window: id, width: w as f64, height: h as f64 })?,
                Command::Drag(label, items) => self.send(WinRequest::DragStart { window: id, label, items })?,
                Command::Activate => self.send(WinRequest::Activate { window: id })?,
                Command::ContextMenu(x, y) => {
                    // The menu shows the commands as they are now.
                    if let Some((terminal, commands)) = self.windows[i].app.get().commands() {
                        self.send(WinRequest::Commands { window: id, terminal, commands })?;
                    }
                    self.send(WinRequest::ContextMenu { window: id, x: x as f64, y: y as f64 })?;
                }
                Command::Menu(x, y, lines) => self.send(WinRequest::Menu { window: id, x: x as f64, y: y as f64, lines })?,
                Command::Minimize => self.send(WinRequest::Minimize { window: id })?,
                Command::Copy(text) => {
                    self.send(WinRequest::Copy { input_serial: self.input_serial, kinds: TEXT_KINDS.iter().map(|k| k.to_string()).collect() })?;
                    self.copied = Some(text.into());
                    self.own_offer_due = true;
                }
                Command::Paste => self.start_paste(id)?,
                Command::Timer(timer, at) => set_timer(&mut self.windows[i].timers, timer, at),
                Command::Place(place) => self.send(WinRequest::Place { window: id, place })?,
                Command::Open(app) => self.open(app, id)?,
            }
        }
        Ok(())
    }

    /// Another window of the program, opened by window `opener` (its parent,
    /// for a place it asks relative to it): asked of the server now, run when
    /// it is created.
    fn open(&mut self, mut app: Box<dyn App>, opener: u32) -> io::Result<()> {
        let id = self.next_window;
        self.next_window += 1;
        let serial = self.client.next_serial();
        let size = app.opening_size().map_or(OPENED_SIZE, |(w, h)| (w.clamp(64, 16_384), h.clamp(48, 16_384)));
        let (bar_by_server, mut request) = create_request(app.as_ref(), id, serial, size);
        if let WinRequest::Create { parent, .. } = &mut request {
            *parent = opener;
        }
        app.opened(id);
        app.waker(self.waker.clone());
        self.send(request)?;
        self.creating.push((serial, id));
        let mut win = Win::new(id, Run::Owned(app), size, bar_by_server);
        win.opened_by = opener;
        self.windows.push(win);
        Ok(())
    }

    /// Asks whoever owns the clipboard for its text, for window `id`.
    fn start_paste(&mut self, id: u32) -> io::Result<()> {
        // Our own copy needs no trip through the server.
        if let Some(text) = &self.copied {
            self.inbox.push((id, Input::Paste(text.to_string())));
            return Ok(());
        }
        let Some(kind) = TEXT_KINDS.iter().find(|k| self.clip_kinds.iter().any(|have| have == *k)) else { return Ok(()) };
        let serial = self.client.next_serial();
        self.send(WinRequest::Paste { serial, kind: kind.to_string() })?;
        self.paste_serial = Some(serial);
        self.paste_window = id;
        Ok(())
    }

    /// As lib.rs: what has arrived of a paste; all of it (or too much, or too
    /// late) goes to the window that asked.
    fn read_paste(&mut self, ready: bool) {
        let Some(paste) = &mut self.paste else { return };
        let mut done = paste.started.elapsed() >= PASTE_TIMEOUT;
        if ready {
            let mut chunk = [0u8; 64 * 1024];
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

    fn handle(&mut self, m: ServerMsg) -> io::Result<()> {
        let ev = match m {
            ServerMsg::Window(ev) => ev,
            // Asked whether the loop runs (Force Quit's "not responding"): it does.
            ServerMsg::Alive { serial } => {
                self.client.send(&ClientMsg::Alive { serial }).map_err(client_error)?;
                return Ok(());
            }
            // A paste the server could not serve: nothing comes.
            ServerMsg::Error { serial, .. } if Some(serial) == self.paste_serial => {
                self.paste_serial = None;
                return Ok(());
            }
            // A window the server would not open: it goes, unseen.
            ServerMsg::Error { serial, .. } if self.creating.iter().any(|c| c.0 == serial) => {
                let pos = self.creating.iter().position(|c| c.0 == serial).unwrap();
                let (_, id) = self.creating.remove(pos);
                if let Some(i) = self.index(id) {
                    self.windows.remove(i);
                }
                return Ok(());
            }
            _ => return Ok(()),
        };
        match ev {
            WinEvent::Created { serial, window, token } => {
                if let Some(pos) = self.creating.iter().position(|c| c.0 == serial && c.1 == window) {
                    self.creating.remove(pos);
                    if let Some(i) = self.index(window) {
                        self.windows[i].created = true;
                        self.windows[i].app.get().token_given(&token);
                        if self.windows[i].app.get().under_bar() {
                            self.send(WinRequest::UnderBar { window })?;
                        }
                        // A window another opened always says its kind: until it
                        // does, the server takes it for a dialog (no window memory).
                        let style = self.windows[i].app.get().style();
                        let opened = self.windows[i].opened_by != 0;
                        if let Some(r) = style_request(style, window).or_else(|| opened.then_some(WinRequest::Style { window, kind: style.kind, modal: style.modal, min: style.min, max: style.max })) {
                            self.send(r)?;
                        }
                    }
                }
            }
            WinEvent::Configure { window, serial, width, height, scale, inset, .. } => {
                self.send(WinRequest::AckConfigure { window, serial })?;
                let Some(i) = self.index(window) else { return Ok(()) };
                let w = &mut self.windows[i];
                w.inset = if inset.is_finite() && (0.0..=vergence_protocol::window::MAX_INSET).contains(&inset) { inset as f32 } else { 0.0 };
                if width.is_finite() && height.is_finite() && width >= 1.0 && height >= 1.0 {
                    (w.width, w.height) = (width.round() as u32, height.round() as u32);
                }
                if scale.is_finite() && scale > 0.0 {
                    w.scale = scale;
                }
                w.configured = true;
                w.dirty = true;
            }
            WinEvent::Close { window } => {
                // Closed if its program agrees (App::close_asked).
                let mut yes = false;
                self.with_window(window, |app, w| yes = app.close_asked(w))?;
                if yes && let Some(i) = self.index(window) {
                    self.windows[i].closing = true;
                }
            }
            WinEvent::Frame { window, .. } => {
                let Some(i) = self.index(window) else { return Ok(()) };
                let w = &mut self.windows[i];
                w.frame_pending = false;
                // Every GPU buffer but the one now showing is free again.
                for (j, s) in w.slots.iter_mut().enumerate() {
                    if Some(j) != w.showing {
                        s.busy = false;
                    }
                }
                if w.app.get().animating() {
                    w.dirty = true;
                }
            }
            WinEvent::PointerEnter { window, x, y, .. } => {
                let Some(i) = self.index(window) else { return Ok(()) };
                // Positions in the scene's points: below the strip under the bar.
                let y = y + self.windows[i].inset as f64;
                self.windows[i].pointer_at = Some((x as f32, y as f32));
                // A pointer that enters has no shape until it is given one.
                let shape = self.windows[i].cursor.wire() as u16;
                self.send(WinRequest::Cursor { window, shape })?;
                self.tell(window, Input::PointerMoved { x: x as f32, y: y as f32 })?;
            }
            WinEvent::PointerLeave { window } => {
                if let Some(i) = self.index(window) {
                    self.windows[i].pointer_at = None;
                }
                self.tell(window, Input::PointerLeft)?;
            }
            WinEvent::PointerMotion { window, x, y, .. } => {
                let y = y + self.index(window).map_or(0.0, |i| self.windows[i].inset as f64);
                if let Some(i) = self.index(window) {
                    self.windows[i].pointer_at = Some((x as f32, y as f32));
                }
                self.tell(window, Input::PointerMoved { x: x as f32, y: y as f32 })?;
            }
            WinEvent::PointerButton { window, serial, button, pressed, .. } => {
                let Some(i) = self.index(window) else { return Ok(()) };
                if pressed {
                    self.input_serial = serial;
                }
                let button = match button {
                    0x110 => Button::Left,
                    0x111 => Button::Right,
                    0x112 => Button::Middle,
                    other => Button::Other(other),
                };
                let (x, y) = self.windows[i].pointer_at.unwrap_or_default();
                let modifiers = self.modifiers();
                self.tell(window, Input::Button { button, pressed, x, y, modifiers })?;
            }
            WinEvent::PointerAxis { window, dx, dy, .. } => {
                let Some(i) = self.index(window) else { return Ok(()) };
                let (x, y) = self.windows[i].pointer_at.unwrap_or_default();
                self.tell(window, Input::Scroll { x, y, dx: dx as f32, dy: dy as f32 })?;
            }
            WinEvent::Touch { window, id, phase, x, y, .. } => {
                let Some(i) = self.index(window) else { return Ok(()) };
                let (x, y) = (x as f32, y as f32 + self.windows[i].inset);
                match phase {
                    0 => {
                        // A finger's press is as good as a button's (the clipboard, moving).
                        let fingers = &mut self.windows[i].fingers;
                        fingers.retain(|f| f.0 != id);
                        if fingers.len() < 16 {
                            fingers.push((id, x, y));
                        }
                        self.tell(window, Input::Touch { id, phase: touch::Phase::Down, x, y })?;
                    }
                    1 => {
                        if let Some(f) = self.windows[i].fingers.iter_mut().find(|f| f.0 == id) {
                            (f.1, f.2) = (x, y);
                            self.tell(window, Input::Touch { id, phase: touch::Phase::Moved, x, y })?;
                        }
                    }
                    2 => {
                        if let Some(j) = self.windows[i].fingers.iter().position(|f| f.0 == id) {
                            let (_, x, y) = self.windows[i].fingers.remove(j);
                            self.tell(window, Input::Touch { id, phase: touch::Phase::Up, x, y })?;
                        }
                    }
                    _ => {
                        for (id, x, y) in std::mem::take(&mut self.windows[i].fingers) {
                            self.tell(window, Input::Touch { id, phase: touch::Phase::Cancelled, x, y })?;
                        }
                    }
                }
            }
            WinEvent::Keymap { fd, size } => {
                if let Ok(text) = read_shared(&fd.0, size as usize, MAX_KEYMAP) {
                    self.keyboard = keyboard::Keyboard::from_text(&text);
                }
            }
            WinEvent::Repeat { rate, delay } => (self.repeat_rate, self.repeat_delay) = (rate, delay),
            WinEvent::KeyboardEnter { window, .. } => {
                self.focus = Some(window);
                self.tell(window, Input::Focus(true))?;
            }
            WinEvent::KeyboardLeave { window } => {
                if self.focus == Some(window) {
                    self.focus = None;
                }
                self.repeat = None;
                if let Some(k) = &mut self.keyboard {
                    k.reset_compose();
                }
                self.tell(window, Input::Focus(false))?;
            }
            WinEvent::Modifiers { depressed, latched, locked, group } => {
                if let Some(k) = &mut self.keyboard {
                    k.update(depressed, latched, locked, group);
                }
            }
            WinEvent::Key { window, serial, key, pressed, .. } => {
                if pressed {
                    self.input_serial = serial;
                    let repeats = self.repeat_rate > 0 && self.keyboard.as_ref().is_some_and(|k| k.repeats(key));
                    self.repeat = repeats.then(|| Repeat { key, next: Instant::now() + Duration::from_millis(self.repeat_delay.max(0) as u64) });
                } else if self.repeat.as_ref().is_some_and(|r| r.key == key) {
                    self.repeat = None;
                }
                self.key(window, key, pressed, false)?;
            }
            WinEvent::ClipboardOffer { kinds } => {
                // The offer that follows our own copy keeps it; any other means
                // someone else has the clipboard now.
                if !std::mem::take(&mut self.own_offer_due) {
                    self.copied = None;
                }
                self.clip_kinds = kinds.into_iter().take(crate::MAX_KINDS).filter(|k| k.len() <= crate::MAX_KIND_NAME).collect();
            }
            WinEvent::ClipboardSend { kind, fd } => {
                if let Some(text) = &self.copied
                    && TEXT_KINDS.contains(&kind.as_str())
                {
                    // Written on a thread of its own: the reader may be slow, or never read.
                    let text = text.clone();
                    let pipe = Arc::try_unwrap(fd.0).map_or_else(|a| a.try_clone(), Ok)?;
                    std::thread::spawn(move || {
                        let _ = std::fs::File::from(pipe).write_all(text.as_bytes());
                    });
                }
            }
            WinEvent::ClipboardData { serial, fd } => {
                if self.paste_serial == Some(serial) {
                    self.paste_serial = None;
                    let pipe = Arc::try_unwrap(fd.0).map_or_else(|a| a.try_clone(), Ok)?;
                    self.paste = Some(Paste { pipe: pipe.into(), data: Vec::new(), started: Instant::now() });
                }
            }
            // The bar's events (bar milestone 1): what was done with the tabs the server drew.
            WinEvent::TabChosen { window, tab } => self.with_window(window, |app, w| app.tab(TabEvent::Chosen(tab), w))?,
            WinEvent::BarButton { window, id } => self.with_window(window, |app, w| app.bar_button(id, w))?,
            WinEvent::Command { window, id } => self.with_window(window, |app, w| app.command(id, w))?,
            // Drag and drop (files milestone 1, step 8).
            WinEvent::DragOver { window, x, y, kinds, mods } => {
                let mut action = crate::DROP_NONE;
                self.with_window(window, |app, _| action = app.drag_over(x as f32, y as f32, &kinds, mods & vergence_protocol::window::DRAG_CTRL != 0))?;
                self.send(WinRequest::DragAccept { window, action })?;
            }
            WinEvent::DragLeave { window } => self.with_window(window, |app, _| app.drag_left())?,
            WinEvent::DragDrop { window, x, y, action, items } => self.with_window(window, |app, w| app.dropped(x as f32, y as f32, action, &items, w))?,
            WinEvent::DragEnded { window, action } => self.with_window(window, |app, w| app.drag_ended(action, w))?,
            WinEvent::Tint { window, rgb } => {
                if let Some(i) = self.index(window) {
                    let w = &mut self.windows[i];
                    w.tint = rgb.map(|c| Color::rgb((c >> 16) as u8, (c >> 8) as u8, c as u8));
                    w.dirty = true;
                }
            }
            WinEvent::TabClose { window, tab } => self.with_window(window, |app, w| app.tab(TabEvent::Close(tab), w))?,
            WinEvent::TabCloseOthers { window, tab } => self.with_window(window, |app, w| app.tab(TabEvent::CloseOthers(tab), w))?,
            WinEvent::TabNew { window } => self.with_window(window, |app, w| app.tab(TabEvent::New, w))?,
            WinEvent::TabMoved { window, tab, to } => self.with_window(window, |app, w| app.tab(TabEvent::Moved { tab, to }, w))?,
            WinEvent::TabOut { window, tab, place } => self.with_window(window, |app, w| app.tab(TabEvent::Out { tab, place }, w))?,
            WinEvent::TabJoin { window, tab, into, to } => self.with_window(window, |app, w| app.tab(TabEvent::Join { tab, into, to }, w))?,
            WinEvent::TabMenu { window, tab, entry } => self.with_window(window, |app, w| app.tab(TabEvent::Menu { tab, entry }, w))?,
        }
        Ok(())
    }

    fn key(&mut self, window: u32, key: u32, pressed: bool, repeat: bool) -> io::Result<()> {
        let Some(keyboard) = &mut self.keyboard else { return Ok(()) };
        let modifiers = keyboard.modifiers();
        let (sym, text, composing) = if pressed {
            let p = keyboard.press(key);
            (p.sym, p.text, p.composing)
        } else {
            (keyboard.key(key).0, None, false)
        };
        self.tell(window, Input::Key { sym, text, pressed, repeat, composing, modifiers })
    }

    fn timers_due(&mut self) -> io::Result<()> {
        let now = Instant::now();
        let mut due: Vec<(u32, u64, Instant)> = Vec::new();
        for w in &mut self.windows {
            w.timers.retain(|t| {
                let keep = t.1 > now;
                if !keep {
                    due.push((w.id, t.0, t.1));
                }
                keep
            });
        }
        due.sort_by_key(|t| t.2);
        for (window, id, _) in due {
            self.with_window(window, |app, w| app.timer(id, w))?;
        }
        Ok(())
    }

    fn repeat_due(&mut self) -> io::Result<()> {
        let Some(window) = self.focus else { return Ok(()) };
        let step = Duration::from_secs_f64(1.0 / self.repeat_rate.max(1) as f64);
        for _ in 0..4 {
            let now = Instant::now();
            let Some(r) = self.repeat.as_mut().filter(|r| r.next <= now) else { break };
            r.next = (r.next + step).max(now);
            let key = r.key;
            self.key(window, key, true, true)?;
        }
        Ok(())
    }

    /// Lets go of the GPU renderer and every window's buffers, for good: software from here on.
    fn give_up_gpu(&mut self) {
        if let Some(mut gpu) = self.gpu.take() {
            for w in &mut self.windows {
                for s in w.slots.drain(..) {
                    gpu.release(s.buffer);
                }
                w.showing = None;
                w.dirty = true;
            }
        }
    }

    /// Draws the scene into a free GPU buffer of this size and attaches it,
    /// for window `i`. Ok(false): not this time (draw in software).
    fn draw_on_gpu(&mut self, i: usize, scene: &Scene, scale: f32, background: Color, pw: u32, ph: u32) -> io::Result<bool> {
        let Some(gpu) = self.gpu.as_mut() else { return Ok(false) };
        let w = &mut self.windows[i];
        // Buffers of another size the server is done with are let go.
        let mut j = 0;
        while j < w.slots.len() {
            let s = &w.slots[j];
            if !s.busy && (s.buffer.width, s.buffer.height) != (pw, ph) {
                let old = w.slots.remove(j);
                gpu.release(old.buffer);
                if let Some(sh) = w.showing.as_mut() {
                    if *sh == j {
                        w.showing = None;
                    } else if *sh > j {
                        *sh -= 1;
                    }
                }
            } else {
                j += 1;
            }
        }
        let fits = |s: &Slot| (s.buffer.width, s.buffer.height) == (pw, ph);
        let index = match w.slots.iter().position(|s| !s.busy && fits(s)) {
            Some(k) => k,
            None if w.slots.iter().filter(|s| fits(s)).count() < GPU_BUFFERS => match gpu.buffer(pw, ph) {
                Ok(buffer) => {
                    w.slots.push(Slot { buffer, busy: false });
                    w.slots.len() - 1
                }
                Err(_) => {
                    self.give_up_gpu();
                    return Ok(false);
                }
            },
            // All busy: this frame in software.
            None => return Ok(false),
        };
        match gpu.draw_into(&w.slots[index].buffer, scene, &mut self.fonts, scale, background) {
            Ok(()) => {}
            Err(GpuError::Unsupported(_)) => return Ok(false),
            Err(_) => {
                self.give_up_gpu();
                return Ok(false);
            }
        }
        let b = &w.slots[index].buffer;
        let plane = Plane { fd: Fd(Arc::new(b.fd.try_clone()?)), offset: b.offset, stride: b.stride };
        let buffer = Buffer::Dmabuf { planes: vec![plane], width: b.width, height: b.height, format: b.format, modifier: b.modifier };
        let id = w.id;
        w.slots[index].busy = true;
        w.showing = Some(index);
        self.send(WinRequest::Attach { window: id, buffer, damage: Vec::new() })?;
        Ok(true)
    }

    fn draw(&mut self, i: usize) -> io::Result<()> {
        let t = Instant::now();
        let w = &mut self.windows[i];
        let frame = Frame { width: w.width as f32, height: w.height as f32 + w.inset, scale: w.scale as f32, time: w.started.elapsed().as_secs_f64(), number: w.frames, drawn_by: w.drawn_by, bar_by_server: w.bar_by_server, top: w.inset , tint: w.tint };
        let id = w.id;
        let scene = w.app.get().scene(&frame, &mut self.fonts);
        let background = self.windows[i].app.get().background();
        let (pw, ph) = pixel_size(&scene, frame.scale).map_err(other)?;
        if self.draw_on_gpu(i, &scene, frame.scale, background, pw, ph)? {
            self.windows[i].drawn_by = "GPU";
        } else {
            let w = &mut self.windows[i];
            w.drawn_by = "CPU";
            let alpha = background.a != 255;
            let stride = pw as usize * 4;
            if !w.shm.as_ref().is_some_and(|(_, sw, sh, a)| (*sw, *sh, *a) == (pw, ph, alpha)) {
                w.shm = Some((ShmFile::new(stride * ph as usize)?, pw, ph, alpha));
            }
            let (file, ..) = w.shm.as_mut().unwrap();
            // Drawn in place in the server's byte order; the server copies it at attach.
            let target = Target { data: file.bytes(), width: pw, height: ph, order: Order::Bgra };
            self.cpu.render_target(&scene, &mut self.fonts, frame.scale, background, target, 0..ph).map_err(other)?;
            // SAFETY: the ShmFile's descriptor is open while the window's shm holds it, which outlives this borrow.
            let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(file.fd()) }.try_clone_to_owned()?;
            let format = if alpha { SHM_ARGB8888 } else { SHM_XRGB8888 };
            let buffer = Buffer::Shm { fd: Fd(Arc::new(fd)), width: pw, height: ph, stride: stride as u32, format };
            w.showing = None;
            self.send(WinRequest::Attach { window: id, buffer, damage: Vec::new() })?;
        }
        let w = &mut self.windows[i];
        w.frame_pending = true;
        w.frames += 1;
        if !w.app.get().shown(&frame, t.elapsed().as_secs_f64() * 1e3) {
            w.closing = true;
        }
        // The bar, when the server draws it and it has changed (bar milestone 1).
        if w.app.get().tabs_by_server()
            && let Some(bar) = w.app.get().bar()
        {
            self.send(WinRequest::Bar { window: id, bar })?;
        }
        // The window's meaning, when it has changed (Contract 3).
        if let Some(tree) = self.windows[i].app.get().meaning() {
            self.send(meaning_request(id, &tree))?;
        }
        // Its app menu and bar style, when they have changed (app settings milestone 1).
        if let Some((terminal, commands)) = self.windows[i].app.get().commands() {
            self.send(WinRequest::Commands { window: id, terminal, commands })?;
        }
        if let Some(style) = self.windows[i].app.get().bar_style() {
            self.send(WinRequest::BarStyle { window: id, style })?;
        }
        Ok(())
    }
}

/// The most of a meaning message's frame its nodes may fill (the protocol's
/// frames are at most 64 KB), and the longest name or value sent. A tree
/// that does not fit is cut at a node (a first version: Contract 3's tree in
/// pieces comes when a window needs more).
const MEANING_BUDGET: usize = 60 * 1024;
const MEANING_TEXT: usize = 512;

/// The tree as the protocol's Meaning request, within its limits.
fn meaning_request(window: u32, tree: &MeaningTree) -> WinRequest {
    let cut = |s: &str| -> String { s.chars().take(MEANING_TEXT).collect() };
    let mut nodes = Vec::new();
    let mut used = 64;
    for n in tree.nodes.iter().take(MAX_NODES) {
        let node = Node {
            id: n.id,
            parent: n.parent,
            role: cut(&n.role),
            name: cut(&n.name),
            value: n.value.as_deref().map(cut),
            states: n.states.iter().take(MAX_STATES).map(|s| cut(s)).collect(),
            x: n.x as f64,
            y: n.y as f64,
            w: n.w as f64,
            h: n.h as f64,
        };
        // Its share of the frame: ids and bounds, the strings with their lengths.
        let size = 8 + 8 + 32 + 4 * (3 + node.states.len()) + 1 + node.role.len() + node.name.len() + node.value.as_ref().map_or(0, String::len) + node.states.iter().map(String::len).sum::<usize>();
        if used + size > MEANING_BUDGET {
            break;
        }
        used += size;
        nodes.push(node);
    }
    let caret = tree.caret.map(|(x, y, w, h)| (x as f64, y as f64, w as f64, h as f64));
    WinRequest::Meaning { window, focus: tree.focus, caret, nodes }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(i: u64, name_len: usize) -> crate::MeaningNode {
        crate::MeaningNode { id: i + 1, parent: 0, role: "label".into(), name: "n".repeat(name_len), value: Some("v".repeat(name_len)), states: vec!["focused".into(); 20], x: 0.0, y: 0.0, w: 1.0, h: 1.0 }
    }

    #[test]
    fn a_tree_too_big_for_one_message_is_cut_to_fit_and_still_encodes() {
        // 5,000 widgets with 2,000-character names: far past the 64 KB frame.
        let tree = MeaningTree { focus: 1, caret: None, nodes: (0..5000).map(|i| node(i, 2000)).collect() };
        let req = meaning_request(1, &tree);
        let WinRequest::Meaning { nodes, .. } = &req else { panic!("not a Meaning") };
        assert!(!nodes.is_empty() && nodes.len() < 5000, "{} nodes kept", nodes.len());
        assert!(nodes.iter().all(|n| n.name.chars().count() <= MEANING_TEXT && n.states.len() <= MAX_STATES));
        let bytes = ClientMsg::Window(req).encode().expect("fits one frame");
        assert!(bytes.len() <= 64 * 1024, "{} bytes", bytes.len());
    }

    #[test]
    fn a_small_tree_goes_whole() {
        let tree = MeaningTree { focus: 2, caret: Some((1.0, 2.0, 1.0, 14.0)), nodes: (0..40).map(|i| node(i, 12)).collect() };
        let WinRequest::Meaning { focus, caret, nodes, .. } = meaning_request(1, &tree) else { panic!() };
        assert_eq!((focus, nodes.len(), caret), (2, 40, Some((1.0, 2.0, 1.0, 14.0))));
    }
}
