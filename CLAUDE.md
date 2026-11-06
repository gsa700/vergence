# Working in this repository

Vergence (named 2026-11-05, formerly "App Consistency"): the design is docs/Vergence-Whitepaper.md,
the current milestone is docs/milestone-1.md, status is in README.md.

- **Not started on purpose.** Dave, 2026-11-05: set up on Control, but build
  on real hardware later, so the work isn't spent around QEMU's quirks. Don't
  start writing the daemon until he says so.
- **Names:** the project and its crates are `vergence` / `vergence-*`; the
  phase 1 daemon is `vergd`. "Verge" for short in conversation.
- **Rust throughout** (whitepaper §8): x11rb for phase 1, Smithay for the
  server, Slint (or iced) for tools, Vello for display lists. EDID, profile and
  rules logic live in library crates the server reuses.
- **Debian 13 (X11, XFCE) first, FreeBSD second, both at every phase.**
  Nothing Linux-only where FreeBSD has an equivalent; if something must
  differ, say so in the code and the docs.
- **Phase 1 security rules** (whitepaper §7.3): no network socket, no
  unauthenticated control interface, saved state owner-only (0600).
- **Local repository only, no remote yet.** If it ever goes on GitHub, the
  rules of the app repos apply: author email is the no-reply address
  (298062495+gsa700@users.noreply.github.com), no LAN addresses, no personal
  email, keys or tokens in any file or commit.
- **The whitepaper** is edited here (docs/). The PDF is made by
  docs/build-pdf.py, which renders HTML that Edge prints to PDF (Mobilenode
  made the first one).
