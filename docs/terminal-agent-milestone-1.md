# Terminal agent milestone 1: Terminal on the agent interface

*Drafted 2026-11-22 with Dave's answers.* The roadmap's "The terminal on the
agent interface" (docs/apps-roadmap.md): today vergence-term publishes no
meaning (`vagent windows` lists it as "(no meaning)"), so an agent can neither
read a terminal nor type into one, and work like writing a USB stick goes over
ssh where Dave cannot watch it. Dave: "the whole point of the socket is so the
user sees what you are doing and can interact with you graphically ... You
need to be able to read it." Builds on meaning milestone 1
(docs/meaning-milestone-1.md: the tree, the server's grants per agent and per
app, the server-drawn dialog).

## Dave's answers (2026-11-22)

1. **A password prompt refuses typing.** While the terminal's input does not
   echo (a `sudo` or `ssh` password, `read -s`), an agent may not type; it is
   told "Terminal is asking for a password: the person types it".
2. **Never "Always" for Terminal.** Its dialog offers "Just now" and "For an
   hour" (and Don't allow); a remembered Always from before is not honored.
3. **Scrollback within the same permission** as the screen (read level).
4. **Root tabs refuse typing.** A tab whose foreground runs as root (`sudo -s`,
   already red) takes no agent keys.
5. **No all-or-nothing** (Dave: "one thing we ran into on macOS was ... it was
   ALL or NOTHING, that we should avoid"):
   - **Scoped to a tab.** A grant covers one tab, never the whole Terminal.
     The easy way: the agent asks for a tab of its own, labeled with its name,
     and only that tab exists for it. Working in one of the person's tabs
     needs that tab granted by itself. Other tabs and windows are not in the
     agent's tree at all.
   - **Read and type apart.** "May read this tab" is its own, smaller yes;
     typing is the larger one and includes reading.

## The design

**What a tab publishes** (the meaning tree, per tab the agent may see):
- role `terminal`, name = the tab's title; states: the foreground program
  (`zsh`, `vim`, `sudo`), `root` when it runs as root, `password` while input
  does not echo, `busy` while a program other than the shell runs;
- the working directory (the terminal knows it, program.rs `cwd_of`);
- value = the visible screen as text, with the cursor's row and column;
- the scrollback, read in pages (below), not in the tree's value.

**Reading long text in pages.** The tree cuts a value at 512 characters (a
known rough edge, also Edit's). New agent request: read a node's text from an
offset, a few kilobytes at a time; Terminal answers for the screen and the
scrollback, Edit for its document. Frames stay under their 64 KB.

**What an agent may do** (type level): type text; press named keys (Enter,
Tab, Ctrl+C, Ctrl+D, arrows, Escape); open its own tab ("Claude", the agent's
name, in the window it names or a new one); close its own tab.

**The person stays in charge:**
- What the agent types goes in as typing does, at a visible pace (a short
  burst for a line, not one character a second), and shows on screen as it
  goes.
- While an agent types in a tab, the tab carries a badge with its name.
- Any key the person presses in that tab stops the agent's typing there at
  once (the agent is told "the person is typing").
- A stop key revokes that agent's grant for the tab at once (Q1 below).
- The refusals: password prompt (answer 1), root tab (answer 4), a tab not
  granted (answer 5); each is an error the agent sees, in plain words.

**The dialog** (server-drawn, as now), for Terminal: "Claude wants to read
this tab" or "Claude wants to type in this tab: anything it types runs as
you", naming the tab; buttons Don't allow, Just now, For an hour.

## Steps

1. Grants with a scope and a level: the server's grant (agent, app) gains a
   node scope (a tab's id, or the whole app for every other app, as now) and
   a level (read, act); the dialog's words and buttons by app (Terminal: no
   Always). Unit tests for matching a request to a grant.
2. Paged reading: the protocol request and the toolkit's answer; Edit's
   document through it (the 512-character rough edge closed).
3. Terminal's tree: tabs and panes as nodes, the screen as value, the states
   (root, password from the pty's echo flag, foreground program), the cwd.
4. Terminal's actions: type and keys into a pane through the same path as the
   keyboard, at a visible pace; the agent's own tab; the badge; the person's
   key interrupting; the refusals.
5. vagent and the MCP server: `read`, `type`, `key`, `tab` for terminals;
   agent-check gains a terminal part.
6. Checks: unit tests, a display-suite scenario (a headless terminal works
   since c8ee629): an agent's tab opened, a command typed and read back, a
   refusal at a password prompt, the badge in a shot; tests on Hubble,
   techbench, kepler; an independent review (it is shell access).

## Dave's answers to the last three (2026-11-22)

"1 both, 2 your lean, 3 its own window." So:
1. **The stop**: Escape twice in quick succession *and* a click on the badge,
   either revoking that agent's grant for the tab at once.
2. **The pace**: a typed line appears over about a quarter of a second; a
   long paste goes in at once.
3. **The agent's own tab opens in a window of its own**, never among the
   person's tabs (the window titled with the agent's name).

All questions answered: ready to build.

## Progress

- **Paged reading** (plan step 2, built first; bus v37), 2026-11-22.
  `AgentRequest::Read` / `AgentEvent::Text` for agents,
  `WinEvent::ReadText` / `WinRequest::TextPage` for windows; pages of at
  most 8,192 characters (`window::TEXT_PAGE_MAX`), counted in characters,
  never split. The toolkit keeps each window's last whole tree and answers
  from it; the server checks the app as for a tree read, refuses a secret
  field, a gone widget or one that changed (role/name), logs the read, and
  fails a read the window does not answer in 10 s or whose window goes.
  `vagent read <client> <window> <node> [offset] [len]` (whole text, or one
  page) and the MCP `read` tool. Edit's text area now publishes the text
  alone (it ended in "(line l, column c)", which a reader got back as part
  of the document; the caret stays the tree's caret) and whole up to 4 MB
  (64 KB before: past it there was nothing to read). Checked in a headless
  session on Hubble: a 4,260-character file with é, ü, ï read back exactly
  in pages, one page from offset 1000, a gone node refused.
- **Terminal's tree** (plan step 3), 2026-11-22. **In git, not installed**:
  until the tab-scoped grants (step 1) exist, an app-wide grant would read
  every tab, so no box runs it. crates/vergence-term/src/meaning.rs: window
  > tab (each one, "selected" for the one in front, its active pane's
  states) > terminal ("pane N", the visible screen as value, "focused",
  "program:<name>", "root", "password", "busy", "exited", "cwd:<path>"; the
  caret at the cursor of the front tab's active pane) > log ("scrollback",
  the last 10,000 lines / 1 MB, read whole through paged reading). Ids stable
  while a tab and a pane live: tab = 1<<60 | the tab's id (the bar's, a
  process-wide counter never reused), pane = 2<<60 | tab id<<20 | its number
  in the tab's split tree, its scrollback = 3<<60 | the same; `tab_of(node)`
  finds the tab of any node, for step 1. Every tab publishes its panes (the
  scoped grants hide the others). **Password** = input read a line at a time
  (ICANON) without echo, asked of the pty's master with tcgetattr (Linux
  hands a master's terminal-mode request to its slave; FreeBSD's pts takes
  it on either side; ICANON is 0o2 on Linux, 0x100 on FreeBSD): echo off
  alone was wrong, since zsh's zle, readline and libedit turn echo off while
  a command is typed (the headless check caught it). The working directory
  is read at most every two seconds per pane (FreeBSD runs procstat); the
  scrollback text is made again only when lines were added. Checked in a
  headless session on Hubble: the tree's nodes and states; `echo hello`
  typed with vctl and seen in the pane's value; "password" on during `read
  -s` and off after; the scrollback after `seq 1 2000` read whole with
  `vagent read` (8,831 characters). FreeBSD's ICANON/termios reading is by
  its headers, not yet run (kepler not synced in this step; its unit test,
  a_password_prompt_is_seen_from_the_master, runs there with the next sync).
  Workspace tests on Hubble: 1,005 passed.
- **Step 1, grants scoped to a tab** (2026-11-22). Terminal's root says
  "shell-access", each tab "scope"; the server (agents.rs, meaning.rs) asks
  for such a window a tab at a time at a level, Read (tree, focus, changes,
  pages) or Act (it includes Read), offering only Don't allow, Just now and
  the timed length; an app-wide Always is not honored there; these grants
  live in memory. A tree or the focus reads the tab in front; a read or an
  act names its node's tab; a node in a tab the agent holds nothing for, and
  not in front, is refused ("not in a tab you may use") without asking.
  Replies hold the root (named "Terminal": the window's title is the tab in
  front's) and the granted tabs' nodes only; Find skips other tabs and never
  asks about a shell window; changes tell focus only in held tabs and the
  caret only while a held tab is in front. The dialog's heading names the
  tab ("An agent wants to type in the tab ..."), the agent's name staying in
  its quoted row as before. `vctl agents` lists tab grants.
  `agent_revoke_tab(client, window, tab)` ends every agent's grant for a tab
  (for step 4's stop keys). Checked headless on Hubble: a tree asked about
  the tab in front at Read, "always" and "session" not offered, the reply
  that tab's nodes only; the other tab's node refused; typing asked again at
  Act; both dialogs shot and fit. Still not installed: step 4 next.
- **Step 4, typing, and step 5's vagent/MCP part** (bus v38, 2026-11-22).
  **In git, not installed** (install waits for steps 5-6: agent-check, the
  display-suite scenario, kepler and techbench, the independent review).
  Protocol: `Action::Type(text)` (at most 16 KB) and `Action::Key(name)`
  (`window::KEY_NAMES`: Return, Tab, Escape, BackSpace, Delete, Up, Down,
  Left, Right, Home, End, Prior, Next, Insert, space, or ctrl+<letter>;
  anything else refused on decode); `AgentRequest::OpenTerminal { cwd }` and
  its answer `AgentEvent::Terminal { client, window, tab }`;
  `WinRequest::RevokeAgent` (the window's stop) and `WinEvent::AgentActing`
  (who is about to act, for the badge). Widgets refuse Type/Key.
  **Server**: Type and Key need an Act grant for the node's tab. OpenTerminal
  asks once, "An agent wants a terminal of its own" (Don't allow, Just now,
  For an hour), then starts `vergence-term --for-agent NAME` (the one beside
  the server, else on PATH) in the cwd (a folder, else home); the new
  window's first tree (its root saying "owner:NAME", same pid and program)
  gets that grant on its tab and the agent its answer. A window that does
  not come in 20 s is forgotten (the agent's own timeout applies). A stop
  (RevokeAgent) ends every agent's grant for the tab and refuses them there
  for 60 s with "the person stopped you in this tab (ask again later)".
  agents.log keeps a typed text's length and its first 40 characters,
  never more; keys by name.
  **Terminal** (crates/vergence-term/src/agent.rs), at the moment of typing:
  "Terminal is asking for a password: the person types it"; "this tab runs
  as root: agents may not type here"; "the person is typing" (a keystroke in
  that pane within 1 s); a burst the person cut short is told at the next act
  ("the person typed: the rest of what you typed last was not typed; read
  the screen"). Text up to a line (120 characters, at most one line end, at
  the end) is typed over 0.25 s in 16 ms steps, never splitting a
  character; longer goes in at once, bracketed if the program asked (its
  own end-of-paste removed); "\n" is Return; keys go through the keyboard's
  encoder; a key waits behind a burst in progress. The person's keystroke in
  the pane drops the rest of a burst. **Badge**: the tab's title in the bar
  reads "⌨ Claude · ..." while the agent types and 3 s after, and a pill
  "Claude is typing · stop" sits at the pane's top right; **a click on the
  pill** revokes. **Escape twice** within 0.5 s in a tab an agent holds
  revokes; the first Escape reaches the program (vim needs it), the second
  does not. Either stop notes "[Claude's access to this tab ended]" in the
  pane. `--for-agent NAME` opens its own process and window ("NAME's
  terminal"), one tab, no handover or restored session. The tree is now
  made only when a cheap key changes (size, front tab, tab titles, each
  pane's output count, grid size, cursor, states), not every frame.
  **vagent**: `terminal open [cwd]`, `type <client> <window> <node>
  <text>`, `key <client> <window> <node> <name>`; **MCP**: `terminal`,
  `type`, `key`, their descriptions naming the refusals, the pace and the
  stops. Checked headless on Hubble: the dialog (Always and Session not
  offered), the agent's own window "Claude's terminal" with owner:Claude;
  `echo hi` typed and read back; `read -s` refused with the password words,
  and after the person answered it, typing again; a person's key cutting a
  48-character line short, told at the next act; "the person is typing";
  Escape twice and a click on the pill both revoking; the person's own
  terminal not found by `find`. Shots of the dialog, the badge and the note
  looked at. Workspace tests on Hubble: 1,013 passed. Not run: kepler and
  techbench (not synced in this step), agent-check's terminal part.
