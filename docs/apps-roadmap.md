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
- **The terminal's settings window a little larger**, so all of it shows
  without scrolling. Small.
- **A terminal setting to scroll on output** (jump to the bottom when a
  program writes, or stay where you scrolled to). Small.
- **Terminal programs as default apps** (nano, vi, emacs, ed for text
  files). Opening already runs a desktop entry with `Terminal=true` in our
  terminal (`vergence-term -e`); what is missing is that nano and the others
  ship no desktop entry on Fedora or Void. Settings > Apps would offer the
  terminal editors found on the system and write a small entry of our own
  for the chosen one (`vergence-term-nano.desktop`, `Terminal=true`) plus
  the line in vergence-mimeapps.list.
- **The Text Editor's right-click menu as full as the terminal's.** The text
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

- **Only one System Settings at a time** (Dave: "there should only ever
  be one copy of System Settings at any one time"). Starting
  vergence-settings while one is running (a dock tile's click, `--page`,
  `--app`, `--forget`, an app menu's Settings…) brings the running window
  forward and turns it to the page asked for, instead of opening a second
  window. Seen when the clock tile's click opened a new one (dock milestone
  1, step 2).

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
