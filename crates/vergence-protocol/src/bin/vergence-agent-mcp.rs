// vergence-agent-mcp: the agent interface (meaning milestone 1, step 6) for
// any agent that speaks the Model Context Protocol: JSON-RPC 2.0, one
// message a line, on its standard input and output (the MCP "stdio"
// transport). Its tools read the windows' meaning and act on widgets by
// name; every app is asked about in the desktop's own question first, and
// the person sees a ring where the agent acts (agents.rs, the server's).
//
//   vergence-agent-mcp [--name NAME]     NAME is shown to the person ("Claude" by default)
//
// For Claude Code, in a Vergence session:
//   claude mcp add vergence -- vergence-agent-mcp --name Claude
// From another machine over SSH (the session's bus by its runtime directory):
//   claude mcp add vergence -- ssh HOST env XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 vergence-agent-mcp --name Claude
//
// Our own JSON code (no serde_json in the workspace). Node ids are 64-bit,
// past JSON numbers' exact range, so they travel as strings.

use std::io::{BufRead, Read, Write};
use std::time::{Duration, Instant};

use vergence_protocol::{
    Client,
    agent::{AgentChange, AgentEvent, AgentFound, AgentRequest},
    bus::{ClientMsg, ServerMsg},
    window::{Action, Node},
};

/// The protocol versions known, newest first.
const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// The longest line read, and how deep JSON may nest.
const LINE_MAX: usize = 1 << 20;
const DEPTH_MAX: usize = 64;
/// How long `act` waits for the window's tree to show what it did.
const SETTLE: Duration = Duration::from_millis(1500);
/// The longest `wait`.
const WAIT_MAX: u64 = 30_000;

// ------------------------------------------------------------------ JSON

#[derive(Debug, Clone, PartialEq)]
enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    fn get(&self, k: &str) -> Option<&J> {
        match self {
            J::Obj(v) => v.iter().find(|(n, _)| n == k).map(|(_, x)| x),
            _ => None,
        }
    }
    fn str(&self) -> Option<&str> {
        match self {
            J::Str(s) => Some(s),
            _ => None,
        }
    }
    fn num(&self) -> Option<f64> {
        match self {
            J::Num(n) => Some(*n),
            J::Str(s) => s.parse().ok(),
            _ => None,
        }
    }
}

fn obj(pairs: Vec<(&str, J)>) -> J {
    J::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn s(v: impl Into<String>) -> J {
    J::Str(v.into())
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Result<J, String> {
        if depth > DEPTH_MAX {
            return Err("nested too deep".into());
        }
        self.ws();
        match self.b.get(self.i) {
            None => Err("ended early".into()),
            Some(b'{') => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(J::Obj(v));
                }
                loop {
                    self.ws();
                    let k = match self.value(depth + 1)? {
                        J::Str(k) => k,
                        _ => return Err("a key is a string".into()),
                    };
                    self.ws();
                    if self.b.get(self.i) != Some(&b':') {
                        return Err("expected :".into());
                    }
                    self.i += 1;
                    v.push((k, self.value(depth + 1)?));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(J::Obj(v));
                        }
                        _ => return Err("expected , or }".into()),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(J::Arr(v));
                }
                loop {
                    v.push(self.value(depth + 1)?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(J::Arr(v));
                        }
                        _ => return Err("expected , or ]".into()),
                    }
                }
            }
            Some(b'"') => self.string().map(J::Str),
            Some(b't') if self.b[self.i..].starts_with(b"true") => {
                self.i += 4;
                Ok(J::Bool(true))
            }
            Some(b'f') if self.b[self.i..].starts_with(b"false") => {
                self.i += 5;
                Ok(J::Bool(false))
            }
            Some(b'n') if self.b[self.i..].starts_with(b"null") => {
                self.i += 4;
                Ok(J::Null)
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => {
                let start = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[start..self.i])
                    .ok()
                    .and_then(|t| t.parse().ok())
                    .map(J::Num)
                    .ok_or_else(|| "a bad number".into())
            }
            Some(_) => Err("unexpected character".into()),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.b.get(self.i..self.i + 4).ok_or("a short \\u")?;
        self.i += 4;
        u32::from_str_radix(std::str::from_utf8(h).map_err(|_| "a bad \\u")?, 16).map_err(|_| "a bad \\u".into())
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.b.len() && self.b[self.i] != b'"' && self.b[self.i] != b'\\' {
                if self.b[self.i] < 0x20 {
                    return Err("a control character in a string".into());
                }
                self.i += 1;
            }
            out.push_str(std::str::from_utf8(&self.b[start..self.i]).map_err(|_| "not UTF-8")?);
            match self.b.get(self.i) {
                None => return Err("a string not ended".into()),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => {
                    self.i += 1;
                    let c = *self.b.get(self.i).ok_or("a string not ended")?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut u = self.hex4()?;
                            // A surrogate pair.
                            if (0xD800..0xDC00).contains(&u) && self.b.get(self.i..self.i + 2) == Some(b"\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    u = 0x10000 + ((u - 0xD800) << 10) + (lo - 0xDC00);
                                }
                            }
                            out.push(char::from_u32(u).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err("a bad escape".into()),
                    }
                }
            }
        }
    }
}

fn parse(text: &str) -> Result<J, String> {
    let mut p = Parser { b: text.as_bytes(), i: 0 };
    let v = p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        return Err("more after the value".into());
    }
    Ok(v)
}

fn write_json(v: &J, out: &mut String) {
    match v {
        J::Null => out.push_str("null"),
        J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        J::Num(n) if n.is_finite() && n.fract() == 0.0 && n.abs() < 9.0e15 => out.push_str(&format!("{}", *n as i64)),
        J::Num(n) if n.is_finite() => out.push_str(&format!("{n}")),
        J::Num(_) => out.push_str("null"),
        J::Str(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        J::Arr(v) => {
            out.push('[');
            for (i, x) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(x, out);
            }
            out.push(']');
        }
        J::Obj(v) => {
            out.push('{');
            for (i, (k, x)) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(&J::Str(k.clone()), out);
                out.push(':');
                write_json(x, out);
            }
            out.push('}');
        }
    }
}

fn to_json(v: &J) -> String {
    let mut out = String::new();
    write_json(v, &mut out);
    out
}

// ------------------------------------------------------------------ tools

fn schema(props: Vec<(&str, J)>, required: &[&str]) -> J {
    obj(vec![
        ("type", s("object")),
        ("properties", obj(props)),
        ("required", J::Arr(required.iter().map(|r| s(*r)).collect())),
    ])
}

fn prop(kind: &str, about: &str) -> J {
    obj(vec![("type", s(kind)), ("description", s(about))])
}

const PATHS: &str = "A path names a window by its title or app id (\"*\" for every window), then widgets below it, \" > \" between: each part a name, \"role:name\" or \"role:\" (any name), matched anywhere below the part before, case not minded. Examples: \"Calculator > button:7\", \"Windows > switch:Minimize\", \"* > button:Save\".";

fn tools() -> J {
    J::Arr(vec![
        obj(vec![
            ("name", s("windows")),
            (
                "description",
                s(
                    "Every window on the Vergence desktop: its id (client/window), kind, title, app, which one has the keyboard, which publish their meaning (Vergence's own apps; only these can be read and acted on), and what you may do with each app (asked, just now, timed with minutes left, session, always, never).",
                ),
            ),
            ("inputSchema", schema(vec![], &[])),
        ]),
        obj(vec![
            ("name", s("tree")),
            (
                "description",
                s(
                    "A window's widgets: each one's id, role, name, value, states (focused, off, selected, secret...), a choice's items, and its place on the screens in points. The first use of an app asks the person; the call waits for the answer.",
                ),
            ),
            (
                "inputSchema",
                schema(
                    vec![("window", prop("string", "The window's id from `windows`, as client/window (\"4/1\")."))],
                    &["window"],
                ),
            ),
        ]),
        obj(vec![
            ("name", s("find")),
            (
                "description",
                s(format!(
                    "Widgets by path, with their window and id. {PATHS} A search across every window only shows apps you are already allowed; name a window to be asked about its app."
                )),
            ),
            ("inputSchema", schema(vec![("path", prop("string", "The path."))], &["path"])),
        ]),
        obj(vec![
            ("name", s("focused")),
            (
                "description",
                s("The focused widget of the window the person is using, and the text caret, if a field has one."),
            ),
            ("inputSchema", schema(vec![], &[])),
        ]),
        obj(vec![
            ("name", s("act")),
            (
                "description",
                s(format!(
                    "Act on one widget as the person would: press (buttons, switches, choices), focus, reveal (scroll it into view), set (a field's or text area's whole text, a slider's number, a switch's on/off; never a secret field), choose (a choice's item by name). Name the widget by `path` (it must lead to exactly one) or by `window` and `node`. Waits for the window to show what happened and answers with the widget as it is after. The person sees a ring where you act; the first use of an app asks them, and the call waits. {PATHS}"
                )),
            ),
            (
                "inputSchema",
                schema(
                    vec![
                        (
                            "action",
                            obj(vec![
                                ("type", s("string")),
                                ("enum", J::Arr(vec![s("press"), s("focus"), s("reveal"), s("set"), s("choose")])),
                                ("description", s("What to do.")),
                            ]),
                        ),
                        ("path", prop("string", "The widget, by path.")),
                        ("window", prop("string", "Or the window (client/window) ...")),
                        ("node", prop("string", "... and the widget's id (a string: ids are 64-bit).")),
                        ("value", prop("string", "For set: the value. For choose: the item's name.")),
                    ],
                    &["action"],
                ),
            ),
        ]),
        obj(vec![
            ("name", s("read")),
            (
                "description",
                s(format!(
                    "A widget's whole text, past the tree's 512-character cut: an editor's document, a long field, later a terminal's screen and scrollback. Long texts come in pages: `offset` (characters, from 0) and `length` (at most 8192 a page); the answer says which characters it holds of how many, so ask again from where it ended until you have what you need. Name the widget by `path` or by `window` and `node`. Never a secret field. {PATHS}"
                )),
            ),
            (
                "inputSchema",
                schema(
                    vec![
                        ("path", prop("string", "The widget, by path.")),
                        ("window", prop("string", "Or the window (client/window) ...")),
                        ("node", prop("string", "... and the widget's id (a string: ids are 64-bit).")),
                        ("offset", prop("number", "The first character to read (default 0).")),
                        ("length", prop("number", "How many characters (default and most: 8192).")),
                    ],
                    &[],
                ),
            ),
        ]),
        obj(vec![
            ("name", s("terminal")),
            (
                "description",
                s(
                    "A terminal of your own, in a window of its own (the person's terminals and tabs stay hidden from you). The person is asked first (Just now, or for an hour); the call waits for the answer. Answers with its window and its tab; the pane to type into is the tab's \"terminal\" child (see `tree`). Then `type` and `key` into that pane, and `read` its screen (the pane's value) or its scrollback (the pane's \"log\" child).",
                ),
            ),
            (
                "inputSchema",
                schema(vec![("cwd", prop("string", "The folder it starts in (default: the home folder)."))], &[]),
            ),
        ]),
        obj(vec![
            ("name", s("type")),
            (
                "description",
                s(
                    "Type text into a terminal's pane, as the keyboard would: \"\\n\" ends a line (Return). Up to a line goes in at a visible pace (about a quarter of a second); longer text at once, as a paste. Refused when the pane is at a password prompt (the person types passwords), runs as root, or the person is typing there; and in tabs you hold no typing grant for. The person sees your name on the tab and can stop you at any time (Escape twice in the tab, or a click on the \"is typing · stop\" pill); then you are refused there for a while. Read the screen afterwards to see what happened.",
                ),
            ),
            (
                "inputSchema",
                schema(
                    vec![
                        ("window", prop("string", "The window (client/window).")),
                        ("node", prop("string", "The pane's id (role \"terminal\"), as a string.")),
                        ("text", prop("string", "What to type.")),
                    ],
                    &["window", "node", "text"],
                ),
            ),
        ]),
        obj(vec![
            ("name", s("key")),
            (
                "description",
                s(
                    "Press one key in a terminal's pane: Return, Tab, Escape, BackSpace, Delete, Up, Down, Left, Right, Home, End, Prior, Next, Insert, space, or ctrl+<letter> (ctrl+c to interrupt, ctrl+d to end input). Refused as `type` is.",
                ),
            ),
            (
                "inputSchema",
                schema(
                    vec![
                        ("window", prop("string", "The window (client/window).")),
                        ("node", prop("string", "The pane's id (role \"terminal\"), as a string.")),
                        ("name", prop("string", "The key's name.")),
                    ],
                    &["window", "node", "name"],
                ),
            ),
        ]),
        obj(vec![
            ("name", s("wait")),
            (
                "description",
                s(
                    "Waits until something changes after `since` (the focus, a caret, a window's widgets, the window with the keyboard, your timed grant running out), or until timeout_ms; answers with the changes and `latest`, the number to wait from next time. Start with since 0 to learn the latest.",
                ),
            ),
            (
                "inputSchema",
                schema(
                    vec![
                        ("since", prop("number", "The last change number seen (0 at first).")),
                        ("timeout_ms", prop("number", "How long to wait at most, up to 30000 (default 5000).")),
                    ],
                    &[],
                ),
            ),
        ]),
    ])
}

// ------------------------------------------------------------------ the bus

struct Bus {
    name: String,
    client: Option<Client>,
}

impl Bus {
    fn connect(&mut self) -> Result<&mut Client, String> {
        if self.client.is_none() {
            let mut c = Client::connect("vergence-agent-mcp").map_err(|e| format!("no Vergence session bus: {e}"))?;
            match c
                .request(|serial| ClientMsg::Agent(AgentRequest::Hello { serial, name: self.name.clone() }))
                .map_err(|e| e.to_string())?
            {
                ServerMsg::Done { .. } => {}
                other => return Err(format!("unexpected {other:?}")),
            }
            self.client = Some(c);
        }
        self.client.as_mut().ok_or_else(|| "no bus".into())
    }

    fn ask(&mut self, make: impl FnOnce(u32) -> AgentRequest) -> Result<AgentEvent, String> {
        let r = self.connect()?.request(|serial| ClientMsg::Agent(make(serial)));
        match r {
            Ok(ServerMsg::Agent(e)) => Ok(e),
            Ok(other) => Err(format!("unexpected {other:?}")),
            Err(e) => {
                // A lost connection is made again at the next call.
                let msg = e.to_string();
                if !msg.contains("Denied") && !msg.contains("refused") {
                    self.client = None;
                }
                Err(msg)
            }
        }
    }
}

fn node_text(n: &Node) -> String {
    let value = n.value.as_ref().map(|v| format!(" = {v:?}")).unwrap_or_default();
    let states = if n.states.is_empty() { String::new() } else { format!(" [{}]", n.states.join(", ")) };
    let items = if n.items.is_empty() { String::new() } else { format!(" items {:?}", n.items) };
    format!(
        "node \"{}\" {} {:?}{value}{states}{items} at {:.0},{:.0} {:.0}x{:.0}",
        n.id, n.role, n.name, n.x, n.y, n.w, n.h
    )
}

fn found_text(f: &AgentFound) -> String {
    format!("window \"{}/{}\" ({:?}) {}", f.client, f.window, f.title, node_text(&f.node))
}

fn change_text(n: u64, c: &AgentChange) -> String {
    match c {
        AgentChange::Window { client, window } => format!("{n}: the keyboard went to window \"{client}/{window}\""),
        AgentChange::Focus { client, window, node } => {
            format!("{n}: in window \"{client}/{window}\" the focus moved to node \"{node}\"")
        }
        AgentChange::Caret { client, window, at } => format!(
            "{n}: in window \"{client}/{window}\" the caret moved{}",
            at.map(|(x, y, _, _)| format!(" to {x:.0},{y:.0}")).unwrap_or_default()
        ),
        AgentChange::Tree { client, window } => format!("{n}: window \"{client}/{window}\" changed"),
        AgentChange::Time { app, minutes_left: 0 } => {
            format!("{n}: your time with {app} is up; the person is asked again at your next request")
        }
        AgentChange::Time { app, minutes_left } => format!(
            "{n}: your time with {app} ends in {minutes_left} min; ask the person for more (the tile's button) before a long step"
        ),
    }
}

fn window_id(v: Option<&J>) -> Result<(u32, u32), String> {
    let t = v.and_then(J::str).ok_or("window is \"client/window\"")?;
    let (a, b) = t.split_once('/').ok_or("window is \"client/window\"")?;
    Ok((
        a.trim().parse().map_err(|_| "window is \"client/window\"")?,
        b.trim().parse().map_err(|_| "window is \"client/window\"")?,
    ))
}

fn latest(bus: &mut Bus) -> Result<u64, String> {
    match bus.ask(|serial| AgentRequest::Changes { serial, since: u64::MAX - 1 })? {
        AgentEvent::Changes { latest, .. } => Ok(latest),
        other => Err(format!("unexpected {other:?}")),
    }
}

/// A tool's work: its text, or an error's.
fn call(bus: &mut Bus, name: &str, args: &J) -> Result<String, String> {
    match name {
        "windows" => match bus.ask(|serial| AgentRequest::Windows { serial })? {
            AgentEvent::Windows { windows, .. } => Ok(windows
                .iter()
                .map(|w| {
                    let mark = if w.keyboard { " (has the keyboard)" } else { "" };
                    let meaning = if w.meaning { "" } else { ", no meaning (cannot be read)" };
                    let allowed = match w.allowed.as_str() {
                        "" => ", not asked yet".to_string(),
                        "timed" => format!(", allowed for {} more min", w.minutes_left),
                        "just now" => ", allowed while you stay connected".into(),
                        "session" => ", allowed until the person logs out".into(),
                        "always" => ", allowed always".into(),
                        "never" => ", never allowed".into(),
                        other => format!(", {other}"),
                    };
                    let allowed = if w.meaning { allowed } else { String::new() };
                    format!(
                        "window \"{}/{}\" {} {:?} (app {}){mark} at {:.0},{:.0} {:.0}x{:.0} on {}{meaning}{allowed}",
                        w.client, w.window, w.kind, w.title, w.app_id, w.at.0, w.at.1, w.at.2, w.at.3, w.screen
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")),
            other => Err(format!("unexpected {other:?}")),
        },
        "tree" => {
            let (client, window) = window_id(args.get("window"))?;
            match bus.ask(|serial| AgentRequest::Tree { serial, client, window })? {
                AgentEvent::Tree { title, focus, caret, nodes, .. } => {
                    let mut out = vec![format!("window \"{client}/{window}\" {title:?}, focus node \"{focus}\"")];
                    out.extend(
                        nodes.iter().map(|n| format!("{}{}", if n.id == focus { "> " } else { "  " }, node_text(n))),
                    );
                    if let Some((x, y, w, h)) = caret {
                        out.push(format!("caret at {x:.0},{y:.0} {w:.0}x{h:.0}"));
                    }
                    Ok(out.join("\n"))
                }
                other => Err(format!("unexpected {other:?}")),
            }
        }
        "find" => {
            let path = args.get("path").and_then(J::str).ok_or("path is needed")?.to_string();
            match bus.ask(|serial| AgentRequest::Find { serial, path: path.clone() })? {
                AgentEvent::Found { found, .. } if found.is_empty() => {
                    Ok(format!("nothing at {path:?} (in apps you may use)"))
                }
                AgentEvent::Found { found, .. } => Ok(found.iter().map(found_text).collect::<Vec<_>>().join("\n")),
                other => Err(format!("unexpected {other:?}")),
            }
        }
        "focused" => match bus.ask(|serial| AgentRequest::Focused { serial })? {
            AgentEvent::Focused { found: None, .. } => Ok("nothing focused in a window with meaning".into()),
            AgentEvent::Focused { found: Some(f), caret, .. } => Ok(format!(
                "{}{}",
                found_text(&f),
                caret.map(|(x, y, w, h)| format!("\ncaret at {x:.0},{y:.0} {w:.0}x{h:.0}")).unwrap_or_default()
            )),
            other => Err(format!("unexpected {other:?}")),
        },
        // A terminal of its own, typing and keys (bus v38).
        "terminal" => {
            let cwd = args.get("cwd").and_then(J::str).unwrap_or("").to_string();
            match bus.ask(|serial| AgentRequest::OpenTerminal { serial, cwd: cwd.clone() })? {
                AgentEvent::Terminal { client, window, tab, .. } => {
                    let pane = match bus.ask(|serial| AgentRequest::Tree { serial, client, window }) {
                        Ok(AgentEvent::Tree { nodes, .. }) => {
                            nodes.iter().find(|n| n.parent == tab && n.role == "terminal").map(|n| n.id)
                        }
                        _ => None,
                    };
                    Ok(match pane {
                        Some(p) => {
                            format!("your terminal: window {client}/{window}, tab {tab}, pane {p} (type into node {p})")
                        }
                        None => format!("your terminal: window {client}/{window}, tab {tab}"),
                    })
                }
                other => Err(format!("unexpected {other:?}")),
            }
        }
        "type" | "key" => {
            let (client, window) = window_id(args.get("window"))?;
            let node: u64 = args
                .get("node")
                .and_then(J::str)
                .and_then(|n| n.trim().parse().ok())
                .ok_or("node is the pane's id, as a string")?;
            let action = if name == "type" {
                let t = args.get("text").and_then(J::str).ok_or("text is needed")?;
                Action::Type(t.to_string())
            } else {
                let k = args.get("name").and_then(J::str).ok_or("name is needed")?;
                if !vergence_protocol::window::key_name_ok(k) {
                    return Err(format!("no key {k:?}"));
                }
                Action::Key(k.to_string())
            };
            match bus.ask(|serial| AgentRequest::Act {
                serial,
                client,
                window,
                node,
                action: action.clone(),
                role: "terminal".into(),
                name: String::new(),
            })? {
                AgentEvent::Acted { ok: true, .. } => Ok("done; read the pane to see the screen".into()),
                AgentEvent::Acted { why, .. } => Err(format!("refused: {why}")),
                other => Err(format!("unexpected {other:?}")),
            }
        }
        "read" => {
            let (client, window, node, role, wname) = if let Some(path) = args.get("path").and_then(J::str) {
                let found = match bus.ask(|serial| AgentRequest::Find { serial, path: path.to_string() })? {
                    AgentEvent::Found { found, .. } => found,
                    other => return Err(format!("unexpected {other:?}")),
                };
                match found.as_slice() {
                    [one] => (one.client, one.window, one.node.id, one.node.role.clone(), one.node.name.clone()),
                    [] => return Err(format!("nothing at {path:?} (in apps you may use)")),
                    many => {
                        return Err(format!(
                            "{} widgets at {path:?}; name one exactly (a fuller path, or window and node):\n{}",
                            many.len(),
                            many.iter().map(found_text).collect::<Vec<_>>().join("\n")
                        ));
                    }
                }
            } else {
                let (c, w) = window_id(args.get("window"))?;
                let n = args
                    .get("node")
                    .and_then(J::str)
                    .and_then(|n| n.trim().parse().ok())
                    .ok_or("node is the widget's id, as a string")?;
                (c, w, n, String::new(), String::new())
            };
            let number = |k: &str, d: u32| {
                args.get(k)
                    .and_then(|v| match v {
                        J::Num(n) if *n >= 0.0 => Some(n.min(u32::MAX as f64) as u32),
                        J::Str(t) => t.trim().parse().ok(),
                        _ => None,
                    })
                    .unwrap_or(d)
            };
            let (offset, len) = (number("offset", 0), number("length", 8192));
            match bus.ask(|serial| AgentRequest::Read {
                serial,
                client,
                window,
                node,
                offset,
                len,
                role: role.clone(),
                name: wname.clone(),
            })? {
                AgentEvent::Text { offset, total, text, .. } => {
                    let end = offset + text.chars().count() as u32;
                    let more = if end < total { format!("; more from offset {end}") } else { String::new() };
                    Ok(format!("{text}\n[characters {offset}..{end} of {total}{more}]"))
                }
                other => Err(format!("unexpected {other:?}")),
            }
        }
        "act" => {
            let value = args.get("value").and_then(J::str).unwrap_or("").to_string();
            let action = match args.get("action").and_then(J::str).unwrap_or("") {
                "press" => Action::Press,
                "focus" => Action::Focus,
                "reveal" => Action::Reveal,
                "set" => Action::Set(value),
                "choose" if !value.is_empty() => Action::Choose(value),
                "choose" => return Err("choose needs the item's name in value".into()),
                other => return Err(format!("no action {other:?}: press, focus, reveal, set, choose")),
            };
            let (client, window, node, role, wname) = if let Some(path) = args.get("path").and_then(J::str) {
                let found = match bus.ask(|serial| AgentRequest::Find { serial, path: path.to_string() })? {
                    AgentEvent::Found { found, .. } => found,
                    other => return Err(format!("unexpected {other:?}")),
                };
                match found.as_slice() {
                    [one] => (one.client, one.window, one.node.id, one.node.role.clone(), one.node.name.clone()),
                    [] => return Err(format!("nothing at {path:?} (in apps you may use)")),
                    many => {
                        return Err(format!(
                            "{} widgets at {path:?}; name one exactly (a fuller path, or window and node):\n{}",
                            many.len(),
                            many.iter().map(found_text).collect::<Vec<_>>().join("\n")
                        ));
                    }
                }
            } else {
                let (c, w) = window_id(args.get("window"))?;
                let n = args
                    .get("node")
                    .and_then(J::str)
                    .and_then(|n| n.trim().parse().ok())
                    .ok_or("node is the widget's id, as a string")?;
                (c, w, n, String::new(), String::new())
            };
            let since = latest(bus)?;
            match bus.ask(|serial| AgentRequest::Act { serial, client, window, node, action, role, name: wname })? {
                AgentEvent::Acted { ok: true, .. } => {}
                AgentEvent::Acted { why, .. } => return Err(format!("refused: {why}")),
                other => return Err(format!("unexpected {other:?}")),
            }
            // The window's tree once it shows what happened (a frame later).
            let until = Instant::now() + SETTLE;
            while Instant::now() < until {
                if let AgentEvent::Changes { changes, .. } = bus.ask(|serial| AgentRequest::Changes { serial, since })?
                    && changes.iter().any(|(_, c)| matches!(c, AgentChange::Tree { client: c2, window: w2 } if *c2 == client && *w2 == window))
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
            let after = match bus.ask(|serial| AgentRequest::Tree { serial, client, window }) {
                Ok(AgentEvent::Tree { nodes, .. }) => nodes
                    .into_iter()
                    .find(|n| n.id == node)
                    .map(|n| format!("\nnow: {}", node_text(&n)))
                    .unwrap_or_else(|| "\n(the widget is gone now: the window changed around it)".into()),
                _ => String::new(),
            };
            Ok(format!("done in window \"{client}/{window}\"{after}"))
        }
        "wait" => {
            let since = args.get("since").and_then(J::num).unwrap_or(0.0).max(0.0) as u64;
            let timeout = args.get("timeout_ms").and_then(J::num).unwrap_or(5000.0).clamp(0.0, WAIT_MAX as f64) as u64;
            let until = Instant::now() + Duration::from_millis(timeout);
            loop {
                if let AgentEvent::Changes { latest, changes, .. } =
                    bus.ask(|serial| AgentRequest::Changes { serial, since })?
                    && (!changes.is_empty() || since == 0 || Instant::now() >= until)
                {
                    {
                        let mut out: Vec<String> = changes.iter().map(|(n, c)| change_text(*n, c)).collect();
                        if out.is_empty() {
                            out.push("nothing changed".into());
                        }
                        out.push(format!("latest {latest}"));
                        return Ok(out.join("\n"));
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        other => Err(format!("no tool {other:?}")),
    }
}

// ------------------------------------------------------------------ JSON-RPC

fn reply(id: &J, result: J) -> String {
    to_json(&obj(vec![("jsonrpc", s("2.0")), ("id", id.clone()), ("result", result)]))
}

fn error(id: &J, code: i64, message: &str) -> String {
    to_json(&obj(vec![
        ("jsonrpc", s("2.0")),
        ("id", id.clone()),
        ("error", obj(vec![("code", J::Num(code as f64)), ("message", s(message))])),
    ]))
}

/// One message: the line to answer with, if any (notifications have none).
fn handle(bus: &mut Bus, line: &str) -> Option<String> {
    let m = match parse(line) {
        Ok(m) => m,
        Err(e) => return Some(error(&J::Null, -32700, &format!("parse error: {e}"))),
    };
    let id = m.get("id").cloned();
    let method = m.get("method").and_then(J::str).unwrap_or("");
    let params = m.get("params").cloned().unwrap_or(J::Obj(Vec::new()));
    let Some(id) = id else {
        // A notification (initialized, cancelled): nothing to say.
        return None;
    };
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(J::str).unwrap_or(VERSIONS[0]);
            let version = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(VERSIONS[0]);
            reply(
                &id,
                obj(vec![
                    ("protocolVersion", s(version)),
                    ("capabilities", obj(vec![("tools", obj(vec![]))])),
                    (
                        "serverInfo",
                        obj(vec![("name", s("vergence-agent-mcp")), ("version", s(env!("CARGO_PKG_VERSION")))]),
                    ),
                    (
                        "instructions",
                        s(
                            "Vergence's apps, by meaning: read windows and act on widgets by name. Every app is asked about once, in the desktop's own question; the person sees a ring where you act. Use `windows` first, then `find` or `tree`, then `act`.",
                        ),
                    ),
                ]),
            )
        }
        "ping" => reply(&id, obj(vec![])),
        "tools/list" => reply(&id, obj(vec![("tools", tools())])),
        "tools/call" => {
            let name = params.get("name").and_then(J::str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(J::Obj(Vec::new()));
            let (text, failed) = match call(bus, name, &args) {
                Ok(t) => (t, false),
                Err(e) => (e, true),
            };
            reply(
                &id,
                obj(vec![
                    ("content", J::Arr(vec![obj(vec![("type", s("text")), ("text", s(text))])])),
                    ("isError", J::Bool(failed)),
                ]),
            )
        }
        _ => error(&id, -32601, &format!("no method {method:?}")),
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = args
        .iter()
        .position(|a| a == "--name")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "Claude".into());
    let mut bus = Bus { name, client: None };
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    let mut line = String::new();
    let mut input = stdin.lock();
    loop {
        line.clear();
        match input.by_ref().take(LINE_MAX as u64 + 1).read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(n) if n > LINE_MAX => {
                let _ = writeln!(out, "{}", error(&J::Null, -32700, "a message longer than 1 MB"));
                // The rest of that line is skipped.
                let mut rest = String::new();
                let _ = input.read_line(&mut rest);
                continue;
            }
            Ok(_) => {}
        }
        if line.trim().is_empty() {
            continue;
        }
        if let Some(answer) = handle(&mut bus, line.trim_end())
            && (writeln!(out, "{answer}").is_err() || out.flush().is_err())
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trips() {
        let text = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"act","arguments":{"node":"18446744073709551615","value":"caf\u00e9 \ud83d\ude00 \"q\"\n","n":-1.5e3,"ok":[true,false,null]}}}"#;
        let v = parse(text).unwrap();
        assert_eq!(v.get("id"), Some(&J::Num(7.0)));
        let args = v.get("params").unwrap().get("arguments").unwrap();
        assert_eq!(args.get("node").and_then(J::str), Some("18446744073709551615"), "64-bit ids stay strings");
        assert_eq!(args.get("value").and_then(J::str), Some("café 😀 \"q\"\n"));
        assert_eq!(args.get("n"), Some(&J::Num(-1500.0)));
        assert_eq!(parse(&to_json(&v)).unwrap(), v, "written and read back");
    }

    #[test]
    fn bad_json_is_refused_not_panicked() {
        for bad in ["", "{", "{\"a\"}", "[1,]", "\"\\x\"", "tru", "{\"a\":1} x", "\"\u{1}\""] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(1000);
        assert!(parse(&deep).is_err(), "nesting is bounded");
        let mut seed = 99u64;
        let base = to_json(&tools());
        for _ in 0..3000 {
            let mut b = base.clone().into_bytes();
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let at = (seed >> 33) as usize % b.len();
            b[at] = (seed >> 7) as u8;
            if let Ok(t) = String::from_utf8(b) {
                let _ = parse(&t);
            }
        }
    }

    #[test]
    fn the_protocol_without_a_bus() {
        let mut bus = Bus { name: "Test".into(), client: None };
        let init = handle(&mut bus, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#).unwrap();
        let v = parse(&init).unwrap();
        assert_eq!(
            v.get("result").and_then(|r| r.get("protocolVersion")).and_then(J::str),
            Some("2025-03-26"),
            "the client's version, known"
        );
        assert!(
            handle(&mut bus, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none(),
            "a notification gets no answer"
        );
        let list = parse(&handle(&mut bus, r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#).unwrap()).unwrap();
        let tools = list.get("result").and_then(|r| r.get("tools")).unwrap();
        let J::Arr(t) = tools else { panic!() };
        assert_eq!(t.len(), 10, "windows, tree, find, focused, act, read, terminal, type, key, wait");
        let unknown = parse(&handle(&mut bus, r#"{"jsonrpc":"2.0","id":2,"method":"nope"}"#).unwrap()).unwrap();
        assert_eq!(unknown.get("error").and_then(|e| e.get("code")), Some(&J::Num(-32601.0)));
        let garbage = parse(&handle(&mut bus, "{oops").unwrap()).unwrap();
        assert_eq!(garbage.get("error").and_then(|e| e.get("code")), Some(&J::Num(-32700.0)));
    }
}
