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

Terminal on the agent interface (docs/terminal-agent-milestone-1.md):
**closed 2026-11-23** after Dave's hands-on pass on techbench; three dock and
grant fixes from it (the Dockworker's text kept in its slot, the agent tile
gone after a stop, a closed tab's grant ended) wait for a release.
**The agent layer is frozen** (Dave, 2026-11-22): no new agent features
unless day-to-day use asks; maintained and reviewed; the meaning tree keeps
growing for accessibility and testing; revisit after about a month of real
use.
Typing stays (Dave, 2026-11-23): its use is a person who cannot type,
helped by voice; the safeguards that assume hands are the known work for a
voice milestone (the milestone doc, "Why typing stays").

Sound for other programs on Void (docs/sound-milestone-1.md, 2026-11-22):
the session starts PipeWire with WirePlumber and pipewire-pulse
(soundserver.rs; AlbumWall was silent without a sound server), the package
depends on them, vergence-setup routes ALSA programs through it. Our D-Bus
edge's name signals now come in dbus-daemon's order, which WirePlumber's
device reservation needed (it gave only Dummy Output). Tested on Hubble in
headless sessions; a real session restart there waits for its screen to be
unlocked. Package vergence-0.2.0_2 (built from a commit after the v0.2.0
tag) waits in techbench's local repository.
Packaging milestone 1 closed 2026-11-22 (docs/packaging-milestone-1.md):
the `vergence` xbps package and `vergence-setup`; techbench runs from it.
Defaults milestone 1 closed the same day.

## Next

1. **Extracting archives in FileView** (Dave, 2026-11-22: "queue it after
   the terminal fixes"): double-click a .zip, .tar.*, .7z to extract it beside
   itself into a folder of its name, and "Extract here" in the item's menu;
   by bsdtar (libarchive: base tar on FreeBSD, the `bsdtar` package on Void,
   added to the package's depends), in a confined helper, nothing written
   outside the target (no `..`, absolute paths or symlinks out), caps
   against archive bombs, progress as for copies. Later: an archive browsed
   as a folder, and Space previewing its list. **Built 2026-11-23**
   (docs/archive-milestone-1.md): libarchive through FFI in a confined
   helper (vergence-unpack) instead of the bsdtar program, and Compress to
   .zip too (Dave: no zip/unzip needed); waits for the independent review
   and Dave's hands-on pass.
2. **Remote access** (Dave, 2026-11-22: the next milestone after Terminal on
   the agent interface): the protocol and its client(s); display lists, not
   video (whitepaper phase 4, principle 8: transport by SSH or WireGuard,
   logins through vdm). Plan: docs/remote-milestone-1.md (a remote screen
   with three modes: extend, mirror, take over; the Vergence side and client
   first, all three modes, attach and own login), then a portable build of
   Vergence apps with the client as its first (the Mac first).
3. **AlbumWall for Vergence** (generic name Music): AlbumWall's wall of
   covers and player rebuilt in Rust on the toolkit and Hymie
   (crates/vergence-hymie), for Vergence (docs/apps-roadmap.md).
4. **The web component**: WebKit (WPE) as a component any app can embed,
   likely a service the server composites; then Mail and a browser on it.
5. **The Vergence system** (release pathway §4): our installer and ISO on a
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
| Terminal agent | Terminal on the agent interface: tabs read and typed in by grant per tab, the agent's own window, refusals at password prompts and root tabs, marks in the bar, Escape twice to stop | terminal-agent-milestone-1 |
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

## Known issues

- **Avalonia apps on Wayland show the old X11 pointer** (AlbumWall on
  techbench, 2026-11-22): Avalonia's Wayland backend loads the pointer theme
  with no name at size 24 (WaylandCursorManager.cs), ignoring XCURSOR_THEME,
  so libwayland-cursor falls back to its built-in cursors. Later: a "default"
  theme inheriting Vergence's at login, and an upstream Avalonia change
  (theme, size, scale, wp-cursor-shape-v1). Shared memory 01M837J58YTX473JVW9R2N90A7.
- **The HDMI/DisplayPort sound of a monitor is not offered** as an output on
  techbench yet (PipeWire picked the analog profile); choosing outputs later.

- **A GPU reset leaves the screen white** (kepler, 2026-11-22): when the
  monitor was plugged into a session that had run a day without one, drm-kmod
  reported repeated GPU hangs on the UHD 630 and reset the contexts of the
  server and the desktop's icons; the server kept running but drew only
  white until restarted. Two parts: the hang (drm-kmod; report kept,
  ~/vergence-gpu-reports, kepler-20261122-194358) and ours: the server does
  not notice a lost GL context and rebuild its renderer.
- **The UPS tile floods the log on kepler**: "vtile: more than 16 rows, 240
  chart values or 8 controls", about 8,500 lines in a day from the
  ups-tile lab script (a lab tile, not in packages).

- **A stale `/run/vdm/power-key`** (vdm killed and not restarted) makes
  acpid ignore the power key until a reboot (packaging review, L3).

## Not built yet

The drawer's filter field; a calendar in the clock's card; weather and
system-log tiles; docks on several screens; explicit sync and NVIDIA's own
driver (whitepaper §8.3); AMD testing (a Ryzen mini PC is on the wish list);
the rest of Smithay's replacement (docs/borrowed-review.md); connecting to servers
from FileView; packages.
