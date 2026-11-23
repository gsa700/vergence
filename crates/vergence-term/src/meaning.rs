//! Terminal's meaning (terminal agent milestone 1, step 3): what an agent or
//! a screen reader is told about a window's tabs and the terminals in them.
//!
//! The tree:
//!
//!   window   the window (role "window", named as its title)
//!   └ tab    each tab (role "tab", named as its title; "selected" for the
//!   │        one in front; its active terminal's states)
//!     └ terminal   each pane (role "terminal", "pane 1", "pane 2" in the
//!       │          tab's order; value: the visible screen as text; states:
//!       │          "focused" for the tab's active one, "program:<name>",
//!       │          "root", "password", "busy", "exited", "cwd:<path>")
//!       └ log      its scrollback (role "log", "scrollback"; value: the
//!                  lines above the screen, oldest first, the last
//!                  SCROLLBACK_LINES and SCROLLBACK_BYTES at most), read
//!                  whole through paged reading (bus v37): the tree's send
//!                  cuts every value at 512 characters.
//!
//! Every tab publishes its terminals, the ones behind too: the window says
//! "shell-access" and each tab "scope", so the server's grants (step 1) cover
//! one tab at a time and hide the others from an agent.
//!
//! Ids are stable for as long as a tab and a pane live, whatever moves:
//!   window          1
//!   tab             TAB  | tab id            (the id the server's bar knows
//!                                             the tab by; a process-wide
//!                                             counter, never reused)
//!   terminal        PANE | tab id << 20 | pane number   (the number the
//!                                             tab's split tree knows it by)
//!   its scrollback  LOG  | tab id << 20 | pane number
//! with TAB, PANE and LOG in the top bits so the three never meet. A grant
//! for a tab (step 1) names the tab's node id; its terminals and their
//! scrollback are recognized by the tab id in their bits ([`tab_of`]).

use std::collections::HashMap;

use vergence_ui::{MeaningNode, MeaningTree};

use crate::{PAD, Terminal, session::lock};

/// The scrollback published: at most this many lines, and bytes.
pub(crate) const SCROLLBACK_LINES: usize = 10_000;
pub(crate) const SCROLLBACK_BYTES: usize = 1 << 20;

const TAB: u64 = 1 << 60;
const PANE: u64 = 2 << 60;
const LOG: u64 = 3 << 60;
const KIND: u64 = 0xf << 60;
/// A pane's number within its tab, in its id's low bits.
const PANE_BITS: u64 = 20;

pub(crate) const WINDOW_ID: u64 = 1;

pub(crate) fn tab_node(tab: u32) -> u64 {
    TAB | u64::from(tab)
}

pub(crate) fn pane_node(tab: u32, pane: u64) -> u64 {
    PANE | (u64::from(tab) << PANE_BITS) | (pane & ((1 << PANE_BITS) - 1))
}

pub(crate) fn log_node(tab: u32, pane: u64) -> u64 {
    LOG | (u64::from(tab) << PANE_BITS) | (pane & ((1 << PANE_BITS) - 1))
}

/// The tab a node belongs to (its own id for a tab), or None (the window).
#[allow(dead_code)] // the tab-scoped grants (step 1) and actions (step 4) use it
pub(crate) fn tab_of(node: u64) -> Option<u32> {
    match node & KIND {
        TAB => Some((node & !KIND) as u32),
        PANE | LOG => Some(((node & !KIND) >> PANE_BITS) as u32),
        _ => None,
    }
}

/// A pane's scrollback as last made: the size it was made at (lines ever
/// pushed, lines kept, columns) and its text. Made again only when output
/// has added lines, not at every frame.
#[derive(Default)]
pub(crate) struct ScrollbackCache {
    made: HashMap<PaneKey, (ScrollSize, String)>,
}

/// A pane by its tab's id and its number in the tab.
type PaneKey = (u32, u64);
/// A scrollback's size: lines ever pushed, lines kept, columns.
type ScrollSize = (usize, usize, usize);

impl ScrollbackCache {
    fn text(&mut self, key: PaneKey, size: ScrollSize, make: impl FnOnce() -> String) -> String {
        match self.made.get(&key) {
            Some((was, text)) if *was == size => text.clone(),
            _ => {
                let text = make();
                self.made.insert(key, (size, text.clone()));
                text
            }
        }
    }

    /// Forgets the panes that are gone.
    fn keep_only(&mut self, live: &[PaneKey]) {
        self.made.retain(|k, _| live.contains(k));
    }
}

fn node(id: u64, parent: u64, role: &str, name: String, value: Option<String>, states: Vec<String>) -> MeaningNode {
    MeaningNode {
        id,
        parent,
        role: role.into(),
        name,
        value,
        states,
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
        items: Vec::new(),
    }
}

/// What a window's tree is made from, cheaply: the window's size and front
/// tab; each tab's id and title; each pane's output count, size and states.
/// The tree is made again only when this changes.
#[derive(PartialEq)]
pub(crate) struct MeaningKey {
    size: (u32, u32),
    shown: usize,
    tabs: Vec<(u32, String, u64, Vec<(u64, u64, (usize, usize, usize, usize, bool), Vec<String>)>)>,
}

impl Terminal {
    pub(crate) fn meaning_key(&self) -> MeaningKey {
        MeaningKey {
            size: (self.width as u32, self.height as u32),
            shown: self.shown,
            tabs: self
                .tabs
                .iter()
                .map(|t| {
                    let panes = t
                        .panes
                        .iter()
                        .map(|(p, s)| {
                            let (out, grid) = {
                                let sc = lock(&s.screen);
                                (sc.output, (sc.grid.cols, sc.grid.rows, sc.grid.cx, sc.grid.cy, sc.grid.cursor_visible))
                            };
                            (*p, out, grid, s.state().states())
                        })
                        .collect();
                    (t.id, t.label(), t.active, panes)
                })
                .collect(),
        }
    }

    /// The window's meaning now (the toolkit sends it when it differs from
    /// the last one: main.rs's `meaning`).
    pub(crate) fn meaning_now(&mut self) -> MeaningTree {
        let (cw, ch) = self.metrics.as_ref().map_or((0.0, 0.0), |m| (m.cell_w, m.cell_h));
        let room = self.room();
        let title = self.tabs.get(self.shown).map_or_else(|| "Terminal".to_string(), |t| t.label());
        let mut nodes = vec![MeaningNode {
            x: 0.0,
            y: 0.0,
            w: self.width,
            h: self.height,
            // "shell-access": the server asks for this window's grants a tab at
            // a time, never Always (terminal agent milestone 1, step 1).
            ..node(WINDOW_ID, 0, "window", title, None, {
                let mut states = vec!["shell-access".to_string()];
                // A terminal of an agent's own: the server gives that agent its tab.
                if let Some(name) = &self.for_agent {
                    states.push(format!("owner:{name}"));
                }
                states
            })
        }];
        let mut focus = 0;
        let mut caret = None;
        let mut live = Vec::new();
        for (index, tab) in self.tabs.iter().enumerate() {
            let selected = index == self.shown;
            let tab_id = tab_node(tab.id);
            // "scope": a grant covers this tab and what is under it, nothing else.
            let mut tab_states = vec!["scope".to_string()];
            if selected {
                tab_states.push("selected".to_string());
            }
            if let Some(active) = tab.session(tab.active) {
                tab_states.extend(active.state().states());
            }
            nodes.push(node(tab_id, WINDOW_ID, "tab", tab.label(), None, tab_states));
            let areas: HashMap<u64, vergence_scene::Rect> = tab.tree.layout(room).0.into_iter().collect();
            for (number, (pane, session)) in tab.panes.iter().enumerate() {
                live.push((tab.id, *pane));
                let mut states = session.state().states();
                let active = *pane == tab.active;
                if active {
                    states.insert(0, "focused".to_string());
                }
                let (screen, size, cursor) = {
                    let s = lock(&session.screen);
                    let g = &s.grid;
                    (g.text(), (g.pushed, g.scrollback.len(), g.cols), (g.cx, g.cy, g.cursor_visible))
                };
                let area = areas.get(pane).copied().unwrap_or(vergence_scene::Rect::new(0.0, 0.0, 0.0, 0.0));
                let id = pane_node(tab.id, *pane);
                nodes.push(MeaningNode {
                    x: area.x,
                    y: area.y,
                    w: area.w,
                    h: area.h,
                    ..node(id, tab_id, "terminal", format!("pane {}", number + 1), Some(screen), states)
                });
                let scrollback = self.scrollback_cache.text((tab.id, *pane), size, || {
                    lock(&session.screen).grid.scrollback_text(SCROLLBACK_LINES, SCROLLBACK_BYTES)
                });
                nodes.push(node(log_node(tab.id, *pane), id, "log", "scrollback".into(), Some(scrollback), Vec::new()));
                if selected && active {
                    focus = id;
                    if cursor.2 && cw > 0.0 {
                        caret =
                            Some((area.x + PAD + cursor.0 as f32 * cw, area.y + PAD + cursor.1 as f32 * ch, cw, ch));
                    }
                }
            }
        }
        self.scrollback_cache.keep_only(&live);
        MeaningTree { focus, caret, nodes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_name_their_tab_and_never_meet() {
        let (tab, pane, log) = (tab_node(7), pane_node(7, 3), log_node(7, 3));
        assert_eq!(tab_of(tab), Some(7));
        assert_eq!(tab_of(pane), Some(7));
        assert_eq!(tab_of(log), Some(7));
        assert_eq!(tab_of(WINDOW_ID), None);
        assert!(tab != pane && pane != log && tab != log);
        // Another tab's panes are told apart, however the numbers go.
        assert_ne!(pane_node(7, 3), pane_node(8, 3));
        assert_ne!(pane_node(7, 3), pane_node(7, 4));
        assert_eq!(tab_of(pane_node(u32::MAX, 1)), Some(u32::MAX));
    }

    /// What a pane is doing, as its states say it.
    #[test]
    fn a_pane_s_states() {
        use crate::session::PaneState;
        let shell = PaneState { program: "zsh".into(), cwd: Some("/home/me".into()), ..Default::default() };
        assert_eq!(shell.states(), vec!["program:zsh", "cwd:/home/me"]);
        let sudo = PaneState { program: "sudo".into(), root: true, password: true, busy: true, ..Default::default() };
        assert_eq!(sudo.states(), vec!["program:sudo", "root", "password", "busy"]);
        let gone = PaneState { exited: true, ..Default::default() };
        assert_eq!(gone.states(), vec!["exited"]);
    }

    #[test]
    fn scrollback_is_made_again_only_when_it_grew() {
        let mut c = ScrollbackCache::default();
        let mut made = 0;
        let make = |n: &mut i32| {
            *n += 1;
            format!("text {n}")
        };
        assert_eq!(c.text((1, 1), (10, 10, 80), || make(&mut made)), "text 1");
        assert_eq!(c.text((1, 1), (10, 10, 80), || make(&mut made)), "text 1", "unchanged: kept");
        assert_eq!(c.text((1, 1), (11, 11, 80), || make(&mut made)), "text 2", "a line more: made again");
        c.keep_only(&[]);
        assert_eq!(c.text((1, 1), (11, 11, 80), || make(&mut made)), "text 3", "a closed pane forgotten");
    }
}
