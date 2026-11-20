# Meaning milestone 1: agents act by name, and the magnifier follows

Status: **accepted 2026-11-20** (Dave: "Yes, we might also consider a
time based or session based permission option?": the MCP adapter in,
asking per app, both magnifier styles, the name; grants for a time or a
session added to step 4). The core-vision
milestone due by the rule of 15 November (an app or polish milestone, then
one of the core vision; docs/vision-check-2026-11-15.md): Contract 3's
first readers (whitepaper §6.5).

## Why now

Every native window has published its tree of meaning since 11 November
(roles, names, values, states, bounds, the focus and the text caret;
`vctl tree` prints it), and nothing reads it. The vision check named this
the first under-developed part of what makes Vergence different. It pays
off daily from the first step:

- **Agents.** Checking the apps this month meant arrow keys through menus
  and clicks at pixel coordinates read off screenshots (`vctl`). An agent
  on the contract asks for "Terminal Settings, the Text menu's items" and
  presses one by name. It is also the start of Dave's "agentic JIT app
  building" idea, and the named desktop actions (dock milestone 1) are the
  same command surface.
- **The magnifier.** The contract's first accessibility reader: zoom that
  follows the focus and the text caret, not only the pointer. It is what
  display-list surfaces (§6.6, the next core milestone) will make sharp.

## What is built today

- vergence-protocol `WinRequest::Meaning { focus, caret, nodes }`, sent by
  vergence-ui (`MeaningTree`, vergence-widgets fills it from its elements;
  a secret field's text never leaves the program).
- The server keeps each native window's last tree (native.rs) and prints
  it for `vctl tree`. Nothing flows the other way: no one can ask a window
  to do anything by name.
- The control socket (`vctl`) is for tests only: it goes around the
  security model and exists only when a developer turns it on (§7.3).

## The steps

1. **Actions on the wire (bus v24).** `WinEvent::Act { window, serial,
   node, action, value }` and `WinRequest::Acted { serial, ok, why }`.
   Actions: press (buttons, menu items, check boxes, switches, tabs, list
   items), focus, set text (a field's whole text; never a secret field's),
   choose (a list's or a menu's item by name), scroll into view, expand
   and collapse. vergence-ui routes an action to the element with that
   node's id as if the person had used it (the same handler a click or a
   key reaches), so every app on the toolkit answers with no code of its
   own. Tests per widget kind.
2. **The server's model of meaning.** Every native window's tree kept with
   its window's place on screen (Contract 1 and 2: bounds in reference
   units and on which screen); queries by path ("Terminal Settings > Text >
   Font size"), by role and name, and the focused node; changes kept as
   events (focus moved, caret moved, tree changed) for readers to follow.
3. **The agent interface on the session bus.** An agent says who it is
   (`Hello` as an agent, with a name the person will see); it can list
   windows, read a tree, find, act, wait for a change, and take a window's
   picture for what has no meaning yet (other programs' windows). Limits on
   rates and sizes as every bus topic has. A secret field is a role and a
   name, never a value.
4. **Permission (§7.2).** An agent's first action on an app asks the
   person, in a dialog the server draws (no app can draw it): "Claude wants
   to use Terminal", with how long: **Just now** (until this agent
   disconnects), **For an hour** (the length can be changed, 15 minutes to
   a day), **Until I log out** (this session), **Always**, or **Don't
   allow** (asked again next time; "Never" kept on the Privacy page).
   Reading is asked too, once per app, with the same choices. Grants are
   kept per agent and per app (0600, under ~/.local/state/vergence; the
   session's and timed ones forgotten when they end, timed ones checked at
   every action, not by a timer); shown while it works (a live tile,
   "Claude is using Terminal, 42 min left", and a mark on the window);
   every action logged; listed with what is left of each and taken back on
   a new Settings page, Privacy > Agents, which can also end every grant at
   once.
5. **Seen, not felt.** The agent never moves the person's pointer or
   types into the person's window: its actions arrive as actions, not as
   input. The server draws a brief ring on each node an agent acts on, in
   the agent's own color, so the person can watch or carry on beside it.
6. **The adapter: vergence-agent-mcp.** A small program speaking the Model
   Context Protocol (JSON-RPC over its standard input and output, our own
   JSON code) on one side and the agent interface on the other: tools for
   windows, tree, find, act, wait, picture. Any MCP agent (Claude Code
   among them) can then work Vergence's apps by name. `vctl tree` and a new
   `vctl act` move onto the interface, as §6.5 says the test tool should.
7. **The magnifier.** Drawn by the server: the whole screen zoomed (the
   Mac's default), or a lens around the point of interest; that point is
   the pointer, the focused control or the text caret, whichever moved last
   (each can be turned off). Keys: Super+Alt+= and Super+Alt+- to zoom,
   Super+Alt+8 on and off, Super+Alt+L the lens. Smooth zooming on the GPU,
   pixels for now (sharp once display lists come). The Accessibility page
   gains its settings. Works for every window, our own following focus and
   caret, others following the pointer.
8. **A focus ring the server draws** (setting, off by default): the same
   clear ring around the focused control in every native window, from the
   tree, for keyboard users and in the magnifier.
9. **Checks.** A display-suite scenario (the magnifier following a caret
   in a headless session, pixel-checked); the agent interface driven end
   to end by vergence-agent-mcp against Edit, the settings app and
   FileView; the permission dialog's refusals; tests and the suite on all
   four machines.

## Not in milestone 1

Speech (a screen reader needs a voice: its own question of borrowing);
reading the trees of programs that are not ours (GTK, Qt and Firefox
publish AT-SPI on D-Bus: our edge could read it later); an agent's own
pointer and keyboard focus as a second seat (step 5's actions make it
unnecessary for now); voice control.

## Questions for Dave

1. *Answered: yes.* **The MCP adapter (step 6) in this milestone?**
   Recommended: yes. It is what makes the interface useful to an agent at
   once, and the way these apps will be tested from now on.
2. *Answered: per app, and for a time or a session too.* **Permission's
   default:** ask per app the first time, with "Always" remembered
   (recommended), or one switch per agent for all apps?
3. *Answered: both, full-screen first.* **The magnifier:** full-screen
   zoom, a lens, or both?
4. *Answered: meaning milestone 1.* **The name.**

## Progress

- **Step 1** (2026-11-20 afternoon): actions on the wire. Bus v24:
  vergence-protocol `window::Action` (Press, Focus, Set, Choose, Reveal),
  `WinEvent::Act { window, serial, node, action }`, `WinRequest::Acted {
  window, serial, ok, why }`. vergence-ui: `App::act` (default: "this
  window takes no actions"), the native road answers every Act with
  Acted. vergence-widgets: `Ui::act` through the paths input takes (a
  press is the keyboard's activation and reveals what it presses; a
  field's new text typed over its whole text, one step to undo; a text
  area's the same through select all and insert; a switch "on"/"off", a
  slider's number kept within its range with its release message; a
  choice's item by name, any case, "set" meaning "choose", a miss listing
  the items; Focus the only action that moves the focus; a secret field's
  text never set; the off and the unknown refused in a few words), the
  host's `App::act` delivering its messages as any input's. The server:
  `send_act`, the last 256 actions' answers by serial (only the window
  sent to can answer), `vctl act <i> <node> press|focus|reveal|set|choose
  [value]` and `vctl acted <serial>`; `vctl tree` shows each window's
  index and each node's `#id`. Tests: the protocol's round trips, every
  widget kind's action, a text area's one-step undo, reveal in a scroll.
  Seen in a headless session on Hubble: 7 × 6 = pressed by name in the
  Calculator ("Earlier: 7×6 = 42"), Settings' "Minimize" switched off,
  "Snap a window dragged to an edge" chosen by name after a miss named
  its items, "Window keys" focused, an unknown node refused. Seen too: a
  choice's items are not in the tree yet (step 2's model should carry
  them).
- **Step 2** (2026-11-20 afternoon): the server's model of meaning, in
  the server's meaning.rs. Bus v25: `window::Node::items`, what a choice
  offers (vergence-widgets fills it from a choice's options; cut to its
  share of the frame). Paths: a window's title or app id ("*" any), then
  widgets by name, "role:name" or "role:", each anywhere below the last,
  case not minded ("Calculator > 7", "Windows > switch:Minimize", "* >
  button:Save"); answers with the window's index, the node, its place on
  the screens in logical points and the screen's name. The focused widget
  of the window with the keyboard, with its caret on the screens. A log
  of changes (the keyboard's window, a window's focus, its caret on the
  screens, its tree), the last 1024 by number, for readers to follow.
  `vctl find <path>`, `vctl focused`, `vctl changes [since]`. Tests: the
  paths (names, roles, below a part, case, nothing), the window part, the
  log. Seen headless on Hubble: a switch found by path, every choice with
  its items, focus moved by an action and the log saying so.
