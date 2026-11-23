//! `vergence-unpack` (libexec): reads one archive from its standard input
//! through libarchive and passes its entries on its standard output (the
//! stream, stream.rs), confined before it reads a byte of the archive
//! (`vergence_decode::confine`: no file system or network, no new
//! descriptor, limits; Capsicum on FreeBSD, seccomp on Linux). It writes
//! no file: the caller (unpack.rs) decides what is made, and where.
//!
//! Arguments: `--list` (headers only, no contents), `--max-bytes=N` and
//! `--max-entries=N` (it stops with an error past either), and
//! `--check-confined` (for the tests: confine, then try to open a file).

use std::ffi::c_int;
use std::io::Write;

use crate::ffi;
use crate::stream::{self, Header, Kind};

/// The helper's processor time, seconds: a big archive takes a while.
pub const CPU_SECONDS: u64 = 4 * 3600;

/// Names converted to UTF-8 need the locale's character set to be UTF-8
/// before the helper confines itself (glibc loads conversion modules from
/// files; after confinement it could not).
fn utf8_locale() {
    unsafe extern "C" {
        fn setlocale(category: c_int, locale: *const std::ffi::c_char) -> *mut std::ffi::c_char;
    }
    #[cfg(target_os = "freebsd")]
    const LC_ALL: c_int = 0;
    #[cfg(not(target_os = "freebsd"))]
    const LC_ALL: c_int = 6;
    for name in [c"C.UTF-8", c"C.utf8"] {
        // SAFETY: setlocale(3) with a NUL-terminated name.
        if !unsafe { setlocale(LC_ALL, name.as_ptr()) }.is_null() {
            return;
        }
    }
}

/// On FreeBSD, the archive's descriptor may only be read, sought and
/// looked at, and the two outputs only written: a compromised helper cannot
/// change the archive's mode, times or locks (the review, 2026-11-23).
/// Linux's seccomp list already allows no such call.
#[cfg(target_os = "freebsd")]
fn limit_pipes() {
    #[repr(C)]
    struct Rights {
        r: [u64; 2],
    }
    unsafe extern "C" {
        fn __cap_rights_init(version: c_int, rights: *mut Rights, ...) -> *mut Rights;
        fn cap_rights_limit(fd: c_int, rights: *const Rights) -> c_int;
    }
    // sys/capsicum.h: CAPRIGHT(0, bit) is (1 << 57) | bit; version 0.
    const R: u64 = 1 << 57;
    const READ: u64 = R | 0x1;
    const WRITE: u64 = R | 0x2;
    const SEEK: u64 = R | 0x4 | 0x8;
    const FSTAT: u64 = R | 0x80000;
    for (fd, a, b, c) in [(0, READ, SEEK, FSTAT), (1, WRITE, FSTAT, 0), (2, WRITE, FSTAT, 0)] {
        let mut rights = Rights { r: [0; 2] };
        // SAFETY: cap_rights_init's version 0, a place for the rights, the
        // rights, and the 0 that ends the list; then cap_rights_limit(2).
        let ok = unsafe {
            __cap_rights_init(0, &mut rights, a, b, c, 0u64);
            cap_rights_limit(fd, &rights)
        } == 0;
        if !ok {
            eprintln!("vergence-unpack: the descriptors' rights could not be limited; not reading");
            std::process::exit(3);
        }
    }
}

#[cfg(not(target_os = "freebsd"))]
fn limit_pipes() {}

struct Reader(*mut ffi::Archive);

impl Drop for Reader {
    fn drop(&mut self) {
        // SAFETY: an archive from archive_read_new, freed once.
        unsafe { ffi::archive_read_free(self.0) };
    }
}

fn number(arg: &str, name: &str) -> Option<u64> {
    arg.strip_prefix(name)?.parse().ok()
}

pub fn helper_main() -> std::process::ExitCode {
    let (mut list, mut max_bytes, mut max_entries) = (false, u64::MAX, u64::MAX);
    for a in std::env::args().skip(1) {
        if a == "--list" {
            list = true;
        } else if a == "--check-confined" {
            vergence_decode::confine(10);
            return match std::fs::File::open("/") {
                Ok(_) => {
                    eprintln!("vergence-unpack: NOT confined: a file could be opened");
                    std::process::ExitCode::SUCCESS
                }
                Err(_) => std::process::ExitCode::from(4),
            };
        } else if let Some(n) = number(&a, "--max-bytes=") {
            max_bytes = n;
        } else if let Some(n) = number(&a, "--max-entries=") {
            max_entries = n;
        } else {
            eprintln!("vergence-unpack: unknown argument {a:?}");
            return std::process::ExitCode::from(2);
        }
    }
    utf8_locale();
    // A zip's times are local: the time zone is read now, once (glibc keeps
    // it), not at the first entry, when /etc/localtime could no longer be
    // opened (the seccomp filter killed the helper there).
    unsafe extern "C" {
        fn tzset();
    }
    // SAFETY: tzset(3) takes nothing.
    unsafe { tzset() };
    // SAFETY: archive_read_new takes nothing; null is checked.
    let a = unsafe { ffi::archive_read_new() };
    if a.is_null() {
        return std::process::ExitCode::from(1);
    }
    let reader = Reader(a);
    // SAFETY: a live archive; these only register readers.
    unsafe {
        ffi::archive_read_support_filter_all(reader.0);
        ffi::archive_read_support_format_all(reader.0);
    }
    limit_pipes();
    vergence_decode::confine(CPU_SECONDS);
    let out = std::io::stdout();
    let mut out = std::io::BufWriter::with_capacity(1 << 20, out.lock());
    let r = run(&reader, &mut out, list, max_bytes, max_entries);
    let _ = out.flush();
    match r {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => std::process::ExitCode::from(1),
    }
}

/// The entries passed on until the end, an error (said on the stream) or a
/// cap. Err only when the stream itself could not be written.
fn run(reader: &Reader, out: &mut impl Write, list: bool, max_bytes: u64, max_entries: u64) -> std::io::Result<()> {
    let a = reader.0;
    // SAFETY: a live archive; standard input is the archive's file (the
    // caller opened it), read with a 64 KB block.
    if unsafe { ffi::archive_read_open_fd(a, 0, 64 * 1024) } != ffi::OK {
        // SAFETY: a live archive.
        return stream::write_failed(out, &unsafe { ffi::error(a, "not an archive this can read") });
    }
    let (mut bytes, mut entries) = (0u64, 0u64);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let mut e: *mut ffi::Entry = std::ptr::null_mut();
        // SAFETY: a live archive and a place for the entry pointer.
        let r = unsafe { ffi::archive_read_next_header(a, &mut e) };
        if r == ffi::EOF {
            return stream::write_end(out);
        }
        if r < ffi::WARN || e.is_null() {
            // SAFETY: a live archive.
            return stream::write_failed(out, &unsafe { ffi::error(a, "the archive is damaged") });
        }
        entries += 1;
        if entries > max_entries {
            return stream::write_failed(out, "too many entries");
        }
        // SAFETY: the entry archive_read_next_header gave, valid until the next call.
        if unsafe { ffi::archive_entry_is_encrypted(e) } != 0 {
            return stream::write_password(out);
        }
        let Some(h) = header(e) else {
            // No name at all: nothing to make.
            continue;
        };
        stream::write_entry(out, &h)?;
        if list || h.kind != Kind::File {
            // SAFETY: a live archive positioned at this entry's contents.
            unsafe { ffi::archive_read_data_skip(a) };
            continue;
        }
        loop {
            // SAFETY: a live archive and a buffer of buf.len() bytes.
            let n = unsafe { ffi::archive_read_data(a, buf.as_mut_ptr().cast(), buf.len()) };
            if n == 0 {
                break;
            }
            if n < 0 {
                // SAFETY: a live archive.
                return stream::write_failed(out, &unsafe { ffi::error(a, "the archive is damaged") });
            }
            bytes += n as u64;
            if bytes > max_bytes {
                return stream::write_failed(out, "larger than it says");
            }
            stream::write_data(out, &buf[..n as usize])?;
        }
    }
}

/// An entry's header, or None when it has no name.
// mode_t is u16 on FreeBSD: u32::from is a conversion there.
#[allow(clippy::useless_conversion)]
fn header(e: *mut ffi::Entry) -> Option<Header> {
    // SAFETY (each call below): the live entry from archive_read_next_header;
    // the strings are copied at once.
    let name = unsafe { ffi::bytes(ffi::archive_entry_pathname_utf8(e)) }
        .or_else(|| unsafe { ffi::bytes(ffi::archive_entry_pathname(e)) })?;
    let hard = unsafe { ffi::bytes(ffi::archive_entry_hardlink_utf8(e)) }
        .or_else(|| unsafe { ffi::bytes(ffi::archive_entry_hardlink(e)) });
    let kind_bits = u32::from(unsafe { ffi::archive_entry_filetype(e) }) & ffi::IFMT;
    // A hard link that carries contents (cpio's newc, as in an .rpm, puts
    // them with the last link) is passed on as a file of its own, or they
    // would be lost (the review, 2026-11-23).
    let carries = unsafe { ffi::archive_entry_size_is_set(e) } != 0 && unsafe { ffi::archive_entry_size(e) } > 0;
    let (kind, link) = match (hard, kind_bits) {
        (Some(_), ffi::IFREG) if carries => (Kind::File, Vec::new()),
        (Some(target), _) => (Kind::Hardlink, target),
        (None, ffi::IFREG) => (Kind::File, Vec::new()),
        (None, ffi::IFDIR) => (Kind::Dir, Vec::new()),
        (None, ffi::IFLNK) => (
            Kind::Symlink,
            unsafe { ffi::bytes(ffi::archive_entry_symlink_utf8(e)) }
                .or_else(|| unsafe { ffi::bytes(ffi::archive_entry_symlink(e)) })
                .unwrap_or_default(),
        ),
        _ => (Kind::Other, Vec::new()),
    };
    let size = (kind == Kind::File && unsafe { ffi::archive_entry_size_is_set(e) } != 0)
        .then(|| u64::try_from(unsafe { ffi::archive_entry_size(e) }).ok())
        .flatten()
        .or(if kind == Kind::File { None } else { Some(0) });
    Some(Header {
        kind,
        perm: u32::from(unsafe { ffi::archive_entry_perm(e) }) & 0o7777,
        mtime: unsafe { ffi::archive_entry_mtime(e) },
        size,
        name,
        link,
    })
}
