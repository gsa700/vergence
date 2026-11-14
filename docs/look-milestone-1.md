# Look milestone 1: the frosted look, light and dark, rounded corners

Drafted 2026-11-13 from docs/next-look-and-launcher.md (agreed with Dave
2026-11-12, "yes to all", from his macOS screenshots of the Apps launcher
and Mail's translucent toolbar; "an inverse model for the dark lovers";
corners and the bar's proportions added the same evening). **Accepted
2026-11-13: Dave answered (a) to all four questions at the end.** Shelf milestone 2 (the
Apps launcher, docs/shelf-milestone-2.md) uses this milestone's blur, so
this one goes first.

## Where things stand (read 2026-11-13, 02638a2)

- **The server composites with Smithay's GLES renderer** (decor.rs
  `SceneElement`, drm.rs `DrmOutput`), with its damage tracking. Bars, the
  dock, the apps popup and menus are scenes drawn by vergence-render's CPU
  renderer into pictures the GPU then shows. Replacing that renderer with
  vergence-render's own GPU renderer is step 2 of Smithay's replacement
  (docs/borrowed-review.md), not done yet. vergence-render's GPU renderer
  (gpu.rs) is OpenGL ES 3, like Smithay's, so shaders written now move over.
- **A native window's picture is its content only.** The bar is drawn above
  it (`decor.rs` `title` rectangle, `TITLE_H` = `BAR_H` = 36 pt), so there is
  nothing under the bar to frost. Window memory stores the content
  rectangle (vergence-profile `WindowSetting`).
- **The server's bars are dark only.** `vergence_look::BAR_FOCUSED` and
  friends are single colors; light or dark (Settings > Appearance > Style,
  desktop.toml `color-scheme`) reaches the widget palettes
  (`PALETTE_DARK`, `PALETTE_LIGHT`) and the notification cards, not the
  bars, the dock or the menus.
- **Corners**: `vergence_look::RADIUS` = 5 pt is already the one radius
  (buttons, tabs' top corners, menus); window corners are square.
- **Accessibility switches** today: High contrast and Reduce motion
  (desktop.toml `[appearance] contrast`, `reduced-motion`).
- **Machines without a GPU**: the server always uses GLES, on Mesa's
  software driver (llvmpipe) where there is no GPU.

## Design

### The blur

One blur, two sources:

- **A window's own picture**: what lies under a native window's own bar.
  The server already holds the window's picture as a texture; the frost is
  its top strip, blurred and tinted. Cheap: it changes only when the window
  draws.
- **The screen under a rectangle** (the backdrop): what the other windows
  and the wallpaper show beneath a Wayland or X11 window's bar, the
  launcher's panel, and (Question 3) the dock and the server's menus.
  Rendered to a small offscreen texture, then blurred, only when something
  under that rectangle has changed (the damage the server already tracks).

The blur itself: downsample to a quarter, a dual-Kawase pass (two or three
small passes down and up, a few texture reads per pixel), upsample, then the
tint laid over and a faint noise (so a smooth gradient does not band). It is
written as GLES 3 shaders behind a small interface of our own
(`vergence-render`'s new `blur` module), used by the server's compositor
now and by vergence-render's GPU renderer after Smithay's step 2.

**When there is no blur**, the frost is a plain tint, opaque enough to read
on anything: on the software driver (the GL renderer string names llvmpipe
or softpipe), with the new **Reduce transparency** switch, and with **High
contrast** (which implies it, with its stronger edges).

### Content under the bar

A native window may **draw under its bar** (`App::under_bar`, opt-in): its
picture then reaches the top edge of the frame and the server tells it how
tall the bar is (the frame's top inset, in points, per window, since the
bar's height may change with the proportions review). The program keeps
anything that takes input below the inset; what lies under the bar is only
seen, frosted. The terminal opts in: its first line starts below the bar
and lines scrolling off the top slide up under it, as the letter does under
Mail's toolbar. A program that does not opt in (the settings app, for now)
gets the bar frosting nothing, so a plain tint, which looks like today's bar
in the new colors.

**The terminal's rows never start under the bar** (Dave, 2026-11-13: "how
will we make sure the prompt doesn't start under there?"). Its grid is
laid out below the inset: the size it reports to the program (rows for the
pty) counts only the rows below the bar, so the shell, vim and htop cannot
write there. The strip under the bar shows only the scrollback lines just
above row 1, frosted; on the alternate screen (vim, htop, less), which has
no scrollback, it is empty frost. After `clear` the prompt is on row 1
below the bar. Scrolled back, the strip shows the line above the top
visible one. Only the top panes of a split reach under the bar. Text under
the bar cannot be selected (presses there are the bar's). A change of the
inset is a resize: new rows, the program told.

Input over the bar stays the server's. **Window memory and placement keep
the content rectangle below the bar**, so no remembered place moves when a
program starts drawing under its bar; only the picture grows by the inset.

The native protocol gains the inset (bus protocol version 3; length and
range checked like the rest, the fuzz test extended).

### Frosted bars, light and dark

The bar's ground becomes the frost: the blurred picture, then a tint.

- **Light**: a pale frost (a near-white tint, dark ink on it).
- **Dark**: a smoky frost (a near-black tint, light ink on it).
- **Focused or not**: the unfocused bar's tint is more opaque and greyer, so
  the focused window stands out without a color change in the content.
- **Tabs**: the tab in front is shaded on the frost (a lighter veil in dark,
  a darker one in light); the others sit on the frost with their ink only.
- **Every server bar follows Style from now on**, frosted or not: light and
  dark sets of `BAR_*`, `TAB_FRONT_*`, `EDGE_*`, `INK_*` in vergence-look
  (a `BarLook` per style, as the widget palettes are), applied live.
- **Contrast is checked, not guessed**: a test lays each ink over the tint
  above a pure white and a pure black backdrop (the worst cases a blur can
  give) and requires 4.5:1 for titles and 3:1 for the buttons' shapes, in
  both styles, focused and not, and for the red tab below. The tints'
  opacity is set by what passes.

Wayland and X11 windows the server frames get the same bar over the
backdrop (they cannot draw under it). Firefox and other programs that draw
their own bars are unchanged.

### Red for the superuser

A terminal tab turns red while the program in its foreground runs as root
(`sudo -s`, `su`, `sudo vim ...`, and sudo itself while it asks for the
password): the tab's ground red in both styles, and the frost takes a red
tint while that tab is in front. The terminal knows each tab's foreground
process group (`tcgetpgrp` on the tab's pty) and that process's effective
user (Linux /proc/<pid>/status, FreeBSD the kern.proc sysctl), looked at
when the tab's output arrives and once a second, so it turns red at once
and back within a second of leaving root. The bar description's `Tab`
gains an `alert` flag (protocol version 3 again); the color is the
server's, so no program chooses how alarming its own tab looks. A root
shell over ssh on another machine cannot be seen and is not shown.

### Rounded corners

The server rounds the windows it frames at the one radius (5 pt): the
bar's top corners, the content's bottom corners (a mask in the shader that
draws the window's picture), the 1 pt border following the curve.
Maximized and full-screen windows stay square. The resize band keeps its
corner cursors (the band is outside the curve). Programs that draw their
own frame (Firefox) keep theirs.

### The bar's proportions

Before anything above is drawn for real: the bar's height (36 pt), the
window buttons (32 x 30) and the tabs' size and spacing (the 6 pt margin
rule) reviewed together, with mock-ups side by side for Dave (the current
sizes and two or three alternatives, in light and dark, frosted, with the
5 pt corners), like the corner-radius mock-up of 2026-11-10. What he
chooses goes into vergence-look and vergence-bar; the drag model, hit
tests and the window placement steps follow `BAR_H`.

**Decided from the mock-up** (Dave, 2026-11-13; private artifact "Vergence
Bar Proportions"):

- **Tabs stay connected; the window buttons float.** The tabs (and +)
  stand on the bar's bottom edge as now. The buttons' drawn shape is inset
  2 pt at the sides and 3 pt at the top and bottom (28 x 24 at today's
  sizes), centered on the tabs' line so their icons are level with the tab
  titles; the press area stays the full 32 x 30. Dave: "a small style
  difference to set the buttons off some from the tabs". The buttons keep
  their size (Claude's advice, 2026-11-13: little room gained, touch
  worse).
- **Inactive tabs are shaded** in both styles (they disappeared into the
  frost): a light veil in dark, a soft grey in light.
- **The bar and the terminal may differ**: all four (dark bar on a dark or
  light terminal, light bar on either) are looks we support, since the bar
  follows Style and the terminal its own scheme; the contrast test covers
  the mixed pairs (the text under the bar shows through more there).
- **The window buttons' resting shade** as strong as the inactive tabs'
  while the window is focused; none when it is not (icons only), one more
  sign of which window is in front.
- **The superuser's red is deeper** (both styles, the tab and the bar's
  tint), which also gives the white title more contrast.
- **How much shows through the frost is a setting** (Dave, 2026-11-13,
  after the mock-up's first frost, blur 14 px and tint 72%, hid the text
  under the bar until he "looked really hard"; at 6 px and 50% "better, I
  see it now. let's have that a setting as well"). Settings > Appearance,
  beside Reduce transparency: **Frost: Clear, Standard, Heavy** (blur and
  tint in steps; Standard about the mock-up's 6 px and 50%), Reduce
  transparency still giving the solid tint. The tint never goes below what
  the contrast test passes (mixed pairs included), so Clear is as clear as
  stays legible, not a free choice of zero.
- **The tab in front is the page's color** (Dave, 2026-11-13: "front tab
  should be page color, maybe with the alt in settings"): solid in the
  program's own background (the terminal's scheme, as today's bar does),
  so it reads as the top of the page below it and the text scrolls behind
  the frost everywhere but there; in a mixed pair it matches the terminal,
  not the bar. A root tab stays red. Settings > Appearance offers the
  other look, **Front tab: Page color, Frosted** (a veil over the frost).
- **Size of controls** (Dave, 2026-11-13: "yes, per-monitor automatic,
  standard 36"): Compact 32, **Standard 36** (today's), Large 44 (bar
  height; tabs, buttons, margins and gaps with it, as the mock-up's
  presets). Not tied to scale: scale already makes a point the same
  physical size on every monitor. A desktop-wide choice in Settings >
  Appearance, and per screen in Settings > Screens: Automatic (Large on a
  monitor with a touch panel, from the server's panel ties; the desktop's
  choice elsewhere), Compact, Standard or Large. A window takes the size
  of the monitor its center is on (memory's rule); moving it changes only
  the bar's inset. The size is one value in vergence-look so the dock,
  menus and widgets can follow later; text size stays a separate setting.

## Where it stands

- **Step 1 done** (2026-11-13): the proportions and the decisions above,
  from the mock-up; sizes unchanged (Standard 36).
- **Step 2, the bars done** (2026-11-13): vergence-look's `BarLook` in a
  dark and a light style (`BAR_DARK`, `BAR_LIGHT`, `bar_look`), with
  `contrast`, `over` and `ink_on`, and the contrast test (titles 4.5:1
  focused, 3:1 unfocused, on the ground, a tab behind, under the pointer and
  the tab in front; the buttons' marks 3:1; white on the close red; the dark
  style's unfocused ink went from 158 to 172 to pass). vergence-bar draws
  from the style it is given: the window buttons float (drawn inset 2 x 3
  pt, `BUTTON_INSET_X`/`_Y`, pressed in the whole box), shaded at rest only
  while focused, the close mark white on the red; tabs behind and + in
  `tab_rest`. The server's bars and borders follow Style live
  (`Server::light_style`, the bar picture's key and the border's commit
  include it); the tab in front is the program's page color with
  `ink_on` (the terminal now sends its ground as its bar's accent). The
  toolkit's own bar and border (other desktops) follow `desktop_light`:
  Vergence's desktop.toml, or GNOME's color-scheme under GNOME. 381 tests
  on Hubble, kepler and Techbench. Seen on Hubble in both styles, focused
  and not (vctl shots).
- **Step 2 done** (2026-11-13): the dock, the apps popup, the name beside a
  dock icon and the server's menus (desktop, dock icon, tab) follow Style
  too: vergence-look's `ShelfLook` (`SHELF_DARK`, `SHELF_LIGHT`,
  `shelf_look`) and its contrast test (text 4.5:1 on a menu, the row under
  the pointer, the search field and a letter tile; hints 3:1; the dock's
  marks 3:1). menu.rs lost its own colors. The dock's and popup's pictures,
  the name label and the menu's label rasters are keyed by the style; the
  menu's solid parts are redrawn when it changes. 382 tests on all three;
  the desktop menu, dock and apps popup seen on Hubble in both styles.
- **Step 3, the frost in, as a preview** (2026-11-13): vergence-render's
  `blur` module: `FROST_SHADER` (GLSL ES 1.00 for Smithay's texture-shader
  programs: 24 samples on a golden-angle spiral weighted by a bell, then the
  style's tint) and `frost`, the same arithmetic on the CPU as the
  reference. **The tint adapts per pixel**: it starts at the frost's own
  strength (`FROST_STRENGTH`, 0.5) and is raised, by halving ten times, just
  far enough that the bar's ink keeps `frost_need` over what is under it
  (7.0 / 5.1 dark focused / unfocused, 5.85 / 4.05 light: the floor that
  keeps 4.5:1 / 3:1 on the shapes laid on the frost too, checked over a grid
  of colors). Dark content under a dark bar stays at 50%; a white page under
  a dark bar takes about 91%, nearly solid. The server compiles the program
  on first use (`Server::frost_program`; none with High contrast, on a
  software renderer, or if it fails) and, for a native window whose picture
  is on the GPU, draws the bar's ground transparent (`Look::frosted`) over a
  `TextureShaderElement` of the picture's top strip; shared-memory pictures
  keep the solid bar for now. Until step 4 the picture's top is the content
  just below the bar, so the frost is a preview: on only with
  `VERGENCE_FROST=1` or `~/.config/vergence/frost-preview`. Seen on Hubble
  (Mesa, UHD CML GT2) over htop: its meters blurred in the bar, titles
  readable; one 159 ms frame when the shader compiled. 387 tests on all
  three. **Left of step 3**: the GPU test against the CPU reference, the
  frost for shared-memory pictures, and frame-time checks on kepler.
- **Step 4 done** (2026-11-13): content under the bar. Bus protocol
  version 3: `WinRequest::UnderBar` (a window draws under its bar) and
  `WinEvent::Configure`'s `inset` (the strip at the top of its picture under
  the bar, 0 to `MAX_INSET` = 256, checked on decoding; the round-trip lists
  carry both). The server answers UnderBar with a configure carrying the
  bar's height, draws such a window's picture that much higher, and frosts
  exactly that strip; the preview switch is gone: windows that draw under
  their bar are frosted, others keep a solid bar. The window's size, memory
  and placement are its content's, unchanged. vergence-ui: `App::under_bar`,
  `Frame::top` (the scene `height` includes it), pointer and finger
  positions moved down by the inset. The terminal draws under its bar: its
  rows start below the strip (`bar_h` is the inset, so the size it reports
  to its programs counts only those rows), and a pane along the top fills
  the strip with the lines before its top row (`PaneLook::above`; none on
  the alternate screen), clipped to the strip so none shows in the pane's
  own room (a sliver did, first try; fixed). Seen on Hubble in both styles:
  output scrolls up under the bar; the frost is measurably there (light bar
  237-243 against its solid 228-235, dark 66-72 against 56) but subtle: the
  14 pt blur with the contrast floor turns lines into an even haze, so a
  smaller FROST_RADIUS is the lever if Dave wants lines to read as lines.
  387 tests on all three (no new ones beyond the protocol lists).

## Steps

1. **The proportions**: mock-ups of bar height, buttons and tabs, light and
   dark, frosted, with corners; Dave chooses; the sizes go in.
2. **Light and dark bars**: a `BarLook` per style in vergence-look; the
   server's bars, tabs, border, dock and menus follow Style live; the
   contrast test (solid grounds for now).
3. **The blur**: vergence-render's blur shaders and a compositor element in
   the server that frosts a window's own top strip; the plain-tint road
   (software driver, Reduce transparency, High contrast); a GPU test on the
   dev boxes comparing the shader's output with a CPU reference blur.
4. **Content under the bar**: the inset in the protocol (version 3),
   `App::under_bar` in vergence-ui, the terminal drawing and scrolling under
   its bar; memory and placement unchanged; the frosted native bar.
5. **The backdrop**: the screen under a rectangle, rendered and blurred when
   damaged; Wayland and X11 bars frosted over it (and the dock and menus,
   Question 3).
6. **Red for the superuser**: `Tab.alert`, the terminal's root check on
   Linux and FreeBSD, the colors in both styles.
7. **Rounded corners**: the mask, the border on the curve, square when
   maximized.
8. **Size of controls**: Compact, Standard, Large in vergence-look and
   vergence-bar; the per-screen Automatic (touch panel) rule in the
   server; a window's bar follows the monitor its center is on; the
   settings (Appearance and Screens).
9. **Settings**: Frost (Clear, Standard, Heavy), Front tab (Page color, Frosted) and Reduce transparency beside High contrast and Reduce motion;
   docs (whitepaper §6 look, this plan's notes).

## Acceptance

1. Tests green on Hubble, kepler and Techbench, the contrast test and the
   protocol fuzz test with the new fields among them.
2. In a Vergence session on all three: the terminal's output scrolls up
   under its frosted bar; Style changed in Settings switches every bar,
   the dock and the menus between the pale and the smoky frost, live.
3. Reduce transparency and High contrast each give the plain tint at once;
   a session on the software driver gives it without being asked.
4. A tab with `sudo -s` in front is red within a frame of the prompt
   appearing and back within a second of `exit`; another tab of the same
   window stays as it was.
5. Corners: 5 pt on every framed window, square when maximized, resizable
   from each corner by mouse and finger.
6. Cost: the click test still 50 of 50 within one frame at 144 Hz on
   Hubble with six tabs; on kepler (UHD 630, the 4K panel at scale 2) the
   terminal scrolling at full speed under its bar drops no frames.

## Security (whitepaper §7.3)

- The frost shows a window only what was under its own bar: its own
  picture, or for the backdrop, what was already on the screen around it;
  nothing is handed to a program.
- `Tab.alert` is one more flag in an untrusted description: checked on
  decoding, fuzzed; the server decides the color.
- The terminal reads process information for its own children only.

## Not in this milestone

- Frost under other programs' own bars (Firefox): they draw them.
- Animations of the frost (fading in): Reduce motion would turn them off;
  none planned yet.
- Smithay's replacement step 2 (the window space and the renderer); this
  milestone's shaders move into it when it comes.

## Questions for Dave (answered 2026-11-13: (a) to all four)

1. **Where the blur is built.** (a) Now, in today's compositor, with the
   shaders behind our own small interface so they move into
   vergence-render with Smithay's step 2; (b) Smithay's step 2 first, then
   the blur on our own renderer. Proposed: **(a)**: the look comes weeks
   sooner, and the shader code is the same either way; only a thin element
   in the server is redone.
2. **Programs that do not draw under the bar** (the settings app now;
   Wayland programs always). (a) Their bar is the frost's plain tint, in
   the new light or dark colors; (b) the settings app also opts in now (its
   side list running up under the bar, as macOS's do). Proposed: **(a)**,
   the settings app in a later pass.
3. **The dock and the server's menus.** (a) Frosted too, over the backdrop,
   so everything the server draws matches; (b) bars and the launcher only
   this time. Proposed: **(a)**: once the backdrop exists it is the same
   element, and you asked for consistency throughout.
4. **What turns a tab red.** (a) Anything in the tab's foreground running
   as root, briefly too (`sudo make install` turns it red while it runs);
   (b) only root shells (`sudo -s`, `sudo -i`, `su`). Proposed: **(a)**:
   simpler to explain ("what you are typing to is root") and it also
   covers sudo's password prompt.
