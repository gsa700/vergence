# Files milestone 3: FileView, with the best of Explorer and Finder

Status: **questions answered 2026-11-21** (Dave: 1 yes; 2 "recents should
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

