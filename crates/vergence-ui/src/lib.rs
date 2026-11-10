//! A window (whitepaper §6.10; toolkit milestone 1, step 2).
//!
//! The first piece of the toolkit proper: a window on the display server
//! that shows a scene. A program says what its window looks like at a size
//! ([`App::scene`]); this crate asks the server for the window, learns its
//! size and the exact scale of the monitor it is on, draws the scene at that
//! scale and hands the server the pixels, once for each frame the server
//! asks for and only when something changed.
//!
//! For now the pixels are drawn by the CPU renderer into memory shared with
//! the server (`wl_shm`); the GPU renderer and its buffers are step 3 of
//! the milestone. Input (pointer, keyboard, clipboard) is the next part of
//! this step. There are no widgets yet.
//!
//! Sizes a program sees are in reference units (whitepaper Contract 1). It
//! is never told to scale anything: the same scene is drawn sharper on a
//! denser monitor, and drawn again when the window moves to another.

use std::{io, time::Instant};

use vergence_render::{Cpu, Pixmap, pixel_size};
use vergence_scene::{Color, Scene};
use vergence_text::Fonts;
use vergence_wl::{
    Client, Event, Interface,
    client::{DECORATION_CLIENT, DECORATION_SERVER, FORMAT_ARGB8888, FORMAT_XRGB8888},
    shm::ShmFile,
};

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

/// Opens the window and runs until it is closed. `size` is the size asked
/// for, in reference units; the server may give another.
pub fn run(app: &mut dyn App, size: (u32, u32)) -> io::Result<()> {
    let mut client = Client::connect()?;
    let registry = client.get_registry()?;
    let ready = client.sync()?;

    // What the server offers, of what a window needs.
    let wanted = [Interface::Compositor, Interface::Shm, Interface::WmBase, Interface::Viewporter, Interface::FractionalScaleManager, Interface::DecorationManager];
    let mut offered: Vec<(Interface, u32, u32)> = Vec::new();
    'listing: loop {
        for event in client.dispatch()? {
            match event {
                Event::Global { name, interface, version } => {
                    if let Some(i) = wanted.iter().find(|i| i.global_name() == Some(interface.as_str())) {
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

    let surface = client.create_surface(compositor)?;
    let xdg_surface = client.get_xdg_surface(wm_base, surface)?;
    let toplevel = client.get_toplevel(xdg_surface)?;
    client.set_title(toplevel, &app.title())?;
    client.set_app_id(toplevel, &app.app_id())?;
    if let Some(manager) = decoration_manager {
        let decoration = client.get_decoration(manager, toplevel)?;
        client.set_decoration_mode(decoration, if app.own_frame() { DECORATION_CLIENT } else { DECORATION_SERVER })?;
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

    let cpu = Cpu::new();
    let mut fonts = Fonts::system();
    let started = Instant::now();
    let (mut width, mut height) = (size.0.max(1), size.1.max(1));
    let mut suggested: Option<(u32, u32)> = None;
    let mut scale120: u32 = 120;
    let mut configured = false;
    let mut dirty = false;
    let mut frame_callback: Option<u32> = None;
    let mut frames: u64 = 0;
    let mut buffers: Vec<Buffer> = Vec::new();
    let mut pixmap: Option<Pixmap> = None;

    loop {
        for event in client.dispatch()? {
            match event {
                Event::Ping { serial } => client.pong(wm_base, serial)?,
                Event::ToplevelConfigure { width: w, height: h, .. } => {
                    if w > 0 && h > 0 {
                        suggested = Some((w as u32, h as u32));
                    }
                }
                Event::Configure { serial, .. } => {
                    client.ack_configure(xdg_surface, serial)?;
                    if let Some((w, h)) = suggested.take()
                        && (w, h) != (width, height)
                    {
                        (width, height) = (w, h);
                    }
                    configured = true;
                    dirty = true;
                }
                Event::PreferredScale { scale120: s, .. } => {
                    if exact_scale && s > 0 && s != scale120 {
                        scale120 = s;
                        dirty = true;
                    }
                }
                Event::BufferRelease { buffer } => {
                    if let Some(b) = buffers.iter_mut().find(|b| b.id == buffer) {
                        b.busy = false;
                    }
                }
                Event::Done { callback, .. } if Some(callback) == frame_callback => {
                    frame_callback = None;
                    if app.animating() {
                        dirty = true;
                    }
                }
                Event::Close { .. } => return Ok(()),
                Event::Error { object, code, message } => return Err(other(format!("the display server refused: {message} (object {object}, code {code})"))),
                _ => {}
            }
        }
        if !(configured && dirty && frame_callback.is_none()) {
            continue;
        }
        dirty = false;

        // Draw.
        let t = Instant::now();
        let frame = Frame { width: width as f32, height: height as f32, scale: scale120 as f32 / 120.0, time: started.elapsed().as_secs_f64(), number: frames };
        let scene = app.scene(&frame, &mut fonts);
        let background = app.background();
        let (pw, ph) = pixel_size(&scene, frame.scale).map_err(other)?;
        if pixmap.as_ref().is_none_or(|p| (p.width, p.height) != (pw, ph)) {
            pixmap = Some(Pixmap::new(pw, ph));
        }
        let picture = pixmap.as_mut().expect("just made");
        cpu.render_rows(&scene, &mut fonts, frame.scale, background, picture, 0..ph).map_err(other)?;

        // A buffer of this size that the server is not using; others of another size are let go.
        let mut stale = Vec::new();
        buffers.retain(|b| {
            let keep = b.busy || (b.width, b.height) == (pw, ph);
            if !keep {
                stale.push(b.id);
            }
            keep
        });
        for id in stale {
            client.destroy_buffer(id)?;
        }
        let stride = pw as usize * 4;
        let index = match buffers.iter().position(|b| !b.busy && (b.width, b.height) == (pw, ph)) {
            Some(i) => i,
            None => {
                let file = ShmFile::new(stride * ph as usize)?;
                let pool = client.create_pool(shm, file.fd(), file.len() as i32)?;
                let format = if background.a == 255 { FORMAT_XRGB8888 } else { FORMAT_ARGB8888 };
                let id = client.create_buffer(pool, 0, pw as i32, ph as i32, stride as i32, format)?;
                client.destroy_pool(pool)?;
                buffers.push(Buffer { file, id, width: pw, height: ph, busy: false });
                buffers.len() - 1
            }
        };
        let buffer = &mut buffers[index];
        // The renderer's bytes are red, green, blue, alpha; the server's format has blue first.
        for (to, from) in buffer.file.bytes().chunks_exact_mut(4).zip(picture.data.chunks_exact(4)) {
            to.copy_from_slice(&[from[2], from[1], from[0], from[3]]);
        }
        buffer.busy = true;
        let id = buffer.id;
        client.attach(surface, id)?;
        client.damage_buffer(surface, 0, 0, pw as i32, ph as i32)?;
        if let Some(v) = viewport {
            client.set_destination(v, width as i32, height as i32)?;
        }
        frame_callback = Some(client.frame(surface)?);
        client.commit(surface)?;
        client.flush()?;
        frames += 1;
        if !app.shown(&frame, t.elapsed().as_secs_f64() * 1e3) {
            return Ok(());
        }
    }
}
