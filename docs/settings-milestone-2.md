# Settings milestone 2: screens, cables, energy and window places

Drafted 2026-11-14 from Dave's afternoon notes (docs/next-settings.md, "From
Dave, 2026-11-14 afternoon"), first in docs/apps-roadmap.md's order ("That
looks good"). **Accepted 2026-11-14: "1 yes, 2 off, 3 more than one tab open
or an active tab present, 4 agree with settings"** (the keep-or-revert dialog;
the physical-size switch off by default; the terminal asks with several tabs
or a tab running something; the dock's Forget opens the settings app).

## Where things stand

- **Screens**: each screen runs its monitor's preferred mode; a screen's page
  has its scale, "Shown at" with what it comes to ("looks like 1280 x 720
  (3840 x 2160 pixels)", since GPUs milestone 1), size of controls, and which
  GPU shows and draws it. A scale is whole where close (the 14" at 300%,
  calculated 328%).
- **Network page**: each cable connected or not, with its addresses; no
  speed, no traffic. The Wi-Fi page has signal, speed, a chart and traffic.
- **Energy page**: screens off after, sleep after, one setting for all
  power; vergence-power knows battery or mains.
- **Window memory** (the server's memory.rs, ~/.config/vergence/
  server-windows.toml): places per app and per screen setup, kept until a
  window is closed for good or unseen 90 days; nothing forgets them by hand.
- **The terminal** closes a window and every tab in it without asking.

## Design

### Resolution and refresh rate

Each screen's page gets a **Resolution** choice (the monitor's modes, largest
first, the preferred one marked) and a **Refresh rate** choice (the rates of
that resolution). Bus protocol v9: `OutputInfo` lists the modes and the
current one; desktop.toml `[[displays.screen]] mode = "3840x2160@60"`, which
the server sets (the mode is checked against the connector's list, never
taken on trust). A new mode shows a **"Keep this resolution?"** dialog that
puts the old one back after 15 s unless Keep is pressed, so a mode the
monitor cannot show never strands anyone.

### The same physical size on every screen

A switch on the Screens overview: **"Windows the same size on every
screen"**. On: every screen whose scale is not chosen by hand takes its exact
calculated scale (Contract 1's reference unit: the 14" at 328%, not 300%),
connected or remembered, so a window keeps its size in millimeters moving
between screens. Off (as now): whole numbers where close, for sharpness. The
overview says the cost in a line ("slightly soft in apps that only draw at
whole scales").

### Cables like Wi-Fi

The Network page's cable shows its **link speed and duplex** (1 Gb/s, full)
in the connected row, and **Traffic** in and out with a two-minute chart (the
Wi-Fi page's), sampled every 2 s while the page is open. vergence-net reads
them: Linux from /sys (speed, duplex), FreeBSD from the interface's media
(SIOCGIFMEDIA), the byte counters it already has.

### Energy on battery and on power

The Energy page in two columns where there is a battery: **On battery** and
**On power**, each with screens off after and sleep after. desktop.toml
`[energy]` stays the on-power one; `[energy.battery]` is new, and absent
means the same as on power. The server's energy.rs follows the mains state
(vergence-power, read every 30 s and on input) and uses the matching pair.
A UPS's battery joins with the UPS page (not this milestone).

### Forget an app's window places

- **The Windows page**: a "Window places" group listing the apps with saved
  places (read from the server, not the file), each with **Forget**, and
  **Forget all**; each asks first in a dialog ("Forget where Terminal's
  windows go? They open in the default place next time.").
- **The dock's app menu**: "Forget window places..." opens the same dialog
  (the settings app, at the Windows page).
- Bus protocol v9: `ListPlaces` and `ForgetPlaces { app }` (or all); the
  server clears the app from window memory for every setup and size and
  saves. Open windows stay where they are.

### Closing a terminal with several tabs

Closing a terminal window asks first when it has **more than one tab, or a
tab running something** other than its idle shell (a build, an SSH session,
an editor: the tab's shell has a child process), with Close and Cancel, a
modal dialog over the window (window styles milestone 1): "Close 3 tabs?",
or "A program is running in this tab (ssh). Close it?". One idle tab closes
as now.

## Progress

- **Step 1 in: bus protocol v9** (2026-11-14). `OutputInfo::modes` (each
  mode once: pixels, refresh in millihertz from the pixel clock and totals
  so 59.94 and 60 stay apart, preferred) and `mode` (the running one's
  index); `ListPlaces`, `ForgetPlaces { app }` (empty: all) and `Places`
  (app, count); vergence-profile's `WindowStore::apps`, `forget_app`,
  `WindowId::app_name` (class, else instance, lower case), tested; the
  server answers from window memory and saves. `vbus outputs` prints the
  modes (`*` running, `+` preferred), `vbus places` the apps. Hubble: the
  panel 1920x1080 at 144 (preferred, running) and 60 Hz; 13 apps with
  places (the terminal 124). kepler (the 14" moved back): 3840x2160 at 30 Hz
  (its HDMI's limit; 60 on Hubble's NVIDIA HDMI), 34 modes, the 200% chosen
  on Hubble followed the monitor. All three on v9. 477 tests (kepler 473).
- **Step 2 in: resolution and refresh rate** (2026-11-14). Each screen's
  page: Resolution (the monitor's sizes, its preferred one "(recommended)")
  and Refresh rate (that size's rates, fastest first; "59.94 Hz" apart from
  "60 Hz"), written as `[[displays.screen]] mode = "2560x1440@60"`; the
  server picks that mode only if the connector offers it (within 0.05 Hz;
  else its preferred, logged) and sets a screen up again when its chosen
  mode changes. A new mode opens "Keep this resolution?" (modal, a countdown
  driven from its waker), and the settings window's own 15 s timer puts the
  old choice back unless Keep is pressed, so closing the dialog reverts too.
  Found on the way and fixed: dialogs took window memory's place and size
  of the program's last closed window (the dialog came up 840 x 1044, its
  buttons off the screen): windows with a parent, dialogs, palettes and
  splash screens now have no memory identity, and the toolkit opens a
  dialog or palette at its own smallest size; the dialog used the toolkit's
  default theme, now the app's. kepler (the 14", 4K at 30 Hz): 2560 x 1440
  at 60 picked, not kept: back after 15 s; picked and kept: stayed; put back
  to 4K. Hubble's panel 144 to 60 Hz and back (by desktop.toml). 479 tests
  (kepler 475).
- **Rates labeled** (Dave: "label it"): a rate a thousandth below a whole
  one (59.94, 29.97, 23.976) reads "59.94 Hz (video)": television's rates,
  kept for video made at them.
- **Step 3 in: the same physical size** (2026-11-14). desktop.toml
  `[displays] same-size = true`: every screen whose scale is not chosen by
  hand takes its exact calculated scale (in 120ths, never below 1) instead
  of the nearest whole number; a chosen scale still wins. The calculation
  includes each screen's viewing distance, so it is the same size to the
  eye (Contract 1's reference unit), the same in millimeters on screens at
  the same distance. The switch on the Screens overview, and on a single
  screen's page (it decides for screens met later too). Hubble's panel: off
  100%, on 104% ("the same size as on every screen"), off again.
- **Step 4 in: cables like Wi-Fi** (2026-11-14). vergence-net's `Iface`
  has the link speed and duplex: Linux from /sys (`speed`, `duplex`),
  FreeBSD from the interface's media word (SIOCGIFMEDIA, tested on the
  subtype table); `vnet` prints them. The Network page's connected row
  reads "Connected at 1 Gb/s, full duplex", and a Traffic row has in and
  out with the Wi-Fi page's two-minute chart, sampled every 2 s while the
  page is open. kepler's em0: 1000 Mb/s full, shown. Also (Dave's ask):
  **no Wi-Fi where there is no Wi-Fi hardware**: the sidebar's Wi-Fi entry
  and the Network page's Wi-Fi group are hidden (checked again on each
  tick, so a USB card plugged in brings them back). kepler's sidebar has
  Network only, Hubble's Wi-Fi and Network. This replaces settings
  milestone 1's "kepler says no Wi-Fi device" page. 481 tests (kepler 477).
- **Step 5 in: energy on battery and on power** (2026-11-14). desktop.toml
  `[energy.battery]` (screens-off, sleep; a key left out means the same as
  on power); vergence-power's `Power::on_battery` (the mains, else a
  battery running down); the server reads it on a thread every 30 s (a
  machine with neither battery nor mains report is read once) and uses the
  matching pair from the current idle clock, logging each change. The
  Energy page, where there is a battery: "On battery" and "On power"
  groups, the one in use marked "(now)", followed while the page is open;
  without a battery one group as before. The settings app's `--page` now
  takes any sidebar page by name (`--page energy`), for the dock's Forget
  next. Hubble, battery screens off at 1 min, power never: unplugged, the
  server saw the battery 33 s later and, idle 2 min already, turned the
  screens off at once; plugged in, back to the power times. 483 tests
  (kepler 479).
- **Asked on the way** (Dave, 2026-11-14): the settings app's bar is light
  by default (with Style "App default"; the server's decor.rs `bar_light`,
  a built-in list until the app settings milestone makes it each app's
  choice), and the buttons of a bar without tabs (plain and sidebar bars)
  stand in the bar's middle; a bar with tabs keeps them at its bottom edge
  with the tabs.

## Steps

1. **Protocol v9**: modes and the current mode in `OutputInfo`; `ListPlaces`,
   `ForgetPlaces`; tests; all three machines reinstalled together.
2. **Resolution and refresh rate**, with the keep-or-revert dialog.
3. **The same physical size switch.**
4. **Cables like Wi-Fi** (vergence-net: speed, duplex on both systems).
5. **Energy on battery and on power.**
6. **Forget window places**: Windows page, dock menu, dialogs.
7. **The terminal's confirmation.**
8. **Docs**: whitepaper §6.9 and §9, README, CLAUDE.md, this plan.

## Acceptance

1. Tests green on Hubble, kepler and Techbench.
2. On Hubble: the 14" switched to 1920 x 1080 and back, and a refresh rate
   changed on the panel (144 to 60 Hz); a mode not kept reverts by itself.
3. The physical-size switch on: the 14" at its calculated scale; a window
   moved between the panel and the 14" keeps its size (measured).
4. Hubble's cable shows 1 Gb/s full duplex and traffic; kepler's em0 too.
5. On battery, the battery pair applies (screens off sooner); on power, the
   other.
6. An app's places forgotten from the Windows page and from the dock: its
   next window opens in the default place; Forget all empties the list.
7. Closing a terminal with three tabs asks, and with one tab running `sleep
   100` asks; Cancel keeps it; one idle tab closes without asking.

## Not in this milestone

The UPS page (needs a UPS by USB on one of the machines), low-battery actions
(Dave: "not yet"), the apps in docs/apps-roadmap.md.

## Questions for Dave

1. **The keep-or-revert dialog** after a resolution change, 15 s: (a) yes;
   (b) no dialog. Proposed: **(a)**, every desktop does it for good reason.
2. **The physical-size switch's default**: (a) off (sharp whole scales, as
   now); (b) on. Proposed: **(a)**.
3. **The terminal's question**: (a) only with several tabs, as you said; (b)
   also with one tab running something other than the shell (a build, an
   SSH session). Proposed: **(a)** now, (b) worth a later look.
4. **The dock's Forget**: (a) opens the settings app's dialog at the Windows
   page; (b) a quick question in a notification card. Proposed: **(a)**: one
   dialog, one place to manage it.
