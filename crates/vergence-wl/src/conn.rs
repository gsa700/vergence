//! The socket: bytes and file descriptors to and from the display server.
//!
//! Wayland passes file descriptors (shared memory for pixels, a keymap, the
//! clipboard's pipe) beside the bytes, as `SCM_RIGHTS` control messages, so
//! the socket is written with `sendmsg` and read with `recvmsg`. Rust's
//! standard library does not offer those yet; they are declared here, with
//! the structures as Linux and FreeBSD lay them out (they differ).
//!
//! Outgoing messages are queued and written by [`Connection::flush`].
//! [`Connection::read`] blocks for more input; [`Connection::message`] then
//! hands out whole messages, and [`Connection::take_fd`] the descriptors in
//! the order they arrived.

use std::{
    collections::VecDeque,
    io,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::net::UnixStream,
    },
    path::PathBuf,
};

use crate::wire::{self, Header, WireError, Writer};

/// The most file descriptors sent with one write (the reference library's
/// limit), and the most kept waiting to be claimed.
const MAX_FDS: usize = 28;
const MAX_WAITING_FDS: usize = 256;

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

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

const POLLIN: i16 = 1;

unsafe extern "C" {
    fn sendmsg(fd: i32, msg: *const MsgHdr, flags: i32) -> isize;
    fn recvmsg(fd: i32, msg: *mut MsgHdr, flags: i32) -> isize;
    fn poll(fds: *mut PollFd, count: std::ffi::c_ulong, timeout_ms: i32) -> i32;
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

pub struct Connection {
    stream: UnixStream,
    incoming: Vec<u8>,
    fds: VecDeque<OwnedFd>,
    outgoing: Vec<u8>,
    outgoing_fds: Vec<RawFd>,
    /// Object ids: the next never used, and those the server gave back.
    next_id: u32,
    free_ids: Vec<u32>,
}

impl Connection {
    /// To the display server this session points at: `WAYLAND_DISPLAY`
    /// (default `wayland-0`) under `XDG_RUNTIME_DIR`, or a whole path.
    pub fn connect() -> io::Result<Connection> {
        let name = std::env::var_os("WAYLAND_DISPLAY").unwrap_or_else(|| "wayland-0".into());
        let mut path = PathBuf::from(&name);
        if !path.is_absolute() {
            let dir = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
            path = PathBuf::from(dir).join(name);
        }
        Ok(Connection::from_stream(UnixStream::connect(path)?))
    }

    pub fn from_stream(stream: UnixStream) -> Connection {
        // Id 1 is the display itself.
        Connection { stream, incoming: Vec::new(), fds: VecDeque::new(), outgoing: Vec::new(), outgoing_fds: Vec::new(), next_id: 2, free_ids: Vec::new() }
    }

    /// An id for a new object.
    pub fn new_id(&mut self) -> u32 {
        self.free_ids.pop().unwrap_or_else(|| {
            self.next_id += 1;
            self.next_id - 1
        })
    }

    /// The server has let go of this id (wl_display.delete_id): it may be used again.
    pub fn release_id(&mut self, id: u32) {
        if id >= 2 && id < self.next_id && !self.free_ids.contains(&id) {
            self.free_ids.push(id);
        }
    }

    /// Queues a message. Nothing is sent until [`Connection::flush`].
    pub fn send(&mut self, message: Writer) -> Result<(), WireError> {
        let (bytes, fds) = message.finish()?;
        self.outgoing.extend_from_slice(&bytes);
        self.outgoing_fds.extend_from_slice(&fds);
        Ok(())
    }

    /// Writes everything queued.
    pub fn flush(&mut self) -> io::Result<()> {
        let mut sent = 0;
        while sent < self.outgoing.len() {
            // Descriptors go with the first bytes, a few at a time.
            let fds: Vec<RawFd> = self.outgoing_fds.drain(..self.outgoing_fds.len().min(MAX_FDS)).collect();
            sent += send_with_fds(&self.stream, &self.outgoing[sent..], &fds)?;
        }
        self.outgoing.clear();
        while !self.outgoing_fds.is_empty() {
            // More descriptors than bytes to carry them is a caller's mistake.
            self.outgoing_fds.clear();
        }
        Ok(())
    }

    /// Whether there is input to [`Connection::read`], waiting up to
    /// `timeout` for some (None: for as long as it takes). Messages already
    /// read but not yet handed out count.
    pub fn wait(&mut self, timeout: Option<std::time::Duration>) -> io::Result<bool> {
        Ok(self.wait_also([None, None], timeout)?.0)
    }

    /// As [`Connection::wait`], while also watching up to two other
    /// descriptors (a pipe being read, a pipe that wakes the program):
    /// whether the server has input, and whether each of the others has (or
    /// has been closed at its far end).
    pub fn wait_also(&mut self, others: [Option<RawFd>; 2], timeout: Option<std::time::Duration>) -> io::Result<(bool, [bool; 2])> {
        let buffered = wire::split(&self.incoming).is_ok_and(|m| m.is_some());
        // With a message already in hand, only look at what is ready now.
        let ms = if buffered { 0 } else { timeout.map_or(-1, |t| t.as_millis().min(i32::MAX as u128) as i32) };
        let mut fds = [
            PollFd { fd: self.stream.as_raw_fd(), events: POLLIN, revents: 0 },
            PollFd { fd: others[0].unwrap_or(-1), events: POLLIN, revents: 0 },
            PollFd { fd: others[1].unwrap_or(-1), events: POLLIN, revents: 0 },
        ];
        loop {
            // SAFETY: three valid pollfds (a negative descriptor is skipped by poll), and the count says so.
            let n = unsafe { poll(fds.as_mut_ptr(), 3, ms) };
            if n >= 0 {
                // Readable, or closed or in error: a read will say which.
                return Ok((buffered || fds[0].revents != 0, [others[0].is_some() && fds[1].revents != 0, others[1].is_some() && fds[2].revents != 0]));
            }
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
    }

    /// Waits for input and takes what has arrived. Ok(0): the server closed
    /// the connection.
    pub fn read(&mut self) -> io::Result<usize> {
        let mut buf = [0u8; 16 * 1024];
        let mut control = Control([0; CMSG_DATA + MAX_FDS * 4]);
        let mut iov = IoVec { base: buf.as_mut_ptr(), len: buf.len() };
        let mut msg = MsgHdr { name: std::ptr::null_mut(), namelen: 0, iov: &mut iov, iovlen: 1, control: control.0.as_mut_ptr(), controllen: control.0.len() as sys::ControlLen, flags: 0 };
        let n = loop {
            // SAFETY: msg points at buffers that live across the call and says their true sizes.
            let n = unsafe { recvmsg(self.stream.as_raw_fd(), &mut msg, sys::MSG_CMSG_CLOEXEC) };
            if n >= 0 {
                break n as usize;
            }
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        };
        self.incoming.extend_from_slice(&buf[..n]);
        // The descriptors that came with it.
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
                        // One that nobody will claim is closed rather than kept for ever.
                        if self.fds.len() < MAX_WAITING_FDS {
                            self.fds.push_back(fd);
                        }
                    }
                }
            }
            at += len.div_ceil(CMSG_ALIGN) * CMSG_ALIGN;
        }
        Ok(n)
    }

    /// The next whole message that has arrived: its header and its
    /// arguments. None: [`Connection::read`] more.
    pub fn message(&mut self) -> Result<Option<(Header, Vec<u8>)>, WireError> {
        let Some(header) = wire::split(&self.incoming)? else { return Ok(None) };
        let arguments = self.incoming[8..header.size].to_vec();
        self.incoming.drain(..header.size);
        Ok(Some((header, arguments)))
    }

    /// The next file descriptor that arrived, for an event that carries one.
    pub fn take_fd(&mut self) -> Option<OwnedFd> {
        self.fds.pop_front()
    }
}

/// One `sendmsg`: as many of `bytes` as the socket takes, and all of `fds`.
fn send_with_fds(stream: &UnixStream, bytes: &[u8], fds: &[RawFd]) -> io::Result<usize> {
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

#[cfg(test)]
mod tests {
    use std::io::{Read, Seek, SeekFrom, Write};

    use super::*;
    use crate::wire::Reader;

    #[test]
    fn messages_cross_a_socket_in_order() {
        let (a, b) = UnixStream::pair().unwrap();
        let (mut client, mut server) = (Connection::from_stream(a), Connection::from_stream(b));
        client.send(Writer::new(1, 1).uint(2)).unwrap();
        client.send(Writer::new(2, 0).uint(7).string("wl_compositor").uint(4).uint(3)).unwrap();
        client.flush().unwrap();
        assert!(server.read().unwrap() > 0);
        let (h, args) = server.message().unwrap().unwrap();
        assert_eq!((h.object, h.opcode), (1, 1));
        assert_eq!(Reader::new(&args).uint(), Ok(2));
        let (h, args) = server.message().unwrap().unwrap();
        assert_eq!((h.object, h.opcode), (2, 0));
        let mut r = Reader::new(&args);
        assert_eq!((r.uint(), r.string(), r.uint(), r.uint()), (Ok(7), Ok(Some("wl_compositor".into())), Ok(4), Ok(3)));
        assert_eq!(server.message(), Ok(None));
        assert!(server.take_fd().is_none());
    }

    /// The part that differs between operating systems: a file descriptor
    /// sent beside a message arrives, and is the same file.
    #[test]
    fn a_file_descriptor_crosses_with_its_message() {
        let (a, b) = UnixStream::pair().unwrap();
        let (mut client, mut server) = (Connection::from_stream(a), Connection::from_stream(b));
        let dir = std::env::temp_dir().join(format!("vergence-wl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut files = Vec::new();
        for i in 0..3 {
            let mut f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(dir.join(format!("f{i}"))).unwrap();
            write!(f, "file number {i}").unwrap();
            files.push(f);
        }
        client.send(Writer::new(5, 0).uint(9).fd(files[0].as_raw_fd()).int(4096)).unwrap();
        client.send(Writer::new(6, 2).fd(files[1].as_raw_fd()).fd(files[2].as_raw_fd())).unwrap();
        client.flush().unwrap();
        server.read().unwrap();
        assert_eq!(server.message().unwrap().unwrap().0.object, 5);
        assert_eq!(server.message().unwrap().unwrap().0.object, 6);
        for i in 0..3 {
            let mut f = std::fs::File::from(server.take_fd().expect("the descriptor arrived"));
            f.seek(SeekFrom::Start(0)).unwrap();
            let mut text = String::new();
            f.read_to_string(&mut text).unwrap();
            assert_eq!(text, format!("file number {i}"));
        }
        assert!(server.take_fd().is_none());
        // The sender still has its own.
        files[0].seek(SeekFrom::Start(0)).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_closed_connection_reads_nothing() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut client = Connection::from_stream(a);
        drop(b);
        assert_eq!(client.read().unwrap(), 0);
        client.send(Writer::new(1, 0).uint(2)).unwrap();
        assert!(client.flush().is_err(), "writing to a closed socket is an error, not a signal");
    }

    #[test]
    fn waiting_has_a_time_limit() {
        let (a, b) = UnixStream::pair().unwrap();
        let (mut client, mut server) = (Connection::from_stream(a), Connection::from_stream(b));
        let t = std::time::Instant::now();
        assert!(!client.wait(Some(std::time::Duration::from_millis(30))).unwrap());
        assert!(t.elapsed() >= std::time::Duration::from_millis(25));
        server.send(Writer::new(1, 1).uint(2)).unwrap();
        server.flush().unwrap();
        assert!(client.wait(Some(std::time::Duration::from_millis(1000))).unwrap());
        assert!(client.wait(None).unwrap(), "still unread");
        client.read().unwrap();
        assert!(client.wait(Some(std::time::Duration::ZERO)).unwrap(), "read but not handed out");
        client.message().unwrap().unwrap();
        assert!(!client.wait(Some(std::time::Duration::ZERO)).unwrap());
    }

    #[test]
    fn a_pipe_is_watched_beside_the_server() {
        let (a, b) = UnixStream::pair().unwrap();
        let (mut client, mut server) = (Connection::from_stream(a), Connection::from_stream(b));
        let (read_end, write_end) = pipe().unwrap();
        let (second_read, second_write) = pipe().unwrap();
        let fd = [Some(read_end.as_raw_fd()), Some(second_read.as_raw_fd())];
        assert_eq!(client.wait_also(fd, Some(std::time::Duration::from_millis(10))).unwrap(), (false, [false, false]));
        std::fs::File::from(write_end.try_clone().unwrap()).write_all(b"clip").unwrap();
        assert_eq!(client.wait_also(fd, Some(std::time::Duration::from_millis(500))).unwrap(), (false, [true, false]));
        std::fs::File::from(second_write).write_all(b"!").unwrap();
        assert_eq!(client.wait_also([None, fd[1]], None).unwrap(), (false, [false, true]));
        server.send(Writer::new(1, 1).uint(2)).unwrap();
        server.flush().unwrap();
        assert_eq!(client.wait_also(fd, None).unwrap(), (true, [true, true]));
        let fd = fd[0];
        let mut four = [0u8; 4];
        std::fs::File::from(read_end.try_clone().unwrap()).read_exact(&mut four).unwrap();
        assert_eq!(&four, b"clip");
        // The far end closed: the pipe is "ready", and a read finds the end.
        drop(write_end);
        assert!(client.wait_also([fd, None], Some(std::time::Duration::from_millis(500))).unwrap().1[0]);
        assert_eq!(std::fs::File::from(read_end).read(&mut four).unwrap(), 0);
    }

    #[test]
    fn ids_are_given_out_and_taken_back() {
        let (a, _b) = UnixStream::pair().unwrap();
        let mut c = Connection::from_stream(a);
        assert_eq!((c.new_id(), c.new_id(), c.new_id()), (2, 3, 4));
        c.release_id(3);
        c.release_id(3);
        c.release_id(1);
        c.release_id(99);
        assert_eq!((c.new_id(), c.new_id()), (3, 5));
    }

    #[test]
    fn a_message_in_pieces_is_put_together() {
        let (a, b) = UnixStream::pair().unwrap();
        let (mut raw, mut server) = (a, Connection::from_stream(b));
        let (bytes, _) = Writer::new(3, 1).string("in two pieces").finish().unwrap();
        raw.write_all(&bytes[..10]).unwrap();
        server.read().unwrap();
        assert_eq!(server.message(), Ok(None));
        raw.write_all(&bytes[10..]).unwrap();
        server.read().unwrap();
        let (h, args) = server.message().unwrap().unwrap();
        assert_eq!(h.object, 3);
        assert_eq!(Reader::new(&args).string(), Ok(Some("in two pieces".into())));
    }
}
