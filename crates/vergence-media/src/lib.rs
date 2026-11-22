//! Removable drives (files milestone 1, step 10; Dave, 2026-11-15: "yes,
//! udisks2 on Linux and automounter on kepler"): a USB stick or card is
//! mounted when it is plugged in, so the desktop and File Viewer show it,
//! and Eject lets go of it.
//!
//! Linux: the udisks2 system daemon, over the system bus (our own client,
//! vergence-dbus): [`automount`] watches for new file systems and mounts
//! those udisks says are meant to be (HintAuto, not HintIgnore or
//! HintSystem), under /run/media/<user>, noexec (and udisks's nosuid,
//! nodev); [`eject`] unmounts and powers the
//! drive off. Permission is udisks's (polkit: the active local session may).
//! FreeBSD: the base system's automounter (autofs, the `-media` map under
//! /media); see freebsd.rs.

use std::time::Duration;

pub mod drives;
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

/// One plain name: letters, digits, `._-` and spaces; not `.` or `..`.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 255 && name != "." && name != ".." && name.chars().all(|c| c.is_ascii_alphanumeric() || "._- ".contains(c))
}

/// Whether `arg` is a path the eject helper may unmount: `/media/<name>`
/// (a drive), or `/mnt/<name>` (a disk image it attached), one plain name.
pub fn helper_may_eject(arg: &str) -> bool {
    arg.strip_prefix("/media/").or_else(|| arg.strip_prefix(IMAGES)).is_some_and(plain_name)
}

/// Where disk images are mounted on FreeBSD (`/media` is the automounter's).
pub const IMAGES: &str = "/mnt/";

/// The folder a disk image is mounted at: its name without the extension,
/// made plain (anything else a `_`), under [`IMAGES`].
pub fn image_mount_point(image: &std::path::Path) -> Option<String> {
    let stem = image.file_stem()?.to_string_lossy();
    let name: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || "._- ".contains(c) { c } else { '_' }).take(64).collect();
    let name = name.trim_matches([' ', '.']).to_string();
    plain_name(&name).then(|| format!("{IMAGES}{name}"))
}

/// Whether a file starts as an ISO 9660 image does: "CD001" in the first
/// volume descriptor, at byte 32769.
pub fn is_iso9660(head: &[u8]) -> bool {
    head.len() >= 32774 && &head[32769..32774] == b"CD001"
}

/// Mounts the disk image `image` (an ISO), read-only and not to run
/// programs from: where it is mounted, or why not. Linux: udisks2 (a loop
/// device set up by `udisksctl`, then mounted as a drive is); FreeBSD: the
/// root helper's `attach` (mdconfig, mount_cd9660 under /mnt).
pub fn mount_image(image: &std::path::Path) -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "linux")]
    return udisks::mount_image(image);
    #[cfg(target_os = "freebsd")]
    return freebsd::mount_image(image);
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        let _ = image;
        Err("disk images are mounted on Linux and FreeBSD only".into())
    }
}

/// The eject helper's whole job (FreeBSD; setuid root, runnable by the
/// video group only): unmount the one path it is given, if it is
/// `/media/<name>` and is mounted there now. It trusts nothing: one
/// argument, checked; no environment read; only /sbin/mount -p and
/// /sbin/umount run. Returns the exit status.
///
/// Its second job (files milestone 3, step 7; Dave, 2026-11-21: "in
/// downloads I have an iso, can i mount it"): `media-eject attach IMAGE`
/// attaches an ISO image the calling user owns (a regular file, an ISO 9660
/// signature) read-only with mdconfig and mounts it at `/mnt/<name>`
/// read-only, nosuid and noexec, printing where. An image's mount point is
/// ejected as a drive's is, and then its md device is detached and the
/// folder taken away; only mounts of md devices under /mnt, never anything
/// else there.
pub fn helper_main(args: Vec<std::ffi::OsString>) -> i32 {
    let fail = |why: &str| {
        eprintln!("media-eject: {why}");
        1
    };
    if !cfg!(target_os = "freebsd") {
        return fail("FreeBSD only (Linux ejects through udisks2)");
    }
    let run = |program: &str, args: &[&str]| std::process::Command::new(program).args(args).env_clear().output();
    if let [verb, image] = args.as_slice()
        && verb.to_str() == Some("attach")
    {
        return match attach(image, &run) {
            Ok(at) => {
                println!("{at}");
                0
            }
            Err(why) => fail(&why),
        };
    }
    let [arg] = args.as_slice() else { return fail("usage: media-eject /media/NAME, or media-eject attach IMAGE") };
    let Some(arg) = arg.to_str().filter(|a| helper_may_eject(a)) else { return fail("only a drive under /media or an image under /mnt is ejected here") };
    let table = run("/sbin/mount", &["-p"]).map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let device = table.lines().find_map(|l| {
        let mut f = l.split_whitespace();
        let dev = f.next()?;
        (f.next().map(|m| m.replace("\\040", " ")) == Some(arg.to_string())).then(|| dev.to_string())
    });
    let Some(device) = device else {
        // Listed but never looked into: nothing is mounted; the drive is
        // ejected all the same (found by its name or its label).
        if let Some(disk) = std::path::Path::new(arg).file_name().and_then(drives::device_of).and_then(|node| disk_of(&node.to_string_lossy(), &run)) {
            eject_disk(&disk, &table, &run);
        }
        return 0;
    };
    // Under /mnt: only an image this attached (an md device), nothing else mounted there.
    let md_unit = device.strip_prefix("/dev/md").filter(|u| !u.is_empty() && u.chars().all(|c| c.is_ascii_digit())).map(str::to_string);
    if arg.starts_with(IMAGES) && md_unit.is_none() {
        return fail("only a disk image is ejected from /mnt here");
    }
    match run("/sbin/umount", &[arg]) {
        Ok(o) if o.status.success() => {}
        Ok(_) => return fail(&format!("{arg} could not be unmounted (it may be busy)")),
        Err(e) => return fail(&e.to_string()),
    }
    if let (Some(unit), true) = (md_unit, arg.starts_with(IMAGES)) {
        let _ = run("/sbin/mdconfig", &["-d", "-u", &unit]);
        let _ = std::fs::remove_dir(arg);
        return 0;
    }
    // A drive: the whole disk let go of, as a Mac ejects (Dave, 2026-11-21:
    // an ejected stick unmounted only was mounted again by the next look,
    // and came back under its device's name).
    if let Some(disk) = disk_of(&device, &run) {
        eject_disk(&disk, &table, &run);
    }
    0
}

/// The removable disk a device node is on: "/dev/da0s1" and the label
/// "/dev/msdosfs/USB" (by `glabel status`) are on "da0". Only da, cd and
/// sdda disks (CAM's, which camcontrol ejects).
fn disk_of(node: &str, run: &dyn Fn(&str, &[&str]) -> std::io::Result<std::process::Output>) -> Option<String> {
    let labels = run("/sbin/glabel", &["status", "-s"]).map(|o| drives::parse_glabel(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default();
    removable_disk(drives::provider_of(node, &labels))
}

/// "da0s1", "da0p2", "cd0" → the disk ("da0", "cd0"), if it is a removable one.
fn removable_disk(provider: &str) -> Option<String> {
    let kind: String = provider.chars().take_while(|c| c.is_ascii_lowercase()).collect();
    let unit: String = provider[kind.len()..].chars().take_while(|c| c.is_ascii_digit()).collect();
    (matches!(kind.as_str(), "da" | "cd" | "sdda") && !unit.is_empty()).then(|| format!("{kind}{unit}"))
}

/// Unmounts what else of the disk is mounted under /media (its other
/// partitions), then ejects it: its partitions and labels leave /dev, so
/// nothing mounts it again until it is plugged in again. A disk that will
/// not eject stays unmounted (the server's ejected note hides it).
fn eject_disk(disk: &str, table: &str, run: &dyn Fn(&str, &[&str]) -> std::io::Result<std::process::Output>) {
    for l in table.lines() {
        let mut f = l.split_whitespace();
        let (Some(dev), Some(at)) = (f.next(), f.next()) else { continue };
        let at = at.replace("\\040", " ");
        let on_disk = dev.strip_prefix("/dev/").and_then(removable_disk).as_deref() == Some(disk);
        if on_disk && helper_may_eject(&at) && at.starts_with("/media/") {
            let _ = run("/sbin/umount", &[&at]);
        }
    }
    let _ = run("/sbin/camcontrol", &["eject", disk]);
}

/// The attach: the checks, mdconfig, the mount point, mount_cd9660; the md
/// device let go of again if the mount fails.
fn attach(image: &std::ffi::OsStr, run: &dyn Fn(&str, &[&str]) -> std::io::Result<std::process::Output>) -> Result<String, String> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    // The checks are made on the file as opened (its owner, its kind, its
    // first bytes), and after mdconfig the path must still lead to that same
    // file (the user owns the folder it is in and could put something else
    // there meanwhile). What is left, a swap and a swap back in between,
    // could attach only another ISO 9660 image, and /dev/mdN stays the
    // system's (root:operator 0640).
    let path = std::fs::canonicalize(image).map_err(|e| format!("the image: {e}"))?;
    let mut file = std::fs::File::open(&path).map_err(|e| format!("the image: {e}"))?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    // SAFETY: getuid has no arguments and cannot fail.
    let uid = unsafe { getuid() };
    if !meta.is_file() {
        return Err("not a file".into());
    }
    if meta.uid() != uid {
        return Err("only an image of your own is mounted here".into());
    }
    let mut head = vec![0u8; 32774];
    file.read_exact(&mut head).map_err(|_| "not an ISO image".to_string())?;
    if !is_iso9660(&head) {
        return Err("not an ISO image (only ISO 9660 images are mounted)".into());
    }
    let at = image_mount_point(&path).ok_or("the image's name makes no folder name")?;
    let table = run("/sbin/mount", &["-p"]).map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    if table.lines().any(|l| l.split_whitespace().nth(1).map(|m| m.replace("\\040", " ")) == Some(at.clone())) {
        return Err(format!("something is mounted at {at} already"));
    }
    let p = path.to_str().ok_or("the image's path is not text")?;
    let out = run("/sbin/mdconfig", &["-a", "-t", "vnode", "-o", "readonly", "-f", p]).map_err(|e| e.to_string())?;
    let unit = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || !unit.starts_with("md") {
        return Err(format!("mdconfig: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let undo = || {
        let _ = run("/sbin/mdconfig", &["-d", "-u", unit.trim_start_matches("md")]);
    };
    let same = std::fs::metadata(&path).is_ok_and(|m| m.dev() == meta.dev() && m.ino() == meta.ino());
    if !same {
        undo();
        return Err("the image changed while it was being attached".into());
    }
    if let Err(e) = std::fs::create_dir(&at)
        && e.kind() != std::io::ErrorKind::AlreadyExists
    {
        undo();
        return Err(format!("{at}: {e}"));
    }
    match run("/sbin/mount_cd9660", &["-o", "ro,nosuid,noexec", &format!("/dev/{unit}"), &at]) {
        Ok(o) if o.status.success() => Ok(at),
        Ok(o) => {
            undo();
            let _ = std::fs::remove_dir(&at);
            Err(format!("mount_cd9660: {}", String::from_utf8_lossy(&o.stderr).trim()))
        }
        Err(e) => {
            undo();
            let _ = std::fs::remove_dir(&at);
            Err(e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_helper_takes_only_a_plain_name_under_media() {
        for ok in ["/media/VTEST", "/media/da0s1", "/media/My Stick", "/media/a.b-c_d"] {
            assert!(super::helper_may_eject(ok), "{ok}");
        }
        for bad in ["/media", "/media/", "/media/..", "/media/a/b", "/mnt", "/mnt/", "/mnt/a/b", "/mnt/..", "/media/x;rm", "/media/$(id)", "/home/u", "media/x", "/media/\u{e9}"] {
            assert!(!super::helper_may_eject(bad), "{bad}");
        }
        assert!(super::helper_may_eject("/mnt/Fedora-45"), "an image's mount point");
    }

    /// Against the real system, given an ISO in VERGENCE_TEST_ISO: mounted
    /// (its file readable), then ejected (gone from the mount table).
    #[test]
    fn mounts_and_ejects_a_real_image_if_given_one() {
        let Some(iso) = std::env::var_os("VERGENCE_TEST_ISO") else { return };
        let at = super::mount_image(std::path::Path::new(&iso)).expect("mounted");
        eprintln!("mounted at {at:?}: {:?}", std::fs::read_dir(&at).map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>()));
        assert!(std::fs::read_dir(&at).is_ok_and(|mut d| d.next().is_some()), "something in {at:?}");
        assert_eq!(super::eject(&at), Ok(true));
        let table = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
        assert!(!table.contains(&*at.to_string_lossy()), "unmounted");
    }

    #[test]
    fn a_partition_is_on_its_removable_disk() {
        assert_eq!(super::removable_disk("da0s1").as_deref(), Some("da0"));
        assert_eq!(super::removable_disk("da12p3").as_deref(), Some("da12"));
        assert_eq!(super::removable_disk("cd0").as_deref(), Some("cd0"));
        assert_eq!(super::removable_disk("sdda0s1").as_deref(), Some("sdda0"));
        for no in ["ada0p2", "nda0p1", "md0", "da", "msdosfs/USB", ""] {
            assert_eq!(super::removable_disk(no), None, "{no}");
        }
    }

    #[test]
    fn an_image_mounts_at_a_plain_name_and_must_be_iso9660() {
        use std::path::Path;
        assert_eq!(super::image_mount_point(Path::new("/home/u/Downloads/Fedora-Everything-netinst-x86_64-45-1.4.iso")).as_deref(), Some("/mnt/Fedora-Everything-netinst-x86_64-45-1.4"));
        assert_eq!(super::image_mount_point(Path::new("/x/we$ird;name.iso")).as_deref(), Some("/mnt/we_ird_name"));
        assert_eq!(super::image_mount_point(Path::new("/x/...iso")), None);
        let mut head = vec![0u8; 32774];
        assert!(!super::is_iso9660(&head));
        head[32769..32774].copy_from_slice(b"CD001");
        assert!(super::is_iso9660(&head));
        assert!(!super::is_iso9660(&head[..100]));
    }
}

/// How long a call to the daemon may take (mounting a large drive with a
/// file system check can take a while).
#[allow(dead_code)]
const CALL: Duration = Duration::from_secs(30);
