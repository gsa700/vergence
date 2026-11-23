//! The few libarchive calls used, declared by hand (archive milestone 1:
//! libarchive is borrowed, docs/borrowed-review.md; no bindings crate).
//! Signatures from libarchive 3's archive.h and archive_entry.h. `mode_t`
//! is 32 bits on Linux and 16 on FreeBSD, so it is a type of its own here.

use std::ffi::{c_char, c_int, c_long, c_uint, c_void};

#[repr(C)]
pub struct Archive {
    _private: [u8; 0],
}

#[repr(C)]
pub struct Entry {
    _private: [u8; 0],
}

#[cfg(target_os = "freebsd")]
pub type Mode = u16;
#[cfg(not(target_os = "freebsd"))]
pub type Mode = u32;

pub const OK: c_int = 0;
pub const EOF: c_int = 1;
pub const WARN: c_int = -20;

pub const IFMT: u32 = 0o170000;
pub const IFREG: u32 = 0o100000;
pub const IFDIR: u32 = 0o040000;
pub const IFLNK: u32 = 0o120000;

#[link(name = "archive")]
unsafe extern "C" {
    pub fn archive_read_new() -> *mut Archive;
    pub fn archive_read_support_filter_all(a: *mut Archive) -> c_int;
    pub fn archive_read_support_format_all(a: *mut Archive) -> c_int;
    pub fn archive_read_open_fd(a: *mut Archive, fd: c_int, block_size: usize) -> c_int;
    pub fn archive_read_next_header(a: *mut Archive, entry: *mut *mut Entry) -> c_int;
    pub fn archive_read_data(a: *mut Archive, buf: *mut c_void, len: usize) -> isize;
    pub fn archive_read_data_skip(a: *mut Archive) -> c_int;
    pub fn archive_read_free(a: *mut Archive) -> c_int;

    pub fn archive_write_new() -> *mut Archive;
    pub fn archive_write_set_format_zip(a: *mut Archive) -> c_int;
    pub fn archive_write_set_format_pax_restricted(a: *mut Archive) -> c_int;
    pub fn archive_write_add_filter_gzip(a: *mut Archive) -> c_int;
    pub fn archive_write_set_options(a: *mut Archive, options: *const c_char) -> c_int;
    pub fn archive_write_open_filename(a: *mut Archive, path: *const c_char) -> c_int;
    pub fn archive_write_open_fd(a: *mut Archive, fd: c_int) -> c_int;
    pub fn archive_write_header(a: *mut Archive, entry: *mut Entry) -> c_int;
    pub fn archive_write_data(a: *mut Archive, buf: *const c_void, len: usize) -> isize;
    pub fn archive_write_close(a: *mut Archive) -> c_int;
    pub fn archive_write_free(a: *mut Archive) -> c_int;

    pub fn archive_error_string(a: *mut Archive) -> *const c_char;

    pub fn archive_entry_new() -> *mut Entry;
    pub fn archive_entry_clear(e: *mut Entry) -> *mut Entry;
    pub fn archive_entry_free(e: *mut Entry);
    pub fn archive_entry_pathname(e: *mut Entry) -> *const c_char;
    pub fn archive_entry_pathname_utf8(e: *mut Entry) -> *const c_char;
    pub fn archive_entry_symlink(e: *mut Entry) -> *const c_char;
    pub fn archive_entry_symlink_utf8(e: *mut Entry) -> *const c_char;
    pub fn archive_entry_hardlink(e: *mut Entry) -> *const c_char;
    pub fn archive_entry_hardlink_utf8(e: *mut Entry) -> *const c_char;
    pub fn archive_entry_size(e: *mut Entry) -> i64;
    pub fn archive_entry_size_is_set(e: *mut Entry) -> c_int;
    pub fn archive_entry_filetype(e: *mut Entry) -> Mode;
    pub fn archive_entry_perm(e: *mut Entry) -> Mode;
    pub fn archive_entry_mtime(e: *mut Entry) -> i64;
    pub fn archive_entry_is_encrypted(e: *mut Entry) -> c_int;

    pub fn archive_entry_set_pathname(e: *mut Entry, name: *const c_char);
    pub fn archive_entry_set_pathname_utf8(e: *mut Entry, name: *const c_char);
    pub fn archive_entry_set_symlink(e: *mut Entry, target: *const c_char);
    pub fn archive_entry_set_symlink_utf8(e: *mut Entry, target: *const c_char);
    pub fn archive_entry_set_hardlink(e: *mut Entry, target: *const c_char);
    pub fn archive_entry_set_size(e: *mut Entry, size: i64);
    pub fn archive_entry_set_filetype(e: *mut Entry, kind: c_uint);
    pub fn archive_entry_set_perm(e: *mut Entry, perm: Mode);
    pub fn archive_entry_set_mtime(e: *mut Entry, sec: i64, nsec: c_long);
}

/// The archive's last error in words, or `what` when it has none.
///
/// # Safety
/// `a` is a live archive from `archive_read_new` or `archive_write_new`.
pub unsafe fn error(a: *mut Archive, what: &str) -> String {
    // SAFETY: the caller's archive; the string is copied before any other call.
    let p = unsafe { archive_error_string(a) };
    if p.is_null() {
        return what.to_string();
    }
    // SAFETY: libarchive's error string is NUL-terminated.
    unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// A C string's bytes, or None for a null pointer.
///
/// # Safety
/// `p` is null or points at a NUL-terminated string that lives for the call.
pub unsafe fn bytes(p: *const c_char) -> Option<Vec<u8>> {
    if p.is_null() {
        return None;
    }
    // SAFETY: as the caller promises.
    Some(unsafe { std::ffi::CStr::from_ptr(p) }.to_bytes().to_vec())
}
