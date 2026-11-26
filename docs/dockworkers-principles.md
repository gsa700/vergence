# Dockworkers: Principles

Version 2, 2026-11-25, after Dave's markup (written with techbench/claude). Agreed with Dave
("Dockworkers principles v2 AGREED", shared memory src:vergence-dockworkers-principles); brought
into the repo 2026-11-26. Dock milestone 2 is planned against it. Builds on dock milestone 1's design
(the server draws every tile, bus v19, vtile) and tonight's decisions (three depths, the list
in cards, the ribbon as a second drawer layout). Dave's answers to the draft's open questions
are kept word for word at the end.

Dockworkers are the dock's tiles: small live readouts with a card behind each, made by
Vergence, by programs, by scripts, and one day by anyone. Every desktop has tried something
like this. Most attempts died of one of three things: hidden away (macOS Dashboard), running
any code they liked (Windows gadgets), or sprawl (every widget its own little toolkit). What
survived was small, always visible and one click deep (the OS 9 Control Strip, the menu bar
extras, the tray). These principles keep Dockworkers on the side of the survivors.


## What a Dockworker is

1. **Three depths: glance, look, open.**
   - *Tile*: a glance. One value, one label, a state.
   - *Card*: a look. Rows, a chart, a list, a few controls. It opens when you rest on the
     tile, or tap it.
   - *Window*: the whole thing. Opened from the card's title or a button.
   The dock holds only the first two. A full app puts its face in the dock and its depth in a
   window, never more.

2. **Tiles show state, notifications tell of events.** "3 unread" is a tile. "New mail from
   Bob" is a notification. A Dockworker may send both, but the tile never flashes to say what
   a notification already said. Going into trouble is an event too (principle 3).


## How it behaves

3. **Calm until it matters, and trouble stays tile-sized.** A dock you have to read is a
   chore. Tiles are quiet by default: no motion, no color, nothing blinking. Three states
   only: fine, warning, problem. When a tile goes into trouble:
   - **the tile only changes color and shape**: an amber or red mark, never bigger than the
     tile. A tile in the drawer puts the mark on the drawer's tile.
   - **one notification says why**, shown beside the dock near that tile so it is clear
     which tile it belongs to.
   - **the card holds the whole story** whenever you rest on the tile.
   - **when the trouble ends** the mark goes quietly; a problem (not a warning) also sends
     one "back to normal" notice.
   Thresholds belong to the description, with a margin so a value hovering at the line
   doesn't flicker between states.

4. **Quiet when you're busy.** While an app is fullscreen (a film, a game), and whenever you
   turn quiet on (a switch, or a schedule), warnings still change their tile's color but
   hold their notification until quiet ends. Problems always notify: a UPS on battery
   interrupts the film.

5. **Honest data.** A tile never shows an old value as if it were current. Every reading
   has an age. When the source stops answering, the tile turns gray and says so ("no answer
   for 3 min") instead of freezing on its last number. Units are always shown.

6. **Nothing puts itself on the dock.** Vergence's own tiles live on the dock. Everything
   else (yours and others') starts in the drawer (grid or ribbon), and you move it onto the
   dock if you want it there. An installed Dockworker from someone else stays in the drawer
   until you deliberately promote it.

7. **Costs nothing while nobody looks.** A card streams only while it is open; a tile
   updates at the slowest rate that keeps it truthful. Nothing is read for a hidden drawer
   or a closed card. A laptop's battery doesn't pay for a dock.


## How it is made

8. **The server draws everything.** A Dockworker *declares* what it shows; it never draws.
   So every tile is sharp at any scale, looks like the desktop, follows the theme, and
   cannot draw anything misleading (no fake buttons, no lookalike password prompts).

9. **A small vocabulary, and additions must earn their place.** Tile: value, label, state.
   Card: rows, chart, list (rows with a control each), switch, slider, button, link. A new
   piece comes in only when at least three real Dockworkers need it (as the list did: mail
   accounts, mixer streams, lab hosts; next in line, a picture: now-playing art, a camera
   snapshot, a radar image). If a card wants more than this, it is a window.

10. **Anyone can make one, at three levels, and we write the how-to.**
    - A *description file*: a source (a command or an address), how often, thresholds,
      actions. No code. This is the "monitoring and control kit".
    - *vtile* for scripts and cron jobs.
    - The *bus* for real programs.
    All three produce the same tiles; nobody can tell which way one was made. A "Writing a
    Dockworker" guide with worked examples (disk space, ping a host, the UPS) ships with the
    kit; without it nobody else can use it.


## Trust

11. **Every tile shows where it came from.** Three kinds:
    - *Vergence's own*: no mark.
    - *Yours* (a description file you wrote, a script you run): a subtle mark.
    - *Installed from others*: a clearer mark, and the card says where it came from.
    The card always names the maker and the machine ("ups-tile, made by you, on techbench").

12. **Every action has a name and an owner.** A card's buttons are named actions (they also
    show in the launcher). An action that changes something says so on its button; one that
    is destructive asks first; one that needs root goes through the elevation prompt, every
    time. A description file's commands run as you, never as root.

13. **Sources are declared.** A Dockworker says what it reads (a command, a file, an address
    on the network) in its description, and Settings shows it. Nothing reaches the network
    that its description didn't name.


## For everyone, everywhere

14. **Readable by everyone.** Every tile is in the accessibility tree with its name, value
    and state in words. State is never color alone: warning and problem also have a shape,
    for color-blind eyes and for the screen reader.

15. **Yours to arrange.** Drag tiles along the dock, into and out of the drawer, off to
    remove; grid or ribbon for the drawer. Your arrangement is kept per monitor setup, as
    windows are.

16. **The same on every system.** A Dockworker looks and works the same on Linux and
    FreeBSD. Where a system can't provide something, the card says so plainly ("Master
    volume only: this system has no sound server") rather than showing a control that does
    nothing.

17. **Local first.** Tiles come from this machine. A local Dockworker may read another
    machine's data through a source it declares (principle 13): the UPS tile reads NUT over
    the network, a Proxmox tile would read its address. Tiles *published by* other machines
    are set aside; if they ever come, they are opt-in per machine, added by name in Settings.


## What Dockworkers are not

- Not windows: nothing is drawn by the program.
- Not a news feed, ads, or "tips": a tile reports the state of something you chose.
- Not a place for apps to nag: an app that wants attention sends one notification; its tile
  stays calm.
- Not a second desktop to summon: they live where you already look.


## Decided with Dave, 2026-11-25 (his answers to the draft's questions, word for word)

1. *How far does trouble come forward?* Dave: "I think maybe a notification of some kind
   would be better than putting the alert into the dock. When we have tested this with the
   agentic assistance testing the alert was bigger than the dockworker so it didn't look
   great and I couldn't read it. So maybe a system notification with a color alert on the
   dock or a separate notifier just for dockworker right near them?"
   -> Principle 3: color and shape on the tile, one notification beside the dock near it,
   the whole story in the card.

2. *A "quiet hours" switch?* Dave: "Could be useful, like windows does when apps are
   fullscreen."
   -> Principle 4: quiet automatically in fullscreen and on demand; problems always notify.

3. *Other machines' tiles?* Dave: "Not sure on this one. I think the local system only, but
   open to ideas?"
   -> Principle 17: local first; reading other machines' data through declared sources covers
   the lab; tiles published by other machines set aside.

4. *Shareable description files?* Dave: "We almost have to publish a howto of some kind.
   Otherwise how would anyone put it to use? Alternatively we could keep the main
   dockworkers for system use only, and add software and other to the OS 9 inspired ribbon?"
   -> Principle 10 (the how-to ships with the kit). On the dock question Claude recommended,
   and Dave approved ("go"), the middle path over a system-only dock, because his own lab
   tiles (UPS, NAS, Proxmox, made by description file or vtile) would otherwise be barred
   from it: principles 6 and 11 (Vergence's tiles on the dock, everything else starts in the
   drawer and may be promoted, three kinds of origin mark). The ribbon stays a second drawer
   layout, as decided earlier tonight.
