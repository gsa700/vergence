// The server's model of meaning (Contract 3; meaning milestone 1, step 2):
// every native window's tree with its place on the screens, found by path or
// by role and name, the focused widget and the caret, and a log of what
// changed (the focus, the caret, a tree, the window with the keyboard) for
// readers to follow: the agent interface (step 3) and the magnifier (step 7).
//
// A path names a window, then widgets below it, " > " between them:
//   "Calculator > 7", "Windows > switch:Minimize", "* > button:Save".
// The first part is a window's title or application id ("*": any window);
// each later part is a name, "role:name" or "role:" (any name), matched
// anywhere below the previous part's match, case not minded. The last
// part's matches are the answer.

use std::collections::VecDeque;

use vergence_protocol::window::Node;

use crate::{Server, win::Win};

/// A rectangle: x, y, width, height (a caret, a widget on the screens).
pub type Rect4 = (f64, f64, f64, f64);
/// A window's meaning as sent: the focused widget, the caret in window
/// points, the tree.
pub type WindowMeaning = (u64, Option<Rect4>, Vec<Node>);

/// A native window with its meaning, for searching.
struct Listed {
    win: Win,
    index: Option<usize>,
    title: String,
    app_id: String,
    origin: (f64, f64),
    meaning: WindowMeaning,
}

/// How many changes the log keeps.
const LOG_KEPT: usize = 1024;
/// The most widgets one search returns.
const FOUND_MAX: usize = 200;

/// What changed, for readers to follow.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// The keyboard went to this native window (or to none of them).
    Window { client: u32, window: u32 },
    /// The focused widget in a window moved (0: none).
    Focus { client: u32, window: u32, node: u64 },
    /// A window's text caret moved, on the screens (logical points).
    Caret { client: u32, window: u32, at: Option<Rect4> },
    /// A window's tree changed (widgets, values, states).
    Tree { client: u32, window: u32 },
    /// An agent's timed grant for an app is running out, or has (0): only
    /// that agent hears it (agents.rs).
    Time { agent: crate::agents::Who, app: String, minutes_left: u32 },
}

#[derive(Default)]
pub struct MeaningLog {
    seq: u64,
    changes: VecDeque<(u64, Change)>,
}

impl MeaningLog {
    pub fn push(&mut self, c: Change) {
        self.seq += 1;
        self.changes.push_back((self.seq, c));
        while self.changes.len() > LOG_KEPT {
            self.changes.pop_front();
        }
    }

    /// The changes after `since`, and the latest number.
    pub fn since(&self, since: u64) -> (Vec<(u64, Change)>, u64) {
        (self.changes.iter().filter(|(n, _)| *n > since).cloned().collect(), self.seq)
    }
}

/// A widget found: its window (index as `vctl windows` counts), the node,
/// and where it is on the screens (logical points) and on which screen.
#[derive(Debug, Clone)]
pub struct Found {
    pub index: Option<usize>,
    pub client: u32,
    pub window: u32,
    pub title: String,
    pub app_id: String,
    pub node: Node,
    pub at: Rect4,
    pub screen: Option<String>,
}

/// One part of a path: a name, a role and a name, or a role alone.
#[derive(Debug, Clone, PartialEq)]
struct Part {
    role: Option<String>,
    name: Option<String>,
}

fn part(s: &str) -> Part {
    let s = s.trim();
    if s == "*" {
        return Part { role: None, name: None };
    }
    match s.split_once(':') {
        Some((r, n)) if !r.contains(' ') => Part { role: Some(r.trim().to_lowercase()), name: (!n.trim().is_empty()).then(|| n.trim().to_lowercase()) },
        _ => Part { role: None, name: Some(s.to_lowercase()) },
    }
}

fn matches(p: &Part, n: &Node) -> bool {
    p.role.as_ref().is_none_or(|r| n.role.to_lowercase() == *r) && p.name.as_ref().is_none_or(|m| n.name.to_lowercase() == *m)
}

/// The path split into its window part and its widget parts.
fn parse(path: &str) -> (String, Vec<Part>) {
    let mut parts = path.split(" > ");
    let window = parts.next().unwrap_or("*").trim().to_lowercase();
    (window, parts.map(part).collect())
}

/// Whether a window part names this window.
fn window_matches(part: &str, title: &str, app_id: &str) -> bool {
    part == "*" || title.to_lowercase() == part || app_id.to_lowercase() == part
}

/// The nodes (by their place in `nodes`) that the widget parts lead to:
/// each part's matches below the previous part's (anywhere below, through
/// the parent links). No parts: every node.
/// The most steps one search may take (Fable's review: a crafted tree must
/// not hold the server); past it, what was found so far is the answer.
const FIND_BUDGET: usize = 2_000_000;

fn find_in(nodes: &[Node], parts: &[Part]) -> Vec<usize> {
    // Trees arrive repaired (native.rs sane_tree): ids unique, parents
    // without loops; still, each walk is bounded.
    let index: std::collections::HashMap<u64, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    let mut budget = FIND_BUDGET;
    let mut current: Option<std::collections::HashSet<u64>> = None;
    let mut last: Vec<usize> = Vec::new();
    for p in parts {
        let mut next = Vec::new();
        for (i, n) in nodes.iter().enumerate() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            if !matches(p, n) {
                continue;
            }
            let ok = match &current {
                None => true,
                Some(set) => {
                    let mut q = n.parent;
                    let mut found = false;
                    for _ in 0..64 {
                        if q == 0 || budget == 0 {
                            break;
                        }
                        budget -= 1;
                        if set.contains(&q) {
                            found = true;
                            break;
                        }
                        q = index.get(&q).map_or(0, |&k| nodes[k].parent);
                    }
                    found
                }
            };
            if ok {
                next.push(i);
            }
        }
        if next.is_empty() {
            return next;
        }
        current = Some(next.iter().map(|&i| nodes[i].id).collect());
        last = next;
    }
    if parts.is_empty() { (0..nodes.len()).collect() } else { last }
}

impl Server {
    /// A window's new meaning compared with its old: the log learns what
    /// changed. `origin` is the window's place on the screens.
    pub fn meaning_changed(&mut self, client: u32, window: u32, old: Option<&WindowMeaning>, new: &WindowMeaning, origin: (f64, f64)) {
        let (old_focus, old_caret, old_nodes) = match old {
            Some((f, c, n)) => (Some(*f), *c, Some(n)),
            None => (None, None, None),
        };
        if old_focus != Some(new.0) {
            self.meaning_log.push(Change::Focus { client, window, node: new.0 });
        }
        // The magnifier follows the window with the keyboard's focus and
        // caret (zoom.rs): the focused control's middle, then the caret's.
        let keyboard = self.input.keyboard_native.as_ref().and_then(|w| w.native()).is_some_and(|n| n.client == client && n.id == window);
        if keyboard
            && old_focus != Some(new.0)
            && let Some(n) = new.2.iter().find(|n| n.id == new.0)
        {
            self.zoom_follow("focus", (origin.0 + n.x + n.w / 2.0, origin.1 + n.y + n.h / 2.0));
        }
        if old_caret != new.1 {
            let at = new.1.map(|(x, y, w, h)| (origin.0 + x, origin.1 + y, w, h));
            if keyboard && let Some((x, y, w, h)) = at {
                self.zoom_follow("caret", (x + w / 2.0, y + h / 2.0));
            }
            self.meaning_log.push(Change::Caret { client, window, at });
        }
        if old_nodes != Some(&new.2) {
            self.meaning_log.push(Change::Tree { client, window });
        }
    }

    /// Every native window with its meaning: the window, its index, title,
    /// app id, place on the screens, focus, caret and tree.
    fn meaning_windows(&self) -> Vec<Listed> {
        let mut out = Vec::new();
        for (i, w) in self.space.elements().enumerate() {
            let Some(n) = w.native() else { continue };
            let Some(meaning) = n.meaning() else { continue };
            let loc = self.space.element_location(w).unwrap_or_default();
            out.push(Listed { win: w.clone(), index: Some(i), title: n.title(), app_id: n.app_id.clone(), origin: (loc.x as f64, loc.y as f64), meaning });
        }
        out
    }

    fn found(&self, w: &Win, index: Option<usize>, title: &str, app_id: &str, origin: (f64, f64), node: &Node) -> Found {
        let at = (origin.0 + node.x, origin.1 + node.y, node.w, node.h);
        let (cx, cy) = (at.0 + at.2 / 2.0, at.1 + at.3 / 2.0);
        let screen = self
            .space
            .outputs()
            .find(|o| self.space.output_geometry(o).is_some_and(|g| cx >= g.loc.x as f64 && cy >= g.loc.y as f64 && cx < (g.loc.x + g.size.w) as f64 && cy < (g.loc.y + g.size.h) as f64))
            .map(|o| o.name());
        let n = w.native();
        Found { index, client: n.map_or(0, |n| n.client), window: n.map_or(0, |n| n.id), title: title.into(), app_id: app_id.into(), node: node.clone(), at, screen }
    }

    /// The widgets a path leads to, in every window it names (see the top).
    pub fn meaning_find(&self, path: &str) -> Vec<Found> {
        self.meaning_find_in(path, None)
    }

    /// As meaning_find, only in the windows `only` names (client, window),
    /// if given: an agent searches only what it may use, before any work.
    fn meaning_find_in(&self, path: &str, only: Option<&std::collections::HashSet<(u32, u32)>>) -> Vec<Found> {
        let (wpart, parts) = parse(path);
        let mut out = Vec::new();
        for l in self.meaning_windows() {
            if !window_matches(&wpart, &l.title, &l.app_id) {
                continue;
            }
            if let (Some(set), Some(n)) = (only, l.win.native())
                && !set.contains(&(n.client, n.id))
            {
                continue;
            }
            let nodes = &l.meaning.2;
            for i in find_in(nodes, &parts) {
                out.push(self.found(&l.win, l.index, &l.title, &l.app_id, l.origin, &nodes[i]));
                if out.len() >= FOUND_MAX {
                    return out;
                }
            }
        }
        out
    }

    /// The focused widget of the window with the keyboard, and its caret on
    /// the screens, if it is a native window with a meaning.
    pub fn meaning_focus(&self) -> Option<(Found, Option<Rect4>)> {
        let k = self.input.keyboard_native.as_ref()?;
        let l = self.meaning_windows().into_iter().find(|l| &l.win == k)?;
        let (focus, caret, nodes) = &l.meaning;
        let node = nodes.iter().find(|n| n.id == *focus)?;
        let caret = caret.map(|(x, y, cw, ch)| (l.origin.0 + x, l.origin.1 + y, cw, ch));
        Some((self.found(&l.win, l.index, &l.title, &l.app_id, l.origin, node), caret))
    }

    /// For `vctl find`: one line a widget.
    pub fn meaning_find_report(&self, path: &str) -> String {
        let found = self.meaning_find(path);
        if found.is_empty() {
            return format!("nothing at {path:?}");
        }
        found.iter().map(found_line).collect::<Vec<_>>().join("\n")
    }

    /// For `vctl focused`.
    pub fn meaning_focus_report(&self) -> String {
        match self.meaning_focus() {
            None => "no focused widget in a native window".into(),
            Some((f, caret)) => {
                let caret = caret.map(|(x, y, w, h)| format!("\n  caret at {x:.0},{y:.0} {w:.0}x{h:.0}")).unwrap_or_default();
                format!("{}{caret}", found_line(&f))
            }
        }
    }

    /// For `vctl changes [since]`.
    pub fn meaning_changes_report(&self, since: u64) -> String {
        let (changes, latest) = self.meaning_log.since(since);
        let mut out: Vec<String> = changes.iter().map(|(n, c)| format!("{n} {c:?}")).collect();
        out.push(format!("latest {latest}"));
        out.join("\n")
    }
}

/// Whether agents may use the interface: Privacy > Agents' switch (on by
/// default; every app is still asked about first, agents.rs).
pub fn agents_allowed() -> bool {
    crate::agents::agents_settings().0
}

/// How much of a reply's frame its list may fill (frames are at most 64 KB).
const REPLY_BUDGET: usize = 60 * 1024;

/// A node's share of a frame, roughly (ids, bounds, its strings).
fn node_size(n: &Node) -> usize {
    64 + n.role.len() + n.name.len() + n.value.as_ref().map_or(0, String::len) + n.states.iter().map(|s| s.len() + 4).sum::<usize>() + n.items.iter().map(|s| s.len() + 4).sum::<usize>()
}

/// A node moved onto the screens.
fn on_screens(n: &Node, origin: (f64, f64)) -> Node {
    let mut n = n.clone();
    n.x += origin.0;
    n.y += origin.1;
    n
}

impl Server {
    /// The screen a point is on (its name).
    fn screen_name_at(&self, x: f64, y: f64) -> String {
        self.space
            .outputs()
            .find(|o| self.space.output_geometry(o).is_some_and(|g| x >= g.loc.x as f64 && y >= g.loc.y as f64 && x < (g.loc.x + g.size.w) as f64 && y < (g.loc.y + g.size.h) as f64))
            .map(|o| o.name())
            .unwrap_or_default()
    }

    /// A window's kind and application id, whatever kind it is.
    fn window_kind_app(&self, w: &Win) -> (&'static str, String) {
        if let Some(n) = w.native() {
            return ("native", n.app_id.clone());
        }
        if let Some(x) = w.x11_surface() {
            return ("x11", x.class());
        }
        let app = w
            .toplevel()
            .map(|t| {
                smithay::wayland::compositor::with_states(t.wl_surface(), |s| {
                    s.data_map.get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>().and_then(|d| d.lock().unwrap().app_id.clone()).unwrap_or_default()
                })
            })
            .unwrap_or_default();
        ("wayland", app)
    }

    fn agent_found(&self, f: &Found) -> vergence_protocol::agent::AgentFound {
        let mut node = f.node.clone();
        (node.x, node.y) = (f.at.0, f.at.1);
        vergence_protocol::agent::AgentFound { client: f.client, window: f.window, title: f.title.clone(), node, screen: f.screen.clone().unwrap_or_default() }
    }

    /// An app as grants know it: its id and the program the server saw run
    /// it (Fable's review: an app id alone is any program's word).
    pub fn agent_app_key(&self, app_client: u32, app_id: &str) -> String {
        let program = self.bus_pid(app_client).and_then(vergence_dm::elevate::program_path).map(|p| p.display().to_string()).unwrap_or_default();
        format!("{app_id} @ {program}")
    }

    /// The native windows an agent may read and use now, asking nothing.
    fn agent_windows(&mut self, id: u32) -> std::collections::HashSet<(u32, u32)> {
        let keys: Vec<((u32, u32), String)> = self.natives.keys().filter_map(|k| self.natives.get(k).and_then(|w| w.native()).map(|n| (*k, self.agent_app_key(n.client, &n.app_id)))).collect();
        keys.into_iter().filter(|(_, key)| self.agent_may_quietly(id, key)).map(|(k, _)| k).collect()
    }

    /// An agent's request (it has said who it is; bus.rs checks).
    pub fn agent_request(&mut self, id: u32, r: vergence_protocol::agent::AgentRequest) {
        use vergence_protocol::agent::{AgentChange, AgentEvent, AgentRequest, AgentWindow, MAX_LISTED};
        use vergence_protocol::bus::{ErrorCode, ServerMsg};
        let reply = |s: &mut Server, e: AgentEvent| s.bus_send(id, ServerMsg::Agent(e));
        let refuse = |s: &mut Server, serial: u32, code: ErrorCode, message: String| s.bus_send(id, ServerMsg::Error { serial, code, message });
        // Permission (agents.rs): the app a request is about must be
        // allowed; the person is asked first if nothing says yet, and the
        // request waits for the answer.
        let mut apps: Vec<String> = Vec::new();
        let key_of = |s: &Server, w: &Win| w.native().map(|n| s.agent_app_key(n.client, &n.app_id));
        match &r {
            AgentRequest::Tree { client, window, .. } | AgentRequest::Act { client, window, .. } => {
                if let Some(w) = self.natives.get(&(*client, *window)).cloned() {
                    apps.extend(key_of(self, &w));
                }
            }
            AgentRequest::Focused { .. } => {
                if let Some(w) = self.input.keyboard_native.clone() {
                    apps.extend(key_of(self, &w));
                }
            }
            // A search in named windows asks about their apps; one across
            // every window ("*") only shows the apps already allowed.
            AgentRequest::Find { path, .. } => {
                let (wpart, _) = parse(path);
                if wpart != "*" {
                    for l in self.meaning_windows() {
                        if window_matches(&wpart, &l.title, &l.app_id)
                            && let Some(k) = key_of(self, &l.win)
                            && !apps.contains(&k)
                        {
                            apps.push(k);
                        }
                    }
                }
            }
            _ => {}
        }
        for app in &apps {
            let shown = self.app_shown_name(app);
            match self.agent_gate(id, app, &shown, &r) {
                None => return,
                Some(true) => {}
                Some(false) if matches!(r, AgentRequest::Find { .. }) => {}
                Some(false) => {
                    self.agent_log(id, app, "refused: not allowed");
                    let name = self.bus_agent_name(id).unwrap_or_default();
                    return refuse(self, r.serial(), ErrorCode::Denied, format!("{name} may not use {shown}"));
                }
            }
        }
        match r {
            AgentRequest::Hello { .. } => {}
            AgentRequest::Windows { serial } => {
                let keyboard = self.input.keyboard_native.clone();
                let mut windows = Vec::new();
                let wins: Vec<Win> = self.space.elements().take(MAX_LISTED).cloned().collect();
                for w in &wins {
                    let g = self.space.element_geometry(w).unwrap_or_default();
                    let (kind, app_id) = self.window_kind_app(w);
                    let (client, window, meaning) = w.native().map_or((0, 0, false), |n| (n.client, n.id, n.meaning().is_some()));
                    let at = (g.loc.x as f64, g.loc.y as f64, g.size.w as f64, g.size.h as f64);
                    let screen = self.screen_name_at(at.0 + at.2 / 2.0, at.1 + at.3 / 2.0);
                    let key = w.native().map(|n| self.agent_app_key(n.client, &n.app_id)).unwrap_or_default();
                    let (allowed, minutes_left) = self.agent_grant_for(id, &key);
                    // A window's title only where the agent may read its app
                    // (Fable's review: titles are content); else its app's name.
                    let may = !allowed.is_empty() && allowed != "never";
                    let title = if may { self.window_title(w) } else { self.app_shown_name(&app_id) };
                    windows.push(AgentWindow { client, window, title, app_id, kind: kind.into(), keyboard: keyboard.as_ref() == Some(w), meaning, at, screen, allowed, minutes_left });
                }
                reply(self, AgentEvent::Windows { serial, windows });
            }
            AgentRequest::Tree { serial, client, window } => {
                let Some(w) = self.natives.get(&(client, window)).cloned() else {
                    return refuse(self, serial, ErrorCode::Failed, format!("no native window {client}/{window}"));
                };
                let Some(n) = w.native() else { return };
                let Some((focus, caret, nodes)) = n.meaning() else {
                    return refuse(self, serial, ErrorCode::Failed, "that window has sent no meaning".into());
                };
                let loc = self.space.element_location(&w).unwrap_or_default();
                let origin = (loc.x as f64, loc.y as f64);
                let mut used = 256;
                let nodes: Vec<Node> = nodes.iter().map(|m| on_screens(m, origin)).take_while(|m| (used + node_size(m) <= REPLY_BUDGET).then(|| used += node_size(m)).is_some()).collect();
                let caret = caret.map(|(x, y, cw, ch)| (origin.0 + x, origin.1 + y, cw, ch));
                reply(self, AgentEvent::Tree { serial, client, window, title: n.title(), focus, caret, nodes });
            }
            AgentRequest::Find { serial, path } => {
                let only = self.agent_windows(id);
                let allowed = self.meaning_find_in(&path, Some(&only));
                let mut used = 256;
                let found: Vec<_> = allowed.iter().map(|f| self.agent_found(f)).take(MAX_LISTED).take_while(|f| (used + node_size(&f.node) + f.title.len() + 32 <= REPLY_BUDGET).then(|| used += node_size(&f.node) + f.title.len() + 32).is_some()).collect();
                reply(self, AgentEvent::Found { serial, found });
            }
            AgentRequest::Focused { serial } => {
                let (found, caret) = match self.meaning_focus() {
                    Some((f, caret)) => (Some(Box::new(self.agent_found(&f))), caret),
                    None => (None, None),
                };
                reply(self, AgentEvent::Focused { serial, found, caret });
            }
            AgentRequest::Act { serial, client, window, node, action, role, name } => {
                let Some(w) = self.natives.get(&(client, window)).cloned() else {
                    return reply(self, AgentEvent::Acted { serial, ok: false, why: format!("no native window {client}/{window}") });
                };
                // A secret field is never set, whatever its program would do.
                let secret = w.native().and_then(|n| n.meaning()).is_some_and(|(_, _, nodes)| nodes.iter().any(|m| m.id == node && m.states.iter().any(|s| s == "secret")));
                if secret && matches!(action, vergence_protocol::window::Action::Set(_)) {
                    return reply(self, AgentEvent::Acted { serial, ok: false, why: "a secret field is not set by agents".into() });
                }
                // The widget must still be what the agent meant: ids are by
                // place, and a window that changed may have moved them.
                let current = w.native().and_then(|n| n.meaning()).and_then(|(_, _, nodes)| nodes.into_iter().find(|m| m.id == node));
                let Some(m) = current else {
                    return reply(self, AgentEvent::Acted { serial, ok: false, why: "no such widget now (the window changed)".into() });
                };
                if (!role.is_empty() && m.role != role) || (!name.is_empty() && m.name != name) {
                    return reply(self, AgentEvent::Acted { serial, ok: false, why: format!("the widget changed: it is now the {} {:?}", m.role, m.name) });
                }
                let app = w.native().map(|n| n.app_id.clone()).unwrap_or_default();
                self.agent_log(id, &app, &format!("{action:?} #{node} ({} {:?})", m.role, m.name));
                // A ring where it acts, for the person to see (step 5), kept
                // within the window.
                let g = self.space.element_geometry(&w).unwrap_or_default();
                let loc = self.space.element_location(&w).unwrap_or_default();
                let agent = self.bus_agent_name(id).unwrap_or_default();
                let win = (g.loc.x as f64, g.loc.y as f64, g.size.w as f64, g.size.h as f64);
                self.agent_ring(&agent, (loc.x as f64 + m.x, loc.y as f64 + m.y, m.w, m.h), win);
                if self.send_act(&w, node, action, Some((id, serial))).is_none() {
                    reply(self, AgentEvent::Acted { serial, ok: false, why: "not a native window".into() });
                }
            }
            AgentRequest::Changes { serial, since } => {
                let (changes, latest) = self.meaning_log.since(since);
                let me = self.bus_agent_who(id).unwrap_or_default();
                // Only what happens in windows it may use (Fable's review:
                // focus and caret elsewhere would tell typing's rhythm).
                let mine = self.agent_windows(id);
                let seen = |c: u32, w: u32| mine.contains(&(c, w));
                let mut out = Vec::new();
                let mut upto = since;
                for (n, c) in changes.into_iter().take(MAX_LISTED) {
                    upto = n;
                    out.push((
                        n,
                        match c {
                            Change::Window { client, window } if seen(client, window) => AgentChange::Window { client, window },
                            Change::Window { .. } => AgentChange::Window { client: 0, window: 0 },
                            Change::Focus { client, window, node } if seen(client, window) => AgentChange::Focus { client, window, node },
                            Change::Caret { client, window, at } if seen(client, window) => AgentChange::Caret { client, window, at },
                            Change::Tree { client, window } if seen(client, window) => AgentChange::Tree { client, window },
                            Change::Focus { .. } | Change::Caret { .. } | Change::Tree { .. } => continue,
                            // Only its own grants' time, by the app's id.
                            Change::Time { agent, app, minutes_left } if agent == me => AgentChange::Time { app: app.split(" @ ").next().unwrap_or(&app).to_string(), minutes_left },
                            Change::Time { .. } => continue,
                        },
                    ));
                }
                // Cut short: the agent asks again from where this ends.
                let latest = if out.len() == MAX_LISTED { upto } else { latest };
                reply(self, AgentEvent::Changes { serial, latest, changes: out });
            }
        }
    }
}

fn found_line(f: &Found) -> String {
    let n = &f.node;
    let value = n.value.as_ref().map(|v| format!(" = {v:?}")).unwrap_or_default();
    let states = if n.states.is_empty() { String::new() } else { format!(" [{}]", n.states.join(", ")) };
    let items = if n.items.is_empty() { String::new() } else { format!(" items {:?}", n.items) };
    let index = f.index.map_or("-".to_string(), |i| i.to_string());
    let screen = f.screen.as_deref().unwrap_or("-");
    format!("window {index} {:?} #{} {} {:?}{value}{states}{items} at {:.0},{:.0} {:.0}x{:.0} on {screen}", f.title, n.id, n.role, n.name, f.at.0, f.at.1, f.at.2, f.at.3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u64, parent: u64, role: &str, name: &str) -> Node {
        Node { id, parent, role: role.into(), name: name.into(), value: None, states: vec![], x: 0.0, y: 0.0, w: 1.0, h: 1.0, items: vec![] }
    }

    /// A settings window: a page with a group holding a switch and a button,
    /// and a second group with another button of the same name.
    fn tree() -> Vec<Node> {
        vec![
            node(1, 0, "scroll area", "Page"),
            node(2, 1, "label", "Windows"),
            node(3, 1, "group", "Snapping"),
            node(4, 3, "switch", "Minimize"),
            node(5, 3, "button", "Reset"),
            node(6, 1, "group", "Keys"),
            node(7, 6, "button", "Reset"),
            node(8, 0, "label", "Minimize"),
        ]
    }

    #[test]
    fn paths_lead_down_the_tree() {
        let t = tree();
        let ids = |path: &str| -> Vec<u64> {
            let (_, parts) = parse(path);
            find_in(&t, &parts).into_iter().map(|i| t[i].id).collect()
        };
        assert_eq!(ids("W > Reset"), vec![5, 7], "a name anywhere");
        assert_eq!(ids("W > Snapping > Reset"), vec![5], "below a part");
        assert_eq!(ids("W > keys > reset"), vec![7], "case not minded");
        assert_eq!(ids("W > switch:Minimize"), vec![4], "a role picks the switch over the label");
        assert_eq!(ids("W > Minimize"), vec![4, 8]);
        assert_eq!(ids("W > button:"), vec![5, 7], "a role alone");
        assert_eq!(ids("W > Page > Snapping > switch:"), vec![4]);
        assert_eq!(ids("W > Keys > switch:Minimize"), Vec::<u64>::new(), "not below Keys");
        assert_eq!(ids("W > Nothing"), Vec::<u64>::new());
        assert_eq!(ids("W").len(), 8, "a window alone: all of it");
    }

    #[test]
    fn the_window_part() {
        assert!(window_matches("*", "Windows", "vergence-settings"));
        assert!(window_matches("windows", "Windows", "vergence-settings"));
        assert!(window_matches("vergence-settings", "Windows", "vergence-settings"));
        assert!(!window_matches("calculator", "Windows", "vergence-settings"));
        let (w, parts) = parse("Calculator > button:7");
        assert_eq!(w, "calculator");
        assert_eq!(parts, vec![Part { role: Some("button".into()), name: Some("7".into()) }]);
        // A name with a colon and a space stays a name ("Ratio: 4:3" is no role).
        assert_eq!(part("Ratio x: 4:3"), Part { role: None, name: Some("ratio x: 4:3".into()) });
    }

    #[test]
    fn the_log_keeps_the_latest() {
        let mut log = MeaningLog::default();
        for i in 0..(LOG_KEPT as u32 + 10) {
            log.push(Change::Tree { client: 1, window: i });
        }
        let (all, latest) = log.since(0);
        assert_eq!(latest, LOG_KEPT as u64 + 10);
        assert_eq!(all.len(), LOG_KEPT);
        let (some, _) = log.since(latest - 3);
        assert_eq!(some.len(), 3);
        assert!(log.since(latest).0.is_empty());
    }
}
