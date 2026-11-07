# Working in this repository

Vergence (named 2026-11-05, formerly "App Consistency"): the design is docs/Vergence-Whitepaper.md,
the current milestone is docs/milestone-2.md (milestone 1 in docs/milestone-1.md), status is in README.md.

- **Started 2026-11-06** (Dave's go). The source lives here on Control (the
  only git copy; Control has no Rust). Build and test on Hubble, the FreeBSD
  dev box: copy the tree over SSH without .git and target, then run
  `cargo test` and `DISPLAY=:0 target/debug/vergd probe` there. Fake monitors
  for tests: docs/milestone-1.md, "Found on Hubble".
- **vergd runs in Dave's Xfce session on Hubble** (tools/install-autostart.sh:
  ~/.local/bin/vergd, log ~/.local/state/vergence/vergd.log, profiles
  ~/.config/vergence/profiles.toml). Before a test that plugs fake monitors,
  stop it or expect it to act; after rebuilding, rerun the install script to
  update the running copy.
- **Names:** the project and its crates are `vergence` / `vergence-*`; the
  phase 1 daemon is `vergd`. "Verge" for short in conversation.
- **Rust throughout** (whitepaper §8): x11rb for phase 1, Smithay for the
  server, Slint (or iced) for tools, Vello for display lists. EDID, profile and
  rules logic live in library crates the server reuses.
- **FreeBSD first (bare hardware, X11), Debian 13 (X11, XFCE) second, both
  at every phase.** Dave, 2026-11-06: development happens on FreeBSD; it was
  Debian first until then. Stay with the X protocol (RandR) rather than
  kernel or Linux interfaces (udev, systemd, /sys); if something must
  differ, say so in the code and the docs. VMs (canary) are for build checks
  only.
- **Phase 1 security rules** (whitepaper §7.3): no network socket, no
  unauthenticated control interface, saved state owner-only (0600).
- **Local repository only, no remote yet.** If it ever goes on GitHub, the
  rules of the app repos apply: author email is the no-reply address
  (298062495+gsa700@users.noreply.github.com), no LAN addresses, no personal
  email, keys or tokens in any file or commit.
- **The whitepaper** is edited here (docs/Vergence-Whitepaper.md). After an
  edit, rebuild the PDF with docs/make-pdf.sh on Control: build-pdf.py makes
  the HTML (python-markdown in a throwaway venv), html2pdf.swift prints it
  with WebKit. No Homebrew. On Windows, Edge can print build-pdf.py's HTML.
  Look at the pages before committing (a cover too tall for the page spills
  onto a blank page 2).
