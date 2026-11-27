# AlbumWall milestone 2: the music server, lyrics, and Now Playing for the whole desktop

*Drafted 2026-11-26*, after milestone 1 closed in v0.4.0 (Dave: "draft the
albumwall milestone 2 plan"). From milestone 1's "Later" list, the Avalonia
AlbumWall's own code for each piece (Navidrome.cs, Lyrics.cs, LyricsView.cs,
Mpris.cs, SingleInstance.cs, CommandLine.cs), and techbench's handoffs of
2026-11-25 (src:vergence-now-playing). Questions at the end, with leans;
nothing is built before Dave's answers.

**Answered 2026-11-26** (Dave: "all leans, go ahead"): milestone 2 as below,
the extras in milestone 3; HTTPS through the system's OpenSSL in a confined
helper (HTTP allowed with a warning, HTTPS supported, never required); the
password checked once and forgotten, the user, salt and token kept; the
album list and covers kept, a track only while it plays; lyrics shown in the
open album behind the kept row's button; Now Playing Vergence's for every
player, with the media keys and the picture primitive; one instance and the
command line over MPRIS; a Subsonic server of our own in the suite, then
Dave's MusicServer read-only. Building, step 1.

## What milestone 2 does

- **A music server as a library: Navidrome.** The welcome's second half ("On
  a music server") and Settings › Library's "Add a Navidrome server…": an
  address, a user and a password, checked once and never kept (see question
  3). The server's albums become the same wall, panel and play bar as a
  folder's, so nothing above the library knows the difference. Read-only, all
  of it: no tags, ratings or play counts written. The files asked for as they
  are (`format=raw`), bit-perfect and gapless, as the Avalonia app measured.
  The album list and the covers kept on this machine so the wall opens at
  once and is checked against the server behind it; its own page under
  Settings › Library (name, address, Forget, its numbers).
- **Lyrics** in the open album that is playing, behind the Lyrics button in
  the row kept under the panel's header for it (milestone 1 kept that row on
  purpose: Dave, 2026-11-26). Where they come from, first found wins, as in
  the Avalonia app: a `.lrc` beside the track, the track's own tag (FLAC and
  Ogg LYRICS, MP3 USLT, M4A ©lyr; read in Hymie's confined helper, as every
  tag), and for a Navidrome library the server's own (timed preferred).
  Timed lines (LRC) light up as the music reaches them and the view follows;
  plain lyrics read as text. Shown only: editing them is Marginal's, the
  family's tag editor, which is not a Vergence app yet.
- **MPRIS**: AlbumWall on the session bus as a media player
  (org.mpris.MediaPlayer2), so anything that speaks it (playerctl, other
  desktops' controls) can play, pause, skip and see what plays.
- **Now Playing, for the whole desktop** (techbench's handoff, Dave: "a now
  playing tile like the other OS's we support. Tracks, art, seek, etc.
  mapped to media keys if present"). Owned by Vergence, not AlbumWall: the
  server is the session bus, so it sees every MPRIS player (AlbumWall,
  Firefox playing YouTube, mpv). A **Dockworker tile** (small cover and the
  track, quiet when nothing plays) and its **card** (the cover, artist,
  album, track; previous, play or pause, next; a seek bar with elapsed and
  remaining, its position read only while the card is open; that player's
  volume row from the mixer; the title raises the player's window). Several
  players: the one most recently playing, the others under it.
- **The media keys** (play/pause, next, previous, stop) routed by the server
  to the Now Playing player, whichever window has the keyboard.
- **One AlbumWall at a time, and the command line**: starting it again brings
  its window forward; `vergence-albumwall --play | --pause | --next |
  --previous | --stop` reach the running one (CommandLine.cs's commands),
  carried over MPRIS, so they need nothing of their own.

## Later (milestone 3, and not ported)

- **Milestone 3**: compact mode (the window becomes the now-playing view, its
  own size remembered) and the front panel view; the LED bars and the
  31-band analyzer beside the VU pair (one strip, cycled by a click); the
  four-band parametric equalizer and its window (off by default, with
  headroom so a boost never clips); the Properties window (every tag, shown
  only); the FLAC integrity check (each file's audio against the MD5 in its
  STREAMINFO, off by default, decoded by Hymie in its helper, so no libFLAC).
- **Not in AlbumWall, but asked for with it**: the window's own bar items
  (AlbumWall's wordmark and buttons, drawn by the server: Dave, "I am hoping
  we can get the custom titlebar back at some point but we'll have to build
  it into the toolkit first"). It changes every app's bar, so it is a
  milestone of its own (window styles milestone 2), laid out before and
  after for Dave, AlbumWall its first user.
- **Not ported**: the kiosk's front-panel window rules (the Pi keeps the
  Avalonia AlbumWall), Windows' media controls, "Edit in Marginal".

## How it is built

What Vergence has, and what this milestone adds (each addition where any
program can use it):

- **HTTP**: nothing in the tree speaks it. Added: our own small HTTP/1.1
  client (requests, chunked and fixed-length answers, range requests for
  seeking, redirects within a server, time limits), in a crate of its own;
  every byte from the network parsed as untrusted, fuzzed as the D-Bus wire
  code is. HTTPS: question 2.
- **Subsonic**: the API Navidrome serves, the token sign-in (an MD5 of the
  password and a salt, sent with the salt; question 3), the album list,
  album details, covers, streams and lyrics; its answers (JSON) parsed by
  our own reader.
- **Streaming into Hymie**: Hymie's decoders read from a source that can
  seek. A track is fetched into a file under ~/.cache as it plays, the next
  one ahead (so it stays gapless), and a seek past what has arrived asks
  the server for that range. The helper decodes the file as it fills.
- **vergence-dbus**: today a client that makes calls (udisks2). Added: the
  other half a service needs, exporting objects on the session bus,
  answering method calls and property reads, and sending PropertiesChanged.
  MPRIS is its first user; any app can then offer a D-Bus interface.
- **The server**: the session bus is its own (dbus.rs), so it knows when an
  `org.mpris.MediaPlayer2.*` name appears or goes. Added: an MPRIS reader
  (metadata, playback status, position, art), the media keys as named
  actions (`media.play-pause`, `media.next`, ...) sent to the Now Playing
  player, and the Now Playing Dockworker.
- **Dockworkers**: a **picture** primitive for cards (the cover art), as the
  agreed Dockworkers principles require: server-drawn, scaled, no app
  pixels outside it, and named with its three real users (Now Playing's
  art, a camera snapshot, a weather or radar image).
- **AlbumWall itself**: the Navidrome library and its pages, the lyrics
  view, MPRIS, the command line.

## Questions (with leans)

1. **The split**: milestone 2 as above (the server, lyrics, MPRIS, Now
   Playing and the media keys, the command line), and compact mode, the
   other meters, the equalizer, Properties and the integrity check in
   milestone 3? *Lean: yes. Milestone 2 is the music reaching you from
   everywhere and being controllable from everywhere; milestone 3 is the
   extras inside the window.*
2. **HTTPS.** The Avalonia app's rule (2026-10-10): HTTP allowed with a
   clear warning, HTTPS supported, never required. Your Navidrome is on
   the LAN over HTTP, but a server reached from outside needs HTTPS. Ways
   to have it: (a) **the system's OpenSSL** (LibreSSL-compatible), already
   on both systems (Void's package manager uses it; FreeBSD has it in
   base), called through FFI in a confined helper, as libarchive is for
   archives; (b) **rustls** and a crypto library as borrowed crates
   (pure Rust, but several crates and a crypto implementation brought in);
   (c) **our own TLS 1.3 client**: months of work, and cryptography is the
   one place where writing our own is the risk rather than the safety;
   (d) **HTTP only** for now. *Lean: (a), keeping the Avalonia rule. It is
   a borrowing question: OpenSSL is what makes HTTPS possible here at a
   sane cost, it is already on every machine Vergence runs on, and it runs
   confined, so a flaw in it reaches no further than the helper.*
3. **What is kept of the sign-in.** Subsonic's token sign-in sends an MD5 of
   the password and a salt, never the password. *Lean: as the Avalonia app:
   the password is checked once and forgotten; the user, the salt and the
   token are kept in the library list (`~/.config/vergence/albumwall`,
   readable by you only). The token works only with that salt and is not
   the password, but it signs in to that server, so the file says so and
   Forget removes it.*
4. **What is kept of the music.** *Lean: the album list and the covers kept
   on disk (a folder's covers are read from its files); a track's file kept only while it plays
   and the next is prepared, then deleted. No offline copies in milestone
   2.*
5. **Lyrics: shown where?** *Lean: as the Avalonia app: in the open album
   that is playing, the Lyrics button in the kept row turning the track
   list into the lyrics and back; timed lines lit and followed; nothing
   edited.*
6. **Now Playing: Vergence's, for every player** (the handoff), with the
   media keys and the picture primitive in this milestone? *Lean: yes. It
   is what gives MPRIS its point on Vergence, and AlbumWall is its first
   and best-tested player.*
7. **One AlbumWall at a time, and the command line over MPRIS?** *Lean: yes:
   no channel of its own; a second start asks the first, over the bus, to
   come forward, and exits.*
8. **Testing.** *Lean: a small Subsonic server of our own for the display
   suite (made in the scenario: answers, covers, ranges, a slow and a
   broken server), MPRIS checked through the bus, Now Playing's tile and
   card in the suite; then your MusicServer from Hubble and techbench,
   read-only, before the milestone closes.*

## Steps

1. **HTTP**: the client crate, fuzzed; HTTPS per question 2.
2. **Subsonic and the Navidrome library**: the sign-in, the album list and
   its cache, covers, album details; the welcome's second half, Settings ›
   Library's server pages; "cannot be reached" for a server as for a folder.
3. **Streaming**: tracks fetched as they play, ranges for seeking, the next
   ahead for gapless playback, ReplayGain from the server's tags.
4. **Lyrics**: the three sources, LRC parsed, the lyrics view and its button.
5. **vergence-dbus as a service, and AlbumWall's MPRIS**; the command line
   and one instance.
6. **The server's Now Playing**: the MPRIS reader, the media keys, the
   picture primitive, the tile and card.
7. **The checks**: the suite's Subsonic server and scenarios; a package for
   techbench; your pass with your own server.
