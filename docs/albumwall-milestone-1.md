# AlbumWall milestone 1: AlbumWall for Vergence

*Drafted 2026-11-25* from two inventories made that evening: what the
Avalonia AlbumWall does (0.9.7, ~19k lines of app plus ~3k of library code
and Spindle's ~10k of codecs and playback), and what Vergence already has to
build it from. Dave (2026-11-21): AlbumWall is rebuilt in Rust on the toolkit
and Hymie, for Vergence only for now; "it will be different but look and
function the same". The Avalonia AlbumWall stays as it is on the Mac,
Windows and the Pi kiosk.

**Answered 2026-11-25** (Dave: "all leans, go ahead"): milestone 1 a folder
library with everything seen and played, Navidrome and the rest in
milestone 2; the server's bar with the menu as its app menu, the volume in
the play bar; AlbumWall's own dark wall inside; our own index file, no
SQLite; `[albumwall]` in desktop.toml plus `~/.config/vergence/albumwall/`;
tags and covers parsed only in the helpers, scan speed measured early; the
VU pair only; Hubble's test music and the suite, then his library on
techbench. Building, step 1.

## Names (decided, 2026-11-21 and 2026-11-25)

- The program is `vergence-albumwall` (crate `crates/vergence-albumwall`).
- Its name everywhere a person reads it is **AlbumWall**. The desktop
  entry's generic name is "Music", so the launcher finds it by that word.
  About says "AlbumWall for Vergence".
- Its settings are its own, apart from the Avalonia AlbumWall's
  (`~/.config/albumwall`).

## What milestone 1 does

AlbumWall as you use it every day on a folder library, looking and working
as the Avalonia app does:

- **The library**: one or more music folders (local, external drive,
  network share). FLAC, MP3, ALAC/M4A, WAV, and AAC through the AAC helper.
  The same rules as today: an album is (album artist, album); the year from
  a "YYYY - " folder name, else the tag, the earliest across a box set's
  folders; tracks by disc and number; sorted by artist with "The/A/An"
  skipped, then year, then title. Covers: embedded art first, then
  cover/folder/front.jpg beside the tracks; back.jpg for the sleeve's back.
  An index kept as a cache (unchanged = same path, size and time), a rescan
  after changes settle, several libraries never merged, a picker in the
  bottom bar, the first-run welcome (choose a folder, or look in the usual
  places).
- **The wall**: covers aiming at 185 px, the column count chosen so a row
  fills the width (132 to 268 px), title and artist under each, never
  cropped, placeholders, art fading in; only the visible rows built.
- **The album unfolding in place** under its row: tinted from its art, the
  arrow pointing at the tile, the two-column track list with disc headings,
  the 260 px sleeve that is the play button, the tile turning over to its
  back, the wall centering it; the swap in place for the row below; Esc or
  another album folds it. The timings are lifted from MainWindow.axaml.
- **Playing**: gapless, at each file's own rate, with ReplayGain by album,
  by track or off (albums by album and shuffles by track until chosen), the
  app's own volume. What happens when an album ends: stop, the next album,
  a random album, shuffle all songs; the wall following what plays.
  Picking up where you left off, paused (or playing, if chosen).
- **The play bar**, floating and sliding in: cover (to the playing album),
  title and artist, previous / play-pause / next / play mode, elapsed, seek,
  remaining, and the VU meters with their spring ballistics.
- **Colors from the covers**: the wall's ground from the library's hue
  (Light, Tint, Bars), the album's panel and play bar from its art, every
  text color held to 4.5:1.
- **Search** in the bottom bar: live, over album artist and title, with
  `art:missing`, `art:small`, `art:nonsquare`; "N of M albums".
- **The keyboard** as today: arrows cover to cover, into the tracks, Enter,
  PgUp/PgDn, Home/End, Space, Ctrl+arrows next/previous (previous restarts
  past 3 s), Shift+arrows seek 10 s, Ctrl+Up/Down volume, Ctrl+L to the
  playing album, `/` or Ctrl+F to search, F11 full screen, the Esc chain.
- **Preferences** on Vergence's own preferences window: Library (libraries,
  add a folder, statistics), Playback, Appearance, Accessibility (text size,
  motion), Keys and search, About. The errors you see today (no sound
  output, a library that cannot be reached, "Reading your library").

## Later (planned, not in milestone 1)

- **Milestone 2**: Navidrome (an HTTP client with TLS, the Subsonic token
  sign-in, the cover and album list cache, streaming); lyrics (.lrc, tags,
  the server's; timed lines); MPRIS and the media keys (Play/Next/Previous
  in the server's keys, routed to the player); compact mode and the front
  panel view; the LED and 31-band meters; the equalizer; the Properties
  window; single instance and the command-line remote; the FLAC integrity
  check.
- **Not ported**: the self-updater and installer (Vergence's package does
  that), the Mac menu bar and Windows' media controls, the kiosk scripts
  (the Pi keeps the Avalonia AlbumWall), "Edit in Marginal" (Marginal is not
  a Vergence app yet).

## How it is built

What Vergence has, and what this milestone adds to it (each addition lives
where any app can use it, not inside AlbumWall):

- **Hymie** (crates/vergence-hymie) has the decoders with gapless trimming,
  exact seeking, tags and ReplayGain parsing. Added: the helper's answer
  carries album artist, track and disc numbers, year, duration and format
  (today only title, artist and album cross the pipe); a scan mode, so one
  confined helper reads a whole library's tags instead of one process per
  file.
- **Sound** (crates/vergence-sound) writes i16 and blocks to pace. Added:
  pause without closing the stream, flush, the position actually heard
  (latency), a stream name for PipeWire; dither from Hymie's f32.
- **The player engine**, a new crate or module beside the preview's
  listen.rs (which drops the stream on pause and plays one file): one
  long-lived stream fed track after track, the next one opened ahead,
  ReplayGain applied, resampled only when the device will not take the
  file's rate.
- **Pictures** (crates/vergence-decode) decode files in a confined helper.
  Added: an entry for bytes already in hand (embedded covers), and decoding
  to a size bucket (256, 384, 512 px) as AlbumWall does, with an in-memory
  cache of decoded covers.
- **The toolkit** (crates/vergence-widgets) has rows, scroll, pictures,
  sliders, fields, menus and drawing. Added: a **virtualized grid** (only
  the visible rows built and painted; FileView's icon view builds every
  item each time), and **timed transitions** for the unfold, the turn-over,
  the fades and the play bar's slide, honoring Reduce motion.
- **Preferences** (prefs.rs) has Choose, Switch, Button and Percent rows.
  Added: a folder row (the Open/Save dialog's folder picker) and a slider.
- **Watching folders**: there is no file watcher in the workspace yet.
  Added: a small one (inotify on Linux, kqueue on FreeBSD), polling for
  network shares as AlbumWall does every 15 minutes.
- **AlbumWall itself**: the library rules, the wall and album panel, the
  color code (Palette, Ground, Contrast ported), the play bar and meters,
  search, keys, session.

## Questions (with leans)

1. **The split**: milestone 1 a folder library and everything you see and
   play with it (above); Navidrome, lyrics, MPRIS and the media keys,
   compact mode, the extra meters and the equalizer in milestone 2? *Lean:
   yes. Milestone 1 needs no networking, so the TLS question waits for
   milestone 2.*
2. **The window's top bar**: AlbumWall draws its own (wordmark, volume,
   compact, the menu). On Vergence the server draws every window's bar.
   *Lean: the server's bar, titled AlbumWall, with the menu as the bar's app
   menu; the volume moves into the play bar; the wordmark goes to About.
   Inside the window everything looks as today.*
3. **The look inside the window**: AlbumWall's own dark wall with the
   ground taken from the covers, or Vergence's light/dark? *Lean: AlbumWall's
   own: the covers are the point. Preferences and other paperwork windows
   follow the desktop's light or dark, as every Vergence app's.*
4. **The index**: AlbumWall keeps an SQLite cache. *Lean: our own file (a
   plain table of path, size, time and the tags, rewritten when it changes),
   no SQLite: it is only a cache, and SQLite would be a borrowing ask.*
5. **Where its settings live**: *Lean: the look and playback choices in
   desktop.toml's `[albumwall]` (Vergence's preferences framework, applied
   live); the library list and the session in `~/.config/vergence/albumwall/`
   (apart from the Avalonia app's).*
6. **Untrusted bytes**: tags and covers are parsed only in the confined
   helpers (whitepaper §6.10), so a scan sends each file's bytes to one
   long-lived helper. *Lean: yes, measured on your library early (step 2);
   if a big library is too slow that way, the helper reads only the parts
   that hold the tags.*
7. **Meters in milestone 1**: *Lean: the VU pair only (the default today);
   LED and the analyzer in milestone 2.*
8. **Testing on your library**: *Lean: Hubble's 56 test FLACs and the
   display suite for every step; then your music on techbench (the NAS
   share), read-only, before the milestone closes.*

## Steps

1. **The skeleton**: crates/vergence-albumwall, the desktop entry (Music),
   an icon, install.sh and the package; a window with the first-run welcome.
2. **The library**: the helper's tag answer and scan mode; the grouping,
   year and sort rules with AlbumWall's tests ported; the index file; several
   libraries and the picker; the folder watcher. Scan time measured on a big
   folder.
3. **Covers and colors**: embedded and sidecar art, the bytes entry and size
   buckets in vergence-decode, the cover cache; Palette, Ground and Contrast
   ported with their tests.
4. **The wall**: the virtualized grid in the toolkit, the tile layout
   arithmetic, labels, placeholders, the fade-in. Scrolling a 2,000-album
   wall measured on kepler and techbench.
5. **The album panel**: timed transitions in the toolkit; the unfold, the
   arrow, the track list, the sleeve play button, the turn-over, the swap in
   place, centering; Reduce motion.
6. **Playing**: vergence-sound's pause, flush, position and stream name;
   the engine (gapless, the next track ahead, ReplayGain, the app's volume);
   the play bar and VU meters; when an album ends; the wall following; the
   session.
7. **Search and keys**: the live search and `art:` queries; the whole
   keyboard model and the Esc chain; full screen.
8. **Preferences**: the folder and slider rows; every page; text size and
   motion; the error screens.
9. **The checks**: display-suite scenarios (the wall, unfolding, playing
   with the paced silent output, search); a package for techbench; your pass
   on your own library.

## Progress

- **Step 1, the skeleton** (2026-11-25): crates/vergence-albumwall; the
  window (AlbumWall's dark one whatever the desktop's scheme, the server's
  dark bar titled AlbumWall); the first-run welcome in the Avalonia app's
  words, its "On this computer" half (the music server's comes with
  Navidrome); Choose a folder… through the toolkit's folder dialog; the
  library list in `~/.config/vergence/albumwall/libraries` (our own small
  text file, written whole and renamed over, tested); the app menu (Library
  › Add a folder…, About); the desktop entry (Name AlbumWall, GenericName
  Music) and AlbumWall's own icon, in install.sh's lists. AlbumWall is
  GPL-3.0; what is ported from it (its rules, colors, icon) is Dave's own
  work, brought under Vergence's license as its author, as Hymie's decoders
  were. Tests: Hubble 1,101, kepler 1,090, techbench 1,101.
