# Editor milestone 1: text editing in the toolkit, the Text Editor, spellchecking

**Drafted 2026-11-14**, item 4 of docs/apps-roadmap.md.

**Accepted 2026-11-15** (Dave): "1 plain text now - other formats later md
rtf etc" (plain text; Markdown, RTF and others in a later milestone), "2 a"
(the Open/Save dialog is built now, as the first part of item 5), "3 ship
our own plus try and do the auto-complete like text-edit does" (a word list
in the repo, so checking works without a system package; completion also
offers dictionary words, the way TextEdit does), "4 a" (Find: case and
whole words, no regular expressions yet), "5 a now b later maybe" (non-UTF-8
files open read-only), "6 a" (the order as planned).

Dave's words: "a native text editor for Vergence. Like TextEdit on Mac or
GNOME Edit. Autocomplete and line numbers required." Spellchecking: "all
native apps and maybe others via extensions".

## Where things stand

- **The toolkit's only text input is the one-line field** (vergence-widgets
  ui.rs `FieldState`): the text, the caret and anchor as byte offsets, a
  horizontal shift, character stops; moving by character and word
  (`prev_word`/`next_word`), Shift to select, double-click for a word,
  triple-click for everything, the clipboard. **No undo**, no lines, no
  vertical scrolling, no input method.
- **vergence-text** lays out plain text in one face with glyph fallback
  (`Fonts::layout`, `advance`, `raster`). No shaping (ligatures, combining
  marks drawn as they come), no right-to-left text, no line breaking.
- **The app framework is ready for the editor**: command roles already
  include New window/tab, Open…, Save, Save as…, Print…, Undo, Redo, Cut,
  Copy, Paste, Select all, Find…, Find next, Replace…, zoom and tabs
  (vergence-protocol commands.rs). It also has the app button and its
  menu, each app's settings window (prefs.rs), About, one process for all
  of an app's windows with tabs in the server-drawn bar (as the terminal
  does), window memory, and the close question (vergence-term confirm.rs).
- **Open/Save dialogs and file types are item 5** of the roadmap. The
  editor is the first app that needs them, so this milestone has to settle
  how it opens and saves files: the dialog is built now (Dave, "2 a").

## Design

### 1. A text area in the toolkit (vergence-widgets `textarea.rs`)

This is the core every later text-handling app needs (mail, notes, the
code navigator), so it lives in the toolkit, not in the editor.

- **The buffer** (`vergence-widgets::text::Buffer`, our own): text kept as
  lines, with a byte-offset ↔ (line, column) mapping and edits as
  insert/delete ranges. A piece table or gap buffer replaces the plain
  `Vec<String>` only if measurements on large files call for it (target:
  a 10 MB, 200,000-line file opens in under a second and types without
  lag).
- **Undo and redo**: every edit recorded as its inverse. Typing groups into
  one step until a pause (1 s), a caret jump or a change between typing
  and deleting. Undo puts the caret and selection back where they were.
  Saving marks the point the document is "clean" at. **The one-line field
  gains undo too**, from the same code.
- **Caret and selection**: the field's moves plus Up/Down (keeping the
  column one meant), Home/End (line), Ctrl+Home/End (document), Page
  Up/Down, Shift for each, double-click word, triple-click line,
  click-drag and Shift+click. Word boundaries are shared with the field.
  A block selection (Alt+drag) is left for later.
- **Drawing only what shows**: lines are laid out as they scroll into view
  and kept in a small cache, so the size of the file does not slow typing
  or scrolling. Wrapping at the window's edge is on by default and can be
  turned off (horizontal scrolling then).
- **Line numbers**: a gutter in the window's secondary text color, the
  current line's number in the primary color, wrapped lines numbered
  once. The current line has a faint highlight (can be turned off).
- **Tabs and indenting**: tab width (default 4), Tab inserts spaces or a
  tab (default: what the file already uses; spaces for a new file). Enter
  keeps the line's indent. Tab/Shift+Tab indent and outdent the selected
  lines.
- **Find and replace**: a bar at the window's top (under the server's
  bar), not a dialog. Matches are highlighted as you type, with "3 of 17".
  Options: match case, whole words. Replace and Replace all; Replace all
  is a single undo step. No regular expressions yet (Dave, "4 a").
- **Meaning (Contract 3)**: the area is an editable text with its caret and
  selection in the meaning tree, so `vctl tree` and later a screen reader
  can read it.
- **Not in this milestone**: shaping and right-to-left (vergence-text's
  limits stand; noted for "rendering to publishing standards" on the
  wishlist), input methods for CJK (needs a text-input protocol on both
  roads), syntax colors.

### 2. Autocomplete, the TextEdit way

macOS TextEdit completes in two ways, and the editor does both (Dave: "try
and do the auto-complete like text-edit does"):

- **Inline completion while typing**: after two letters and a short pause
  (250 ms), the most likely end of the word is drawn after the caret in the
  secondary text color. Tab takes it; any other key goes on as if it were
  not there. It never takes a word by itself.
- **The completion list** (Option+Escape or F5 on the Mac; here Alt+Escape
  and F5, also Ctrl+Space): a list under the caret of every word that starts
  with what is typed (at most 12 shown, the list scrolls), the best one
  selected; Up/Down move, Enter or Tab takes it, Escape closes it, typing
  goes on narrowing it.
- **Where the words come from**: the words of three or more letters in all
  the editor's open documents first (ranked by how near the caret and how
  often), then the dictionary's words (ranked by how common they are: the
  word list's size levels, below). Languages (keywords, a language server)
  come later.
- In the toolkit as a service the text area asks (`Completer`), so other
  apps can supply their own words later. Off in the one-line field by
  default.
- Both ways can be turned off in the editor's settings.

### 3. Spellchecking, a toolkit service

- **One checker for every native text field and area**: words not in the
  dictionary get a wavy red underline after the word is finished, never
  while it is being typed. A right click on one gives up to five
  suggestions, "Add to dictionary" and "Ignore" (for this document).
- **Our own checker and our own copy of the words** (Dave: "ship our
  own"): the word list lives in the repo (data/words/en_US.txt), installed
  with the apps, so checking works on every machine without a package.
  It is made from SCOWL (Spell Checker Oriented Word Lists, Kevin
  Atkinson's; permissive, MIT-like terms), sizes up to 60 (the size
  Hunspell's en_US uses), American spelling, with every inflected form
  already written out, so no affix rules are needed. Each word keeps its
  SCOWL size level (10 = most common … 60), which ranks suggestions and
  completions. A small tool (tools/make-words.py) rebuilds the list from
  SCOWL's release; the list, its source and terms go in
  THIRD-PARTY-NOTICES.md and docs/borrowed-review.md (data, not code).
  Other languages later, from the same kind of lists.
- **Suggestions**: words within two edits (letters swapped, missing,
  extra, wrong), with common typing slips (a key's neighbors) and the
  size level weighing in; the best five shown.
- **The user's own words**: ~/.local/share/vergence/words.txt (0600, one
  word per line), shared by every app.
- **The language** follows the desktop's locale. A setting on the settings
  app's Keyboard (or a new Language) page chooses another or turns
  checking off for the whole desktop. Each app's settings window can turn
  it off for that app, under the usual inherit-or-override rule.
- **Others "via extensions"**: foreign apps (Firefox, GTK, Qt) check with
  their own spellers. Offering ours to them would mean a D-Bus service
  they do not ask for. Left for later; the checker is a library
  (`vergence-spell`) so it can be served later without rewriting.

### 4. The Open and Save dialog (vergence-widgets `files.rs`)

The first part of roadmap item 5, built now (Dave: "2 a"); file types and
the rest of item 5 come with the Viewer and the File Manager.

- **A dialog window of the asking app** (the window styles milestone's
  dialog kind, attached to its window), drawn by the toolkit, so every
  native app gets the same one by calling `open_file`/`save_file`.
- **Sidebar of places**: Home, Desktop, Documents, Downloads (those that
  exist), the root, mounted disks, and recent folders; the user's own
  places later with the file manager.
- **The folder's list**: name, size and date, folders first, sorted by any
  heading; type to jump to a name; hidden files on Ctrl+H; Enter opens;
  double-click opens. Icons by kind (folder, text, image, other) until
  file types exist.
- **A path line** above the list: the folder as clickable parts; typing a
  path (with `~` and Tab completion) goes there.
- **Save**: a name field with the name selected without its extension, the
  folder chosen in the same list, a question before replacing a file.
- **Filters**: an app gives the kinds it opens ("Text files", "All files");
  the editor opens anything and warns only for files that are not text.
- **Remembers** the last folder per app and the dialog's size
  (~/.local/state/vergence/settings-windows.toml's neighbor, 0600).

### 5. The Text Editor app (`vergence-edit`)

- **A plain-text editor**, like GNOME Text Editor (Dave: "plain text now
  - other formats later md rtf etc").
- **Windows and tabs**: one process for all of its windows, a document per
  tab in the server-drawn bar (as the terminal's tabs), tabs dragged
  between windows. A tab shows the file's name, with a dot while it has
  unsaved changes.
- **Files**: UTF-8, with the file's line endings (LF or CRLF) and final
  newline kept as found. A file that is not valid UTF-8 opens read-only
  with a notice (Dave, "5 a now b later maybe"). Save writes a
  temporary file next to the original and renames it over the original,
  so a crash never leaves half a file; the file's permissions are kept.
  If the file changed on disk while open: a bar offers Reload or Keep
  mine.
- **Unsaved work**: closing a tab, a window or the app with unsaved
  changes asks first (Save / Don't save / Cancel, the close question's
  pattern). Untitled documents and unsaved changes are kept across a
  session restart in ~/.local/state/vergence/edit/ (0600), the way Mac
  and GNOME editors do. Saved files are not touched.
- **The command line**: `vergence-edit [FILE…]` opens the files in tabs of
  one window (in the running process if there is one). A desktop entry
  for text/plain, so the dock and launcher can open files with it once
  file types exist (item 5).
- **Status line** at the window's foot: line and column, the selection's
  length, tab width/spaces, line endings, the language for spellchecking.
  Each item can be clicked to change it.
- **The app menu** (server's app button): featured New tab, Open…, Save,
  Find…; pins; Text size; Settings…; Keyboard shortcuts; About; Quit.
  All commands come from the existing roles; "Go to line…" (Ctrl+L) and
  "Show line numbers" are the editor's own.
- **Settings window** (prefs.rs rows): Font (monospace default; any face
  the system has), Text size, Line numbers, Highlight current line, Wrap
  lines, Tab width, Insert spaces, Autocomplete, Spellcheck (inherit the
  desktop's or override), Style (the app's light/dark under the master
  switch).
- **Print**: left out unless Dave wants it now (there is no print path in
  Vergence yet).

## Progress

- **Step 1** (83144f4): vergence-widgets edit.rs, the buffer and the undo
  history; the one-line field undoes too.
- **Step 2** (f3adf65): textarea.rs, the text area; `--example varea`.
- **Step 3** (cc77515): vergence-edit, the editor's window. Found on the
  way: a column's `padding` is ignored (only panels and buttons pad); the
  toolkit read a program's title only at open (the widgets host now sends
  it when it changes); vergence-ui gives a window its number before its
  waker, and one waker serves all of a process's windows.
- **Dave's notes, 2026-11-15** (ad7081b): the line numbers on a gray of
  their own; a word count in the status line; tabs with unsaved edits
  colored, by the server for every app (bus v14 `Tab::edited`, an amber
  stripe and tint). The editor's name is Dave's to give (it shows as
  "Text Editor" until then).
- **Step 4**: vergence-widgets filedialog.rs, the Open and Save dialog
  (places, the path as buttons, Back and Up, sorting, hidden files,
  typing to narrow or a path, a text file's first lines at the side, New
  folder, the replace question, the last folder per app and the recent
  files in ~/.local/state/vergence/file-dialog); Element::keep_in_view;
  the editor's Open…, Save as… and Save of a new document (the name
  suggested from its first words, as TextEdit does). Server fixes found
  with it: a dialog of the focused window takes the keyboard (opened.rs
  judged a dialog by an identity dialogs do not have), and the keyboard
  goes back to a closed window's parent or the window in front (native.rs;
  it went nowhere). Still to come: the Save name chosen without its
  extension (the toolkit cannot set a field's selection yet), choosing
  several files at once.
- **Step 5**: Find and Replace. The text area finds (`Doc::find_all`: case
  or not, folded beyond ASCII, whole words; within a line, at most 10,000),
  lights the matches (`set_marks`, let go at any edit) and replaces them
  all as one undo (`replace_all`). The editor's find bar under the bar:
  Ctrl+F, Ctrl+H with Replace, searching as you type from where the caret
  was, "3 of 30", ‹ › and Ctrl+G / Ctrl+Shift+G, Aa, Whole words, Replace
  (the chosen match, then the next) and Replace all ("Replaced 12"),
  Escape closes and gives the keyboard back to the text.

## Steps

1. **Buffer and undo** in vergence-widgets, with tests (edits, the offset
   mapping, undo grouping, a large file's timings); the one-line field
   moved onto the shared undo.
2. **The text area**: caret, selection, keys, mouse, visible-lines-only
   layout and drawing, wrapping, scrolling, line numbers, indenting; an
   example (`--example varea`) to try it by itself.
3. **The Text Editor's first window**: open from the command line, edit,
   save (atomic), the close question, tabs and windows, the status line,
   command roles, the settings window, the desktop entry and icon,
   install.sh.
4. **The Open and Save dialog** in vergence-widgets (`--example vfiles`),
   then the editor's Open…, Save as… and the recent files.
5. **Find and replace** bar.
6. **Autocomplete**.
7. **Spellchecking**: the word list (tools/make-words.py, data/words),
   `vergence-spell` (lookup, suggestions, the user's words, size levels
   for ranking) with tests; then
   underlines and the right-click menu in the field and the area; the
   desktop setting and each app's override.
8. **Keeping unsaved work across restarts**, reload-on-change, Go to line.
9. **Docs**: whitepaper (the toolkit's text area and spellchecking, the
   editor), README, CLAUDE.md, apps-roadmap, the Vergence Map.

Each step is tested on Hubble, kepler and Techbench and pushed.

## Acceptance

1. A 10 MB, 200,000-line file opens in under a second and typing,
   scrolling and selecting stay smooth (no frame over 16 ms on Hubble).
2. Line numbers show and follow wrapping; turning wrapping off scrolls
   sideways.
3. Undo and redo bring back every edit with its caret, in sensible steps;
   the one-line fields in the settings app undo too.
4. Inline completion shows a word's likely end in gray and Tab takes it;
   Alt+Escape/F5 lists the words from open documents and the dictionary;
   neither ever takes a word by itself.
5. Find highlights every match as you type; Replace all is one undo.
6. Open… and Save as… use the toolkit's dialog: places, sorting, typed
   paths, the replace question; the last folder is remembered.
7. Misspelled words get underlined in the editor and in a settings-app
   field; the right-click suggestions are sensible; an added word is no
   longer underlined in any app.
8. A crash or `kill -9` during a save never leaves a damaged file; closing
   with unsaved changes asks; untitled work survives a session restart.
9. Works the same on Hubble (Void), kepler (FreeBSD) and Techbench
   (Fedora).

## Questions for Dave (answered 2026-11-15, see the header)

1. **Plain text or rich text?** (a) Plain text, like GNOME Text Editor:
   what this plan describes, and a good base for code. (b) Rich text, like
   TextEdit: fonts, bold, sizes, saved as RTF or similar: a much larger
   project (rich layout and an RTF reader and writer of our own). Lean
   (a), with rich text left for a later milestone if wanted.
2. **Opening and saving files before item 5's dialogs**: (a) build the
   Open/Save dialog now as the first part of item 5 (a native file
   chooser in the toolkit: places sidebar, folder list, path field,
   filter by type), and the editor uses it. This makes the milestone
   bigger, but every later app has the dialog. (b) For now, a path field
   with completion in a small dialog, and the real chooser waits for
   item 5. Lean (a).
3. **Where the spelling dictionaries come from**: (a) our own checker
   reading the Hunspell dictionaries the system already has (Void
   `hunspell-en_US`, FreeBSD `en-hunspell`, Fedora `hunspell-en`;
   install.sh names the package, like NVK). We ship no word lists, so
   their licenses (several, mostly LGPL/MPL/BSD-like) stay with the OS
   packages. (b) Ship a word list of our own in the repo (SCOWL's lists
   are permissive), so it works without a package. (c) Spellchecking
   later, not in this milestone. Lean (a). Either way the checker is our
   code; no Hunspell library.
4. **Regular expressions in Find**: (a) not now: case and whole words
   only. (b) Yes, with our own small regex engine (a few hundred lines
   for the common syntax). (c) Yes, borrowing the `regex` crate
   (MIT/Apache, a borrowing per the rule). Lean (a) now, (b) later.
5. **Files that are not UTF-8** (Latin-1, UTF-16): (a) open read-only with
   a notice. (b) Guess and convert, and save back in the same encoding.
   Lean (a).
6. **Order**: (a) as above: the text area, the editor, find, autocomplete,
   then spellchecking. (b) Spellchecking earlier, so every settings field
   gets it sooner. Lean (a).
