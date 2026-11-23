# Archive milestone 1: archives in FileView, extracted and made

Status: **drafted 2026-11-23, waiting for Dave's answers.** Queued
2026-11-22 (Dave: "queue it after the terminal fixes"; STATUS Next #1).
Dave, 2026-11-23: "i installed zip and unzip but it would be nice if we
didn't have too": so this milestone also *makes* archives, and needs no
outside program to do either.

## What changes

1. **Double-click an archive** (.zip, .tar and its compressed kinds, .7z,
   .rar, and the rest libarchive reads) and FileView extracts it beside
   itself, into a folder named after it, with progress as for a copy (the
   same bar and Stop). The folder is selected when it is done.
2. **"Extract here"** in an archive's menu, and **"Extract to..."** asking
   for a folder (the toolkit's file dialog). Several archives selected:
   each into its own folder.
3. **"Compress"** in the menu of any selection: one `.zip` beside the
   items ("Archive.zip" for several, the item's name for one), so the
   person needs no `zip` program. Compressing is undone as a copy is (the
   new file goes to the Trash).
4. **Nothing written outside the folder it extracts into**, whatever the
   archive says (the rules below).
5. **No outside program**: FileView uses libarchive, the library behind
   bsdtar, already on every machine Vergence runs on (Void's package
   manager xbps is built on it; FreeBSD has it in its base system).

## How it stays safe

The archive is untrusted input, as a picture or a PDF is, and gets the same
treatment (whitepaper §6.10): its formats are read in a confined helper,
**`vergence-unpack`** in libexec, beside vergence-decode and the PDF helper.

- FileView opens the archive and hands the helper the open file as its
  standard input (seekable, so .zip and .7z work), as `decode_file` does.
- The helper reads it through libarchive, then confines itself before it
  reads a byte of the archive: Capsicum on FreeBSD, seccomp on Linux, the
  shared `vergence_decode::confine` with its limits (CPU time, memory). No
  file system, no network: it can only read the archive and write to its
  pipe.
- It writes a simple stream of our own to standard output: each entry's
  kind, cleaned name, mode, time and contents.
- **FileView writes the files**, from that stream, into a new hidden folder
  beside the archive, and renames it into place at the end (so a stopped or
  failed extract leaves no half folder behind: the hidden one is removed).
  Our own code decides every path:
  - names with `..`, absolute paths and drive letters are refused (the
    entry is skipped and counted);
  - links are kept only when they point inside the folder; others are
    skipped; nothing is ever written *through* a link;
  - setuid, setgid and sticky bits dropped; owners never set;
  - device files and pipes skipped.
- **Caps against archive bombs**: the helper stops past a total size or an
  entry count (Question 4), and FileView checks again as it writes.

Making a .zip (step 5) reads the person's own files, not untrusted input:
FileView writes it through libarchive directly, on its worker thread, with
progress.

## Borrowing (needs Dave's yes)

On 2026-11-22 the plan named **bsdtar** (the program). This draft uses
**libarchive** (the library bsdtar is a thin front end to) through our own
small FFI, instead of running bsdtar. Why:
- bsdtar writes the files itself, so its own path checks decide what is
  written; with the library our code decides.
- A program run from the helper cannot be confined on FreeBSD (Capsicum
  ends at `exec`), and seccomp on Linux would have to allow everything a
  program needs to start. A library call fits the helper we already have.
- It is the same code, already installed: nothing new comes in.

What our own would take: readers for zip (deflate), tar, gzip, bzip2, xz,
zstd, 7z (LZMA) and rar, months of work, and the formats' corner cases are
where the security bugs live. Lean: libarchive, listed in
docs/borrowed-review.md and THIRD-PARTY-NOTICES.md (BSD 2-clause).

Packaging: the Void template gains `libarchive-devel` in makedepends (the
library itself is already there with xbps); FreeBSD needs nothing.

## Questions (with leans)

1. **Libarchive through FFI instead of the bsdtar program** (above)?
   *Lean: yes.*
2. **An archive holding one folder at its top** (most do: `project-1.2/`):
   extract that folder as it is, without wrapping it in another folder of
   the same name (as macOS's Archive Utility does)? Loose files at the top
   still go into a folder named after the archive. *Lean: yes.*
3. **After extracting, the archive** stays where it was (macOS can move it
   to the Trash; Windows leaves it). *Lean: it stays; a setting later if
   wanted.*
4. **The caps**: stop and ask ("This archive would unpack to 210 GB.
   Extract anyway?") past **20 GB** in total, **100,000** entries, or a
   ratio over **1,000 to 1** once past 1 GB. *Lean: these numbers, with
   "Extract anyway".*
5. **Password-protected archives** (zip, 7z): milestone 1 says "This
   archive needs a password" and stops; asking for it comes later. *Lean:
   yes, later.*
6. **Compress**: `.zip` only (what everyone can open), or a choice with
   `.tar.zst`/`.tar.xz` too? *Lean: .zip in the menu; the others later, if
   wanted.*
7. **.iso and disk images** stay as they are (mounted on double-click,
   files milestone 3), not extracted, though libarchive can read an .iso.
   *Lean: yes.*
8. **Later, not now**: an archive browsed as a folder without extracting,
   and Space previewing its list of files. *Lean: next archive milestone.*

## Steps

1. The FFI: the few libarchive calls used (read: open on a descriptor,
   next header, read data, the entry's name, mode, size, time, link target;
   write: zip with deflate), in a new crate `vergence-archive`, with unit
   tests on small archives made in the test.
2. `vergence-unpack`: the helper, confined before reading, its stream and
   its caps; tests as vergence-decode's (the real helper, its limits on, a
   file system call killed).
3. FileView's writer: the stream into a hidden folder, the path rules, the
   rename into place, progress and Stop; tests with hostile archives (`..`,
   absolute names, links out, a link then a file through it, a bomb, a
   setuid file, a device).
4. FileView: double-click, Extract here, Extract to..., several at once,
   one-folder-at-the-top, the ask past a cap, the messages (needs a
   password, not an archive, damaged).
5. Compress to .zip, with undo.
6. Packaging (`libarchive-devel`), borrowed-review and notices, docs;
   tests on Hubble, techbench and kepler; a display-suite scenario
   (extract, see the folder); an independent review (untrusted input).
