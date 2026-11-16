# Dock milestone 1: "Dockworkers" (live tiles, the drawer, drag and drop)

**Drafted 2026-11-16** (Dave: "let's be Dockworkers today"). The first
core-vision milestone under the alternating order (docs/vision-check-2026-11-15.md):
**live tiles with named desktop actions**, as designed in whitepaper §6.9
(13 November) and the shell-layers mock-up, together with Dave's wish from
~/vergence-app-ideas (14 November): **drag and drop the dock's icons and
tiles**, the dock itself moved by its menu or a modifier and drag.

**Accepted 2026-11-16** (Dave): "1 a" (Alt+drag and "Move dock"), "2
both" (dragging off removes, and Unpin stays in the icon's menu), "3 a"
(applications and tiles keep their own parts), "4 kept to a minimum enough
to develop and prove the concept" (one real lab tile, the rack UPS from the
NUT server, plus a `vtile` demo script that can fake a problem; the time
source, rack temperatures and Proxmox later), "5 a" (named actions in this
milestone: the list and the launcher only).

## Progress

- **Step 1 done** (2026-11-16): icons lift after 6 pt and the others close
  up round the place they would land (vergence-shell running.rs `gap_at`,
  `dropped`, with tests); a running application dropped among the pinned is
  pinned there; held 24 pt off the dock a pinned icon shows "Remove" on the
  windows' side and is unpinned when let go (Unpin stays in its menu);
  Escape puts everything back; Alt+drag moves the dock; "Move dock" in
  every dock menu (and a menu of its own on the apps button and empty parts)
  carries the dock's middle under the pointer until a press sets it down. A
  plain drag on the apps button does nothing. Seen on kepler with vctl:
  reorder, remove, pin Calculator by dropping, Escape, Alt+drag to the
  bottom, "Move dock" back to the right. Tests 638 Hubble/Techbench, 631
  kepler. By hand still: a mouse and a finger on kepler.
- **Step 2 done** (2026-11-16): vergence-protocol tile.rs (Tile, Card,
  TileState: the values a tile is drawn from, on the bus from step 3);
  vergence-shell tiles.rs (tile places after the applications, `tile_at`,
  the clock tile, local time and UTC by localtime_r/gmtime_r, with tests);
  the server's tiles.rs (the list by owner, the clock set each minute from
  the one-second tick, the card: opens after 250 ms on a tile, closes 200
  ms after the pointer leaves tile and card, drawn beside the tile on the
  windows' side with title, a large line, rows, a chart and small print;
  a click on a tile or the card's title opens its settings page); seat
  `Part::Card`. The dock draws tiles after a line: value over label, in the
  warning or problem color when not fine. Seen on kepler: the clock tile,
  its card, a click opening the settings app (no Date & Time page yet: it
  comes with step 4). Tests 641 Hubble/Techbench, 634 kepler.

- **Step 3 done** (2026-11-16): bus **v19**: `SetTile` (publish or change
  one of this client's tiles; checked by `Tile::check`: id, lengths, at most
  32 tiles, 16 rows, 240 chart values, 8 controls) and `RemoveTile`; events
  to the owner `CardOpened`, `CardClosed`, `TileClicked` (a tile with no
  page), `TileAction` (a card's control: button, switch or slider, defined
  now so step 4 needs no second bump). A client's tiles go when it leaves.
  Tiles are keyed by owner and id. `vtile show|run <id> [options]` in
  vergence-protocol (installed by install.sh); `vctl tiles`. Charts scale
  to their values' range. Seen on kepler: a vtile demo tile put up,
  changed to a warning (amber on the dock), its card opened, a click and
  the card's open and close printed by vtile, gone when vtile ended.
  Installed and restarted on all three (Hubble, Techbench's dev session,
  kepler). Tests 644 Hubble/Techbench, 637 kepler.

- **Step 4 done** (2026-11-16): vergence-shell tiles.rs builds the
  desktop's tiles from what was read (`sound`, `network` with
  `main_iface`/`rate_text`, `power`; the clock with `ClockLook`), with
  tests; the server's applets.rs reads them on a thread of its own (the
  network every second while its card is open, else every two; sound and
  power every four, sent only when changed) and carries out the desktop's
  card controls (volume, mute; a volume key updates the tile at once).
  Cards draw and work their controls: a button across the card, a switch,
  a slider (pressed or dragged, to its step; the hit area spans the card);
  the change shows at once and goes to the owner (`TileAction`) or to
  applets.rs. Tiles keep a fixed order: Sound, Network, Power, programs'
  tiles, the clock last. The settings app's **Date & Time** page
  (datetime.rs: `[clock] 24-hour`, `date`, the time zone from $TZ,
  /etc/localtime or /var/db/zoneinfo); the clock follows [clock] as soon as
  desktop.toml changes. After a click the card does not open again under
  the pointer. Seen on kepler: Sound, Network and clock tiles; the volume
  slider (90 %, then past its end to 100 %), mute on and off; the clock's
  click opening Date & Time (America/Chicago). Power is only on a machine
  with a battery (Hubble's Mobilenode): not yet seen. Tests 648
  Hubble/Techbench, 641 kepler.

- **Step 5 done** (2026-11-16): the drawer (the server's drawer.rs;
  vergence-shell tiles.rs `on_dock`, `DOCKED`, `drawer_layout` with tests).
  Which tiles stand on the dock is desktop.toml `[shelf] tiles` (unset: the
  desktop's Sound, Network, Power, clock); every other tile is in the
  drawer, whose tile stands first among the tiles with nine dots and how
  many it holds in the color of the worst state. A click opens it beside
  the dock: "Needs attention" first, then System, Lab, Background apps, the
  rest by name; four 60 x 52 cells a row. Each cell has its card (beside
  the drawer) and its click. It closes 500 ms after the pointer leaves it,
  its tile and the card, or on a press anywhere else. A tile in trouble
  comes out onto the dock while it lasts (`[shelf] trouble-out`, on unless
  set off). Dragging: out of the drawer onto the dock puts a tile on the
  dock; off the dock onto the drawer's tile or into the open drawer puts it
  in the drawer; Escape cancels; the tile is drawn under the pointer. No
  card opens during a press or a drag. Seat `Part::Drawer`. Power's label
  says Full or Plugged in when not charging. Seen on kepler with two vtile
  demo tiles (Lab): the drawer, a cell's card, UPS dragged out onto the
  dock and back off it into the drawer, NAS put in warning coming out by
  itself; kepler's desktop.toml put back afterwards. Seen on Hubble: the
  Power tile (100 %, Charging, before the label fix). Not yet: the filter
  field past twelve tiles (the drawer has no text field; it comes with the
  lab's tiles if they grow past twelve), reordering tiles on the dock.
  Tests 650 Hubble/Techbench, 643 kepler.

- **Step 6 done** (2026-11-16): a mouse press on an application in the
  launcher (its grid or the Vergence row) waits: let go there, it starts as
  before; moved 6 points, the launcher closes and the application goes
  with the pointer as a dock item does (shelf.rs `shelf_lift_new`,
  `FROM_LAUNCHER`), the dock making a place where it would land; let go on
  the dock it is pinned there, anywhere else nothing happens; Escape
  cancels. A finger in the launcher still scrolls (pin with its menu). The
  dragged icon shows the name's first letter until its icon is decoded at
  the dock's size, then the icon. Seen on kepler: Calculator dragged from
  the launcher's grid between Terminal and Firefox and pinned there;
  Hardware dragged and given up with Escape; kepler's pins put back.
  Tests 650 Hubble, 643 kepler (Techbench left alone at Dave's word).

## Where things stand

- The dock (shelf milestones 1 and 2): the apps button, pinned and running
  applications, marks, window lists, minimize into the icon, hiding, the
  launcher. Pins are kept in `desktop.toml [shelf] pinned`, ordered only by
  the Dock page's up/down buttons.
- **A press anywhere on the dock that moves 6 pt drags the whole dock**
  (`shelf_press`/`shelf_release`, shelf.rs). That is why icons cannot be
  dragged today: the gesture is taken.
- Files dropped on an application's icon open in it (files milestone 1,
  drag.rs). This must keep working.
- **No tiles exist.** No clock, volume, network or battery on the dock;
  the server does already read vergence-net, vergence-power and
  vergence-sound, and the settings app has the Wi-Fi chart the network card
  is to reuse (decision of 14 November).
- Notification cards (notify.rs) are the only thing other programs can put
  on the desktop today (bus v18 `Notify`).

## Design

### Moving things on the dock

1. **Drag an icon or a tile** (past 6 pt) and it lifts off the dock; the
   others part to show where it will land. Dropped on the dock it stays in
   its new place, and the order is saved.
2. **Pin by dropping.** A running application that is not pinned, dropped
   among the pinned ones, becomes pinned there.
3. **Remove by dragging off** (as macOS): held away from the dock, the icon
   shows a "Remove" mark; let go there and it is unpinned. A running
   application stays on the dock as running until it quits. The apps button
   cannot be removed.
4. **Tiles move the same way**, and between the dock and the drawer: dropped
   on the drawer's tile it goes into the drawer, dragged out of the drawer
   onto the dock it stays on the dock.
5. **From the launcher**: an application dragged from the launcher's grid
   onto the dock is pinned where it is dropped.
6. **Moving the dock itself** no longer happens from a plain drag:
   - **Alt+drag** anywhere on the dock moves it (Alt+drag already moves any
     window, so one rule: Alt+drag moves what is under the pointer);
   - **"Move dock"** in the dock's menu (right click or long press on its
     empty part or the apps button): the dock follows the pointer, or the
     finger, until a click or a lift sets it down. Edge snapping, floating
     and the saved place are as they are now.
7. Escape during any of these puts everything back where it was.

### Tiles

8. **A tile** has an id, its owner, a short value and a label (one line
   each), a state (fine, warning, problem), a group (System, Lab, Background
   apps), its card, the actions it offers, and where a click goes (a
   settings page by name, or the owner, told by an event).
9. **Its card** opens when the pointer rests on the tile (or a finger taps
   it): rows of name and value, an optional chart of recent values, a few
   controls (switch, slider, buttons) that are the tile's actions. A card's
   title goes where the tile's click goes.
10. **Live cards**: the server tells the owner when its card opens and
    closes; an owner may stream once a second while it is open and stays
    quiet otherwise.
11. **The server draws everything.** A tile is not a window: it is sharp at
    any scale, looks like the desktop, is read by the screen reader, and
    cannot draw anything misleading.
12. **Bus v19**: `PublishTile`, `UpdateTile` (only what changed),
    `WithdrawTile`; events `CardOpened`, `CardClosed`, `TileClicked`,
    `TileAction`. A tile goes when its owner's connection does.
13. **`vtile`**, a small command for scripts:
    `vtile set ups --value "23 m" --label UPS --state ok --row "Load=27 %"`,
    so a shell script or a cron job can publish without writing a bus
    client; it keeps a connection while it runs (`vtile run` reads lines of
    updates from its standard input).

### The desktop's own tiles

14. **Clock**: time and date; card with the day, UTC and the time source
    (chrony's view); click opens Date & Time.
15. **Sound**: volume; card with the slider and mute; click opens Sound.
16. **Network**: a mark only when something is wrong (no link, on a
    fallback, Wi-Fi weak); card with rates, latency and the Wi-Fi page's
    chart, live; click opens Network.
17. **Power**: battery or UPS runtime where there is one; card with charge,
    time left, mains; click opens Power.

### Applets for later (Dave, 2026-11-16: "some initial ideas")

- **Weather**: a publisher of its own (a network client, never the
  server), the temperature on the dock and the next hours in its card. Its
  source and the place it asks about are the user's choice and need Dave's
  word (the forecast service, and whether a location leaves the machine).
- **Calendar**: a month in the clock's card (the mock-up had one) needs a
  card part the protocol does not have yet (a grid of days), so a later bus
  version; events from a calendar file (ICS) later still.
- **System log**: the server's own log and the system's (dmesg, the
  journal or syslog), the newest lines in its card; a warning when an error
  has come in since it was last looked at.
- More from the mock-ups: CPU, memory, disk, NAS, firewall, Nextcloud,
  KeePassXC (the drawer's sample tiles).

### The drawer

18. A tile of its own, showing how many it holds and the color of the
    worst state among them. A click slides it out as a grid: trouble first,
    then by group; a filter field past twelve tiles. Each tile has its card
    and click there too. It closes half a second after the pointer leaves it
    and its cards (text in the filter keeps it open).
19. **A tile in trouble comes out by itself** onto the dock while the
    problem lasts, and goes back when it clears.
20. New tiles go into the drawer unless the user has put them on the dock
    before (remembered by id in `[shelf] tiles`).

### Dave's lab as the first publishers

21. Kept to a minimum (Dave): **the rack UPS** through NUT (`upsc`), a
    shell script in tools/lab/ with `vtile run`, and **tools/lab/vtile-demo**,
    which publishes a tile and can put it in trouble and out again. Each is
    a separate script, so a lab without one simply lacks that tile. The time
    source (chrony), rack temperatures and Proxmox come later on the same
    road.

### Named desktop actions (the start)

22. Every tile action is a **named action** ("Mute", "Wi-Fi off", "Open
    Power settings"), and the server adds a starter set of its own (screens
    off, lock when the lock screen exists, move this window to the next
    screen, minimize all). The launcher's search finds and runs them below
    the applications; `vctl action <name>` runs one. This is the command
    surface voice and agents will use later (§6.5, §7), with their
    permission questions; this milestone builds only the list and the
    launcher's use of it.

### Settings

23. **The Dock page: Applets instead of Pinned** (Dave, 2026-11-16:
    "remove the pinned apps section and replace it with some Dock Applets a
    user can activate and deactivate"). Pinning is done on the dock itself
    now (drag, the icon's menu, the launcher's Pin), so the Pinned list with
    its up/down buttons goes. In its place, **Applets**: every tile the
    desktop has (Clock, Sound, Network, Power) and every one a program has
    published (the rack UPS), each with its name, who supplies it, and a
    switch. Off means off: not on the dock, not in the drawer, and it does
    not come out when in trouble. Kept in `[shelf] applets` by tile id. A
    switch for "an applet in trouble comes out of the drawer by itself".
    "Applet" is the word in the settings app and the dock's menus; the code
    keeps "tile".

## Steps

1. Dock drag and drop for application icons (1-3, 7), Alt+drag and "Move
   dock" (6). Small, and pays off at once.
2. Tiles in the server: the model, drawing on the dock, cards, clicks (8-11).
3. Bus v19 and `vtile` (12-13); vctl gains `tiles`.
4. The desktop's own tiles: clock, sound, network, power (14-17).
5. The drawer, trouble coming out, tiles dragged between dock and drawer (4, 18-20).
6. Dragging from the launcher onto the dock (5).
7. The rack UPS and the demo publisher (21).
8. Named actions and the launcher (22).
9. The Dock page's Applets part in place of Pinned (23); whitepaper §6.9
   and its PDF; CLAUDE.md.

Each step is tested and installed on kepler and Techbench's dev session as
it lands; Hubble on Dave's word (restart only when needed).

## Acceptance

- Icons reordered, pinned and removed by hand with a mouse and a finger;
  the order survives a restart; files still open by dropping on an icon.
- The dock moved by Alt+drag and by "Move dock", to every edge and free.
- Clock, sound, network and power tiles on all three machines; cards open
  on rest, open the right settings page on click; the network card streams
  only while open (seen in the log).
- A `vtile` script's tile appears, updates, goes when the script ends.
- The drawer with Dave's lab tiles; pulling the UPS's mains (or a fake
  problem from `vtile`) brings its tile out, clearing puts it back.
- "mute" typed in the launcher mutes.
- Tests green on Linux and FreeBSD.

## Questions

1. **Moving the dock.** (a) Alt+drag *and* "Move dock" in its menu; (b)
   only "Move dock" in the menu; (c) Super+drag and the menu.
   *Suggested: (a).*
2. **Removing an icon.** (a) Drag it off the dock (macOS, with a Remove
   mark while it is off); (b) only by "Unpin" in its menu, drag only
   reorders. *Suggested: (a).*
3. **Order on the dock.** (a) Applications and tiles keep their own parts
   (apps button, applications, then the drawer and tiles), each ordered
   freely; (b) everything mixed in any order. *Suggested: (a).*
4. **Which lab tiles first**, and where each one's figures are reached
   from Control, kepler and Hubble: UPS (NUT server host?), time (chrony on
   which host?), rack temperatures (from what?), Proxmox (an API token on
   the Proxmox host?).
5. **Named actions** in this milestone (step 8), or kept for the next core
   milestone so this one ends at the drawer and the lab? *Suggested: in
   this one, the list and the launcher only.*
