# Vision check, 15 November 2026

Where Vergence stands against the design it set out with (the whitepaper's
§§1–6, first drafted on 5 November), written at the end of files milestone 1
and the evening's security pass, at Dave's question: "I am curious what you
think of where we are compared to the original vision and if we have left
good ideas under-developed?" To be folded into the whitepaper (§9) next.

## Delivered, and true to the original

- **Contract 1, geometry in visual size.** Scale derived from each panel's
  EDID and viewing distance, the nearest whole number by default; running
  daily on mixed real hardware (Hubble, kepler's 14" 4K touch panel,
  Techbench).
- **Contract 2, placement.** The server draws every bar and owns placement;
  places kept per monitor set and per resolution and scale; persistence
  tokens; session restore; unplugging no longer piles windows up. The
  original pain is solved.
- **Our own parts** (principle 7) proved stronger than expected: our own
  toolkit, renderer, bus, D-Bus client and picture decoder made the
  15 November security work quick (seccomp on a helper we wrote is a day,
  not a negotiation), and the result is fast: Dave, 15 November, "the one
  thing I notice when using it is it is so FAST ... I loaded a 600000 line
  file today and it didn't even blink."

## Changed by use, rightly

The vertical shelf became a floating dock; snapping went from Ctrl-only to
always; FreeBSD-first became Linux-primary (FreeBSD kept on kepler); the
desktop gained icons (Dave's call over "wallpaper only"); exact per-screen
scale stayed a switch, off, because whole-number sharpness won in use. No
workspaces, no top bar, no global menu: still right.

## Under-developed: the part that makes Vergence different

1. **Contract 3, meaning.** The tree exists (`vctl tree`); nothing uses it
   yet: no magnifier following focus and caret, no screen reader, no agent
   interface (§6.5). Seen the same evening: checking a settings window
   through `vctl` meant arrow keys through a menu and clicks at pixel
   coordinates read off a screenshot, where an agent on the contract would
   have asked for "Terminal Settings, the Text menu's items". It joins
   Dave's "agentic JIT app building" idea.
2. **Display-list surfaces (§6.6), the NeXTSTEP idea in the name.** The
   toolkit builds scenes in our own format, then sends pixels: the protocol
   carries only Shm and dmabuf buffers. Sending our own apps' scenes would
   bring what nothing else has: sharp across two monitors, truly sharp
   magnification, remote display as drawing commands.
3. **Live tiles and the drawer (§6.9, designed 13 November).** Anyone
   publishes a value, the server draws it, a tile in trouble comes out by
   itself. Fits Dave's lab: UPS through NUT, the GPS time source, rack
   temperatures, Proxmox.
4. **Named desktop actions**: one command surface for the launcher's
   search, voice and agents; the link between 1 and 3.
5. Smaller pieces of the vision: user window rules ("the logbook always on
   the left monitor"), pointer speed and cursor size the same physical size
   on every monitor, per-monitor color and night light (§6.7).

## Why the apps came first, and were worth it

Dave, 15 November: the terminal and the Text Editor took long, "but I think
it was worth it because they are great apps and they helped with the
toolkit work that has made the subsequent apps so easy." The record agrees:
the editor gave the toolkit its text area, history, file dialog and
spelling, which File Viewer, the desktop, Close-Up and the settings app
reuse; the terminal drove the bar protocol (server-drawn tabs, one process
for all windows, the close question). Files milestone 1 went from plan to
eleven steps in about a day. And the apps are what the contracts need to
mean anything: Contract 3 needs real apps publishing real trees, display
lists need real scenes to send.

## The rule from here (Dave, 15 November: "a good way forward")

**Alternate.** After each app or polish milestone comes one milestone of
the core vision (Contract 3, display lists, live tiles and named actions,
the §6.7 pieces), chosen where it also pays off in daily use. The first
proposed: **live tiles with named desktop actions**, with Dave's lab as the
first publishers; then the agent and magnifier side of Contract 3; then
display lists for our own apps. The lock screen (the start of VDM) stands
beside them on the app side.
