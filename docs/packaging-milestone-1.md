# Packaging milestone 1: the xbps package for Void

*Drafted 2026-11-22.* Dave chose it next, after the platform decision (Void
first, docs/release-pathway.md §1.1) and the first clean install on
techbench. The package is the base the rest stands on: `xbps-install
vergence` in place of CONTRIBUTING's build-and-install, and later the
Vergence system's ISO (release pathway §4), which is made from it. It
settles the release pathway's open packaging questions (§1.2 to §1.6).

## What install.sh does today, and where each part goes

| install.sh does | In the package |
|---|---|
| binaries, helpers, data, fonts, icons, wallpapers, defaults file | files in the package (prefix `/usr`) |
| icon and pointer themes drawn by vergence-harness | drawn at build time, shipped as files |
| PAM files for vdm | package files (conf_files, kept if edited) |
| fontconfig conf, polkit rule, zzz.d hook | package files |
| the `vergence` group, the `_vdm` account | the template's `system_groups` / `system_accounts` |
| setuid helpers (root:vergence 4750) | modes set in the package |
| the runit service for vdm | `/etc/sv/vdm` in the package, not enabled |
| enables seatd, dbus, polkitd, wpa_supplicant; adds the user to `_seatd` and `vergence`; turns the login screen on | not done by the package (Void's convention: a package enables nothing); a `vergence-setup` command does it, asked for once (Q4) |
| patches acpid's handler for the power key | not by the package (another package's file); `vergence-setup`, asked |
| restarts desktop icons, vdm | not by the package |

## Steps

1. A void-packages checkout on Hubble (a dev box), `xbps-src
   binary-bootstrap`; find how its `cargo` build style gets crates (the
   build chroot's network rules) and what Void's own Rust packages do.
2. The template, kept in our tree (`packaging/void/srcpkgs/vergence/`),
   built against that checkout: `build_style=cargo` or our own steps, the
   files table above, licenses (`Apache-2.0 OR GPL-2.0-or-later`) and
   THIRD-PARTY-NOTICES in `/usr/share/doc/vergence`.
3. `vergence-setup` (root): the system setup install.sh does, made one
   script both use, so there is one copy of it; it says what it changes
   and asks before each.
4. Build on Hubble; `xlint` clean; a local repository (`hostdir/binpkgs`).
5. Test on techbench, as a person would: the install.sh copy removed
   (`tools/install.sh --uninstall`), `xbps-install` from the local
   repository, `vergence-setup`, reboot, log in; then an update (version
   bumped) and a removal, each leaving the machine sane.
6. CONTRIBUTING and the release pathway: install from the package; how a
   release is cut; an independent review of the template and
   vergence-setup (they run as root).

## Dave's answers (2026-11-22)

"yes to all six, go ahead. Proceed with the assumption that once we have the
installation part figured out, we will make the repo public." So: one
`vergence` package; prefix `/usr`; version 0.1.0 by hand, the bus protocol
its own number; `vergence-setup` run once after installing, asking before
each change, shared with install.sh; the template in our tree, a local
repository for now, offered to void-packages once public, a signed
repository of our own if not; a `git archive` tarball while private.
**Going public follows this milestone** (release pathway §3): the template
is written for public release tarballs from the start, the local tarball
only standing in for one.

## Questions for Dave

1. **One package or several** (release pathway §1.2). Lean: **one**,
   `vergence` (server, apps, helpers, vdm). Its login screen is optional
   anyway (vdm's service is not enabled until asked), and a split can come
   when someone wants the apps under another desktop.
2. **Prefix `/usr`** for the package (Void's standard; install.sh stays at
   `/usr/local` for development). Lean: yes; a machine has one or the
   other, and the package warns at install if it finds the other.
3. **Version** (§1.3). Lean: `0.1.0` (the workspace's), raised by hand at
   each release; the bus protocol keeps its own number, checked at run
   time as now, so the two need not move together.
4. **`vergence-setup`**: the package itself enables nothing (Void's way);
   after installing, one command, run once, does the system setup and asks
   before each change. Lean: yes. Or should the package's install script
   do it all by itself, as install.sh does?
5. **Where the template lives** (§1.5 in part). Lean: in our tree, built
   locally into a local repository for now; offered to void-packages once
   the project is public (Void decides what it takes); a signed repository
   of our own (`xbps-rindex --sign`) if it does not, or meanwhile.
6. **The source while the repository is private**: a `git archive` tarball
   of a tagged commit, placed where xbps-src looks for sources, until
   release tarballs can come from GitHub. Lean: yes.
