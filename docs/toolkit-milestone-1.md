# Toolkit milestone 1: drawing, text, a window, and the terminal on them (whitepaper §6.10)

Planned 2026-11-09 (Dave: "write the toolkit milestone", after deciding that
the tools leave iced for a toolkit of our own, GPU first); plan accepted the
same evening. Nothing here is built yet.

The goal: the terminal runs on our own drawing, text and window code, on the
GPU, and passes the stress test that the iced terminal failed. Widgets, the
settings app and the shelf come after and build on what this milestone
leaves behind.

## What the 2026-11-09 stress test measured

One terminal, 1264 x 706 points at scale 3 (3792 x 2118 pixels) on Hubble,
six tabs: btop, htop, an endless `seq 1 1000000`, a flood with a different
24-bit colour in every cell, random cursor-addressed writes, a Unicode
flood.

| | iced, software | iced, GPU |
|---|---|---|
| `seq 1 1000000` | 5.5 to 30 s | 1.4 to 1.7 s |
| Colour flood | ~200 lines/s | ~3,000 lines/s |
| Cursor writes (x250) | ~170 /s | ~2,300 /s |
| Unicode flood | ~2,000 lines/s | ~28,000 lines/s |
| Terminal CPU | one core, pinned | one core, pinned |

Both held exactly one core: every tab's parsing and all drawing share the
interface thread. Profile, software: 64% rasterising, 20% glyph outlines and
hinting, 4% our parser and grid. Defects found: a redraw for every read of
output; clicks lost under load (input judged at the pointer's latest
position); six tabs push + off the window; combining marks dropped; programs
left running when the terminal is killed (the pty master is inherited by
every child); with multisampling on, the GPU renderer drew no icons.

These numbers are the bar. The acceptance section below is written against
them.

## The pieces

Five crates, each usable without the ones after it.

1. **`vergence-scene`**: what a window looks like, as data. A scene is a
   list of commands in reference units: filled and outlined rectangles with
   rounded corners, lines, glyph runs (font, size, positions, colour),
   images, clips and layers. This is the display-list format of whitepaper
   §6.6 in its first version, so it is written to be read from an untrusted
   process from the start: fixed-size records, every count and index
   checked, no pointers, a fuzz test. No dependencies.
2. **`vergence-text`**: fonts and glyphs. Finds font files (the desktop's
   font from the settings, and a fallback per character through the
   installed fonts), turns text into positioned glyphs, and rasterises a
   glyph once per (font, glyph, pixel size, subpixel offset) into a cache.
   **Colour is not part of the key**: iced's cache missed on every new
   colour, which is why btop cost what it did. Plain text only in this
   milestone: left to right, kerning, combining marks placed on their base,
   wide characters. It reads font files with ab_glyph, as the server's
   title bars do today.
3. **`vergence-render`**: turns a scene into pixels, two ways.
   - **GPU**: rectangles and glyphs as instanced quads, glyphs from an
     atlas texture, each monitor at its exact scale. A terminal screen is
     two instances a cell; 3792 x 2118 pixels is about 5,300 cells at the
     terminal's size, which is small work for any GPU. Only changed rows are sent again.
   - **CPU, the reference**: the same scene rasterised in software, in
     bands across the cores. It is what tests compare the GPU against, what
     runs on a machine without a GPU (canary), and what draws a window that
     changes once.
   The two must agree within a small tolerance on every test scene. That
   comparison is how the GPU renderer is tested, and later how any second
   GPU back end is.
4. **`vergence-wl`**: our own client side of the Wayland wire protocol, as
   `vergence-dbus` is our own D-Bus: the socket, file-descriptor passing,
   the object table, and exactly the interfaces the terminal uses (below).
   The server's own messages are parsed as carefully as a stranger's.
5. **`vergence-ui`**: a window. It owns the connection, turns events into
   input **in the order and at the place they happened**, asks for a scene
   when something changed, and shows it once per frame the server asks
   for. No widgets yet: in this milestone the terminal lays out its own bar
   and panes. It also holds the start of the accessibility tree (a window,
   its tabs, its panes, the text under the cursor) so that Contract 3 has
   something real to carry later.

**Threads.** The interface thread handles input and builds scenes; it never
parses program output and never rasterises. Each terminal (tab or pane) has
a thread that reads its pty and runs the parser into its own grid. One
render thread draws. A frame takes whatever the grids hold when the server
asks for it, so a flood in one tab costs that tab's thread and nothing else,
and the window never redraws more often than the monitor shows.

**How a GPU frame reaches the server.** Without the Wayland library there
is no `wl_egl_window`. The renderer draws into a buffer it allocates
through GBM on the GPU's render node and hands the server that buffer's
file descriptor with `zwp_linux_dmabuf_v1`, which the server already
accepts on real hardware. Where there is no GPU, or the server runs nested
without dmabuf, the CPU renderer draws into shared memory (`wl_shm`). The
choice is made at start and again if the GPU is lost; a tool keeps running
through a GPU reset.

**Wayland interfaces used.** wl_compositor, wl_surface, wl_shm,
zwp_linux_dmabuf_v1, xdg_wm_base and xdg_toplevel (move, resize, maximise,
minimise), xdg-decoration (to say the terminal draws its own bar),
wp-fractional-scale and wp-viewporter (the exact scale), wl_seat with
pointer and keyboard, wp-cursor-shape, wl_data_device (clipboard). All are
ones the server offers today.

**What stays borrowed under it**, each already on the list in whitepaper
§8: ab_glyph for reading font files and outlining glyphs, the system's
xkbcommon for keymaps (through declarations of our own, not a crate), and
the operating system's EGL, OpenGL ES and GBM, which are interfaces, not
borrowing. Nothing new comes in.

## Steps

1. **Scene, text and the CPU renderer.** No window. A command-line tool
   renders a scene file to a PNG. Golden-image tests; a test scene that is
   a full terminal screen of coloured cells at 3792 x 2118 (5,338 cells). The font
   fallback and the glyph cache with their tests. Runs on all three
   machines.
2. **A window, in software.** `vergence-wl` and `vergence-ui`: a window
   showing a scene through shared memory at the monitor's exact scale,
   following a move between the monitors, with pointer, keyboard,
   clipboard and resizing. A small demo program stands in for the
   terminal. The wire decoder gets a fuzz test.
3. **The GPU renderer.** GBM and EGL without a window, the atlas, dmabuf
   to the server, the agreement tests against the CPU renderer, falling
   back to software when there is no GPU. The demo program draws a
   scrolling terminal screen.
4. **The terminal on it.** The parser, grid and pty move over unchanged
   (they have no iced in them). New: a parser thread per terminal, frames
   on the server's request, the bar (tabs that shrink to fit so + never
   leaves the window, window buttons on the configured side, tab shapes as
   now), panes with dividers, the scrollbar, selection, the right-click
   menu, colour schemes, the settings followed live. Fixed here because
   they are in code this step rewrites or touches: combining marks kept on
   their cell, and the pty master closed in children so a dead terminal's
   programs get their hangup.
5. **Stress, soak, switch.** The six-tab test again, an hour's soak on the
   GPU on Hubble, a run on Techbench and, in software, on canary. Then
   `tools/install.sh` installs this terminal, and the iced terminal's
   interface code is deleted. iced stays in the workspace for the settings
   app only.

## Progress

- **2026-11-09, step 1, first part: `vergence-scene`.** The scene types
  (fill, stroke, line, glyph run, image, clip), a builder, `validate` with
  the limits a renderer relies on, and the wire format ("VSCN", version 1)
  with a decoder that refuses anything invalid and reserves no memory a
  lying count asks for. 14 unit tests and a three-part fuzz test (random
  scenes round-trip; damaged scenes and random bytes never panic, and what
  decodes re-encodes to the same bytes); 2,000,000 fuzz iterations clean on
  Hubble. Green on Hubble, canary and Techbench. Layers with their own
  opacity were left out until a tool needs them.
- **2026-11-09, step 1, second part: `vergence-text`.** Our own reader of
  font files (sfnt.rs: family, weight, slant, fixed pitch, a variable
  font's weight axis, character coverage; a few kilobytes read per file),
  the font database with generic names and fallback per character (db.rs),
  and `Fonts`: faces, glyphs with fallback, one-line layout with kerning,
  and the glyph cache keyed by face, glyph, size and sub-pixel position,
  never colour. ab_glyph outlines and rasterises. 23 tests, two of them
  fuzz tests of the font reader (2,000,000 iterations clean); green on
  Hubble, canary and Techbench. `cargo run --example fonts` reports what a
  machine has: Hubble's 349 font files are indexed in 7.5 ms; "monospace"
  is Noto Sans Mono on all three; Techbench's variable fonts give a true
  bold. Found on the way:
  - Noto Sans Mono says "not fixed pitch" in its `post` table and "fixed
    pitch" in PANOSE; both are read now.
  - Hubble has no CJK font (the missing glyph is drawn); the first lookup
    of a character only a far-down font has costs 10 to 30 ms while
    coverage tables are read, then nothing.
  - A colour emoji font (canary's Noto Color Emoji) is chosen as a fallback
    but has no outlines, so nothing is drawn: colour glyphs are outside
    this milestone, and until then such a face should be passed over for
    one that can draw.
  - Outlines are not hinted. Fine at scale 2 and 3; to be judged on the
    1080p monitor when the renderer draws.
- **2026-11-09, step 1, third part: the CPU renderer.** `vergence-render`
  with `Cpu` (cpu.rs): the scene is prepared once (positions to pixels at
  the scale asked for, clips to a scissor rectangle plus shapes to test
  against, glyphs from the text cache at the exact pixel size), then drawn
  in bands of 32 rows on every core. Fills, rounded fills, outlines, lines
  with round ends, tinted glyphs, pictures stretched smoothly, clips
  (rounded and between pixels too). `render_rows` redraws a range of rows
  to exactly what a whole render gives. `examples/vscene.rs` draws two
  demonstration scenes to a PNG with timings. 17 tests: pixel-exact
  shapes, a circle's area, symmetry, 1 thread against 2, 3, 6 and 16 (the
  same picture), rows against the whole, refusals, text where it was laid
  out, colour not rasterising again, and **the same picture on Linux and
  FreeBSD** (hashes of a busy scene at three scales). Green on Hubble,
  canary and Techbench. Measured on the stress test's window, 3792 x 2118
  pixels with every cell its own colours and character:

  | | cores | a frame | one core | one text row redrawn |
  |---|---|---|---|---|
  | Hubble (i5-8500T) | 6 | 13.5 ms | 64 ms | 2.0 ms |
  | canary (VM) | 16 | 5.8 ms | 32 ms | 0.9 ms |
  | Techbench | 20 | 4.1 ms | 28 ms | 1.0 ms |

  That window holds 5,338 cells (158 x 34 at 13.33 units), not the "about
  30,000" this plan guessed; acceptance 1 and the renderer notes are
  corrected below. Acceptance 1 is met for the frame (13.5 ms against 30)
  and at its limit for one row on Hubble (2.0 ms against "under 2"): most
  of a row's cost is preparing the whole scene again, which the window
  code in step 2 avoids by keeping the prepared scene between frames.
  With that, **step 1 is complete**. Next: step 2, the window.
- **2026-11-09, step 2, first part: a window.** `vergence-wl`, our own
  client side of the Wayland wire protocol with no dependencies: wire.rs
  (messages as bytes), conn.rs (the socket, with file descriptors passed
  by `sendmsg`/`recvmsg` declared for Linux's and FreeBSD's layouts),
  shm.rs (shared memory from `memfd_create`), client.rs (the requests and
  events a window needs, written by hand: registry, surface, shm buffers,
  xdg-shell, wp-viewporter, wp-fractional-scale, xdg-decoration). 17 tests
  and a fuzz test of everything read from the server (2,000,000
  iterations clean); green on Hubble, canary and Techbench, descriptor
  passing included. `vergence-ui` is a window: `App` says what the window
  looks like at a size, `run` asks the server for it, follows its size
  and the monitor's exact scale, draws with the CPU renderer into shared
  memory, and shows one frame for each the server asks for, only when
  something changed. `examples/window.rs` is the demonstration.
  **On Hubble, 21:19:** the first window of our own toolkit, under the
  server's title bar, 640 x 420 units drawn as 1920 x 1260 pixels on the
  14" 4K at scale 3 and sharp; animated, 240 frames in 7.98 s, 30.1 a
  second on a 30 Hz monitor; moved to the 1080p monitor it drew itself
  again at scale 1 (640 x 420 pixels). Window memory recorded it
  (vergence-demo). Open:
  - drawing takes 15 ms a frame for that window, more than the renderer
    alone needs: the picture is drawn, then copied into the shared memory
    with red and blue swapped. The renderer should draw straight into the
    buffer in the server's byte order;
  - one "slow frame" (56 ms) in the server's log as the window first
    appeared;
  - the server may configure a size before the scale is known, so the
    first frame can be drawn at scale 1 and again at the real scale.
  Next in step 2: input (pointer, keyboard, clipboard), resizing by the
  user, and the wire fuzz extended to them.
- **2026-11-09, step 2, second part: input.** `vergence-wl` gained the
  seat, pointer and keyboard events, the pointer's shape by name
  (wp-cursor-shape), the requests to move, resize, maximise and minimise,
  a wait with a time limit (`poll`), and a bounded reader for files the
  server sends (the keymap); 21 tests, fuzz extended. `vergence-ui`
  gained `App::input` with `Input` (pointer moved and left, button,
  scroll, key with symbol, text and modifiers, focus), a `Window` handle
  for what a program may ask in return (redraw, close, move, resize,
  cursor, title, maximise, minimise), key repeat at the server's rate, and
  keyboard.rs, which turns keys into symbols and text with the system's
  xkbcommon through declarations of our own (3 tests). Every event is
  given to the program as it is read, a press with the position the
  pointer had at that press. **On Hubble: 100 clicks, each followed at
  once by a pointer move away, while the window redrew every frame: 100
  of 100 arrived at the place they were pressed** (acceptance 2's click
  test; the iced terminal lost such clicks). The hand cursor over a
  shape, the pressed shape and the keyboard focus showed in a screenshot.
  Green on Hubble, canary and Techbench. Found on FreeBSD: the linker
  does not search /usr/local/lib, so vergence-ui has a build script that
  asks pkg-config where xkbcommon is. Not yet checked: typing, key repeat,
  scrolling, and moving and resizing by hand (the control socket has no
  keys or wheel; Dave to try the demo). Still to do in step 2: the
  clipboard, dead keys and compose, the buffer drawn in place.
  Tried at the desk the same evening (Dave; read from the demo's log and
  the server's): typing arrives key by key with its text, drag-to-move
  and corner resize work (640 x 420 to 777 x 518). Key repeat and the
  wheel were not seen in the log and are still unverified.
- **2026-11-09, step 2, third part: the clipboard (text).** `vergence-wl`:
  wl_data_device (sources, offers, the selection), a pipe, and a wait that
  watches a second descriptor; an "offer" whose id is not in the server's
  range is not believed. 23 tests. `vergence-ui`: `Window::copy` puts text
  on the clipboard and writes it to whoever asks on a thread of its own;
  `Window::paste` asks for it and it arrives as `Input::Paste`, read in
  pieces between other events, at most 16 MiB and 5 seconds; pasting our
  own copy takes no trip through the server. **On Hubble: text with
  accents and symbols copied in one toolkit window by a click and pasted
  in another through the server, exact; and pasted back into the window
  that copied it.** Green on Hubble, canary and Techbench. Not yet
  checked: copy and paste with another toolkit's programs (Firefox, the
  iced terminal), and a large paste. Kinds other than text, drag and
  drop, and the primary selection (middle click; the server does not
  offer it) are not done.
- **2026-11-09, step 2, fourth part: drawing in place, and compose.**
  The CPU renderer draws straight into a window's buffer in the server's
  byte order (`Cpu::render_target`, `Order::Bgra`; a test checks it is the
  same picture byte for byte with red and blue changed places), so the
  window keeps no picture of its own and copies nothing. The demo window,
  1920 x 1260 pixels on Hubble: **4.8 ms a frame, from 15.1**, still 30.1
  frames a second on the 30 Hz monitor. Dead keys and compose sequences:
  keyboard.rs feeds every pressed key through xkbcommon's compose table
  for the user's locale; a key taken into an unfinished sequence types
  nothing and is marked `composing`, the key that completes one types the
  result (dead acute then e: é; Compose, o, c: ©; a sequence that leads
  nowhere types nothing). 4 keyboard tests, 18 renderer tests; green on
  Hubble, canary and Techbench. Hubble's session runs with
  LANG=en_US.UTF-8, which has a compose table. Not tried at a keyboard:
  the US layout has no dead keys, so a sequence needs a Compose key set
  in the layout.
  **Step 2 is complete** but for what only hands can check (key repeat,
  the wheel, a compose sequence, copy and paste with other programs).
  Left for later steps: only the changed rows redrawn (the window still
  draws every row of a frame), and the prepared scene kept between
  frames. Next: step 3, the GPU renderer.
- **2026-11-09, step 3, first part: the GPU renderer, without a window.**
  `vergence-render` gl.rs loads EGL, OpenGL ES 3 and GBM when a renderer
  is made (`dlopen`, declarations of our own): nothing is linked, so a
  machine without the libraries or their development files still builds
  (Techbench has only the run-time ones). gpu.rs: a context on the GPU's
  render node through GBM with no window, or Mesa's surfaceless software
  GL where there is no GPU; the scene prepared exactly as for the CPU
  renderer; one instance a shape; one fragment program with cpu.rs's
  coverage arithmetic line for line; a glyph atlas (a glyph uploaded once,
  tinted when drawn); shapes drawn in order in as few calls as there are
  pictures to switch between; the pixels read back for comparison.
  tests/gpu.rs draws the same scenes both ways. **Agreement:** plain
  shapes, outlines, lines, clips and pictures 0 or 1 step apart in every
  byte; a screen of text never more than 2; four hundred translucent
  shapes over each other never more than 3 (each layer may round
  differently). Green on Hubble (UHD 630), Techbench (UHD 770) and canary,
  where the "GPU" is llvmpipe, so FreeBSD runs the same comparison.
  The stress test's window, 3792 x 2118 pixels, 5,338 cells:

  | | GPU | a frame drawn | CPU renderer |
  |---|---|---|---|
  | Hubble | UHD 630 | 8.0 ms | 13.7 ms on 6 cores |
  | Techbench | UHD 770 | 6.5 ms | 4.6 ms on 20 cores |
  | canary | llvmpipe | 14.8 ms | 6.1 ms on 16 cores |

  Most of the GPU's time is ours, not the card's: the whole scene is
  prepared and every instance rebuilt and uploaded for each frame, on one
  thread. Keeping what did not change is where the next gain is, for both
  renderers. Reading the pixels back costs more than drawing them (18.7 ms
  on Hubble) and is for tests only.
  **The comparison found a fault in the reference:** a line's coverage was
  multiplied by a clip's before being limited to 1, so a line was too
  strong at the edge of a rounded clip. Fixed in cpu.rs.
  Not done: a shape inside more than one rounded or off-pixel clip (the
  GPU renderer refuses the scene, and a caller falls back to the CPU);
  the atlas is one texture, emptied when full. Next: the server shown a
  GPU buffer (linux-dmabuf from `vergence-wl`, GBM buffers), then the
  window choosing GPU or CPU by itself.

## Acceptance

Every step: `cargo test --workspace` green on Hubble, canary and Techbench;
no new warnings.

1. The CPU renderer draws the full-screen cell scene (5,338 cells, every
   one its own colours) in under 30 ms on Hubble using all six cores, and
   a one-row change in under 2 ms. **Met 2026-11-09** for the frame
   (13.5 ms); one row is 2.0 ms, to come down in step 2 (Progress). Golden images match on all
   three machines, FreeBSD included. The scene decoder survives the fuzz
   test.
2. The demo window is sharp on both of Hubble's monitors (a screenshot of
   each, compared with the CPU renderer's PNG of the same scene) and
   re-renders at the new scale when dragged across. Typing, clicking,
   copy and paste work. A click followed at once by a pointer move lands
   where it was pressed, 100 times of 100, while the window is redrawing
   continuously.
3. GPU and CPU renders of every test scene agree within the tolerance.
   The scrolling demo holds the monitor's refresh rate at 3792 x 2118 with
   the renderer's thread under 25% of a core. With the GPU taken away
   (the render node made unreadable) the window comes up in software.
4. Everything the iced terminal does today still works, checked against a
   list made from its code before step 4 starts. Seven tabs fit and + is
   visible. `e` + U+0301 shows as é. Killing the terminal with SIGKILL
   leaves none of its programs running.
5. The six-tab load, same window size, on Hubble's GPU:
   - `seq 1 1000000`: under 1 s whichever tab is shown;
   - the colour flood above 10,000 lines/s, the cursor test above 10,000
     batches/s;
   - no thread at 100% except the ones parsing a flood; the interface
     thread under 10% of a core;
   - a tab click shown within two frames, 50 of 50;
   - memory flat (within 5%) between minute 10 and minute 60;
   - nothing in the kernel log, no slow frames in the server's.
   In software on canary the same load completes without a crash and the
   interface still answers. Idle, the terminal uses no measurable CPU.

Done, as every milestone, when it has run on Hubble (Void) and Techbench
(Fedora) and builds and passes on canary (FreeBSD).

## Security and isolation (whitepaper §6.10, §7.3)

- A scene is data and is validated wherever it is read; the format has no
  way to name memory or run anything.
- Font files and program output are untrusted input. In this milestone
  both are handled off the interface thread and behind fuzz tests. Moving
  font parsing and glyph outlining into a process of their own (Capsicum
  on FreeBSD, seccomp on Linux) is designed for, by keeping
  `vergence-text`'s interface to bytes in and bitmaps out, and done in a
  later milestone.
- The GPU buffer handed to the server is one the tool itself allocated;
  the tool receives no handle to anyone else's.
- The toolkit opens no socket of its own. It gets what any Wayland client
  gets, which is the point.

## Decided (Dave, 2026-11-09 20:38)

1. **OpenGL ES 3 through EGL is the first GPU interface.** It is what the
   server draws with and it works today on all three machines; Vulkan found
   no adapter on Hubble on 2026-11-09 (the loader is probably not
   installed). The renderer keeps its GPU calls behind one small seam so
   Vulkan, which the whitepaper (§8.3) wants in the end, can follow.
2. **Crate names**: vergence-scene, -text, -render, -wl, -ui.
3. **ab_glyph and xkbcommon stay** for this milestone, each to be decided
   on later like the rest of the §8 table.
4. **The iced terminal while this is built** stays as it is, with the two
   fixes of 2026-11-09 and no more work. (The question offered two
   choices and the answer was "yes to all five"; this is the one the plan
   proposed. The other, giving it the pty and combining-mark fixes now, is
   an hour's work if wanted.)
5. **Smithay.** `vergence-render` is the renderer the 8 November decision
   said would replace Smithay's in the server first. The order: this
   milestone, then the shelf drawn through it inside the server, then a
   server milestone that moves compositing itself onto it, then Smithay's
   display and input back ends, and its Wayland front end last or kept as
   an edge. No second replacement runs beside this milestone.

## Not in this milestone

- Widgets and layout, and the settings app (the third piece of work).
- The shelf, and the server drawing its title bars, menu and
  notifications with this renderer (the second).
- Replacing Smithay's renderer, DRM/KMS or input in the server.
- The native window protocol; scenes sent to the server instead of pixels;
  remote display.
- Text shaping for scripts that need it, right-to-left text, input
  methods, emoji in colour.
- A process of its own for fonts and decoding.
- Vulkan.
