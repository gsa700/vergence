//! An agent typing in a terminal (terminal agent milestone 1, step 4; bus v38).
//!
//! The server lets an agent type only in a tab it holds a typing grant for
//! (step 1), and tells the window which tabs agents hold grants for
//! (WinEvent::AgentTabs, bus v39); the terminal adds what only it can see, at
//! the moment of typing and again before every step of a paced burst:
//!
//! - a pane at a password prompt refuses: "Terminal is asking for a password:
//!   the person types it" (line mode with echo off, or the text before the
//!   cursor reading as a prompt: session::password_line);
//! - a pane whose foreground runs as root refuses (asked afresh at each act);
//! - a pane whose foreground is ssh, mosh, tmux, screen, su, sudo, doas, a
//!   pinentry and the like refuses (session::NO_AGENT_PROGRAMS): their
//!   prompts and panes are out of the terminal's sight;
//! - a pane the person typed in during the last second refuses ("the person
//!   is typing"), and a keystroke of the person's during an agent's burst
//!   drops the rest of it; the agent hears so at its next act;
//! - a pane still typing what the agent sent refuses the next act ("still
//!   typing"), and for SETTLE after a line goes in (the program it started
//!   may be about to ask for a password or become root);
//! - a tab the person stopped the agent in refuses for COOL_DOWN, whatever
//!   the server's answer on its way says.
//!
//! What it types goes in as typing does, at a visible pace: up to a line
//! (PACED_MAX characters, no line end but a last one) over PACE; longer text
//! at once, as a paste, and only to a program that asked for bracketed paste
//! (else its lines would run one by one as they arrive, the later ones in
//! whatever the earlier started). Escape and other control characters are
//! left out of typed text: keys go by name (Action::Key).
//!
//! A tab an agent holds a grant for is marked in the bar by the server, in
//! the agent's color, for as long as the grant lasts (read: outlined; type:
//! tinted). While an agent types in a tab, and for MARK_FOR after, the tab's
//! title starts with "⌨ ", and a pill in the pane's top-right corner says
//! "Claude is typing · stop": a click on it ends the agent's grant for the
//! tab (WinRequest::RevokeAgent), as does Escape twice within DOUBLE_ESCAPE
//! in any tab an agent holds a grant for, used or not. The first Escape goes
//! to the program as always (it may be meant for vim); the second is the
//! stop, and is not sent. The terminal then notes in the pane "<agent>'s
//! access to this tab ended".

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use vergence_scene::Rect;
use vergence_ui::{Action, Modifiers, Window, sym};

use crate::meaning::{pane_node, tab_node, tab_of};
use crate::session::lock;
use crate::{TIMER_AGENT, Terminal};

/// A typed line's time on the way in.
pub(crate) const PACE: Duration = Duration::from_millis(250);
/// How often a paced burst moves on.
const STEP: Duration = Duration::from_millis(16);
/// The most characters typed at the pace; more go in at once.
pub(crate) const PACED_MAX: usize = 120;
/// How long after the person's last keystroke in a pane an agent waits.
pub(crate) const PERSON_QUIET: Duration = Duration::from_millis(1000);
/// How long the mark stays after the agent's last act.
pub(crate) const MARK_FOR: Duration = Duration::from_secs(3);
/// Escape twice within this is the stop.
pub(crate) const DOUBLE_ESCAPE: Duration = Duration::from_millis(500);
/// How long after a line goes in (Return) the pane takes no act: time for
/// what it started to turn echo off or become root before the next.
pub(crate) const SETTLE: Duration = Duration::from_millis(300);
/// How long a stop holds in the terminal (the server's cool-down, too).
pub(crate) const COOL_DOWN: Duration = Duration::from_secs(60);

/// A paced burst on its way into a pane: what is left, and how much a step.
struct Burst {
    tab: u32,
    pane: u64,
    bytes: VecDeque<u8>,
    step: usize,
}

/// The agents at work in this window.
#[derive(Default)]
pub(crate) struct Agents {
    /// The name the server last said is acting, and on what node.
    acting: Option<(u64, String)>,
    /// By tab: the agent that typed there and when it last did.
    typing: HashMap<u32, (String, Instant)>,
    /// By tab: the agents holding a grant there, as the server last said
    /// (names, and whether one may type). The stop keys are for these tabs.
    granted: HashMap<u32, (String, bool)>,
    bursts: VecDeque<Burst>,
    /// By pane: the person's last keystroke.
    person: HashMap<(u32, u64), Instant>,
    /// By pane: why an agent's burst stopped short (told at its next act).
    told: HashMap<(u32, u64), String>,
    /// By pane: when a line last went in.
    settle: HashMap<(u32, u64), Instant>,
    /// By tab: when the person stopped the agents there.
    stopped: HashMap<u32, Instant>,
    /// The last Escape the person pressed: in what tab, and when.
    escape: Option<(u32, Instant)>,
    /// The mark as last drawn: where, and for what tab.
    pub(crate) mark: Option<(Rect, u32)>,
    timer: bool,
}

/// A key name (window::KEY_NAMES, or ctrl+letter) as the key's symbol, its
/// text and modifiers.
pub(crate) fn key_of(name: &str) -> Option<(u32, Option<String>, Modifiers)> {
    let plain = |k: u32| Some((k, None, Modifiers::default()));
    match name {
        "Return" => plain(sym::RETURN),
        "Tab" => plain(sym::TAB),
        "Escape" => plain(sym::ESCAPE),
        "BackSpace" => plain(sym::BACKSPACE),
        "Delete" => plain(sym::DELETE),
        "Up" => plain(sym::UP),
        "Down" => plain(sym::DOWN),
        "Left" => plain(sym::LEFT),
        "Right" => plain(sym::RIGHT),
        "Home" => plain(sym::HOME),
        "End" => plain(sym::END),
        "Prior" => plain(sym::PAGE_UP),
        "Next" => plain(sym::PAGE_DOWN),
        "Insert" => plain(0xff63),
        "space" => Some((0x20, Some(" ".into()), Modifiers::default())),
        _ => {
            let c = name.strip_prefix("ctrl+")?;
            let b = *c.as_bytes().first().filter(|b| c.len() == 1 && b.is_ascii_lowercase())?;
            Some((u32::from(b), Some(c.into()), Modifiers { ctrl: true, ..Default::default() }))
        }
    }
}

/// Why an agent may not type in a pane now, if it may not: what only the
/// terminal sees at the moment of typing (the tree it last sent may be old).
pub(crate) fn refusal(state: &crate::session::PaneState, person_typed: Option<Instant>) -> Option<&'static str> {
    if state.exited {
        Some("the program in this pane has ended")
    } else if state.root {
        Some("this tab runs as root: agents may not type here")
    } else if state.password {
        Some("Terminal is asking for a password: the person types it")
    } else if crate::session::no_agent_program(&state.program) {
        Some("this tab runs a program agents may not type into (a remote shell, a multiplexer, su or sudo)")
    } else if person_typed.is_some_and(|t| t.elapsed() < PERSON_QUIET) {
        Some("the person is typing")
    } else {
        None
    }
}

/// Typed text as it goes in: line ends as "\n" (one definition of a line:
/// "\r\n", "\r" and "\n" alike), and no control character but the tab and
/// line ends: no Escape (a sequence of its own could move the cursor, end a
/// paste, or be a key the agent was not granted), no C1 control, no DEL.
pub(crate) fn clean_typed(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    text.chars()
        .filter(|&c| c == '\t' || c == '\n' || !(c.is_control() || ('\u{80}'..='\u{9f}').contains(&c)))
        .collect()
}

/// Whether clean typed text goes at the pace (up to a line) or at once.
pub(crate) fn paced(text: &str) -> bool {
    let body = text.strip_suffix('\n').unwrap_or(text);
    text.chars().count() <= PACED_MAX && !body.contains('\n')
}

/// The bytes clean typed text sends: line ends as the Return key; a long text
/// as a paste (bracketed when the program asked; it cannot hold an Escape, so
/// cannot end its own brackets). None: more than a line for a program that
/// did not ask for bracketed paste.
pub(crate) fn typed_bytes(text: &str, bracketed: bool) -> Option<Vec<u8>> {
    let cr = text.replace('\n', "\r");
    if paced(text) {
        Some(cr.into_bytes())
    } else if bracketed {
        Some(format!("\x1b[200~{cr}\x1b[201~").into_bytes())
    } else if !text.strip_suffix('\n').unwrap_or(text).contains('\n') {
        // One long line: typed at once, as a line is.
        Some(cr.into_bytes())
    } else {
        None
    }
}

impl Terminal {
    /// The server says who is about to act at `node`.
    pub(crate) fn on_agent_acting(&mut self, node: u64, agent: &str) {
        self.agents.acting = Some((node, agent.to_string()));
    }

    /// The server says which tabs agents hold grants for now (bus v39).
    pub(crate) fn on_agent_tabs(&mut self, tabs: &[(u64, String, bool)], window: &mut Window) {
        self.agents.granted = tabs
            .iter()
            .filter_map(|(node, agent, typing)| tab_of(*node).map(|t| (t, (agent.clone(), *typing))))
            .collect();
        // A grant that ended: whatever it was still typing stops.
        let granted = &self.agents.granted;
        self.agents.bursts.retain(|b| granted.get(&b.tab).is_some_and(|g| g.1));
        window.redraw();
    }

    /// Why the agent may not act in this pane now, before anything about the
    /// program in it: stopped, still typing, a line just in, or a burst cut.
    fn act_gate(&mut self, tab: u32, pane: u64) -> Result<(), String> {
        if self.agents.stopped.get(&tab).is_some_and(|t| t.elapsed() < COOL_DOWN) {
            return Err("the person stopped you in this tab (ask again later)".into());
        }
        if !self.agents.granted.get(&tab).is_some_and(|g| g.1) {
            return Err("not in a tab you may type in".into());
        }
        if let Some(why) = self.agents.told.remove(&(tab, pane)) {
            return Err(why);
        }
        if self.agents.bursts.iter().any(|b| b.tab == tab && b.pane == pane) {
            return Err("still typing what you sent last: wait for it, then read the screen".into());
        }
        if self.agents.settle.get(&(tab, pane)).is_some_and(|t| t.elapsed() < SETTLE) {
            return Err("the last line is still starting: read the screen, then go on".into());
        }
        Ok(())
    }

    /// An agent's action on a node (main.rs's `act`): typing and keys into a
    /// pane, with the refusals; every other action as before (none).
    pub(crate) fn on_act(&mut self, node: u64, action: &Action, window: &mut Window) -> Result<(), String> {
        let (Action::Type(_) | Action::Key(_)) = action else {
            return Err("this window takes typing and keys only".into());
        };
        let tab_id = tab_of(node).ok_or("not a terminal")?;
        let tab = self.tabs.iter().find(|t| t.id == tab_id).ok_or("no such tab now")?;
        let pane = tab
            .panes
            .iter()
            .map(|(p, _)| *p)
            .find(|p| pane_node(tab_id, *p) == node)
            .ok_or("not a terminal's pane (type into a \"terminal\" node)")?;
        let agent = match self.agents.acting.take() {
            Some((n, a)) if n == node => a,
            _ => self.agents.granted.get(&tab_id).map_or_else(|| "An agent".to_string(), |g| g.0.clone()),
        };
        self.act_gate(tab_id, pane)?;
        let session =
            self.tabs.iter().find(|t| t.id == tab_id).and_then(|t| t.session(pane)).ok_or("no such pane now")?;
        // What only the terminal sees, now, not as the last tree said.
        let state = session.state_for_typing();
        if let Some(why) = refusal(&state, self.agents.person.get(&(tab_id, pane)).copied()) {
            return Err(why.into());
        }
        let (bracketed, app_cursor) = {
            let s = lock(&session.screen);
            (s.grid.bracketed_paste, s.grid.app_cursor)
        };
        let (bytes, pace) = match action {
            Action::Type(text) => {
                let text = clean_typed(text);
                if text.is_empty() {
                    return Err("nothing to type (control characters are left out: send keys by name)".into());
                }
                let bytes = typed_bytes(&text, bracketed).ok_or(
                    "more than one line at once goes only to a program that takes pastes: type a line at a time",
                )?;
                (bytes, paced(&text))
            }
            Action::Key(name) => {
                let (k, text, m) = key_of(name).ok_or_else(|| format!("no key {name:?}"))?;
                let bytes =
                    Terminal::key_bytes(k, text.as_deref(), m, app_cursor).ok_or_else(|| format!("no key {name:?}"))?;
                (bytes, false)
            }
            _ => unreachable!(),
        };
        let now = Instant::now();
        self.agents.typing.insert(tab_id, (agent, now));
        if pace {
            let step = bytes.len().div_ceil((PACE.as_millis() / STEP.as_millis()) as usize).max(1);
            self.agents.bursts.push_back(Burst { tab: tab_id, pane, bytes: bytes.into(), step });
        } else {
            self.write_agent_bytes(tab_id, pane, &bytes);
        }
        self.agent_timer(window);
        self.bar_told = None;
        window.redraw();
        Ok(())
    }

    /// An agent's bytes into a pane: a line end starts the settling time.
    fn write_agent_bytes(&mut self, tab: u32, pane: u64, bytes: &[u8]) {
        self.write_pane(tab, pane, bytes);
        if bytes.contains(&b'\r') {
            self.agents.settle.insert((tab, pane), Instant::now());
        }
    }

    /// Bytes into a pane as typing: to the live screen, the selection let go.
    fn write_pane(&mut self, tab: u32, pane: u64, bytes: &[u8]) {
        let Some(session) = self.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.session_mut(pane)) else {
            return;
        };
        {
            let mut s = lock(&session.screen);
            s.scroll = 0;
            s.selection = None;
        }
        session.pty.write(bytes);
    }

    fn agent_timer(&mut self, window: &mut Window) {
        if !self.agents.timer {
            self.agents.timer = true;
            window.wake_after(TIMER_AGENT, STEP);
        }
    }

    /// The pace's timer: each burst a step further, each step only if the
    /// pane still takes typing (a program started meanwhile may ask for a
    /// password or be root: the rest is dropped and the agent told); the
    /// marks that are due go.
    pub(crate) fn on_agent_timer(&mut self, window: &mut Window) {
        self.agents.timer = false;
        // One step of the first burst of each pane, in order.
        let mut seen: Vec<(u32, u64)> = Vec::new();
        let mut steps: Vec<(u32, u64)> = Vec::new();
        for b in self.agents.bursts.iter() {
            if !seen.contains(&(b.tab, b.pane)) {
                seen.push((b.tab, b.pane));
                steps.push((b.tab, b.pane));
            }
        }
        for (tab, pane) in steps {
            let why = match self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.session(pane)) {
                None => Some("the pane closed".to_string()),
                Some(session) => {
                    let st = session.state_between_steps();
                    refusal(&st, None).map(|w| format!("stopped typing: {w}; the rest was not typed"))
                }
            };
            if let Some(why) = why {
                self.agents.bursts.retain(|b| !(b.tab == tab && b.pane == pane));
                self.agents.told.insert((tab, pane), why);
                continue;
            }
            let Some(b) = self.agents.bursts.iter_mut().find(|b| b.tab == tab && b.pane == pane) else { continue };
            let n = b.step.min(b.bytes.len());
            // Never cut a character in two.
            let mut chunk: Vec<u8> = b.bytes.drain(..n).collect();
            while let Some(&next) = b.bytes.front()
                && next & 0xc0 == 0x80
            {
                chunk.push(next);
                b.bytes.pop_front();
            }
            self.write_agent_bytes(tab, pane, &chunk);
            if let Some(t) = self.agents.typing.get_mut(&tab) {
                t.1 = Instant::now();
            }
        }
        self.agents.bursts.retain(|b| !b.bytes.is_empty());
        // Marks whose time is up.
        let before = self.agents.typing.len();
        let typing_bursts: Vec<u32> = self.agents.bursts.iter().map(|b| b.tab).collect();
        self.agents.typing.retain(|tab, (_, at)| at.elapsed() < MARK_FOR || typing_bursts.contains(tab));
        if self.agents.typing.len() != before {
            self.bar_told = None;
        }
        if !self.agents.bursts.is_empty() {
            self.agent_timer(window);
        } else if !self.agents.typing.is_empty() {
            self.agents.timer = true;
            window.wake_after(TIMER_AGENT, Duration::from_millis(250));
        }
        window.redraw();
    }

    /// Whether the stop keys are for this tab: an agent holds a grant there
    /// (read or type, used or not), or typed there lately.
    fn agent_holds(&self, tab: u32) -> bool {
        self.agents.granted.contains_key(&tab) || self.agents.typing.contains_key(&tab)
    }

    /// The person's input, before the terminal's own handling: true when it
    /// was the stop and is not to go on. A keystroke in a pane notes the
    /// person typing there (and cuts an agent's burst short); Escape twice in
    /// a tab an agent holds stops it; a click on the mark does too.
    pub(crate) fn agent_input(&mut self, input: &vergence_ui::Input, window: &mut Window) -> bool {
        use vergence_ui::{Button, Input};
        match input {
            Input::Key { sym: key, pressed: true, composing: false, modifiers, .. } => {
                let Some(tab) = self.tabs.get(self.shown) else { return false };
                let (tab_id, pane) = (tab.id, tab.active);
                // Modifier keys alone are not typing.
                if (0xffe1..=0xffee).contains(key) {
                    return false;
                }
                self.agents.person.insert((tab_id, pane), Instant::now());
                let before = self.agents.bursts.len();
                self.agents.bursts.retain(|b| !(b.tab == tab_id && b.pane == pane));
                if self.agents.bursts.len() != before {
                    self.agents.told.insert(
                        (tab_id, pane),
                        "the person typed: the rest of what you typed last was not typed; read the screen".into(),
                    );
                }
                let plain = !modifiers.ctrl && !modifiers.alt && !modifiers.shift && !modifiers.logo;
                if *key == sym::ESCAPE && plain && self.agent_holds(tab_id) {
                    let now = Instant::now();
                    if self.agents.escape.is_some_and(|(t, at)| t == tab_id && now - at < DOUBLE_ESCAPE) {
                        self.agents.escape = None;
                        self.agent_stop(tab_id, window);
                        return true;
                    }
                    self.agents.escape = Some((tab_id, now));
                } else {
                    self.agents.escape = None;
                }
                false
            }
            Input::Button { button: Button::Left, pressed: true, x, y, .. } => {
                let Some((rect, tab)) = self.agents.mark else { return false };
                if *x >= rect.x && *x < rect.x + rect.w && *y >= rect.y && *y < rect.y + rect.h {
                    self.agent_stop(tab, window);
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// The person stopped the agents in this tab: the server ends their
    /// grants, their typing stops, the terminal refuses them there for the
    /// cool-down itself (an act already on its way included), and the pane
    /// says so.
    fn agent_stop(&mut self, tab: u32, window: &mut Window) {
        let agent = self
            .agents
            .granted
            .remove(&tab)
            .map(|g| g.0)
            .or_else(|| self.agents.typing.get(&tab).map(|t| t.0.clone()))
            .unwrap_or_else(|| "The agent".into());
        self.agents.typing.remove(&tab);
        self.agents.bursts.retain(|b| b.tab != tab);
        self.agents.told.retain(|(t, _), _| *t != tab);
        self.agents.stopped.insert(tab, Instant::now());
        self.agents.mark = None;
        window.revoke_agent(tab_node(tab), &agent);
        if let Some(t) = self.tabs.iter().find(|t| t.id == tab)
            && let Some(session) = t.session(t.active)
        {
            let mut s = lock(&session.screen);
            let s = &mut *s;
            let note = format!("\r\n\x1b[2m[{agent}'s access to this tab ended]\x1b[0m\r\n");
            s.parser.advance(&mut s.grid, note.as_bytes());
        }
        self.bar_told = None;
        window.redraw();
    }

    /// The tab's title in the bar: "⌨ " first while an agent types there (the
    /// server puts the agent's name before it, in its color, for as long as
    /// it holds a grant there).
    pub(crate) fn agent_title(&self, tab: u32, title: String) -> String {
        match self.agents.typing.get(&tab) {
            Some(_) => format!("\u{2328} {title}"),
            None => title,
        }
    }

    /// The agent typing in the tab in front, if one is (for the mark).
    pub(crate) fn agent_typing_in_front(&self) -> Option<(u32, String)> {
        let tab = self.tabs.get(self.shown)?;
        self.agents.typing.get(&tab.id).map(|(a, _)| (tab.id, a.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_at_the_moment_of_typing() {
        use crate::session::PaneState;
        let shell = PaneState { program: "zsh".into(), ..Default::default() };
        assert_eq!(refusal(&shell, None), None);
        let pw = PaneState { password: true, ..shell.clone() };
        assert_eq!(refusal(&pw, None), Some("Terminal is asking for a password: the person types it"));
        let root = PaneState { root: true, ..shell.clone() };
        assert_eq!(refusal(&root, None), Some("this tab runs as root: agents may not type here"));
        // Root wins over a password prompt (sudo -s asking): either way refused.
        assert!(refusal(&PaneState { root: true, password: true, ..shell.clone() }, None).is_some());
        assert_eq!(refusal(&shell, Some(Instant::now())), Some("the person is typing"));
        assert_eq!(refusal(&shell, Some(Instant::now() - PERSON_QUIET * 2)), None, "quiet long enough");
        assert!(refusal(&PaneState { exited: true, ..Default::default() }, None).is_some());
        // Programs whose prompts and panes the terminal cannot see.
        for p in ["ssh", "mosh-client", "tmux", "screen", "su", "sudo", "doas", "pinentry-curses", "/usr/bin/ssh"] {
            let st = PaneState { program: p.into(), busy: true, ..Default::default() };
            assert!(refusal(&st, None).is_some(), "{p}");
        }
        for p in ["vim", "cargo", "sshfs-helper", "less"] {
            let st = PaneState { program: p.into(), busy: true, ..Default::default() };
            assert_eq!(refusal(&st, None), None, "{p}");
        }
    }

    #[test]
    fn password_prompts_by_their_text() {
        use crate::session::password_line;
        for l in [
            "Password:",
            "[sudo] password for dave: ",
            "Enter passphrase for key '/home/me/.ssh/id_ed25519': ",
            "dave@host's password:",
            "Enter PIN for 'YubiKey':",
            "Verification code:",
            "Mot de passe :",
        ] {
            assert!(password_line(l), "{l:?}");
        }
        for l in ["$ ", "dave@techbench ~ % ", "vim notes.txt", "Spinning:", "password manager open", "> "] {
            assert!(!password_line(l), "{l:?}");
        }
    }

    #[test]
    fn keys_by_name() {
        assert_eq!(key_of("Return").map(|k| k.0), Some(sym::RETURN));
        let (k, text, m) = key_of("ctrl+c").unwrap();
        assert_eq!((k, text.as_deref(), m.ctrl), (u32::from(b'c'), Some("c"), true));
        assert_eq!(Terminal::key_bytes(k, text.as_deref(), m, false), Some(vec![3]));
        assert_eq!(
            Terminal::key_bytes(key_of("Up").unwrap().0, None, Modifiers::default(), false),
            Some(b"\x1b[A".to_vec())
        );
        assert!(key_of("alt+F4").is_none() && key_of("ctrl+C").is_none() && key_of("").is_none());
        // Every name the wire takes (vergence_protocol::window::KEY_NAMES) is a key here.
        for n in [
            "Return",
            "Tab",
            "Escape",
            "BackSpace",
            "Delete",
            "Up",
            "Down",
            "Left",
            "Right",
            "Home",
            "End",
            "Prior",
            "Next",
            "Insert",
            "space",
        ] {
            assert!(key_of(n).is_some(), "{n}");
        }
    }

    #[test]
    fn a_line_at_the_pace_more_at_once() {
        assert!(paced("echo hi\n"));
        assert!(paced("ls -la"));
        assert!(!paced("one\ntwo\n"), "two lines");
        assert!(!paced(&"x".repeat(PACED_MAX + 1)), "longer than a line");
        assert_eq!(typed_bytes("echo hi\n", true).unwrap(), b"echo hi\r");
        assert_eq!(typed_bytes("a\nb\n", true).unwrap(), b"\x1b[200~a\rb\r\x1b[201~");
        // More than a line to a program that takes no pastes: refused, never
        // typed line by line (the later lines would go to what the first started).
        assert_eq!(typed_bytes("sudo -s\nwhoami\n", false), None);
        // One long line goes at once either way.
        let long = "x".repeat(PACED_MAX + 5) + "\n";
        assert!(typed_bytes(&long, false).unwrap().ends_with(b"\r"));
    }

    #[test]
    fn one_definition_of_a_line_and_no_controls() {
        // "\r\n", "\r" and "\n" are each a line end: "a\rb" is two lines.
        assert_eq!(clean_typed("a\r\nb\rc\n"), "a\nb\nc\n");
        assert!(!paced(&clean_typed("ls\rrm -rf x\r")), "two lines by \\r alone");
        // Escape, other C0 and C1 controls and DEL are left out; tab kept.
        assert_eq!(clean_typed("x\x1b[201~\x1b]52;c;AAAA\x07y\tz\x7f\u{9b}2J"), "x[201~]52;c;AAAAy\tz2J");
        // So a paste can never end its own brackets.
        let bytes = typed_bytes(&clean_typed("x\n\x1b[201~y\n"), true).unwrap();
        assert_eq!(bytes, b"\x1b[200~x\r[201~y\r\x1b[201~");
    }
}
