//! The drives (drives milestone 1): the system's mount table, the disks and
//! network mounts in it, the automounter's /media on FreeBSD, and drives
//! ejected but still plugged in. Shared by the server, which keeps the
//! drive list on the bus, and the apps, which read it themselves outside a
//! Vergence session (vergence-widgets files.rs).

use std::fs;
use std::path::{Path, PathBuf};

use vergence_protocol::bus::{DriveInfo, DriveKind};

/// One line of the system's mount table: what is mounted, where, and its
/// file system's type.
#[derive(Clone, Debug, PartialEq)]
pub struct Mount {
    pub what: String,
    pub at: PathBuf,
    pub kind: String,
}

/// A mount table's text read: Linux /proc/self/mounts, or FreeBSD's
/// `mount -p` (fstab's form); the same columns. Octal escapes (\040 a
/// space) undone.
pub fn parse_mounts(text: &str) -> Vec<Mount> {
    let unescape = |s: &str| -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
                out.push(((b[i + 1] - b'0') << 6) | ((b[i + 2] - b'0') << 3) | (b[i + 3] - b'0'));
                i += 4;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    text.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let mut what = f.next()?.to_string();
            // FreeBSD's automounter: "map -media  /media  autofs ..." (a space in the first field).
            if what == "map" {
                what = format!("map {}", f.next()?);
            }
            let (at, kind) = (f.next()?, f.next()?);
            (!what.starts_with('#')).then(|| Mount {
                what: unescape(&what),
                at: PathBuf::from(unescape(at)),
                kind: kind.to_string(),
            })
        })
        .collect()
}

/// The system's mounts now.
pub fn mounts() -> Vec<Mount> {
    let text = fs::read_to_string("/proc/self/mounts").ok().or_else(|| {
        // FreeBSD: mount -p prints the table in fstab's form. By its full
        // path: a session's PATH may have no /sbin (and then no drive ever
        // shows on the desktop or in FileView), and a PATH
        // is not a thing to trust for a system tool anyway.
        let mount = if Path::new("/sbin/mount").exists() { "/sbin/mount" } else { "mount" };
        std::process::Command::new(mount)
            .arg("-p")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    });
    parse_mounts(&text.unwrap_or_default())
}

/// Whether a file system type is one reached over the network (NFS, SMB,
/// sshfs and other FUSE network file systems, WebDAV).
pub fn network_kind(kind: &str) -> bool {
    matches!(kind, "nfs" | "nfs4" | "cifs" | "smb3" | "smbfs" | "davfs" | "9p" | "afs")
        || ["fuse.sshfs", "fusefs.sshfs", "fuse.rclone", "fusefs.rclone", "fuse.gvfsd-fuse"].contains(&kind)
}

/// Disks mounted where removable and extra disks go (network mounts aside).
pub fn mounted_disks() -> Vec<PathBuf> {
    disks_of(&mounts())
}

pub fn disks_of(table: &[Mount]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = table
        .iter()
        .filter(|m| !network_kind(&m.kind) && m.kind != "autofs")
        .map(|m| m.at.clone())
        .filter(|m| {
            let s = m.to_string_lossy();
            ["/media/", "/mnt/", "/run/media/"].iter().any(|p| s.starts_with(p)) || s == "/mnt"
        })
        .collect();
    // FreeBSD's automounter (vergence-media; install.sh): /media lists the
    // drives plugged in before they are mounted (looking into one mounts it).
    if table.iter().any(|m| m.at == Path::new("/media") && m.kind == "autofs")
        && let Ok(rd) = fs::read_dir("/media")
    {
        // A name stays listed after its drive is pulled out: kept only while
        // it is mounted or its device is there (by name, or by its label),
        // and not while it is a drive ejected and still plugged in (the
        // same device node as when it was ejected: FreeBSD does not power
        // a stick off, so an ejected stick would stay listed; see
        // docs/files-milestone-3.md).
        let mounted: Vec<&Path> = table.iter().map(|m| m.at.as_path()).collect();
        let ejected = ejected_now();
        let on_images = labels_on_images(&label_providers());
        out.extend(rd.flatten().map(|e| e.path()).filter(|p| {
            if mounted.contains(&p.as_path()) {
                return true;
            }
            let Some(name) = p.file_name() else { return false };
            match device_of(name) {
                // A label on a disk image's own partitions (a Fedora ISO brought
                // /media/ANACONDA and its volume as drives): the
                // image is listed by where it is mounted, not as a stick.
                Some(node) if node.strip_prefix("/dev").is_ok_and(|l| on_images.iter().any(|i| Path::new(i) == l)) => {
                    false
                }
                Some(node) => !ejected.iter().any(|(n, ino)| n.as_os_str() == name && Some(*ino) == node_ino(&node)),
                None => false,
            }
        }));
    }
    out.sort();
    out.dedup();
    out
}

/// The device node of a drive the automounter lists under /media by
/// `name` (by name, or by its label), if it is there.
pub(crate) fn device_of(name: &std::ffi::OsStr) -> Option<PathBuf> {
    std::iter::once(PathBuf::from("/dev").join(name))
        .chain(
            ["msdosfs", "ufs", "ext2fs", "ntfs", "exfat", "iso9660", "label", "gpt"]
                .iter()
                .map(|d| PathBuf::from("/dev").join(d).join(name)),
        )
        .find(|p| p.exists())
}

/// FreeBSD: each label and the GEOM provider under it ("msdosfs/USB",
/// "da0s1"), from `glabel status -s`.
fn label_providers() -> Vec<(String, String)> {
    if !cfg!(target_os = "freebsd") {
        return Vec::new();
    }
    // By its full path: the session's PATH may not have /sbin.
    let glabel = if Path::new("/sbin/glabel").exists() { "/sbin/glabel" } else { "glabel" };
    std::process::Command::new(glabel)
        .args(["status", "-s"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map(|o| parse_glabel(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// `glabel status -s`'s lines, "NAME STATUS COMPONENT".
pub(crate) fn parse_glabel(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            (w.len() >= 3).then(|| (w[0].to_string(), w[w.len() - 1].to_string()))
        })
        .collect()
}

/// The labels whose provider is an md device or a partition of one.
fn labels_on_images(labels: &[(String, String)]) -> Vec<String> {
    labels
        .iter()
        .filter(|(_, c)| c.chars().take_while(|x| x.is_ascii_alphabetic()).collect::<String>() == "md")
        .map(|(n, _)| n.clone())
        .collect()
}

/// The provider a device is: a label's node is the provider under it.
pub(crate) fn provider_of<'a>(device: &'a str, labels: &'a [(String, String)]) -> &'a str {
    let dev = device.strip_prefix("/dev/").unwrap_or(device);
    labels.iter().find(|(n, _)| n == dev).map_or(dev, |(_, c)| c.as_str())
}

/// One entry per drive: the automounter lists a stick by its label and,
/// once looked up by it, by its device name too (/media/USB and
/// /media/da0s1). The mounted one is kept, else the one by its label.
fn one_per_device(list: &mut Vec<DriveInfo>, labels: &[(String, String)]) {
    let keys: Vec<String> = list.iter().map(|d| provider_of(&d.device, labels).to_string()).collect();
    let rank = |d: &DriveInfo| (d.mounted, !d.path.ends_with(&format!("/{}", provider_of(&d.device, labels))));
    let mut keep = vec![true; list.len()];
    for i in 0..list.len() {
        if list[i].kind != DriveKind::Removable || keys[i].is_empty() {
            continue;
        }
        for j in 0..list.len() {
            if i != j
                && keep[j]
                && keys[i] == keys[j]
                && list[j].kind == DriveKind::Removable
                && (rank(&list[j]), j < i) > (rank(&list[i]), false)
            {
                keep[i] = false;
                break;
            }
        }
    }
    let mut k = keep.into_iter();
    list.retain(|_| k.next().unwrap_or(true));
}

fn node_ino(p: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(p).ok().map(|m| m.ino())
}

/// ~/.local/state/vergence/ejected: drives ejected (their /media name and
/// device node's inode), hidden while that node is there; plugged in again,
/// a drive has a new node and shows.
fn ejected_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("vergence/ejected"))
}

fn ejected_now() -> Vec<(PathBuf, u64)> {
    let text = ejected_file().and_then(|p| fs::read_to_string(p).ok()).unwrap_or_default();
    text.lines()
        .filter_map(|l| l.split_once('\t'))
        .filter_map(|(n, i)| Some((PathBuf::from(n), i.parse().ok()?)))
        .collect()
}

/// Notes a drive under /media as ejected (its device node now), so the lists
/// leave it out until it is plugged in again.
pub fn note_ejected(path: &Path) {
    let (Some(name), Some(file)) = (path.strip_prefix("/media").ok().and_then(|n| n.file_name()), ejected_file())
    else {
        return;
    };
    let Some(ino) = device_of(name).and_then(|d| node_ino(&d)) else { return };
    let mut all = ejected_now();
    all.retain(|(n, _)| n.as_os_str() != name);
    all.push((PathBuf::from(name), ino));
    let keep = all.len().saturating_sub(32);
    let text: String =
        all[keep..].iter().map(|(n, i)| format!("{}\t{i}\n", n.to_string_lossy().replace(['\t', '\n'], " "))).collect();
    if let Some(d) = file.parent() {
        let _ = fs::create_dir_all(d);
    }
    let _ = fs::write(file, text);
}

/// Forgets the drives ejected whose device is gone (pulled out): plugged
/// in again, a drive shows (devfs may give the new node the old one's
/// number, which hid the same stick plugged in again).
pub fn prune_ejected() {
    let all = ejected_now();
    let kept: Vec<(PathBuf, u64)> = all.iter().filter(|(n, _)| device_of(n.as_os_str()).is_some()).cloned().collect();
    if kept.len() == all.len() {
        return;
    }
    let Some(file) = ejected_file() else { return };
    let text: String = kept.iter().map(|(n, i)| format!("{}\t{i}\n", n.to_string_lossy())).collect();
    let _ = if text.is_empty() { fs::remove_file(file) } else { fs::write(file, text) };
}

/// What the system has mounted over the network, anywhere (files
/// milestone 1, step 10: shown under Network in the sidebar).
pub fn network_mounts() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = mounts().into_iter().filter(|m| network_kind(&m.kind)).map(|m| m.at).collect();
    out.sort();
    out.dedup();
    out
}

/// Every drive now, as the server lists it (drives milestone 1): the disks
/// where drives are mounted (and, on FreeBSD, the drives the automounter
/// lists before they are mounted), then the network's mounts. A disk of the
/// machine's own (shown, as Explorer does; docs/drives-milestone-1.md) cannot be
/// ejected; everything else can.
pub fn scan() -> Vec<DriveInfo> {
    let table = mounts();
    let labels = label_providers();
    let on_images = labels_on_images(&labels);
    let mut out: Vec<DriveInfo> = Vec::new();
    for at in disks_of(&table) {
        let m = table.iter().find(|m| m.at == at);
        // A drive pulled out while mounted: FreeBSD keeps the mount of a
        // device that is gone, which made a pulled stick come back; not a
        // drive (stale_mounts lets go of it).
        if m.is_some_and(gone) {
            continue;
        }
        let device = m
            .map(|m| m.what.clone())
            .or_else(|| at.file_name().and_then(device_of).map(|p| p.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let kind = match m {
            Some(_) if device.starts_with("/dev/md") || device.starts_with("/dev/loop") => DriveKind::Image,
            // A disk image's own partition, mounted by the automounter under its label.
            Some(_) if device.strip_prefix("/dev/").is_some_and(|l| on_images.iter().any(|i| i == l)) => {
                DriveKind::Image
            }
            Some(m) if matches!(m.kind.as_str(), "cd9660" | "iso9660" | "udf") => DriveKind::Optical,
            // Listed by the automounter and not mounted yet: a drive plugged in.
            None => DriveKind::Removable,
            Some(_) if removable(&device) => DriveKind::Removable,
            Some(_) => DriveKind::Internal,
        };
        out.push(DriveInfo {
            name: name_of(&at),
            path: at.to_string_lossy().into_owned(),
            kind,
            mounted: m.is_some(),
            ejectable: kind != DriveKind::Internal,
            device,
        });
    }
    for m in table.iter().filter(|m| network_kind(&m.kind)) {
        out.push(DriveInfo {
            name: name_of(&m.at),
            path: m.at.to_string_lossy().into_owned(),
            kind: DriveKind::Network,
            mounted: true,
            ejectable: true,
            device: m.what.clone(),
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    one_per_device(&mut out, &labels);
    out
}

/// A mount whose device is a node that is not there any more.
fn gone(m: &Mount) -> bool {
    m.what.starts_with("/dev/") && !Path::new(&m.what).exists()
}

/// Drives under /media still mounted though their device is gone (pulled
/// out while mounted): to be unmounted (the server does, through eject).
pub fn stale_mounts() -> Vec<PathBuf> {
    mounts().into_iter().filter(|m| m.at.starts_with("/media/") && gone(m)).map(|m| m.at).collect()
}

fn name_of(p: &Path) -> String {
    p.file_name().map_or_else(|| p.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
}

/// Whether a device is a drive plugged in rather than one of the machine's
/// own. FreeBSD: by its driver (da, cd, mmcsd and sdda are; ada, nvd, nda
/// and vtbd are not; a label's node, as the automounter's map uses, is).
/// Linux: by sysfs (on USB, or the block device says removable; cards are).
pub fn removable(device: &str) -> bool {
    let Some(dev) = device.strip_prefix("/dev/") else { return false };
    if cfg!(target_os = "linux") {
        if dev.starts_with("mmcblk") {
            return true;
        }
        let Ok(real) = fs::canonicalize(Path::new("/sys/class/block").join(dev)) else { return false };
        if real.to_string_lossy().contains("/usb") {
            return true;
        }
        let says = |p: &Path| fs::read_to_string(p.join("removable")).is_ok_and(|v| v.trim() == "1");
        return says(&real) || real.parent().is_some_and(says);
    }
    if ["msdosfs/", "iso9660/", "label/", "ext2fs/", "ntfs/", "exfat/", "ufs/"].iter().any(|d| dev.starts_with(d)) {
        return true;
    }
    let base: String = dev.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    matches!(base.as_str(), "da" | "cd" | "mmcsd" | "sdda")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freebsd_devices_by_their_driver() {
        if cfg!(target_os = "linux") {
            return;
        }
        assert!(removable("/dev/da0s1") && removable("/dev/msdosfs/USB") && removable("/dev/cd0"));
        assert!(!removable("/dev/ada0p2") && !removable("/dev/nvd0p1") && !removable("zroot/usr/home"));
    }

    #[test]
    fn labels_on_a_disk_image_are_told_apart() {
        let text = "iso9660/Fedora-E-dvd-x86_64-45  N/A  md0\n msdosfs/ANACONDA  N/A  md0p2\nmsdosfs/USB  N/A  da0s1\ngpt/efiboot0  N/A  nda0p1\n";
        assert_eq!(labels_on_images(&parse_glabel(text)), vec!["iso9660/Fedora-E-dvd-x86_64-45", "msdosfs/ANACONDA"]);
    }

    #[test]
    fn a_stick_by_its_label_and_its_device_is_one_drive() {
        let labels = parse_glabel("msdosfs/USB  N/A  da0s1\n");
        let d = |path: &str, device: &str, mounted: bool| DriveInfo {
            name: path.rsplit('/').next().unwrap().into(),
            path: path.into(),
            kind: DriveKind::Removable,
            mounted,
            ejectable: true,
            device: device.into(),
        };
        // Neither mounted: the one by its label.
        let mut l = vec![d("/media/USB", "/dev/msdosfs/USB", false), d("/media/da0s1", "/dev/da0s1", false)];
        one_per_device(&mut l, &labels);
        assert_eq!(l.iter().map(|x| x.path.as_str()).collect::<Vec<_>>(), ["/media/USB"]);
        // Mounted by its label (the label node withers then): that one.
        let mut l = vec![d("/media/USB", "/dev/da0s1", true), d("/media/da0s1", "/dev/da0s1", false)];
        one_per_device(&mut l, &[]);
        assert_eq!(l.iter().map(|x| x.path.as_str()).collect::<Vec<_>>(), ["/media/USB"]);
        // Mounted by its device name: that one.
        let mut l = vec![d("/media/USB", "/dev/msdosfs/USB", false), d("/media/da0s1", "/dev/da0s1", true)];
        one_per_device(&mut l, &labels);
        assert_eq!(l.iter().map(|x| x.path.as_str()).collect::<Vec<_>>(), ["/media/da0s1"]);
        // Two sticks stay two.
        let mut l = vec![d("/media/USB", "/dev/msdosfs/USB", false), d("/media/da1s1", "/dev/da1s1", false)];
        one_per_device(&mut l, &labels);
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn the_drives_now_are_listed_with_their_kinds() {
        // Whatever this machine has: every entry has a path and a name, and
        // only disks of its own cannot be ejected.
        for d in scan() {
            assert!(!d.path.is_empty() && !d.name.is_empty(), "{d:?}");
            assert_eq!(d.ejectable, d.kind != DriveKind::Internal);
        }
    }
}
