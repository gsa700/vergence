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
4. **Tab dragging**: reorder in the bar, drop outside for a new window,
   drop on another window of the same program to join it. The terminal
   implements all three (a tab's session moves with it).
5. **Clean-up**: the "draws its own bar" flag kept only for the
   toolkit's road on other desktops; the native move and resize requests
   go when nothing native sends them; docs (whitepaper §6.4, the native
   milestone's protocol notes).

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
