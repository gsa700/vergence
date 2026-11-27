//! Linux: disks from `/sys/block`, what udev learned of each (`/run/udev/data/b<maj>:<min>`),
//! mounts from `/proc/mounts`, swap from `/proc/swaps`. All readable by any user.

use std::path::Path;

use crate::{Disk, Kind, Part, attach_mount, fs_word, parse_proc_mounts, parse_udev, ptype_word};

fn read(p: impl AsRef<Path>) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

fn udev(sys: &Path) -> std::collections::HashMap<String, String> {
    let dev = read(sys.join("dev"));
    parse_udev(&read(format!("/run/udev/data/b{}", dev.trim())))
}

pub fn disks(fs: &[vergence_sysinfo::FileSystem]) -> Vec<Disk> {
    let mounts = parse_proc_mounts(&read("/proc/mounts"));
    let swaps: Vec<String> =
        read("/proc/swaps").lines().skip(1).filter_map(|l| l.split_whitespace().next()).map(String::from).collect();
    let Ok(dir) = std::fs::read_dir("/sys/block") else { return Vec::new() };
    let mut out = Vec::new();
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // Not disks: RAM disks, compressed swap in memory, device-mapper's views (an unlocked
        // LUKS volume shows on its partition instead), and optical drives.
        if ["ram", "zram", "dm-", "sr", "md", "fd"].iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let sys = e.path();
        let size = read(sys.join("size")).trim().parse::<u64>().unwrap_or(0) * 512;
        if size == 0 {
            continue; // an empty loop device or card reader
        }
        let u = udev(&sys);
        let link = std::fs::read_link(&sys).map(|l| l.to_string_lossy().into_owned()).unwrap_or_default();
        let removable = read(sys.join("removable")).trim() == "1";
        let (bus, kind) = if name.starts_with("loop") {
            ("Disk image", Kind::Image)
        } else if link.contains("/usb") || u.get("ID_BUS").is_some_and(|b| b == "usb") {
            ("USB", Kind::External)
        } else if name.starts_with("mmcblk") {
            ("Memory card", if removable { Kind::External } else { Kind::Internal })
        } else if name.starts_with("nvme") {
            ("NVMe", Kind::Internal)
        } else if name.starts_with("vd") || link.contains("/virtio") {
            ("Virtual", Kind::Internal)
        } else if link.contains("/ata") {
            ("SATA", if removable { Kind::External } else { Kind::Internal })
        } else {
            ("", if removable { Kind::External } else { Kind::Internal })
        };
        let dev = sys.join("device");
        let vendor = read(dev.join("vendor")).trim().to_string();
        let mut model = read(dev.join("model")).trim().to_string();
        if bus == "USB" && !vendor.is_empty() && !model.starts_with(&vendor) {
            model = format!("{vendor} {model}");
        }
        if model.is_empty() {
            model = u.get("ID_MODEL").cloned().unwrap_or_default().replace('_', " ");
        }
        if bus == "Disk image" {
            let file = read(sys.join("loop/backing_file")).trim().to_string();
            model = Path::new(&file).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or(file);
        }
        let serial = {
            let s = read(dev.join("serial")).trim().to_string();
            if s.is_empty() { u.get("ID_SERIAL_SHORT").cloned().unwrap_or_default() } else { s }
        };
        let firmware = {
            let f = read(dev.join("firmware_rev")).trim().to_string();
            if f.is_empty() { read(dev.join("rev")).trim().to_string() } else { f }
        };
        let rotational = match read(sys.join("queue/rotational")).trim() {
            "1" if bus != "USB" && bus != "Disk image" => Some(true),
            "0" => Some(false),
            _ => None,
        };
        let mut d = Disk {
            model,
            serial,
            firmware,
            size,
            bus: bus.into(),
            kind: Some(kind),
            rotational,
            scheme: match u.get("ID_PART_TABLE_TYPE").map(String::as_str) {
                Some("gpt") => "GPT".into(),
                Some("dos") => "MBR".into(),
                Some(o) => o.to_uppercase(),
                None => String::new(),
            },
            temp: temperature(&dev),
            name: name.clone(),
            ..Default::default()
        };
        let mut parts: Vec<_> = std::fs::read_dir(&sys)
            .map(|r| {
                r.flatten().filter(|p| p.file_name().to_string_lossy().starts_with(&name)).map(|p| p.path()).collect()
            })
            .unwrap_or_default();
        parts.sort();
        for p in parts {
            let pname = p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let pu = udev(&p);
            let mut part = Part {
                index: read(p.join("partition")).trim().parse().unwrap_or(0),
                start: read(p.join("start")).trim().parse::<u64>().unwrap_or(0) * 512,
                size: read(p.join("size")).trim().parse::<u64>().unwrap_or(0) * 512,
                ptype: pu.get("ID_PART_ENTRY_TYPE").map(|t| ptype_word(t)).unwrap_or_default(),
                ..Default::default()
            };
            fill(&mut part, &pu, &pname, &p, &swaps);
            part.name = pname;
            attach_mount(&mut part, &mounts, fs);
            d.parts.push(part);
        }
        if d.parts.is_empty() && u.contains_key("ID_FS_TYPE") {
            let mut whole = Part { size, ..Default::default() };
            fill(&mut whole, &u, &name, &sys, &swaps);
            whole.name = name.clone();
            attach_mount(&mut whole, &mounts, fs);
            d.whole = Some(whole);
        }
        out.push(d);
    }
    out
}

/// A partition's file system, label, UUID, and what holds it, from udev and sysfs.
fn fill(part: &mut Part, u: &std::collections::HashMap<String, String>, name: &str, sys: &Path, swaps: &[String]) {
    part.fs = fs_word(u.get("ID_FS_TYPE").map_or("", String::as_str));
    part.label = u.get("ID_FS_LABEL").or_else(|| u.get("ID_PART_ENTRY_NAME")).cloned().unwrap_or_default();
    part.uuid = u.get("ID_FS_UUID").cloned().unwrap_or_default();
    if swaps.iter().any(|s| s == &format!("/dev/{name}")) {
        part.role = "Swap, in use".into();
    } else if std::fs::read_dir(sys.join("holders")).is_ok_and(|mut h| h.next().is_some()) {
        part.role = if part.fs.starts_with("Encrypted") {
            "Encrypted, unlocked".into()
        } else {
            "In use by another volume".into()
        };
    } else if part.fs.starts_with("Encrypted") {
        part.role = "Encrypted, locked".into();
    }
}

/// The drive's own temperature where it gives one any user may read: NVMe's hwmon on the
/// controller, a SATA drive's through the drivetemp module.
fn temperature(dev: &Path) -> Option<f32> {
    let mon = std::fs::read_dir(dev)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("hwmon"))
        .map(|e| e.path())
        .or_else(|| std::fs::read_dir(dev.join("hwmon")).ok()?.flatten().next().map(|e| e.path()))?;
    read(mon.join("temp1_input")).trim().parse::<f32>().ok().map(|m| m / 1000.0)
}
