//! The system calls VDM's parts share, declared here (no libc crate): the
//! peer of a local socket, memory that stays out of swap and core files, a
//! process no debugger of the same user can attach to, no core files.
//! Each says what it relies on; Linux and FreeBSD differ where marked.

use std::ffi::{c_int, c_void};
#[cfg(target_os = "linux")]
use std::ffi::c_long;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

unsafe extern "C" {
    fn getuid() -> u32;
    fn geteuid() -> u32;
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn mlock(addr: *const c_void, len: usize) -> c_int;
    fn munlock(addr: *const c_void, len: usize) -> c_int;
    fn madvise(addr: *mut c_void, len: usize, advice: c_int) -> c_int;
    fn setrlimit(resource: c_int, rlim: *const [u64; 2]) -> c_int;
    #[cfg(target_os = "linux")]
    fn prctl(option: c_int, arg2: c_long, arg3: c_long, arg4: c_long, arg5: c_long) -> c_int;
    #[cfg(target_os = "freebsd")]
    fn procctl(idtype: c_int, id: i64, cmd: c_int, data: *mut c_void) -> c_int;
    #[cfg(not(target_os = "linux"))]
    fn getpeereid(fd: c_int, euid: *mut u32, egid: *mut u32) -> c_int;
    #[cfg(target_os = "linux")]
    fn getsockopt(fd: c_int, level: c_int, name: c_int, val: *mut c_void, len: *mut u32) -> c_int;
}

pub fn uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { getuid() }
}

pub fn euid() -> u32 {
    // SAFETY: geteuid cannot fail.
    unsafe { geteuid() }
}

/// The connecting process's effective uid, as the kernel recorded it when it
/// connected (threat model T7: never a name sent in a request). getpeereid on
/// FreeBSD; SO_PEERCRED on Linux, where glibc has no getpeereid.
pub fn peer_uid(stream: &UnixStream) -> Option<u32> {
    #[cfg(not(target_os = "linux"))]
    {
        let (mut uid, mut gid) = (u32::MAX, u32::MAX);
        // SAFETY: two out-parameters we own; the fd is the stream's.
        (unsafe { getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0 && uid != u32::MAX).then_some(uid)
    }
    #[cfg(target_os = "linux")]
    {
        // struct ucred { pid_t pid; uid_t uid; gid_t gid; }; SOL_SOCKET 1, SO_PEERCRED 17.
        let mut buf = [0u8; 12];
        let mut len = buf.len() as u32;
        // SAFETY: a buffer of the size we pass; the fd is the stream's.
        let ok = unsafe { getsockopt(stream.as_raw_fd(), 1, 17, buf.as_mut_ptr().cast(), &mut len) } == 0;
        (ok && len == 12).then(|| u32::from_ne_bytes([buf[4], buf[5], buf[6], buf[7]]))
    }
}

/// One page of memory of its own: locked in RAM where the system allows it
/// (never written to swap) and left out of core files. For passwords
/// ([`crate::secret::Secret`]). x86_64 only, as Vergence is: 4096-byte pages.
pub struct LockedPage {
    ptr: *mut u8,
}

pub const PAGE: usize = 4096;

const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_PRIVATE: c_int = 2;
#[cfg(target_os = "linux")]
const MAP_ANON: c_int = 0x20;
#[cfg(not(target_os = "linux"))]
const MAP_ANON: c_int = 0x1000;
// MADV_DONTDUMP on Linux; MADV_NOCORE on FreeBSD.
#[cfg(target_os = "linux")]
const MADV_NO_CORE: c_int = 16;
#[cfg(not(target_os = "linux"))]
const MADV_NO_CORE: c_int = 8;

impl LockedPage {
    pub fn new() -> Option<LockedPage> {
        // SAFETY: an anonymous private mapping of one page; checked below.
        let p = unsafe { mmap(std::ptr::null_mut(), PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0) };
        if p.is_null() || p as isize == -1 {
            return None;
        }
        // SAFETY: the page just mapped. Either may fail (RLIMIT_MEMLOCK): the
        // page is still wiped when dropped, so that is not fatal.
        unsafe {
            mlock(p, PAGE);
            madvise(p, PAGE, MADV_NO_CORE);
        }
        Some(LockedPage { ptr: p.cast() })
    }

    pub fn bytes(&self) -> &[u8] {
        // SAFETY: PAGE bytes mapped read-write for our lifetime.
        unsafe { std::slice::from_raw_parts(self.ptr, PAGE) }
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: as above, and we are its only owner.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, PAGE) }
    }

    /// Every byte zero, written so the compiler cannot leave it out.
    pub fn wipe(&mut self) {
        for i in 0..PAGE {
            // SAFETY: within the page.
            unsafe { std::ptr::write_volatile(self.ptr.add(i), 0) };
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

impl Drop for LockedPage {
    fn drop(&mut self) {
        self.wipe();
        // SAFETY: the page we mapped, unmapped once.
        unsafe {
            munlock(self.ptr.cast(), PAGE);
            munmap(self.ptr.cast(), PAGE);
        }
    }
}

// A page belongs to one owner at a time; sending it to another thread moves it.
unsafe impl Send for LockedPage {}

/// No core files from this process (review I1): they could hold a password.
pub fn no_core_files() {
    // RLIMIT_CORE: 4 on both Linux (x86_64) and FreeBSD.
    let zero = [0u64, 0u64];
    // SAFETY: a struct rlimit of two rlim_t (u64 on both, x86_64).
    unsafe { setrlimit(4, &zero) };
}

/// No debugger of the same user may attach to this process (threat model
/// T6, review I1): PR_SET_DUMPABLE 0 on Linux (also hides /proc/<pid>/mem
/// from the user); on FreeBSD procctl PROC_TRACE_CTL with
/// PROC_TRACE_CTL_DISABLE_EXEC, which lasts until this process runs another
/// program, so the programs the server starts can still be debugged.
/// Whether it was set.
pub fn no_debugger() -> bool {
    #[cfg(target_os = "linux")]
    {
        // PR_SET_DUMPABLE 4, value 0.
        // SAFETY: prctl with integer arguments only.
        unsafe { prctl(4, 0, 0, 0, 0) == 0 }
    }
    #[cfg(target_os = "freebsd")]
    {
        // P_PID 0; PROC_TRACE_CTL 7; PROC_TRACE_CTL_DISABLE_EXEC 3.
        let mut v: c_int = 3;
        // SAFETY: our own pid, an int argument we own.
        unsafe { procctl(0, std::process::id() as i64, 7, (&mut v as *mut c_int).cast()) == 0 }
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locked_page_is_wiped() {
        let mut p = LockedPage::new().expect("a page");
        p.bytes_mut()[..6].copy_from_slice(b"secret");
        assert_eq!(&p.bytes()[..6], b"secret");
        p.wipe();
        assert!(p.bytes().iter().all(|b| *b == 0));
    }

    #[test]
    fn the_peer_of_our_own_socket_is_us() {
        let (a, _b) = UnixStream::pair().unwrap();
        assert_eq!(peer_uid(&a), Some(euid()));
    }
}
