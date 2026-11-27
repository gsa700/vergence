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
  - **The battery** (Dave, 2026-11-26; CONTRIBUTING.md "Mind the
    battery"): playing wakes the window about 20 times a second for the
    clock, the seek bar and the meters, even when it cannot be seen. To do:
    keep playing, but redraw nothing while the window is hidden or
    minimized, and update slower while it is not in front.
    Part done 2026-11-26: the player wakes the window at most 4 times a
    second (a track, a pause or a problem at once), and the meters step at
    30 Hz only while music plays or a needle moves (in front or not: Dave,
    2026-11-26, "they should work all the time"); still to do: nothing at
    all while the window is hidden or minimized.
  - **The meters, ported** (2026-11-26; Dave: "port the meters properly"):
    vumeter.rs is VuMeter.cs: the yellowed face, the scale printed in volts
    (0 VU at 71%), RMS with 0 VU at -10 dBFS, a spring needle (99% in
    300 ms, a little overshoot), read at the block being heard (player.rs
    keeps RMS per 1,024 frames, before gain, and reads it at the heard
    frame), resting when still. The stream's name in the sound server is
    "AlbumWall" (it was "Vergence": techbench, pw-top). Verified in pre15 on
    techbench: the stream named AlbumWall, and the card following 44.1 kHz
    from the session's allowed rates (no resampling).
  - **The play controls, as in the Avalonia bar** (Dave, 2026-11-26: the
    remaining time "bouncing around ... taking the track slider length with
    it"; "style them as we did in avalonia"): the times in fixed places (46
    and 52 points), round buttons (32, play 40) with a faint dark disc, the
    Avalonia glyphs point for point, the play mode dimmed at Stop, the seek
    bar dark on light (the toolkit's slider takes a program's colors:
    `slider_colors`).
- **Step 7, search and keys** (2026-11-26; Dave: "go ahead with step 7"):
  - **Search** in the bottom bar (320 points, "Search artist or album…",
    between the status line and the counts): live over album artist and
    title; `art:missing`, `art:small` (under 300 pixels on the shorter
    side), `art:nonsquare` (sides more than 5% apart; unmeasured art matches
    none); "N of M albums" in the status line. No pause for the keys (the
    Avalonia app waited 150 ms because each key rebuilt the whole wall; the
    toolkit's grid builds only the rows in view). A search that hides the
    open album closes it; clearing it puts the wall back (the album that was
    open, else the album at the top of the view). Playing an album the
    search hides clears the search. search.rs, with tests.
  - **The keyboard** (MainWindow.Keyboard.cs): the arrows from cover to
    cover with a ring on the keyboard's album (light, with a dark rim, for
    any cover), Up off the top row stays (wall::row_move, with Dave's
    2026-10-22 cases as tests), Down from the open album into its tracks
    and off their end to the next row, Enter opens or plays, Home/End,
    Page Up/Down; Space plays or pauses (once a press, not a repeat),
    Ctrl+Left/Right previous/next, Shift+Left/Right seek 10 s, Ctrl+L to
    the album playing, `/` or Ctrl+F to the search box (its text chosen),
    Down from the box to the first album found. A click clears the ring.
    Not ported: Ctrl+Up/Down volume (no in-app volume, decided).
  - **Escape**: the tracks back to their cover, then full screen, then the
    search, then the box, then the open album.
  - **Full screen** (F11): Vergence's own windows had no way to ask for it,
    so the bus gained `WinRequest::Fullscreen` (v41) and the server answers
    it as it does a Wayland or X11 window's (no bar, no dock, the place
    kept); vergence-ui's `Window::set_fullscreen` (xdg_toplevel's under
    other desktops).
  - **Toolkit**: `Program::focus_to` (the keyboard put in a named element,
    a field's text chosen) and `Program::focus_moved` (where it went), so a
    program's keys can mean one thing on its page and another in a field.
  - Checked headless on techbench against Dave's library: "alice" finds 6
    of 372; Down, Enter, Down into the tracks; Escape back out; the wall put
    back; F11 and Escape.
  - **The look's reference** (Dave, src:vergence-albumwall-look-reference):
    the Avalonia AlbumWall; at each step a side-by-side, every difference
    fixed or written down as a decision. Shots in techbench
    ~/Pictures/AlbumWall-Reference/.
- **Step 8, Preferences** (2026-11-26; Dave: "lets continue the albumwall
  porting"):
  - **Decided** (Dave, 2026-11-26), asked whether AlbumWall should use
    every Vergence app's one-page settings window: "There are too many
    settings for a single page layout. we learned that building albumwall.
    so use our toolkit but mimic the layout from avalonia". So prefswin.rs
    is AlbumWall's own window, laid out as PrefsWindow.axaml: a 168-point
    sidebar of pages (About set apart at its foot, the arrow keys moving
    through it), the page on the light sheet beside it, each page
    scrolling on its own, 780 by 640 and never under 640 by 480; the
    Avalonia window's colors (the desk, the sheet, its edge, the picked
    line, the ink); written straight through, no OK button. Its title is
    Vergence's word, "AlbumWall Settings" (Ctrl+, and the app menu's
    Settings…).
  - **The pages**: Library (the libraries, each line with Switch to and
    Settings…, Add a folder…, and the selected library's numbers: the five
    totals, file types with their shares, bitrates, lossless rates, cover
    art with "show on the wall" for `art:missing` and `art:small`; stats.rs,
    kept beside the index so a library not on the wall shows "as of" its
    last read); Playback (pick up where I left off, …and start playing,
    when an album ends, ReplayGain by album, track or off, meters read
    after ReplayGain); Appearance (the hue found and where it came from,
    Light and Tint shown on the wall as they move and written when let go,
    Bars, show the meters, back to the defaults); Accessibility (motion:
    follow the desktop, always or never reduced; text size 100, 115, 130,
    150%); Keys and search (Vergence's keys: Ctrl+arrows, no volume keys);
    About. A library's own window (LibrarySettingsWindow's milestone 1
    part): its name, its folder, Rescan while it is on the wall, Forget.
    Not ported: Updates (the package), Navidrome, the equalizer, peak hold
    (milestone 2), the Start Menu and install sections.
  - settings.rs: every choice in desktop.toml's `[albumwall]` (resume,
    auto-play, after-album, replaygain, meters-after-gain, light, tint,
    bars, meters, reduced-motion, text-size), absent = the Avalonia app's
    default, the default chosen again takes the key away; tested.
  - **Shuffle all songs**, the fourth way an album can end (it was left
    out of step 6): player.rs's `Command::Shuffle` plays every song the
    wall shows in a random order and reports each song's album and number,
    so the play bar and the panel show it and the wall opens each album as
    its song starts; Next while an album plays in this mode starts the
    shuffle (MainWindow.PlayNext). The play bar's button goes round all
    four and writes the same setting. Next and random albums now take the
    albums the wall shows (with a search, only those), as in the Avalonia
    app. Loudness until chosen: by album, by track for a shuffle of songs
    (step 6 had random albums by track).
  - **Toolkit**: `checkbox` and `radio` (ButtonLook::Check and ::Radio: a
    button with its words, so focus, keys, hover and the agent interface
    are a button's; the meaning tree says "checkbox" or "radio" with on or
    off; a chosen radio sends nothing); `Program::text_scale`, a program's
    own text size times the screen's (every named size grows with it).
  - The error screens were already in (no sound output in the status
    line, "cannot be reached", "Reading your library"); nothing changed.
  - Checked headless on Hubble: every page shot; a check box, a radio
    button, Bars, the Light slider and the text size, each written to
    desktop.toml and the wall following (Light 35 and the lift bars on the
    wall, the text 130% in both windows, the covers their size); a
    library's window.
  - **Decided** (Dave, 2026-11-26): the open album's empty row under its
    header (30 points and its 14-point gap, kept since the Shuffle button
    went on 2026-11-01; Lyrics went under the sleeve) stays, in the port
    and in the Avalonia app: "keep in both for now, there was a reason we
    kept them, as I recall I thought it looked crowded right up against
    the titles".
- **Step 9, the checks** (2026-11-26, Dave: "start on step 9 while it
  builds"): display-suite scenario 32-albumwall. A library made in the
  scenario (five albums of tone16.flac with tags written in and its own
  picture taken out, colored covers beside four, one under 300 pixels, one
  album with none); the counts; the covers drawn (colored pixels); an
  album unfolding (its sleeve, its tracks); playing on the paced silent
  output (the play bar, the position moving); with "next album" chosen the
  album ends, the next plays and the wall opens it; the play mode's button
  on to "random album", written to desktop.toml; search: "cobalt" 2 of 5,
  `art:missing` 1, `art:small` 1; Ctrl+, opens Settings, the Library page's
  numbers "as it is now", a check box written and shown off. Passed three
  times on Hubble and once on techbench. For it, and for a screen reader,
  the wall's controls got names (the Avalonia app's): each tile its album
  and artist, the sleeve "Play <album>", the play bar's Previous, Play or
  Pause, Next, the play mode in words, "Go to <album>" on its cover, the
  seek bar "Position". Package vergence-0.4.0pre17 (step 8) installed on
  techbench at 18:48. Left: Dave's pass on his own library on techbench.
- **The app menu** (Dave, 2026-11-26: "go ahead, build the menu. also add
  the file edit etc as needed and the keyboard shortcuts"), as every
  Vergence app's: File (Add a folder… Ctrl+O, Rescan the library Ctrl+R,
  Close window), Edit (Paste into the search box, Find… Ctrl+F), View (Full
  screen F11, Meters in the play bar, Go to what's playing Ctrl+L, Keys and
  search… F1), Playback (Play / Pause Space, Next and Previous track
  Ctrl+Right and Left, forward and back ten seconds Shift+Right and Left,
  the four ways an album ends and the three ReplayGain choices as checked
  items), Library (each library, the one on the wall checked; Library
  statistics…), then Settings… Ctrl+, and About. The choices are Settings'
  own, as the Avalonia gear menu's were; the everyday few on the first
  panel.
- **Decided** (Dave, 2026-11-26), asked whether AlbumWall needs its own
  accessibility choices on Vergence ("since it's coded to vergence and we
  handle that in the server"): "Yes, do it". The Accessibility page is
  gone, with its motion and text size settings and the toolkit's
  `Program::text_scale`; motion, text size and contrast are the desktop's
  (Settings › Accessibility), as in every Vergence app. The Avalonia app
  had its own because other desktops gave it nothing to read. Its "Always
  on" paragraph moved to About.
- **The battery, done** (Dave, 2026-11-26: "go ahead, do it in the
  toolkit"): what moves while music plays is the seek bar and its times,
  the meters and the playing track's ▶; nothing else redraws but on an
  event. The server now tells each of our windows when it cannot be seen,
  minimized or the session locked (not under the remote curtain, which is
  watched from elsewhere; screens off is not counted, since the
  monitorless remote view shows windows with them off): bus v42,
  `WinEvent::Seen`, vergence-ui's `App::seen`, the toolkit's
  `Program::seen`, and the toolkit asks for no frames for a window that
  cannot be seen. AlbumWall plays on, keeps what happens (the next album,
  the shuffle's albums), draws nothing and stops the meters' tick; seen
  again it draws whole and the needles start from where the music is.
  The ▶ now holds still with motion reduced. `vctl restore` (the window
  minimized last, back as the dock brings it) for the tests. Scenario
  32-albumwall measures it: 2.3 s minimized while playing, 1 scene (the
  redraw on the way back); locked, 3; in view, about 35 a second.
