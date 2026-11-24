//! A session's window (remote milestone 1, step 4;
//! docs/remote-milestone-1.md): another Vergence machine's desktop in a
//! window, used as if sitting at it.
//!
//!     vergence-remote-viewer HOST      (ssh HOST vergence-remote)
//!     vergence-remote-viewer localhost (this machine's own session)
//!
//! The connection is the portable core's (vergence-remote's client.rs);
//! this is the shell for Vergence: the picture drawn in the window, the
//! window's pointer, buttons, keys and scrolling sent as the host's own, and
//! the view resized with the window (the host draws at exactly the window's
//! pixels, so the picture is sharp, one pixel to one).
//!
//! The picture is kept as tiles: the toolkit sends the server only images
//! it does not have yet, so a frame that changed a corner costs that corner.

use std::sync::Arc;

use vergence_remote::client::{Session, State};
use vergence_remote::proto::ToHost;
use vergence_scene::{Color, Image, Point, Rect, Scene};
use vergence_text::Fonts;
use vergence_ui::{App, Button, Frame, Input, Waker, Window};

/// A tile's side in pixels.
const TILE: u32 = 256;
const GROUND: Color = Color::rgb(14, 14, 16);
const INK: Color = Color::rgb(226, 230, 235);
const SOFT: Color = Color::rgb(150, 156, 164);

pub struct Viewer {
    host: String,
    session: Option<Session>,
    waker: Option<Waker>,
    /// The picture as tiles (columns x rows), its size in pixels.
    tiles: Vec<Arc<Image>>,
    cols: u32,
    rows: u32,
    size: (u32, u32),
    /// The view's size last asked for (the window's pixels).
    want: (u32, u32),
    /// Where the picture is in the window: its top left, and window units per pixel.
    at: (f32, f32, f32),
    /// Keys held down, released if the window loses the keyboard (no key
    /// stays down on the host).
    held: Vec<u32>,
    buttons: Vec<u32>,
    /// The title last set.
    titled: String,
    /// The window has every key (the server's own shortcuts too), until
    /// Ctrl+Alt gives the keyboard back; a click takes it again.
    captured: bool,
    /// The command that reaches the host (`ssh HOST vergence-remote`).
    command: Vec<String>,
    /// Only looking, no input sent: a view of this very session, where input
    /// would come back to this window and go out again, round and round.
    view_only: bool,
}

fn this_host() -> String {
    let mut buf = [0u8; 256];
    unsafe extern "C" {
        fn gethostname(name: *mut std::ffi::c_char, len: usize) -> i32;
    }
    // SAFETY: gethostname(3) into a buffer of its length.
    if unsafe { gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return "A viewer".into();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

impl Viewer {
    /// A window for a session with `host` (an SSH destination, or "localhost").
    pub fn new(host: &str) -> Viewer {
        Viewer::with_command(host, vergence_remote::client::command_for(host))
    }

    /// A window for a session through `command` (tests: a stand-in host).
    pub fn with_command(host: &str, command: Vec<String>) -> Viewer {
        let view_only = host == "localhost";
        Viewer {
            command,
            view_only,
            host: host.to_string(),
            session: None,
            waker: None,
            tiles: Vec::new(),
            cols: 0,
            rows: 0,
            size: (0, 0),
            want: (0, 0),
            at: (0.0, 0.0, 0.0),
            held: Vec::new(),
            buttons: Vec::new(),
            titled: String::new(),
            captured: false,
        }
    }

    /// The tiles a frame touched, made again from the picture.
    fn take_frame(&mut self) {
        let Some(s) = &self.session else { return };
        let Ok(mut sh) = s.shared.lock() else { return };
        if sh.size.0 == 0 || sh.size.1 == 0 {
            return;
        }
        if sh.size != self.size {
            self.size = sh.size;
            self.cols = self.size.0.div_ceil(TILE);
            self.rows = self.size.1.div_ceil(TILE);
            let blank = Arc::new(Image { width: 1, height: 1, rgba: vec![0; 4] });
            self.tiles = (0..self.cols * self.rows).map(|_| blank.clone()).collect();
            sh.dirty = vec![(0, 0, self.size.0, self.size.1)];
        }
        let dirty = std::mem::take(&mut sh.dirty);
        let mut touched = vec![false; self.tiles.len()];
        for (x, y, w, h) in dirty {
            for ty in y / TILE..(y + h).div_ceil(TILE).min(self.rows) {
                for tx in x / TILE..(x + w).div_ceil(TILE).min(self.cols) {
                    touched[(ty * self.cols + tx) as usize] = true;
                }
            }
        }
        let stride = self.size.0 as usize * 4;
        for (i, t) in touched.iter().enumerate() {
            if !t {
                continue;
            }
            let (tx, ty) = (i as u32 % self.cols, i as u32 / self.cols);
            let (x0, y0) = (tx * TILE, ty * TILE);
            let (w, h) = (TILE.min(self.size.0 - x0), TILE.min(self.size.1 - y0));
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in y0..y0 + h {
                let at = y as usize * stride + x0 as usize * 4;
                rgba.extend_from_slice(&sh.picture[at..at + w as usize * 4]);
            }
            self.tiles[i] = Arc::new(Image { width: w, height: h, rgba });
        }
    }

    /// A place in the window as a place in the view's pixels, if on the picture.
    fn to_view(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (ox, oy, k) = self.at;
        if k <= 0.0 || self.size.0 == 0 {
            return None;
        }
        let (px, py) = ((x - ox) / k, (y - oy) / k);
        (px >= 0.0 && py >= 0.0 && px < self.size.0 as f32 && py < self.size.1 as f32).then_some((px, py))
    }

    /// Input to the host; a resize always (the view follows the window).
    fn send(&self, m: ToHost) {
        let input = !matches!(m, ToHost::Resize { .. } | ToHost::Bye | ToHost::Ack { .. } | ToHost::Hello { .. });
        if input && self.view_only {
            return;
        }
        if let Some(s) = &self.session {
            s.send(m);
        }
    }

    fn say(&self, scene: &mut Scene, fonts: &mut Fonts, frame: &Frame, line: &str, small: &str) {
        let Some(face) = fonts.face("sans-serif", 600, false) else { return };
        let w = vergence_render::text(scene, fonts, face, 18.0, INK, Point::new(-10_000.0, -10_000.0), line);
        let y = frame.top + (frame.height - frame.top) / 2.0;
        vergence_render::text(scene, fonts, face, 18.0, INK, Point::new((frame.width - w) / 2.0, y), line);
        if let Some(face) = fonts.face("sans-serif", 400, false) {
            let w = vergence_render::text(scene, fonts, face, 13.0, SOFT, Point::new(-10_000.0, -10_000.0), small);
            vergence_render::text(scene, fonts, face, 13.0, SOFT, Point::new((frame.width - w) / 2.0, y + 28.0), small);
        }
    }
}

impl App for Viewer {
    fn title(&self) -> String {
        let host = self
            .session
            .as_ref()
            .and_then(|s| s.shared.lock().ok().map(|sh| sh.host.clone()))
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| self.host.clone());
        if self.view_only {
            format!("{host} — Vergence Remote (this computer: looking only)")
        } else if self.captured {
            format!("{host} — Vergence Remote (Ctrl+Alt gives the keyboard back)")
        } else {
            format!("{host} — Vergence Remote")
        }
    }

    fn keys_captured(&mut self, on: bool, window: &mut Window) {
        self.captured = on;
        let title = self.title();
        if title != self.titled {
            window.set_title(&title);
            self.titled = title;
        }
    }

    fn app_id(&self) -> String {
        "vergence-remote-viewer".into()
    }

    fn background(&self) -> Color {
        GROUND
    }

    fn waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    fn woken(&mut self, window: &mut Window) {
        // The host's own name, once its Welcome came, in the title.
        let title = self.title();
        if title != self.titled {
            window.set_title(&title);
            self.titled = title;
        }
        window.redraw();
    }

    fn scene(&mut self, frame: &Frame, fonts: &mut Fonts) -> Scene {
        let mut scene = Scene::new(frame.width, frame.height);
        scene.fill(Rect::new(0.0, 0.0, frame.width, frame.height), GROUND);
        // The window's own pixels, below the bar: the view the host draws.
        let px = (
            (frame.width * frame.scale).round().max(16.0) as u32,
            ((frame.height - frame.top) * frame.scale).round().max(16.0) as u32,
        );
        if self.session.is_none() {
            let waker = self.waker.clone();
            let wake = move || {
                if let Some(w) = &waker {
                    w.wake();
                }
            };
            self.session = Some(Session::connect(&self.command, &this_host(), px, wake));
            self.want = px;
        } else if px != self.want {
            self.want = px;
            self.send(ToHost::Resize { width: px.0, height: px.1 });
        }
        self.take_frame();
        let state =
            self.session.as_ref().and_then(|s| s.shared.lock().ok().map(|sh| (sh.state.clone(), sh.notice.clone())));
        match state {
            Some((State::Ended(why), notice)) => {
                let line = notice.unwrap_or_else(|| "The connection ended".into());
                self.say(&mut scene, fonts, frame, &line, &why);
                return scene;
            }
            Some((State::Connecting, _)) if self.size.0 == 0 => {
                self.say(&mut scene, fonts, frame, &format!("Connecting to {}…", self.host), "over SSH");
                return scene;
            }
            _ => {}
        }
        if self.size.0 == 0 {
            return scene;
        }
        // The picture fitted into the window below the bar, centered: one
        // pixel to one when the host drew it at the window's size.
        let (aw, ah) = (frame.width, frame.height - frame.top);
        let k = (aw / self.size.0 as f32).min(ah / self.size.1 as f32);
        let (ox, oy) = ((aw - self.size.0 as f32 * k) / 2.0, frame.top + (ah - self.size.1 as f32 * k) / 2.0);
        self.at = (ox, oy, k);
        for (i, t) in self.tiles.iter().enumerate() {
            let (tx, ty) = (i as u32 % self.cols, i as u32 / self.cols);
            let id = scene.image(t.clone());
            let r = Rect::new(
                ox + (tx * TILE) as f32 * k,
                oy + (ty * TILE) as f32 * k,
                t.width as f32 * k,
                t.height as f32 * k,
            );
            scene.draw_image(id, r, 1.0);
        }
        scene
    }

    fn input(&mut self, input: &Input, window: &mut Window) {
        // The keyboard taken whole when the window gets it, and again at a
        // click after Ctrl+Alt gave it back (the server's shortcuts then go
        // to the far machine too; switching consoles stays here).
        match input {
            _ if self.view_only => {}
            Input::Focus(true) => window.capture_keys(true),
            Input::Button { pressed: true, .. } if !self.captured => window.capture_keys(true),
            _ => {}
        }
        match input {
            Input::PointerMoved { x, y } => {
                if let Some((px, py)) = self.to_view(*x, *y) {
                    self.send(ToHost::Pointer { x: px, y: py });
                }
            }
            Input::Button { button, pressed, x, y, .. } => {
                let code = match button {
                    Button::Left => 0x110,
                    Button::Right => 0x111,
                    Button::Middle => 0x112,
                    Button::Other(c) => *c,
                };
                if let Some((px, py)) = self.to_view(*x, *y) {
                    self.send(ToHost::Pointer { x: px, y: py });
                }
                if *pressed {
                    self.buttons.push(code);
                } else {
                    self.buttons.retain(|b| *b != code);
                }
                self.send(ToHost::Button { code, pressed: *pressed });
            }
            Input::Scroll { dx, dy, .. } => self.send(ToHost::Scroll { dx: *dx, dy: *dy }),
            // The physical key, so the host applies its own layout; repeats
            // are the host's to make.
            Input::Key { code, pressed, repeat: false, .. } if *code > 0 => {
                if *pressed {
                    self.held.push(*code);
                } else {
                    self.held.retain(|c| c != code);
                }
                self.send(ToHost::Key { code: *code, pressed: *pressed });
            }
            Input::Focus(false) => {
                for code in std::mem::take(&mut self.held) {
                    self.send(ToHost::Key { code, pressed: false });
                }
                for code in std::mem::take(&mut self.buttons) {
                    self.send(ToHost::Button { code, pressed: false });
                }
            }
            _ => {}
        }
    }

    fn close_asked(&mut self, _window: &mut Window) -> bool {
        if let Some(s) = self.session.take() {
            s.close();
        }
        true
    }
}
