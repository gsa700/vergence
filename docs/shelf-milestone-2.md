# Shelf milestone 2: the Apps launcher

Drafted 2026-11-13 from docs/next-look-and-launcher.md (agreed with Dave
2026-11-12, "yes to all", from his screenshot of macOS's Apps launcher).
**Not yet accepted**: the questions at the end are Dave's. It comes after
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

## Questions for Dave

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
