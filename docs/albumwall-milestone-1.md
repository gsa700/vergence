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
  were (Dave, 2026-11-25: "bring the port under the vergence dual license,
  we'll leave the original as is"; the Avalonia AlbumWall stays GPL-3.0). Tests: Hubble 1,101, kepler 1,090, techbench 1,101.
- **Step 2, the library** (2026-11-25, all but the folder watcher):
  - Hymie (f5dae74, d50dae5): a file's facts from its two ends
    (crates/vergence-hymie/src/facts.rs): the program sends 256 KB of head,
    128 KB of tail and the size; the helper answers the facts or how much
    more of each end it wants (a FLAC's metadata chain, an MP3's ID3 tags,
    an MP4's moov, a WAV's chunks). The walk that decides runs in the
    helper. AAC measured from its container. ID3 and MP4 now give the
    album artist, track, disc and date. `helper::Scanner`: one helper for a
    whole scan; `I` measures sidecar pictures there.
  - AlbumWall: the library rules with the Avalonia app's tests (library.rs,
    f9327b7/7a21f13); the index, our own file per library in
    ~/.cache/vergence/albumwall (index.rs); the scan (scan.rs): listed
    first, read second; missing, emptied and failed libraries told apart;
    `--scan FOLDER [--rescan]` for measuring.
  - **Measured** on a synthetic library on Hubble (1,000 albums, 12,000
    FLACs of 30 MB, sparse; tools in ~/.cache/vergence-lab/scratch): first
    scan 13.1 s, unchanged 0.07 s, Rescan 11.5 s. A first scan of a real
    library over the network reads about 384 KB a file (4.6 GB for 12,000
    tracks, ~40 s over gigabit) instead of every byte. Not yet measured on
    Dave's own library (on NASBOX; not mounted on the lab machines).
  - The window (reading.rs): the read in the background with a true
    progress bar, then the counts until the wall (step 4); "cannot be
    reached" in the Avalonia app's words, Try again, looked for again
    every 30 seconds; a wall already shown stays while its library is out
    of reach; Library › Rescan the library.
  - Left: the folder watcher, and the library picker (it lives in the
    bottom bar, which comes with the wall).
- **Step 3, covers and colors** (2026-11-25):
  - Embedded covers are decoded by Hymie's helper (`C`: the file's ends as
    for facts, the picture decoded there by vergence-decode's decoders at
    the size asked for), so no bytes entry in vergence-decode was needed;
    sidecars by vergence-decode's helper (decode_file, shrunk to the
    bucket). helper::Scanner's request loop is shared by `F` and `C`.
  - covers.rs: buckets 256/384/512, keys by source, size, time and bucket,
    a 160 MB LRU, worker threads taking the newest request first, a cover
    that cannot be decoded not asked for again; each cover measured for
    its colors from a 16-pixel sample.
  - colors.rs: the contrast floor, the album palette and the wall's ground
    and bars (recede, lift, ink, warm) ported from Contrast.cs, Palette.cs
    and Ground.cs, with tests that every album's panel and play-bar text
    clears 4.5:1 at every hue and Light, and the wall's text and bars at
    every Light and Chrome.
  - `--colors FOLDER`: on techbench, 200 of Dave's covers each: FLAC
    sidecars 15 ms a cover, the lossy library's embedded covers 7 ms; none
    failed; library hue 11° and 22°.
- **Step 4, the wall** (2026-11-25): the toolkit's grid (only rows in view
  built); wall.rs: the columns nearest a 185-point cover filling the row
  exactly (132 to 268), the tile's height, each album's placeholder color
  (AlbumVm's FNV over the names); wallview.rs: tiles (cover from the cache,
  never cropped, on its placeholder; a tile with no art its own sleeve),
  title and artist under it in the ground's text colors, the margins
  (26/20/26), the ground and bars from the library's hue (measured from
  200 covers spread across it, on a thread of its own), the bottom bar
  with the status line, the counts and the library picker. Seen on
  techbench with Dave's FLAC library (372 albums): covers load, scrolling
  to the end builds the rows there. Moved to step 5 with the transitions:
  the covers' 220 ms fade-in; the scrolling measurement on kepler.
- **Step 5, the album panel** (2026-11-25): panel.rs (the header line,
  "mixed" when tracks differ; the track list down the left column and then
  the right, never ending on a disc heading; the panel's height worked out,
  at least the sleeve tall); panelview.rs (the arrow over the tile and the
  body in the album's palette, fading over its last 28%; title, artist,
  header line, the two-column list with lengths at the right, the 260-point
  sleeve with its play overlay under the pointer); the unfold (330 ms, the
  contents fading in after, 110 ms), the tile turning over to back.jpg
  (400 ms) and ringed in the album's accent, another album in the same row
  swapping the contents in place, a press on the open album or Escape
  folding it (330 ms), the wall gliding to center an album as it opens,
  covers fading in (220 ms). AAC files now report no bit depth (index
  version 3). Seen on techbench with Dave's library. Not yet: the panel
  staying above when an album in the row below it is pressed (pointing
  down at it); the scrolling measurement on kepler.
- **Step 6, playing** (2026-11-25): player.rs (one stream, tracks back to
  back with nothing between, the next opened ahead, a new output only for
  another rate or channels, ReplayGain by album or track, dither to 16
  bits, where it is as heard, previous going back a track only in a
  track's first 3 s, loading paused at a position for resume; tested with
  made-up tracks and a recording stream: two tracks of 3,001 and 2,999
  frames come out as 6,000); playbar.rs (82 points over the wall's foot
  in the album's light tone: cover to the playing album, title and artist,
  previous, play or pause, next, the play mode, elapsed, seek, remaining,
  the VU meters with 300 ms ballistics); the playing track's pulsing ▶ in
  the open panel; when an album ends: stop, the next album, or a random
  one not played yet (the wall opening it); picking up where it was left
  (~/.config/vergence/albumwall/session: the album open, paused). Played
  silently (VERGENCE_SOUND_OFF=pace) on techbench against Dave's library.
  - **Decided** (Dave, 2026-11-25, techbench handoff
    src:vergence-now-playing): no in-app volume: the stream stays at 100%
    (bit-perfect), turned down in the dock's mixer row (the Avalonia app's
    132-point slider is left out on purpose). Later, not this milestone:
    app-declared title bar items (AlbumWall's own wordmark and buttons, the
    server drawing them); a system Now Playing Dockworker fed by MPRIS,
    with the media keys (AlbumWall publishes MPRIS in milestone 2).
  - **The look's reference** (Dave, src:vergence-albumwall-look-reference):
    the Avalonia AlbumWall; at each step a side-by-side, every difference
    fixed or written down as a decision. Shots in techbench
    ~/Pictures/AlbumWall-Reference/.
