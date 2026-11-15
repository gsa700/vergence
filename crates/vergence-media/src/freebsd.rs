// FreeBSD: the base system's automounter (autofs). install.sh turns it on
// and points /media at tools/freebsd/media-map, installed as the special map /etc/autofs/special_vergence_media (FreeBSD's -media map, FAT
// drives writable by the video group); a drive plugged in shows in /media,
// by its label, and is mounted when it is first looked into. Eject is the
// root helper `media-eject` (setuid root, the video group only, as
// sound-default is): unmounting needs root (vfs.usermount is 0).

use std::path::{Path, PathBuf};

pub const HELPER: &str = "media-eject";

fn helper_path() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.parent()?.join("libexec/vergence").join(HELPER)));
    beside.into_iter().chain([PathBuf::from("/usr/local/libexec/vergence").join(HELPER)]).find(|p| p.is_file())
}

pub fn eject(path: &Path) -> Result<bool, String> {
    if !path.starts_with("/media") || path == Path::new("/media") {
        return Ok(false);
    }
    let helper = helper_path().ok_or("the eject helper (libexec/vergence/media-eject) is not installed; sudo tools/install.sh puts it there")?;
    let out = std::process::Command::new(helper).arg(path).env_clear().output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied { "only members of the video group may eject (sudo pw groupmod video -m $USER, then log in again)".to_string() } else { e.to_string() }
    })?;
    if out.status.success() {
        return Ok(true);
    }
    let said = String::from_utf8_lossy(&out.stderr);
    let said = said.trim().trim_start_matches("media-eject: ");
    Err(if said.contains("busy") { "something is still using it (a window showing it, or a file open).".into() } else { said.to_string() })
}
