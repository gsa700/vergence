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
Smithay's smallvil example": `main.rs` (144 lines today), `state.rs` (417),
`winit.rs` (189), `handlers/mod.rs` (177), `handlers/compositor.rs` (64),
`handlers/xdg_shell.rs` (251). License: `LICENSE-smithay.txt`.

**Seat milestone 1 (2026-11-12) took out the input part:** `grabs/` (598
lines, the move and resize grabs) is deleted, replaced by our own in
`dispatch.rs`; `input.rs` (427 lines then, 267 now) is rewritten in our own
words and no longer carries the attribution; `xdg_shell.rs` lost its grab
code (311 to 251). The decisions themselves are crates/vergence-seat (no
Smithay in it). So 1,025 lines of smallvil's input code are gone or
rewritten; 1,242 lines of files still marked as smallvil's remain.

The server started from it on 2026-11-07 and these files have been rewritten
heavily since (energy, touch, the shelf, `Win`, keyboard repeat, placement),
so the counts are today's file sizes, not smallvil's share. Review: go
through each file, mark what is still smallvil's, and rewrite those parts in
our own words, so the attribution can go and the files read as one design.
Natural moment: native milestone 1 steps 3 to 5, which touch `state.rs`,
`input.rs`, the grabs and `xdg_shell.rs` anyway.

### SCOWL word lists (permissive, MIT-like), as data

Where: `data/words/en_US.txt` (123,679 words, about 1.2 MB), made by
`tools/make-words.py` from SCOWL 2020.12.07 (Kevin Atkinson's Spell
Checker Oriented Word Lists; sizes 10 to 60, American spelling, as
Hunspell's en_US); its terms in `data/words/SCOWL-COPYRIGHT.txt`, installed
beside it and printed in THIRD-PARTY-NOTICES.md. Dave, 2026-11-15: "ship
our own" (editor milestone 1, question 3). The checker reading it is ours
(crates/vergence-spell); only the words are borrowed.

Review: none needed for the code. Writing our own list of English words is
not a sensible project; other languages would come the same way, from
lists under terms like these.

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

Step 1 is done (seat milestone 1, 2026-11-12, docs/seat-milestone-1.md):
every pointer, keyboard and touch event goes through our seat; Smithay's
pointer, keyboard and touch are called only for Wayland and X11 surfaces;
the parallel native paths and the keyboard lock hazard are gone. Kept on
purpose (Dave, 2026-11-12): Smithay's drag and drop between Wayland
programs (the seat's Edge grab), and its keyboard as the Wayland edge, fed
every key beside our own xkb state.

**The xdg-dialog protocol (window styles milestone 1, Dave 2026-11-14).**
Smithay's `wayland::shell::xdg::dialog` (in the `wayland_frontend` feature,
no new dependency) answers GTK's and Qt's modal hint at the Wayland edge;
the server records the hint on the window and decides what modal does.
Goes with the rest of the Wayland front end when that is replaced.

### libinput, libseat/seatd, xkbcommon (through Smithay)

Input devices, seat and device access, keymaps. C libraries every Wayland
desktop uses. Review: keep unless one of them blocks us (FreeBSD support
included); xkbcommon's keymap handling is the most likely to stay for good.
Since seat milestone 1 the server also uses xkbcommon directly (the same
crate and version Smithay builds on) for its own keyboard state, keys.rs.

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

Decoding wallpapers and pictures, reading cursor themes. Files milestone 1
(Dave, 2026-11-15) turns on `image`'s GIF and WebP decoders (the `gif` and
`image-webp` crates, pure Rust, MIT / Apache-2.0) for Close-Up, run in the
decode helper; BMP is decoded by our own code. Review: the image
decoders read untrusted files, so they belong to the isolation work in
whitepaper §6.10 whichever way this goes; png and xcursor are small enough
to replace.

### toml, serde, bitflags, tracing, tracing-subscriber

The settings file format, its (de)serialization, flag types, logging.
Everyday Rust crates. Review: low priority; replacing them buys little.

### Adwaita cursor theme (artwork, read at run time)

The pointer. Not shipped by us. Review: draw our own cursors.

### udisks2 (Linux; a system daemon, talked to over D-Bus)

Mounts a drive plugged in, unmounts and powers it off on Eject (files
milestone 1; Dave's yes, 2026-11-15: "udisks2 on Linux and automounter on
kepler"). Not linked or shipped: vergence-media talks to it with our own
D-Bus client (vergence-dbus client.rs), and polkit decides what the
session may do. FreeBSD uses the base system's automounter instead
(autofs, /etc/autofs/special_vergence_media wrapping special_media, and our
setuid helper media-eject). Review: our own root helper that finds and
mounts drives on Linux too, if udisks ever stands in the way.

### Fonts and themes from the system

Read at run time (fontconfig names, the GTK theme name for X11 apps). Not
shipped by us.

## 3. Everything else

The rest of the 209 crates in THIRD-PARTY-NOTICES.md come in under the ones
above (mostly Smithay and the image decoders). They go when their parent
goes.
