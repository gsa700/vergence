# Files milestone 3: FileView, with the best of Explorer and Finder

Status: **draft 2026-11-21**, for Dave's approval. An app milestone (after
screenshots, which closed 2026-11-21). Dave compared FileView with Windows
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
2. **One toolbar row under the tab bar**, as Explorer's: Back, Forward, Up;
   the path (the breadcrumb, which becomes a field to type a path when
   clicked, as Explorer's address bar); the views (list, icons) and the
   preview pane; New Folder, Trash; the filter field at the right. The
   status line stays at the bottom: the item count, the selection, and
   the free space of the folder's disk (one `statvfs` call per folder:
   cheap, so in).
3. **The sidebar**, from both: full height under the toolbar, in sections
   as Finder's and Explorer's (**Favorites**: Home, Desktop, Documents,
   Downloads, Pictures and the user's own; **Locations**: drives, network
   mounts, Trash; **Recent**). Reordered by dragging; a folder or file
   dragged from the main view drops in as a shortcut (a file's opens it, a
   folder's goes there); a shortcut's right-click: Open in New Tab, Remove
   from Sidebar, Show in its folder. Kept in ~/.config/vergence's FileView
   settings. A section folds by its heading.
4. **The list**, Finder as the model: grouped by kind by default (Folders,
   Documents, Pictures, Music, Videos, Archives, Developer, Other; Group by
   in the View menu: Kind, Date, Size, None); stripes under the files only;
   rows a little taller (Medium by default, about Finder's 24 points, the
   icons a little larger).
5. **Rename in place with a single click** (Dave's list, 2026-11-21): a
   click on the name of an item already selected, a pause, and the name is
   a field (Enter keeps it, Escape lets go, a click elsewhere keeps it); a
   double click still opens; F2 renames too (Explorer's key).
6. **Opens in the home folder** (not Computer); New Window and New Tab too.

## Steps

1. Tabs (and keeping each window's tabs across restarts).
2. The toolbar row and the path field; free space in the status line.
3. The sidebar: sections, dragging to reorder, shortcuts by drop, their
   menu, folding.
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
Close-Up), the mouse's Back button (also on the wishlist; small, could join
step 2 if Dave likes).

## Questions for Dave

1. **The mouse's Back and Forward buttons** (wishlist, 2026-11-16: "go back
   in history with the mouse button 5 as web browsers do"), and history
   remembering each folder's scroll position for the session: (a) in step 2
   (recommended: small, and it is how people move back with the new tabs);
   (b) later.
2. **The sidebar's Recent**: (a) recent folders and files (Finder's Recents,
   Explorer's Home), from what FileView, Edit, Reader and Close-Up open
   (recommended); (b) leave it out.
