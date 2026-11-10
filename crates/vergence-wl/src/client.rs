//! The protocol itself: the requests the toolkit makes and the events it is
//! told, by name.
//!
//! Wayland describes its interfaces in XML and most clients generate code
//! from it. This file is written by hand from the protocol's documents
//! instead, and holds only what the toolkit uses: a window with a surface,
//! shared-memory buffers, the exact scale (wp-fractional-scale, wp-viewporter)
//! and who draws the title bar (xdg-decoration). Input, the clipboard and
//! GPU buffers come with the steps that need them.
//!
//! [`Client`] remembers which interface each object id belongs to, so an
//! incoming message can be read as the right event. An event for an object
//! or of a kind it does not know is passed over, never guessed at.

use std::{collections::HashMap, io, os::fd::RawFd};

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
            _ => return None,
        })
    }

    /// The newest version of it this file was written against.
    pub fn version(self) -> u32 {
        match self {
            Interface::Compositor => 4,
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
}

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
        self.conn.flush()?;
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
                events.push(event);
            }
        }
        Ok(events)
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
