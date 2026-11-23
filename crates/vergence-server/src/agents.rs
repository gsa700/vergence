// Agents' permission (meaning milestone 1, step 4, part 1; whitepaper §7.2):
// an agent's first request about an app asks the person, in a dialog the
// server draws (no app can draw it), how long it may use that app: Just now
// (until it disconnects), For an hour, Until I log out (this session),
// Always, or Don't allow (asked again after a minute). Grants are per agent
// (the name it gave and the program the server saw run it: Fable's review,
// a name alone is any program's word) and per app; Always and Never and the
// timed ones are kept (0600) in
// ~/.local/state/vergence/agents.tsv, the others in memory. A timed grant is
// checked at each request, not by a timer. While an agent works a tile says
// so ("Claude is using Calculator", and the time a timed grant has left);
// every request it makes of an app is logged in agents.log (0600).
//
// The dialog looks as the log out dialog does (session_dialog.rs): every
// screen dimmed, a panel on the pointer's screen, one row of buttons. Enter
// and Escape mean Don't allow (no keystroke meant for something else may
// grant; see docs/meaning-milestone-1.md); nothing counts for a moment after
// a question appears; Space
// answers nothing. The question names the app by its entry's name and id
// and the agent by its name (cleaned, quoted) and program, in rows its
// words cannot push out. Requests wait while it asks, and run when it answers.
//
// Shell access (terminal agent milestone 1, step 1): a window whose tree's
// root says "shell-access" (Terminal) is granted a tab at a time, never the
// whole app: a node's scope is its nearest ancestor saying "scope" (a tab),
// and a grant covers that scope at a level, Read (its tree, focus, changes,
// pages) or Act (typing; it includes Read). The dialog names the tab and the
// level and offers only Don't allow, Just now and the timed length: no
// "Until I log out", no "Always" (Dave: "never"), and an Always kept for the
// whole app from before is not honored there. These grants live in memory
// only (a tab's id is its process's).

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

/// How long a timed grant lasts unless Privacy > Agents says otherwise.
pub const TIMED_MINUTES: u64 = 60;

/// desktop.toml's `[agents]`: whether agents may ask at all (Privacy > Agents'
/// switch, off by default since defaults milestone 1: a person who never uses
/// agents never sees a prompt) and how long a timed grant lasts (15 minutes to
/// a day).
pub fn agents_settings() -> (bool, u64) {
    // Read at most once a second (Fable's review: not a file read per request).
    static CACHE: std::sync::Mutex<Option<(Instant, (bool, u64))>> = std::sync::Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((t, v)) = *cache
        && t.elapsed() < std::time::Duration::from_secs(1)
        && !cfg!(test)
    {
        return v;
    }
    let v = read_agents_settings();
    *cache = Some((Instant::now(), v));
    v
}

fn read_agents_settings() -> (bool, u64) {
    let text = vergence_config::effective_text_with(crate::wallpaper::desktop_path().as_deref());
    let table = text.parse::<toml::Table>().ok();
    let t = table.as_ref().and_then(|t| t.get("agents")).and_then(|a| a.as_table());
    let enabled = t.and_then(|t| t.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(false);
    let minutes = t
        .and_then(|t| t.get("timed-minutes"))
        .and_then(|v| v.as_integer())
        .map_or(TIMED_MINUTES, |m| m.clamp(15, 24 * 60) as u64);
    (enabled, minutes)
}
/// Who an agent is: the name it gave (cleaned) and the program the server
/// saw run it (its executable, from the bus socket's peer). Under one user
/// this is attribution, not proof (whitepaper §7.4): a program can run the
/// same executable; but a name alone no longer carries anyone's grants.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Who {
    pub name: String,
    pub program: String,
}

/// An agent's name as shown: printable, no format or direction marks, at
/// most 24 characters.
pub fn clean_agent_name(name: &str) -> String {
    let format = |c: char| matches!(c as u32, 0x00AD | 0x061C | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB);
    let out: String = name.chars().filter(|c| !c.is_control() && !format(*c)).take(24).collect();
    out.trim().to_string()
}

/// A grant's app key is "`<app id> @ <program>`" (meaning.rs agent_app_key); its id.
pub fn app_id_of(key: &str) -> &str {
    key.split(" @ ").next().unwrap_or(key)
}

/// A grant's app key's program.
pub fn app_program_of(key: &str) -> &str {
    key.split_once(" @ ").map_or("", |(_, p)| p)
}

/// agents.log past 4 MB becomes agents.log.1 (one kept).
fn rotate_log(dir: &std::path::Path) {
    let log = dir.join("agents.log");
    if std::fs::metadata(&log).is_ok_and(|m| m.len() > 4 << 20) {
        let _ = std::fs::rename(&log, dir.join("agents.log.1"));
    }
}

/// How long nothing counts after a question appears.
const ARMING: std::time::Duration = std::time::Duration::from_millis(1000);
/// After Don't allow, the same agent's asks about the same app are refused
/// without a question this long.
const COOL_DOWN_SECS: u64 = 60;
/// The most questions one connection may have waiting, and requests.
const ASKS_PER_CLIENT: usize = 2;
const PENDING_PER_CLIENT: usize = 16;

/// An agent counts as working this long after its last request (the tile).
const WORKING_SECS: u64 = 120;
/// A timed grant's last minutes: the tile warns, and offers more.
const ENDING_MINUTES: u64 = 5;
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
    /// A timed grant ran out: asked again, saying so.
    Expired,
}

/// How much a scoped grant allows: reading a tab, or typing in it too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    Read,
    Act,
}

/// What a request on a shell-access window is about: the tab (its node id
/// and title, for the dialog) and the level it needs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Scope {
    pub id: u64,
    pub tab: String,
    pub level: Level,
}

/// The scope a request for a terminal of the agent's own is asked under
/// (bus v38): no tab exists yet; its grant moves to the new terminal's tab
/// when that window says what it means.
pub const NEW_TERMINAL: u64 = u64::MAX;

/// How long a terminal asked for may take to say what it means.
pub const TERMINAL_WAIT_PUB: std::time::Duration = std::time::Duration::from_secs(20);

/// A terminal started for an agent, waiting for its window's first tree: who
/// asked (its bus connection, the request's serial, who it is and the name it
/// was given), the program started (its pid and path) and the grant to give.
#[derive(Debug, Clone)]
pub struct WaitingTerminal {
    pub client: u32,
    pub serial: u32,
    pub agent: Who,
    pub name: String,
    pub pid: u32,
    pub app: String,
    pub span: Span,
    pub since: Instant,
}

/// The key a question, a refusal and a waiting request are kept under: the
/// app's for an ordinary window; with the tab and the level for a scoped one.
pub fn gate_key(app: &str, scope: Option<&Scope>) -> String {
    match scope {
        None => app.to_string(),
        Some(s) => format!("{app}\u{0}tab {} {:?}", s.id, s.level),
    }
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

const BUTTONS: [(Button, &str); 5] = [
    (Button::DontAllow, "Don't allow"),
    (Button::JustNow, "Just now"),
    (Button::Session, "Until I log out"),
    (Button::Always, "Always"),
    (Button::Hour, "For an hour"),
];

/// A shell-access tab's buttons: never "Until I log out" or "Always".
const SHELL_BUTTONS: [(Button, &str); 3] =
    [(Button::DontAllow, "Don't allow"), (Button::JustNow, "Just now"), (Button::Hour, "For an hour")];

/// The timed button's words for a length: "For an hour", "For 30 minutes",
/// "For 4 hours", "For a day".
fn timed_label(minutes: u64) -> String {
    match minutes {
        60 => "For an hour".into(),
        1440 => "For a day".into(),
        m if m % 60 == 0 => format!("For {} hours", m / 60),
        m => format!("For {m} minutes"),
    }
}
/// The keyboard's default: Don't allow, so no keystroke meant for something
/// else can grant. The timed answer is still drawn as the suggested one
/// (see docs/meaning-milestone-1.md).
const DEFAULT: usize = 0;
const W: i32 = 660;
const H: i32 = 214;
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
    agent: Who,
    client: u32,
    app: String,
    shown: String,
    /// Its timed grant ran out (the question says so).
    expired: bool,
    /// A shell-access window's tab and level (None: the whole app, as ever).
    scope: Option<Scope>,
}

impl Question {
    /// The buttons this question offers.
    fn buttons(&self) -> &'static [(Button, &'static str)] {
        if self.scope.is_some() { &SHELL_BUTTONS } else { &BUTTONS }
    }

    /// What its waiting requests and refusal are kept under.
    fn key(&self) -> String {
        gate_key(&self.app, self.scope.as_ref())
    }
}

#[derive(Default)]
pub struct Agents {
    /// By (agent, app id).
    grants: HashMap<(Who, String), Span>,
    /// Shell access, by (agent, app, tab node id): how long, and the level.
    /// In memory only.
    scoped: HashMap<(Who, String, u64), (Span, Level)>,
    /// Tabs the person stopped an agent in (Escape twice, the mark's click):
    /// refused, saying so, for the cool-down.
    stopped: HashMap<(Who, String, u64), Instant>,
    /// Terminals started for agents, waiting for their windows (bus v38).
    pub terminals: Vec<WaitingTerminal>,
    /// When the person last said Don't allow to an agent about an app.
    refused: HashMap<(Who, String), Instant>,
    /// When the question showing appeared (nothing counts for ARMING).
    armed: Option<Instant>,
    loaded: bool,
    /// Requests waiting for an answer: the agent's bus client, the app, the request.
    pending: VecDeque<(u32, String, AgentRequest)>,
    /// The questions, the first one showing.
    asking: VecDeque<Question>,
    /// Who worked last, with what app, and when (the tile).
    last: Option<(Who, String, Instant)>,
    /// The grant the tile shows now (its card's "Allow ... more").
    tiled: Option<(Who, String)>,
    tile_up: bool,
    focus: usize,
    panel: Rectangle<i32, Logical>,
    buttons: Vec<(usize, Rectangle<i32, Logical>)>,
    dim: Option<(Id, CommitCounter)>,
    picture: Option<(u64, MemoryRenderBuffer)>,
    /// The rings where agents acted (step 5), and whether their timer runs.
    rings: Vec<Ring>,
    ring_timer: bool,
    /// The minutes left each running-out grant was last told at.
    told: HashMap<(Who, String), u64>,
}

/// A ring where an agent acted: on the screens, its color, when, its picture.
struct Ring {
    at: (f64, f64, f64, f64),
    color: (u8, u8, u8),
    since: Instant,
    picture: Option<(u64, MemoryRenderBuffer)>,
}

/// How long a ring stays, then fades; how often it is drawn while it does.
const RING_HOLD: std::time::Duration = std::time::Duration::from_millis(600);
const RING_FADE: std::time::Duration = std::time::Duration::from_millis(600);
const RING_STEP: std::time::Duration = std::time::Duration::from_millis(16);
/// Room around what it rings, in points; the most rings at once.
const RING_PAD: f64 = 4.0;
const RINGS_MAX: usize = 32;

/// An agent's own color, the same each time for the same name: one of a
/// few that read on light and dark alike.
fn ring_color(agent: &str) -> (u8, u8, u8) {
    const COLORS: [(u8, u8, u8); 6] = [
        (0x1f, 0xa9, 0x9a),
        (0xf0, 0x86, 0x22),
        (0x8e, 0x5c, 0xf0),
        (0xe0, 0x45, 0x86),
        (0x3f, 0xa3, 0x44),
        (0x2f, 0x7f, 0xf0),
    ];
    let h = agent.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3));
    COLORS[(h % COLORS.len() as u64) as usize]
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn state_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))?;
    Some(base.join("vergence"))
}

/// A name as kept in the file: no tabs or line ends.
fn clean(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// The file's lines: agent's name, its program, app, and always, never or
/// until `<seconds>`. (Lines of three fields, from before programs were
/// kept, are dropped: those agents are asked again.)
fn parse(text: &str) -> HashMap<(Who, String), Span> {
    let mut out = HashMap::new();
    for line in text.lines().take(10_000) {
        let mut f = line.split('\t');
        let (Some(agent), Some(program), Some(app), Some(v)) = (f.next(), f.next(), f.next(), f.next()) else {
            continue;
        };
        let span = match v.split_once(' ') {
            None if v == "always" => Span::Always,
            None if v == "never" => Span::Never,
            Some(("until", t)) => match t.parse() {
                Ok(t) => Span::Until(t),
                Err(_) => continue,
            },
            _ => continue,
        };
        out.insert((Who { name: agent.to_string(), program: program.to_string() }, app.to_string()), span);
    }
    out
}

fn render(grants: &HashMap<(Who, String), Span>) -> String {
    let mut lines: Vec<String> = grants
        .iter()
        .filter_map(|((a, p), s)| {
            let v = match s {
                Span::Always => "always".to_string(),
                Span::Never => "never".to_string(),
                Span::Until(t) => format!("until {t}"),
                _ => return None,
            };
            Some(format!("{}\t{}\t{}\t{v}", clean(&a.name), clean(&a.program), clean(p)))
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
        let _ = std::fs::remove_file(&tmp);
        let ok = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp).and_then(|mut f| {
            f.write_all(render(&self.grants).as_bytes())?;
            f.sync_all()
        });
        if ok.is_ok() {
            let _ = std::fs::rename(&tmp, dir.join("agents.tsv"));
        }
    }

    /// What agent `agent` (bus client `client`) may do with `app` now.
    fn may(&mut self, agent: &Who, client: u32, app: &str) -> May {
        self.load();
        let key = (agent.clone(), app.to_string());
        match self.grants.get(&key).copied() {
            None if self.refused.get(&key).is_some_and(|t| t.elapsed().as_secs() < COOL_DOWN_SECS) => May::No,
            None => May::Ask,
            Some(Span::Never) => May::No,
            Some(Span::Always | Span::Session) => May::Yes,
            Some(Span::Connection(c)) if c == client => May::Yes,
            Some(Span::Connection(_)) => May::Ask,
            Some(Span::Until(t)) if now_secs() < t => May::Yes,
            Some(Span::Until(_)) => {
                self.grants.remove(&key);
                self.save();
                May::Expired
            }
        }
    }

    /// What agent `agent` may do in tab `scope.id` of shell-access `app` at
    /// `scope.level` now. A Read grant asks again for Act; an Act grant
    /// covers Read. An app-wide grant, Always included, counts for nothing.
    fn may_scoped(&mut self, agent: &Who, client: u32, app: &str, scope: &Scope) -> May {
        let key = (agent.clone(), app.to_string(), scope.id);
        let refused = (agent.clone(), gate_key(app, Some(scope)));
        match self.scoped.get(&key).copied() {
            None if self.refused.get(&refused).is_some_and(|t| t.elapsed().as_secs() < COOL_DOWN_SECS) => May::No,
            None => May::Ask,
            Some((_, level)) if level < scope.level => {
                if self.refused.get(&refused).is_some_and(|t| t.elapsed().as_secs() < COOL_DOWN_SECS) {
                    May::No
                } else {
                    May::Ask
                }
            }
            Some((Span::Connection(c), _)) if c == client => May::Yes,
            Some((Span::Until(t), _)) if now_secs() < t => May::Yes,
            Some((Span::Until(_), _)) => {
                self.scoped.remove(&key);
                May::Expired
            }
            Some(_) => May::Ask,
        }
    }

    /// The tabs of shell-access `app` an agent holds a live grant for, at
    /// `level` or more.
    pub fn scopes_held(&self, agent: &Who, client: u32, app: &str, level: Level) -> Vec<u64> {
        self.scoped
            .iter()
            .filter(|((a, p, _), (span, l))| {
                a == agent
                    && p == app
                    && *l >= level
                    && match span {
                        Span::Connection(c) => *c == client,
                        Span::Until(t) => now_secs() < *t,
                        _ => false,
                    }
            })
            .map(|((_, _, id), _)| *id)
            .collect()
    }

    /// Every agent's grant for a tab ends now (the stop keys, step 4): each
    /// is refused there for the cool-down, told the person stopped it, rather
    /// than asked again at once.
    pub fn revoke_scope(&mut self, app: &str, scope: u64) -> usize {
        let gone: Vec<Who> = self
            .scoped
            .keys()
            .filter(|(_, p, id)| p == app && *id == scope)
            .map(|(a, _, _)| a.clone())
            .collect();
        self.scoped.retain(|(_, p, id), _| !(p == app && *id == scope));
        let now = Instant::now();
        for a in &gone {
            for level in [Level::Read, Level::Act] {
                let sc = Scope { id: scope, tab: String::new(), level };
                self.refused.insert((a.clone(), gate_key(app, Some(&sc))), now);
            }
            self.stopped.insert((a.clone(), app.to_string(), scope), now);
        }
        gone.len()
    }

    /// Whether the person stopped this agent in this tab, lately.
    pub fn stopped_lately(&self, agent: &Who, app: &str, scope: u64) -> bool {
        self.stopped
            .get(&(agent.clone(), app.to_string(), scope))
            .is_some_and(|t| t.elapsed().as_secs() < COOL_DOWN_SECS)
    }

    /// The grant given for a terminal of the agent's own, taken to move to
    /// its tab once it exists.
    pub fn take_new_terminal(&mut self, agent: &Who, app: &str) -> Option<Span> {
        self.scoped.remove(&(agent.clone(), app.to_string(), NEW_TERMINAL)).map(|(s, _)| s)
    }

    /// A tab's grant at the typing level (the agent's own terminal).
    pub fn grant_tab(&mut self, agent: &Who, app: &str, tab: u64, span: Span) {
        self.scoped.insert((agent.clone(), app.to_string(), tab), (span, Level::Act));
    }

    /// The minutes a timed grant has left, if the agent has one for the app.
    fn minutes_left(&self, agent: &Who, app: &str) -> Option<u64> {
        match self.grants.get(&(agent.clone(), app.to_string())) {
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
    /// `scope`: a shell-access window's tab and level (granted on its own).
    pub fn agent_gate(
        &mut self,
        client: u32,
        app: &str,
        shown: &str,
        request: &AgentRequest,
        scope: Option<Scope>,
    ) -> Option<bool> {
        let agent = self.bus_agent_who(client)?;
        let key = gate_key(app, scope.as_ref());
        let may = match &scope {
            Some(sc) => self.agents.may_scoped(&agent, client, app, sc),
            None => self.agents.may(&agent, client, app),
        };
        match may {
            May::Yes => {
                self.agents.last = Some((agent, app.to_string(), Instant::now()));
                self.agent_tile();
                Some(true)
            }
            May::No => Some(false),
            may @ (May::Ask | May::Expired) => {
                // Bounded per connection, so one cannot crowd out the rest.
                let mine = self.agents.pending.iter().filter(|(c, _, _)| *c == client).count();
                if self.agents.pending.len() >= PENDING_MAX || mine >= PENDING_PER_CLIENT {
                    return Some(false);
                }
                let asked = self.agents.asking.iter().any(|a| a.agent == agent && a.key() == key);
                if !asked && self.agents.asking.iter().filter(|a| a.client == client).count() >= ASKS_PER_CLIENT {
                    return Some(false);
                }
                self.agents.pending.push_back((client, key.clone(), request.clone()));
                if !asked {
                    let q = Question {
                        agent,
                        client,
                        app: app.to_string(),
                        shown: shown.to_string(),
                        expired: may == May::Expired,
                        scope,
                    };
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
        let Some(agent) = self.bus_agent_who(client) else { return false };
        self.agents.may(&agent, client, app) == May::Yes
    }

    /// The tabs of shell-access `app` the agent on `client` may read now.
    pub fn agent_scopes(&mut self, client: u32, app: &str, level: Level) -> Vec<u64> {
        let Some(agent) = self.bus_agent_who(client) else { return Vec::new() };
        self.agents.scopes_held(&agent, client, app, level)
    }

    /// Every agent's grant for tab `scope` of the shell-access window
    /// (`app_client`, `window`) ends now: the stop keys (step 4). How many.
    pub fn agent_revoke_tab(&mut self, app_client: u32, window: u32, scope: u64) -> usize {
        let Some(app) = self
            .natives
            .get(&(app_client, window))
            .and_then(|w| w.native())
            .map(|n| self.agent_app_key(n.client, &n.app_id))
        else {
            return 0;
        };
        let n = self.agents.revoke_scope(&app, scope);
        if n > 0 {
            tracing::info!("agents: {n} grant(s) for tab {scope} of {app:?} revoked");
        }
        n
    }

    /// One line in agents.log: who, what app, what, how it went.
    pub fn agent_log(&mut self, client: u32, app: &str, what: &str) {
        if cfg!(test) {
            return;
        }
        let agent = self.bus_agent_name(client).unwrap_or_default();
        let Some(dir) = state_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        rotate_log(&dir);
        if let Ok(mut f) =
            std::fs::OpenOptions::new().append(true).create(true).mode(0o600).open(dir.join("agents.log"))
        {
            let what: String = clean(what).chars().take(200).collect();
            let _ = writeln!(f, "{}\t{}\t{}\t{what}", utc(now_secs()), clean(&agent), clean(app));
        }
    }

    /// An agent's connection ended: its "Just now" grants go, and its waiting requests.
    pub fn agents_forget(&mut self, client: u32) {
        self.agents.grants.retain(|_, s| *s != Span::Connection(client));
        self.agents.scoped.retain(|_, (s, _)| *s != Span::Connection(client));
        self.agents.pending.retain(|(c, _, _)| *c != client);
        let before = self.agents.asking.len();
        let was_first = self.agents.asking.front().is_some_and(|q| q.client == client);
        self.agents.asking.retain(|q| q.client != client);
        if self.agents.asking.len() != before {
            if was_first {
                self.agents.focus = DEFAULT;
                self.agents.armed = Some(Instant::now());
                self.agents.picture = None;
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
        self.agents.armed = Some(Instant::now());
        if let Some(q) = self.agents.asking.front() {
            tracing::info!("agents: asking whether {:?} may use {:?}", q.agent, q.app);
        }
        self.redraw_all();
    }

    /// Whether the question showing has been up long enough for an answer
    /// to count (a key or press already on its way when it appeared is not
    /// an answer).
    fn agent_dialog_armed(&self) -> bool {
        self.agents.armed.is_some_and(|t| t.elapsed() >= ARMING)
    }

    /// The person answered the question showing: the grant is kept, the
    /// requests waiting on it run (or are refused), the next question shows.
    fn agent_answer(&mut self, i: usize) {
        let Some(q) = self.agents.asking.pop_front() else { return };
        let buttons = q.buttons();
        let button = buttons.get(i).map_or(Button::DontAllow, |b| b.0);
        let span = match button {
            Button::DontAllow => None,
            Button::JustNow => Some(Span::Connection(q.client)),
            Button::Session => Some(Span::Session),
            Button::Always => Some(Span::Always),
            Button::Hour => Some(Span::Until(now_secs() + agents_settings().1 * 60)),
        };
        tracing::info!("agents: {:?} and {:?}: {:?}", q.agent, q.app, span);
        if span.is_none() {
            self.agents.refused.insert((q.agent.clone(), q.key()), Instant::now());
        }
        match (span, &q.scope) {
            // A tab's grant: in memory, at its level (never Always or the session).
            (Some(s @ (Span::Connection(_) | Span::Until(_))), Some(sc)) => {
                self.agents.scoped.insert((q.agent.clone(), q.app.clone(), sc.id), (s, sc.level));
            }
            (Some(_), Some(_)) => {}
            (Some(s), None) => {
                self.agents.grants.insert((q.agent.clone(), q.app.clone()), s);
                if matches!(s, Span::Always | Span::Until(_)) {
                    self.agents.save();
                }
            }
            (None, _) => {}
        }
        let scope_words = q.scope.as_ref().map_or(String::new(), |sc| {
            format!(" for the tab {:?} ({:?}, #{})", sc.tab.chars().take(48).collect::<String>(), sc.level, sc.id)
        });
        self.agent_log_named(
            &q.agent.name,
            &q.app,
            &format!("asked ({}){scope_words}: {}", q.agent.program, buttons.get(i).map_or("Don't allow", |b| b.1)),
        );
        // The requests about this app (this tab, at this level or below) from
        // this agent, in order: a typing grant lets its reads run too.
        let key = q.key();
        let read_key = q.scope.as_ref().map(|sc| gate_key(&q.app, Some(&Scope { level: Level::Read, ..sc.clone() })));
        let mut mine = Vec::new();
        let mut rest = VecDeque::new();
        for (c, app, r) in std::mem::take(&mut self.agents.pending) {
            let covered = app == key || (span.is_some() && read_key.as_deref() == Some(app.as_str()));
            if covered && self.bus_agent_who(c).as_ref() == Some(&q.agent) {
                mine.push((c, r));
            } else {
                rest.push_back((c, app, r));
            }
        }
        self.agents.pending = rest;
        for (c, r) in mine {
            if span.is_some() && crate::meaning::agents_allowed() {
                self.agent_request(c, r);
            } else {
                let serial = r.serial();
                self.agent_log(c, &q.app, "refused: the person did not allow it");
                self.bus_send(
                    c,
                    vergence_protocol::bus::ServerMsg::Error {
                        serial,
                        code: vergence_protocol::bus::ErrorCode::Denied,
                        message: format!("the person did not allow {} to use {}", q.agent.name, q.shown),
                    },
                );
            }
        }
        self.agents.focus = DEFAULT;
        self.agents.picture = None;
        self.agents.armed = Some(Instant::now());
        if let Some(next) = self.agents.asking.front() {
            tracing::info!("agents: asking whether {:?} may use {:?}", next.agent, next.app);
        }
        self.redraw_all();
    }

    /// A key while it asks: every key is its, none counting for a moment
    /// after a question appears (Escape: Don't allow; Enter: the button with
    /// the focus, Don't allow at first; Tab and the arrows; Space nothing).
    pub fn agent_dialog_key(&mut self, sym: u32, pressed: bool) {
        use xkbcommon::xkb::keysyms as k;
        if !pressed || !self.agent_dialog_armed() {
            return;
        }
        let n = self.agents.asking.front().map_or(BUTTONS.len(), |q| q.buttons().len());
        let a = &mut self.agents;
        match sym {
            k::KEY_Escape => return self.agent_answer(0),
            k::KEY_Return | k::KEY_KP_Enter => {
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
        if !self.agent_dialog_armed() {
            return;
        }
        if let Some(&(i, _)) = self.agents.buttons.iter().find(|(_, r)| r.to_f64().contains(at)) {
            self.agent_answer(i);
        }
    }

    /// What the agent on bus client `client` may do with `app`: its span's
    /// word ("" if not asked yet) and the minutes a timed one has left.
    pub fn agent_grant_for(&mut self, client: u32, app: &str) -> (String, u32) {
        let Some(agent) = self.bus_agent_who(client) else { return (String::new(), 0) };
        self.agents.load();
        let word = match self.agents.grants.get(&(agent.clone(), app.to_string())) {
            Some(Span::Connection(c)) if *c == client => "just now",
            Some(Span::Until(t)) if *t > now_secs() => "timed",
            Some(Span::Session) => "session",
            Some(Span::Always) => "always",
            Some(Span::Never) => "never",
            _ => "",
        };
        (word.into(), self.agents.minutes_left(&agent, app).filter(|_| word == "timed").unwrap_or(0) as u32)
    }

    /// Timed grants running out: each agent hears of its own, each minute
    /// of the last five, and once at the end (step 6; the change log).
    fn agents_time_told(&mut self) {
        let now = now_secs();
        let mut tell = Vec::new();
        for ((agent, app), span) in &self.agents.grants {
            let Span::Until(t) = span else { continue };
            let left = t.saturating_sub(now).div_ceil(60);
            if left > ENDING_MINUTES {
                continue;
            }
            let key = (agent.clone(), app.clone());
            if self.agents.told.get(&key) != Some(&left) {
                tell.push((key, left));
            }
        }
        for ((agent, app), left) in tell {
            self.agents.told.insert((agent.clone(), app.clone()), left);
            self.meaning_log.push(crate::meaning::Change::Time { agent, app, minutes_left: left as u32 });
        }
        let grants = &self.agents.grants;
        self.agents.told.retain(|k, _| matches!(grants.get(k), Some(Span::Until(_))));
    }

    /// The tile while an agent works (with the dock's tick, once a second).
    pub fn agents_tick(&mut self) {
        self.agents_time_told();
        if self.agents.tile_up
            && self.agents.last.as_ref().is_none_or(|(_, _, t)| t.elapsed().as_secs() >= WORKING_SECS)
        {
            self.agents.tile_up = false;
            self.tile_remove(crate::tiles::Owner::Desktop, "agent");
        } else if self.agents.tile_up {
            let ending = self
                .agents
                .last
                .as_ref()
                .and_then(|(a, p, _)| self.agents.minutes_left(a, p))
                .is_some_and(|m| m <= ENDING_MINUTES + 1);
            if ending || now_secs().is_multiple_of(30) {
                self.agent_tile();
            }
        }
    }

    fn agent_tile(&mut self) {
        let Some((agent, app, _)) = self.agents.last.clone() else { return };
        let shown = self.app_shown_name(&app);
        let minutes = self.agents.minutes_left(&agent, &app);
        let left = minutes.map(|m| format!(", {m} min left")).unwrap_or_default();
        // Its last minutes: the tile in trouble, and one more span at a press.
        let ending = minutes.is_some_and(|m| m <= ENDING_MINUTES);
        let (state, label) = match minutes {
            Some(m) if ending => (vergence_protocol::tile::TileState::Problem, format!("{m} min left")),
            _ => (vergence_protocol::tile::TileState::Warning, format!("using {shown}")),
        };
        let controls = if minutes.is_some() {
            let more = timed_label(agents_settings().1).replacen("For ", "", 1);
            vec![vergence_protocol::tile::Control {
                id: "extend".into(),
                label: format!("Allow {more} more"),
                kind: vergence_protocol::tile::ControlKind::Button,
            }]
        } else {
            Vec::new()
        };
        let name = &agent.name;
        let title = if ending {
            format!("{name}'s time with {shown} is nearly up")
        } else {
            format!("{name} is using {shown}")
        };
        let tile = vergence_protocol::tile::Tile {
            id: "agent".into(),
            value: agent.name.chars().take(12).collect(),
            label,
            state,
            group: "System".into(),
            card: vergence_protocol::tile::Card {
                title,
                big: String::new(),
                rows: vec![("Allowed".into(), format!("{}{left}", self.agent_span_word(&agent, &app)))],
                chart: Vec::new(),
                controls,
                note: "Settings > Privacy > Agents takes it back.".into(),
            },
            page: "agents".into(),
        };
        self.agents.tile_up = true;
        self.agents.tiled = Some((agent.clone(), app.clone()));
        self.tile_set(crate::tiles::Owner::Desktop, tile);
    }

    fn agent_span_word(&self, agent: &Who, app: &str) -> &'static str {
        match self.agents.grants.get(&(agent.clone(), app.to_string())) {
            Some(Span::Always) => "always",
            Some(Span::Session) => "until you log out",
            Some(Span::Connection(_)) => "just now",
            Some(Span::Until(_)) => "for a time",
            _ => "",
        }
    }

    /// An app's name as people know it: its desktop entry's (by id, or the
    /// id's last part), else its id. Never a window's title: that is the
    /// app's own word, and a terminal's is whatever runs in it (Fable's review).
    pub fn app_shown_name(&self, app: &str) -> String {
        let app = app_id_of(app);
        let low = app.to_lowercase();
        self.shelf
            .apps
            .iter()
            .find(|e| {
                let id = e.id.to_lowercase();
                id == low || id.rsplit('.').next() == Some(low.as_str())
            })
            .map(|e| e.name.clone())
            .filter(|n| !n.trim().is_empty())
            .map(|n| clean_agent_name(&n))
            .unwrap_or_else(|| app.to_string())
    }

    /// In front of everything on a screen: the panel (on the pointer's
    /// screen), then the dimming.
    pub fn agent_dialog_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        out: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Vec<SceneElement> {
        let mut elements = Vec::new();
        if out.to_f64().contains(self.pointer_location())
            && let Some(e) = self.agent_dialog_panel(renderer, out, scale)
        {
            elements.push(e);
        }
        let (id, commit) = self.agents.dim.get_or_insert_with(|| (Id::new(), CommitCounter::default())).clone();
        let phys =
            Rectangle::<i32, smithay::utils::Physical>::from_size(out.size.to_f64().to_physical(scale).to_i32_round());
        elements.push(SceneElement::Solid(SolidColorRenderElement::new(
            id,
            phys,
            commit,
            [0.0, 0.0, 0.0, 0.45],
            Kind::Unspecified,
        )));
        elements
    }

    fn agent_dialog_panel(
        &mut self,
        renderer: &mut GlesRenderer,
        out: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Option<SceneElement> {
        let q = self.agents.asking.front()?.clone();
        let light = self.light_style();
        let focus = self.agents.focus;
        let accent = self.accent();
        let at = (out.loc.x + (out.size.w - W) / 2, out.loc.y + (out.size.h - H) / 2);
        let panel: Rectangle<i32, Logical> = Rectangle::new(at.into(), (W, H).into());
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        let face = fonts.face("sans-serif", 400, false)?;
        let bold = fonts.face("sans-serif", 600, false).unwrap_or(face);
        let timed = timed_label(agents_settings().1);
        let label_of =
            |b: Button, l: &'static str| -> String { if b == Button::Hour { timed.clone() } else { l.to_string() } };
        let buttons = q.buttons();
        let widths: Vec<i32> = buttons
            .iter()
            .map(|(b, label)| {
                let (f, side) = if *b == Button::Hour { (bold, DEFAULT_SIDE) } else { (face, BUTTON_SIDE) };
                (fonts.layout(f, TEXT, &label_of(*b, label)).1 + 2.0 * side).ceil() as i32
            })
            .collect();
        let row_y = H - PAD - BUTTON_H;
        // Don't allow on the left, the four lengths to the right.
        let mut local: Vec<(usize, i32, i32)> = vec![(0, PAD, widths[0])];
        let mut x = W - PAD;
        for i in (1..buttons.len()).rev() {
            x -= widths[i];
            local.push((i, x, widths[i]));
            x -= BUTTON_GAP;
        }
        local.sort_by_key(|(i, _, _)| *i);
        self.agents.panel = panel;
        self.agents.buttons = local
            .iter()
            .map(|(i, x, w)| (*i, Rectangle::new((at.0 + x, at.1 + row_y).into(), (*w, BUTTON_H).into())))
            .collect();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (focus, light, scale.to_bits(), accent.r, accent.g, accent.b, &q.agent, &q.shown, &timed, q.expired, &q.scope)
            .hash(&mut h);
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
            // The heading in the desktop's words; the agent's name only quoted.
            let shown: String = q.shown.chars().take(32).collect();
            // A shell-access tab: its name, quoted, and what the agent would do.
            let tab: Option<String> = q.scope.as_ref().map(|sc| {
                let t: String = clean_agent_name(&sc.tab).chars().take(28).collect();
                format!("\u{201c}{t}\u{201d}")
            });
            let heading: String = match (&q.scope, &tab, q.expired) {
                (Some(sc), _, _) if sc.id == NEW_TERMINAL => "An agent wants a terminal of its own".to_string(),
                (Some(sc), Some(t), _) if sc.level == Level::Act => format!("An agent wants to type in the tab {t}"),
                (Some(_), Some(t), _) => format!("An agent wants to read the tab {t}"),
                (_, _, true) => format!("An agent's time with {shown} is up"),
                _ => format!("An agent wants to use {shown}"),
            };
            vergence_render::text(
                &mut scene,
                fonts,
                bold,
                18.0,
                ink(s.menu_ink),
                ScenePoint::new(PAD as f32, (PAD as f32 + m.ascent).round()),
                &heading,
            );
            let m = fonts.metrics(face, TEXT);
            let (a, b) = (s.menu_ink, s.menu_ink_off);
            let soft = Color::rgb(
                ((a.0 as u16 + b.0 as u16) / 2) as u8,
                ((a.1 as u16 + b.1 as u16) / 2) as u8,
                ((a.2 as u16 + b.2 as u16) / 2) as u8,
            );
            // Fixed rows: what each one says cannot push another out.
            let program: String =
                q.agent.program.chars().rev().take(56).collect::<Vec<_>>().into_iter().rev().collect();
            let rows = [
                ("Agent", format!("\u{201c}{}\u{201d}", q.agent.name)),
                ("Program", if program.is_empty() { "not known".into() } else { program }),
                ("App", {
                    let p = app_program_of(&q.app);
                    let p = p.rsplit('/').next().unwrap_or(p);
                    format!(
                        "{shown} ({}{})",
                        app_id_of(&q.app).chars().take(32).collect::<String>(),
                        if p.is_empty() {
                            String::new()
                        } else {
                            format!(", {}", p.chars().take(24).collect::<String>())
                        }
                    )
                }),
            ];
            for (k, (label, value)) in rows.iter().enumerate() {
                let y = (PAD as f32 + 32.0 + 19.0 * k as f32 + m.ascent).round();
                vergence_render::text(&mut scene, fonts, face, TEXT, soft, ScenePoint::new(PAD as f32, y), label);
                vergence_render::text(
                    &mut scene,
                    fonts,
                    face,
                    TEXT,
                    ink(s.menu_ink),
                    ScenePoint::new(PAD as f32 + 72.0, y),
                    value,
                );
            }
            let line = match &q.scope {
                Some(sc) if sc.id == NEW_TERMINAL => {
                    "In a window of its own: what it types runs as you. Password prompts and root shells refuse it."
                }
                Some(sc) if sc.level == Level::Act => {
                    "This tab only: what it types runs as you. Password prompts and root shells refuse it."
                }
                Some(_) => "This tab only: its screen and what scrolled by. Other tabs stay hidden from it.",
                None if q.expired => {
                    "Keep allowing it to read the app's windows and act in them? What it was doing waits."
                }
                None => {
                    "It could read the app's windows and press, type and choose in them. A secret field stays secret."
                }
            };
            vergence_render::text(
                &mut scene,
                fonts,
                face,
                TEXT,
                soft,
                ScenePoint::new(PAD as f32, (PAD as f32 + 32.0 + 19.0 * 3.0 + 6.0 + m.ascent).round()),
                line,
            );
            let plain = Color::rgba(s.menu_ink.0, s.menu_ink.1, s.menu_ink.2, 20);
            for (i, x, bw) in &local {
                let r = SceneRect::new(*x as f32, row_y as f32, *bw as f32, BUTTON_H as f32);
                let default = buttons[*i].0 == Button::Hour;
                let (ground, text, f) =
                    if default { (accent, Color::rgb(0xff, 0xff, 0xff), bold) } else { (plain, ink(s.menu_ink), face) };
                scene.fill_rounded(r, 8.0, ground);
                if *i == focus {
                    scene.stroke(
                        SceneRect::new(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0),
                        10.0,
                        2.0,
                        ink(s.menu_ink),
                    );
                }
                let label = label_of(buttons[*i].0, buttons[*i].1);
                let lw = fonts.layout(f, TEXT, &label).1;
                let lm = fonts.metrics(f, TEXT);
                vergence_render::text(
                    &mut scene,
                    fonts,
                    f,
                    TEXT,
                    text,
                    ScenePoint::new(
                        (r.x + (r.w - lw) / 2.0).round(),
                        (r.y + (r.h - lm.ascent - lm.descent) / 2.0 + lm.ascent).round(),
                    ),
                    &label,
                );
            }
            let p = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)).ok()?;
            let buffer = MemoryRenderBuffer::from_slice(
                &p.data,
                Fourcc::Abgr8888,
                (p.width as i32, p.height as i32),
                1,
                Transform::Normal,
                None,
            );
            self.agents.picture = Some((key, buffer));
        }
        let (_, buffer) = self.agents.picture.as_ref()?;
        let a = (panel.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let b = (panel.loc + panel.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            a.to_f64(),
            buffer,
            None,
            Some(src),
            Some(panel.size),
            Kind::Unspecified,
        )
        .ok()
        .map(SceneElement::Memory)
    }

    /// Every grant, for Privacy > Agents (bus v27).
    pub fn agents_grants(&mut self) -> Vec<vergence_protocol::bus::Grant> {
        self.agents.load();
        let now = now_secs();
        let working = self
            .agents
            .last
            .as_ref()
            .filter(|(_, _, t)| t.elapsed().as_secs() < WORKING_SECS)
            .map(|(a, p, _)| (a.clone(), p.clone()));
        let mut out: Vec<vergence_protocol::bus::Grant> = self
            .agents
            .grants
            .iter()
            .filter(|(_, s)| !matches!(s, Span::Until(t) if *t <= now))
            .map(|((a, p), s)| {
                let (span, left) = match s {
                    Span::Connection(_) => ("just now", 0),
                    Span::Until(t) => ("timed", t.saturating_sub(now).div_ceil(60) as u32),
                    Span::Session => ("session", 0),
                    Span::Always => ("always", 0),
                    Span::Never => ("never", 0),
                };
                vergence_protocol::bus::Grant {
                    agent: a.name.clone(),
                    program: a.program.clone(),
                    app: p.clone(),
                    shown: self.app_shown_name(p),
                    span: span.into(),
                    minutes_left: left,
                    working: working.as_ref() == Some(&(a.clone(), p.clone())),
                }
            })
            .collect();
        out.sort_by_key(|g| (g.agent.to_lowercase(), g.shown.to_lowercase()));
        out
    }

    /// Takes grants back (Privacy > Agents): one, an agent's (`app` empty)
    /// or all (both empty). The agent tile goes if its agent lost its grant.
    pub fn agents_revoke(&mut self, agent: &str, program: &str, app: &str) {
        self.agents.load();
        let before = self.agents.grants.len();
        self.agents.grants.retain(|(a, p), _| {
            !((agent.is_empty() || a.name == agent)
                && (program.is_empty() || a.program == program)
                && (app.is_empty() || p == app))
        });
        if self.agents.grants.len() != before {
            tracing::info!("agents: grants taken back ({agent:?}, {program:?}, {app:?})");
            self.agent_log_named(agent, app, &format!("taken back from Privacy > Agents ({program})"));
            self.agents.save();
            if let Some((a, p, _)) = &self.agents.last
                && !self.agents.grants.contains_key(&(a.clone(), p.clone()))
            {
                self.agents.last = None;
                if self.agents.tile_up {
                    self.agents.tile_up = false;
                    self.tile_remove(crate::tiles::Owner::Desktop, "agent");
                }
            }
        }
    }

    /// The agent tile's "Allow ... more": the timed grant the tile shows
    /// goes on by one more span, from when it would have ended, never past
    /// a day from now.
    pub fn agent_extend(&mut self) {
        let Some((agent, app)) = self.agents.tiled.clone() else { return };
        let key = (agent.clone(), app.clone());
        let Some(Span::Until(t)) = self.agents.grants.get(&key).copied() else { return };
        let until = (t.max(now_secs()) + agents_settings().1 * 60).min(now_secs() + 24 * 3600);
        self.agents.grants.insert(key, Span::Until(until));
        self.agents.save();
        self.agent_log_named(&agent.name, &app, "extended from the tile");
        tracing::info!("agents: {agent:?} and {app:?} extended");
        self.agent_tile();
    }

    fn agent_log_named(&mut self, agent: &str, app: &str, what: &str) {
        if cfg!(test) {
            return;
        }
        let Some(dir) = state_dir() else { return };
        rotate_log(&dir);
        if let Ok(mut f) =
            std::fs::OpenOptions::new().append(true).create(true).mode(0o600).open(dir.join("agents.log"))
        {
            let what: String = clean(what).chars().take(200).collect();
            let _ = writeln!(f, "{}\t{}\t{}\t{what}", utc(now_secs()), clean(agent), clean(app));
        }
    }

    /// A ring where an agent acts (step 5): `at` on the screens, in the
    /// agent's own color; held, then faded.
    pub fn agent_ring(&mut self, agent: &str, at: (f64, f64, f64, f64), within: (f64, f64, f64, f64)) {
        if !(at.0.is_finite() && at.1.is_finite() && at.2.is_finite() && at.3.is_finite()) {
            return;
        }
        // Kept within the window (Fable's review: an app's word must not
        // draw a ring over another app, nor a huge one).
        let x0 = at.0.max(within.0);
        let y0 = at.1.max(within.1);
        let x1 = (at.0 + at.2).min(within.0 + within.2);
        let y1 = (at.1 + at.3).min(within.1 + within.3);
        if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
            return;
        }
        let at = (x0, y0, x1 - x0, y1 - y0);
        let color = ring_color(agent);
        self.agents.rings.retain(|r| r.since.elapsed() < RING_HOLD + RING_FADE);
        if self.agents.rings.len() >= RINGS_MAX {
            self.agents.rings.remove(0);
        }
        self.agents.rings.push(Ring { at, color, since: Instant::now(), picture: None });
        if !self.agents.ring_timer {
            self.agents.ring_timer = true;
            use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
            let _ = self.loop_handle.insert_source(Timer::from_duration(RING_STEP), |_, _, st| {
                st.agents.rings.retain(|r| r.since.elapsed() < RING_HOLD + RING_FADE);
                st.redraw_all();
                if st.agents.rings.is_empty() {
                    st.agents.ring_timer = false;
                    return TimeoutAction::Drop;
                }
                TimeoutAction::ToDuration(RING_STEP)
            });
        }
        self.redraw_all();
    }

    /// The rings on this screen, in front of the windows.
    pub fn agent_ring_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        out: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Vec<SceneElement> {
        let motion = self.motion_wanted();
        let mut elements = Vec::new();
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        for r in self.agents.rings.iter_mut() {
            let t = r.since.elapsed();
            if t >= RING_HOLD + RING_FADE {
                continue;
            }
            let alpha = if !motion || t <= RING_HOLD {
                1.0
            } else {
                1.0 - (t - RING_HOLD).as_secs_f32() / RING_FADE.as_secs_f32()
            };
            let (x, y, w, h) = (r.at.0 - RING_PAD, r.at.1 - RING_PAD, r.at.2 + 2.0 * RING_PAD, r.at.3 + 2.0 * RING_PAD);
            let o = out.to_f64();
            if x + w < o.loc.x || y + h < o.loc.y || x > o.loc.x + o.size.w || y > o.loc.y + o.size.h {
                continue;
            }
            if r.picture.as_ref().map(|(k, _)| *k) != Some(scale.to_bits()) {
                let mut scene = Scene::new(w as f32, h as f32);
                let (c0, c1, c2) = r.color;
                scene.stroke(
                    SceneRect::new(1.5, 1.5, w as f32 - 3.0, h as f32 - 3.0),
                    9.0,
                    3.0,
                    Color::rgb(c0, c1, c2),
                );
                let Ok(p) = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0))
                else {
                    continue;
                };
                let buffer = MemoryRenderBuffer::from_slice(
                    &p.data,
                    Fourcc::Abgr8888,
                    (p.width as i32, p.height as i32),
                    1,
                    Transform::Normal,
                    None,
                );
                r.picture = Some((scale.to_bits(), buffer));
            }
            let Some((_, buffer)) = r.picture.as_ref() else { continue };
            let loc = Point::<f64, Logical>::from((x - o.loc.x, y - o.loc.y)).to_physical(scale);
            let size = smithay::utils::Size::<i32, Logical>::from((w.round() as i32, h.round() as i32));
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                loc,
                buffer,
                Some(alpha),
                None,
                Some(size),
                Kind::Unspecified,
            ) {
                elements.push(SceneElement::Memory(e));
            }
        }
        elements
    }

    /// For `vctl agents`: the grants, the questions, the waiting requests.
    pub fn agents_report(&mut self) -> String {
        self.agents.load();
        let mut out: Vec<String> = self
            .agents
            .grants
            .iter()
            .map(|((a, p), s)| match s {
                Span::Until(t) => format!(
                    "{:?} ({}) {p:?}: for {} more min",
                    a.name,
                    a.program,
                    t.saturating_sub(now_secs()).div_ceil(60)
                ),
                s => format!("{:?} ({}) {p:?}: {s:?}", a.name, a.program),
            })
            .collect();
        // Tabs of shell-access apps (memory only).
        out.extend(self.agents.scoped.iter().map(|((a, p, id), (s, l))| match s {
            Span::Until(t) => format!(
                "{:?} ({}) {p:?} tab #{id} {l:?}: for {} more min",
                a.name,
                a.program,
                t.saturating_sub(now_secs()).div_ceil(60)
            ),
            s => format!("{:?} ({}) {p:?} tab #{id} {l:?}: {s:?}", a.name, a.program),
        }));
        out.sort();
        if let Some(q) = self.agents.asking.front() {
            out.push(format!(
                "asking: {:?} ({}) wants to use {:?}{} ({} questions, {} requests waiting{})",
                q.agent.name,
                q.agent.program,
                q.shown,
                q.scope.as_ref().map_or(String::new(), |sc| format!(", tab {:?} #{} {:?}", sc.tab, sc.id, sc.level)),
                self.agents.asking.len(),
                self.agents.pending.len(),
                if self.agent_dialog_armed() { "" } else { "; not yet answerable" }
            ));
        }
        if out.is_empty() { "no grants".into() } else { out.join("\n") }
    }

    /// For `vctl agent-answer <button>`: answers the question showing, as
    /// a press of that button would (tests in headless sessions).
    pub fn agent_answer_by_word(&mut self, word: &str) -> String {
        if self.agents.asking.is_empty() {
            return "nothing is being asked".into();
        }
        let want = match word {
            "deny" | "dont" | "no" => Button::DontAllow,
            "now" | "just-now" => Button::JustNow,
            "session" => Button::Session,
            "always" => Button::Always,
            "hour" | "timed" => Button::Hour,
            _ => return "agent-answer deny|now|session|always|hour".into(),
        };
        let buttons = self.agents.asking.front().map_or(&BUTTONS[..], |q| q.buttons());
        let Some(i) = buttons.iter().position(|(b, _)| *b == want) else {
            return format!("this question does not offer {word:?}");
        };
        let label = buttons[i].1;
        self.agent_answer(i);
        format!("answered {label:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(name: &str) -> Who {
        Who { name: name.into(), program: "/usr/local/bin/vergence-agent-mcp".into() }
    }

    #[test]
    fn grants_are_kept_and_read_back() {
        let mut g = HashMap::new();
        g.insert((who("Claude"), "vergence-calc".to_string()), Span::Always);
        g.insert((who("Claude"), "vergence-term".to_string()), Span::Never);
        g.insert((who("Other\tagent"), "x".to_string()), Span::Until(1_800_000_000));
        g.insert((who("Claude"), "vergence-edit".to_string()), Span::Session);
        g.insert((who("Claude"), "vergence-files".to_string()), Span::Connection(7));
        let text = render(&g);
        let back = parse(&text);
        assert_eq!(back.len(), 3, "only always, never and timed ones are kept: {text}");
        assert_eq!(back.get(&(who("Claude"), "vergence-calc".into())), Some(&Span::Always));
        assert_eq!(back.get(&(who("Claude"), "vergence-term".into())), Some(&Span::Never));
        assert_eq!(
            back.get(&(who("Other agent"), "x".into())),
            Some(&Span::Until(1_800_000_000)),
            "a tab in a name cannot split its line"
        );
        assert!(
            parse("junk\nClaude\tapp\talways\nClaude\tp\tapp\tsometimes\n").is_empty(),
            "old three-field lines and nonsense are dropped"
        );
    }

    #[test]
    fn what_a_grant_says() {
        let mut a = Agents { loaded: true, ..Default::default() };
        let c = who("Claude");
        assert_eq!(a.may(&c, 3, "calc"), May::Ask);
        a.grants.insert((c.clone(), "calc".into()), Span::Connection(3));
        assert_eq!(a.may(&c, 3, "calc"), May::Yes);
        assert_eq!(a.may(&c, 4, "calc"), May::Ask, "another connection is asked again");
        let impostor = Who { name: "Claude".into(), program: "/tmp/evil".into() };
        a.grants.insert((c.clone(), "calc".into()), Span::Always);
        assert_eq!(a.may(&impostor, 4, "calc"), May::Ask, "the same name from another program has nothing");
        a.grants.insert((c.clone(), "calc".into()), Span::Until(now_secs() + 600));
        assert_eq!(a.may(&c, 4, "calc"), May::Yes);
        assert_eq!(a.minutes_left(&c, "calc"), Some(10));
        a.grants.insert((c.clone(), "calc".into()), Span::Until(now_secs().saturating_sub(1)));
        assert_eq!(a.may(&c, 4, "calc"), May::Expired, "an hour that has passed asks again, saying so");
        assert!(!a.grants.contains_key(&(c.clone(), "calc".into())));
        a.grants.insert((c.clone(), "calc".into()), Span::Never);
        assert_eq!(a.may(&c, 4, "calc"), May::No);
        assert_eq!(a.may(&who("Other"), 4, "calc"), May::Ask, "per agent");
        assert_eq!(a.may(&c, 4, "term"), May::Ask, "per app");
        // Don't allow: refused quietly for a minute, then asked again.
        a.refused.insert((c.clone(), "term".into()), Instant::now());
        assert_eq!(a.may(&c, 4, "term"), May::No);
        a.refused
            .insert((c.clone(), "term".into()), Instant::now() - std::time::Duration::from_secs(COOL_DOWN_SECS + 1));
        assert_eq!(a.may(&c, 4, "term"), May::Ask);
    }

    #[test]
    fn a_tab_s_grant() {
        let mut a = Agents { loaded: true, ..Default::default() };
        let c = who("Claude");
        let tab = |id: u64, level: Level| Scope { id, tab: "zsh".into(), level };
        let app = "vergence-term @ /usr/bin/vergence-term";
        assert_eq!(a.may_scoped(&c, 3, app, &tab(10, Level::Read)), May::Ask);
        // An Always for the whole app, kept from before, counts for nothing here.
        a.grants.insert((c.clone(), app.into()), Span::Always);
        assert_eq!(a.may_scoped(&c, 3, app, &tab(10, Level::Read)), May::Ask);
        // Read granted: reading yes, typing asks again, another tab asks.
        a.scoped.insert((c.clone(), app.into(), 10), (Span::Connection(3), Level::Read));
        assert_eq!(a.may_scoped(&c, 3, app, &tab(10, Level::Read)), May::Yes);
        assert_eq!(a.may_scoped(&c, 3, app, &tab(10, Level::Act)), May::Ask, "reading is not typing");
        assert_eq!(a.may_scoped(&c, 3, app, &tab(11, Level::Read)), May::Ask, "one tab, not the app");
        assert_eq!(a.may_scoped(&c, 4, app, &tab(10, Level::Read)), May::Ask, "Just now is this connection's");
        // Typing granted: it covers reading.
        a.scoped.insert((c.clone(), app.into(), 10), (Span::Until(now_secs() + 600), Level::Act));
        assert_eq!(a.may_scoped(&c, 4, app, &tab(10, Level::Read)), May::Yes);
        assert_eq!(a.may_scoped(&c, 4, app, &tab(10, Level::Act)), May::Yes);
        assert_eq!(a.scopes_held(&c, 4, app, Level::Read), vec![10]);
        assert!(a.scopes_held(&who("Other"), 4, app, Level::Read).is_empty(), "per agent");
        // A refusal is the tab's and the level's.
        a.refused.insert((c.clone(), gate_key(app, Some(&tab(11, Level::Read)))), Instant::now());
        assert_eq!(a.may_scoped(&c, 4, app, &tab(11, Level::Read)), May::No);
        assert_eq!(a.may_scoped(&c, 4, app, &tab(12, Level::Read)), May::Ask);
        // Timed out: asked again, saying so; the stop keys revoke at once.
        a.scoped.insert((c.clone(), app.into(), 13), (Span::Until(now_secs().saturating_sub(1)), Level::Act));
        assert_eq!(a.may_scoped(&c, 4, app, &tab(13, Level::Read)), May::Expired);
        assert_eq!(a.revoke_scope(app, 10), 1);
        // Stopped by the person: refused there for the cool-down, saying so,
        // at both levels, not asked again at once (step 4).
        assert_eq!(a.may_scoped(&c, 4, app, &tab(10, Level::Read)), May::No);
        assert_eq!(a.may_scoped(&c, 4, app, &tab(10, Level::Act)), May::No);
        assert!(a.stopped_lately(&c, app, 10));
        assert!(!a.stopped_lately(&c, app, 11));
    }

    /// A terminal of the agent's own (step 4): the grant asked under
    /// NEW_TERMINAL moves to its tab, at the typing level, once.
    #[test]
    fn a_terminal_of_its_own() {
        let mut a = Agents { loaded: true, ..Default::default() };
        let c = who("Claude");
        let tab = |id: u64, level: Level| Scope { id, tab: "zsh".into(), level };
        let app = "vergence-term @ /usr/bin/vergence-term";
        assert_eq!(a.take_new_terminal(&c, app), None, "nothing granted");
        a.scoped.insert((c.clone(), app.into(), NEW_TERMINAL), (Span::Connection(3), Level::Act));
        let span = a.take_new_terminal(&c, app).unwrap();
        assert_eq!(a.take_new_terminal(&c, app), None, "taken once");
        a.grant_tab(&c, app, 42, span);
        assert_eq!(a.may_scoped(&c, 3, app, &tab(42, Level::Act)), May::Yes);
        assert_eq!(a.scopes_held(&c, 3, app, Level::Act), vec![42]);
        assert_eq!(a.may_scoped(&who("Other"), 3, app, &tab(42, Level::Read)), May::Ask, "its own, no one else's");
    }

    #[test]
    fn a_tab_s_question_offers_no_always() {
        let q = |scope: Option<Scope>| Question {
            agent: who("Claude"),
            client: 3,
            app: "vergence-term @ x".into(),
            shown: "Terminal".into(),
            expired: false,
            scope,
        };
        let shell = q(Some(Scope { id: 10, tab: "zsh".into(), level: Level::Act }));
        let words: Vec<&str> = shell.buttons().iter().map(|b| b.1).collect();
        assert_eq!(words, vec!["Don't allow", "Just now", "For an hour"]);
        assert!(!shell.buttons().iter().any(|b| matches!(b.0, Button::Always | Button::Session)));
        assert_eq!(q(None).buttons().len(), 5, "an ordinary app as ever");
        assert_ne!(shell.key(), q(None).key(), "a tab's question is kept apart from the app's");
    }

    #[test]
    fn agent_names_are_cleaned() {
        assert_eq!(clean_agent_name("  Claude  "), "Claude");
        assert_eq!(clean_agent_name("Cl\u{202e}aude\u{200b}"), "Claude", "direction and zero-width marks go");
        assert_eq!(
            clean_agent_name("System update: press Enter to continue now"),
            "System update: press Ent",
            "at most 24 characters"
        );
        assert_eq!(clean_agent_name("\u{7}\n"), "");
    }

    #[test]
    fn times_in_the_log() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_763_668_800), "2025-11-20T20:00:00Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
    }
}
