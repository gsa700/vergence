//! The disks, their partitions, file systems and pools, as any user may read them.
//!
//! Disks milestone 1, step 1 (docs/disks-milestone-1.md). Linux: sysfs (`/sys/block`), udev's
//! database (`/run/udev/data`), `/proc/mounts`, `/proc/swaps`. FreeBSD: GEOM's picture, the
//! sysctl `kern.geom.confxml`, and `mount -p`. Both: `df` for how full (vergence-sysinfo) and
//! `zpool` for ZFS pools, run as the person.
//!
//! Rule: **never read a disk's bytes.** A stick someone hands you is untrusted; its partition
//! table and file systems are parsed by the kernel and udev (or GEOM), never by us. Everything
//! here is text the system already wrote.
//!
//! Start reading: [`disks`] and [`pools`]; the pure parsers below them are what the tests check.

use std::collections::HashMap;

mod geom;
#[cfg(target_os = "linux")]
mod linux;

/// Where a disk lives, for the sidebar's groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Internal,
    External,
    Image,
}

/// One disk: what it is, how it is connected, and its partitions in order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Disk {
    /// The kernel's name: "nvme0n1", "sda", "nda0", "da0".
    pub name: String,
    pub model: String,
    pub serial: String,
    pub firmware: String,
    pub size: u64,
    /// "NVMe", "SATA", "USB", "Memory card", "Disk image", "Virtual".
    pub bus: String,
    pub kind: Option<Kind>,
    /// Spinning (true), solid state (false), unknown.
    pub rotational: Option<bool>,
    /// "GPT", "MBR", or empty (no table: a file system on the whole disk, or blank).
    pub scheme: String,
    pub parts: Vec<Part>,
    /// A file system on the whole disk, no table (a stick formatted bare, a disk image).
    pub whole: Option<Part>,
    pub temp: Option<f32>,
}

/// One partition (or a file system on a whole disk).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Part {
    pub name: String,
    pub index: u32,
    /// Bytes from the disk's start.
    pub start: u64,
    pub size: u64,
    /// The file system in words: "Btrfs", "FAT32", "exFAT", "ZFS", "Swap".
    pub fs: String,
    pub label: String,
    pub uuid: String,
    /// The partition's type in words: "EFI system", "Linux file system".
    pub ptype: String,
    /// Where it is mounted (the shortest path when several: Btrfs subvolumes).
    pub mount: Option<String>,
    /// (size, free) of the mounted file system.
    pub usage: Option<(u64, u64)>,
    /// What else holds it: "Swap, in use", "Part of ZFS pool zroot", "Encrypted, unlocked".
    pub role: String,
    /// Its other names, as mount tables may give them: "gpt/efiboot0", "nvd0p1" (FreeBSD).
    pub aliases: Vec<String>,
}

/// A segment of the partition bar: a partition, or free space between.
#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Part(usize),
    Free { start: u64, size: u64 },
}

impl Disk {
    /// The bar's segments in disk order, free gaps of at least 8 MiB shown (alignment, the
    /// first MiB and the backup GPT's room say nothing).
    pub fn segments(&self) -> Vec<Segment> {
        const MIB: u64 = 1 << 20;
        let mut out = Vec::new();
        let mut at = 0u64;
        let mut order: Vec<usize> = (0..self.parts.len()).collect();
        order.sort_by_key(|&i| self.parts[i].start);
        for i in order {
            let p = &self.parts[i];
            if p.start > at && p.start - at >= 8 * MIB {
                out.push(Segment::Free { start: at, size: p.start - at });
            }
            out.push(Segment::Part(i));
            at = at.max(p.start + p.size);
        }
        if self.size > at && self.size - at >= 8 * MIB && !self.parts.is_empty() {
            out.push(Segment::Free { start: at, size: self.size - at });
        }
        out
    }
}

/// A ZFS pool: its health, size, members and last scrub or resilver.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pool {
    pub name: String,
    pub size: u64,
    pub alloc: u64,
    pub free: u64,
    /// "ONLINE", "DEGRADED", "FAULTED"...
    pub health: String,
    /// (device as zpool names it, its state).
    pub members: Vec<(String, String)>,
    /// zpool's scan line: "scrub repaired 0B in 00:01:02 with 0 errors on Sun Nov 22 ...".
    pub scan: String,
    /// zpool's errors line: "No known data errors".
    pub errors: String,
}

/// Every disk the system knows, internal first, by name.
pub fn disks() -> Vec<Disk> {
    let fs = vergence_sysinfo::file_systems();
    let pools = pools();
    #[cfg(target_os = "linux")]
    let mut list = linux::disks(&fs);
    #[cfg(target_os = "freebsd")]
    let mut list = freebsd_disks(&fs);
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    let mut list: Vec<Disk> = Vec::new();
    for d in &mut list {
        for p in d.parts.iter_mut().chain(d.whole.iter_mut()) {
            if p.role.is_empty()
                && let Some(pool) = pools.iter().find(|pool| pool.members.iter().any(|(m, _)| member_is(m, &p.name)))
            {
                p.role = format!("Part of ZFS pool {}", pool.name);
                if p.fs.is_empty() {
                    p.fs = "ZFS".into();
                }
            }
        }
    }
    list.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
    list
}

/// Whether zpool's member name ("nda0p4", "/dev/nda0p4", "gpt/zfs0", "sdb1") is partition `part`.
fn member_is(member: &str, part: &str) -> bool {
    let m = member.strip_prefix("/dev/").unwrap_or(member);
    m == part
}

/// The system's own tool by a fixed path, never PATH: a session's PATH may leave out the
/// sbin directories (kepler's did, 2026-11-27, and Disks found no disks), and a program
/// earlier in PATH is not the system's.
fn system_tool(name: &str) -> std::path::PathBuf {
    let dirs = ["/sbin", "/usr/sbin", "/bin", "/usr/bin", "/usr/local/sbin", "/usr/local/bin"];
    dirs.iter()
        .map(|d| std::path::Path::new(d).join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| std::path::Path::new("/sbin").join(name))
}

#[cfg(target_os = "freebsd")]
fn freebsd_disks(fs: &[vergence_sysinfo::FileSystem]) -> Vec<Disk> {
    let run = |cmd: &str, args: &[&str]| {
        std::process::Command::new(system_tool(cmd))
            .args(args)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let xml = run("sysctl", &["-n", "kern.geom.confxml"]);
    let mounts = parse_mount_p(&run("mount", &["-p"]));
    let umass = !run("sysctl", &["-n", "dev.umass"]).trim().is_empty();
    let mut list = geom::disks(&xml, umass);
    for d in &mut list {
        for p in d.parts.iter_mut().chain(d.whole.iter_mut()) {
            attach_mount(p, &mounts, fs);
        }
    }
    list
}

/// `mount -p` (FreeBSD): device, mount point, type; devices as named there ("/dev/gpt/efiboot0",
/// "/dev/nda0p2", "zroot/ROOT/default").
pub fn parse_mount_p(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            Some((w.next()?.to_string(), w.next()?.to_string(), w.next()?.to_string()))
        })
        .collect()
}

/// Gives `p` its mount point (the shortest of those on its device or one of its other names)
/// and usage.
#[cfg_attr(not(target_os = "freebsd"), allow(dead_code))]
fn attach_mount(p: &mut Part, mounts: &[(String, String, String)], fs: &[vergence_sysinfo::FileSystem]) {
    let mut names = vec![format!("/dev/{}", p.name)];
    names.extend(p.aliases.iter().map(|a| format!("/dev/{a}")));
    let mut found: Vec<&(String, String, String)> = mounts.iter().filter(|m| names.contains(&m.0)).collect();
    found.sort_by_key(|m| m.1.len());
    if let Some(m) = found.first() {
        p.mount = Some(m.1.clone());
        if p.fs.is_empty() {
            p.fs = fs_word(&m.2);
        }
        p.usage = fs.iter().find(|f| f.mount == m.1).map(|f| (f.total, f.free));
    }
}

/// Every ZFS pool, with `zpool` run as the person (read-only; any user may). None when ZFS is
/// not there.
pub fn pools() -> Vec<Pool> {
    let run = |args: &[&str]| {
        std::process::Command::new(system_tool("zpool"))
            .args(args)
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let Some(list) = run(&["list", "-Hp", "-o", "name,size,alloc,free,health"]) else { return Vec::new() };
    let mut pools = parse_zpool_list(&list);
    for p in &mut pools {
        if let Some(status) = run(&["status", "-p", &p.name]) {
            parse_zpool_status(&status, p);
        }
    }
    pools
}

/// `zpool list -Hp -o name,size,alloc,free,health`: tab-separated, exact bytes.
pub fn parse_zpool_list(text: &str) -> Vec<Pool> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 5).then(|| Pool {
                name: f[0].to_string(),
                size: f[1].parse().unwrap_or(0),
                alloc: f[2].parse().unwrap_or(0),
                free: f[3].parse().unwrap_or(0),
                health: f[4].to_string(),
                ..Default::default()
            })
        })
        .collect()
}

/// `zpool status -p <pool>`: the scan and errors lines, and the members under the pool's own
/// line in its config table (vdev groups such as "mirror-0" are not members; their disks are).
pub fn parse_zpool_status(text: &str, pool: &mut Pool) {
    let mut in_config = false;
    let mut pool_indent = None;
    let mut last_key = "";
    for line in text.lines() {
        let t = line.trim();
        if let Some((k, v)) = t.split_once(':')
            && !in_config
            && ["scan", "errors", "status", "action", "state", "pool", "config"].contains(&k)
        {
            last_key = k;
            match k {
                "scan" => pool.scan = v.trim().to_string(),
                "errors" => pool.errors = v.trim().to_string(),
                "config" => in_config = true,
                _ => {}
            }
            continue;
        }
        if in_config {
            if let Some(v) = t.strip_prefix("errors:") {
                pool.errors = v.trim().to_string();
                in_config = false;
                continue;
            }
            let indent = line.len() - line.trim_start().len();
            let mut w = t.split_whitespace();
            let (Some(name), state) = (w.next(), w.next()) else { continue };
            if name == "NAME" {
                continue;
            }
            match pool_indent {
                None if name == pool.name => pool_indent = Some(indent),
                Some(pi) if indent > pi => {
                    let group = ["mirror", "raidz", "draid", "spare", "log", "cache", "special", "dedup", "logs"]
                        .iter()
                        .any(|g| name.starts_with(g));
                    if !group {
                        pool.members.push((name.to_string(), state.unwrap_or("").to_string()));
                    }
                }
                Some(_) => pool_indent = None,
                None => {}
            }
        } else if last_key == "scan" && !t.is_empty() && line.starts_with('\t') {
            // The scan line continues on the next line while one runs.
            pool.scan.push(' ');
            pool.scan.push_str(t);
        }
    }
}

/// A file system's name as the system spells it, in words.
pub fn fs_word(fs: &str) -> String {
    match fs.to_ascii_lowercase().as_str() {
        "vfat" | "msdosfs" | "fat32" => "FAT32".into(),
        "fat16" => "FAT16".into(),
        "fat12" => "FAT12".into(),
        "exfat" => "exFAT".into(),
        "ntfs" | "ntfs3" => "NTFS".into(),
        "ext4" => "ext4".into(),
        "ext3" => "ext3".into(),
        "ext2" => "ext2".into(),
        "btrfs" => "Btrfs".into(),
        "xfs" => "XFS".into(),
        "f2fs" => "F2FS".into(),
        "zfs" | "zfs_member" => "ZFS".into(),
        "ufs" => "UFS".into(),
        "swap" => "Swap".into(),
        "crypto_luks" => "Encrypted (LUKS)".into(),
        "lvm2_member" => "LVM".into(),
        "linux_raid_member" => "RAID member".into(),
        "iso9660" | "cd9660" => "ISO 9660 (disc image)".into(),
        "udf" => "UDF".into(),
        "apfs" => "APFS".into(),
        "hfsplus" => "HFS+".into(),
        "squashfs" => "SquashFS".into(),
        "" => String::new(),
        other => other.to_string(),
    }
}

/// A partition's type in words: GPT type GUIDs (Linux, udev's ID_PART_ENTRY_TYPE), GEOM's type
/// names (FreeBSD), and MBR type bytes ("0x83").
pub fn ptype_word(t: &str) -> String {
    let words: &[(&str, &str)] = &[
        ("c12a7328-f81f-11d2-ba4b-00a0c93ec93b", "EFI system"),
        ("efi", "EFI system"),
        ("0xef", "EFI system"),
        ("21686148-6449-6e6f-744e-656564454649", "BIOS boot"),
        ("0fc63daf-8483-4772-8e79-3d69d8477de4", "Linux file system"),
        ("linux-data", "Linux file system"),
        ("0x83", "Linux file system"),
        ("4f68bce3-e8cd-4db1-96e7-fbcaf984b709", "Linux root (x86-64)"),
        ("933ac7e1-2eb4-4f13-b844-0e14e2aef915", "Linux home"),
        ("0657fd6d-a4ab-43c4-84e5-0933c84b4f4f", "Linux swap"),
        ("linux-swap", "Linux swap"),
        ("0x82", "Linux swap"),
        ("e6d6d379-f507-44c2-a23c-238f2a3df928", "Linux LVM"),
        ("ca7d7ccb-63ed-4c53-861c-1742536059cc", "Linux encrypted (LUKS)"),
        ("a19d880f-05fc-4d3b-a006-743f0f84911e", "Linux RAID"),
        ("ebd0a0a2-b9e5-4433-87c0-68b6b72699c7", "Microsoft basic data"),
        ("ms-basic-data", "Microsoft basic data"),
        ("e3c9e316-0b5c-4db8-817d-f92df00215ae", "Microsoft reserved"),
        ("ms-reserved", "Microsoft reserved"),
        ("de94bba4-06d1-4d40-a16a-bfd50179d6ac", "Windows recovery"),
        ("ms-recovery", "Windows recovery"),
        ("0x07", "Windows (NTFS or exFAT)"),
        ("0x0b", "FAT32"),
        ("0x0c", "FAT32"),
        ("0x0e", "FAT16"),
        ("0x06", "FAT16"),
        ("fat32", "FAT32"),
        ("fat32lba", "FAT32"),
        ("fat16", "FAT16"),
        ("ntfs", "Windows (NTFS or exFAT)"),
        ("83bd6b9d-7f41-11dc-be0b-001560b84f0f", "FreeBSD boot"),
        ("freebsd-boot", "FreeBSD boot"),
        ("516e7cb5-6ecf-11d6-8ff8-00022d09712b", "FreeBSD swap"),
        ("freebsd-swap", "FreeBSD swap"),
        ("516e7cb6-6ecf-11d6-8ff8-00022d09712b", "FreeBSD UFS"),
        ("freebsd-ufs", "FreeBSD UFS"),
        ("516e7cba-6ecf-11d6-8ff8-00022d09712b", "FreeBSD ZFS"),
        ("freebsd-zfs", "FreeBSD ZFS"),
        ("516e7cb4-6ecf-11d6-8ff8-00022d09712b", "FreeBSD"),
        ("freebsd", "FreeBSD"),
        ("0xa5", "FreeBSD"),
        ("7c3457ef-0000-11aa-aa11-00306543ecac", "Apple APFS"),
        ("apple-apfs", "Apple APFS"),
        ("48465300-0000-11aa-aa11-00306543ecac", "Apple HFS+"),
        ("apple-hfs", "Apple HFS+"),
        ("0xee", "GPT protective"),
    ];
    let low = t.to_ascii_lowercase();
    words.iter().find(|(k, _)| *k == low).map_or_else(|| t.to_string(), |(_, w)| w.to_string())
}

/// udev's database record (`/run/udev/data/b259:1`): its `E:KEY=value` lines.
pub fn parse_udev(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.strip_prefix("E:")?.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// The name to show for a partition from udev's record: the file system's label as written
/// (`ID_FS_LABEL_ENC`; plain `ID_FS_LABEL` has its spaces turned to `_`), else the GPT
/// partition's own name (`ID_PART_ENTRY_NAME`). udev writes both with `\xHH` escapes; they
/// are undone here, and control characters dropped (a stick's label is anyone's bytes).
pub fn udev_label(u: &HashMap<String, String>) -> String {
    let undo = |s: &str| {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\\'
                && b.get(i + 1) == Some(&b'x')
                && let Some(v) = s.get(i + 2..i + 4).and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 4;
                continue;
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).chars().filter(|c| !c.is_control()).collect::<String>()
    };
    u.get("ID_FS_LABEL_ENC")
        .map(|v| undo(v))
        .or_else(|| u.get("ID_FS_LABEL").cloned())
        .or_else(|| u.get("ID_PART_ENTRY_NAME").map(|v| undo(v)))
        .unwrap_or_default()
}

/// Mount tables of the `/proc/mounts` shape: device, point, type; octal escapes ("\040") undone.
pub fn parse_proc_mounts(text: &str) -> Vec<(String, String, String)> {
    let unescape = |s: &str| {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\\'
                && i + 3 < b.len()
                && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 4]).unwrap_or(""), 8)
            {
                out.push(v);
                i += 4;
                continue;
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    parse_mount_p(text).into_iter().map(|(d, m, t)| (unescape(&d), unescape(&m), t)).collect()
}

/// A short size for the bar: "1 TB", "512 MB", "8 GB".
/// What a disk is for, in a word or two, so two disks of one model can be told apart in the
/// sidebar: "System" (it holds `/`), "Pool zroot", "Windows", else its first mount point or
/// volume label; None when nothing on it says.
pub fn disk_use(d: &Disk) -> Option<String> {
    let mut parts: Vec<&Part> = d.parts.iter().chain(d.whole.as_ref()).collect();
    parts.sort_by_key(|p| p.start);
    if parts.iter().any(|p| p.mount.as_deref() == Some("/")) {
        return Some("System".into());
    }
    if let Some(pool) = parts.iter().find_map(|p| p.role.strip_prefix("Part of ZFS pool ")) {
        return Some(format!("Pool {pool}"));
    }
    if parts.iter().any(|p| p.fs == "NTFS" && p.ptype == "Microsoft basic data") {
        return Some("Windows".into());
    }
    parts
        .iter()
        .find_map(|p| p.mount.clone())
        .or_else(|| parts.iter().find(|p| !p.label.is_empty()).map(|p| p.label.clone()))
}

pub fn size_text(b: u64) -> String {
    vergence_sysinfo::bytes_text(b)
}

#[cfg(test)]
mod tests;
