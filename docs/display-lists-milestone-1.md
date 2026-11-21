# Display lists milestone 1: windows sent as scenes, drawn sharp everywhere

Status: **draft 2026-11-20**, for Dave (Dave: "do the Super+Tab fix then
draft display lists"). The core-vision milestone due after Reader (an app
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

## Questions for Dave

1. **Which apps first?** (a) all of the toolkit's apps at once, with the
   pixel road kept for any window and forced by a switch (recommended:
   they all build scenes already, so one change covers them, and the
   checks compare both roads); (b) one app first (Edit or the terminal),
   the rest in a second milestone.
2. **Where scenes are drawn.** (a) in the server, on its GPU, with
   vergence-render (recommended: our own renderer, no borrowing, the same
   code the apps use now, so the pictures match); (b) in a confined helper
   process, as PDFs are read (safer against a hostile scene, a copy of
   every frame between processes, slower).
3. **Reader's pages.** A PDF page reaches Reader as a picture from its
   helper, so even in a scene it stays a picture: sharp in Reader (it asks
   for the page at the zoom it shows), enlarged in the magnifier. Leave it
   so in milestone 1 (recommended), or have the PDF helper hand over pages
   as scenes later (a large piece of work of its own)?
4. **The magnifier's level** of ambition in this milestone: (a) sharp for
   scene windows, enlarged pixels for the rest (recommended); (b) leave
   the magnifier as it is and do only the scales (straddling windows).

## Progress

(none yet)
