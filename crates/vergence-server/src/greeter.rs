//! The login screen (VDM milestone 1, step 4), drawn by the server itself in
//! greeter mode, as the lock is (the password never leaves a `Secret`; the
//! server starts no program: main.rs, review C3). It shows the machine's
//! people (Dave: "1 a", a list with Other… to type a name; initials, no
//! pictures: "3 yes"), the chosen person's PAM conversation with vdm over the
//! greeter socket (whatever the system's login stack asks), and the session
//! to start (Dave: "5 b"; vergence-dm's sessions list, by index). Every key
//! and press is the login screen's: the seat is in its lock state for the
//! whole time (Ctrl+Alt+Backspace and the rest do nothing). No paste.
//!
//! Sleep, Restart and Shut Down along the bottom (step 6), carried out by
//! vdm; Restart and Shut Down ask for a second press within five seconds.
//!
//! Keys: Up and Down choose the person; typing goes into the field (the name
//! for Other…, then the password); Enter answers; Escape clears; F2 changes
//! the session. A press on a person chooses them; on the session line,
//! changes it.

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
    output::Output,
    reexports::calloop::channel::{self, Event},
    utils::{Logical, Point, Rectangle, Transform},
};
use vergence_dm::proto::{self, Service, ToVdm};
use vergence_dm::secret::Secret;
use vergence_scene::{Color, Point as ScenePoint, Rect as SceneRect, Scene};

use crate::{Server, decor::SceneElement, lock::Heard};

/// A person who may log in here: their account's name and what to call them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Person {
    login: String,
    name: String,
}

/// The machine's people, from /etc/passwd (world-readable): not root, not a
/// system account, with a real shell (vdm checks again before PAM runs).
fn people() -> Vec<Person> {
    let text = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let mut out: Vec<Person> = text
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            let uid: u32 = f[2].parse().ok()?;
            let shell = f[6];
            let real = !shell.ends_with("/nologin") && !shell.ends_with("/false") && std::path::Path::new(shell).is_file();
            (uid != 0 && uid >= vergence_dm::paths::MIN_UID && real).then(|| {
                let gecos = f[4].split(',').next().unwrap_or("").trim();
                Person { login: f[0].to_string(), name: if gecos.is_empty() { f[0].to_string() } else { gecos.to_string() } }
            })
        })
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    out.dedup();
    out
}

#[derive(Default)]
pub struct Greeter {
    people: Vec<Person>,
    /// The chosen person; `people.len()` is Other….
    chosen: usize,
    /// Other…: the name typed (names are not secret); asked first.
    other: String,
    naming: bool,
    field: Option<Secret>,
    prompt: String,
    echo: bool,
    message: Option<(String, bool)>,
    sessions: Vec<vergence_dm::sessions::Session>,
    session: usize,
    talk: Option<std::sync::mpsc::Sender<Option<Secret>>>,
    /// vdm or PAM said why during this conversation: a "no" keeps its words.
    told_why: bool,
    asked: bool,
    submit: bool,
    waiting: bool,
    pause_until: Option<Instant>,
    heard: Option<channel::Sender<Heard>>,
    /// Where the people's rows and the session line are, as last drawn.
    rows: Vec<(usize, Rectangle<i32, Logical>)>,
    session_line: Rectangle<i32, Logical>,
    power: Vec<(vergence_dm::power::Power, Rectangle<i32, Logical>)>,
    /// Restart or Shut Down pressed once: a second press within 5 s does it.
    confirm: Option<(vergence_dm::power::Power, Instant)>,
    dim: Option<(Id, CommitCounter)>,
    picture: Option<(u64, MemoryRenderBuffer, i32, i32)>,
}

impl Server {
    /// The login screen begins: the seat takes every key and press, the people
    /// and the sessions are read.
    pub fn start_greeter(&mut self) {
        let (tx, rx) = channel::channel::<Heard>();
        let res = self.loop_handle.insert_source(rx, |event, _, st| {
            if let Event::Msg(h) = event {
                st.greeter_heard(h);
            }
        });
        if let Err(e) = res {
            tracing::warn!("greeter: {e}");
        }
        let g = &mut self.greeter;
        g.heard = Some(tx);
        g.people = people();
        g.sessions = vergence_dm::sessions::list();
        g.field = Secret::new();
        g.prompt = "Password".into();
        g.naming = g.people.is_empty();
        tracing::info!("greeter: {} people, {} sessions", g.people.len(), g.sessions.len());
        let out = self.input.seat.lock();
        let pos = self.pointer_location();
        self.apply(out, pos, smithay::backend::input::InputTime::now(), &crate::dispatch::Came::Nothing);
        self.redraw_all();
    }

    fn greeter_login(&self) -> Option<String> {
        let g = &self.greeter;
        match g.people.get(g.chosen) {
            Some(p) => Some(p.login.clone()),
            None => (!g.other.is_empty()).then(|| g.other.clone()),
        }
    }

    fn greeter_pause_left(&self) -> u64 {
        self.greeter.pause_until.map_or(0, |u| {
            let left = u.saturating_duration_since(Instant::now());
            left.as_secs() + u64::from(left.subsec_nanos() > 0)
        })
    }

    /// Once a second (settings.rs): the clock, and the pause counting down.
    pub fn greeter_tick(&mut self) {
        if !crate::greeter() {
            return;
        }
        if self.greeter.pause_until.is_some() && self.greeter_pause_left() == 0 {
            self.greeter.pause_until = None;
            self.greeter.message = None;
        }
        self.redraw_all();
    }

    /// Give up a conversation in progress (another person chosen, Escape).
    fn greeter_drop_talk(&mut self) {
        let g = &mut self.greeter;
        if let Some(t) = g.talk.take() {
            let _ = t.send(None);
        }
        g.asked = false;
        g.submit = false;
        g.waiting = false;
        g.prompt = "Password".into();
        g.echo = false;
    }

    fn greeter_choose(&mut self, i: usize) {
        if i == self.greeter.chosen || i > self.greeter.people.len() {
            return;
        }
        self.greeter_drop_talk();
        let g = &mut self.greeter;
        g.chosen = i;
        g.naming = i == g.people.len();
        g.other.clear();
        g.message = None;
        if let Some(f) = g.field.as_mut() {
            f.clear();
        }
    }

    pub fn greeter_key(&mut self, sym: u32, typed: Option<char>, pressed: bool) {
        use xkbcommon::xkb::keysyms as k;
        if !pressed || self.greeter_pause_left() > 0 {
            return;
        }
        let n = self.greeter.people.len() + 1;
        match sym {
            k::KEY_Up if !self.greeter.waiting => self.greeter_choose((self.greeter.chosen + n - 1) % n),
            k::KEY_Down if !self.greeter.waiting => self.greeter_choose((self.greeter.chosen + 1) % n),
            k::KEY_F2 if !self.greeter.sessions.is_empty() => self.greeter.session = (self.greeter.session + 1) % self.greeter.sessions.len(),
            k::KEY_Escape => {
                self.greeter_drop_talk();
                if let Some(f) = self.greeter.field.as_mut() {
                    f.clear();
                }
                if self.greeter.chosen == self.greeter.people.len() {
                    self.greeter.other.clear();
                    self.greeter.naming = true;
                }
            }
            k::KEY_BackSpace => {
                if self.greeter.naming {
                    self.greeter.other.pop();
                } else if let Some(f) = self.greeter.field.as_mut() {
                    f.pop_char();
                }
            }
            k::KEY_Return | k::KEY_KP_Enter => {
                if self.greeter.naming {
                    if !self.greeter.other.is_empty() {
                        self.greeter.naming = false;
                    }
                } else {
                    if self.greeter.talk.is_none() && !self.greeter.waiting {
                        self.greeter_talk();
                    }
                    self.greeter.submit = true;
                    self.greeter_answer();
                }
            }
            _ => {
                let Some(c) = typed.filter(|c| !c.is_control()) else { return };
                if self.greeter.naming {
                    // A name: only what vdm takes (letters, digits, . _ -).
                    if (c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) && self.greeter.other.len() < 64 {
                        self.greeter.other.push(c);
                    }
                } else {
                    if self.greeter.talk.is_none() && !self.greeter.waiting {
                        self.greeter_talk();
                    }
                    if let Some(f) = self.greeter.field.as_mut() {
                        let mut b = [0u8; 4];
                        f.push_str(c.encode_utf8(&mut b));
                    }
                }
            }
        }
        self.redraw_all();
    }

    /// A press: a person, or the session line.
    pub fn greeter_press(&mut self, at: Point<f64, Logical>) {
        if self.greeter.waiting {
            return;
        }
        if let Some(&(p, _)) = self.greeter.power.iter().find(|(_, r)| r.to_f64().contains(at)) {
            self.greeter_power(p);
        } else if let Some(&(i, _)) = self.greeter.rows.iter().find(|(_, r)| r.to_f64().contains(at)) {
            self.greeter_choose(i);
        } else if self.greeter.session_line.to_f64().contains(at) && !self.greeter.sessions.is_empty() {
            self.greeter.session = (self.greeter.session + 1) % self.greeter.sessions.len();
        }
        self.redraw_all();
    }

    /// Sleep at once; Restart and Shut Down on a second press.
    fn greeter_power(&mut self, p: vergence_dm::power::Power) {
        use vergence_dm::power::Power;
        let confirmed = matches!(self.greeter.confirm, Some((q, at)) if q == p && at.elapsed() < Duration::from_secs(5));
        if p != Power::Sleep && !confirmed {
            self.greeter.confirm = Some((p, Instant::now()));
            let what = if p == Power::Restart { "Restart" } else { "Shut Down" };
            self.greeter.message = Some((format!("Press {what} again to {}", what.to_lowercase()), false));
            return;
        }
        self.greeter.confirm = None;
        match vergence_dm::power::ask(p) {
            Ok(()) => {
                tracing::info!("greeter: vdm will {}", p.word());
                self.greeter.message = Some((if p == Power::Sleep { "Going to sleep…" } else if p == Power::Restart { "Restarting…" } else { "Shutting down…" }.into(), false));
            }
            Err(e) => {
                tracing::warn!("greeter: {}: {e}", p.word());
                self.greeter.message = Some((e, true));
            }
        }
    }

    /// A conversation with vdm for the chosen person, in a thread of its own.
    fn greeter_talk(&mut self) {
        let (Some(user), Some(heard)) = (self.greeter_login(), self.greeter.heard.clone()) else { return };
        let session = self.greeter.session.min(u8::MAX as usize) as u8;
        let (tx, rx) = std::sync::mpsc::channel();
        self.greeter.talk = Some(tx);
        self.greeter.told_why = false;
        let first = vec![ToVdm::Hello { version: proto::VERSION, service: Service::Login }, ToVdm::Login { user, session }];
        if let Err(e) = std::thread::Builder::new().name("greeter".into()).spawn(move || crate::lock::converse(heard, rx, vergence_dm::paths::GREETER_SOCKET, first)) {
            tracing::warn!("greeter: {e}");
            self.greeter.talk = None;
        }
    }

    fn greeter_answer(&mut self) {
        let g = &mut self.greeter;
        if !g.asked || !g.submit {
            return;
        }
        let Some(t) = g.talk.as_ref() else { return };
        let Some(answer) = std::mem::replace(&mut g.field, Secret::new()) else { return };
        if t.send(Some(answer)).is_ok() {
            g.asked = false;
            g.submit = false;
            g.waiting = true;
            g.message = None;
        }
    }

    fn greeter_heard(&mut self, h: Heard) {
        match h {
            Heard::Prompt { echo, text } => {
                let g = &mut self.greeter;
                g.prompt = text.trim().trim_end_matches(':').to_string();
                g.echo = echo;
                g.waiting = false;
                g.asked = true;
                self.greeter_answer();
            }
            Heard::Info(t) => self.greeter.message = Some((t, false)),
            Heard::Error(t) => {
                self.greeter.message = Some((t, true));
                self.greeter.told_why = true;
            }
            Heard::Done { ok: true, .. } => {
                tracing::info!("greeter: vdm said yes");
                let g = &mut self.greeter;
                g.talk = None;
                g.waiting = false;
                g.message = Some(("Welcome. Starting your session…".into(), false));
            }
            Heard::Done { ok: false, wait } => {
                let g = &mut self.greeter;
                g.talk = None;
                g.waiting = false;
                g.asked = false;
                g.prompt = "Password".into();
                g.echo = false;
                if let Some(f) = g.field.as_mut() {
                    f.clear();
                }
                if wait > 0 {
                    g.pause_until = Some(Instant::now() + Duration::from_secs(wait as u64));
                }
                if !g.told_why {
                    g.message = Some(("That was not right. Try again.".into(), true));
                }
            }
            Heard::Gone(why) => {
                tracing::warn!("greeter: {why}");
                let g = &mut self.greeter;
                g.talk = None;
                g.waiting = false;
                g.message = Some(("Waiting for vdm…".into(), true));
            }
            Heard::LockNow => {}
        }
        self.redraw_all();
    }

    /// All a screen shows at the login screen: the dialog (on the pointer's
    /// screen), a dimming, the wallpaper.
    pub fn greeter_elements(&mut self, renderer: &mut GlesRenderer, output: &Output, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        let mut elements = Vec::new();
        if out.to_f64().contains(self.pointer_location())
            && let Some(e) = self.greeter_panel(renderer, out, scale)
        {
            elements.push(e);
        }
        let (id, commit) = self.greeter.dim.get_or_insert_with(|| (Id::new(), CommitCounter::default())).clone();
        let phys = Rectangle::<i32, smithay::utils::Physical>::from_size(out.size.to_f64().to_physical(scale).to_i32_round());
        elements.push(SceneElement::Solid(SolidColorRenderElement::new(id, phys, commit, [0.0, 0.0, 0.0, 0.45], Kind::Unspecified)));
        if let Some(mode) = output.current_mode()
            && let Some(e) = self.wallpaper_element(renderer, &output.name(), mode.size, out.size)
        {
            elements.push(SceneElement::Memory(e));
        }
        elements
    }

    fn greeter_panel(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Option<SceneElement> {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let (local, _) = vergence_shell::tiles::now_at(secs)?;
        let time = format!("{:02}:{:02}", local.hour, local.min);
        let host = std::fs::read_to_string("/etc/hostname").ok().map(|h| h.trim().to_string()).filter(|h| !h.is_empty()).unwrap_or_else(|| {
            std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
        });
        let paused = self.greeter_pause_left();
        let g = &self.greeter;
        let n_rows = g.people.len() + 1;
        let dots = g.field.as_ref().map_or(0, |f| f.chars());
        let shown = if g.naming { Some(g.other.clone()) } else if g.echo { g.field.as_ref().map(|f| String::from_utf8_lossy(f.as_bytes()).into_owned()) } else { None };
        let session_name = g.sessions.get(g.session).map_or("Vergence".to_string(), |s| s.name.clone());
        let (w, row_h) = (420.0f32, 34.0f32);
        let list_top = 120.0f32;
        let field_top = list_top + n_rows as f32 * row_h + 18.0;
        let h = field_top + 160.0;
        let (wi, hi) = (w as i32, h as i32);
        let at = (out.loc.x + (out.size.w - wi) / 2, out.loc.y + (out.size.h - hi) / 2);
        let mut hsh = std::collections::hash_map::DefaultHasher::new();
        (&time, &host, &g.people, g.chosen, dots, &shown).hash(&mut hsh);
        (&g.prompt, &g.message, g.waiting, paused, &session_name, g.naming, scale.to_bits()).hash(&mut hsh);
        let key = hsh.finish();
        self.greeter.rows = (0..n_rows).map(|i| (i, Rectangle::new((at.0 + 30, at.1 + (list_top + i as f32 * row_h) as i32).into(), (wi - 60, row_h as i32).into()))).collect();
        self.greeter.session_line = Rectangle::new((at.0 + 30, at.1 + (field_top + 86.0) as i32).into(), (wi - 60, 22).into());
        // Sleep, Restart, Shut Down: three buttons in a row along the bottom.
        let (pw, ph, pgap) = (100.0f32, 28.0f32, 12.0f32);
        let power_y = field_top + 120.0;
        let power_x0 = (w - 3.0 * pw - 2.0 * pgap) / 2.0;
        let power_local: Vec<(vergence_dm::power::Power, &str, f32)> = [(vergence_dm::power::Power::Sleep, "Sleep"), (vergence_dm::power::Power::Restart, "Restart"), (vergence_dm::power::Power::ShutDown, "Shut Down")]
            .into_iter()
            .enumerate()
            .map(|(i, (p, l))| (p, l, power_x0 + i as f32 * (pw + pgap)))
            .collect();
        self.greeter.power = power_local.iter().map(|(p, _, x)| (*p, Rectangle::new((at.0 + *x as i32, at.1 + power_y as i32).into(), (pw as i32, ph as i32).into()))).collect();
        if self.greeter.picture.as_ref().map(|(k, ..)| *k) != Some(key) {
            let s = vergence_look::shelf_look(false);
            let ink = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
            let accent = { let a = vergence_look::ACCENT; Color::rgb(a.0, a.1, a.2) };
            let people = self.greeter.people.clone();
            let chosen = self.greeter.chosen;
            let naming = self.greeter.naming;
            let prompt = if naming { "Name".to_string() } else { self.greeter.prompt.clone() };
            let message = self.greeter.message.clone();
            let waiting = self.greeter.waiting;
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            let face = fonts.face("sans-serif", 400, false)?;
            let bold = fonts.face("sans-serif", 600, false).unwrap_or(face);
            let mut scene = Scene::new(w, h);
            scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS * 2.0, Color::rgba(s.menu.0, s.menu.1, s.menu.2, 240));
            scene.stroke(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS * 2.0, vergence_look::EDGE, ink(s.edge));
            let centered = |fonts: &mut vergence_text::Fonts, scene: &mut Scene, f, size: f32, color, y: f32, t: &str| {
                let m = fonts.metrics(f, size);
                let tw = fonts.layout(f, size, t).1;
                vergence_render::text(scene, fonts, f, size, color, ScenePoint::new(((w - tw) / 2.0).round(), (y + m.ascent).round()), t);
            };
            centered(fonts, &mut scene, face, 44.0, ink(s.menu_ink), 16.0, &time);
            centered(fonts, &mut scene, face, 13.0, ink(s.menu_ink_off), 78.0, &host);
            // The people: initials in a circle and a name; the chosen one shaded.
            for i in 0..n_rows {
                let y = list_top + i as f32 * row_h;
                let r = SceneRect::new(30.0, y, w - 60.0, row_h - 4.0);
                if i == chosen {
                    scene.fill_rounded(r, vergence_look::RADIUS, ink(s.hover));
                }
                let (initials, name) = match people.get(i) {
                    Some(p) => (p.name.split_whitespace().filter_map(|x| x.chars().next()).take(2).collect::<String>().to_uppercase(), p.name.clone()),
                    None => ("…".to_string(), "Other…".to_string()),
                };
                scene.fill_rounded(SceneRect::new(r.x + 6.0, r.y + 3.0, 24.0, 24.0), 12.0, ink(s.tile));
                let iw = fonts.layout(bold, 11.0, &initials).1;
                let im = fonts.metrics(bold, 11.0);
                vergence_render::text(&mut scene, fonts, bold, 11.0, ink(s.tile_ink), ScenePoint::new((r.x + 18.0 - iw / 2.0).round(), (r.y + 15.0 - (im.ascent + im.descent) / 2.0 + im.ascent).round()), &initials);
                let m = fonts.metrics(face, 14.0);
                vergence_render::text(&mut scene, fonts, face, 14.0, ink(s.menu_ink), ScenePoint::new(r.x + 40.0, (r.y + (r.h - m.ascent - m.descent) / 2.0 + m.ascent).round()), &name);
            }
            // The field.
            centered(fonts, &mut scene, face, 12.0, ink(s.menu_ink_off), field_top, &prompt);
            let field = SceneRect::new(40.0, field_top + 20.0, w - 80.0, 34.0);
            if paused > 0 {
                scene.fill_rounded(field, vergence_look::RADIUS, Color::rgba(s.field.0, s.field.1, s.field.2, 110));
                scene.stroke(field, vergence_look::RADIUS, vergence_look::EDGE, ink(s.edge));
            } else {
                scene.fill_rounded(field, vergence_look::RADIUS, ink(s.field));
                scene.stroke(field, vergence_look::RADIUS, vergence_look::EDGE, accent);
            }
            match &shown {
                Some(t) => {
                    let m = fonts.metrics(face, 14.0);
                    vergence_render::text(&mut scene, fonts, face, 14.0, ink(s.menu_ink), ScenePoint::new(field.x + 12.0, (field.y + (field.h - m.ascent - m.descent) / 2.0 + m.ascent).round()), t);
                }
                None => {
                    for i in 0..dots.min(24) {
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
            let color = if bad { Color::rgb(0xff, 0x8a, 0x80) } else { ink(s.menu_ink_off) };
            centered(fonts, &mut scene, face, 12.0, color, field_top + 62.0, &msg);
            centered(fonts, &mut scene, face, 12.0, ink(s.menu_ink_off), field_top + 88.0, &format!("Session: {session_name}  (F2 to change)"));
            for (_, label, x) in &power_local {
                let r = SceneRect::new(*x, power_y, pw, ph);
                scene.fill_rounded(r, vergence_look::RADIUS, ink(s.tile));
                let lw = fonts.layout(face, 12.5, label).1;
                let lm = fonts.metrics(face, 12.5);
                vergence_render::text(&mut scene, fonts, face, 12.5, ink(s.tile_ink), ScenePoint::new((r.x + (r.w - lw) / 2.0).round(), (r.y + (r.h - lm.ascent - lm.descent) / 2.0 + lm.ascent).round()), label);
            }
            let p = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)).ok()?;
            let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
            self.greeter.picture = Some((key, buffer, wi, hi));
        }
        let (_, buffer, wi, hi) = self.greeter.picture.as_ref()?;
        let area: Rectangle<i32, Logical> = Rectangle::new(at.into(), (*wi, *hi).into());
        let a = (area.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let b = (area.loc + area.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(area.size), Kind::Unspecified).ok().map(SceneElement::Memory)
    }
}
