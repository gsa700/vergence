# Milestone 2: remember each monitor setup (DRAFT, for Dave's review)

Whitepaper §4.1. Milestone 1 recognises a monitor setup by its profile key.
Milestone 2 makes the setup's arrangement stick: each profile stores which
outputs are on, their resolution, refresh rate, position and the primary,
and `vergd` puts it back whenever that setup comes back (plug, dock, wake,
login).

**This is the first milestone where `vergd` changes the desktop.** So it is
small on purpose, it can always be undone, and every change is logged.

**Status: draft, not started.** The decisions below are Dave's; the
recommended answer is listed first.

## Scope

In:

- A profile per key: for each connected monitor (by EDID, not by port),
  on/off, mode (width, height, refresh), position, rotation, primary.
- Learning: when the setup is changed by the user (Xfce Display settings,
  `xrandr`), `vergd` saves it for the current key.
- Applying: when the key changes to a known profile, `vergd` applies it.
- `vergd profiles` lists saved profiles; `vergd forget <key>` drops one;
  `vergd apply --dry-run` prints what it would do.

Out (later milestones):

- Scaling, both modes and every toolkit (§4.2, §4.3): milestone 3. It
  needs the arrangement to be stable first, and Mode A changes the RandR
  transform, which this milestone leaves alone.
- Window layout memory (§4.4): milestone 4.
- Edge alignment as a setting (§4.6): comes with scaling, where logical
  heights start to differ.

## Decisions for Dave

1. **Scope.** (a) Arrangement only, as above. *(recommended: the first
   change to the desktop should be the simplest one)* (b) Arrangement and
   scaling together.
2. **How a profile is learned.** (a) Automatically: whatever the user last
   set for a setup is what comes back, the Windows way. *(recommended)*
   (b) Only on an explicit `vergd save`.
3. **A setup never seen before.** (a) Leave X's and Xfce's choice alone,
   then learn from what the user does. *(recommended for this milestone)*
   (b) Compute a default from the EDIDs (native modes, side by side, sizes
   matched): that belongs with scaling in milestone 3.
4. **Xfce's own display profiles.** Xfce can also re-apply saved display
   setups on hot-plug (xfconf channel `displays`, `/AutoEnableProfiles`,
   `/Notify`). Two programs arranging the screens fight. (a) Turn Xfce's
   auto-apply and its "new display" dialog off while `vergd` runs, and say
   so in the docs. *(recommended)* (b) Leave Xfce in charge and have
   `vergd` only observe. To check first: what Xfce 4.20 does by default
   (Hubble has no stored Xfce profiles; those two keys are unset).
5. **Where profiles live.** `~/.config/vergence/profiles.toml`, one file,
   owner-only (0600, directory 0700), human-readable and hand-editable.
   *(recommended)* Alternative: one file per profile.

## How

- **Reading the arrangement:** as milestone 1, plus each CRTC's mode,
  position, rotation and the primary output (RandR 1.5 `RRGetOutputPrimary`).
- **Monitors, not ports:** a profile names monitors by their EDID
  (`canonical` blocks, hashed), so a monitor moved to another port keeps its
  place and mode. Identical twins with no serial fall back to the connector
  name, and `vergd` says so.
- **When to learn:** after a burst of RandR events (the 400 ms settle from
  `watch`) where the key did **not** change, the new arrangement is the
  user's choice: save it. When the key **did** change, it is a plug, unplug
  or wake: apply, don't learn. Changes `vergd` makes itself are recognised
  (by the RandR config timestamp of its own request) and not learned back.
- **Applying, directly over RandR** (x11rb), not by running `xrandr`:
  1. Check every mode exists on its output and the screen fits
     `RRGetScreenSizeRange`; if not, apply nothing and log why.
  2. Grab the server, so clients see one change, not five.
  3. Turn off CRTCs that won't fit the new screen size, set the screen
     size, set each CRTC (mode, position, rotation, outputs), set the
     primary.
  4. Ungrab; read back and compare. If the result differs, log it and put
     the previous arrangement back.
- **Safety:** `vergd apply --dry-run`; a kill switch (`vergd watch
  --observe-only`, the milestone 1 behaviour); every apply logged with the
  before and after; never more than one apply per settle window, so a
  monitor that flaps cannot make `vergd` loop.
- **Security (whitepaper §7.3):** still no network socket and no control
  interface; the profile file is the only file written, 0600.

## Acceptance (on Hubble)

1. Arrange ASUS + a fake monitor (left/right, a non-default mode), unplug
   the fake, plug it back: the arrangement comes back.
2. Same, with the fake on a different port: still comes back (monitors,
   not ports).
3. Change the arrangement by hand with Xfce Display settings: `vergd` learns
   it and does not fight it; the next replug gives the new one.
4. Suspend and wake, and log out and in: the arrangement stays.
5. A profile that cannot be applied (a mode the monitor no longer offers,
   an edited file) is refused, logged, and the desktop is left as it was.
6. Flapping: the harness plugs and unplugs a fake 50 times in 10 s;
   `vergd` applies at most once per settle and ends in the right state.
7. With the real monitors from Dave's cables: 1 to 4 again, plus a
   DisplayPort monitor powered off and on at its own button.

The harness gets the plug/unplug sequences (1, 2, 6); 3 and 7 are partly by
hand.
