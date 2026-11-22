# Defaults milestone 1: what a new user sees first

**CLOSED 2026-11-22.** All steps in: the layered defaults file (5c10271), the standard folders (824bf75), our wallpapers and the defaults (0b0738a, 6f62afa), Theme and fonts / Screens / Sound (353eb7b, 0f3a55f, 52daa78), agents off by default (70b676b), idle sleep through vdm (01a3c07), the color-only tint at 25% (c582668), the login screen's picture and the lock's (e192a89, ceecb0a), SVG text (ffaa2f5); an independent review of the root side, ten findings fixed (a94fa04). Tests: Hubble 996, techbench 996, kepler 985; display suite 18/18 on Hubble and techbench.

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

## Settings, page by page (Dave's first-login walk, 2026-11-22)

Good as they are: Windows, Dock, Notifications, Screenshots, Apps, Default
Apps, Keyboard and Mouse, Accessibility, Date & Time, Lock Screen, Session.

7. **Theme and fonts shows nothing chosen.** The fonts are installed and in
   use: the server falls back to `Inter Variable 10` and `JetBrains Mono 12`
   (vergence-server settings.rs DEFAULT_FONT/DEFAULT_MONO) when
   settings.toml names none, but a fresh account has no settings.toml and
   the page reads blanks, so every list shows no choice. Fix: the page shows
   the defaults the server uses (one shared constant, not two copies); the
   GTK theme list always offers Adwaita (built into GTK, so no folder names
   it; a base Void's /usr/share/themes has none with a GTK 3 part); icons and
   pointer default to Adwaita.
8. **Wallpapers live in the system's backgrounds folder**
   (`$prefix/share/backgrounds/vergence`, which the Wallpaper page already
   offers first), always there and updated with Vergence, not copied into
   each home. Dave asked: Pictures/Wallpaper or a dot folder? Lean: the
   system folder for ours, and `~/Pictures/Wallpaper` offered automatically
   when it exists, for the user's own.
9. **Screens: say what the scale ticks are.** The marked steps are the
   sharp ones (whole divisions of the screen's pixels); say so on the page,
   and mark the recommended one ("200% (recommended)").
10. **Sound: calmer.** Show the device in use (output and input) with its
    volume; the other devices under "Other devices", closed by default.
11. **Agents: off by default?** Today on (desktop.toml `[agents] enabled`,
    default true); every app still asks in a server-drawn dialog before an
    agent may act. For off: a user who never uses agents never sees a
    prompt, and a program can't use the dialogs to nag. For on: the first
    use just works. Lean: off, and when an agent connects while it is off,
    one notification ("An agent asked to use your apps; Privacy > Agents
    turns it on"), not silence. No screen reader depends on it today (there
    is no AT-SPI bridge yet); when one comes it gets its own switch.
12. **Energy: no defaults at all today** (screens off and sleep both
    "Never"), and sleep on Void needs a sudoers line for zzz that nobody
    writes, so a new user's machine never sleeps. Lean: screens off after
    10 minutes (5 on battery); sleep never on power, 30 minutes on battery;
    and sleep through vdm (already root, already locking before sleep), so
    no sudoers line.
13. **Login: the login screen's wallpaper, and on the lock screen too.**
    The roadmap's machine-wide login look, first part: Settings > Login
    chooses the login screen's wallpaper (default: Vergence's default);
    Lock Screen gets "Show: my wallpaper / the login screen's".
14. **About: bling and the Hardware app** (later, not this milestone).
16. *Done 2026-11-22.* **SVG text is never drawn** (found 2026-11-22, a labeled swatch sheet
    in Close-Up showed no words): vergence-decode gives resvg an empty font
    database, so every `<text>` in an SVG vanishes, in Close-Up, previews
    and wallpapers alike. Fix: load the system's fonts and Vergence's own
    into usvg's font database in the decode helper (it runs confined: check
    it may read the font folders).
15. **The wallpaper tint carries color, not brightness** (Dave, 2026-11-22:
    "the interface takes some color from them"). Today a window's grounds
    take 3% of the raw average under it, so dark wallpapers tint nothing
    (Dusk's `#232849` moves the dark page `#1c1c1e` by one level) and light
    ones only lighten dark windows (Mist: +6, gray). Proposed: the average's
    hue and saturation, re-lit to each ground's own lightness, then mixed;
    every wallpaper then gives a subtle cast of its color. Ours keep their
    color moderate in the upper two thirds, where windows sit. Reaches every
    app's look: waits for Dave's word.

## Dave's answers (2026-11-22)

"1 yes, 2 yes minus Reader, 3 yes, 4 yes, 5 likely, 6 yes I think so, 7 yes,
8 yes." So: our own SVG wallpapers; the dock's pins Files, Firefox, Edit,
Terminal, Settings; the shipped defaults file with /etc between it and the
user's; Firefox recommended, not installed by us; agents off by default with
one notification; the energy defaults as leaned, sleep through vdm; ours in
the system backgrounds folder, `~/Pictures/Wallpaper` offered when it exists.

Added: "I think we should create a set of folders if they don't exist as is
standard for most desktops: Documents Downloads Pictures Music Video
Desktop". The freedesktop names (xdg-user-dirs): Desktop, Documents,
Downloads, Music, Pictures, Videos; made at login when missing, and
`~/.config/user-dirs.dirs` written when absent so other programs (Firefox's
downloads, GTK's file chooser) find them. An existing user-dirs.dirs (other
names, another language) is respected, never rewritten.

## Steps

1. The defaults file: one loader that layers
   `$prefix/share/vergence/defaults/desktop.toml`, then
   `/etc/vergence/desktop.toml` (FreeBSD `/usr/local/etc/vergence/`), under
   the user's, key by key; the server and Settings read through it, Settings
   writes only the user's file and shows what is in effect.
2. The standard folders and user-dirs.dirs at login.
3. Wallpapers: our SVGs in `$prefix/share/backgrounds/vergence`, the default
   in the defaults file; `~/Pictures/Wallpaper` offered when it exists.
4. The dock's pins in the defaults file (missing apps skipped quietly).
5. Settings: Theme and fonts shows what is in effect (Adwaita offered);
   Screens names its sharp steps and the recommended one; Sound shows the
   device in use, the rest under Other devices.
6. Agents off by default, one notification when an agent asks while off.
7. Energy defaults (10/5 min screens off; sleep never on power, 30 min on
   battery) and sleep through vdm.
8. The login screen's wallpaper (Settings > Login, through elevation), and
   the lock screen's choice of it.
9. Checks: display suite on Hubble and techbench, a fresh account on
   techbench end to end, kepler builds and tests; an independent review.

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
6. **Agents off by default** with one notification when asked (lean), or on?
7. **Energy defaults** (lean: screens off 10 min, 5 on battery; sleep never
   on power, 30 min on battery; sleep through vdm)?
8. **Wallpaper folders** (lean: ours in the system folder, the user's
   `~/Pictures/Wallpaper` offered when it exists)?
