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
- **Xwayland (2026-11-07):** rootless Xwayland started by the server
  (FreeBSD: no abstract socket; the first app starts when it is ready, so it
  inherits DISPLAY, :1 on Hubble beside Xorg's :0). X windows share the
  Space with Wayland windows, open centred on the monitor under the pointer,
  and take keyboard focus through an X11-aware focus target (X input focus /
  WM_TAKE_FOCUS, `focus.rs`); clipboard bridged. One client scale for all of
  Xwayland (VERGENCE_XWAYLAND_SCALE, default 1): X apps are the right size
  but upscaled (soft) on the 14" until per-monitor X scaling (next step).
  Xwayland renders in software (no wl_drm/dmabuf from the server yet).
  Alt+left drag moves and Alt+right drag resizes any window. Smithay gives
  Xwayland's client its own client data (XWaylandClientData): the
  compositor handler must look for both. Nested testing stalls while Xorg's
  VT is switched away (its EGL clients block); test on the real monitors.
  Dave: xterm and xclock run, Alt+drag works.
- **Title bars (2026-11-07):** drawn by the server (`decor.rs`) for X
  windows that do not decorate themselves and Wayland apps that ask for
  server-side decorations (xdg-decoration; foot does). 24 points tall, same
  physical height on every monitor, solid colours (sharp at any scale),
  edges rounded like window positions so bar and content meet without a
  seam; maximise and close buttons; drag to move, double-click or the button
  to maximise on the monitor the window is mostly on, drag a maximised
  window to restore it; invisible 6-point resize band with resize pointer
  shapes. The scene is built per window front to back, so a bar stacks with
  its window. No title text yet. Dave: "looks right".
- **Sharp X apps (2026-11-07):** Xwayland's client scale is the densest
  monitor's (3.2833 on Hubble; VERGENCE_XWAYLAND_SCALE=<n> overrides), so X
  apps draw at the 14"'s pixels and the server scales each X window down on
  the ASUS. X apps learn the density from XSETTINGS served by the server
  (`xsettings.rs`: the user's settings.toml, parser now shared in
  vergence-xsettings, with Gdk/WindowScalingFactor 3, Gdk/UnscaledDPI for
  exact text, Xft/DPI, Gtk/CursorThemeSize) and X resources (Xft.dpi 315,
  Xcursor.size 79, via xrdb). Two bugs found: Xwayland bound the outputs
  before its client scale was set, so it saw 1170 x 658 screens (fix: an empty
  output state change re-sends them in client pixels: 3840 / 5910 wide); and
  Avalonia MULTIPLIES AVALONIA_GLOBAL_SCALE_FACTOR with its Xft.dpi scale
  (10.77 = 3.28^2), so the server clears AVALONIA_*_SCALE_FACTOR(S).
  Measured from screenshots (Qt size box): 655 px = 52.9 mm on the 14", 212
  px = 52.4 mm on the ASUS. GTK 3 boxes are 9% small (integer window scale
  on X11). AlbumWall: right size, crisp on the 14", clean on the ASUS (small
  text slightly grainy: 3x linear minification; mipmaps later). Apps that
  ignore DPI (xclock, bitmap-font xterm) come out about 1/3.28 size. Open:
  2 px gap between the outputs in X pixels (3842 vs 3840); Avalonia uses
  315/96 = 3.281.
- **Control socket (2026-11-07):** `$XDG_RUNTIME_DIR/vergence-control`
  (0600 in the private runtime dir, peer uid checked: owner-only, no network)
  with `tools/vctl`: `shot` (PNG of each monitor as shown, rendered once more
  offscreen, to ~/vergence-shots/), `windows`, `spawn`, `move`, `focus`,
  `restart` (exit 75; `tools/run-server-drm.sh` loops on it), `quit`. Lets
  the session on the real monitors be seen and driven over SSH.
- **Title text (2026-11-07):** window titles in the server's title bars
  (`text.rs`): one line rasterised with ab_glyph (pure Rust) separately for
  each monitor at its own pixels and drawn 1:1, so it is sharp on both; the
  desktop font from Gtk/FontName in settings.toml (Sans 10 -> Noto Sans via
  fc-match); light on the focused window, grey otherwise; ellipsis before the
  buttons; re-rasterised only when title, width, focus or scale changes.
  Kerning, no shaping (Latin titles; real text stack later). Checked from
  screenshots on both monitors.
- **Placement (2026-11-07):** new windows open on the monitor under the
  pointer and fit it (`place.rs`): no larger than the monitor below a title
  bar (when the server draws one), centred; an X app that asks for a
  position (USPosition/PPosition) gets it if the window then lies wholly on a
  monitor. Wayland windows get configure_bounds up front and are fitted and
  centred on their first sized commit. Fixed: an X window's resize request
  was answered with the position X last reported, not where the server had
  just put it (AlbumWall snapped back to 0,0). AlbumWall (1131 x 710 saved)
  now opens at 1131 x 658 on the 14", all on screen.
