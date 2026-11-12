//! File descriptors beside the bytes on a Unix socket (native milestone 1,
//! step 1; moved here from vergence-wl, which uses it too).
//!
//! A window's pixels (shared memory, GPU buffers), a keymap and the
//! clipboard's pipe travel as file descriptors, sent as `SCM_RIGHTS` control
//! messages, so such a socket is written with `sendmsg` and read with
//! `recvmsg`. Rust's standard library does not offer those yet; they are
//! declared here, with the structures as Linux and FreeBSD lay them out
//! (they differ).

use std::{
    collections::VecDeque,
    io,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::net::UnixStream,
    },
};

/// The most file descriptors sent with one write (the Wayland reference
/// library's limit too).
pub const MAX_FDS: usize = 28;

#[cfg(target_os = "freebsd")]
mod sys {
    pub const SOL_SOCKET: i32 = 0xffff;
    pub const MSG_NOSIGNAL: i32 = 0x20000;
    pub const MSG_CMSG_CLOEXEC: i32 = 0x40000;
    pub type MsgLen = i32;
    pub type ControlLen = u32;
    pub type CmsgLen = u32;
}

#[cfg(not(target_os = "freebsd"))]
mod sys {
    pub const SOL_SOCKET: i32 = 1;
    pub const MSG_NOSIGNAL: i32 = 0x4000;
    pub const MSG_CMSG_CLOEXEC: i32 = 0x4000_0000;
    pub type MsgLen = usize;
    pub type ControlLen = usize;
    pub type CmsgLen = usize;
}

const SCM_RIGHTS: i32 = 1;
/// Where a control message's data starts, and what its length is rounded up to
/// (8 bytes on the 64-bit Linux and FreeBSD this is built for).
const CMSG_DATA: usize = 16;
const CMSG_ALIGN: usize = 8;

#[repr(C)]
struct IoVec {
    base: *mut u8,
    len: usize,
}

#[repr(C)]
struct MsgHdr {
    name: *mut u8,
    namelen: u32,
    iov: *mut IoVec,
    iovlen: sys::MsgLen,
    control: *mut u8,
    controllen: sys::ControlLen,
    flags: i32,
}

#[repr(C)]
struct CmsgHdr {
    len: sys::CmsgLen,
    level: i32,
    kind: i32,
}

unsafe extern "C" {
    fn sendmsg(fd: i32, msg: *const MsgHdr, flags: i32) -> isize;
    fn recvmsg(fd: i32, msg: *mut MsgHdr, flags: i32) -> isize;
    fn pipe2(fds: *mut [i32; 2], flags: i32) -> i32;
}

#[cfg(target_os = "freebsd")]
const O_CLOEXEC: i32 = 0x0010_0000;
#[cfg(not(target_os = "freebsd"))]
const O_CLOEXEC: i32 = 0o2_000_000;

/// A pipe: what is written to the second end is read from the first. The
/// clipboard's contents travel through one, handed to the other program by
/// the server. Neither end is passed on to programs this one starts.
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1i32; 2];
    // SAFETY: room for the two descriptors the call writes.
    if unsafe { pipe2(&mut fds, O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel just gave this process these two descriptors.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Room for [`MAX_FDS`] descriptors in one control message, aligned.
#[repr(C, align(8))]
struct Control([u8; CMSG_DATA + MAX_FDS * 4]);

/// One `recvmsg` into `buf`: the bytes that came (0: the other end closed),
/// with the descriptors that came beside them appended to `fds`. Past
/// `max_waiting` descriptors waiting there, more are closed at once: a peer
/// cannot make this side hold descriptors nobody claims.
pub fn recv_with_fds(stream: &UnixStream, buf: &mut [u8], fds: &mut VecDeque<OwnedFd>, max_waiting: usize) -> io::Result<usize> {
    let mut control = Control([0; CMSG_DATA + MAX_FDS * 4]);
    let mut iov = IoVec { base: buf.as_mut_ptr(), len: buf.len() };
    let mut msg = MsgHdr { name: std::ptr::null_mut(), namelen: 0, iov: &mut iov, iovlen: 1, control: control.0.as_mut_ptr(), controllen: control.0.len() as sys::ControlLen, flags: 0 };
    let n = loop {
        // SAFETY: msg points at buffers that live across the call and says their true sizes.
        let n = unsafe { recvmsg(stream.as_raw_fd(), &mut msg, sys::MSG_CMSG_CLOEXEC) };
        if n >= 0 {
            break n as usize;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    };
    let filled = (msg.controllen as usize).min(control.0.len());
    let mut at = 0;
    while at + CMSG_DATA <= filled {
        // SAFETY: at + size_of::<CmsgHdr>() is within the filled part of the aligned buffer.
        let head = unsafe { &*(control.0.as_ptr().add(at) as *const CmsgHdr) };
        let len = head.len as usize;
        if len < CMSG_DATA || at + len > filled {
            break;
        }
        if head.level == sys::SOL_SOCKET && head.kind == SCM_RIGHTS {
            for chunk in control.0[at + CMSG_DATA..at + len].chunks_exact(4) {
                let fd = i32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                if fd >= 0 {
                    // SAFETY: the kernel just gave this process this descriptor; nothing else owns it.
                    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
                    if fds.len() < max_waiting {
                        fds.push_back(fd);
                    }
                }
            }
        }
        at += len.div_ceil(CMSG_ALIGN) * CMSG_ALIGN;
    }
    Ok(n)
}

/// One `sendmsg`: as many of `bytes` as the socket takes, and all of `fds`
/// (at most [`MAX_FDS`]).
pub fn send_with_fds(stream: &UnixStream, bytes: &[u8], fds: &[RawFd]) -> io::Result<usize> {
    let mut control = Control([0; CMSG_DATA + MAX_FDS * 4]);
    let fds = &fds[..fds.len().min(MAX_FDS)];
    let mut controllen = 0;
    if !fds.is_empty() {
        let len = CMSG_DATA + fds.len() * 4;
        let head = CmsgHdr { len: len as sys::CmsgLen, level: sys::SOL_SOCKET, kind: SCM_RIGHTS };
        // SAFETY: the buffer is aligned for CmsgHdr and larger than it.
        unsafe { (control.0.as_mut_ptr() as *mut CmsgHdr).write(head) };
        for (i, fd) in fds.iter().enumerate() {
            control.0[CMSG_DATA + i * 4..CMSG_DATA + i * 4 + 4].copy_from_slice(&fd.to_ne_bytes());
        }
        controllen = len.div_ceil(CMSG_ALIGN) * CMSG_ALIGN;
    }
    let mut iov = IoVec { base: bytes.as_ptr() as *mut u8, len: bytes.len() };
    let msg = MsgHdr {
        name: std::ptr::null_mut(),
        namelen: 0,
        iov: &mut iov,
        iovlen: 1,
        control: if controllen == 0 { std::ptr::null_mut() } else { control.0.as_mut_ptr() },
        controllen: controllen as sys::ControlLen,
        flags: 0,
    };
    loop {
        // SAFETY: msg points at buffers that live across the call and says their true sizes; sendmsg does not write through iov.base.
        let n = unsafe { sendmsg(stream.as_raw_fd(), &msg, sys::MSG_NOSIGNAL) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Writes all of `bytes`, with `fds` going beside the first of them.
pub fn send_all(stream: &UnixStream, bytes: &[u8], fds: &[RawFd]) -> io::Result<()> {
    let mut sent = 0;
    let mut fds = fds;
    while sent < bytes.len() {
        let now = &fds[..fds.len().min(MAX_FDS)];
        let n = send_with_fds(stream, &bytes[sent..], now)?;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        fds = &fds[now.len()..];
        sent += n;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn descriptors_cross_with_their_bytes() {
        let (a, b) = UnixStream::pair().unwrap();
        let (r, w) = pipe().unwrap();
        send_all(&a, b"hello", &[w.as_raw_fd()]).unwrap();
        drop(w);
        let mut buf = [0u8; 16];
        let mut fds = VecDeque::new();
        let n = recv_with_fds(&b, &mut buf, &mut fds, 8).unwrap();
        assert_eq!(&buf[..n], b"hello");
        let w2 = fds.pop_front().expect("a descriptor");
        // It is the same pipe: what goes in one end comes out of the reader.
        std::fs::File::from(w2).write_all(b"through").unwrap();
        let mut got = [0u8; 7];
        std::fs::File::from(r).read_exact(&mut got).unwrap();
        assert_eq!(&got, b"through");
    }

    #[test]
    fn too_many_waiting_are_closed() {
        let (a, b) = UnixStream::pair().unwrap();
        let pipes: Vec<_> = (0..4).map(|_| pipe().unwrap()).collect();
        let raw: Vec<RawFd> = pipes.iter().map(|p| p.1.as_raw_fd()).collect();
        send_all(&a, b"x", &raw).unwrap();
        let mut buf = [0u8; 4];
        let mut fds = VecDeque::new();
        recv_with_fds(&b, &mut buf, &mut fds, 2).unwrap();
        assert_eq!(fds.len(), 2, "the rest were closed, not kept");
    }
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

const POLLIN: i16 = 1;

unsafe extern "C" {
    fn poll(fds: *mut PollFd, count: std::ffi::c_ulong, timeout_ms: i32) -> i32;
}

/// Waits until one of `fds` can be read (or has been closed at its far
/// end), or `timeout` passes (None: no limit). Returns, for each, whether it
/// is ready; a None in `fds` is not watched. The same on Linux and FreeBSD
/// (struct pollfd and POLLIN agree). Moved here from vergence-wl on
/// 2026-11-11, so the toolkit's native and Wayland roads share it.
pub fn poll_readable(fds: &[Option<RawFd>], timeout: Option<std::time::Duration>) -> io::Result<Vec<bool>> {
    let ms = timeout.map_or(-1, |t| t.as_millis().min(i32::MAX as u128) as i32);
    let mut set: Vec<PollFd> = fds.iter().map(|f| PollFd { fd: f.unwrap_or(-1), events: POLLIN, revents: 0 }).collect();
    loop {
        // SAFETY: `set` holds `set.len()` valid pollfds (a negative descriptor is skipped).
        let n = unsafe { poll(set.as_mut_ptr(), set.len() as std::ffi::c_ulong, ms) };
        if n >= 0 {
            return Ok(set.iter().zip(fds).map(|(p, f)| f.is_some() && p.revents != 0).collect());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}
