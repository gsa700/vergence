# Where Vergence stands

The one status page (updated 2026-11-22). Each milestone's plan and progress
is in docs/<area>-milestone-N.md; the order of work and its reasons are in
the whitepaper's §9 (docs/Vergence-Whitepaper.md); wishes and what comes
after are in docs/apps-roadmap.md.

Vergence runs as the daily desktop on Void Linux and FreeBSD machines (Void
first since 2026-11-22), Intel graphics first; NVIDIA works as a second GPU
through Mesa; AMD is not tested yet; NVIDIA's own driver waits for explicit
sync.

## In progress

Nothing: packaging milestone 1 closed 2026-11-22 (docs/packaging-milestone-1.md):
the `vergence` xbps package and `vergence-setup`; techbench runs from it.
Defaults milestone 1 closed the same day.

## Next

1. **Going public** (docs/release-pathway.md §3): the checks there, then a
   first release (tag, tarball, checksum, a signed repository).
2. **AlbumWall for Vergence** (generic name Music): AlbumWall's wall of
   covers and player rebuilt in Rust on the toolkit and Hymie
   (crates/vergence-hymie), for Vergence (docs/apps-roadmap.md).
3. **The web component**: WebKit (WPE) as a component any app can embed,
   likely a service the server composites; then Mail and a browser on it.
4. **The Vergence system** (release pathway §4): our installer and ISO on a
   Void base, after the package; then a FreeBSD port.

## Built

| Area | What | Plan |
|---|---|---|
| Server | The display server: DRM/KMS, GPU rendering with a CPU fallback, every screen at its own scale, several GPUs, headless screens | server-, gpus-, headless-milestone-1 |
| Protocol | The session bus and native window protocol (one exact version shared by server and apps), the control socket for tests | bus-, native-milestone-1 |
| Display lists | Toolkit windows sent as scenes and drawn by the server; the magnifier sharp | display-lists-milestone-1 |
| Toolkit | vergence-ui and vergence-widgets: text, widgets, the Open/Save dialog, text areas, spelling | toolkit-milestone-1, -2; editor-milestone-1 |
| Windows | The server-drawn bar with tabs, window memory, snapping, Alt+Tab, styles (dialogs, palettes, sidebar windows), full screen | bar-, windows-, window-styles-milestone-1 |
| Seat | Pointer, keyboard and touch decided in vergence-seat | seat-milestone-1 |
| Look | Frost, light and dark with contrast floors, rounded corners, our own icons, pointers and fonts | look-milestone-1 |
| Shell | The dock with live tiles and the drawer, the launcher, named desktop actions, the overview | shelf-milestone-1, -2; dock-, overview-milestone-1 |
| Login | vdm: the login screen and the lock screen | vdm-milestone-1 |
| Session | Everything back after a restart; documents fresh after a deliberate close | session-milestone-1 |
| Meaning | Contract 3: widgets acted on by meaning, the agent interface with permission, the magnifier | meaning-milestone-1 |
| Settings | Grouped pages, displays, input, Wi-Fi, energy, notifications, apps | settings-milestone-1, -2; app-settings-milestone-1 |
| Sound | OSS, ALSA and the PulseAudio protocol: outputs, volume, playback streams | sound-milestone-1 |
| Apps | Edit, FileView (tabs, sidebar, drives, disk images, the Space preview), Reader (PDF on our own engine), Close-Up, Terminal, Calculator, Hardware, Force Quit, Screenshot | editor-, files-milestone-1..3, reader-, pdf-, tools-, screenshots-milestone-1 |
| Drives | One drive list kept by the server, eject that lets a stick go | drives-milestone-1 |
| Preview | Space's window shaped for what it shows; Hymie (WAV, FLAC, MP3, ALAC, AAC) in Rust | preview-milestone-1 |
| The tree | One format, files split by part, comments that keep the why, these documents | tree-milestone-1 |

## Built, with checks by hand still open

Seat 1 (key repeat, X11, drag and drop by hand), shelf 1 (two monitors on one
machine), look 1, VDM 1 (a YubiKey at the login screen, several monitors):
each milestone doc lists what is left.

## Not built yet

The drawer's filter field; a calendar in the clock's card; weather and
system-log tiles; docks on several screens; explicit sync and NVIDIA's own
driver (whitepaper §8.3); AMD testing (a Ryzen mini PC is on the wish list);
the rest of Smithay's replacement (docs/borrowed-review.md); connecting to servers
from FileView; packages.
