//! One terminal and its tab: the screen the parsing thread and the window
//! share, the session that starts the program and reads it on a thread of
//! its own, and a tab's terminals. The window itself is in main.rs.

use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use vergence_scene::Rect;
use vergence_ui::Waker;

use crate::grid::{Cell, Grid};
use crate::panes::Node;
use crate::scrollbar::Scrollbar;
use crate::select::{Pos, Selection};
use crate::settings::SCROLL_ON_OUTPUT;
use crate::{PAD, program, pty, vt};

/// What the parser's thread and the window's thread share.
pub(crate) struct Screen {
    pub(crate) grid: Grid,
    pub(crate) parser: vt::Parser,
    /// Lines the view is scrolled back from the live screen.
    pub(crate) scroll: usize,
    /// The program has ended.
    pub(crate) exited: bool,
    /// Text the program copied (OSC 52), on its way to the clipboard.
    pub(crate) copied: Option<String>,
    /// The text selected with the mouse.
    pub(crate) selection: Option<Selection>,
    /// Output read so far, by pieces: the meaning tree is made again when it moves.
    pub(crate) output: u64,
}

impl Screen {
    /// The alternate screen has no scrollback to show; else up to all of it.
    pub(crate) fn clamp_scroll(&mut self) {
        let most = if self.grid.alt { 0 } else { self.grid.scrollback.len() };
        self.scroll = self.scroll.min(most);
    }

    pub(crate) fn scroll_by(&mut self, lines: isize) {
        self.scroll = (self.scroll as isize).saturating_add(lines).max(0) as usize;
        self.clamp_scroll();
    }

    /// The line shown at row `r` (from the scrollback while scrolled back).
    pub(crate) fn shown(&self, r: usize) -> &[Cell] {
        self.grid.abs_line(self.top_line() + r).unwrap_or(&[])
    }

    /// The scrollbar of a terminal that has this room, if there is anything
    /// to scroll (the alternate screen has no scrollback).
    pub(crate) fn scrollbar(&self, area: Rect) -> Option<Scrollbar> {
        let back = if self.grid.alt { 0 } else { self.grid.scrollback.len() };
        Scrollbar::new(area, back, self.grid.rows, self.scroll)
    }

    /// The absolute number of the line shown in the top row.
    pub(crate) fn top_line(&self) -> usize {
        self.grid.pushed - self.scroll
    }

    /// The cell under a point of the window, as a selection counts it, for
    /// a terminal in `area` with cells of `cell` (width, height); points
    /// outside the cells count as the nearest one.
    pub(crate) fn cell_at(&self, x: f32, y: f32, area: Rect, cell: (f32, f32)) -> Pos {
        let (col, row) = self.screen_cell(x, y, area, cell);
        (col, self.top_line() + row)
    }

    /// The column and row of the screen under a point of the window (what
    /// mouse reporting tells the program); the nearest, for points outside.
    pub(crate) fn screen_cell(&self, x: f32, y: f32, area: Rect, cell: (f32, f32)) -> (usize, usize) {
        let col = ((x - area.x - PAD) / cell.0).floor().max(0.0) as usize;
        let row = ((y - area.y - PAD) / cell.1).floor().max(0.0) as usize;
        (col.min(self.grid.cols - 1), row.min(self.grid.rows - 1))
    }

    /// Moves the end of the selection being dragged; true if that changed it.
    pub(crate) fn drag_selection(&mut self, pos: Pos) -> bool {
        let Some(mut selection) = self.selection else { return false };
        selection.drag(&self.grid, pos);
        let changed = self.selection != Some(selection);
        self.selection = Some(selection);
        changed
    }
}

pub(crate) fn lock(screen: &Mutex<Screen>) -> MutexGuard<'_, Screen> {
    // A panic on the other thread must not take this one with it.
    screen.lock().unwrap_or_else(|e| e.into_inner())
}

/// One terminal: a program, its screen, and the thread that reads it.
pub(crate) struct Session {
    pub(crate) screen: Arc<Mutex<Screen>>,
    pub(crate) pty: pty::Pty,
    pub(crate) title: String,
    /// What the parsing thread has to say since the window last looked
    /// ([`NEWS_DRAW`], [`NEWS_STATE`]), kept outside the screen's lock: a
    /// terminal in a flood holds that nearly all the time, and the window
    /// would wait for it at every wake only to learn "draw me".
    pub(crate) news: Arc<AtomicU8>,
    /// The foreground process group last looked at, whether it runs as the
    /// superuser, and when that was asked (look milestone 1: the red tab).
    root_seen: std::cell::Cell<Option<(u32, bool, Instant)>>,
    /// Started with a command (`-e`), not the shell: the program is the
    /// pty's own process, and running while it lives.
    command: bool,
    /// The foreground's working directory, the group it was read for and
    /// when (FreeBSD runs procstat for it: at most every two seconds).
    cwd_seen: std::cell::RefCell<Option<(u32, String, Instant)>>,
    /// Its state as last read for the meaning tree, and when: read at most
    /// every STATE_EVERY there (each read is several system calls a pane,
    /// and the tree's key is made at every frame: the security review).
    state_seen: std::cell::RefCell<Option<(PaneState, Instant)>>,
}

/// How often the meaning tree reads a pane's state afresh.
const STATE_EVERY: Duration = Duration::from_millis(250);

/// Programs an agent may never type into, whatever their terminal modes say:
/// a remote shell (its password prompt is on the far side, where this pane's
/// modes do not show it), a multiplexer (its panes are other terminals the
/// grant never named), a program that makes its user someone else, and a
/// PIN entry.
pub(crate) const NO_AGENT_PROGRAMS: [&str; 13] = [
    "ssh",
    "mosh",
    "mosh-client",
    "telnet",
    "tmux",
    "screen",
    "su",
    "sudo",
    "doas",
    "login",
    "passwd",
    "ksu",
    "pkexec",
];

/// Whether an agent may not type to this foreground program (by name; any
/// pinentry, and names as the system cuts them).
pub(crate) fn no_agent_program(name: &str) -> bool {
    let name = name.rsplit('/').next().unwrap_or(name);
    NO_AGENT_PROGRAMS.contains(&name) || name.starts_with("pinentry")
}

/// Whether the text before the cursor reads as a password prompt ("Password:",
/// "Enter passphrase for key '...':", "[sudo] password for dave:", "PIN:"),
/// whatever the terminal's modes say (a program that reads a password in raw
/// mode, as some do, has echo off but no line mode).
pub(crate) fn password_line(line: &str) -> bool {
    let l = line.trim_end().to_lowercase();
    if !(l.ends_with(':') || l.ends_with("):") || l.ends_with('>')) {
        return false;
    }
    let words: Vec<&str> = l.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    ["password", "passphrase", "passcode", "pin", "otp", "passwort", "contraseña"].iter().any(|w| words.contains(w))
        || l.contains("verification code")
        || l.contains("one-time")
        || l.contains("mot de passe")
}

/// What a terminal is doing, for its meaning (terminal agent milestone 1).
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct PaneState {
    /// The foreground program's name ("zsh", "vim", "sudo").
    pub(crate) program: String,
    /// It runs as the superuser (the red tab).
    pub(crate) root: bool,
    /// Typed input is not echoed: a password is being asked.
    pub(crate) password: bool,
    /// A program other than the shell is in front.
    pub(crate) busy: bool,
    /// Where the foreground program works, if it can be read.
    pub(crate) cwd: Option<String>,
    /// The program has ended (the pane says so and waits to be closed).
    pub(crate) exited: bool,
}

impl PaneState {
    /// As the meaning tree's states: "program:<name>", "root", "password",
    /// "busy", "exited", "cwd:<path>".
    pub(crate) fn states(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.program.is_empty() {
            out.push(format!("program:{}", self.program));
        }
        for (on, word) in
            [(self.root, "root"), (self.password, "password"), (self.busy, "busy"), (self.exited, "exited")]
        {
            if on {
                out.push(word.to_string());
            }
        }
        if let Some(cwd) = &self.cwd {
            out.push(format!("cwd:{cwd}"));
        }
        out
    }
}

/// The screen changed: a frame is wanted if this terminal is in front. Up
/// from the first output after a frame until the next frame draws it.
pub(crate) const NEWS_DRAW: u8 = 1;
/// The title, the clipboard or the program's life changed: look into the screen.
pub(crate) const NEWS_STATE: u8 = 2;

impl Session {
    /// Starts the user's shell, or `command` (`-e`), on a new terminal of
    /// cols x rows. Its output is read and parsed on a thread of its own,
    /// which wakes the window.
    pub(crate) fn start(
        cols: usize,
        rows: usize,
        waker: Waker,
        command: Option<Vec<String>>,
        cwd: Option<&str>,
    ) -> std::io::Result<Session> {
        let is_command = matches!(command.as_deref(), Some([_, ..]));
        let pty = match command.as_deref() {
            Some([program, args @ ..]) => pty::Pty::spawn(program, args, cols as u16, rows as u16, cwd)?,
            _ => pty::Pty::spawn(
                &std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
                &[],
                cols as u16,
                rows as u16,
                cwd,
            )?,
        };
        let reader = pty.reader()?;
        let mut replies = reader.try_clone()?;
        let screen = Arc::new(Mutex::new(Screen {
            grid: Grid::new(cols, rows),
            parser: vt::Parser::new(),
            scroll: 0,
            exited: false,
            copied: None,
            selection: None,
            output: 0,
        }));
        let shared = screen.clone();
        let news = Arc::new(AtomicU8::new(0));
        let said = news.clone();
        // Named, so that a look at the threads says which is parsing.
        std::thread::Builder::new().name("parse".into()).spawn(move || {
            pty::read_loop(reader, |bytes| {
                let (out, bits) = {
                    let mut guard = lock(&shared);
                    let s = &mut *guard;
                    let (before, alt) = (s.grid.pushed, s.grid.alt);
                    let title = s.grid.title.clone();
                    s.parser.advance(&mut s.grid, &bytes);
                    s.output = s.output.wrapping_add(1);
                    let mut bits = if std::mem::take(&mut s.grid.dirty) { NEWS_DRAW } else { 0 };
                    if s.grid.title != title {
                        bits |= NEWS_STATE;
                    }
                    if s.grid.alt != alt {
                        // Another screen: what was selected is not on it.
                        s.selection = None;
                    }
                    if s.grid.pushed < before {
                        // A full reset: the line numbers started again.
                        s.selection = None;
                        s.scroll = 0;
                    } else if s.scroll > 0 && SCROLL_ON_OUTPUT.load(Ordering::Relaxed) && s.grid.pushed != before {
                        // Scrolled back, with "Scroll on output" on: new lines bring the view back down.
                        s.scroll = 0;
                    } else if s.scroll > 0 {
                        // Scrolled back: stay on the same text as new lines arrive.
                        s.scroll += s.grid.pushed - before;
                    }
                    s.clamp_scroll();
                    if let Some(text) = s.grid.clipboard.take() {
                        s.copied = Some(text);
                        bits |= NEWS_STATE;
                    }
                    (std::mem::take(&mut s.grid.out), bits)
                };
                if !out.is_empty() {
                    let _ = replies.write_all(&out);
                }
                // The window is woken for news it has not been told already: "draw"
                // stays up until a frame has been drawn, so a flood wakes it once a
                // frame, not once for every piece read.
                if bits & !said.fetch_or(bits, Ordering::SeqCst) != 0 {
                    waker.wake();
                }
                true
            });
            lock(&shared).exited = true;
            said.fetch_or(NEWS_STATE, Ordering::SeqCst);
            waker.wake();
        })?;
        Ok(Session {
            screen,
            pty,
            title: String::new(),
            news,
            root_seen: std::cell::Cell::new(None),
            command: is_command,
            cwd_seen: std::cell::RefCell::new(None),
            state_seen: std::cell::RefCell::new(None),
        })
    }

    fn label(&self) -> String {
        if self.title.is_empty() { "Terminal".into() } else { self.title.clone() }
    }

    /// The program the shell is running, if it is running one: the
    /// terminal's foreground process group is then not the shell's own
    /// (settings milestone 2: closing asks first). Its name, or "a program".
    fn running(&self) -> Option<String> {
        if lock(&self.screen).exited {
            return None;
        }
        let group = self.pty.foreground()?;
        // A command's own process counts; a shell's, only another group.
        (self.command || group != self.pty.pid()).then(|| program::name_of(group).unwrap_or_else(|| "a program".into()))
    }

    /// Whether the program the user is typing to runs as the superuser
    /// (sudo -s, su, sudo's own password prompt, sudo make install while it
    /// runs, a script's sudo). The foreground group is asked each time (a
    /// system call); whether any of its processes is the superuser when the
    /// group changes, or every two seconds while it does not (Linux reads
    /// /proc; FreeBSD runs one `ps`, so at most one every two seconds a pane).
    /// A terminal started as root is red throughout.
    fn as_root(&self) -> bool {
        // Every process of the group counts, the shell's own group included: a
        // script leads the group the sudo it runs is in.
        let Some(group) = self.pty.foreground() else { return false };
        if let Some((seen, root, at)) = self.root_seen.get()
            && seen == group
            && at.elapsed() < Duration::from_secs(2)
        {
            return root;
        }
        let root = program::group_has_root(group);
        self.root_seen.set(Some((group, root, Instant::now())));
        root
    }

    /// Its state for the meaning tree: as read in the last STATE_EVERY.
    pub(crate) fn state_recent(&self) -> PaneState {
        if let Some((st, at)) = self.state_seen.borrow().as_ref()
            && at.elapsed() < STATE_EVERY
        {
            return st.clone();
        }
        let st = self.state();
        *self.state_seen.borrow_mut() = Some((st.clone(), Instant::now()));
        st
    }

    /// Its state for an agent's typing, at the moment of an act: whether the
    /// foreground runs as the superuser asked afresh (not the two seconds'
    /// answer the red tab uses), and a password prompt by its text too.
    pub(crate) fn state_for_typing(&self) -> PaneState {
        let mut st = self.state();
        if !st.exited {
            st.root = self.pty.foreground().is_some_and(program::group_has_root);
            self.root_seen.set(self.pty.foreground().map(|g| (g, st.root, Instant::now())));
            st.password |= password_line(&lock(&self.screen).grid.cursor_line_text());
        }
        st
    }

    /// Its state between the steps of a paced burst: the superuser asked
    /// again whenever the foreground changes (else every two seconds), the
    /// password prompt by the modes and the text.
    pub(crate) fn state_between_steps(&self) -> PaneState {
        let mut st = self.state();
        if !st.exited {
            st.password |= password_line(&lock(&self.screen).grid.cursor_line_text());
        }
        st
    }

    /// What it is doing now, for its meaning.
    pub(crate) fn state(&self) -> PaneState {
        let exited = lock(&self.screen).exited;
        if exited {
            return PaneState { exited: true, ..Default::default() };
        }
        let group = self.pty.foreground();
        let busy = self.running().is_some();
        let program = group.and_then(program::name_of).unwrap_or_default();
        let cwd = group.and_then(|g| {
            let mut seen = self.cwd_seen.borrow_mut();
            if let Some((was, cwd, at)) = seen.as_ref()
                && *was == g
                && at.elapsed() < Duration::from_secs(2)
            {
                return Some(cwd.clone());
            }
            let cwd = program::cwd_of(g).or_else(|| program::cwd_of(self.pty.pid()))?;
            *seen = Some((g, cwd.clone(), Instant::now()));
            Some(cwd)
        });
        PaneState {
            program,
            root: self.as_root(),
            password: self.pty.asks_password() == Some(true),
            busy,
            cwd,
            exited: false,
        }
    }

    /// Makes the terminal's grid cols x rows, if it is not, and tells its program.
    pub(crate) fn fit(&self, s: &mut Screen, cols: usize, rows: usize) {
        if (cols, rows) == (s.grid.cols, s.grid.rows) {
            return;
        }
        if cols != s.grid.cols {
            // Lines are cut or padded: the selection's cells are not what they were.
            s.selection = None;
        }
        s.grid.resize(cols, rows);
        s.clamp_scroll();
        self.pty.resize(cols as u16, rows as u16);
    }
}

/// A tab: its room divided among one or more terminals.
pub(crate) struct Tab {
    /// Its name to the server, which draws the bar on the native road (bar milestone 1).
    pub(crate) id: u32,
    pub(crate) tree: Node,
    /// The terminals, by the numbers the tree knows them by.
    pub(crate) panes: Vec<(u64, Session)>,
    /// The one the keys go to.
    pub(crate) active: u64,
}

impl Tab {
    pub(crate) fn session(&self, id: u64) -> Option<&Session> {
        self.panes.iter().find(|(own, _)| *own == id).map(|(_, s)| s)
    }

    pub(crate) fn session_mut(&mut self, id: u64) -> Option<&mut Session> {
        self.panes.iter_mut().find(|(own, _)| *own == id).map(|(_, s)| s)
    }

    /// The tab is called what its active terminal is.
    /// A pane of the tab runs its program as the superuser.
    pub(crate) fn as_root(&self) -> bool {
        self.panes.iter().any(|(_, s)| s.as_root())
    }

    pub(crate) fn label(&self) -> String {
        self.session(self.active).map_or_else(|| "Terminal".into(), Session::label)
    }

    /// The programs its panes' shells are running.
    pub(crate) fn running(&self) -> Vec<String> {
        self.panes.iter().filter_map(|(_, s)| s.running()).collect()
    }
}
