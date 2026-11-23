//! Making archives through libarchive: Compress's .zip of the person's own
//! files (archive milestone 1, step 5), and [`write_raw`], entries exactly
//! as given (hostile names included) for the tests. The files are the
//! person's own, not untrusted input, so this runs in the app, on its
//! worker.

use std::ffi::CString;
use std::fs;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use crate::ffi;
use crate::unpack::Progress;

// The third field is held for its Drop: the thread's locale put back after the archive is freed.
struct Writer(*mut ffi::Archive, *mut ffi::Entry, #[allow(dead_code)] Utf8Thread);

/// This thread's character set made UTF-8 while it writes an archive, and
/// put back after. libarchive's writers pass names through the locale's
/// character set even when given them as UTF-8, and a Rust program runs in
/// the plain C locale: on FreeBSD "é" became "?" in a .zip. Per thread
/// (uselocale), so the rest of the app is untouched.
struct Utf8Thread(*mut std::ffi::c_void, *mut std::ffi::c_void);

unsafe extern "C" {
    fn newlocale(
        mask: std::ffi::c_int,
        name: *const std::ffi::c_char,
        base: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
    fn uselocale(l: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn freelocale(l: *mut std::ffi::c_void);
}

#[cfg(target_os = "freebsd")]
const LC_CTYPE_MASK: std::ffi::c_int = 1 << 1;
#[cfg(not(target_os = "freebsd"))]
const LC_CTYPE_MASK: std::ffi::c_int = 1 << 0;

impl Utf8Thread {
    fn new() -> Utf8Thread {
        for name in [c"C.UTF-8", c"C.utf8"] {
            // SAFETY: newlocale(3) with a NUL-terminated name and no base.
            let l = unsafe { newlocale(LC_CTYPE_MASK, name.as_ptr(), std::ptr::null_mut()) };
            if !l.is_null() {
                // SAFETY: a locale newlocale made; uselocale returns the old one.
                return Utf8Thread(l, unsafe { uselocale(l) });
            }
        }
        Utf8Thread(std::ptr::null_mut(), std::ptr::null_mut())
    }
}

impl Drop for Utf8Thread {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the old locale back on this thread, then ours freed (no
            // longer in use).
            unsafe {
                uselocale(self.1);
                freelocale(self.0);
            }
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // SAFETY: the archive and entry made in Writer::new, freed once.
        unsafe {
            ffi::archive_entry_free(self.1);
            ffi::archive_write_free(self.0);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    TarGz,
}

fn c(b: &[u8]) -> io::Result<CString> {
    CString::new(b).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a name with a NUL in it"))
}

impl Writer {
    fn new(to: &Path, format: Format) -> io::Result<Writer> {
        let utf8 = Utf8Thread::new();
        // SAFETY: archive_write_new and archive_entry_new take nothing; null is checked.
        let (a, e) = unsafe { (ffi::archive_write_new(), ffi::archive_entry_new()) };
        let w = Writer(a, e, utf8);
        if a.is_null() || e.is_null() {
            return Err(io::Error::other("libarchive could not start"));
        }
        let path = c(to.as_os_str().as_bytes())?;
        // SAFETY: a live archive; the strings outlive the calls.
        let ok = unsafe {
            match format {
                Format::Zip => {
                    ffi::archive_write_set_format_zip(a) == ffi::OK
                        && ffi::archive_write_set_options(a, c"zip:hdrcharset=UTF-8".as_ptr()) == ffi::OK
                }
                Format::TarGz => {
                    ffi::archive_write_set_format_pax_restricted(a) == ffi::OK
                        && ffi::archive_write_add_filter_gzip(a) == ffi::OK
                }
            }
        } && unsafe { ffi::archive_write_open_filename(a, path.as_ptr()) } == ffi::OK;
        if !ok {
            // SAFETY: a live archive.
            return Err(io::Error::other(unsafe { ffi::error(a, "the archive could not be started") }));
        }
        Ok(w)
    }

    /// One entry's header: its name (UTF-8 when `utf8`, else as given),
    /// kind bits, permissions, time, size, and a link target.
    #[allow(clippy::too_many_arguments)]
    fn header(
        &mut self,
        name: &[u8],
        utf8: bool,
        kind: u32,
        perm: u32,
        mtime: i64,
        size: i64,
        symlink: Option<&[u8]>,
        hardlink: Option<&[u8]>,
    ) -> io::Result<()> {
        let n = c(name)?;
        let (s, h) = (symlink.map(c).transpose()?, hardlink.map(c).transpose()?);
        // SAFETY: the live entry and archive; the strings outlive the calls
        // (libarchive copies them).
        let r = unsafe {
            ffi::archive_entry_clear(self.1);
            if utf8 {
                ffi::archive_entry_set_pathname_utf8(self.1, n.as_ptr());
            } else {
                ffi::archive_entry_set_pathname(self.1, n.as_ptr());
            }
            ffi::archive_entry_set_filetype(self.1, kind);
            ffi::archive_entry_set_perm(self.1, (perm & 0o7777) as ffi::Mode);
            ffi::archive_entry_set_mtime(self.1, mtime, 0);
            ffi::archive_entry_set_size(self.1, size);
            if let Some(s) = &s {
                if utf8 {
                    ffi::archive_entry_set_symlink_utf8(self.1, s.as_ptr());
                } else {
                    ffi::archive_entry_set_symlink(self.1, s.as_ptr());
                }
            }
            if let Some(h) = &h {
                ffi::archive_entry_set_hardlink(self.1, h.as_ptr());
            }
            ffi::archive_write_header(self.0, self.1)
        };
        if r < ffi::WARN {
            // SAFETY: a live archive.
            return Err(io::Error::other(unsafe { ffi::error(self.0, "an entry could not be written") }));
        }
        Ok(())
    }

    fn data(&mut self, b: &[u8]) -> io::Result<()> {
        // SAFETY: a live archive and b.len() bytes.
        let n = unsafe { ffi::archive_write_data(self.0, b.as_ptr().cast(), b.len()) };
        if n < 0 || n as usize != b.len() {
            // SAFETY: a live archive.
            return Err(io::Error::other(unsafe { ffi::error(self.0, "the archive could not be written") }));
        }
        Ok(())
    }

    fn close(self) -> io::Result<()> {
        // SAFETY: a live archive; it is freed by Drop after.
        if unsafe { ffi::archive_write_close(self.0) } != ffi::OK {
            // SAFETY: a live archive.
            return Err(io::Error::other(unsafe { ffi::error(self.0, "the archive could not be finished") }));
        }
        Ok(())
    }
}

/// An entry for [`write_raw`]: written exactly as given.
#[derive(Debug, Clone, Default)]
pub struct Raw {
    pub name: Vec<u8>,
    /// ffi::IFREG, IFDIR or IFLNK bits (0 means a file); a hard link has
    /// `hardlink` set.
    pub kind: u32,
    pub perm: u32,
    pub data: Vec<u8>,
    pub symlink: Option<Vec<u8>>,
    pub hardlink: Option<Vec<u8>>,
    /// The size said in the header, if not the data's.
    pub said: Option<i64>,
}

/// An archive of `entries` as given, at `to` (the tests' hostile archives).
pub fn write_raw(to: &Path, format: Format, entries: &[Raw]) -> io::Result<()> {
    let mut w = Writer::new(to, format)?;
    for r in entries {
        let kind = if r.kind == 0 { ffi::IFREG } else { r.kind };
        let size = r.said.unwrap_or(if kind == ffi::IFREG { r.data.len() as i64 } else { 0 });
        // A zip takes UTF-8 names as UTF-8 (a raw name is read in the
        // program's locale, plain C in a test, which turns "é" into question
        // marks). A tar takes them raw: libarchive 3.8's pax writer crashes
        // on a non-ASCII UTF-8 name in the C locale (it cannot make the
        // locale's form and uses the missing one); the raw name it stores
        // as bytes, which is what a test wants anyway.
        let utf8 = format == Format::Zip && std::str::from_utf8(&r.name).is_ok();
        w.header(&r.name, utf8, kind, r.perm, 0, size, r.symlink.as_deref(), r.hardlink.as_deref())?;
        if !r.data.is_empty() {
            w.data(&r.data)?;
        }
    }
    w.close()
}

/// The bytes Compress will read from `items`: their files' sizes added up
/// (links not followed).
pub fn size_of(items: &[PathBuf]) -> u64 {
    fn walk(p: &Path) -> u64 {
        match p.symlink_metadata() {
            Ok(m) if m.is_dir() => {
                fs::read_dir(p).map(|d| d.filter_map(Result::ok).map(|e| walk(&e.path())).sum()).unwrap_or(0)
            }
            Ok(m) if m.is_file() => m.len(),
            _ => 0,
        }
    }
    items.iter().map(|p| walk(p)).sum()
}

/// A .zip of `items` (all in one folder) at `to`, which must not exist yet:
/// folders with what is in them, files, symbolic links as links (never
/// followed); anything else left out. Written beside `to` under a hidden
/// name and linked into place, so a stop or an error leaves nothing and an
/// item made at `to` meanwhile is never replaced.
pub fn compress(items: &[PathBuf], to: &Path, progress: &Progress) -> io::Result<()> {
    let dir = to.parent().ok_or_else(|| io::Error::other("no folder"))?;
    let part = dir.join(format!(".vergence-compress-{}-{}", std::process::id(), to_name(to)));
    let r = (|| {
        let mut w = Writer::new(&part, Format::Zip)?;
        for item in items {
            let name = item.file_name().ok_or_else(|| io::Error::other("an item with no name"))?;
            add(&mut w, item, name.as_bytes(), progress)?;
        }
        w.close()?;
        // A hard link fails if `to` is there: never a replacement.
        fs::hard_link(&part, to)
    })();
    let _ = fs::remove_file(&part);
    r
}

fn to_name(to: &Path) -> String {
    to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn add(w: &mut Writer, path: &Path, name: &[u8], progress: &Progress) -> io::Result<()> {
    if progress.stop.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
    }
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let m = path.symlink_metadata()?;
    let perm = m.permissions().mode() & 0o777;
    let mtime = m.mtime();
    if m.is_dir() {
        let mut folder = name.to_vec();
        folder.push(b'/');
        w.header(&folder, true, ffi::IFDIR, perm, mtime, 0, None, None)?;
        let mut inside: Vec<_> = fs::read_dir(path)?.filter_map(Result::ok).collect();
        inside.sort_by_key(|e| e.file_name());
        for e in inside {
            let mut n = name.to_vec();
            n.push(b'/');
            n.extend(e.file_name().as_bytes());
            add(w, &e.path(), &n, progress)?;
        }
    } else if m.file_type().is_symlink() {
        let target = fs::read_link(path)?;
        w.header(name, true, ffi::IFLNK, 0o777, mtime, 0, Some(target.as_os_str().as_bytes()), None)?;
    } else if m.is_file() {
        let mut f = fs::File::open(path)?;
        w.header(name, true, ffi::IFREG, perm, mtime, m.len() as i64, None, None)?;
        let mut buf = vec![0u8; 1 << 20];
        let mut left = m.len();
        // What the header said is what goes in: a file that grew meanwhile
        // is cut at its size then, one that shrank is an error.
        while left > 0 {
            let want = left.min(buf.len() as u64) as usize;
            let n = f.read(&mut buf[..want])?;
            if n == 0 {
                return Err(io::Error::other(format!("{} changed while it was read", path.display())));
            }
            w.data(&buf[..n])?;
            left -= n as u64;
            progress.done.fetch_add(n as u64, Ordering::Relaxed);
            if progress.stop.load(Ordering::Relaxed) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
            }
        }
    }
    Ok(())
}
