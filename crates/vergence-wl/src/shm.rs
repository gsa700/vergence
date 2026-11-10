//! Memory shared with the display server: where a window's pixels are put
//! when they are drawn in software (`wl_shm`).
//!
//! An anonymous file (`memfd_create`, which Linux and FreeBSD both have) of
//! the size asked for, mapped into this process. The server maps the same
//! file from the descriptor sent to it.

use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
};

unsafe extern "C" {
    fn memfd_create(name: *const std::ffi::c_char, flags: u32) -> i32;
    fn ftruncate(fd: i32, length: i64) -> i32;
    fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut u8;
    fn munmap(addr: *mut u8, len: usize) -> i32;
}

const MFD_CLOEXEC: u32 = 1;
const PROT_READ_WRITE: i32 = 1 | 2;
const MAP_SHARED: i32 = 1;

/// The largest mapping made: a 32,768-pixel square would be 4 GiB, far past
/// any window; this is room for 16,384 x 8,192 at four bytes a pixel.
pub const MAX_SHM: usize = 512 << 20;

pub struct ShmFile {
    fd: OwnedFd,
    ptr: *mut u8,
    len: usize,
}

// SAFETY: the mapping belongs to this value alone; nothing else in the process points into it.
unsafe impl Send for ShmFile {}

impl ShmFile {
    /// `len` bytes of shared memory, zeroed.
    pub fn new(len: usize) -> io::Result<ShmFile> {
        if len == 0 || len > MAX_SHM {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "shared memory size out of range"));
        }
        // SAFETY: the name is a valid C string; the result is checked.
        let raw = unsafe { memfd_create(c"vergence-shm".as_ptr(), MFD_CLOEXEC) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: memfd_create just returned this descriptor to us.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: a valid descriptor and a length that fits.
        if unsafe { ftruncate(fd.as_raw_fd(), len as i64) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: mapping a file we own, of the length we just gave it; the result is checked.
        let ptr = unsafe { mmap(std::ptr::null_mut(), len, PROT_READ_WRITE, MAP_SHARED, fd.as_raw_fd(), 0) };
        if ptr as isize == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(ShmFile { fd, ptr, len })
    }

    /// The descriptor to send to the server.
    pub fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: ptr..ptr+len is our mapping, alive as long as self, and borrowed mutably here.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for ShmFile {
    fn drop(&mut self) {
        // SAFETY: the mapping made in new(), unmapped once.
        unsafe { munmap(self.ptr, self.len) };
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Seek, SeekFrom};

    use super::*;

    #[test]
    fn what_is_written_is_in_the_file() {
        let mut shm = ShmFile::new(8192).unwrap();
        assert_eq!(shm.len(), 8192);
        assert!(shm.bytes().iter().all(|b| *b == 0), "new memory is zeroed");
        shm.bytes()[4096..4100].copy_from_slice(b"here");
        // Another view of the same file, as the server would have.
        let mut file = std::fs::File::from(shm.fd.try_clone().unwrap());
        file.seek(SeekFrom::Start(4096)).unwrap();
        let mut four = [0u8; 4];
        file.read_exact(&mut four).unwrap();
        assert_eq!(&four, b"here");
    }

    #[test]
    fn sizes_that_make_no_sense_are_refused() {
        assert!(ShmFile::new(0).is_err());
        assert!(ShmFile::new(MAX_SHM + 1).is_err());
    }
}
