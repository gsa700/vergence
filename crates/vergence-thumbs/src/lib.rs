//! Thumbnails (files milestone 1, step 10; docs/files-milestone-1.md §5) by
//! the freedesktop thumbnail spec, so they are shared with other desktops:
//! `$XDG_CACHE_HOME/thumbnails/large/<md5 of the file's URI>.png`, at most
//! 256 pixels a side, carrying `Thumb::URI` and `Thumb::MTime` (the file's
//! modification time in seconds); one whose time differs is stale and made
//! again. A picture that cannot be made into one is marked in
//! `fail/vergence/` so it is not tried again until it changes.
//!
//! Decoding stays apart (§3): the pixels of the picture and of a cached
//! thumbnail (which any program could have written) go through
//! vergence-decode's helper; this crate only walks a cached file's chunks
//! for its text, without inflating anything, and writes PNGs of pixels it
//! made. [`Thumbnailer`] keeps a queue for two worker threads, newest asks
//! first, so a folder of a thousand photos does not start a thousand
//! helpers.

mod md5;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// A file as it was when asked about: its path and modification time (a
/// file changed since is another key, so its thumbnail is made again).
type Key = (PathBuf, Option<SystemTime>);

use vergence_scene::Image;

/// The largest side of a thumbnail (the spec's "large").
pub const SIZE: u32 = 256;
/// Pictures larger than this are not made into thumbnails (the helper's
/// limits would stop them anyway; this spares it the try).
const MAX_FILE: u64 = 200 * 1024 * 1024;
/// The most read of a file in the cache itself (a 256 x 256 PNG is far less).
const MAX_CACHED: u64 = 16 * 1024 * 1024;

/// The file's URI as the spec (and GLib's g_filename_to_uri) writes it:
/// `file://` and the path, every byte but letters, digits and
/// `!$&'()*+,-./:=@_~` escaped as %XX.
pub fn uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"!$&'()*+,-./:=@_~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The thumbnail cache's top: `$XDG_CACHE_HOME/thumbnails`, else `~/.cache/thumbnails`.
pub fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("thumbnails"))
}

fn name_of(path: &Path) -> String {
    format!("{}.png", md5::hex(uri(path).as_bytes()))
}

/// Where a file's large thumbnail goes, and its failure mark.
fn places(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let dir = cache_dir()?;
    let name = name_of(path);
    Some((dir.join("large").join(&name), dir.join("fail/vergence").join(&name)))
}

fn mtime(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// A PNG's tEXt chunks before its pixels (only these: nothing inflated),
/// as (key, value). None if it is not a PNG.
fn text_chunks(bytes: &[u8]) -> Option<Vec<(String, String)>> {
    if bytes.len() < 8 || bytes[..8] != [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
        return None;
    }
    let mut out = Vec::new();
    let mut at = 8usize;
    while at + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
        let kind = &bytes[at + 4..at + 8];
        let body = bytes.get(at + 8..at + 8 + len)?;
        match kind {
            b"tEXt" => {
                if let Some(z) = body.iter().position(|b| *b == 0) {
                    let latin = |s: &[u8]| s.iter().map(|&b| b as char).collect::<String>();
                    out.push((latin(&body[..z]), latin(&body[z + 1..])));
                }
            }
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    Some(out)
}

/// Whether a cached thumbnail's text says it is of this file as it is now.
fn fresh(bytes: &[u8], uri: &str, mtime: u64) -> bool {
    let Some(text) = text_chunks(bytes) else { return false };
    let get = |k: &str| text.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    get("Thumb::URI") == Some(uri) && get("Thumb::MTime").and_then(|v| v.trim().parse::<u64>().ok()) == Some(mtime)
}

fn decoded(p: vergence_decode::Picture) -> Option<Image> {
    let f = p.frames.into_iter().next()?;
    (f.rgba.len() == p.width as usize * p.height as usize * 4).then_some(Image { width: p.width, height: p.height, rgba: f.rgba })
}

/// The file's thumbnail if the cache has a fresh one (never made here).
pub fn cached(path: &Path) -> Option<Image> {
    let (large, _) = places(path)?;
    let bytes = vergence_types::read_regular(&large, MAX_CACHED).ok()?;
    if !fresh(&bytes, &uri(path), mtime(path)?) {
        return None;
    }
    decoded(vergence_decode::decode_file(&large, None).ok()?)
}

/// The picture shrunk to fit SIZE (never grown), by averaging the pixels
/// each new one covers.
pub fn shrink(img: &Image, size: u32) -> Image {
    let (w, h) = (img.width.max(1), img.height.max(1));
    if w <= size && h <= size {
        return img.clone();
    }
    let k = (size as f64 / w as f64).min(size as f64 / h as f64);
    let (nw, nh) = (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1));
    let mut rgba = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        let (y0, y1) = ((y as u64 * h as u64 / nh as u64) as u32, (((y + 1) as u64 * h as u64).div_ceil(nh as u64) as u32).min(h));
        for x in 0..nw {
            let (x0, x1) = ((x as u64 * w as u64 / nw as u64) as u32, (((x + 1) as u64 * w as u64).div_ceil(nw as u64) as u32).min(w));
            // Colors weighted by their alpha, so clear pixels do not darken the edges.
            let mut sum = [0u64; 4];
            for sy in y0..y1.max(y0 + 1) {
                for sx in x0..x1.max(x0 + 1) {
                    let i = ((sy * w + sx) * 4) as usize;
                    let a = img.rgba[i + 3] as u64;
                    sum[0] += img.rgba[i] as u64 * a;
                    sum[1] += img.rgba[i + 1] as u64 * a;
                    sum[2] += img.rgba[i + 2] as u64 * a;
                    sum[3] += a;
                }
            }
            let n = ((y1.max(y0 + 1) - y0) * (x1.max(x0 + 1) - x0)) as u64;
            let o = ((y * nw + x) * 4) as usize;
            if sum[3] > 0 {
                for c in 0..3 {
                    rgba[o + c] = (sum[c] / sum[3]) as u8;
                }
            }
            rgba[o + 3] = (sum[3] / n) as u8;
        }
    }
    Image { width: nw, height: nh, rgba }
}

/// A PNG of these pixels carrying the spec's text.
fn encode(img: &Image, uri: &str, mtime: u64) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.add_text_chunk("Thumb::URI".into(), uri.into()).ok()?;
        enc.add_text_chunk("Thumb::MTime".into(), mtime.to_string()).ok()?;
        enc.add_text_chunk("Software".into(), "Vergence".into()).ok()?;
        let mut w = enc.write_header().ok()?;
        w.write_image_data(&img.rgba).ok()?;
    }
    Some(out)
}

/// Written owner-only (the spec: 0600, folders 0700), by a temporary file renamed over.
fn write_private(to: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(dir) = to.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let tmp = to.with_extension(format!("png.{}", std::process::id()));
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(bytes)?;
    }
    std::fs::rename(&tmp, to)
}

/// The file's thumbnail: the cache's if fresh, else made (decoded apart,
/// shrunk, written to the cache). None for a file that is not a picture
/// the helper can read (marked failed until it changes), or that is in the
/// thumbnail cache itself.
pub fn get(path: &Path) -> Option<Image> {
    let (large, fail) = places(path)?;
    if cache_dir().is_some_and(|c| path.starts_with(&c)) {
        return None;
    }
    let (link, time) = (uri(path), mtime(path)?);
    if let Ok(bytes) = vergence_types::read_regular(&large, MAX_CACHED)
        && fresh(&bytes, &link, time)
        && let Some(img) = vergence_decode::decode_file(&large, None).ok().and_then(decoded)
    {
        return Some(img);
    }
    if vergence_types::read_regular(&fail, MAX_CACHED).is_ok_and(|b| fresh(&b, &link, time)) {
        return None;
    }
    let made = std::fs::metadata(path).ok().filter(|m| m.is_file() && m.len() <= MAX_FILE).and_then(|_| vergence_decode::decode_file(path, Some((SIZE, SIZE))).ok()).and_then(decoded).map(|img| shrink(&img, SIZE));
    match made {
        Some(img) => {
            if let Some(bytes) = encode(&img, &link, time) {
                let _ = write_private(&large, &bytes);
            }
            Some(img)
        }
        None => {
            // The spec's failure mark: a 1x1 PNG with the same text.
            let dot = Image { width: 1, height: 1, rgba: vec![0; 4] };
            if let Some(bytes) = encode(&dot, &link, time) {
                let _ = write_private(&fail, &bytes);
            }
            None
        }
    }
}

struct Shared {
    queue: Mutex<(VecDeque<Key>, HashSet<Key>)>,
    ready: Condvar,
    done: Mutex<HashMap<Key, Option<Arc<Image>>>>,
}

/// Thumbnails made in the background: ask for a file's with [`want`],
/// look with [`get`]; `woken` is called (from a worker) as each is ready.
///
/// [`want`]: Thumbnailer::want
/// [`get`]: Thumbnailer::get
pub struct Thumbnailer {
    shared: Arc<Shared>,
}

/// Worker threads making thumbnails.
const WORKERS: usize = 2;

impl Thumbnailer {
    pub fn new(woken: impl Fn() + Send + Sync + 'static) -> Thumbnailer {
        let shared = Arc::new(Shared { queue: Mutex::new((VecDeque::new(), HashSet::new())), ready: Condvar::new(), done: Mutex::new(HashMap::new()) });
        let woken = Arc::new(woken);
        for _ in 0..WORKERS {
            let (s, woken) = (shared.clone(), woken.clone());
            std::thread::spawn(move || {
                loop {
                    let key = {
                        let mut q = s.queue.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if let Some(p) = q.0.pop_front() {
                                break p;
                            }
                            q = s.ready.wait(q).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    let img = get(&key.0).map(Arc::new);
                    s.queue.lock().unwrap_or_else(|e| e.into_inner()).1.remove(&key);
                    s.done.lock().unwrap_or_else(|e| e.into_inner()).insert(key, img);
                    woken();
                }
            });
        }
        Thumbnailer { shared }
    }

    /// The thumbnail of a file as it was at `changed`, if made: Some(None)
    /// when it has none.
    pub fn get(&self, path: &Path, changed: Option<SystemTime>) -> Option<Option<Arc<Image>>> {
        self.shared.done.lock().unwrap_or_else(|e| e.into_inner()).get(&(path.to_path_buf(), changed)).cloned()
    }

    /// Asks for a file's thumbnail as it was at `changed` (once each; in
    /// the order asked).
    pub fn want(&self, path: &Path, changed: Option<SystemTime>) {
        let key = (path.to_path_buf(), changed);
        if self.shared.done.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&key) {
            return;
        }
        let mut q = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        if q.1.insert(key.clone()) {
            q.0.push_back(key);
            self.shared.ready.notify_one();
        }
    }

    /// How many have been made (it changes as each is ready).
    pub fn made(&self) -> usize {
        self.shared.done.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Asks not yet begun are dropped (another folder is shown).
    pub fn forget_waiting(&self) {
        let mut q = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        let waiting: Vec<Key> = q.0.drain(..).collect();
        for p in waiting {
            q.1.remove(&p);
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_files_uri_escapes_as_glib_does() {
        assert_eq!(uri(Path::new("/home/u/My Pictures/a b+c(1).jpg")), "file:///home/u/My%20Pictures/a%20b+c(1).jpg");
        assert_eq!(uri(Path::new("/tmp/é#?.png")), "file:///tmp/%C3%A9%23%3F.png");
        // The spec's own example: the name is the MD5 of the URI.
        assert_eq!(md5::hex(b"file:///home/jens/photos/me.png"), "c6ee772d9e49320e97ec29a7eb5b1697");
    }

    #[test]
    fn a_thumbnails_text_says_whether_it_is_fresh() {
        let img = Image { width: 2, height: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 255] };
        let bytes = encode(&img, "file:///a.png", 42).unwrap();
        assert!(fresh(&bytes, "file:///a.png", 42));
        assert!(!fresh(&bytes, "file:///a.png", 43), "the file changed since");
        assert!(!fresh(&bytes, "file:///b.png", 42), "another file's");
        assert!(!fresh(b"not a png", "file:///a.png", 42));
        // Cut short: no panic, nothing found.
        assert!(!fresh(&bytes[..20], "file:///a.png", 42));
    }

    #[test]
    fn shrinking_fits_the_size_keeps_the_shape_and_averages() {
        let img = Image { width: 1000, height: 500, rgba: [200u8, 100, 50, 255].repeat(500_000) };
        let s = shrink(&img, 256);
        assert_eq!((s.width, s.height), (256, 128));
        assert_eq!(&s.rgba[..4], &[200, 100, 50, 255]);
        let small = Image { width: 10, height: 10, rgba: vec![9; 400] };
        assert_eq!(shrink(&small, 256).width, 10, "never grown");
    }

    #[test]
    fn made_once_then_read_from_the_cache_and_marked_when_not_a_picture() {
        let dir = std::env::temp_dir().join(format!("vthumbs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: tests in this crate do not read the environment on other threads.
        unsafe { std::env::set_var("XDG_CACHE_HOME", dir.join("cache")) };
        let pic = dir.join("p.png");
        let img = Image { width: 600, height: 300, rgba: [10u8, 20, 30, 255].repeat(180_000) };
        let bytes = encode(&img, "x", 0).unwrap();
        std::fs::write(&pic, bytes).unwrap();
        if vergence_decode::decode_file(&pic, None).is_err() {
            // No helper on this machine's PATH or beside the test: nothing more to check.
            return;
        }
        let t = get(&pic).expect("made");
        assert_eq!((t.width, t.height), (256, 128));
        let (large, fail) = places(&pic).unwrap();
        assert!(large.is_file());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&large).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(cached(&pic).is_some());
        let not = dir.join("n.png");
        std::fs::write(&not, b"no picture").unwrap();
        assert!(get(&not).is_none());
        assert!(places(&not).unwrap().1.is_file(), "marked failed");
        let _ = fail;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
