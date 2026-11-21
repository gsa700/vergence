# Display lists milestone 1: windows sent as scenes, drawn sharp everywhere

Status: **questions answered 2026-11-20** (all four as recommended: every
app at once, drawn in the server, Reader's pages stay pictures, the
magnifier sharp for our apps); drafted the same evening (Dave: "do the
Super+Tab fix then draft display lists"). The core-vision milestone due after Reader (an app
milestone) by the rule of 15 November; whitepaper §6.6, the NeXTSTEP idea
in the name.

## Why now

Every Vergence app already draws its window as a scene: vergence-scene's
display list (fills, strokes, lines, glyph runs, pictures, clips, a
terminal's cells), in reference units, with no scale of its own, validated
wherever it is read, with a wire format (wire.rs). But the app then renders
it to pixels itself and sends the server a picture at one scale. So:

- **The magnifier is fuzzy** (Dave, tonight: "works, a little fuzzy"): it
  can only enlarge pixels.
- **A window across two screens of different scales** is drawn at one
  scale and stretched on the other.
- The server can draw a window again only by asking the app.

If the server receives the scene instead, it can draw the window at any
density, at any time: each screen at its own scale, the magnifier at the
zoomed scale (text at 4× is text again), the overview and the dock's
pictures at their size. Bitmaps stay: always allowed, and what the other
programs (Wayland, X11) give.

## What is there to build on

- vergence-scene: the format, its limits and `validate`, `encode`/`decode`.
- vergence-render: the GPU renderer (its own context, rendering to a GPU
  buffer of the kind native windows already hand the server) and the CPU
  reference renderer (tests, GPU-less machines).
- The native window protocol: pictures as GPU buffers or memory (native.rs
  `Content`), the meaning tree beside them, a window's scale from the
  server (`Configure`).
- The display suite: headless screens, pixel comparisons
  (`vergence-harness same`, `zoomed`).

## The steps

1. **Scenes on the wire (bus v30).** `WinRequest::Scene { window, serial }`
   with the encoded scene in a sealed memfd (as large pictures go now);
   pictures in a scene's image table sent once and kept by id until the
   window drops them (a page of a PDF, an icon), so a new frame names them
   instead of sending them again. The server decodes and validates every
   scene (wire.rs never returns an invalid one); limits per window (bytes,
   commands, glyphs, images) as every bus message has.
2. **The server draws scenes.** A native window's content is a scene or a
   picture (`Content::Scene`). The server renders a scene with
   vergence-render's GPU renderer into a buffer at the scale it is shown
   at, keeps it while the scene and the scale stay, and shows it as it
   shows a picture now (rounded corners, the frost under the bar, the
   overview, the dock's pictures). Only what changed is drawn again: the
   scenes compared command by command, the damage from the commands that
   differ. Fonts: the server finds a scene's fonts by family, weight and
   slant as the app does (vergence-text), the same files on the same
   machine.
3. **The toolkit sends scenes.** vergence-ui's native road sends the scene
   it builds instead of rendering it, when the server says it draws scenes
   (`Configure`); otherwise pixels as now. An app may still send pixels
   (a video, a game). `VERGENCE_UI_PIXELS=1` forces pixels, for comparing.
4. **Each screen at its own scale.** A window across two screens is drawn
   twice, at each screen's scale, each half sharp; the same for its
   picture in the overview and Alt+Tab.
5. **The magnifier, sharp.** Zoomed, a scene window is drawn from its scene
   at the screen's scale times the level, the part in sight only (tiles,
   as Reader draws deep zooms); other windows are enlarged as now.
6. **Safety and cost.** A scene is untrusted: decoding and validation
   bounded (already); drawing bounded too: a time budget per frame per
   window, a scene that blows it drawn at a lower density (or its last
   good frame kept), and the app told; fuzz tests through the server's
   path; memory for kept buffers and images bounded per window.
7. **Checks.** A display-suite scenario: a window drawn from its scene and
   from its pixels identical at every clean scale (`same`), sharp halves
   across two headless screens of different scales, the magnifier at 3×
   sharper than enlarged pixels (edges measured); timing on kepler (the
   terminal scrolling a large window, Edit typing) against today's; tests
   on all four.

## Not in milestone 1

Remote display (scenes to another machine: phase 4); Avalonia's backend
(§8.1); scenes from Wayland programs; gradients, paths and layers in the
format (added when a tool needs them); the server's own panels (dock,
menus) as scenes (they are drawn in the server already, at each scale).

## Questions for Dave (answered 2026-11-20)

1. *Answered: (a) all at once.* **Which apps first?** (a) all of the toolkit's apps at once, with the
   pixel road kept for any window and forced by a switch (recommended:
   they all build scenes already, so one change covers them, and the
   checks compare both roads); (b) one app first (Edit or the terminal),
   the rest in a second milestone.
2. *Answered: (a) in the server.* **Where scenes are drawn.** (a) in the server, on its GPU, with
   vergence-render (recommended: our own renderer, no borrowing, the same
   code the apps use now, so the pictures match); (b) in a confined helper
   process, as PDFs are read (safer against a hostile scene, a copy of
   every frame between processes, slower).
3. *Answered: leave them pictures.* **Reader's pages.** A PDF page reaches Reader as a picture from its
   helper, so even in a scene it stays a picture: sharp in Reader (it asks
   for the page at the zoom it shows), enlarged in the magnifier. Leave it
   so in milestone 1 (recommended), or have the PDF helper hand over pages
   as scenes later (a large piece of work of its own)?
4. *Answered: (a) sharp for scene windows.* **The magnifier's level** of ambition in this milestone: (a) sharp for
   scene windows, enlarged pixels for the rest (recommended); (b) leave
   the magnifier as it is and do only the scales (straddling windows).

## Progress

- **Step 1, scenes on the wire** (2026-11-20, bus v30). vergence-scene's
  keyed wire form, version 2 (`Scene::encode_keyed`, `decode_keyed`, the
  `Keyed` result): each picture under a key the sender chose, sent whole
  once (the reader keeps it) and then named by key alone; a key never sent
  is refused (`DecodeError::UnknownImage`); version 1 stays for files.
  `WinRequest::Scene { window, fd, len, forget }`: the scene in a memfd
  (read at once, at most 300 MB), keys to forget (at most 4096). The
  server reads it, resolves its pictures from those it keeps for the
  window (at most 512 MB), decodes and validates it (vergence-scene never
  returns an invalid scene) and keeps the latest per window; a refused
  scene is logged and counted, and the window keeps what it showed.
  Nothing is drawn from scenes yet (step 2). `vctl windows` reports each
  window's scenes ("scenes 210, 3 commands, 480x320, 0 pictures kept").
  vnative `--gpu --scene` sends its scene after each picture. Tests: keyed
  round trip, pictures named by key, unknown keys refused, versions kept
  apart, 20,000 damaged keyed scenes decoded without a panic; the
  protocol's round trip with the new message.
- **Step 2, the server draws scenes** (2026-11-20). The server's scenes.rs:
  a window's scene drawn with vergence-render's GPU renderer (its own
  context, the apps' code) at the scale the window is shown at, into one
  of three GPU buffers the window takes in turn; the buffer then becomes
  the window's picture through the path an app's GPU picture takes
  (native.rs `native_attach_gpu`), so the bar's frost, rounded corners, the
  overview and the dock's pictures show it unchanged; the first picture,
  attached or drawn, puts the window on the screen (`native_picture_came`,
  shared). No GPU renderer (canary) or a failure: the CPU reference
  renderer into memory (`VERGENCE_SCENES_CPU=1` forces it, for tests). A
  scene brings its own ground (the toolkit's first fill, step 3) and is
  drawn on clear. Not yet: drawing only what changed (the whole scene is
  drawn each time, as the apps do now). `vctl scenes` says how scenes are
  drawn and how many. vnative `--gpu --scene` now sends only its scene.
  Tried headless on Hubble: a window drawn from its scene beside one
  drawn by its program, alike; on the GPU (206 scenes) and forced to the
  CPU (158).
- **Step 3, the toolkit sends scenes** (2026-11-20). vergence-ui's native
  road sends each frame's scene (`send_scene`) instead of drawing it: the
  app's ground added as the scene's first fill, each picture under a key
  (by where it is in memory, a Weak telling a freed one from a new one),
  sent whole only the first time, keys the frame no longer shows
  forgotten; a new memfd each frame. The server and the apps always speak
  one bus version, so a v30 app knows the server draws scenes: no new
  handshake. `VERGENCE_UI_PIXELS=1` keeps a program on pixels (for
  comparing, or a program that wants its pixels shown as drawn). Tried
  headless on Hubble: Calculator and Settings drawn by the server from
  their scenes against the same apps drawing their own pixels, **0 of
  201,600 and 0 of 865,200 pixels different** (largest difference 0);
  the terminal (6 commands a frame: its grid is one Cells command), Reader
  (one page picture kept at a time, 4.9 MB, earlier pages let go), Edit.
  Tests on all four.
- **Step 4, each screen at its own scale** (2026-11-20). A window's
  picture is drawn at its own screen's scale; on any other screen of
  another scale it overlaps, a window with a scene is drawn again from it
  at that screen's scale (scenes.rs `scene_alt`: kept by window and scale
  until the scene changes, two GPU buffers in turn, imported as a texture,
  its own serials for the rounded corners' shader), and decor.rs draws
  that half with it, so both halves are sharp. Drawn on the CPU (no GPU),
  the other screen shows the picture stretched as before. `vctl scenes`
  counts the windows drawn again. Tried headless on Hubble: Calculator
  across a 1× and a 2× screen, its 2× half drawn from the scene (crisp)
  against the same app on pixels (its 1× picture stretched, soft).
- **Step 5, the magnifier sharp** (2026-11-20). Once the zoom has settled
  (zoom.rs `zoom_sharp`: on, not moving, the point on this screen), a
  window with a scene is drawn from it at the screen's density times the
  level (scenes.rs `scene_alt`, step 4's), and decor.rs puts all of that
  picture into the window's body: the magnifier then enlarges a picture
  that already has that many pixels, so text is drawn sharp, not enlarged.
  While the zoom moves, pictures are enlarged as before (a new large
  picture every frame would cost too much). Windows without scenes (other
  programs) are enlarged as before; past 8192 pixels a side (or 40 million
  in all) a window is enlarged too (tiles later). Tried headless on
  Hubble: Calculator magnified 3.1×, drawn from its scene (crisp digits
  and borders) against the same app on pixels (soft).
- **Step 6, safety and cost** (2026-11-20, bus v31). Before a scene is
  drawn its cost is estimated (vergence-render `cost`: pixels touched, each
  command's area clipped to the window, glyphs and cells each by their
  size, pictures by their pixels too); over 400 M (about fifty 4K screens
  filled; real windows are a few) it is not drawn. A draw over 100 ms is a
  strike; three, or one over 500 ms, the same. Either way the window's
  scenes are switched off: the app is told (`WinEvent::ScenesOff`) and the
  toolkit draws that window's pixels from then on, so a hostile or broken
  scene costs the server one bounded draw at most; what the server kept
  for its scenes is let go. Copies for other scales: the same budget, and a
  slow one stops them for the window (enlarged as before). Memory: GPU
  buffers are now given back to the renderer when a window resizes or goes
  (steps 2-5 dropped them without that, leaking GPU memory on every
  resize); copies for other scales at most two per window, let go after
  10 s unused, 1 GB in all (step 5 kept two buffers for every zoom level
  settled at, for the window's life). Tests: cost (a window costs its
  pixels, a flood of large glyphs is costly, odd numbers never panic);
  20,000 damaged keyed scenes through the server's own intake
  (`take_keyed`) and cost, the pictures kept counted right and bounded.
  `VERGENCE_SCENES_BUDGET` lowers the budget for tests. Tried headless on
  Hubble: a budget of 1,000 switches Calculator to its own pixels (shown,
  answering keys); the real budget at 4K 2x with five apps (a maximized
  terminal full of text among them), none switched; magnified at five
  levels, two copies kept (94 MB), none 11 s after the zoom went off. Not
  done: drawing at a lower density instead of switching to pixels (pixels
  are simpler and the app has the scene anyway).
- **Step 7, checks** (2026-11-20). Display-suite scenario 13-scenes (the
  rich test pattern, drawn by the server or, with VERGENCE_UI_PIXELS=1,
  by itself): drawn from its scene, the same picture its app draws, at 1x
  and 2x (0 of 240,000 and 0 of 960,000 pixels different); across a 1x
  and a 2x screen, its 2x half drawn again and sharper than its pixels
  stretched (1.90x); magnified at 3.12x, sharper than its pixels enlarged
  (3.42x); no window switched to pixels. `vergence-harness sharper`
  measures it (the typical brightness step across an edge: larger when
  sharp). Passes on Hubble, kepler, techbench and apollo, the other 12
  with it. Timing on kepler (2560x1440, headless; per frame): the
  terminal maximized and scrolling, the app 0.29 ms building and sending
  its scene plus the server 2.4 ms drawing it, against the app 1.70 ms
  drawing its own pixels; Edit typing, 0.43 ms plus the server's ~2.3,
  against 2.81 ms. About as much work in all (the server's draw waits for
  the GPU to finish), moved from the app to the server; far inside a
  frame. To follow: the server draws on its own thread, or only what
  changed (step 2's hope), so a window's draw never holds up the others.
