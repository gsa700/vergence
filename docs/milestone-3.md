# Milestone 3: remember where windows go

Whitepaper §4.4. Milestone 2 puts the monitors back the way they were for
each setup. Milestone 3 puts the **windows** back: when a monitor comes back
(plug, dock, wake), every window that lived on it returns to its place,
size and state; and optionally, when an application starts, its window
opens where it was last time on this setup.

**Order changed on 2026-11-06 (Dave):** window memory is milestone 3 and
scaling milestone 4. Window memory needs no new hardware and is tested with
the fake monitor; scaling waits for real monitors of different densities.

**Status (2026-11-06): built; acceptance 1, 2 (ports: not yet), 3, 4, 5, 6,
7 and app start pass on Hubble; 8 (real monitors) still to do.**
Dave decided all four on 2026-11-06, as
recommended: (1a) restore on monitor changes and at app start, with an
opt-out list; (2a) never store titles; (3a) remember workspaces; (4a)
install Thunderbird on Hubble for the tests.

## What xfwm4 offers (checked on Hubble)

`_NET_CLIENT_LIST` (which windows exist), `_NET_MOVERESIZE_WINDOW` (move
and resize a window through the window manager), `_NET_WM_STATE` with
`_MAXIMIZED_HORZ/VERT` and fullscreen, `_NET_WM_DESKTOP` (workspace),
`_NET_FRAME_EXTENTS` (xfwm4's border) and `_GTK_FRAME_EXTENTS` (the
invisible shadow GTK apps draw themselves). That is all this milestone
needs; it works through the window manager, never around it.

## What is remembered

Per monitor setup (profile key), per window:

- **Which window:** its class (`WM_CLASS`, e.g. `Navigator`/`firefox`),
  its role (`WM_WINDOW_ROLE`) if it has one, and its place among windows of
  the same class (1st, 2nd... by when they opened). See decision 2 on
  titles.
- **Where:** the monitor (by EDID ID, as in milestone 2) and the position
  **relative to that monitor's corner**, so moving a monitor in the
  arrangement does not throw its windows off; the size; maximised
  (horizontally, vertically), fullscreen; the workspace (decision 3).

Only ordinary application windows: `_NET_WM_WINDOW_TYPE_NORMAL`, not
dialogs, menus, docks, panels, the desktop, or windows that skip the
taskbar.

## How

- **Tracking:** vergd watches `_NET_CLIENT_LIST` and each window's
  `ConfigureNotify` and `_NET_WM_STATE` changes, and keeps the last known
  place of every window for the current setup.
- **Not recording the evacuation:** when a monitor goes, xfwm4 moves its
  windows to the remaining screens. Those moves must not overwrite the
  remembered places. So window changes are only recorded when the monitor
  setup has been **stable for 3 s**; moves during or right after a RandR
  change are ignored.
- **Restoring on a setup change:** after milestone 2 has put the monitors
  back (and the desktop has settled), every open window with a remembered
  place for the new setup is moved there: un-maximise first, move and
  resize with `_NET_MOVERESIZE_WINDOW`, then re-maximise or fullscreen,
  then the workspace.
- **Shadows and borders:** positions are stored as the window's visible
  frame. `_GTK_FRAME_EXTENTS` and `_NET_FRAME_EXTENTS` are subtracted and
  added back on restore, so a window does not creep a few pixels on every
  restore (the drift check in acceptance 4 catches it if it does).
- **Restoring on start (decision 1):** when a new window appears whose class
  (and role, and place among its class) has a remembered spot on this
  setup, it is moved there once, about 0.3 s after it maps, after the
  application's own placement. Apps on an opt-out list are left alone.
- **Storage:** `~/.config/vergence/windows.toml`, 0600, separate from the
  profiles (it changes often; the profiles file stays small and readable).
  Written at most every few seconds, and only after the setup is stable.
- **Safety:** `vergd watch --observe-only` still observes only; a
  `--no-windows` switch turns this milestone off on its own; every restore
  is logged ("moved firefox #1 to ASUS VS229 +100+50 1200x900").

## Decisions (Dave, 2026-11-06)

1. **When to restore.** (a) On monitor changes **and** when an app starts,
   with an opt-out list for apps that place themselves. *(recommended)*
   (b) Only on monitor changes; apps place themselves at start.
2. **Window titles.** Titles carry document names, email subjects, web page
   titles. (a) **Never store titles**: class, role and order only.
   *(recommended; the file stays free of private data)* (b) Store titles, to
   tell windows of one app apart better (e.g. which Firefox window is
   which).
3. **Workspaces.** (a) Remember the workspace too, and put windows back on
   theirs. *(recommended)* (b) Leave workspaces alone.
4. **Test apps.** Firefox is on Hubble; the whitepaper names Thunderbird too.
   (a) Install Thunderbird on Hubble for the tests (`pkg install
   thunderbird`). *(recommended)* (b) Test with Firefox, xfce4-terminal and
   Mousepad only.

## Acceptance (on Hubble, with the fake monitor; windows on the fake are invisible, which is fine)

1. Two xfce4-terminals and Mousepad on the fake, one maximised; unplug
   (xfwm4 moves them to the ASUS); replug: each back on the fake, same place,
   size and maximised state.
2. The same after moving the fake from HDMI-1 to HDMI-3 (monitors, not
   ports) and after moving the fake's position in the arrangement (windows
   follow their monitor).
3. Firefox with three windows on different monitors and workspaces, and
   Thunderbird: unplug and replug; then quit and restart both: windows
   return (decision 1a).
4. Drift: 20 unplug/replug cycles; no window moves by even one pixel.
5. Evacuation is not learned: after an unplug the remembered places are
   still the pre-unplug ones (checked in windows.toml).
6. A window moved by hand is remembered, and comes back after the next
   unplug/replug.
7. No titles in windows.toml (decision 2a).
8. With the real monitors: 1 and 3 again.

## Results on Hubble (2026-11-06, fake monitor right of the ASUS)

Two xfce4-terminals (one maximised), xclock and Mousepad on the fake.

- **5, evacuation not learned:** on unplug xfwm4 moved all four onto the
  ASUS; windows.toml was byte for byte unchanged. Pass.
- **1, back on replug:** all four back with the same position, size and
  maximised state. Pass.
- **4, drift:** 10 more unplug/replug cycles; not one pixel moved. Pass.
- **6, moved by hand:** Mousepad moved on the fake, then unplug/replug: back
  at its new place. Pass.
- **App start (decision 1):** xclock closed and started again with no
  position: opened at its remembered place. Pass.
- **7, no titles:** windows.toml holds class, role, order and places only.
  Pass.

**What xfwm4 does by itself:** on replug it moved the three normal windows
back to the fake on its own; vergd's log shows it moved only the
**maximised** terminal (which xfwm4 leaves on the remaining screen) on every
cycle. The result is right either way; vergd's own part is the maximised
window, not learning the evacuation, hand-moved places, and app start
(xfwm4 does neither of the last two).

**Fixed during the run:** xfce4-terminal gives every window a unique role
("xfce4-terminal-1794016006-744545988"), which would never match after a
restart and would add an entry for every terminal ever opened. A role with
a run of 6 or more digits is now dropped (`stable_role`); the window is
known by class and order. Checked: a terminal closed and started again
opened at its remembered place, with one entry in the file.

Test tools installed on Hubble: wmctrl-fork (placing and maximising windows
from scripts) and Thunderbird.

## Acceptance 3 on Hubble (2026-11-06 21:15): Firefox and Thunderbird

Three Firefox windows (fake/workspace 1; ASUS/workspace 2; fake/workspace 3
maximised) and Thunderbird (fake/workspace 2). Firefox set to restore its
session (`browser.startup.page = 3` in the test profile's user.js).

- **Unplug:** windows.toml unchanged. On the ASUS-alone setup vergd moved
  Firefox #1 to the place it had there before.
- **Replug:** all four back on their monitor, place, size, maximised state
  and workspace, including the windows on workspaces not being shown.
  Pass.
- **Quit and restart both:** Firefox restored three windows; vergd placed
  them and Thunderbird; the set of places afterwards was identical to before
  (monitor, place, size, state, workspace). Pass. Windows of one app are
  told apart by opening order only (decision 2a); with blank pages it can't
  be seen whether Firefox reopened them in the same order.

**Found: a window can be stranded on unplug.** Thunderbird, on a workspace
not being shown and with no remembered place on the ASUS-alone setup, was
left by xfwm4 at x=1901 on the 1920-wide screen: 19 px visible. Both
unplugs did the same.

**Fixed (21:21): rescue.** After every monitor change, a window with no
place on the new setup (none remembered, or its monitor is not lit) that is
less than half visible is moved fully onto the monitor it overlaps most, or
else the nearest one, inside the work area (clear of the panel) and allowing
for its title bar; it moves as little as it can. Maximised and fullscreen
windows (xfwm4 fits those itself) and skipped apps are left alone. Checked
on Hubble: Thunderbird's ASUS-alone entry removed, fake plugged and
unplugged; log: "brought Mail/thunderbird (3pane) #1 fully onto ASUS VS229
(it was mostly off-screen)", window at x=620, its right edge on the
screen's edge. The rescued place is then remembered like any other.

## Known limitation: the jump at app start (Dave, 2026-11-06 19:56)

Opening a terminal that was closed earlier: it appears where xfwm4 puts it
(top left), then about 0.7 s later jumps to its remembered place. On X11
the window manager owns placement, so a separate daemon only sees a window
after it is shown and always acts second. Dave: acceptable for now, as long
as the plan gets rid of it. Ways out, in order: (1) place the window before
it is shown (set the position and USPosition on the still-unmapped window
once its WM_CLASS appears; a race, with the current move as fallback);
(2) shorten the delay; and for good: our own window manager (the NeXT-model
step) and then the display server, where placement happens before a window
is ever drawn (whitepaper §6.4, Contract 2).
