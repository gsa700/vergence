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
     two instances a cell; 3792 x 2118 pixels is about 30,000 cells, which
     is small work for any GPU. Only changed rows are sent again.
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
   a full terminal screen of coloured cells at 3792 x 2118. The font
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

## Acceptance

Every step: `cargo test --workspace` green on Hubble, canary and Techbench;
no new warnings.

1. The CPU renderer draws the full-screen cell scene (about 30,000 cells,
   every one its own colours) in under 30 ms on Hubble using all six
   cores, and a one-row change in under 2 ms. Golden images match on all
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
