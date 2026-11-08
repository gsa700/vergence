# Slint vs iced on Hubble: results (2026-11-08)

Same window in both (README.md), FreeBSD 15.1, Intel UHD 630, in the live
Vergence session on the 14" 4K at scale 3. Slint 1.18.1 (femtovg/OpenGL
renderer, `unstable-winit-030` for the title-bar drag), iced 0.14.0.

| | Slint | iced |
|---|---|---|
| Builds on FreeBSD | yes | yes |
| Clean release build | 3 min 40 s | about 2 min 20 s |
| Binary (stripped) | 21.4 MiB | 13.0 MiB |
| Window up after start | ~200 ms | ~50 ms (OpenGL) |
| Memory (RSS) | 122 MiB | 136 MiB (OpenGL) |
| CPU when idle | 0% | 0% |
| Own title bar, server bar dropped | yes | yes |
| Drag the bar to move | yes (winit access) | yes (`window::drag`) |
| Sharp at scale 3 | yes | yes |
| Monospace text | the alias "Monospace" was not found: fell back to a proportional font (needs a real family name) | `Font::MONOSPACE` worked |
| Coloured text runs | one Text per run in a row | `rich_text` with spans |
| Split panes | hand-made (divider, a few lines) | **built in** (`pane_grid`: resizable, drag to rearrange) |
| Right-click menu | **built in** (`ContextMenuArea`: a real popup, separators, keyboard) | none: hand-made overlay inside the window |
| Default GPU path | worked | **Vulkan hangs on our server** (window never drawn); `WGPU_BACKEND=gl` or the software renderer work |
| Licence | GPLv3, or royalty-free with attribution, or commercial | MIT |
| What COSMIC Terminal uses | | iced (through libcosmic) |

## Findings for the server

- **Vulkan clients hang on vergence-server.** iced's default wgpu path on
  Vulkan (Mesa ANV) never presented a frame: the window stayed 0x0, no error.
  OpenGL and software rendering work. Likely what Vulkan's Wayland presentation
  needs from the server (explicit sync, linux-drm-syncobj, or dmabuf feedback
  details; whitepaper §8.3). Every Vulkan app (games, wgpu apps) will hit it.

## The choice

- **iced**: MIT (no licence question), splits built in, the foundation of
  the terminal Dave liked; costs a hand-made context menu, the Vulkan issue
  (OpenGL until the server is fixed), and porting the settings app later so
  the tools share one toolkit.
- **Slint**: already in the tree, native context menus, a little leaner in
  memory; the licence decides it (fine if Vergence is GPL, or with the
  attribution if it is permissive).
- Either way the terminal's character grid is drawn by our own code; the
  toolkit gives the window, tabs, menus, splits, input and clipboard.
