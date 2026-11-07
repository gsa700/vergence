//! vergd's files, under ~/.config/vergence (or $XDG_CONFIG_HOME/vergence):
//!
//! - profiles.toml: the monitor arrangement per setup (milestone 2)
//! - windows.toml:  where windows were per setup (milestone 3)
//! - settings.toml: the XSETTINGS vergd publishes (milestone 4)
//!
//! Owner-only: the directory 0700, the files 0600 (whitepaper §7.3).
//! Written atomically (a temp file in the same directory, then renamed), so
//! a crash never leaves half a file. A file that does not parse is never
//! overwritten: vergd stops and says why. vergd re-reads them before every
//! decision, so they can be edited at any time.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::PathBuf;

use vergence_profile::arrangement::Store;
use vergence_profile::windows::WindowStore;

fn dir() -> Result<PathBuf, String> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => {
            let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
            PathBuf::from(home).join(".config")
        }
    };
    Ok(base.join("vergence"))
}

pub fn path() -> Result<PathBuf, String> {
    Ok(dir()?.join("profiles.toml"))
}

pub fn windows_path() -> Result<PathBuf, String> {
    Ok(dir()?.join("windows.toml"))
}

pub fn settings_path() -> Result<PathBuf, String> {
    Ok(dir()?.join("settings.toml"))
}

/// The settings file's text, or None if there is none yet.
pub fn load_settings() -> Result<Option<String>, String> {
    read(&settings_path()?)
}

pub fn save_settings(body: &str) -> Result<(), String> {
    write(
        &settings_path()?,
        "# Vergence XSETTINGS: what vergd publishes to GTK apps (theme, icons,\n\
         # fonts, cursor...). Imported once from the desktop's settings manager.\n\
         # Edit freely: vergd re-reads this file within 2 s. Scaling values\n\
         # (window scale, DPI, cursor size) are laid over these by vergd.\n\
         # See docs/milestone-4.md.\n",
        body,
    )
}

/// When the settings file last changed, if it exists.
pub fn settings_mtime() -> Option<std::time::SystemTime> {
    fs::metadata(settings_path().ok()?).and_then(|m| m.modified()).ok()
}

/// The file's text, or None if it does not exist.
fn read(p: &PathBuf) -> Result<Option<String>, String> {
    match fs::read_to_string(p) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

fn write(p: &PathBuf, header: &str, body: &str) -> Result<(), String> {
    let dir = p.parent().ok_or("path has no directory")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    // Tighten a directory made earlier by someone else's umask.
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));

    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.{}", std::process::id()));
    let text = format!("{header}\n{body}");
    let go = || -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, p)
    };
    go().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("{}: {e}", p.display())
    })
}

pub fn load() -> Result<Store, String> {
    let p = path()?;
    match read(&p)? {
        Some(t) => Store::from_toml(&t).map_err(|e| format!("{}: {e}", p.display())),
        None => Ok(Store::default()),
    }
}

pub fn save(store: &Store) -> Result<(), String> {
    write(
        &path()?,
        "# Vergence monitor-setup profiles. Written by vergd, which re-reads this\n\
         # file before every change, so it can be edited at any time.\n\
         # See docs/milestone-2.md.\n",
        &store.to_toml(),
    )
}

pub fn load_windows() -> Result<WindowStore, String> {
    let p = windows_path()?;
    match read(&p)? {
        Some(t) => WindowStore::from_toml(&t).map_err(|e| format!("{}: {e}", p.display())),
        None => Ok(WindowStore::default()),
    }
}

pub fn save_windows(store: &WindowStore) -> Result<(), String> {
    write(
        &windows_path()?,
        "# Vergence window memory: where each window was on each monitor setup.\n\
         # No window titles are stored. Add apps vergd should leave alone to\n\
         # skip = [\"...\"] (instance or class, any case). vergd re-reads this\n\
         # file before every change. See docs/milestone-3.md.\n",
        &store.to_toml(),
    )
}
