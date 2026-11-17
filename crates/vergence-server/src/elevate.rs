//! The elevation prompt (docs/elevation-milestone-1.md): vdm asks, over the
//! session's standing connection, for the person to approve one named
//! action a program asked for. The server draws it as it draws the log out
//! dialog (every screen dimmed, a panel in front of everything, every key
//! and press its own while it shows) and the password as the lock does (a
//! `Secret` from the first key, no paste, PAM's conversation with vdm on
//! the elevate socket). It says what is asked, in vdm's words, and which
//! program asks, as the kernel named it to vdm. Escape or Cancel declines;
//! Enter or Allow answers. The lock, if it comes on, is in front of it.

use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Id, Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::SolidColorRenderElement,
            },
            gles::GlesRenderer,
            utils::CommitCounter,
        },
    },
    reexports::calloop::channel::{self, Event},
    utils::{Logical, Point, Rectangle, Transform},
};
use vergence_dm::proto::{self, Service, ToVdm};
use vergence_dm::secret::Secret;
use vergence_scene::{Color, Point as ScenePoint, Rect as SceneRect, Scene};

use crate::{Server, decor::SceneElement, lock::Heard};

/// What vdm asked to show.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ask {
    pub id: u128,
    pub what: String,
    pub who: String,
}

#[derive(Default)]
pub struct Elevate {
    pub ask: Option<Ask>,
    field: Option<Secret>,
    prompt: String,
    echo: bool,
    message: Option<(String, bool)>,
    talk: Option<std::sync::mpsc::Sender<Option<Secret>>>,
    asked: bool,
    submit: bool,
    waiting: bool,
    pause_until: Option<Instant>,
    heard: Option<channel::Sender<Heard>>,
    /// Cancel and Allow, as last drawn.
    buttons: [Rectangle<i32, Logical>; 2],
    dim: Option<(Id, CommitCounter)>,
    picture: Option<(u64, MemoryRenderBuffer, i32, i32)>,
    /// When it came up: keys and presses are taken only after a moment
    /// (review finding 4: typing meant for another window ran into it).
    shown_at: Option<Instant>,
    /// The window that had the keyboard, given it back when it closes.
    before: Option<crate::win::Win>,
}

/// How long the prompt takes nothing after it comes up.
const SETTLE: Duration = Duration::from_millis(750);

const W: f32 = 480.0;

impl Server {
    /// Whether the prompt is up (and not behind the lock).
    pub fn elevating(&self) -> bool {
        self.elevate.ask.is_some() && !self.lock.on
    }

    /// vdm's session messages for the prompt: show it, or it is over.
    pub(crate) fn elevate_from_vdm(&mut self, h: Heard) {
        match h {
            Heard::Ask { id, what, who } => {
                if self.elevate.ask.is_some() {
                    // One at a time: vdm keeps it so too; a second is declined.
                    decline(id);
                    return;
                }
                if self.elevate.heard.is_none() {
                    let (tx, rx) = channel::channel::<Heard>();
                    if let Err(e) = self.loop_handle.insert_source(rx, |event, _, st| {
                        if let Event::Msg(h) = event {
                            st.elevate_heard(h);
                        }
                    }) {
                        tracing::warn!("elevate: {e}");
                    }
                    self.elevate.heard = Some(tx);
                }
                tracing::info!("elevate: {who} asks: {what}");
                self.close_menu();
                self.launcher_close();
                self.session_dialog_close();
                self.card_close();
                self.drawer_close();
                // No program keeps the keyboard or the pointer while it shows,
                // nor draws its own cursor over it (finding 1: a program's
                // cursor could have covered the panel's words).
                self.elevate.before = self.keyboard_window();
                if let Some(k) = self.seat.get_keyboard() {
                    k.set_focus(self, None, smithay::utils::SERIAL_COUNTER.next_serial());
                }
                if let Some(old) = self.input.keyboard_native.take() {
                    self.native_keyboard_leave(&old);
                }
                if let Some(p) = self.seat.get_pointer() {
                    let pos = self.pointer_location();
                    p.motion(self, None, &smithay::input::pointer::MotionEvent { location: pos, serial: smithay::utils::SERIAL_COUNTER.next_serial(), time: smithay::backend::input::InputTime::now() });
                    p.frame(self);
                }
                self.pointer_image.status = smithay::input::pointer::CursorImageStatus::default_named();
                let e = &mut self.elevate;
                e.shown_at = Some(Instant::now());
                e.ask = Some(Ask { id, what, who });
                e.field = Secret::new();
                e.prompt = "Password".into();
                e.echo = false;
                e.message = None;
                e.talk = None;
                e.asked = false;
                e.submit = false;
                e.waiting = false;
                self.redraw_all();
            }
            Heard::Withdrawn(id) if self.elevate.ask.as_ref().is_some_and(|a| a.id == id) => self.elevate_close(),
            _ => {}
        }
    }

    fn elevate_close(&mut self) {
        let e = &mut self.elevate;
        e.ask = None;
        e.field = None;
        if let Some(t) = e.talk.take() {
            let _ = t.send(None);
        }
        e.picture = None;
        // The keyboard back where it was.
        if let Some(w) = self.elevate.before.take().filter(smithay::utils::IsAlive::alive)
            && !self.lock.on
        {
            self.focus_window(&w, smithay::utils::SERIAL_COUNTER.next_serial());
        }
        self.redraw_all();
    }

    /// Still settling: it takes no key or press yet.
    fn elevate_settling(&self) -> bool {
        self.elevate.shown_at.is_some_and(|t| t.elapsed() < SETTLE)
    }

    fn elevate_decline(&mut self) {
        if let Some(a) = self.elevate.ask.as_ref() {
            tracing::info!("elevate: declined");
            decline(a.id);
        }
        self.elevate_close();
    }

    fn elevate_pause_left(&self) -> u64 {
        self.elevate.pause_until.map_or(0, |u| {
            let left = u.saturating_duration_since(Instant::now());
            left.as_secs() + u64::from(left.subsec_nanos() > 0)
        })
    }

    /// A key while it shows: typed into the field, Backspace, Escape
    /// (declines), Enter (answers).
    pub fn elevate_key(&mut self, sym: u32, typed: Option<char>, pressed: bool) {
        use xkbcommon::xkb::keysyms as k;
        if !pressed || self.elevate_settling() {
            return;
        }
        match sym {
            k::KEY_Escape => return self.elevate_decline(),
            k::KEY_Return | k::KEY_KP_Enter => self.elevate_submit(),
            _ if self.elevate_pause_left() > 0 => {}
            k::KEY_BackSpace => {
                if let Some(f) = self.elevate.field.as_mut() {
                    f.pop_char();
                }
            }
            _ => {
                if let Some(c) = typed.filter(|c| !c.is_control())
                    && let Some(f) = self.elevate.field.as_mut()
                {
                    let mut b = [0u8; 4];
                    f.push_str(c.encode_utf8(&mut b));
                }
            }
        }
        self.redraw_all();
    }

    /// A press: Cancel or Allow; elsewhere nothing.
    pub fn elevate_press(&mut self, at: Point<f64, Logical>) {
        if self.elevate_settling() {
            return;
        }
        if self.elevate.buttons[0].to_f64().contains(at) {
            return self.elevate_decline();
        }
        if self.elevate.buttons[1].to_f64().contains(at) {
            self.elevate_submit();
            self.redraw_all();
        }
    }

    fn elevate_submit(&mut self) {
        if self.elevate_pause_left() > 0 || self.elevate.waiting {
            return;
        }
        if self.elevate.talk.is_none() {
            let (Some(a), Some(heard)) = (self.elevate.ask.clone(), self.elevate.heard.clone()) else { return };
            let (tx, rx) = std::sync::mpsc::channel();
            self.elevate.talk = Some(tx);
            let first = vec![ToVdm::Hello { version: proto::VERSION, service: Service::Elevate }, ToVdm::Answering(a.id)];
            if let Err(e) = std::thread::Builder::new().name("elevate".into()).spawn(move || crate::lock::converse(heard, rx, vergence_dm::paths::ELEVATE_SOCKET, first)) {
                tracing::warn!("elevate: {e}");
                self.elevate.talk = None;
                return;
            }
        }
        self.elevate.submit = true;
        self.elevate_answer();
    }

    fn elevate_answer(&mut self) {
        let e = &mut self.elevate;
        if !e.asked || !e.submit {
            return;
        }
        let Some(t) = e.talk.as_ref() else { return };
        let Some(answer) = std::mem::replace(&mut e.field, Secret::new()) else { return };
        if t.send(Some(answer)).is_ok() {
            e.asked = false;
            e.submit = false;
            e.waiting = true;
            e.message = None;
        }
    }

    fn elevate_heard(&mut self, h: Heard) {
        if self.elevate.ask.is_none() {
            return;
        }
        match h {
            Heard::Prompt { echo, text } => {
                let e = &mut self.elevate;
                e.prompt = text.trim().trim_end_matches(':').to_string();
                e.echo = echo;
                e.waiting = false;
                e.asked = true;
                self.elevate_answer();
            }
            Heard::Info(t) => self.elevate.message = Some((t, false)),
            Heard::Error(t) => self.elevate.message = Some((t, true)),
            Heard::Done { ok: true, .. } => {
                tracing::info!("elevate: allowed");
                return self.elevate_close();
            }
            Heard::Done { ok: false, wait } => {
                let e = &mut self.elevate;
                e.talk = None;
                e.waiting = false;
                e.asked = false;
                e.prompt = "Password".into();
                e.echo = false;
                if let Some(f) = e.field.as_mut() {
                    f.clear();
                }
                if wait > 0 {
                    e.pause_until = Some(Instant::now() + Duration::from_secs(wait as u64));
                }
                e.message = Some(("That was not right. Try again.".into(), true));
            }
            Heard::Gone(why) => {
                tracing::warn!("elevate: {why}");
                let e = &mut self.elevate;
                e.talk = None;
                e.waiting = false;
                e.message = Some(("vdm could not be reached".into(), true));
            }
            Heard::Idle => {
                let e = &mut self.elevate;
                e.talk = None;
                e.waiting = false;
                e.asked = false;
            }
            _ => {}
        }
        self.redraw_all();
    }

    /// Once a second (settings.rs): the pause counting down.
    pub fn elevate_tick(&mut self) {
        if self.elevate.ask.is_some() && self.elevate.pause_until.is_some() {
            if self.elevate_pause_left() == 0 {
                self.elevate.pause_until = None;
                self.elevate.message = None;
            }
            self.redraw_all();
        }
    }

    /// In front of everything on a screen: the panel (on the pointer's
    /// screen), then the dimming over the whole of it.
    pub fn elevate_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        let mut elements = Vec::new();
        if out.to_f64().contains(self.pointer_location())
            && let Some(e) = self.elevate_panel(renderer, out, scale)
        {
            elements.push(e);
        }
        let (id, commit) = self.elevate.dim.get_or_insert_with(|| (Id::new(), CommitCounter::default())).clone();
        let phys = Rectangle::<i32, smithay::utils::Physical>::from_size(out.size.to_f64().to_physical(scale).to_i32_round());
        elements.push(SceneElement::Solid(SolidColorRenderElement::new(id, phys, commit, [0.0, 0.0, 0.0, 0.5], Kind::Unspecified)));
        elements
    }

    fn elevate_panel(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Option<SceneElement> {
        let light = self.light_style();
        let accent = self.accent();
        let paused = self.elevate_pause_left();
        let e = &self.elevate;
        let ask = e.ask.clone()?;
        let dots = e.field.as_ref().map_or(0, |f| f.chars());
        let shown = if e.echo { e.field.as_ref().map(|f| String::from_utf8_lossy(f.as_bytes()).into_owned()) } else { None };
        let (prompt, message, waiting) = (e.prompt.clone(), e.message.clone(), e.waiting);
        let mut hsh = std::collections::hash_map::DefaultHasher::new();
        (&ask, dots, &shown, &prompt, &message, waiting, paused, light, scale.to_bits(), accent.r, accent.g, accent.b).hash(&mut hsh);
        let key = hsh.finish();
        if self.elevate.picture.as_ref().map(|(k, ..)| *k) != Some(key) {
            let s = vergence_look::shelf_look(light);
            let ink = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            let face = fonts.face("sans-serif", 400, false)?;
            let bold = fonts.face("sans-serif", 600, false).unwrap_or(face);
            // What is asked, wrapped to the panel.
            let pad = 22.0f32;
            let mut lines: Vec<String> = Vec::new();
            for word in ask.what.split_whitespace() {
                match lines.last_mut() {
                    Some(l) if fonts.layout(face, 13.5, &format!("{l} {word}")).1 <= W - 2.0 * pad => {
                        l.push(' ');
                        l.push_str(word);
                    }
                    _ => lines.push(word.to_string()),
                }
            }
            let what_top = 58.0f32;
            let field_top = what_top + lines.len() as f32 * 19.0 + 30.0;
            let h = field_top + 120.0;
            let mut scene = Scene::new(W, h);
            scene.fill_rounded(SceneRect::new(0.0, 0.0, W, h), vergence_look::RADIUS * 2.0, ink(s.menu));
            scene.stroke(SceneRect::new(0.0, 0.0, W, h), vergence_look::RADIUS * 2.0, vergence_look::EDGE, ink(s.edge));
            let m = fonts.metrics(bold, 16.0);
            vergence_render::text(&mut scene, fonts, bold, 16.0, ink(s.menu_ink), ScenePoint::new(pad, (22.0 + m.ascent).round()), &format!("{} asks for permission", ask.who));
            let m = fonts.metrics(face, 13.5);
            for (i, l) in lines.iter().enumerate() {
                vergence_render::text(&mut scene, fonts, face, 13.5, ink(s.menu_ink), ScenePoint::new(pad, (what_top + i as f32 * 19.0 + m.ascent).round()), l);
            }
            let m = fonts.metrics(face, 12.0);
            vergence_render::text(&mut scene, fonts, face, 12.0, ink(s.menu_ink_off), ScenePoint::new(pad, (field_top - 18.0 + m.ascent).round()), &format!("{prompt}, to allow it (an administrator's own):"));
            let field = SceneRect::new(pad, field_top, W - 2.0 * pad, 32.0);
            scene.fill_rounded(field, vergence_look::RADIUS, if paused > 0 { Color::rgba(s.field.0, s.field.1, s.field.2, 110) } else { ink(s.field) });
            scene.stroke(field, vergence_look::RADIUS, vergence_look::EDGE, if paused > 0 { ink(s.edge) } else { accent });
            match &shown {
                Some(t) => {
                    let m = fonts.metrics(face, 14.0);
                    vergence_render::text(&mut scene, fonts, face, 14.0, ink(s.menu_ink), ScenePoint::new(field.x + 12.0, (field.y + (field.h - m.ascent - m.descent) / 2.0 + m.ascent).round()), t);
                }
                None => {
                    for i in 0..dots.min(28) {
                        scene.fill_rounded(SceneRect::new(field.x + 14.0 + i as f32 * 12.0, field.y + field.h / 2.0 - 4.0, 8.0, 8.0), 4.0, ink(s.menu_ink));
                    }
                }
            }
            let (msg, bad) = match (&message, waiting) {
                _ if paused > 0 => (format!("Try again in {paused} s"), true),
                (_, true) => ("Checking…".to_string(), false),
                (Some((m, bad)), _) => (m.clone(), *bad),
                (None, _) => (String::new(), false),
            };
            let m = fonts.metrics(face, 12.0);
            vergence_render::text(&mut scene, fonts, face, 12.0, if bad { Color::rgb(0xd9, 0x3f, 0x3f) } else { ink(s.menu_ink_off) }, ScenePoint::new(pad, (field_top + 44.0 + m.ascent).round()), &msg);
            // Cancel and Allow, Allow the default (Enter), at the bottom right.
            let (bw, bh, gap) = (104.0f32, 32.0f32, 10.0f32);
            let row_y = h - pad - bh;
            for (i, label) in ["Cancel", "Allow"].iter().enumerate() {
                let r = SceneRect::new(W - pad - 2.0 * bw - gap + i as f32 * (bw + gap), row_y, bw, bh);
                let (ground, text) = if i == 1 { (accent, Color::rgb(0xff, 0xff, 0xff)) } else { (ink(s.tile), ink(s.tile_ink)) };
                scene.fill_rounded(r, vergence_look::RADIUS, ground);
                let lw = fonts.layout(face, 13.5, label).1;
                let lm = fonts.metrics(face, 13.5);
                vergence_render::text(&mut scene, fonts, face, 13.5, text, ScenePoint::new((r.x + (r.w - lw) / 2.0).round(), (r.y + (r.h - lm.ascent - lm.descent) / 2.0 + lm.ascent).round()), label);
            }
            let p = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)).ok()?;
            let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
            self.elevate.picture = Some((key, buffer, W as i32, h as i32));
        }
        let (_, _, wi, hi) = self.elevate.picture.as_ref()?;
        let (wi, hi) = (*wi, *hi);
        let at = (out.loc.x + (out.size.w - wi) / 2, out.loc.y + (out.size.h - hi) / 2);
        let (bw, bh, gap, pad) = (104, 32, 10, 22);
        let row_y = at.1 + hi - pad - bh;
        self.elevate.buttons = [Rectangle::new((at.0 + wi - pad - 2 * bw - gap, row_y).into(), (bw, bh).into()), Rectangle::new((at.0 + wi - pad - bw, row_y).into(), (bw, bh).into())];
        let area: Rectangle<i32, Logical> = Rectangle::new(at.into(), (wi, hi).into());
        let a = (area.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let b = (area.loc + area.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
        let buffer = &self.elevate.picture.as_ref()?.1;
        MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(area.size), Kind::Unspecified).ok().map(SceneElement::Memory)
    }
}

/// Tell vdm the person said no to prompt `id` (in a thread: vdm answers nothing).
fn decline(id: u128) {
    let _ = std::thread::Builder::new().name("elevate-no".into()).spawn(move || {
        if let Ok(mut s) = std::os::unix::net::UnixStream::connect(vergence_dm::paths::ELEVATE_SOCKET) {
            let _ = proto::send_to_vdm(&mut s, &ToVdm::Hello { version: proto::VERSION, service: Service::Elevate });
            let _ = proto::send_to_vdm(&mut s, &ToVdm::Decline(id));
        }
    });
}
