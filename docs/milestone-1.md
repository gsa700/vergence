# Milestone 1: know the monitor setup

Whitepaper §4.7. Read every connected monitor's EDID, turn the set of
monitors into a profile key, and log hot-plug events. Nothing is changed on
the desktop yet: this milestone only observes, so it is safe to run on a
machine in daily use.

**Not started.** It is built and tested on real hardware (a Debian 13 X11
machine with real monitors), not in a VM: QEMU's synthetic EDIDs and its
hot-plug behaviour would be worked around instead of the real thing.

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
4. The same build passes 1-3 on FreeBSD with real hardware, or on canary
   with what QEMU offers if no FreeBSD machine has monitors.
5. No network socket, no files written (whitepaper §7.3, phase 1 row).

## Questions to settle on the real hardware

- Which monitors report a physical size of zero or something wrong
  (whitepaper §10, item 3)?
- Identical monitors: are serial numbers present? If not, how stable is the
  connector name as a tiebreak across reboots and docks?
- What does a KVM switch or a dock do to EDID and hot-plug events?
- Which machine and monitors: to be decided (the Techlab workstation).
