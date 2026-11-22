//! The socket: bytes and file descriptors to and from the display server.
//!
//! Wayland passes file descriptors (shared memory for pixels, a keymap, the
//! clipboard's pipe) beside the bytes, as `SCM_RIGHTS` control messages, so
//! the socket is written with `sendmsg` and read with `recvmsg`
//! (vergence_protocol::sock, shared with the native protocol).
//!
//! Outgoing messages are queued and written by [`Connection::flush`].
//! [`Connection::read`] blocks for more input; [`Connection::message`] then
//! hands out whole messages, and [`Connection::take_fd`] the descriptors in
//! the order they arrived.

use std::{
    collections::VecDeque,
    io,
    os::{
        fd::{AsRawFd, OwnedFd, RawFd},
        unix::net::UnixStream,
    },
    path::PathBuf,
};

use crate::wire::{self, Header, WireError, Writer};

/// The most kept waiting to be claimed.
const MAX_WAITING_FDS: usize = 256;

pub use vergence_protocol::sock::pipe;
use vergence_protocol::sock::{self, MAX_FDS};

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
            let dir = std::env::var_os("XDG_RUNTIME_DIR")
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
            path = PathBuf::from(dir).join(name);
        }
        Ok(Connection::from_stream(UnixStream::connect(path)?))
    }

    pub fn from_stream(stream: UnixStream) -> Connection {
        // Id 1 is the display itself.
        Connection {
            stream,
            incoming: Vec::new(),
            fds: VecDeque::new(),
            outgoing: Vec::new(),
            outgoing_fds: Vec::new(),
            next_id: 2,
            free_ids: Vec::new(),
        }
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
            sent += sock::send_with_fds(&self.stream, &self.outgoing[sent..], &fds)?;
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
    pub fn wait_also(
        &mut self,
        others: [Option<RawFd>; 2],
        timeout: Option<std::time::Duration>,
    ) -> io::Result<(bool, [bool; 2])> {
        let buffered = wire::split(&self.incoming).is_ok_and(|m| m.is_some());
        // With a message already in hand, only look at what is ready now.
        let wait = if buffered { Some(std::time::Duration::ZERO) } else { timeout };
        // Readable, or closed or in error: a read will say which.
        let ready = sock::poll_readable(&[Some(self.stream.as_raw_fd()), others[0], others[1]], wait)?;
        Ok((buffered || ready[0], [ready[1], ready[2]]))
    }

    /// Waits for input and takes what has arrived. Ok(0): the server closed
    /// the connection.
    pub fn read(&mut self) -> io::Result<usize> {
        let mut buf = [0u8; 16 * 1024];
        let n = sock::recv_with_fds(&self.stream, &mut buf, &mut self.fds, MAX_WAITING_FDS)?;
        self.incoming.extend_from_slice(&buf[..n]);
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
            let mut f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(dir.join(format!("f{i}")))
                .unwrap();
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
