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
- **The floating thumbnail** (Dave, 2026-11-20, in place of the card of
  15 November: "the floating thumbnail, maybe we should anchor it to
  Main"): a small picture of each capture floats in the bottom-right corner
  of the main screen's work area, in front of everything, about 5 s (kept
  while the pointer is on it); a click opens it in Close-Up; a drag drops
  the file into any app that takes files; a flick to the right sends it
  away. Not a notification: Do Not Disturb doesn't hide it. The next
  capture's thumbnail takes its place.
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
4. **The floating thumbnail**: on the main screen; click opens Close-Up;
   drag the file out; flick away.
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
- **Step 2, the keys and the picker** (2026-11-20). vergence-seat: Key
  Three/Four/Five by their place (the server's keys.rs maps evdev's
  KEY_3..KEY_5 by code, so Shift's # and other layouts don't matter),
  `KeyOut::Capture { key, clip }` for Super+Shift+3/4/5 (+Ctrl),
  `Menu::Capture`/`Part::Capture`: while the picker is open every key and
  the whole screen are its. The server's capture.rs: Super+Shift+3 takes
  the screen under the pointer; Super+Shift+4 opens the picker: a
  crosshair, a drag takes the area (lightened, outlined, its size in
  points in its corner while dragging; a click without a drag takes
  nothing), Space turns it into choosing a window (the window under the
  pointer lit in the accent, its bar included; a click takes it, Alt+click
  without its shadow) and back, Escape lets go. Super+Shift+5 opens the
  picker until step 5's bar. Ctrl's clipboard is step 3 (a file until
  then). Tried headless on Hubble: Super+Shift+3 (1920x1080), a drag
  (250x250), Space and a click on Calculator's bar (362x598 with Alt),
  Escape (nothing taken). Seat test: the keys, release swallowed, Super+3
  without Shift the program's, the picker's keys.
- **Step 3, the clipboard** (2026-11-20). With Ctrl (Ctrl+Super+Shift+3,
  Ctrl+Super+Shift+4 and its picker) a capture goes to the clipboard, not
  a file: made PNG on a thread, then the server holds the clipboard itself
  (native.rs `ClipOwner::Picture`, `server_copy_picture`) and offers
  image/png to native apps, Wayland programs and X11 programs alike (the
  same three roads its own text takes); asked for as any other kind,
  nothing is written. `vctl capture ... --clip`, and `vctl clipboard` says
  what the clipboard holds (a picture: its bytes and size). Tried headless
  on Hubble: Ctrl+Super+Shift+3 (a 1920x1080 PNG on the clipboard),
  Ctrl+Super+Shift+4, Space and a click (Calculator, 410x646), no files
  written.
- **Step 4, the floating thumbnail** (2026-11-21). capture.rs `Shot`:
  after a capture is written (or put on the clipboard), its picture
  averaged down to fit 200 x 130 points at the main screen's scale, in a
  white frame with a soft shadow, floats at the bottom right of the main
  screen's work area (`shelf_screen`, the dock's), in front of everything
  (drm.rs screen_elements); 5 s, kept while hovered or held (a timer looks
  again). vergence-seat `Part::Shot`; a click opens the file in Close-Up
  (`vergence-closeup <file>`); a drag of 8 points (not rightward) starts
  the server's own drag (drag.rs `server_drag_start`, a drag with no
  source window) carrying text/uri-list and text/plain; a flick right of
  30 points sends it away. A clipboard capture shows it with nothing to
  open or drag; vctl's own files (`--to`) show none. Tried headless on
  Hubble: it showed after Super+Shift+3, a flick sent it away, a click
  opened Close-Up on the file, a drag dropped it on Edit (the ghost
  accented, "dropped (copied)"), and it went after 5 s. Tests: the link
  comes back as its path, the thumbnail's size, frame and picture.
- **Step 5, Super+Shift+5's bar** (2026-11-21). capture.rs `Bar`: a small
  bar at the bottom of the work area of the screen under the pointer
  (over the dock's band): Close; the modes Screen, All screens, Window,
  Area (the chosen one lit); No timer / 5 s / 10 s, To file / To
  clipboard, No pointer / With pointer, each a button stepping through
  its choices (sized for its widest, so nothing jumps); Capture in the
  accent. Enter captures, Escape or a click outside closes, Left and Right
  step the modes; Ctrl+Super+Shift+5 opens it set to the clipboard. A
  window or an area hands over to Super+Shift+4's picker with the bar's
  choices; a screen with a timer counts down in the Capture button ("3…"),
  the desktop usable meanwhile (Escape stops it), and the bar is gone
  before the picture is taken. The choices are kept in
  ~/.local/state/vergence/screenshots (0600). Tried headless on Hubble:
  the bar, the modes stepped by the arrows, the choices kept after
  Escape, Enter taking the screen, a 5 s timer (nothing at 3 s, the file
  at 7 s, no bar in it), Window through the bar taking Calculator. Test:
  the choices read back, nonsense ignored.
- **Step 6, Close-Up's minor edits** (2026-11-21; crop and rotate, markup
  later as Dave chose). vergence-closeup: a Crop tool (vergence-widgets
  toolbar `Icon::Crop`; C, or the Picture menu): drag over the part to
  keep (the rest dimmed, an outline), then Enter, Ctrl+K or the status
  line's Crop button; Escape lets go. Turning a still picture now counts
  as an edit too (an animation's turn stays for looking, and animations
  can't be cropped: saving would lose them). Edited, the status line
  offers Save (Ctrl+S, the Save role): a PNG is written back in place
  (beside it and moved, its permissions kept), any other kind as
  "<name> (edited).png" beside it, which the window then shows (a JPEG is
  not made again). Closing with edits asks, as Edit does (Don't save,
  Cancel, Save); Next and Previous wait until the edits are saved.
  pictures.rs `crop`, `copy_name`, `save_png` (png, already the server's).
  Tried headless on kepler: a 1920x1080 capture opened, C, a drag, Enter,
  Ctrl+S: saved as 400x240 in place, still 0600. Tests: crop takes the
  part, a PNG saved reads back the same, the copy's name.
- **The Screenshot app, and the bar fixed** (2026-11-21, Dave: "How will
  users know about all this? Maybe we should make a little utility that
  calls the bar directly and let's the user know about the rest? Also,
  the bar is over the dock and they are interacting and the 'Capture'
  button at the right end is off the right edge of the bar itself").
  **Screenshot** (vergence-protocol src/bin vergence-screenshot.rs, its
  desktop entry in Utilities, its own icon: a camera in a selection's
  corners) opens the bar over the bus (v32 `ClientMsg::ScreenshotBar`) and
  ends, as the Mac's Screenshot app; found by typing "screenshot" in Apps,
  pinnable to the dock. The bar now shows the keys on a line under it
  ("Super+Shift+3 the screen · Super+Shift+4 an area (Space: a window) ·
  add Ctrl for the clipboard"), so whoever opens it learns them. Fixed:
  its width counted two of its three gaps and measured labels in another
  font than drawn (Capture stuck out); it stands above the dock when the
  dock is at the bottom of its screen (even one that hides); the dock
  takes no hover while the picker or the bar is open.
  The bar can be dragged by its ground (anywhere but a button), as a
  window by its bar (Dave, 2026-11-21: "can the screenshot bar be made
  movable by dragging it like a window?"); it stays on its screen and
  opens where it was left next time (`place` in the kept choices, from the
  screen's corner, kept on the screen). Tried headless on Hubble: dragged
  up and left, "place = 357,180" kept, opened there again.
  A grip at its left end (two columns of dots; Dave: "maybe a little
  'handle' on the left end?"): the pointer's open hand over it, closed
  while the bar is dragged (dispatch.rs passes the motion's point to the
  pointer's picture, and sets the closed hand while a button is held).
- **Step 7, other programs** (2026-11-21). crates/vergence-server
  portal_shot.rs: org.freedesktop.portal.Screenshot (version 2) on the
  D-Bus edge, Screenshot and PickColor. Without `interactive` every screen
  is taken as one picture; with it, Super+Shift+4's picker (an area, Space
  for a window); PickColor opens the picker as a crosshair and a click
  picks the pixel. Answered by the Request's Response: 0 with `uri` (a PNG
  in $XDG_RUNTIME_DIR/vergence-portal, 0600) or `color` (three doubles),
  1 cancelled or not allowed, 2 failed. Each program asked first by a
  question the server draws (its name, and its executable from its
  process, which the grant is tied to): Don't allow (the keyboard's: Enter
  and Escape), Allow once, Always allow; nothing counts for a second;
  after Don't allow the same program is refused without asking for a
  minute; Always kept in ~/.local/state/vergence/screenshot-programs
  (0600). A program that leaves takes its question or picker with it.
  Never while locked. vergence-dbus example `portal-shot
  [screenshot|interactive|color]` asks as a program does and prints the
  answer. Tried headless on kepler: the question (Enter refused it; asked
  again within the minute, refused without asking), Always allow (a PNG
  answered; the next request answered without asking), interactive with
  Escape (cancelled), PickColor (the ground's color, 0.102 each).

