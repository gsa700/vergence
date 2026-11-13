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
