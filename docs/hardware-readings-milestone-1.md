# Hardware readings milestone 1: what only root may read

*Drafted 2026-11-26 on techbench (Dave: "write up the plan for the root
helper, should we use the elevation prompt for that?"). Follows the
Hardware app's Details (branch hw-core-grid, 6fa0bbb), which show what any
user may read; this plan covers the rest of CPU-Z's pages. Nothing here is
built yet.*

## Why

The Hardware app now shows the processor, its caches and clocks, the board
and firmware, and the graphics card, all without root. Three things people
look for are still missing:

1. **The memory as it runs:** its speed now (which may be the modules'
   XMP/EXPO profile, not their rated speed), the slots in use, each
   module's maker, part number and size.
2. **The memory's rated speed and timings** (CL-tRCD-tRP-tRAS).
3. **The processor's power now** in watts (package power), as Task
   Manager and Activity Monitor show it.

## What this machine says, and who may read it (techbench, 2026-11-26)

| Reading | Where | Who may read it |
|---|---|---|
| Rated speed and timings (2) | each module's SPD chip: `/sys/bus/i2c/devices/*/eeprom` (the kernel's `spd5118` driver for DDR5, `ee1004` for DDR4) | **any user** (0444) |
| Module maker, part number (1) | the SPD chip's second half | any user, **but** techbench's board blocks the chip's page switch, so the second half reads back as a copy of the first |
| Speed now, slots, maker, part number, size (1) | the firmware's memory table (SMBIOS type 17): `/sys/firmware/dmi/tables/DMI` | root only (0400) |
| Package power (3) | `/sys/class/powercap/intel-rapl:0/energy_uj` | root only (0400) since PLATYPUS (CVE-2020-8694) |

techbench's modules decode as DDR5, rated 4800 MT/s (tCKAVGmin 416 ps),
tAA/tRCD/tRP 16.0 ns, tRAS 32.0 ns: JEDEC 40-39-39-77 at 4800.

Why each is root's: the firmware table holds serial numbers, asset tags and
the machine's UUID (fingerprints; the kernel already gives any user the
rest of the same table in `/sys/class/dmi/id`, board and firmware but not
the serials). The power counter was closed because sampling it every few
microseconds to milliseconds lets a program recover secrets (keys) from
how much power the processor draws.

## Should the elevation prompt be used? Lean: no, not for these

The elevation prompt (docs/elevation-milestone-1.md) is for **changes** a
person approves: vdm carries out one named action, the yes is spent, and
"a yes is never remembered, every action asks" (Dave, 2026-11-16,
question 3 a). For these readings it fits badly:

- **Live power cannot work with it**: every sample (or every time Hardware
  opens) would ask for the password again, or a yes would have to be
  remembered, which question 3 rules out.
- **Asking for the password to look at the memory's speed teaches people to
  type it without thinking**, which is what makes a real prompt worth
  less (prompt fatigue).
- **None of it needs to be secret once filtered**: without the serials, the
  memory table says what the kernel already says of the board; power
  averaged over a second is what every other desktop shows, and is far
  too coarse for PLATYPUS, which needs thousands of samples a second.

So the lean is a narrow **read service in vdm** that answers without a
prompt, gives only filtered readings, and only to a person at the machine.

## How it would work (the lean)

1. **vdm reads, the app asks.** vdm already runs as root on every Vergence
   machine and already checks who is asking (uid and pid from the kernel,
   a session at one of the machine's consoles: elevate's rule). It gains
   two named reads on its own socket (not elevate's, not the lock's):
   - `memory.modules`: vdm parses SMBIOS type 17 itself (our own parser;
     FreeBSD: the table found through `kenv hint.smbios.0.mem` and read by
     vdm, as root) and returns, per slot: locator ("DIMM_A2"), size, type,
     speed now and rated, maker, part number. **Never** serial numbers,
     asset tags, or the system UUID.
   - `power.package`: vdm reads the energy counter itself, **at most once a
     second**, and returns the average watts over the last second, rounded
     to 0.1 W. Many askers share the one reading; vdm never reads faster
     however many ask. FreeBSD: MSR_PKG_ENERGY_STATUS through cpuctl, the
     same rules (later; see question 4).
2. **The Hardware app shows them** where they belong: Memory > "Modules"
   (each slot) and "Speed now"; Processors > "Power now" with the minute's
   chart. With no vdm (another desktop, an older system), the rows say
   "needs Vergence's login service" instead of failing silently.
3. **The SPD readings need nothing new**: vergence-sysinfo reads the SPD
   chips directly (any user), decodes DDR4 and DDR5 rated speed and
   timings, and fills Memory > "Rated" now, prompt or no prompt.
4. **Logged** like vdm's other requests, but **once per asking program**,
   not once a second.

What it never does: hand out raw tables, read a counter faster than once a
second, answer anyone not at the machine, or take a path or argument from
the asker (both reads take none).

## Threats, and what answers each

- **PLATYPUS through vdm**: one reading a second, averaged and rounded,
  shared by all askers; no way to ask for a faster or finer one.
- **Fingerprinting through the memory table**: serials, asset tags and the
  UUID never leave vdm; what does is what a shop's spec sheet says.
- **A parser bug in root** (a hostile or broken firmware table): vdm's
  SMBIOS parser is small, bounds-checked, fuzzed, and runs in vdm's forked
  worker (as PAM checks do), never in vdm's main loop.
- **Someone over ssh, or another account**: elevate's rule; only a person
  with a session at a console gets an answer.
- **Wear**: none; nothing is written, and the energy counter is a register.

## Steps

1. vergence-sysinfo: SPD decoding (DDR4 `ee1004`, DDR5 `spd5118`) for rated
   speed and timings; Memory > "Rated" in Hardware. **No root; can be built
   now.**
2. vergence-dm: the read socket, `memory.modules` (the SMBIOS type 17
   parser, fuzzed, in the worker) and `power.package` (1 s, shared,
   rounded), the console check, the log line.
3. Hardware: Memory > "Modules", Processors > "Power now" and its chart;
   the "needs Vergence's login service" fallback.
4. Tests on all three machines, then an independent review of the vdm part
   (Dave's code security policy: a new root interface is a review trigger).

## Questions

1. **The elevation prompt for these readings:** (a) no prompt: vdm's
   filtered read service, as above (lean); (b) the prompt once each time
   Hardware opens, the yes good for that window's life (bends the
   2026-11-16 decision 3); (c) the prompt for the memory table only, power
   left out.
2. **Where the root code lives:** (a) in vdm, which is root already and
   already checks who asks (lean); (b) a small helper of its own, set-uid
   root, group vergence, mode 4750, like `sound-default` (a process per
   reading, and live power would mean one a second).
3. **Power's resolution:** (a) 1 s average, 0.1 W (lean; what other
   desktops show); (b) 1 s, whole watts; (c) power left to root's tools.
4. **FreeBSD:** (a) the same readings later, after Linux (lean); (b) Linux
   only, FreeBSD's rows say "not available on this system".
5. **Step 1 now?** The SPD readings need no root and no decision above;
   build them on hw-core-grid while the rest waits for the answers.
