# Backup milestone 1: Vergence's own Backup, and File Viewer's right-click menu

*Drafted 2026-11-16, rewritten the same evening (Dave: "I don't want to
set up backups to the nas, I want to work on the backup features built
into vergence"). The apps roadmap's item 6 ("Backup, then into the file
manager: ZFS snapshots on FreeBSD (and Linux with ZFS), zfs send to remote
hosts over SSH, rsync where there is no ZFS; a file's versions from its
snapshots in the file manager"), Btrfs from Dave's list of 2026-11-15, and
File Viewer's right-click menu from his list of 2026-11-16, which touches
the same app. A feature of Vergence for anyone who uses it; the three
machines are where it is tried, and nothing of the lab's own backups
(NASBOX, Techbench's nightly script) is touched.*

## What it is for

The Mac's Time Machine is the model people know: turn it on, pick a disk,
forget it, and get any file back as it was. Vergence's version, built on
what each system already has (principle 8: the system's tools run, never
rewritten):

1. **Versions on the machine itself**, where the file system keeps them
   cheaply: ZFS snapshots (FreeBSD, Linux with ZFS), Btrfs snapshots
   (Fedora's default). Taken on a schedule, kept by rules, pruned by
   themselves. A system without either (ext4, UFS) says so plainly and
   relies on the copy below.
2. **A copy somewhere else**: an external disk, or another machine over
   SSH. On a plain file system the copy is a dated folder per run, files
   that did not change hard-linked to the previous run (rsync
   `--link-dest`, as Time Machine does), so every run is a whole,
   browsable tree that takes only the changed files' room, readable
   without Vergence. Later: `zfs send` / `btrfs send` when both ends speak it.
3. **Getting things back**: File Viewer's **Earlier Versions…** on a file
   or folder lists its versions by time from both (local snapshots and the
   copy, when it is reachable), opens one read-only, or restores it beside
   the original (never over it without asking).

**The Backup app**: on or off, where the copy goes, what is included (and
what never is: caches, trash, credentials by default), the schedule, the
last run and its log, Back Up Now. **The Backup docket** shows the last
good backup and turns amber when one is overdue or failed, so a backup
that stopped is never silent.

**Root**: snapshots of a person's own data need root on both systems (or a
one-time ZFS delegation, `zfs allow`); turning them on goes through the
elevation prompt as a named vdm action, so the app itself never runs as
root. Copies run as the person.

## File Viewer's right-click menu

Dave: "a blend of macOS and Windows with Linux flavoring". On an item:
Open, Open With ▸, (Show in Folder in search results); Cut, Copy, Paste,
Duplicate, Make Link; Rename, Move to Trash; Compress, Get Info;
**Earlier Versions…**; Copy Path, Open in Terminal (the Linux flavor). On
empty ground: New Folder, New File, Paste, Sort By ▸, Show Hidden, Open
in Terminal, Get Info (of the folder). To be checked against what files
milestone 1's menus already hold.

## Steps (rough)

1. File Viewer's menu (the quick, visible part).
2. The Backup engine (a library: the copy with `--link-dest`, include and
   exclude rules, a run's record) and the app: a disk or an SSH
   destination, Back Up Now, the log; the docket.
3. Local snapshots: ZFS and Btrfs back ends, the vdm actions and their
   elevation, the schedule and the pruning rules.
4. Earlier Versions in File Viewer, from snapshots and from the copy.
5. Running on schedule (question 4); tests (the engine against a scratch
   tree; each back end where its file system is: kepler's ZFS, Techbench's
   Btrfs, Hubble's ext4 copy-only); an independent review.

## Questions

1. **Where a copy can go in this first milestone:** (a) an external disk
   and another machine over SSH; (b) an external disk only, SSH later.
2. **What is backed up by default:** (a) the whole home folder, minus
   caches, trash, browser caches and credentials; (b) chosen folders
   (Documents, Pictures, Music, Videos, Desktop).
3. **Local versions by default, where the file system allows:** (a) every
   hour kept a day, every day kept two weeks; (b) daily only; (c) off
   until turned on.
4. **When no one is logged in:** (a) scheduled backups run anyway (a
   system timer set up once through elevation); (b) only while the
   person is logged in.
