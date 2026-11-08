# Server milestone 1: the server, nested (phase 2, stage 1)

Started 2026-11-07 (Dave: "focus on getting the full stack running so we can
see it and work on it as a whole as soon as possible"). Phase 1 (vergd, X11)
is frozen except blockers; its open items stay in milestone-4.md. Mode B is
the design (decision 2026-11-07): every app draws natively at each monitor's
calculated scale; the server, not X, owns the screens and the input.

## Goal

`vergence-server`, a Wayland server built on Smithay, running **nested** in a
window on Hubble's Xfce desktop, showing Wayland apps at the right physical
size on **two outputs with different scales**, with windows crossing between
them **smoothly**: each part of a window drawn at the scale of the output it
is on, so it is the same physical size all the way across, and the app told
to switch scale once most of it has crossed.

## Step 0 (done, 2026-11-07)

Smithay (main at 65aa86f, 0.7.0) builds on Hubble (FreeBSD 15.1, Rust 1.98):
`anvil --winit` runs nested in Xfce, renders with GLES2 through Mesa EGL on
the UHD 630, and serves foot as a Wayland client. Xfce sets
`XDG_RUNTIME_DIR=/var/run/xdg/<user>`. Installed: foot, xwayland, seatd.

## Stages

- **1a, skeleton.** `crates/vergence-server` from Smithay's `smallvil`
  example (MIT, attribution kept): one nested output, xdg-shell windows,
  move and resize, keyboard and pointer. Smithay pinned to 65aa86f.
- **1b, fractional scale.** The output's scale is a fraction (wp-fractional-
  scale-v1 and wp-viewporter), so foot and GTK 4 draw at, say, 1.64 natively.
- **1c, two outputs.** One nested window shows two outputs side by side with
  the scales of Hubble's real monitors, derived from their EDIDs through
  `vergence-edid` and `vergence-profile`: ASUS VS229 1.07, the 14" 4K at
  1920x1080 1.64. (On the real hardware in stage 2 each output is its own
  panel; nested, they are two halves of one window, each drawn at its own
  physical size relative to the host monitor.)
- **1d, crossing.** A window straddling the two outputs is drawn on each at
  that output's scale (the bitmap resampled for the part on the output it was
  not drawn for); its preferred scale switches when most of it has crossed;
  input lands where it is shown.

## Acceptance

1. `vergence-server` builds on Hubble with no warnings and runs nested;
   foot and a GTK 4 app open in it, take keyboard and pointer input, move
   and resize.
2. With an output scale of 1.64 the clients draw at 1.64 natively (foot
   reports fractional scale; no blur from upscaling).
3. Two outputs at 1.07 and 1.64 side by side; a 200 x 200 logical box is the
   same size on both, relative to each output's physical size (measured).
4. A window dragged from one output to the other keeps its physical size the
   whole way, with no jump when its preferred scale switches; a click on
   either part reaches the right point of the app.
5. Nothing listens on the network; the Wayland socket is in the user's
   private runtime directory (whitepaper §7).

## Not in this milestone

DRM/KMS on the real monitors, libinput and seats (stage 2); Xwayland for X11
apps; window placement memory (Contract 2, moved over from vergd);
accessibility (Contract 3); display-list surfaces.

## Progress

- **1a, 1b done (2026-11-07):** foot draws natively at 1.64 (preferred_scale
  197, a 1144 x 775 buffer for about 697 x 472 points through wp-viewporter).
- **1c done (2026-11-07):** two virtual outputs side by side in the nested
  window, scales derived through vergence-profile from the panels' EDID
  sizes ("14in 4K" 1920x1080, 310 x 174 mm: 1.64; "ASUS VS229" 1920x1080,
  475 x 267 mm: 1.07; `VERGENCE_OUTPUTS` overrides). Each output renders into
  a texture of its own at its own scale; the textures are composed side by
  side. Each window gets the preferred scale of the output most of it is on,
  every frame; the pointer maps through the output it is over. Dave on
  Hubble: "it is as it should be" (two halves, the right way up, foot larger
  on the 1.64 side, switching as it crosses, pointer accurate on both).
- **1d:** Smithay draws a straddling window on both outputs, each at its own
  scale. Nested, both outputs are on one host monitor, so "the same physical
  size the whole way" is proved on the real panels in stage 2.
- **Stage 2, first light (2026-11-07):** `vergence-server --drm` from a text
  console on Hubble (libseat/seatd, udev, libinput; one GPU, GBM + EGL, one
  `DrmOutputManager`), adapted from anvil's backend. Each monitor's EDID is
  read from DRM, parsed by vergence-edid and derived by vergence-profile:
  HDMI-A-2 the 14" at native 3840x2160 -> 3.28 (left), HDMI-A-1 the ASUS
  1920x1080 -> 1.07 (right; order from `VERGENCE_LEFT_TO_RIGHT`). Needs the
  vendored drm-rs FreeBSD fix (patches/README.md); smithay-drm-extras without
  libdisplay-info. Scales are rounded to 120ths, the unit wp-fractional-scale-v1
  gives apps, so outputs match what apps draw at: the ASUS at its exact 1.0703
  was soft (apps drew at 128/120), at 128/120 it is sharp. Dave: "looks really
  good", text size right on both, window and mouse as expected, "snappy";
  after the 120ths fix, the ASUS "sharp and clear as it should be".
  Ctrl+Alt+Backspace quits, Ctrl+Alt+Fn switches consoles. Launcher on Hubble:
  ~/run-vergence-drm.sh.

- **Pointer image (2026-11-07):** themed shapes in place of the white square
  (`cursor.rs`). XCURSOR_SIZE (default 24) is in points; XCURSOR_THEME
  defaults to Adwaita. Apps name shapes with wp-cursor-shape-v1 (foot does);
  a client's own cursor surface is drawn at the output's scale with its
  hotspot; over the bare background it goes back to the arrow. Two findings:
  (1) themes draw different designs per size (Adwaita: a thin, hinted arrow
  at 24/32 px, a fat outlined one at 48-96 px), so picking the nearest image
  per monitor showed a different arrow on each. Each shape is now taken from
  the theme's largest image only, shrunk on the CPU by area averaging to
  exactly the output's pixels (79 px on the 14", 26 px on the ASUS) and drawn
  1:1. (2) Smithay's MemoryRenderBufferRenderElement crops the source to the
  logical size when only a size is given; pass the whole buffer as `src`.
  Dave: "looks right now, same on both".
