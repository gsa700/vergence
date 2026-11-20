//! The sandboxed PDF helper and its client (PDF milestone 1, step 6).
//!
//! A PDF is untrusted input, so nothing in a window parses one: the
//! helper `libexec/vergence/vergence-pdf` does, confined as the picture
//! helper is (`vergence_decode::confine`: no file system or network, no new
//! descriptors, limits; Capsicum on FreeBSD, seccomp on Linux). Unlike the
//! picture helper it stays: one per open document, answering requests on
//! its pipe while the window lives. The client ([`Viewer`]) sends the file
//! itself (the helper opens nothing), times every answer, and starts a
//! new helper after a crash or a hang, so a bad page is an error on that
//! page, never a crash of the app.
//!
//! The pipe: a request is an op byte, a u32 length (big-endian) and that
//! many bytes; an answer is a status byte (0 ok, 1 refused with a reason
//! in UTF-8), a u32 length and the bytes.
//!
//! - `D` the document's bytes. Answer: the [`Info`].
//! - `U` a password. Answer: the [`Info`] (`needs_password` false if it
//!   opened the file), or refused.
//! - `R` page u32, scale f32 (pixels per point), x, y, width, height u32
//!   (width 0: the whole page). Answer: width, height u32, then
//!   premultiplied RGBA rows (opaque: pages have a white ground).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The largest document sent to the helper.
pub const MAX_FILE: usize = 1 << 30;
/// The most pixels one answer carries (the client asks for tiles beyond).
pub const MAX_PIXELS: usize = 1 << 25;
/// How long one page may take before its helper is stopped.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// The helper's processor time over its life, seconds.
pub const CPU_SECONDS: u64 = 4 * 3600;

/// What the helper says about a document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Info {
    /// Encrypted, and no password yet opens it (no pages until one does).
    pub needs_password: bool,
    /// Each page's size as shown, in points.
    pub pages: Vec<(f32, f32)>,
}

impl Info {
    fn encode(&self) -> Vec<u8> {
        let mut out = vec![self.needs_password as u8];
        out.extend((self.pages.len() as u32).to_be_bytes());
        for (w, h) in &self.pages {
            out.extend(w.to_be_bytes());
            out.extend(h.to_be_bytes());
        }
        out
    }

    fn decode(b: &[u8]) -> Option<Info> {
        let needs_password = *b.first()? != 0;
        let n = u32::from_be_bytes(b.get(1..5)?.try_into().ok()?) as usize;
        let rest = b.get(5..)?;
        if rest.len() != n.checked_mul(8)? {
            return None;
        }
        let f = |c: &[u8]| f32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        let pages = rest.as_chunks::<8>().0.iter().map(|c| (f(&c[..4]), f(&c[4..]))).collect();
        Some(Info { needs_password, pages })
    }
}

/// A page or a part of it, drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA, rows from the top, opaque.
    pub rgba: Vec<u8>,
}

fn read_frame(r: &mut impl Read, limit: usize) -> std::io::Result<Option<(u8, Vec<u8>)>> {
    let mut head = [0u8; 5];
    match r.read_exact(&mut head) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > limit {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "a frame too large"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(Some((head[0], body)))
}

fn write_frame(w: &mut impl Write, kind: u8, body: &[u8]) -> std::io::Result<()> {
    w.write_all(&[kind])?;
    w.write_all(&(body.len() as u32).to_be_bytes())?;
    w.write_all(body)?;
    w.flush()
}

/// The helper's work: requests from `input`, answers to `output`, until
/// the pipe closes. A panic while drawing refuses that request and the
/// helper goes on.
pub fn serve(input: &mut impl Read, output: &mut impl Write) -> std::io::Result<()> {
    let mut doc: Option<crate::Document> = None;
    let mut pages: Vec<crate::Page> = Vec::new();
    let info = |doc: &crate::Document, pages: &[crate::Page]| Info {
        needs_password: doc.needs_password(),
        pages: pages
            .iter()
            .map(|p| {
                let (w, h) = crate::page_size(p);
                (w as f32, h as f32)
            })
            .collect(),
    };
    while let Some((op, body)) = read_frame(input, MAX_FILE)? {
        let answer: Result<Vec<u8>, String> = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match op {
            b'D' => match crate::Document::open(body) {
                Ok(d) => {
                    pages = if d.needs_password() { Vec::new() } else { d.pages() };
                    let i = info(&d, &pages);
                    doc = Some(d);
                    Ok(i.encode())
                }
                Err(e) => Err(format!("{e:?}")),
            },
            b'U' => {
                let d = doc.as_mut().ok_or("no document")?;
                if !d.unlock(&body) {
                    return Err("wrong password".to_string());
                }
                pages = d.pages();
                Ok(info(d, &pages).encode())
            }
            b'R' => {
                let d = doc.as_ref().ok_or("no document")?;
                if body.len() != 24 {
                    return Err("a bad request".into());
                }
                let u = |i: usize| u32::from_be_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]) as usize;
                let page = pages.get(u(0)).ok_or("no such page")?;
                let scale = f32::from_be_bytes([body[4], body[5], body[6], body[7]]) as f64;
                if !(scale.is_finite() && scale > 0.0 && scale <= 64.0) {
                    return Err("a bad scale".into());
                }
                let region = if u(16) == 0 { None } else { Some((u(8), u(12), u(16), u(20))) };
                let (pw, ph) = crate::page_size(page);
                let (w, h) = region.map_or(((pw * scale).ceil() as usize, (ph * scale).ceil() as usize), |r| (r.2, r.3));
                if w.saturating_mul(h) > MAX_PIXELS {
                    return Err("too many pixels: ask for tiles".into());
                }
                let c = crate::render_region(d, page, scale, region).ok_or("the page could not be drawn")?;
                let mut out = Vec::with_capacity(8 + c.data.len());
                out.extend((c.width as u32).to_be_bytes());
                out.extend((c.height as u32).to_be_bytes());
                out.extend_from_slice(&c.data);
                Ok(out)
            }
            _ => Err("an unknown request".into()),
        }))
        .unwrap_or_else(|_| Err("the page could not be drawn (a fault in the file)".into()));
        match answer {
            Ok(b) => write_frame(output, 0, &b)?,
            Err(why) => write_frame(output, 1, why.as_bytes())?,
        }
    }
    Ok(())
}

/// The helper program's main: the substitute fonts read, then confined,
/// then [`serve`] on the standard descriptors.
pub fn helper_main() -> std::process::ExitCode {
    crate::font::preload_substitutes();
    vergence_decode::confine(CPU_SECONDS);
    let (mut i, mut o) = (std::io::stdin().lock(), std::io::stdout().lock());
    match serve(&mut i, &mut o) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => std::process::ExitCode::from(1),
    }
}

/// Why a document could not be shown.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    NoHelper,
    TooLarge,
    /// The helper said no (not a PDF, wrong password, a page it could not draw).
    Refused(String),
    /// The helper stopped or ran past the time; a new one starts on the next request.
    Crashed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::NoHelper => write!(f, "the PDF helper (libexec/vergence/vergence-pdf) is not installed"),
            Error::TooLarge => write!(f, "the file is larger than {} MB", MAX_FILE >> 20),
            Error::Refused(why) => write!(f, "{why}"),
            Error::Crashed(how) => write!(f, "the PDF reader stopped: {how}"),
        }
    }
}

/// The helper: beside this program (a checkout's target/), else
/// PREFIX/libexec/vergence, else /usr/local's.
fn helper_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("vergence-pdf"), dir.parent()?.join("libexec/vergence/vergence-pdf"), PathBuf::from("/usr/local/libexec/vergence/vergence-pdf")].into_iter().find(|p| p.is_file())
}

struct Running {
    child: std::process::Child,
    to: std::process::ChildStdin,
    from: std::process::ChildStdout,
}

/// An open document, through its helper.
pub struct Viewer {
    program: PathBuf,
    data: Vec<u8>,
    password: Option<Vec<u8>>,
    running: Option<Running>,
    pub info: Info,
}

impl Viewer {
    /// Opens a PDF file in a new helper.
    pub fn open(path: &Path) -> Result<Viewer, Error> {
        let mut file = vergence_types::open_regular(path).map_err(Error::Io)?;
        let mut data = Vec::new();
        Read::by_ref(&mut file).take(MAX_FILE as u64 + 1).read_to_end(&mut data).map_err(Error::Io)?;
        if data.len() > MAX_FILE {
            return Err(Error::TooLarge);
        }
        Viewer::with_bytes(helper_path().ok_or(Error::NoHelper)?, data)
    }

    /// A document's bytes in the helper at `program` (tests use the one cargo built).
    pub fn with_bytes(program: PathBuf, data: Vec<u8>) -> Result<Viewer, Error> {
        let mut v = Viewer { program, data, password: None, running: None, info: Info::default() };
        v.start()?;
        Ok(v)
    }

    /// A helper started, the document (and password) given to it again.
    fn start(&mut self) -> Result<(), Error> {
        self.stop();
        let mut child = std::process::Command::new(&self.program).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().map_err(Error::Io)?;
        let (Some(to), Some(from)) = (child.stdin.take(), child.stdout.take()) else { return Err(Error::Crashed("no pipe".into())) };
        self.running = Some(Running { child, to, from });
        let data = std::mem::take(&mut self.data);
        let answer = self.ask(b'D', &data);
        self.data = data;
        self.info = Info::decode(&answer?).ok_or_else(|| Error::Crashed("a garbled answer".into()))?;
        if let Some(pw) = self.password.clone() {
            self.info = Info::decode(&self.ask(b'U', &pw)?).ok_or_else(|| Error::Crashed("a garbled answer".into()))?;
        }
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(mut r) = self.running.take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
    }

    /// One request and its answer, timed: past the time, or on a broken
    /// pipe, the helper is stopped (the next request starts another).
    fn ask(&mut self, op: u8, body: &[u8]) -> Result<Vec<u8>, Error> {
        let r = self.running.as_mut().ok_or_else(|| Error::Crashed("not running".into()))?;
        let pid = r.child.id();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let watchman = std::thread::spawn(move || {
            if done_rx.recv_timeout(TIMEOUT) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                unsafe extern "C" {
                    fn kill(pid: i32, sig: i32) -> i32;
                }
                // SAFETY: kill(2) on our own child; no memory passed.
                unsafe { kill(pid as i32, 9) };
                return true;
            }
            false
        });
        let result = write_frame(&mut r.to, op, body).and_then(|_| read_frame(&mut r.from, MAX_PIXELS * 4 + 64));
        let _ = done_tx.send(());
        let timed_out = watchman.join().unwrap_or(false);
        match result {
            Ok(Some((0, b))) => Ok(b),
            Ok(Some((_, why))) => Err(Error::Refused(String::from_utf8_lossy(&why).into_owned())),
            Ok(None) | Err(_) => {
                self.stop();
                Err(Error::Crashed(if timed_out { "it ran past the time".into() } else { "it stopped (a limit, or a fault in the file)".into() }))
            }
        }
    }

    /// A request, with a fresh helper first if the last one stopped.
    fn request(&mut self, op: u8, body: &[u8]) -> Result<Vec<u8>, Error> {
        if self.running.is_none() {
            self.start()?;
        }
        self.ask(op, body)
    }

    /// Tries a password; true if it opened the document.
    pub fn unlock(&mut self, password: &[u8]) -> Result<bool, Error> {
        match self.request(b'U', password) {
            Ok(b) => {
                self.info = Info::decode(&b).ok_or_else(|| Error::Crashed("a garbled answer".into()))?;
                self.password = Some(password.to_vec());
                Ok(true)
            }
            Err(Error::Refused(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Page `page` (from 0) at `scale` pixels per point, whole or only
    /// `region` (x, y, width, height in those pixels).
    pub fn render(&mut self, page: usize, scale: f32, region: Option<(u32, u32, u32, u32)>) -> Result<Tile, Error> {
        let mut body = Vec::with_capacity(24);
        body.extend((page as u32).to_be_bytes());
        body.extend(scale.to_be_bytes());
        let (x, y, w, h) = region.unwrap_or((0, 0, 0, 0));
        for v in [x, y, w, h] {
            body.extend(v.to_be_bytes());
        }
        let b = self.request(b'R', &body)?;
        let garbled = || Error::Crashed("a garbled answer".into());
        let width = u32::from_be_bytes(b.get(0..4).ok_or_else(garbled)?.try_into().map_err(|_| garbled())?);
        let height = u32::from_be_bytes(b.get(4..8).ok_or_else(garbled)?.try_into().map_err(|_| garbled())?);
        if (width as usize).saturating_mul(height as usize).saturating_mul(4) != b.len() - 8 {
            return Err(garbled());
        }
        Ok(Tile { width, height, rgba: b[8..].to_vec() })
    }
}

impl Viewer {
    /// Stops the helper as a crash would (tests: the next request must
    /// say so, the one after start a new helper).
    #[doc(hidden)]
    pub fn kill_helper(&mut self) {
        if let Some(r) = self.running.as_mut() {
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
    }
}

/// A one-page PDF for tests: a black square (10, 10, 20 x 20) on a 100 x
/// 50 point page.
#[doc(hidden)]
pub fn tiny_pdf() -> Vec<u8> {
    let content = b"0 0 0 rg 10 10 20 20 re f";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 50] /Contents 4 0 R >>".to_string(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), String::from_utf8_lossy(content)),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

impl Drop for Viewer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::helper::tiny_pdf;


    fn frame(op: u8, body: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        write_frame(&mut v, op, body).unwrap();
        v
    }

    fn answers(mut out: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut v = Vec::new();
        while let Some(f) = read_frame(&mut out, usize::MAX).unwrap() {
            v.push(f);
        }
        v
    }

    #[test]
    fn serve_opens_draws_and_tiles() {
        let mut req = frame(b'D', &tiny_pdf());
        let mut r = Vec::new();
        r.extend(0u32.to_be_bytes());
        r.extend(2.0f32.to_be_bytes());
        r.extend([0u8; 16]);
        req.extend(frame(b'R', &r));
        // The tile holding the square's top-left corner at scale 2: (20, 40) .. (60, 80).
        let mut t = Vec::new();
        t.extend(0u32.to_be_bytes());
        t.extend(2.0f32.to_be_bytes());
        for v in [10u32, 30, 20, 20] {
            t.extend(v.to_be_bytes());
        }
        req.extend(frame(b'R', &t));
        req.extend(frame(b'R', &[1, 2, 3]));
        let mut out = Vec::new();
        serve(&mut &req[..], &mut out).unwrap();
        let a = answers(&out);
        assert_eq!(a.len(), 4);
        assert_eq!(a[0].0, 0);
        assert_eq!(Info::decode(&a[0].1), Some(Info { needs_password: false, pages: vec![(100.0, 50.0)] }));
        let (w, h) = (u32::from_be_bytes(a[1].1[0..4].try_into().unwrap()), u32::from_be_bytes(a[1].1[4..8].try_into().unwrap()));
        assert_eq!((a[1].0, w, h), (0, 200, 100));
        let px = |b: &[u8], w: usize, x: usize, y: usize| b[8 + (y * w + x) * 4];
        // The square covers x 20..60, y (from the top) 40..80 at scale 2.
        assert_eq!(px(&a[1].1, 200, 30, 60), 0);
        assert_eq!(px(&a[1].1, 200, 100, 20), 255);
        // The tile: x 10..30, y 30..50; inside the square from (20, 40).
        assert_eq!(&a[2].1[0..8], &[0, 0, 0, 20, 0, 0, 0, 20]);
        assert_eq!(px(&a[2].1, 20, 15, 15), 0);
        assert_eq!(px(&a[2].1, 20, 2, 2), 255);
        assert_eq!(a[3].0, 1);
    }

    #[test]
    fn serve_refuses_garbage_and_goes_on() {
        let mut req = frame(b'D', b"not a pdf at all");
        req.extend(frame(b'Z', b""));
        req.extend(frame(b'D', &tiny_pdf()));
        let mut out = Vec::new();
        serve(&mut &req[..], &mut out).unwrap();
        let a = answers(&out);
        assert_eq!(a.iter().map(|f| f.0).collect::<Vec<_>>(), vec![1, 1, 0]);
    }
}
