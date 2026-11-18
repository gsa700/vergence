# The apps: a roadmap (draft)

Drafted 2026-11-14 from Dave's list (`~/vergence-app-ideas` on Control,
"not complete but enough to keep us busy for weeks at least") and the
settings ideas of the same afternoon (docs/next-settings.md). An order, not
a plan: each app gets its own milestone plan when its turn comes. **Draft:
the order and the questions at the end are for Dave.**

## Dave's list

| App | In his words |
|---|---|
| Text Editor | "a native text editor for Vergence. Like TextEdit on Mac or GNOME Edit. Autocomplete and line numbers required." |
| Spellchecking | "all native apps and maybe others via extensions" |
| Calculator | "a basic calculator with advanced scientific as an option" |
| File manager | "with space bar preview ala macOS finder ... remote file system support as well (using native OS tools?)" |
| Backup | "zfs snapshots on FreeBSD and to remote hosts that support it. Rsync as a fallback. Possibly integrated right into the file manager for file restoration and versions support." |
| Web browser? | "big project but maybe we could use webkit and build a native app around it with ublock origin support" |
| Mail? | "only if we do a browser first" |
| Disk utility | "useful tool all desktops should have" |
| Viewer | "picture, video, and pdf viewer. Connected to file manager by file type association, but a standalone app" |
| Code Navigator | "walks source code trees (like Vergence) and explains what a section of code does and why (may need agentic assistance)" |
| Hardware | "monitor details on the hardware, CPU, GPU, Network, Memory, etc." |
| Running Applications | "process monitoring and control, kill stuck programs etc." |
| Agentic JIT apps | "A base framework with the toolkit and system reference that uses your choice of AI agent to build apps to spec on demand. Maybe shared with others via central hub, not the app itself but just its spec?" |

## What the apps stand on

Most of the list needs the same few things first, and each is cheaper to
build once than in every app:

- **The app framework** (whitepaper §6.10): an app's own settings window
  with inherit-or-override rows, its menu (a menu button in the bar the
  server draws), About, window memory, one process for all its windows (as
  the terminal does).
- **Text editing in the toolkit**: today vergence-widgets has a one-line
  field; the editor, the file manager's rename and every form need a
  multi-line editor: selection, undo, find, large files, input methods
  later (§10).
- **Open and Save dialogs** of our own (the dialog kind exists since window
  styles milestone 1), and **file types**: which app opens what
  (freedesktop MIME data, read, not borrowed code).
- **Lists, trees and tables** in vergence-widgets, with Contract 3 meaning.
- **Decoders**: pictures (what the wallpaper code uses today), and the
  questions of PDF and video (below).

## A proposed order

1. *Done 2026-11-14.* **Settings milestone 2**: the afternoon's notes (Ethernet like Wi-Fi,
   resolution and refresh rate, "looks like" sizes, same physical size on
   every screen, energy on battery and on power, forget an app's window
   places with a confirmation, confirm closing several tabs, the UPS page,
   low-battery actions). Small, and most are asked for already.
2. *Done 2026-11-14 (docs/app-settings-milestone-1.md).* **App settings** (Dave, 2026-11-14: "settings per app (terminal and
   settings itself right now), and the ability to choose per app display
   settings ... dark terminal with light toolbar, light terminal with dark
   titlebar, etc."): the toolkit's preferences window with inherit-or-
   override rows (whitepaper §6.10, decided 2026-11-10), the terminal and
   the settings app first; and per-app look: the app's content style and
   its window bar's style (light or dark) chosen apart. This **changes a
   decision**: §6.10 says window furniture "is the desktop's alone and has
   no per-application override"; the bar's light/dark becomes a per-app
   override (the bar's layout, buttons and corners stay the desktop's).
3. *Built 2026-11-14/15 (docs/tools-milestone-1.md): Calculator, Force Quit, Hardware.* **Three small native tools that make the app framework**: **Calculator**
   (basic, with a scientific mode), **Running Applications** (processes:
   Linux /proc, FreeBSD's kern.proc sysctls; end, kill, by window too),
   **Hardware** (CPU, GPU, memory, disks, network: vergence-power,
   vergence-net and the GPU probe already read much of it). Each small;
   together they settle the framework every later app uses.
4. **Text editing in the toolkit, then the Text Editor** (built
   2026-11-15, docs/editor-milestone-1.md; the Open/Save dialog of item 5
   came with it): line numbers,
   autocomplete (words of the open documents first; languages later), find
   and replace, tabs; **Spellchecking** with it, as a toolkit service every
   native text field can use.
5. *Built 2026-11-15 (docs/files-milestone-1.md, steps 1-11): File Viewer, its Preview, Close-Up, the portal's file chooser, thumbnails, network mounts and removable drives, a Windows-style desktop. Not in it: PDF and video (question 2), connecting to servers (question 7), drags with Wayland programs.* *Accepted 2026-11-15; names (Dave): File Viewer, its Preview, Close-Up.* **Open/Save dialogs and file types, then the Viewer** (pictures first,
   PDF and video by the answers below) **and the File Manager** (Finder's
   space-bar preview through the Viewer; remote file systems by the
   system's own mounts). The Open/Save dialog itself was built early, in
   editor milestone 1 (vergence-widgets filedialog.rs); here it also
   becomes **the file chooser for apps that are not ours** (Dave,
   2026-11-15: "yes add the portal to item 5"): the server answers the
   desktop portal's FileChooser on the session D-Bus (our own D-Bus code,
   vergence-dbus), so Firefox, GTK and Qt apps open the same dialog, and
   whatever it gains later reaches them too.
6. **Backup**, then into the file manager: ZFS snapshots on FreeBSD (and
   Linux with ZFS), `zfs send` to remote hosts over SSH, rsync where there
   is no ZFS; a file's versions from its snapshots in the file manager.
7. **Disk utility**: disks, partitions, file systems, ZFS pools, SMART;
   changing anything needs root, so a small root helper of our own (as the
   sound helper on FreeBSD), designed with §7.3.
8. **Code Navigator** and **agentic apps**: on the agent interface the
   whitepaper plans (§6.8, "any agent can be plugged in"); the agentic app
   idea wants a section of its own in the whitepaper first: what a spec is,
   how a generated app is confined (it runs as the user), and what a shared
   hub would hold.
9. **Web browser, then Mail**: the largest by far, and the most borrowed;
   last, and only by Dave's word.

## From Dave's list, 2026-11-14 night

Added to the roadmap at his word ("Yes add them"); each gets its place in
the order when its turn is near:

- **Fonts of our own**: "not every font, just a few: Sans Mono etc."
- **Font rendering checked against publishing standards and readability
  guidelines**, to make Vergence "the BEST at font rendering and scaling",
  with "extra focus on the window and app proportions"; and **the same
  layout (fonts, spacing) on FreeBSD and Linux**.
- **The Sound page picks up a card's extra features**: "mostly things like
  IO and loopback, midi, etc."
- **The dock's icons by drag and drop** (dock milestone 1, step 1, 2026-11-16), while the dock itself is moved
  "with its menu or a modifier and drag".
- **The dark title bar's color chosen**, "maybe something like the accent
  color picker".
- **The wallpaper tint toned down** (2026-11-15): "the option to shade the
  window with the desktop background needs settings to tone it down. I
  have it turned off on hubble and just now kepler. it covers the frosted
  text in dark mode." Today it is on or off only (Settings > Appearance;
  a fixed 8%, theme.rs `TINT_MIX`): an amount to choose, and the text
  scrolled under the frosted bar kept readable with it on.

- **Screenshots and screen recording built in, with macOS's keys**
  (2026-11-15: "a built in for screen shots and video capture that mimics
  the macos keyboard shortcuts, I love that so much more than diving into a
  menu or print screen copy paste"; "when we get to the right spot"). As on
  the Mac, with Super for Command: Super+Shift+3 the whole screen (each
  screen its own file), Super+Shift+4 an area dragged out (Space turns it
  into a window picker: the window alone, with its shadow), Super+Shift+5
  a small bar for both, with screen and area recording; Ctrl added sends
  the picture to the clipboard instead of a file. The capture shows as a
  **notification card holding the picture** (Dave, 2026-11-15: "that
  floating image could just be a notification containing the picture and
  clicking opens it in Close-up for viewing and minor edits"): the
  server's own cards (notify.rs) gain a picture; a click opens it in
  Close-Up, whose **minor edits** (crop, rotate, simple markup, as macOS
  Preview's) come with this or before it. Files to ~/Pictures/Screenshots by default, named by
  date and time. The server does the capturing (it has every pixel; vctl
  shot already captures a screen); recording needs a video encoder, which
  is the video milestone's question.

## From Dave's list, 2026-11-15 evening

New above the line in ~/vergence-app-ideas; what each touches, and how far
it is today:

- **Our own display manager ("VDM"), the lock screen with it, and other
  ways in: YubiKey, fingerprint.** The lock screen belongs in the server
  (only the server can be sure nothing else draws or reads keys while
  locked), and a login screen is the same picture before a session
  exists, so one piece of work: a greeter run by the server on the seat,
  password through PAM (FreeBSD's and Linux's, an OS interface, not
  borrowing), YubiKey through PAM's U2F module, fingerprint through fprintd
  on Linux (FreeBSD has no common one). On Fedora it takes GDM's place,
  which systemd's logind expects to be told about; on Void and FreeBSD it
  replaces logging in on the text console. There is no lock screen today:
  the lock comes first.
- **Syntax highlighting in the Text Editor**, common languages. The text
  area already draws colored runs (spelling waves, marks); what is needed is
  a highlighter per language. Our own small tokenizers (keywords, strings,
  comments, numbers) cover most of the value; tree-sitter would be
  borrowing, so it would need Dave's word.
- **A mouse cursor set of our own**: the cursors are Adwaita's today
  (borrowed, whitepaper §8). Goes with our own fonts and icons as the
  look's own assets.
- **Our native apps first in the app picker**: done on 15 November
  (00eacae, the launcher's "Vergence" row at the top). If "app picker"
  also means File Viewer's "Open with" list: our own apps are already first
  among those that open a type, after the user's own choices.
- *Done 2026-11-15 (8bbcc0c).* **The terminal's settings window a little larger**, so all of it shows
  without scrolling. Small.
- *Done 2026-11-15 (8bbcc0c).* **A terminal setting to scroll on output** (jump to the bottom when a
  program writes, or stay where you scrolled to). Small.
- *Done 2026-11-15 (8bbcc0c: terminal editors as the default for text).* **Terminal programs as default apps** (nano, vi, emacs, ed for text
  files). Opening already runs a desktop entry with `Terminal=true` in our
  terminal (`vergence-term -e`); what is missing is that nano and the others
  ship no desktop entry on Fedora or Void. Settings > Apps would offer the
  terminal editors found on the system and write a small entry of our own
  for the chosen one (`vergence-term-nano.desktop`, `Terminal=true`) plus
  the line in vergence-mimeapps.list.
- *Done 2026-11-16 (polish batch): New Tab, New Window and Find… after the text's lines, through vergence-widgets' AreaOptions::commands.* **The Text Editor's right-click menu as full as the terminal's.** The text
  area gained a standard menu on 15 November at about 15:10 (Undo, Redo,
  Cut, Copy, Paste, Select all, spelling suggestions above); a session
  started before then does not have it. The terminal's also has New tab,
  the splits and moving a pane: the editor's equivalents (New tab, New
  window, Find) can join it.
- **Snapshots for the backup app on Linux, as ZFS gives on FreeBSD**: Btrfs
  has them (read-only subvolume snapshots, `btrfs send` to another
  machine), and Fedora installs on Btrfs (Techbench's / and /home are
  Btrfs); Hubble's ext4 has none, so rsync stays the fallback there. The
  backup app would work through one interface with three back ends: ZFS,
  Btrfs, rsync.

## From Dave, 2026-11-16

- *Done 2026-11-16 (polish batch, vergence-settings single.rs): a second start hands its --page/--app/--forget to the running one, which turns to the page and comes forward.* **Only one System Settings at a time** (Dave: "there should only ever
  be one copy of System Settings at any one time"). Starting
  vergence-settings while one is running (a dock tile's click, `--page`,
  `--app`, `--forget`, an app menu's Settings…) brings the running window
  forward and turns it to the page asked for, instead of opening a second
  window. Seen when the clock tile's click opened a new one (dock milestone
  1, step 2).

- *Done 2026-11-16 (polish batch): a search field across the top and six apps under it, two rows of three, in the dock's colors, after macOS's Apps icon (Dave's description).* **An apps button that says what it is** (Dave, 2026-11-16, after it became
  four squares in dock milestone 1): "I suspect we will want to replace it
  with an icon that tells the user their apps live there". The four squares
  stay until then.

## From Dave, 2026-11-16 afternoon (wish list)

- *Done 2026-11-16 (polish batch 2).* - **Text size up and down in the Text Editor as in the terminal.** The
  editor has Zoom in, Zoom out and Actual size (Ctrl+=, Ctrl+-, Ctrl+0),
  but they are not featured, so its app menu shows no "− 100% +" Text size
  row as the terminal's does; featuring them (and limiting them at the
  ends, as the terminal does) is likely all it takes. Small. As in the
  terminal (Dave): the row's change is for now (this window, this run);
  the lasting text size is the app's own settings window's.
- **A context menu for File Viewer's items**, "a blend of macOS and
  Windows with Linux flavoring": Open, Open With ▸, Show in Folder; Cut,
  Copy, Paste, Duplicate, Make Alias/Link; Rename, Move to Trash; Compress,
  Get Info/Properties; Copy Path and Open in Terminal (the Linux flavor);
  the folder's own menu on empty ground (New Folder, New File, Paste,
  Sort by, Show hidden). To be drafted against what files milestone 1's
  menus already hold.
- *Done 2026-11-16 (polish batch 2): the terminal keeps its columns clear of the bar (the Text Editor already did).* - **Room for the scroll bar at the right margin** in the terminal and
  probably other apps: when a scroll bar shows, the content stops short of
  it instead of running under it. A toolkit fix (vergence-widgets keeps a
  gutter already for some views: settings milestone 2), then the terminal.
- *Done 2026-11-16 (polish batch 2).* - **A click on the desktop closes any open menu or dock applet card**
  ("dockets", Dave's name for the dock's applets and tiles): today a click
  on the wallpaper leaves a tile's card or a menu open.
- *Done 2026-11-16 (polish batch 2).* - **The volume docket: the mouse wheel over it changes the volume** (as
  the volume keys do, 5% a notch), on the dock's Sound tile and in its card.

- *Done 2026-11-16 (polish batch 2).* **Settings' sidebar under the title bar** (Dave: "the gray
  background on the left panel shows through the title bar"): kept, as
  macOS does, but with a crisp edge: the frost shader takes a split line it
  does not blur across (vergence-render blur.rs `split`), a sidebar
  window's from its bar's `Tools { from }`.
- *Done 2026-11-16 (polish batch 2).* **kepler without its touch panel: shorter bars, wide buttons**:
  the bar's picture was kept when only its height changed (the touch
  size's buttons squeezed into the standard bar); its height is part of
  the picture's key now, and a touch panel leaving redraws.
- **kepler's picture looking slightly taller than wide**: Vergence drives
  3840x2160 on a 310x174 mm panel (both 16:9), square pixels; a test
  picture (/tmp/aspect-test.png on kepler, a 1600-pixel square) for Dave
  to measure; if it is not square on the glass, the monitor's own scaling.

## VDM follow-ups (Dave, 2026-11-16)

- *Done 2026-11-16 (polish batch 2): the Lock Screen page's "Require the password", `[lock] grace`.* - **A delay between the screens going off and the lock coming on**
  (10 s, 30 s, 1 minute): a touch within it brings the screens back with
  no password. The Lock Screen page gains "Require the password: at once,
  after 10 s, 30 s, 1 minute" (desktop.toml `[lock] grace`). Dave: it
  applies only when the screens went off by their timeout; a lock asked
  for (Super+L, Lock in the log out dialog, the named action, vctl) locks
  at once, and so does locking before sleep.
- **The login screen's look, machine-wide** (Dave, 2026-11-16: "it would
  be nice if the user could set some vdm prefs like wallpaper"), as KDE
  does for SDDM: a Login screen part on Settings' Login page (wallpaper:
  a picture, a color or the default; light or dark and the accent; its
  text size; maybe a short message, as macOS offers), each change through
  the elevation prompt. vdm never opens a path a program names as root:
  it reads the chosen picture *as the person* (a child that becomes them),
  keeps a copy in the login screen's own directory (/var/lib/vdm-greeter,
  /var/db/vdm-greeter), and the login screen decodes it as _vdm, so root
  never handles picture data. Settings in a small file there the login
  screen reads.
- **Each person's own look at the login screen, if they choose** (as
  slick-greeter, macOS and Windows do): "Show my wallpaper on the login
  screen when I'm selected", off by default (it shows the wallpaper to
  anyone at the machine). vdm keeps a copy per person, read as them, as
  AccountsService does; the same copy gives **account pictures** in the
  login screen's list in place of initials (VDM milestone 1's "pictures
  later from a vdm-kept copy"). Also possibly: **system-wide defaults**
  for new accounts (/etc/vergence/desktop.toml, /usr/local/etc/vergence on
  FreeBSD, read under each person's own settings), the machine-wide look
  above being one use of it.
- Open by hand from VDM milestone 1: a YubiKey at the login screen, a
  screenshot while locked, `--no-vdm`. (*Done 2026-11-17: the login screen
  on several monitors, on Techbench's LG + ASUS; it starts on the last
  session's main screen, 3ca9a85.*)

## From Dave, 2026-11-17 (to do)

- **The dock's size, chosen by the person**: by dragging its edge (as the
  macOS Dock's divider does), or a slider on Settings' Dock page, or both.
  Today the dock's size follows the bar's proportions only.
- **As compatible as possible without Wayland's own software installed**:
  Vergence speaks Wayland to outside apps but needs no libwayland (Dave's
  decision of 2026-11-16: Smithay's pure-Rust backend instead of
  `use_system_lib`; the running server dlopens libwayland-server today, seen
  on kepler). Find every part that pulls in Wayland's packages (the server,
  Xwayland's needs, the toolkit's Wayland road, install.sh's package lists)
  and drop or replace each, with outside apps still working. Apps bring
  their own libwayland-client; that's theirs, not ours.
- *Done 2026-11-17 (21dd778): the D-Bus edge passes file descriptors, the portal opens the folder in File Viewer; Dave's click on Hubble works.* **Bug: Firefox crashes opening the Downloads folder** from its downloads
  panel. Firefox asks the desktop to show the folder (the portal's
  OpenURI/OpenDirectory, or org.freedesktop.FileManager1.ShowItems over
  D-Bus); our portal answers fd-based OpenFile/OpenDirectory with
  NotAllowed (files milestone 1), which may be what Firefox doesn't survive.
  Reproduce, read Firefox's crash report, then answer it properly (show the
  folder in File Viewer).
- *Done 2026-11-17 (6db311d): the range is the text's height less the view's; the last line stops at the bottom.* **Bug: the Text Editor scrolls when the text doesn't fill the window.**
  A few lines still give a scroll range; probably the area counts a page of
  room past the last line, or the under-bar inset, as content. Scrolling
  should start only when the text is taller than the view.
- **Next (Dave, 2026-11-18): native windows on whole pixels at fractional
  scales.** At 133⅓% a window's bar and body still disagree by a pixel by
  position (the bar's outline shows or is cropped; a line flickers at the
  seam while dragging). Snap native windows' positions and the sizes the
  server gives out to multiples of q points for a scale p/q (4/3: 3), so
  nothing rounds. Done before it: 94c8862 (points back exactly), e24e617
  (edges rounded once). Shared memory has the details.
- **Polish: default text sizes that fit the screen** (Dave, techbench,
  2026-11-17). Today every app's text starts at a fixed point size (10 pt,
  the terminal's too) and only the screen's scale grows it. The scale
  already follows pixel density and viewing distance, so the open question
  is the rest: a big monitor read from farther away, a small laptop panel
  held close, a TV. Consider a default text size from the screen's physical
  size (EDID) as well as its density, offered as the "Desktop default", and
  check it on techbench's 27" LG (Dave's pick there: 133⅓% scale, terminal
  at 15) against Hubble's 15.6" panel and kepler's 14". Dave: the LG sits
  closer than it should, and the default "13.3" was too small, 15 right:
  the scale was right for the interface, the text wanted a step more.
  Units to fix with it: the terminal's "Desktop default (13.3)" is 10 pt in
  pixels (×96/72) while its list's 8–24 are pixels taken as they are, so
  15 there is about 11.3 pt; the menu should speak points, as the desktop
  does.
- **Bug to watch: the terminal's text size once did not change** from its
  settings window (techbench, 2026-11-17); a live retry worked and writes
  were seen landing. Note what was picked and what else was open if it
  comes back.
- **Packages for Void (xbps) and Fedora (RPM)**, after the FreeBSD one
  (tools/freebsd/make-pkg.sh, docs/packaging.md); then a clean first
  install on techbench following only pkg's message, to list every root
  step a real user meets.

## From Dave, 2026-11-18 (wish list, ~/Vergence-wishlist-2 on techbench)

- **A remote viewer**: native Vergence (scenes, not pixels) and VNC, SPICE,
  RDP; each outside protocol's library is a borrowing question.
- **PDF in Close-Up** (or a viewer of its own): a PDF library is a
  borrowing question (pdfium BSD, MuPDF AGPL, poppler GPL).
- **File Viewer's Space preview as a window of its own**, attached to File
  Viewer, with Open in… for full viewing and editing.
- **File Viewer's history**: the mouse's back button (BTN_SIDE) goes back
  as in a browser; going back gives each folder its state (scroll,
  selection) again; forgotten when the session ends.
- **The dock's minimized apps look ambiguous**: a design to choose.
- *Done 2026-11-18: every right-click menu is the server's on the native road (bus v20 Menu): the terminal's (menu::server_lines), a text area's and a field's (vergence-widgets ui.rs ServerMenu, MENU_ID_BASE; a chosen line comes back as a command and is done as a click on the old menu was); the drawn menus stay for the Wayland road. vergence-ui Frame::native says which road.* **The Text Editor's right-click menu not the same kind as the terminal's**
  (Dave: "Why?"): there were three kinds, the server's (File Viewer), the
  terminal's own, and the toolkit's.
- *Done 2026-11-18: Tab takes the suggestion and a space; Space takes it too where the word typed is no word of its own (not in the dictionary, not written whole nearby: "in" stays "in"); Escape lets it go (textarea.rs take_ghost, known_word).* **Completion accepted with Space** (as macOS), the space kept; Tab adds
  a space too. A word wanted as typed must stay possible (Escape, or a
  click on the typed word, as macOS).

- *Done 2026-11-18: vergence-shell appmenu.rs State caret/anchor and the field's keys (Left/Right in the text, Shift to select, Home/End, Delete, Ctrl+Backspace, Ctrl+A, typing over a selection), a click places the caret and a drag selects; Ctrl+C/X/V through the server's own clipboard (native.rs ClipOwner::Server, server_copy, server_paste); Escape clears first.* **The app menu's (and launcher's) search field a real field** (Dave: "I'd like it to be real not just a picture with input").
- **Edit and FileView** are the apps' names (Dave, 2026-11-18; Text Editor and File Viewer before).

## Questions for Dave (when each app's turn comes)

1. **Spellchecking**: dictionaries are data from other projects (Hunspell's
   dictionaries, under several licenses); our own checker reading them, or
   another source?
2. **PDF**: rendering PDF well is large; write our own (a long project) or
   borrow a renderer (Poppler, GPL; PDFium, BSD)?
3. **Video**: decoding needs codecs (FFmpeg, LGPL/GPL; or the GPU's own
   decoders through VA-API on Linux); which, and how far?
4. **Remote file systems**: the system's own (sshfs, NFS, SMB mounts) and
   the file manager shows them, or our own SFTP client?
5. **The browser**: WebKit's embeddable port (WPE WebKit, no GTK) would fit a
   native shell; uBlock Origin is a WebExtension, which WebKit's ports do not
   run outside Apple's, so blocking would be WebKit's content rules (as
   uBlock Origin Lite does on Safari). Worth it, and when?
6. **Agentic apps**: generated code runs as the user; a sandbox and a
   permission model first, or a trusted-agent-only start?
