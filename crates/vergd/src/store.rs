//! The profile file: ~/.config/vergence/profiles.toml (or under
//! $XDG_CONFIG_HOME). Owner-only: the directory 0700, the file 0600
//! (whitepaper §7.3). Written atomically (a temp file in the same directory,
//! then renamed), so a crash never leaves half a file. A file that does not
//! parse is never overwritten: vergd stops and says why.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::PathBuf;

use vergence_profile::arrangement::Store;

pub fn path() -> Result<PathBuf, String> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => {
            let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
            PathBuf::from(home).join(".config")
        }
    };
    Ok(base.join("vergence").join("profiles.toml"))
}

pub fn load() -> Result<Store, String> {
    let p = path()?;
    match fs::read_to_string(&p) {
        Ok(text) => Store::from_toml(&text).map_err(|e| format!("{}: {e}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

pub fn save(store: &Store) -> Result<(), String> {
    let p = path()?;
    let dir = p.parent().ok_or("profile path has no directory")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    // Tighten a directory made earlier by someone else's umask.
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));

    let tmp = dir.join(format!(".profiles.toml.{}", std::process::id()));
    let text = format!(
        "# Vergence monitor-setup profiles. Written by vergd; safe to edit while\n\
         # vergd is stopped. See docs/milestone-2.md.\n\n{}",
        store.to_toml()
    );
    let write = || -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, &p)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("{}: {e}", p.display())
    })
}
