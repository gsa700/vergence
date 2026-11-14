# Window styles milestone 1: dialogs, fixed sizes, palettes, the sidebar window

Drafted 2026-11-14 from the window kinds agreed with Dave the same morning
("Window styles you pretty much got, so yes to that"), the settings window's
shape from his screenshot of macOS's System Settings
(docs/reference/macos-system-settings-2026-11-14.png, docs/next-settings.md:
"shows what should be the default layout", the page's translucent bar "a
good place for it"), and wallpaper tinting ("yes, add it to the window
styles plan"). Comes after windows milestone 1 and before the settings
milestone ("finish windows first"; window styles next, then settings).
**Accepted 2026-11-14: Dave answered (a) to all four questions at the
end** ("yes to all").

## Where things stand (read 2026-11-14, 0e34eb3)

- Every window with a bar gets the same one: 36 points, tabs where the
  program has them, the buttons the settings switch on (vergence-bar,
  drawn by the server for native windows).
- **Native windows** tell the server their parent (`WinRequest::Create`,
  `parent`, used to place a window relative to it) and nothing about their
  kind or their size limits.
- **Wayland windows**: the server does not read xdg-shell's parent, minimum
  or maximum size.
- **X11 windows**: the minimum size is used when a window is fitted to a
  screen (place.rs); window types (`_NET_WM_WINDOW_TYPE`) and
  `WM_TRANSIENT_FOR` are not read.
- So a dialog opens anywhere, can be maximized, is listed in the dock and
  Alt+Tab as a window of its own, and a fixed-size window has a resize band
  and a maximize button that do nothing useful.

## Design

### The kinds

| Kind | Bar | Buttons | Behavior |
|---|---|---|---|
| **Normal** | as now, tabs where the program has them | as set | as now |
| **Dialog** (has a parent) | slim (28 points), the title, no tabs | close only | opens centered over its parent, moves with it, comes to the front with it, minimizes with it; not in the dock's window list or Alt+Tab on its own (its parent stands for it). **Modal**: the parent is dimmed and a click on it brings the dialog forward with a short flash of its bar, until the dialog closes (Question 2) |
| **Fixed size** (smallest size = largest) | as Normal | no maximize | no resize band, no snapping to a part it cannot fit |
| **Palette** (tool panels, pickers) | thin (22 points), small close | close only | floats above its own application's windows, never above others'; hides while another application has the keyboard, back when it returns (Question 3); not in the dock's list or Alt+Tab |
| **Sidebar** (the settings app first) | the frosted bar over the whole top, no line between it and the content | as set, no maximize where it has a largest width | see below |

Menus, tooltips and splash screens stay without a bar, as now.

### Size limits from the program

A window may say its smallest and largest size (native: on Create and later;
Wayland: xdg-shell's min and max size; X11: `WM_NORMAL_HINTS`). The server
keeps every resize within them; a window whose largest size is smaller than
its screen's work area gets **no maximize button** (and no Maximize in its
layouts menu); snapping and the window keys put such a window in its part at
its largest, aligned to that side, never stretched past it; double-clicking
its bar does nothing.

### The sidebar window (the settings app)

From Dave's screenshot of macOS's System Settings:

- **Shape**: portrait. The settings app asks for about 840 points wide and as
  tall as the screen allows up to about 1100 (window memory decides after the
  first time), smallest 760 x 480, largest width 1100, height free.
- **No line across the top**: the program's content runs up under the
  server's frosted bar (look milestone 1, content under the bar). Over the
  sidebar the frost blends into it, the window buttons on it; over the page
  it frosts the page scrolling up under it, as the terminal's output does.
- **The page's title and back and forward in the bar**, over the page, not
  over the sidebar: the bar description (bus protocol) gains the program's
  own **bar buttons** (an icon, a name for Contract 3, enabled or not; the
  server draws them, the program hears the press, as with tabs) and **where
  the title starts** (the sidebar's width).
- The sidebar's own content (search, the grouped pages, later the account
  card and page icons) is the settings milestone's.

### Wallpaper tint (optional, last)

As macOS's "Tint window background with wallpaper color": the server takes
the average color of the wallpaper under each native window (not the windows
under it; a few sample points of the wallpaper's own picture at the window's
place on its screen), and sends it as a **ground tint** in the window's
configure when it changes enough to see (throttled, not every frame of a
move). The toolkit mixes a few percent of it into the window's ground and
its sidebar, keeping the contrast floors of look milestone 1. It shows in
dark style, where a colored wallpaper turns dark gray grounds slightly blue
or purple; in light style it is nearly invisible (Dave saw no difference on
his Mac in light mode). A switch in Settings > Appearance beside Frost
(Question 4); off with Reduce transparency and High contrast.

### How the server knows a window's kind

- **Native windows** say it: `Create` gains `kind` (normal, dialog,
  palette, sidebar), `modal`, and the size limits; a `Limits` request changes
  them later (bus protocol v4).
- **Wayland windows**: a parent makes a dialog; equal min and max a fixed
  size; modal from the xdg-dialog protocol (Question 1, (a)): Smithay
  65aa86f has it (`wayland::shell::xdg::dialog`, in the `wayland_frontend`
  feature we build with, no new dependency); we register `XdgDialogState`
  at the Wayland edge and record `dialog_hint_changed` on the window; what
  modal does is the server's, the same for every road. Listed in
  docs/borrowed-review.md with the rest of Smithay's front end.
- **X11 windows**: `_NET_WM_WINDOW_TYPE` (DIALOG, UTILITY as palette,
  SPLASH, NORMAL), `WM_TRANSIENT_FOR` (the parent), `_NET_WM_STATE_MODAL`,
  `WM_NORMAL_HINTS` (the limits).

## Settings

Nothing new for the kinds (they are the program's to say). Settings >
Appearance gains **Tint windows with the wallpaper's color** (step 6).
The settings app is the first sidebar window.

## Steps

1. **Kinds and limits on the wire**: bus protocol v4 (`Create`'s kind, modal
   and limits; `Limits`); the server reads Wayland's parent and limits and
   X11's type, transient-for, modal and hints; one `Kind` per window in the
   server; `vctl windows` shows it; tests of the wire and the X11 mapping.
2. **Size limits kept**: resizing within them; no maximize button, no
   Maximize in the layouts menu, snapping and window keys at the largest
   size; fixed size without a resize band; the toolkit sends the settings
   app's and the terminal's limits.
3. **Dialogs**: the slim bar, close only; placed over the parent; moving,
   raising and minimizing with it; out of the dock's list and Alt+Tab;
   modal dimming and the click that brings the dialog forward.
4. **Palettes**: the thin bar; above their application's windows; hidden
   while another application has the keyboard.
5. **The sidebar window**: bar buttons and the title's start in the bar
   description; the settings app as a sidebar window at its shape, back and
   forward through the pages it has shown, its page title in the bar.
6. **Wallpaper tint**: the sampling, the configure's ground tint, the
   toolkit's mix, the Appearance switch.
7. **Docs**: whitepaper §6.4 and §6.9 (dialogs, kinds), §6.10; this plan's
   notes.

## Acceptance

1. Tests green on Hubble, kepler and Techbench.
2. A dialog from a native test program, from GTK 4 (a file chooser in a
   Wayland program) and, with X11 on, from an X11 program: slim bar, close
   only, centered on its parent, moves with it, not in Alt+Tab; a modal one
   dims its parent, and a click on the parent brings it forward.
3. A fixed-size window (a native test window, and an X11 one by its hints):
   no maximize button, no resize band, double-click on its bar does nothing.
4. The settings app opens portrait at its shape, cannot be made wider than
   1100 or maximized; snapped right on a wide screen it sits at the right at
   1100 wide; its page title and back and forward are in the frosted bar over
   the page, the sidebar runs up under the frost; back and forward walk the
   pages.
5. A palette (native test window) stays above its application's windows,
   below another application's, and hides while another application has
   the keyboard.
6. With the tint on in dark style, a window dragged from a blue part of the
   wallpaper to a red one changes its ground's cast within a second; the
   contrast tests pass.

## Security (whitepaper §7.3)

- A program can make only its own windows dialogs or palettes, and only of
  its own windows (a parent of another program is refused), so it cannot
  dim, cover or ride along with another program's window.
- A modal dialog blocks only its own parent; the server's keys (Alt+Tab,
  the window keys) and other windows stay usable, so a program cannot take
  the desktop hostage with a modal window.
- Palettes stay above their own application's windows only, never over
  another program's or the server's own surfaces.
- The size limits are clamped to sane values (a largest size of a few
  points cannot hide a window: the server keeps its bar usable).
- The ground tint tells a window the color of the wallpaper behind it,
  which the user chose; nothing of other windows.

## Not in this milestone

- The settings app's sidebar content, page icons and account card (the
  settings milestone).
- Applications' own preferences windows (whitepaper §6.10; after this).
- Tabbed dialogs, sheets sliding from the bar (macOS), and per-kind
  animations.

## Questions for Dave (answered 2026-11-14: (a) to all four)

1. **Wayland's modal hint.** GTK and Qt mark a modal dialog through the
   xdg-dialog protocol. (a) Speak it at the Wayland edge (a protocol
   extension for other people's programs, so the rule says ask); (b) treat
   every Wayland dialog as not modal. Proposed: **(a)**: it is small, and
   without it a GTK file chooser leaves its parent clickable.
2. **A modal dialog's parent.** (a) Dimmed slightly, a click on it brings the
   dialog forward with a short flash of its bar; (b) not dimmed, the click
   only brings the dialog forward. Proposed: **(a)**: it shows at a glance
   why the window does not answer.
3. **Palettes when their application is not in front.** (a) Hidden, back
   when it returns (macOS); (b) always shown above their application's
   windows. Proposed: **(a)**: palettes clutter the screen otherwise.
4. **Wallpaper tint by default.** (a) On in dark style, off in light (where
   it does not show); (b) off until switched on. Proposed: **(a)**.
