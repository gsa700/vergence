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
- **2026-11-09, step 3, second part: GPU frames in the window.**
  `vergence-wl`: linux-dmabuf (formats with their modifiers, a buffer
  offered and the server's created or failed; a "created" whose id is not
  in the server's range is not believed); 24 tests. `vergence-render`:
  `Gpu::buffer` makes a `GpuBuffer`, GPU memory from GBM imported as an EGL
  image and drawn into through a framebuffer, with the file descriptor,
  stride, offset, format (ARGB8888) and modifier the server needs;
  `draw_into`, `read`, `release`; a test that the shared buffer holds the
  same pixels as the renderer's own texture. `vergence-ui`: a window
  draws on the GPU when the server offers linux-dmabuf and the machine
  has a GPU that shares buffers; it asks the server to take up to three
  buffers and draws in software until one is taken; a frame the GPU
  renderer cannot draw, or a refusal from the server or the driver, goes
  to the CPU renderer and shared memory, which stays as it was.
  `VERGENCE_RENDER=cpu` keeps to the CPU. **On Hubble (UHD 630, Void):**
  the demo window's frames reach the server as dmabufs (modifier
  I915 Y-tiled), 237 of 240 on the GPU (the first two in software while
  the buffers were taken), 30.2 frames a second on the 30 Hz monitor, a
  screenshot correct. Animating, **the demo uses 2% of one core with GPU
  frames against 114% with CPU frames**, and the server 9% against 15%.
  (The time to draw one frame reads about the same, 7.8 ms, because the
  GPU path waits for the card to finish before handing the buffer over:
  that wait is `glFinish`, to be replaced by explicit sync.) No GPU hang,
  nothing in the kernel log; one 22 ms slow frame in the server's.
  Green on Hubble (whole workspace), canary and Techbench. Not tried on
  screen: Techbench (its monitors are on Hubble). Acceptance 3's "with the
  GPU taken away the window comes up in software" is met by
  `VERGENCE_RENDER=cpu` and by canary, not yet by removing access to the
  render node.
  Also on 2026-11-09, at the desk: key repeat (Backspace held) and the
  wheel work in the demo window.
- **2026-11-09, step 4, first part: the terminal.** `crates/vterm`: the
  new terminal, one window under the server's title bar. The screen, the
  parser and the pty are vergence-term's grid.rs, vt.rs and pty.rs, used
  from there unchanged (they never had iced in them; they move here when
  the first terminal is retired). New around them: the program's output
  is read and parsed on a thread of its own into the screen behind a
  lock, and the window is woken (`vergence_ui::Waker`, `App::woken`; the
  toolkit's wait now watches two more descriptors); the window draws what
  the screen holds once for each frame the monitor shows; the picture is
  a scene (a rectangle a run of cells, glyphs cell by cell with font
  fallback), on the GPU; keys go to the program as xterm's sequences;
  paste (bracketed), the scrollback (wheel, Shift+PageUp/PageDown, "n
  lines back"), resizing, the title, OSC 52 copies, the Tango or
  Catppuccin scheme from desktop.toml. 17 tests (the engine's 14 and 3 of
  its own). The binary is 1.2 MB; the iced terminal's is 22.7 MB.
  **On Hubble, a window of 3783 x 2019 pixels (156 x 35 cells) on the 4K,
  the stress test's jobs one after another:**

  | | iced, software | iced, GPU | vterm |
  |---|---|---|---|
  | `seq 1 1000000` | 5.5 to 30 s | 1.4 to 1.7 s | **0.61 s** |
  | Colour flood, lines/s | ~200 | ~3,000 | **20,700** |
  | Unicode flood, lines/s | ~2,000 | ~28,000 | **222,700** |
  | Cursor writes (x250) /s | ~170 | ~2,300 | **14,900** |

  (The iced figures are from six tabs at once; these are one job at a
  time, so the comparison is fair for the single-thread ceiling, not yet
  for several terminals together.) The parsing thread ran at 95 to 99%
  of a core, as a flood should; the window's thread at about 20%, which
  misses acceptance 5's "under 10%": it builds the whole scene and every
  instance again for each frame. 141 MB of memory; no GPU hang; nothing
  from the server.
  Green on Hubble (whole workspace), canary and Techbench.
- **Step 4, second part (2026-11-09): tabs and the window's own bar.**
  `vterm` draws its own frame: a bar (`bar.rs`) with the window buttons on
  the side `Gtk/DecorationLayout` names, tabs at most 200 units wide that
  shrink so + is always in view, and the fonts from `Gtk/FontName` and
  `Gtk/MonospaceFontName` (read at start; not yet followed live). New tab
  Ctrl+Shift+T or +, close Ctrl+Shift+W or a middle click, next and
  previous Ctrl+PageDown and Ctrl+PageUp; the bar drags the window and a
  double click maximizes it. Each tab is its own session with its own
  parsing thread. Seven tabs fit with + visible (screenshot on Hubble).
  Two faults in the shared `pty.rs` were found and fixed on the way:
  - the pty master was handed down to every program the terminal started
    (and each later tab's shell held the earlier tabs' masters), so a
    killed terminal's programs never saw the hangup. It is now opened
    close-on-exec. Seven tabs, SIGKILL: no program left (acceptance 4);
  - `ptsname` answers in one buffer shared by the process, so two
    terminals opened at the same moment from two threads could get each
    other's name, or half of one. It showed on canary as a test that
    failed one run in three ("Is a directory") or hung; the call is now
    made one at a time. Twelve runs of twelve clean after.
  Green on Hubble, canary and Techbench (`vterm`, `vergence-term`).
- **Step 4, third part (2026-11-09): selecting with the mouse.**
  `select.rs`: a selection is counted in cells and absolute line numbers,
  so it stays on its text as output scrolls. A drag selects characters, a
  second press on the same cell words (a path or an address is one word),
  a third whole lines, and a drag after either goes on by that unit;
  Shift and a press reaches the selection to there. Held past the top or
  bottom edge the view scrolls a line every 40 ms with the selection's
  end on the edge row. Ctrl+Shift+C copies: lines joined by newlines,
  the cells that only pad a line dropped, double-width characters whole.
  Typing, another screen (the alternate one), a reset or a change of
  width lets the selection go. A program that asked for the mouse keeps
  it unless Shift is held (reporting itself is the next part). Not done:
  a wrapped line is copied as two lines (the screen does not record where
  it wrapped), and there is no primary selection (middle-click paste):
  that needs a protocol the toolkit does not speak yet.
  Tests (4 new, 27 in `vterm`) green on Hubble, canary and Techbench; the
  pointer's part awaits a hand test.
- **Step 4, fourth part (2026-11-09): mouse reporting.** `mouse.rs`:
  xterm's modes 9, 1000, 1002 and 1003 in the SGR (1006) and the old
  encoding, as the first terminal sent them. A program that asked for the
  mouse is told of presses and releases of the three buttons (a release
  wherever the pointer has gone), of moves from cell to cell if its mode
  asks, and of each notch of the wheel. Shift keeps the mouse for
  selecting; so does a view scrolled back, where what is under the
  pointer is not the program's screen. The pointer is an arrow over a
  program that has the mouse, a text bar otherwise. Tests (2 new, 29 in
  `vterm`) green on Hubble, canary and Techbench; awaits a hand test in
  `mc` and `htop`.
- **Step 4, fifth part (2026-11-09): the scrollbar.** `scrollbar.rs`: a
  bar 5 units wide in the padding at the right edge, there whenever there
  is scrollback (never on the alternate screen). The thumb is the
  screen's share of all the lines, at least 24 units long; faint at the
  live screen, clearer scrolled back or held. A press within 12 units of
  the edge takes hold of it (on the track, the thumb comes to the
  pointer) and the view follows the drag. The "lines back" label moved
  left of it. Tests (2 new, 31 in `vterm`) green on Hubble, canary and
  Techbench; seen in a screenshot on Hubble, the drag awaits a hand test.
- **Step 4, sixth part (2026-11-09): combining marks and live settings.**
  - A cell of the screen (`grid.rs`, shared with the first terminal)
    now keeps up to two combining marks with its character; before, they
    were dropped. A mark goes on the character printed last (under the
    cursor when the line is full, on the left half of a double-width
    one), takes no cell, and is copied with its character. `vterm` draws
    each mark on its character: after it for a font whose marks have no
    advance, in its place for one whose marks are a cell wide. This is
    placement without the font's own mark tables, enough for a fixed-pitch
    face. `e` + U+0301 shows as é (acceptance 4), and "Việt", with two
    marks on one letter, matches the precomposed form in a screenshot on
    Hubble. The first terminal keeps the marks but does not draw them.
  - The settings are followed while the terminal runs: `settings.toml`
    and `desktop.toml` are looked at once a second (as the first terminal
    does; the session bus replaces this when the toolkit speaks it), and
    the colour scheme, both fonts and the buttons' side change at the
    next frame, the grids following the new cell size. Checked on Hubble
    against a copy of the configuration: Tango to Catppuccin, buttons
    left to right, 10 to 14 points, in one running window.
  Tests (2 new: 33 in `vterm`, 24 in `vergence-term`) green on Hubble,
  canary and Techbench.
- **Step 4, seventh part (2026-11-09): the right-click menu.** `menu.rs`:
  Copy (greyed with nothing selected), Paste, New tab and Close tab,
  each with its keys, drawn in the window over the terminal and kept
  inside it. The right button opens it at the pointer (Shift and the
  right button where a program has the mouse); a press on a line does
  what it says, a press elsewhere or Escape closes it. The split lines
  join it with panes. Tests (2 new, 35 in `vterm`) green on Hubble,
  canary and Techbench; its picture checked from the CPU renderer, its
  use awaits a hand test.
- **Step 4, eighth part (2026-11-09): panes, and Ctrl+C and Ctrl+V.**
  - `panes.rs`: a tab is a tree; a leaf is a terminal, a split divides
    its room in two, side by side or one above the other. One bar of tabs
    at the top, each tab with its own panes (Dave, 2026-11-09: "one tab
    bar is fine"; tabs in each pane were the alternative).
    Ctrl+Shift+R splits side by side, Ctrl+Shift+D top and bottom, both
    also in the menu; a press makes a pane the one the keys go to (a
    block cursor there, an outline in the others); the line between two
    panes can be dragged (no pane below a tenth of its split); the wheel
    is the pane's under the pointer; Ctrl+Shift+W, or the program
    ending, closes the pane, its neighbour takes the room, and the tab
    goes with its last pane. Selection, the scrollbar, mouse reporting
    and the "lines back" label are each pane's own. `--panes N` opens
    the first tab divided, for trying it. Three panes seen in a
    screenshot on Hubble, each with its own shell and size.
  - Ctrl+C copies when text is selected and lets the selection go; with
    nothing selected it is the interrupt key as ever. Ctrl+V pastes
    (Dave, 2026-11-09). The Ctrl+Shift forms still work. Given up for
    it: the control character Ctrl+V sent (the shell's "next key as it
    is", vim's block selection, for which vim also takes Ctrl+Q).
  Tests (3 new, 38 in `vterm`) green on Hubble, canary and Techbench;
  the pointer's and keys' part awaits a hand test.
  Step 4's list is done; what is left of it is the hand tests and the
  check against the first terminal's list (acceptance 4).
- **Step 4, ninth part (2026-11-10): the check against the first
  terminal.** Dave's hand tests of the pointer and keys are done. The
  first terminal's code (main.rs, term.rs, look.rs) read against
  `vterm`'s; four things were missing and are in now:
  - Shift+Home and Shift+End go to the two ends of the scrollback.
  - On the alternate screen Shift+PageUp/PageDown/Home/End are the
    program's (they were swallowed).
  - Ctrl+Tab and Ctrl+Shift+Tab change tab, as Ctrl+PageDown/PageUp do.
  - The desktop's appearance from desktop.toml: `color-scheme = "light"`
    (Catppuccin Latte, or Tango on white), `contrast = "high"` (text on
    pure black or white), `accent` (cursor, selection, highlights); live
    like the rest. Seen in screenshots on Hubble with a throwaway
    configuration.
  Left as it is on purpose: the first terminal dropped typed text that
  contained a control character, a guard against what iced handed it.
  Here the text comes from xkbcommon, whose control characters are the
  ones xterm sends (Ctrl+2 is NUL, Ctrl+3 to Ctrl+7 are 0x1b to 0x1f);
  dropping them would take those keys away.
  Tests (2 new, 40 in `vterm`) green on Hubble, canary and Techbench.
- **The window's look (Dave, 2026-11-10).** One corner radius for
  everything `vterm` draws round, the buttons' 5 units (`bar::RADIUS`):
  the tabs' top corners (were 7), the menu and its lines (were 8 and 5),
  the "lines back" label (was 6), the scrollbar's ends as far as its
  width lets them, and the window's own corners when the server rounds
  them. The window buttons have a shape at rest, the fill of a tab that
  is not shown, and are as tall as the tabs, standing on the bar's bottom
  edge with them. Tried and dropped the same day: the Super key as a
  command key (it is not under the thumb on a PC keyboard, and other
  applications use Ctrl), and one 6-unit margin for bar, text and
  scrollbar.
- **Step 5 begun (2026-11-10): the stress test, kept.**
  `crates/vterm/examples/flood.rs` holds the jobs (colour, cursor,
  unicode, and `time` for `seq`), each writing its rate to a report;
  `tools/vterm-stress/run.sh [minutes]` opens one `vterm` with the six
  tabs through `vctl` and writes a line a minute: the rates, the memory,
  and each thread's share of a core (the parsing threads are named
  `parse`). **The first figures written here (commit 17ea6fd) were
  wrong** and are withdrawn: the flood jobs asked the terminal's size
  once, as they started, when it was still 80 x 24, so "full-width"
  colour lines were 80 columns; and the interface thread's share for a
  job alone was read from an average that included starting up. The
  jobs now ask the size every second and report it.
- **Step 5, the soak (2026-11-10).** Six tabs for 62 minutes on Hubble.
  Memory flat: 175.3 MB in the first minute, 175.6 MB from the 13th to
  the last. The interface thread 3 to 4% throughout (btop shown). No
  crash; nothing in the kernel log but the profiler's own notices. The
  server logged 25 slow frames (21 to 165 ms), every one in a minute
  when test windows were being opened and closed, screenshots taken or
  a profile recorded beside the soak, none in the stretches left alone:
  "no slow frames" is **not shown** and needs a run nobody disturbs.
- **Step 5, the interface thread (2026-11-10).** Measured properly: one
  tab, the colour flood at full width (135 x 38 cells, 21,700 lines/s),
  a quiet machine, 30 Hz, CPU time by the frame (`VTERM_FRAMES=1`
  counts frames). At 17ea6fd: 6.5 ms a frame, **19.5% of a core**.
  With this part: 5.0 ms, **15.0%**. Acceptance 5 asks for under 10%:
  **not met.** What this part did:
  - `vergence-text`: the glyph cache has a quick front, 2,048 places
    each holding the glyph last asked for there, so a frame's glyphs
    skip the cache's own hash (kept as it is: its keys can come from a
    program that means harm).
  - `vergence-render`: the bands of rows are worked out only for the
    CPU renderer, which is the one that uses them; the atlas's lookup by
    address has a one-multiplication hash; an instance is put together
    and added whole.
  - `vterm`: the parsing thread leaves its news (draw; title, clipboard
    or exit) in an atomic, so the window no longer takes a flooding
    terminal's lock at every wake to learn "draw"; a plain character's
    font and glyph are found once a frame; the scene's list starts at
    the last frame's length. `--maximized` opens the window filling its
    screen (asked after the first frame; before it, the server's
    remembered place wins).
  What is left is in the shape of things: a cell with colours of its
  own is two commands, two prepared shapes and two 128-byte instances,
  and the window waits for the parser's lock to draw. The next piece of
  toolkit work is a primitive for a grid of cells (scene, both
  renderers, the shader), and drawing from a copy of the screen taken
  under a short lock.
- **Found by the stress test, not yet fixed: a tab that is not shown
  keeps the size it started with** (80 x 24) until it is looked at; only
  the terminals drawn are fitted to the window. So in the six-tab run
  the three floods behind btop ran at 80 x 24: `seq 1 1000000` 0.71 to
  0.74 s; colour 12,400 to 12,600 lines/s of 80 columns; cursor 6,700
  batches/s, **under acceptance 5's 10,000**; Unicode 220,000 lines/s;
  the interface thread 8% with this part's build (the machine full: the
  jobs, the kernel carrying their bytes, a video in Firefox). The
  six-tab figures at the window's real size wait for that fix. Each job
  alone at 80 columns: cursor 19,500 batches/s, colour 36,000 lines/s;
  colour at 135 columns, 21,700 lines/s.
- **Step 5, the tabs behind (2026-11-10).** Fixed: when the window's or
  the cells' size changes, every tab's terminals are fitted, not only
  the ones drawn. The six floods now all report the window's size.
- **Step 5, the interface thread again (2026-11-10): met.** Two things,
  and the second was the larger by far.
  - **A grid of cells in the scene** (`Command::Cells`, opcode 8 on the
    wire; the version stays 1, nothing outside this workspace reads it
    yet): origin, cell size, baseline, font size, columns, and sixteen
    bytes a cell (ground, ink, glyph, font, flags for an underline and
    for "has a glyph": glyph 0 is a font's "no such character" box and
    is drawn). Validated and fuzzed like the rest. The CPU renderer makes
    of it exactly the fills and glyph runs it stands for (a test holds
    the two pictures equal to the byte at four scales); the GPU renderer
    goes from the cells to instances with no shapes kept between, each
    glyph's place in the atlas remembered for the grid (a test holds it
    equal to the byte to the old way, with and without clips, and to
    the reference within three steps: translucent ink is one more
    rounding than opaque text's two). `vterm` draws a terminal's screen
    as one grid; combining marks, the selection and the cursor go over
    it as before. Screenshots of a screen of styled text before and
    after are the same file.
  - **The window was being woken for every piece of output read**: some
    25,000 times a second in a flood, each a trip through the kernel
    and the event loop, which is where most of the thread's time went
    (in the kernel, where the profile of user time did not show it).
    "Draw" now stays up from the first output after a frame until that
    terminal is next drawn, and the parsing thread wakes the window only
    for news it has not been told: once a frame.
  Measured as before (one tab, the colour flood at 135 x 38 cells and
  about 21,800 lines/s, 30 Hz, CPU time by the frame; the absolute
  figures move with what else the machine's GPU is doing, so builds are
  compared run beside run): 17ea6fd 8.0 ms a frame (24% of a core);
  01899b5 6.0 ms (18%); with the grid 4.2 ms (12.5%); with the wakes
  4.2 -> **0.83 ms a frame, 2.5% of a core**. The cursor test shown:
  1.7%. Acceptance 5's "under 10%" is met with room for a 60 Hz monitor.
- **Step 5, six tabs at the window's real size (2026-11-10):** 135 x 38
  cells in every tab, btop shown, a video in Firefox beside it:
  `seq 1 1000000` 0.84 to 0.86 s (under 1 s: met; the slowest single
  run 1.2 s); colour 7,500 to 7,700 lines/s and cursor 6,600 batches/s,
  both **under acceptance 5's 10,000**; Unicode 143,000 to 151,000
  lines/s; the interface thread under half a percent; 220 MB. Each
  flood alone passes several times over (colour 21,800 lines/s, cursor
  19,400 batches/s); four at once on six cores do not, with the kernel
  carrying their bytes: the machine's limit, not the terminal's, and a
  question for the acceptance figures.
  Still to do in step 5: an undisturbed soak, the tab-click test, the
  runs on canary and Techbench, the install.

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
