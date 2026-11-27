//! HTTPS for Vergence's own HTTP client (AlbumWall milestone 2, step 1):
//! TLS through the system's OpenSSL, in a confined helper (Dave,
//! 2026-11-26, the lean of question 2: OpenSSL is on every machine Vergence
//! runs on, and confined a flaw in it reaches no further than the helper).
//!
//! The program connects the socket itself and hands it to the helper,
//! `vergence-tls` (libexec), as its standard input; the helper's standard
//! output is one end of a socket pair whose other end is the program's
//! clear stream. The helper loads the system's certificates and OpenSSL
//! before it confines itself (`vergence_decode::confine_allowing`: no file
//! system, no network of its own, no new descriptor), then does the
//! handshake: the server's certificate checked against those certificates
//! and the name asked for, TLS 1.2 at least. It says how that went in one
//! line ("+" or "-why") and then passes bytes both ways until either side
//! closes. Every byte from the server reaches OpenSSL only inside the
//! confined helper.

use std::io::{self, Read};
use std::net::TcpStream;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// The helper, installed beside the program's helpers: next to the program
/// (a build directory), `../libexec/vergence/`, or `/usr/local/libexec/vergence/`.
pub fn helper() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("VERGENCE_TLS_HELPER").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [
        dir.join("vergence-tls"),
        dir.parent()?.join("vergence-tls"),
        dir.parent()?.join("libexec/vergence/vergence-tls"),
        PathBuf::from("/usr/local/libexec/vergence/vergence-tls"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// The TLS connector: HTTPS for vergence-http through the helper.
pub struct Tls {
    pub helper: PathBuf,
    /// A certificate file to trust instead of the system's (the tests'
    /// own authority).
    pub ca: Option<PathBuf>,
}

impl Tls {
    /// With the installed helper, trusting the system's certificates.
    pub fn system() -> Option<Tls> {
        helper().map(|helper| Tls { helper, ca: None })
    }
}

/// The helper's clear stream, and the helper itself (ended with the stream).
struct Stream {
    pair: UnixStream,
    child: std::process::Child,
}

impl Read for Stream {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.pair.read(b)
    }
}

impl io::Write for Stream {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.pair.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.pair.flush()
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.pair.shutdown(std::net::Shutdown::Both);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl vergence_http::Connector for Tls {
    fn tls(&self, socket: TcpStream, host: &str) -> io::Result<Box<dyn vergence_http::Conn>> {
        let (mine, theirs) = UnixStream::pair()?;
        let mut cmd = Command::new(&self.helper);
        cmd.arg(format!("--host={host}"));
        if let Some(ca) = &self.ca {
            cmd.arg(format!("--ca={}", ca.display()));
        }
        // The read timeout the client sets on the socket is on the clear end now.
        let timeout = socket.read_timeout().ok().flatten();
        let child = cmd
            .stdin(Stdio::from(OwnedFd::from(socket)))
            .stdout(Stdio::from(OwnedFd::from(theirs)))
            .stderr(Stdio::null())
            .spawn()?;
        // The Command holds the helper's ends until dropped: without this the
        // program never sees the stream close if the helper dies.
        drop(cmd);
        let mut stream = Stream { pair: mine, child };
        stream.pair.set_read_timeout(timeout.or(Some(std::time::Duration::from_secs(30))))?;
        // The verdict on the handshake: "+" or "-why", one line.
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        while line.len() < 512 {
            match stream.pair.read(&mut byte)? {
                0 => break,
                _ if byte[0] == b'\n' => break,
                _ => line.push(byte[0]),
            }
        }
        let line = String::from_utf8_lossy(&line).to_string();
        match line.strip_prefix('+') {
            Some(_) => {
                stream.pair.set_read_timeout(timeout)?;
                Ok(Box::new(stream))
            }
            None => Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("a secure connection could not be made: {}", line.trim_start_matches('-').trim()),
            )),
        }
    }
}
