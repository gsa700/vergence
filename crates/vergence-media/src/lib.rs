//! Removable drives (files milestone 1, step 10; Dave, 2026-11-15: "yes,
//! udisks2 on Linux and automounter on kepler"): a USB stick or card is
//! mounted when it is plugged in, so the desktop and File Viewer show it,
//! and Eject lets go of it.
//!
//! Linux: the udisks2 system daemon, over the system bus (our own client,
//! vergence-dbus): [`automount`] watches for new file systems and mounts
//! those udisks says are meant to be (HintAuto, not HintIgnore or
//! HintSystem), under /run/media/<user>; [`eject`] unmounts and powers the
//! drive off. Permission is udisks's (polkit: the active local session may).
//! FreeBSD: the base system's automounter (autofs, the `-media` map under
//! /media); see freebsd.rs.

use std::time::Duration;

#[cfg(target_os = "linux")]
mod udisks;
#[cfg(target_os = "freebsd")]
mod freebsd;
#[cfg(target_os = "freebsd")]
pub use freebsd::HELPER;

/// Watches for drives plugged in and mounts them, for ever (run it on a
/// thread of its own). `told` hears what happened, for the log.
pub fn automount(told: impl Fn(&str) + Send + 'static) {
    #[cfg(target_os = "linux")]
    udisks::automount(&told);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = &told;
    }
}

/// Lets go of the drive mounted at `path`: Ok(true) if done, Ok(false) if
/// it is not a drive this knows about (the caller unmounts it otherwise).
pub fn eject(path: &std::path::Path) -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    return udisks::eject(path);
    #[cfg(target_os = "freebsd")]
    return freebsd::eject(path);
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        let _ = path;
        Ok(false)
    }
}

/// Whether `arg` is a path the eject helper may unmount: `/media/<name>`,
/// one plain name (letters, digits, `._-` and spaces; no `..`).
pub fn helper_may_eject(arg: &str) -> bool {
    let Some(name) = arg.strip_prefix("/media/") else { return false };
    !name.is_empty() && name.len() <= 255 && name != "." && name != ".." && name.chars().all(|c| c.is_ascii_alphanumeric() || "._- ".contains(c))
}

/// The eject helper's whole job (FreeBSD; setuid root, runnable by the
/// video group only): unmount the one path it is given, if it is
/// `/media/<name>` and is mounted there now. It trusts nothing: one
/// argument, checked; no environment read; only /sbin/mount -p and
/// /sbin/umount run. Returns the exit status.
pub fn helper_main(args: Vec<std::ffi::OsString>) -> i32 {
    let fail = |why: &str| {
        eprintln!("media-eject: {why}");
        1
    };
    if !cfg!(target_os = "freebsd") {
        return fail("FreeBSD only (Linux ejects through udisks2)");
    }
    let [arg] = args.as_slice() else { return fail("usage: media-eject /media/NAME") };
    let Some(arg) = arg.to_str().filter(|a| helper_may_eject(a)) else { return fail("only a drive under /media is ejected here") };
    let table = std::process::Command::new("/sbin/mount").arg("-p").env_clear().output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let mounted = table.lines().any(|l| l.split_whitespace().nth(1).map(|m| m.replace("\\040", " ")) == Some(arg.to_string()));
    if !mounted {
        // Listed but never looked into: nothing is mounted, nothing to let go of.
        return 0;
    }
    match std::process::Command::new("/sbin/umount").arg(arg).env_clear().status() {
        Ok(s) if s.success() => 0,
        Ok(_) => fail(&format!("{arg} could not be unmounted (it may be busy)")),
        Err(e) => fail(&e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_helper_takes_only_a_plain_name_under_media() {
        for ok in ["/media/VTEST", "/media/da0s1", "/media/My Stick", "/media/a.b-c_d"] {
            assert!(super::helper_may_eject(ok), "{ok}");
        }
        for bad in ["/media", "/media/", "/media/..", "/media/a/b", "/mnt/x", "/media/x;rm", "/media/$(id)", "/home/u", "media/x", "/media/\u{e9}"] {
            assert!(!super::helper_may_eject(bad), "{bad}");
        }
    }
}

/// How long a call to the daemon may take (mounting a large drive with a
/// file system check can take a while).
#[allow(dead_code)]
const CALL: Duration = Duration::from_secs(30);
