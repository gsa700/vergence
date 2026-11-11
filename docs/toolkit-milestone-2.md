# Toolkit milestone 2: widgets, and the settings app on them (whitepaper §6.10)

Drafted 2026-11-10 (Dave: "lets go on Vergence"; the agreed order after
shelf step 5 is "widgets + the settings app's move, then shelf step 6 in
the new settings app"). **Accepted 2026-11-10 20:00** (Dave: "yes to all
three, go": the three open questions answered as proposed; see Decided).

The goal: the settings app runs on our own toolkit, every page doing what it
does today, and iced leaves the workspace. When this milestone closes, shelf
step 6 writes its two settings pages in the new app.

## What the settings app asks of a toolkit (read 2026-11-10, dc44551)

`crates/vergence-settings`, about 2,450 lines, nine pages (Appearance,
Theme and fonts, Windows, Keyboard, Energy, the screens overview, one page a
screen, the picture browser, About). Its iced use, counted:

| iced piece | uses | what it is for |
|---|---|---|
| `text` | 33 | labels, notes, values beside sliders |
| `container` | 18 | cards, the sidebar, padding, fills and edges |
| `button` | 14 | sidebar items, swatches, Left/Right, Make primary, Add, Remove, Up, Cancel, wallpaper tiles |
| `pick_list` | 8 | style, themes, icons, pointer, fonts and sizes, terminal colors, energy times, fit |
| `toggler` | 6 | contrast, motion, key repeat, minimize/maximize, same-as-primary |
| `text_input` | 4 | custom accent, key-repeat test, sleep command, background color |
| `slider` | 4 | key delay and rate, scale (with marks), size |
| `scrollable` | 2 | the page, the picture browser's list |
| `image` | 1 | wallpaper thumbnails (decoded on a thread already, `thumbs.rs`) |
| `Canvas` | 2 | the screens' arrangement: drawn, dragged, clicked to open a screen |
| `Subscription` | 1 | the server's screens over the bus (`bus_outputs`) |
| `Task` | 2 | work done off the interface thread, its answer as a message |

Everything else in it (desktop.toml, xsettings, the bus, wallpapers,
thumbnails, the font and theme lists) has no iced in it and moves over as
it is.

## What the toolkit has now (dc44551)

`vergence-ui` gives a window: [`App::scene`] returns a scene at a size,
[`App::input`] gets input in order at the place it happened, a `Waker` lets
other threads ask for attention, the clipboard is text, touch is in. There
are no widgets: the terminal lays out its own bar, panes and menu by hand,
and the bar is shared as `vergence_ui::bar`. There are **no timers**: the
terminal's long press and flick momentum are on the open list for that
reason.

## Design

**Shape: a tree built from the program's state, kept between frames by
id.** The program writes a `view` that returns a tree of widgets from its
state, as the settings app's iced code does today, so its pages port nearly
line for line. The toolkit lays the tree out, draws it into the scene, and
judges input against the bounds it laid out. What a widget must remember
between builds (hovered, pressed, focused, a text field's caret and
selection, a list's scroll offset, an open menu) is kept by the toolkit
under the widget's **id**: its place in the tree, or a key the program gives
where a list changes. Input on a widget returns the program's **message**;
the program updates its state and the tree is built again. A tree is built
only when something changed, never once a frame.

Why not draw-as-you-go (immediate mode): input must be judged at the place
and in the order it happened against what was on the screen then, and the
accessibility tree (below) needs a stable shape to publish. Why not a
widget object graph the program mutates: it is the most code, and the
settings app is already written the other way.

**Layout.** Row, column and stack; each child's width and height is fixed,
shrink to its content, or fill (with a weight); padding, spacing, alignment
on both axes; a maximum width (the settings pages are kept readable on a
wide window). Text is measured by `vergence-text`, so a label's size is
exact at every scale. All in reference units (Contract 1); the toolkit
never sees a scale except to draw.

**Widgets** (the whitepaper's list, and what the settings app needs):

- **Label**: text in a size, weight and color; wraps to its width;
  aligned.
- **Panel**: a box with a fill, an edge and the one radius
  (`vergence_look::RADIUS`, 5), around one child. Cards, the sidebar.
- **Button**: any child; rest, hover, pressed, focused and off looks;
  activates on release inside, on Space or Enter when focused, on a tap.
- **Switch**: on or off; click, tap, Space; a short slide unless motion is
  reduced.
- **Slider**: a range and a step, optional marks (the scale slider's);
  drag, click to jump, wheel, the arrow keys and Home/End; a message
  while it moves and another when it is let go (the key-repeat sliders
  write desktop.toml only on release today).
- **Text field**: one line; caret, selection by mouse, Shift+arrows and
  double-click; Home/End; copy, cut and paste with the Super and Ctrl
  forms the terminal uses; a placeholder; Enter submits; a limit on
  length. Plain text, left to right, as the text core is (whitepaper §10
  names the rest). A password form (contents never leave the program,
  Contract 3) is in the design and not used yet.
- **Choice** (iced's pick_list): a button showing the chosen item that
  opens a list of the others. The list is drawn **inside the window**, in
  front of everything, kept within the window's edges, scrolled when long.
  A popup surface of its own (xdg_popup), which could reach outside the
  window, waits for a need for it.
- **Scroll**: one child taller than its place; wheel and touchpad, a
  finger drag with momentum, a scrollbar to drag (the terminal's look),
  Page Up/Down; a focused widget is scrolled into view.
- **Picture**: an image from pixels the program already decoded
  (decoding stays off the interface thread: whitepaper §6.10, isolation).
- **Drawing** (iced's Canvas): the program draws into a part of the
  scene itself and gets that part's input, in its own coordinates. The
  screens' arrangement is one.
- **Space** and **Rule** (a line): layout only.

**Focus and the keyboard.** Every widget that takes input takes focus;
Tab and Shift+Tab move it in reading order; a focused widget shows a ring
in the accent color. Every page of the settings app must be usable with
the keyboard alone. Escape closes an open list.

**Meaning (Contract 3).** As the tree is laid out, each widget records its
role, name, value, state and bounds. Nothing reads that record yet but a
dump (`VERGENCE_UI_TREE=1` prints it, and a test reads it); it is the tree
the accessibility bridge, named actions and voice will address later, and
building it now is what keeps it from being bolted on.

**Timers.** `Window::wake_after(duration)` and `wake_at(instant)`: the
window is woken at that time on its own thread and the program is told
which timer it was. The caret's blink, a switch's slide, scroll momentum
and the terminal's long press all use it; with it, the terminal's two open
items (long press in our own windows, flick momentum) can be closed.

**Off the interface thread.** iced's `Task` and `Subscription` become a
thread and the `Waker`: work runs on a thread, puts its answer in a channel
and wakes the window; the window reads the channel in `woken`. The bus
watcher for screens is one such thread for the program's life.

**The look.** Colors, the radius and sizes come from `vergence-look`, with
the light and dark sets, high contrast and the accent from desktop.toml
`[appearance]` read live, as the terminal reads them today. The settings
app's own colors (its sidebar, page and card grays) move into
`vergence-look` so another tool of ours can share them. `[settings]
opacity` follows the rule the terminal and the dock follow
(`opacity_setting`).

**Where the code lives.** A new crate, `vergence-widgets`, on top of
`vergence-ui`: the window crate stays a window, and a program that draws
its own picture (the terminal) does not carry widgets it does not use.
The terminal's bar stays in `vergence_ui::bar` for now.

## Steps

1. **Timers, layout and the first widgets.** `wake_after`/`wake_at` in
   `vergence-ui`. `vergence-widgets`: the tree, ids and kept state, layout,
   drawing into a scene, input and hit-testing, hover and press, focus and
   Tab, the meaning record. Label, Panel, Button, Space, Rule. A demo
   program (`vwidgets`) shows them. Tests: layout arithmetic, hit-testing,
   focus order, the meaning record, golden images through the CPU renderer.
2. **The rest of the widgets.** Switch, Slider, Text field, Choice,
   Scroll, Picture, Drawing, each in the demo and each tested with the
   mouse, the keyboard and a finger (synthetic input in tests; by hand on
   Hubble's 4K). A fuzz test feeds random input to a page of every widget.
   The terminal's long press and flick momentum are closed with the
   timers.
3. **The settings app moves.** Page by page in `crates/vergence-settings`,
   the iced code replaced, the rest kept: sidebar and Appearance first,
   then Theme and fonts, Windows, Keyboard, Energy, About, then the
   screens (the arrangement as a Drawing, the per-screen page, the picture
   browser). Before and after screenshots of every page, light and dark,
   on Hubble.
4. **iced leaves.** The iced dependency and every line that named it are
   removed; `Cargo.lock` loses it; the About page says what it is built
   with; whitepaper §8 and CLAUDE.md say iced is gone. Installed on Hubble
   and kepler; whole workspace green on the four machines.

## Acceptance

1. Every setting on every page does what it does in the iced app today,
   checked against a list written from the iced code before it is removed
   (as the terminal's parity check was).
2. Every page usable with the keyboard alone, and with a finger alone on
   Hubble's 4K.
3. No click lost under load: the settings app open beside the six-tab
   terminal stress test, every press acted on.
4. Idle costs nothing: no frames drawn and no CPU used while nothing
   changes (iced's settings app redrew on every bus message).
5. Scrolling the longest page at Hubble's 4K draws each frame in under
   8 ms on the GPU, and under 16 ms on canary's CPU renderer.
6. The meaning dump lists every control on every page with its role,
   name and value.
7. iced is not in `Cargo.lock`. Build time and the settings binary's size,
   before and after, recorded here.

## Progress

- **Step 1 (2026-11-10): timers, layout and the first widgets.**
  - `vergence-ui`: `Window::wake_at(id, instant)`, `wake_after(id,
    duration)`, `cancel_timer(id)`, and `App::timer(id)`. The window's
    wait is cut short by the next timer as by a key repeat; asking again
    for an id moves it; at most 256 are kept (the latest-due go).
  - `vergence-look`: `Palette`, `PALETTE_DARK` and `PALETTE_LIGHT`, the
    settings app's grays (sidebar, page, card, line, dim, text, hover).
  - New crate `vergence-widgets`:
    - `Element`: label (size, weight, color, dim, wrapping, alignment),
      panel (ground, edge, radius, padding, alignment of its child),
      button (any child; Normal, Accent, Plain, Chosen and the program's
      own looks; off without a message), row, column, stack, space,
      rule. Widths and heights fixed, shrinking or filling by weight; a
      maximum width; spacing; a key; a name for the meaning.
    - Layout: a flat list in drawing order, each with its id (place or
      key under its parent's), text measured through a `Measure` (the
      window's fonts; a stand-in in tests).
    - `Ui`: hover, press with the pointer (judged at the press's and the
      release's own places), a finger (a cancel drops it), Tab and
      Shift+Tab through the controls in reading order, Enter and Space
      press the focused one, a focus ring in the accent only after the
      keyboard moved the focus. Ids that leave the tree take their
      focus, hover and press with them.
    - `Meaning`: role, name, value, states and bounds of every label,
      button and rule (a button's name is its text unless given one);
      `VERGENCE_UI_TREE=1` prints it at every layout.
    - `Theme`: the palette with `[appearance]` dark or light, accent,
      high contrast and reduced motion, and the window's opacity
      (`opacity_setting`), from desktop.toml's text.
    - `Program` and `run`: view, update, the input no widget used,
      waker, woken and timer (ids below 2^63; the rest are the
      toolkit's). The tree is built again only after a message, or when
      `woken`, `timer` or `input` say the state changed, or the size changed.
    - Demo: `cargo run --release --example vwidgets`.
  - Tests: 13 in `vergence-widgets` (layout arithmetic, wrapping, ids,
    clicks judged where they happened, a tap, Tab order, keys keeping
    what is remembered, the meaning of a page, the CPU renderer's
    pixels with and without the ring) and one of the timers' bookkeeping.
  - Whole workspace green: Hubble 300, Techbench 300, kepler 301,
    canary 301.
  - On Hubble (the 4K, scale 3): the demo drawn as designed; through
    `vctl`, three clicks on + and one Return on − gave 2, the look button
    turned the window light, Tab showed the ring and a click took it away.

- **Step 2 (2026-11-10): the rest of the widgets, and the terminal's
  touch.**
  - Switch (click, tap, Space or Enter; slides in 120 ms unless motion is
    reduced; one that cannot be changed is drawn faded); Slider (a press
    jumps the thumb there and drags it, a finger the same, the arrow
    keys, Page Up/Down by ten steps, Home and End; a step, marks under
    the track, a message as it moves and another when it is let go). The
    wheel does not move a slider: on a page that scrolls it would take
    the page's wheel.
  - Field: the caret, a selection by dragging, Shift and the arrows,
    double-click for a word and triple for all; Ctrl or Super with the
    arrows by words, A, C, X, V; Backspace and Delete (by words with
    Ctrl); Enter sends `on_submit`; pasted text keeps its first line and
    loses control characters; 4096 bytes at most, cut on a character's
    edge. **A key typed before the tree is built again is not lost**: the
    field keeps its own text while the program's value is what it last
    sent. Tab into a field chooses all its text. The caret blinks on the
    toolkit's own timer, steady when motion is reduced. A secret field
    shows dots, is never copied, and its meaning has no value.
  - Choice: a click, a tap, Space, Enter or Down opens its list in front
    of everything, inside the window (below, or above where there is
    more room), scrolled when long; the arrows, Page Up/Down, Home, End,
    Enter, Space and a click choose; Escape, Tab or a press elsewhere
    close it, and that press does nothing else.
  - Scroll: the wheel, a finger (a press becomes a scroll after it moves
    8 points, and is then no press; a flick goes on, slowing, frame by
    frame), the scrollbar's thumb dragged or its track pressed, Page
    Up/Down, Home and End; Tab scrolls the focused control into view;
    what is out of view is not drawn, and input is judged against what
    is shown. Its place is kept when the tree is built again.
  - Picture (contain, cover or stretch; rounded; opacity) from pixels the
    program decoded; Drawing (the program draws into its place and hears
    the pointer and fingers in its own coordinates, held while pressed).
  - Meanings: switch (on or off), slider (its value), text field (its
    text, never a secret's), choice (the chosen item; open), scroll
    area, picture, drawing; "off" for what cannot be used.
  - `vergence_ui::touch`: a long press (held still 500 ms, asked for
    after a timer) and a drag's end with the finger's speed. **The
    terminal**: a long press on the text is the right button there (the
    menu); a flick of the text goes on, slowing, on a 16 ms timer, until
    another finger comes down. Both open items from toolkit milestone 1.
  - Tests: 11 more in `vergence-widgets` (24 in all) with a fuzz test of
    every kind of input on a page of every widget (20,000 rounds by
    default; 400,000 clean on Hubble with `FUZZ_ITERS`), and 2 more of
    the touch reader.
  - Whole workspace green: Hubble 313, Techbench 313, kepler 314,
    canary 314.
  - The demo has every widget. On Hubble through `vctl`: Tab down the
    page scrolled to each control, the slider moved by the keys, both
    fields typed in, the choice opened above itself (no room below) and
    chose, the scrollbar dragged, the picture covered its place, the
    drawing's dot dragged. Not yet tried by hand: a finger on the
    widgets, the terminal's long press and flick.

- **Step 3 (2026-11-10): the settings app moves.** `crates/vergence-settings`
  runs on `vergence-widgets`; iced is gone from its dependencies (and so
  from the build: Cargo warns that the iced patch is unused, which step 4
  clears up). Everything that reads and writes the settings (`disp`,
  `theme`, `wall`, `winbar`, the save functions, the messages' handling)
  is as it was; the pages are built of our widgets in the same shapes.
  - iced's subscription and tasks became threads and a channel: the bus
    watcher for screens and each preview drawer send their message and
    wake the window (`Program::woken` reads the channel).
  - The screens' arrangement and the title bar preview are drawings
    (now given the fonts, to draw names and tabs); the overview's drag,
    double-click and the screen page's map are their touches, in the
    app's own state. The wallpaper grid is a wrapping row (new), its
    pictures made at the tile's exact pixel size for the window's scale
    (`Program::scale`, new, told when the scale changes).
  - Toolkit changes for it: `row(..).wrap()`; a drawing's `draw` gets
    `&mut Fonts`; `Program::scale`; a button that cannot be pressed is
    faded; a chosen button's meaning says "selected".
  - The look: `vergence-look`'s palette is the old app's grays, so the
    pages match. Before and after screenshots of every page on Hubble's
    4K: the same layout, wording, wrapping and controls; the sidebar's
    lines are a point closer.
  - Checked on Hubble with a throwaway HOME holding copies of Dave's two
    files (his were not touched): an accent swatch, high contrast, Style
    Dark (choice by keys), the title bar's side and Maximize, the key
    repeat delay by dragging, screens-off by choice, a wallpaper tile,
    the scrollbar to the page's end, the picture browser (two pictures
    picked, Cancel). desktop.toml and settings.toml came out with exactly
    those changes. The meaning dump names every control.
  - Whole workspace green: Hubble 314, Techbench 314, kepler 315, canary 315.
  - Not seen: the overview and the screen page's map (Hubble has one
    screen now); the browser's Add with a real folder.

- **Step 4 (2026-11-10): iced leaves.** No crate depends on it since step 3;
  now its patch (`[patch.crates-io] iced_tiny_skia`), the vendored
  `third_party/iced_tiny_skia` and `patches/iced-tiny-skia-clip-mask.patch`
  are gone too, and `Cargo.lock` has no iced, wgpu or cosmic-text (the
  `tiny-skia` left is resvg's). Whitepaper §8 (the borrowed-parts table,
  the language list, the glossary), README, CLAUDE.md and
  patches/README.md say so; PDF rebuilt (32 pages, looked at). base64,
  which came in with iced, is now the terminal's alone (noted in its
  Cargo.toml).
  - Acceptance 7: the settings binary is 6.2 MB (Hubble, release; 6.3 MB
    on kepler), was 35 MB on iced; `Cargo.lock` lost about 3,200 lines.
    Build time was not measured like for like (the iced tree was gone
    from Hubble's build by then).
  - Installed on Hubble and kepler (release, `sudo tools/install.sh`);
    no server change, so no restart.

- **Acceptance checks (2026-11-10, evening).**
  1. *Parity.* The iced app's controls, read from its code before it was
     removed (ab7e126^): Appearance (style choice, eight swatches and
     none, custom accent field with Enter, high contrast, reduce motion);
     Theme and fonts (GTK theme, icons, pointer, interface and monospace
     family and size, terminal colors); Windows (preview, left/right,
     minimize, maximize, close shown on); Keyboard (repeat switch, delay
     and speed sliders saved on release, test field); Energy (screens
     off, sleep, sleep command with Enter); Screens (overview with drag,
     double-click and size; per screen: map, scale slider with marks and
     its stick, Default, shown at, size, same as primary, make primary,
     fit, background color with Enter, the picture grid, your pictures
     with remove and the two add buttons, the browser with up, pick,
     cancel and add); About. All are in the new app. Tried by hand
     (through `vctl`, throwaway HOME): every Appearance and Windows
     control, delay, screens-off, a picture, the browser's pick and
     cancel. By sight only: the rest. **Not yet seen at all: the overview
     and the map** (no second screen on Hubble; canary's nested session
     has two, but too small to judge).
  2. *Keyboard alone, finger alone.* Tab reaches every control (the
     sidebar's pages are buttons); Dave's hands owed for both.
  3. *No click lost under load.* The six-tab stress test running on
     Hubble (seq 0.79 to 0.85 s, color 8,500 to 8,700 lines/s, parsers at
     about two cores), the demo's + pressed 50 times through `vctl` as
     fast as it would send them: the count read 50 (the tree was built
     21 times; several presses fell in one frame). **Met.**
  4. *Idle costs nothing.* The installed settings app on Hubble, left
     alone 30 s: 0 CPU ticks. **Met.** (RSS 141 MB, mostly the 38
     previews.)
  5. *Scrolling the longest page* (Screens with its 38 pictures,
     scrollbar dragged down and up, 880 x 640 points at scale 3 = 5.1
     megapixels, `VERGENCE_UI_FRAMES=1`). First run on the GPU: median 20
     ms, 12 of it building the scene; `perf`: 28% copying memory, every
     visible picture copied into each frame's scene. Fixed:
     `vergence_scene::Scene::images` holds `Arc<Image>` (a program's
     picture is shared, not copied) and the GPU renderer keeps a
     picture's texture while frames keep showing it. Now: scene 0.5 ms;
     **frame median 8.9 ms, p95 9.7 ms on the GPU: just over the 8 ms
     asked.** What is left is the wait for the card to finish
     (`glFinish`, as the terminal found in milestone 1; explicit sync is
     to replace it); the CPU's part is about 0.5 ms. With the CPU
     renderer on Hubble at the same 5.1 megapixels: median 43 ms. On
     canary the nested session's screens are 390 x 488 and 600 x 750
     points, the window was fitted to 330 x 464, and its frames (0.3 ms)
     say nothing about the target; **canary's figure is still owed** (a
     larger nested screen, or a real display).
  6. *Meaning.* The dump names every control on every page tried, with
     its role, value and states. **Met.**
  7. Recorded under step 4. **Met.**

## Decided (Dave, 2026-11-10 20:00)

1. Widgets in a new crate, `vergence-widgets`.
2. Choice lists drawn inside the window; popup surfaces later.
3. The settings app's grays move into `vergence-look` and are kept.

## Not in this milestone

- Popup surfaces outside the window; menus beyond the Choice list.
- Multi-line text editing, shaping, right-to-left text, input methods
  (whitepaper §10).
- The accessibility bridge itself (AT-SPI or our own); only the record.
- Moving the terminal's bar or panes onto widgets.
- Shelf step 6's settings pages (they come right after, in the new app).
- Animations beyond the switch's slide, the caret and scroll momentum.
