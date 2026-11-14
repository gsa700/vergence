# Windows milestone 1: snapping and Alt+Tab

Drafted 2026-11-14 (Dave: "draft the windows milestone"), from whitepaper
§6.9, Windows: "dragging a window to an edge gives it half the screen, to a
corner a quarter, with the shelf counting as the edge. It can be switched
off", and "Alt+Tab steps through windows, most recently used first, in a
small list in the middle of the screen". Shelf milestone 1 left both for "a
windows milestone of their own". **Accepted 2026-11-14** with Dave's
answers (end): snapping **off by default** ("I am really not a snap window
person, so just go with a sane default that is off by default and add all
the settings to the settings app"); the dock counts as the edge exactly as
its own settings say ("honor the docks own setting whether or not it hides
from near windows").

## Where things stand (read 2026-11-14, f700755)

- **Maximize** (decor.rs `toggle_maximize`): a window fills the monitor it is
  mostly on, as far as the dock's band (shelf.rs `work_area`); its earlier
  place and size are kept in `Server::restore` until it comes back. Dragging
  a maximized window by its bar more than 6 points (dispatch.rs `DRAG`) brings
  it out at its old size, under the pointer by its bar. Window memory
  (memory.rs, vergence-profile `windows.rs`) records maximized windows with
  their restore place (`max_horz`, `max_vert`).
- **Moving** (dispatch.rs `Running::Move`, begun by the seat's `Grab::Move`):
  by the bar, by Alt+drag, by a finger on the bar; the window follows the
  pointer with nothing else drawn. Native, Wayland and X11 windows all move
  this way.
- **The keyboard** (vergence-seat keyboard.rs): the server's keys come first:
  Ctrl+Alt+Backspace, Escape for menus, the volume keys, the console keys,
  Super tapped alone for the launcher. The seat names no Tab or arrow keys,
  so Alt+Tab and Super+arrows go to the focused program today.
- **Focus** (input.rs `focus_window`): a click or a finger raises a window and
  gives it the keyboard. The stacking order is Smithay's space; **nothing
  keeps the order in which windows were last used.**
- **Minimized windows** live in `shelf.minimized`, with the place each goes
  back to.

## Design

### Snapping

- **By dragging**: while a window is moved by the pointer or a finger, the
  pointer reaching a monitor's edge shows where the window would go, and
  letting go puts it there:
  - the left or right edge: that half of the monitor;
  - a corner (the last 24 points of an edge, in either direction): that
    quarter;
  - the top edge: maximized (a setting can make it the top half);
  - the bottom edge: nothing (it is where the dock usually is, and a window
    dragged down is usually being moved, not placed).
  "The monitor" is its work area, the one maximize already uses (shelf.rs
  `work_area` through `shelf_placement`), so the dock counts as the edge
  just as its own settings say: a dock that never hides and keeps windows
  clear is the edge (a window snapped beside it starts where it ends); a
  dock that hides (when a window covers it, or always), or does not keep
  windows clear, leaves the whole screen edge to the window, and hides
  behind it as it would for any window.
- **Only outer edges snap** by default: an edge with another monitor beyond
  it lets the window across, as now (a setting can make it snap there after
  the pointer has pressed against it for half a second). The keys below
  reach every half on every monitor.
- **The preview**: a rounded outline of the target area, frosted where
  there is a GPU (look milestone 1's backdrop) and a plain tint otherwise,
  drawn by the server under the moving window. It appears when the pointer
  has rested at the edge for 0.15 s, so a window dragged quickly across a
  corner does not flicker one, and goes when the pointer leaves the edge.
  Reduce motion: it appears without growing.
- **A snapped window remembers where it was**, as a maximized one does (the
  same `restore` map): dragging it away brings it out at its earlier size
  under the pointer; double-clicking its bar maximizes it, and again brings
  it back to its earlier place, not the half.
- **Programs are told**: Wayland windows get xdg-shell's tiled states for the
  edges they touch (so GTK and others drop shadows and resize grips there);
  native windows get the same in their configure (a field added to the bus
  protocol's Configure); X11 windows are configured to the new place. The
  server keeps drawing round corners on a snapped window (only a maximized
  one is square), as Windows 11 does.
- **Window memory** records a snapped window's zone (left, right, one of the
  four quarters) with its restore place, as it records maximized: after a
  logout or a restart the window comes back snapped, fitted to the work area
  of the monitor setup it comes back to (a new size of the setup refits the
  zone instead of scaling a stale rectangle).
- **Off by default** (Dave, 2026-11-14): dragging a window anywhere only
  moves it until snapping is switched on in Settings > Windows (below).

### Settings > Windows

Every choice in this milestone is a setting on the settings app's Windows
page, in a "Snapping and switching" group, written to desktop.toml
`[windows]` and applied live:

| Setting | Choices | Default |
|---|---|---|
| Snap windows dragged to the screen's edges | Off, While Ctrl is held, Always | **While Ctrl is held** |
| Layouts on the maximize button open on a hover | on, off | on |
| Room round and between snapped windows (3 points) | on, off | on |
| The top edge | Maximizes, Top half | Maximizes |
| Edges between screens | Never snap, Snap after a short press | Never snap |
| Show where it will go | on (the preview), off | on |
| Window keys: Super with the arrows | on, off | on |
| Alt+Tab | Windows on every screen, Windows on this screen | every screen |
| Alt+` steps among one application's windows | on, off | on |

The snapping rows under the first are dimmed while it is Off. **Added
2026-11-14** (Dave: "make it like windows and macos and build snap options
into the maximize button, and maybe drag with a modifier like
control+drag"; "I don't mind the feature as long as it's not overbearing"):
with "While Ctrl is held", the default, a drag to an edge only moves the
window unless Ctrl is held at some time during the drag; desktop.toml
`snap = "ctrl"` (`true` and `false` still read as Always and Off). The window
keys are a switch of their own: they are asked for, never stumbled into, so
they stay on with snapping off. Alt+Tab itself has no off switch (a program
cannot have it either way, see Security). The dock's band, and whether it
counts as the edge, stays the Dock page's to set, as now.

### Layouts on the maximize button

(Added 2026-11-14, from Dave's use of macOS's green button on Control.)

- **A small frosted panel** beside the maximize button, of pictures of the
  layouts: the left and right halves, the four quarters, maximized (the top
  half too where the top edge is set to give it). A click on one puts the
  window there, on the screen it is on, exactly as a snap does (the same
  place before, memory, tiled states); Escape or the pointer leaving closes
  it.
- **It opens on a deliberate pause**: the pointer resting 0.7 s on the
  button (longer than Windows' delay, so passing over does nothing), and
  closes when the pointer has left both the button and the panel. A
  right-click on the button, or a long press with a finger, opens it at
  once. A plain click still maximizes or restores, as now.
- **Settings**: the hover can be switched off; right-click and long press
  always work. The panel does not depend on the drag setting.
- **With the maximize button hidden** (Dave hides it: "if like me a user
  hides the maximize button, the keys still work"): the window keys have
  their own switch and never depend on the buttons, and the title bar's
  right-click menu gets **Snap to…** with the same pictures, so the layouts
  are there with or without the button.

### Keys for windows

All the server's, ahead of any program, through the seat's filter:

- **Super+Left / Super+Right**: snap the focused window to that half of its
  monitor; from the other half, Super+Right goes to the right half, and from
  the right half to the left half of the next monitor to the right (and
  Super+Left the mirror of it), so a window walks across every half of every
  monitor by keyboard.
- **Super+Up**: maximize; from a lower quarter, the upper one.
  **Super+Down**: from maximized or snapped, back to its earlier place; from
  an upper quarter, the lower one; otherwise minimize.
- Super pressed with any of these never opens the launcher (the filter
  already opens it only for Super tapped alone).

### Alt+Tab

- **Alt+Tab** switches to the window used before this one. **Holding Alt**
  after the first Tab shows the list; each Tab steps down, Shift+Tab up; the
  arrow keys move too; letting go of Alt chooses; Escape closes the list and
  leaves the focus where it was. A click on a row chooses it.
- **The order is the order of use**, most recent first, kept by the server: a
  window moves to the front when it gets the keyboard; a new window enters at
  the front; a closed one leaves. One list for all monitors (there are no
  workspaces). Minimized windows are in it at their place in the order, shown
  as minimized; choosing one brings it back.
- **The list**: in the middle of the monitor with the focused
  window, a frosted panel of rows, each the application's icon (the dock's,
  from its desktop entry, a letter tile where there is none) and the window's
  title, cut with an ellipsis; light and dark, contrast checked by look
  milestone 1's test. It appears when Alt has been held 0.15 s after the
  first Tab, so a quick Alt+Tab switches without a flash. At most as many rows
  as fit; it scrolls past that.
- **Alt+`** (the key above Tab) does the same among the focused application's
  windows only: useful with one terminal process for all its windows.

## Progress

- **Step 1 in** (2026-11-14): the order of use is the seat's
  (`Seat::order_of_use`, kept wherever a window takes the keyboard: a press,
  a finger, the server's own focus; a window leaves it when it goes, not when
  minimized); the seat names Tab, the key above Tab (by position, evdev 41),
  Alt and the arrows; its filter switches with Alt+Tab and Alt+` (every key
  the switcher's while it runs, Escape cancels, Alt let go chooses; the
  popup keeps its keys; Ctrl or Super with Alt+Tab stay the program's) and
  turns Super with an arrow into a window key, each behind its setting
  (`set_window_keys`). The server (switch.rs) snapshots the dock's windows
  in that order when switching begins and steps through them; the choice
  takes the keyboard, a minimized one coming back. Alt+Tab works already,
  unseen (the list is step 4); the window keys are caught and do nothing
  until step 3. 12 new seat tests; the fuzz test takes the new keys, checks
  the order of use (no window twice, the focused one first) and that every
  new rule runs. 414 tests on Hubble, kepler and Techbench.
  Run live on kepler by vctl (installed, restarted): Alt+Tab goes to the
  window used before; Alt+Tab+Tab past the used ones reaches a window never
  used (foot); Alt+Shift+Tab steps back; Alt+Tab+Escape leaves the focus;
  Alt+` alternates the two terminal windows and never reaches foot, and
  from foot (alone in its application) stays. vctl gained `used` (the order
  of use) and chords with several keys (`alt+Tab+Tab`, `grave`).
  **Found**: the server gives a new window no keyboard of its own (only a
  click, a finger, the dock or vctl do): a window opened from the launcher
  waits for a click before it can be typed in.
  **Fixed the same day** (Dave: "build the new window focus first";
  opened.rs): a new window takes the keyboard when nothing has it, when it
  is the focused program's own (a new terminal window, a dialog, a torn-off
  tab), or when its application was just started from the dock, the
  launcher or session restore (shelf.rs `launched`, the first window of
  each start within 15 s) and no key has gone into another window since.
  Otherwise it opens behind and its dock icon's long mark blinks twice a
  second (steady where motion is not wanted) until it has had the keyboard.
  Found on the way: a Wayland window is in the space from its creation, so
  the dock's `starting` had already ended when it was first shown; hence
  `launched`, kept apart. Live on kepler by vctl: after a restart the
  restored windows take the keyboard; foot spawned by itself under a
  focused terminal stays behind and blinks (seen in shots); foot started
  from the launcher takes the keyboard; a second terminal window takes it.
  4 tests of the rule; 418 on Hubble, kepler and Techbench.
- **Step 2 in** (2026-11-14): vergence-shell `snap` (the zones: halves,
  quarters by 24 points along either edge, the top edge, the bottom edge
  alone nothing; shared edges; rectangles in the work area, 7 tests); the
  server's snap.rs (desktop.toml `[windows]` snap, top-edge, between-screens,
  preview, read with the shelf's settings; the aim while a window is moved by
  the pointer or by a finger on its bar; the preview after 0.15 s at an outer
  edge, 0.5 s at a shared one; the snap on release; out again by dragging,
  at the size it had where the drag took it from; double-click maximizes and
  restores to that place; xdg-shell tiled states for Wayland windows, nothing
  for native ones, whose frames are the server's, so no bus protocol change;
  window memory's `snap` field, put back when the window is first shown and
  refitted when the setup changes; `vctl snap`). Snapped windows are kept in
  `Snap::snapped` with their place before, apart from `Server::restore`,
  which several parts read as "maximized"; they keep round corners.
  **Deviation**: the preview is a rounded accent tint drawn over the windows
  (the dragged one too), not frosted under the dragged window: drawing between
  windows needs the compositing order changed; noted for later. Live on
  kepler by vctl: off by default (a drag to the edge only moves); with
  `snap = true`, the left half (preview seen in a shot), out again at its own
  size, the top-left quarter, the top edge maximizing and out again, the
  bottom edge nothing, the right half; foot snapped right came back snapped
  after a restart; with the dock set never to hide (keep-clear) the right half
  stops at the dock (935 to 1870), with it hiding it reaches the edge. 426
  tests on Hubble, kepler and Techbench.
- **Step 3 in** (2026-11-14): vergence-shell `key_act` and `beside` (the
  window keys' rule, the screen beside by overlap up and down; 5 tests: a
  window walked across every half of two screens and back, quarters along
  their row and between rows, Up and Down, screens diagonal or missing); the
  server's `window_key` applies it to the window with the keyboard (the
  screen most of it is on). Live on kepler by vctl with drag-snapping off:
  Right from the right half nothing (one screen), Up maximized, Down back to
  its place before the snap, Left the left half, Down back, Down minimized;
  then Alt+Tab brought the minimized window back. 431 tests on Hubble,
  kepler and Techbench.
- **Step 4 in** (2026-11-14): `[windows] snap = "ctrl"` by default (Off,
  Ctrl, Always; true and false still read), Ctrl held at some time during the
  drag arms it, and pressing Ctrl at an edge shows the preview without a
  move; the seat's `Out::WindowMenu` for a right click on the title bar or
  maximize button and a long still press on the title bar (a finger that
  moves along it is a move; a long press on the button is not used: a finger
  there maximizes on contact); the window's menu (menu.rs): Left half, Right
  half, the quarters, (Top half), Maximize, Back to where it was, and from
  the bar Minimize and Close, each layout with a 24 x 16 picture of its part;
  `layouts_tick` opens it after 0.7 s on the maximize button and closes it
  once the pointer has left both (`layouts-hover`); and, from Dave the same
  morning ("should the windows have a little buffer between them? Like the
  terminal when we split it?"), 3 points round and between snapped windows
  (6 at first: "a little too much ... about half that much")
  (`gaps`, on), the preview showing exactly where the window goes. Live on
  kepler by vctl: a plain drag to the edge only moves, Ctrl tapped at the
  edge aims and snaps; the bar's right-click menu (shot) and Right half
  chosen from it; the hover opens it (shot), a row lights under the pointer,
  it closes when the pointer leaves; terminal and foot side by side 6 points
  apart (shot). 434 tests on Hubble, kepler and Techbench.
  **Found**: vergence-render's GPU test hung once on kepler (2026-11-14
  09:04, 0% CPU for 16 minutes, the session running on the same card);
  killed, it then passed three times alone in 0.7 s. Intermittent; to watch.
- **Step 5 in** (2026-11-14): Alt+Tab's list (switch.rs): once Alt has been
  held 0.15 s after the first Tab, a frosted panel in the middle of the
  focused window's screen (two fifths of its width within 360 to 640 points,
  rows of 40, at most three fifths of its height, then it scrolls to keep the
  one stepped to in view): the dock item's icon and the window's title (no
  control characters, at most 200, cut with an ellipsis), minimized windows
  faint and marked, the one stepped to shaded (its text full strength even
  when minimized: faint ink on the shading did not read, seen in a shot); a
  click on a row chooses it (the seat's `Part::Switcher`). vctl gained
  `switching` and `hold`/`let-go` (a modifier held across commands). Live on
  kepler: the list over two and three windows (shots), Escape left the focus
  on foot, a click on minimized foot's row brought it back with the keyboard.
  434 tests on Hubble, kepler and Techbench.
- **Step 6 in** (2026-11-14): Settings > Windows, "Snapping and switching":
  the drag setting (Off / While Ctrl is held / Always), the top edge, edges
  between screens, the preview, room between snapped windows, the layouts on
  the maximize button's hover, the window keys, what Alt+Tab shows (every
  screen / this screen: the server keeps to the focused window's screen,
  minimized windows by where they go back to), Alt+`; the rows that only
  matter while drag snapping is on are disabled while it is Off. The server
  tells the seat the window keys' and Alt+`'s settings. Whitepaper §6.9 and
  §9 (Draft 0.13), README, CLAUDE.md. Live on Hubble: the page (shot; the
  tree lists every row with its default), and `window-keys = false` taking
  Super+Left from the server at once, back on when removed. 434 tests on
  Hubble and Techbench; kepler's suite waits for the GPU loop it is running.
- **Acceptance 3 and 7, and two findings** (2026-11-14, Hubble): a nested
  server with two 1280 x 800 virtual screens side by side (scale 1.125):
  Super+Right walked foot from the left screen's right half (beside its
  dock) to the right screen's left and right halves, then nothing. **Found**:
  the screens overlapped by a point (854 wide, the next at 853: the layout
  rounds at a fractional scale), and the shared edge snapped from both
  sides; vergence-shell's `zone_at` now counts screens within a point of
  each other as touching (a test of that layout). Then with the default
  "never" the shared edge aimed at nothing from either side and the outer
  edge snapped; with "press", let go after 0.1 s the window only moved,
  after 0.7 s it snapped. Clicks (vterm-stress, 50 tab clicks at 144 Hz):
  50 of 50 drawn, each at most a frame after the press reached the
  terminal, but 3.4 to 3.6 ms on average and 15.5 to 18 ms at worst in
  light style against 1.85 and 12.0 in dark. **Found**: the terminal's
  light contrast floor (e32c846) recomputed each cell's pair whenever the
  pair changed; now remembered per pair (`readable_cached`): light 2.73 to
  2.83 ms, worst 13.2 to 17.5 ms; still about a millisecond over dark, to
  look at again. 436 tests on Hubble, kepler (under test-guard) and
  Techbench.
- **Acceptance, where it stands** (2026-11-14): 1 met on Hubble, kepler and
  Techbench. 2 met on kepler by
  vctl (native terminal and foot; with a finger not yet tried by hand; the
  settings app snapped on Hubble by key). 3 not yet: the nested server with
  two virtual monitors. 4 met on kepler (Alt+Tab, the list, minimized, Escape,
  Alt+`). 5 met for a logout and login on kepler (refitting to a new size of
  the setup by the unit tests). 6 met (snapping Off/Ctrl by default; window
  keys with drag snapping off). 7 met (50 of 50 within a frame of arriving;
  the light-style cost above). Left: a finger drag on kepler by hand.

## Steps

1. **The order of use and the keys in the seat**: the server's list of
   windows by use (focus.rs or input.rs), the seat's new keys (Tab, the
   arrows, the key above Tab) and filter rules (Alt+Tab held, Super with an
   arrow) with tests, the fuzz test taking the new keys.
2. **Snapping by dragging**: edge and corner zones from each monitor's work
   area and its neighbors; the preview; the snap on release; out again by
   dragging; tiled states for Wayland, the Configure field for native
   windows (bus protocol v4), X11 configure; window memory's zone; tests of
   the zones (a dock on each edge, two monitors side by side and stacked,
   scales 1, 2 and 2.5).
3. **Snapping by keys**: Super+arrows, across monitors.
4. **Ctrl+drag and the maximize button's layouts**: the three-way drag
   setting; the panel (hover after 0.7 s, right-click, long press), its
   pictures, light and dark, frosted; Snap to… in the title bar's menu.
5. **Alt+Tab**: the panel, the rows, the keys, the delay, minimized windows,
   the click; Alt+`.
6. **The settings and the docs**: Settings > Windows, every row in the table
   above; whitepaper §6.9 (snapping off by default, the top edge, the dock
   as its settings say) and §9; shelf milestone 1's pointer; this plan's
   notes.

## Acceptance

1. Tests green on Hubble, kepler and Techbench.
2. With snapping switched on, dragged by the bar to the left edge, a native terminal, foot (Wayland) and
   the settings app each take the left half of the work area (beside the
   dock, not under it), with the preview before letting go; to a corner, a
   quarter; to the top, maximized. Dragged away, each comes out at its
   earlier size under the pointer. On kepler, the same with a finger. With
   the dock set to hide when a window covers it, the left half reaches the
   screen's edge and the dock hides behind it; set never to hide, the half
   stops at the dock.
3. On a nested server with two virtual monitors side by side (vctl), the
   shared edge lets a window across; the outer edges snap. Super+Right
   walks a window from the left monitor's left half to its right half, then
   to the right monitor's left half, then its right half.
4. With five windows, Alt+Tab once gives the previous window, without the
   list showing; holding Alt shows the list in the order of use within a
   frame of the delay; a minimized window chosen from it comes back;
   Escape leaves the focus where it was. Alt+` steps among the terminal's
   windows only.
5. A snapped window comes back snapped after logging out and in, and refits
   when the setup's resolution or scale changes.
6. On a fresh profile snapping is off: dragging to an edge only moves the
   window; Super+arrows still snap. Every row of the Windows page's new group
   changes the behavior live, and the dimmed rows follow the first.
7. Clicks still drawn within a frame (50 of 50) on both roads with the new
   keys in the filter.

## Security (whitepaper §7.3)

- No program can take Alt+Tab, Alt+` or Super+arrows from the server, or snap,
  maximize or move another program's window: the keys are filtered before
  any window has them, and places stay the server's to decide (Contract 2).
- Window titles in the list are a program's text: drawn as plain text,
  control characters dropped, length limited, as in the bar.
- The list shows the titles of every open window, including minimized ones,
  to whoever is at the keyboard; no more than the dock's icon cards will.

## Not in this milestone

- Resizing two windows snapped side by side together, and offering the other
  windows for the free half after a snap (Windows' Snap Assist).
- Live window pictures in the list; they belong with the
  dock's icon cards (whitepaper §6.9, 2026-11-13).
- Dialogs moving with their parent window (whitepaper §6.9): its own step,
  with the native protocol's parent.
- The switcher read by a screen reader: phase 3, with the server's own parts
  in Contract 3.

## Questions for Dave (answered 2026-11-14)

Dave: snapping off by default, sane defaults, every choice in the settings
app; the dock counts as the edge as its own hide setting says. So each
question became a setting with the proposed answer as its default:

1. **The top edge**: Maximizes (default) or Top half.
2. **Edges between screens**: Never snap (default) or Snap after a short
   press.
3. **The list**: rows of icon and title. Live window pictures are not a
   setting yet; they come with the dock's cards.
4. **Alt+`**: on (default), switchable.
