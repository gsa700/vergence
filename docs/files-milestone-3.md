# Files milestone 3: FileView, with the best of Explorer and Finder

Status: **closed 2026-11-21** (Dave: "close it"), seven steps in (disk images the seventh). Earlier: **questions answered 2026-11-21** (Dave: 1 yes; 2 "recents should
probably exist, but I don't care for it and always turn it off. so lets
have the option to do that"). An app milestone (after
screenshots, which closed 2026-11-21), now after session milestone 1
(docs/session-milestone-1.md, 2026-11-21): its rules decide what FileView
keeps. After a restart, its tabs come back with their folders and scroll
places; a folder closed by the user opens at the top. Dave compared FileView with Windows
11's File Explorer and macOS's Finder (his screenshots on Control's
Desktop, 2026-11-21) and answered the same night:

- "I like the explorer left panel and the mac one as well. what I would
  take away from them is drag and drop reordering, shortcuts from folders
  and files dropped there from the main window, etc."
- "I know I said we didn't need tabs in the file manager, but I think we
  should bring them back."
- "the unified toolbar is a good idea, but lets make it like the windows
  version. unified on a toolbar under the tab/title bar."
- "Group by: if you mean the left column, yes. drive space bars, no, open
  in home directory. free space, not unless it's cheap. stripes under
  files only, yes, and the separation by type by default separated as on
  the mac. slightly taller rows, yes."

Two of these change decisions of files milestone 2 (2026-11-16): **tabs
come back** (decision 1 there removed them), and the stripes **stop at the
last file** (decision 4 carried them on into the room below); grouping by
kind, an option there, is **on by default** here.

## What changes

1. **Tabs.** FileView's windows get the server's tabs again, as Edit's and
   Reader's: each tab its own folder, view, selection and Back/Forward;
   New Tab (Ctrl+T, and the bar's +), Close Tab (Ctrl+W; the window with
   its last), tabs dragged out to a window of their own and between
   windows; a folder handed over (from the desktop, Open with, the dock)
   opens in a tab of the front window. Folders kept across restarts per
   window and tab, as Reader keeps its documents.
2. **The mouse's Back and Forward buttons** move through the tab's history,
   as in a browser, and each folder in it comes back at the scroll place it
   was left at (for the session).
3. **One toolbar row under the tab bar**, as Explorer's: Back, Forward, Up;
   the path (the breadcrumb, which becomes a field to type a path when
   clicked, as Explorer's address bar); the views (list, icons) and the
   preview pane; New Folder, Trash; the filter field at the right. The
   status line stays at the bottom: the item count, the selection, and
   the free space of the folder's disk (one `statvfs` call per folder:
   cheap, so in).
4. **The sidebar**, from both: full height under the toolbar, in sections
   as Finder's and Explorer's (**Favorites**: Home, Desktop, Documents,
   Downloads, Pictures and the user's own; **Locations**: drives, network
   mounts, Trash; **Recent**: recent folders and files from FileView, Edit,
   Reader and Close-Up, with a switch to turn it off in FileView's settings
   and on the section's own right-click, Hide Recent; Dave turns it off). Reordered by dragging; a folder or file
   dragged from the main view drops in as a shortcut (a file's opens it, a
   folder's goes there); a shortcut's right-click: Open in New Tab, Remove
   from Sidebar, Show in its folder. Kept in ~/.config/vergence's FileView
   settings. A section folds by its heading.
5. **The list**, Finder as the model: grouped by kind by default (Folders,
   Documents, Pictures, Music, Videos, Archives, Developer, Other; Group by
   in the View menu: Kind, Date, Size, None); stripes under the files only;
   rows a little taller (Medium by default, about Finder's 24 points, the
   icons a little larger).
6. **Rename in place with a single click** (Dave's list, 2026-11-21): a
   click on the name of an item already selected, a pause, and the name is
   a field (Enter keeps it, Escape lets go, a click elsewhere keeps it); a
   double click still opens; F2 renames too (Explorer's key).
7. **Opens in the home folder** (not Computer); New Window and New Tab too.

## Steps

1. Tabs (and keeping each window's tabs across restarts).
2. The toolbar row and the path field; free space in the status line; the
   mouse's Back and Forward, scroll places kept.
3. The sidebar: sections, dragging to reorder, shortcuts by drop, their
   menu, folding; Recent and its switch.
4. The list: grouping by kind by default, stripes under files only, taller
   rows.
5. Single-click rename, and F2.
6. Checks: a display-suite scenario (a window with two tabs, the toolbar
   row, the sidebar's sections, grouped rows), agent-check through
   FileView, tests on all four; Dave's hands-on pass.

## Not in it

Column view and gallery (Finder), Explorer's command bar (its commands stay
in the right-click and app menus), drive space bars (Dave: no), a separate
preview window (on the wishlist since 2026-11-16: a later milestone with
Close-Up).

## Questions for Dave (answered 2026-11-21)

1. *Answered: yes, in step 2.* The mouse's Back and Forward buttons, and
   history keeping each folder's scroll place for the session.
2. *Answered: Recent exists, with a switch to turn it off* ("I don't care
   for it and always turn it off").

## Progress

- **Step 1, tabs: in 2026-11-21.** The server's tabs again (Reader's way):
  New Tab (Ctrl+T and +, at home), Close Tab (Ctrl+W, the window with its
  last), next and previous (Ctrl+Page Down/Up), torn off into a window of
  its own and joined to another, each with its folder, history and choice
  (a tab moved takes the new window's folder sizes). A folder handed over
  (the desktop, Open with, a start with folders) opens in a tab of the
  window last pressed or typed in; a start with nothing opens a window at
  home. New Window opens at home too. Each window's tabs kept across
  restarts by session milestone 1's rules (crates/vergence-files/src/keep.rs,
  ~/.local/state/vergence/files-session: kept when ended from outside or by
  an exit, forgotten when the user closes the window, `[session] restore =
  false` forgets them). The Trash window keeps its plain bar and keeps
  nothing. Checked in a headless session on Hubble: two folders on the
  command line make two tabs; a folder handed over by a second start joins
  that window as a third; killed and started again, the three come back
  with the same one in front; Ctrl+W and Ctrl+T change what is kept.
  Tests on Hubble and techbench; agent-check 0 failed. The folders' scroll
  places come with step 2.
- **Step 2, the toolbar row: in 2026-11-21.** Explorer's order: Back,
  Forward, Up; the path (a click on the room after the folders, or Ctrl+L,
  makes it a field: Enter goes to a folder, or a file's folder with the file
  chosen, "~" is home; Escape lets go; a path not there is said); the views
  and the panes; New Folder and Trash; the filter at the right. The status
  line: the count and the choice at the left, the disk's free space at the
  right (`df -kP`, read apart for each folder the first time, as
  vergence-sysinfo reads file systems on both systems). The mouse's Back and
  Forward (BTN_SIDE, BTN_EXTRA) move through the tab's history. Each folder
  of a tab's history comes back at its scroll place (for the session), and
  each tab's scroll place is kept across restarts ("at" lines in
  files-session, written at most every 2 s; `[session] places = false`
  starts at the top). The folder's scroll is the toolkit's named scroll
  "Folder" (session milestone 1's Program::scroll_to/scrolled). Found on
  the way: the path field took no keyboard, the path row having stood in
  its place (the toolkit took them for one widget); a key of its own fixed
  it. Checked in a headless session on Hubble: scrolled to the end of
  crates, a path typed to docs, Back: crates at the same place (its first
  row at -496), Forward and Back the same, killed and started again the
  same. Tests on Hubble and techbench (the free space read from FreeBSD's
  df too); agent-check 0 failed.
- **Step 3, the sidebar: in 2026-11-21** (crates/vergence-files/src/sidebar.rs).
  FileView's own sidebar (the Open dialog keeps the shared one): Favorites
  (Home and the user's folders at first; desktop.toml `[files] favorites`, a
  list of paths, once changed), Locations (the computer, disks, the Trash,
  the network's places), Recent (the six files opened most lately, which
  FileView now adds to as the other apps do, and All recent). A heading
  folds its section (`[files] sidebar-folded`). A favorite pulled away moves
  within the sidebar (a drag kind of its own, application/x-vergence-sidebar:
  dropped on a folder it does nothing); folders and files dragged from the
  main view onto the favorites become shortcuts where dropped (accepted as a
  copy, so nothing on the disk moves). Right click: Open in New Tab, Show in
  Its Folder, Remove from Sidebar; on the Recent heading, Hide Recent;
  FileView's settings have "Recent in the sidebar". Rows are of fixed heights
  and the sidebar remembers where it was drawn and how far it is scrolled,
  so a drop's place is worked out from where it is. A favorite that is not
  Home or a user folder shows its kind's mark (as a place it was taken for a
  disk). Found on the way, in the toolkit: drops came in without the strip
  under the server's bar (the pointer's had it), so they landed a row too
  high in any window drawn under its bar; vergence-ui's native road now adds
  it (only FileView and the desktop, which has no bar, use where a drop is).
  Checked in a headless session on Hubble with real drags (vctl pointer,
  press, release): a folder from the list dropped between Home and Desktop
  lands there; Pictures dragged onto the Favorites heading goes first;
  Locations folds. Not checked by hand: the right-click menus' lines (the
  headless session's menus are the server's). Tests on Hubble and techbench;
  agent-check 0 failed.
- **Step 4, the list: in 2026-11-21.** Group by Kind (the default: the
  list under Folders, Documents, Text, Pictures...), Date (Finder's: Today,
  Yesterday, Previous 7 Days, Previous 30 Days, Earlier This Year, then each
  year), Size (Folders, then the files biggest first in steps of ten) or
  None: vergence-widgets files.rs `GroupBy`, `group_key`; FileView's View
  menu, the folder's right-click Group By, and its settings (`[files]
  group-by`; the older `group-by-kind = false` still means none). The
  stripes stop at the last row. Rows: Medium was already 30 points (the
  plan's "about Finder's 24" was written against an older size), so the
  rows stay and the marks grow a little (Medium 1.3, Large 1.6). The Open
  dialog keeps its plain list. Checked in a headless session on Hubble: the
  repository's folder grouped by kind, then by date. Tests on Hubble and
  techbench; agent-check 0 failed.
- **Step 5, rename in place: in 2026-11-21.** The one chosen item clicked
  again with no keys held, then the double-click time passing with no
  second click: its name becomes a field in its row (list and icons;
  vergence-widgets files.rs `View::rename`, the field keyed so it takes the
  keyboard where the name stood), its text chosen to be typed over. Enter
  keeps it, Escape lets it go, anything else done (a click on another row,
  a command, a tab) keeps it; a click on its own row leaves it be. F2 and
  the menu's Rename do the same. A double click still opens. The
  double-click time is the desktop's (Settings > Mouse), as the toolkit's
  widgets use (FileView had its own 450 ms). New Folder and New File keep
  the naming strip (there is no row yet). The click counts anywhere on the
  chosen row, not only on its name (the list does not know where the name's
  words end). Checked in a headless session on Hubble (a click, a pause, a
  click, a pause: the field, Enter renamed it; F2 then Escape: nothing; F2,
  typed, a click on another row: renamed), and by a test for the double
  click (vctl's clicks come too slowly to make one). Tests on Hubble and
  techbench; agent-check 0 failed.
- **Step 6, checks: in 2026-11-21.** Display-suite scenario 16
  (tools/display-suite/16-fileview.sh): two folders as two tabs of one
  window, the toolbar row, the sidebar's sections, the list grouped by kind,
  the free space, a folder handed over as a third tab and kept for a
  restart, F2's field in its row. The full suite 16 of 16 on Hubble and
  techbench; agent-check 0 failed. With it, Dave's ask: the sidebar's names
  at 13 points (12 before), headings 11, marks 18, rows 28.
- **Step 7, disk images: in 2026-11-21** (Dave: "in downloads I have an iso,
  can i mount it in Fileview as we do the USB drive?"; added before the
  milestone closes). An ISO opened (double click, Enter) or Mount Disk Image
  on its right click is mounted read-only on a worker; it
  shows under Locations (no tab of its own since 2026-11-21, Dave: "A new tab
  should never appear unless the user calls it"), with Eject (the File menu's, and now the right
  click of a disk under Locations). vergence-media `mount_image`:
  - FreeBSD: the root helper's `attach` (media-eject, setuid root, its
    group only): an ISO 9660 image the caller owns, checked as opened
    (owner, a regular file, "CD001" at byte 32769), attached with `mdconfig
    -o readonly`, the path checked to lead to the same file after, mounted
    with mount_cd9660 `ro,nosuid,noexec` at /mnt/<name> (/media is the
    automounter's). Eject of /mnt/<name>: only an md device there; umount,
    mdconfig -d, the folder taken away. Left: a swap and a swap back of the
    file between the checks and mdconfig could attach another ISO 9660 image
    the user may not read; /dev/mdN stays root:operator 0640.
  - Linux: `udisksctl loop-setup --read-only` (udisks2's own tool: udisks
    takes the file as a descriptor, which our D-Bus client cannot pass yet),
    then udisks's Mount (noexec) through our client; Eject deletes the loop
    device. The polkit rule (tools/linux/50-vergence-drives.rules) allows
    loop-setup for the seat's group.
  Checked: on techbench the helper attached and mounted a test ISO (made
  with makefs -t cd9660), its file readable, then ejected it, the md device
  gone; it refused a root-owned ISO, /etc/passwd and a link to the
  root-owned ISO. On Hubble a test against udisks mounted the ISO at
  /run/media/<user>/VTEST and ejected it, and FileView in a headless
  session mounted it from Enter into a tab with VTEST under Locations.
- **Dave's notes after step 7, 2026-11-21:** the sidebar and the folder on
  one page as Explorer's (the sidebar's gray card gone), a line between
  them pulled to change the sidebar's width (`[files] sidebar-width`, 140
  to 400; taken in window points, since the line moves as it is pulled: a
  pull of 80 moved it 80); Medium rows 34 and Large 42 with marks 1.5 and
  1.8 and names 13 and 14, as Finder's beside his screenshot; an eject
  button at the end of a drive's or a disk image's row under Locations
  (toolbar `Icon::Eject`), besides its right click's Eject. Checked in a
  headless session on Hubble: the divider pulled; an ISO mounted, the
  button shown, pressed: unmounted and the tab gone home.
- **The desktop and FileView, 2026-11-21:** an ejected stick still plugged
  in (FreeBSD does not power it off) is left out of the drive lists while
  its device node is the one ejected (~/.local/state/vergence/ejected), and
  a disk image put away says nothing (no "Safe to remove"). The desktop's
  icons disagreed with FileView: a process of their own on the program of
  the session's start, and each reading the mounts on its own. Dave chose
  the server owning the drive list (a milestone of its own, after this one)
  and the desktop restarted on install: the server now keeps the desktop
  running (started again when it exits, five times a minute at most, not
  while the session exits), and tools/install.sh stops it when the running
  server's log says it does so. Checked on kepler: killed, the desktop came
  back; installed again, the new program came up.

