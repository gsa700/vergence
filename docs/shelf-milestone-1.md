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
- **No minimize.** The settings app has the Minimize switch and the
  terminal draws the button, but the server has no minimized state: the
  request goes nowhere.
- **No primary screen in the server.** `[displays] primary` is written by
  the settings app and read only there (`wall::primary`: the recorded one if
  connected, else the first in arrangement order).
- **Keys**: Ctrl+Alt+Backspace, Escape for the menu, VT switching. No
  desktop key bindings.
- **Drawing**: title bars, the menu and notification cards are drawn by the
  server (solid rectangles, text rasterized per monitor at its own pixels,
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
      pinned = ["firefox", "vergence-term"]   # desktop entry ids, in their order on the dock
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
  icon gets a tile with its first letter. Rasterized once per (icon, pixel
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
   maximize, default placement, fitting, the menu and notification cards
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
   is in the accent color. The name appears beside the icon when the
   pointer rests on it. A click: one window, focus and raise it; several, a
   short list opens to the left (title, and the screen it is on when that
   is not the shelf's). **Minimize** arrives here, since the shelf is where
   minimized windows go: the server honors the minimize request (Wayland
   and X11), its own title bars gain the button when the settings name it,
   and a minimized window comes back by its icon.
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
    Maximize, default placement, fitting, the rescue of a window from a
    screen that went, the desktop menu and the notification cards use
    the work area; a remembered place the strip would cover is moved
    left; a maximized window is fitted again when the strip moves. A
    press on the strip does nothing and reaches nothing behind it.
    `[shelf] hide` is read and not acted on. `vctl shelf` says where the
    strip and the work area are.
  - **The strip is drawn by the project's own drawing code**, the first
    thing in the server that is: a scene, rendered by
    `vergence-render`'s CPU renderer at the screen's exact pixels into a
    buffer the server shows, kept until its height or scale changes.
    Its ground and edge are in `vergence-look`.
  - On Hubble (the 14" 4K alone, scale 3): the strip at 1232,0, 48 x 720
    points; a terminal started maximized is 1232 x 720 and stops at it;
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

- **Step 3, first part (2026-11-10): running applications on the
  strip.**
  - `vergence-shell`'s `running.rs`: the application a window belongs
    to, by its class (the entry's id, the id's last part, its
    StartupWMClass, its program's name; and a process name the system
    cut to fifteen characters, which is how the settings app is known
    until it names itself); one item for each application in the order
    they started; each item's place under the apps button's. Tests.
  - The server (`shelf.rs`): the windows in the order they were first
    seen, each item's icon and mark drawn on the strip. The mark is on
    the windows' side: short and gray for an application that is
    running, long and in the accent for the one with the keyboard. An
    application with no icon, or whose icon has not come yet, has a tile
    with its name's first letter. `vctl shelf` lists the items.
  - **Icons are found and decoded on a thread of the shelf's own**
    (PNG through `image`, SVG through `resvg`, as the plan's Security
    asks), at the pixels the screen needs (32 points times its scale),
    kept by name and size; a file over 8 MB or a picture over 4096
    pixels a side is not an icon. A file that never finished decoding
    would cost that thread and leave a letter on the strip. The
    applications are read again when a directory of desktop entries
    changes.
  - On Hubble: two terminals share the terminal's icon, the settings
    app has its own with the accent mark when it has the keyboard, and
    the toolkit's demo window, which no entry owns, has a "V".
- **Step 3, second part (2026-11-10): a click on an icon.** An
  application with one window: the window comes forward and has the
  keyboard. With several: a list of their titles opens beside the icon,
  to the left of the strip (the desktop menu's own drawing and input,
  with a new kind of entry), and the one chosen comes forward. Driven
  through `vctl` on Hubble: the settings app's icon, then the
  terminal's with two windows and the second of the list. Two windows
  of one title are two lines that read the same; the screen each is on
  is to be added beside the title where it is not the shelf's.
- **Step 3, third part (2026-11-10): minimize.** The server has a
  minimized state: a minimized window is on no screen and is kept, with
  the place it goes back to, by the shelf, whose items count it with the
  rest. The request is honored from Wayland windows (the terminal's
  own button) and X11 ones, which may also ask to come back. The
  keyboard goes to the window then in front. An application all of
  whose windows are minimized keeps its icon, faint and without a mark.
  From the dock: a click on a minimized application brings its window
  back to its place, in front, with the keyboard; a click on the
  application that has the keyboard, when it has one window, minimizes
  it (decided 2026-11-09); in the list of several windows a minimized
  one comes back. A window whose screen went while it was minimized
  comes back on the screen under the pointer. Driven through `vctl` on
  Hubble: minimized from the dock and from the terminal's button, and
  back, each time to the same place.
- **Step 3, last part (2026-11-10): the name, and the title bars'
  button.** When the pointer has rested half a second on an item of the
  dock, the application's name appears to the left of its icon, in a
  small rounded label drawn like the dock; it goes when the pointer
  leaves the item or presses it. The server's own title bars have a
  minimize button, left of maximize, when the settings' window buttons
  name it (`Gtk/DecorationLayout`, which the server now also offers as
  `org.gnome.desktop.wm.preferences button-layout`; without a setting,
  it is there). On Hubble through `vctl`: "Settings" beside the settings
  app's icon; its title bar's button minimizes it and the dock brings it
  back to its place.
  **Step 3 is done**, but for two things left for the list of several
  windows: a minimized window is not told apart in it, and the screen a
  window is on is not named. Acceptance 3's hand checks (Firefox,
  AlbumWall, icons at two scales) are Dave's to make.
- **The shelf is a dock (Dave, 2026-11-10).** "Like a dock, rounded
  with the same radius as all the other elements": a panel as tall as
  the apps button's place and its items, in the middle of the screen's
  right edge, 6 points in from the edge ("leave a little light showing
  through"), its corners the one radius, drawn on nothing so the desktop
  shows around it. It stands in a column the height of the screen, the
  shelf's width and the room on both its sides (60 points); the work
  area is the screen less that column, so a maximized window still stops
  clear of it. `vergence-shell` has `dock` beside `strip` (the column);
  the server tests a point against the dock as it was last drawn. On
  Hubble: a dock of two items at 1226,288, 48 x 144; a maximized
  terminal 1220 wide; a click on an icon in it still brings its window
  forward.
- **Decided the same evening, not built: the dock moves, and all of it
  is a setting** (Dave: "maybe both?", "let's have it all, adjustable
  in settings"). It can be dragged. Dropped near an edge of any screen
  it settles there, the gap kept, lying along that edge (upright on the
  left and right, level on the top and bottom), and windows keep clear
  of it; dropped elsewhere it floats where it was put, over the windows,
  and reserves nothing. Which of the two it does, the edge or the place,
  the gap, and whether windows keep clear are in `[shelf]` in
  desktop.toml, followed live, and get a page in the settings app when
  that has moved to the toolkit (step 6). This replaces "the edge is the
  right one; it is not a setting in this milestone" under Design, and
  whitepaper §6.9's "on the right edge" becomes its default. It is a
  step of its own, after step 3: the dock's arithmetic for four edges
  and a free place, a level layout, the drag, the settings.

- **Decided 2026-11-10, not built: session restore** (Dave: "I update a
  kernel and need to reboot and it all comes back like I had it";
  whitepaper §6.9). A first form is small, because its parts are here:
  window memory puts windows back, step 2 knows how each application is
  started, step 3 knows which are running. What is new: the list of
  running applications written to `~/.local/state/vergence` whenever it
  changes; each started again from its desktop entry when a session
  begins; a setting to switch it off and a way to start clean; a clean
  start by itself after two sessions that ended soon after a restore.
  What an application had open is its own to restore (Firefox does; Dave:
  "just bringing firefox back may be enough"); the terminal reopening its
  tabs comes later, through the toolkit. It follows step 4, which adds
  starting applications from the shelf.

- **The dock moves (2026-11-10; Dave: snap to any edge of any screen or
  float anywhere, "lets have it all, adjustable in settings").**
  - vergence-shell: `Edge`, `Place` (along an edge, or free at a point of
    its screen, upright or level), `Placement` (place, gap, keep-clear);
    `column`, `dock`, `work_area`, `clear_of_dock` take a placement;
    `settle` says where a dropped dock goes: within 40 points of a
    screen's side it lies along that side (level at the top and bottom),
    else it floats where it was dropped, on the screen its middle is on.
    Slots and `item_at` run down an upright dock and along a level one.
  - Server: a press on the dock does nothing until it is let go. Let go
    where it was pressed it is the click of before; moved more than
    6 points first, the dock follows the pointer and settles on release.
    Its place is written to `[shelf]` in desktop.toml (`place`, `edge`,
    `screen`, `x`, `y`, `upright`; `gap` and `keep-clear` are read; the
    rest of the file kept, written beside and renamed, mode 0600) and
    is back after a restart. A screen named there that is not connected:
    the primary one. Marks, the name label and the window list are on the
    side of the dock facing the middle of its screen. A floating dock
    reserves nothing; at an edge the work area gives up its band and
    maximized windows are refitted.
  - Driven on Hubble with `vctl pointer/press/release`: right, left, top,
    bottom, free in the middle, restart, back to the right. Tests green on
    Hubble, Techbench, kepler, canary.
  - Not yet: a settings page for it (step 6); the dock keeps the way it
    stood (upright or level) when dropped free, with no way to turn it
    but by an edge; nothing shows where it will snap while dragging; a
    hand's drag with a real mouse (Dave).

- **A minimized window goes into its icon (2026-11-10).** For 220 ms
  the window's picture shrinks and fades towards its application's icon
  on the dock, slow at first and quick at the end; coming back it runs
  the other way, and the window is put in its place when the picture
  arrives. One turned round on its way in comes back from where it had
  got to. Not done when the settings ask for reduced motion
  (`enable-animations` false). The picture is the window's own surfaces
  (Smithay's rescale and relocate elements), so a title bar the server
  draws is not in it yet. Seen on Hubble in a frame taken mid-way; the
  feel of it is for Dave's eye.

- **Step 4 (2026-11-10): pinned applications.**
  - vergence-shell: `running::items` takes the pinned ids: those come
    first, in their order, running or not (an id no entry has, or one
    named twice, adds nothing), then the others as before. A test.
  - Server: `pinned` in `[shelf]`, written the way the dock's place is.
    A click on a pinned application that is not running starts it: the
    entry's own arguments, no shell between. Right-click on an icon:
    Open or New window, Pin or Unpin, Close (or "Close all N windows");
    an application with no desktop entry has only Close. A started
    application's mark is in the accent and grows and shrinks, ten steps
    a second, until a window more of it is there or 15 seconds have gone
    (a still mark when reduced motion is asked for). One that is neither
    running nor starting has no mark.
  - On Hubble by `vctl`: pin the terminal, close it, restart the session
    (still there, not running), start it by a click, a second window by
    the menu, unpin, close both; the starting mark seen with an entry
    that opens no window.
  - Not yet: an entry with `Terminal=true` is not started (our terminal
    cannot yet be told what to run); dragging to reorder pins; the
    acceptance check with Firefox is Dave's (his profile, his hand).

- **Session restore (2026-11-10; decided the same day).**
  - The server keeps the list of running applications (desktop entry
    ids; those with a window, an entry, and no need of a terminal) in
    `~/.local/state/vergence/session.toml` (0600), written within a
    second of a change, so it survives a crash as well as a logout. Three
    seconds into the next session each is started from its desktop entry,
    as a click on a pin starts it; window memory places the windows.
  - Not restoring: `[session] restore = false` in desktop.toml;
    `vergence --clean` for one start; and by itself after two sessions in
    a row that ended within a minute of their restore (the rule and its
    test are vergence-shell's `session.rs`), so an application that takes
    the session down as it starts cannot do so for ever. A nested run
    (not `--drm`) neither restores nor records. `vctl session` says what
    the start came to.
  - On Hubble: a terminal comes back after `vctl restart`; a second
    quick restart restores and counts; the third starts clean; a session
    that lasts a minute is forgiven; the switch in desktop.toml is obeyed.
  - Not yet: one start for each application however many windows it had
    (a second terminal window does not come back); what the terminal had
    open (tabs, directories) waits for the toolkit; an application
    started from a shell or with no entry is not restored; `--clean` is
    not tried by hand at a console; the settings page for the switch.

- **Step 5 (2026-11-10): the apps button and popup.**
  - vergence-shell `launcher.rs`: which applications a typed word finds
    and in what order, the grid (cells, hit test, scrolling to show the
    selection), the arrows. Three tests.
  - Server `launcher.rs`: nine dots at the dock's head are the button. The
    popup (536 by 452 points, less on a small screen) opens beside the
    dock on the windows' side, kept on the work area: a search field and
    the grid, icon and name, drawn like the dock. While it is open every
    key is its own (Ctrl+Alt+Backspace and the console keys excepted):
    typing filters, Backspace, arrows, Enter starts the selected one,
    Escape closes; a click on an application starts it, a click anywhere
    else closes; the pointer over a cell selects it; the wheel scrolls.
    Super pressed and let go with nothing else between opens or closes
    it. Starting goes the way a pin's click does (the starting mark).
  - `vctl key <name>`, `vctl type <text>` (the kernel's key numbers of a
    US keyboard, through the same path as the keyboard), `vctl apps`.
    The keyboard's handling moved into `handle_key` for it.
  - On Hubble by `vctl`: Super opens, "term" leaves Terminal, Enter
    starts it, arrows move, Escape closes, the button toggles, a click
    elsewhere closes, a click on AlbumWall's cell starts it.
  - Not yet: a real keyboard and mouse on it (Dave); scrolling seen with
    more applications than fit (Hubble has six); the popup under a
    dock that floats or lies level is placed by the same rule but was
    only tried at the right edge; windows under the popup still get the
    pointer's motion.

- **Touch, the plain version (2026-11-10; Dave: "do the cheap version.
  touch keyboard later if ever": basic touch beside a keyboard and,
  later, a spoken interface).** Server `touch.rs`.
  - A finger on a window is the window's: it comes forward, has the
    keyboard, and gets the touch as Wayland touch (every finger). A first
    finger anywhere else (the dock, a menu, the applications popup, a
    notification, a server title bar, the desktop) is the pointer with
    its left button held, so a tap is a click and a drag a drag.
  - A panel is on one screen, never across all: `[[touch.panel]]` in
    desktop.toml ties a panel's name to a screen's id; without a tie, or
    with its screen away, the primary one. The log names each panel and
    its screen at its first touch. A touch that wakes the screens goes no
    further, as a key does.
  - `vctl touch down|move <x> <y> [finger]`, `touch up [finger]`, and
    `vctl touch` alone for a report. On Hubble with these: a tap on the
    apps button opens the popup, a tap on a cell starts the application,
    a finger on a window focuses it, the dock is dragged to the left edge
    and back, two fingers on a window are both the window's.
  - Not yet: the real panel under a real finger (Hubble's 4K has one,
    plugged in; Dave); the tie made by touching the screen (settings
    app); hit areas forgiving of a finger; a long press as the right
    button; the arrow stays where the finger last was; an on-screen
    keyboard is not planned. (Our own toolkit reads touch from the next
    entry on.)

- **Touch in our own toolkit (2026-11-10).**
  - vergence-wl: `wl_touch` from the seat (down, up, motion, frame,
    cancel), in the fuzz test's list with the rest.
  - vergence-ui: `Input::Touch { id, phase, x, y }`, each finger as it
    is; a finger's press counts as a button's for what needs a serial.
    `touch::Reader` reads the two things a finger most often means out
    of the first finger: a tap (lifted within 10 units of where it came
    down) and a drag (how far since the last told). Three tests.
  - The terminal: a tap is a click where it tapped (tabs, the bar's
    buttons, a pane); a drag up or down scrolls the pane under where the
    finger came down, a line for a line's height, the text going with
    the finger (arrow keys to a full-screen program, as from the wheel).
  - On Hubble with `vctl touch`: 200 lines printed, a finger dragged 280
    points down shows "16 lines back"; a tap on the bar's + opens a tab.
  - Not yet: a real finger; a long press (select, paste, the menu); a
    flick's momentum; pinch for the font's size; dragging the window by
    its bar with a finger; a program that asked for the mouse does not
    get a finger's drag as wheel reports.

- **Touch, the rest that needs no clock in the toolkit (2026-11-10,
  after Dave's finger on Hubble's 4K: "all good").**
  - Server: a finger held still half a second on a dock icon is the right
    button there (the icon's menu), on the bare desktop the desktop's
    menu. One that comes down within 10 points of a title bar's button,
    the dock, a window's edge or a title bar is taken to have meant it.
    A window that asks to be moved from a finger's press goes with that
    finger (`touch_move_request`; a maximized one stays), and its place
    is remembered when the finger lifts.
  - Terminal: a finger's drag that begins on the bar is the pointer's
    with its button held, so the window moves by its bar and a tab along
    the row; in a program that asked for the mouse a finger's drag is the
    wheel, a notch for every three lines.
  - On Hubble with `vctl touch`: the long press on the terminal's icon
    opens its menu, on the desktop the desktop menu; a double tap on the
    bar un-maximizes; the window dragged by its bar goes 100 left and 80
    down with the finger.
  - Not tried: the forgiving reach (no server title bar was on screen);
    the mouse program's wheel; a real finger on any of this.
  - Waiting for the toolkit to have a clock of its own (widgets
    milestone): a long press in our own windows, a flick's momentum;
    pinch waits for the terminal to have a font size to change.

- **Seen through (2026-11-10; Dave: the terminal and maybe the dock now,
  the option for our own programs).**
  - vergence-ui: `opacity_setting(text, "[program]")`: `opacity` in the
    program's own table of desktop.toml, else `window-opacity` under
    `[appearance]` (the desktop's word for every program of ours), else
    1; kept between 0.3 and 1. A test. A window's buffer has alpha when
    its background does (both renderers already multiply alpha in, as
    Wayland wants it); a buffer made opaque is not reused once the
    background is not.
  - The terminal: its ground has that opacity (`[terminal] opacity`),
    read again when desktop.toml changes; text, the cursor, colored
    cells and the bar stay as they are.
  - The dock: `[shelf] opacity`, its panel only.
  - On Hubble: the terminal at 0.8 and the dock at 0.7 over the
    wallpaper, in a screenshot.
  - Not yet: blur of what is behind (the server's to do); the bar and
    server-drawn title bars seen through; a page in the settings app.

- **Step 6, first part (2026-11-10): the Dock page and hiding.**
  - The settings app (now on the project's own toolkit, toolkit milestone
    2) has a **Dock** page: where (an edge; "Where you put it" shown while
    it floats), the screen (when there are several), the gap (0 to 24
    points), keep windows clear, hide automatically, opacity (30 to 100%),
    and the pinned applications with up, down and Unpin, and a choice of
    installed applications to pin. It writes `[shelf]`; the server follows
    within a second. The app reads desktop.toml again when a page opens
    and when its window has the keyboard again, since the server writes
    the file too (a drag, a pin from the dock).
  - **Hiding** (Dave, 2026-11-10), at an edge: `hide = "windows"` (or
    `true`, as Dave meant "autohide": "visible unless a window came over
    it, then it will hide. several desktops do this") keeps the dock out
    until a window overlaps where it stands, then slides it away;
    `hide = "always"` keeps it away until it is wanted; `"never"` (or
    `false`) is the default. Away, it slides past the edge in 160 ms and
    leaves a 3-point sliver, **as LXDE's panel hides**; the pointer on the
    sliver, or a press on it (a finger has no hover), brings it straight
    out, and once the pointer has been away from it for 0.7 s (no menu or
    apps popup open) the mode decides again. Windows have the whole screen
    in both modes. A floating or dragged dock never hides. `vctl shelf`
    says the mode and "(away now)". The settings page offers Never, When a
    window covers it, Always.
  - The gap's default is 1 point (Dave: "the 1px should be the default"),
    at most 24; the apps popup and an icon's menu keep their own 6 points
    off the dock (`vergence_look::SHELF_POPUP_GAP`).
  - On Hubble: "always": hidden with the pointer mid-screen, out at the
    sliver, away again after leaving. "windows": out with nothing over it;
    the settings window moved over its place sent it away, moved back
    brought it out. Work area 1280 x 720 (the whole screen) in both.
  - **Parked (Dave, 2026-11-10: "undecided ... revisit when we have a bunch
    of screens again"):** docks on more than one screen (the `screens`
    list of the original plan) against the one movable dock with its
    screen setting. Unplugging the dock's screen already moves it to the
    primary (`shelf_screen`: the named screen if connected, else the
    primary).

- **Step 6, the rest (2026-11-11): terminal applications.** A desktop
  entry with `Terminal=true` (htop, btop) was refused ("wants a terminal
  to run in: not started"); the terminal now takes `-e PROGRAM ARGS...`
  (as xterm's: the first tab runs it, no shell between, and closes when
  it ends), and the shelf and the apps popup start such entries with
  `vergence-term -e`. On Hubble: Super, "htop", Enter opened htop in a
  terminal; `q` closed the window. Test: a program with its arguments on
  the pty. 333 green on Hubble, kepler and Techbench. Docks on several
  screens stay parked (Dave, 2026-11-10), so step 6 is done.

### Acceptance, where it stands (2026-11-11)

2. **Met on Hubble**: `vshell apps` lists Firefox, htop, Terminal and
   Settings, each with an icon file (canary is dropped; kepler stands for
   FreeBSD).
5. **Met on Hubble** for the popup: Super opens it, typing filters, Enter
   starts the entry (htop, not Firefox, to keep Dave's profile out of it).
1, 3, 6 need Hubble's two monitors (the second is on Techbench now); 4 and
the Techbench parts of 2 and 5 are still to run.

## Acceptance

Every step: `cargo test --workspace` green on Hubble, canary and Techbench,
the server builds with no new warnings.

1. On Hubble's two monitors: the strip is the same width measured with a
   ruler on both when the shelf is moved between them (primary changed in
   Settings). A maximized window stops at the strip. Notification cards
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
   Minimize from the terminal's button, from Firefox's, and from a server
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
6. Hide on: a maximized window uses the whole monitor, the strip returns
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
   minimizes it.
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
