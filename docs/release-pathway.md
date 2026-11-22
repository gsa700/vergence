# The release pathway: packages, the tree made approachable, going public

Status: **started 2026-11-21, to be planned** (Dave: "let's plan all of this
as part of the release pathway. we are going to have to make some packaging
choices and so on and that seems like a good time to do this as well").
Nothing here is decided yet unless it says so. It gathers what has to be
settled before anyone outside the lab installs Vergence or reads its source.

## 1. Packaging choices

Decided already:
- **The server and the apps require each other at the exact same version**
  (Dave, 2026-11-14): they speak one bus protocol version, so packages
  depend on each other at that version, or they are one package.
- `tools/install.sh` installs under a prefix today (`--prefix`, `--destdir`
  for packages, `--uninstall`).

To settle:
1. *Settled 2026-11-22:* **Void first, FreeBSD second**, replacing
   2026-11-21's FreeBSD first. Dave: "it's app and driver compatibility
   that keeps dragging me back ... the graphics, wifi drivers, laptop power
   save, and so on." The rule from 2026-11-21 stands: "The goal is not to
   exclude other platforms but to minimize our testing and distribution
   support requirements. If someone wants to port it to their systemd
   distro they can, but they support that not me and you." So the packages
   are an xbps template for Void (glibc) first, then a FreeBSD port (and
   pkg), and nothing else from us. Void defines what a feature is; FreeBSD
   gets equivalents where they exist. **Near term, no installer of our
   own**: Vergence is added to a stock system set up by that system's
   installer, which owns the disk layout and encryption. Backup is left to
   the user (Rewind shelved, 2026-11-22). Long term, the Vergence system
   (§4).
2. **One package or several**: server + apps + vdm as one, or split (the
   login manager is root and optional; the apps could be wanted under other
   desktops).
3. **Version numbers**: what 0.1 means, and how a version relates to the bus
   protocol version.
4. **What install.sh does that a package must do instead**: seatd, the seat
   group, vdm enabled at boot, the FreeBSD media helper (setuid), the
   cursor and icon themes drawn at install (vergence-harness), the devd and
   autofs files.
5. **Where release builds come from**: the dev boxes, CI, or both; signing.
6. **Third-party notices** in every package (THIRD-PARTY-NOTICES.md, from
   tools/notices.py).

## 2. The source tree, made approachable

From Claude's review, 2026-11-21 (clarity B, approachability C+: the code
reads well; getting started depends on knowing the lab):

1. **ARCHITECTURE.md**: one diagram (server, bus, toolkit, apps, vdm) and a
   tour: how an app gets a window, how a frame reaches the screen, where
   input goes, what each crate is for.
2. **CONTRIBUTING.md**: build and run on a single Linux or FreeBSD box
   (`cargo build`, a nested session with tools/vergence-session), the tests,
   the display suite, the rules (security, borrowing, license, American
   spelling, the bus version lockstep).
3. **A human README**: what Vergence is, screenshots, status, how to try it.
   CLAUDE.md stays as the working notes, or is split into lab notes and
   project rules.
4. **rustfmt.toml with one width, and the tree reformatted once** (about
   1,650 lines run over 150 characters, and 52 over 300; most are struct
   literals on one line).
5. **Big files split by part**: Settings' main.rs (about 3,900 lines) by
   page, as wifi.rs and places.rs already are; the terminal, FileView,
   Reader and Edit main.rs files (about 1,900 to 2,800 lines each) where a
   part stands alone.
6. **Comments keep the why and drop the dialogue**: dated quotes move to the
   milestone docs (or a decisions log), and the code keeps the rule plus a
   pointer.
7. **One status page**: where things stand, instead of the whitepaper's §9,
   the README, CLAUDE.md and 55 docs each holding part of it.
8. **Lab-only tools set apart**: sync-hubble.sh, fetch-gpu-reports.sh and
   the host names, from what any contributor uses.

## 3. Going public (Dave's call)

*Dave, 2026-11-22: "once we have the installation part figured out, we will
make the repo public"*: after packaging milestone 1
(docs/packaging-milestone-1.md). The checks below are done before then.

- **A digestible summary of the design** (Dave, 2026-11-22: "boil it down to a
  more digestible summary highlighting why Vergence isn't just another Wayland
  compositor. And how awesome it is in general"): a few pages, ahead of the
  whitepaper, for people deciding whether to look closer. What sets it apart:
  its own server, protocol and toolkit rather than a compositor among many;
  windows sent as display lists the server draws at each screen's exact scale
  (sharp text everywhere, the magnifier sharp); placement owned by the server
  and kept per monitor setup; meaning built in, so screen readers and agents
  act by name with permission; untrusted files parsed in confined helpers;
  Void first, FreeBSD second; our own parts throughout. With pictures.

- The repository is private (`gsa700/vergence`). Going public is Dave's
  call (CLAUDE.md).
- Check before: the whitepaper's byline (his callsign), the ASUS test
  EDID's serial, any LAN address, name or email in files or history; the
  commit author is the no-reply address already.
- Whether the history is published as is, or squashed.
- Where the project lives, how issues are taken, and the license text
  (`Apache-2.0 OR GPL-2.0-or-later`, decided 2026-11-11).

## 4. The Vergence system (long term)

*Settled 2026-11-22* (Dave: "I think that is a good balance. We benefit
from an OS configuration that we like and get the FS and layout we want for
Rewind as a bonus. In the worst case, we rebase to something else, heck that
could be a feature sometime: Linux or FreeBSD from our installer?"). After
the packages (1), not before:

1. **Void's repository is the software source.** We package Vergence and
   choose a base set; the kernel, Mesa, firmware and security fixes stay
   Void's. Not a distribution of our own.
2. **Our own installer and ISO** (the ISO made with Void's void-mklive).
   We own the layout: encryption is a checkbox (LUKS2), Btrfs is the
   default.
3. **A normal, changeable system** (Dave is no fan of immutable ones), with
   a Btrfs snapshot before every system update and a boot entry for the
   state before it.
4. **Apps Void lacks**: Flatpak while it works without systemd (Flatpak's
   next major version is heading toward systemd: watched, not built on);
   AppImage for proprietary apps.
5. **The installer's engine takes the OS as a back end** from the start:
   Void first, FreeBSD later (pkgbase into ZFS on GELI). A choice for
   users, and the way out if Void ever has to be replaced.
6. **Rewind comes back** here, on the layout the installer made (Btrfs on
   Void, ZFS on FreeBSD).

Weighed and set aside the same morning: ZFS root on Linux (ZFSBootMenu),
a distribution of our own, an immutable image with Flathub at its center.

## Order (proposed)

The platforms (1.1) are settled. Do the
tree work (2) before the first package, so the source a package points at
is the one people will read. Going public (3) comes last. The Vergence
system (4) follows the packages.
