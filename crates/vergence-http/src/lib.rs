//! Vergence's own HTTP/1.1 client (AlbumWall milestone 2, step 1): what a
//! music server's client needs and no more. GET with a query and a Range,
//! one request a connection, an answer read as fixed-length, chunked or
//! until close; redirects within the same server; a deadline for the whole
//! answer and a limit on its size; the address actually connected to, for
//! the local-network rule ([`is_local`]).
//!
//! Every byte from the network is untrusted: the head is limited in size and
//! in headers, the parsers are pure functions of what was read and are given
//! random bytes in the tests, and a bad answer is an error, never a panic.
//!
//! HTTPS goes through a [`Connector`]: a plain TCP connection is ours; TLS is
//! the system's OpenSSL in a confined helper (Dave, 2026-11-26), which the
//! program gives a connected socket and gets a clear stream back.
//!
//! No dependencies (whitepaper §8, "Our own parts").

pub mod json;

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

/// The head of an answer may be at most this long, with at most this many headers.
pub const MAX_HEAD: usize = 64 * 1024;
pub const MAX_HEADERS: usize = 100;
/// Redirects followed, within the same server.
pub const MAX_REDIRECTS: usize = 5;

#[derive(Debug)]
pub enum Error {
    /// The address could not be read.
    Url(String),
    /// The name could not be looked up, or came to nothing.
    Resolve(String),
    /// No connection (refused, unreachable, timed out connecting).
    Connect(io::Error),
    /// The whole answer took longer than allowed.
    Timeout,
    /// Reading or writing failed after connecting.
    Io(io::Error),
    /// The answer was not HTTP as this reads it.
    BadAnswer(String),
    /// The answer was larger than allowed.
    TooLarge,
    /// Too many redirects, or one to another server.
    Redirect(String),
    /// HTTPS was asked for and no TLS connector was given.
    NoTls,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Url(s) => write!(f, "not an address this can use: {s}"),
            Error::Resolve(s) => write!(f, "{s}"),
            Error::Connect(e) => write!(f, "cannot connect: {e}"),
            Error::Timeout => write!(f, "the server took too long to answer"),
            Error::Io(e) => write!(f, "{e}"),
            Error::BadAnswer(s) => write!(f, "the server's answer could not be read: {s}"),
            Error::TooLarge => write!(f, "the server's answer was too large"),
            Error::Redirect(s) => write!(f, "{s}"),
            Error::NoTls => write!(f, "HTTPS is not available"),
        }
    }
}

impl std::error::Error for Error {}

/// An address: http or https, a host (a name, an IPv4 address or an IPv6
/// one in brackets), a port, and the path with its query ("/" if none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub https: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Url {
    pub fn parse(s: &str) -> Result<Url, Error> {
        let bad = || Error::Url(s.chars().take(200).collect());
        let s = s.trim();
        let (https, rest) = if let Some(r) = s.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = s.strip_prefix("http://") {
            (false, r)
        } else {
            return Err(bad());
        };
        let (authority, path) = match rest.find(['/', '?']) {
            Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], rest[i..].to_string()),
            Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
            None => (rest, "/".to_string()),
        };
        // A fragment is the reader's, never sent.
        let path = path.split('#').next().unwrap_or("/").to_string();
        if authority.contains('@') || authority.is_empty() {
            return Err(bad());
        }
        let default = if https { 443 } else { 80 };
        let (host, port) = if let Some(r) = authority.strip_prefix('[') {
            let end = r.find(']').ok_or_else(bad)?;
            let host = &r[..end];
            host.parse::<std::net::Ipv6Addr>().map_err(|_| bad())?;
            let port = match &r[end + 1..] {
                "" => default,
                p => p.strip_prefix(':').and_then(|p| p.parse().ok()).ok_or_else(bad)?,
            };
            (host.to_string(), port)
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), p.parse().map_err(|_| bad())?),
                None => (authority.to_string(), default),
            }
        };
        let ok = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':');
        if host.is_empty() || !host.chars().all(ok) || port == 0 || path.chars().any(|c| c.is_control() || c == ' ') {
            return Err(bad());
        }
        Ok(Url { https, host: host.to_ascii_lowercase(), port, path })
    }

    /// The same server, another path ("/rest/ping?..."): for a client that
    /// keeps the server's address and adds its calls.
    pub fn with_path(&self, path: &str) -> Url {
        let path = if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
        Url { path, ..self.clone() }
    }

    fn host_header(&self) -> String {
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        if self.port == if self.https { 443 } else { 80 } { host } else { format!("{host}:{}", self.port) }
    }

    /// What it is written as.
    pub fn to_text(&self) -> String {
        format!("{}://{}{}", if self.https { "https" } else { "http" }, self.host_header(), self.path)
    }
}

/// A query's value, percent-encoded (RFC 3986's unreserved characters kept).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Whether an address is on the local network (Dave, 2026-11-26: no warning
/// for HTTP to one): IPv4 private, link-local and loopback; IPv6 unique
/// local, link-local and loopback. Not 100.64/10 (carrier-grade NAT, shared
/// with an ISP's other customers).
pub fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_private() || v.is_link_local() || v.is_loopback(),
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return is_local(IpAddr::V4(v4));
            }
            let s = v.segments()[0];
            v.is_loopback() || (s & 0xfe00) == 0xfc00 || (s & 0xffc0) == 0xfe80
        }
    }
}

/// A connection the client can talk over.
pub trait Conn: Read + Write + Send {}
impl<T: Read + Write + Send> Conn for T {}

/// Makes the TLS stream over a connected socket, for HTTPS (the helper's).
pub trait Connector: Send + Sync {
    fn tls(&self, socket: TcpStream, host: &str) -> io::Result<Box<dyn Conn>>;
}

/// How a request is made.
#[derive(Clone)]
pub struct Options {
    /// Connecting may take this long.
    pub connect: Duration,
    /// The whole answer, its head and body, may take this long from the
    /// request (a stream's caller sets it long).
    pub answer: Duration,
    /// The body may be at most this many bytes.
    pub max_body: u64,
    /// The bytes wanted (from, through), inclusive; None for all.
    pub range: Option<(u64, Option<u64>)>,
    /// Sent as User-Agent.
    pub agent: String,
    pub tls: Option<std::sync::Arc<dyn Connector>>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            connect: Duration::from_secs(10),
            answer: Duration::from_secs(30),
            max_body: 64 * 1024 * 1024,
            range: None,
            agent: "Vergence".into(),
            tls: None,
        }
    }
}

/// An answer's head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

impl Head {
    /// A header's value, by its name in any case (the first if several).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Reads an answer's head from its first bytes: `Ok(None)` while it is not
/// all there yet, else the head and how many bytes it took.
pub fn parse_head(buf: &[u8]) -> Result<Option<(Head, usize)>, Error> {
    let bad = |s: &str| Error::BadAnswer(s.into());
    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        return if buf.len() > MAX_HEAD { Err(bad("its head is too long")) } else { Ok(None) };
    };
    if end > MAX_HEAD {
        return Err(bad("its head is too long"));
    }
    let text = std::str::from_utf8(&buf[..end]).map_err(|_| bad("its head is not text"))?;
    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or_default();
    if !(version == "HTTP/1.1" || version == "HTTP/1.0") {
        return Err(bad("not HTTP/1"));
    }
    let code = parts.next().unwrap_or_default();
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad("no status"));
    }
    let status: u16 = code.parse().map_err(|_| bad("no status"))?;
    if !(100..600).contains(&status) {
        return Err(bad("no status"));
    }
    let mut headers = Vec::new();
    for line in lines {
        if headers.len() >= MAX_HEADERS {
            return Err(bad("too many headers"));
        }
        let (name, value) = line.split_once(':').ok_or_else(|| bad("a header without a colon"))?;
        if name.is_empty() || name.bytes().any(|b| !(b.is_ascii_graphic()) || b == b':') {
            return Err(bad("a header's name"));
        }
        if value.bytes().any(|b| b.is_ascii_control() && b != b'\t') {
            return Err(bad("a header's value"));
        }
        headers.push((name.to_string(), value.trim().to_string()));
    }
    Ok(Some((Head { status, headers }, end + 4)))
}

/// A chunked body's decoder: fed what arrives, it gives the body's bytes,
/// across any split of the chunks' lines (RFC 9112 §7.1).
#[derive(Debug, Default)]
pub struct Chunked {
    /// Bytes left in the chunk being read, or None between chunks.
    left: Option<u64>,
    /// A size line or a chunk's CRLF being collected.
    line: Vec<u8>,
    /// After the data of a chunk: its CRLF still to come.
    crlf: bool,
    /// The last chunk was read; trailers are being skipped until an empty line.
    trailers: bool,
    pub done: bool,
}

impl Chunked {
    /// Takes `input`, appends the body's bytes found in it to `out`.
    pub fn feed(&mut self, mut input: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
        let bad = |s: &str| Error::BadAnswer(format!("chunked body: {s}"));
        while !input.is_empty() && !self.done {
            if let Some(n) = self.left {
                let take = (n as usize).min(input.len());
                out.extend_from_slice(&input[..take]);
                input = &input[take..];
                let rest = n - take as u64;
                if rest == 0 {
                    self.left = None;
                    self.crlf = true;
                } else {
                    self.left = Some(rest);
                }
                continue;
            }
            // A line: a size line, a chunk's closing CRLF, or a trailer.
            let Some(nl) = input.iter().position(|&b| b == b'\n') else {
                self.line.extend_from_slice(input);
                if self.line.len() > 1024 {
                    return Err(bad("a line too long"));
                }
                return Ok(());
            };
            self.line.extend_from_slice(&input[..=nl]);
            input = &input[nl + 1..];
            let line = std::mem::take(&mut self.line);
            if line.len() > 1024 {
                return Err(bad("a line too long"));
            }
            let line = line.strip_suffix(b"\n").unwrap_or(&line);
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if self.crlf {
                if !line.is_empty() {
                    return Err(bad("no CRLF after a chunk"));
                }
                self.crlf = false;
                continue;
            }
            if self.trailers {
                if line.is_empty() {
                    self.done = true;
                }
                continue;
            }
            let size = line.split(|&b| b == b';').next().unwrap_or_default();
            let size = std::str::from_utf8(size).map_err(|_| bad("a size"))?.trim();
            if size.is_empty() || size.len() > 15 {
                return Err(bad("a size"));
            }
            let n = u64::from_str_radix(size, 16).map_err(|_| bad("a size"))?;
            if n == 0 {
                self.trailers = true;
            } else {
                self.left = Some(n);
            }
        }
        Ok(())
    }
}

/// How the body ends.
enum Framing {
    Length(u64),
    Chunked(Chunked),
    Close,
}

/// An answer: its head, the address its server was reached at, and its
/// body to read (within the size limit and the deadline).
pub struct Response {
    pub head: Head,
    pub peer: SocketAddr,
    /// The address it was answered from, after any redirects.
    pub url: Url,
    conn: Box<dyn Conn>,
    framing: Framing,
    /// Bytes read past the head, not yet decoded; body bytes not yet given out.
    pending: Vec<u8>,
    decoded: Vec<u8>,
    /// Body bytes given out, and the most allowed.
    read: u64,
    max: u64,
    deadline: Instant,
    ended: bool,
    socket: Option<TcpStream>,
}

impl Response {
    pub fn status(&self) -> u16 {
        self.head.status
    }

    /// The body's length, if the server said.
    pub fn length(&self) -> Option<u64> {
        match self.framing {
            Framing::Length(n) => Some(n),
            _ => None,
        }
    }

    /// The whole body, at most `max_body` (else TooLarge).
    pub fn bytes(mut self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            match self.read(&mut buf) {
                Ok(0) => return Ok(out),
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) => return Err(unwrap_err(e)),
            }
        }
    }

    fn fill(&mut self) -> io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, Wrapped(Error::Timeout)));
        }
        if let Some(s) = &self.socket {
            let _ = s.set_read_timeout(Some(left.max(Duration::from_millis(1))));
        }
        let mut buf = [0u8; 16 * 1024];
        match self.conn.read(&mut buf) {
            Ok(n) => {
                self.pending.extend_from_slice(&buf[..n]);
                Ok(n)
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                Err(io::Error::new(io::ErrorKind::TimedOut, Wrapped(Error::Timeout)))
            }
            Err(e) => Err(e),
        }
    }
}

/// An Error carried through io::Read.
#[derive(Debug)]
struct Wrapped(Error);
impl std::fmt::Display for Wrapped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Wrapped {}

fn wrap(e: Error) -> io::Error {
    io::Error::other(Wrapped(e))
}

/// The client's error inside an io::Error, back out.
pub fn unwrap_err(e: io::Error) -> Error {
    match e.get_ref().and_then(|i| i.downcast_ref::<Wrapped>()) {
        Some(_) => match e.into_inner().and_then(|i| i.downcast::<Wrapped>().ok()) {
            Some(w) => w.0,
            None => Error::BadAnswer("lost".into()),
        },
        None => Error::Io(e),
    }
}

impl Read for Response {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            // Body bytes ready: given out, counted against the limit.
            if !self.decoded.is_empty() {
                let take = self.decoded.len().min(out.len());
                out[..take].copy_from_slice(&self.decoded[..take]);
                self.decoded.drain(..take);
                self.read += take as u64;
                if self.read > self.max {
                    return Err(wrap(Error::TooLarge));
                }
                return Ok(take);
            }
            if self.ended {
                return Ok(0);
            }
            // What arrived, turned into body bytes by the framing.
            if !self.pending.is_empty() {
                let raw = std::mem::take(&mut self.pending);
                match &mut self.framing {
                    Framing::Length(n) => {
                        let left = n.saturating_sub(self.read) as usize;
                        self.decoded.extend_from_slice(&raw[..left.min(raw.len())]);
                    }
                    Framing::Close => self.decoded.extend_from_slice(&raw),
                    Framing::Chunked(c) => c.feed(&raw, &mut self.decoded).map_err(wrap)?,
                }
                continue;
            }
            let finished = match &self.framing {
                Framing::Length(n) => self.read >= *n,
                Framing::Chunked(c) => c.done,
                Framing::Close => false,
            };
            if finished {
                self.ended = true;
                return Ok(0);
            }
            if self.fill()? == 0 {
                return match self.framing {
                    Framing::Close => {
                        self.ended = true;
                        Ok(0)
                    }
                    _ => Err(wrap(Error::BadAnswer("it ended early".into()))),
                };
            }
        }
    }
}

/// GET `url`: the answer's head read, its body ready to read. Redirects
/// within the same server are followed; any status is returned as it is
/// (the caller decides what a 404 means).
pub fn get(url: &Url, opts: &Options) -> Result<Response, Error> {
    let mut url = url.clone();
    for _ in 0..=MAX_REDIRECTS {
        let r = get_once(&url, opts)?;
        if matches!(r.head.status, 301 | 302 | 303 | 307 | 308)
            && let Some(loc) = r.head.header("location")
        {
            let next = if loc.starts_with("http://") || loc.starts_with("https://") {
                Url::parse(loc)?
            } else if loc.starts_with('/') {
                url.with_path(loc)
            } else {
                return Err(Error::Redirect("the server sent a redirect this cannot follow".into()));
            };
            if next.host != url.host || next.port != url.port || next.https != url.https {
                return Err(Error::Redirect(format!("the server sent it to another server ({})", next.host)));
            }
            url = next;
            continue;
        }
        return Ok(r);
    }
    Err(Error::Redirect("too many redirects".into()))
}

fn get_once(url: &Url, opts: &Options) -> Result<Response, Error> {
    let started = Instant::now();
    let deadline = started + opts.answer;
    if url.https && opts.tls.is_none() {
        return Err(Error::NoTls);
    }
    let addrs: Vec<SocketAddr> = (url.host.as_str(), url.port)
        .to_socket_addrs()
        .map_err(|e| Error::Resolve(format!("{}: {e}", url.host)))?
        .collect();
    if addrs.is_empty() {
        return Err(Error::Resolve(format!("{}: no address", url.host)));
    }
    let mut last = None;
    let mut found = None;
    for a in &addrs {
        match TcpStream::connect_timeout(a, opts.connect) {
            Ok(s) => {
                found = Some((s, *a));
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    let (socket, peer) = match found {
        Some(f) => f,
        None => return Err(Error::Connect(last.unwrap_or_else(|| io::Error::other("no address answered")))),
    };
    let _ = socket.set_nodelay(true);
    let _ = socket.set_write_timeout(Some(opts.connect));
    let timing = socket.try_clone().ok();
    let mut conn: Box<dyn Conn> = match (&opts.tls, url.https) {
        (Some(tls), true) => tls.tls(socket, &url.host).map_err(Error::Connect)?,
        _ => Box::new(socket),
    };
    let mut req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nAccept: */*\r\nAccept-Encoding: identity\r\nConnection: close\r\n",
        url.path,
        url.host_header(),
        opts.agent.replace(['\r', '\n'], " ")
    );
    if let Some((from, to)) = opts.range {
        req.push_str(&match to {
            Some(t) => format!("Range: bytes={from}-{t}\r\n"),
            None => format!("Range: bytes={from}-\r\n"),
        });
    }
    req.push_str("\r\n");
    conn.write_all(req.as_bytes()).map_err(Error::Io)?;
    conn.flush().map_err(Error::Io)?;
    // The head.
    let mut buf: Vec<u8> = Vec::new();
    let mut piece = [0u8; 8 * 1024];
    let (head, used) = loop {
        if let Some(h) = parse_head(&buf)? {
            break h;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(Error::Timeout);
        }
        if let Some(s) = &timing {
            let _ = s.set_read_timeout(Some(left));
        }
        match conn.read(&mut piece) {
            Ok(0) => return Err(Error::BadAnswer("it closed before answering".into())),
            Ok(n) => buf.extend_from_slice(&piece[..n]),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                return Err(Error::Timeout);
            }
            Err(e) => return Err(Error::Io(e)),
        }
    };
    let pending = buf[used..].to_vec();
    let chunked = head.header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let framing = if chunked {
        Framing::Chunked(Chunked::default())
    } else if let Some(n) = head.header("content-length") {
        let n: u64 = n.trim().parse().map_err(|_| Error::BadAnswer("its length".into()))?;
        if n > opts.max_body {
            return Err(Error::TooLarge);
        }
        Framing::Length(n)
    } else if head.status == 204 || head.status == 304 {
        Framing::Length(0)
    } else {
        Framing::Close
    };
    Ok(Response {
        head,
        peer,
        url: url.clone(),
        conn,
        framing,
        pending,
        decoded: Vec::new(),
        read: 0,
        max: opts.max_body,
        deadline,
        ended: false,
        socket: timing,
    })
}

#[cfg(test)]
mod tests;
