# Windows milestone 1: snapping and Alt+Tab

Drafted 2026-11-14 (Dave: "draft the windows milestone"), from whitepaper
§6.9, Windows: "dragging a window to an edge gives it half the screen, to a
corner a quarter, with the shelf counting as the edge. It can be switched
off", and "Alt+Tab steps through windows, most recently used first, in a
small list in the middle of the screen". Shelf milestone 1 left both for "a
windows milestone of their own". **Not accepted yet: four questions at the
end.**

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
  - the top edge: maximized (Question 1);
  - the bottom edge: nothing (it is where the dock usually is, and a window
    dragged down is usually being moved, not placed).
  "The monitor" is its work area: the dock counts as the edge, so a window
  snapped beside a dock on the left starts where the dock ends.
- **Only outer edges snap** (Question 2): an edge with another monitor beyond
  it lets the window across, as now. The keys below reach every half on
  every monitor.
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
- **Switched off** in Settings > Windows ("Snap windows to the screen's
  edges", on by default; desktop.toml `[windows] snap`). The keys still work
  when it is off: they are asked for, never stumbled into.

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
- **The list** (Question 3): in the middle of the monitor with the focused
  window, a frosted panel of rows, each the application's icon (the dock's,
  from its desktop entry, a letter tile where there is none) and the window's
  title, cut with an ellipsis; light and dark, contrast checked by look
  milestone 1's test. It appears when Alt has been held 0.15 s after the
  first Tab, so a quick Alt+Tab switches without a flash. At most as many rows
  as fit; it scrolls past that.
- **Alt+`** (the key above Tab) does the same among the focused application's
  windows only (Question 4): useful with one terminal process for all its
  windows.

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
4. **Alt+Tab**: the panel, the rows, the keys, the delay, minimized windows,
   the click; Alt+`.
5. **The switch and the docs**: Settings > Windows; whitepaper §6.9 and §9;
   shelf milestone 1's pointer; this plan's notes.

## Acceptance

1. Tests green on Hubble, kepler and Techbench.
2. Dragged by the bar to the left edge, a native terminal, foot (Wayland) and
   the settings app each take the left half of the work area (beside the
   dock, not under it), with the preview before letting go; to a corner, a
   quarter; to the top, maximized. Dragged away, each comes out at its
   earlier size under the pointer. On kepler, the same with a finger.
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
6. With snapping switched off, dragging to an edge only moves the window;
   Super+arrows still snap.
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
- Live window pictures in the list (Question 3 (b)); they belong with the
  dock's icon cards (whitepaper §6.9, 2026-11-13).
- Dialogs moving with their parent window (whitepaper §6.9): its own step,
  with the native protocol's parent.
- The switcher read by a screen reader: phase 3, with the server's own parts
  in Contract 3.

## Questions for Dave

1. **The top edge.** (a) Maximizes (Windows, GNOME, macOS's tiling); (b) the
   top half, so every edge is a half as the whitepaper's sentence reads.
   Proposed: **(a)**: a top half is rarely wanted on a wide monitor, and the
   whitepaper's sentence gets a word changed.
2. **Edges between monitors.** (a) Never snap there; the keys reach those
   halves; (b) snap after the pointer has pressed against the edge for half
   a second. Proposed: **(a)**: (b) makes every crossing hesitate.
3. **The list.** (a) Rows of icon and title, small; (b) live pictures of the
   windows. Proposed: **(a)** now: it is what the whitepaper describes,
   reads at a glance with many terminal windows, and costs no window
   pictures; (b) later, with the dock's cards.
4. **Alt+`.** (a) Yes, among the focused application's windows; (b) not
   now. Proposed: **(a)**: small once the list exists, and the terminal's
   windows are all one application.
