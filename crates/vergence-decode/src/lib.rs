//! Pictures decoded apart (files milestone 1, step 4; whitepaper §6.10:
//! decoding untrusted input happens in a process that can do nothing else).
//!
//! A program asks with [`decode_file`]: the file is opened here and given
//! to the helper `vergence-decode` (libexec) as its standard input, so the
//! helper needs no file system at all. The helper ([`helper_main`]) reads
//! it, then limits itself before decoding: on FreeBSD it enters Capsicum
//! capability mode (`cap_enter`: no file system, no network, no new
//! descriptors); on Linux it sets no-new-privileges and enters a seccomp
//! filter that allows only memory, the standard descriptors, locks and the
//! end (x86_64; another processor is refused until it has its list); on
//! both it closes every other descriptor and lowers its limits: CPU time,
//! memory, file size, open files (three: no new one). A limit that cannot
//! be set ends it before it decodes. It writes the picture to standard output and ends. A crash, a
//! hang (killed after [`TIMEOUT`]) or an error in a bad file is an error
//! here, never a crash of the program that asked.
//!
//! Formats: PNG, JPEG (turned upright by its orientation tag), GIF and
//! WebP (with their animation frames) by the `image` crate (borrowed, in
//! docs/borrowed-review.md); SVG by resvg, with no outside resources; BMP by
//! our own decoder (bmp.rs). By the file's bytes, not its name.
//!
//! The answer on the pipe: `VDEC1`, then width, height and the number of
//! frames (u32, little-endian), then each frame: its delay in milliseconds
//! and width x height x 4 bytes of straight RGBA.

pub mod bmp;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the helper has for one picture.
pub const TIMEOUT: Duration = Duration::from_secs(5);
/// The largest file the helper reads.
pub const MAX_FILE: usize = 512 << 20;
/// The most bytes of pixels one answer may carry (all frames).
pub const MAX_PIXEL_BYTES: usize = 1 << 30;
/// The most animation frames kept.
pub const MAX_FRAMES: usize = 500;
const MAGIC: &[u8; 5] = b"VDEC1";

/// One frame: how long it shows (0 for a still picture) and its pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub delay_ms: u32,
    pub rgba: Vec<u8>,
}

/// A decoded picture: width, height, and its frames (one for a still).
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<Frame>,
}

/// Why a picture could not be shown.
#[derive(Debug)]
pub enum Error {
    /// The file could not be opened or read here.
    Io(std::io::Error),
    /// The helper is not installed.
    NoHelper,
    /// The helper said why (not a picture, a format not supported, a bad file).
    Refused(String),
    /// The helper took too long and was stopped.
    TimedOut,
    /// The helper ended without an answer (crashed, or killed by its limits).
    Crashed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::NoHelper => write!(f, "the picture decoder (libexec/vergence/vergence-decode) is not installed"),
            Error::Refused(why) => write!(f, "{why}"),
            Error::TimedOut => write!(f, "decoding took too long"),
            Error::Crashed(how) => write!(f, "the decoder stopped: {how}"),
        }
    }
}

/// The helper: beside this program (a checkout's target/), else
/// PREFIX/libexec/vergence, else /usr/local's.
fn helper() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("vergence-decode"), dir.parent()?.join("libexec/vergence/vergence-decode"), PathBuf::from("/usr/local/libexec/vergence/vergence-decode")].into_iter().find(|p| p.is_file())
}

/// Decodes a picture file in the helper. `at_least`: shrink it (never
/// enlarge) so that it is still at least this wide and this tall (what a
/// preview filling that box needs); None for every pixel.
pub fn decode_file(path: &Path, at_least: Option<(u32, u32)>) -> Result<Picture, Error> {
    let file = vergence_types::open_regular(path).map_err(Error::Io)?;
    let helper = helper().ok_or(Error::NoHelper)?;
    let mut cmd = std::process::Command::new(helper);
    if let Some((w, h)) = at_least {
        cmd.arg(format!("--at-least={w}x{h}"));
    }
    let mut child = cmd.stdin(file).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(Error::Io)?;
    let pid = child.id();
    // A watchman: kills the helper if it runs past the time.
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let watchman = std::thread::spawn(move || {
        if done_rx.recv_timeout(TIMEOUT).is_err() {
            unsafe extern "C" {
                fn kill(pid: i32, sig: i32) -> i32;
            }
            // SAFETY: kill(2) on our own child; no memory passed.
            unsafe { kill(pid as i32, 9) };
            return true;
        }
        false
    });
    let mut out = Vec::new();
    let read = child.stdout.take().map(|so| so.take(MAX_PIXEL_BYTES as u64 + 64).read_to_end(&mut out));
    let mut why = String::new();
    if let Some(mut se) = child.stderr.take() {
        let _ = se.by_ref().take(4096).read_to_string(&mut why);
    }
    let status = child.wait().map_err(Error::Io)?;
    let _ = done_tx.send(());
    let timed_out = watchman.join().unwrap_or(false);
    if timed_out {
        return Err(Error::TimedOut);
    }
    if let Some(Err(e)) = read {
        return Err(Error::Io(e));
    }
    match status.code() {
        Some(0) => parse(&out).ok_or_else(|| Error::Crashed("a garbled answer".into())),
        Some(1) => Err(Error::Refused(why.trim().to_string())),
        Some(c) => Err(Error::Crashed(format!("exit {c}: {}", why.trim()))),
        None => Err(Error::Crashed("killed (a limit, or a fault in the file)".into())),
    }
}

/// An answer read back.
fn parse(b: &[u8]) -> Option<Picture> {
    if b.get(..5)? != MAGIC {
        return None;
    }
    let u = |at: usize| -> Option<u32> { Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?)) };
    let (width, height, n) = (u(5)?, u(9)?, u(13)? as usize);
    let size = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
    if n == 0 || n > MAX_FRAMES || size.checked_mul(n)? > MAX_PIXEL_BYTES {
        return None;
    }
    let mut at = 17;
    let mut frames = Vec::with_capacity(n);
    for _ in 0..n {
        let delay_ms = u(at)?;
        let rgba = b.get(at + 4..at + 4 + size)?.to_vec();
        frames.push(Frame { delay_ms, rgba });
        at += 4 + size;
    }
    (at == b.len()).then_some(Picture { width, height, frames })
}

/// A picture as the answer's bytes.
fn encode(p: &Picture) -> Vec<u8> {
    let mut out = Vec::with_capacity(17 + p.frames.iter().map(|f| f.rgba.len() + 4).sum::<usize>());
    out.extend(MAGIC);
    for v in [p.width, p.height, p.frames.len() as u32] {
        out.extend(v.to_le_bytes());
    }
    for f in &p.frames {
        out.extend(f.delay_ms.to_le_bytes());
        out.extend(&f.rgba);
    }
    out
}

/// The size that keeps `w` x `h`'s shape and is at least `min` on both
/// sides, never larger than it is.
fn shrunk(w: u32, h: u32, min: (u32, u32)) -> (u32, u32) {
    let s = (min.0 as f64 / w as f64).max(min.1 as f64 / h as f64).min(1.0);
    (((w as f64 * s).round() as u32).max(1), ((h as f64 * s).round() as u32).max(1))
}

fn resized(w: u32, h: u32, rgba: Vec<u8>, to: (u32, u32)) -> Vec<u8> {
    if to == (w, h) {
        return rgba;
    }
    match image::RgbaImage::from_raw(w, h, rgba) {
        Some(img) => image::imageops::resize(&img, to.0, to.1, image::imageops::FilterType::Triangle).into_raw(),
        None => vec![0; to.0 as usize * to.1 as usize * 4],
    }
}

/// Decodes a picture's bytes, here (the helper's work; tests call it too).
pub fn decode_bytes(data: &[u8], at_least: Option<(u32, u32)>) -> Result<Picture, String> {
    use image::{AnimationDecoder, ImageDecoder};
    let mime = vergence_types::of_bytes(data).unwrap_or(vergence_types::BINARY);
    let still = |w: u32, h: u32, rgba: Vec<u8>| -> Picture {
        let to = at_least.map_or((w, h), |m| shrunk(w, h, m));
        Picture { width: to.0, height: to.1, frames: vec![Frame { delay_ms: 0, rgba: resized(w, h, rgba, to) }] }
    };
    let animated = |frames: Vec<image::Frame>| -> Result<Picture, String> {
        let mut out: Option<Picture> = None;
        let mut bytes = 0usize;
        for f in frames.into_iter().take(MAX_FRAMES) {
            let (num, den) = f.delay().numer_denom_ms();
            let delay_ms = if den == 0 { 100 } else { (num / den).max(20) };
            let buf = f.into_buffer();
            let (w, h) = buf.dimensions();
            let to = at_least.map_or((w, h), |m| shrunk(w, h, m));
            bytes += to.0 as usize * to.1 as usize * 4;
            if bytes > MAX_PIXEL_BYTES {
                break;
            }
            let rgba = resized(w, h, buf.into_raw(), to);
            match &mut out {
                Some(p) if (p.width, p.height) == to => p.frames.push(Frame { delay_ms, rgba }),
                Some(_) => break,
                None => out = Some(Picture { width: to.0, height: to.1, frames: vec![Frame { delay_ms, rgba }] }),
            }
        }
        out.ok_or_else(|| "no frames".to_string())
    };
    match mime {
        "image/png" | "image/jpeg" => {
            let reader = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().map_err(|e| e.to_string())?;
            let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
            let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
            let mut img = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
            img.apply_orientation(orientation);
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            Ok(still(w, h, rgba.into_raw()))
        }
        "image/gif" => {
            let d = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(data)).map_err(|e| e.to_string())?;
            let frames = d.into_frames().collect_frames().map_err(|e| e.to_string())?;
            let mut p = animated(frames)?;
            if p.frames.len() == 1 {
                p.frames[0].delay_ms = 0;
            }
            Ok(p)
        }
        "image/webp" => {
            let d = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(data)).map_err(|e| e.to_string())?;
            if d.has_animation() {
                let frames = d.into_frames().collect_frames().map_err(|e| e.to_string())?;
                return animated(frames);
            }
            let img = image::DynamicImage::from_decoder(d).map_err(|e| e.to_string())?.to_rgba8();
            let (w, h) = img.dimensions();
            Ok(still(w, h, img.into_raw()))
        }
        "image/bmp" => {
            let (w, h, rgba) = bmp::decode(data)?;
            Ok(still(w, h, rgba))
        }
        "image/svg+xml" => {
            // No outside files or fonts: what the drawing holds itself.
            let opt = resvg::usvg::Options::default();
            let tree = resvg::usvg::Tree::from_data(data, &opt).map_err(|e| e.to_string())?;
            let (sw, sh) = (tree.size().width(), tree.size().height());
            // A drawing has no pixels of its own: drawn at its size, or as large as asked.
            let s = at_least.map_or(1.0, |(mw, mh)| (mw as f32 / sw).max(mh as f32 / sh)).clamp(0.01, 64.0);
            let (w, h) = (((sw * s).ceil() as u32).clamp(1, 16384), ((sh * s).ceil() as u32).clamp(1, 16384));
            let mut canvas = resvg::tiny_skia::Pixmap::new(w, h).ok_or("SVG: no canvas")?;
            resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(w as f32 / sw, h as f32 / sh), &mut canvas.as_mut());
            // tiny-skia is premultiplied; answers are straight.
            let mut px = canvas.take();
            for p in px.chunks_exact_mut(4) {
                let a = p[3] as u32;
                if a > 0 && a < 255 {
                    for c in &mut p[..3] {
                        *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
            Ok(Picture { width: w, height: h, frames: vec![Frame { delay_ms: 0, rgba: px }] })
        }
        other => Err(format!("{} ({other}) is not a picture this can show", vergence_types::describe(other))),
    }
}

/// The helper's limits, set before it decodes (see the crate's comment).
/// Any limit that cannot be set ends the helper (exit 3): it never decodes
/// with less confinement than it means to have (security pass 2026-11-15,
/// finding 3).
fn limit_self() {
    #[repr(C)]
    struct Rlimit {
        cur: u64,
        max: u64,
    }
    unsafe extern "C" {
        fn setrlimit(resource: i32, rlim: *const Rlimit) -> i32;
        fn close(fd: i32) -> i32;
    }
    fn refuse(what: &str) -> ! {
        eprintln!("vergence-decode: {what} could not be set; not decoding");
        std::process::exit(3);
    }
    // CPU seconds, file size (none written), open files (only the three
    // standard ones: no new descriptor at all), address space.
    #[cfg(target_os = "linux")]
    const LIMITS: [(i32, u64); 4] = [(0, 10), (1, 0), (7, 3), (9, 4 << 30)]; // CPU, FSIZE, NOFILE, AS
    #[cfg(target_os = "freebsd")]
    const LIMITS: [(i32, u64); 4] = [(0, 10), (1, 0), (8, 3), (10, 4 << 30)]; // CPU, FSIZE, NOFILE, VMEM (AS)
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    const LIMITS: [(i32, u64); 0] = [];
    // Nothing but the three standard descriptors stays open.
    for fd in 3..1024 {
        // SAFETY: close(2) on descriptors this process may hold; none is used after.
        unsafe { close(fd) };
    }
    for (resource, value) in LIMITS {
        let r = Rlimit { cur: value, max: value };
        // SAFETY: setrlimit(2) with a valid struct of two rlim_t.
        if unsafe { setrlimit(resource, &r) } != 0 {
            refuse("a resource limit");
        }
    }
    #[cfg(target_os = "linux")]
    {
        unsafe extern "C" {
            fn prctl(option: i32, a: u64, b: u64, c: u64, d: u64) -> i32;
        }
        // SAFETY: PR_SET_NO_NEW_PRIVS (38) takes plain numbers.
        if unsafe { prctl(38, 1, 0, 0, 0) } != 0 {
            refuse("no-new-privileges");
        }
        if let Err(what) = seccomp::enter() {
            refuse(what);
        }
    }
    #[cfg(target_os = "freebsd")]
    {
        unsafe extern "C" {
            fn cap_enter() -> i32;
        }
        // SAFETY: cap_enter(2) takes nothing; from here no file system or network.
        if unsafe { cap_enter() } != 0 {
            refuse("capability mode");
        }
    }
}

/// Linux's counterpart of Capsicum for the helper (security pass
/// 2026-11-15, finding 2): a seccomp filter that allows only what decoding
/// a picture already in memory and writing the answer needs (memory,
/// reading and writing the standard descriptors, locks, the end). No open,
/// no socket, no unlink or rename, no signal to another process: any other
/// system call kills the helper, which its caller reports as a crash.
#[cfg(target_os = "linux")]
mod seccomp {
    #[repr(C)]
    struct SockFilter {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }
    #[repr(C)]
    struct SockFprog {
        len: u16,
        filter: *const SockFilter,
    }

    const LD_W_ABS: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS
    const JEQ_K: u16 = 0x15; // BPF_JMP | BPF_JEQ | BPF_K
    const RET_K: u16 = 0x06; // BPF_RET | BPF_K
    const RET_ALLOW: u32 = 0x7fff_0000;
    const RET_KILL_PROCESS: u32 = 0x8000_0000;
    // struct seccomp_data: nr at 0, arch at 4.
    const OFF_NR: u32 = 0;
    const OFF_ARCH: u32 = 4;

    #[cfg(target_arch = "x86_64")]
    const ARCH: u32 = 0xC000_003E; // AUDIT_ARCH_X86_64
    /// read, write, close, fstat, lseek, mmap, mprotect, munmap, brk,
    /// rt_sigreturn, readv, writev, sched_yield, mremap, madvise, exit,
    /// futex, sched_getaffinity, clock_gettime, exit_group, newfstatat,
    /// getrandom, statx.
    #[cfg(target_arch = "x86_64")]
    pub const ALLOWED: &[u32] = &[0, 1, 3, 5, 8, 9, 10, 11, 12, 14, 15, 19, 20, 24, 25, 28, 60, 202, 204, 228, 231, 262, 318, 332];

    /// The filter program: the wrong architecture or a call not listed is
    /// killed.
    #[cfg(target_arch = "x86_64")]
    fn program() -> Vec<SockFilter> {
        let f = |code, jt, jf, k| SockFilter { code, jt, jf, k };
        let mut p = vec![f(LD_W_ABS, 0, 0, OFF_ARCH), f(JEQ_K, 1, 0, ARCH), f(RET_K, 0, 0, RET_KILL_PROCESS), f(LD_W_ABS, 0, 0, OFF_NR)];
        for &nr in ALLOWED {
            p.push(f(JEQ_K, 0, 1, nr));
            p.push(f(RET_K, 0, 0, RET_ALLOW));
        }
        p.push(f(RET_K, 0, 0, RET_KILL_PROCESS));
        p
    }

    #[cfg(target_arch = "x86_64")]
    pub fn enter() -> Result<(), &'static str> {
        unsafe extern "C" {
            fn prctl(option: i32, a: u64, b: u64, c: u64, d: u64) -> i32;
        }
        let prog = program();
        let fprog = SockFprog { len: prog.len() as u16, filter: prog.as_ptr() };
        // SAFETY: PR_SET_SECCOMP (22), SECCOMP_MODE_FILTER (2) with a valid
        // sock_fprog whose filter outlives the call (the kernel copies it).
        if unsafe { prctl(22, 2, &fprog as *const SockFprog as u64, 0, 0) } != 0 {
            return Err("the seccomp filter");
        }
        Ok(())
    }

    /// No list of system calls for this processor yet: refused rather than
    /// decoding unconfined.
    #[cfg(not(target_arch = "x86_64"))]
    pub fn enter() -> Result<(), &'static str> {
        Err("the seccomp filter (no system call list for this processor)")
    }
}

/// For the tests: limit itself as for a picture, then try to open a file.
/// Exit 4 if refused (FreeBSD's capability mode says ECAPMODE); on Linux the
/// seccomp filter kills it at the open instead. Exit 0 would mean it is not
/// confined.
fn check_confined() -> std::process::ExitCode {
    limit_self();
    match std::fs::File::open("/") {
        Ok(_) => {
            eprintln!("vergence-decode: NOT confined: a file could be opened");
            std::process::ExitCode::SUCCESS
        }
        Err(_) => std::process::ExitCode::from(4),
    }
}

/// The helper's whole work: the file from standard input, limits, the
/// picture to standard output. Exit 0 with an answer; 1 with why on
/// standard error; 2 for bad arguments.
pub fn helper_main() -> std::process::ExitCode {
    let mut at_least = None;
    for a in std::env::args().skip(1) {
        if a == "--check-confined" {
            return check_confined();
        }
        let Some((w, h)) = a.strip_prefix("--at-least=").and_then(|s| s.split_once('x')) else {
            eprintln!("usage: vergence-decode [--at-least=WxH] < picture | --check-confined");
            return std::process::ExitCode::from(2);
        };
        match (w.parse::<u32>(), h.parse::<u32>()) {
            (Ok(w), Ok(h)) if w > 0 && h > 0 => at_least = Some((w.min(16384), h.min(16384))),
            _ => return std::process::ExitCode::from(2),
        }
    }
    let mut data = Vec::new();
    if let Err(e) = std::io::stdin().lock().take(MAX_FILE as u64 + 1).read_to_end(&mut data) {
        eprintln!("{e}");
        return std::process::ExitCode::from(1);
    }
    if data.len() > MAX_FILE {
        eprintln!("the file is larger than {} MB", MAX_FILE >> 20);
        return std::process::ExitCode::from(1);
    }
    limit_self();
    match decode_bytes(&data, at_least) {
        Ok(p) => {
            let mut out = std::io::stdout().lock();
            if out.write_all(&encode(&p)).and_then(|_| out.flush()).is_err() {
                return std::process::ExitCode::from(1);
            }
            std::process::ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("{why}");
            std::process::ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([x as u8, y as u8, 7, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn a_png_decodes_and_shrinks_to_at_least_the_box() {
        let p = decode_bytes(&png(40, 20), None).unwrap();
        assert_eq!((p.width, p.height, p.frames.len()), (40, 20, 1));
        assert_eq!(&p.frames[0].rgba[..4], &[0, 0, 7, 255]);
        let p = decode_bytes(&png(400, 200), Some((100, 100))).unwrap();
        assert_eq!((p.width, p.height), (200, 100), "at least 100 on both sides, shape kept");
        let p = decode_bytes(&png(40, 20), Some((100, 100))).unwrap();
        assert_eq!((p.width, p.height), (40, 20), "never enlarged");
    }

    #[test]
    fn an_svg_is_drawn_at_its_size_or_as_asked() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#ff0000"/></svg>"##;
        let p = decode_bytes(svg, None).unwrap();
        assert_eq!((p.width, p.height), (20, 10));
        assert_eq!(&p.frames[0].rgba[..4], &[255, 0, 0, 255]);
        let p = decode_bytes(svg, Some((200, 50))).unwrap();
        assert_eq!((p.width, p.height), (200, 100));
    }

    #[test]
    fn what_is_not_a_picture_is_refused() {
        assert!(decode_bytes(b"just some text", None).unwrap_err().contains("not a picture"));
        assert!(decode_bytes(b"\x89PNG\r\n\x1a\nbroken", None).is_err());
        assert!(decode_bytes(b"GIF89a\x01", None).is_err());
    }

    #[test]
    fn an_answer_round_trips_and_garbage_is_refused() {
        let p = Picture { width: 2, height: 1, frames: vec![Frame { delay_ms: 0, rgba: vec![1, 2, 3, 4, 5, 6, 7, 8] }, Frame { delay_ms: 40, rgba: vec![9; 8] }] };
        assert_eq!(parse(&encode(&p)), Some(p.clone()));
        let mut cut = encode(&p);
        cut.pop();
        assert_eq!(parse(&cut), None);
        assert_eq!(parse(b"VDEC1\xff\xff\xff\xff\xff\xff\xff\xff\x01\0\0\0"), None, "a size beyond the limit");
        assert_eq!(shrunk(7776, 5184, (560, 315)), (560, 373));
    }

    /// The helper itself, run as the program that asks does, when it is built
    /// (it is a bin of this crate: cargo builds it with the tests).
    #[test]
    fn the_helper_decodes_a_file_apart() {
        let Some(helper) = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.parent()?.join("vergence-decode"))).filter(|p| p.is_file()) else { return };
        let dir = std::env::temp_dir().join(format!("vergence-decode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("p.png");
        std::fs::write(&file, png(30, 10)).unwrap();
        let out = std::process::Command::new(&helper).arg("--at-least=6x2").stdin(std::fs::File::open(&file).unwrap()).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let p = parse(&out.stdout).unwrap();
        assert_eq!((p.width, p.height), (6, 2));
        std::fs::write(&file, b"not a picture").unwrap();
        let out = std::process::Command::new(&helper).stdin(std::fs::File::open(&file).unwrap()).output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn helper() -> Option<PathBuf> {
        std::env::current_exe().ok().and_then(|e| Some(e.parent()?.parent()?.join("vergence-decode"))).filter(|p| p.is_file())
    }

    /// A 2 x 2 BMP, 32 bits, bottom-up.
    fn bmp() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&(54u32 + 16).to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&54u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&32u16.to_le_bytes());
        b.extend_from_slice(&[0; 24]);
        for _ in 0..4 {
            b.extend_from_slice(&[0, 0, 255, 0]);
        }
        b
    }

    fn encoded(format: image::ImageFormat) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(16, 8, |x, y| image::Rgba([x as u8 * 9, y as u8 * 20, 7, 255]));
        let mut out = Vec::new();
        let pic = if format == image::ImageFormat::Jpeg { image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(img).to_rgb8()) } else { image::DynamicImage::ImageRgba8(img) };
        pic.write_to(&mut std::io::Cursor::new(&mut out), format).unwrap();
        out
    }

    fn animated_gif() -> Vec<u8> {
        use image::codecs::gif::GifEncoder;
        let mut out = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut out);
            for i in 0..3u8 {
                let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([i * 80, 0, 0, 255]));
                enc.encode_frame(image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(100, 1))).unwrap();
            }
        }
        out
    }

    /// Every format, through the real helper with its limits on (seccomp on
    /// Linux, Capsicum on FreeBSD): a system call the filter lacks would kill
    /// it here.
    #[test]
    fn the_confined_helper_decodes_every_format() {
        let Some(helper) = helper() else { return };
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#ff0000"/></svg>"##.to_vec();
        let cases: Vec<(&str, Vec<u8>, (u32, u32), usize)> = vec![
            ("png", encoded(image::ImageFormat::Png), (16, 8), 1),
            ("jpeg", encoded(image::ImageFormat::Jpeg), (16, 8), 1),
            ("webp", encoded(image::ImageFormat::WebP), (16, 8), 1),
            ("gif", animated_gif(), (4, 4), 3),
            ("bmp", bmp(), (2, 2), 1),
            ("svg", svg, (20, 10), 1),
        ];
        let dir = std::env::temp_dir().join(format!("vergence-decode-all-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, bytes, size, frames) in cases {
            let file = dir.join(name);
            std::fs::write(&file, &bytes).unwrap();
            let out = std::process::Command::new(&helper).stdin(std::fs::File::open(&file).unwrap()).output().unwrap();
            assert!(out.status.success(), "{name}: {:?} {}", out.status, String::from_utf8_lossy(&out.stderr));
            let p = parse(&out.stdout).unwrap();
            assert_eq!(((p.width, p.height), p.frames.len()), (size, frames), "{name}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Once limited, the helper cannot open a file: refused (exit 4) or
    /// killed by the filter, never allowed (exit 0).
    #[test]
    fn the_confined_helper_cannot_open_a_file() {
        let Some(helper) = helper() else { return };
        let out = std::process::Command::new(&helper).arg("--check-confined").output().unwrap();
        assert!(!out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(out.status.signal(), Some(31), "killed by seccomp (SIGSYS): {:?}", out.status);
        }
        #[cfg(target_os = "freebsd")]
        assert_eq!(out.status.code(), Some(4), "{}", String::from_utf8_lossy(&out.stderr));
    }
}
