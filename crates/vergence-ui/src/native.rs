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

/// The one window a toolkit program has, by the id it gives it.
const WINDOW: u32 = 1;

/// A GPU buffer and whether the server may still be reading it.
struct Slot {
    buffer: GpuBuffer,
    busy: bool,
}

struct Native {
    client: Client,
    gpu: Option<Gpu>,
    slots: Vec<Slot>,
    /// The slot attached last: the one the server shows until the next.
    showing: Option<usize>,
    drawn_by: &'static str,
    cpu: Cpu,
    fonts: Fonts,
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
    /// The serial of the last button press (to move or resize) and of the
    /// last key or button press (to copy).
    press_serial: u32,
    input_serial: u32,
    cursor: Cursor,
    fingers: Vec<(i32, f32, f32)>,
    keyboard: Option<keyboard::Keyboard>,
    repeat_rate: i32,
    repeat_delay: i32,
    repeat: Option<Repeat>,

    /// What the clipboard holds, as the server last offered it.
    clip_kinds: Vec<String>,
    /// What we put there, and whether the offer echoing our copy is yet to come.
    copied: Option<Arc<str>>,
    own_offer_due: bool,
    /// The paste asked for (its serial), and the text on its way in.
    paste_serial: Option<u32>,
    paste: Option<Paste>,
    inbox: Vec<Input>,
    wake: std::fs::File,
    wake_pending: Arc<AtomicBool>,
    timers: Vec<(u64, Instant)>,
    /// The server draws this window's tabbed bar (App::tabs_by_server).
    bar_by_server: bool,
}

fn client_error(e: ClientError) -> io::Error {
    match e {
        ClientError::Io(e) => e,
        other => io::Error::other(other.to_string()),
    }
}

/// Opens the window natively and runs until it is closed. Ok(None): not in
/// a Vergence session, or the server would not open a window this way; the
/// program has not been called, and the caller opens it over Wayland.
pub(crate) fn run(app: &mut dyn App, size: (u32, u32)) -> io::Result<Option<()>> {
    if std::env::var("VERGENCE_UI_WAYLAND").as_deref() == Ok("1") || std::env::var_os(vergence_protocol::BUS_ENV).is_none() {
        return Ok(None);
    }
    let Ok(mut client) = Client::connect(&app.app_id()) else { return Ok(None) };
    // A window whose tabs the server may draw leaves its frame to the server here.
    let bar_by_server = app.own_frame() && app.tabs_by_server();
    let (title, own_frame, place) = (app.title(), app.own_frame() && !bar_by_server, app.place());
    let created = client.request(|serial| {
        ClientMsg::Window(WinRequest::Create {
            serial,
            window: WINDOW,
            role: "main".into(),
            token: String::new(),
            title,
            width: size.0.max(1) as f64,
            height: size.1.max(1) as f64,
            own_frame,
            parent: 0,
            place,
        })
    });
    match created {
        Ok(ServerMsg::Window(WinEvent::Created { .. })) => {}
        // A server without native windows says "not yet", or something else:
        // Wayland it is.
        _ => return Ok(None),
    }
    let gpu = (std::env::var("VERGENCE_RENDER").as_deref() != Ok("cpu")).then(|| Gpu::new().ok()).flatten().filter(Gpu::shares_buffers);
    let (wake_read, wake_write) = sock::pipe()?;
    let wake_pending = Arc::new(AtomicBool::new(false));
    app.waker(Waker { pipe: Arc::new(wake_write.into()), pending: wake_pending.clone() });
    let mut n = Native {
        client,
        gpu,
        slots: Vec::new(),
        showing: None,
        drawn_by: "",
        cpu: Cpu::new(),
        fonts: Fonts::system(),
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
        press_serial: 0,
        input_serial: 0,
        cursor: Cursor::Default,
        fingers: Vec::new(),
        keyboard: None,
        repeat_rate: 0,
        repeat_delay: 0,
        repeat: None,
        clip_kinds: Vec::new(),
        copied: None,
        own_offer_due: false,
        paste_serial: None,
        paste: None,
        inbox: Vec::new(),
        wake: wake_read.into(),
        wake_pending,
        timers: Vec::new(),
        bar_by_server,
    };
    let result = n.run(app);
    let _ = n.client.send(&ClientMsg::Window(WinRequest::Destroy { window: WINDOW }));
    result.map(Some)
}

impl Native {
    fn send(&mut self, r: WinRequest) -> io::Result<()> {
        self.client.send(&ClientMsg::Window(r)).map_err(client_error)
    }

    fn run(&mut self, app: &mut dyn App) -> io::Result<()> {
        while !self.closing {
            // Wait for the server, but no longer than the next key repeat, the
            // paste's limit or the next timer.
            let now = Instant::now();
            let repeat = self.repeat.as_ref().map(|r| r.next.saturating_duration_since(now));
            let paste = self.paste.as_ref().map(|p| (p.started + PASTE_TIMEOUT).saturating_duration_since(now));
            let timer = self.timers.iter().map(|t| t.1).min().map(|t| t.saturating_duration_since(now));
            let wait = if self.client.has_buffered() { Some(Duration::ZERO) } else { [repeat, paste, timer].into_iter().flatten().min() };
            let pipe = self.paste.as_ref().map(|p| p.pipe.as_raw_fd());
            let ready = sock::poll_readable(&[Some(self.client.raw_fd()), pipe, Some(self.wake.as_raw_fd())], wait)?;
            if ready[0] && !self.client.read_ready().map_err(client_error)? {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the display server closed the connection"));
            }
            while let Some(m) = self.client.try_event().map_err(client_error)? {
                self.handle(m, app)?;
            }
            self.read_paste(ready[1]);
            if ready[2] {
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
            self.timers_due(app)?;
            if self.configured && self.dirty && !self.frame_pending && !self.closing {
                self.dirty = false;
                self.draw(app)?;
            }
        }
        Ok(())
    }

    fn tell(&mut self, app: &mut dyn App, input: Input) -> io::Result<()> {
        self.with_window(app, |app, window| app.input(&input, window))
    }

    fn with_window(&mut self, app: &mut dyn App, call: impl FnOnce(&mut dyn App, &mut Window)) -> io::Result<()> {
        let mut window = Window::default();
        call(app, &mut window);
        for command in window.commands {
            match command {
                Command::Redraw => self.dirty = true,
                Command::Close => self.closing = true,
                Command::Move => self.send(WinRequest::Move { window: WINDOW, input_serial: self.press_serial })?,
                Command::Resize(edges) => self.send(WinRequest::Resize { window: WINDOW, input_serial: self.press_serial, edges: edges.wire() as u8 })?,
                Command::Cursor(cursor) => {
                    if cursor != self.cursor {
                        self.cursor = cursor;
                        if self.pointer_at.is_some() {
                            self.send(WinRequest::Cursor { window: WINDOW, shape: cursor.wire() as u16 })?;
                        }
                    }
                }
                Command::Title(title) => self.send(WinRequest::Title { window: WINDOW, title })?,
                Command::Maximize(on) => self.send(WinRequest::Maximize { window: WINDOW, on })?,
                Command::Minimize => self.send(WinRequest::Minimize { window: WINDOW })?,
                Command::Copy(text) => {
                    self.send(WinRequest::Copy { input_serial: self.input_serial, kinds: TEXT_KINDS.iter().map(|k| k.to_string()).collect() })?;
                    self.copied = Some(text.into());
                    self.own_offer_due = true;
                }
                Command::Paste => self.start_paste()?,
                Command::Timer(id, at) => set_timer(&mut self.timers, id, at),
                Command::Place(place) => self.send(WinRequest::Place { window: WINDOW, place })?,
            }
        }
        Ok(())
    }

    /// Asks whoever owns the clipboard for its text.
    fn start_paste(&mut self) -> io::Result<()> {
        // Our own copy needs no trip through the server.
        if let Some(text) = &self.copied {
            self.inbox.push(Input::Paste(text.to_string()));
            return Ok(());
        }
        let Some(kind) = TEXT_KINDS.iter().find(|k| self.clip_kinds.iter().any(|have| have == *k)) else { return Ok(()) };
        let serial = self.client.next_serial();
        self.send(WinRequest::Paste { serial, kind: kind.to_string() })?;
        self.paste_serial = Some(serial);
        Ok(())
    }

    /// As lib.rs: what has arrived of a paste; all of it (or too much, or too
    /// late) goes to the program.
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
                self.inbox.push(Input::Paste(String::from_utf8_lossy(&paste.data).into_owned()));
            }
        }
    }

    fn modifiers(&self) -> Modifiers {
        self.keyboard.as_ref().map(keyboard::Keyboard::modifiers).unwrap_or_default()
    }

    fn handle(&mut self, m: ServerMsg, app: &mut dyn App) -> io::Result<()> {
        let ev = match m {
            ServerMsg::Window(ev) => ev,
            // A paste the server could not serve: nothing comes.
            ServerMsg::Error { serial, .. } if Some(serial) == self.paste_serial => {
                self.paste_serial = None;
                return Ok(());
            }
            _ => return Ok(()),
        };
        match ev {
            WinEvent::Configure { serial, width, height, scale, .. } => {
                self.send(WinRequest::AckConfigure { window: WINDOW, serial })?;
                if width.is_finite() && height.is_finite() && width >= 1.0 && height >= 1.0 {
                    (self.width, self.height) = (width.round() as u32, height.round() as u32);
                }
                if scale.is_finite() && scale > 0.0 {
                    self.scale = scale;
                }
                self.configured = true;
                self.dirty = true;
            }
            WinEvent::Close { .. } => self.closing = true,
            WinEvent::Frame { .. } => {
                self.frame_pending = false;
                // Every GPU buffer but the one now showing is free again.
                for (i, s) in self.slots.iter_mut().enumerate() {
                    if Some(i) != self.showing {
                        s.busy = false;
                    }
                }
                if app.animating() {
                    self.dirty = true;
                }
            }
            WinEvent::PointerEnter { x, y, .. } => {
                self.pointer_at = Some((x as f32, y as f32));
                // A pointer that enters has no shape until it is given one.
                self.send(WinRequest::Cursor { window: WINDOW, shape: self.cursor.wire() as u16 })?;
                self.tell(app, Input::PointerMoved { x: x as f32, y: y as f32 })?;
            }
            WinEvent::PointerLeave { .. } => {
                self.pointer_at = None;
                self.tell(app, Input::PointerLeft)?;
            }
            WinEvent::PointerMotion { x, y, .. } => {
                self.pointer_at = Some((x as f32, y as f32));
                self.tell(app, Input::PointerMoved { x: x as f32, y: y as f32 })?;
            }
            WinEvent::PointerButton { serial, button, pressed, .. } => {
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
            WinEvent::PointerAxis { dx, dy, .. } => {
                let (x, y) = self.pointer_at.unwrap_or_default();
                self.tell(app, Input::Scroll { x, y, dx: dx as f32, dy: dy as f32 })?;
            }
            WinEvent::Touch { id, phase, x, y, .. } => {
                let (x, y) = (x as f32, y as f32);
                match phase {
                    0 => {
                        // A finger's press is as good as a button's (the clipboard, moving).
                        self.fingers.retain(|f| f.0 != id);
                        if self.fingers.len() < 16 {
                            self.fingers.push((id, x, y));
                        }
                        self.tell(app, Input::Touch { id, phase: touch::Phase::Down, x, y })?;
                    }
                    1 => {
                        if let Some(f) = self.fingers.iter_mut().find(|f| f.0 == id) {
                            (f.1, f.2) = (x, y);
                            self.tell(app, Input::Touch { id, phase: touch::Phase::Moved, x, y })?;
                        }
                    }
                    2 => {
                        if let Some(i) = self.fingers.iter().position(|f| f.0 == id) {
                            let (_, x, y) = self.fingers.remove(i);
                            self.tell(app, Input::Touch { id, phase: touch::Phase::Up, x, y })?;
                        }
                    }
                    _ => {
                        for (id, x, y) in std::mem::take(&mut self.fingers) {
                            self.tell(app, Input::Touch { id, phase: touch::Phase::Cancelled, x, y })?;
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
            WinEvent::KeyboardEnter { .. } => self.tell(app, Input::Focus(true))?,
            WinEvent::KeyboardLeave { .. } => {
                self.repeat = None;
                if let Some(k) = &mut self.keyboard {
                    k.reset_compose();
                }
                self.tell(app, Input::Focus(false))?;
            }
            WinEvent::Modifiers { depressed, latched, locked, group } => {
                if let Some(k) = &mut self.keyboard {
                    k.update(depressed, latched, locked, group);
                }
            }
            WinEvent::Key { serial, key, pressed, .. } => {
                if pressed {
                    self.input_serial = serial;
                    let repeats = self.repeat_rate > 0 && self.keyboard.as_ref().is_some_and(|k| k.repeats(key));
                    self.repeat = repeats.then(|| Repeat { key, next: Instant::now() + Duration::from_millis(self.repeat_delay.max(0) as u64) });
                } else if self.repeat.as_ref().is_some_and(|r| r.key == key) {
                    self.repeat = None;
                }
                self.key(app, key, pressed, false)?;
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
            WinEvent::Created { .. } => {}
            // The bar's events (bar milestone 1): what was done with the tabs the server drew.
            WinEvent::TabChosen { tab, .. } => self.with_window(app, |app, w| app.tab(TabEvent::Chosen(tab), w))?,
            WinEvent::TabClose { tab, .. } => self.with_window(app, |app, w| app.tab(TabEvent::Close(tab), w))?,
            WinEvent::TabCloseOthers { tab, .. } => self.with_window(app, |app, w| app.tab(TabEvent::CloseOthers(tab), w))?,
            WinEvent::TabNew { .. } => self.with_window(app, |app, w| app.tab(TabEvent::New, w))?,
            WinEvent::TabMoved { tab, to, .. } => self.with_window(app, |app, w| app.tab(TabEvent::Moved { tab, to }, w))?,
            WinEvent::TabOut { tab, place, .. } => self.with_window(app, |app, w| app.tab(TabEvent::Out { tab, place }, w))?,
            WinEvent::TabJoin { tab, into, to, .. } => self.with_window(app, |app, w| app.tab(TabEvent::Join { tab, into, to }, w))?,
            WinEvent::TabMenu { tab, entry, .. } => self.with_window(app, |app, w| app.tab(TabEvent::Menu { tab, entry }, w))?,
        }
        Ok(())
    }

    fn key(&mut self, app: &mut dyn App, key: u32, pressed: bool, repeat: bool) -> io::Result<()> {
        let Some(keyboard) = &mut self.keyboard else { return Ok(()) };
        let modifiers = keyboard.modifiers();
        let (sym, text, composing) = if pressed {
            let p = keyboard.press(key);
            (p.sym, p.text, p.composing)
        } else {
            (keyboard.key(key).0, None, false)
        };
        self.tell(app, Input::Key { sym, text, pressed, repeat, composing, modifiers })
    }

    fn timers_due(&mut self, app: &mut dyn App) -> io::Result<()> {
        let now = Instant::now();
        let mut due: Vec<(u64, Instant)> = Vec::new();
        self.timers.retain(|t| {
            let keep = t.1 > now;
            if !keep {
                due.push(*t);
            }
            keep
        });
        due.sort_by_key(|t| t.1);
        for (id, _) in due {
            self.with_window(app, |app, window| app.timer(id, window))?;
        }
        Ok(())
    }

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

    /// Lets go of the GPU renderer and its buffers, for good: software from here on.
    fn give_up_gpu(&mut self) {
        if let Some(mut gpu) = self.gpu.take() {
            for s in self.slots.drain(..) {
                gpu.release(s.buffer);
            }
        }
        self.showing = None;
        self.dirty = true;
    }

    /// Draws the scene into a free GPU buffer of this size and attaches it.
    /// Ok(false): not this time (draw in software).
    fn draw_on_gpu(&mut self, scene: &Scene, scale: f32, background: Color, pw: u32, ph: u32) -> io::Result<bool> {
        let Some(gpu) = self.gpu.as_mut() else { return Ok(false) };
        // Buffers of another size the server is done with are let go.
        let mut i = 0;
        while i < self.slots.len() {
            let s = &self.slots[i];
            if !s.busy && (s.buffer.width, s.buffer.height) != (pw, ph) {
                let old = self.slots.remove(i);
                gpu.release(old.buffer);
                if let Some(sh) = self.showing.as_mut() {
                    if *sh == i {
                        self.showing = None;
                    } else if *sh > i {
                        *sh -= 1;
                    }
                }
            } else {
                i += 1;
            }
        }
        let fits = |s: &Slot| (s.buffer.width, s.buffer.height) == (pw, ph);
        let index = match self.slots.iter().position(|s| !s.busy && fits(s)) {
            Some(i) => i,
            None if self.slots.iter().filter(|s| fits(s)).count() < GPU_BUFFERS => match gpu.buffer(pw, ph) {
                Ok(buffer) => {
                    self.slots.push(Slot { buffer, busy: false });
                    self.slots.len() - 1
                }
                Err(_) => {
                    self.give_up_gpu();
                    return Ok(false);
                }
            },
            // All busy: this frame in software.
            None => return Ok(false),
        };
        match gpu.draw_into(&self.slots[index].buffer, scene, &mut self.fonts, scale, background) {
            Ok(()) => {}
            Err(GpuError::Unsupported(_)) => return Ok(false),
            Err(_) => {
                self.give_up_gpu();
                return Ok(false);
            }
        }
        let b = &self.slots[index].buffer;
        let plane = Plane { fd: Fd(Arc::new(b.fd.try_clone()?)), offset: b.offset, stride: b.stride };
        let buffer = Buffer::Dmabuf { planes: vec![plane], width: b.width, height: b.height, format: b.format, modifier: b.modifier };
        self.send(WinRequest::Attach { window: WINDOW, buffer, damage: Vec::new() })?;
        self.slots[index].busy = true;
        self.showing = Some(index);
        Ok(true)
    }

    fn draw(&mut self, app: &mut dyn App) -> io::Result<()> {
        let t = Instant::now();
        let frame = Frame { width: self.width as f32, height: self.height as f32, scale: self.scale as f32, time: self.started.elapsed().as_secs_f64(), number: self.frames, drawn_by: self.drawn_by, bar_by_server: self.bar_by_server };
        let scene = app.scene(&frame, &mut self.fonts);
        let background = app.background();
        let (pw, ph) = pixel_size(&scene, frame.scale).map_err(other)?;
        if self.draw_on_gpu(&scene, frame.scale, background, pw, ph)? {
            self.drawn_by = "GPU";
        } else {
            self.drawn_by = "CPU";
            let alpha = background.a != 255;
            let stride = pw as usize * 4;
            if !self.shm.as_ref().is_some_and(|(_, w, h, a)| (*w, *h, *a) == (pw, ph, alpha)) {
                self.shm = Some((ShmFile::new(stride * ph as usize)?, pw, ph, alpha));
            }
            let (file, ..) = self.shm.as_mut().unwrap();
            // Drawn in place in the server's byte order; the server copies it at attach.
            let target = Target { data: file.bytes(), width: pw, height: ph, order: Order::Bgra };
            self.cpu.render_target(&scene, &mut self.fonts, frame.scale, background, target, 0..ph).map_err(other)?;
            // SAFETY: the ShmFile's descriptor is open while `self.shm` holds it, which outlives this borrow.
            let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(file.fd()) }.try_clone_to_owned()?;
            let format = if alpha { SHM_ARGB8888 } else { SHM_XRGB8888 };
            let buffer = Buffer::Shm { fd: Fd(Arc::new(fd)), width: pw, height: ph, stride: stride as u32, format };
            self.send(WinRequest::Attach { window: WINDOW, buffer, damage: Vec::new() })?;
            self.showing = None;
        }
        self.frame_pending = true;
        self.frames += 1;
        if !app.shown(&frame, t.elapsed().as_secs_f64() * 1e3) {
            self.closing = true;
        }
        // The bar, when the server draws it and it has changed (bar milestone 1).
        if self.bar_by_server
            && let Some(bar) = app.bar()
        {
            self.send(WinRequest::Bar { window: WINDOW, bar })?;
        }
        // The window's meaning, when it has changed (Contract 3).
        if let Some(tree) = app.meaning() {
            self.send(meaning_request(&tree))?;
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
fn meaning_request(tree: &MeaningTree) -> WinRequest {
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
    WinRequest::Meaning { window: WINDOW, focus: tree.focus, caret, nodes }
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
        let req = meaning_request(&tree);
        let WinRequest::Meaning { nodes, .. } = &req else { panic!("not a Meaning") };
        assert!(!nodes.is_empty() && nodes.len() < 5000, "{} nodes kept", nodes.len());
        assert!(nodes.iter().all(|n| n.name.chars().count() <= MEANING_TEXT && n.states.len() <= MAX_STATES));
        let bytes = ClientMsg::Window(req).encode().expect("fits one frame");
        assert!(bytes.len() <= 64 * 1024, "{} bytes", bytes.len());
    }

    #[test]
    fn a_small_tree_goes_whole() {
        let tree = MeaningTree { focus: 2, caret: Some((1.0, 2.0, 1.0, 14.0)), nodes: (0..40).map(|i| node(i, 12)).collect() };
        let WinRequest::Meaning { focus, caret, nodes, .. } = meaning_request(&tree) else { panic!() };
        assert_eq!((focus, nodes.len(), caret), (2, 40, Some((1.0, 2.0, 1.0, 14.0))));
    }
}
