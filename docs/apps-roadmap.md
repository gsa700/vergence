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
4. **Text editing in the toolkit, then the Text Editor** (plan:
   docs/editor-milestone-1.md): line numbers,
   autocomplete (words of the open documents first; languages later), find
   and replace, tabs; **Spellchecking** with it, as a toolkit service every
   native text field can use.
5. **Open/Save dialogs and file types, then the Viewer** (pictures first,
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
- **The dock's icons by drag and drop**, while the dock itself is moved
  "with its menu or a modifier and drag".
- **The dark title bar's color chosen**, "maybe something like the accent
  color picker".

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
