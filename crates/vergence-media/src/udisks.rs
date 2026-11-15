// udisks2 over the system bus (Linux).

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use vergence_dbus::client::{Client, bytes_str, lookup};
use vergence_dbus::{Message, Value};

const DEST: &str = "org.freedesktop.UDisks2";
const ROOT: &str = "/org/freedesktop/UDisks2";
const FS: &str = "org.freedesktop.UDisks2.Filesystem";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";

/// Every object udisks has: (path, its interfaces' properties).
fn objects(c: &mut Client) -> Result<Vec<(String, Value)>, String> {
    let r = c.call(Message::method_call(DEST, ROOT, "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", &[]), super::CALL).map_err(|e| e.to_string())?;
    let args = r.args().map_err(|e| format!("{e:?}"))?;
    let Some(Value::Array(_, items)) = args.into_iter().next() else { return Err("udisks gave no objects".into()) };
    Ok(items
        .into_iter()
        .filter_map(|e| match e {
            Value::Entry(k, v) => match *k {
                Value::Path(p) => Some((p, *v)),
                _ => None,
            },
            _ => None,
        })
        .collect())
}

/// Options that keep udisks from asking for a password (polkit says yes or no).
fn quiet() -> Value {
    Value::Array("{sv}".into(), vec![Value::Entry(Box::new(Value::Str("auth.no_user_interaction".into())), Box::new(Value::Variant(Box::new(Value::Bool(true)))))])
}

fn flag(dict: &Value, key: &str) -> bool {
    matches!(lookup(dict, key), Some(Value::Bool(true)))
}

/// A file system's mount points.
fn mount_points(ifaces: &Value) -> Vec<String> {
    match lookup(ifaces, FS).and_then(|fs| lookup(fs, "MountPoints")) {
        Some(Value::Array(_, pts)) => pts.iter().filter_map(bytes_str).collect(),
        _ => Vec::new(),
    }
}

/// Whether udisks means a file system to be mounted for the user, and it is not yet.
fn wants_mount(ifaces: &Value) -> bool {
    let Some(block) = lookup(ifaces, BLOCK) else { return false };
    lookup(ifaces, FS).is_some() && flag(block, "HintAuto") && !flag(block, "HintIgnore") && !flag(block, "HintSystem") && mount_points(ifaces).is_empty()
}

fn label(ifaces: &Value) -> String {
    lookup(ifaces, BLOCK).and_then(|b| lookup(b, "IdLabel")).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| "a drive".into())
}

/// Mounts the file systems that are new since `seen` (and, the first time,
/// those already there), and returns what is there now.
fn mount_new(c: &mut Client, seen: Option<&HashSet<String>>, told: &dyn Fn(&str)) -> Result<HashSet<String>, String> {
    let all = objects(c)?;
    for (path, ifaces) in &all {
        if seen.is_some_and(|s| s.contains(path)) || !wants_mount(ifaces) {
            continue;
        }
        let m = Message::method_call(DEST, path, FS, "Mount", &[quiet()]);
        match c.call(m, super::CALL) {
            Ok(r) => {
                let at = r.args().ok().and_then(|a| a.first().and_then(|v| v.as_str().map(str::to_string))).unwrap_or_default();
                told(&format!("media: {} mounted at {at}", label(ifaces)));
            }
            Err(e) => told(&format!("media: {} could not be mounted: {e}", label(ifaces))),
        }
    }
    Ok(all.into_iter().map(|(p, _)| p).collect())
}

pub fn automount(told: &dyn Fn(&str)) {
    // udisks may start after us, or restart: try again now and then.
    loop {
        match watch(told) {
            Ok(()) => {}
            Err(e) => told(&format!("media: udisks2: {e}")),
        }
        std::thread::sleep(Duration::from_secs(30));
    }
}

fn watch(told: &dyn Fn(&str)) -> Result<(), String> {
    let mut c = Client::system().map_err(|e| e.to_string())?;
    c.add_match(&format!("type='signal',sender='{DEST}',interface='org.freedesktop.DBus.ObjectManager'")).map_err(|e| e.to_string())?;
    // What is plugged in at the session's start is mounted too.
    let mut seen = mount_new(&mut c, None, told)?;
    told(&format!("media: watching udisks2 ({} objects)", seen.len()));
    loop {
        let Some(sig) = c.next_signal(Duration::from_secs(300)).map_err(|e| e.to_string())? else { continue };
        if sig.member.as_deref() == Some("InterfacesAdded") || sig.member.as_deref() == Some("InterfacesRemoved") {
            // A drive's partitions and file systems come one after another.
            std::thread::sleep(Duration::from_millis(400));
            while c.next_signal(Duration::from_millis(100)).map_err(|e| e.to_string())?.is_some() {}
            seen = mount_new(&mut c, Some(&seen), told)?;
        }
    }
}

pub fn eject(path: &Path) -> Result<bool, String> {
    let mut c = Client::system().map_err(|e| e.to_string())?;
    let all = objects(&mut c)?;
    let want = path.to_string_lossy();
    let Some((obj, ifaces)) = all.iter().find(|(_, i)| mount_points(i).iter().any(|m| *m == want)) else { return Ok(false) };
    c.call(Message::method_call(DEST, obj, FS, "Unmount", &[quiet()]), super::CALL).map_err(|e| plain(&e.to_string()))?;
    // Then the drive itself is powered off, if it can be (best effort: the
    // file system is let go of already, so it is safe to pull out).
    if let Some(Value::Path(drive)) = lookup(ifaces, BLOCK).and_then(|b| lookup(b, "Drive"))
        && let Some((_, d)) = all.iter().find(|(p, _)| p == drive)
        && lookup(d, DRIVE).is_some_and(|d| flag(d, "CanPowerOff"))
    {
        let _ = c.call(Message::method_call(DEST, drive, DRIVE, "PowerOff", &[quiet()]), super::CALL);
    }
    Ok(true)
}

/// udisks's error, in words: "busy" is the usual one.
fn plain(e: &str) -> String {
    if e.contains("DeviceBusy") || e.contains("target is busy") {
        "something is still using it (a window showing it, or a file open).".into()
    } else if e.contains("NotAuthorized") {
        "this session is not allowed to (polkit said no).".into()
    } else {
        e.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(items: Vec<(&str, Value)>) -> Value {
        Value::Array("{sv}".into(), items.into_iter().map(|(k, v)| Value::Entry(Box::new(Value::Str(k.into())), Box::new(Value::Variant(Box::new(v))))).collect())
    }

    fn ifaces(auto: bool, system: bool, mounted: Option<&str>) -> Value {
        let pts = Value::Array("ay".into(), mounted.into_iter().map(|m| Value::Array("y".into(), m.bytes().chain([0]).map(Value::Byte).collect())).collect());
        Value::Array(
            "{sa{sv}}".into(),
            vec![
                Value::Entry(Box::new(Value::Str(BLOCK.into())), Box::new(dict(vec![("HintAuto", Value::Bool(auto)), ("HintSystem", Value::Bool(system)), ("HintIgnore", Value::Bool(false)), ("IdLabel", Value::Str("STICK".into()))]))),
                Value::Entry(Box::new(Value::Str(FS.into())), Box::new(dict(vec![("MountPoints", pts)]))),
            ],
        )
    }

    #[test]
    fn only_drives_meant_for_the_user_and_not_mounted_are_mounted() {
        assert!(wants_mount(&ifaces(true, false, None)));
        assert!(!wants_mount(&ifaces(false, false, None)), "not HintAuto");
        assert!(!wants_mount(&ifaces(true, true, None)), "a system disk");
        assert!(!wants_mount(&ifaces(true, false, Some("/run/media/u/STICK"))), "mounted already");
        assert_eq!(mount_points(&ifaces(true, false, Some("/run/media/u/STICK"))), vec!["/run/media/u/STICK"]);
        assert_eq!(label(&ifaces(true, false, None)), "STICK");
    }

    /// Against the real daemon where it runs: its objects can be listed.
    #[test]
    fn lists_udisks_objects_if_it_runs() {
        let Ok(mut c) = Client::system() else { return };
        if let Ok(all) = objects(&mut c) {
            assert!(all.iter().all(|(p, _)| p.starts_with(ROOT)));
        }
    }
}
