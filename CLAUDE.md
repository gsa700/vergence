# Working in this repository

Vergence (named 2026-11-05, formerly "App Consistency"): the design is docs/Vergence-Whitepaper.md (Draft 0.20, 2026-11-15; §9 has the status and the order of work), status is also in README.md, and each milestone has its plan and progress in docs/*-milestone-*.md. Phase 1 (docs/milestone-1.md to -4.md, the X11 daemon vergd) is frozen and its daemon retired. Phase 2, the server, is the daily session on Hubble, kepler and Techbench.

**Phase 2 milestones, as of 2026-11-14:**
- Done and closed: the session bus (docs/bus-milestone-1.md; the server's bring-up is docs/server-milestone-1.md); toolkit 1 and 2 (docs/toolkit-milestone-1.md, -2.md, 2026-11-10: vergence-ui with crates/vergence-scene, -render, -text, -wl, -widgets; the terminal, crates/vergence-term, and the settings app on it; iced removed; two items carried forward, see toolkit-milestone-2's header); native windows (docs/native-milestone-1.md, 2026-11-11: crates/vergence-server/src/native.rs, the toolkit's native road crates/vergence-ui/src/native.rs, `VERGENCE_UI_WAYLAND=1` for the Wayland road, placement requests and memory, the meaning tree, `vctl tree`; under GNOME the toolkit draws its own bar and border); the bar (docs/bar-milestone-1.md, 2026-11-12: the server draws every native window's bar, tabs included, from crates/vergence-bar, bus protocol v2, persistence tokens, one terminal process for all its windows, tab dragging and menus, window memory per resolution and scale of a setup; under GNOME our bars follow GNOME's button layout; tab dragging under other desktops carried forward).
- All steps in: sound (docs/sound-milestone-1.md, 2026-11-11: crates/vergence-sound, OSS and ALSA back ends, the FreeBSD root helper libexec/vergence/sound-default for the default output only, volume keys 5%; PipeWire not yet; kepler's sound works (2026-11-14: its front headphone jack, cabled to the Audient iD24 on Control, was never broken; the iD24's inputs were muted on the Mac), so the Sound page and volume keys can be tried there on real FreeBSD hardware).
- All steps in, checks by hand still open: seat 1 (docs/seat-milestone-1.md, 2026-11-12: every pointer, keyboard and touch event goes through crates/vergence-seat, decisions with no Smithay, and the server's dispatch.rs; nothing of ours runs in Smithay's KeyboardTarget callbacks, which hold the keyboard's lock: the server froze there on 2026-11-11; by hand: key repeat, Alt+drag, Firefox, X11, clipboard, drag and drop, dragging and hiding docks, notification cards); shelf 1 (docs/shelf-milestone-1.md: the dock, applications, minimize, pins, session restore, Dock page, hiding, Terminal=true entries through `vergence-term -e`; crates/vergence-shell, the server's shelf.rs; acceptance 1, 3 and 6 wait for two monitors on one machine; docks on several screens parked by Dave); shelf 2, the Apps launcher (docs/shelf-milestone-2.md, 2026-11-13: launcher.rs in the server and vergence-shell; Super's tap and Hubble at scale 1 are Dave's); look 1 (docs/look-milestone-1.md, 2026-11-13: the frost, a blur pass in the server's GPU renderer, Clear/Standard/Heavy, Reduce transparency; light and dark with contrast floors; content under the frosted bar; red tab for root; rounded corners at 5 pt; control sizes; crates/vergence-look; corner resizing and the red tab on kepler are Dave's to try).
- Windows milestone 1 (docs/windows-milestone-1.md, closed 2026-11-14): snapping in crates/vergence-shell snap.rs (zones, gaps, the window keys' rule) and the server's snap.rs (desktop.toml `[windows]`: snap off/ctrl/always, default always since 2026-11-14 evening, "ctrl" shows a faint preview saying to hold Ctrl; top-edge, between-screens, preview, gaps, layouts-hover, window-keys, app-switch, alt-tab), the window's layouts menu (menu.rs, `open_window_menu`), Alt+Tab and its list (switch.rs), new windows and the keyboard (opened.rs), the settings app's Windows page "Snapping and switching". vctl: used, switching, snap, hold/let-go, chords like alt+Tab+Tab.
- Window styles milestone 1 (docs/window-styles-milestone-1.md, 2026-11-14, all seven steps in): kinds and limits (bus protocol v4 `Style`; the server's style.rs `window_style` for every road), limits kept (maximizable, within_limits), dialogs and palettes (dialogs.rs), the sidebar window (v5 `Bar::Tools`, `BarButton`; vergence-bar `tools_sized`), wallpaper tint (v6 `Tint`; tint.rs, wallpaper.rs grids); full screen (fullscreen.rs). The example `vdialog` (vergence-widgets) opens a dialog, a modal one and a palette.
- Settings milestone 1 (docs/settings-milestone-1.md, 2026-11-14, all steps in; the iPhone by USB cable parked by Dave: iOS needs usbmuxd-style pairing): grouped sidebar, Accessibility, Wallpaper, Advanced parts, input pages by kind (v7 topic `inputs`; the server's inputs.rs, ~/.local/state/vergence/inputs.toml), crates/vergence-net (wpa_supplicant client, interfaces; `--example vnet`), Wi-Fi (the server's network.rs follows the cable by desktop.toml `[network] wifi`; the settings app's wifi.rs; install.sh enables wpa_supplicant where there is a card; Hubble's is enabled since 2026-11-14). Bus protocol is **v18** (files milestone 1: v18 `WinRequest::ContextMenu`, v17 the Desktop window kind, v16 native drag and drop and Activate, v15 `Open`; v14 editor milestone 1: `Tab::edited`).
- Settings milestone 2 (docs/settings-milestone-2.md, closed 2026-11-14): bus v9 (`OutputInfo::modes`/`mode`, `ListPlaces`/`ForgetPlaces`/`Places`); resolution and refresh rate (settings modes.rs, `[[displays.screen]] mode = "WxH@R"`, keep-or-revert dialog; server parse_mode/mode_for); `[displays] same-size`; vergence-net link speed/duplex and the Network page's traffic; Wi-Fi hidden without hardware; `[energy.battery]` (server energy.rs follows vergence-power's `Power::on_battery`); the Notifications page (settings notify.rs; server notify.rs `[notifications]` do-not-disturb, seconds, corner, off; ~/.local/state/vergence/notify-apps.toml); forgetting window places (settings places.rs, dock's "Forget window places...", `vergence-settings --page <name> --forget <app>`); the terminal's close question (vergence-ui `App::close_asked`, vergence-term confirm.rs, which now depends on vergence-widgets); the settings app's bar light under "App default" (server decor.rs `bar_light`, a built-in list until per-app look); bars without tabs center their buttons; the toolkit keeps a gutter for a scroll bar.
- GPUs milestone 1 (docs/gpus-milestone-1.md, closed 2026-11-14; the CPU copy fallback carried over): every GPU probed (server gpus.rs, `vctl gpus`), screens on a second GPU drawn by the primary into a linear buffer and copied by the showing GPU's own renderer, damage only, pipelined (drm.rs `Relay`, `relay_frame`); Hubble's 14" on the NVIDIA HDMI needs NVK (`mesa-vulkan-nouveau`, `vulkan-loader`; install.sh says so); bus v8 `OutputInfo::shown_by`/`drawn_by`. `VERGENCE_RELAY_TIMES=1` logs every relay turn.
- App settings milestone 1 (docs/app-settings-milestone-1.md, 2026-11-14, built): the app button (vergence-bar `WinButton::AppMenu`, layout item `appmenu`, `with_app_menu` puts it opposite the controls; seat `Part::AppMenu`/`AppList`, `Menu::AppMenu`, Alt+Space), its menu (vergence-shell appmenu.rs: featured commands, pins, Text size row, Settings…, Keyboard shortcuts, About, Quit; the server's appmenu.rs draws it and keeps pins in ~/.local/state/vergence/app-menu.toml), commands with roles (vergence-protocol commands.rs `ROLES`: the server fixes names and keys; terminals get Shift; bus v10 `Commands`/`BarStyle`/`Command`, v11 `featured`), each app's settings window (vergence-widgets prefs.rs: inherit-or-override rows, `set_key` edits the app's desktop.toml table line by line; vergence-term prefs.rs, the settings app's OwnRows), per-app look (vergence-ui `content_light`, `bar_style_setting`: the desktop's Style forces all, App default lets each app choose; the settings app is light always), the Apps page and `vergence-settings --app <id>` (apps.rs; the server's decor.rs `app_bars` reads `[app."<id>"] bar-style` for apps not ours). The rule: the app button is the app, a right click is the thing under the pointer, the title bar is the window.
- Tools milestone 1 (docs/tools-milestone-1.md, built 2026-11-14/15; Hardware's pages on screen and the kepler/Techbench installs open): crates/vergence-calc (dec.rs exact decimals, expr.rs, the keypad window; bus v12 `WinRequest::Size`, vergence-widgets about.rs and `Program::key_first`), crates/vergence-quit (Force Quit; bus v13 `ListApps`/`Apps`/`ForceQuit`/`Alive`; the server's forcequit.rs: SIGTERM, 2 s, SIGKILL, "not responding" by the bus `Alive` and xdg ping; seat Ctrl+Alt+Escape), crates/vergence-sysinfo (Linux /proc and /sys, FreeBSD sysctlbyname, `df -kP`) and crates/vergence-hw (Hardware; vergence-widgets chart.rs). Each app has a desktop entry and icon in data/ and is listed in tools/install.sh. vctl type sends only small letters, digits, space - = , . ; click keypad buttons for the rest.
- Editor milestone 1 (docs/editor-milestone-1.md, built 2026-11-15, steps 1-8 in): vergence-widgets edit.rs (Buffer, History: undo for the area and every field), textarea.rs (TextDoc handle, text_area(&doc); only lines in view laid out; line numbers, wrap, indent, find_all/marks/replace_all, completion TextEdit-style with Words sources, spelling waves and a right-click menu; draws under the frosted bar with an inset worked out from Ui::under_bar, no ground of its own), filedialog.rs (the Open/Save dialog for every app; state in ~/.local/state/vergence/file-dialog), Element::keep_in_view, Program gains tab/close_asked/closed/opened/token/token_given, the host sends the title when it changes; crates/vergence-spell (our checker; data/words/en_US.txt from SCOWL by tools/make-words.py, installed in share/vergence/words; user words ~/.local/share/vergence/words.txt; desktop.toml `[text] spellcheck`, Settings > Keyboard > Spelling); crates/vergence-edit (the Text Editor: hand.rs one process, files.rs atomic saves, keep.rs unsaved work in ~/.local/state/vergence/edit; `[editor]` settings). Bus v14 `Tab::edited` (amber stripe, the server's). Server fixes with it: dialogs take the keyboard by their parent (opened.rs), the keyboard goes back to a closed window's parent or the window in front (native.rs), the round shader cuts an under-bar window's top corners (`top`). Gotchas: `Element::padding` pads only panels and buttons (a column ignores it); vergence-ui calls opened() before waker(), and one waker serves all of a process's windows; vctl over ssh has no XDG_RUNTIME_DIR (hand-over needs it). The editor's name is Dave's to give ("Text Editor" until then).
- Files milestone 1 (docs/files-milestone-1.md, built 2026-11-15, steps 1-11): crates/vergence-types (own type table, mimeapps lists, ~/.config/vergence-mimeapps.list first, then our own `vergence-*` apps, then other desktops' choices), vergence-open, the server's portal.rs (FileChooser via libexec vergence-choose, OpenURI; Settings > Apps `[apps] own-file-dialog`), crates/vergence-decode (the sandboxed picture helper, libexec), crates/vergence-closeup (Close-Up), crates/vergence-files (File Viewer: ops.rs file ops and Trash, search.rs, desktop.rs the desktop's icons as `vergence-files --desktop`, WinKind::Desktop), vergence-widgets files.rs (shared views, mount table, eject) and toolbar.rs, crates/vergence-thumbs (freedesktop thumbnails, own MD5), crates/vergence-media (udisks2 over vergence-dbus's client.rs on Linux; FreeBSD autofs with /etc/autofs/special_vergence_media from tools/freebsd/media-map and the setuid helper libexec media-eject), server drag.rs (native drag and drop), desktop.rs, exit.rs (Exit Vergence…), menu.rs's context menus (bus v18 `WinRequest::ContextMenu`); toolkit touch-and-hold = right click. vctl over ssh: `vctl touch down X Y` / `vctl touch up` simulate a finger; kepler's scale may be changed by Dave (check `vctl windows` before aiming).
- Not built yet: live tiles, docks on several screens, the rest of Smithay's replacement in docs/borrowed-review.md's order (the window space and the renderer next), explicit sync and NVIDIA's own driver (whitepaper §8.3), packages.

**Order of work since 2026-11-09 (whitepaper §9):** 1. the toolkit's drawing and text core with the terminal on it (done 2026-11-10); 2. the shell on the same drawing code (dock, session restore, launcher done; live tiles, snapping, keyboard switching left); 3. widgets and the settings app, iced removed (done 2026-11-10); 4. windows, placement and accessibility on the native protocol (done 2026-11-11, with the seat and the bar on them), explicit sync and the rest of §8.3 alongside (left). The look both the server and the toolkit draw with is crates/vergence-look. GPU first; a CPU reference renderer for tests and GPU-less machines (`VERGENCE_RENDER=cpu`). **Since 2026-11-15 the order alternates** (Dave: "a good way forward"): after each app or polish milestone, one milestone of the core vision (Contract 3, display lists, live tiles and named actions, §6.7), chosen where it also pays off daily; first proposed: live tiles with named desktop actions. Why and what is under-developed: docs/vision-check-2026-11-15.md (to be folded into the whitepaper's §9).

- **Started 2026-11-06** (Dave's go). The source lives here on Control (the
  main git copy, pushed to GitHub; Control has no Rust). Build and test on the
  dev boxes: `tools/sync-hubble.sh [host]` (rsync; keeps the host's build output;
  default hubble, also kepler and techbench), then `cargo test --workspace`
  there: Hubble, kepler and Techbench for every change. After adding a
  crate, a dependency or a version, copy Cargo.lock back from the build box
  (`scp hubble:src/vergence/Cargo.lock .`) and commit it: Cargo updates it
  there, never here (it was stale for three changes, 2026-11-11). Fake monitors for tests: docs/milestone-1.md, "Found on Hubble".
- **GPU hangs: keep the evidence** (2026-11-14). On kepler run tests as
  `tools/test-guard.sh 480 -- cargo test -q --workspace`: if they run over,
  it saves `tools/gpu-report.sh`'s report (each stuck process's threads and
  kernel stacks, the i915 error state, the kernel's log) before stopping
  them. Run `tools/gpu-report.sh [pid...]` by hand on any hang or freeze
  before killing or rebooting, then `tools/fetch-gpu-reports.sh` to copy the
  reports to Control (~/vergence-gpu-reports/; a reinstall lost the
  2026-11-10 dump). Found 2026-11-14: the vergence-render GPU test hung on
  kepler in the kernel (a page fault on a mapped GPU buffer in drm-kmod's
  linuxkpi path, `_vm_page_busy_sleep`, during glReadPixels in
  `a_buffer_for_the_display_server_holds_the_same_picture`), not a GPU
  reset: unkillable until reboot.
- **vergd, phase 1's X11 daemon, is retired** (Dave, 2026-11-10): removed
  from the workspace with tools/install-autostart.sh; git keeps it (last as
  `crates/vergd` in 4a2f00f). Its files' formats live on in vergence-profile
  and vergence-xsettings, which the server uses. **X11 applications are off
  by default** in a Vergence session since the same day: desktop.toml
  `[x11] enabled = true` (the settings app's Windows page) starts Xwayland.
- **The phase 2 server on Hubble** (Void since 2026-11-09; since 2026-11-12
  its disk runs in Mobilenode, the ASUS TUF F15: Intel UHD panel at 1080p
  144 Hz, GTX 1650 on the external outputs, dual boot with Windows): no
  display manager and no Xfce. Log in on the text console (agetty), then
  `vergence` (installed in /usr/local/bin; Ctrl+Alt+Backspace ends it). After
  a change: `tools/sync-hubble.sh`, `cargo build --release` there, `sudo
  tools/install.sh`, `vctl restart`. Log ~/.local/state/vergence/server.log;
  settings in ~/.config/vergence (desktop.toml, settings.toml; the
  control-socket file is there, so the installed session has `vctl`). Over
  SSH, `vctl` (tools/vctl, installed in /usr/local/bin; finds the socket in
  $XDG_RUNTIME_DIR, /tmp/xdg-<uid> on Void, /run/user/<uid> on Fedora,
  /var/run/xdg/<user> on FreeBSD) drives it: `vctl shot` then, after about 2 s
  (it returns before the PNG is written), copy ~/vergence-shots/*.png back to
  look; `vctl windows`, `vctl spawn`, `vctl move`, `vctl key`, `vctl restart`.
  Window indices are the stacking order (they change on focus).
- **Hubble is a project machine** (Dave, 2026-11-06): breaking its session
  for the project is fine; his accounts and data still need care (Dave
  2026-11-12: Hubble, kepler and Techbench are all set up for this work).
  Fake EDIDs are made when needed, none are kept: `vergence-harness fake
  --name N --product 0xNNNN --serial N --mode 4k30|1080p60 --size-mm WxH
  --out FILE` (the /tmp/fake* files on FreeBSD Hubble went with its
  reinstall, 2026-11-09). Avalonia apps on FreeBSD: parked (Dave,
  2026-11-11); BsdProbe's natives (~/vergence-native: libSkiaSharp 119,
  libc-epoll, HarfBuzz link) are on neither Hubble nor kepler since their
  reinstalls, and docs/milestone-4.md has what they were.
- **Names:** the project and its crates are `vergence` / `vergence-*`; the
  phase 1 daemon was `vergd` (retired). "Verge" for short in conversation.
- **Rust throughout** (whitepaper §8). EDID, profile and rules logic live in
  library crates the server reuses.
- **No borrowing unless absolutely necessary, and always ask Dave first**
  (Dave, 2026-11-08; replaces "borrow first, replace later"). Before adding
  any part from another project (crate, program, protocol extension added only
  for borrowed clients, borrowed panel or daemon), ask him: what it is, why it
  is necessary, what writing our own would take. Parts already in (Smithay,
  libinput/seatd/xkbcommon, Xwayland, ab_glyph, resvg, Adwaita cursors;
  table in whitepaper §8) stay until he decides on each. The list of code we
  did not write, for later review and possible rewrite: docs/borrowed-review.md
  (Dave, 2026-11-11); add to it when anything borrowed comes in. OS interfaces (DRM/KMS,
  evdev, GBM, EGL, Vulkan) are not borrowing.
- **Three platforms, one code base** (Dave, 2026-11-09; replaces "FreeBSD
  first" of 2026-11-06). The primary development platform is **Linux without
  systemd: Void (glibc) on Hubble**, reinstalled from FreeBSD on 2026-11-09
  because the work ahead (GPUs, several cards, explicit sync, suspend soak) is
  where FreeBSD's drm-kmod lagged and its tools are thin. **FreeBSD** stays a
  real-hardware platform on **kepler**: since 2026-11-12 the Lenovo Tiny that
  was Hubble's (i5-8500T, Intel UHD 630, the 14" 4K touch monitor; 10.0.1.31;
  FreeBSD 15.1 on ZFS reinstalled that day; the M910q with the HD 530 it ran on
  from 2026-11-10 is a spare). Its touch panel comes up in HID mouse mode on
  FreeBSD (touches as one absolute mouse): a devd rule,
  /usr/local/etc/devd/touchscreen-sw4101c.conf with
  /usr/local/libexec/touchscreen-mode, sets `dev.hconf.N.input_mode=2` each
  time it attaches. drm-kmod with `kld_list="i915kms"`, seatd, Rust by rustup so
  `. ~/.cargo/env` over ssh; `tools/sync-hubble.sh kepler`; passwordless
  sudo; Vergence installed under /usr/local, `vergence` from its text
  console starts the session, and ~/.config/vergence/control-socket is
  there so `vctl` works). **canary**, the FreeBSD VM that was the build gate
  while FreeBSD had no hardware, is **dropped** (Dave, 2026-11-10): kepler
  covers FreeBSD; the VM is not synced or tested any more. (It has no
  display device; a session there was the server nested in its Xfce, see
  docs/toolkit-milestone-1.md, "canary in software".) The second Linux is
  **Fedora on
  Techbench** (Fedora 45, Intel UHD 770), replacing Debian 13, which had been
  picked for hosting the X11 daemon. Techbench is Dave's workstation, not a
  project machine: Rust and -devel packages only (dnf needs his sudo), build
  in ~/src/vergence (`tools/sync-hubble.sh techbench`). Start it from GDM:
  "Vergence (dev)" in the gear menu runs this checkout (tools/install-session.sh,
  /usr/share/wayland-sessions/vergence-dev.desktop; renamed 2026-11-12), plain
  "Vergence" the installed build (tools/install.sh, run there 2026-11-12: no
  control socket, so no `vctl`); log out of
  GNOME first, GDM won't start a second session for the same user. NOT from a
  text console: Fedora 45's consoles are kmscon, which holds the GPU (the
  server fails "Device or resource busy"); the kernel console is a dummy.
  Ctrl+Alt+Backspace ends it, back to GDM. Log ~/.local/state/vergence/server.log, `vctl` as on
  Hubble (socket in /run/user/1000). Techbench is normally headless: the two
  monitors move between it and Hubble. Keep
  Linux-only interfaces (udev, systemd, logind, /sys) out of the core; where
  the platforms must differ, say so in the code and the docs. VMs are for
  build checks only. A milestone is done when it builds and passes
  its tests on all three and has run on the FreeBSD and Fedora machines.
- **Installing on a system**: `cargo build --release`, then `sudo tools/install.sh`
  (binaries, `vergence-session` and the `vergence` console command in
  /usr/local/bin, a Wayland session file, seatd enabled where there is no
  logind, the user in the seat group; `--prefix`, `--destdir` for packages,
  `--uninstall`). An installed session has NO control socket: for `vctl`,
  create ~/.config/vergence/control-socket or start with VERGENCE_CONTROL=1. A
  checkout's tools/vergence-session is a development session (socket on).
  **The server and the apps speak one exact bus protocol version** (the
  server refuses any other: vergence-protocol `bus::VERSION`, 18 since 2026-11-15 (files milestone 1's desktop menu; 17, 16 and 15 earlier that day); 13 since
  2026-11-14), so they are installed together (install.sh does) and, when
  packages come (xbps template, FreeBSD port, Fedora spec), **each requires
  the other at the same version** (Dave, 2026-11-14: "lets make sure they are
  each others deps"), or they are one package.
- **Xfce is only a host** (Dave, 2026-11-06): nothing new is built on
  Xfce's internals (xfconf, xfsettingsd); everything works under any EWMH
  window manager, and pieces the phase 2 server needs (e.g. the XSETTINGS
  manager) are written to carry over. No machine runs Xfce for the project
  any more: Hubble left it with FreeBSD on 2026-11-09.
- **Security rules** (whitepaper §7.3): no network socket, no
  unauthenticated control interface, saved state owner-only (0600). The
  server's sockets (control, bus, D-Bus edge) check the peer's uid. The
  control socket (`vctl`) exists only when the server starts with
  `VERGENCE_CONTROL=1` (the launchers in tools/ set it; a nested run needs it
  by hand): it bypasses every §7.3 mitigation for any program running as the
  user. Security pass 2026-11-08: docs/security-pass-2026-11-08.md; the
  fuzz tests (vergence-protocol, vergence-dbus, vergence-term) run with
  `cargo test`; `FUZZ_ITERS=<n>` for a long run.
- **License: `Apache-2.0 OR GPL-2.0-or-later`** (Dave, 2026-11-11): the
  workspace `license` field, LICENSE-APACHE, LICENSE-GPL, README's License
  section. After adding or updating a dependency, rerun `python3
  tools/notices.py` on a dev box (needs cargo) to regenerate
  THIRD-PARTY-NOTICES.md (`--check` says whether it is current). Before
  adding a dependency, check its license: anything copyleft (GPL, LGPL, MPL,
  EPL) or unusual needs Dave's word, besides the borrowing rule.
- **GitHub: private `gsa700/vergence`** (origin, created 2026-11-06 at
  Dave's go). Push after committing. Going public is Dave's call (the
  whitepaper byline has his callsign, the ASUS test EDID its serial). The rules of the app repos apply: author email is the no-reply address
  (298062495+gsa700@users.noreply.github.com), no LAN addresses, no personal
  email, keys or tokens in any file or commit.
- **The whitepaper** is edited here (docs/Vergence-Whitepaper.md). After an
  edit, rebuild the PDF with docs/make-pdf.sh on Control: build-pdf.py makes
  the HTML (python-markdown in a throwaway venv), html2pdf.swift prints it
  with WebKit. No Homebrew. On Windows, Edge can print build-pdf.py's HTML.
  Look at the pages before committing (a cover too tall for the page spills
  onto a blank page 2).
