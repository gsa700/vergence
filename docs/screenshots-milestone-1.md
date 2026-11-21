# Screenshots milestone 1: the Mac's keys, built into the server

Status: **questions answered 2026-11-20** (all four as recommended:
recording its own milestone next, with our own encoder; Close-Up's crop
and rotate now, markup later; other programs through the portal, each
asked first; a window with its shadow, Alt+click without). An app milestone,
after display lists (core vision) by the rule of 15 November. Asked for
2026-11-15 ("a built in for screen shots and video capture that mimics the
macos keyboard shortcuts, I love that so much more than diving into a menu
or print screen copy paste"); the card idea the same day ("that floating
image could just be a notification containing the picture and clicking
opens it in Close-up for viewing and minor edits"); where they go
2026-11-20: **~/Pictures/Screenshots/$DATE**.

## What it does

As on the Mac, Super standing in for Command:

- **Super+Shift+3**: the screen under the pointer (Dave, 2026-11-20:
  the Mac takes every screen, but the second file is usually thrown
  away); Settings > Screenshots can choose every screen, each its own file.
- **Super+Shift+4**: drag out an area (crosshair, its size shown beside
  it; Escape cancels; Shift and Alt while dragging as on the Mac: one side
  fixed, from the center). **Space** turns it into a window picker: the
  window under the pointer lit, a click takes that window alone (its bar
  and its shadow, on a clear ground; Alt+click without the shadow).
- **Super+Shift+5**: a small bar on the screen: a screen, all screens, a window,
  an area; a timer (none, 5 s, 10 s); where to save; show the pointer or
  not; Capture. Recording's two buttons come with the recording milestone.
- **Ctrl** with any of them: to the clipboard instead of a file (image/png,
  so any program can paste it).
- Files: `~/Pictures/Screenshots/2026-11-20/Screenshot 2026-11-20 at
  22.56.01.png` (a folder for each day; two in the same second numbered;
  several screens named by screen: `... (DP-1).png`). PNG, made 0600 like
  vctl's shots, the screen's real pixels at its scale.
- **The card**: each capture shows as one of the server's notification
  cards holding the picture (notify.rs cards gain a picture); a click opens
  it in Close-Up; a drag from the card drops the file into any app (Edit,
  FileView, Firefox); it goes after a few seconds, or swiped away. While a
  card shows, the next capture takes its place.
- A camera's sound, as the Mac, off in Settings (and under Do Not Disturb).
- Settings > Screenshots (or Keyboard > Shortcuts): what Super+Shift+3
  takes (the screen under the pointer, or every screen), the folder, a folder
  per day or not, the sound, the card, the keys.

Never while the screen is locked; the lock screen and the password dialogs
are not captured (the server leaves them out, as it leaves out the pointer
unless asked).

## The steps

1. **Capture in the server**: a screen, an area (in points, across screens
   of different scales: drawn at the densest screen's scale it touches), a
   window alone with its shadow (drawn on its own, as the overview draws
   it); to PNG off the main thread; the folder and names; vctl `capture
   screen|area X Y W H|window N [--clip]` for tests.
2. **The keys and the area picker**: Super+Shift+3/4 (+Ctrl), the
   crosshair and its numbers, Space to pick a window, Escape.
3. **The clipboard**: the server offers image/png itself (it already
   offers text it owns), to native apps and Wayland programs alike.
4. **The card**: pictures in notification cards; click opens Close-Up; drag
   the file out.
5. **Super+Shift+5's bar**: modes, the timer, the folder, the pointer.
6. **Close-Up's minor edits**: crop and rotate, saved back or as a copy
   (markup in a later Close-Up milestone).
7. **Other programs**: the desktop portal's Screenshot interface,
   answered by the server, each program asked first (the agents' kind of
   dialog), its answer kept and shown in Settings > Privacy.
8. **Settings and checks**: the settings page; a display-suite scenario
   (a capture is the screen's pixels exactly; an area across a 1x and a 2x
   screen; a window alone with its shadow); tests on all four.

## Not in milestone 1

Recording, by question 1. OCR (text out of a capture) and the Mac's
"Live Text". Scrolling captures (a whole long page).

## Questions for Dave

1. *Answered: as recommended.* **Recording.** It needs a video encoder, which we would write ourselves
   (as the PDF engine and the JPEG 2000 decoder): (a) screenshots in this
   milestone, recording its own milestone next with the encoder
   (recommended: an encoder for a format every player opens is a real
   piece of work, and screenshots stand on their own); (b) both now, with
   a simple format first (Motion JPEG in a file mpv and VLC open; not
   Firefox); (c) recording only as an animated picture (APNG) for short
   clips, the video later.
2. *Answered: as recommended.* **Close-Up's edits.** (a) crop and rotate in this milestone, markup
   (arrows, boxes, text, highlighter) in a Close-Up milestone later
   (recommended: a capture is useful at once; markup is a small drawing
   app of its own); (b) all of it now; (c) none now.
3. *Answered: as recommended.* **Other programs taking screenshots** (Firefox's own screenshot tool,
   OBS, Flameshot) through the desktop portal: (a) yes, each program asked
   first with the agents' kind of dialog, its answer kept (recommended);
   (b) not in this milestone.
4. *Answered: as recommended.* **A window alone**: with its shadow on a clear ground, Alt+click
   without, as the Mac (recommended), or without the shadow by default?

## Progress

- **Step 1, capture in the server** (2026-11-20). crates/vergence-server
  capture.rs: a request names a screen (the one under the pointer unless
  named), every screen, an area in points, or a window alone; drm.rs
  `run_captures` takes it on the next frame: a screen as it shows
  (unmagnified, the pointer left out unless asked); an area as each
  screen it touches shows its part, at the densest of their scales (a less
  dense screen's part enlarged to it, as it really shows); a window alone
  (the scene builds only its elements, `capture_only`), at the densest
  scale of the screens it is on, so a window drawn from its scene is drawn
  sharp there, on a clear ground with a soft shadow added (Vergence's
  windows have none of their own; `--no-shadow` without). Files in
  ~/Pictures/Screenshots/<date>/ as "Screenshot <date> at <h.m.s>.png",
  " (2)" and on for the same second (names given lately count as taken
  while their files are still written), every screen named by screen;
  folder 0700, file 0600, written beside and moved into place, off the
  main thread. Never while locked or at the login screen; the password
  prompt is left out. `vctl capture screen [NAME] | screens | area X Y W H
  | window I [--pointer] [--no-shadow] [--to FILE]`. Tried headless on
  Hubble (a 1x and a 2x screen, Calculator across both): the screen, both
  screens (1920x1080 and 3840x2160), an area across both (800x600 at 2x),
  the window alone (820x1292 with its shadow, 724x1196 without), all
  0600. Tests: names counting up, the day's folder, pasting one to one
  and enlarged, the shadow around and below.

