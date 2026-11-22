# Drives milestone 1: one drive list, kept by the server

Status: **answered 2026-11-21** (Dave: "yes to all three, go ahead":
internal file systems shown, plugged-in drives not yet mounted shown, the
server's one "Safe to remove"); all steps in the same evening (see
Progress), installed 2026-11-21; **Dave: "seriously broken"** with a USB
stick and the Fedora ISO together on techbench. Open issues below. Decided (Dave, 2026-11-21): "2, and restart the desktop on
install". The second half was done the same evening: the server keeps the
desktop's icons running, and tools/install.sh restarts them.

## Why

The desktop's icons (`vergence-files --desktop`) and FileView disagreed
about a USB stick on techbench. They are two processes. Each reads the mount
table and the automounter's /media on its own timer, and the desktop ran
the program of the session's start while FileView ran the new one. The Open
dialog and the dock would be a third and fourth opinion. One list, kept in
one place and pushed to everyone, is how Vergence already does monitors,
settings and input devices: the bus topics `Outputs`, `Settings`, `Inputs`.

## What it does

1. **The server keeps the drive list** (crates/vergence-server/src/drives.rs),
   from what it watches already or can:
   - removable drives: udisks2's objects on Linux (the server's automounter
     thread watches them now); on FreeBSD the automounter's /media names
     whose device is there, with devd's notices for da, cd and md devices
     coming and going (the server reads devd for monitors already);
   - what is mounted where drives are (/media, /mnt, /run/media), disk
     images (md and loop devices) and network mounts, from the mount table:
     looked at when devd or udisks says something changed, and every few
     seconds besides (a mount by hand says nothing);
   - each drive: its name and label, kind (drive, card, optical, disk image,
     network), device, where it is mounted (if it is), whether it can be
     ejected, and whether it was ejected and is still plugged in (FreeBSD:
     hidden, as since 2026-11-21; the note kept by the server now).
2. **A bus topic, `Drives`** (bus v34): the whole list on subscribe, again
   whenever it changes. vergence-protocol `drive.rs`; `vctl drives`.
3. **Actions through the server** (bus v34 too): Eject, Mount (a drive the
   automounter lists but has not mounted), Mount Disk Image. The server does
   them (vergence-media, as the apps do now) and tells once: the one "Safe to
   remove" notice comes from the server, never from each app; a disk image
   put away says nothing.
4. **Everyone shows that list:** FileView's Locations and its eject buttons
   (and whether the folder shown can be ejected), the desktop's drive icons,
   the Open/Save dialog's sidebar. Outside a Vergence session (no bus) they
   read the mount table themselves, as now.

## Steps

1. The server's model and watching (drives.rs, devd and udisks notices, the
   mount table), `vctl drives`; tests on parsed tables and notices.
2. Bus v34: `Topic::Drives`, the list's wire form, fuzzed with the rest;
   vergence-widgets subscribes once per process and keeps it for files.rs.
3. Actions: Eject, Mount, Mount Disk Image as bus requests; the server's
   notices; the ejected note moves into the server.
4. FileView, the desktop and the Open dialog on the topic (files.rs's own
   reading kept as the fallback).
5. Checks: a display-suite scenario (a disk image mounted in a headless
   session on Linux: FileView and a second client see it at once, eject
   from one, gone from both); a USB stick on techbench by hand; tests on
   Hubble and techbench.

## Not in it

A drive tile on the dock (later, with live tiles), connecting to servers
(files milestone 1's question 7), formatting and partitioning (the disk
utility on the wish list).

## Questions for Dave

1. **Internal disks.** Show other internal file systems mounted for the user
   (a data disk at /mnt/data, say) under Locations, as Explorer does, or
   only removable drives, images and network mounts, as Finder does by
   default? Claude's lean: show them, without an eject button unless they
   can be ejected.
2. **Plugged in, not mounted.** On FreeBSD the automounter lists a stick's
   name before it is mounted, and mounts it when first looked into. Keep
   showing such drives (a click mounts it), as now? Claude's lean: yes. Linux
   mounts them at once, so it is the same there.
3. **The notice.** "Safe to remove" from the server alone, once, whichever
   app ejected (Claude's lean), or none at all, as macOS (the drive just
   leaves the sidebar)?

## Progress

All five steps in 2026-11-21. Tests on Hubble and techbench, the display
suite 17 of 17 on both (scenario 17 new), agent-check 0 failed.

- **Steps 1-3** (044d2f5): vergence-media drives.rs holds the mount table,
  the disks and network mounts, the automounter's /media, the ejected
  sticks (moved from vergence-widgets files.rs) and `scan()`, which gives
  each drive its kind: removable (FreeBSD by driver, da, cd, mmcsd and sdda,
  and label nodes; Linux by sysfs, on USB or marked removable), optical,
  image (md, loop), network, internal (not ejectable). The server's
  drives.rs: a thread scanning every 2 s and when poked (the plan's devd and
  udisks notices became the poll and the pokes after actions: two seconds
  at most, and nothing a mount by hand can slip past), the list to the
  `Drives` topic's subscribers; Eject, Mount and Mount Disk Image on a
  worker, refused for a path the list does not have (an image's file
  excepted: vergence-media and the FreeBSD helper check it); the one "Safe
  to remove" for a drive or a disc. `vctl drives`, `vbus drive`.
- **Step 4:** vergence-widgets follows the topic on a thread of its own
  from the first ask (`files::drive_list`, `drives_seen`); `mounted_disks`,
  `network_mounts`, `ejectable`, `is_image_mount` read it; `eject` and
  `mount_image` go through the server in a Vergence session (Ok(true): the
  server told the user, the app says nothing). FileView redraws its
  sidebar when a new list comes, and a tab on a drive that went away goes
  home; the desktop's timer compares as before. Outside a session, all as
  read here.
- **Step 5:** display-suite scenario 17 (tools/display-suite/17-drives.sh):
  an ISO written by `vergence-harness iso` (our own ISO 9660 writer, so no
  xorriso or makefs is needed) mounted through the server, in its list and
  in FileView's Locations with an eject button; that button lets it go,
  gone from the system, the list and the sidebar. Passes on Hubble (udisks)
  and techbench (the helper).

## Open (Dave's pass, 2026-11-21 evening, techbench)

Fixed that evening: an ejected stick remounted by FileView looking into its
folder, a stick pulled out while mounted kept listed (37e9baf); an ejected
stick not showing when plugged in again, devfs having reused its node's
number, and rescans on devd's disk events (810c570).

Fixed later that night (Dave and Claude together on techbench, a recorder
logging the list, mounts, labels and /media each second; Dave: "Worked
perfectly"):

1. **A mounted ISO brought phantom drives** (a9aacb3, aaa5e2b): labels whose
   provider is an md device or its partition (`glabel status -s`) are not
   drives; the server ran glabel by name and the session's PATH has no
   /sbin, so it is called by its full path (umount too, e563529).
2. **Clicking the stick "dumped" Dave at Home** (0f3b73e): FileView's watch
   sent a tab on a /media folder not listed as mounted home, which is also
   a stick just clicked and mounting; it now only skips reading it again.
3. **The stick opened empty** (e572446): autofs keeps an answer that mounted
   nothing (its folder looked into while the stick was out) for
   vfs.autofs.cache seconds, 600 by default, and `automount -c` clears only
   /media itself. install.sh sets the cache to 5 s (sysctl.conf), and the
   toolkit does not look into a /media folder whose drive is not listed.
4. **One stick, two names** (a0ef549): looked up by its device name, the
   automounter lists /media/da0s1 beside /media/USB; one entry per GEOM
   provider, the mounted one, else the one by its label.
5. **Three "Safe to remove" and the stick still there** (cd75869): the
   helper only unmounted, so the next look mounted it again. Eject now
   unmounts every partition of the disk and runs `camcontrol eject` (da,
   cd, sdda): its partitions and labels leave /dev until plugged in again.
6. **A disk image mounted opened a tab** (292d850): it shows under
   Locations only (Dave: "A new tab should never appear unless the user
   calls it").

Still open:

- **The delay when a stick is plugged in:** devd pokes a rescan (810c570);
  Dave to say whether it is short enough.
- The automounter keeps every name it has served in /media until a reboot;
  techbench's /media/da0s1 (from the night's debugging) shows for a moment
  while the stick mounts. Harmless; gone at the next reboot.
- Hubble has not been given this night's build yet; the display suite on
  both machines before the milestone closes.
