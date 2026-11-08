// Notifications, drawn by the server (whitepaper §6.8; bus milestone 1 step 3,
// 2026-11-08).
//
// One service in the core, two front ends: the session bus's Notify message
// (bus.rs) and org.freedesktop.Notifications on the D-Bus edge (dbus.rs). A
// notification belongs to the connection that made it: only that connection
// can replace or close it, and only it hears that it closed or that an action
// was chosen (unicast, never broadcast).
//
// Cards stack down from the top-right corner of the monitor the pointer was on
// when the first of them appeared, sized in points like the menu and title
// bars, so they are the same physical size on every monitor; text is
// rasterised at each monitor's own pixels. Light or dark follows the
// color-scheme setting. A click on a card runs its "default" action (if it has
// one) and closes it; other actions are buttons under the text. Cards expire
// after their timeout (5 s when the app leaves it to the server; critical ones
// stay until clicked).

use std::collections::HashMap;
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
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::{Logical, Point, Rectangle, Transform},
};

use crate::{Server, decor::SceneElement};

const WIDTH: i32 = 340;
const PAD: i32 = 10;
const GAP: i32 = 8;
const MARGIN: i32 = 12;
const MAX_BODY_LINES: usize = 4;
const MAX_SHOWN: usize = 5;
const BUTTON_H: i32 = 24;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// Laid out: the card, its text lines and its buttons (logical).
    rect: Rectangle<i32, Logical>,
    lines: Vec<(String, Rectangle<i32, Logical>, bool)>,
    buttons: Vec<(String, Rectangle<i32, Logical>)>,
    ids: Vec<Id>,
    commit: CommitCounter,
    rasters: HashMap<(usize, u64), MemoryRenderBuffer>,
}

#[derive(Default)]
pub struct Notifications {
    notes: Vec<Note>,
    next_id: u32,
    /// The monitor the stack is on (its top-left), while there is a stack.
    anchor: Option<Rectangle<i32, Logical>>,
    timer_running: bool,
    /// A press on a card: its button's release is swallowed.
    pub swallow: Option<u32>,
}

struct Palette {
    bg: [f32; 4],
    edge: [f32; 4],
    critical: [f32; 4],
    button: [f32; 4],
    title: [f32; 3],
    text: [f32; 3],
}

const DARK: Palette = Palette {
    bg: [0.13, 0.14, 0.17, 0.97],
    edge: [0.30, 0.32, 0.37, 1.0],
    critical: [0.80, 0.25, 0.22, 1.0],
    button: [0.22, 0.24, 0.29, 1.0],
    title: [0.95, 0.95, 0.95],
    text: [0.78, 0.79, 0.82],
};

const LIGHT: Palette = Palette {
    bg: [0.97, 0.97, 0.98, 0.98],
    edge: [0.72, 0.74, 0.78, 1.0],
    critical: [0.80, 0.25, 0.22, 1.0],
    button: [0.88, 0.89, 0.91, 1.0],
    title: [0.08, 0.08, 0.10],
    text: [0.25, 0.26, 0.29],
};

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
        let timeout = match req.timeout_ms {
            t if t < 0 => (req.urgency < 2).then_some(DEFAULT_TIMEOUT),
            0 => None,
            t => Some(Duration::from_millis(t as u64)),
        };
        let expires = timeout.map(|t| Instant::now() + t);
        let replacing = (req.replaces != 0).then(|| self.notes.notes.iter().position(|n| n.id == req.replaces && n.origin == origin)).flatten();
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
            rect: Rectangle::default(),
            lines: Vec::new(),
            buttons: Vec::new(),
            ids: Vec::new(),
            commit: CommitCounter::default(),
            rasters: HashMap::new(),
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
        }
    }

    fn notify_tell_action(&mut self, origin: Origin, id: u32, key: &str) {
        match origin {
            Origin::DBus(conn) => self.dbus_action_invoked(conn, id, key),
            Origin::Bus(client) => self.bus_action_invoked(client, id, key),
        }
    }

    /// Place the cards (after any change).
    fn notify_layout(&mut self) {
        if self.notes.notes.is_empty() {
            self.notes.anchor = None;
            self.redraw_all();
            return;
        }
        // Stay on the stack's monitor while it exists (and is still there).
        let outputs: Vec<Rectangle<i32, Logical>> = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).collect();
        let anchor = match self.notes.anchor {
            Some(a) if outputs.contains(&a) => a,
            _ => match self.output_at_pointer() {
                Some(o) => o,
                None => return,
            },
        };
        self.notes.anchor = Some(anchor);
        let font = self.title_font.as_ref();
        let line_h = font.map(|f| (f.points * 96.0 / 72.0 * 1.45).round() as i32).unwrap_or(18);
        let width = |s: &str| font.map(|f| f.width(s)).unwrap_or(s.chars().count() as f32 * 7.0);
        let text_w = (WIDTH - 2 * PAD) as f32;
        let x = anchor.loc.x + anchor.size.w - WIDTH - MARGIN;
        let mut y = anchor.loc.y + MARGIN;
        for n in self.notes.notes.iter_mut() {
            let mut lines = Vec::new();
            let mut ty = y + PAD;
            let title = if n.summary.is_empty() { n.app.clone() } else { n.summary.clone() };
            for t in wrap(&title, text_w, 2, &width) {
                lines.push((t, Rectangle::new((x + PAD, ty).into(), (WIDTH - 2 * PAD, line_h).into()), true));
                ty += line_h;
            }
            for t in wrap(&n.body, text_w, MAX_BODY_LINES, &width) {
                lines.push((t, Rectangle::new((x + PAD, ty).into(), (WIDTH - 2 * PAD, line_h).into()), false));
                ty += line_h;
            }
            let mut buttons = Vec::new();
            let shown: Vec<&(String, String)> = n.actions.iter().filter(|(k, _)| k != "default").collect();
            if !shown.is_empty() {
                ty += 4;
                let mut bx = x + PAD;
                for (key, label) in shown {
                    let w = (width(label).ceil() as i32 + 20).min(WIDTH - 2 * PAD);
                    if bx + w > x + WIDTH - PAD {
                        break;
                    }
                    buttons.push((key.clone(), Rectangle::new((bx, ty).into(), (w, BUTTON_H).into())));
                    bx += w + 6;
                }
                ty += BUTTON_H;
            }
            let h = ty + PAD - y;
            n.rect = Rectangle::new((x, y).into(), (WIDTH, h).into());
            n.lines = lines;
            n.buttons = buttons;
            n.ids = (0..3 + n.buttons.len()).map(|_| Id::new()).collect();
            n.rasters.clear();
            n.commit.increment();
            y += h + GAP;
        }
        self.redraw_all();
    }

    fn notify_start_timer(&mut self) {
        if self.notes.timer_running {
            return;
        }
        self.notes.timer_running = true;
        let res = self.loop_handle.insert_source(Timer::from_duration(Duration::from_millis(250)), |_, _, state| {
            let now = Instant::now();
            let expired: Vec<(Origin, u32)> =
                state.notes.notes.iter().filter(|n| n.expires.is_some_and(|e| e <= now)).map(|n| (n.origin, n.id)).collect();
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
    }

    /// The cards' elements for an output, front to back.
    pub fn notify_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        if self.notes.notes.is_empty() {
            return Vec::new();
        }
        let light = self.settings.current.get(&(crate::settings::APPEARANCE.to_string(), "color-scheme".to_string()))
            == Some(&vergence_protocol::bus::SettingValue::U32(2));
        let pal = if light { &LIGHT } else { &DARK };
        let phys = |r: Rectangle<i32, Logical>| {
            let a = (r.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            let b = (r.loc + r.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            Rectangle::new(a, (b.x - a.x, b.y - a.y).into())
        };
        let font = self.title_font.as_ref();
        let mut els = Vec::new();
        for n in self.notes.notes.iter_mut() {
            if !n.rect.overlaps(out) {
                continue;
            }
            // Text and button labels (front).
            let mut texts: Vec<(String, Rectangle<i32, Logical>, [f32; 3])> =
                n.lines.iter().map(|(t, r, title)| (t.clone(), *r, if *title { pal.title } else { pal.text })).collect();
            for (key, r) in &n.buttons {
                let label = n.actions.iter().find(|(k, _)| k == key).map(|(_, l)| l.clone()).unwrap_or_default();
                let inner = Rectangle::new((r.loc.x + 10, r.loc.y).into(), (r.size.w - 20, r.size.h).into());
                texts.push((label, inner, pal.title));
            }
            if let Some(font) = font {
                for (i, (t, r, color)) in texts.iter().enumerate() {
                    let (pw, ph) = ((r.size.w as f64 * scale).round() as i32, (r.size.h as f64 * scale).round() as i32);
                    if pw <= 0 || ph <= 0 {
                        continue;
                    }
                    let buf = n.rasters.entry((i, scale.to_bits())).or_insert_with(|| {
                        let rgba = font.render(t, pw as usize, ph as usize, scale, *color);
                        MemoryRenderBuffer::from_slice(&rgba, Fourcc::Abgr8888, (pw, ph), 1, Transform::Normal, None)
                    });
                    let at = (r.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>().to_f64();
                    let src = Rectangle::<f64, Logical>::from_size((pw as f64, ph as f64).into());
                    if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(renderer, at, buf, None, Some(src), Some(r.size), Kind::Unspecified) {
                        els.push(SceneElement::Memory(el));
                    }
                }
            }
            // Buttons, background, edge (back).
            for (i, (_, r)) in n.buttons.iter().enumerate() {
                els.push(SceneElement::Solid(SolidColorRenderElement::new(n.ids[3 + i].clone(), phys(*r), n.commit, pal.button, Kind::Unspecified)));
            }
            let r = n.rect;
            let edge = if n.urgency >= 2 { pal.critical } else { pal.edge };
            let inner = Rectangle::new((r.loc.x + 1, r.loc.y + 1).into(), (r.size.w - 2, r.size.h - 2).into());
            els.push(SceneElement::Solid(SolidColorRenderElement::new(n.ids[0].clone(), phys(inner), n.commit, pal.bg, Kind::Unspecified)));
            els.push(SceneElement::Solid(SolidColorRenderElement::new(n.ids[1].clone(), phys(r), n.commit, edge, Kind::Unspecified)));
        }
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
                let left = n.expires.map(|e| format!("{:.1} s left", e.saturating_duration_since(Instant::now()).as_secs_f64())).unwrap_or_else(|| "stays".into());
                format!("#{} {:?} {} {:?} {:?} urgency {} actions {:?} {left} at {:?}", n.id, n.origin, n.app, n.summary, n.body, n.urgency, n.actions, n.rect)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::wrap;

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
