# Code we did not write: to review, and perhaps rewrite

Dave, 2026-11-11: "make note to revisit any code we didn't write for later
review and possible re-write." This is that list. The rule for adding to it
is the borrowing rule (CLAUDE.md: nothing borrowed without asking him); the
policy record is whitepaper §8's table. Licenses are in
THIRD-PARTY-NOTICES.md (tools/notices.py).

Each entry: what it is, where, what we use it for, and what a review should
decide. Nothing here is scheduled; it is the list to work from when there is
time, or when one of them gets in the way.

## 1. Copied into our tree

### Smithay's smallvil example (MIT)

Where: `crates/vergence-server`, the files whose first line says "Based on
Smithay's smallvil example": `main.rs` (141 lines today), `input.rs` (427),
`state.rs` (423), `winit.rs` (184), `handlers/compositor.rs` (64),
`handlers/xdg_shell.rs` (311), `grabs/mod.rs` (6), `grabs/move_grab.rs`
(193), `grabs/resize_grab.rs` (399). License: `LICENSE-smithay.txt`.

The server started from it on 2026-11-07 and these files have been rewritten
heavily since (energy, touch, the shelf, `Win`, keyboard repeat, placement),
so the counts are today's file sizes, not smallvil's share. Review: go
through each file, mark what is still smallvil's, and rewrite those parts in
our own words, so the attribution can go and the files read as one design.
Natural moment: native milestone 1 steps 3 to 5, which touch `state.rs`,
`input.rs`, the grabs and `xdg_shell.rs` anyway.

### drm-rs 0.14.1 (MIT), patched

Where: `third_party/drm-rs` (about 4,700 lines of Rust), our patch in
`patches/drm-rs-freebsd-node.patch` (see `patches/README.md`). Smithay's DRM
crate; we carry it for the FreeBSD node fix, and the fix is meant to go
upstream (Dave's account).

Review: once upstream has the fix, drop our copy and depend on the release.
Rewriting drm-rs belongs with the decision about Smithay's DRM back end
below, not on its own.

## 2. Depended on (whitepaper §8)

### Smithay (MIT), git rev 65aa86f

The server's Wayland front end, DRM/KMS and input back ends, the renderer.
By far the largest borrowed part. Decided 2026-11-08: moving to a support
role. Step 2 of native milestone 1 (`Win`, 2026-11-10) already put our own
window element in front of Smithay's. Review per piece: the native protocol
replaces its Wayland-facing parts for our own tools; the renderer is a
candidate for vergence-render; the DRM and input back ends are the last and
hardest (they wrap the OS interfaces, which are not borrowing).

**What the native milestone showed (2026-11-11, steps 2 to 4).** Smithay
assumes every window is a Wayland or X11 window, and each native feature
went around it rather than through it: its `Window` knows only those two
(hence `Win`); pointer and touch focus are typed as `WlSurface` (native
windows run on a parallel pointer and touch path); the keyboard focus
callback holds the keyboard's lock (the server froze when our code read
the keymap there); useful pieces are private (the cursor-shape table was
rewritten). Dave: "we spend enough time working around other people's code
pretty soon it's easier to do our own. As we saw on Deadwax and
AlbumWall." The seams are where our own code already stands beside it:
`Win`, the native pointer focus, `ClipOwner`.

**Order of replacement, after native milestone 1 (agreed 2026-11-11):**

1. **The seat**: our own input dispatch, focus and grabs, with Wayland
   surfaces one kind of target among others. Removes the parallel pointer
   and touch paths and the keyboard lock hazard.
2. **The window space and the renderer**: the server draws with
   vergence-render, as the toolkit does; `Win` becomes the server's own
   element without Smithay's `Window` behind every case.
3. **The Wayland front end**: kept as long as other people's applications
   speak Wayland. It is the part Smithay does best and was always meant to
   be an edge.
4. **The DRM/KMS and input back ends**, last: thin wrappers over the OS's
   interfaces, which are not borrowing (the drm-rs patch goes with them).

### libinput, libseat/seatd, xkbcommon (through Smithay)

Input devices, seat and device access, keymaps. C libraries every Wayland
desktop uses. Review: keep unless one of them blocks us (FreeBSD support
included); xkbcommon's keymap handling is the most likely to stay for good.

### Xwayland

X11 applications, off by default since 2026-11-10. Review: keep as the only
X11 route, or drop X11 entirely; an X server of our own is not planned.

### ab_glyph, owned_ttf_parser, ab_glyph_rasterizer (Apache-2.0)

Glyph outlines and rasterizing, in the server's title bars and in
vergence-text. Small. Review: our own font reading and rasterizer is a
contained project; text shaping is a separate, larger question (whitepaper
§10, item 8).

### resvg (Apache-2.0 OR MIT)

SVG wallpapers and icons. Review: keep for SVG (a full SVG renderer is
large), or limit what we accept and draw it with vergence-render.

### image, png, xcursor (MIT / Apache-2.0)

Decoding wallpapers and pictures, reading cursor themes. Review: the image
decoders read untrusted files, so they belong to the isolation work in
whitepaper §6.10 whichever way this goes; png and xcursor are small enough
to replace.

### toml, serde, bitflags, tracing, tracing-subscriber

The settings file format, its (de)serialization, flag types, logging.
Everyday Rust crates. Review: low priority; replacing them buys little.

### Adwaita cursor theme (artwork, read at run time)

The pointer. Not shipped by us. Review: draw our own cursors.

### Fonts and themes from the system

Read at run time (fontconfig names, the GTK theme name for X11 apps). Not
shipped by us.

## 3. Everything else

The rest of the 209 crates in THIRD-PARTY-NOTICES.md come in under the ones
above (mostly Smithay and the image decoders). They go when their parent
goes.
