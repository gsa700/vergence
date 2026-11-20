//! PDFs in Close-Up (PDF milestone 1, step 6): the document's pages in a
//! column, scrolled continuously, drawn by vergence-pdf's confined helper
//! (never in this process) on a worker thread that owns it.
//!
//! The view asks for the pages in sight at the screen's pixels (the one in
//! the middle first, then the one below); until a page comes back at the
//! new size its last picture is drawn scaled. Far pages are forgotten.
//! Past [`MAX_PAGE_PIXELS`] a page is drawn at that many pixels and, over
//! it, the part in sight in [`TILE`]-pixel tiles at the screen's pixels.
//! Each document's page and zoom are remembered (`places`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use vergence_scene::{Color, Image, Rect, Scene};
use vergence_widgets::Waker;

/// Between pages, in the document's points (scaled with the zoom).
pub const GAP: f32 = 12.0;
/// Around the column, in the window's points.
pub const MARGIN: f32 = 16.0;
pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 16.0;
/// The most pixels one page is drawn with (tiles beyond).
pub const MAX_PAGE_PIXELS: f32 = (1 << 24) as f32;
/// A tile's side, in pixels.
pub const TILE: u32 = 512;
/// The most tiles in sight at once (more: the page picture alone).
const MAX_TILES: usize = 96;

/// Something for the worker to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Job {
    /// A whole page at a scale.
    Page(usize, f32),
    /// One tile of a page at a scale: its column and row.
    Tile(usize, f32, u32, u32),
}

/// A tile's key: page, scale in thousandths, column, row.
type TileKey = (usize, u32, u32, u32);

fn tile_key(page: usize, scale: f32, tx: u32, ty: u32) -> TileKey {
    (page, (scale * 1000.0).round() as u32, tx, ty)
}

/// How the zoom is chosen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// The widest page fills the window's width.
    Width,
    /// The current page fits whole.
    Page,
    /// Document points per window point.
    Zoom(f32),
}

/// What the view and the worker share.
pub struct Shared {
    /// What to draw, most wanted first.
    wanted: Mutex<Vec<Job>>,
    ready: Condvar,
    /// The latest picture of each page and the scale it was drawn at.
    done: Mutex<HashMap<usize, (f32, Arc<Image>)>>,
    tiles: Mutex<HashMap<TileKey, Arc<Image>>>,
    /// Pages the helper could not draw, and why.
    failed: Mutex<HashMap<usize, String>>,
    closed: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl Shared {
    fn new(waker: Option<Waker>) -> Shared {
        Shared { wanted: Mutex::new(Vec::new()), ready: Condvar::new(), done: Mutex::new(HashMap::new()), tiles: Mutex::new(HashMap::new()), failed: Mutex::new(HashMap::new()), closed: AtomicBool::new(false), waker: Mutex::new(waker) }
    }

    fn wake(&self) {
        if let Some(w) = self.waker.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            w.wake();
        }
    }

    pub fn failure(&self, page: usize) -> Option<String> {
        self.failed.lock().unwrap_or_else(|e| e.into_inner()).get(&page).cloned()
    }
}

/// The worker: draws what is wanted, newest wishes first, until the
/// document closes (the helper goes with it).
fn work(mut viewer: vergence_pdf::helper::Viewer, shared: Arc<Shared>) {
    loop {
        let job = {
            let mut w = shared.wanted.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if shared.closed.load(Ordering::Relaxed) {
                    return;
                }
                if !w.is_empty() {
                    break w.remove(0);
                }
                w = shared.ready.wait(w).unwrap_or_else(|e| e.into_inner());
            }
        };
        let (page, result) = match job {
            Job::Page(page, scale) => {
                let have = shared.done.lock().unwrap_or_else(|e| e.into_inner()).get(&page).map(|d| d.0);
                if have.is_some_and(|s| close_enough(s, scale)) {
                    continue;
                }
                (page, viewer.render(page, scale, None).map(|t| {
                    shared.done.lock().unwrap_or_else(|e| e.into_inner()).insert(page, (scale, Arc::new(Image { width: t.width, height: t.height, rgba: t.rgba })));
                }))
            }
            Job::Tile(page, scale, tx, ty) => {
                let key = tile_key(page, scale, tx, ty);
                if shared.tiles.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&key) {
                    continue;
                }
                (page, viewer.render(page, scale, Some((tx * TILE, ty * TILE, TILE, TILE))).map(|t| {
                    shared.tiles.lock().unwrap_or_else(|e| e.into_inner()).insert(key, Arc::new(Image { width: t.width, height: t.height, rgba: t.rgba }));
                }))
            }
        };
        match result {
            Ok(()) => {
                shared.failed.lock().unwrap_or_else(|e| e.into_inner()).remove(&page);
            }
            Err(e) => {
                shared.failed.lock().unwrap_or_else(|e| e.into_inner()).insert(page, e.to_string());
            }
        }
        shared.wake();
    }
}

fn close_enough(a: f32, b: f32) -> bool {
    (a - b).abs() <= b * 0.02
}

/// Where an open PDF is shown: cheap to copy into a drawing.
#[derive(Clone)]
pub struct Layout {
    /// Each page's size as shown, in points.
    pub pages: Arc<Vec<(f32, f32)>>,
    pub shared: Arc<Shared>,
    pub mode: Mode,
    /// The document's y (points, from the first page's top) at the top of the view.
    pub top: f32,
    /// Sideways, in window points, when the pages are wider than the window.
    pub pan_x: f32,
    /// A page to go to once the view has a place (a remembered one).
    pub restore: Option<usize>,
}

/// An open PDF: its layout, and its worker and helper until dropped.
pub struct Pdf(pub Layout);

impl std::ops::Deref for Pdf {
    type Target = Layout;
    fn deref(&self) -> &Layout {
        &self.0
    }
}

impl std::ops::DerefMut for Pdf {
    fn deref_mut(&mut self) -> &mut Layout {
        &mut self.0
    }
}

impl Drop for Pdf {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Relaxed);
        self.shared.ready.notify_all();
    }
}

/// A document opened in its helper: ready, or waiting for its password.
pub enum Opened {
    Ready(Pdf),
    Locked(vergence_pdf::helper::Viewer),
}

/// Opens a document in its helper and, unless it needs a password, starts
/// its worker. Err: why not.
pub fn open(path: &std::path::Path, waker: Option<Waker>) -> Result<Opened, String> {
    let viewer = vergence_pdf::helper::Viewer::open(path).map_err(|e| e.to_string())?;
    if viewer.info.needs_password {
        return Ok(Opened::Locked(viewer));
    }
    start(viewer, waker).map(Opened::Ready)
}

/// A worker for an open (and unlocked) document.
pub fn start(viewer: vergence_pdf::helper::Viewer, waker: Option<Waker>) -> Result<Pdf, String> {
    if viewer.info.pages.is_empty() {
        return Err("This PDF has no pages.".into());
    }
    let pages = Arc::new(viewer.info.pages.iter().map(|&(w, h)| (w.max(1.0), h.max(1.0))).collect());
    let shared = Arc::new(Shared::new(waker));
    let s = shared.clone();
    std::thread::spawn(move || work(viewer, s));
    Ok(Pdf(Layout { pages, shared, mode: Mode::Width, top: 0.0, pan_x: 0.0, restore: None }))
}

impl Layout {
    /// For tests: pages with no helper behind them.
    #[cfg(test)]
    fn sized(pages: Vec<(f32, f32)>) -> Layout {
        Layout { pages: Arc::new(pages), shared: Arc::new(Shared::new(None)), mode: Mode::Width, top: 0.0, pan_x: 0.0, restore: None }
    }

    /// Each page's top in the document, in points.
    pub fn tops(&self) -> Vec<f32> {
        let mut y = 0.0;
        self.pages
            .iter()
            .map(|p| {
                let t = y;
                y += p.1 + GAP;
                t
            })
            .collect()
    }

    pub fn total(&self) -> f32 {
        self.pages.iter().map(|p| p.1 + GAP).sum::<f32>() - GAP
    }

    fn widest(&self) -> f32 {
        self.pages.iter().map(|p| p.0).fold(1.0, f32::max)
    }

    /// Document points per window point, in a place `r`.
    pub fn zoom(&self, r: Rect) -> f32 {
        let room = ((r.w - 2.0 * MARGIN).max(40.0), (r.h - 2.0 * MARGIN).max(40.0));
        match self.mode {
            Mode::Width => room.0 / self.widest(),
            Mode::Page => {
                let p = self.pages[self.current(r).min(self.pages.len() - 1)];
                (room.0 / p.0).min(room.1 / p.1)
            }
            Mode::Zoom(z) => z,
        }
        .clamp(MIN_ZOOM, MAX_ZOOM)
    }

    /// The page at the middle of the view (by the zoom of Width or Zoom:
    /// Page's zoom depends on it).
    pub fn current(&self, r: Rect) -> usize {
        let z = match self.mode {
            Mode::Zoom(z) => z,
            _ => ((r.w - 2.0 * MARGIN).max(40.0) / self.widest()).clamp(MIN_ZOOM, MAX_ZOOM),
        };
        let mid = self.top + (r.h / 2.0 - MARGIN) / z;
        let tops = self.tops();
        tops.iter().rposition(|&t| t <= mid + GAP / 2.0).unwrap_or(0)
    }

    /// The view kept on the document.
    pub fn clamp(&mut self, r: Rect) {
        let z = self.zoom(r);
        let view = (r.h - 2.0 * MARGIN) / z;
        let max = (self.total() - view).max(0.0);
        self.top = if self.total() <= view { (self.total() - view) / 2.0 } else { self.top.clamp(0.0, max) };
        let wide = self.widest() * z + 2.0 * MARGIN;
        let room = (wide - r.w).max(0.0) / 2.0;
        self.pan_x = self.pan_x.clamp(-room, room);
    }

    /// Page `p` at the top of the view (centered when it fits whole).
    pub fn go_to(&mut self, p: usize, r: Rect) {
        let p = p.min(self.pages.len() - 1);
        let top = self.tops()[p];
        // Page mode's zoom is the page's own: worked out with it current.
        self.top = top;
        let z = self.zoom(r);
        let view = (r.h - 2.0 * MARGIN) / z;
        let h = self.pages[p].1;
        self.top = if h <= view { top - (view - h) / 2.0 } else { top };
        self.clamp(r);
    }

    pub fn scroll(&mut self, dy: f32, dx: f32, r: Rect) {
        let z = self.zoom(r);
        self.top += dy / z;
        self.pan_x -= dx;
        self.clamp(r);
    }

    /// A new zoom keeping the document point under `at` (window y) where it is.
    pub fn set_mode(&mut self, mode: Mode, at: Option<f32>, r: Rect) {
        let before = self.zoom(r);
        let ay = at.unwrap_or(r.y + r.h / 2.0) - r.y - MARGIN;
        let doc_y = self.top + ay / before;
        let current = self.current(r);
        self.mode = match mode {
            Mode::Zoom(z) => Mode::Zoom(z.clamp(MIN_ZOOM, MAX_ZOOM)),
            m => m,
        };
        if self.mode == Mode::Page {
            self.go_to(current, r);
            return;
        }
        let after = self.zoom(r);
        self.top = doc_y - ay / after;
        self.pan_x *= after / before;
        self.clamp(r);
    }

    /// Whether the current page fits whole: then Left and Right go a page.
    pub fn page_fits(&self, r: Rect) -> bool {
        let z = self.zoom(r);
        let p = self.pages[self.current(r)];
        p.1 * z <= r.h - MARGIN && p.0 * z <= r.w
    }

    /// The pages in sight, `[first, last]`.
    pub fn in_sight(&self, r: Rect) -> (usize, usize) {
        let z = self.zoom(r);
        let (a, b) = (self.top - MARGIN / z, self.top + (r.h - MARGIN) / z);
        let tops = self.tops();
        let first = tops.iter().zip(self.pages.iter()).position(|(t, p)| t + p.1 >= a).unwrap_or(0);
        let last = tops.iter().rposition(|&t| t <= b).unwrap_or(first).max(first);
        (first, last)
    }

    /// Draws the pages in `r` and asks the worker for what is missing.
    #[allow(clippy::needless_range_loop)]
    pub fn draw(&self, scene: &mut Scene, r: Rect, scale: f32, ground: Color) {
        scene.push_clip(r, 0.0);
        scene.fill(r, ground);
        let z = self.zoom(r);
        let (first, last) = self.in_sight(r);
        let tops = self.tops();
        let done = self.shared.done.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let failed: Vec<usize> = self.shared.failed.lock().unwrap_or_else(|e| e.into_inner()).keys().copied().collect();
        let tiles = self.shared.tiles.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut wanted = Vec::new();
        let mut tile_jobs = Vec::new();
        let mut keep_tiles: Vec<TileKey> = Vec::new();
        let middle = self.current(r);
        for p in first..=last {
            let (pw, ph) = self.pages[p];
            let dest = Rect::new(r.x + (r.w - pw * z) / 2.0 + self.pan_x, r.y + MARGIN + (tops[p] - self.top) * z, pw * z, ph * z);
            scene.fill(Rect::new(dest.x + 1.0, dest.y + 2.0, dest.w, dest.h), Color::rgba(0, 0, 0, 0x50));
            scene.fill(dest, Color::rgb(0xff, 0xff, 0xff));
            if let Some((_, img)) = done.get(&p) {
                let id = scene.image(img.clone());
                scene.draw_image(id, dest, 1.0);
            }
            if failed.contains(&p) {
                scene.fill(dest, Color::rgba(0xd0, 0x30, 0x30, 0x30));
            }
            let s = page_scale(pw, ph, z * scale);
            if !done.get(&p).is_some_and(|d| close_enough(d.0, s)) && !failed.contains(&p) {
                wanted.push(Job::Page(p, s));
            }
            // Deeper than a page picture goes: the part in sight in tiles.
            let deep = round3(z * scale);
            if deep > s * 1.02 && !failed.contains(&p) {
                let k = deep / z; // pixels per window point
                let vis = (dest.x.max(r.x), dest.y.max(r.y), (dest.x + dest.w).min(r.x + r.w), (dest.y + dest.h).min(r.y + r.h));
                if vis.2 > vis.0 && vis.3 > vis.1 {
                    let t = TILE as f32;
                    let (c0, r0) = (((vis.0 - dest.x) * k / t).floor() as u32, ((vis.1 - dest.y) * k / t).floor() as u32);
                    let (c1, r1) = (((vis.2 - dest.x) * k / t).ceil() as u32, ((vis.3 - dest.y) * k / t).ceil() as u32);
                    if ((c1 - c0) * (r1 - r0)) as usize <= MAX_TILES {
                        for ty in r0..r1 {
                            for tx in c0..c1 {
                                let key = tile_key(p, deep, tx, ty);
                                match tiles.get(&key) {
                                    Some(img) => {
                                        let id = scene.image(img.clone());
                                        let at = Rect::new(dest.x + tx as f32 * t / k, dest.y + ty as f32 * t / k, img.width as f32 / k, img.height as f32 / k);
                                        scene.draw_image(id, at, 1.0);
                                    }
                                    None => tile_jobs.push(Job::Tile(p, deep, tx, ty)),
                                }
                                keep_tiles.push(key);
                            }
                        }
                    }
                }
            }
        }
        // The page below, ahead of being scrolled to.
        if last + 1 < self.pages.len() && !done.contains_key(&(last + 1)) {
            let (pw, ph) = self.pages[last + 1];
            wanted.push(Job::Page(last + 1, page_scale(pw, ph, z * scale)));
        }
        let page_of = |j: &Job| match j {
            Job::Page(p, _) | Job::Tile(p, ..) => *p,
        };
        wanted.sort_by_key(|j| (page_of(j) as isize - middle as isize).unsigned_abs());
        // Page pictures first (they cover the page soonest), then its tiles.
        tile_jobs.sort_by_key(|j| (page_of(j) as isize - middle as isize).unsigned_abs());
        wanted.extend(tile_jobs);
        {
            let mut w = self.shared.wanted.lock().unwrap_or_else(|e| e.into_inner());
            if *w != wanted {
                *w = wanted;
                self.shared.ready.notify_all();
            }
        }
        // Pages far from sight are forgotten.
        let keep = first.saturating_sub(3)..=last + 3;
        self.shared.done.lock().unwrap_or_else(|e| e.into_inner()).retain(|p, _| keep.contains(p));
        // Tiles: only those of this zoom in sight.
        if tiles.len() > keep_tiles.len() {
            self.shared.tiles.lock().unwrap_or_else(|e| e.into_inner()).retain(|k, _| keep_tiles.contains(k));
        }
        scene.pop_clip();
    }
}

/// Pixels per point for a page drawn at `want`, within [`MAX_PAGE_PIXELS`]
/// and rounded so small changes do not redraw it.
fn page_scale(w: f32, h: f32, want: f32) -> f32 {
    let most = (MAX_PAGE_PIXELS / (w * h)).sqrt();
    round3(want.min(most).max(0.05))
}

/// Three significant digits.
fn round3(s: f32) -> f32 {
    let s = s.max(0.01);
    let k = 10f32.powf(2.0 - s.log10().floor());
    (s * k).round() / k
}

/// Each document's page and zoom: ~/.local/state/vergence/closeup-pdf-places,
/// owner-only, a line each (page, mode, path), newest first, at most 500.
pub mod places {
    use super::Mode;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    const MOST: usize = 500;

    fn file() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
        Some(base.join("vergence/closeup-pdf-places"))
    }

    fn mode_text(m: Mode) -> String {
        match m {
            Mode::Width => "width".into(),
            Mode::Page => "page".into(),
            Mode::Zoom(z) => format!("{z:.3}"),
        }
    }

    fn parse_mode(s: &str) -> Option<Mode> {
        match s {
            "width" => Some(Mode::Width),
            "page" => Some(Mode::Page),
            z => z.parse::<f32>().ok().filter(|z| z.is_finite()).map(|z| Mode::Zoom(z.clamp(super::MIN_ZOOM, super::MAX_ZOOM))),
        }
    }

    fn parse(text: &str) -> Vec<(usize, Mode, String)> {
        text.lines()
            .filter_map(|l| {
                let mut f = l.splitn(3, '\t');
                Some((f.next()?.parse().ok()?, parse_mode(f.next()?)?, f.next()?.to_string()))
            })
            .collect()
    }

    pub fn load(path: &Path) -> Option<(usize, Mode)> {
        let text = std::fs::read_to_string(file()?).ok()?;
        let key = path.to_string_lossy();
        parse(&text).into_iter().find(|e| e.2 == key).map(|e| (e.0, e.1))
    }

    pub fn save(path: &Path, page: usize, mode: Mode) {
        let Some(f) = file() else { return };
        let key = path.to_string_lossy().replace(['\n', '\t'], " ");
        let mut all = parse(&std::fs::read_to_string(&f).unwrap_or_default());
        all.retain(|e| e.2 != key);
        all.insert(0, (page, mode, key));
        all.truncate(MOST);
        let text: String = all.iter().map(|(p, m, k)| format!("{p}\t{}\t{k}\n", mode_text(*m))).collect();
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = f.with_extension("new");
        #[cfg(unix)]
        let opened = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)
        };
        #[cfg(not(unix))]
        let opened = std::fs::File::create(&tmp);
        if let Ok(mut out) = opened
            && out.write_all(text.as_bytes()).is_ok()
        {
            let _ = std::fs::rename(&tmp, &f);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn lines_round_trip() {
            let text = format!("12\t{}\t/a/b c.pdf\n3\t{}\t/x.pdf\nbad line\n", mode_text(Mode::Page), mode_text(Mode::Zoom(1.5)));
            let all = parse(&text);
            assert_eq!(all, vec![(12, Mode::Page, "/a/b c.pdf".to_string()), (3, Mode::Zoom(1.5), "/x.pdf".to_string())]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LETTER: (f32, f32) = (612.0, 792.0);

    fn place() -> Rect {
        Rect::new(0.0, 0.0, 644.0, 500.0)
    }

    #[test]
    fn fit_width_and_whole_page() {
        let mut d = Layout::sized(vec![LETTER; 5]);
        let r = place();
        // 644 - 2 x 16 = 612: one point a point.
        assert!((d.zoom(r) - 1.0).abs() < 1e-5);
        d.set_mode(Mode::Page, None, r);
        assert!((d.zoom(r) - 468.0 / 792.0).abs() < 1e-5);
        assert!(d.page_fits(r));
    }

    #[test]
    fn scrolling_stays_on_the_document() {
        let mut d = Layout::sized(vec![LETTER; 3]);
        let r = place();
        d.scroll(-100.0, 0.0, r);
        assert_eq!(d.top, 0.0);
        d.scroll(1e6, 0.0, r);
        let view = (r.h - 2.0 * MARGIN) / d.zoom(r);
        assert!((d.top - (d.total() - view)).abs() < 1e-3);
        assert_eq!(d.in_sight(r).1, 2);
    }

    #[test]
    fn pages_in_sight_and_going_to_one() {
        let mut d = Layout::sized(vec![LETTER; 10]);
        let r = place();
        assert_eq!(d.in_sight(r), (0, 0));
        d.go_to(4, r);
        assert_eq!(d.current(r), 4);
        // Page 3's last points show in the margin above the gap.
        assert_eq!(d.in_sight(r), (3, 4));
        d.top -= 100.0;
        assert_eq!(d.in_sight(r), (3, 4));
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let mut d = Layout::sized(vec![LETTER; 10]);
        let r = place();
        d.go_to(3, r);
        let (at, z0) = (300.0, d.zoom(r));
        let doc_y = d.top + (at - MARGIN) / z0;
        d.set_mode(Mode::Zoom(2.0), Some(at), r);
        assert!((d.top + (at - MARGIN) / 2.0 - doc_y).abs() < 1e-3);
    }

    #[test]
    fn tiles_are_keyed_by_page_scale_and_place() {
        assert_eq!(tile_key(3, 6.0, 1, 2), (3, 6000, 1, 2));
        assert_eq!(round3(6.0012), 6.0);
        assert_eq!(round3(12.345), 12.3);
    }

    #[test]
    fn page_scales_are_bounded_and_rounded() {
        assert_eq!(page_scale(612.0, 792.0, 2.0), 2.0);
        assert_eq!(page_scale(612.0, 792.0, 1.23456), 1.23);
        let s = page_scale(612.0, 792.0, 40.0);
        assert!(612.0 * 792.0 * s * s <= MAX_PAGE_PIXELS * 1.01);
    }
}
