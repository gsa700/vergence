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
fake monitors of any density, not a virtual machine. `vergd` recognises each
monitor setup by its EDIDs, puts its arrangement back, puts every window back
(also across scale changes), is the desktop's XSETTINGS manager, and scales
in Mode A (everything the same physical size: a 200-unit box measured 52.9 mm
on a 103 ppi and a 163 ppi monitor in GTK 3/4, Qt, Electron and Avalonia) or
Mode B (native pixels, per-monitor factors). Waiting for real monitors (a
14" 4K, a 32" 4K) for the final checks. Debian 13 is to be tested alongside.

Crates: `vergence-edid` (EDID parser), `vergence-profile` (profile keys,
arrangements, window memory, scaling), `vergence-xsettings` (XSETTINGS wire
format), `vergd` (the daemon), `vergence-harness` (EDID sweeps, fake
monitors, size measurement). `tools/sizebox`: one size-box program per
toolkit.

| Phase | What | Status |
|---|---|---|
| 1 | X11 consistency daemon: EDID profiles, scaling modes, layout memory | Milestone 1 done except real monitors and an automated sleep loop ([docs/milestone-1.md](docs/milestone-1.md)); milestone 2 built and running on Hubble ([docs/milestone-2.md](docs/milestone-2.md)); milestone 3, window memory, acceptance 1-7 pass ([docs/milestone-3.md](docs/milestone-3.md)); milestone 4, scaling, built: Mode A and B, acceptance 1-7 pass except Java and Qt-in-Mode-B findings ([docs/milestone-4.md](docs/milestone-4.md)); real monitors pending |
| 2 | Display server (Smithay) | Design only |
| 3 | Accessibility contract, display-list surfaces, Avalonia backend | Research |
| 4 | Remote display over display lists | Research |

Language: Rust throughout (whitepaper §8). Platforms: FreeBSD first, Debian
second, both from day one (FreeBSD made primary 2026-11-06). Whitepaper
Draft 0.7.
