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
