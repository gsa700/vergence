# Vergence

Vergence is the movement of both eyes converging on one point so that you see
one clear image. This project does the same for a desktop: every monitor and
every toolkit converge on one answer, so every application is the right size,
in the right place and readable, on any monitor setup, for every user.

The design is in
[docs/Vergence-Whitepaper.md](docs/Vergence-Whitepaper.md)
(PDF beside it; `docs/make-pdf.sh` rebuilds it on a Mac), written while the project was still called "App
Consistency"; it was named Vergence on 2026-11-05.

**Status (2026-11-06): phase 1 milestones 1 to 4 built** on real hardware,
a FreeBSD 15.1 X11 machine (Intel UHD 630) with a real monitor and debugfs
fake monitors of any density, not a virtual machine. `vergd` recognizes each
monitor setup by its EDIDs, puts its arrangement back, puts every window back
(also across scale changes), is the desktop's XSETTINGS manager, and scales
in Mode A (everything the same physical size: a 200-unit box measured 52.9 mm
on a 103 ppi and a 163 ppi monitor in GTK 3/4, Qt, Electron and Avalonia) or
Mode B (native pixels, per-monitor factors). Phase 1 was frozen on
2026-11-07 and `vergd` retired on 2026-11-10; git keeps it.

Crates: `vergence-edid` (EDID parser), `vergence-profile` (profile keys,
arrangements, window memory, scaling), `vergence-xsettings` (XSETTINGS wire
format), `vergence-harness` (EDID sweeps, fake
monitors, size measurement), `vergence-server` (the phase 2 server),
`vergence-protocol` (the native protocol: session bus messages, wire format,
the `vbus` tool), `vergence-dbus` (our own D-Bus wire protocol, for the bus's
D-Bus edge), `vergence-settings` (the settings app), `vergence-net` (wpa_supplicant client, interfaces), `vergence-power` (batteries, GPU sleep, battery history), `vergence-term` (the terminal); the toolkit, **vergence-ui**: `vergence-ui` (a window, its input and frames), `vergence-widgets`, `vergence-scene` (the display list), `vergence-render` (GPU and CPU reference renderers), `vergence-text`, `vergence-wl` (our own Wayland client code). `tools/sizebox`: one size-box program per
toolkit.

| Phase | What | Status |
|---|---|---|
| 1 | X11 consistency daemon: EDID profiles, scaling modes, layout memory | Milestone 1 done except real monitors and an automated sleep loop ([docs/milestone-1.md](docs/milestone-1.md)); milestone 2 built and running on Hubble ([docs/milestone-2.md](docs/milestone-2.md)); milestone 3, window memory, acceptance 1-7 pass ([docs/milestone-3.md](docs/milestone-3.md)); milestone 4, scaling, built: Mode A and B, acceptance 1-7 pass except Java and Qt-in-Mode-B findings ([docs/milestone-4.md](docs/milestone-4.md)); frozen since 2026-11-07 except blockers |
| 2 | Display server | The daily session on Void (Hubble), FreeBSD (kepler) and Fedora (Techbench), every change tested on all three ([docs/server-milestone-1.md](docs/server-milestone-1.md)). Done (whitepaper §9): the session bus ([docs/bus-milestone-1.md](docs/bus-milestone-1.md), 2026-11-08); the toolkit, vergence-ui, with the terminal and the settings app on it, iced removed ([docs/toolkit-milestone-1.md](docs/toolkit-milestone-1.md), [docs/toolkit-milestone-2.md](docs/toolkit-milestone-2.md), 2026-11-10); the native window protocol ([docs/native-milestone-1.md](docs/native-milestone-1.md), 2026-11-11); sound ([docs/sound-milestone-1.md](docs/sound-milestone-1.md), 2026-11-11); our own seat ([docs/seat-milestone-1.md](docs/seat-milestone-1.md), 2026-11-12); the server-drawn bar with tabs and window memory ([docs/bar-milestone-1.md](docs/bar-milestone-1.md), 2026-11-12); the dock and session restore ([docs/shelf-milestone-1.md](docs/shelf-milestone-1.md)); the Apps launcher ([docs/shelf-milestone-2.md](docs/shelf-milestone-2.md), 2026-11-13); the frosted look, light and dark, rounded corners ([docs/look-milestone-1.md](docs/look-milestone-1.md), 2026-11-13); snapping, window keys and Alt+Tab ([docs/windows-milestone-1.md](docs/windows-milestone-1.md), 2026-11-14); dialogs, palettes, size limits, the sidebar window and wallpaper tint ([docs/window-styles-milestone-1.md](docs/window-styles-milestone-1.md), 2026-11-14); the settings app in groups with search, Wallpaper, Accessibility and input pages by kind, Wi-Fi following the cable, the Network page ([docs/settings-milestone-1.md](docs/settings-milestone-1.md), 2026-11-14; settings 2 (resolution and refresh rate, the same size on every screen, cable speed and traffic, energy on battery and on power, Notifications with per-app switches, forgetting window places, the terminal's close question; [docs/settings-milestone-2.md](docs/settings-milestone-2.md), 2026-11-14); `vergence-net`, our own wpa_supplicant client); screens on a second GPU, drawn by the main one and copied over, and the Battery page ([docs/gpus-milestone-1.md](docs/gpus-milestone-1.md), 2026-11-14; `vergence-power`); app settings: the app button in every window's bar, commands with roles, each app's settings window, the Apps page, the desktop's Style as the master switch for light and dark ([docs/app-settings-milestone-1.md](docs/app-settings-milestone-1.md), 2026-11-14); the first tools: Calculator, Force Quit, Hardware ([docs/tools-milestone-1.md](docs/tools-milestone-1.md), 2026-11-14/15); the Text Editor, with the toolkit's text area, completion, spelling (`vergence-spell`, our own checker, the word list from SCOWL) and the Open/Save dialog ([docs/editor-milestone-1.md](docs/editor-milestone-1.md), 2026-11-15); files: our own file types and default apps, the file chooser for other programs (the portal), pictures decoded apart (`vergence-decode`), Close-Up and File Viewer, thumbnails shared with other desktops (`vergence-thumbs`), drag and drop, network mounts and removable drives (`vergence-media`: udisks2 on Linux, the automounter on FreeBSD), a desktop of icons, Exit Vergence that closes the apps first ([docs/files-milestone-1.md](docs/files-milestone-1.md), 2026-11-15). Open: hand checks in the seat, shelf and look milestones; live tiles; the rest of Smithay's replacement; explicit sync and NVIDIA's own driver; packages |
| 3 | Accessibility contract, display-list surfaces, Avalonia backend | Research |
| 4 | Remote display over display lists | Research |

Language: Rust throughout (whitepaper §8). The terminal has been on the project's own
toolkit, vergence-ui, since 2026-11-10 (whitepaper §6.10; docs/toolkit-milestone-1.md), and the settings app since the same evening (docs/toolkit-milestone-2.md); iced is gone. Platforms (since 2026-11-09): a
Linux without systemd, Void on Hubble, for development; FreeBSD (kepler, a
second Lenovo Tiny) and Fedora (Techbench) at every
milestone. Install on a system with `sudo tools/install.sh` after
`cargo build --release` (see the script's header). Whitepaper Draft 0.20.

## License

Copyright 2026 David Erickson.

Vergence is licensed under either of

- the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)), or
- the GNU General Public License, version 2 or (at your option) any later
  version ([LICENSE-GPL](LICENSE-GPL)),

at your option (SPDX: `Apache-2.0 OR GPL-2.0-or-later`). Apache-2.0 for
permissive use, as in the Rust and BSD worlds, with its patent grant; the GPL
for GPL projects, including GPLv2 ones, which cannot take Apache-2.0 code.
Both come with no warranty and no liability for the authors.

Unless you say otherwise, a contribution you submit is licensed the same way,
with no additional terms.

The code Vergence is built with keeps its own licenses: see
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md), generated by
`tools/notices.py` (run it after changing dependencies). Code copied in keeps
its license file beside it (crates/vergence-server/LICENSE-smithay.txt,
third_party/drm-rs/LICENSE).
