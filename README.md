# Vergence

Vergence is the movement of both eyes converging on one point so that you see
one clear image. This project does the same for a desktop: every monitor and
every toolkit converge on one answer, so every application is the right size,
in the right place and readable, on any monitor setup, for every user.

The design is in
[docs/Vergence-Whitepaper.md](docs/Vergence-Whitepaper.md)
(the PDF beside it is still Draft 0.3, from before the rename, until it is rebuilt), written while the project was still called "App
Consistency"; it was named Vergence on 2026-11-05.

**Status (2026-11-05): set up, not started.** The first build waits for real
hardware: a Debian 13 X11 machine with real monitors, so the work is not spent
on a virtual machine's synthetic EDIDs and hot-plug quirks. FreeBSD is tested
alongside at every phase.

| Phase | What | Status |
|---|---|---|
| 1 | X11 consistency daemon: EDID profiles, scaling modes, layout memory | Milestone 1 planned ([docs/milestone-1.md](docs/milestone-1.md)) |
| 2 | Display server (Smithay) | Design only |
| 3 | Accessibility contract, display-list surfaces, Avalonia backend | Research |
| 4 | Remote display over display lists | Research |

Language: Rust throughout (whitepaper §8). Platforms: Debian first, FreeBSD
second, both from day one.
