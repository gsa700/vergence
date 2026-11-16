# Backup milestone 1: Backup, and File Viewer's right-click menu

*Drafted 2026-11-16 (Dave: "now what about that file manager and backup
work?"). The apps roadmap's item 6 ("Backup, then into the file manager:
ZFS snapshots on FreeBSD (and Linux with ZFS), zfs send to remote hosts
over SSH, rsync where there is no ZFS; a file's versions from its
snapshots in the file manager"), Btrfs from Dave's list of 2026-11-15, and
File Viewer's right-click menu from his list of 2026-11-16, which touches
the same app.*

## What is there today (looked up 2026-11-16)

- **kepler** (FreeBSD): ZFS, `zroot/home/derickson` its own dataset
  (10.8 GB). Snapshots need root, or a one-time `zfs allow` that lets the
  person snapshot their own dataset.
- **Techbench** (Fedora): Btrfs on LUKS, `/home` its own subvolume.
  Snapshots need root.
- **Hubble** (Void): ext4 `/home` (43 GB used): no snapshots possible.
- **NASBOX** (FreeBSD, the lab's NAS): the lab's backup system since
  2026-07-11: each workstation rsyncs to `/NAS/data/<Machine>` over SSH;
  NASBOX snapshots NAS/data daily (2 weeks), mirrors it to LTS/data and
  snapshots that in tiers (3 months, 12 months, 5 years). Techbench feeds
  it nightly from cron (`~/Documents/rsync_to_fBSD.sh`, 03:30: chosen
  folders and dot files, its own include list); kepler and Hubble do not.

So the lab already has the history and the second copy; what each
machine lacks is (a) a simple, visible way to feed NASBOX, and (b) on the
machines that can, quick local versions between those nightly copies.

## The proposal

**One Backup app (and a docket), three parts:**

1. **Local versions** where the file system has them: ZFS snapshots on
   kepler, Btrfs snapshots on Techbench, taken on a schedule (say hourly),
   kept by rules (say every hour for a day, every day for two weeks),
   pruned automatically. Hubble has none and says so. Taking and pruning
   them needs root: one elevation per machine when it is turned on (a
   `zfs allow` of snapshot/destroy on the person's own dataset on
   FreeBSD; on Linux a small named vdm action, `snapshot.home`, since
   Btrfs has no delegation).
2. **A copy to another machine**, NASBOX by default, in the lab's own
   layout (`/NAS/data/<Machine>/`) with rsync over SSH as the person,
   with the person's key, nightly or on a schedule, with an include and
   exclude list (Techbench's script's choices as the starting point:
   documents, pictures, music, video, the person's own scripts and
   settings, never credentials or browser caches). NASBOX's own
   snapshots and LTS tiers then give the long history, as today. Later:
   `zfs send` / `btrfs send` where the other end takes it.
3. **Earlier versions in File Viewer**: a file's or folder's versions from
   the local snapshots (`.zfs/snapshot`, the Btrfs snapshot directory),
   listed by time, opened read-only or restored beside the original (never
   over it without asking). NASBOX's versions come with "connecting to
   servers" (roadmap question 7), later.

**The Backup docket** says when the last copy and the last snapshot were
and turns amber when one is overdue or failed (the lab's rsync tile has
a known blind spot here: it showed green while a feed had stopped).

**Principle 8:** snapshots and SSH are the system's (`zfs`, `btrfs`,
`ssh`, `rsync` are run, never rewritten); Vergence schedules, shows and
restores. Scheduling: while the person is logged in, by the session;
the nightly copy also needs to run when no one is: a system timer (cron,
a systemd timer) that the app sets up through elevation (question 4).

## File Viewer's right-click menu

Dave: "a blend of macOS and Windows with Linux flavoring". On an item:
Open, Open With ▸, (Show in Folder in search results); Cut, Copy, Paste,
Duplicate, Make Link; Rename, Move to Trash; Compress, Get Info;
**Earlier Versions…** (part 3 above); Copy Path, Open in Terminal (the
Linux flavor). On empty ground: New Folder, New File, Paste, Sort By ▸,
Show Hidden, Open in Terminal, Get Info (of the folder). To be checked
against what files milestone 1's menus already hold.

## Steps (rough)

1. File Viewer's menu (the quick, visible part).
2. The Backup app's skeleton and the copy to NASBOX (rsync over SSH,
   lists, a run by hand, its log); the docket.
3. Local snapshots: ZFS on kepler, Btrfs on Techbench, the elevation
   to turn them on, schedule and pruning.
4. Earlier Versions in File Viewer.
5. Scheduling when no one is logged in (question 4); tests; a review.

## Questions

1. **What to copy:** (a) Techbench's script's choice of folders and dot
   files, for every machine; (b) the whole home folder minus caches,
   browser profiles and credentials; (c) each machine its own list.
2. **Where:** (a) NASBOX `/NAS/data/<Machine>/` as today; (b) somewhere
   else as well.
3. **Techbench's cron script:** (a) the Backup app takes its place (the
   script retired once the app's copy has run a week); (b) both run.
4. **When no one is logged in:** (a) the nightly copy runs anyway (a
   system timer, set up through elevation); (b) only while logged in.
5. **Local snapshots** (kepler, Techbench): (a) hourly, kept a day, daily
   kept two weeks; (b) another rhythm; (c) none, NASBOX's are enough.
