# Defaults milestone 1: what a new user sees first

*Drafted 2026-11-22, from the first clean install (techbench, stock Void,
set up from CONTRIBUTING alone). Dave: "some of the defaults are now
obviously lacking since this is our first clean install we see them now for
the first time as a user would: there is no wallpaper on the login screen or
the desktop. It doesn't have to be the worlds greatest but we should ship a
few options and a default. None of the built in apps are pinned to the Dock
by default, we should pick some that are - Settings, Terminal, Edit, ???
Maybe Firefox for now which we should probably install until our own is
usable."*

A feature for anyone who installs Vergence, not lab setup.

## Today

- `[wallpaper]` in `~/.config/vergence/desktop.toml` (image, mode, color);
  unset, the desktop is a flat color. The login screen draws a wallpaper
  too, but has none to draw.
- `[shelf] pinned = [...]`, desktop entry ids in order; unset, the dock has
  no apps until the user pins some.
- Every default lives in code. There is no shipped defaults file.

## Plan

1. **Wallpapers of our own.** Four to six, one the default, drawn as SVG
   (sharp at every density, kilobytes each, made by us so no license
   question): calm, dark-leaning abstracts in the NeXTSTEP spirit, plus a
   lighter one. Installed to `$prefix/share/vergence/backgrounds`, offered
   first in Settings > Wallpaper.
2. **A shipped defaults file**: `$prefix/share/vergence/defaults/desktop.toml`,
   read under each person's own desktop.toml (theirs wins key by key).
   It holds the default wallpaper and the dock's pins; a distribution, or
   our own ISO later (docs/release-pathway.md §4), can change them without
   touching code. `/etc/vergence/desktop.toml` (FreeBSD:
   `/usr/local/etc/vergence/desktop.toml`) for the machine's own, between
   the two, as the roadmap's system-wide defaults.
3. **The dock's default pins**, an entry skipped quietly when its app is
   not installed: Files, Firefox, Edit, Terminal, Reader, Settings.
4. **The login screen** shows the default wallpaper (dimmed, as it draws
   any), until the machine-wide login look (apps-roadmap) lets the
   administrator choose.
5. **Firefox**: listed in CONTRIBUTING as recommended (`xbps-install
   firefox`; FreeBSD `pkg install firefox`), not installed by install.sh
   (programs are the system's packages); our ISO includes it later.
6. Checks: the display suite on Hubble and techbench (a fresh home shows
   the wallpaper and the pins), a fresh user on techbench end to end, kepler
   builds and tests.

## Questions for Dave

1. **Wallpapers: our own SVG abstracts (lean), or photographs** (which need
   CC0 or our own photos, and weigh megabytes each)? Any direction for the
   look, colors or mood?
2. **The dock's pins and order**: lean Files, Firefox, Edit, Terminal,
   Reader, Settings. Add or drop any (Calculator? Close-Up?)
3. **The defaults file** (lean: yes, with `/etc` between it and the user's),
   or keep defaults in code for now?
4. **Firefox recommended, not installed by us** (lean), or should
   install.sh offer to install it?
5. Anything else from the first login that looked unfinished? This is
   the place to gather it.
