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
use vergence_ui::{App, Bar, BarButton, BarIcon, Button, Frame, Input, Waker, Window};

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
    /// The bar's privacy button as last described (the host's answer, and
    /// whether the session is live): asked again only when it changed.
    bar_said: Option<(bool, bool, String)>,
    /// The pointer is over the window, and the window has the keyboard: the
    /// keyboard is taken whole only while both hold.
    inside: bool,
    focused: bool,
    /// This keyboard's Caps Lock and Num Lock as last seen, and as last told
    /// the host (version 3): the host sets its own to match.
    locks_seen: Option<(bool, bool)>,
    locks_told: Option<(bool, bool)>,
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
            bar_said: None,
            inside: false,
            focused: false,
            locks_seen: None,
            locks_told: None,
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
        // (Privacy counts as input: a view of this very computer never blanks its own desk.)
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
        // The far machine's lock keys put in step as it takes the keyboard.
        if on && let Some(l) = self.locks_seen {
            self.send(ToHost::Locks { caps: l.0, num: l.1 });
            self.locks_told = Some(l);
        }
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
        // On whole device pixels: a tile at a fraction of a pixel is blended
        // at its edges, and the seams show as a faint grid (Dave, 2026-11-23).
        let snap = |v: f32| (v * frame.scale).round() / frame.scale;
        let (ox, oy) =
            (snap((aw - self.size.0 as f32 * k) / 2.0), snap(frame.top + (ah - self.size.1 as f32 * k) / 2.0));
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
        // As a KVM's software does, the keyboard follows the pointer (Dave,
        // 2026-11-23): leaving the window gives it back to this computer at
        // once (its shortcuts work again), coming back in takes it again if
        // the window has the keyboard; a click takes it too.
        match input {
            _ if self.view_only => {}
            Input::Focus(on) => {
                self.focused = *on;
                if *on && self.inside {
                    window.capture_keys(true);
                }
            }
            Input::PointerLeft => {
                self.inside = false;
                if self.captured {
                    window.capture_keys(false);
                }
            }
            Input::PointerMoved { .. } if !self.inside => {
                self.inside = true;
                if self.focused && !self.captured {
                    window.capture_keys(true);
                }
            }
            Input::Button { pressed: true, .. } if !self.captured => window.capture_keys(true),
            _ => {}
        }
        match input {
            Input::PointerMoved { x, y } => {
                if let Some((px, py)) = self.to_view(*x, *y) {
                    self.send(ToHost::Pointer { x: px, y: py });
                }
            }
            Input::Button { button, pressed, x, y, modifiers } => {
                self.locks_seen = Some((modifiers.caps_lock, modifiers.num_lock));
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
            // are the host's to make. Its lock keys are put in step first if
            // they drifted (Caps Lock pressed while the keyboard was this
            // machine's), but never on Caps Lock or Num Lock themselves: those
            // go over and toggle both.
            Input::Key { code, pressed, repeat: false, modifiers, .. } if *code > 0 => {
                let locks = (modifiers.caps_lock, modifiers.num_lock);
                self.locks_seen = Some(locks);
                if *pressed && *code != 58 && *code != 69 && self.locks_told != Some(locks) {
                    self.send(ToHost::Locks { caps: locks.0, num: locks.1 });
                    self.locks_told = Some(locks);
                } else if *code == 58 || *code == 69 {
                    self.locks_told = None;
                }
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

    /// The server draws the bar, and with it the window's own button (no
    /// tabs: Bar::Tools only).
    fn tabs_by_server(&self) -> bool {
        true
    }

    /// The bar's own button: blank the host's desk (its monitors locked
    /// behind a curtain while this window goes on using it), or show it.
    fn bar(&mut self) -> Option<Bar> {
        let (live, privacy, host) = self
            .session
            .as_ref()
            .and_then(|s| s.shared.lock().ok().map(|sh| (sh.state == State::Live, sh.privacy, sh.host.clone())))
            .unwrap_or((false, false, String::new()));
        // The machine named, not "remote" (from the far side, this one is the
        // remote): "Lock Hubble's Screen" says what happens there (Dave,
        // 2026-11-23: "Lock Remote Screen and Show Remote Screen would be
        // more accurate").
        let host = if host.is_empty() { self.host.clone() } else { host };
        let now = (live && !self.view_only, privacy, host);
        if self.bar_said.as_ref() == Some(&now) {
            return None;
        }
        let name = if now.1 { format!("Unlock {}'s Screen", now.2) } else { format!("Lock {}'s Screen", now.2) };
        let enabled = now.0;
        self.bar_said = Some(now);
        Some(Bar::Tools { buttons: vec![BarButton { id: 1, icon: BarIcon::Label, name, enabled }], from: 0.0 })
    }

    fn bar_button(&mut self, id: u32, _window: &mut Window) {
        if id != 1 || self.view_only {
            return;
        }
        let privacy = self.session.as_ref().and_then(|s| s.shared.lock().ok().map(|sh| sh.privacy)).unwrap_or(false);
        self.send(ToHost::Privacy { on: !privacy });
    }

    fn close_asked(&mut self, _window: &mut Window) -> bool {
        if let Some(s) = self.session.take() {
            s.close();
        }
        true
    }
}
