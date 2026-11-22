# Hymie issues found while porting it to Rust (for Spindle / AlbumWall)

Found 2026-11-21 while porting Hymie (Spindle `src/Spindle.Codecs`, at
005f289) to Vergence's vergence-hymie (preview milestone 1). Dave: "take
notes on the Hymie issues and we can address them after we finish the
current work." Kept here, in Vergence's private repository, because Spindle
is public and item 1 is a memory-safety issue: fix first, describe after.

Each is fixed (or guarded) in the Rust port already; none is fixed in Spindle.

## FLAC

1. **A seek can play a frame twice** (FlacTrack.cs `SeekNow`). When the
   skip after the binary search exactly uses up a decoded frame (`skip -=
   _frameLength` reaching 0), `_frameOffset` stays 0 with that frame
   decoded, so it is handed out again. Seen at a seek to the end (32 frames
   came back instead of none); in the middle wherever the search lands just
   short of a frame boundary. Fix: after `skip -= _frameLength`, set
   `_frameOffset = _frameLength`. (Rust: flac.rs `seek`, 841d735.)

## MP3 (found by reading; none shown with a crafted file)

1. **Possible out-of-bounds read in `L3Huffman`** (Mp3.cs, about line 409).
   The big-values loop decodes `big_values` pairs (up to 288) without
   checking `part2_3_length`; with a linbits table it can use about 47 bits
   a pair, about 1.7 KB past where it starts, while `_maindata` (line 84)
   has 64 bytes of slack. In minimp3's C the overrun lands in the same
   struct's next fields (harmless); in C# the buffer is its own pinned
   array, so a crafted frame near the end of the main data reads other heap
   memory. Fix: about 2 KB of slack, or a bounds-checked refill. (Rust:
   reads past the end give zeros, 908f3f2.)
2. **`L3SaveReservoir`** (about line 795): a negative `remains` would become
   `_reserv` and the next `Buffer.MemoryCopy` (line 813) would throw.
   Probably unreachable (the bit position stays at or under the limit).
   Rust clamps it to 0.
3. **The length estimate is never corrected** (Mp3Track.cs line 41,
   `_estimate`; lines 101 and 112): the comment says the end of the data
   corrects it, but nothing updates it after the constructor, so a file
   without an Info tag keeps the bitrate estimate as its length.
4. **Seeking in free-format files** (`IndexTo`, about line 366):
   `_scanFreeFormat` is still 0, so `HdrFrameBytes` gives 0, indexing fails
   and the seek decodes on from the last known frame. Correct, but slow.
5. **Intensity stereo ignores the channel mode** (about line 833, from
   minimp3): a plain-stereo frame, or a damaged mono one, with the
   mode-extension bit set gets intensity processing (the mono case reads the
   next `gr_info`). FFmpeg ignores mode_ext outside joint stereo. The Rust
   port keeps minimp3's behavior, to stay bit-alike.
6. **`LYRICS200`** (about line 324) reads `end - 15` with only `end - 9`
   checked. Harmless in practice (a file with a frame is longer); Rust
   guards it.

## Already known (Marginal's security review, 2026-10-31)

- A1: deeply nested .m4a boxes overflow the stack in `Mp4Track.Walk` (and
  TagLib# 2.3.0). To be fixed in the Rust port's MP4 reader with a nesting
  limit.
