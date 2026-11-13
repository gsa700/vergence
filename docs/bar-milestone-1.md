# Bar milestone 1: the server draws every native window's bar, tabs included

Drafted 2026-11-12 from Dave's decision the same evening: "the server
should draw all the native apps where feasible", the tabbed title bar
included, with tabs that can be dragged; the wiring is Claude's to choose.
**Accepted 2026-11-12: Dave answered (a) to all four questions at the
end.** His reason for the server drawing every bar: "we will want some
different color options and maybe theming to some extent although i
generally favor minimalism and consistency over tons of options". So: a
few well-chosen color options, applied by the server to every bar alike;
no per-program styling, no large theming system.

## Where things stand (2026-11-12)

- **Two bars for one look.** The server draws title bars for Wayland and
  X11 windows and for native windows that ask for one (decor.rs: a title and
  the window's buttons). Our toolkit draws its own bar
  (crates/vergence-ui/src/bar.rs: tabs, each tab's close button, +, the
  buttons where the desktop's layout puts them) for the terminal, which says
  "I draw my own bar" when it opens its window, because the server's bar has
  no tabs. Both read their sizes and colors from vergence-look, so they
  match, but they are two programs drawing.
- **The settings app** already has the server's frame in a Vergence
  session (Dave, 2026-11-11); its own toolkit bar, following the desktop's
  button layout, is drawn only where the server draws none (GNOME).
- **What a self-drawn bar costs**, all seen in the last two days: a window
  moved by its own bar has to ask the server, so every way of pressing
  (mouse, finger) needs its own path to grant that (the finger path for
  native windows came only on 2026-11-12, e662b8d); a hung program's bar
  cannot be used to move or close it; and nothing stops a program drawing a
  bar that imitates another window's (§7.3).
- **The terminal's tabs** can be clicked, closed (their button, or a middle
  click) and added with +; they cannot be dragged yet.
- **The server's title bar ignores the button layout.** decor.rs reads the
  layout setting only to decide whether there is a minimize button and
  always puts the buttons on the right, while the toolkit's bar follows
  `Gtk/DecorationLayout` (on Hubble "close,minimize,maximize:", the left).
  So in a Vergence session the settings app's buttons are on the right and
  the terminal's on the left (Dave noticed; under GNOME, where the settings
  app draws its own bar, they agree).

## Design

**A native window describes its bar; the server draws it and says what
happened.** The window sends a **bar description** whenever it changes:

- `Plain`: the title (what every window has now), or
- `Tabs`: a list of tabs, each with a stable id the program chose, its
  title and whether it may be closed; which one is in front; and whether
  there is a +.

The server lays the bar out and draws it with **the same code the toolkit
uses today**: bar.rs moves into a small crate of its own, `vergence-bar`
(layout, hit test, drawing into a scene, the tab drag model), which both
the server and vergence-ui depend on, so the two can never draw a
different bar. The server renders it as it renders the dock
(vergence-render's CPU renderer into a picture), into the frame around
the window it already draws.

**What the server sends back**, as window events: a tab chosen, a tab's
close asked for, + pressed, a tab moved to another place in the bar, a tab
dropped outside the window (with the point), a tab dropped on another
window's bar of the same program (that window and the place), and a
right-click on a tab (see Questions, 3). The program decides what each
means: the terminal shows the tab, asks its shell, opens a tab, reorders,
or opens a new window and moves the tab's session into it.

**The seat** (vergence-seat) gains the bar's parts as targets: a tab, a
tab's close button, +. A press on a tab that moves beyond a few points is
a **tab drag**, a grab like a move: while it lasts the server draws the
tab under the pointer, the other tabs make room live, and the program hears
nothing until the drop. Touch works the same way (a finger on a tab), and
there is no "please move me" request any more: the server moves a window
by its bar itself, for every kind of press.

**Every bar the server draws comes from vergence-bar**, the plain ones
too: the frame around a Wayland or X11 window that asks for one is a
`Plain` bar with its title, so all of them follow the button layout, live.
decor.rs keeps the frame's border, shadow and hit test around the window
and stops drawing a bar of its own. Firefox and other programs that draw
their own bars keep doing so (client-side decorations). **Under GNOME and
other desktops** our toolkit keeps drawing its own bar (from vergence-bar,
so it is the same one); the native road is where the server draws it.

**Size and scale**: the bar is in reference units (Contract 1) like the
rest of the frame, drawn at each monitor's scale by the server, so a
window straddling two monitors has a sharp bar on both.

## Windows that stay where they were (added 2026-11-12, Dave's go)

Dave, 2026-11-12: tear-offs and joins must not make window memory messy:
"we really need to get this right because this is my big deal ... its what
I hate about wayland etc. make it scale and stay fast." His test case:
thirty terminals remembered on seven screens.

**What was wrong with memory for this.** A window's identity was its
program, its role and its place among that program's open windows (#1, #2,
#3, in the order they opened). A torn-off tab would have taken the next
free number and jumped to wherever that number last was, not where it was
dropped; and with thirty windows, closing one in the middle shifts every
later one into the place of the one before it at the next login.

**Persistence tokens** (whitepaper §6, Contract 2; the protocol's Create
already carries one). The server gives every native window a token when it
is created, unique and meaningless (a random id, with the program's name
and role beside it in the store); a program that kept it presents it again
when it opens that window on a later run, and memory is keyed by it. So:

- each window comes back to its own place, whatever else was opened or
  closed meanwhile; numbering is left only for programs that keep no token
  (Wayland and X11 programs today);
- a window opened for a torn-off tab gets a new token and **the drop point
  wins over memory** for it (a `Place` with the drop flag); it is remembered
  where it then is;
- a window **the user closes** (its close button, its last tab closed, the
  dock's Close) has its memory dropped; windows still open when **the
  session ends** keep theirs; tokens not seen for 90 days are pruned, so
  the store cannot silt up;
- lookups are by token (a hash map), the store is written once a move or
  resize ends and at most once a second otherwise, and stays small (one
  line per remembered window per monitor setup).

**One terminal, many windows.** vergence-ui gains several windows per
program; the terminal becomes one process for all its windows (starting
`vergence-term` again asks the running one for a new window). It keeps its
windows' tokens and their tabs in its own small state file and, at login,
opens them again itself, each with its token, so each comes back where it
was with its tabs (their working directories; a shell's scrollback does not
survive a logout). Joining a tab to another window, tearing one off, and
moving a section of a tab into its own tab or window all happen inside the
one process.

## Where it stands

- **Step 1 done** (2026-11-12): crates/vergence-bar (bar.rs moved out of
  vergence-ui with its history; vergence-ui keeps `bar` as an alias, so the
  terminal and the settings app are unchanged), plus the tab drag model
  (`TabDrag`: when a press is a drag, where the tab is drawn, where it would
  land, in the row or out of the window; `reorder`), with tests. 371 tests
  on Hubble, kepler and Techbench.
- **Step 2 done** (2026-11-12): the protocol. `WinRequest::Bar { window,
  bar }` with `Bar::Plain` or `Bar::Tabs { tabs, front, plus, accent,
  menu }`; events `TabChosen`, `TabClose`, `TabCloseOthers`, `TabNew`,
  `TabMoved`, `TabOut` (with the place for the new window's Create),
  `TabJoin` and `TabMenu`. Checked on decoding: at most 256 tabs, titles
  and labels up to 1 KB, 16 menu entries, tab ids unique and not 0, the tab
  in front one of the tabs. Round-trip tests, a refusal test, the fuzz test
  seeded with bar messages (200,000 rounds clean). The server keeps each
  native window's bar description; the toolkit ignores the tab events
  until step 3. 372 tests on Hubble and Techbench; on kepler one terminal
  pty test failed once under the full parallel run and passed five times
  alone (a race on FreeBSD to look at, unrelated to the bar).
- **Step 3, first half done** (2026-11-12): every title bar the server
  draws is vergence-bar's plain bar, laid out with the desktop's button
  layout (the WM `button-layout` setting, from Gtk/DecorationLayout) and
  hit-tested by the same layout, drawn into a picture per scale with the
  dock's CPU path and redrawn when the title, width, focus, hover or layout
  changes. The bar is 36 points tall now, the toolkit's (the server's was
  24). On Hubble (layout "close,minimize,maximize:") the settings app's
  buttons are on the left like the terminal's. 372 tests on all three.
- **Step 3 done** (2026-11-12): the second half. A native window whose
  tabs the server may draw (`App::tabs_by_server` in vergence-ui) leaves
  its frame to the server on the native road, sends its bar description
  after each frame in which it changed (`App::bar`), hears the tab events
  in `App::tab`, and leaves its own bar out of its picture
  (`Frame::bar_by_server`); over Wayland and under other desktops it draws
  its own bar as before. The server lays a described window's bar out with
  `Layout::new`, draws its tabs (the tab in front in the desktop's new
  TAB_FRONT colors, or the program's accent) and hit-tests them: a tab,
  its close button and + are seat parts (vergence-seat `Part::Tab`,
  `CloseTab`, `Plus`); a left press on a tab chooses it, a middle press or
  its close button closes it, + opens one. The terminal is the first user
  (tab ids, a bar height of 0 when the server draws it). On Hubble by
  `vctl`: + opens a tab and the bar shows it within 0.3 s, a tab is chosen,
  a tab's close button closes it, the last one closes the window. 372 tests
  on all three. The left controls are the default since the same evening
  (cbe95ae).
- **Step 4, the native road done** (2026-11-12): vergence-ui runs several
  windows per program on the native road. The connection keeps what the
  windows share (the keyboard and its repeat, the clipboard and a paste in
  progress, the GPU, fonts, the waker: a wake goes to every window); each
  window keeps its program (`App`), picture buffers, size and scale,
  frames, pointer, fingers, cursor and timers; events go by window id, keys
  to the window with the keyboard. `Window::open_window(Box<dyn App>)`
  opens another (created asynchronously; the first stays synchronous, since
  it decides between the roads); the program ends with its last window.
  `App::token` and `App::token_given` carry the persistence token for step
  5. Single-window programs are unchanged: on Hubble the click test is 50 of
  50 on both roads, natively 2.1 ms on average from press to frame through
  the server-drawn tabs (clicks.sh now finds the bar above a window the
  server frames). The Wayland road (other desktops) keeps one window for
  now: step 4b, after the terminal's own steps.
- **Step 5, the server and the store done** (2026-11-12): native windows
  get a persistence token at Create: the one they present if it is 32
  lowercase hex digits and not another open window's of the same program,
  else a new 128-bit one (/dev/urandom). A program that keeps tokens
  presents its saved one, or "new" for a new window (no memory then, so the
  place it asks for wins: a torn-off tab's drop point); memory goes by the
  token for it. A program that presents nothing keeps the numbering, so it
  leaves no token entries behind (otherwise every run would have added
  one). The store (vergence-profile) carries `token` and `seen` (a day
  number); a window closed for good (the program's Destroy) has its place
  kept as its program's last-closed one, where a new window starts, and its
  token forgotten on every setup; a program that goes away or a session
  that ends forgets nothing; tokens unseen for 90 days are pruned at save.
  375 tests on all three. The terminal keeping its tokens is step 6.
- **Step 6 done** (2026-11-12): the terminal is one process for all its
  windows (crates/vergence-term/src/program.rs). In a Vergence session the
  first `vergence-term` listens on an owner-only socket in the runtime
  directory; a later start hands its request (working directory, `-e`
  program) over and exits; a terminal started to be measured (--tabs,
  --panes, VTERM_FRAMES) keeps its own process. Each window is its own
  `Terminal` sharing the program; panes and tabs are numbered across the
  process. The terminal presents its saved token or "new", keeps each
  window's token and its tabs' working directories (Linux /proc, FreeBSD
  procstat) in ~/.local/state/vergence/terminal-windows (0600, its own
  small line format), written on every change, and at start opens them
  again with their tokens and tabs. The toolkit's new `App::closed` tells a
  window it closed while the program runs on (not at a logout), so a window
  the user closes leaves the state file. New windows starting from their
  program's last-closed place cascade down and right by a bar's height
  instead of landing on one another. On Hubble (tools in the session's
  scratchpad, memtest.sh): three windows in one process, two moved, the
  process killed as a logout would: all three back exactly; one closed for
  good: forgotten by the terminal and the server, the other two back
  exactly after another start. A remembered place partly off its monitor
  is not restored (the existing rule: the whole window must fit). 378 tests
  on all three.
- **Step 7 done** (2026-11-12): tab dragging and the menus. The server
  holds a pressed tab (dispatch.rs `TabHold`): past the drag distance it
  draws the tab following the pointer and where it would land
  (vergence-bar's model); let go in the row it sends `TabMoved`, over
  another window's bar of the same program `TabJoin` (the place in that
  row by the pointer), anywhere else `TabOut` with the drop point: the
  new window's bar under the pointer, slid onto the monitor's work area as
  far as the window's size needs (a place partly off the monitor is
  refused, which centered the window). A window with a fresh token that
  asks for a place gets it ahead of its program's last-closed place (the
  rule step 5 meant; memory came first in the code), and neither lands
  with its corner exactly on another window of its program: it steps
  down and right by a bar's height, up and left where that runs off the
  monitor (memory.rs `clear_of_siblings`). Right-click on a tab: the
  server's tab menu (Close, Close the others, Move to a new window, the
  program's entries). The terminal takes a tab out (`take_tab`; a window's
  only tab may join another window, which then closes), hands it to
  another window of the process or opens one for it at the asked place;
  its section menu has Move to a new tab and Move to a new window. On
  Hubble by vctl (scratchpad tabtest.sh, sectest.sh): reorder, tear-off at
  the drop point, a one-tab window joined into another, the tab menu's
  Move to a new window, both section-menu moves; one process and every
  shell kept throughout. 378 tests on all three.
- **Step 8 done** (2026-11-12): clean-up. The server draws every native
  window's bar: `WinRequest::Create` lost `own_frame`, and the window's own
  `Move` and `Resize` requests are gone (ops 106 and 107 left unused), so
  the bus protocol is version 2. On the native road an app that draws its
  own bar elsewhere (`App::own_frame`) is told the server draws it
  (`Frame::bar_by_server`); the toolkit's Wayland road keeps the flag for
  other desktops. Gone with them: the server's finger-moves-a-native-window
  path (touch.rs, seat `finger_on`) and vnative's `--own-frame`. Docs:
  whitepaper §6.4 (tokens kept by the application, the server's bar and
  tabs), native-milestone-1.md's protocol note. Besides (Dave, 2026-11-12):
  the settings app runs in a Vergence session only (it refuses to start
  without `VERGENCE_BUS`; its desktop entry has `OnlyShowIn=Vergence;`), and
  the shelf honors OnlyShowIn and NotShowIn, so another desktop's own
  entries (GNOME's Settings) leave Vergence's popup. 377 tests on all three.
- **Acceptance 7 met** (2026-11-12, Hubble): a nested server with seven
  virtual monitors (`VERGENCE_OUTPUTS`, 244 x 928 points each) in Hubble's
  session, with a throwaway HOME and runtime directory; thirty terminal
  windows in one process, four or five to a monitor (some reaching past its
  bottom, the bar on it), fifteen with extra tabs (45 shells), placed with
  `vctl move` (which now remembers the place, as a move by hand does when
  it ends). The terminal killed as a logout would and the server ended and
  started again: all thirty back exactly, 45 shells, in 0.17 s from the
  start (one window alone: 0.06 to 0.12 s). Three in the middle closed
  with their close buttons: forgotten ("closed for good" three times), and
  after another logout and login the other twenty-seven exactly where they
  were with their 41 shells, none of the three back. The store: 8,383
  bytes for thirty windows, 7,795 after the three went. Scripts in the
  session's scratchpad (nested.sh, scale.sh).
- **A tab's or section's new window is as large as the one it left**
  (Dave, 2026-11-12): the toolkit's `App::opening_size` gives the size a
  window opened by another asks for, and the opener is its parent, so the
  section menu's Move to a new window asks for a place a bar's height down
  and right of it (`Place::Parent`). On Hubble: a source resized to 650 x
  410, the section's window and a torn-off tab's both 650 x 410.
- **A window's only tab moves the window** (Dave, 2026-11-12), as a
  browser's: past the drag distance the press becomes the seat's window
  move (from the press point, so the window keeps up with the pointer);
  let go over another window's bar of the same program, the tab joins it
  there and its own window closes (`join_target`, which looks through the
  window being moved but not through a tab's window that stays put).
- **Run on kepler and Techbench** (2026-11-12, 7eb1a42): the same live
  scripts as on Hubble (tabtest, sectest, sizetest, onetab) pass on kepler
  (FreeBSD, the 14" 4K at scale 2) and Techbench (Fedora 45, the dev
  session from GDM, scale 1): reorder, tear-off at the drop point, a
  one-tab window joined into another, the tab menu's and the section
  menu's moves, new windows the size of their source, a window moved by
  its only tab and joined by letting go on another bar; one process and
  every shell kept. kepler's bar at scale 2 is drawn 1:1 and sharp (it
  shows close and minimize only: its settings.toml has
  `Gtk/DecorationLayout = "close,minimize:"`; put back to
  "close,minimize,maximize:" at Dave's word, live).
- **Fractional scales** (2026-11-12, Hubble's panel by desktop.toml at
  1.25, 1.5 and 1.75: 1536 x 864, 1280 x 720 and 1098 x 618 points): the
  four live scripts pass at each (clicks land on what is drawn: reorder,
  tear-off, join, menus, sizes, the only tab moving its window); the
  server's bars, tabs and buttons and the settings app are sharp at each
  (pictures drawn at the screen's pixels, shown 1:1). A torn-off tab's
  window at 1.5 slid exactly onto the screen (630,310 for 650 x 410 on
  1280 x 720). Found on the way: window memory keys a setup by its
  monitors alone, not their scale or resolution (see the next note).
- **Window memory per size of the setup** (Dave, 2026-11-12: "yes, build
  it that way"): places are kept under the setup's key and each monitor's
  size in points (`windows::sized_key`, "<key>|<monitor>=<w>x<h>,..."), so
  a change of resolution or scale on the same monitors has places of its
  own and going back finds the old ones untouched. A size seen for the
  first time starts from the most recent other size of the same monitors
  (`WindowStore::variants`), each place and size scaled in proportion to
  its monitor (`WindowSetting::scaled`); a profile from before sizes were
  kept counts as such a size, unscaled. When the size changes with windows
  open, they go to their places for the new size at once; the server's own
  placing after a monitor change (place.rs `set_rect`, which now resizes
  native windows too) is no longer remembered as if the user had moved
  them. On Hubble (scratchpad sizemem.sh): a terminal at 960,540 820 x 520
  at scale 1; at 1.5 first time 640,360 547 x 347; moved there to 100,100;
  back at 1: 960,540 820 x 520; at 1.5: 100,100; the same live, without a
  restart. The scale test still passes (30 windows exact, then 27).
  `vctl` takes `VCTL_SOCKET` for one socket and no fallback: the nested
  test's stop, with its server not running, quit Hubble's session.
- **Acceptance 2, 3, 4, 6 and 8** (2026-11-12, Hubble, by vctl; scratchpad
  acc.sh, acc2.sh, clicks-both.sh): by a finger, + adds a tab, the bar
  moves the window, a quick double-tap maximizes and restores, tabs
  reorder, tear off and join (a one-tab window's tab too); a torn-off tab
  keeps its output and scrollback. A terminal stopped with `kill -STOP` is
  still moved by its bar, and its close press waits: the window closes as
  soon as the program runs again (4). Tab clicks with the stress test's
  six tabs at 144 Hz: native road 50 of 50 drawn, at most one frame after
  the press, 49 within 13.9 ms (one at 14.5 ms); Wayland road 50 of 50,
  all within (6). A torn-off tab's window and a section's window come back
  exactly after a logout-like kill and a start (8).
- **Acceptance 5 met** (2026-11-12, Techbench, Dave in GNOME 49): today's
  terminal on the Wayland road draws the same bar itself: buttons on the
  left as Vergence's settings say (the toolkit follows Vergence's
  settings.toml, not GNOME's button-layout, which there is
  "icon:minimize,close"), tabs chosen, added and closed, the window moved,
  resized and maximized by its bar; Dave's screenshot and checks. All
  eight acceptance checks are met; step 4b is left.

## Steps

1. **`vergence-bar`**: bar.rs moved out of vergence-ui (no change in what
   it draws), plus the tab drag model: where a dragged tab would land,
   when a drag leaves the window, with tests.
2. **The protocol**: the bar description and the bar's events in
   vergence-protocol, length-limited (tab count, title length), with the
   fuzz test extended.
3. **The server draws every bar with vergence-bar**: first its plain title
   bars (so they follow the button layout, which fixes the left/right
   mismatch on its own), then native bars from their descriptions; the
   bar's parts are seat targets; clicks, closes and + reach the program.
   The terminal's native road stops drawing its own bar.
4. **Several windows per program** in vergence-ui, on the native road and
   the Wayland road: a window id per window, each window's own frame,
   input, bar and meaning; the one-window API kept as the common case.
5. **Persistence tokens and memory by token**: the server issues and honors
   tokens, memory keyed by them (numbering kept for programs without one),
   the drop point winning for a torn-off tab's window, memory dropped when
   the user closes a window, pruning after 90 days unseen; tests for the
   rules, and a test with thirty windows over seven fake monitors.
6. **One terminal process**: a second start hands over to the running one;
   the terminal keeps its windows' tokens, tabs and working directories and
   opens them again at login (session restore starts it once).
7. **Tab dragging and the menus**: reorder in the bar, a tab dragged out
   becomes a window where it was dropped, dropped on another terminal
   window it joins it; the server's tab menu (Close, Close the others, Move
   to a new window, the program's own entries); the terminal's section menu
   (Move to a new tab, Move to a new window).
8. **Clean-up**: the "draws its own bar" flag kept only for the toolkit's
   road on other desktops; the native move and resize requests go when
   nothing native sends them; docs (whitepaper §6.4 and Contract 2, the
   native milestone's protocol notes).

## Acceptance

1. Tests green on Hubble, kepler and Techbench, the bar's own and the
   protocol fuzz test with the bar messages among them.
2. In a Vergence session on all three: the terminal's bar is the server's
   (one bar look for every window the server frames, Wayland and X11 ones
   included, buttons on the side the settings say, changed live); tabs chosen, closed and added by mouse and finger;
   the window moved and resized by its bar with mouse and finger; a
   double-click on the bar maximizes.
3. Tab dragging: reorder by mouse and finger, a tab dragged out becomes a
   window of its own where it was dropped, a tab dropped on another
   terminal's bar joins it; the session (shell, scrollback) goes with it.
4. A terminal whose program is stopped (`kill -STOP`) can still be moved,
   and its close button asks the server to close it.
5. Under GNOME the terminal draws the same bar itself, as now.
6. Clicks 50 of 50 on tabs on both roads, as before.
7. **Memory at scale** (Dave's case): thirty terminal windows spread over
   seven (fake) monitors, logout and login: every window back on its
   monitor and place with its tabs; close three in the middle, log out and
   in: the other twenty-seven exactly where they were, the three forgotten.
   Restoring them takes no more than a second beyond starting the programs;
   the store stays under 64 KB.
8. A tab torn off opens where it was dropped and, after a logout, comes back
   there; a section moved to a new window likewise.

## Security (whitepaper §7.3)

- No program draws in the bar: it describes, the server draws, so a
  window cannot imitate another's frame.
- Bar descriptions are untrusted input: limits on the number of tabs and
  the length of titles; a fuzz test.
- A tab dropped on another window is offered only to a window of the same
  program (the same client); the server never moves content between
  programs.

## Not in this milestone

- Menus in the bar (an application menu button) beyond the tab menu.
- Wayland and X11 windows' bars (they keep their own or today's).
- Showing that a program is not answering, and offering to stop it: noted
  for later (it needs a ping on the native protocol).

## Questions for Dave (answered 2026-11-12: (a) to all four)

1. **The bar's colors.** (a) The desktop's bar colors for every window,
   so every bar looks the same; (b) a program may tint its bar (the
   terminal's bar follows its color scheme today). Proposed: **(a)**, with
   the program's accent allowed on the tab in front.
2. **A tab dragged out of the window.** (a) It becomes a new window of
   that program where it is dropped, and dropped on another window of the
   same program's bar it joins that window (as in Firefox); (b) reorder
   only. Proposed: **(a)**.
3. **A right-click on a tab.** (a) A menu drawn by the server with the
   usual entries (Close, Close the others, Move to a new window) and any
   the program adds; (b) nothing for now. Proposed: **(a)**.
4. **Order of work.** (a) This milestone before the window space and
   renderer (step 2 of replacing Smithay; this one does not depend on it);
   (b) after. Proposed: **(a)**: it is smaller, it removes the move
   request paths that caused trouble this week, and it uses the drawing
   code the renderer work will build on.
