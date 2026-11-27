# Hardware readings milestone 1: what only root may read, behind Advanced

*Drafted 2026-11-26 on techbench; rewritten the same evening around Dave's
model: "put the user readable data into the Hardware app, and use an
elevation prompt under the 'advanced' model used elsewhere in vergence".
Follows the Hardware app's Details (branch hw-core-grid, 6fa0bbb), which
show what any user may read. Nothing here is built yet.*

## Why

The Hardware app shows the processor, its caches and clocks, the board and
firmware, and the graphics card, all without root. What people still look
for, and CPU-Z and GPU-Z show:

1. **The memory as it runs:** its speed now (which may be the modules'
   XMP/EXPO profile, not their rated speed), the slots in use, each
   module's maker, part number and size.
2. **The memory's rated speed and timings** (CL-tRCD-tRP-tRAS).
3. **Power now** in watts: the processor's package, and the graphics' where
   the card or the chip reports it.

CPU-Z gets all of it on Windows by loading its own kernel driver, which
needs administrator rights: one UAC prompt, and the whole program sees more
for as long as it runs. Linux already gives any user most of it; what is
left needs root, and that part goes behind a prompt the person asks for.

## What this machine says, and who may read it (techbench, 2026-11-26)

| Reading | Where | Who may read it |
|---|---|---|
| Rated speed and timings (2) | each module's SPD chip: `/sys/bus/i2c/devices/*/eeprom` (the kernel's `spd5118` driver for DDR5, `ee1004` for DDR4) | **any user** (0444) |
| Module maker, part number (1) | the SPD chip's second half | any user, **but** techbench's board blocks the chip's page switch, so the second half reads back as a copy of the first |
| Speed now, slots, maker, part number, size (1) | the firmware's memory table (SMBIOS type 17): `/sys/firmware/dmi/tables/DMI` | root only (0400) |
| Processor package power (3) | `/sys/class/powercap/intel-rapl:0/energy_uj` | root only (0400) since PLATYPUS (CVE-2020-8694) |
| Integrated graphics power (3) | the same counter's "uncore" part, `intel-rapl:0:1` (the UHD 770 has no sensor of its own) | root only (0400) |
| Graphics clock | `/sys/class/drm/card0/gt_act_freq_mhz` (i915) | any user (shown today) |

techbench's modules decode as DDR5, rated 4800 MT/s (tCKAVGmin 416 ps),
tAA/tRCD/tRP 16.0 ns, tRAS 32.0 ns: JEDEC 40-39-39-77 at 4800.

Other graphics cards, from their drivers (to check on the hardware when
the lab has it): AMD's amdgpu gives any user power, temperature, fan, busy
percent, memory used and its firmware's version (`device/hwmon/*`,
`gpu_busy_percent`, `mem_info_*`, `vbios_version`): nothing needs root.
Intel Arc reports its own energy through hwmon (who may read it: to
check). NVIDIA's own driver reports only through NVIDIA's library (NVML):
borrowing, Dave's call, parked. nouveau reports little either way.

Why each root-only reading is root's: the firmware table holds serial
numbers, asset tags and the machine's UUID (fingerprints; the kernel
already gives any user the rest of the same table in `/sys/class/dmi/id`).
The power counters were closed because sampling them every few
microseconds to milliseconds lets a program recover secrets (keys) from
how much power the processor draws.

## The model: Advanced, opened on purpose, asks

As the settings app does (settings milestone 1): each page opens on what
anyone may read, with a collapsed **Advanced** part at its end.

- **Every page shows what any user may read, without a prompt**: all of
  today's Details, plus Memory > "Rated" (speed and timings from the
  modules' own chips), plus everything an AMD card gives.
- **Memory, Processors and Graphics each get an Advanced part** holding
  the root-only readings: Memory > Advanced: each slot, the speed now,
  each module's maker, part number and size; Processors > Advanced:
  package power now and its minute's chart; Graphics > Advanced: the
  integrated graphics' power (uncore) and its chart, where the card has no
  sensor anyone may read.
- **Opening an Advanced part is what asks**, with the elevation prompt
  (docs/elevation-milestone-1.md): drawn by the server, the administrator's
  own password, checked by vdm through PAM. Inside the part, before the
  yes: one line, "These readings need an administrator's password", and a
  **Show** button; the prompt comes only from that click.
- **The yes is a read lease for that window**: it covers every Advanced
  part of that Hardware window (open one, the others need no new prompt)
  until the window closes, the program ends, or the screen locks. The next
  look asks again.
- **Never a prompt by itself**: an Advanced part starts closed each time
  Hardware opens (the settings app remembers its open parts; this one must
  not, or opening Hardware would bring up a prompt). Only a click asks.

Someone who never opens Advanced never sees a prompt; someone who wants to
see more asks for it, as with CPU-Z.

## How it works

1. **The lease.** A new elevate action, `hardware.readings`: the prompt says
   "Let Hardware read the memory's slots and the power counters" and who
   asks (as every prompt does, from the program's file). On a yes vdm
   gives the asking program a lease: 128 random bits only vdm and that
   program know, tied to its pid. vdm ends it when the program goes away,
   when the program says it is done (its window closed), or when the
   person's screen locks.
2. **The reads**, on vdm's elevate socket, each with the lease:
   - `memory.modules`: vdm parses SMBIOS type 17 itself (our own parser, in
     its forked worker, fuzzed; FreeBSD: the table found through `kenv
     hint.smbios.0.mem`, later) and returns, per slot: locator ("DIMM_A2"),
     size, type, speed now and rated, maker, part number. **Never** serial
     numbers, asset tags or the system UUID, lease or not.
   - `power.package` and `power.graphics`: vdm reads the counters itself,
     **at most once a second**, and returns the average watts over the last
     second, rounded to 0.1 W; every lease holder shares the one reading,
     and vdm reads only while some lease holder asks (nothing while no
     Advanced part is open: mind the battery).
3. **vdm still does the reading and the filtering**: the Hardware app never
   runs as root and never sees a raw table or a raw counter. A yes grants
   a view, not a hole.
4. **Logged** as every elevate request is: the lease (who, which program,
   yes or no) and its end; not each one-second read.

What it never does: hand out raw tables or serials, read a counter faster
than once a second, give a lease to anyone not at the machine (elevate's
rule), outlive the window, the program or the unlocked screen, or let a
lease change anything.

## A change to a decision

Elevation milestone 1, question 3 (Dave, 2026-11-16): "a yes is never
remembered, every action asks". This plan needs a written exception:
**a yes may cover a read-only view for as long as it stays open; it never
covers a change.** Every action that changes something still asks each
time.

## Threats, and what answers each

- **PLATYPUS through vdm**: one reading a second, averaged and rounded,
  shared by all; no way to ask for a faster or finer one, lease or not.
- **Fingerprinting through the memory table**: serials, asset tags and the
  UUID never leave vdm.
- **A lease stolen or reused**: 128 random bits, tied to the pid that asked;
  a read from any other process with it is refused and logged; it dies
  with the program, its window, or the lock.
- **A program asking to wear the person down**: elevate's existing pause
  (10 s, doubling to 5 minutes after a decline) and one request at a time.
- **A parser bug in root** (a hostile or broken firmware table): vdm's
  SMBIOS parser is small, bounds-checked, fuzzed, and runs in vdm's forked
  worker, never in its main loop.
- **Someone over ssh, or another account**: elevate's rule; only a person
  with a session at a console can get a lease.

## Steps

1. vergence-sysinfo: SPD decoding (DDR4 `ee1004`, DDR5 `spd5118`) for rated
   speed and timings; Memory > "Rated" in Hardware; amdgpu's power,
   temperature, fan, busy and memory where present. **No root, no decision
   pending: can be built now.**
2. The toolkit: the settings app's Advanced part (`disclosure` in
   vergence-settings' look.rs) moves into vergence-widgets, so Hardware and
   any app can use it (the settings app keeps its look; control's call on
   where it sits).
3. vergence-dm: `hardware.readings` (the prompt's words, the lease and its
   ends), `memory.modules` (the SMBIOS type 17 parser, fuzzed, in the
   worker), `power.package` and `power.graphics` (1 s, shared, rounded,
   only while asked), the log lines.
4. Hardware: the Advanced parts on Memory, Processors and Graphics (the
   line and Show, then the readings and charts); the lease given back when
   the window closes; with no vdm (another desktop), the part says
   "needs Vergence's login service".
5. Tests on all three machines, then an independent review of the vdm part
   (Dave's code security policy: a new root interface is a review trigger).

## Questions

1. **When the lease ends:** (a) the window closes, the program ends, or the
   screen locks (lean); (b) also after a set time (an hour?); (c) only when
   the window closes or the program ends.
2. **One prompt for all Advanced parts of a window** (lean), or one per
   page (Memory's yes not covering power)?
3. **Power's resolution:** (a) 1 s average, 0.1 W (lean; what other
   desktops show); (b) 1 s, whole watts.
4. **FreeBSD:** (a) the same readings later, after Linux (lean); (b) Linux
   only, FreeBSD's Advanced parts say "not available on this system".
5. **The exception to elevation's question 3** as worded above: agreed?
6. **Step 1 now?** It needs no root and no answer above; build it on
   hw-core-grid while the rest waits.
