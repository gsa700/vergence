# Vergence Remote protocol, version 1

The stream between a viewer and a Vergence host (remote milestone 1,
docs/remote-milestone-1.md). The reference implementation is
`crates/vergence-remote` (proto.rs, lz.rs); this document is the contract
for anyone writing a viewer of their own.

## Transport

The viewer runs `ssh <host> vergence-remote`; the program's standard input
and output are the channel. SSH does the authentication, the encryption and
the host-key check; WireGuard, when used, is only the network path under
SSH. Nothing listens on the network.

## The stream

Each side first sends 8 bytes: `VRMT` and the version as a u32. A side that
reads another version ends the stream. Then messages, each:

| Field | Size | |
|---|---|---|
| type | u8 | per direction, below |
| length | u32 | the body's length; at most 128 MiB |
| body | length | the fields, below |

All numbers are big-endian. `str` is a u16 length and that many bytes of
UTF-8. `f32` is IEEE 754 and must be finite. A body must hold exactly its
fields: shorter or longer is an error, as is an unknown type.

## Viewer to host

| Type | Message | Fields |
|---|---|---|
| 1 | Hello | name str (shown on the host's Dockworker tile), width u32, height u32 (the viewer's pixels), screen str (empty: the host's main screen) |
| 2 | Resize | width u32, height u32 |
| 3 | Ack | seq u32: frame `seq` is shown; the host may send the next |
| 4 | Key | code u32 (Linux evdev key code), pressed u8 |
| 5 | Pointer | x f32, y f32, in the view's pixels |
| 6 | Button | code u32 (evdev: 0x110 left, 0x111 right, 0x112 middle), pressed u8 |
| 7 | Scroll | dx f32, dy f32 |
| 8 | Bye | (none) |

Widths and heights are 1 to 16,384.

## Host to viewer

| Type | Message | Fields |
|---|---|---|
| 1 | Welcome | host str, count u16, then each screen: name str, width u32, height u32 (logical) |
| 2 | Frame | seq u32, width u32, height u32 (the view's pixels), scale f32, count u32 (at most 4,096), then each rectangle: x, y, w, h u32, encoding u8, length u32, data |
| 3 | Notice | text str (to show the person: "Ended from the desk") |
| 4 | Bye | reason str |

## Frames

The host's server draws the screen at the viewer's size and scale (the view
fits the screen into the viewer's pixels; `scale` says how much), and sends
the rectangles that changed since the last frame the viewer acknowledged.
The viewer keeps one picture, RGBA, the view's size, starting as all zeros;
a frame of another size starts a new picture of zeros. The host sends only
pixels that differ from what it has sent, so a first frame is everything
that is not zero.

A frame is applied whole or not at all: every rectangle must lie inside the
view and decode to exactly `w * h * 4` bytes, or the frame is refused.

Flow: the host sends a frame, then waits for its Ack before sending the
next; what changes meanwhile is merged into that next frame. A slow viewer
gets fewer, larger frames, never a queue.

## Encodings

| Value | Encoding |
|---|---|
| 0 | Raw: `w * h * 4` bytes of RGBA, rows from the top |
| 1 | LZ: the same bytes compressed as one block in LZ4's block format |

The LZ4 block format: sequences, each a token byte (high four bits the
literals' count, low four the match length minus 4; 15 means more length
bytes follow, each added, a byte under 255 ending them), the literals, a
two-byte little-endian offset (1 to 65,535 back), and the match's extra
length bytes. The last sequence is literals only. A decoder must check
every length and offset and must produce exactly `w * h * 4` bytes.

## Changes

Version 1 (2026-11-23): the first.
