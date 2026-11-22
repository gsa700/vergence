# Preview milestone 1: a window shaped for what it shows, and Hymie

Status: **planned 2026-11-21**, decided the same evening.
- Dave showed macOS Quick Look on Control: a text file, a picture, a
  cheat-sheet picture and a PDF, each in a window shaped for it, with "Open
  with ..." in the title bar.
- He asked what decodes and displays the data, and what happens with
  formats we have not built, such as sound.
- His decisions: "we already built all those decoders for albumwall; let's
  use Hymie!", then "separate GPL helper for AAC, keep Hymie, yes to all
  three". The third answer means "Open with" names the app chosen for the
  file's type.

This replaces FileView's Space card, whose text ran past the window (Dave,
2026-11-21: "Text flows past the window"), and the wish list's "Space
preview as a window of its own, with Open in...".

## What it does

1. **A window of its own.** Space in FileView, or on a desktop icon, opens
   the preview over the window it came from.
   - Its title bar shows the file's name, and on the right one button,
     "Open with <app>", for the app chosen for the file's type.
   - The arrow keys move FileView's selection, and the preview follows.
     Enter opens the file. Space or Escape closes the preview.
   - There is no facts panel and no hint line: just the thing itself.
2. **Shaped for what it shows**, within about 80% of the screen. When the
   shape changes, the window keeps its center.
   - **Picture:** fitted at its own proportions.
   - **Pages** (PDF): the pages at reading size, scrolling, with a strip of
     page thumbnails beside them.
   - **Text:** a fixed-width font, wrapped and scrolling, the whole file up
     to 1 MiB.
   - **Sound:** cover art, title, artist, album and length over a
     **waveform**, as GNOME's sound recorder and Audacity draw it (Dave,
     2026-11-21). The waveform is also the scrubber: a playhead moves across
     it, and a click seeks. Play/pause sits beside it.
     - The helper decodes the whole track once, which takes well under a
       second for a CD-length FLAC or MP3.
     - It sends back only a min/max peak per slice (stereo, about 2,000
       slices), never the samples, and the waveform fills in from left to
       right as decoding goes.
   - **Facts:** for anything else, a large icon with the name, kind, size
     and dates. For a folder, how many items it holds.
3. **What decodes the data.** The rule is whitepaper §6.10: a file's bytes
   are never parsed in the window's process. Confined helpers parse them, and
   the window gets back one of the results above.
   - Pictures go through `vergence-decode` and PDFs through `vergence-pdf`'s
     helper, as now.
   - Sound goes through Hymie's helper, and AAC through a helper of its own.
   - Text is only checked for UTF-8 and shown, so it is read in-process.
   - A table maps each type to its result and its helper. A new format means
     a new helper, and the window does not change. Later, other programs may
     add helpers under the same confinement, as Quick Look's plugins do.
4. **Hymie, AlbumWall's audio engine, ported to Rust.** The source is
   Spindle's `src/Spindle.Codecs`, about 5,700 lines of C#. The formats and
   where each came from:
   - **FLAC** and **WAV**: our own (the FLAC decoder is Deadwax's, written
     from RFC 9639). They come under Vergence's license, `Apache-2.0 OR
     GPL-2.0-or-later`, which Dave can do as the author.
   - **MP3**: ported from minimp3, which is CC0.
   - **ALAC**: ported from Apple's decoder, which is Apache-2.0.
   - **The MP4 container, the tags** (ID3v2, Vorbis comments, MP4 atoms)
     **and ReplayGain**: our own.
   - **AAC**: ported from FAAD2, which is GPL-2.0-or-later. It goes in its
     own crate and helper program, licensed GPL-2.0-or-later, so the rest
     stays dual-licensed.
   - The Marginal security review's finding A1 is fixed in the port: deeply
     nested .m4a boxes overflowed the stack, so the nesting gets a limit.
   - minimp3, Apple's ALAC and FAAD2 are listed in docs/borrowed-review.md,
     and their license texts are kept beside them.

## Steps

1. **Bus v35:** a text button in a window's bar (`BarIcon::Label`, which
   shows the button's name), drawn by the server on any bar, a dialog's
   included. A dialog that asks for a new size keeps its center over its
   parent. The wire form is fuzzed with the rest.
2. **The preview window** (crates/vergence-files/src/preview.rs): a
   non-modal Dialog that shares state with the window it came from (which
   file it shows, and the keys it passes back). It covers the title, the
   "Open with" button, its own size, and the desktop's Space.
3. **The results:** picture, pages (rendered at the window's scale, page by
   page as they come into view, with the thumbnail strip), text, facts, and
   the type table.
4. **Hymie:** crates/vergence-hymie, ported from Spindle at a named commit
   (WAV, FLAC, MP3, ALAC, MP4, tags, cover art, ReplayGain), and
   crates/vergence-hymie-aac (GPL-2.0-or-later). Tests decode fixtures and
   compare them sample for sample with Hymie's own output (Hymie was
   bit-identical to mpv, 2026-10-26).
5. **The sound helpers:** libexec/vergence/hymie, confined as
   vergence-decode is, decodes one file to PCM on a pipe; AAC goes through
   libexec/vergence/hymie-aac. Packaging and THIRD-PARTY-NOTICES name the
   GPL helper.
6. **Sound in the preview:** cover art, tags and length, the waveform (the
   helper's peaks, drawn as they arrive) as the scrubber, and play/pause,
   through vergence-sound. Playing stops when the preview closes or moves on
   to another file.
7. **Checks:**
   - Display-suite scenario 18: in a headless session, preview a text file,
     a picture, a PDF and a WAV, checking each window's shape and the "Open
     with" button, and that the arrow keys move on.
   - Hymie's tests on Hubble and techbench.
   - tools/agent-check.sh.
   - A pass by hand with Dave.

## Not in it

- Video, and its codecs.
- Editing in the preview (Quick Look's markup).
- Previews from other programs' helpers.
- AlbumWall itself moving to the Rust Hymie.

## Polish, later (Dave, 2026-11-21)

- **The preview window's decorations** (Dave: "We might want to change the
  window decorations but we can look at that as a polish item later"):
  today a dialog's slim bar with close, the file's name and "Open with".
