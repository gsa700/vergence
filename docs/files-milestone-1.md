# Files milestone 1: file types, the file chooser for every app, Close-Up, File Viewer

**Drafted 2026-11-15**, item 5 of docs/apps-roadmap.md. **Draft: the
questions at the end are for Dave.**

**Accepted 2026-11-15** (Dave). **Answers:** 1 a (the order as planned); 2 a (PDF
and video not now, their own milestone with the borrowing question then);
4 b (our own file type table, not the system's database: written by us,
since shared-mime-info's data is GPL and the workspace is Apache-2.0 OR
GPL-2.0-or-later); 7 to revisit (only the system's mounts until then).
3 Claude's lean (GIF
and WebP by the `image` crate's own features, BMP written by us, HEIC moved
to the video milestone: libheif is LGPL and HEVC carries patents); 5
Claude's lean (`~/.config/vergence-mimeapps.list`, the XDG spec's
per-desktop file, read first in a Vergence session, falling back to the
shared `mimeapps.list`; our choices never written into the shared one); 6
Claude's lean (the session sets the portal variables for the apps it
starts, behind a switch on the Apps page, "Use Vergence's file dialog in
other apps", on by default once Firefox, a GTK app and a Qt app pass). 8: the file manager is **File Viewer** (after
NeXTSTEP Workspace Manager's File Viewer window), its quick look is
**Preview** (the space bar or a preview pane), and the full app is
**Close-Up**, the step after Preview (`vergence-closeup`; the launcher finds
it as "closeup" and "close up" too).

Dave's words: File manager "with space bar preview ala macOS finder ...
remote file system support as well (using native OS tools?)". Viewer:
"picture, video, and pdf viewer. Connected to file manager by file type
association, but a standalone app". The portal: "yes add the portal to
item 5" (2026-11-15).

## Where things stand

- **The Open/Save dialog is built** (editor milestone 1, vergence-widgets
  filedialog.rs, 1,040 lines): places (Home and its folders, the computer,
  mounted disks, Recent), a sortable file list, typed paths, a look at a
  text file before opening, each app's last folder remembered. The File
  Manager needs its places and its list, so those parts move into widgets
  both can use.
- **File types: nothing yet.** The dialog filters by name patterns only.
  The freedesktop MIME database (shared-mime-info's data: `globs2`,
  `magic`, `subclasses`, `aliases`, `generic-icons`) is installed on all
  three machines (/usr/share/mime on Void and Fedora, /usr/local/share/mime
  on FreeBSD). Each has a ~/.config/mimeapps.list written by other desktops
  or by browsers.
- **Desktop entries** are read by vergence-shell apps.rs (the dock and the
  launcher): name, Exec, icon, TryExec. Their `MimeType=` lines are not
  read yet.
- **The D-Bus edge** (server dbus.rs, our own vergence-dbus) owns
  `org.freedesktop.portal.Desktop` and answers its Settings interface.
  Every other portal interface answers UnknownMethod, so apps fall back to
  their own dialogs.
- **Pictures**: the server and the settings app decode PNG and JPEG with
  the `image` crate (borrowed, listed in docs/borrowed-review.md), and SVG
  with resvg. Both read untrusted files, which whitepaper §6.10 says belong
  off the window's thread, and in a process that can do nothing else where
  the platform allows.
- **Native windows have a clipboard (bus window protocol
  `ClipboardOffer`/`ClipboardSend`) but no drag and drop.** Dragging files
  out of File Viewer needs it.

## Design

### 1. File types (`crates/vergence-types`, our own)

- **What a file is**: its name against `globs2` (weights, case, longest
  match wins), then, when the name says nothing or two types tie, its first
  bytes against `magic`. Plus `subclasses` (a shell script is also plain
  text), `aliases`, and each type's description and icon name. Read from
  the system's database. If none is installed, a small table built into the
  crate covers the common types (text, pictures, PDF, archives, audio,
  video) so nothing breaks (question 4).
- **Which apps open it**: each desktop entry's `MimeType=` list, a
  subclass's apps after the type's own, and the user's choices in
  `mimeapps.list` (XDG spec: `[Default Applications]`, `[Added
  Associations]`, `[Removed Associations]`; ~/.config first, then the
  system's). Vergence's own choices go in the spec's per-desktop file,
  `~/.config/vergence-mimeapps.list`, read before the shared one in a
  Vergence session (`XDG_CURRENT_DESKTOP=Vergence`), so GNOME on Techbench
  keeps its own and a browser rewriting the shared file cannot undo a
  choice made here (answer 5). Whether each platform's xdg-open reads
  per-desktop files gets checked; where it does not, `vergence-open` is
  what our session hands links and files to.
- **Opening**: `vergence-types` gives the command line (desktop entry Exec
  with `%f %F %u %U` filled in), and the server starts it as the dock
  does, so the app gets the session's environment and a place in the dock.
  A `vergence-open <file|url>` command does the same from a terminal or a
  script.
- **In the Open/Save dialog**: filters by type ("Pictures" means every
  picture type, not a list of patterns), and each file's icon by its type.
- **Settings > Default apps** (new page, in the Desktop group): the usual
  kinds (web browser, mail, text, pictures, PDF, music, video, the file
  manager, the terminal) with a menu of the apps that can open each, and
  an Advanced part listing every type that has a choice.

### 2. The file chooser for every app (the portal's FileChooser)

The edge answers `org.freedesktop.portal.FileChooser` (`OpenFile`,
`SaveFile`, `SaveFiles`): options for title, accept label, multiple,
directory, filters, current filter, current name, current folder, and
choices (extra check boxes or menus, shown under the list).

- **The dialog is ours**, from a small native helper (`vergence-choose`,
  installed in libexec) that the server starts for each request. It shows
  vergence-widgets' FileDialog and prints what was chosen. The server sends
  the portal's `Response` (0 with `uris`, 1 cancelled, 2 failed) on the
  request's `Request` object, as the portal spec says.
- **Over its app's window**: the portal passes a parent handle
  (`x11:<xid>` or `wayland:<handle>`). The Wayland handle comes from the
  xdg-foreign protocol, which the server does not offer, so the dialog is
  placed over the caller's window by the caller's process instead: the
  window of that pid with the keyboard, else its newest window. It is
  modal to it (dialogs.rs: the dim, the flash). A window of another client
  as a dialog's parent is new for the server; native dialogs today are
  only the same program's.
- **Paths, not copies**: our apps are not sandboxed, so the answer is the
  file's own `file://` URI. The Documents portal (FUSE copies for
  sandboxed apps) is not needed and not built; a Flatpak app is the case
  that would need it.
- **Getting apps to ask**: Firefox asks the portal when
  `widget.use-xdg-desktop-portal.file-picker` is 1; GTK 3 when
  `GTK_USE_PORTAL=1`; GTK 4 by `GDK_DEBUG=portals` or its own rules; Qt
  through its `xdgdesktopportal` platform theme. The session can set the
  environment variables for every app it starts, behind a switch on the
  Apps page (answer 6), off while this is built and on by default once
  Firefox, a GTK app and a Qt app pass the acceptance. Firefox's preference
  lives in the user's profile, which we do not touch: the install notes
  give the line. Each app's actual behavior gets checked here, not assumed
  from documentation.
- **Also on the edge** (small, same pattern): `org.freedesktop.portal.OpenURI`
  (`OpenURI`, `OpenFile`, `OpenDirectory`) through section 1, so "open
  file" and "show in folder" in Firefox's downloads reach our apps.

### 3. Decoding, kept apart (`vergence-decode`)

Pictures from anywhere are untrusted input (§6.10, §7.3). Close-Up, the
File Viewer's previews and thumbnails, and the file chooser all decode
through one helper process: it reads one file, decodes it, and returns the
pixels over a pipe as shared memory. It runs with nothing else: Capsicum
`cap_enter` on FreeBSD after opening the file; on Linux, closed descriptors
and no network (Landlock and seccomp later, tracked in §8.3's list). A
crash or a hang (5 s) in a bad file costs a "Could not be shown" in the
app, never the app. Formats (answer 3): PNG, JPEG, GIF (animated too) and
WebP by the `image` crate's own decoders (pure Rust, already borrowed; GIF
and WebP are features turned on, listed in docs/borrowed-review.md), SVG by
resvg, BMP by our own small decoder. HEIC (the iPhone's photos) waits for
the video milestone.

### 4. Close-Up (`vergence-closeup`)

A standalone app, one process for all its windows (tabs in the server's
bar, as the terminal and the editor do).

- **Pictures**: fit to the window, 100%, zoom by keys, pinch and scroll,
  pan by dragging, rotate the view, full screen, the picture's size and
  type and, for JPEG, when it was taken (EXIF read by us, a small parser).
  Next and previous file in the folder (Left/Right, sorted as the File
  Manager sorts). Transparency on a checkerboard.
- **Text**: shown read-only in the toolkit's text area, with "Open in Text
  Editor".
- **PDF and video**: by question 2. Without them, Close-Up says it cannot
  show this type and offers the app that can (section 1).
- **Preview mode** (`vergence-closeup --preview`): the same viewer in a
  borderless palette window, for File Viewer's space bar. It follows
  File Viewer's selection over the bus. Space or Escape closes it;
  "Open in Close-Up" makes it a full window.

### 5. File Viewer (`vergence-files`)

- **The window**: a sidebar (the dialog's places, now shared: Home and its
  folders, Recent, the computer, disks and network mounts, Trash, and
  pinned folders), the folder as **a list with columns** (name, size, kind,
  modified; sorted by any) or **icons**, the path as buttons in the
  toolbar, Back, Forward, Up. Tabs in the server's bar; one process for all
  windows; each window's folder and view remembered.
- **Preview**, the first look, two ways. **The preview pane**: a column at
  the window's right (View > Show Preview, remembered per window) showing
  the selected file small, its kind, size and dates, with "Open in
  Close-Up". **The space bar**, Finder style: Space shows the selected file
  large, over File Viewer's own list (drawn in its window, decoded by
  vergence-decode: a window of Close-Up's own would take the keyboard, and
  the arrows must go on moving through the files; 2026-11-15); arrow keys
  move the selection and the preview follows; Space or Escape closes it,
  Enter opens the file (in Close-Up for a picture).
- **Opening**: Enter or a double click opens with the default app; "Open
  with" lists the others and "Other..." chooses any app (section 1).
- **File operations**: new folder, rename (in place), copy, cut, paste,
  duplicate, move to Trash, by keys and the right-click menu. Copies and
  moves run off the window's thread with a progress card for large ones,
  ask before replacing (Replace, Keep both, Skip, for all), and can be
  cancelled. Undo for rename, move and move to Trash.
- **Trash** by the freedesktop Trash spec (~/.local/share/Trash, and
  `.Trash-<uid>` on other disks), so files trashed here show in other
  desktops' trash and back. Restore puts a file back where it came from.
  Empty Trash asks first. Shift+Delete deletes for good after a question.
- **Drag and drop**: within and between File Viewer windows (move on the
  same disk, copy across disks, Ctrl to copy, as other desktops), onto the
  dock's apps, into other apps. That needs **drag and drop on the native
  protocol** (a new bus version: start, offer kinds, enter, motion, drop,
  data by fd, as the clipboard does), plus the server bridging native and
  Wayland drags both ways.
- **Thumbnails** in the icon view, by the freedesktop thumbnail spec
  (~/.cache/thumbnails, shared with other desktops), made by
  `vergence-decode` in the background, only for pictures (and PDFs and
  videos if question 2 brings them).
- **Remote file systems**: what the system has mounted (sshfs, NFS, SMB:
  `mount` output, with Linux /proc/self/mountinfo and FreeBSD getmntinfo)
  shows under Network in the sidebar, with Eject (`umount`, or `fusermount
  -u` for sshfs). Connecting to new servers is by question 7.
- **Search**: typing filters the folder at once (as the dialog does);
  "Search in subfolders" walks down from here, off the thread, as results
  come. An index (as macOS Spotlight has) is not part of this milestone.

### 6. The desktop (`~/Desktop` on the wallpaper)

Dave, 2026-11-15: "on many systems but especially windows i use it like a
scratch folder: app launchers and random files ... Let's include it as
windows does and make it suit our setup." So the files in `~/Desktop` (the
XDG desktop folder, wherever user-dirs.dirs puts it) are shown as icons on
the wallpaper, a scratch place for launchers and files.

- **Drawn by File Viewer**, as Windows draws its desktop with Explorer: a
  borderless window of a new kind, **Desktop** (bus window `Style` kind),
  one per screen with icons (the main screen by default; Settings says
  which), which the server keeps under every other window, over the
  wallpaper, sized to the screen less the dock, never in the dock or Alt+Tab,
  and never given the keyboard but by a click on it. Its ground is clear, so
  the wallpaper and its tint show; icon labels carry a shadow that reads on
  any picture.
- **What it does is File Viewer's**: its icon view, so double click opens
  (by type, section 1), right click has Open with, Rename, Move to Trash,
  Copy, Paste, New Folder, Sort by and "Show the desktop in File Viewer";
  a rubber-band selection; undo; drag and drop with File Viewer windows and
  apps (step 8).
- **Arranged as Windows does**: icons in a grid from the top left down,
  then across, moved freely by dragging and kept where put, or "Sort by"
  name, kind, date; new files take the next free cell.
- **Positions follow the monitor memories** (Dave, 2026-11-15: "the desktop
  folder layout needs to track the monitor memories as well"): kept per
  setup of screens exactly as window memory keeps window places (the same
  setup key: which monitors, their resolutions and scales;
  ~/.local/state/vergence/desktop.toml), so docking, undocking or changing
  a resolution brings back that setup's own arrangement. Icons whose screen
  is gone show on the main screen meanwhile, in free cells, without
  changing that screen's saved layout; when the screen comes back they
  return to their places. A file new while its setup was away takes the
  next free cell when that setup comes back. "Forget window places" on the
  Windows page asks whether to forget the desktop's layouts too.
- **Launchers**: an app dragged from the launcher or the dock onto the
  desktop makes a desktop entry there, shown with its app's icon and name,
  and double click starts it. A desktop entry that arrives any other way (a
  download, a copy) runs only after a question, once ("Allow this
  launcher to start a program?"), marked by its execute bit, as GNOME does:
  a file from the web must not start a program by a double click.
- **Show desktop**: Super+D (and a corner of the dock, later) minimizes
  every window to show the desktop, and again brings them back.
- **Settings > Desktop**: Show icons on the desktop (on by default), which
  screens, icon size, sort.

## Progress

- **Step 1 built** (2026-11-15): crates/vergence-types (table.rs: our own
  ~140 types with names, generic icons, patterns and parents, and ~55
  aliases; lib.rs: `of_name` (longest pattern), `of_bytes` (signatures,
  RIFF and ISO-media kinds, shebangs, SVG/HTML/XML, the text test),
  `of_file`, `ancestors`/`is_a`, `of_link` (x-scheme-handler) and
  `path_of_link`, `MimeApps` read/write, `list_paths` (vergence-mimeapps.list
  before mimeapps.list in each directory), `apps_for`/`default_for`,
  `choose_default` writing only Vergence's list). vergence-shell desktop.rs
  keeps `MimeType` and the raw Exec line, and `command_for` fills `%f %F %u
  %U` (one command per file for %f/%u). `vergence-open` (vergence-shell
  bin, installed): open, `--with`, `--type`, `--apps`, `--default`.
  Checked on kepler and Hubble against real entries and files (Markdown and
  scripts to the Text Editor through text/plain, links to Firefox by the
  default it wrote, photos and an extensionless PNG by their bytes). Tests:
  Hubble 584, kepler 580, Techbench 584. Opening through the server (so a
  started app gets the session's place in the dock) is step 2.
- **Step 2, first parts** (2026-11-15): the Open/Save dialog's filters take
  types (`Filter::of_types`, `image/*`; the Text Editor's "Text files" is
  now every kind of text) and its rows draw a mark per family of types
  (`Kind`: folder, text, picture, sound, video, archive, document,
  program). Bus **v15** `ClientMsg::Open { items, app }` and
  `ErrorCode::Failed`: the server opens files and links by type
  (shelf.rs `open_items`, `start_entry` shared with the dock's start, so the
  dock shows the app starting); vergence-shell open.rs (`item`, `plan`,
  `commands`) is shared by the server and `vergence-open`, which asks the
  server in a session. Checked on kepler: two Markdown files opened as two
  tabs of one Text Editor through the server; `/bin/ls` refused with its
  reason. Tests: Hubble 586, kepler 582, Techbench 586.
- **Step 2 built** (2026-11-15): Settings > **Default apps** (settings
  defaults.rs, in the Desktop group after Apps): Web browser, Mail, Text,
  Pictures, PDF, Music, Video, Folders, each a menu of the apps that open
  it (today's first) with "Not chosen here" once Vergence's list chooses;
  a kind sets all its types at once (vergence-types `choose_defaults`,
  `of_kind`). Seen on kepler's screen. Later: two entries of one app
  (Firefox's own userapp-Firefox-*.desktop beside firefox.desktop) both
  listed.
- **Step 3 built and tried with Firefox on kepler** (2026-11-15, 427fc19 and
  the fixes after): Firefox (a throwaway profile with
  `widget.use-xdg-desktop-portal.file-picker = 1`) showed our dialog for
  Open File, a modal dialog over its window (Firefox dimmed), and opened
  the README chosen in it; Save Page As showed its suggested name and
  filter, and Escape answered "cancelled". Fixed on the way: the server
  turned a dialog with no parent of its own program into a normal window;
  an adopted one now counts as owned (native.rs). Found: kepler's `gdbus
  call` sends wrong bytes for a `b"..."` byte string (the raw body held
  the call's interface name), while `@ay [...]` arrives right: a fault of
  the tool, not of our edge (vergence-dbus gained a round-trip test of
  byte arrays in options). Not yet tried: a GTK and a Qt app, Hubble and
  Techbench. Built as: server
  portal.rs: FileChooser OpenFile (open, multiple, directory), SaveFile
  (current_name, current_file), SaveFiles (a folder chosen, the names
  joined), filters and current_filter, accept_label, current_folder; the
  Request's Response to the caller alone; Request.Close ends the dialog;
  OpenURI.OpenURI by type (OpenFile/OpenDirectory by descriptor answer
  NotAllowed: the edge takes no fds). The dialog is
  libexec/vergence/vergence-choose (vergence-widgets bin; installed), whose
  window the server gives the caller's window as a modal dialog
  (`Portal::adopted`, style.rs, native.rs). FileDialog gains
  `choose_folder`, `multiple` (Ctrl+click, Shift+click) and `accept`; the
  toolkit's `Program::saw` sees every input before the widgets. Settings >
  Apps: "Vergence's file dialog in other apps" (`[apps] own-file-dialog`),
  read at the session's start (GTK_USE_PORTAL, GDK_DEBUG=portals,
  QT_QPA_PLATFORMTHEME). Not yet: `choices`. Tests: Hubble 587, kepler
  583, Techbench 587.

- **Step 4 built** (2026-11-15): crates/vergence-decode: `decode_file`
  gives the opened file to libexec/vergence/vergence-decode as its standard
  input (the helper needs no file system), with a 5 s watchman; the helper
  reads it, closes every other descriptor, sets CPU 10 s, file size 0,
  8 descriptors, 4 GiB of memory, no-new-privileges on Linux and
  `cap_enter` on FreeBSD, then decodes by the bytes: PNG and JPEG (upright
  by EXIF orientation), GIF and WebP with their frames (image crate,
  `gif`/`image-webp` turned on; THIRD-PARTY-NOTICES regenerated), SVG by
  resvg with no outside resources, BMP by our own bmp.rs (1/4/8/24/32 bits,
  bit fields, top-down; RLE refused); `at_least` shrinks it there. The
  settings app's wallpaper previews now come from it (resvg and JPEG gone
  from the settings app). Checked on kepler: its 17 wallpapers, 8K
  included, decoded in Capsicum; the Wallpaper page's previews rebuilt
  through it. Tests: Hubble 596, kepler 592, Techbench 596.
- **Step 5 built** (2026-11-15): crates/vergence-closeup, **Close-Up**
  (data/applications/vergence-closeup.desktop: the picture types it shows,
  keywords closeup / close up; data/icons/vergence-closeup.svg; install.sh).
  Pictures through vergence-decode, a pyramid of halvings made off the
  window's thread (pictures.rs: the renderer samples by the nearest pixel,
  so a large photo is drawn from the level just larger than shown), fitted
  (small ones kept at actual size) or zoomed by the wheel about the pointer,
  + − 0 1 and the roles, panned by dragging, a double click between fitted
  and actual size; R / Shift+R turn a quarter; GIF and WebP animations
  played; a checkerboard under transparency; Left/Right/Home/End through
  the folder's pictures in File Viewer's order, "5 of 17". Text read-only
  in the toolkit's text area with "Open in Text Editor"; other types say
  what they are and offer "Open with its app". One process (hand.rs), a
  file a tab; settings (`[closeup]` ground, small-actual, animate, and the
  usual look rows); dark by default. `--preview FILE`: one file, plain bar,
  Space/Escape close, Enter opens it in full. Seen on kepler's screen: an
  8K photo fitted, next picture, actual size, a quarter turn, a text tab
  handed to the running Close-Up, a tar archive's notice, the preview.
  Not yet: EXIF date in the status line; the preview following File
  Viewer's selection (with File Viewer, step 7). Tests: Hubble 602,
  kepler 598, Techbench 602.
- **Step 6 built** (2026-11-15): vergence-widgets files.rs, taken out of
  filedialog.rs: `Entry` (with its type), `read_folder`, `recent`, `sort`
  (folders first, names naturally), `Place`/`places` with the user's
  folders by the XDG user-dirs file (`user_dirs`: a renamed or translated
  Desktop or Downloads is found), the helpers (`natural`, `size_words`,
  `when`, `tilde`, `mounted_disks`, the recent-files state), and the views
  both draw: `sidebar`, `list` (sortable headings and rows, `View`
  carrying the entries, which are chosen, the row kept in view and the
  click) and the new `icons` view (a wrapping row of cells, a large mark
  over the name); the marks draw at any size (`kind_icon_sized`). The Open
  dialog uses them. New in the toolkit: `label(...).elide()`, one line cut
  with "…" where its place ends (long names in the list no longer run into
  the Size column). Tests: Hubble 603, kepler 599, Techbench 603.
- **The toolbar** (2026-11-15, Dave: "b"): vergence-widgets toolbar.rs, a
  strip under the title bar reaching up under it (one band), with its own
  line icons; Close-Up has one (previous/next, zoom, fit, 1:1, turns,
  Open).
- **Step 7, first part** (2026-11-15): crates/vergence-files, **File
  Viewer** (desktop entry for inode/directory, icon, install.sh): toolbar
  (Back, Forward, Up, the path, "Narrow this folder", list or icons, the
  preview pane, hidden files), sidebar (places and Trash), list or icon
  view, foot ("17 items", "3 of 17 chosen"); tabs, one process (hand.rs;
  a file given opens its folder with it chosen); click, Ctrl+click,
  Shift+click, the arrows (rows in the icon view by its width), Enter and
  double click open (folders here, files by the server's Open); Space's
  Preview over the list (pictures decoded apart at up to 1600 x 1200, the
  start of text, else the mark), the arrows following; the preview pane
  with Open and Open in Close-Up; the folder read again when it changes
  (every 2 s); settings `[files]` (view, pane, hidden; the usual look
  rows). Seen on kepler's screen. Tests: Hubble 607, kepler 603,
  Techbench 607.
- **Step 7, second part** (2026-11-15): vergence-files ops.rs: the
  freedesktop Trash in the user's data folder (an info file made new
  first, then the file renamed in; copied in across file systems;
  `trashed`, `restore` to where it came from under a free name,
  `empty_trash`), `free_name` ("name 2.ext") and `copy_name` ("name
  copy.ext"), `copy_tree` (links as links, permissions and times kept,
  progress by bytes, stopping between pieces), `move_item` (a rename, else
  copy and remove), `plan_paste`/`clashes`/`run_paste` (a replaced target
  goes to the Trash first, so Replace can be undone), `Done` and `undo`.
  In the window: New folder (Ctrl+Shift+N), Rename (F2) in a name strip;
  Cut, Copy, Paste (paths also on the clipboard as text), a clash asking
  Replace / Keep both / Skip; Duplicate (Ctrl+D); Move to the Trash
  (Delete, the toolbar; in the Trash itself it means for good); Delete for
  good (Shift+Delete, after a question); copies and moves on a worker
  with a progress strip and Stop; Undo (Ctrl+Z) for each; in the Trash,
  "Put back" and "Empty Trash" (after a question). Tried on kepler in a
  scratch folder: new folder, copy into the same folder ("b copy.txt"),
  undo of the copy (to the Trash), Move to the Trash by the toolbar and
  its undo. Not yet seen on screen: the progress strip on a large copy;
  drag and drop is step 8. Tests: Hubble 611, kepler 607, Techbench 611.
- **Step 8, native drags** (2026-11-15): bus **v16**: `WinRequest::DragStart`
  (the data travels with it, `MAX_DRAG_BYTES` 48 KB: file drags are a
  few paths, so no pipes, and no deadlock when a program drops on its own
  other window) and `DragAccept`; `WinEvent::DragOver` (with Ctrl and
  Shift), `DragLeave`, `DragDrop` (with the data), `DragEnded`. The server
  (drag.rs) takes a drag only while the left button is held on the window,
  follows the pointer beside the seat, tells the native window under it,
  takes drops on the dock's app icons itself (`dock_item_at`; the files
  opened with that app), drops on the release, and draws a ghost of the
  drag's words, in the accent while a drop would do something. Toolkit:
  `Window::start_drag`, `App`/`Program` `drag_over`, `drag_left`,
  `dropped`, `drag_ended`; buttons gain `.on_drag(...)` (pulled 6 points
  away); `files::View::drag`. File Viewer drags its chosen files (or the
  row pulled) as text/uri-list and text/plain, and takes drops into the
  folder shown (moved, Ctrl copies; clashes ask; undo); Close-Up and the
  Text Editor open dropped files; Ctrl+N in File Viewer and Close-Up
  (role keys are each app's to catch). Tried on kepler in a scratch folder:
  File Viewer to the Text Editor (opened), between two File Viewer windows
  (moved; the source saw it at its next folder watch, about 2 s), onto the
  dock's Text Editor (opened in the running one). Not yet: drags to and
  from Wayland and X11 programs (Smithay's drag grab knows only Wayland
  surfaces), drops on a folder row or a sidebar place. Tests: Hubble 612,
  kepler 608, Techbench 612.
- **Opening brings the app forward** (2026-11-15, Dave: "double clicking a
  file in the file manager should open it in the app associated with it
  AND bring that window forward focussed"): a program of one process (the
  Text Editor, Close-Up, File Viewer) handed a file by a new start opened
  no window, so nothing came forward. Bus v16 `WinRequest::Activate`
  (toolkit `Window::activate()`, asked on each hand-over): the server
  (opened.rs `window_activate`) grants it by the new window's rule (the
  app just started by the person, nothing typed elsewhere since, or the
  focused program's own): raised, brought back if minimized, given the
  keyboard; else refused (no stealing focus). Seen on kepler: a double
  click in File Viewer sent the file to the running Text Editor, which came
  forward with the keyboard. Wayland programs ask by xdg-activation, which
  the server does not offer yet.
- **Step 9 built** (2026-11-15): the desktop. Bus **v17** `WinKind::Desktop`.
  Server desktop.rs: a Desktop window is placed on the main screen's work
  area (at its kind and again at its first showing, and whenever the work
  area changes: `shelf_moved`), kept under every window (never raised;
  `desktop_to_bottom`), without bar or border, left out of the dock,
  Alt+Tab, window memory and the session's list; the dock does not hide
  for it; a right press on it is the desktop's menu. The session starts
  `vergence-files --desktop` unless `[desktop] icons = false` (Settings >
  Dock > Desktop). Super+D (the seat's ShowDesktop) minimizes every window
  and brings the same back. The dock's menu has "Add to desktop" (the
  entry copied in, executable). File Viewer's desktop mode (desktop.rs, app
  id vergence-desktop): ~/Desktop's icons on a clear ground, a grid down
  then across, pictures' and launchers' own marks (decoded apart), names
  with a shadow; click, Ctrl+click, double click to open (folders in File
  Viewer, files by type, launchers start; an untrusted launcher asks once
  and is made executable); drags out, drops in (moved, Ctrl copies), the
  desktop's own icons placed where let go; Delete to the Trash, F2 to
  rename, the arrows; cells kept per desktop size in
  ~/.local/state/vergence/desktop-places (a size stands for the set of
  screens: the full window-memory key later). Seen on kepler at 1920 x
  1080: the icons over the wallpaper after Super+D, a picture's own mark,
  launchers' application icons, the question for an untrusted launcher,
  the trusted Calculator starting. Not yet: a rubber-band choice, desktops
  on other screens, rename in place, thumbnails of other kinds. Tests:
  Hubble 614, kepler 610, Techbench 614.
- **Fixed desktop icons and the text area's menu** (2026-11-15, Dave: "a
  default set of icons there: Home, Trash, Mounted Volumes"; "text edit to
  have a standard style context menu as terminal does"): the desktop shows
  Home, each disk mounted (files::mounted_disks) and the Trash before its
  files, with marks of their own; they open in File Viewer, take drops
  (the Trash trashes, Home and a disk take files in, as any folder icon
  does now), cannot be renamed or trashed, and dragged only move on the
  desktop (their drag carries `application/x-vergence-desktop-icons`, no
  file links); `[desktop] home`, `volumes`, `trash` = false leaves one out.
  The toolkit's text area's right-click menu is the standard one: the
  spelling lines first, then Undo, Redo | Cut, Copy, Paste | Select all,
  every line always there (faint when it cannot be used), the keys at the
  right, the desktop's menu colors; the arrows step over faint lines. Seen
  on kepler: Home and the Trash on the desktop; the menu in Close-Up's text
  view. Tests: Hubble 614, kepler 610, Techbench 614.
## Steps

1. **vergence-types**: the database reader (globs2, magic, subclasses,
   aliases, icons), the built-in fallback, desktop entries' MimeType,
   mimeapps.list read and write, the Exec line. Tests against samples of
   each file. `vergence-open`.
2. **Types in the toolkit and the shell**: the Open/Save dialog filters by
   type and shows icons by type; the server opens files and URLs with the
   right app; Settings > Default apps.
3. **The portal**: FileChooser (`vergence-choose`, the Request/Response
   objects, a dialog over another client's window), OpenURI, the session's
   environment. Checked with Firefox, a GTK app and a Qt app on each
   machine that has them.
4. **vergence-decode**: the helper, its limits per platform, timeouts;
   the settings app's wallpaper previews move onto it.
5. **Close-Up**: pictures and text, the folder's next and previous,
   preview mode.
6. **Shared file widgets**: the places sidebar and the file list come out
   of filedialog.rs into vergence-widgets parts the dialog and the File
   Manager share; the list gains an icon view.
7. **File Viewer**: the window, tabs, opening, Open with, space bar
   preview, file operations with progress and undo, Trash.
8. **Drag and drop**: on the native protocol (bus version bump), the
   server's bridge to Wayland, used by File Viewer and the dock.
9. **The desktop**: the Desktop window kind in the server, File Viewer's
   desktop windows, positions, launchers and their question, Super+D,
   Settings > Desktop.
10. **Thumbnails, mounts, search in subfolders.**
11. **Docs**: whitepaper (§6.10 and §9), README, the Map, CLAUDE.md.

PDF and video, if question 2 brings them in, are their own steps after 5.

## Acceptance

- A file opens in the right app by its type from File Viewer, the
  dialog, `vergence-open` and Firefox's downloads; a changed default in
  Settings is what xdg-open uses too.
- Firefox's Open and Save use our dialog over Firefox's window, and so do a
  GTK and a Qt app where installed; Cancel returns nothing; filters work.
- Close-Up shows PNG, JPEG, GIF (animated), WebP, BMP and SVG pictures at
  every zoom, steps through a folder, and a broken or hostile file
  shows "Could not be shown" without harming Close-Up.
- File Viewer copies, moves, renames, trashes and restores, with
  undo; a copy of a few GB shows progress and can be cancelled; a
  conflict asks; Trash agrees with another desktop's (checked on Techbench
  with GNOME's Files).
- Space previews the selection and follows the arrow keys.
- Files drag between File Viewer windows and onto another app.
- `~/Desktop`'s files show on the wallpaper, open, rename and move to the
  Trash there; an app dragged from the launcher becomes a launcher that
  starts it; a downloaded launcher asks before it runs; Super+D shows the
  desktop and brings the windows back.
- Builds and passes its tests on Hubble, kepler and Techbench; run on
  kepler (FreeBSD) and Techbench (Fedora).

## Questions for Dave

1. **Order**: (a) as above: types, portal, decoding, Close-Up, File Viewer,
   drag and drop; (b) File Viewer first (with types), the portal and
   Close-Up after.
2. **PDF and video in this milestone?** (a) not now: pictures and text only;
   PDF and video get their own milestone with the borrowing question then
   (PDF: our own renderer, a long project, or Poppler (GPL) / PDFium (BSD);
   video: FFmpeg (LGPL/GPL) or the GPU's decoders through VA-API on
   Linux); (b) decide now and include them.
3. **More picture formats**: the `image` crate (already borrowed for PNG and
   JPEG) also decodes GIF, WebP, BMP, TIFF if those features are turned on.
   That borrows more of a crate already in. (a) turn on GIF and WebP (the web's
   formats); (b) PNG, JPEG, SVG only for now; (c) more (BMP, TIFF, ...).
4. **File type data**: (a) read the system's shared-mime-info database,
   with our small built-in table only as a fallback; (b) ship our own
   table and ignore the system's.
5. **Default apps**: (a) kept in the XDG `mimeapps.list`, so xdg-open,
   browsers and other desktops share them; (b) in our desktop.toml only.
6. **Make other apps use our dialog**: (a) the session sets
   `GTK_USE_PORTAL=1` and Qt's portal theme for every app it starts, and
   our install notes say how to set Firefox's preference; (b) leave each
   app as it is, so only apps that already ask the portal get it.
7. **Remote file systems**: (a) only what the system mounts (sshfs, NFS,
   SMB), shown and ejected in File Viewer; (b) also "Connect to
   server..." that runs the system's mount tools (sshfs) for you; (c) our
   own SFTP client (a long project, later).
8. **Names**: *answered 2026-11-15:* **File Viewer** (the file manager),
   **Preview** (its quick look), **Close-Up** (the full app).
