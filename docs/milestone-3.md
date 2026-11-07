# Milestone 3: remember where windows go

Whitepaper §4.4. Milestone 2 puts the monitors back the way they were for
each setup. Milestone 3 puts the **windows** back: when a monitor comes back
(plug, dock, wake), every window that lived on it returns to its place,
size and state; and optionally, when an application starts, its window
opens where it was last time on this setup.

**Order changed on 2026-11-06 (Dave):** window memory is milestone 3 and
scaling milestone 4. Window memory needs no new hardware and is tested with
the fake monitor; scaling waits for real monitors of different densities.

**Status: planned, not started.** Dave decided all four on 2026-11-06, as
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
