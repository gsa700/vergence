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

## Progress

- **Step 1 in** (2026-11-14): bus protocol v4: `WinRequest::Style` (kind:
  normal, dialog, palette, sidebar; modal, a dialog's only; smallest and
  largest size, 0 none; refused when not finite, negative, or a largest
  below the smallest), sent by the toolkit after Create when an app's
  `App::style` says more than a normal window without limits. The server
  (style.rs) has one answer for every road, `window_style`: native windows
  by what they said (a dialog or palette whose parent is not the same
  connection's window becomes normal, §7.3), Wayland windows by their parent,
  xdg-shell's limits and the xdg-dialog hint (Smithay's `XdgDialogState`,
  registered; the handler redraws), X11 windows by type, transient-for,
  modal and size hints (`x11_kind`); limits kept sane (none above 16384, no
  largest below 64 or below the smallest). `vctl windows` gives each
  window's kind, modal, limits and parent. Tests: the wire (both
  requests), refusals and modal-only-for-dialogs, the X11 kinds, the
  limits; the fuzz test seeds Style. 439 on Hubble, kepler and Techbench.
  Live on Hubble: the terminal and settings app normal, Firefox normal with
  its 500 x 120 smallest size, its Open File dialog `kind=Dialog` with
  parent "Mozilla Firefox" and its 590 x 374 smallest (not modal: Firefox's
  GTK 3 has no xdg-dialog).
- **Step 2 in** (2026-11-14): style.rs `maximizable` (no largest size, or
  one that holds its screen's work area; never a fixed size), `resizable`
  (not fixed) and `within_limits`; the bar leaves out Maximize where it does
  not apply, and the layouts menu its row; toggle-maximize (the button, a
  double-click on the bar, Super+Up, the menu, the top edge) leaves such a
  window as it is; a fixed-size window has no resize band; resizing keeps
  every road's limits (the old Wayland/X11-only check in dispatch.rs gave
  way to `within_limits`); snapping puts a window at its largest size
  against its part's side (right parts right, bottom parts bottom) and at
  least its smallest. The toolkit's `Program::style` passes through
  vergence-widgets; the settings app says 760 x 480 to 1100 wide, the
  terminal 320 x 160 smallest. Live on Hubble: the settings bar has close
  and minimize only (shot), its menu no Maximize row (shot), a double-click
  on its bar and Super+Up leave it as it is, its right edge dragged 195
  points further stops at 1100; on a nested screen 1889 wide with the top
  edge set to the top half, it snapped to the top at 1100 x 480 against the
  left (the part 1833 x 473). 439 tests on Hubble, kepler and Techbench.
- **Step 3 in** (2026-11-14): dialogs.rs. A dialog's bar is 28 of the
  standard 36 points (a palette's 22), the screen's proportions scaled,
  close only on the controls' side, no tabs; a dialog opens over the middle
  of its parent, kept on the parent's screen; each frame it goes where its
  parent went (`dialogs_follow`; a dialog the user moved keeps its new offset);
  focusing the parent raises its dialogs and palettes; minimizing it takes
  them, bringing it back brings them; the dock's list, and so Alt+Tab, leave
  out a window that belongs to a parent. Modal: the parent gets a shade in
  front of its own bar and content (18% in light style, 35% in dark), behind
  the dialog; every part of it is the seat's `Part::Blocked(dialog)`, so a
  press focuses the dialog and flashes its border in the accent for 0.3 s,
  and nothing reaches the parent; focusing the parent hands the keyboard to
  the dialog. vergence-widgets gained `window(program)` (a program as a window
  another opens) and `Program::style`; vergence-ui re-exports `WinKind`; the
  example `vdialog` opens a dialog and a modal one. Live on Hubble with it:
  the dialog `kind=Dialog` with its parent, centered over it (both centers
  960,558), the slim bar with close only (shot); the dock lists one window;
  the parent moved by vctl, the dialog by the same amount; focusing the
  parent raised the dialog; minimizing the parent minimized both, Alt+Tab
  listed only the parent and brought both back; the modal one dimmed its
  parent (shot), a click on the parent left the keyboard with the dialog and
  flashed its border (shot: the line in the accent; a one-point line, so
  subtle), a click on the parent's Open button opened nothing; once it
  closed, the button worked again. 440 tests on Hubble, kepler and
  Techbench.
  The flash made bolder the same morning (Dave: "make the flash bolder if
  you think we should"): a 3-point outline in the accent just outside the
  border for its 0.3 s, as well as the border line (seen on a nested server).
- **Step 4 in** (2026-11-14): palettes (dialogs.rs `palettes_follow_focus`,
  run whenever the keyboard moves): another application's palettes are
  taken off the screen with their place kept, this application's come back
  there; shown palettes are raised over every window, and the application's
  dialogs over them again (a dialog is what is being answered: found when a
  palette covered a modal dialog). The thin bar (22 of 36 points, close only)
  came with step 3. vdialog opens a palette too. On a nested server on
  Hubble: the palette opened on top; foot given the keyboard, the palette
  left the screen; the window given it back, the palette returned at its
  place, on top; a modal dialog opened over the palette, the parent dimmed,
  a press on it flashed the bolder outline (shot). 440 tests on Hubble,
  kepler and Techbench.
- **Step 5 in** (2026-11-14): bus protocol v5: `Bar::Tools` (the program's
  own buttons, each an id, an icon (back, forward), a name for Contract 3 and
  whether it can be pressed, up to 8; and where the title starts) and the
  event `BarButton`. vergence-bar's `Layout::tools_sized` (the buttons from
  `from` or after the window's, then the title), `Part::Tool`, chevrons,
  faint when they cannot be pressed. The server draws them (decor.rs
  `tools_of`), the seat's `Part::Tool` takes their presses, and the program
  hears `BarButton` for one that can be pressed. The toolkit: `App::bar_button`;
  vergence-widgets `Program::under_bar`, `bar`, `bar_button` (the bar sent
  when it changes, a frame drawn after a press) and `Theme::top`. **Found**:
  the toolkit sent a bar description and the request to draw under the bar
  only for apps that draw a frame of their own elsewhere (the terminal), so
  the settings app was heard on neither; on the native road both now go for
  any app that asks. The settings app: `WinKind::Sidebar`, 760 x 480 to 1100
  wide, asks 840 x 1100 (the server fits the height to the screen), draws
  under the bar (its sidebar's color up behind the window buttons, its page
  scrolling up under the frost), back and forward after the sidebar (200
  points) and the page's name as the title (`set_title` on every page
  change), history through the pages it has shown. Live on Hubble: the
  layout (shot: sidebar to the top, chevrons after it, "Appearance"), then
  Windows, Dock, back, back, forward, forward: the title Windows, Dock,
  Windows, Appearance, Windows, Dock, the chevrons enabled and faint to
  match. 440 tests on Hubble, kepler and Techbench.

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
