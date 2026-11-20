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

use vergence_pdf::outline::{Action, Entry, Link, Target};
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
    /// A page's thumbnail for the sidebar, at a scale.
    Thumb(usize, f32),
    /// A page's links.
    Links(usize),
}

/// The sidebar's thumbnails: the widest box, in points, and the gap.
pub const THUMB_W: f32 = 132.0;
pub const THUMB_GAP: f32 = 16.0;

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
    /// The sidebar's wishes (thumbnails), after the view's.
    side: Mutex<Vec<Job>>,
    thumbs: Mutex<HashMap<usize, Arc<Image>>>,
    links: Mutex<HashMap<usize, Vec<Link>>>,
    /// The outline, once the worker has read it.
    pub outline: Mutex<Option<Arc<Vec<Entry>>>>,
    /// Pages the helper could not draw, and why.
    failed: Mutex<HashMap<usize, String>>,
    closed: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl Shared {
    fn new(waker: Option<Waker>) -> Shared {
        Shared { wanted: Mutex::new(Vec::new()), ready: Condvar::new(), done: Mutex::new(HashMap::new()), tiles: Mutex::new(HashMap::new()), side: Mutex::new(Vec::new()), thumbs: Mutex::new(HashMap::new()), links: Mutex::new(HashMap::new()), outline: Mutex::new(None), failed: Mutex::new(HashMap::new()), closed: AtomicBool::new(false), waker: Mutex::new(waker) }
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
    let outline = viewer.outline().unwrap_or_default();
    *shared.outline.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(outline));
    shared.wake();
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
                let mut side = shared.side.lock().unwrap_or_else(|e| e.into_inner());
                if !side.is_empty() {
                    break side.remove(0);
                }
                drop(side);
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
            Job::Thumb(page, scale) => {
                if shared.thumbs.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&page) {
                    continue;
                }
                (page, viewer.render(page, scale, None).map(|t| {
                    shared.thumbs.lock().unwrap_or_else(|e| e.into_inner()).insert(page, Arc::new(Image { width: t.width, height: t.height, rgba: t.rgba }));
                }))
            }
            Job::Links(page) => {
                let l = viewer.links(page).unwrap_or_default();
                shared.links.lock().unwrap_or_else(|e| e.into_inner()).insert(page, l);
                continue;
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
    /// The sidebar's thumbnails scrolled this far, in points.
    pub thumb_top: f32,
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
    Ok(Pdf(Layout { pages, shared, mode: Mode::Width, top: 0.0, pan_x: 0.0, restore: None, thumb_top: 0.0 }))
}

impl Layout {
    /// For tests: pages with no helper behind them.
    #[cfg(test)]
    fn sized(pages: Vec<(f32, f32)>) -> Layout {
        Layout { pages: Arc::new(pages), shared: Arc::new(Shared::new(None)), mode: Mode::Width, top: 0.0, pan_x: 0.0, restore: None, thumb_top: 0.0 }
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
            Job::Page(p, _) | Job::Tile(p, ..) | Job::Thumb(p, _) | Job::Links(p) => *p,
        };
        wanted.sort_by_key(|j| (page_of(j) as isize - middle as isize).unsigned_abs());
        // Page pictures first (they cover the page soonest), then its tiles.
        tile_jobs.sort_by_key(|j| (page_of(j) as isize - middle as isize).unsigned_abs());
        wanted.extend(tile_jobs);
        // Links of the pages in sight (quick), before any picture.
        {
            let links = self.shared.links.lock().unwrap_or_else(|e| e.into_inner());
            let missing: Vec<Job> = (first..=last).filter(|p| !links.contains_key(p)).map(Job::Links).collect();
            wanted.splice(0..0, missing);
        }
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

impl Layout {
    /// The page and the point on it (points from its top left) under a
    /// window point, if any.
    pub fn hit(&self, r: Rect, x: f32, y: f32) -> Option<(usize, f32, f32)> {
        let z = self.zoom(r);
        let tops = self.tops();
        let (first, last) = self.in_sight(r);
        (first..=last).find_map(|p| {
            let (pw, ph) = self.pages[p];
            let (dx, dy) = (r.x + (r.w - pw * z) / 2.0 + self.pan_x, r.y + MARGIN + (tops[p] - self.top) * z);
            let (px, py) = ((x - dx) / z, (y - dy) / z);
            (px >= 0.0 && py >= 0.0 && px <= pw && py <= ph).then_some((p, px, py))
        })
    }

    /// What the link under a window point does, if there is one.
    pub fn link_at(&self, r: Rect, x: f32, y: f32) -> Option<Action> {
        let (p, px, py) = self.hit(r, x, y)?;
        let links = self.shared.links.lock().unwrap_or_else(|e| e.into_inner());
        links.get(&p)?.iter().rev().find(|l| px >= l.rect[0] && px <= l.rect[2] && py >= l.rect[1] && py <= l.rect[3]).map(|l| l.action.clone())
    }

    /// A destination: its page, and the spot on it when it names one.
    pub fn go_to_target(&mut self, t: Target, r: Rect) {
        let page = t.page.min(self.pages.len() - 1);
        match t.top {
            Some(top) => {
                let z = self.zoom(r);
                self.top = self.tops()[page] + top.clamp(0.0, self.pages[page].1) - 8.0 / z;
                self.clamp(r);
            }
            None => self.go_to(page, r),
        }
    }

    /// Each thumbnail's size, in points.
    fn thumb_size(&self, p: usize) -> (f32, f32) {
        let (pw, ph) = self.pages[p];
        let k = (THUMB_W / pw).min(THUMB_W * 1.5 / ph);
        (pw * k, ph * k)
    }

    fn thumb_tops(&self) -> Vec<f32> {
        let mut y = THUMB_GAP;
        (0..self.pages.len())
            .map(|p| {
                let t = y;
                y += self.thumb_size(p).1 + THUMB_GAP;
                t
            })
            .collect()
    }

    fn thumbs_total(&self) -> f32 {
        (0..self.pages.len()).map(|p| self.thumb_size(p).1 + THUMB_GAP).sum::<f32>() + THUMB_GAP
    }

    /// The thumbnails kept within the strip.
    pub fn clamp_thumbs(&mut self, r: Rect) {
        self.thumb_top = self.thumb_top.clamp(0.0, (self.thumbs_total() - r.h).max(0.0));
    }

    /// The strip scrolled so page `p`'s thumbnail shows.
    pub fn show_thumb(&mut self, p: usize, r: Rect) {
        let Some(&t) = self.thumb_tops().get(p) else { return };
        let h = self.thumb_size(p).1;
        if t < self.thumb_top + THUMB_GAP {
            self.thumb_top = t - THUMB_GAP;
        } else if t + h > self.thumb_top + r.h - THUMB_GAP {
            self.thumb_top = t + h + THUMB_GAP - r.h;
        }
        self.clamp_thumbs(r);
    }

    /// The page whose thumbnail is at window y.
    pub fn thumb_at(&self, r: Rect, y: f32) -> Option<usize> {
        let at = y - r.y + self.thumb_top;
        let tops = self.thumb_tops();
        (0..self.pages.len()).find(|&p| at >= tops[p] - THUMB_GAP / 2.0 && at <= tops[p] + self.thumb_size(p).1 + THUMB_GAP / 2.0)
    }

    /// The sidebar's thumbnails, the current page marked; asks for the
    /// ones in sight.
    #[allow(clippy::needless_range_loop)]
    pub fn draw_thumbs(&self, scene: &mut Scene, r: Rect, scale: f32, current: usize, ground: Color, accent: Color) {
        scene.push_clip(r, 0.0);
        scene.fill(r, ground);
        let tops = self.thumb_tops();
        let thumbs = self.shared.thumbs.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut wanted = Vec::new();
        let (mut first, mut last) = (usize::MAX, 0);
        for p in 0..self.pages.len() {
            let (w, h) = self.thumb_size(p);
            let y = r.y + tops[p] - self.thumb_top;
            if y + h < r.y || y > r.y + r.h {
                continue;
            }
            (first, last) = (first.min(p), last.max(p));
            let dest = Rect::new(r.x + (r.w - w) / 2.0, y, w, h);
            if p == current {
                scene.stroke(Rect::new(dest.x - 3.0, dest.y - 3.0, dest.w + 6.0, dest.h + 6.0), 3.0, 2.0, accent);
            }
            scene.fill(Rect::new(dest.x + 1.0, dest.y + 1.5, w, h), Color::rgba(0, 0, 0, 0x50));
            scene.fill(dest, Color::rgb(0xff, 0xff, 0xff));
            match thumbs.get(&p) {
                Some(img) => {
                    let id = scene.image(img.clone());
                    scene.draw_image(id, dest, 1.0);
                }
                None => wanted.push(Job::Thumb(p, round3(w * scale.max(0.25) / self.pages[p].0))),
            }
        }
        {
            let mut side = self.shared.side.lock().unwrap_or_else(|e| e.into_inner());
            if *side != wanted {
                *side = wanted;
                self.shared.ready.notify_all();
            }
        }
        if first != usize::MAX {
            let keep = first.saturating_sub(30)..=last + 30;
            if thumbs.keys().any(|p| !keep.contains(p)) {
                self.shared.thumbs.lock().unwrap_or_else(|e| e.into_inner()).retain(|p, _| keep.contains(p));
            }
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
    fn hits_links_and_targets() {
        let mut d = Layout::sized(vec![LETTER; 3]);
        let r = place();
        // Width fit at zoom 1: page 0 at x 16, y 16.
        assert_eq!(d.hit(r, 16.0 + 100.0, 16.0 + 50.0), Some((0, 100.0, 50.0)));
        assert_eq!(d.hit(r, 5.0, 50.0), None);
        d.shared.links.lock().unwrap().insert(0, vec![Link { rect: [90.0, 40.0, 200.0, 60.0], action: Action::Uri("https://example.org/".into()) }]);
        assert_eq!(d.link_at(r, 116.0, 66.0), Some(Action::Uri("https://example.org/".into())));
        assert_eq!(d.link_at(r, 116.0, 166.0), None);
        d.go_to_target(Target { page: 1, top: Some(100.0) }, r);
        assert!((d.top - (792.0 + GAP + 100.0 - 8.0)).abs() < 1e-3);
    }

    #[test]
    fn thumbnails_follow_the_page() {
        let mut d = Layout::sized(vec![LETTER; 50]);
        let r = Rect::new(0.0, 0.0, 180.0, 600.0);
        d.show_thumb(20, r);
        let p = d.thumb_at(r, 300.0).unwrap();
        assert!((17..=21).contains(&p), "{p}");
        d.show_thumb(0, r);
        assert_eq!(d.thumb_top, 0.0);
        assert_eq!(d.thumb_at(r, THUMB_GAP + 10.0), Some(0));
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
