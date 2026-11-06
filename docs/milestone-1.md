# Milestone 1: know the monitor setup

Whitepaper §4.7. Read every connected monitor's EDID, turn the set of
monitors into a profile key, and log hot-plug events. Nothing is changed on
the desktop yet: this milestone only observes, so it is safe to run on a
machine in daily use.

**Not started.** It is built and tested on real hardware (a FreeBSD X11
machine with real monitors; Debian 13 second), not in a VM: QEMU's synthetic
EDIDs and its hot-plug behaviour would be worked around instead of the real
thing. FreeBSD became the primary platform on 2026-11-06.

## What it does

A small Rust program, `vergd` (the Vergence daemon), with two commands:

- `vergd probe`: print each connected output and the profile key, then exit.
- `vergd watch`: the same at start, then one line per RandR change (monitor
  plugged or unplugged, mode changed, wake from sleep), with the new key.

Per output it reports: connector name (DP-1, HDMI-1...), EDID manufacturer
code, product code, serial (numeric and the text descriptor if present),
monitor name, physical size (cm from the base block; mm from the first
detailed timing, which is more precise), native mode, current mode and
position.

## How

- **X11 access:** the `x11rb` crate (XCB), RandR 1.5. Outputs from
  `RRGetScreenResourcesCurrent`; EDID from the `EDID` output property
  (`RRGetOutputProperty`); changes from `RRSelectInput` with
  `RRScreenChangeNotify` and `RRNotify` (output changes).
- **EDID parsing:** our own small parser for the 128-byte base block plus
  the descriptors we need. It goes in a library crate (`vergence-edid`)
  so the later server reuses it unchanged (whitepaper §8). Bad or short EDIDs
  are reported, never trusted.
- **Profile key:** SHA-256 over the sorted EDID blobs of the connected
  monitors, so the key does not depend on which port a monitor is in.
  Open: two identical monitors with no serial number have identical EDIDs;
  the key then needs the connector name as a tiebreak (see below).

## Acceptance (on the real machine)

1. `vergd probe` lists every monitor with sane manufacturer, model and
   physical size; compare against the monitors' own labels and spec sheets.
2. The key is the same across reboots and across swapping which port each
   monitor uses.
3. `vergd watch` logs unplug and replug of each monitor, and sleep/wake, with
   the right key each time.
4. The same build passes 1-3 on Debian 13 with real hardware.
5. No network socket, no files written (whitepaper §7.3, phase 1 row).

## Test automation

Writing the daemon takes about a week of sessions; testing it on real
hardware would take weeks by hand. Most of that testing can be automated.
The harness below starts with milestone 1 and grows with phase 1 (profiles,
scaling, layout memory). It is a separate test tool: the harness may need
root, the daemon never does.

**1. Plug and unplug without touching a cable (Linux).** The kernel's DRM
layer gives every connector two debugfs files (DRM core, so any KMS driver,
the M910q's Intel HD 630 included):

- `/sys/kernel/debug/dri/<N>/<connector>/force`: `on` / `off` / `unspecified`
  makes the connector look plugged in or unplugged, with the hot-plug event
  that goes with it.
- `/sys/kernel/debug/dri/<N>/<connector>/edid_override`: write an EDID blob
  and the connector reports it; write `reset` to go back to the real one.

With these, a script can run the daemon against thousands of real monitors'
EDIDs from the public linuxhw EDID database (github.com/linuxhw/EDID), broken
ones included (zero or wrong physical size, missing serials, identical
twins). That covers whitepaper §10 item 3 far better than a shelf of
monitors. The exact behaviour (whether `force` produces the same RandR
events as a real unplug) is the first thing to check on the machine.

**2. Sleep and wake, overnight.** `rtcwake -m mem -s 30` suspends and wakes
the machine after 30 s. A loop checks after every cycle that the profile key
is unchanged and (later) that the layout survived.

**3. Same physical size everywhere (scaling milestones).** Small test
programs in each toolkit (GTK, Qt, Avalonia, Electron, Java) each draw a box
of a known logical size. The harness screenshots each monitor, measures the
box in pixels and converts to millimetres with the EDID's physical size. Pass:
every toolkit's box is the same physical size on every monitor, within a
tolerance. Dave's eye is needed once, to calibrate what "the right size" is,
not on every run.

**4. Layout memory (layout milestones).** The harness opens windows, Firefox
with several windows and Thunderbird included, places them, fakes an unplug
and replug (1), restarts the apps and compares every window's monitor,
position and size with what was saved. A drift check catches a window
creeping a few pixels per restore (GTK client-side decoration shadows,
`_GTK_FRAME_EXTENTS`).

**5. Record and replay.** The first runs on real monitors, a dock and a KVM
switch record every RandR event and EDID the daemon sees. The recordings
become ordinary tests that run anywhere, Control and CI included, with no
monitors attached. The daemon's X11 access sits behind an interface so a
recording can stand in for the server.

**What stays manual:**

- Dave's judgment of "the right size", once, as the calibration for (3).
- Real docks and KVM switches now and then: they do things a script won't
  predict, and a recording only captures what happened once.
- **FreeBSD (now the primary platform):** whether drm-kmod offers `force`
  and `edid_override` through lindebugfs (`mount -t lindebugfs`) is unknown,
  and FreeBSD has no `rtcwake`; both are the first things to check on the
  machine. If they are missing, the automated sweeps (1) and sleep loops (2)
  run on a Debian machine, and FreeBSD gets real cables plus the replayed
  recordings from (5).

For milestone 1 itself, (1), (2) and (5) apply: acceptance tests 2 and 3
above become scripts, run over the EDID database and overnight sleep cycles.

## Found on Hubble (FreeBSD 15.1, Intel UHD 630), 2026-11-06

- **Fake monitors work.** lindebugfs mounts and drm-kmod has `force` and
  `edid_override` on every connector. Write the EDID in one `dd` (`cat` fails
  with "Operation not supported"), then `force` = `on`. After a re-probe, X
  shows the output as connected, with the fake EDID in the RandR property.
  Forcing does not send a hot-plug event, so the harness re-probes
  (`RRGetScreenResources`) after each change.
- **Real hot-plug stops at devd.** Unplugging and replugging a real HDMI
  cable gives the kernel event
  `!system=DRM subsystem=CONNECTOR type=HOTPLUG cdev=dri/card0 connector=115`
  on `/var/run/devd.pipe`, but Xorg (with libudev-devd installed) does not
  re-probe and sends no RandR event. So on FreeBSD `vergd watch` also
  listens on `/var/run/devd.seqpacket.pipe` for DRM HOTPLUG events and calls
  `RRGetScreenResources` itself; X then sends the usual RandR notifies. This
  is a FreeBSD-only path and is marked as such in the code. On Debian, Xorg
  re-probes by itself through udev.

## Questions to settle on the real hardware

- Which monitors report a physical size of zero or something wrong
  (whitepaper §10, item 3)?
- Identical monitors: are serial numbers present? If not, how stable is the
  connector name as a tiebreak across reboots and docks?
- What does a KVM switch or a dock do to EDID and hot-plug events?
- Which machine and monitors: Dave is setting up the FreeBSD machine
  (2026-11-06). Prefer an Intel or AMD GPU (drm-kmod). The M910q
  (i5-7500T, Intel HD 630, i915) is a good test machine either way: on
  Debian its driver has the debugfs connector controls the test harness uses.
