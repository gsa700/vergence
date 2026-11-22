# Working in this repository (the lab's notes)

Vergence is a Rust desktop: its own display server, native protocol, toolkit
and apps, FreeBSD first and Void Linux second. Start here:

- **CONTRIBUTING.md**: building, running, testing, and the rules the code
  keeps (Rust throughout, our own parts unless borrowing is necessary and
  Dave agrees, the license, security §7.3, one bus protocol version,
  comments that explain why, American spelling, rustfmt at width 120).
- **STATUS.md**: where things stand and what is next. **ARCHITECTURE.md**:
  the map of the code. **docs/Vergence-Whitepaper.md**: the design (§9 the
  order of work). **docs/<area>-milestone-N.md**: each milestone's plan,
  Dave's answers and its progress. **docs/milestone-notes.md**: the dense
  per-milestone technical notes (files, environment variables, gotchas)
  this file carried until 2026-11-22. **docs/decisions.md**: Dave's
  decisions in his words where no milestone doc holds them.
  **docs/apps-roadmap.md**: the wish list and what comes after.

This file is the lab: the machines, how work moves between them, and what
an AI collaborator must know to work here safely.

## The machines

- **Control** (the Mac): the git copy, pushed to GitHub. No Rust, no
  Homebrew. Docs and the whitepaper's PDF are made here.
- **Hubble** (Void Linux, glibc): a project machine; breaking its session for
  the project is fine, Dave's accounts and data still need care. Its disk
  runs in the ASUS TUF F15 (Intel UHD panel at 1080p 144 Hz, GTX 1650 on the
  external outputs through Mesa NVK; dual boot with Windows). No display
  manager: log in on the text console, then `vergence`. Passwordless sudo.
  Ask before restarting its session.
- **techbench** (FreeBSD 15.1 on ZFS over GELI, i9-13900K, UHD 770, the LG
  27" 4K at 133⅓%): **Dave's workstation, not a project machine.** Normally no
  root for Claude; for testing Dave gives a test password for sudo, **never
  stored anywhere** (ask him again in a later session). Its disk is
  encrypted: **no autologin there, ever.** Restarts of its session only when
  he agrees.
- **kepler** (FreeBSD, Lenovo Tiny, i5-8500T, UHD 630, the 14" 4K touch
  monitor): only when needed (slower hardware, touch). Its touch panel needs
  the devd rule that sets `dev.hconf.N.input_mode=2`. Passwordless sudo.
  Clippy is not installed there: run it on Hubble.
- **canary** (a FreeBSD VM, no GPU): runs the display suite in software when
  wanted. apollo (Fedora) is dropped.
- How they came to be: docs/milestone-notes.md, last section.

## How work moves

- Edit on Control only. **Sync** with `tools/lab/sync.sh <host>` (rsync to
  `~/src/vergence`, keeping the host's build output). After a sync cargo may
  skip edits that look older than its last build: touch the changed files on
  the box if a change seems not to take.
- **Build and test on Hubble and techbench for every change** (kepler only
  when needed): `. ~/.cargo/env; cd ~/src/vergence; CARGO_TARGET_DIR=target-check
  cargo test -q --workspace`. Test builds always go to target-check: a
  rebuilt target/release under a running session broke techbench once.
- **Formatting:** run `cargo fmt` on a box and copy the files back
  (`rsync -a --include='*/' --include='*.rs' --exclude='*'
  <host>:src/vergence/crates/ crates/`); `cargo fmt --all -- --check` clean.
- **Cargo.lock:** after a new crate, dependency or version, copy it back
  (`scp hubble:src/vergence/Cargo.lock .`) and commit it; cargo updates it
  there, never here.
- **The display suite** from Control: `tools/lab/display-suite.sh <host>
  [scenario...]`, one host per command (parallel syncs collide), about 4-5
  minutes a machine; before pushing anything that draws, on Hubble and
  techbench. Never run a headless test on a host while the suite runs there
  (they share the headless session). Logs and shots stay on the host in
  `~/vergence-display-suite/`: copy shots back and look at them.
- **The display check** on a live screen (`tools/lab/display-check.sh`)
  changes that screen's scale for minutes: never on a screen Dave is working
  at without asking; `--headless "<spec>"` instead.
- **Installing:** `cargo build --release` on the box, `sudo tools/install.sh`
  (it restarts the desktop's icons and installs the helpers in
  libexec/vergence), then `vctl restart` only when the server changed or the
  bus version moved (and only with Dave's go on techbench; ask on Hubble).
  An installed session has no control socket unless
  `~/.config/vergence/control-socket` exists (it does on the lab's boxes).
- **Commit and push** after each piece (GitHub `gsa700/vergence`, private;
  going public is Dave's call). Author email is the no-reply address
  (298062495+gsa700@users.noreply.github.com); no LAN addresses, personal
  email, keys or tokens in any file or commit.
- **The whitepaper**: edit docs/Vergence-Whitepaper.md, rebuild the PDF with
  docs/make-pdf.sh on Control (python-markdown in a throwaway venv, printed
  by WebKit), look at the pages before committing.

## Driving a session over ssh

- `vctl` (tools/vctl, installed) finds the socket in `$XDG_RUNTIME_DIR`:
  `/tmp/xdg-<uid>` on Void, `/var/run/xdg/<user>` on FreeBSD. `vctl shot`
  returns before the PNG is written: wait about 2 s, then copy
  `~/vergence-shots/*.png`. Window indices are the stacking order (they
  change on focus). `vctl tree` prints every native window's tree (its
  argument is ignored): cut the window's part out by its title.
- `vctl key` goes to the focused window, and a vctl-spawned window takes no
  focus: **never type into a session Dave is using.** `vctl type` sends only
  small letters, digits, space and `- = , . ;`: click keypad buttons for the
  rest. vctl refuses while the screen is locked.
- The agent interface (`vagent --name Claude ...`, or `vergence-agent-mcp`)
  needs `XDG_RUNTIME_DIR` and `WAYLAND_DISPLAY=wayland-1` over ssh; each app
  is asked of Dave in a server-drawn dialog. A text area's value in the tree
  is cut at 512 characters: read long documents from disk. vtile needs the
  same environment.
- Never start the login screen on a machine where Dave is logged in (vdm
  refuses while a session is registered). vdm's log: /var/log/messages on
  FreeBSD, socklog on Void.
- Scripts use the checkout's tools/vctl, never an installed one (an older
  one ignores `VCTL_SOCKET` and would reach the real session).
- Phase 1's X11 daemon, vergd, is retired; git keeps it (last as
  `crates/vergd` in 4a2f00f), and its file formats live on in
  vergence-profile and vergence-xsettings.
- `VERGENCE_SOUND_OFF=pace` plays sound nowhere at the real rate (`=1`
  refuses it): test sessions must not make a sound at Dave's desk.

## GPU hangs: keep the evidence

On kepler run tests as `tools/test-guard.sh 480 -- cargo test -q
--workspace`; on any hang run `tools/gpu-report.sh [pid...]` before killing
or rebooting, then `tools/lab/fetch-gpu-reports.sh` to copy the reports to
Control (`~/vergence-gpu-reports/`). A vergence-render GPU test once hung on
kepler in the kernel (a page fault on a mapped GPU buffer in drm-kmod's
linuxkpi path, during glReadPixels), unkillable until reboot.

## Working with Dave

- Plans first: a milestone doc with questions and leans; build after his
  answers. Changes that reach every app (the app menu, the look) are laid
  out before and after and wait for his word.
- No borrowing without asking him first: what it is, why it is necessary,
  what our own would take (docs/borrowed-review.md lists what is in).
- His wishes are features for any user, not lab setup.
- He calls Claude "C"; "FB" is "fine business" (good).
