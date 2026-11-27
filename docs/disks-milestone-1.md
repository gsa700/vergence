# Disks milestone 1: the disk utility

*Drafted 2026-11-26 on techbench from the apps roadmap's item 7 (Dave's
list, 2026-11-14: "useful tool all desktops should have"; the roadmap:
"disks, partitions, file systems, ZFS pools, SMART; changing anything needs
root, so a small root helper of our own"). It builds on two plans already
answered: elevation milestone 1 (the server-drawn prompt, vdm doing one
named action) and hardware readings milestone 1 (a read lease for root-only
readings, opened from Advanced). Nothing here is built yet. **Draft: the
questions at the end are for Dave.***

## Why

Every desktop has one (macOS Disk Utility, GNOME Disks, KDE Partition
Manager, Windows' Disk Management), and today on Vergence the answers are
terminal commands: `lsblk`, `smartctl`, `zpool status`, `mkfs`. The common
jobs are few: *what disks do I have, how full and how healthy are they, and
make this stick usable* (erase it and give it one volume that other
computers can read). Those are milestone 1. The rare and dangerous ones
(resizing, moving partitions, the system disk's layout) are the installer's
or later.

## What this machine says, and who may read it (techbench, 2026-11-26)

| Reading | Where (Linux) | Who may read it |
|---|---|---|
| Disks, their model, firmware, size, rotational or not, removable | `/sys/block/*`, `device/model`, `firmware_rev` | any user |
| Serial number | `/sys/block/*/device/serial` | any user (question 4) |
| Partitions, start and size | `/sys/block/<disk>/<part>/start`, `size` | any user |
| Partition scheme, type, file system, label, UUID | udev's database, `/run/udev/data/b<maj>:<min>` (`ID_PART_ENTRY_SCHEME`, `ID_FS_TYPE`, `ID_FS_LABEL`, `ID_FS_UUID`) | any user (eudev on Void writes it; the kernel's and blkid's parse, not ours) |
| Where each is mounted, how full | `/proc/mounts`, `statvfs` | any user |
| Temperature (NVMe) | the drive's hwmon, `/sys/class/nvme/nvme0/hwmon0` (Composite, Sensor 1, 2) | any user (Hardware shows it) |
| Health: SMART / NVMe health log (wear, spare, power-on hours, errors, unsafe shutdowns) | NVMe admin command on `/dev/nvme0` (0600 root); ATA SMART through SG_IO on `/dev/sdX` (0660 root:disk) | **root only** |
| The raw device (the partition table's bytes) | `/dev/nvme0n1` (0660 root:disk) | root and the disk group (Dave is not in it) |

FreeBSD (kepler; from the system's documentation, to be checked there):
GEOM's whole picture is the sysctl `kern.geom.confxml` (any user: disks,
partitions, labels, types, sizes); `zpool list -Hp` and `zpool status` work
for any user; health (`nvmecontrol logpage`, `camcontrol` for ATA) needs
root (`/dev/nvme0` is root:operator 0600).

So everything but health can be shown to anyone, from what the kernel and
udev (or GEOM) have already parsed. **Viewing never reads a disk's bytes:**
a stick someone hands you is untrusted, and its partition table is parsed
by the kernel, not by us, and never by us as root.

## What milestone 1 does

**Seeing (any user, no prompt):**

- A sidebar of disks, grouped: *Internal*, *External* (USB, cards),
  *Disk images* (loop and md devices), *Pools* (ZFS). Each with its model
  in words ("Samsung SSD 980 PRO", "SanDisk Ultra"), size, how it is
  connected (NVMe, SATA, USB).
- A disk's page: a **bar of its partitions to scale** (as GNOME Disks and
  macOS draw it), each part labeled (name, file system, size), free space
  shown as free; under it the chosen partition's facts: file system,
  label, UUID, mounted where, used of size, the partition's type in words
  ("EFI system", "Linux file system", "Microsoft basic data").
- **Used space** per volume as a bar; Btrfs and ZFS say so where `statvfs`
  is not the whole truth (a pool's free space is the pool's).
- **Temperature** where the drive gives it (NVMe hwmon; SATA drivetemp if
  loaded), the same reading as Hardware's.
- **Mount, Unmount, Eject** for removable drives through the server's
  existing drive actions (drives milestone 1, bus v34): no new code path.
- Kept current: the server's `Drives` topic says when drives come and go;
  the page reads sysfs and udev again then.

**Health (Advanced, asks once per window, as Hardware's readings):**

- An **Advanced** part on each disk's page, closed each time: "These
  readings need an administrator's password" and **Show**. On a yes, a
  read lease (the connection-held lease of hardware readings milestone 1,
  its read-lease class in vdm's action table) covers every disk in that
  window until it closes or the screen locks.
- vdm sends the drive's own health command itself (NVMe Get Log Page 02h,
  SMART / Health Information; ATA SMART READ DATA and the thresholds
  through SG_IO on Linux, CAM on FreeBSD), parses the fixed-size answer in
  its forked worker (bounds-checked, fuzzed: the bytes come from the
  drive's firmware), and returns named fields only.
- Shown in words first, numbers under: **"Healthy"**, **"Wearing: 12% of
  rated writes used"**, **"Failing: replace this drive"** (the drive's own
  critical-warning bits or a SMART attribute past its threshold), then
  power-on hours, data written, available spare, media errors, unsafe
  shutdowns, temperature history if the drive keeps it.
- Never: a self-test started (later, question 6), a raw log handed out,
  the drive's serial or WWN added beyond what sysfs already shows.

**Changing (each one asks, every time; elevation's rule, no lease):**

- **Erase a removable drive** ("Erase…", as macOS's word): a sheet first
  in the app saying what will be lost (each volume's name, file system and
  how full it is), choosing the format and a name; then the prompt in
  vdm's words, naming the drive so a person can be sure:
  "Erase SanDisk Ultra 32 GB (USB) and make one exFAT volume named BACKUP".
  - Formats offered: **exFAT** (reads everywhere: Mac, Windows, Linux,
    FreeBSD; the lean default), **FAT32** (old cameras, car stereos,
    firmware updates; up to 4 GB a file), **ext4** (Linux only), **UFS**
    (FreeBSD only); on each system only those it can mount.
  - One partition, GPT (MBR for FAT32 when asked "for older devices").
- **Rename a volume** (its label), removable or not, while unmounted.
- **Check a file system** read-only (fsck's no-change mode, `fsck_msdosfs
  -n`, `btrfs check --readonly` on an unmounted one) and **Scrub** a mounted
  Btrfs or ZFS pool (start, and its progress while it runs): reads
  everything, changes nothing, but needs root, so it asks.

**vdm's checks before any change** (in vdm, on the device it opens, not on
the program's word): the device is removable or was plugged in at runtime
(question 2); nothing on it is mounted, swapped on, an open LUKS or GELI
volume, a pool member, a RAID or LVM member, or the disk holding `/`,
`/boot` or the EFI partition; its size and model still match what the
prompt said (a stick swapped between the sheet and the yes is refused).

## How it works

1. **A new app**, `vergence-disks` (question 1 for its name), on the
   toolkit: sidebar, the partition bar (a new drawn widget, shared with
   the installer later), the facts as Hardware's lines.
2. **A small reading library** (`vergence-disks` crate's lib, or a module
   of vergence-sysinfo): disks and partitions from sysfs and udev's
   database on Linux, `kern.geom.confxml` on FreeBSD (our own small XML
   reader for its fixed shape; the sysctl is the kernel's, the text is
   trusted as much as the kernel), mounts and usage, ZFS from `zpool list
   -Hp` / `zpool status -p` run as the person (the system's own tool, its
   output parsed; question 5).
3. **vdm's new actions**:
   - read-lease class: `disks.health` (a lease, then `health <device>` per
     disk while it lasts);
   - changing class, each asking: `disks.erase <device> <format> <label>
     <scheme> <expect-size> <expect-model>`, `disks.rename <partition>
     <label>`, `disks.check <partition>`, `disks.scrub <fs> start|stop`.
   - The device is named by its kernel name only ("sdb", "da0"), checked
     against `/sys/block` or GEOM, never a path the program builds.
4. **How vdm erases** (question 3): the partition table written by our own
   code (GPT: a protective MBR, two headers and the entry arrays with their
   CRC32s; small and well specified, and the installer needs it anyway);
   the file system by our own writer for exFAT and FAT32 (both small,
   fixed layouts), by the system's own `mkfs.ext4` / `newfs` with an
   argument list vdm builds from fixed parts (never a string from the
   program) for ext4 and UFS. The old signatures wiped first (the first
   and last megabyte, as wipefs does) so nothing stale is found later.
   Then the kernel told to read the table again (BLKRRPART / GEOM's own
   retaste), and the drive left unmounted; the server's automounter
   mounts it as for any stick.
5. **Logged** as every elevate request: who, which program, what (device,
   model, size, format), yes or no, the outcome; a lease at its start and
   end.
6. **Hardware links to it**: Hardware's Disks page gets "More in Disks",
   as Settings > About links to Hardware.

What it never does in milestone 1: resize, move or delete one partition
among others, touch an internal disk's layout, create pools or RAID,
encrypt, run a self-test, or read a disk's bytes for viewing.

## Threats, and what answers each

- **Erasing the wrong disk** (the worst outcome): the internal and system
  disks refused by vdm's own checks; the prompt names model, size and bus;
  the size and model checked again at the yes; nothing mounted or in use.
- **A program erasing silently**: every change asks (elevation's rule 3,
  no lease for changes; the read-lease class holds no changing action, and
  its test says so).
- **A hostile drive** (a stick's firmware answering a health command with
  crafted bytes): fixed-size answers, bounds-checked, parsed in vdm's
  forked worker, fuzzed; the partition table is never parsed by us for
  viewing; erasing writes and never reads it.
- **Shelling out** (`mkfs.ext4`, `newfs`, `zpool`): fixed program paths,
  argument lists built from vdm's own constants and checked values, no
  shell, a clean environment; the label limited to the format's own rules
  (exFAT 11 characters, FAT 11 upper-case, ext4 16 bytes) and to printable
  characters.
- **Someone over ssh, or another account**: elevation's rule; only a person
  at the machine's console gets the prompt or the lease.
- **A health lease used to fingerprint**: no more than sysfs already
  gives any user (question 4); no raw logs.

## Steps

1. The reading library and the app, seeing only (Linux sysfs + udev, then
   FreeBSD GEOM on kepler); the partition bar; Mount, Unmount, Eject
   through the server; "More in Disks" in Hardware. **No root, no vdm:
   can be built as soon as the plan is answered.**
2. ZFS and Btrfs on their pages (pools, members, usage, scrub state as any
   user can read it).
3. vdm: `disks.health` in the read-lease class (rides on hardware readings
   milestone 1's lease, so after its step 3), the NVMe and ATA parsers in
   the worker, fuzzed; the Advanced part on each disk.
4. vdm: `disks.erase`, `disks.rename`, `disks.check`, `disks.scrub` with
   their checks; the GPT writer and the exFAT and FAT32 writers, each
   tested by reading the result back with the system's own tools
   (`fsck.exfat`, `fsck_msdosfs`, `gpart show`, `sfdisk -d`) on disk images,
   never a real disk in tests.
5. Tests on all three machines (erasing only loop and md disk images and a
   lab USB stick); then **an independent review** of the vdm parts (Dave's
   code security policy: a new root interface, and a destructive one).

## Questions (with leans)

1. **Its name.** *Lean: "Disks"* (as GNOME's; short, as Edit, FileView,
   Reader, Hardware). Or "Disk Utility" (macOS's).
2. **Which disks can be changed in milestone 1:** (a) removable and
   hot-plugged ones only, the internal disks seen but never changed (lean:
   the jobs people need are sticks and external drives; the internal
   layout is the installer's); (b) also a second internal disk that holds
   nothing mounted (a spare SATA drive), with a stronger warning.
3. **How vdm erases:** (a) our own GPT, exFAT and FAT32 writers, the
   system's `mkfs.ext4` / `newfs` for ext4 and UFS (lean: the first three
   are small and fixed, and the installer will need GPT; ext4 and UFS are
   large and the system ships their tools); (b) the system's tools for all
   (`sgdisk`/`sfdisk`, `gpart`, `mkfs.exfat`, `newfs_msdos`), which Void
   may not have installed (exfatprogs is a separate package); (c) udisks2's
   Format over D-Bus on Linux (it asks through polkit, which would bypass
   our server-drawn prompt and is not on FreeBSD: lean against).
4. **Serial numbers:** any user can already read them from sysfs on Linux.
   (a) Show them (warranty claims, telling two identical drives apart),
   with Copy (lean); (b) leave them out, as hardware readings milestone 1
   keeps SMBIOS serials out.
5. **ZFS:** (a) read with `zpool` as the person (lean: the system's own
   tool, read-only, already allowed to anyone); (b) through libzfs's
   ioctls ourselves (unstable across ZFS versions: against).
6. **Later, not in milestone 1** (agree?): SMART self-tests; partitions
   added, deleted or resized among others; pools created; LUKS or GELI
   encryption of a stick ("Erase… encrypted"); disk images made from a
   drive and written back (a USB installer written from an ISO, as Fedora
   Media Writer does: likely wanted soon, its own small step); a
   benchmark.
7. **Order:** this after AlbumWall (0.4.0) and Remote (0.5.0), with step 3
   waiting on hardware readings milestone 1's vdm lease? *Lean: yes; steps
   1 and 2 need nothing from vdm and could go earlier if a gap opens.*

## Progress

- **Step 1, seeing, started 2026-11-26 on techbench** (Dave: "please try and
  make the utility work there [kepler] as well as we can"; the plan's
  questions not yet answered, so built on the leans that touch it: the
  name Disks, serial numbers shown). crates/vergence-disks: the library
  (lib.rs; linux.rs from sysfs, udev's database, /proc/mounts and
  /proc/swaps; geom.rs, a small reader of our own for
  `kern.geom.confxml`'s fixed shape, with LABEL's other names and SWAP's
  consumers; `zpool list -Hp` and `zpool status -p` as the person) and the
  window (main.rs: the sidebar by Internal, External, Disk images, Pools;
  a disk's page with its partition bar to scale, the chosen partition's
  facts and how full it is, Open in FileView and Eject for external
  disks through the toolkit's drive actions; a pool's page with health,
  usage, members, last scrub, errors). `vergence-disks --list` prints
  what the window shows, for checking a machine over ssh. Tests from
  kepler's own GEOM picture and zpool status (serial and UUIDs replaced),
  a FreeBSD stick with no table, a mirror's status, udev records, mount
  tables, the bar's gaps. Desktop entry (Disks), icon, install.sh's lists,
  the launcher's order after Hardware.
  - **techbench** (Void): the Samsung 980 PRO, EFI (FAT32, /boot/efi),
    swap in use, Btrfs on /, 37 °C; window opened in Dave's session.
  - **kepler** (FreeBSD 15.1): the Micron 2210, EFI (gpt/efiboot0 on
    /boot/efi), FreeBSD boot, swap in use, ZFS partition as part of pool
    zroot; the pool healthy, never scrubbed. Firmware is not readable
    there without root (camcontrol/nvmecontrol), so it is not shown.
    Not yet seen on its screen (locked while built).
  - Not yet: a USB stick and a disk image on each, Hubble, the display
    suite, a package.
