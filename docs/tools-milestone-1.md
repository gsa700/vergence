# Tools milestone 1: Calculator, Running Applications, Hardware

Drafted 2026-11-14, item 3 of docs/apps-roadmap.md: three small native
tools from Dave's list, each small, together settling the app framework
every later app stands on.

**Accepted 2026-11-14** (Dave): "1 a" (a keypad window), "2 a" (the tape
for the session only), 3 RPN left to me: not now, the arithmetic and the
window kept so an RPN mode can come later; "4 this is meant to be a simple
app stopper in the mac tradition - others can use terminal - any of the
names is fine" (Running Applications is macOS's Force Quit Applications:
applications only, no process table), "5 a" (other users' processes are
not the app's business: it shows only the user's own applications), "6 a"
(Calculator, then Running Applications, then Hardware).

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

1. **A list** in vergence-widgets: rows with an icon and words, a selected
   row (keys and pointer), rows updated in place without losing the
   selection or the scroll, Contract 3 meaning (a list, its items, named).
   Running Applications needs it first; the table with headings and
   sorting comes with the file manager.
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

A simple app stopper in the Mac tradition (Dave, 2026-11-14; macOS's Force
Quit Applications); anything more is the terminal's.

- **A list of the user's running applications**, as the dock knows them:
  icon and name, and "not responding" after an application has not
  answered for a few seconds (the server asks each window's client: a
  ping on the native protocol, xdg-shell's own ping for Wayland, the X11
  window manager's ping where the client offers it).
- **End**: the application's windows are asked to close, as their close
  buttons ask (the terminal's question still comes); **Force Quit**, after
  a question: its processes are sent SIGKILL. Opened from the launcher
  like any app, and by **Ctrl+Alt+Escape**, a server key, as macOS has
  Cmd+Option+Esc.
- **Pick a window**: the next click on any window selects its application
  (the server tells which, by the window's client), for a stuck window
  whose application is hard to find.
- Only the user's own applications: no other users' processes, no root.

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
  its own, `vergence-sysinfo`.

## Steps

1. **Toolkit**: About, the app skeleton.
2. **Calculator**: the decimal arithmetic (tests first: the precision, the
   functions, rounding), then the window, keys, tape, scientific mode,
   settings.
3. **Toolkit and server for Running Applications**: the list widget; the
   server's ping and "not responding", the application list and "pick a
   window" on the bus (v12).
4. **Running Applications.**
5. **vergence-sysinfo and Hardware**: CPU, memory, disks, temperatures on
   Linux and FreeBSD (tests against known /proc and sysctl texts); the
   chart moved from the settings app; the pages.
6. **Docs**: whitepaper (the apps, §9), the Map, desktop entries and
   install.sh for the three.

## Acceptance

- Calculator: 0.1 + 0.2 = 0.3; 1 ÷ 3 × 3 = 1; sin 30° = 0.5 exactly;
  the number pad and Enter work; the tape copies a line.
- Running Applications: an app stopped with `kill -STOP` shows as not
  responding and Force Quit ends it; End closes an app's windows as their
  close buttons do; "Pick a window" selects the window's application.
- Hardware: every reading on Hubble (Void, two GPUs, a battery), kepler
  (FreeBSD) and Techbench (Fedora); charts move only while shown.
- Each app: app button with its commands, its settings window, About, its
  windows remembered; full suites on all three machines.
