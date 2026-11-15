# Tools milestone 1: Calculator, Running Applications, Hardware

Drafted 2026-11-14, item 3 of docs/apps-roadmap.md: three small native
tools from Dave's list, each small, together settling the app framework
every later app stands on. **Draft: the questions at the end are for Dave.**

## Where things stand

- **The app framework so far** (app settings milestone 1): the app button
  and its menu from commands with roles (vergence-protocol commands.rs),
  each app's settings window (vergence-widgets prefs.rs), per-app look
  under the desktop's master switch, window memory by persistence token,
  one process for all of an app's windows (the terminal's program.rs),
  the dialog kind for questions. About is a notification card from the
  desktop entry for any app that has no About of its own.
- **Widgets** (vergence-widgets): label, button, switch, slider, field,
  choice, scroll, picture, drawing, rows and columns. No list or table
  with selection and sorting; charts exist only inside the settings app
  (wifi.rs: signal and traffic).
- **What the system already reads for us**: vergence-power (batteries,
  mains, each GPU awake), vergence-net (interfaces, addresses, byte
  counters, Wi-Fi), the server's gpus.rs (every GPU, its driver and
  outputs), vergence-sound. No reading of processes, CPU load,
  temperatures, memory or disks yet.

## Design

### What the framework gains (used by all three, then by every app)

1. **A table** in vergence-widgets: rows of cells under headings, a
   selected row (keys and pointer), sorting by a heading, rows updated in
   place without losing the selection or the scroll, Contract 3 meaning
   (a table, its rows and cells, named). Running Applications needs it
   first; the file manager and the text editor's lists later.
2. **A chart** in vergence-widgets, lifted out of the settings app's Wi-Fi
   page: a line or two over the last N samples, with a scale, in the
   accent and a second color; Hardware uses it for every reading, the
   settings app goes on using it.
3. **About** for our apps: a small dialog from the toolkit (name, icon,
   version, what it is, the license) that the About role opens, the same
   in every app.
4. **An app's skeleton**: what a new native app must give (id, name,
   icon, desktop entry, commands, settings rows, About text) gathered in
   one place, so each later app starts from it. The three tools are its
   proof.

### Calculator (`vergence-calc`)

- **Basic**: a display with the expression and the result, keys 0-9, the
  four operations, percent, sign, clear and backspace, by pointer, touch
  and keyboard (the number pad too). **Scientific** (View in its menu, or
  a switch in the window, kept by its settings): powers and roots,
  logarithms, trigonometry in degrees or radians, π and e, parentheses,
  memory.
- **Exact decimal arithmetic of our own**: numbers held as decimal digits
  (34 significant, as IEEE decimal128 has), so 0.1 + 0.2 is 0.3; the
  scientific functions computed to that precision. No floating-point
  surprises, no borrowed library.
- **A tape**: the calculations of this session, each one a line to copy or
  reuse; kept until the app closes (or kept between runs: question 2).
- Its menu: New window, Copy result, the Scientific switch, Text size;
  its settings: angle unit, decimal places shown, the thousands separator.

### Running Applications (`vergence-procs`)

- **Applications first**: each running application as the dock knows it
  (icon, name, its windows), with its processes' CPU and memory summed;
  opened, a table of its processes. **All processes** as a second view: a
  table of every process (name, user, CPU, memory, started), sortable,
  with search.
- **End and Force quit**: End asks politely (the app's windows are asked
  to close first, then SIGTERM), Force quit sends SIGKILL after a
  question; processes of other users and of the system are shown but
  cannot be ended (no root here; §7.3).
- **By window**: "Pick a window" lets the next click on any window select
  its application (the server tells which, by the window's client; a
  native or Wayland window's pid, an X11 window's): the way to find what
  a stuck window belongs to.
- **Reading processes, our own code**: Linux /proc (stat, status, cmdline),
  FreeBSD the kern.proc sysctls; CPU from the change in each process's
  time between samples (every 2 s while open).

### Hardware (`vergence-hw`)

- **One page per kind, the sidebar of the settings app's shape**: CPU
  (model, cores, load per core with a chart, frequency, temperature where
  the system gives it), GPUs (from the server: name, driver, which screens,
  awake or asleep, and load and temperature where readable), memory (used,
  cached, swap, a chart), disks (each with its file systems, used and free,
  read and write rates; SMART health later, with the disk utility),
  network (each interface: addresses, link speed, traffic charts, from
  vergence-net), power (battery and mains, from vergence-power).
- Read-only, refreshed once a second while shown, nothing read while
  hidden.
- **Reading, our own code**: Linux /proc and /sys (stat, meminfo,
  hwmon, thermal_zone, diskstats), FreeBSD sysctls (kern.cp_times,
  hw.physmem, vm.stats, dev.cpu.N.temperature, kern.devstat). A crate of
  its own, `vergence-sysinfo`, that Running Applications shares.

## Steps

1. **vergence-sysinfo**: processes, CPU, memory, disks, temperatures on
   Linux and FreeBSD, with tests against known /proc and sysctl texts.
2. **Toolkit**: the table, the chart (moved from the settings app), About,
   the app skeleton.
3. **Calculator**: the decimal arithmetic (tests first: the precision, the
   functions, rounding), then the window, keys, tape, scientific mode,
   settings.
4. **Running Applications**: the two views, End and Force quit, the
   server's "pick a window" (bus v12: a pick request and its answer).
5. **Hardware**: the pages.
6. **Docs**: whitepaper (the apps, §9), the Map, desktop entries and
   install.sh for the three.

## Acceptance

- Calculator: 0.1 + 0.2 = 0.3; 1 ÷ 3 × 3 = 1; sin 30° = 0.5 exactly;
  the number pad and Enter work; the tape copies a line.
- Running Applications: a busy `yes` is found by its CPU, ended by End;
  a window picked with "Pick a window" selects its application; another
  user's process cannot be ended.
- Hardware: every reading on Hubble (Void, two GPUs, a battery), kepler
  (FreeBSD) and Techbench (Fedora); charts move only while shown.
- Each app: app button with its commands, its settings window, About, its
  windows remembered; full suites on all three machines.

## Questions for Dave

1. **Calculator's look**: (a) a keypad window like GNOME's and macOS's
   calculators (recommended for basic; scientific widens it), or (b) an
   expression line first, with the keypad below as an option?
2. **The tape**: (a) kept for the session only (recommended), (b) kept
   between runs.
3. **RPN**: do you want an RPN mode (HP style) beside the algebraic one?
4. **Running Applications' name**: keep "Running Applications", or
   "Activity" / "Processes"?
5. **End for processes of other users**: (a) shown, cannot be ended
   (recommended; no root in the app), (b) ask for a password and end them
   through a small root helper as the sound one does on FreeBSD?
6. **Order**: (a) Calculator first, then Running Applications, then
   Hardware (recommended: the smallest proves the skeleton), or another?
