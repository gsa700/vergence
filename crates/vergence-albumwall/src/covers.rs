//! Covers for the wall (the Avalonia app's ArtCache; docs/albumwall-milestone-1.md,
//! step 3): decoded once per size bucket and kept in memory, the least
//! recently used let go past a budget; decoded on worker threads, the
//! newest request first (what is on screen now, not what scrolled past).
//!
//! - **Buckets**: 256, 384 or 512 device pixels, the smallest at least the
//!   tile's size, so a resize does not decode every cover again and a
//!   cover is never drawn from far fewer pixels than it is shown at.
//! - **Keys**: the picture's source (a sidecar file, or the track it is
//!   embedded in), that file's size and modified time, and the bucket: a
//!   sleeve corrected in place at the same path is decoded again.
//! - **Budget**: 160 MB of pixels.
//! - Every picture is decoded in a confined helper: a sidecar by
//!   vergence-decode's, an embedded cover by Hymie's (helper::Scanner::cover).
//! - Each decoded cover is also measured for its colors (colors.rs): its
//!   palette and its share of the library's hue, from a 16-pixel sample.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

use vergence_scene::Image;

use crate::colors::{self, Analysis, HueSum, Sample};
use crate::library::Art;

/// The sizes covers are decoded at, in device pixels.
pub const BUCKETS: [u32; 3] = [256, 384, 512];
/// The most memory decoded covers take.
pub const BUDGET: usize = 160 << 20;

/// The bucket for a tile `px` device pixels wide: the smallest at least
/// that, else the largest.
pub fn bucket(px: f32) -> u32 {
    BUCKETS.into_iter().find(|b| *b as f32 >= px).unwrap_or(BUCKETS[BUCKETS.len() - 1])
}

/// Which picture, at which size.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub path: PathBuf,
    pub embedded: bool,
    pub size: u64,
    pub modified: i64,
    pub bucket: u32,
}

impl Key {
    pub fn of(art: &Art, bucket: u32) -> Key {
        Key { path: art.path.clone(), embedded: art.embedded, size: art.size, modified: art.modified, bucket }
    }
}

/// A decoded cover and what its colors are.
#[derive(Debug, Clone)]
pub struct Cover {
    pub image: Arc<Image>,
    pub colors: Analysis,
    pub hue: HueSum,
}

/// The least recently used covers let go past a budget of bytes.
#[derive(Default)]
pub struct Lru {
    map: HashMap<Key, (Cover, u64)>,
    bytes: usize,
    clock: u64,
    budget: usize,
}

impl Lru {
    pub fn new(budget: usize) -> Lru {
        Lru { budget, ..Lru::default() }
    }

    pub fn get(&mut self, k: &Key) -> Option<Cover> {
        self.clock += 1;
        let clock = self.clock;
        self.map.get_mut(k).map(|(c, used)| {
            *used = clock;
            c.clone()
        })
    }

    pub fn insert(&mut self, k: Key, c: Cover) {
        self.clock += 1;
        self.bytes += c.image.rgba.len();
        if let Some((old, _)) = self.map.insert(k, (c, self.clock)) {
            self.bytes -= old.image.rgba.len();
        }
        while self.bytes > self.budget && self.map.len() > 1 {
            let Some(oldest) = self.map.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone()) else {
                break;
            };
            if let Some((c, _)) = self.map.remove(&oldest) {
                self.bytes -= c.image.rgba.len();
            }
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Requests waiting: the newest first; asking again for one waiting moves
/// it to the front; at most `most` wait (the oldest are let go: they have
/// scrolled away).
#[derive(Default)]
pub struct Queue {
    items: VecDeque<Key>,
    most: usize,
}

impl Queue {
    pub fn new(most: usize) -> Queue {
        Queue { items: VecDeque::new(), most }
    }

    pub fn push(&mut self, k: Key) {
        if let Some(i) = self.items.iter().position(|q| *q == k) {
            self.items.remove(i);
        }
        self.items.push_front(k);
        self.items.truncate(self.most);
    }

    pub fn pop(&mut self) -> Option<Key> {
        self.items.pop_front()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// A cover's 16-pixel-wide sample, by box-averaging the decoded picture.
fn sample16(w: u32, h: u32, rgba: &[u8]) -> (usize, usize, Vec<u8>) {
    let sw = 16usize.min(w as usize).max(1);
    let sh = ((h as usize * sw) / (w as usize).max(1)).clamp(1, 64);
    let mut out = vec![0u8; sw * sh * 4];
    for sy in 0..sh {
        for sx in 0..sw {
            let (x0, x1) = (sx * w as usize / sw, ((sx + 1) * w as usize / sw).max(sx * w as usize / sw + 1));
            let (y0, y1) = (sy * h as usize / sh, ((sy + 1) * h as usize / sh).max(sy * h as usize / sh + 1));
            let mut sum = [0u64; 4];
            let mut n = 0u64;
            for y in y0..y1.min(h as usize) {
                for x in x0..x1.min(w as usize) {
                    let p = &rgba[(y * w as usize + x) * 4..][..4];
                    for c in 0..4 {
                        sum[c] += p[c] as u64;
                    }
                    n += 1;
                }
            }
            let o = &mut out[(sy * sw + sx) * 4..][..4];
            for c in 0..4 {
                o[c] = (sum[c] / n.max(1)) as u8;
            }
        }
    }
    (sw, sh, out)
}

/// A decoded picture made a cover: kept as it is, its colors measured.
pub fn cover_of(w: u32, h: u32, rgba: Vec<u8>) -> Cover {
    let (sw, sh, small) = sample16(w, h, &rgba);
    let sample = Sample { width: sw, height: sh, rgba: &small };
    Cover {
        colors: colors::analyze(&sample),
        hue: colors::hue_sum(&sample),
        image: Arc::new(Image { width: w, height: h, rgba }),
    }
}

/// Decodes the picture `key` names, in a confined helper.
pub type Decode = dyn Fn(&Key) -> Option<(u32, u32, Vec<u8>)> + Send + Sync;

/// The covers: the cache, the requests, the workers.
pub struct Covers {
    shared: Arc<Shared>,
}

struct Shared {
    lru: Mutex<Lru>,
    queue: Mutex<Queue>,
    /// What could not be decoded (not asked again).
    failed: Mutex<std::collections::HashSet<Key>>,
    ready: Condvar,
    wake: Box<dyn Fn() + Send + Sync>,
}

impl Covers {
    /// The covers held decoded, and their bytes (the status line's "covers
    /// held · MB art", as the Avalonia app's ArtCache told it).
    pub fn held(&self) -> (usize, usize) {
        let l = self.shared.lru.lock().unwrap_or_else(|e| e.into_inner());
        (l.len(), l.bytes())
    }

    /// `workers` threads decoding with `decode`; `wake` told when a cover is ready.
    pub fn new(workers: usize, decode: Arc<Decode>, wake: impl Fn() + Send + Sync + 'static) -> Covers {
        let shared = Arc::new(Shared {
            lru: Mutex::new(Lru::new(BUDGET)),
            queue: Mutex::new(Queue::new(512)),
            failed: Mutex::default(),
            ready: Condvar::new(),
            wake: Box::new(wake),
        });
        for i in 0..workers {
            let (s, decode) = (shared.clone(), decode.clone());
            let _ = std::thread::Builder::new().name(format!("covers-{i}")).spawn(move || {
                loop {
                    let key = {
                        let mut q = s.queue.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if let Some(k) = q.pop() {
                                break k;
                            }
                            // The program gone: nothing will ask again.
                            if Arc::strong_count(&s) == 1 {
                                return;
                            }
                            q = s
                                .ready
                                .wait_timeout(q, std::time::Duration::from_secs(1))
                                .unwrap_or_else(|e| e.into_inner())
                                .0;
                        }
                    };
                    if s.lru.lock().unwrap_or_else(|e| e.into_inner()).get(&key).is_some() {
                        continue;
                    }
                    match decode(&key) {
                        Some((w, h, rgba)) if rgba.len() == (w * h * 4) as usize => {
                            let c = cover_of(w, h, rgba);
                            s.lru.lock().unwrap_or_else(|e| e.into_inner()).insert(key, c);
                        }
                        _ => {
                            s.failed.lock().unwrap_or_else(|e| e.into_inner()).insert(key);
                        }
                    }
                    (s.wake)();
                }
            });
        }
        Covers { shared }
    }

    /// The cover for `key` if it is decoded; else asked for (the newest
    /// request first) and None for now.
    pub fn get(&self, key: &Key) -> Option<Cover> {
        if let Some(c) = self.shared.lru.lock().unwrap_or_else(|e| e.into_inner()).get(key) {
            return Some(c);
        }
        if !self.shared.failed.lock().unwrap_or_else(|e| e.into_inner()).contains(key) {
            self.shared.queue.lock().unwrap_or_else(|e| e.into_inner()).push(key.clone());
            self.shared.ready.notify_one();
        }
        None
    }

    /// Whether `key` could not be decoded (its tile shows its words instead).
    pub fn failed(&self, key: &Key) -> bool {
        self.shared.failed.lock().unwrap_or_else(|e| e.into_inner()).contains(key)
    }
}

/// The decoder of the installed helpers: a sidecar by vergence-decode, an
/// embedded cover by Hymie (one helper per worker, kept between covers).
pub fn helpers() -> Arc<Decode> {
    let scanners: Mutex<Vec<vergence_hymie::helper::Scanner>> = Mutex::new(Vec::new());
    Arc::new(move |k: &Key| {
        if k.embedded {
            let mut s = scanners
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop()
                .or_else(|| vergence_hymie::helper::Scanner::new().ok())?;
            let r = s.cover(&k.path, k.bucket).ok().flatten();
            scanners.lock().unwrap_or_else(|e| e.into_inner()).push(s);
            r
        } else {
            let p = vergence_decode::decode_file(&k.path, Some((k.bucket, k.bucket))).ok()?;
            let f = p.frames.into_iter().next()?;
            Some((p.width, p.height, f.rgba))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u32, bucket: u32) -> Key {
        Key { path: format!("/m/{n}.flac").into(), embedded: true, size: 1, modified: 1, bucket }
    }

    fn solid(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
        c.iter().copied().cycle().take((w * h * 4) as usize).collect()
    }

    #[test]
    fn buckets_round_up() {
        assert_eq!(bucket(100.0), 256);
        assert_eq!(bucket(256.0), 256);
        assert_eq!(bucket(300.0), 384);
        assert_eq!(bucket(800.0), 512);
    }

    #[test]
    fn the_least_recently_used_go_past_the_budget() {
        let one = cover_of(16, 16, solid(16, 16, [10, 20, 30, 255]));
        let mut lru = Lru::new(3 * 16 * 16 * 4);
        lru.insert(key(1, 256), one.clone());
        lru.insert(key(2, 256), one.clone());
        lru.insert(key(3, 256), one.clone());
        assert!(lru.get(&key(1, 256)).is_some());
        lru.insert(key(4, 256), one);
        assert_eq!(lru.len(), 3);
        assert!(lru.get(&key(2, 256)).is_none(), "the least recently used went");
        assert!(lru.get(&key(1, 256)).is_some());
        assert_eq!(lru.bytes(), 3 * 16 * 16 * 4);
    }

    #[test]
    fn the_newest_request_is_first() {
        let mut q = Queue::new(3);
        for n in 1..=4 {
            q.push(key(n, 256));
        }
        q.push(key(3, 256));
        let order: Vec<Key> = std::iter::from_fn(|| q.pop()).collect();
        assert_eq!(order, [key(3, 256), key(4, 256), key(2, 256)]);
    }

    #[test]
    fn a_cover_is_measured_for_its_colors() {
        let red = cover_of(64, 64, solid(64, 64, [200, 30, 30, 255]));
        assert!(red.colors.hue < 5.0 || red.colors.hue > 355.0);
        assert!(red.colors.saturation > 0.5);
        assert!(red.hue.hue().is_some());
    }

    #[test]
    fn covers_are_decoded_on_workers_and_kept() {
        let decoded = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let d = decoded.clone();
        let decode: Arc<Decode> = Arc::new(move |k: &Key| {
            d.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            (!k.path.ends_with("bad.flac")).then(|| (8, 8, solid(8, 8, [0, 0, 255, 255])))
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        let covers = Covers::new(2, decode, move || {
            let _ = tx.lock().unwrap().send(());
        });
        let k = key(7, 256);
        assert!(covers.get(&k).is_none());
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(covers.get(&k).unwrap().image.width, 8);
        let bad = Key { path: "/m/bad.flac".into(), ..k };
        assert!(covers.get(&bad).is_none());
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(covers.failed(&bad));
        assert!(covers.get(&bad).is_none());
        assert_eq!(decoded.load(std::sync::atomic::Ordering::Relaxed), 2, "a failure is not asked again");
    }
}
