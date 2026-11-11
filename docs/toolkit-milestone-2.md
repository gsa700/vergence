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
| `pick_list` | 8 | style, themes, icons, pointer, fonts and sizes, terminal colours, energy times, fit |
| `toggler` | 6 | contrast, motion, key repeat, minimise/maximise, same-as-primary |
| `text_input` | 4 | custom accent, key-repeat test, sleep command, background colour |
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

- **Label**: text in a size, weight and colour; wraps to its width;
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
in the accent colour. Every page of the settings app must be usable with
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

**The look.** Colours, the radius and sizes come from `vergence-look`, with
the light and dark sets, high contrast and the accent from desktop.toml
`[appearance]` read live, as the terminal reads them today. The settings
app's own colours (its sidebar, page and card greys) move into
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
    settings app's greys (sidebar, page, card, line, dim, text, hover).
  - New crate `vergence-widgets`:
    - `Element`: label (size, weight, colour, dim, wrapping, alignment),
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

## Decided (Dave, 2026-11-10 20:00)

1. Widgets in a new crate, `vergence-widgets`.
2. Choice lists drawn inside the window; popup surfaces later.
3. The settings app's greys move into `vergence-look` and are kept.

## Not in this milestone

- Popup surfaces outside the window; menus beyond the Choice list.
- Multi-line text editing, shaping, right-to-left text, input methods
  (whitepaper §10).
- The accessibility bridge itself (AT-SPI or our own); only the record.
- Moving the terminal's bar or panes onto widgets.
- Shelf step 6's settings pages (they come right after, in the new app).
- Animations beyond the switch's slide, the caret and scroll momentum.
