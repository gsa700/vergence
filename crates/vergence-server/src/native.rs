// Windows on the project's own protocol (native milestone 1, step 3).
//
// A client of the session bus asks for a window (`WinRequest::Create`) with
// its identity (Contract 2: the connection's application id and a role), a
// title, a size and whether it draws its own bar. The server places it
// (window memory first, then the monitor under the pointer), answers
// `Created` and a `Configure` with the size in reference units and the exact
// scale of its monitor (Contract 1), and shows it with its first picture.
// Pictures are shared memory here (GPU buffers come in step 4): the server
// reads the damaged rows with pread, never mapping the client's file, so a
// client that shrinks its file under us gets a short read and loses its
// connection, where a mapping would take the server down with SIGBUS. The
// pixels go into a MemoryRenderBuffer, drawn by decor.rs's scene like a
// title bar's text. A window asks for its next frame by attaching; the
// server says `Frame` after drawing a monitor it is on, at the monitor's
// pace. Input, the clipboard and GPU pictures are step 4.

use std::{
    os::{fd::AsFd, unix::fs::FileExt},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::element::memory::MemoryRenderBuffer,
    },
    output::Output,
    utils::{Buffer as BufferCoords, Logical, Point, Rectangle, Size, Transform, user_data::UserDataMap},
};
use vergence_protocol::{
    bus::{ErrorCode, ServerMsg},
    window::{Buffer, Damage, SHM_ARGB8888, SHM_XRGB8888, WinEvent, WinRequest},
};

use crate::{Server, win::Win};

/// Most windows one client may have open.
const MAX_WINDOWS: usize = 64;
/// The smallest size a window is given, in reference units.
const MIN_SIDE: i32 = 32;

/// A window of the project's own protocol.
pub struct NativeWindow {
    /// The bus connection it belongs to, and the id the client chose.
    pub client: u32,
    pub id: u32,
    /// Its identity (Contract 2).
    pub app_id: String,
    pub role: String,
    /// Whether the client draws its own bar (otherwise the server does).
    pub own_frame: bool,
    alive: AtomicBool,
    state: Mutex<State>,
    user_data: UserDataMap,
}

struct State {
    title: String,
    /// The size it was last told to draw at, in reference units, and the scale.
    size: Size<i32, Logical>,
    scale: f64,
    maximized: bool,
    activated: bool,
    /// The last configure sent and the last one the client said it drew for;
    /// while they differ, a newer size waits in `pending` (a resize drag sends
    /// one size per acknowledgement, not one per mouse movement).
    sent_serial: u32,
    acked_serial: u32,
    pending: Option<(Size<i32, Logical>, bool)>,
    /// Where it goes when its first picture comes, and whether memory had it maximized.
    place: Point<i32, Logical>,
    place_maximized: bool,
    shown: bool,
    picture: Option<Picture>,
    /// It attached a picture and waits to hear when to draw the next.
    frame_wanted: bool,
}

/// A native window's pixels, as the scene draws them.
#[derive(Clone)]
pub struct Picture {
    pub buffer: MemoryRenderBuffer,
    /// Its size in pixels, and in reference units at the scale it was drawn for.
    pub px: Size<i32, BufferCoords>,
    pub logical: Size<i32, Logical>,
    format: u32,
}

impl NativeWindow {
    pub fn title(&self) -> String {
        self.state.lock().unwrap().title.clone()
    }

    pub fn size(&self) -> Size<i32, Logical> {
        self.state.lock().unwrap().size
    }

    pub fn picture(&self) -> Option<Picture> {
        self.state.lock().unwrap().picture.clone()
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Returns whether it changed.
    pub fn set_activated(&self, on: bool) -> bool {
        let mut s = self.state.lock().unwrap();
        std::mem::replace(&mut s.activated, on) != on
    }

    pub fn user_data(&self) -> &UserDataMap {
        &self.user_data
    }

    fn kill(&self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

impl std::fmt::Debug for NativeWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NativeWindow({}:{} {:?} {:?})", self.client, self.id, self.app_id, self.role)
    }
}

/// Copy the damaged parts of a client's shared-memory picture into `mem`
/// (tightly packed, `width * 4` bytes a row), reading the file with pread.
/// Returns the rectangles copied.
fn copy_damage(
    file: &std::fs::File,
    mem: &mut [u8],
    width: i32,
    height: i32,
    stride: u32,
    damage: &[Damage],
) -> std::io::Result<Vec<Rectangle<i32, BufferCoords>>> {
    let whole = Rectangle::<i32, BufferCoords>::from_size((width, height).into());
    let rects: Vec<Rectangle<i32, BufferCoords>> = if damage.is_empty() {
        vec![whole]
    } else {
        damage
            .iter()
            .filter_map(|d| Rectangle::new((d.x, d.y).into(), (d.w, d.h).into()).intersection(whole))
            .filter(|r| r.size.w > 0 && r.size.h > 0)
            .collect()
    };
    let row = width as usize * 4;
    let mut band = Vec::new();
    for r in &rects {
        // The rows the rectangle covers, read in one call, then its columns copied.
        let (y0, rows) = (r.loc.y as usize, r.size.h as usize);
        let len = (rows - 1) * stride as usize + row;
        band.resize(len, 0);
        file.read_exact_at(&mut band, y0 as u64 * stride as u64)?;
        let (x0, w) = (r.loc.x as usize * 4, r.size.w as usize * 4);
        for i in 0..rows {
            let from = i * stride as usize + x0;
            let to = (y0 + i) * row + x0;
            mem[to..to + w].copy_from_slice(&band[from..from + w]);
        }
    }
    Ok(rects)
}

impl Server {
    /// A window message from bus client `client` (bus.rs).
    pub fn native_request(&mut self, client: u32, req: WinRequest) {
        match req {
            WinRequest::Create { serial, window, role, title, width, height, own_frame, .. } => {
                if window == 0 || self.natives.contains_key(&(client, window)) {
                    return self.native_refuse(client, serial, ErrorCode::BadMessage, format!("window id {window} is 0 or in use"));
                }
                if self.natives.keys().filter(|(c, _)| *c == client).count() >= MAX_WINDOWS {
                    return self.native_refuse(client, serial, ErrorCode::Denied, format!("at most {MAX_WINDOWS} windows"));
                }
                let side = |v: f64| if v.is_finite() { (v.round() as i32).clamp(MIN_SIDE, 16_384) } else { 640 };
                let asked: Size<i32, Logical> = (side(width), side(height)).into();
                let app_id = self.bus_app_id(client);
                let n = Arc::new(NativeWindow {
                    client,
                    id: window,
                    app_id: app_id.clone(),
                    role: role.clone(),
                    own_frame,
                    alive: AtomicBool::new(true),
                    state: Mutex::new(State {
                        title,
                        size: asked,
                        scale: 1.0,
                        maximized: false,
                        activated: false,
                        sent_serial: 0,
                        acked_serial: 0,
                        pending: None,
                        place: Point::default(),
                        place_maximized: false,
                        shown: false,
                        picture: None,
                        frame_wanted: false,
                    }),
                    user_data: UserDataMap::new(),
                });
                let w = Win::Native(n.clone());
                // Where it goes: memory first (by identity, memory.rs), else fitted
                // and centered on the monitor under the pointer (place.rs).
                let (rect, maximized) = match self.remembered_place(&w) {
                    Some(r) => r,
                    None => {
                        let out = self.output_at_pointer().unwrap_or_default();
                        let area = crate::place::content_area(self.work_area(out), self.has_title(&w));
                        let size = crate::place::fit(asked, area.size, Some((MIN_SIDE, MIN_SIDE).into()));
                        (Rectangle::new(crate::place::center(area, size), size), false)
                    }
                };
                let scale = self.scale_at(rect);
                {
                    let mut s = n.state.lock().unwrap();
                    (s.size, s.scale, s.place, s.place_maximized) = (rect.size, scale, rect.loc, maximized);
                }
                self.natives.insert((client, window), w.clone());
                let token = match self.window_identity(&w) {
                    Some(id) => format!("{}/{}/{}", id.class, id.role, id.index),
                    None => String::new(),
                };
                tracing::info!("native: {app_id:?} window {window} ({role:?}) asked {}x{}, given {}x{} at scale {scale:.4}", asked.w, asked.h, rect.size.w, rect.size.h);
                self.bus_send(client, ServerMsg::Window(WinEvent::Created { serial, window, token }));
                self.native_send_configure(&n);
            }
            WinRequest::Destroy { window } => {
                if let Some(w) = self.natives.remove(&(client, window)) {
                    self.native_drop(&w);
                }
            }
            WinRequest::Title { window, title } => {
                if let Some(Win::Native(n)) = self.natives.get(&(client, window)) {
                    n.state.lock().unwrap().title = title;
                }
            }
            WinRequest::AckConfigure { window, serial } => {
                let Some(Win::Native(n)) = self.natives.get(&(client, window)).cloned() else { return };
                let next = {
                    let mut s = n.state.lock().unwrap();
                    if serial > s.acked_serial && serial <= s.sent_serial {
                        s.acked_serial = serial;
                    }
                    if s.acked_serial == s.sent_serial { s.pending.take() } else { None }
                };
                if let Some((size, maximized)) = next {
                    self.native_configure(&n, size, maximized);
                }
            }
            WinRequest::Attach { window, buffer, damage } => {
                let Some(w) = self.natives.get(&(client, window)).cloned() else { return };
                let Win::Native(n) = &w else { return };
                if let Err(e) = self.native_attach(n, buffer, &damage) {
                    tracing::warn!("native: client {client} window {window}: {e}; disconnecting");
                    return self.bus_end(client);
                }
                let show = {
                    let mut s = n.state.lock().unwrap();
                    s.frame_wanted = true;
                    let was_shown = std::mem::replace(&mut s.shown, true);
                    !was_shown
                };
                if show {
                    let (place, maximized) = {
                        let s = n.state.lock().unwrap();
                        (s.place, s.place_maximized)
                    };
                    self.space.map_element(w.clone(), place, true);
                    if maximized {
                        self.toggle_maximize(&w);
                    }
                    tracing::info!("native: {:?} window {window} shown at {},{}", n.app_id, place.x, place.y);
                }
            }
            WinRequest::Maximize { window, on } => {
                let Some(w) = self.natives.get(&(client, window)).cloned() else { return };
                if on != self.restore.contains_key(&w) {
                    self.toggle_maximize(&w);
                }
            }
            WinRequest::Paste { serial, .. } => self.native_refuse(client, serial, ErrorCode::NotYet, "the clipboard: step 4".into()),
            // Input, moving and resizing from the client, minimizing, placement
            // requests, meaning and the clipboard: steps 4 to 6.
            other => tracing::debug!("native: client {client}: {other:?} (not yet)"),
        }
    }

    fn native_refuse(&mut self, client: u32, serial: u32, code: ErrorCode, message: String) {
        self.bus_send(client, ServerMsg::Error { serial, code, message });
    }

    /// Read a picture into the window's buffer.
    fn native_attach(&mut self, n: &NativeWindow, buffer: Buffer, damage: &[Damage]) -> Result<(), String> {
        let Buffer::Shm { fd, width, height, stride, format } = buffer else {
            return Err("GPU pictures come in step 4".into());
        };
        let fourcc = match format {
            SHM_ARGB8888 => Fourcc::Argb8888,
            SHM_XRGB8888 => Fourcc::Xrgb8888,
            other => return Err(format!("picture format {other:#x} is not one the server shows")),
        };
        // The decoder checked the numbers (sizes, stride); the file must hold them.
        let need = stride as u64 * height as u64;
        let file = std::fs::File::from(fd.0.as_fd().try_clone_to_owned().map_err(|e| e.to_string())?);
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        if len < need {
            return Err(format!("picture of {need} bytes in a file of {len}"));
        }
        let (w, h) = (width as i32, height as i32);
        let px: Size<i32, BufferCoords> = (w, h).into();
        let mut s = n.state.lock().unwrap();
        let reuse = s.picture.as_ref().is_some_and(|p| p.px == px && p.format == format);
        if !reuse {
            let opaque = (fourcc == Fourcc::Xrgb8888).then(|| vec![Rectangle::from_size(px)]);
            let buffer = MemoryRenderBuffer::new(fourcc, px, 1, Transform::Normal, opaque);
            s.picture = Some(Picture { buffer, px, logical: Size::default(), format });
        }
        let scale = s.scale;
        let p = s.picture.as_mut().unwrap();
        // A new buffer starts empty: all of the picture is copied.
        let damage = if reuse { damage } else { &[] };
        p.buffer
            .render()
            .draw(|mem| copy_damage(&file, mem, w, h, stride, damage))
            .map_err(|e| format!("reading the picture: {e}"))?;
        p.logical = ((w as f64 / scale).round() as i32, (h as f64 / scale).round() as i32).into();
        Ok(())
    }

    /// The window is gone: remembered, taken off the screen, forgotten.
    fn native_drop(&mut self, w: &Win) {
        let Win::Native(n) = w else { return };
        if n.state.lock().unwrap().shown {
            self.remember(w);
        }
        self.space.unmap_elem(w);
        self.restore.remove(w);
        n.kill();
        tracing::info!("native: {:?} window {} closed", n.app_id, n.id);
    }

    /// Bus client `client` has gone: its windows go with it.
    pub fn native_forget(&mut self, client: u32) {
        let mine: Vec<(u32, u32)> = self.natives.keys().filter(|(c, _)| *c == client).copied().collect();
        for k in mine {
            if let Some(w) = self.natives.remove(&k) {
                self.native_drop(&w);
            }
        }
    }

    /// Tell the window its size, scale and state (paced: one configure waits
    /// for the client to acknowledge the last).
    pub fn native_configure(&mut self, n: &NativeWindow, size: Size<i32, Logical>, maximized: bool) {
        {
            let mut s = n.state.lock().unwrap();
            if s.acked_serial != s.sent_serial {
                s.pending = Some((size, maximized));
                return;
            }
            if s.size == size && s.maximized == maximized {
                return;
            }
            (s.size, s.maximized) = (size, maximized);
        }
        self.native_send_configure(n);
    }

    fn native_send_configure(&mut self, n: &NativeWindow) {
        let (serial, size, scale, maximized, activated) = {
            let mut s = n.state.lock().unwrap();
            s.sent_serial = s.sent_serial.wrapping_add(1).max(1);
            (s.sent_serial, s.size, s.scale, s.maximized, s.activated)
        };
        let ev = WinEvent::Configure { window: n.id, serial, width: size.w as f64, height: size.h as f64, scale, maximized, activated };
        self.bus_send(n.client, ServerMsg::Window(ev));
    }

    /// Ask the client to close the window (its close button, the dock's menu).
    pub fn native_close(&mut self, n: &NativeWindow) {
        self.bus_send(n.client, ServerMsg::Window(WinEvent::Close { window: n.id }));
    }

    /// The scale of the monitor a rectangle is mostly on.
    fn scale_at(&self, rect: Rectangle<i32, Logical>) -> f64 {
        self.space
            .outputs()
            .filter_map(|o| {
                let g = self.space.output_geometry(o)?;
                let i = g.intersection(rect)?;
                Some((i.size.w as i64 * i.size.h as i64, o.current_scale().fractional_scale()))
            })
            .max_by_key(|(a, _)| *a)
            .map(|(_, s)| s)
            .or_else(|| self.space.outputs().next().map(|o| o.current_scale().fractional_scale()))
            .unwrap_or(1.0)
    }

    /// `output` has been drawn: windows on it that wait for a frame hear so,
    /// and a window that has moved to a monitor of another scale is told it.
    pub fn native_frames(&mut self, output: &Output) {
        let time_ms = self.start_time.elapsed().as_millis() as u32;
        let shown: Vec<Win> = self.space.elements().filter(|w| matches!(w, Win::Native(_))).cloned().collect();
        for w in shown {
            let Win::Native(n) = &w else { continue };
            if !self.space.outputs_for_element(&w).contains(output) {
                continue;
            }
            if std::mem::take(&mut n.state.lock().unwrap().frame_wanted) {
                self.bus_send(n.client, ServerMsg::Window(WinEvent::Frame { window: n.id, time_ms }));
            }
            let Some(geo) = self.space.element_geometry(&w) else { continue };
            let scale = self.scale_at(geo);
            let changed = {
                let mut s = n.state.lock().unwrap();
                let moved = (s.scale - scale).abs() > 1e-6;
                if moved {
                    s.scale = scale;
                }
                moved
            };
            if changed {
                tracing::info!("native: {:?} window {} now at scale {scale:.4}", n.app_id, n.id);
                self.native_send_configure(n);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn file_with(bytes: &[u8]) -> std::fs::File {
        let path = std::env::temp_dir().join(format!("vergence-native-test-{}-{}", std::process::id(), bytes.len()));
        let mut f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&path).unwrap();
        f.write_all(bytes).unwrap();
        let _ = std::fs::remove_file(&path);
        f
    }

    #[test]
    fn damage_copies_only_its_rectangle_through_the_stride() {
        // A 4 x 3 picture with a stride of 20 bytes (4 bytes of padding a row);
        // each pixel's bytes are its index.
        let mut src = vec![0xEEu8; 20 * 3];
        for y in 0..3 {
            for x in 0..4 {
                src[y * 20 + x * 4..y * 20 + x * 4 + 4].fill((y * 4 + x) as u8);
            }
        }
        let f = file_with(&src);
        let mut mem = vec![0u8; 4 * 4 * 3];
        let got = copy_damage(&f, &mut mem, 4, 3, 20, &[Damage { x: 1, y: 1, w: 2, h: 2 }]).unwrap();
        assert_eq!(got, vec![Rectangle::new((1, 1).into(), (2, 2).into())]);
        let px = |x: usize, y: usize| mem[(y * 4 + x) * 4];
        assert_eq!((px(1, 1), px(2, 1), px(1, 2), px(2, 2)), (5, 6, 9, 10));
        assert_eq!((px(0, 0), px(3, 1), px(0, 2)), (0, 0, 0), "outside the damage, untouched");
    }

    #[test]
    fn no_damage_means_all_of_it_and_damage_outside_is_clipped() {
        let src: Vec<u8> = (0..(8 * 2)).map(|i| i as u8).collect();
        let f = file_with(&src);
        let mut mem = vec![0u8; 8 * 2];
        copy_damage(&f, &mut mem, 2, 2, 8, &[]).unwrap();
        assert_eq!(mem, src);
        let got = copy_damage(&f, &mut mem, 2, 2, 8, &[Damage { x: 1, y: 1, w: 50, h: 50 }, Damage { x: 9, y: 9, w: 1, h: 1 }]).unwrap();
        assert_eq!(got, vec![Rectangle::new((1, 1).into(), (1, 1).into())]);
    }

    #[test]
    fn a_file_shorter_than_the_picture_is_an_error_not_a_crash() {
        let f = file_with(&[1, 2, 3, 4]);
        let mut mem = vec![0u8; 4 * 4 * 4];
        assert!(copy_damage(&f, &mut mem, 4, 4, 16, &[]).is_err());
    }
}
