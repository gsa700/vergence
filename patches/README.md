# Patches carried by Vergence

## drm-rs 0.14.1: FreeBSD DRM node handling (`drm-rs-freebsd-node.patch`)

`third_party/drm-rs` is drm-rs 0.14.1 (crates.io, MIT, Smithay's DRM crate)
with this patch; the workspace uses it through `[patch.crates-io]`. Found on
Hubble (FreeBSD 15.1, Intel UHD 630) on 2026-11-07, when Smithay's `anvil`
crashed on the real monitors and sway did not:

1. `DrmNode::from_dev_id` took the node type from `minor(dev) >> 6`, Linux's
   numbering. FreeBSD does not encode it in the number (`/dev/drm/0` is 103,
   `/dev/drm/128` is 258), so the primary node looked like a control node and
   the GPU lookup failed. On FreeBSD the type now comes from the device name,
   `drm/N`, as libdrm does.
2. `dev_path`: `old_id & id_mask + ty.minor_base()` parses as
   `old_id & (id_mask + base)` in Rust, so the render node of a primary node
   came out as `renderD0`. Parenthesised (FreeBSD and the other-BSD variant).
3. Still open, cosmetic: the node's display name uses `minor()` ("renderD2").

With it, anvil drives both of Hubble's monitors. Upstream:
https://github.com/Smithay/drm-rs (a pull request is Dave's call). Drop the
vendored copy once a release has the fix.

## iced_tiny_skia 0.14.1: the clip mask rebuilt for every text (`iced-tiny-skia-clip-mask.patch`)

`third_party/iced_tiny_skia` is iced_tiny_skia 0.14.1 (crates.io, MIT, iced's
software renderer) with this patch; the workspace uses it through
`[patch.crates-io]`. Found on Hubble (Void, 14" 4K at scale 3) on 2026-11-09,
when the terminal with htop, btop and mc in its tabs held a whole CPU core
and crawled (Dave):

- A canvas's text is drawn with an infinite clip rectangle
  (`geometry.rs`, `fill_text`), so `Engine::draw_text` never finds it "within"
  the layer and calls `adjust_clip_mask` for **every** text: the window-sized
  mask is cleared and filled again (7 MB each way for a 3546 x 2034 pixel
  window), always with the bounds it already held. The terminal draws one
  text per run of same-coloured cells, hundreds to thousands a frame. `perf`
  on the running terminal: 78% of its time in that `memset`.
- The patch passes the bounds the mask holds into `draw_text` and rebuilds
  the mask only when a text needs different ones. What is drawn is unchanged.

Measured on Hubble with btop in a 652 x 454 point window at scale 3: 19.5% of
a core before, 8.2% after; the cost removed grows with the window's pixels
times its text runs, so large windows gain most. What is left is glyph
rasterising and blitting. The settings app uses the same renderer.

Upstream (https://github.com/iced-rs/iced, `tiny_skia/src/engine.rs`) still
had this on 2026-11-09; a pull request is Dave's call. Drop the vendored copy
once a release has a fix, or when the terminal draws its own cells.
