# Files milestone 2: File Viewer refined

*Planned 2026-11-16 evening, after File Viewer's right-click menu (backup
milestone 1, step 1) went in and Dave used it on all three machines. His
words: "we may be far enough along to do a little refinement on File
Viewer", with Finder's list view as the picture of what he likes.*

## Decisions (Dave, 2026-11-16)

1. **No tabs** in File Viewer or Close-Up: one folder (or picture) per
   window, a plain title bar. New Window stays; New Tab, Close Tab and the
   tab keys go.
2. **The right-click menu opens under the pointer** (fixed the same
   evening: the toolkit's scene points start below the strip under the bar,
   and requests carrying a point now take that strip off again; the
   desktop's older ContextMenu had the same fault, hidden because the
   desktop has no bar).
3. **Get Info is its own window, Windows-style Properties with tabs**:
   General, Permissions, and later Earlier Versions (backup milestone 1,
   step 4: the Windows "Previous Versions" tab is its place). One window
   per item.
4. **Details view** (the list), Finder as the model:
   - striped rows, the bands carried on into the empty space below;
   - a **Kind** column, and sorting by it as by Name, Size and Changed;
   - **column widths** changed by dragging the edges between headings,
     remembered;
   - a **row size** setting, Small / Medium / Large (Medium by default),
     in the View menu and the app's settings; text size stays the app's
     zoom;
   - **folder sizes** counted in the background and kept, shown as "—"
     until known; not on network mounts or in the Trash;
   - **Group by Kind** as a View option, off by default (Folders,
     Documents, Pictures, Other...).

## Steps

1. Tabs out of File Viewer and Close-Up. **Done**: a plain title bar
   (`Bar::Plain`); Close window (Ctrl+W) replaces Close tab; folders handed
   over or given open in this window if it is empty, else in windows of
   their own (`pending`, opened by woken()); Close-Up the same, except a
   file dropped on a window takes its place.
2. Details view. **Done** (vergence-widgets files.rs, the Open dialog too in
   a small, kindless form): `ListLook` (row size, widths, Kind column,
   stripes, grouping), `COLUMNS` Date Changed / Size / Kind with grips
   between the headings (`View::grip`, a pull changes the column to its
   right; kept as `[files] columns = "140,80,150"`), `RowSize` (`[files]
   row-size`, View menu and settings), striped rows carried on through the
   room below by a filler drawing, `SortBy::Kind`, `group_of` headings
   (`[files] group-by-kind`, the ground's menu, View menu, settings).
   Folder sizes: one worker per window (`size_worker`, `count_tree`: one
   file system, links not followed, at most 400,000 entries), kept by path
   and the folder's time, not in the Trash, /proc, /sys, /dev, /run or a
   network mount; shown "—" until counted.
3. The Properties window. **Done** (props.rs): General (mark, the name
   changed in place with undo, kind, where, size and items counted on a
   worker, created, changed, opened, a link's target, Opens with that sets
   the type's default through `vergence_types::choose_default`) and
   Permissions (owner, group, read and write for each, a folder that can be
   read can be entered, "Allow running as a program"; only the owner or
   root changes them). The menus call it Properties (Ctrl+I); one window per
   chosen item, at most eight. No spelling marks in File Viewer's fields.
4. Tests green on Hubble, kepler, Techbench 2026-11-16 evening; installed.
   Not yet: a ↔ pointer over the column grips; folder sizes in the Size
   sort after a folder is read again (the sizes are put back only when new
   ones arrive).
