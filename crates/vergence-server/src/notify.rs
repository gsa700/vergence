// Notifications, drawn by the server (whitepaper §6.8; bus milestone 1 step 3,
// 2026-11-08).
//
// One service in the core, two front ends: the session bus's Notify message
// (bus.rs) and org.freedesktop.Notifications on the D-Bus edge (dbus.rs). A
// notification belongs to the connection that made it: only that connection
// can replace or close it, and only it hears that it closed or that an action
// was chosen (unicast, never broadcast).
//
// Cards stack down from the top-right corner (or the place chosen) of the
// monitor the pointer was on when the first of them appeared, sized in points
// like the menu and title bars, so they are the same physical size on every
// monitor. Each is drawn as a scene (as a Dockworker's card): the app's name
// small, the title semibold, the body, its buttons; an accent stripe down its
// left edge (the alarm red for a critical one) on the cards' frost, so they
// are seen (Dave, 2026-11-27: "I have had notifications go unseen for a while
// because it just gets lost in the desktop"). Light or dark follows the
// color-scheme setting. A click on a card runs its "default" action (if it has
// one) and closes it; other actions are buttons under the text. Cards expire
// after their timeout (when the app leaves it to the server: 5 s, 10 s for a
// card with buttons, and critical ones stay until clicked). While the pointer
// is over any card, every countdown pauses, and resumes when it leaves.
//
// desktop.toml [notifications] (the settings app's Notifications page; see
// docs/settings-milestone-2.md):
//
//   do-not-disturb = false   true: apps' cards are not shown (critical ones and
//                            the server's own, the volume level, still are)
//   seconds = 5              how long a card stays when the app leaves it to the
//                            desktop (a card with buttons at least 10 s)
//   corner = "top-right"     top-right, top-left, top-center (as GNOME's),
//                            bottom-right or bottom-left; at the bottom the
//                            newest card is nearest the corner
//   off = ["Firefox"]        apps whose cards are never shown, critical ones too
//                            (the name an app gives, any case)
//
// Every app that sends a card is remembered, with when it last did, in
// ~/.local/state/vergence/notify-apps.toml (owner only), for the page's list
// of apps; the server's own cards are not. Written when an app is new or was
// last written down more than an hour ago, so a chatty app costs nothing.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            },
            gles::GlesRenderer,
        },
    },
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::{Logical, Point, Rectangle, Transform},
};

use vergence_scene::{Color, Point as ScenePoint, Rect as SceneRect, Scene};

use crate::{Server, decor::SceneElement};

/// A card: wider and roomier than it was (340 and 10), to be seen.
const WIDTH: i32 = 400;
const PAD: i32 = 14;
/// The accent stripe down the card's left edge.
const STRIPE: i32 = 4;
const GAP: i32 = 10;
const MARGIN: i32 = 12;
const MAX_BODY_LINES: usize = 4;
const MAX_SHOWN: usize = 5;
const BUTTON_H: i32 = 28;
/// The app's name, the title and the body: sizes and line heights, in points.
const APP_SIZE: f32 = 11.0;
const APP_H: i32 = 17;
const TITLE_SIZE: f32 = 15.0;
const TITLE_H: i32 = 21;
const BODY_SIZE: f32 = 13.0;
const BODY_H: i32 = 18;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// For cards with buttons: time to read them and reach for one.
const BUTTONS_TIMEOUT: Duration = Duration::from_secs(10);
/// After a pause, at least this long is left.
const MIN_AFTER_PAUSE: Duration = Duration::from_secs(2);

/// Reasons in NotificationClosed (the Desktop Notifications specification's numbers).
pub const EXPIRED: u32 = 1;
pub const DISMISSED: u32 = 2;
pub const CLOSED: u32 = 3;

/// Who made a notification, and so who hears about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A D-Bus edge connection.
    DBus(u32),
    /// A session bus client.
    Bus(u32),
    /// The server itself (the volume keys' level, sound.rs).
    Server,
}

pub struct Request {
    pub app: String,
    pub replaces: u32,
    pub summary: String,
    pub body: String,
    /// 0 low, 1 normal, 2 critical.
    pub urgency: u8,
    /// -1: the server decides; 0: never expires; else milliseconds.
    pub timeout_ms: i32,
    /// (key, label) pairs; "default" is the click on the card.
    pub actions: Vec<(String, String)>,
}

struct Note {
    id: u32,
    origin: Origin,
    app: String,
    summary: String,
    body: String,
    urgency: u8,
    actions: Vec<(String, String)>,
    expires: Option<Instant>,
    /// While paused: the time that was left (and `expires` is None).
    left: Option<Duration>,
    /// Laid out: the card, its text lines and its buttons (logical).
    rect: Rectangle<i32, Logical>,
    lines: Vec<(String, Rectangle<i32, Logical>, Line)>,
    buttons: Vec<(String, Rectangle<i32, Logical>)>,
    /// Its picture as last drawn (by what it was drawn from), and its frost.
    picture: Option<(u64, MemoryRenderBuffer)>,
    frost: crate::backdrop::Slot,
}

/// What a line of a card is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Line {
    App,
    Title,
    Body,
}

/// Where the cards stack: a corner of the work area, or the middle of its top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    TopRight,
    TopLeft,
    TopCenter,
    BottomRight,
    BottomLeft,
}

#[derive(Default)]
pub struct Notifications {
    notes: Vec<Note>,
    next_id: u32,
    /// The monitor the stack is on (its top-left), while there is a stack.
    anchor: Option<Rectangle<i32, Logical>>,
    timer_running: bool,
    /// The pointer is over a card: countdowns are paused.
    paused: bool,
    pub config: Config,
    /// App names that sent cards, and when they were last written down (unix
    /// s): only to skip writing; the file is the list.
    apps: Option<HashMap<String, u64>>,
}

/// `[notifications]` from desktop.toml.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub do_not_disturb: bool,
    pub timeout: Duration,
    pub corner: Place,
    /// Apps switched off, in lower case.
    pub off: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config { do_not_disturb: false, timeout: DEFAULT_TIMEOUT, corner: Place::TopRight, off: Vec::new() }
    }
}

pub fn parse(text: &str) -> Config {
    let d = Config::default();
    let Some(t) =
        text.parse::<toml::Table>().ok().and_then(|t| t.get("notifications").and_then(|n| n.as_table()).cloned())
    else {
        return d;
    };
    Config {
        do_not_disturb: t.get("do-not-disturb").and_then(|v| v.as_bool()).unwrap_or(false),
        timeout: t
            .get("seconds")
            .and_then(|v| v.as_integer())
            .map(|s| Duration::from_secs(s.clamp(2, 600) as u64))
            .unwrap_or(d.timeout),
        corner: match t.get("corner").and_then(|v| v.as_str()).unwrap_or("") {
            "top-left" => Place::TopLeft,
            "top-center" => Place::TopCenter,
            "bottom-right" => Place::BottomRight,
            "bottom-left" => Place::BottomLeft,
            _ => Place::TopRight,
        },
        off: t
            .get("off")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).map(|s| s.trim().to_lowercase()).collect())
            .unwrap_or_default(),
    }
}

/// The longest app name kept.
const MAX_APP: usize = 64;

fn apps_path() -> Option<std::path::PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))?;
    Some(state.join("vergence/notify-apps.toml"))
}

/// `[apps]` name = last time (unix seconds).
pub fn parse_apps(text: &str) -> HashMap<String, u64> {
    text.parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("apps").and_then(|a| a.as_table()).cloned())
        .map(|a| a.into_iter().filter_map(|(k, v)| v.as_integer().map(|t| (k, t.max(0) as u64))).collect())
        .unwrap_or_default()
}

fn read_apps() -> HashMap<String, u64> {
    apps_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse_apps(&t)).unwrap_or_default()
}

/// Written beside and moved into place, readable by its owner only.
fn write_apps(apps: &HashMap<String, u64>) -> std::io::Result<()> {
    use std::{
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    };
    let Some(path) = apps_path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let mut table = toml::Table::new();
    table.insert(
        "apps".into(),
        toml::Value::Table(apps.iter().map(|(k, v)| (k.clone(), toml::Value::Integer(*v as i64))).collect()),
    );
    let text = toml::to_string(&table).map_err(std::io::Error::other)?;
    let beside = path.with_extension("toml.new");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&beside)?;
    file.write_all(text.as_bytes())?;
    std::fs::rename(beside, path)
}

pub fn load() -> Config {
    Some(crate::wallpaper::desktop_text()).map(|t| parse(&t)).unwrap_or_default()
}

/// Words into lines no wider than `max` (by `width`), at most `max_lines`, the
/// last cut with an ellipsis if text is left over. Newlines in the text are kept.
pub fn wrap(text: &str, max: f32, max_lines: usize, width: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut leftover = false;
    'outer: for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if width(&candidate) <= max || line.is_empty() {
                line = candidate;
            } else {
                if lines.len() + 1 >= max_lines {
                    lines.push(line);
                    leftover = true;
                    break 'outer;
                }
                lines.push(std::mem::replace(&mut line, word.to_string()));
            }
        }
        if lines.len() >= max_lines {
            leftover = true;
            break;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    if leftover && let Some(last) = lines.last_mut() {
        let ell = "\u{2026}";
        while !last.is_empty() && width(&format!("{last}{ell}")) > max {
            last.pop();
        }
        last.push_str(ell);
    }
    lines
}

impl Server {
    /// A new notification (or a replacement for one this origin made); its id.
    pub fn notify(&mut self, origin: Origin, req: Request) -> u32 {
        let buttons = req.actions.iter().any(|(k, _)| !k.is_empty() && k != "default");
        let c = self.notes.config.clone();
        let app: String =
            req.app.chars().filter(|ch| !ch.is_control()).take(MAX_APP).collect::<String>().trim().to_string();
        if origin != Origin::Server && !app.is_empty() {
            self.notify_remember(&app);
        }
        // Switched off on the page: never shown, whatever its urgency.
        if origin != Origin::Server && c.off.contains(&app.to_lowercase()) {
            self.notes.next_id = self.notes.next_id.wrapping_add(1).max(1);
            let id = self.notes.next_id;
            tracing::info!("notify: #{id} from {app} held back (switched off): {:?}", req.summary);
            return id;
        }
        // Do not disturb: an app's card is not shown; the app still gets an id
        // (critical cards still show). It is not told the card closed: that
        // could reach it before the id does.
        if c.do_not_disturb && req.urgency < 2 && origin != Origin::Server {
            self.notes.next_id = self.notes.next_id.wrapping_add(1).max(1);
            let id = self.notes.next_id;
            tracing::info!("notify: #{id} from {} held back (do not disturb): {:?}", req.app, req.summary);
            return id;
        }
        let timeout = match req.timeout_ms {
            t if t < 0 => (req.urgency < 2).then_some(if buttons { c.timeout.max(BUTTONS_TIMEOUT) } else { c.timeout }),
            0 => None,
            t => Some(Duration::from_millis(t as u64)),
        };
        // Paused (pointer over the stack): the countdown starts when it leaves.
        let (expires, left) =
            if self.notes.paused { (None, timeout) } else { (timeout.map(|t| Instant::now() + t), None) };
        let replacing = (req.replaces != 0)
            .then(|| self.notes.notes.iter().position(|n| n.id == req.replaces && n.origin == origin))
            .flatten();
        let id = match replacing {
            Some(i) => self.notes.notes[i].id,
            None => {
                self.notes.next_id = self.notes.next_id.wrapping_add(1).max(1);
                self.notes.next_id
            }
        };
        let actions: Vec<(String, String)> = req.actions.into_iter().filter(|(k, _)| !k.is_empty()).collect();
        let note = Note {
            id,
            origin,
            app: req.app,
            summary: req.summary,
            body: req.body,
            urgency: req.urgency.min(2),
            actions,
            expires,
            left,
            rect: Rectangle::default(),
            lines: Vec::new(),
            buttons: Vec::new(),
            picture: None,
            frost: None,
        };
        tracing::info!("notify: #{id} from {} ({origin:?}): {:?}", note.app, note.summary);
        match replacing {
            Some(i) => self.notes.notes[i] = note,
            None => self.notes.notes.insert(0, note),
        }
        // Too many: the oldest goes.
        while self.notes.notes.len() > MAX_SHOWN {
            let old = self.notes.notes.pop().unwrap();
            self.notify_tell_closed(old.origin, old.id, EXPIRED);
        }
        self.notify_layout();
        self.notify_start_timer();
        id
    }

    /// Close a notification for its origin (the app asked); false if it has no
    /// such notification.
    pub fn notify_close(&mut self, origin: Origin, id: u32) -> bool {
        let Some(i) = self.notes.notes.iter().position(|n| n.id == id && n.origin == origin) else { return false };
        self.notes.notes.remove(i);
        self.notify_tell_closed(origin, id, CLOSED);
        self.notify_layout();
        true
    }

    fn notify_tell_closed(&mut self, origin: Origin, id: u32, reason: u32) {
        match origin {
            Origin::DBus(conn) => self.dbus_notification_closed(conn, id, reason),
            Origin::Bus(client) => self.bus_notification_closed(client, id, reason),
            Origin::Server => {}
        }
    }

    fn notify_tell_action(&mut self, origin: Origin, id: u32, key: &str) {
        match origin {
            Origin::DBus(conn) => self.dbus_action_invoked(conn, id, key),
            Origin::Bus(client) => self.bus_action_invoked(client, id, key),
            Origin::Server => self.server_note_action(id, key),
        }
    }

    /// Place the cards (after any change).
    fn notify_layout(&mut self) {
        if self.notes.notes.is_empty() {
            self.notes.anchor = None;
            // No cards, nothing to hover: the next card's countdown runs.
            self.notes.paused = false;
            self.redraw_all();
            return;
        }
        // Stay on the stack's monitor while it exists (and is still there).
        let outputs: Vec<Rectangle<i32, Logical>> =
            self.space.outputs().filter_map(|o| self.space.output_geometry(o)).collect();
        let anchor = match self.notes.anchor {
            Some(a) if outputs.contains(&a) => a,
            _ => match self.output_at_pointer() {
                Some(o) => o,
                None => return,
            },
        };
        self.notes.anchor = Some(anchor);
        // Beside the shelf's strip, not under it (shelf.rs).
        let area = self.work_area(anchor);
        let mut fonts = self.shelf.fonts.take().unwrap_or_else(vergence_text::Fonts::system);
        let (regular, semibold) = (
            fonts.face("sans-serif", 400, false),
            fonts.face("sans-serif", 600, false).or_else(|| fonts.face("sans-serif", 700, false)),
        );
        // Measured through a cell: wrapping takes a plain Fn, measuring borrows the fonts.
        let cell = std::cell::RefCell::new(fonts);
        let width = |s: &str, line: Line| -> f32 {
            let (face, size) = match line {
                Line::App => (regular, APP_SIZE),
                Line::Title => (semibold, TITLE_SIZE),
                Line::Body => (regular, BODY_SIZE),
            };
            face.map_or(s.chars().count() as f32 * size * 0.55, |f| cell.borrow_mut().layout(f, size, s).1)
        };
        let text_x = PAD + STRIPE;
        let text_w = (WIDTH - text_x - PAD) as f32;
        let place = self.notes.config.corner;
        let bottom = matches!(place, Place::BottomRight | Place::BottomLeft);
        let x = match place {
            Place::TopRight | Place::BottomRight => area.loc.x + area.size.w - WIDTH - MARGIN,
            Place::TopLeft | Place::BottomLeft => area.loc.x + MARGIN,
            Place::TopCenter => area.loc.x + (area.size.w - WIDTH) / 2,
        };
        // At the bottom: laid out from the top, oldest first, then moved down
        // so the newest ends nearest the corner.
        let top = area.loc.y + MARGIN;
        let mut y = top;
        let order: Vec<usize> =
            if bottom { (0..self.notes.notes.len()).rev().collect() } else { (0..self.notes.notes.len()).collect() };
        for i in order {
            let n = &mut self.notes.notes[i];
            let mut lines = Vec::new();
            let mut ty = y + PAD - 2;
            // The app's name small above (unless the title is the app's own name).
            let title = if n.summary.is_empty() { n.app.clone() } else { n.summary.clone() };
            if !n.app.is_empty() && n.app != title {
                let app = wrap(&n.app, text_w, 1, &|s| width(s, Line::App)).into_iter().next().unwrap_or_default();
                lines.push((
                    app,
                    Rectangle::new((x + text_x, ty).into(), (WIDTH - text_x - PAD, APP_H).into()),
                    Line::App,
                ));
                ty += APP_H;
            }
            for t in wrap(&title, text_w, 2, &|s| width(s, Line::Title)) {
                lines.push((
                    t,
                    Rectangle::new((x + text_x, ty).into(), (WIDTH - text_x - PAD, TITLE_H).into()),
                    Line::Title,
                ));
                ty += TITLE_H;
            }
            for t in wrap(&n.body, text_w, MAX_BODY_LINES, &|s| width(s, Line::Body)) {
                lines.push((
                    t,
                    Rectangle::new((x + text_x, ty).into(), (WIDTH - text_x - PAD, BODY_H).into()),
                    Line::Body,
                ));
                ty += BODY_H;
            }
            let mut buttons = Vec::new();
            let shown: Vec<&(String, String)> = n.actions.iter().filter(|(k, _)| k != "default").collect();
            if !shown.is_empty() {
                ty += 8;
                let mut bx = x + text_x;
                for (key, label) in shown {
                    let w = (width(label, Line::Body).ceil() as i32 + 24).min(WIDTH - text_x - PAD);
                    if bx + w > x + WIDTH - PAD {
                        break;
                    }
                    buttons.push((key.clone(), Rectangle::new((bx, ty).into(), (w, BUTTON_H).into())));
                    bx += w + 8;
                }
                ty += BUTTON_H;
            }
            let h = ty + PAD - y;
            n.rect = Rectangle::new((x, y).into(), (WIDTH, h).into());
            n.lines = lines;
            n.buttons = buttons;
            n.picture = None;
            y += h + GAP;
        }
        self.shelf.fonts = Some(cell.into_inner());
        if bottom {
            let dy = (area.loc.y + area.size.h - MARGIN) - (y - GAP);
            for n in self.notes.notes.iter_mut() {
                n.rect.loc.y += dy;
                for l in n.lines.iter_mut() {
                    l.1.loc.y += dy;
                }
                for b in n.buttons.iter_mut() {
                    b.1.loc.y += dy;
                }
            }
        }
        self.redraw_all();
    }

    /// An app sent a card: written down if new, or not written for an hour.
    fn notify_remember(&mut self, app: &str) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let apps = self.notes.apps.get_or_insert_with(read_apps);
        if apps.get(app).is_some_and(|t| now.saturating_sub(*t) < 3600) {
            return;
        }
        apps.insert(app.to_string(), now);
        // Off the frame: a small file, but a disk can be slow. The file is
        // read again first, so an app taken out of it by hand stays out.
        let app = app.to_string();
        std::thread::spawn(move || {
            let mut apps = read_apps();
            apps.insert(app, now);
            if let Err(e) = write_apps(&apps) {
                tracing::warn!("notify: can't keep the apps list: {e}");
            }
        });
    }

    /// desktop.toml changed: the new choices apply to cards from now on; the
    /// stack moves at once if its corner changed.
    pub fn notify_reload(&mut self) {
        let new = load();
        if new != self.notes.config {
            tracing::info!("notify: {new:?}");
            let moved = new.corner != self.notes.config.corner;
            self.notes.config = new;
            if moved {
                self.notify_layout();
            }
        }
    }

    fn notify_start_timer(&mut self) {
        if self.notes.timer_running {
            return;
        }
        self.notes.timer_running = true;
        let res = self.loop_handle.insert_source(Timer::from_duration(Duration::from_millis(250)), |_, _, state| {
            let now = Instant::now();
            let expired: Vec<(Origin, u32)> = state
                .notes
                .notes
                .iter()
                .filter(|n| n.expires.is_some_and(|e| e <= now))
                .map(|n| (n.origin, n.id))
                .collect();
            if !expired.is_empty() {
                state.notes.notes.retain(|n| !n.expires.is_some_and(|e| e <= now));
                for (o, id) in expired {
                    state.notify_tell_closed(o, id, EXPIRED);
                }
                state.notify_layout();
            }
            if state.notes.notes.is_empty() {
                state.notes.timer_running = false;
                TimeoutAction::Drop
            } else {
                TimeoutAction::ToDuration(Duration::from_millis(250))
            }
        });
        if res.is_err() {
            self.notes.timer_running = false;
        }
    }

    /// Pointer motion: over a card, every countdown pauses; off them, they resume.
    pub fn notify_hover(&mut self, pos: Point<f64, Logical>) {
        let over = self.notify_at(pos);
        if over == self.notes.paused {
            return;
        }
        self.notes.paused = over;
        let now = Instant::now();
        for n in self.notes.notes.iter_mut() {
            if over {
                if let Some(e) = n.expires.take() {
                    n.left = Some(e.saturating_duration_since(now));
                }
            } else if let Some(l) = n.left.take() {
                n.expires = Some(now + l.max(MIN_AFTER_PAUSE));
            }
        }
    }

    /// Is there a card under `pos`?
    pub fn notify_at(&self, pos: Point<f64, Logical>) -> bool {
        self.notes.notes.iter().any(|n| n.rect.to_f64().contains(pos))
    }

    /// A press on a card: a button runs its action, the card its default action;
    /// either way the card closes.
    pub fn notify_press(&mut self, pos: Point<f64, Logical>) {
        let Some(i) = self.notes.notes.iter().position(|n| n.rect.to_f64().contains(pos)) else { return };
        let n = self.notes.notes.remove(i);
        let key = n
            .buttons
            .iter()
            .find(|(_, r)| r.to_f64().contains(pos))
            .map(|(k, _)| k.clone())
            .or_else(|| n.actions.iter().any(|(k, _)| k == "default").then(|| "default".to_string()));
        if let Some(k) = key {
            tracing::info!("notify: #{} action {k:?}", n.id);
            self.notify_tell_action(n.origin, n.id, &k);
        }
        self.notify_tell_closed(n.origin, n.id, DISMISSED);
        self.notify_layout();
        // The cards moved up: the pointer may be over the next one, or none
        // (then the others' countdowns resume).
        self.notify_hover(pos);
    }

    /// The cards' elements for an output, front to back.
    pub fn notify_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        out: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Vec<SceneElement> {
        if self.notes.notes.is_empty() {
            return Vec::new();
        }
        let light = self.light_style();
        let look = vergence_look::shelf_look(light);
        let ink = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
        let accent = self.accent();
        let alarm = vergence_look::bar_look(light).alert;
        let frost = self.frost_program(renderer);
        let frost_look = self.panel_frost(scale);
        let solid = if frost.is_some() || self.reduce_transparency() {
            255
        } else {
            (self.shelf_opacity() * 255.0).round() as u8
        };
        let mut fonts = self.shelf.fonts.take().unwrap_or_else(vergence_text::Fonts::system);
        let (regular, semibold) = (
            fonts.face("sans-serif", 400, false),
            fonts.face("sans-serif", 600, false).or_else(|| fonts.face("sans-serif", 700, false)),
        );
        let mut els = Vec::new();
        for n in self.notes.notes.iter_mut() {
            if !n.rect.overlaps(out) {
                continue;
            }
            let r = n.rect;
            // Drawn again only when what it shows changed.
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            let rect = |r: &Rectangle<i32, Logical>| (r.loc.x, r.loc.y, r.size.w, r.size.h);
            for (t, lr, line) in &n.lines {
                (t, rect(lr), line).hash(&mut hasher);
            }
            for (k, br) in &n.buttons {
                (k, rect(br)).hash(&mut hasher);
            }
            (&n.actions, n.urgency, rect(&r), light, frost.is_some(), solid).hash(&mut hasher);
            (scale.to_bits(), accent.r, accent.g, accent.b).hash(&mut hasher);
            let key = hasher.finish();
            if n.picture.as_ref().map(|(k, _)| *k) != Some(key) {
                let (w, h) = (r.size.w as f32, r.size.h as f32);
                let mut scene = Scene::new(w, h);
                let card = SceneRect::new(0.0, 0.0, w, h);
                if frost.is_none() {
                    scene.fill_rounded(
                        card,
                        vergence_look::RADIUS,
                        Color::rgba(look.menu.0, look.menu.1, look.menu.2, solid),
                    );
                }
                // The stripe down the left edge, inside the card's rounded corners.
                scene.push_clip(card, vergence_look::RADIUS);
                let stripe = if n.urgency >= 2 { Color::rgb(alarm.0, alarm.1, alarm.2) } else { accent };
                scene.fill(SceneRect::new(0.0, 0.0, STRIPE as f32, h), stripe);
                scene.pop_clip();
                scene.stroke(card, vergence_look::RADIUS, vergence_look::EDGE, ink(look.edge));
                for (t, lr, line) in &n.lines {
                    let (face, size, color) = match line {
                        Line::App => (regular, APP_SIZE, ink(look.menu_ink_off)),
                        Line::Title => (semibold, TITLE_SIZE, ink(look.menu_ink)),
                        Line::Body => (regular, BODY_SIZE, ink(look.menu_ink)),
                    };
                    let Some(face) = face else { continue };
                    let m = fonts.metrics(face, size);
                    let (lx, ly) = ((lr.loc.x - r.loc.x) as f32, (lr.loc.y - r.loc.y) as f32);
                    let base = (ly + (lr.size.h as f32 - (m.ascent + m.descent)) / 2.0 + m.ascent).round();
                    vergence_render::text(&mut scene, &mut fonts, face, size, color, ScenePoint::new(lx, base), t);
                }
                for (k, br) in &n.buttons {
                    let label = n.actions.iter().find(|(key, _)| key == k).map(|(_, l)| l.clone()).unwrap_or_default();
                    let b = SceneRect::new(
                        (br.loc.x - r.loc.x) as f32,
                        (br.loc.y - r.loc.y) as f32,
                        br.size.w as f32,
                        br.size.h as f32,
                    );
                    scene.fill_rounded(b, vergence_look::RADIUS, ink(look.hover));
                    if let Some(face) = regular {
                        let m = fonts.metrics(face, BODY_SIZE);
                        let lw = fonts.layout(face, BODY_SIZE, &label).1;
                        let base = (b.y + (b.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round();
                        vergence_render::text(
                            &mut scene,
                            &mut fonts,
                            face,
                            BODY_SIZE,
                            ink(look.menu_ink),
                            ScenePoint::new(b.x + (b.w - lw) / 2.0, base),
                            &label,
                        );
                    }
                }
                match vergence_render::Cpu::new().render(&scene, &mut fonts, scale as f32, Color::rgba(0, 0, 0, 0)) {
                    Ok(p) => {
                        let buffer = MemoryRenderBuffer::from_slice(
                            &p.data,
                            Fourcc::Abgr8888,
                            (p.width as i32, p.height as i32),
                            1,
                            Transform::Normal,
                            None,
                        );
                        n.picture = Some((key, buffer));
                    }
                    Err(e) => {
                        tracing::warn!("notify: a card could not be drawn: {e}");
                        continue;
                    }
                }
            }
            let Some((_, buffer)) = n.picture.as_ref() else { continue };
            let a = (r.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            let b = (r.loc + r.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
            if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                a.to_f64(),
                buffer,
                None,
                Some(src),
                Some(r.size),
                Kind::Unspecified,
            ) {
                els.push(SceneElement::Memory(el));
            }
            // Behind it, the frost (as under a Dockworker's card).
            if let Some(program) = frost.clone() {
                let dst = Rectangle::<i32, smithay::utils::Physical>::new(a, (b.x - a.x, b.y - a.y).into());
                if let Some(e) = crate::backdrop::element(renderer, &mut n.frost, dst, program, &frost_look) {
                    els.push(SceneElement::Backdrop(e));
                }
            }
        }
        self.shelf.fonts = Some(fonts);
        els
    }

    /// Settings changed: cards follow light or dark.
    pub fn notify_restyle(&mut self) {
        if !self.notes.notes.is_empty() {
            self.notify_layout();
        }
    }

    /// For `vctl notes`.
    pub fn notify_report(&self) -> String {
        if self.notes.notes.is_empty() {
            return "no notifications".into();
        }
        self.notes
            .notes
            .iter()
            .map(|n| {
                let left = match (n.expires, n.left) {
                    (Some(e), _) => format!("{:.1} s left", e.saturating_duration_since(Instant::now()).as_secs_f64()),
                    (None, Some(l)) => format!("paused, {:.1} s left", l.as_secs_f64()),
                    _ => "stays".into(),
                };
                format!(
                    "#{} {:?} {} {:?} {:?} urgency {} actions {:?} {left} at {:?}",
                    n.id, n.origin, n.app, n.summary, n.body, n.urgency, n.actions, n.rect
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn notifications_table() {
        use super::{Config, parse};
        use std::time::Duration;
        assert_eq!(parse(""), Config::default());
        let c = parse("[notifications]\ndo-not-disturb = true\nseconds = 20\ncorner = \"bottom-left\"\n");
        assert!(c.do_not_disturb);
        assert_eq!(c.timeout, Duration::from_secs(20));
        assert_eq!(c.corner, super::Place::BottomLeft);
        assert_eq!(parse("[notifications]\ncorner = \"top-center\"\n").corner, super::Place::TopCenter);
        assert_eq!(parse("[notifications]\nseconds = 0\ncorner = \"middle\"\n").timeout, Duration::from_secs(2));
        assert_eq!(parse("[notifications]\ncorner = \"middle\"\n").corner, super::Place::TopRight);
        assert_eq!(
            parse("[notifications]\noff = [\" Firefox \", \"notify-send\"]\n").off,
            vec!["firefox", "notify-send"]
        );
        let apps = super::parse_apps("[apps]\nFirefox = 1731629000\n\"vergence-settings\" = 1731629100\n");
        assert_eq!(apps.get("Firefox"), Some(&1731629000));
        assert_eq!(apps.len(), 2);
        assert!(super::parse_apps("nonsense =").is_empty());
    }

    fn w(s: &str) -> f32 {
        s.chars().count() as f32
    }

    #[test]
    fn wrapping() {
        assert_eq!(wrap("one two three four", 9.0, 4, &w), ["one two", "three", "four"]);
        assert_eq!(wrap("a\nb", 10.0, 4, &w), ["a", "b"]);
        assert_eq!(wrap("", 10.0, 4, &w), Vec::<String>::new());
        // Too many lines: the last is cut with an ellipsis.
        assert_eq!(wrap("aa bb cc dd ee", 5.0, 2, &w), ["aa bb", "cc d\u{2026}"]);
        // A word longer than the line stays whole on its own line.
        assert_eq!(wrap("abcdefghijk x", 5.0, 4, &w), ["abcdefghijk", "x"]);
    }
}
