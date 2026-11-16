# Dock milestone 1: "Dockworkers" (live tiles, the drawer, drag and drop)

**Drafted 2026-11-16** (Dave: "let's be Dockworkers today"). The first
core-vision milestone under the alternating order (docs/vision-check-2026-11-15.md):
**live tiles with named desktop actions**, as designed in whitepaper §6.9
(13 November) and the shell-layers mock-up, together with Dave's wish from
~/vergence-app-ideas (14 November): **drag and drop the dock's icons and
tiles**, the dock itself moved by its menu or a modifier and drag.

Questions for Dave are at the end; nothing is built until they are answered.

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

21. Small publishers in tools/lab/ (shell scripts with `vtile run`, or one
    small Rust program if that proves neater): **UPS** through NUT (`upsc`),
    **time source** through chrony (`chronyc tracking`), **rack
    temperatures**, **Proxmox** (hosts up, guests running). Each is a
    separate script started by the session, so a lab without one simply
    lacks that tile.

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

23. The Dock page gains a Tiles part: on the dock or in the drawer, in
    order, with the same drag as the dock, and a switch for "a tile in
    trouble comes out by itself".

## Steps

1. Dock drag and drop for application icons (1-3, 7), Alt+drag and "Move
   dock" (6). Small, and pays off at once.
2. Tiles in the server: the model, drawing on the dock, cards, clicks (8-11).
3. Bus v19 and `vtile` (12-13); vctl gains `tiles`.
4. The desktop's own tiles: clock, sound, network, power (14-17).
5. The drawer, trouble coming out, tiles dragged between dock and drawer (4, 18-20).
6. Dragging from the launcher onto the dock (5).
7. Lab publishers (21).
8. Named actions and the launcher (22).
9. The Dock page's Tiles part (23); whitepaper §6.9 and its PDF; CLAUDE.md.

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
   which host?), rack temperatures (from what?), Proxmox (API token on
   10.0.1.6?).
5. **Named actions** in this milestone (step 8), or kept for the next core
   milestone so this one ends at the drawer and the lab? *Suggested: in
   this one, the list and the launcher only.*
