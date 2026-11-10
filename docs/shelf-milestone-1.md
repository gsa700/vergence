# Shelf milestone 1: the strip, the apps popup, running apps (whitepaper §6.9)

Planned 2026-11-09 (Dave: "write the shelf milestone", after the desktop
paradigm discussion that became whitepaper §6.9); plan accepted the same
evening. **Begun 2026-11-10**, the day the toolkit's milestone closed;
Progress, below, says how far it has come.

**Order changed the same evening (Dave, 2026-11-09 20:31):** the project's own
toolkit comes first (whitepaper §6.10, §9), with the terminal as its first
user, and the shelf is built after it **on the toolkit's drawing code**
instead of the server's present solid rectangles and per-label rasters. The
steps and decisions below stand; "Drawing" under Design, and the icon
decoding note under Security, are to be revised against the toolkit when its
first milestone is written.

The goal: a session where you can see what is installed, start it, see what
is running and get back to it, without the right-click menu. That is the
shelf's strip, its apps button with the popup grid, and one icon for each
pinned or running application. Live tiles are shelf milestone 2.

## Where the server stands (read 2026-11-09, a1509e8)

- **Starting apps**: only the desktop menu (`menu.rs`): `[[menu.item]]` in
  desktop.toml, else a fixed list of five. Nothing reads desktop entries.
- **No work area.** `toggle_maximize` (decor.rs) fills the whole monitor,
  and `content_area` (place.rs) is the monitor less a title bar. Nothing
  can reserve an edge.
- **No minimise.** The settings app has the Minimise switch and the
  terminal draws the button, but the server has no minimised state: the
  request goes nowhere.
- **No primary screen in the server.** `[displays] primary` is written by
  the settings app and read only there (`wall::primary`: the recorded one if
  connected, else the first in arrangement order).
- **Keys**: Ctrl+Alt+Backspace, Escape for the menu, VT switching. No
  desktop key bindings.
- **Drawing**: title bars, the menu and notification cards are drawn by the
  server (solid rectangles, text rasterised per monitor at its own pixels,
  sized in points). There is no icon drawing, but `image` (PNG) and `resvg`
  (SVG) are already in the server for wallpapers and cursors.
- **Windows know their app**: Wayland app_id or X11 WM_CLASS, as window
  memory uses them (memory.rs).
- **On Hubble (Void)**: icon themes Adwaita and hicolor; desktop entries for
  Firefox, htop and Xwayland only. Our own terminal and settings app
  install no desktop entry, so they would not appear in the grid.

## Design

- **The server draws the shelf**, as it draws title bars, the menu and
  notifications (`shelf.rs`). A shelf that was a separate program would need
  a protocol to dock a surface to an edge and another to list and control
  other applications' windows, which is exactly the access §7.3 keeps away
  from ordinary clients; and whitepaper §6.9 already has the server drawing
  the tiles. The cost: more code in the server, and text entry (the search
  field) in the server for the first time.
- **The logic is a library crate, `vergence-shell`**, with no Smithay in
  it: desktop entries, icon lookup, matching windows to applications, the
  search, the shelf's layout arithmetic. It is tested on all three
  platforms like vergence-profile. `shelf.rs` only draws and routes input.
- **Units are points** (Contract 1): the strip 48 points wide,
  icons 32, so the same physical size on every monitor. One setting later:
  small, medium, large.
- **Configuration** in desktop.toml, re-read within a second like the rest:

      [shelf]
      hide = false                 # true: shown only when the pointer reaches the edge
      pinned = ["firefox", "vergence-term"]   # desktop entry ids, top to bottom
      screens = ["<id>"]           # other screens with a shelf of their own (step 6)

  The edge is the right one. It is not a setting in this milestone.
- **Applications are desktop entries** (`*.desktop` in
  `$XDG_DATA_HOME/applications` and each `$XDG_DATA_DIRS/applications`,
  which covers `/usr/share` on Linux and `/usr/local/share` on FreeBSD).
  This is a file format read at the edge (principle 4), parsed by our own
  code: Name, Exec, Icon, TryExec, Terminal, NoDisplay, Hidden,
  StartupWMClass. A user's file hides the system's of the same id.
- **Exec runs without a shell.** The Exec line is split into arguments by
  the specification's quoting rules and the field codes (%u, %f...) are
  dropped; the menu's `sh -c` is not copied. `Terminal=true` entries (htop)
  run inside vergence-term.
- **Icons**: looked up by name in the icon theme from the settings
  (`icon-theme`), then hicolor, then `/usr/share/pixmaps`; the size nearest
  to what the monitor needs (32 points times its scale), SVG preferred when
  there is one, so icons are sharp at every scale. An application with no
  icon gets a tile with its first letter. Rasterised once per (icon, pixel
  size) and kept.
- **A window belongs to the application** whose desktop entry id equals its
  app_id or WM_CLASS (case ignored), else whose StartupWMClass does, else
  whose Exec's program name does. A window that matches nothing still gets
  an icon (the letter tile, its class as the name), so nothing running is
  ever missing from the shelf.

## Steps

1. **Work area and the bare strip.** The server learns the primary screen
   (the settings app's rule, moved into vergence-profile or vergence-shell
   so both use one function) and draws an empty strip on its right edge.
   A screen with a shelf has a **work area**, the monitor less the strip:
   maximise, default placement, fitting, the menu and notification cards
   stay inside it. A remembered place that the strip would cover is nudged
   left. `hide = true` is read but does nothing yet. `vctl shelf` reports
   the strip's rectangle and, later, its items.
2. **Installed applications** (`vergence-shell`, no drawing): desktop
   entries, Exec splitting, icon lookup. `tools/install.sh` installs desktop
   entries and icons for vergence-term and vergence-settings. A small
   command, `vshell apps`, lists what the shelf will see (name, id, exec,
   icon file), for checking a machine over SSH.
3. **Running applications.** One icon per application with a window open,
   in the order they started, with a mark beside it; the focused one's mark
   is in the accent colour. The name appears beside the icon when the
   pointer rests on it. A click: one window, focus and raise it; several, a
   short list opens to the left (title, and the screen it is on when that
   is not the shelf's). **Minimise** arrives here, since the shelf is where
   minimised windows go: the server honours the minimise request (Wayland
   and X11), its own title bars gain the button when the settings name it,
   and a minimised window comes back by its icon.
4. **Pinned applications.** `pinned` entries sit at the top of the
   applications part whether running or not; a click starts one (window
   memory already puts it back where it was on this setup: "remembered
   launch" needs no new code). Right-click on an icon: Pin or Unpin, New
   window, Close. Until its first window appears a started application's
   icon shows that it is starting (no movement when reduce-motion is on).
5. **The apps button and popup.** The button at the top of the strip opens
   a popup to the left of it: a search field and a grid of every installed
   application, icon and name, sorted by name, scrolling when they don't
   fit. While it is open the keyboard is the popup's: typing filters (the
   start of a word in the name first, then anywhere, then the entry's
   keywords), arrows move, Enter starts the selected one, Escape or a click
   elsewhere closes. **The Super key, tapped alone, opens it**
   from anywhere. `vctl type` and `vctl key` are added so the popup can be
   tested over SSH.
6. **Settings, hiding, and other screens.** The settings app: on each
   screen's page "A shelf on this screen" (always on for the primary), and
   a Shelf page (hide automatically, the pinned list and its order).
   `hide = true` works: the strip slides out of the way, the work area is
   the whole monitor, and the pointer resting at the edge brings it back. A
   second screen's shelf shows that screen's windows and the main shelf
   leaves them out. Unplugging the primary moves the shelf to the new
   primary.

When step 5 is in, the desktop menu drops its fixed list of applications
and keeps `[[menu.item]]` entries and Exit Vergence.

**Order (Dave, 2026-11-10):** steps 1 to 5, then the toolkit's widgets
and the settings app's move onto them, then step 6, whose two settings
pages are written in the new settings app; nothing is added to the iced
one.

## Progress

- **Step 1 (2026-11-10): the strip and the work area.**
  - `crates/vergence-shell` (no Smithay; it depends only on
    vergence-look): the strip on a screen's right edge, 48 points wide;
    the work area it leaves; a remembered window moved clear of it.
    Three tests of the arithmetic, on Hubble's two screens.
  - The primary screen is one rule for the settings app and the server:
    `vergence_profile::primary` (the one recorded in `[displays]
    primary` if it is connected, else the first as the server lists
    them).
  - The server (`shelf.rs`): the strip on the primary screen, in front
    of every window and behind the menu and the notification cards.
    Maximise, default placement, fitting, the rescue of a window from a
    screen that went, the desktop menu and the notification cards use
    the work area; a remembered place the strip would cover is moved
    left; a maximised window is fitted again when the strip moves. A
    press on the strip does nothing and reaches nothing behind it.
    `[shelf] hide` is read and not acted on. `vctl shelf` says where the
    strip and the work area are.
  - **The strip is drawn by the project's own drawing code**, the first
    thing in the server that is: a scene, rendered by
    `vergence-render`'s CPU renderer at the screen's exact pixels into a
    buffer the server shows, kept until its height or scale changes.
    Its ground and edge are in `vergence-look`.
  - On Hubble (the 14" 4K alone, scale 3): the strip at 1232,0, 48 x 720
    points; a terminal started maximised is 1232 x 720 and stops at it;
    a notification card sits 12 points left of it. Not yet looked at:
    its width with a ruler on two screens (acceptance 1; the ASUS is on
    kepler), and a second screen at all.
  Tests green on Hubble, kepler, canary and Techbench (259 on the two
  FreeBSD machines, 258 on the Linux ones).

- **Step 2 (2026-11-10): installed applications.** All in
  `vergence-shell`, with no drawing:
  - `desktop.rs` reads a desktop entry (Type, Name, Exec, Icon, TryExec,
    Terminal, NoDisplay, Hidden, StartupWMClass, Keywords; the first
    `[Desktop Entry]` group only) and splits its Exec line by the
    specification's rules, field codes dropped. Nothing is run through a
    shell.
  - `apps.rs`: the XDG data directories (the user's, then
    `/usr/local/share` and `/usr/share` unless `XDG_DATA_DIRS` says
    otherwise); the first directory that has an id wins and a Hidden
    entry there removes it; an entry whose TryExec program is missing is
    left out; sorted by name.
  - `icons.rs`: an icon by name in the settings' theme, the themes it
    inherits from, hicolor, then `pixmaps`. A theme's index is the first
    found and its directories are looked for under every data directory
    (our own icons are in `/usr/local/share/icons/hicolor`, whose index
    is `/usr/share`'s). An SVG is preferred; else the nearest size, the
    larger first. Found on Hubble: hicolor calls its 256 x 256 directory
    scalable, so a PNG is ranked by its size whatever its directory says.
  - `vshell apps` lists what the shelf will see: name, id, what is run,
    the icon file.
  - `data/applications` and `data/icons`: desktop entries and icons
    (SVG, drawn by the project) for the terminal and the settings app;
    `tools/install.sh` installs them, and `vshell`.
  - Tests: 10 of the reader, the splitter, the directories and the icon
    choice, and a fuzz test of the reader and the splitter (three kinds,
    400,000 rounds clean on Hubble).
  `vshell apps` on the four machines: Hubble 6 listed (AlbumWall, btop,
  Firefox, Htop, Settings, Terminal), each with an icon file; kepler 4;
  canary 49 of its Xfce's 57 entries, 46 with a file; Techbench 63 of a
  GNOME's 238 (175 are marked not for listing), 60 with a file. The few
  without one name icons that only a full icon theme has; the shelf will
  pass the settings' theme and draw a letter where there is still none.
  Tests green on all four (272 on the FreeBSD machines, 271 on Linux).

## Acceptance

Every step: `cargo test --workspace` green on Hubble, canary and Techbench,
the server builds with no new warnings.

1. On Hubble's two monitors: the strip is the same width measured with a
   ruler on both when the shelf is moved between them (primary changed in
   Settings). A maximised window stops at the strip. Notification cards
   and the desktop menu do not cover it. A window remembered under where
   the strip now is opens clear of it. Tests: the work-area arithmetic.
2. `vshell apps` lists Firefox, htop, Terminal and Settings on Hubble with
   an icon file each, the same on canary (FreeBSD paths) and Techbench (a
   full GNOME's worth of entries, none crashing the parser). Tests: Exec
   splitting (quotes, escapes, field codes), a user's entry hiding the
   system's, NoDisplay and Hidden, icon choice by size, and a fuzz test of
   the entry parser beside the others.
3. Start Firefox, two terminals and AlbumWall: three icons, the two
   terminals sharing one. The focused application's mark follows the focus.
   The terminals' icon lists both windows and each one comes forward.
   Minimise from the terminal's button, from Firefox's, and from a server
   title bar: the window goes away, its icon stays, a click brings it back
   to the same place. Icons are sharp in a screenshot of each monitor
   (`vctl shot`) at 2.25 and at 0.75.
4. Pin Firefox and the terminal, restart the session: both are there, not
   running. A click starts Firefox where it last was. Unpin from the
   right-click menu removes it once it is closed.
5. The key opens the popup; "fire", Enter starts Firefox; Escape closes it
   without starting anything; no typed letter reaches the window that had
   the focus. On Techbench the grid scrolls. Driven by `vctl` as well as by
   hand.
6. Hide on: a maximised window uses the whole monitor, the strip returns
   at the edge and leaves again. A shelf on the second monitor shows only
   its windows. Unplug the primary: the shelf is on the other monitor
   within a second, and back after the replug.

Done, as every milestone, when it has run on Hubble (Void), on Techbench
(Fedora) and builds and passes on canary (FreeBSD).

## Security (whitepaper §7.3)

- The shelf adds no socket and no protocol. Applications cannot draw in it,
  dock to it or read its list of windows.
- Desktop entries and icons are files the user or the system's packages
  installed: the same trust as the programs they start. They are still
  parsed as untrusted input (size limits, a fuzz test), because a bad file
  must not take the session down.
- **Icon decoding runs inside the server** (PNG through `image`, SVG
  through `resvg`), which is new exposure for the process that owns the
  screen and the input. Limits: file size, pixel dimensions, and SVG
  rendering off the main thread with a time limit. Moving decoding into a
  small helper process is the stronger answer and is noted for the next
  security pass.
- While the popup has the keyboard, keys go nowhere else, and the popup
  closes when the screens go off (energy.rs) so it never holds input on a
  dark screen.

## Decided (Dave, 2026-11-09 19:36)

1. **The key that opens the apps popup** is the Super (logo) key tapped
   alone.
2. **A click on the icon of the focused application's only window**
   minimises it.
3. **The strip is 48 points wide, icons 32**, to be looked at on both
   monitors in step 1.
4. **Icons.** The shelf reads the icon themes that are installed and draws
   the letter tile otherwise, so nothing is borrowed into the project.
   Vergence's own tools (terminal, settings, the apps button) get icons
   drawn by the project, installed with their desktop entries (step 2).
5. **The server draws the shelf** (Design, first point).

## Not in this milestone

- **Live tiles** (clock, volume, network, battery) and their bus protocol:
  shelf milestone 2. The strip's bottom part stays empty until then.
- **Away windows** (windows of an unplugged monitor shown on the shelf).
- **Named desktop actions** in the search field.
- **Snapping and Alt+Tab** (whitepaper §6.9, Windows): a windows milestone
  of their own.
- Dragging icons to reorder or to pin, badges and progress on icons, a
  desktop entry's extra actions in the right-click menu, recently used
  ordering in the grid, touch.
- The shelf on any edge but the right.
