//! The protocol itself: the requests the toolkit makes and the events it is
//! told, by name.
//!
//! Wayland describes its interfaces in XML and most clients generate code
//! from it. This file is written by hand from the protocol's documents
//! instead, and holds only what the toolkit uses: a window with a surface,
//! shared-memory buffers, the exact scale (wp-fractional-scale, wp-viewporter)
//! and who draws the title bar (xdg-decoration), the pointer and keyboard
//! (wl_seat), the pointer's shape (wp-cursor-shape) and the clipboard
//! (wl_data_device), and buffers drawn by the GPU (linux-dmabuf).
//!
//! [`Client`] remembers which interface each object id belongs to, so an
//! incoming message can be read as the right event. An event for an object
//! or of a kind it does not know is passed over, never guessed at.

use std::{
    collections::HashMap,
    io,
    os::fd::{OwnedFd, RawFd},
    time::Duration,
};

use crate::{
    conn::Connection,
    wire::{Reader, WireError, Writer},
};

/// The interfaces this client can speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interface {
    Display,
    Registry,
    Callback,
    Compositor,
    Surface,
    Shm,
    ShmPool,
    Buffer,
    WmBase,
    XdgSurface,
    Toplevel,
    Viewporter,
    Viewport,
    FractionalScaleManager,
    FractionalScale,
    DecorationManager,
    Decoration,
    Seat,
    Pointer,
    Keyboard,
    CursorShapeManager,
    CursorShapeDevice,
    DataDeviceManager,
    DataDevice,
    DataSource,
    /// Made by the server: what another program has put on the clipboard.
    DataOffer,
    /// linux-dmabuf: buffers that live in the GPU's memory.
    Dmabuf,
    DmabufParams,
}

impl Interface {
    /// The interface's name in the registry, for those that are bound from it.
    pub fn global_name(self) -> Option<&'static str> {
        Some(match self {
            Interface::Compositor => "wl_compositor",
            Interface::Shm => "wl_shm",
            Interface::WmBase => "xdg_wm_base",
            Interface::Viewporter => "wp_viewporter",
            Interface::FractionalScaleManager => "wp_fractional_scale_manager_v1",
            Interface::DecorationManager => "zxdg_decoration_manager_v1",
            Interface::Seat => "wl_seat",
            Interface::CursorShapeManager => "wp_cursor_shape_manager_v1",
            Interface::DataDeviceManager => "wl_data_device_manager",
            Interface::Dmabuf => "zwp_linux_dmabuf_v1",
            _ => return None,
        })
    }

    /// The newest version of it this file was written against.
    pub fn version(self) -> u32 {
        match self {
            Interface::Compositor => 4,
            // 5: pointer events come in frames, and the keyboard says its repeat rate.
            Interface::Seat => 5,
            Interface::DataDeviceManager => 3,
            // 3: formats come with their modifiers, as plain events.
            Interface::Dmabuf => 3,
            _ => 1,
        }
    }
}

/// `wl_shm` pixel formats: 32 bits a pixel, blue first in memory, with and
/// without alpha.
pub const FORMAT_ARGB8888: u32 = 0;
pub const FORMAT_XRGB8888: u32 = 1;

/// `zxdg_toplevel_decoration_v1` modes: who draws the title bar.
pub const DECORATION_CLIENT: u32 = 1;
pub const DECORATION_SERVER: u32 = 2;

/// `xdg_toplevel` states a configure may list.
pub const STATE_MAXIMIZED: u32 = 1;
pub const STATE_FULLSCREEN: u32 = 2;
pub const STATE_RESIZING: u32 = 3;
pub const STATE_ACTIVATED: u32 = 4;

/// `wl_seat` capabilities.
pub const SEAT_POINTER: u32 = 1;
pub const SEAT_KEYBOARD: u32 = 2;

/// `xdg_toplevel.resize` edges.
pub const EDGE_TOP: u32 = 1;
pub const EDGE_BOTTOM: u32 = 2;
pub const EDGE_LEFT: u32 = 4;
pub const EDGE_RIGHT: u32 = 8;

/// `wp_cursor_shape_device_v1` shapes (the ones the toolkit uses).
pub const SHAPE_DEFAULT: u32 = 1;
pub const SHAPE_POINTER: u32 = 4;
pub const SHAPE_TEXT: u32 = 9;
pub const SHAPE_MOVE: u32 = 13;
pub const SHAPE_GRAB: u32 = 16;
pub const SHAPE_E_RESIZE: u32 = 18;
pub const SHAPE_N_RESIZE: u32 = 19;
pub const SHAPE_NE_RESIZE: u32 = 20;
pub const SHAPE_NW_RESIZE: u32 = 21;
pub const SHAPE_S_RESIZE: u32 = 22;
pub const SHAPE_SE_RESIZE: u32 = 23;
pub const SHAPE_SW_RESIZE: u32 = 24;
pub const SHAPE_W_RESIZE: u32 = 25;

/// What the server told us.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The server refuses to go on: the object at fault, a code, and why.
    Error { object: u32, code: u32, message: String },
    /// Something the server offers: bind it with [`Client::bind`].
    Global { name: u32, interface: String, version: u32 },
    GlobalRemove { name: u32 },
    /// A callback fired (a sync point reached, or time to draw a frame).
    Done { callback: u32, data: u32 },
    ShmFormat { format: u32 },
    /// The server has finished with a buffer: it may be drawn into again.
    BufferRelease { buffer: u32 },
    Ping { serial: u32 },
    /// The window's new state is complete; acknowledge it, then draw.
    Configure { xdg_surface: u32, serial: u32 },
    /// The size the server suggests (0: the window's own choice) and its states.
    ToplevelConfigure { toplevel: u32, width: i32, height: i32, states: Vec<u32> },
    /// The user asked for the window to be closed.
    Close { toplevel: u32 },
    /// The scale to draw at, in 120ths.
    PreferredScale { object: u32, scale120: u32 },
    DecorationMode { decoration: u32, mode: u32 },
    SurfaceEnter { surface: u32, output: u32 },
    SurfaceLeave { surface: u32, output: u32 },
    /// What the seat has: [`SEAT_POINTER`], [`SEAT_KEYBOARD`].
    SeatCapabilities { seat: u32, capabilities: u32 },
    /// The pointer came onto a surface, at x, y in the surface's units.
    PointerEnter { serial: u32, surface: u32, x: f64, y: f64 },
    PointerLeave { serial: u32, surface: u32 },
    PointerMotion { time: u32, x: f64, y: f64 },
    /// A button (a Linux input code: 0x110 left, 0x111 right, 0x112 middle).
    PointerButton { serial: u32, time: u32, button: u32, pressed: bool },
    /// Scrolling: `axis` 0 is up and down, 1 is sideways.
    PointerAxis { time: u32, axis: u32, value: f64 },
    /// The pointer events since the last frame belong together.
    PointerFrame,
    /// The keyboard's map: `size` bytes of text in the file descriptor that
    /// came with it ([`Client::take_fd`]). `format` 1 is xkb.
    Keymap { format: u32, size: u32 },
    KeyboardEnter { serial: u32, surface: u32 },
    KeyboardLeave { serial: u32, surface: u32 },
    /// A key (a Linux input code; xkb's keycode is 8 more).
    Key { serial: u32, time: u32, key: u32, pressed: bool },
    Modifiers { depressed: u32, latched: u32, locked: u32, group: u32 },
    /// Held keys repeat `rate` times a second after `delay` milliseconds (0: never).
    RepeatInfo { rate: i32, delay: i32 },
    /// Something is on offer (a clipboard's contents, or a drag): its kinds
    /// of data follow as [`Event::OfferKind`].
    DataOffer { offer: u32 },
    /// One kind of data an offer can give, a MIME type.
    OfferKind { offer: u32, mime: String },
    /// What the clipboard holds now, for the program with the keyboard
    /// (None: nothing).
    Selection { offer: Option<u32> },
    /// Another program wants what we put on the clipboard, as `mime`,
    /// written to the file descriptor that came with this ([`Client::take_fd`]).
    SourceSend { source: u32, mime: String },
    /// Our clipboard contents have been replaced: the source is finished.
    SourceCancelled { source: u32 },
    /// A pixel format (a fourcc code) and memory layout (modifier) the
    /// server can show from the GPU's memory.
    DmabufFormat { format: u32, modifier: u64 },
    /// The buffer asked for with [`Client::create_dmabuf`] exists.
    DmabufCreated { params: u32, buffer: u32 },
    /// The server could not take that buffer; draw into shared memory instead.
    DmabufFailed { params: u32 },
}

/// Object ids from here up are the server's to give out.
const SERVER_IDS: u32 = 0xff00_0000;

fn invalid(e: WireError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// Reads one message as an event of `interface`. None: an event this client
/// has no use for.
pub fn decode(interface: Interface, object: u32, opcode: u16, arguments: &[u8]) -> Result<Option<Event>, WireError> {
    let mut r = Reader::new(arguments);
    Ok(Some(match (interface, opcode) {
        (Interface::Display, 0) => Event::Error { object: r.uint()?, code: r.uint()?, message: r.string()?.unwrap_or_default() },
        (Interface::Registry, 0) => Event::Global { name: r.uint()?, interface: r.string()?.unwrap_or_default(), version: r.uint()? },
        (Interface::Registry, 1) => Event::GlobalRemove { name: r.uint()? },
        (Interface::Callback, 0) => Event::Done { callback: object, data: r.uint()? },
        (Interface::Shm, 0) => Event::ShmFormat { format: r.uint()? },
        (Interface::Buffer, 0) => Event::BufferRelease { buffer: object },
        (Interface::Surface, 0) => Event::SurfaceEnter { surface: object, output: r.uint()? },
        (Interface::Surface, 1) => Event::SurfaceLeave { surface: object, output: r.uint()? },
        (Interface::WmBase, 0) => Event::Ping { serial: r.uint()? },
        (Interface::XdgSurface, 0) => Event::Configure { xdg_surface: object, serial: r.uint()? },
        (Interface::Toplevel, 0) => Event::ToplevelConfigure { toplevel: object, width: r.int()?, height: r.int()?, states: r.words()? },
        (Interface::Toplevel, 1) => Event::Close { toplevel: object },
        (Interface::FractionalScale, 0) => Event::PreferredScale { object, scale120: r.uint()? },
        (Interface::Decoration, 0) => Event::DecorationMode { decoration: object, mode: r.uint()? },
        (Interface::Seat, 0) => Event::SeatCapabilities { seat: object, capabilities: r.uint()? },
        (Interface::Pointer, 0) => Event::PointerEnter { serial: r.uint()?, surface: r.uint()?, x: r.fixed()?, y: r.fixed()? },
        (Interface::Pointer, 1) => Event::PointerLeave { serial: r.uint()?, surface: r.uint()? },
        (Interface::Pointer, 2) => Event::PointerMotion { time: r.uint()?, x: r.fixed()?, y: r.fixed()? },
        (Interface::Pointer, 3) => Event::PointerButton { serial: r.uint()?, time: r.uint()?, button: r.uint()?, pressed: r.uint()? == 1 },
        (Interface::Pointer, 4) => Event::PointerAxis { time: r.uint()?, axis: r.uint()?, value: r.fixed()? },
        (Interface::Pointer, 5) => Event::PointerFrame,
        (Interface::Keyboard, 0) => Event::Keymap { format: r.uint()?, size: r.uint()? },
        (Interface::Keyboard, 1) => Event::KeyboardEnter { serial: r.uint()?, surface: r.uint()? },
        (Interface::Keyboard, 2) => Event::KeyboardLeave { serial: r.uint()?, surface: r.uint()? },
        (Interface::Keyboard, 3) => Event::Key { serial: r.uint()?, time: r.uint()?, key: r.uint()?, pressed: r.uint()? == 1 },
        (Interface::Keyboard, 4) => {
            let _serial = r.uint()?;
            Event::Modifiers { depressed: r.uint()?, latched: r.uint()?, locked: r.uint()?, group: r.uint()? }
        }
        (Interface::Keyboard, 5) => Event::RepeatInfo { rate: r.int()?, delay: r.int()? },
        (Interface::DataDevice, 0) => Event::DataOffer { offer: r.uint()? },
        (Interface::DataDevice, 5) => Event::Selection { offer: Some(r.uint()?).filter(|id| *id != 0) },
        (Interface::DataOffer, 0) => Event::OfferKind { offer: object, mime: r.string()?.unwrap_or_default() },
        (Interface::DataSource, 1) => Event::SourceSend { source: object, mime: r.string()?.unwrap_or_default() },
        (Interface::DataSource, 2) => Event::SourceCancelled { source: object },
        (Interface::Dmabuf, 1) => {
            let (format, high, low) = (r.uint()?, r.uint()?, r.uint()?);
            Event::DmabufFormat { format, modifier: (high as u64) << 32 | low as u64 }
        }
        (Interface::DmabufParams, 0) => Event::DmabufCreated { params: object, buffer: r.uint()? },
        (Interface::DmabufParams, 1) => Event::DmabufFailed { params: object },
        _ => return Ok(None),
    }))
}

pub struct Client {
    conn: Connection,
    objects: HashMap<u32, Interface>,
}

impl Client {
    pub fn connect() -> io::Result<Client> {
        Ok(Client::new(Connection::connect()?))
    }

    pub fn new(conn: Connection) -> Client {
        Client { conn, objects: HashMap::from([(1, Interface::Display)]) }
    }

    fn send(&mut self, message: Writer) -> io::Result<()> {
        self.conn.send(message).map_err(invalid)
    }

    fn create(&mut self, interface: Interface) -> u32 {
        let id = self.conn.new_id();
        self.objects.insert(id, interface);
        id
    }

    /// Sends what is queued, waits for the server, and returns what it said
    /// (an empty list is possible: wait again). An error if the connection
    /// has closed or the server sent something that is not Wayland.
    pub fn dispatch(&mut self) -> io::Result<Vec<Event>> {
        self.dispatch_within(None)
    }

    /// As [`Client::dispatch`], but gives up waiting after `timeout` and
    /// returns nothing, for a caller with a timer of its own (key repeat).
    pub fn dispatch_within(&mut self, timeout: Option<Duration>) -> io::Result<Vec<Event>> {
        Ok(self.dispatch_or(None, timeout)?.0)
    }

    /// As [`Client::dispatch_within`], while also watching another file
    /// descriptor (a pipe the clipboard is being read from): the events,
    /// and whether that descriptor is ready to be read.
    pub fn dispatch_or(&mut self, other: Option<RawFd>, timeout: Option<Duration>) -> io::Result<(Vec<Event>, bool)> {
        self.conn.flush()?;
        let (server, other_ready) = self.conn.wait_also(other, timeout)?;
        if !server {
            return Ok((Vec::new(), other_ready));
        }
        Ok((self.read_events()?, other_ready))
    }

    fn read_events(&mut self) -> io::Result<Vec<Event>> {
        if self.conn.read()? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the display server closed the connection"));
        }
        let mut events = Vec::new();
        while let Some((header, arguments)) = self.conn.message().map_err(invalid)? {
            // wl_display.delete_id: the id is free again. Not an event for the caller.
            if header.object == 1 && header.opcode == 1 {
                let id = Reader::new(&arguments).uint().map_err(invalid)?;
                self.objects.remove(&id);
                self.conn.release_id(id);
                continue;
            }
            let Some(interface) = self.objects.get(&header.object).copied() else { continue };
            if let Some(event) = decode(interface, header.object, header.opcode, &arguments).map_err(invalid)? {
                // An offer is an object the server made: remember what it is,
                // so its own events can be read. Ids below the server's range
                // are ours to give out and are not taken on its word.
                if let Event::DataOffer { offer } = event {
                    if offer < SERVER_IDS {
                        continue;
                    }
                    self.objects.insert(offer, Interface::DataOffer);
                }
                // So is a buffer made from GPU memory.
                if let Event::DmabufCreated { buffer, .. } = event {
                    if buffer < SERVER_IDS {
                        continue;
                    }
                    self.objects.insert(buffer, Interface::Buffer);
                }
                events.push(event);
            }
        }
        Ok(events)
    }

    /// The file descriptor that came with the event just returned (a keymap's).
    pub fn take_fd(&mut self) -> Option<OwnedFd> {
        self.conn.take_fd()
    }

    /// Sends what is queued without waiting for an answer.
    pub fn flush(&mut self) -> io::Result<()> {
        self.conn.flush()
    }

    // wl_display

    /// Asks for the list of what the server offers; each arrives as [`Event::Global`].
    pub fn get_registry(&mut self) -> io::Result<u32> {
        let id = self.create(Interface::Registry);
        self.send(Writer::new(1, 1).uint(id))?;
        Ok(id)
    }

    /// A callback that fires when the server has handled everything sent so far.
    pub fn sync(&mut self) -> io::Result<u32> {
        let id = self.create(Interface::Callback);
        self.send(Writer::new(1, 0).uint(id))?;
        Ok(id)
    }

    /// Takes up one of the server's offers. The version used is the older of
    /// the server's and this file's.
    pub fn bind(&mut self, registry: u32, name: u32, interface: Interface, offered: u32) -> io::Result<u32> {
        let global = interface.global_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not an interface that is bound"))?;
        let id = self.create(interface);
        self.send(Writer::new(registry, 0).uint(name).string(global).uint(offered.min(interface.version())).uint(id))?;
        Ok(id)
    }

    // wl_compositor, wl_surface

    pub fn create_surface(&mut self, compositor: u32) -> io::Result<u32> {
        let id = self.create(Interface::Surface);
        self.send(Writer::new(compositor, 0).uint(id))?;
        Ok(id)
    }

    /// The buffer the surface shows from its next commit (0: none).
    pub fn attach(&mut self, surface: u32, buffer: u32) -> io::Result<()> {
        self.send(Writer::new(surface, 1).object(buffer).int(0).int(0))
    }

    /// The part of the buffer that changed, in its own pixels.
    pub fn damage_buffer(&mut self, surface: u32, x: i32, y: i32, width: i32, height: i32) -> io::Result<()> {
        self.send(Writer::new(surface, 9).int(x).int(y).int(width).int(height))
    }

    /// A callback that fires when it is time to draw this surface's next frame.
    pub fn frame(&mut self, surface: u32) -> io::Result<u32> {
        let id = self.create(Interface::Callback);
        self.send(Writer::new(surface, 3).uint(id))?;
        Ok(id)
    }

    pub fn commit(&mut self, surface: u32) -> io::Result<()> {
        self.send(Writer::new(surface, 6))
    }

    // wl_shm, wl_shm_pool, wl_buffer

    /// Shares `size` bytes of the file `fd` with the server.
    pub fn create_pool(&mut self, shm: u32, fd: RawFd, size: i32) -> io::Result<u32> {
        let id = self.create(Interface::ShmPool);
        self.send(Writer::new(shm, 0).uint(id).fd(fd).int(size))?;
        Ok(id)
    }

    /// A picture in a pool: `width` x `height` pixels, `stride` bytes a row, from `offset`.
    pub fn create_buffer(&mut self, pool: u32, offset: i32, width: i32, height: i32, stride: i32, format: u32) -> io::Result<u32> {
        let id = self.create(Interface::Buffer);
        self.send(Writer::new(pool, 0).uint(id).int(offset).int(width).int(height).int(stride).uint(format))?;
        Ok(id)
    }

    /// Lets go of a pool; buffers made from it stay good.
    pub fn destroy_pool(&mut self, pool: u32) -> io::Result<()> {
        self.send(Writer::new(pool, 1))
    }

    pub fn destroy_buffer(&mut self, buffer: u32) -> io::Result<()> {
        self.send(Writer::new(buffer, 0))
    }

    // xdg_wm_base, xdg_surface, xdg_toplevel

    pub fn pong(&mut self, wm_base: u32, serial: u32) -> io::Result<()> {
        self.send(Writer::new(wm_base, 3).uint(serial))
    }

    pub fn get_xdg_surface(&mut self, wm_base: u32, surface: u32) -> io::Result<u32> {
        let id = self.create(Interface::XdgSurface);
        self.send(Writer::new(wm_base, 2).uint(id).object(surface))?;
        Ok(id)
    }

    pub fn get_toplevel(&mut self, xdg_surface: u32) -> io::Result<u32> {
        let id = self.create(Interface::Toplevel);
        self.send(Writer::new(xdg_surface, 1).uint(id))?;
        Ok(id)
    }

    pub fn ack_configure(&mut self, xdg_surface: u32, serial: u32) -> io::Result<()> {
        self.send(Writer::new(xdg_surface, 4).uint(serial))
    }

    pub fn set_title(&mut self, toplevel: u32, title: &str) -> io::Result<()> {
        // Short enough that the message always fits.
        let end = title.char_indices().map(|(i, c)| i + c.len_utf8()).take_while(|e| *e <= 1024).last().unwrap_or(0);
        self.send(Writer::new(toplevel, 2).string(&title[..end]))
    }

    pub fn set_app_id(&mut self, toplevel: u32, app_id: &str) -> io::Result<()> {
        let end = app_id.char_indices().map(|(i, c)| i + c.len_utf8()).take_while(|e| *e <= 256).last().unwrap_or(0);
        self.send(Writer::new(toplevel, 3).string(&app_id[..end]))
    }

    /// Starts a move of the window by the user, in answer to the button
    /// press with this serial.
    pub fn start_move(&mut self, toplevel: u32, seat: u32, serial: u32) -> io::Result<()> {
        self.send(Writer::new(toplevel, 5).object(seat).uint(serial))
    }

    /// Starts a resize by the user from these edges ([`EDGE_TOP`] and the others, added).
    pub fn start_resize(&mut self, toplevel: u32, seat: u32, serial: u32, edges: u32) -> io::Result<()> {
        self.send(Writer::new(toplevel, 6).object(seat).uint(serial).uint(edges))
    }

    pub fn set_maximized(&mut self, toplevel: u32, maximized: bool) -> io::Result<()> {
        self.send(Writer::new(toplevel, if maximized { 9 } else { 10 }))
    }

    pub fn set_minimized(&mut self, toplevel: u32) -> io::Result<()> {
        self.send(Writer::new(toplevel, 13))
    }

    // wl_seat, wp_cursor_shape_manager_v1

    pub fn get_pointer(&mut self, seat: u32) -> io::Result<u32> {
        let id = self.create(Interface::Pointer);
        self.send(Writer::new(seat, 0).uint(id))?;
        Ok(id)
    }

    pub fn get_keyboard(&mut self, seat: u32) -> io::Result<u32> {
        let id = self.create(Interface::Keyboard);
        self.send(Writer::new(seat, 1).uint(id))?;
        Ok(id)
    }

    /// The object through which a pointer's shape is set by name.
    pub fn get_cursor_shape_device(&mut self, manager: u32, pointer: u32) -> io::Result<u32> {
        let id = self.create(Interface::CursorShapeDevice);
        self.send(Writer::new(manager, 1).uint(id).object(pointer))?;
        Ok(id)
    }

    /// The pointer's shape over our surface ([`SHAPE_DEFAULT`] and the
    /// others), in answer to the pointer entering with this serial.
    pub fn set_cursor_shape(&mut self, device: u32, serial: u32, shape: u32) -> io::Result<()> {
        self.send(Writer::new(device, 1).uint(serial).uint(shape))
    }

    // wl_data_device_manager: the clipboard

    /// The object through which this seat's clipboard is read and set.
    pub fn get_data_device(&mut self, manager: u32, seat: u32) -> io::Result<u32> {
        let id = self.create(Interface::DataDevice);
        self.send(Writer::new(manager, 1).uint(id).object(seat))?;
        Ok(id)
    }

    /// Something to put on the clipboard, offered in these kinds (MIME types).
    pub fn create_data_source(&mut self, manager: u32, kinds: &[&str]) -> io::Result<u32> {
        let id = self.create(Interface::DataSource);
        self.send(Writer::new(manager, 0).uint(id))?;
        for kind in kinds {
            self.send(Writer::new(id, 0).string(kind))?;
        }
        Ok(id)
    }

    pub fn destroy_data_source(&mut self, source: u32) -> io::Result<()> {
        self.send(Writer::new(source, 1))
    }

    /// Puts a source on the clipboard (0: clears it), in answer to the
    /// input with this serial.
    pub fn set_selection(&mut self, device: u32, source: u32, serial: u32) -> io::Result<()> {
        self.send(Writer::new(device, 1).object(source).uint(serial))
    }

    /// Asks for an offer's data as `mime`, to be written to `fd` (the
    /// writing end of a pipe; close our copy after flushing).
    pub fn receive(&mut self, offer: u32, mime: &str, fd: RawFd) -> io::Result<()> {
        self.send(Writer::new(offer, 1).string(mime).fd(fd))
    }

    /// Lets go of an offer that is no longer the clipboard's.
    pub fn destroy_offer(&mut self, offer: u32) -> io::Result<()> {
        if self.objects.remove(&offer).is_some() {
            self.send(Writer::new(offer, 2))?;
        }
        Ok(())
    }

    // zwp_linux_dmabuf_v1: buffers in the GPU's memory

    /// Asks the server to make a buffer of GPU memory we drew into: one
    /// plane, the file descriptor `fd` (ours stays ours), `stride` bytes a
    /// row from `offset`, in `format` (a fourcc code) laid out as `modifier`
    /// says. The answer is [`Event::DmabufCreated`] or
    /// [`Event::DmabufFailed`], for the params object returned here.
    #[allow(clippy::too_many_arguments)]
    pub fn create_dmabuf(&mut self, dmabuf: u32, fd: RawFd, width: i32, height: i32, format: u32, modifier: u64, offset: u32, stride: u32) -> io::Result<u32> {
        let params = self.create(Interface::DmabufParams);
        self.send(Writer::new(dmabuf, 1).uint(params))?;
        self.send(Writer::new(params, 1).fd(fd).uint(0).uint(offset).uint(stride).uint((modifier >> 32) as u32).uint(modifier as u32))?;
        self.send(Writer::new(params, 2).int(width).int(height).uint(format).uint(0))?;
        Ok(params)
    }

    /// Lets go of a params object once it has answered.
    pub fn destroy_dmabuf_params(&mut self, params: u32) -> io::Result<()> {
        self.send(Writer::new(params, 0))
    }

    // wp_viewporter, wp_viewport

    pub fn get_viewport(&mut self, viewporter: u32, surface: u32) -> io::Result<u32> {
        let id = self.create(Interface::Viewport);
        self.send(Writer::new(viewporter, 1).uint(id).object(surface))?;
        Ok(id)
    }

    /// The size the surface is shown at, in the server's units, whatever
    /// the buffer's size in pixels.
    pub fn set_destination(&mut self, viewport: u32, width: i32, height: i32) -> io::Result<()> {
        self.send(Writer::new(viewport, 2).int(width).int(height))
    }

    // wp_fractional_scale_manager_v1

    /// Asks to be told the exact scale for a surface ([`Event::PreferredScale`]).
    pub fn get_fractional_scale(&mut self, manager: u32, surface: u32) -> io::Result<u32> {
        let id = self.create(Interface::FractionalScale);
        self.send(Writer::new(manager, 1).uint(id).object(surface))?;
        Ok(id)
    }

    // zxdg_decoration_manager_v1

    pub fn get_decoration(&mut self, manager: u32, toplevel: u32) -> io::Result<u32> {
        let id = self.create(Interface::Decoration);
        self.send(Writer::new(manager, 1).uint(id).object(toplevel))?;
        Ok(id)
    }

    /// [`DECORATION_CLIENT`]: the window draws its own bar. [`DECORATION_SERVER`]: the server does.
    pub fn set_decoration_mode(&mut self, decoration: u32, mode: u32) -> io::Result<()> {
        self.send(Writer::new(decoration, 1).uint(mode))
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use super::*;

    /// A client and the raw other end of its socket, to play the server by hand.
    fn pair() -> (Client, Connection) {
        let (a, b) = UnixStream::pair().unwrap();
        (Client::new(Connection::from_stream(a)), Connection::from_stream(b))
    }

    fn next(server: &mut Connection) -> (u32, u16, Vec<u8>) {
        loop {
            if let Some((h, args)) = server.message().unwrap() {
                return (h.object, h.opcode, args);
            }
            server.read().unwrap();
        }
    }

    #[test]
    fn the_opening_conversation() {
        let (mut client, mut server) = pair();
        let registry = client.get_registry().unwrap();
        let sync = client.sync().unwrap();
        assert_eq!((registry, sync), (2, 3));
        client.flush().unwrap();
        // What a server reads: get_registry(new id 2), sync(new id 3), both to the display.
        assert_eq!(next(&mut server), (1, 1, 2u32.to_ne_bytes().to_vec()));
        assert_eq!(next(&mut server), (1, 0, 3u32.to_ne_bytes().to_vec()));
        // The server's answer: two globals, the sync callback done, its id deleted.
        server.send(Writer::new(2, 0).uint(1).string("wl_compositor").uint(6)).unwrap();
        server.send(Writer::new(2, 0).uint(2).string("wl_shm").uint(1)).unwrap();
        server.send(Writer::new(3, 0).uint(77)).unwrap();
        server.send(Writer::new(1, 1).uint(3)).unwrap();
        server.flush().unwrap();
        assert_eq!(
            client.dispatch().unwrap(),
            [
                Event::Global { name: 1, interface: "wl_compositor".into(), version: 6 },
                Event::Global { name: 2, interface: "wl_shm".into(), version: 1 },
                Event::Done { callback: 3, data: 77 },
            ]
        );
        // Binding: the version is the older of the two sides', the freed id is used again.
        let compositor = client.bind(registry, 1, Interface::Compositor, 6).unwrap();
        assert_eq!(compositor, 3, "the deleted callback's id");
        client.flush().unwrap();
        let (object, opcode, args) = next(&mut server);
        assert_eq!((object, opcode), (2, 0));
        let mut r = Reader::new(&args);
        assert_eq!((r.uint(), r.string(), r.uint(), r.uint()), (Ok(1), Ok(Some("wl_compositor".into())), Ok(4), Ok(3)));
    }

    #[test]
    fn a_window_is_asked_for_in_the_right_words() {
        let (mut client, mut server) = pair();
        // Ids as if bound: compositor 2, wm_base 3.
        let (compositor, wm) = (client.create(Interface::Compositor), client.create(Interface::WmBase));
        let surface = client.create_surface(compositor).unwrap();
        let xdg = client.get_xdg_surface(wm, surface).unwrap();
        let top = client.get_toplevel(xdg).unwrap();
        client.set_title(top, "Vergence").unwrap();
        client.commit(surface).unwrap();
        client.flush().unwrap();
        assert_eq!(next(&mut server), (compositor, 0, surface.to_ne_bytes().to_vec()));
        assert_eq!(next(&mut server), (wm, 2, [xdg.to_ne_bytes(), surface.to_ne_bytes()].concat()));
        assert_eq!(next(&mut server), (xdg, 1, top.to_ne_bytes().to_vec()));
        let (object, opcode, args) = next(&mut server);
        assert_eq!((object, opcode, Reader::new(&args).string()), (top, 2, Ok(Some("Vergence".into()))));
        assert_eq!(next(&mut server), (surface, 6, vec![]));
        // The server configures the window; a ping; the user closes it.
        server.send(Writer::new(top, 0).int(800).int(600).array(&[STATE_ACTIVATED.to_ne_bytes(), STATE_MAXIMIZED.to_ne_bytes()].concat())).unwrap();
        server.send(Writer::new(xdg, 0).uint(41)).unwrap();
        server.send(Writer::new(wm, 0).uint(9)).unwrap();
        server.send(Writer::new(top, 1)).unwrap();
        server.flush().unwrap();
        assert_eq!(
            client.dispatch().unwrap(),
            [
                Event::ToplevelConfigure { toplevel: top, width: 800, height: 600, states: vec![STATE_ACTIVATED, STATE_MAXIMIZED] },
                Event::Configure { xdg_surface: xdg, serial: 41 },
                Event::Ping { serial: 9 },
                Event::Close { toplevel: top },
            ]
        );
        client.ack_configure(xdg, 41).unwrap();
        client.pong(wm, 9).unwrap();
        client.flush().unwrap();
        assert_eq!(next(&mut server), (xdg, 4, 41u32.to_ne_bytes().to_vec()));
        assert_eq!(next(&mut server), (wm, 3, 9u32.to_ne_bytes().to_vec()));
    }

    #[test]
    fn the_pointer_and_the_keyboard_speak() {
        let (mut client, mut server) = pair();
        let seat = client.create(Interface::Seat);
        let (pointer, keyboard) = (client.get_pointer(seat).unwrap(), client.get_keyboard(seat).unwrap());
        client.flush().unwrap();
        assert_eq!(next(&mut server), (seat, 0, pointer.to_ne_bytes().to_vec()));
        assert_eq!(next(&mut server), (seat, 1, keyboard.to_ne_bytes().to_vec()));
        // A keymap arrives with its file; the pointer comes in, moves, clicks, scrolls; a key goes down.
        let dir = std::env::temp_dir().join(format!("vergence-wl-keymap-{}", std::process::id()));
        std::fs::write(&dir, "xkb_keymap {};").unwrap();
        let map = std::fs::File::open(&dir).unwrap();
        server.send(Writer::new(keyboard, 0).uint(1).fd(std::os::fd::AsRawFd::as_raw_fd(&map)).uint(14)).unwrap();
        server.send(Writer::new(keyboard, 5).int(30).int(500)).unwrap();
        server.send(Writer::new(pointer, 0).uint(70).uint(3).fixed(12.5).fixed(40.25)).unwrap();
        server.send(Writer::new(pointer, 2).uint(1000).fixed(13.0).fixed(41.0)).unwrap();
        server.send(Writer::new(pointer, 3).uint(71).uint(1001).uint(0x110).uint(1)).unwrap();
        server.send(Writer::new(pointer, 4).uint(1002).uint(0).fixed(-15.0)).unwrap();
        server.send(Writer::new(pointer, 5)).unwrap();
        server.send(Writer::new(keyboard, 4).uint(72).uint(1).uint(0).uint(2).uint(0)).unwrap();
        server.send(Writer::new(keyboard, 3).uint(73).uint(1003).uint(30).uint(1)).unwrap();
        server.flush().unwrap();
        assert_eq!(
            client.dispatch().unwrap(),
            [
                Event::Keymap { format: 1, size: 14 },
                Event::RepeatInfo { rate: 30, delay: 500 },
                Event::PointerEnter { serial: 70, surface: 3, x: 12.5, y: 40.25 },
                Event::PointerMotion { time: 1000, x: 13.0, y: 41.0 },
                Event::PointerButton { serial: 71, time: 1001, button: 0x110, pressed: true },
                Event::PointerAxis { time: 1002, axis: 0, value: -15.0 },
                Event::PointerFrame,
                Event::Modifiers { depressed: 1, latched: 0, locked: 2, group: 0 },
                Event::Key { serial: 73, time: 1003, key: 30, pressed: true },
            ]
        );
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::fs::File::from(client.take_fd().expect("the keymap's file")), &mut text).unwrap();
        assert_eq!(text, "xkb_keymap {};");
        let _ = std::fs::remove_file(dir);
        // Moving and resizing answer a press by its serial.
        let top = client.create(Interface::Toplevel);
        client.start_move(top, seat, 71).unwrap();
        client.start_resize(top, seat, 71, EDGE_BOTTOM | EDGE_RIGHT).unwrap();
        client.flush().unwrap();
        assert_eq!(next(&mut server), (top, 5, [seat.to_ne_bytes(), 71u32.to_ne_bytes()].concat()));
        assert_eq!(next(&mut server), (top, 6, [seat.to_ne_bytes(), 71u32.to_ne_bytes(), 10u32.to_ne_bytes()].concat()));
    }

    #[test]
    fn the_clipboard_conversation() {
        let (mut client, mut server) = pair();
        let (manager, seat) = (client.create(Interface::DataDeviceManager), client.create(Interface::Seat));
        let device = client.get_data_device(manager, seat).unwrap();
        // Copy: a source with two kinds, set as the selection.
        let source = client.create_data_source(manager, &["text/plain;charset=utf-8", "text/plain"]).unwrap();
        client.set_selection(device, source, 55).unwrap();
        client.flush().unwrap();
        assert_eq!(next(&mut server), (manager, 1, [device.to_ne_bytes(), seat.to_ne_bytes()].concat()));
        assert_eq!(next(&mut server), (manager, 0, source.to_ne_bytes().to_vec()));
        let (object, opcode, args) = next(&mut server);
        assert_eq!((object, opcode, Reader::new(&args).string()), (source, 0, Ok(Some("text/plain;charset=utf-8".into()))));
        assert_eq!(next(&mut server).1, 0);
        assert_eq!(next(&mut server), (device, 1, [source.to_ne_bytes(), 55u32.to_ne_bytes()].concat()));
        // Another program asks for it: the request comes with a pipe to write into.
        let (read_end, write_end) = crate::conn::pipe().unwrap();
        server.send(Writer::new(source, 1).string("text/plain").fd(std::os::fd::AsRawFd::as_raw_fd(&write_end))).unwrap();
        // Then someone else copies: ours is cancelled, and theirs is offered in two kinds.
        server.send(Writer::new(source, 2)).unwrap();
        let offer = 0xff00_0001u32;
        server.send(Writer::new(device, 0).uint(offer)).unwrap();
        server.send(Writer::new(offer, 0).string("text/html")).unwrap();
        server.send(Writer::new(offer, 0).string("text/plain;charset=utf-8")).unwrap();
        server.send(Writer::new(device, 5).uint(offer)).unwrap();
        server.flush().unwrap();
        drop(write_end);
        assert_eq!(
            client.dispatch().unwrap(),
            [
                Event::SourceSend { source, mime: "text/plain".into() },
                Event::SourceCancelled { source },
                Event::DataOffer { offer },
                Event::OfferKind { offer, mime: "text/html".into() },
                Event::OfferKind { offer, mime: "text/plain;charset=utf-8".into() },
                Event::Selection { offer: Some(offer) },
            ]
        );
        // What we write to the descriptor that came with the request is what the other end reads.
        std::io::Write::write_all(&mut std::fs::File::from(client.take_fd().expect("the pipe to write into")), b"copied").unwrap();
        let mut got = String::new();
        std::io::Read::read_to_string(&mut std::fs::File::from(read_end), &mut got).unwrap();
        assert_eq!(got, "copied");
        // Paste: ask the offer for one of its kinds, then let it go, once.
        let (_r, w) = crate::conn::pipe().unwrap();
        client.receive(offer, "text/plain;charset=utf-8", std::os::fd::AsRawFd::as_raw_fd(&w)).unwrap();
        client.destroy_offer(offer).unwrap();
        client.destroy_offer(offer).unwrap();
        client.flush().unwrap();
        let (object, opcode, args) = next(&mut server);
        assert_eq!((object, opcode, Reader::new(&args).string()), (offer, 1, Ok(Some("text/plain;charset=utf-8".into()))));
        assert!(server.take_fd().is_some(), "the pipe went with the request");
        assert_eq!(next(&mut server), (offer, 2, vec![]));
        // The clipboard emptied; and an "offer" with an id that is ours to give out is not believed.
        server.send(Writer::new(device, 5).uint(0)).unwrap();
        server.send(Writer::new(device, 0).uint(7)).unwrap();
        server.send(Writer::new(7, 0).string("text/plain")).unwrap();
        server.flush().unwrap();
        assert_eq!(client.dispatch().unwrap(), [Event::Selection { offer: None }]);
    }

    #[test]
    fn a_gpu_buffer_is_offered_and_answered() {
        let (mut client, mut server) = pair();
        let dmabuf = client.create(Interface::Dmabuf);
        // What the server can show: one format with two layouts.
        server.send(Writer::new(dmabuf, 1).uint(0x3432_5241).uint(0x0100_0000).uint(2)).unwrap();
        server.send(Writer::new(dmabuf, 1).uint(0x3432_5241).uint(0x00ff_ffff).uint(0xffff_ffff)).unwrap();
        server.flush().unwrap();
        assert_eq!(
            client.dispatch().unwrap(),
            [Event::DmabufFormat { format: 0x3432_5241, modifier: 0x0100_0000_0000_0002 }, Event::DmabufFormat { format: 0x3432_5241, modifier: 0x00ff_ffff_ffff_ffff }]
        );
        let (_r, w) = crate::conn::pipe().unwrap();
        let params = client.create_dmabuf(dmabuf, std::os::fd::AsRawFd::as_raw_fd(&w), 1920, 1260, 0x3432_5241, 0x0100_0000_0000_0002, 0, 7680).unwrap();
        client.flush().unwrap();
        assert_eq!(next(&mut server), (dmabuf, 1, params.to_ne_bytes().to_vec()));
        let (object, opcode, args) = next(&mut server);
        let mut r = Reader::new(&args);
        assert_eq!((object, opcode), (params, 1));
        assert_eq!((r.uint(), r.uint(), r.uint(), r.uint(), r.uint()), (Ok(0), Ok(0), Ok(7680), Ok(0x0100_0000), Ok(2)), "plane, offset, stride, modifier high and low");
        assert!(server.take_fd().is_some(), "the buffer's file descriptor went with it");
        let (object, opcode, args) = next(&mut server);
        let mut r = Reader::new(&args);
        assert_eq!((object, opcode, r.int(), r.int(), r.uint(), r.uint()), (params, 2, Ok(1920), Ok(1260), Ok(0x3432_5241), Ok(0)));
        // The server makes the buffer: it is ours to attach, and its release is heard.
        let buffer = 0xff00_0009u32;
        server.send(Writer::new(params, 0).uint(buffer)).unwrap();
        server.send(Writer::new(buffer, 0)).unwrap();
        // Another is refused; and a "created" with an id from our own range is not believed.
        let second = client.create_dmabuf(dmabuf, std::os::fd::AsRawFd::as_raw_fd(&w), 8, 8, 1, 0, 0, 32).unwrap();
        server.send(Writer::new(second, 1)).unwrap();
        server.send(Writer::new(second, 0).uint(5)).unwrap();
        server.flush().unwrap();
        assert_eq!(client.dispatch().unwrap(), [Event::DmabufCreated { params, buffer }, Event::BufferRelease { buffer }, Event::DmabufFailed { params: second }]);
    }

    #[test]
    fn waiting_can_give_up() {
        let (mut client, mut server) = pair();
        assert_eq!(client.dispatch_within(Some(Duration::from_millis(20))).unwrap(), []);
        server.send(Writer::new(1, 0).uint(1).uint(0).string("x")).unwrap();
        server.flush().unwrap();
        assert_eq!(client.dispatch_within(Some(Duration::from_millis(500))).unwrap().len(), 1);
    }

    #[test]
    fn what_is_not_understood_is_passed_over() {
        let (mut client, mut server) = pair();
        let surface = client.create(Interface::Surface);
        // An event for an object the client never made, one of an unknown kind, then a real one.
        server.send(Writer::new(999, 0).uint(1)).unwrap();
        server.send(Writer::new(surface, 57).uint(1).uint(2)).unwrap();
        server.send(Writer::new(surface, 0).uint(12)).unwrap();
        server.flush().unwrap();
        assert_eq!(client.dispatch().unwrap(), [Event::SurfaceEnter { surface, output: 12 }]);
        // An event cut short is an error, not a guess.
        server.send(Writer::new(surface, 0)).unwrap();
        server.flush().unwrap();
        assert_eq!(client.dispatch().unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_server_error_and_a_closed_connection() {
        let (mut client, mut server) = pair();
        server.send(Writer::new(1, 0).uint(5).uint(2).string("invalid arguments for wl_surface@5.attach")).unwrap();
        server.flush().unwrap();
        assert_eq!(client.dispatch().unwrap(), [Event::Error { object: 5, code: 2, message: "invalid arguments for wl_surface@5.attach".into() }]);
        drop(server);
        assert_eq!(client.dispatch().unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_long_title_is_cut_to_fit() {
        let (mut client, mut server) = pair();
        let top = client.create(Interface::Toplevel);
        client.set_title(top, &"é".repeat(5000)).unwrap();
        client.flush().unwrap();
        let (_, _, args) = next(&mut server);
        let title = Reader::new(&args).string().unwrap().unwrap();
        assert_eq!(title, "é".repeat(512));
    }
}
