# Working in this repository

Vergence (named 2026-11-05, formerly "App Consistency"): the design is docs/Vergence-Whitepaper.md,
the current milestone is docs/bus-milestone-1.md (the session bus; the server's is docs/server-milestone-1.md; phase 1's, frozen, are docs/milestone-1.md to -4.md), status is in README.md.

- **Started 2026-11-06** (Dave's go). The source lives here on Control (the
  main git copy, pushed to GitHub; Control has no Rust). Build and test on Hubble, the FreeBSD
  dev box: `tools/sync-hubble.sh` (rsync; keeps Hubble's build output), then run
  `cargo test` and `DISPLAY=:0 target/debug/vergd probe` there. Fake monitors
  for tests: docs/milestone-1.md, "Found on Hubble".
- **vergd runs in Dave's Xfce session on Hubble** (tools/install-autostart.sh:
  ~/.local/bin/vergd, log ~/.local/state/vergence/vergd.log, profiles
  ~/.config/vergence/profiles.toml, windows.toml, settings.toml,
  session.env). Before a test that plugs fake monitors, stop it or expect it
  to act; after rebuilding, rerun the install script to update the running
  copy. It is the XSETTINGS manager (Xfce's Appearance dialog no longer
  applies); run `xfconf-query` only with the session's
  DBUS_SESSION_BUS_ADDRESS, or a stray xfconfd appears.
- **The phase 2 server on Hubble's real monitors**: Hubble has no display
  manager since 2026-11-07 (lightdm disabled: its Wayland sessions were
  fragile on FreeBSD without logind; `sudo sysrc lightdm_enable=YES` brings it
  back). Log in on the text console, then `vergence` (~/bin link to
  tools/vergence-session; Ctrl+Alt+Backspace ends it) or `xfce`
  (tools/start-xfce: loads ~/.xprofile, then startxfce4). Log
  ~/vergence-drm.log. Wallpaper: ~/.config/vergence/desktop.toml. Over SSH, `~/bin/vctl` (tools/vctl; finds the
  socket in /var/run/xdg/<user> or /tmp/xdg-<uid>) drives it: `vctl shot` then copy
  ~/vergence-shots/*.png back to look, `vctl windows`, `vctl spawn
  tools/xtest.sh`, `vctl move`, and `vctl restart` after a rebuild (no hand at
  the console needed). Window indices are the stacking order (they change on
  focus).
- **Hubble is a project machine** (Dave, 2026-11-06): breaking its session
  for the project is fine; his accounts and data still need care. Fake
  EDIDs: `vergence-harness fake` (in /tmp on Hubble: fake1, fake4k, fake24,
  fake13). Avalonia on FreeBSD needs the natives in ~/vergence-native
  (BsdProbe's libSkiaSharp 119, libc-epoll, HarfBuzz link).
- **Names:** the project and its crates are `vergence` / `vergence-*`; the
  phase 1 daemon is `vergd`. "Verge" for short in conversation.
- **Rust throughout** (whitepaper §8). EDID, profile and rules logic live in
  library crates the server reuses.
- **No borrowing unless absolutely necessary, and always ask Dave first**
  (Dave, 2026-11-08; replaces "borrow first, replace later"). Before adding
  any part from another project (crate, program, protocol extension added only
  for borrowed clients, borrowed panel or daemon), ask him: what it is, why it
  is necessary, what writing our own would take. Parts already in (Smithay,
  libinput/seatd/xkbcommon, Xwayland, iced (replaced Slint), ab_glyph, resvg, Adwaita cursors;
  table in whitepaper §8) stay until he decides on each. OS interfaces (DRM/KMS,
  evdev, GBM, EGL, Vulkan) are not borrowing.
- **FreeBSD first (Hubble, bare hardware), Linux second, both at every
  phase.** Dave, 2026-11-06: development happens on FreeBSD (it was Debian
  first until then). Dave, 2026-11-08: the Linux platform is **Fedora on
  Techbench** (Fedora 45, Intel UHD 770), replacing Debian 13, which had been
  picked for hosting the X11 daemon. Techbench is Dave's workstation, not a
  project machine: Rust and -devel packages only (dnf needs his sudo), build
  in ~/src/vergence (`tools/sync-hubble.sh techbench`). Start it from GDM:
  "Vergence" in the gear menu (tools/install-session.sh put
  /usr/share/wayland-sessions/vergence.desktop there, 2026-11-08); log out of
  GNOME first, GDM won't start a second session for the same user. NOT from a
  text console: Fedora 45's consoles are kmscon, which holds the GPU (the
  server fails "Device or resource busy"); the kernel console is a dummy.
  Ctrl+Alt+Backspace ends it, back to GDM. Log ~/vergence-drm.log, `vctl` as on
  Hubble (socket in /run/user/1000). Techbench is normally headless: the two
  monitors move between it and Hubble. Keep
  Linux-only interfaces (udev, systemd, logind, /sys) out of the core; where
  the platforms must differ, say so in the code and the docs. VMs (canary)
  are for build checks only.
- **Xfce is only a host** (Dave, 2026-11-06): nothing new is built on
  Xfce's internals (xfconf, xfsettingsd); everything works under any EWMH
  window manager, and pieces the phase 2 server needs (e.g. the XSETTINGS
  manager) are written to carry over. Xfce stays the desktop on Hubble until
  the server replaces it.
- **Security rules** (whitepaper §7.3): no network socket, no
  unauthenticated control interface, saved state owner-only (0600). The
  server's sockets (control, bus, D-Bus edge) check the peer's uid. The
  control socket (`vctl`) exists only when the server starts with
  `VERGENCE_CONTROL=1` (the launchers in tools/ set it; a nested run needs it
  by hand): it bypasses every §7.3 mitigation for any program running as the
  user. Security pass 2026-11-08: docs/security-pass-2026-11-08.md; the
  fuzz tests (vergence-protocol, vergence-dbus, vergence-term) run with
  `cargo test`; `FUZZ_ITERS=<n>` for a long run.
- **GitHub: private `gsa700/vergence`** (origin, created 2026-11-06 at
  Dave's go). Push after committing. Going public is Dave's call (needs
  license files; the whitepaper byline has his callsign, the ASUS test EDID
  its serial). The rules of the app repos apply: author email is the no-reply address
  (298062495+gsa700@users.noreply.github.com), no LAN addresses, no personal
  email, keys or tokens in any file or commit.
- **The whitepaper** is edited here (docs/Vergence-Whitepaper.md). After an
  edit, rebuild the PDF with docs/make-pdf.sh on Control: build-pdf.py makes
  the HTML (python-markdown in a throwaway venv), html2pdf.swift prints it
  with WebKit. No Homebrew. On Windows, Edge can print build-pdf.py's HTML.
  Look at the pages before committing (a cover too tall for the page spills
  onto a blank page 2).
