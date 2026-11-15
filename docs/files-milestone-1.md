# Files milestone 1: file types, the file chooser for every app, the Viewer, the File Manager

**Drafted 2026-11-15**, item 5 of docs/apps-roadmap.md. **Draft: the
questions at the end are for Dave.**

**Answers so far (Dave, 2026-11-15):** 1 a (the order as planned); 2 a (PDF
and video not now, their own milestone with the borrowing question then);
4 b (our own file type table, not the system's database: written by us,
since shared-mime-info's data is GPL and the workspace is Apache-2.0 OR
GPL-2.0-or-later); 7 to revisit (only the system's mounts until then).
3, 5 and 6 in discussion. 8: the file manager is named **File Viewer**
(after NeXTSTEP Workspace Manager's File Viewer window); the viewer's name
in discussion.

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
  out of the File Manager needs it.

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
  system's). Writing the choice there means xdg-open, browsers and other
  desktops agree with ours (question 5).
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
  environment variables for every app it starts (question 6). Each app's
  actual behavior gets checked here, not assumed from documentation.
- **Also on the edge** (small, same pattern): `org.freedesktop.portal.OpenURI`
  (`OpenURI`, `OpenFile`, `OpenDirectory`) through section 1, so "open
  file" and "show in folder" in Firefox's downloads reach our apps.

### 3. Decoding, kept apart (`vergence-decode`)

Pictures from anywhere are untrusted input (§6.10, §7.3). The Viewer, the
File Manager's previews and thumbnails, and the file chooser all decode
through one helper process: it reads one file, decodes it, and returns the
pixels over a pipe as shared memory. It runs with nothing else: Capsicum
`cap_enter` on FreeBSD after opening the file; on Linux, closed descriptors
and no network (Landlock and seccomp later, tracked in §8.3's list). A
crash or a hang (5 s) in a bad file costs a "Could not be shown" in the
app, never the app. Formats: PNG, JPEG, SVG now (what is already borrowed),
others by question 3.

### 4. The Viewer (`vergence-view`)

A standalone app, one process for all its windows (tabs in the server's
bar, as the terminal and the editor do).

- **Pictures**: fit to the window, 100%, zoom by keys, pinch and scroll,
  pan by dragging, rotate the view, full screen, the picture's size and
  type and, for JPEG, when it was taken (EXIF read by us, a small parser).
  Next and previous file in the folder (Left/Right, sorted as the File
  Manager sorts). Transparency on a checkerboard.
- **Text**: shown read-only in the toolkit's text area, with "Open in Text
  Editor".
- **PDF and video**: by question 2. Without them, the Viewer says it cannot
  show this type and offers the app that can (section 1).
- **Preview mode** (`vergence-view --preview`): the same viewer in a
  borderless palette window, for the File Manager's space bar. It follows
  the File Manager's selection over the bus. Space or Escape closes it;
  "Open with Viewer" makes it a full window.

### 5. The File Manager (`vergence-files`)

- **The window**: a sidebar (the dialog's places, now shared: Home and its
  folders, Recent, the computer, disks and network mounts, Trash, and
  pinned folders), the folder as **a list with columns** (name, size, kind,
  modified; sorted by any) or **icons**, the path as buttons in the
  toolbar, Back, Forward, Up. Tabs in the server's bar; one process for all
  windows; each window's folder and view remembered.
- **Space bar preview**, Finder style: Space shows the selected file in the
  Viewer's preview window; arrow keys move the selection and the preview
  follows; Space again closes it.
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
- **Drag and drop**: within and between File Manager windows (move on the
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
5. **The Viewer**: pictures and text, the folder's next and previous,
   preview mode.
6. **Shared file widgets**: the places sidebar and the file list come out
   of filedialog.rs into vergence-widgets parts the dialog and the File
   Manager share; the list gains an icon view.
7. **The File Manager**: the window, tabs, opening, Open with, space bar
   preview, file operations with progress and undo, Trash.
8. **Drag and drop**: on the native protocol (bus version bump), the
   server's bridge to Wayland, used by the File Manager and the dock.
9. **Thumbnails, mounts, search in subfolders.**
10. **Docs**: whitepaper (§6.10 and §9), README, the Map, CLAUDE.md.

PDF and video, if question 2 brings them in, are their own steps after 5.

## Acceptance

- A file opens in the right app by its type from the File Manager, the
  dialog, `vergence-open` and Firefox's downloads; a changed default in
  Settings is what xdg-open uses too.
- Firefox's Open and Save use our dialog over Firefox's window, and so do a
  GTK and a Qt app where installed; Cancel returns nothing; filters work.
- The Viewer shows PNG, JPEG and SVG pictures (and the formats question 3
  adds) at every zoom, steps through a folder, and a broken or hostile file
  shows "Could not be shown" without harming the Viewer.
- The File Manager copies, moves, renames, trashes and restores, with
  undo; a copy of a few GB shows progress and can be cancelled; a
  conflict asks; Trash agrees with another desktop's (checked on Techbench
  with GNOME's Files).
- Space previews the selection and follows the arrow keys.
- Files drag between File Manager windows and onto another app.
- Builds and passes its tests on Hubble, kepler and Techbench; run on
  kepler (FreeBSD) and Techbench (Fedora).

## Questions for Dave

1. **Order**: (a) as above: types, portal, decoding, Viewer, File Manager,
   drag and drop; (b) the File Manager first (with types), the portal and
   Viewer after.
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
   SMB), shown and ejected in the File Manager; (b) also "Connect to
   server..." that runs the system's mount tools (sshfs) for you; (c) our
   own SFTP client (a long project, later).
8. **Names**: "File Manager" and "Viewer", plain as "Text Editor"? Or other
   names you'd rather have.
