# Vergence

Vergence is the movement of both eyes converging on one point so that you see
one clear image. This project does the same for a desktop: every monitor and
every toolkit converge on one answer, so every application is the right size,
in the right place and readable, on any monitor setup, for every user.

The design is in
[docs/Vergence-Whitepaper.md](docs/Vergence-Whitepaper.md)
(PDF beside it; `docs/make-pdf.sh` rebuilds it on a Mac), written while the project was still called "App
Consistency"; it was named Vergence on 2026-11-05.

**Status (2026-11-06): milestone 2 built.** `vergd probe` and `vergd watch`
work on real hardware, a FreeBSD 15.1 X11 machine (Intel UHD 630) with real monitors and
debugfs fake monitors, not a virtual machine. Next: the rest of the milestone 1 acceptance runs and the test harness. Debian 13
is tested alongside at every phase.

Crates: `vergence-edid` (EDID parser), `vergence-profile` (profile key),
`vergd` (the daemon).

| Phase | What | Status |
|---|---|---|
| 1 | X11 consistency daemon: EDID profiles, scaling modes, layout memory | Milestone 1 done except real monitors and an automated sleep loop ([docs/milestone-1.md](docs/milestone-1.md)); milestone 2 built and running on Hubble ([docs/milestone-2.md](docs/milestone-2.md)); milestone 3, window memory, planned ([docs/milestone-3.md](docs/milestone-3.md)); scaling is milestone 4 |
| 2 | Display server (Smithay) | Design only |
| 3 | Accessibility contract, display-list surfaces, Avalonia backend | Research |
| 4 | Remote display over display lists | Research |

Language: Rust throughout (whitepaper §8). Platforms: FreeBSD first, Debian
second, both from day one (FreeBSD made primary 2026-11-06, whitepaper
Draft 0.6).
