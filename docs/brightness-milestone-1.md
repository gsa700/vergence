# Brightness milestone 1: the screens' brightness

*Drafted 2026-11-27 on Control from Dave's wish (the wish list, 11-27):
"There is no brightness control on the laptop, it needs one. Maybe a
Display dockworker? Could be a quick way to adjust a screen say when a
projector is connected for a presentation?"*

**Decisions (Dave, 2026-11-27, "go with your leans"):** 1 a, a Display
tile (docked by default only when a screen has a control), Settings >
Displays' slider, and the keys; 2 a, vdm sets it, a named action with no
prompt for a console session; 3 a, external screens over DDC/CI in
milestone 1, Linux first; 4 a, 5% steps, Shift for 1%; 5 a, each screen's
level kept and set again at login; 6 a, now, Hubble's panel first.

## Why

Every desktop has it: a slider for the laptop's screen in the menu bar or
the quick settings, the brightness keys with a level shown, and on macOS and
GNOME (with ddcutil) the same slider for an external screen. On Vergence
today a laptop's screen stays wherever the firmware left it, and the
brightness keys do nothing.

## What the machines say (2026-11-27)

| | Built-in panel | External screens | Who may change it |
|---|---|---|---|
| Hubble (Void, laptop) | `/sys/class/backlight/intel_backlight`, 0 to 120000, on `card1-eDP-1` | the GTX 1650's outputs | the panel file is root's (`-rw-r--r--`); no elogind (seatd), so no logind `SetBrightness` |
| techbench (Void) | none | the LG 4K on DP-1: an I²C line for its DDC (`card0-DP-1/ddc -> i2c-9`) | `/dev/i2c-*` absent: the `i2c-dev` module is not loaded; once loaded, root's |
| kepler (FreeBSD) | none (a Tiny with a 14" monitor) | the touch monitor over HDMI | FreeBSD: `backlight(9)` for panels (`/dev/backlight/*`), DDC through `iic(4)` where drm-kmod offers it (not yet seen) |

So changing brightness needs root on every machine, and a person's program
must not get root for it. vdm already runs as root on all three, and
already does one kind of thing without a prompt for anyone with a session
at the machine's console: Sleep, Restart and Shut Down (power.rs). Setting
a screen's brightness is the same kind of thing: harmless, the person's
own screen, and wanted many times a minute with the keys.

## The plan

1. **Readings** (vergence-power, beside the GPUs): each screen's brightness
   control, found by its connector. A panel's backlight is under the
   connector in sysfs (`card1-eDP-1/intel_backlight`); FreeBSD's through
   `backlight(9)`'s status ioctl. Reading a panel needs no root.
2. **vdm sets it** (question 2): named actions `brightness.set <connector>
   <percent>` (and, for external screens, `brightness.get <connector>`), only
   for someone with a session at a console, as power actions are; vdm checks
   that the connector is a real one with a control and the value is 0 to
   100, maps it onto the device's own range, and never writes 0 to a
   backlight (a black panel looks broken): the lowest step is a dim but
   readable screen.
3. **The keys and the level** (the volume keys' pattern, sound.rs): the
   brightness keys (XF86MonBrightnessUp and Down) move by 5% (question 4)
   and show the level as the volume keys do ("Brightness 40%", the screen's
   name under it), replacing the last one.
4. **The Display tile** (question 1): a dock tile whose card has a slider
   per screen that has a control, named as the Displays page names screens
   (from their EDID: "Built-in display", "LG HDR 4K"). Settings > Displays
   gets the same slider under each screen.
5. **External screens over DDC/CI** (question 3): our own small DDC/CI
   client in vdm (the monitor's Luminance, VCP code 0x10: get and set, at
   I²C address 0x37, with the protocol's checksums, pauses and retries),
   only on the I²C line the DRM connector names as its own; the package
   loads `i2c-dev` at boot (`/etc/modules-load.d/vergence.conf`). Slow
   (tens of milliseconds a command; some monitors take longer), so done off
   the frame, and a slider drag sends only the latest value.
6. **Remembered** (question 5), and the checks: a display suite scenario on
   a stand-in backlight (a directory the server is pointed at in tests,
   never a real one), unit tests for the DDC/CI frames and vdm's argument
   checks, and the slider tried on Hubble's panel and techbench's LG.

## Questions

1. **Where it lives:** (a) a Display tile in the dock (docked by default
   only when a screen has a control), Settings > Displays' slider, and the
   keys; (b) only Settings and the keys; (c) only the tile and the keys.
   *Lean: (a).*
2. **Who changes it:** (a) vdm, as a named action with no prompt for a
   console session, as Sleep is (no new setuid program, the same on Linux
   and FreeBSD); (b) a udev rule giving the video group the backlight and
   I²C files (Linux only; FreeBSD would need a devfs rule; any program the
   person runs could then also talk to the monitor's I²C line); (c) a
   setuid helper of its own, group vergence, as FreeBSD's sound-default.
   *Lean: (a).*
3. **External screens (DDC/CI) in milestone 1:** (a) yes, on Linux, our
   own client in vdm, FreeBSD's later once drm-kmod's I²C is seen working;
   (b) the built-in panel only now, external screens in a milestone 2.
   *Lean: (a), since the wish names a projector, and techbench's LG is
   there to try it on (a projector answers DDC/CI less often than a
   monitor; one that does not simply has no slider).*
4. **The keys' step:** (a) 5%, Shift for 1% (fine control near the dark
   end); (b) 5% only; (c) 10%. *Lean: (a).*
5. **Remembered:** (a) Vergence keeps each screen's level and sets it again
   at login (a laptop's panel comes back at the firmware's level after a
   restart, often full); (b) left to the hardware. *Lean: (a).*
6. **When:** (a) now, steps 1 to 4 on Hubble's panel first, then step 5 on
   techbench's LG; (b) after the kepler GPU recovery. *Lean: (a).*

**Later, not milestone 1:** dimming before the screen blanks when idle (an
Energy setting), a night light (warmer at night), an ambient light sensor.

## Progress

- **Steps 1 to 4, and 6 (2026-11-27):** built and tried on Hubble's panel.
  vergence-dm's brightness.rs finds the controls and maps percents above a
  1% floor; vdm takes `brightness <connector> <percent>` and
  `brightness-list` on its power socket, each on a thread (at most four at
  once), for root and anyone with a session at the machine. The server's
  brightness.rs: the keys (5%, 1% with Shift, the level shown as the
  volume's), the Display tile (a slider per screen), each screen's level
  kept by its monitor's identity (~/.local/state/vergence/brightness.toml)
  and set again 0.3 s after login. Settings > Screens has the slider.
  Display suite 41 on a stand-in backlight.
- **Step 5 (2026-11-27): written, not yet tried on a monitor.** Our own
  DDC/CI client (vergence-dm ddc.rs: Get and Set VCP feature for Luminance,
  0x10, on the connector's own I²C line, its frames checked in unit tests);
  vdm lists external screens whose monitors answer and sets them in their
  own range; the server asks for their levels at start and every minute.
  The package and install.sh load `i2c-dev` (now, and at boot through
  modules-load.d). To try on techbench's LG.

