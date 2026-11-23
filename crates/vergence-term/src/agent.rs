//! An agent typing in a terminal (terminal agent milestone 1, step 4; bus v38).
//!
//! The server lets an agent type only in a tab it holds a typing grant for
//! (step 1); the terminal adds what only it can see, at the moment of typing:
//!
//! - a pane at a password prompt (line mode with echo off) refuses: "Terminal
//!   is asking for a password: the person types it";
//! - a pane whose foreground runs as root refuses;
//! - a pane the person typed in during the last second refuses ("the person
//!   is typing"), and a keystroke of the person's during an agent's burst
//!   drops the rest of it; the agent hears so at its next act.
//!
//! What it types goes in as typing does, at a visible pace: up to a line
//! (PACED_MAX characters, no line end but a last one) over PACE; anything
//! longer at once, as a paste (bracketed where the program asked). A key goes
//! in after whatever is still being typed in that pane.
//!
//! While an agent types in a tab, and for MARK_FOR after, the tab's title in
//! the bar starts with its name ("⌨ Claude · "), and a mark in the pane's
//! top-right corner says "Claude is typing · stop": a click on it ends the
//! agent's grant for the tab (WinRequest::RevokeAgent), as does Escape twice
//! within DOUBLE_ESCAPE in that tab. The first Escape goes to the program as
//! always (it may be meant for vim); the second is the stop, and is not sent.
//! The terminal then notes in the pane "<agent>'s access to this tab ended".

use std::collections::{HashMap, HashSet, VecDeque};
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
    /// By tab: an agent that has typed there and not been stopped (the stop
    /// keys are for it).
    holds: HashMap<u32, String>,
    bursts: VecDeque<Burst>,
    /// By pane: the person's last keystroke.
    person: HashMap<(u32, u64), Instant>,
    /// Panes whose burst the person cut short (told at the agent's next act).
    cut: HashSet<(u32, u64)>,
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
pub(crate) fn refusal(
    state: &crate::session::PaneState,
    person_typed: Option<Instant>,
    cut: bool,
) -> Option<&'static str> {
    if state.exited {
        Some("the program in this pane has ended")
    } else if state.root {
        Some("this tab runs as root: agents may not type here")
    } else if state.password {
        Some("Terminal is asking for a password: the person types it")
    } else if cut {
        Some("the person typed: the rest of what you typed last was not typed; read the screen")
    } else if person_typed.is_some_and(|t| t.elapsed() < PERSON_QUIET) {
        Some("the person is typing")
    } else {
        None
    }
}

/// Whether typed text goes at the pace (up to a line) or at once.
pub(crate) fn paced(text: &str) -> bool {
    let body = text.strip_suffix('\n').unwrap_or(text);
    text.chars().count() <= PACED_MAX && !body.contains('\n')
}

/// The bytes typed text sends: line ends as the Return key; a long text as a
/// paste (bracketed when the program asked, and unable to end its own brackets).
pub(crate) fn typed_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if paced(&text.replace('\r', "\n")) || !bracketed {
        text.into_bytes()
    } else {
        format!("\x1b[200~{}\x1b[201~", text.replace("\x1b[201~", "")).into_bytes()
    }
}

impl Terminal {
    /// The server says who is about to act at `node`.
    pub(crate) fn on_agent_acting(&mut self, node: u64, agent: &str) {
        self.agents.acting = Some((node, agent.to_string()));
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
        let session = tab.session(pane).ok_or("no such pane now")?;
        let agent = match self.agents.acting.take() {
            Some((n, a)) if n == node => a,
            _ => "An agent".to_string(),
        };
        // What only the terminal sees, now, not as the last tree said.
        let state = session.state();
        let cut = self.agents.cut.remove(&(tab_id, pane));
        if let Some(why) = refusal(&state, self.agents.person.get(&(tab_id, pane)).copied(), cut) {
            return Err(why.into());
        }
        let (bracketed, app_cursor) = {
            let s = lock(&session.screen);
            (s.grid.bracketed_paste, s.grid.app_cursor)
        };
        let bytes = match action {
            Action::Type(text) => typed_bytes(text, bracketed),
            Action::Key(name) => {
                let (k, text, m) = key_of(name).ok_or_else(|| format!("no key {name:?}"))?;
                Terminal::key_bytes(k, text.as_deref(), m, app_cursor).ok_or_else(|| format!("no key {name:?}"))?
            }
            _ => unreachable!(),
        };
        let now = Instant::now();
        self.agents.typing.insert(tab_id, (agent.clone(), now));
        self.agents.holds.insert(tab_id, agent);
        let busy = self.agents.bursts.iter().any(|b| b.tab == tab_id && b.pane == pane);
        let pace = matches!(action, Action::Type(t) if paced(t));
        if pace || busy {
            let step = if pace {
                bytes.len().div_ceil((PACE.as_millis() / STEP.as_millis()) as usize).max(1)
            } else {
                bytes.len()
            };
            self.agents.bursts.push_back(Burst { tab: tab_id, pane, bytes: bytes.into(), step });
        } else {
            self.write_pane(tab_id, pane, &bytes);
        }
        self.agent_timer(window);
        self.bar_told = None;
        window.redraw();
        Ok(())
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

    /// The pace's timer: each burst a step further; the marks that are due go.
    pub(crate) fn on_agent_timer(&mut self, window: &mut Window) {
        self.agents.timer = false;
        // One step of the first burst of each pane, in order.
        let mut seen: Vec<(u32, u64)> = Vec::new();
        let mut writes: Vec<(u32, u64, Vec<u8>)> = Vec::new();
        for b in self.agents.bursts.iter_mut() {
            if seen.contains(&(b.tab, b.pane)) {
                continue;
            }
            seen.push((b.tab, b.pane));
            let n = b.step.min(b.bytes.len());
            // Never cut a character in two.
            let mut chunk: Vec<u8> = b.bytes.drain(..n).collect();
            while let Some(&next) = b.bytes.front()
                && next & 0xc0 == 0x80
            {
                chunk.push(next);
                b.bytes.pop_front();
            }
            writes.push((b.tab, b.pane, chunk));
        }
        self.agents.bursts.retain(|b| !b.bytes.is_empty());
        for (tab, pane, bytes) in writes {
            self.write_pane(tab, pane, &bytes);
            if let Some(t) = self.agents.typing.get_mut(&tab) {
                t.1 = Instant::now();
            }
        }
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
                    self.agents.cut.insert((tab_id, pane));
                }
                let plain = !modifiers.ctrl && !modifiers.alt && !modifiers.shift && !modifiers.logo;
                if *key == sym::ESCAPE && plain && self.agents.holds.contains_key(&tab_id) {
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

    /// The person stopped the agent in this tab: the server ends its grant,
    /// its typing stops, and the pane says so.
    fn agent_stop(&mut self, tab: u32, window: &mut Window) {
        let agent = self.agents.holds.remove(&tab).unwrap_or_else(|| "The agent".into());
        self.agents.typing.remove(&tab);
        self.agents.bursts.retain(|b| b.tab != tab);
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

    /// The tab's title in the bar: with the typing agent's name first.
    pub(crate) fn agent_title(&self, tab: u32, title: String) -> String {
        match self.agents.typing.get(&tab) {
            Some((agent, _)) => format!("\u{2328} {agent} \u{b7} {title}"),
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
        assert_eq!(refusal(&shell, None, false), None);
        let pw = PaneState { password: true, ..shell.clone() };
        assert_eq!(refusal(&pw, None, false), Some("Terminal is asking for a password: the person types it"));
        let root = PaneState { root: true, ..shell.clone() };
        assert_eq!(refusal(&root, None, false), Some("this tab runs as root: agents may not type here"));
        // Root wins over a password prompt (sudo -s asking): either way refused.
        assert!(refusal(&PaneState { root: true, password: true, ..shell.clone() }, None, false).is_some());
        assert_eq!(refusal(&shell, Some(Instant::now()), false), Some("the person is typing"));
        assert_eq!(refusal(&shell, Some(Instant::now() - PERSON_QUIET * 2), false), None, "quiet long enough");
        assert!(refusal(&shell, None, true).unwrap().starts_with("the person typed"));
        assert!(refusal(&PaneState { exited: true, ..Default::default() }, None, false).is_some());
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
        assert_eq!(typed_bytes("echo hi\n", true), b"echo hi\r");
        assert_eq!(typed_bytes("a\nb\n", false), b"a\rb\r");
        assert_eq!(typed_bytes("a\nb\n", true), b"\x1b[200~a\rb\r\x1b[201~");
        // A paste cannot end its own brackets.
        assert_eq!(typed_bytes("x\n\x1b[201~y\n", true), b"\x1b[200~x\ry\r\x1b[201~");
    }
}
