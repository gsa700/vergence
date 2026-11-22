# Contributing to Vergence

Vergence is a desktop written in Rust: its own display server, native window
protocol, toolkit and apps. ARCHITECTURE.md is the map; STATUS.md says what is
built and what is next; docs/Vergence-Whitepaper.md is the design. This file
is how to build it, run it, test it, and the rules the code keeps.

## Platforms

FreeBSD first, Void Linux (glibc) second (docs/release-pathway.md). FreeBSD
defines what a feature is; on Void it builds, is tested, and uses the
equivalents where they exist. Other systems may be ported by those who use
them. Keep Linux-only interfaces (udev, systemd, logind, /sys) out of the
core: where the platforms must differ, say so in the code and the docs.

## Build

You need Rust (rustup's stable), a C toolchain, and the system's graphics
stack (DRM/KMS, GBM, EGL and OpenGL ES through Mesa), libinput, xkbcommon and
seatd (or logind).

    cargo build --release
    cargo test --workspace

If you run a Vergence session from your checkout's `target/`, build tests
into another directory (`CARGO_TARGET_DIR=target-check`), so a rebuild never
replaces the binaries a running session uses.

## Run

- **Installed:** `sudo tools/install.sh` puts the server, the apps, the
  helpers (in libexec/vergence), the session files and the login screen's
  daemon under /usr/local (`--prefix`, `--destdir` for packages,
  `--uninstall`). Log in on a text console and run `vergence`, or choose
  Vergence at the login screen. The server and the apps speak one exact bus
  protocol version, so they are always installed together; packages must
  require each other at the same version, or be one package.
- **From the checkout:** `tools/vergence-session` starts a development
  session (the control socket on, so `tools/vctl` can drive it).
- **Headless:** `tools/vergence-headless start|env|stop` runs a session with
  screens in memory, its own home and runtime directory, the real monitors
  untouched; `eval "$(tools/vergence-headless env)"` points `tools/vctl` at
  it. This is how the display suite runs.
- The server's log is `~/.local/state/vergence/server.log`; settings are in
  `~/.config/vergence/desktop.toml`.

## Test

- `cargo test --workspace`: unit tests, geometry tests at every clean scale
  (crates/vergence-server/src/geometry_tests.rs), and fuzz tests of the
  protocols (`FUZZ_ITERS=<n>` for a long run).
- `cargo fmt --all -- --check` and `cargo clippy --workspace`: the tree is
  formatted with rustfmt (rustfmt.toml: width 120); new code adds no clippy
  warnings.
- **The display suite**, `sh tools/display-suite/run.sh [scenario...]`: each
  scenario in tools/display-suite/NN-*.sh runs in a headless session of its
  own and checks windows, pixels and app behavior; logs and screenshots go to
  `~/vergence-display-suite/`. Run it before pushing anything that draws.
- **The display check**, `sh tools/display-check.sh <screen> [scale...]` (or
  `--headless "<spec>"`): one-pixel seams at each scale on a running session.
- **The agent check**, `tools/agent-check.sh`: an MCP agent against Edit,
  Settings and FileView in a headless session.
- Fake monitors: `vergence-harness fake --name N --product 0xNNNN --serial N
  --mode 4k30|1080p60 --size-mm WxH --out FILE` makes an EDID;
  `vctl screen add --fake WxH MMxMM` adds a screen in memory to a running
  session.
- A GPU hang: run `tools/gpu-report.sh [pid...]` before killing anything;
  `tools/test-guard.sh SECONDS -- cargo test ...` does it for tests that run over.

A change is done when it builds and passes its tests on FreeBSD and on Void.

## Rules the code keeps

- **Rust throughout.** EDID, profiles, decoders, protocols and the like live
  in library crates the server and apps share.
- **Our own parts, unless borrowing is necessary.** Before adding code from
  another project (a crate, a program, a protocol extension only for
  borrowed clients), open the question with the maintainer: what it is, why
  it is necessary, what writing our own would take. docs/borrowed-review.md
  lists everything not written here. Interfaces of the operating system
  (DRM/KMS, evdev, GBM, EGL, Vulkan, the system's libraries) are not
  borrowing. Ported code keeps its provenance and license beside it.
- **License: `Apache-2.0 OR GPL-2.0-or-later`** (LICENSE-APACHE, LICENSE-GPL).
  The one exception is crates/vergence-hymie-aac (GPL-2.0-or-later, ported
  from FAAD2), used only by its own helper program. Check a dependency's
  license before adding it (anything copyleft or unusual needs the
  maintainer's word) and rerun `python3 tools/notices.py` to regenerate
  THIRD-PARTY-NOTICES.md (`--check` says whether it is current).
- **Security** (whitepaper §7.3): no network socket in the server; no
  unauthenticated control interface (every socket checks its peer's uid; the
  control socket exists only in test sessions); saved state owner-only
  (0600). Untrusted files are parsed in confined helpers, never in an app's
  own process (whitepaper §6.10; vergence-decode's `confine`: Capsicum on
  FreeBSD, seccomp on Linux).
- **One bus protocol version.** `vergence_protocol::bus::VERSION` changes
  with any change to the wire; the server refuses any other. Note the change
  at the top of crates/vergence-protocol/src/bus.rs and extend the
  round-trip and fuzz tests.
- **No compatibility layers.** Vergence runs natively; no Linux emulation on
  FreeBSD and no shims for other desktops' internals.
- **X11 programs are off by default** (desktop.toml `[x11] enabled = true`
  starts Xwayland).
- **Comments explain why**, in plain present-tense American English: the
  rule and its reason, standing on their own, with a pointer to the doc
  where a decision is recorded ("(see docs/drives-milestone-1.md)"). The
  decisions themselves, in the maintainer's words, live in the docs
  (milestone docs, docs/decisions.md), not in the code.
- **American spelling** in code, comments, docs and the interface.
- After adding a crate, a dependency or a version, commit the updated
  Cargo.lock.

## How work is planned

Each piece of work is a milestone with a plan in docs/<area>-milestone-N.md:
why, what it does, the steps, what is not in it, questions for the
maintainer and their answers, then progress as it is built. A milestone
closes when its checks pass on both platforms and the maintainer has tried
it. The whitepaper's §9 holds the order of work; STATUS.md holds where things
stand.

## The project's own lab

tools/lab/ holds scripts for the machines Vergence is developed on (a git
copy on one machine, builds on dev boxes over ssh); CLAUDE.md describes that
lab and how its AI collaborator works in it. Nothing outside tools/lab/
depends on it.
