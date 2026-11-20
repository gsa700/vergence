// Agents' permission (meaning milestone 1, step 4, part 1; whitepaper §7.2):
// an agent's first request about an app asks the person, in a dialog the
// server draws (no app can draw it), how long it may use that app: Just now
// (until it disconnects), For an hour, Until I log out (this session),
// Always, or Don't allow (asked again next time). Grants are per agent name
// and per app; Always and Never and the timed ones are kept (0600) in
// ~/.local/state/vergence/agents.tsv, the others in memory. A timed grant is
// checked at each request, not by a timer. While an agent works a tile says
// so ("Claude is using Calculator", and the time a timed grant has left);
// every request it makes of an app is logged in agents.log (0600).
//
// The dialog looks as the log out dialog does (session_dialog.rs): every
// screen dimmed, a panel on the pointer's screen, one row of buttons (Dave,
// 2026-11-20: "one row of buttons"), For an hour the default (Enter),
// Escape Don't allow. Requests wait while it asks, and run when it answers.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
    utils::{Logical, Point, Rectangle, Transform},
};
use vergence_protocol::agent::AgentRequest;
use vergence_scene::{Color, Point as ScenePoint, Rect as SceneRect, Scene};

use crate::{Server, decor::SceneElement};

/// How long "For an hour" lasts (Privacy > Agents will set it: step 4, part 2).
pub const TIMED_MINUTES: u64 = 60;
/// An agent counts as working this long after its last request (the tile).
const WORKING_SECS: u64 = 120;
/// The most requests waiting for the person's answer.
const PENDING_MAX: usize = 256;

/// How long a grant lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    /// While this bus connection lasts.
    Connection(u32),
    /// Until this time (Unix seconds).
    Until(u64),
    /// Until the session ends.
    Session,
    Always,
    Never,
}

/// What a grant says now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum May {
    Yes,
    No,
    Ask,
}

/// The buttons, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Button {
    DontAllow,
    JustNow,
    Session,
    Always,
    Hour,
}

const BUTTONS: [(Button, &str); 5] = [(Button::DontAllow, "Don't allow"), (Button::JustNow, "Just now"), (Button::Session, "Until I log out"), (Button::Always, "Always"), (Button::Hour, "For an hour")];
/// The default: For an hour.
const DEFAULT: usize = 4;
const W: i32 = 640;
const H: i32 = 170;
const PAD: i32 = 24;
const BUTTON_H: i32 = 34;
const BUTTON_GAP: i32 = 8;
const BUTTON_SIDE: f32 = 14.0;
const DEFAULT_SIDE: f32 = 16.0;
const TEXT: f32 = 13.5;

/// A question for the person: which agent, which app (its id and the name
/// shown), and the agent's bus connection (for "Just now").
#[derive(Debug, Clone, PartialEq)]
struct Question {
    agent: String,
    client: u32,
    app: String,
    shown: String,
}

#[derive(Default)]
pub struct Agents {
    /// By (agent name, app id).
    grants: HashMap<(String, String), Span>,
    loaded: bool,
    /// Requests waiting for an answer: the agent's bus client, the app, the request.
    pending: VecDeque<(u32, String, AgentRequest)>,
    /// The questions, the first one showing.
    asking: VecDeque<Question>,
    /// Who worked last, with what app, and when (the tile).
    last: Option<(String, String, Instant)>,
    tile_up: bool,
    focus: usize,
    panel: Rectangle<i32, Logical>,
    buttons: Vec<(usize, Rectangle<i32, Logical>)>,
    dim: Option<(Id, CommitCounter)>,
    picture: Option<(u64, MemoryRenderBuffer)>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn state_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))?;
    Some(base.join("vergence"))
}

/// A name as kept in the file: no tabs or line ends.
fn clean(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// The file's lines: agent, app, and always, never or until <seconds>.
fn parse(text: &str) -> HashMap<(String, String), Span> {
    let mut out = HashMap::new();
    for line in text.lines().take(10_000) {
        let mut f = line.split('\t');
        let (Some(agent), Some(app), Some(v)) = (f.next(), f.next(), f.next()) else { continue };
        let span = match v.split_once(' ') {
            None if v == "always" => Span::Always,
            None if v == "never" => Span::Never,
            Some(("until", t)) => match t.parse() {
                Ok(t) => Span::Until(t),
                Err(_) => continue,
            },
            _ => continue,
        };
        out.insert((agent.to_string(), app.to_string()), span);
    }
    out
}

fn render(grants: &HashMap<(String, String), Span>) -> String {
    let mut lines: Vec<String> = grants
        .iter()
        .filter_map(|((a, p), s)| {
            let v = match s {
                Span::Always => "always".to_string(),
                Span::Never => "never".to_string(),
                Span::Until(t) => format!("until {t}"),
                _ => return None,
            };
            Some(format!("{}\t{}\t{v}", clean(a), clean(p)))
        })
        .collect();
    lines.sort();
    lines.join("\n") + "\n"
}

/// The days from 1970 to a civil date (Howard Hinnant's algorithm), for the log's times.
fn utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

impl Agents {
    fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        if let Some(text) = state_dir().and_then(|d| std::fs::read_to_string(d.join("agents.tsv")).ok()) {
            self.grants.extend(parse(&text));
        }
    }

    fn save(&self) {
        // Tests never write the person's own grants.
        if cfg!(test) {
            return;
        }
        let Some(dir) = state_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let tmp = dir.join("agents.tsv.new");
        let ok = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).and_then(|mut f| f.write_all(render(&self.grants).as_bytes()));
        if ok.is_ok() {
            let _ = std::fs::rename(&tmp, dir.join("agents.tsv"));
        }
    }

    /// What agent `agent` (bus client `client`) may do with `app` now.
    fn may(&mut self, agent: &str, client: u32, app: &str) -> May {
        self.load();
        match self.grants.get(&(agent.to_string(), app.to_string())).copied() {
            None => May::Ask,
            Some(Span::Never) => May::No,
            Some(Span::Always | Span::Session) => May::Yes,
            Some(Span::Connection(c)) if c == client => May::Yes,
            Some(Span::Connection(_)) => May::Ask,
            Some(Span::Until(t)) if now_secs() < t => May::Yes,
            Some(Span::Until(_)) => {
                self.grants.remove(&(agent.to_string(), app.to_string()));
                self.save();
                May::Ask
            }
        }
    }

    /// The minutes a timed grant has left, if the agent has one for the app.
    fn minutes_left(&self, agent: &str, app: &str) -> Option<u64> {
        match self.grants.get(&(agent.to_string(), app.to_string())) {
            Some(Span::Until(t)) => Some(t.saturating_sub(now_secs()).div_ceil(60)),
            _ => None,
        }
    }
}

impl Server {
    pub fn agents_asking(&self) -> bool {
        !self.agents.asking.is_empty()
    }

    /// May the agent on bus client `client` use `app`? Asks the person (and
    /// keeps `request` to run after) when nothing says yet; None then.
    pub fn agent_gate(&mut self, client: u32, app: &str, shown: &str, request: &AgentRequest) -> Option<bool> {
        let agent = self.bus_agent_name(client)?;
        match self.agents.may(&agent, client, app) {
            May::Yes => {
                self.agents.last = Some((agent, app.to_string(), Instant::now()));
                self.agent_tile();
                Some(true)
            }
            May::No => Some(false),
            May::Ask => {
                if self.agents.pending.len() >= PENDING_MAX {
                    return Some(false);
                }
                self.agents.pending.push_back((client, app.to_string(), request.clone()));
                let q = Question { agent, client, app: app.to_string(), shown: shown.to_string() };
                if !self.agents.asking.iter().any(|a| a.agent == q.agent && a.app == q.app) {
                    let first = self.agents.asking.is_empty();
                    self.agents.asking.push_back(q);
                    if first {
                        self.agent_dialog_open();
                    }
                }
                None
            }
        }
    }

    /// May the agent use `app` without asking (a search across windows skips
    /// the ones it may not, and never asks about them)?
    pub fn agent_may_quietly(&mut self, client: u32, app: &str) -> bool {
        let Some(agent) = self.bus_agent_name(client) else { return false };
        self.agents.may(&agent, client, app) == May::Yes
    }

    /// One line in agents.log: who, what app, what, how it went.
    pub fn agent_log(&mut self, client: u32, app: &str, what: &str) {
        if cfg!(test) {
            return;
        }
        let agent = self.bus_agent_name(client).unwrap_or_default();
        let Some(dir) = state_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).mode(0o600).open(dir.join("agents.log")) {
            let _ = writeln!(f, "{}\t{}\t{}\t{}", utc(now_secs()), clean(&agent), clean(app), clean(what));
        }
    }

    /// An agent's connection ended: its "Just now" grants go, and its waiting requests.
    pub fn agents_forget(&mut self, client: u32) {
        self.agents.grants.retain(|_, s| *s != Span::Connection(client));
        self.agents.pending.retain(|(c, _, _)| *c != client);
        let before = self.agents.asking.len();
        let was_first = self.agents.asking.front().is_some_and(|q| q.client == client);
        self.agents.asking.retain(|q| q.client != client);
        if self.agents.asking.len() != before {
            if was_first {
                self.agents.focus = DEFAULT;
            }
            self.redraw_all();
        }
    }

    fn agent_dialog_open(&mut self) {
        self.close_menu();
        self.app_menu_close();
        self.launcher_close();
        self.card_close();
        self.drawer_close();
        self.agents.focus = DEFAULT;
        if let Some(q) = self.agents.asking.front() {
            tracing::info!("agents: asking whether {:?} may use {:?}", q.agent, q.app);
        }
        self.redraw_all();
    }

    /// The person answered the question showing: the grant is kept, the
    /// requests waiting on it run (or are refused), the next question shows.
    fn agent_answer(&mut self, i: usize) {
        let Some(q) = self.agents.asking.pop_front() else { return };
        let button = BUTTONS.get(i).map_or(Button::DontAllow, |b| b.0);
        let span = match button {
            Button::DontAllow => None,
            Button::JustNow => Some(Span::Connection(q.client)),
            Button::Session => Some(Span::Session),
            Button::Always => Some(Span::Always),
            Button::Hour => Some(Span::Until(now_secs() + TIMED_MINUTES * 60)),
        };
        tracing::info!("agents: {:?} and {:?}: {:?}", q.agent, q.app, span);
        if let Some(s) = span {
            self.agents.grants.insert((q.agent.clone(), q.app.clone()), s);
            if matches!(s, Span::Always | Span::Until(_)) {
                self.agents.save();
            }
        }
        self.agent_log(q.client, &q.app, &format!("asked: {}", BUTTONS.get(i).map_or("Don't allow", |b| b.1)));
        // The requests about this app from this agent, in order.
        let mut mine = Vec::new();
        let mut rest = VecDeque::new();
        for (c, app, r) in std::mem::take(&mut self.agents.pending) {
            if app == q.app && self.bus_agent_name(c).as_deref() == Some(q.agent.as_str()) {
                mine.push((c, r));
            } else {
                rest.push_back((c, app, r));
            }
        }
        self.agents.pending = rest;
        for (c, r) in mine {
            if span.is_some() {
                self.agent_request(c, r);
            } else {
                let serial = r.serial();
                self.agent_log(c, &q.app, "refused: the person did not allow it");
                self.bus_send(c, vergence_protocol::bus::ServerMsg::Error { serial, code: vergence_protocol::bus::ErrorCode::Denied, message: format!("the person did not allow {} to use {}", q.agent, q.shown) });
            }
        }
        self.agents.focus = DEFAULT;
        self.agents.picture = None;
        if let Some(next) = self.agents.asking.front() {
            tracing::info!("agents: asking whether {:?} may use {:?}", next.agent, next.app);
        }
        self.redraw_all();
    }

    /// A key while it asks: every key is its (Escape: Don't allow; Enter:
    /// the button with the focus, For an hour at first; Tab and the arrows).
    pub fn agent_dialog_key(&mut self, sym: u32, pressed: bool) {
        use xkbcommon::xkb::keysyms as k;
        if !pressed {
            return;
        }
        let n = BUTTONS.len();
        let a = &mut self.agents;
        match sym {
            k::KEY_Escape => return self.agent_answer(0),
            k::KEY_Return | k::KEY_KP_Enter | k::KEY_space => {
                let i = a.focus;
                return self.agent_answer(i);
            }
            k::KEY_Tab | k::KEY_Right => a.focus = (a.focus + 1) % n,
            k::KEY_ISO_Left_Tab | k::KEY_Left => a.focus = (a.focus + n - 1) % n,
            _ => return,
        }
        self.redraw_all();
    }

    pub fn agent_dialog_hover(&mut self, at: Point<f64, Logical>) {
        if self.agents.asking.is_empty() {
            return;
        }
        if let Some(&(i, _)) = self.agents.buttons.iter().find(|(_, r)| r.to_f64().contains(at))
            && i != self.agents.focus
        {
            self.agents.focus = i;
            self.redraw_all();
        }
    }

    /// A press: a button answers; anywhere else does nothing (the question
    /// stays until it is answered).
    pub fn agent_dialog_press(&mut self, at: Point<f64, Logical>) {
        if let Some(&(i, _)) = self.agents.buttons.iter().find(|(_, r)| r.to_f64().contains(at)) {
            self.agent_answer(i);
        }
    }

    /// The tile while an agent works (with the dock's tick, once a second).
    pub fn agents_tick(&mut self) {
        if self.agents.tile_up && self.agents.last.as_ref().is_none_or(|(_, _, t)| t.elapsed().as_secs() >= WORKING_SECS) {
            self.agents.tile_up = false;
            self.tile_remove(crate::tiles::Owner::Desktop, "agent");
        } else if self.agents.tile_up && now_secs().is_multiple_of(30) {
            self.agent_tile();
        }
    }

    fn agent_tile(&mut self) {
        let Some((agent, app, _)) = self.agents.last.clone() else { return };
        let shown = self.app_shown_name(&app);
        let left = self.agents.minutes_left(&agent, &app).map(|m| format!(", {m} min left")).unwrap_or_default();
        let tile = vergence_protocol::tile::Tile {
            id: "agent".into(),
            value: agent.chars().take(12).collect(),
            label: format!("using {shown}"),
            state: vergence_protocol::tile::TileState::Warning,
            group: "System".into(),
            card: vergence_protocol::tile::Card { title: format!("{agent} is using {shown}"), big: String::new(), rows: vec![("Allowed".into(), format!("{}{left}", self.agent_span_word(&agent, &app)))], chart: Vec::new(), controls: Vec::new(), note: "Privacy > Agents takes it back (coming).".into() },
            page: String::new(),
        };
        self.agents.tile_up = true;
        self.tile_set(crate::tiles::Owner::Desktop, tile);
    }

    fn agent_span_word(&self, agent: &str, app: &str) -> &'static str {
        match self.agents.grants.get(&(agent.to_string(), app.to_string())) {
            Some(Span::Always) => "always",
            Some(Span::Session) => "until you log out",
            Some(Span::Connection(_)) => "just now",
            Some(Span::Until(_)) => "for a time",
            _ => "",
        }
    }

    /// An app's name as people know it: a window's title of it if one is
    /// open, else its id.
    pub fn app_shown_name(&self, app: &str) -> String {
        self.space.elements().find_map(|w| w.native().filter(|n| n.app_id == app).map(|n| n.title())).filter(|t| !t.is_empty()).unwrap_or_else(|| app.to_string())
    }

    /// In front of everything on a screen: the panel (on the pointer's
    /// screen), then the dimming.
    pub fn agent_dialog_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        let mut elements = Vec::new();
        if out.to_f64().contains(self.pointer_location())
            && let Some(e) = self.agent_dialog_panel(renderer, out, scale)
        {
            elements.push(e);
        }
        let (id, commit) = self.agents.dim.get_or_insert_with(|| (Id::new(), CommitCounter::default())).clone();
        let phys = Rectangle::<i32, smithay::utils::Physical>::from_size(out.size.to_f64().to_physical(scale).to_i32_round());
        elements.push(SceneElement::Solid(SolidColorRenderElement::new(id, phys, commit, [0.0, 0.0, 0.0, 0.45], Kind::Unspecified)));
        elements
    }

    fn agent_dialog_panel(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Option<SceneElement> {
        let q = self.agents.asking.front()?.clone();
        let light = self.light_style();
        let focus = self.agents.focus;
        let accent = self.accent();
        let at = (out.loc.x + (out.size.w - W) / 2, out.loc.y + (out.size.h - H) / 2);
        let panel: Rectangle<i32, Logical> = Rectangle::new(at.into(), (W, H).into());
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        let face = fonts.face("sans-serif", 400, false)?;
        let bold = fonts.face("sans-serif", 600, false).unwrap_or(face);
        let widths: Vec<i32> = BUTTONS
            .iter()
            .map(|(b, label)| {
                let (f, side) = if *b == Button::Hour { (bold, DEFAULT_SIDE) } else { (face, BUTTON_SIDE) };
                (fonts.layout(f, TEXT, label).1 + 2.0 * side).ceil() as i32
            })
            .collect();
        let row_y = H - PAD - BUTTON_H;
        // Don't allow on the left, the four lengths to the right.
        let mut local: Vec<(usize, i32, i32)> = vec![(0, PAD, widths[0])];
        let mut x = W - PAD;
        for i in (1..BUTTONS.len()).rev() {
            x -= widths[i];
            local.push((i, x, widths[i]));
            x -= BUTTON_GAP;
        }
        local.sort_by_key(|(i, _, _)| *i);
        self.agents.panel = panel;
        self.agents.buttons = local.iter().map(|(i, x, w)| (*i, Rectangle::new((at.0 + x, at.1 + row_y).into(), (*w, BUTTON_H).into()))).collect();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (focus, light, scale.to_bits(), accent.r, accent.g, accent.b, &q.agent, &q.shown).hash(&mut h);
        let key = h.finish();
        if self.agents.picture.as_ref().map(|(k, _)| *k) != Some(key) {
            let s = vergence_look::shelf_look(light);
            let ink = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            let (w, hh) = (W as f32, H as f32);
            let mut scene = Scene::new(w, hh);
            scene.fill_rounded(SceneRect::new(0.0, 0.0, w, hh), 14.0, ink(s.menu));
            scene.stroke(SceneRect::new(0.0, 0.0, w, hh), 14.0, vergence_look::EDGE, ink(s.edge));
            let m = fonts.metrics(bold, 18.0);
            let heading: String = format!("{} wants to use {}", q.agent, q.shown).chars().take(64).collect();
            vergence_render::text(&mut scene, fonts, bold, 18.0, ink(s.menu_ink), ScenePoint::new(PAD as f32, (PAD as f32 + m.ascent).round()), &heading);
            let m = fonts.metrics(face, TEXT);
            let (a, b) = (s.menu_ink, s.menu_ink_off);
            let soft = Color::rgb(((a.0 as u16 + b.0 as u16) / 2) as u8, ((a.1 as u16 + b.1 as u16) / 2) as u8, ((a.2 as u16 + b.2 as u16) / 2) as u8);
            let shown: String = q.shown.chars().take(40).collect();
            let lines = [format!("It can read {shown}'s windows and press, type and choose in them."), "You'll see a ring where it acts. A secret field stays secret.".to_string()];
            for (k, line) in lines.iter().enumerate() {
                vergence_render::text(&mut scene, fonts, face, TEXT, soft, ScenePoint::new(PAD as f32, (PAD as f32 + 30.0 + 19.0 * k as f32 + m.ascent).round()), line);
            }
            let plain = Color::rgba(s.menu_ink.0, s.menu_ink.1, s.menu_ink.2, 20);
            for (i, x, bw) in &local {
                let r = SceneRect::new(*x as f32, row_y as f32, *bw as f32, BUTTON_H as f32);
                let default = BUTTONS[*i].0 == Button::Hour;
                let (ground, text, f) = if default { (accent, Color::rgb(0xff, 0xff, 0xff), bold) } else { (plain, ink(s.menu_ink), face) };
                scene.fill_rounded(r, 8.0, ground);
                if *i == focus {
                    scene.stroke(SceneRect::new(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0), 10.0, 2.0, ink(s.menu_ink));
                }
                let label = BUTTONS[*i].1;
                let lw = fonts.layout(f, TEXT, label).1;
                let lm = fonts.metrics(f, TEXT);
                vergence_render::text(&mut scene, fonts, f, TEXT, text, ScenePoint::new((r.x + (r.w - lw) / 2.0).round(), (r.y + (r.h - lm.ascent - lm.descent) / 2.0 + lm.ascent).round()), label);
            }
            let p = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)).ok()?;
            let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
            self.agents.picture = Some((key, buffer));
        }
        let (_, buffer) = self.agents.picture.as_ref()?;
        let a = (panel.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let b = (panel.loc + panel.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(panel.size), Kind::Unspecified).ok().map(SceneElement::Memory)
    }

    /// For `vctl agents`: the grants, the questions, the waiting requests.
    pub fn agents_report(&mut self) -> String {
        self.agents.load();
        let mut out: Vec<String> = self
            .agents
            .grants
            .iter()
            .map(|((a, p), s)| match s {
                Span::Until(t) => format!("{a:?} {p:?}: for {} more min", t.saturating_sub(now_secs()).div_ceil(60)),
                s => format!("{a:?} {p:?}: {s:?}"),
            })
            .collect();
        out.sort();
        if let Some(q) = self.agents.asking.front() {
            out.push(format!("asking: {:?} wants to use {:?} ({} questions, {} requests waiting)", q.agent, q.shown, self.agents.asking.len(), self.agents.pending.len()));
        }
        if out.is_empty() {
            "no grants".into()
        } else {
            out.join("\n")
        }
    }

    /// For `vctl agent-answer <button>`: answers the question showing, as
    /// a press of that button would (tests in headless sessions).
    pub fn agent_answer_by_word(&mut self, word: &str) -> String {
        if self.agents.asking.is_empty() {
            return "nothing is being asked".into();
        }
        let i = match word {
            "deny" | "dont" | "no" => 0,
            "now" | "just-now" => 1,
            "session" => 2,
            "always" => 3,
            "hour" | "timed" => 4,
            _ => return "agent-answer deny|now|session|always|hour".into(),
        };
        self.agent_answer(i);
        format!("answered {:?}", BUTTONS[i].1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_are_kept_and_read_back() {
        let mut g = HashMap::new();
        g.insert(("Claude".to_string(), "vergence-calc".to_string()), Span::Always);
        g.insert(("Claude".to_string(), "vergence-term".to_string()), Span::Never);
        g.insert(("Other\tagent".to_string(), "x".to_string()), Span::Until(1_800_000_000));
        g.insert(("Claude".to_string(), "vergence-edit".to_string()), Span::Session);
        g.insert(("Claude".to_string(), "vergence-files".to_string()), Span::Connection(7));
        let text = render(&g);
        let back = parse(&text);
        assert_eq!(back.len(), 3, "only always, never and timed ones are kept: {text}");
        assert_eq!(back.get(&("Claude".into(), "vergence-calc".into())), Some(&Span::Always));
        assert_eq!(back.get(&("Claude".into(), "vergence-term".into())), Some(&Span::Never));
        assert_eq!(back.get(&("Other agent".into(), "x".into())), Some(&Span::Until(1_800_000_000)), "a tab in a name cannot split its line");
        assert!(parse("junk\nClaude\tapp\tsometimes\nClaude\tapp\tuntil soon\n").is_empty());
    }

    #[test]
    fn what_a_grant_says() {
        let mut a = Agents { loaded: true, ..Default::default() };
        assert_eq!(a.may("Claude", 3, "calc"), May::Ask);
        a.grants.insert(("Claude".into(), "calc".into()), Span::Connection(3));
        assert_eq!(a.may("Claude", 3, "calc"), May::Yes);
        assert_eq!(a.may("Claude", 4, "calc"), May::Ask, "another connection is asked again");
        a.grants.insert(("Claude".into(), "calc".into()), Span::Until(now_secs() + 600));
        assert_eq!(a.may("Claude", 4, "calc"), May::Yes);
        assert_eq!(a.minutes_left("Claude", "calc"), Some(10));
        a.grants.insert(("Claude".into(), "calc".into()), Span::Until(now_secs().saturating_sub(1)));
        assert_eq!(a.may("Claude", 4, "calc"), May::Ask, "an hour that has passed asks again");
        assert!(!a.grants.contains_key(&("Claude".into(), "calc".into())));
        a.grants.insert(("Claude".into(), "calc".into()), Span::Never);
        assert_eq!(a.may("Claude", 4, "calc"), May::No);
        assert_eq!(a.may("Other", 4, "calc"), May::Ask, "per agent");
        assert_eq!(a.may("Claude", 4, "term"), May::Ask, "per app");
    }

    #[test]
    fn times_in_the_log() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_763_668_800), "2025-11-20T20:00:00Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
    }
}
