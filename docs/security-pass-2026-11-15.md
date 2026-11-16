# Security pass, 15 November 2026 (scoped)

Dave asked for it on the evening files milestone 1 closed: a pass over the
trust boundaries added since the 8 November pass, not the full audit (that
one still belongs before anyone else runs the desktop or the repository goes
public). Done on Hubble by Claude (Fable 5.2), 21:05 to 21:40 CST. **Report
only: nothing in the tree was changed.** Control makes the fixes.

## What was looked at, and what was not

Tree: Hubble's `~/src/vergence`, a copy without git, bus v18, newest file
15:27 on 15 November. It is **behind HEAD c905bc6**: it has no
`vergence-thumbs`, no `vergence-media`, and its `install.sh` has no autofs
or eject-helper part. Those were not reviewed (see "Not covered").

Scope, five areas:

1. code that runs as root: `sound-default`, `tools/install.sh`;
2. untrusted files: File Viewer, the desktop, Close-Up, `vergence-decode`,
   `vergence-types`;
3. other programs driving ours: the portal, the bus requests added up to
   v18 (Open, Activate, ContextMenu, drag and drop, the clipboard);
4. commands made from text: desktop entries, the default-apps lists,
   `sh -c` sites;
5. the session's own plumbing: `tools/vergence-session`, the runtime
   directory, the hand-over sockets, file modes.

Method: read the code; check the live session on Hubble (the running
server's descriptors, its children's, socket and file modes); one live
test (finding 5); the whole test suite and the long fuzz runs.

## Findings

Severity is by the project's own rules (whitepaper §7.3), on the 8 November
scale. "Same uid" findings are marked: today any program running as the
user can do a great deal anyway, so they matter as rules the design means
to keep, not as holes someone outside can use.

| # | Severity | Where | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | Medium | server `native.rs` `native_paste` (about line 1038) | **Any bus client can read the clipboard at any time.** `native_copy` checks that the client has the keyboard; `native_paste` checks nothing, and `WinRequest::Paste` needs no window. §7.3: "Read only on the user's paste or by the focused app". Wayland clients are held to that by Smithay; the bus is the way round it. Same uid. | The same focus check as `native_copy` (the client owns the keyboard window), refused otherwise. A test beside it. |
| 2 | Medium | `vergence-decode` `limit_self` (lib.rs about 283) | **On Linux the decode helper is not confined.** It has resource limits and no-new-privileges only (the crate's comment says Landlock and seccomp are later). A decoder taken over by a bad picture can still open files (the limit of 8 descriptors leaves 5), connect to the session's sockets (the control socket where it is on), unlink and rename files, and signal the user's processes. FreeBSD is fine (Capsicum). The desktop and previews decode files without being asked, and thumbnails (not in this tree) do it for every picture in a folder, a USB stick's included. | A seccomp allow-list after the file is read (read, write, mmap, munmap, mremap, brk, futex, sigreturn, exit, exit_group and what the allocator needs), or Landlock with no paths. Until then: try `RLIMIT_NOFILE` 3 on both systems (no new descriptor at all). |
| 3 | Low | same | The results of `setrlimit` and `prctl` are ignored: if one fails the helper decodes with no limit and says nothing. Only `cap_enter` is checked. | Exit 3 on any failure, as for `cap_enter`. |
| 4 | Medium | server `wallpaper.rs` (about 456 to 464), `shelf.rs` (about 98 to 116); settings `thumbs.rs:52` | **The compositor decodes pictures in its own process**: the wallpaper (`image::open`; SVG through usvg with `resources_dir` set, so a wallpaper SVG can pull in other files beside it) and dock and launcher icons (resvg, `image`), whose paths come from desktop entries' `Icon=`. The server sees every key and every window; it is the last place to parse a downloaded picture. A panic in a decoder there ends the session. (Listed as "not covered" on 8 November.) | Send both through `vergence-decode` (it already does SVG with no outside resources), answers cached. Settings' read of its own cache is the least of the three. |
| 5 | Medium | `vergence-types` `of_file` (lib.rs about 215) and its callers | **Opening a named pipe hangs the opener; in the server it freezes the screen.** `of_file` opens whatever the path names and reads its head; a FIFO with no writer never returns. Confirmed on Hubble: `vergence-open --type` on a FIFO hung until stopped. The server calls it on its own thread from the bus's `Open`, the portal's `OpenURI` and a drop on a dock icon (`open_items`, shelf.rs about 1444), so one message from any program of the user, or one file dragged to the dock, stops the display. The same kind of open is in `vergence_decode::decode_file` (lib.rs 95), the editor's `load` (a device file reports length 0, so the 256 MB limit does not hold and `fs::read` does not end) and `copy_tree`. | One shared "open a regular file" (open with `O_NONBLOCK`, `fstat`, refuse what is not a regular file) used by all four; the server asks for a type off its own thread or by name only. |
| 6 | Medium where others have accounts, Low on Dave's machines | `tools/vergence-session` (about line 47) | **The made-up runtime directory can be someone else's.** With no `XDG_RUNTIME_DIR` (a console start, Void, kepler; Hubble runs this way now: `/tmp/xdg-1000`) the script does `mkdir -p /tmp/xdg-UID && chmod 700`. If another account made that directory first, `mkdir -p` succeeds, `chmod` fails, and the script goes on and exports it. Every socket of the session (Wayland, bus, D-Bus edge, control, hand-over) is then in a directory another user owns and can swap files in. | After making it, check it: a real directory (not a link), owned by us, mode 700; if not, `mktemp -d`. The server should make the same check at start and refuse a runtime directory that is not its user's. |
| 7 | Low | File Viewer `desktop.rs` (about 366 to 372, 479, 590) | **The launcher trust mark is the file's execute bit, and the question shows the launcher's own name.** A launcher copied from an archive or an ext4 stick (our `copy_tree` keeps modes) arrives already "trusted" and starts with no question. Where the question is asked, it names the entry's `Name=`, which the file chooses ("Report.pdf"), not the command it would run. | Show the program from `Exec=` and the file's real name in the question. Keep trust in a list of our own (path and a hash of the content), so a copy or a changed file asks again. |
| 8 | Low, same uid | `hand.rs` in vergence-files, -edit, -closeup; vergence-term `program.rs` (about 178 to 215) | **The four hand-over sockets** check no peer uid (the server's three do), read a line with no limit before testing its length, and serve one connection at a time, so one stalled client blocks the rest. The terminal's takes a command to run. Four near-copies of one file. | One shared module (vergence-widgets or vergence-protocol): peer uid check, `take()` on the read, a total time limit; the four use it. |
| 9 | Low, same uid | server `native.rs` (about 509), `opened.rs` | `WinRequest::ContextMenu` opens a server menu at any time and at any offset: no check that a button was pressed on that window (drag start has that check) and no clamp to the window. `Activate` and a new window's "same program" rule rest on an identity the client declares, so a program that claims the focused app's name passes the focus rule. The second is 8 November's finding 5 with a consequence. | ContextMenu: require the press, clamp the point. Identity: the pid and executable the server sees (now recorded on the bus) beside the declared name. |
| 10 | Low | File Viewer `ops.rs` | `copy_tree`: keeps setuid and setgid bits (`cp` drops them); recursion with no depth limit; checks with `symlink_metadata` and then opens by path, so in a folder others can write a file can be swapped for a link in between. `restore`: believes `Path=` in the info file, makes any folders it names, and takes a relative path from the current directory. | Mask modes with 0o777; a depth limit; `O_NOFOLLOW`; in `restore`, absolute paths only and no making of folders outside the home without asking. |
| 11 | Low | server `portal.rs` `portal_choose` | The caller's title, button word, file name and filters go to the dialog with no limit on length or number (the D-Bus message limit of 8 MiB is the only one). Otherwise sound: no shell, each value its own argument, names with `/` refused by the dialog, no descriptors. | Cap title and word (say 200 characters), name (255), filters (64). |
| 12 | Low | `~/.config/vergence`, `tools/vergence-session` | On Hubble the settings folder is 0775, `desktop.toml` 0644, `settings.toml` 0664, `server.log` 0644 (in a 0700 folder). CLAUDE.md's rule is owner-only. `desktop.toml` holds commands the server runs with `sh -c` (menu entries, the sleep command). Also: every program the server starts inherits `server.log` as its output, so any app writes into the server's log. | `umask 077` in `vergence-session`; make the settings folder 0700 when written; the server warns when `desktop.toml` is writable by others; started programs get `/dev/null` or a log of their own. |
| 13 | Info | vergence-net `wpa.rs` (about 121 to 150) | Seven stale `vergence-wpa-<pid>-<n>` sockets in Hubble's runtime directory (a killed client does not run `Drop`); with no runtime directory the socket goes in `/tmp` under a name that can be guessed. | Remove stale ones at start; no `/tmp` fallback. |
| 14 | Info | Hubble | The control socket is on in Hubble's installed session (`~/.config/vergence/control-socket`, made 9 November). By design and Dave's choice on a development machine; said here so it is known. | None. |

## Still open from 8 November

- Control socket served with blocking I/O on the server's thread (finding 4 there).
- `std::env::set_var` after threads exist: was 3 places, now about 7
  (`portal.rs` `export_env`, `cursor.rs`, `state.rs` DISPLAY, `bus.rs`,
  `dbus.rs`, `main.rs`). The fix suggested then (a map applied in
  `spawn()`) is worth more now.
- `vergence-dbus` `Message::big` is still a public field.
- Identity (finding 5 there): step 1 is partly done, the bus records the
  peer's pid; nothing uses it yet. See finding 9 here.

## What the reading confirmed

- `sound-default` (setuid root, FreeBSD): one argument of one or two
  digits, a device that exists and plays, one sysctl written, the
  environment cleared by the caller, nothing opened. Sound.
- `install.sh`: no temporary-file or quoting problem found. Two things to
  know, not faults: it installs binaries built in the user's checkout as
  root's (one of them setuid), and the `wheel` group may drive Wi-Fi.
- Programs the server starts do not inherit its devices or sockets:
  checked live on Hubble (File Viewer, Firefox, Terminal share none of
  the server's input, DRM, tty or socket descriptors).
- Desktop entries' `Exec=` is split by our own parser and run without a
  shell; files are passed as whole paths, so a file name cannot become an
  option. A link's scheme must begin with a letter, so a link cannot
  either.
- Files are opened by type, never run: a script or a desktop entry in a
  folder opens in an app. Only the desktop's launchers start programs
  (finding 7).
- Drag and drop: a drag starts only with a button held on the source
  window; the data goes only to the window dropped on, at the drop.
- `native_copy` checks the keyboard; the bus refuses other uids; frames
  and counts are capped (`MAX_OPEN`, `MAX_DRAG_BYTES`, and the rest).
- Decode helper, caller's side: the answer is capped (1 GiB of pixels,
  500 frames), a 5 s limit, a crash or a hang is an error to the caller.
- Editor: saves beside and renames, keeps modes; its kept session is
  0700/0600. Close-Up reads at most 4 MB of text. The Trash's info files
  are made new, names do not clash.
- OSC 52 in the terminal: copy only, a read request is ignored.

## Tests

- `cargo test --workspace` on Hubble: 71 suites, 615 tests, none failed.
- Long fuzz runs (release): vergence-protocol and vergence-dbus at
  1 000 000 cases per test, vergence-term at 100 000 streams: no panics
  (39 s in all).
- No fuzz test yet for: `vergence-types` (`of_bytes`, `MimeApps::parse`,
  `path_of_link`), the Trash's info files, the hand-over line, the decode
  answer (`parse`), **our own BMP decoder** (`vergence-decode/src/bmp.rs`,
  the one decoder that is ours), the portal's options, wpa_supplicant's
  replies.

## Not covered

- **`vergence-thumbs` and `vergence-media`, the FreeBSD autofs map and the
  eject helper (root), the udisks2 client**: not in this tree. They are
  root code and untrusted input, the two kinds this pass was for; they
  need the same look on Control. Questions to ask of them: what the root
  helper accepts and whether it can be made to unmount or eject something
  else; mount options for sticks (nosuid, nodev, and noexec or showexec);
  whether a stick's thumbnails are decoded before the person opens it;
  what the shared thumbnail cache believes from files already in it.
- The dependencies: 339 packages in `Cargo.lock`; `cargo audit` is not
  installed here and was not run.
- Smithay's handlers, Xwayland, the settings app, vergence-net's parsing,
  vergence-spell, the D-Bus edge beyond the portal, the toolkit as a
  client.

## Order to fix in

1. Findings 1 and 5: small, and each is a rule of ours broken today.
2. Finding 6: a few lines of shell and one check in the server.
3. Findings 2 to 4: the decoding boundary, as one piece of work (Linux
   confinement, checked limits, wallpaper and icons through the helper).
4. Findings 7 to 12 as their files are next touched.
5. The same pass over the code not in this tree; `cargo audit`; the fuzz
   tests listed above.

A full audit stays where the 8 November report put it: before someone
other than Dave runs the desktop, or before the repository goes public.

---

## Fixed on Control, 15 November (evening)

| # | Commit | What was done |
|---|---|---|
| 1 | e03908e | `native_paste` refused (`Denied`) unless the client's window has the keyboard; one `native_has_keyboard` check for copy and paste. |
| 5 | e03908e | `vergence_types::open_regular` / `read_regular` (open with `O_NONBLOCK`, `O_NOCTTY`; `fstat`; regular files only), used by `of_file` (a pipe or device is a binary file, never opened), `decode_file`, the editor's `load`, Close-Up's text, `copy_tree` (pipes, devices, sockets not copied), the thumbnail cache and the wallpaper cache. `vergence-open --type` on a FIFO returns at once on Hubble and kepler. |
| 6 | 6678331 | `vergence-session` makes `/tmp/xdg-<uid>` with `mkdir -m 700` and checks it (a directory, not a link, ours), else uses a fresh `mktemp -d`; tested with dash and FreeBSD sh. The server refuses a runtime directory that is unset, a link, not ours, or open to others. |
| 2, 3 | f282f82 | Linux: a seccomp allow-list after the file is read (x86_64; another processor is refused, not left unconfined). `RLIMIT_NOFILE` 3 on both systems. Every limit checked: exit 3 before decoding if one cannot be set. Tests run the real helper on PNG, JPEG, WebP, animated GIF, BMP and SVG, and `--check-confined` shows an open after the limits is refused (SIGSYS on Linux, exit 4 on FreeBSD). 28 real pictures on Hubble, 8K JPEGs among them, decode under the filter. |
| 4 | f282f82 | Wallpaper (pictures and SVG) and dock and launcher icons decoded by the helper; the server only resizes, and its `image` dependency has no decoders. Settings' preview cache read through the helper. |
| 12 | f282f82 | Narrow, Dave's choice (no session-wide `umask`): at start the server makes `~/.config/vergence` and `~/.local/state/vergence` 0700 and the files directly in them owner-only, and warns if `desktop.toml` can be changed by another user. The started programs' output going into `server.log` is still open. |

## Control's pass: thumbnails, removable drives, the eject helper

The code Hubble's tree did not have, reviewed on Control at c905bc6 plus
the fixes above. The questions the report asked, answered:

- **What the root helper accepts.** `media-eject` (FreeBSD, setuid root,
  group video, 4750) takes one argument, `/media/<name>` with a plain name
  (letters, digits, `._- `; no `/`, no `..`), checks it is a mount point in
  `mount -p` now, and runs `/sbin/umount` on it with an empty environment.
  It cannot be made to unmount anything outside `/media`, and only root (the
  automounter) can mount there (`vfs.usermount` is 0). Any member of video
  may eject any drive under `/media`, someone else's on a shared machine
  included: by design for one seat. Sound.
- **Mount options for sticks.** FreeBSD: `-nosuid,noatime,autoro` from
  `auto_master`, FAT files 664 with no execute bit; other file systems keep
  their own modes, so a UFS or ext2 stick can carry programs that run (not
  setuid). Linux: udisks2's defaults, `nosuid,nodev`, and `showexec` for
  FAT. Neither has `noexec`; whether sticks should is Dave's question (see
  below).
- **Whether a stick's thumbnails are decoded before it is opened.** No.
  Thumbnails are asked for only by File Viewer's icon view of a folder (and
  the desktop's own files); plugging a stick in shows its icon, nothing
  inside it is read.
- **What the shared thumbnail cache believes.** A cached PNG is read with
  `read_regular` (16 MB at most), its text chunks walked by our own code
  without inflating anything, and its pixels decoded by the helper. A
  thumbnail another program of the user planted with the right `Thumb::URI`
  and `Thumb::MTime` is shown: the spec's trust, same uid.

| # | Severity | Where | Finding | Done |
|---|---|---|---|---|
| C1 | Medium on FreeBSD machines with an unmounted internal FAT partition, none on kepler | `tools/freebsd/media-map` | **Internal partitions could show in /media, FAT ones writable by video.** FreeBSD's `special_media` offers every GEOM provider nobody has open, internal disks' included: an EFI partition not mounted, another system's partitions on a dual boot. Our map then mounted FAT ones with `-g=video,-m=664`, so the video group could write the EFI partition. kepler is not affected (its EFI partition is mounted, the boot partition has no file system). | Fixed: the map passes on only removable devices (`da`, `mmcsd`, `sdda`, `cd`), checked on each entry's device, whether asked by device name or by label; tested on kepler with a stand-in `special_media`. A machine whose internal disks are `da` (SAS) would need the list changed. |
| C2 | Low, same uid | `vergence-thumbs` `write_private` | The temporary file had one name per process (two workers could write the same one) and was opened with `create` and `truncate`, following a link left in its place. | Fixed: a name per write, `create_new` (never an existing file or a link), removed on failure. |
| C3 | Low (robustness) | `vergence-thumbs` `Thumbnailer`, File Viewer | Every thumbnail made stays in memory for the life of File Viewer, and the icon view asks for a whole folder's pictures, not those on screen: a folder of 10 000 photos holds about 2 GB. | Open: ask only for what is on screen, and bound what is kept. |
| C4 | Info | FreeBSD `special_media` (base system) | It writes a drive's label with an unquoted `echo ${_label}`, so a label holding `*` is expanded as a file name pattern (in automountd's working directory, as root). Only the listed name changes; no command runs. Upstream's. | Not ours to change; noted. |
| C5 | Info | `vergence-dbus` client, `vergence-media` udisks | Signals are taken from the system bus by a match on udisks's name (the bus enforces the sender); any other signal only causes a fresh look at udisks's objects. Mounting asks udisks with `auth.no_user_interaction`; drives udisks marks as system or ignored are skipped. Sound. | — |

## Dependencies

`cargo audit` (cargo-audit on Hubble, RustSec database of 15 November,
1509 advisories) over the 341 packages in `Cargo.lock`: **no
vulnerabilities**. Two warnings, both unmaintained crates that parse fonts:
`rustybuzz` 0.20.1 (RUSTSEC-2026-0206) and `ttf-parser` 0.25.1
(RUSTSEC-2026-0192), with no fixed release. Both come in through usvg's text
support (and ttf-parser also through ab_glyph). usvg now runs only in the
confined decode helper: the server's last use of resvg (its canvas) is
tiny-skia used directly. The server still links vergence-decode's library
(for `decode_file`), which depends on usvg and the image decoders, but never
calls them: `nm` on the release server finds none of their code (one stray
generic `RawVec` helper of usvg's). To watch: a usvg release that drops them, or our own text through
vergence-text in the helper.

## Still open after the evening

- Findings 7 to 11 and 13 (launcher trust, the hand-over sockets,
  ContextMenu and identity, `copy_tree`/`restore`, portal caps, stale wpa
  sockets), and 12's second half (started programs write into
  `server.log`).
- C3 (thumbnails kept in memory).
- Mount options: `noexec` for sticks on both systems, if Dave wants it.
- Fuzz tests listed under "Tests" above.
- From 8 November: the control socket's blocking I/O, `set_var` after
  threads, `Message::big`, identity by pid.
