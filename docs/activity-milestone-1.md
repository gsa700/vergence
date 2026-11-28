# Activity milestone 1: Hardware and a process monitor in one app

*Drafted 2026-11-28 on Control from Dave's wishes of 2026-11-27
(docs/apps-roadmap.md, "From Dave, 2026-11-27": a process monitor showing
each process's kind of core, and About, Hardware and the process monitor
aligned) and his word of 2026-11-28: "It might make sense to merge Hardware
with an activity (process) monitor app", then "we can link the About page
with the related sections of the Activity Monitor app". The questions below
wait for his answers; nothing is built.*

**Decided already (Dave, 2026-11-28):**
- Hardware and the process monitor become **one app**, as Windows 11's Task
  Manager is: Hardware's pages plus a Processes page.
- **Force Quit stays its own small window** on Ctrl+Alt+Escape: the way out
  when something is stuck, tiny and instant (the Mac keeps both too).
- **About links into the app**: each of its summary lines opens the matching
  page.

## Why

Three places describe the same machine today, each in its own words:
Settings > About ("This computer"), Hardware's "This computer" page, and,
once it exists, a process monitor reading the same processors again. One
app with one set of readings shows a process's load beside the core grid
it runs on, and About stays a short summary that hands over to it.

## Where things are today

- **Hardware** (vergence-hw, one file): pages This computer, Processors,
  Memory, Storage, Network, Graphics, Power; read-only, charts the last
  minute, readings from vergence-sysinfo, vergence-net and vergence-power
  once a second. It opens on its first page and takes no page argument.
- **Settings > About**: its own summary (System, Computer, Processor,
  Cores, Graphics, Memory, Screens, Up for), built in Settings from the same
  crates but worded separately, a Copy button, and "More in Hardware…",
  which starts vergence-hw on its first page.
- **Force Quit** (vergence-quit): the user's running applications; the
  server ends one after a question (its forcequit.rs), only the user's own.
- **No process list** anywhere.

## The plan for milestone 1

1. **One source of facts:** the machine summary (name, system, processor,
   cores, memory, graphics, firmware) moves into vergence-sysinfo, worded
   once; About and Hardware's This computer page both show it, so they can
   no longer disagree.
2. **Pages by name:** the app opens at a page (`--page processors`), and
   a running copy is brought forward at that page rather than a second one
   started.
3. **About's links:** each summary line opens its page (Processor and
   Cores: Processors; Memory: Memory; Graphics: Graphics; Computer and
   System: This computer; Screens: Settings' own Screens page). Copy stays.
4. **The Processes page:** a list of processes, updated every second,
   sortable by each column: name (the app's name and icon where it is an
   app), CPU, the kind of core it ran on last (Performance, Efficiency,
   or both across its threads, on hybrid processors only), memory, and disk
   reads and writes. Selecting one shows its details (command line, started,
   threads, owner). Linux: `/proc/<pid>/stat` and `task/*/stat` (field 39,
   the last CPU, mapped through `topology()`), `status`, `io`. FreeBSD:
   `kern.proc` through sysctl (`ki_lastcpu`, rusage).
5. **Ending a process** from the Processes page: asked first, then handed
   to the server as Force Quit is (question 4).
6. **Checks:** unit tests for the readings from fixture files (as
   vergence-sysinfo's are), the core-kind mapping on a stand-in topology,
   a display suite scenario opening the app at a page from About, and the
   list on Hubble, techbench (hybrid: P and E cores) and kepler (FreeBSD).

## Questions

1. **The app's name:** (a) keep **Hardware**; (b) **Activity Monitor**, as
   Dave called it (Force Quit is named as on the Mac too); (c) **System
   Monitor**. *Lean: (b); it says what the merged app is for, and the Mac
   precedent is already Vergence's for Force Quit.*
2. **Which processes:** (a) everyone's, others' read-only, their fields
   shown where the system lets any user read them; (b) only the person's
   own. *Lean: (a), as Activity Monitor and top do; on Linux another user's
   disk counts are hidden from you, so those cells stay empty.*
3. **Network per process:** Linux keeps no per-process traffic a user can
   read (it needs root or kernel tracing). (a) leave it out, the Network
   page charts the machine's; (b) later, through a helper with elevation.
   *Lean: (a) now.*
4. **Ending from the Processes page:** (a) the person's own processes,
   asked first, sent to the server like Force Quit (a polite stop, then
   forced if it does not end); others' never; (b) apps only, as Force Quit;
   (c) no ending here, Force Quit only. *Lean: (a).*
5. **About's links:** (a) each line clickable, with a small › at its end;
   (b) one "More in Activity Monitor…" button as now, opening the page that
   fits best. *Lean: (a).*
6. **Where it opens from:** the launcher as Hardware does now; and should
   Force Quit offer "Open Activity Monitor" for anything harder? *Lean:
   yes, one button in Force Quit.*

**Not in this milestone:** energy per process (the Mac's Energy tab),
GPU per process (needs per-driver counters), and a dockworker tile for the
busiest process.
