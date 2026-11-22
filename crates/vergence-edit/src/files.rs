//! Reading and writing the files the editor edits (editor milestone 1).
//!
//! - **Reading**: UTF-8, with a byte-order mark taken off and remembered. A
//!   file that is not UTF-8 opens read-only with a notice, for now (other
//!   encodings may come later; see docs/editor-milestone-1.md): its text
//!   shown as well as it can be, never written back.
//! - **Writing never leaves half a file**: the text goes to a new file
//!   beside the old one, is flushed to the disk, and is then renamed over
//!   it, so a crash or a full disk leaves the old file whole. The file's
//!   permissions are kept; a link is followed, so the link stays a link.
//!   Where the folder cannot take a new file but the file itself can be
//!   written (a file in a folder owned by someone else), it is written in
//!   place, as other editors do.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The largest file opened, in bytes.
pub const MAX_BYTES: u64 = 256 * 1024 * 1024;
const BOM: &str = "\u{feff}";

/// A file read in.
#[derive(Debug, PartialEq)]
pub struct Loaded {
    pub text: String,
    /// It began with a byte-order mark (written back the same way).
    pub bom: bool,
    /// Why it is open read-only, if it is.
    pub read_only: Option<String>,
}

/// Reads `path`. A file that does not exist is an empty new one.
pub fn load(path: &Path) -> Result<Loaded, String> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Loaded { text: String::new(), bom: false, read_only: None });
        }
        Err(e) => return Err(format!("{} could not be opened: {}", name(path), plain(&e))),
    };
    if meta.is_dir() {
        return Err(format!("{} is a folder.", name(path)));
    }
    if !meta.is_file() {
        return Err(format!("{} is not a file the editor can open (a pipe, a device or a socket).", name(path)));
    }
    let too_large = |len: u64| {
        format!(
            "{} is too large to open ({} MB; the most is {} MB).",
            name(path),
            len / (1024 * 1024),
            MAX_BYTES / (1024 * 1024)
        )
    };
    if meta.len() > MAX_BYTES {
        return Err(too_large(meta.len()));
    }
    // Checked again on what is opened: the path could have changed since.
    let bytes = match vergence_types::read_regular(path, MAX_BYTES) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::FileTooLarge => return Err(too_large(MAX_BYTES + 1)),
        Err(e) => return Err(format!("{} could not be read: {}", name(path), plain(&e))),
    };
    let writable = !meta.permissions().readonly() && writable(path);
    match String::from_utf8(bytes) {
        Ok(mut text) => {
            let bom = text.starts_with(BOM);
            if bom {
                text.drain(..BOM.len());
            }
            let read_only = (!writable).then(|| "You can read this file but not change it.".to_string());
            Ok(Loaded { text, bom, read_only })
        }
        Err(e) => {
            let text = String::from_utf8_lossy(e.as_bytes()).into_owned();
            Ok(Loaded {
                text,
                bom: false,
                read_only: Some("This file is not UTF-8 text, so it is open read-only.".into()),
            })
        }
    }
}

/// Whether the user may write `path` (access(2) with W_OK).
fn writable(path: &Path) -> bool {
    unsafe extern "C" {
        fn access(path: *const std::ffi::c_char, mode: i32) -> i32;
    }
    const W_OK: i32 = 2;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else { return false };
    // SAFETY: access(2) reads a NUL-terminated path that lives through the call.
    unsafe { access(c.as_ptr(), W_OK) == 0 }
}

/// Writes `text` to `path` (with a byte-order mark if `bom`), whole or not at all.
pub fn save(path: &Path, text: &str, bom: bool) -> Result<(), String> {
    // A link: write what it points at, so the link stays.
    let target: PathBuf = match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            fs::canonicalize(path).map_err(|e| format!("{} points nowhere: {}", name(path), plain(&e)))?
        }
        _ => path.to_path_buf(),
    };
    let old = fs::metadata(&target).ok();
    if old.as_ref().is_some_and(|m| m.is_dir()) {
        return Err(format!("{} is a folder.", name(path)));
    }
    let dir = target.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let file_name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{file_name}.vergence-{}.tmp", std::process::id()));
    let mut bytes = Vec::with_capacity(text.len() + 3);
    if bom {
        bytes.extend_from_slice(BOM.as_bytes());
    }
    bytes.extend_from_slice(text.as_bytes());
    // New files get the usual 0666 less the umask; old ones keep their mode.
    let mode = old.as_ref().map_or(0o666, |m| m.permissions().mode() & 0o7777);
    let made = OpenOptions::new().write(true).create_new(true).mode(mode).open(&tmp);
    let mut f = match made {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && old.is_some() && writable(&target) => {
            return in_place(&target, &bytes).map_err(|e| format!("{} could not be saved: {}", name(path), plain(&e)));
        }
        Err(e) => return Err(format!("{} could not be saved: {}", name(path), plain(&e))),
    };
    let written = f.write_all(&bytes).and_then(|_| f.sync_all()).and_then(|_| {
        // The mode exactly (the umask took bits off at creation).
        if old.is_some() { fs::set_permissions(&tmp, fs::Permissions::from_mode(mode)) } else { Ok(()) }
    });
    drop(f);
    if let Err(e) = written.and_then(|_| fs::rename(&tmp, &target)) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{} could not be saved: {}", name(path), plain(&e)));
    }
    // The rename itself to the disk.
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Writes over the file itself, where its folder takes no new file.
fn in_place(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = OpenOptions::new().write(true).truncate(true).open(target)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// A file's name as shown: its last part.
pub fn name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
}

/// An error in words, without the "(os error 13)".
fn plain(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "you do not have permission.".into(),
        std::io::ErrorKind::NotFound => "its folder does not exist.".into(),
        std::io::ErrorKind::StorageFull => "the disk is full.".into(),
        std::io::ErrorKind::ReadOnlyFilesystem => "the disk is read-only.".into(),
        _ => {
            let s = e.to_string();
            let s = s.split(" (os error").next().unwrap_or(&s).to_string();
            format!("{}.", s.trim_end_matches('.'))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vergence-edit-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).expect("a scratch folder");
        d
    }

    #[test]
    fn text_is_read_and_written_back_with_its_mark_and_mode() {
        let d = scratch("roundtrip");
        let p = d.join("a.txt");
        fs::write(&p, "\u{feff}one\r\ntwo\r\n").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o640)).unwrap();
        let l = load(&p).unwrap();
        assert_eq!((l.text.as_str(), l.bom, l.read_only.is_none()), ("one\r\ntwo\r\n", true, true));
        save(&p, "changed\r\n", l.bom).unwrap();
        assert_eq!(fs::read(&p).unwrap(), "\u{feff}changed\r\n".as_bytes());
        assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o640, "the mode is kept");
        // Nothing left beside it.
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_link_stays_a_link() {
        let d = scratch("link");
        let real = d.join("real.txt");
        fs::write(&real, "old").unwrap();
        let link = d.join("link.txt");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        save(&link, "new", false).unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "new");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_new_file_a_folder_and_text_that_is_not_utf8() {
        let d = scratch("odd");
        let new = d.join("new.txt");
        assert_eq!(load(&new).unwrap(), Loaded { text: String::new(), bom: false, read_only: None });
        save(&new, "hello", false).unwrap();
        assert_eq!(fs::read_to_string(&new).unwrap(), "hello");
        assert!(load(&d).unwrap_err().contains("is a folder"));
        let latin = d.join("latin1.txt");
        fs::write(&latin, b"caf\xe9").unwrap();
        let l = load(&latin).unwrap();
        assert_eq!(l.text, "caf\u{fffd}");
        assert!(l.read_only.unwrap().contains("not UTF-8"));
        assert!(save(&d.join("missing/x.txt"), "x", false).unwrap_err().contains("folder does not exist"));
        fs::remove_dir_all(&d).unwrap();
    }
}
