# Drives milestone 1: one drive list, kept by the server

Status: **answered 2026-11-21** (Dave: "yes to all three, go ahead":
internal file systems shown, plugged-in drives not yet mounted shown, the
server's one "Safe to remove"); building. Decided (Dave, 2026-11-21): "2, and restart the desktop on
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
