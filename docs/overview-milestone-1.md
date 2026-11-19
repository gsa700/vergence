# Overview milestone 1: live window pictures, the dock's picker, Alt+Tab tiles, Super+Tab

Status: **proposed 2026-11-18**, waiting for Dave's acceptance.

## What Dave asked for (2026-11-18)

- "Click on the desktop makes the windows move aside as on the macOS. This
  is a cool feature." (wishlist) "I'd like to do it if we can, we could also
  build out the alt-tab at the same time?"
- "Super+Tab Mission Control style, Alt+Tab live tiles (thumbnails)."
- "Let's use the live tiles as the window picker on the Dock as well! Hover
  on icon shows the live preview (as Windows does). This solves the
  minimized issue too if we combine it with a visual of some kind": dots
  per window, kept under running apps when minimized, as the Mac and
  Windows do, "that is how you know there is something there".
- It also answers the wishlist's "In the Dock, let's work on the minimized
  app icon appearance. It's ambiguous." Today an app whose windows are all
  minimized is drawn faint with no mark (shelf.rs, `away`).

All four use one new piece: **the server drawing a small live picture of
any window**. That comes first.

## Steps

1. **Window pictures** (the server). A picture of a window at a size asked
   for, drawn on the GPU from the window's own surfaces (native, Wayland
   and X11 alike: `render_elements` into an offscreen texture, as
   backdrop.rs does for the frost), kept per window while something shows
   it and dropped when nothing does. Live: redrawn when the window's
   content changes (its damage), at most 15 times a second, only while a
   picker or the overview is open. A window being minimized has its last
   picture kept, so the dock and Alt+Tab can show it while it is off the
   screen. The CPU renderer (`VERGENCE_RENDER=cpu`, canary) draws them too,
   slower. `vctl thumb <i> <w>` saves one, for tests.
2. **The dock's marks.** One dot under the icon for each window, up to
   three (three for more); a minimized window's dot hollow; the focused
   app's dots in the accent color. The icon is never drawn faint for being
   minimized any more: the dots say what is there. Starting and asking
   keep their marks.
3. **The dock's picker.** The pointer resting on a running app's icon
   (0.4 s), or a touch held: a frosted strip above the icon (beside it for
   a dock at the side) with a live tile per window: its picture, its title
   under it, a close button on the tile under the pointer. A click on a tile
   brings that window forward (back from minimized first). A click on the
   icon itself does what it does now. With one window the strip still
   shows, so a minimized window is seen before it comes back. The strip
   goes when the pointer leaves the icon and the strip.
4. **Alt+Tab's tiles.** The list becomes a row of tiles: each window's live
   picture with its app's icon at the corner and its title under it, the
   one stepped to outlined in the accent, minimized ones faint with their
   last picture; many windows wrap to a second row, then the tiles shrink.
   Same keys and timing (the 0.15 s before it shows), clicks still choose.
   Alt+` keeps to one app, as now.
5. **The overview, Super+Tab** (Mission Control's idea). Every window on
   each screen shown at once, scaled down so none overlap (the arrangement
   is worked out in vergence-shell, pure and tested: rows that keep each
   window's shape and rough place), with its title and app icon; the
   dock stays where it is. Click or Enter brings one forward, the arrows
   move between them, typing narrows them by title or app name, Escape or
   Super+Tab again closes, a click on the empty ground closes. Minimized
   windows (question 1).
6. **The desktop moves aside.** A click on the bare desktop (no icon under
   it) slides every window off toward its nearest screen edge, a strip of
   each still showing; a click on the desktop again, or on a window's
   strip, brings them all back. A short animation, none with Reduce motion.
   Super+D (minimize all) stays as it is. A switch on Settings > Windows
   (question 2).
7. **Tests and checks.** The arrangement's unit tests; a display-suite
   scenario that opens the overview and Alt+Tab in a headless session and
   checks the shots (tiles where expected, none overlapping); the pictures
   at every clean scale; by hand: the picker with a mouse and a finger,
   on all three machines.

## Questions

1. **Minimized windows in the overview:** (a) a row along the bottom,
   faint, with their last pictures (recommended: the overview shows
   everything); (b) left out, as the Mac does.
2. **The desktop moving aside:** (a) on unless switched off, as the Mac
   (recommended); (b) off unless switched on.

## Not in this milestone

Spaces (several desktops), dragging a window between screens in the
overview, a three- or four-finger swipe to open it (needs touchpad
gestures first), previews of tabs inside one window.
