# Working in this repository

Vergence (named 2026-11-05, formerly "App Consistency"): the design is docs/Vergence-Whitepaper.md,
the current milestone is docs/bus-milestone-1.md (the session bus; the server's is docs/server-milestone-1.md; phase 1's, frozen, are docs/milestone-1.md to -4.md), status is in README.md.
The shell (whitepaper §6.9, decided 2026-11-09) is docs/shelf-milestone-1.md: steps 1 to 5 are in (the dock, applications, clicks, minimise, pins, session restore, the apps button and popup; crates/vergence-shell, the server's shelf.rs); step 6 waits for the settings app's move. The current work since 2026-11-10 is docs/toolkit-milestone-2.md: widgets (crates/vergence-widgets), the settings app moved onto them, iced removed; its Progress says how far it has come.
**Order of work since 2026-11-09 (whitepaper §9):** our own toolkit's drawing and text core with the terminal on it (§6.10; docs/toolkit-milestone-1.md, closed 2026-11-10: the terminal, crates/vergence-term, is on it and its iced interface is gone; the look both the server and the toolkit draw with is crates/vergence-look, and the window bar is vergence_ui::bar), then the shelf on the same drawing code, then widgets and the settings app (docs/toolkit-milestone-2.md; crates/vergence-widgets), then iced is removed (done 2026-11-10). GPU first; a CPU reference renderer for tests and GPU-less machines (canary).

- **Started 2026-11-06** (Dave's go). The source lives here on Control (the
  main git copy, pushed to GitHub; Control has no Rust). Build and test on the
  dev boxes: `tools/sync-hubble.sh [host]` (rsync; keeps the host's build output;
  default hubble, also kepler, canary and techbench), then `cargo test --workspace`
  there. Fake monitors for tests: docs/milestone-1.md, "Found on Hubble".
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
  ~/.local/state/vergence/server.log (was ~/vergence-drm.log until 2026-11-09).
  Wallpaper: ~/.config/vergence/desktop.toml. Over SSH, `~/bin/vctl` (tools/vctl; finds the
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
  libinput/seatd/xkbcommon, Xwayland, ab_glyph, resvg, Adwaita cursors;
  table in whitepaper §8) stay until he decides on each. OS interfaces (DRM/KMS,
  evdev, GBM, EGL, Vulkan) are not borrowing.
- **Three platforms, one code base** (Dave, 2026-11-09; replaces "FreeBSD
  first" of 2026-11-06). The primary development platform is **Linux without
  systemd: Void (glibc) on Hubble**, reinstalled from FreeBSD on 2026-11-09
  because the work ahead (GPUs, several cards, explicit sync, suspend soak) is
  where FreeBSD's drm-kmod lagged and its tools are thin. **FreeBSD** stays a
  real-hardware platform on **kepler**, the second Lenovo Tiny (ThinkCentre
  M910q, i5-6500T, Intel HD 530, 16 GB; FreeBSD 15.1 on ZFS, installed
  2026-11-10; drm-kmod with `kld_list="i915kms"`, seatd, Rust by rustup so
  `. ~/.cargo/env` over ssh; `tools/sync-hubble.sh kepler`; passwordless
  sudo; Vergence installed under /usr/local, `vergence` from its text
  console starts the session, and ~/.config/vergence/control-socket is
  there so `vctl` works), and a build-and-test gate on the VM canary
  (`tools/sync-hubble.sh canary`; passwordless sudo there; no display
  device, so a session there is the server nested in its Xfce: see
  docs/toolkit-milestone-1.md, "canary in software"). The second Linux is
  **Fedora on
  Techbench** (Fedora 45, Intel UHD 770), replacing Debian 13, which had been
  picked for hosting the X11 daemon. Techbench is Dave's workstation, not a
  project machine: Rust and -devel packages only (dnf needs his sudo), build
  in ~/src/vergence (`tools/sync-hubble.sh techbench`). Start it from GDM:
  "Vergence" in the gear menu (tools/install-session.sh put
  /usr/share/wayland-sessions/vergence.desktop there, 2026-11-08); log out of
  GNOME first, GDM won't start a second session for the same user. NOT from a
  text console: Fedora 45's consoles are kmscon, which holds the GPU (the
  server fails "Device or resource busy"); the kernel console is a dummy.
  Ctrl+Alt+Backspace ends it, back to GDM. Log ~/.local/state/vergence/server.log, `vctl` as on
  Hubble (socket in /run/user/1000). Techbench is normally headless: the two
  monitors move between it and Hubble. Keep
  Linux-only interfaces (udev, systemd, logind, /sys) out of the core; where
  the platforms must differ, say so in the code and the docs. VMs (canary)
  are for build checks only. A milestone is done when it builds and passes
  its tests on all three and has run on the FreeBSD and Fedora machines.
- **Installing on a system**: `cargo build --release`, then `sudo tools/install.sh`
  (binaries, `vergence-session` and the `vergence` console command in
  /usr/local/bin, a Wayland session file, seatd enabled where there is no
  logind, the user in the seat group; `--prefix`, `--destdir` for packages,
  `--uninstall`). An installed session has NO control socket: for `vctl`,
  create ~/.config/vergence/control-socket or start with VERGENCE_CONTROL=1. A
  checkout's tools/vergence-session is a development session (socket on).
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
