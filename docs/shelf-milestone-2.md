# Shelf milestone 2: the Apps launcher

Drafted 2026-11-13 from docs/next-look-and-launcher.md (agreed with Dave
2026-11-12, "yes to all", from his screenshot of macOS's Apps launcher).
**Accepted 2026-11-13: Dave answered (a) to all four questions at the
end.** It comes after
look milestone 1's blur and backdrop (docs/look-milestone-1.md, steps 3 and
5), since the panel is frosted. (Shelf milestone 1 named live tiles as its
second milestone; they move to a later one.)

## Where things stand (read 2026-11-13, 02638a2)

- **The apps popup** (shelf milestone 1, step 5; the server's launcher.rs,
  vergence-shell's launcher.rs): opened by the dock's apps button or Super
  tapped alone; a search field over a grid of every installed application;
  typing filters, the arrows move, Enter starts, Escape or a click
  elsewhere closes. It opens **beside the dock** (536 x 452 points, cells of
  128 x 84, 32-point icons, a letter tile where an entry has none), drawn
  like the dock by the CPU renderer into one picture.
- **Desktop entries** (vergence-shell desktop.rs `Entry`): id, name, exec,
  icon, TryExec, Terminal, OnlyShowIn and NotShowIn. **Categories are not
  read.**
- **Icons** (vergence-shell icons.rs): themed icons and files, SVG through
  resvg, chosen for the size asked.

## Design

**One thing, not two**: the popup becomes the launcher. The dock's button
and Super open the same panel.

- **Where**: a panel in the middle of the monitor (Question 3), about two
  thirds of its width and height, held between a smallest and largest size
  so it is neither cramped on the 14" panel nor vast on a 4K monitor at
  scale 1. Rounded at the one radius; frosted over the backdrop (look
  milestone 1), the plain tint where there is no blur. The rest of the
  screen is left as it is.
- **The search field** across the top, ready the moment the panel opens:
  typing filters the grid live, by name first, then by the words in the
  entry's GenericName, Keywords and Comment (so "browser" finds Firefox);
  the arrow keys move the selection, Enter starts it, Escape clears what
  was typed and then closes. As AlbumWall's search does.
- **Category tabs** under the field (Question 1): "All" first, then those of
  a short fixed list that have at least one application, from the
  Categories every desktop entry already declares (freedesktop's main
  categories folded into names people use: AudioVideo, Audio and Video
  become "Sound & Video"; Network becomes "Internet"; Utility "Utilities";
  Settings and System "System"; and so on). Nothing is kept by hand. While
  something is typed, the search covers every category and the tabs show
  how many each has. Ctrl+Page Up and Ctrl+Page Down move between tabs
  (the arrows stay with the grid); a click or a finger chooses one.
- **The grid**: icons of 64 points with the name under them (two lines at
  most, then an ellipsis), sorted by name (Question 2); a letter tile in our
  look where there is no icon. It scrolls (wheel, touchpad, finger with the
  terminal's flick momentum) rather than turning pages.
- **Right-click on an application**: Pin to the dock or Unpin, and Open in a
  terminal for Terminal=true entries; the dock's own menu, reused.
- **Light and dark**: the panel, field, tabs and selection follow Style,
  with the bars' colors (look milestone 1), contrast checked by the same
  test.
- **Fast**: the panel appears in the frame after the key or click (icons
  are decoded once, at the grid's size, and kept; the entry list is read at
  start and when an applications directory changes, as now).

Drawn as now (a scene, the CPU renderer, one picture redrawn when what it
shows changes), the frost under it by the server.

## Progress

- **Step 1 done** (2026-11-13): vergence-shell reads GenericName, Comment
  and Categories (lists cut at `MAX_LIST`, 32); `launcher::TABS` and
  `tab_of` fold the main categories into the nine tabs (Network to
  Internet; AudioVideo, Audio and Video to Sound & Video; Education and
  Science to Education; Utility to Utilities; Settings and System to
  System); `search_in` narrows by a tab while nothing is typed and ranks
  finds by a name's word, the name, GenericName ("browser" finds Firefox),
  a keyword, then the comment; `tabs` lists the tabs that have an
  application with the count of finds in each. Tests for each.
- **Steps 2 and 3 done** (2026-11-13): the panel, frosted. It opens in
  the middle of the screen the pointer is on, two thirds of it (560 x 420
  to 1040 x 760 points), frosted over what is behind (look milestone 1's
  backdrop; the plain ground without the frost). Under the search field a
  row of tabs, All and those that have an application (`Open::tab_rects`;
  the chosen one shaded; while something is typed each shows how many of
  the finds it holds); a click or Page Up / Page Down (with Ctrl or
  without) chooses one. The grid: 64-point icons, names under them on two
  lines at most, the second ending in an ellipsis. Escape clears what was
  typed, then closes. `vctl key` knows Page_Up and Page_Down. Seen on
  kepler (FreeBSD, 4K at 2.5): centered at 256,144 1024 x 576, frosted
  over the wallpaper and a terminal; "browser" finds Firefox; Page Down to
  Internet then System, Page Up back to All; a click on System. 400 tests
  on all three.
- **Step 4 done** (2026-11-13): a right press on an application opens the
  dock's own menu beside it (Open; Pin to the dock or Unpin from the dock),
  frosted over the launcher; Open closes the launcher, pinning leaves it
  open. With a menu over the launcher the seat now takes the menu as the one
  open (a press on it had counted as one outside the launcher and closed
  it, found on kepler). Fingers: a finger's press only marks where it came
  down; lifted there it is a click (a tab chosen, an application started),
  moved more than 6 points it drags the grid (no momentum yet). Seen on
  kepler by vctl: pin and unpin through the menu (the dock shows Foot, then
  not), a tap opening the launcher from the dock, a tap on System, a drag
  over an application starting nothing. 400 tests on all three.
- **Step 5 done** (2026-11-13): whitepaper §6.9's apps button describes
  the launcher (Draft 0.11); shelf milestone 1's live tiles point to their
  later place. **Acceptance**: 1 met (400 tests on all three); 2 met on
  kepler (Super's tap not tried from vctl; the dock's button opens it in the
  middle of the pointer's screen with the field ready; "browser" finds
  Firefox; typing "term" and Enter not run by hand); 3 met (tabs only with
  applications; Page Down, a click and a finger change them); 4 met for the
  frost (seen on kepler) and Reduce transparency by the shared code, the
  contrast of light and dark by look milestone 1's tests; 5 seen at kepler's
  scale 2.5 (sharp, in proportion); Hubble's monitor at scale 1 is Dave's to
  look at.

## Steps



1. **Categories and words**: vergence-shell reads Categories, GenericName,
   Keywords and Comment; the fold into the tab list; search by name, then
   by those words; tests.
2. **The panel**: centered, sized to the monitor, the field, the tabs, the
   larger grid with names, scrolling, the keys; solid in light and dark
   until the backdrop is in.
3. **Frosted**: the panel over look milestone 1's backdrop; the plain tint
   road.
4. **The right-click menu** and finger use (tap to start, flick to scroll,
   tap a tab).
5. **Docs**: whitepaper §6.9, this plan's notes, shelf milestone 1's
   pointer to live tiles.

## Acceptance

1. Tests green on Hubble, kepler and Techbench.
2. Super or the dock's button opens the panel in the middle of the monitor
   with the pointer, in the next frame, with the field ready; typing "term"
   and Enter starts the terminal; "browser" finds Firefox.
3. The tabs show only categories that have something; Ctrl+Page Down,
   a click and a finger each change them.
4. Frosted in a session with a GPU, the plain tint with Reduce transparency
   or on the software driver; light and dark both pass the contrast test.
5. On kepler's 4K panel at scale 2 and Hubble's monitor at scale 1, the
   panel's size and the icons are sharp and in proportion.

## Security (whitepaper §7.3)

- Desktop entries are untrusted input, as now: the new keys are length
  limited, and a Categories list longer than its limit is cut.
- Starting an application is unchanged: the entry's program and arguments
  with no shell between.

## Not in this milestone

- Settings pages in the search ("display" opens Screens): next, once apps
  are in.
- Live tiles (shelf milestone 1's "milestone 2"): later.
- The most used applications first: see Question 2.

## Ideas noted for later (Dave, 2026-11-13: "note it as an idea and move on")

- **The dock anchored in a corner**, one end at the corner with the apps
  button in it (the easiest target on a screen); status tiles at the end
  nearest the corner. Claude's view: the cheap, real ergonomic gain.
- **A second arm** from that corner along the other edge (apps down the
  side, a short arm of tiles along the top, or as the user drags them):
  windows keep clear of both, cards open toward the middle. Costs a second
  edge (vertical room on wide screens), longer trips on big desks, two
  places to learn and more settings; an option, not the default, and not
  built until someone misses it.
- **Tiles from other programs**: any program publishes a tile's value and
  actions on the session bus (whitepaper §6.9); the desktop provides the
  everyday ones. A small command-line publisher so a shell script (a rack
  or UPS check) can publish one. A program's new tile lands in the drawer
  until the user pins it, is labeled with its program, and is rate and
  size limited; stepping out onto the dock when in trouble can be turned
  off per tile.

## Questions for Dave (answered 2026-11-13: (a) to all four)

1. **Which tabs.** (a) "All" and a short fixed list (Internet, Office,
   Graphics, Sound & Video, Development, Games, Education, Utilities,
   System), each shown only when it has an application; (b) every category
   the installed entries name. Proposed: **(a)**: (b) gives a dozen tabs
   with one application each on most machines.
2. **The grid's order.** (a) By name; (b) the most used first (a count kept
   per application in the profile), then by name. Proposed: **(a)** now,
   with (b) possible later as a row of its own over "All".
3. **Which monitor.** (a) The one with the pointer; (b) the one with the
   dock. Proposed: **(a)**: it opens where you are looking.
4. **Super.** (a) Super tapped alone opens and closes it, as now; (b) also
   Super+Space. Proposed: **(a)**, with (b) only if a program you use takes
   Super for itself.

## After the milestone

- **Vergence's own apps at the top** (2026-11-15, Dave: "a section at the
  top that holds the native apps", "do it first"): while nothing is typed,
  a compact row headed "Vergence" sits under the search field with our own
  applications (desktop entries `vergence-*`; File Viewer, Text Editor,
  Terminal, Settings, Close-Up, Calculator, Hardware, Force Quit, then any
  other of ours by name; vergence-shell `launcher::own`). They stay in the
  grid and in search: the row is a shortcut. A click starts one, a right
  click gives the dock's menu, the pointer shades it; typing hides the row.

