//! PDFs in Reader (moved from Close-Up, Reader milestone 1; PDF milestone 1, step 6): the document's pages in a
//! column, scrolled continuously, drawn by vergence-pdf's confined helper
//! (never in this process) on a worker thread that owns it.
//!
//! The view asks for the pages in sight at the screen's pixels (the one in
//! the middle first, then the one below); until a page comes back at the
//! new size its last picture is drawn scaled. Far pages are forgotten.
//! Past [`MAX_PAGE_PIXELS`] a page is drawn at that many pixels and, over
//! it, the part in sight in [`TILE`]-pixel tiles at the screen's pixels.
//! Each document's page and zoom are remembered (`places`).
//!
//! Reader milestone 1, step 2: pages in rows ([`View`]): one a row,
//! scrolled (continuous), one at a time, or two side by side (a book's
//! spread, the cover alone); and turned a quarter at a time (`turn`): the
//! worker turns each picture it draws, and the text, links and matches,
//! kept in the page's own coordinates, are turned where they are drawn or
//! hit (no tiles while turned: the page picture alone).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use vergence_pdf::outline::{Action, Entry, Link, Target};
use vergence_pdf::text::{self, Char};
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
    /// A whole page at a scale, in a look.
    Page(usize, f32, Look),
    /// One tile of a page at a scale: its column and row.
    Tile(usize, f32, u32, u32),
    /// A page's thumbnail for the sidebar, at a scale, in a look.
    Thumb(usize, f32, Look),
    /// A page's links.
    Links(usize),
    /// A page's text layer (for selecting).
    Text(usize),
    /// A page searched for the current query (its generation).
    Search(usize, u64),
}

/// A place in the text: page and character.
pub type Spot = (usize, usize);

/// A selection: where it began and where it reaches now (either order).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    pub anchor: Spot,
    pub head: Spot,
}

impl Selection {
    /// Its first and last spot, in reading order.
    pub fn span(&self) -> (Spot, Spot) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }

    /// The characters it covers on `page` of `n`, if any.
    pub fn on_page(&self, page: usize, n: usize) -> Option<(usize, usize)> {
        let (a, b) = self.span();
        if n == 0 || page < a.0 || page > b.0 {
            return None;
        }
        Some((if page == a.0 { a.1 } else { 0 }, if page == b.0 { b.1.min(n - 1) } else { n - 1 }))
    }
}

/// The search: its query and generation, the matches so far (page, boxes),
/// how far it has gone, and whether any page had text at all.
#[derive(Debug, Clone, Default)]
pub struct Search {
    pub query: String,
    pub generation: u64,
    pub matches: Vec<(usize, Vec<[f32; 4]>)>,
    /// Each match with some of the text around it, for the sidebar's results.
    pub snippets: Vec<String>,
    pub next: usize,
    pub chars_seen: usize,
    pub done: bool,
}

/// The scroll bars, as the toolkit draws its own (vergence-widgets ui.rs):
/// a thumb this wide, this far from the edge, at least this long.
pub const BAR_W: f32 = 6.0;
const BAR_INSET: f32 = 2.0;
const BAR_MIN: f32 = 24.0;
/// How near the edge a press counts as on the bar.
pub const BAR_HIT: f32 = 12.0;

/// Which scroll bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Bar {
    Down,
    Across,
}

/// A thumb in a track: its start and length along the track, for content
/// `content` long shown `room` at a time, scrolled `offset` into it.
fn thumb_on(track: f32, room: f32, content: f32, offset: f32) -> Option<(f32, f32)> {
    if content <= room + 0.5 {
        return None;
    }
    let len = (track * room / content).max(BAR_MIN).min(track);
    let at = (track - len) * (offset / (content - room)).clamp(0.0, 1.0);
    Some((at, len))
}

/// The sidebar's thumbnails: the widest box, in points, and the gap.
pub const THUMB_W: f32 = 132.0;
pub const THUMB_GAP: f32 = 16.0;

/// What a document says about itself: names and values.
pub type Meta = Vec<(String, String)>;

/// A page's picture: the scale it was drawn at, its look, the picture.
type Drawn = (f32, Look, Arc<Image>);

/// A tile's key: page, scale in thousandths, column, row.
type TileKey = (usize, u32, u32, u32);

fn tile_key(page: usize, scale: f32, tx: u32, ty: u32) -> TileKey {
    (page, (scale * 1000.0).round() as u32, tx, ty)
}

/// How the pages are set out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// One page a row, scrolled through (the default).
    Continuous,
    /// One page at a time: scrolling stops at its edges, and goes on to the
    /// next page past them.
    Single,
    /// Two pages a row, the first alone (a book's cover), then pairs.
    Two,
}

/// How a page's picture is drawn: turned (degrees clockwise), and in the
/// night look (its lightness inverted, its hues kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Look {
    pub turn: u16,
    pub night: bool,
}

/// A picture as `look` asks.
pub fn apply_look(img: Image, look: Look) -> Image {
    let mut img = turn_image(img, look.turn);
    if look.night {
        night(&mut img.rgba);
    }
    img
}

/// The night look: each pixel's lightness inverted with its hue and
/// saturation kept (white paper goes dark, black text light, a red line
/// stays red), then kept between a soft black and a soft white.
pub fn night(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let (mx, mn) = (px[0].max(px[1]).max(px[2]) as i32, px[0].min(px[1]).min(px[2]) as i32);
        let shift = 255 - mx - mn;
        for c in &mut px[..3] {
            let v = (*c as i32 + shift).clamp(0, 255);
            *c = (28 + v * 205 / 255) as u8;
        }
    }
}

/// A picture turned a quarter, half or three quarters clockwise.
pub fn turn_image(img: Image, turn: u16) -> Image {
    let (w, h) = (img.width as usize, img.height as usize);
    if turn.is_multiple_of(360) || w == 0 || h == 0 {
        return img;
    }
    let mut out = vec![0u8; w * h * 4];
    let (ow, oh) = if turn == 180 { (w, h) } else { (h, w) };
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match turn {
                90 => (h - 1 - y, x),
                180 => (w - 1 - x, h - 1 - y),
                _ => (y, w - 1 - x),
            };
            let (a, b) = ((y * w + x) * 4, (ny * ow + nx) * 4);
            out[b..b + 4].copy_from_slice(&img.rgba[a..a + 4]);
        }
    }
    Image { width: ow as u32, height: oh as u32, rgba: out }
}

/// A point of a page (w x h in its own points) as shown turned: clockwise.
pub fn turn_point(turn: u16, w: f32, h: f32, x: f32, y: f32) -> (f32, f32) {
    match turn {
        90 => (h - y, x),
        180 => (w - x, h - y),
        270 => (y, w - x),
        _ => (x, y),
    }
}

/// A point shown on a turned page, in the page's own points.
pub fn unturn_point(turn: u16, w: f32, h: f32, x: f32, y: f32) -> (f32, f32) {
    match turn {
        90 => (y, h - x),
        180 => (w - x, h - y),
        270 => (w - y, x),
        _ => (x, y),
    }
}

/// A box of a page ([x0, y0, x1, y1] in its own points) as shown turned.
pub fn turn_box(turn: u16, w: f32, h: f32, b: &[f32; 4]) -> [f32; 4] {
    let (ax, ay) = turn_point(turn, w, h, b[0], b[1]);
    let (bx, by) = turn_point(turn, w, h, b[2], b[3]);
    [ax.min(bx), ay.min(by), ax.max(bx), ay.max(by)]
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
    /// The latest picture of each page, the scale it was drawn at and its turn.
    done: Mutex<HashMap<usize, Drawn>>,
    tiles: Mutex<HashMap<TileKey, Arc<Image>>>,
    /// The sidebar's wishes (thumbnails), after the view's.
    side: Mutex<Vec<Job>>,
    thumbs: Mutex<HashMap<usize, (Look, Arc<Image>)>>,
    links: Mutex<HashMap<usize, Vec<Link>>>,
    /// The outline, once the worker has read it.
    pub outline: Mutex<Option<Arc<Vec<Entry>>>>,
    /// What the document says about itself (names and values), once read.
    pub meta: Mutex<Option<Arc<Meta>>>,
    /// The text layers of the pages near sight.
    pub texts: Mutex<HashMap<usize, Arc<Vec<Char>>>>,
    pub search: Mutex<Search>,
    /// Pages the helper could not draw, and why.
    failed: Mutex<HashMap<usize, String>>,
    closed: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl Shared {
    fn new(waker: Option<Waker>) -> Shared {
        Shared {
            wanted: Mutex::new(Vec::new()),
            ready: Condvar::new(),
            done: Mutex::new(HashMap::new()),
            tiles: Mutex::new(HashMap::new()),
            side: Mutex::new(Vec::new()),
            thumbs: Mutex::new(HashMap::new()),
            links: Mutex::new(HashMap::new()),
            outline: Mutex::new(None),
            meta: Mutex::new(None),
            texts: Mutex::new(HashMap::new()),
            search: Mutex::new(Search::default()),
            failed: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            waker: Mutex::new(waker),
        }
    }

    fn wake(&self) {
        if let Some(w) = self.waker.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            w.wake();
        }
    }

    /// The window to wake when something is drawn (a tab moved to another).
    pub fn set_waker(&self, waker: Waker) {
        *self.waker.lock().unwrap_or_else(|e| e.into_inner()) = Some(waker);
    }

    pub fn failure(&self, page: usize) -> Option<String> {
        self.failed.lock().unwrap_or_else(|e| e.into_inner()).get(&page).cloned()
    }
}

/// The worker: draws what is wanted, newest wishes first, until the
/// document closes (the helper goes with it).
fn work(mut viewer: vergence_pdf::helper::Viewer, shared: Arc<Shared>) {
    let pages_n = viewer.info.pages.len();
    let outline = viewer.outline().unwrap_or_default();
    *shared.outline.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(outline));
    let meta = viewer.metadata().unwrap_or_default();
    *shared.meta.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(meta));
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
                // The search, a page at a time, after the view and before thumbnails.
                {
                    let mut se = shared.search.lock().unwrap_or_else(|e| e.into_inner());
                    if !se.done && !se.query.is_empty() {
                        if se.next < pages_n {
                            se.next += 1;
                            break Job::Search(se.next - 1, se.generation);
                        }
                        se.done = true;
                        drop(se);
                        shared.wake();
                    }
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
            Job::Page(page, scale, look) => {
                let have = shared.done.lock().unwrap_or_else(|e| e.into_inner()).get(&page).map(|d| (d.0, d.1));
                if have.is_some_and(|(s, l)| close_enough(s, scale) && l == look) {
                    continue;
                }
                (
                    page,
                    viewer.render(page, scale, None).map(|t| {
                        let img = apply_look(Image { width: t.width, height: t.height, rgba: t.rgba }, look);
                        shared
                            .done
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(page, (scale, look, Arc::new(img)));
                    }),
                )
            }
            Job::Thumb(page, scale, look) => {
                if shared.thumbs.lock().unwrap_or_else(|e| e.into_inner()).get(&page).is_some_and(|t| t.0 == look) {
                    continue;
                }
                (
                    page,
                    viewer.render(page, scale, None).map(|t| {
                        let img = apply_look(Image { width: t.width, height: t.height, rgba: t.rgba }, look);
                        shared.thumbs.lock().unwrap_or_else(|e| e.into_inner()).insert(page, (look, Arc::new(img)));
                    }),
                )
            }
            Job::Text(page) => {
                let t = viewer.text(page).unwrap_or_default();
                shared.texts.lock().unwrap_or_else(|e| e.into_inner()).insert(page, Arc::new(t));
                shared.wake();
                continue;
            }
            Job::Search(page, generation) => {
                let query = shared.search.lock().unwrap_or_else(|e| e.into_inner()).query.clone();
                let chars = viewer.text(page).unwrap_or_default();
                let spans = text::find(&chars, &query);
                let found: Vec<Vec<[f32; 4]>> = spans.iter().map(|&(a, b)| text::boxes(&chars, a, b)).collect();
                let snippets: Vec<String> = spans.iter().map(|&(a, b)| snippet(&chars, a, b)).collect();
                let mut se = shared.search.lock().unwrap_or_else(|e| e.into_inner());
                if se.generation == generation {
                    se.chars_seen += chars.len();
                    let had = se.matches.len();
                    se.matches.extend(found.into_iter().map(|b| (page, b)));
                    se.snippets.extend(snippets);
                    // Woken for the first matches and every so often after.
                    if (had == 0 && !se.matches.is_empty()) || page % 25 == 24 {
                        drop(se);
                        shared.wake();
                    }
                }
                continue;
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
                (
                    page,
                    viewer.render(page, scale, Some((tx * TILE, ty * TILE, TILE, TILE))).map(|t| {
                        shared
                            .tiles
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(key, Arc::new(Image { width: t.width, height: t.height, rgba: t.rgba }));
                    }),
                )
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
    /// Each page's size as shown (turned), in points.
    pub pages: Arc<Vec<(f32, f32)>>,
    /// Each page's own size, in points.
    pub sizes: Arc<Vec<(f32, f32)>>,
    /// How the pages are set out, and turned (0, 90, 180, 270 clockwise).
    pub view: View,
    pub turn: u16,
    /// The night look: pages dark, their text light.
    pub night: bool,
    pub shared: Arc<Shared>,
    pub mode: Mode,
    /// The document's y (points, from the first page's top) at the top of the view.
    pub top: f32,
    /// Sideways, in window points, when the pages are wider than the window.
    pub pan_x: f32,
    /// A place to go to once the view has its size (kept from the last
    /// run): the page, and how far down its row the view's top was.
    pub restore: Option<(usize, f32)>,
    /// The sidebar's thumbnails scrolled this far, in points.
    pub thumb_top: f32,
    /// The text selected, if any.
    pub selection: Option<Selection>,
    /// The search match shown (an index into the matches).
    pub found: Option<usize>,
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
    let pages: Arc<Vec<(f32, f32)>> =
        Arc::new(viewer.info.pages.iter().map(|&(w, h)| (w.max(1.0), h.max(1.0))).collect());
    let shared = Arc::new(Shared::new(waker));
    let s = shared.clone();
    std::thread::spawn(move || work(viewer, s));
    Ok(Pdf(Layout {
        sizes: pages.clone(),
        pages,
        view: View::Continuous,
        turn: 0,
        night: false,
        shared,
        mode: Mode::Width,
        top: 0.0,
        pan_x: 0.0,
        restore: None,
        thumb_top: 0.0,
        selection: None,
        found: None,
    }))
}

impl Layout {
    /// For tests: pages with no helper behind them.
    #[cfg(test)]
    fn sized(pages: Vec<(f32, f32)>) -> Layout {
        let pages = Arc::new(pages);
        Layout {
            sizes: pages.clone(),
            pages,
            view: View::Continuous,
            turn: 0,
            night: false,
            shared: Arc::new(Shared::new(None)),
            mode: Mode::Width,
            top: 0.0,
            pan_x: 0.0,
            restore: None,
            thumb_top: 0.0,
            selection: None,
            found: None,
        }
    }

    /// The pages in rows, as the view sets them out.
    pub fn rows(&self) -> Vec<std::ops::Range<usize>> {
        let n = self.pages.len();
        match self.view {
            View::Continuous | View::Single => (0..n).map(|p| p..p + 1).collect(),
            View::Two => {
                let mut out = Vec::new();
                out.push(0..1.min(n));
                let mut p = 1;
                while p < n {
                    out.push(p..(p + 2).min(n));
                    p += 2;
                }
                out
            }
        }
    }

    /// The row a page is in.
    pub fn row_of(&self, p: usize) -> usize {
        match self.view {
            View::Two => p.div_ceil(2),
            _ => p,
        }
    }

    fn row_size(&self, row: &std::ops::Range<usize>) -> (f32, f32) {
        let w = self.pages[row.clone()].iter().map(|p| p.0).sum::<f32>() + GAP * (row.len().saturating_sub(1)) as f32;
        let h = self.pages[row.clone()].iter().map(|p| p.1).fold(0.0, f32::max);
        (w, h)
    }

    /// Each row's top in the document, in points.
    fn row_tops(&self) -> Vec<f32> {
        let mut y = 0.0;
        self.rows()
            .iter()
            .map(|row| {
                let t = y;
                y += self.row_size(row).1 + GAP;
                t
            })
            .collect()
    }

    /// Each page's place in the document: its left edge from the middle of
    /// the view, and its top (pages in a row share their row's top).
    pub fn spots(&self) -> Vec<(f32, f32)> {
        let mut out = vec![(0.0, 0.0); self.pages.len()];
        for (row, top) in self.rows().iter().zip(self.row_tops()) {
            let (rw, rh) = self.row_size(row);
            let mut x = -rw / 2.0;
            for p in row.clone() {
                let (pw, ph) = self.pages[p];
                // A page shorter than its row's tallest: centered in it.
                out[p] = (x, top + (rh - ph) / 2.0);
                x += pw + GAP;
            }
        }
        out
    }

    /// Each page's top in the document, in points.
    pub fn tops(&self) -> Vec<f32> {
        self.spots().into_iter().map(|s| s.1).collect()
    }

    pub fn total(&self) -> f32 {
        self.rows().iter().map(|row| self.row_size(row).1 + GAP).sum::<f32>() - GAP
    }

    fn widest(&self) -> f32 {
        self.rows().iter().map(|row| self.row_size(row).0).fold(1.0, f32::max)
    }

    /// How the pages' pictures are drawn now.
    pub fn look(&self) -> Look {
        Look { turn: self.turn, night: self.night }
    }

    /// The pages turned to `turn` (0, 90, 180 or 270): their sizes as shown.
    pub fn set_turn(&mut self, turn: u16, r: Rect) {
        let current = self.current(r);
        self.turn = turn % 360;
        let t = self.turn;
        self.pages =
            Arc::new(self.sizes.iter().map(|&(w, h)| if t == 90 || t == 270 { (h, w) } else { (w, h) }).collect());
        self.go_to(current, r);
    }

    /// The pages set out anew, the current page kept in view.
    pub fn set_view(&mut self, view: View, r: Rect) {
        let current = self.current(r);
        self.view = view;
        self.go_to(current, r);
    }

    /// Document points per window point, in a place `r`.
    pub fn zoom(&self, r: Rect) -> f32 {
        let room = ((r.w - 2.0 * MARGIN).max(40.0), (r.h - 2.0 * MARGIN).max(40.0));
        match self.mode {
            Mode::Width => room.0 / self.widest(),
            Mode::Page => {
                let rows = self.rows();
                let row = &rows[self.row_of(self.current(r).min(self.pages.len() - 1)).min(rows.len() - 1)];
                let (w, h) = self.row_size(row);
                (room.0 / w).min(room.1 / h)
            }
            Mode::Zoom(z) => z,
        }
        .clamp(MIN_ZOOM, MAX_ZOOM)
    }

    /// The first page of the row at the middle of the view (by the zoom of
    /// Width or Zoom: Page's zoom depends on it).
    pub fn current(&self, r: Rect) -> usize {
        let z = match self.mode {
            Mode::Zoom(z) => z,
            _ => ((r.w - 2.0 * MARGIN).max(40.0) / self.widest()).clamp(MIN_ZOOM, MAX_ZOOM),
        };
        let mid = self.top + (r.h / 2.0 - MARGIN) / z;
        let row = self.row_tops().iter().rposition(|&t| t <= mid + GAP / 2.0).unwrap_or(0);
        self.rows().get(row).map_or(0, |r| r.start)
    }

    /// The part of the document the view may show: all of it, or (one page
    /// at a time) the current row.
    fn bounds(&self, r: Rect) -> (f32, f32) {
        if self.view == View::Single {
            let rows = self.rows();
            let row = self.row_of(self.current(r)).min(rows.len() - 1);
            let top = self.row_tops()[row];
            return (top, top + self.row_size(&rows[row]).1);
        }
        (0.0, self.total())
    }

    /// The view kept on the document.
    pub fn clamp(&mut self, r: Rect) {
        let z = self.zoom(r);
        let view = (r.h - 2.0 * MARGIN) / z;
        let (a, b) = self.bounds(r);
        let max = (b - view).max(a);
        self.top = if b - a <= view { a - (view - (b - a)) / 2.0 } else { self.top.clamp(a, max) };
        let wide = self.widest() * z + 2.0 * MARGIN;
        let room = (wide - r.w).max(0.0) / 2.0;
        self.pan_x = self.pan_x.clamp(-room, room);
    }

    /// Where the view's top is: the first page of the row there, and how
    /// far down that row, in points (session milestone 1: kept across a
    /// restart, so a document comes back scrolled to where it was).
    pub fn place(&self) -> (usize, f32) {
        let rows = self.rows();
        let tops = self.row_tops();
        let row = tops.iter().rposition(|&t| t <= self.top + 0.5).unwrap_or(0).min(rows.len().saturating_sub(1));
        (rows.get(row).map_or(0, |r| r.start), (self.top - tops.get(row).copied().unwrap_or(0.0)).max(0.0))
    }

    /// Back to a place given by [`Layout::place`].
    pub fn go_to_place(&mut self, p: usize, within: f32, r: Rect) {
        self.go_to(p, r);
        if within > 0.5 && within.is_finite() {
            let row = self.row_of(p.min(self.pages.len() - 1)).min(self.rows().len() - 1);
            self.top = self.row_tops()[row] + within;
            self.clamp(r);
        }
    }

    /// Page `p`'s row at the top of the view (centered when it fits whole).
    pub fn go_to(&mut self, p: usize, r: Rect) {
        let p = p.min(self.pages.len() - 1);
        let rows = self.rows();
        let row = self.row_of(p).min(rows.len() - 1);
        let top = self.row_tops()[row];
        // Page mode's zoom is the row's own: worked out with it current.
        self.top = top;
        let z = self.zoom(r);
        let view = (r.h - 2.0 * MARGIN) / z;
        let h = self.row_size(&rows[row]).1;
        self.top = if h <= view { top - (view - h) / 2.0 } else { top };
        self.clamp(r);
    }

    /// Scrolled by window points; one page at a time, a scroll at the
    /// page's edge goes on to the next page (or back to the one before).
    pub fn scroll(&mut self, dy: f32, dx: f32, r: Rect) {
        let z = self.zoom(r);
        if self.view == View::Single && dy != 0.0 {
            let view = (r.h - 2.0 * MARGIN) / z;
            let (a, b) = self.bounds(r);
            let at_end = self.top >= (b - view).max(a) - 0.5;
            let at_start = self.top <= a + 0.5;
            let rows = self.rows();
            let row = self.row_of(self.current(r));
            if dy > 0.0 && at_end && row + 1 < rows.len() {
                self.go_to(rows[row + 1].start, r);
                let (a, _) = self.bounds(r);
                self.top = self.top.min(a);
                self.clamp(r);
                return;
            }
            if dy < 0.0 && at_start && row > 0 {
                self.go_to(rows[row - 1].start, r);
                let (_, b) = self.bounds(r);
                self.top = (b - view).max(self.top);
                self.clamp(r);
                return;
            }
        }
        // One page at a time: held within the page it was on.
        let held = (self.view == View::Single).then(|| self.bounds(r));
        self.top += dy / z;
        if let Some((a, b)) = held {
            let view = (r.h - 2.0 * MARGIN) / z;
            self.top = self.top.clamp(a, (b - view).max(a));
        }
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

    /// Whether the current row fits whole: then Left and Right go a page.
    pub fn page_fits(&self, r: Rect) -> bool {
        let z = self.zoom(r);
        let rows = self.rows();
        let (w, h) = self.row_size(&rows[self.row_of(self.current(r)).min(rows.len() - 1)]);
        h * z <= r.h - MARGIN && w * z <= r.w
    }

    /// The pages in sight, `[first, last]`.
    pub fn in_sight(&self, r: Rect) -> (usize, usize) {
        let z = self.zoom(r);
        let (a, b) = (self.top - MARGIN / z, self.top + (r.h - MARGIN) / z);
        let (lo, hi) = if self.view == View::Single { self.bounds(r) } else { (f32::MIN, f32::MAX) };
        let spots = self.spots();
        let shown = |p: usize| {
            let (t, h) = (spots[p].1, self.pages[p].1);
            t + h >= a.max(lo) && t <= b.min(hi)
        };
        let first = (0..self.pages.len()).find(|&p| shown(p)).unwrap_or(0);
        let last = (first..self.pages.len())
            .take_while(|&p| shown(p) || spots[p].1 <= spots[first].1)
            .last()
            .unwrap_or(first)
            .max(first);
        (first, last)
    }

    /// The page view's scroll bars' thumbs, in window points: down the
    /// right edge when the document is taller than the view, across the
    /// bottom when wider.
    pub fn bar(&self, r: Rect, which: Bar) -> Option<Rect> {
        let z = self.zoom(r);
        match which {
            Bar::Down => {
                let (content, room) = (self.total() * z + 2.0 * MARGIN, r.h);
                let (at, len) = thumb_on(r.h - 2.0 * BAR_INSET, room, content, self.top * z)?;
                Some(Rect::new(r.x + r.w - BAR_W - BAR_INSET, r.y + BAR_INSET + at, BAR_W, len))
            }
            Bar::Across => {
                let content = self.widest() * z + 2.0 * MARGIN;
                let room_x = (content - r.w).max(0.0) / 2.0;
                let (at, len) = thumb_on(r.w - 2.0 * BAR_INSET - BAR_W - BAR_INSET, r.w, content, room_x - self.pan_x)?;
                Some(Rect::new(r.x + BAR_INSET + at, r.y + r.h - BAR_W - BAR_INSET, len, BAR_W))
            }
        }
    }

    /// The bar a window point is on, if any.
    pub fn bar_at(&self, r: Rect, x: f32, y: f32) -> Option<Bar> {
        if self.bar(r, Bar::Down).is_some() && x >= r.x + r.w - BAR_HIT && y >= r.y && y <= r.y + r.h {
            return Some(Bar::Down);
        }
        if self.bar(r, Bar::Across).is_some() && y >= r.y + r.h - BAR_HIT && x >= r.x && x <= r.x + r.w {
            return Some(Bar::Across);
        }
        None
    }

    /// Scrolled so the bar's thumb starts at window point `at` (y down,
    /// x across): a drag of the thumb, or a press on its track.
    pub fn bar_to(&mut self, r: Rect, which: Bar, at: f32) {
        let Some(t) = self.bar(r, which) else { return };
        let z = self.zoom(r);
        match which {
            Bar::Down => {
                let track = r.h - 2.0 * BAR_INSET;
                let frac = ((at - r.y - BAR_INSET) / (track - t.h).max(1.0)).clamp(0.0, 1.0);
                let view = (r.h - 2.0 * MARGIN) / z;
                self.top = frac * (self.total() - view).max(0.0);
            }
            Bar::Across => {
                let track = r.w - 3.0 * BAR_INSET - BAR_W;
                let frac = ((at - r.x - BAR_INSET) / (track - t.w).max(1.0)).clamp(0.0, 1.0);
                let room = (self.widest() * z + 2.0 * MARGIN - r.w).max(0.0) / 2.0;
                self.pan_x = room - frac * 2.0 * room;
            }
        }
        // One page at a time: the page the bar points at becomes the page.
        if self.view == View::Single {
            let held = self.top;
            let p = self.current(r);
            self.go_to(p, r);
            if which == Bar::Down {
                let (a, b) = self.bounds(r);
                let view = (r.h - 2.0 * MARGIN) / z;
                self.top = held.clamp(a, (b - view).max(a));
            }
        }
        self.clamp(r);
    }

    /// The thumbnail strip's bar thumb, in window points.
    pub fn thumbs_bar(&self, r: Rect) -> Option<Rect> {
        let (at, len) = thumb_on(r.h - 2.0 * BAR_INSET, r.h, self.thumbs_total(), self.thumb_top)?;
        Some(Rect::new(r.x + r.w - BAR_W - BAR_INSET, r.y + BAR_INSET + at, BAR_W, len))
    }

    /// The thumbnail strip scrolled so its thumb starts at window y `at`.
    pub fn thumbs_bar_to(&mut self, r: Rect, at: f32) {
        let Some(t) = self.thumbs_bar(r) else { return };
        let frac = ((at - r.y - BAR_INSET) / (r.h - 2.0 * BAR_INSET - t.h).max(1.0)).clamp(0.0, 1.0);
        self.thumb_top = frac * (self.thumbs_total() - r.h).max(0.0);
        self.clamp_thumbs(r);
    }

    /// Where page `p` is drawn in the window, at zoom `z`.
    pub fn dest(&self, r: Rect, z: f32, spots: &[(f32, f32)], p: usize) -> Rect {
        let (pw, ph) = self.pages[p];
        Rect::new(
            r.x + r.w / 2.0 + spots[p].0 * z + self.pan_x,
            r.y + MARGIN + (spots[p].1 - self.top) * z,
            pw * z,
            ph * z,
        )
    }

    /// Draws the pages in `r` and asks the worker for what is missing.
    #[allow(clippy::needless_range_loop)]
    pub fn draw(&self, scene: &mut Scene, r: Rect, scale: f32, ground: Color, accent: Color, held: Option<Bar>) {
        scene.push_clip(r, 0.0);
        scene.fill(r, ground);
        let z = self.zoom(r);
        let turn = self.turn;
        let look = self.look();
        let paper = if self.night { Color::rgb(28, 28, 28) } else { Color::rgb(0xff, 0xff, 0xff) };
        let (first, last) = self.in_sight(r);
        let spots = self.spots();
        let done = self.shared.done.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let failed: Vec<usize> = self.shared.failed.lock().unwrap_or_else(|e| e.into_inner()).keys().copied().collect();
        let tiles = self.shared.tiles.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let texts = self.shared.texts.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let search = self.shared.search.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut wanted = Vec::new();
        let mut tile_jobs = Vec::new();
        let mut keep_tiles: Vec<TileKey> = Vec::new();
        let middle = self.current(r);
        for p in first..=last {
            let (pw, ph) = self.pages[p];
            let (sw, sh) = self.sizes[p];
            let dest = self.dest(r, z, &spots, p);
            scene.fill(Rect::new(dest.x + 1.0, dest.y + 2.0, dest.w, dest.h), Color::rgba(0, 0, 0, 0x50));
            scene.fill(dest, paper);
            if let Some((_, l, img)) = done.get(&p)
                && *l == look
            {
                let id = scene.image(img.clone());
                scene.draw_image(id, dest, 1.0);
            }
            if failed.contains(&p) {
                scene.fill(dest, Color::rgba(0xd0, 0x30, 0x30, 0x30));
            }
            // Boxes in the page's own points, turned as the page is.
            let to_window = |b: &[f32; 4]| {
                let b = turn_box(turn, sw, sh, b);
                Rect::new(dest.x + b[0] * z, dest.y + b[1] * z, (b[2] - b[0]) * z, (b[3] - b[1]) * z)
            };
            // Search matches: yellow, the one shown orange.
            for (i, (mp, bs)) in search.matches.iter().enumerate() {
                if *mp == p {
                    let c = if self.found == Some(i) {
                        Color::rgba(0xff, 0x8c, 0x00, 0x90)
                    } else {
                        Color::rgba(0xff, 0xd6, 0x00, 0x70)
                    };
                    for b in bs {
                        scene.fill(to_window(b), c);
                    }
                }
            }
            // The selection, in the accent.
            if let (Some(sel), Some(chars)) = (self.selection, texts.get(&p))
                && let Some((a, b)) = sel.on_page(p, chars.len())
            {
                for bx in text::boxes(chars, a, b) {
                    scene.fill(to_window(&bx), Color::rgba(accent.r, accent.g, accent.b, 0x60));
                }
            }
            let s = page_scale(pw, ph, z * scale);
            if !done.get(&p).is_some_and(|d| close_enough(d.0, s) && d.1 == look) && !failed.contains(&p) {
                wanted.push(Job::Page(p, s, look));
            }
            // Deeper than a page picture goes: the part in sight in tiles
            // (not while turned: the page picture alone).
            let deep = round3(z * scale);
            if look == Look::default() && deep > s * 1.02 && !failed.contains(&p) {
                let k = deep / z; // pixels per window point
                let vis = (
                    dest.x.max(r.x),
                    dest.y.max(r.y),
                    (dest.x + dest.w).min(r.x + r.w),
                    (dest.y + dest.h).min(r.y + r.h),
                );
                if vis.2 > vis.0 && vis.3 > vis.1 {
                    let t = TILE as f32;
                    let (c0, r0) =
                        (((vis.0 - dest.x) * k / t).floor() as u32, ((vis.1 - dest.y) * k / t).floor() as u32);
                    let (c1, r1) = (((vis.2 - dest.x) * k / t).ceil() as u32, ((vis.3 - dest.y) * k / t).ceil() as u32);
                    if ((c1 - c0) * (r1 - r0)) as usize <= MAX_TILES {
                        for ty in r0..r1 {
                            for tx in c0..c1 {
                                let key = tile_key(p, deep, tx, ty);
                                match tiles.get(&key) {
                                    Some(img) => {
                                        let id = scene.image(img.clone());
                                        let at = Rect::new(
                                            dest.x + tx as f32 * t / k,
                                            dest.y + ty as f32 * t / k,
                                            img.width as f32 / k,
                                            img.height as f32 / k,
                                        );
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
        if last + 1 < self.pages.len() && !done.get(&(last + 1)).is_some_and(|d| d.1 == look) {
            let (pw, ph) = self.pages[last + 1];
            wanted.push(Job::Page(last + 1, page_scale(pw, ph, z * scale), look));
        }
        let page_of = |j: &Job| match j {
            Job::Page(p, ..)
            | Job::Tile(p, ..)
            | Job::Thumb(p, ..)
            | Job::Links(p)
            | Job::Text(p)
            | Job::Search(p, _) => *p,
        };
        wanted.sort_by_key(|j| (page_of(j) as isize - middle as isize).unsigned_abs());
        // Page pictures first (they cover the page soonest), then its tiles.
        tile_jobs.sort_by_key(|j| (page_of(j) as isize - middle as isize).unsigned_abs());
        wanted.extend(tile_jobs);
        // Links of the pages in sight (quick), before any picture.
        {
            let links = self.shared.links.lock().unwrap_or_else(|e| e.into_inner());
            let texts = self.shared.texts.lock().unwrap_or_else(|e| e.into_inner());
            let mut missing: Vec<Job> = (first..=last).filter(|p| !links.contains_key(p)).map(Job::Links).collect();
            missing.extend((first..=last).filter(|p| !texts.contains_key(p)).map(Job::Text));
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
        self.shared
            .texts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|p, _| keep.contains(p) || self.selection.is_some_and(|s| s.on_page(*p, usize::MAX).is_some()));
        self.shared.done.lock().unwrap_or_else(|e| e.into_inner()).retain(|p, _| keep.contains(p));
        // Tiles: only those of this zoom in sight.
        if tiles.len() > keep_tiles.len() {
            self.shared.tiles.lock().unwrap_or_else(|e| e.into_inner()).retain(|k, _| keep_tiles.contains(k));
        }
        // The scroll bars, over the pages.
        for which in [Bar::Down, Bar::Across] {
            // A mid gray, seen on white paper and on the dark ground alike.
            if let Some(t) = self.bar(r, which) {
                let a = if held == Some(which) { 240 } else { 170 };
                scene.fill_rounded(t, BAR_W / 2.0, Color::rgba(0x8a, 0x8a, 0x8e, a));
            }
        }
        scene.pop_clip();
    }
}

impl Layout {
    /// The page and the point on it (points from its top left) under a
    /// window point, if any.
    /// (In the page's own points: turned back if the page is turned.)
    pub fn hit(&self, r: Rect, x: f32, y: f32) -> Option<(usize, f32, f32)> {
        let z = self.zoom(r);
        let spots = self.spots();
        let (first, last) = self.in_sight(r);
        (first..=last).find_map(|p| {
            let (pw, ph) = self.pages[p];
            let d = self.dest(r, z, &spots, p);
            let (px, py) = ((x - d.x) / z, (y - d.y) / z);
            let (sw, sh) = self.sizes[p];
            (px >= 0.0 && py >= 0.0 && px <= pw && py <= ph).then(|| {
                let (ux, uy) = unturn_point(self.turn, sw, sh, px, py);
                (p, ux, uy)
            })
        })
    }

    /// What the link under a window point does, if there is one.
    pub fn link_at(&self, r: Rect, x: f32, y: f32) -> Option<Action> {
        let (p, px, py) = self.hit(r, x, y)?;
        let links = self.shared.links.lock().unwrap_or_else(|e| e.into_inner());
        links
            .get(&p)?
            .iter()
            .rev()
            .find(|l| px >= l.rect[0] && px <= l.rect[2] && py >= l.rect[1] && py <= l.rect[3])
            .map(|l| l.action.clone())
    }

    /// The text under a window point: on a character (a little grown), its
    /// page and the nearest character.
    pub fn text_at(&self, r: Rect, x: f32, y: f32) -> Option<Spot> {
        let (p, px, py) = self.hit(r, x, y)?;
        let texts = self.shared.texts.lock().unwrap_or_else(|e| e.into_inner());
        let chars = texts.get(&p)?;
        text::on_text(chars, px, py).then(|| text::nearest(chars, px, py)).flatten().map(|i| (p, i))
    }

    /// The nearest character to a window point on whatever page it is over
    /// (a selection being dragged).
    pub fn spot_near(&self, r: Rect, x: f32, y: f32) -> Option<Spot> {
        let (p, px, py) = self.hit(r, x, y)?;
        let texts = self.shared.texts.lock().unwrap_or_else(|e| e.into_inner());
        text::nearest(texts.get(&p)?, px, py).map(|i| (p, i))
    }

    /// The selected text, pages joined by line breaks; None with nothing
    /// selected or a page of it not read yet.
    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection?;
        let (a, b) = sel.span();
        let texts = self.shared.texts.lock().unwrap_or_else(|e| e.into_inner());
        let mut parts = Vec::new();
        for p in a.0..=b.0 {
            let chars = texts.get(&p)?;
            if let Some((x, y)) = sel.on_page(p, chars.len()) {
                parts.push(text::selected(chars, x, y));
            }
        }
        Some(parts.join("\n")).filter(|t| !t.is_empty())
    }

    /// All of page `p`'s text selected (if it has been read).
    pub fn select_page(&mut self, p: usize) -> bool {
        let n = self.shared.texts.lock().unwrap_or_else(|e| e.into_inner()).get(&p).map_or(0, |c| c.len());
        if n == 0 {
            return false;
        }
        self.selection = Some(Selection { anchor: (p, 0), head: (p, n - 1) });
        true
    }

    /// A new search (or none, for an empty query); the worker goes through
    /// every page.
    pub fn start_search(&mut self, query: &str) {
        let mut se = self.shared.search.lock().unwrap_or_else(|e| e.into_inner());
        let generation = se.generation + 1;
        *se =
            Search { query: query.trim().to_string(), generation, done: query.trim().is_empty(), ..Search::default() };
        drop(se);
        self.found = None;
        self.shared.ready.notify_all();
    }

    /// Match `i` (wrapping) brought into view; the index shown.
    pub fn show_match(&mut self, i: isize, r: Rect) -> Option<usize> {
        let (n, m) = {
            let se = self.shared.search.lock().unwrap_or_else(|e| e.into_inner());
            let n = se.matches.len();
            if n == 0 {
                return None;
            }
            let k = i.rem_euclid(n as isize) as usize;
            (k, se.matches[k].clone())
        };
        let (page, boxes) = m;
        let (sw, sh) = self.sizes[page];
        let top = boxes.first().map(|b| turn_box(self.turn, sw, sh, b)[1]).unwrap_or(0.0);
        let z = self.zoom(r);
        // Into view unless it already is, a third of the way down.
        let tops = self.tops();
        let y = tops[page] + top;
        let view = (self.top, self.top + (r.h - 2.0 * MARGIN) / z);
        if y < view.0 || y + 20.0 > view.1 {
            self.top = y - (r.h / 3.0) / z;
            self.clamp(r);
        }
        self.found = Some(n);
        Some(n)
    }

    /// A destination: its page, and the spot on it when it names one.
    pub fn go_to_target(&mut self, t: Target, r: Rect) {
        let page = t.page.min(self.pages.len() - 1);
        match t.top.filter(|_| self.turn == 0) {
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
        (0..self.pages.len())
            .find(|&p| at >= tops[p] - THUMB_GAP / 2.0 && at <= tops[p] + self.thumb_size(p).1 + THUMB_GAP / 2.0)
    }

    /// The sidebar's thumbnails, the current page marked; asks for the
    /// ones in sight.
    #[allow(clippy::needless_range_loop)]
    #[allow(clippy::too_many_arguments)]
    pub fn draw_thumbs(
        &self,
        scene: &mut Scene,
        r: Rect,
        scale: f32,
        current: usize,
        ground: Color,
        accent: Color,
        ink: Color,
        held: bool,
    ) {
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
            scene.fill(dest, if self.night { Color::rgb(28, 28, 28) } else { Color::rgb(0xff, 0xff, 0xff) });
            match thumbs.get(&p).filter(|t| t.0 == self.look()) {
                Some((_, img)) => {
                    let id = scene.image(img.clone());
                    scene.draw_image(id, dest, 1.0);
                }
                None => wanted.push(Job::Thumb(p, round3(w * scale.max(0.25) / self.pages[p].0), self.look())),
            }
        }
        {
            let mut side = self.shared.side.lock().unwrap_or_else(|e| e.into_inner());
            if *side != wanted {
                *side = wanted;
                self.shared.ready.notify_all();
            }
        }
        if let Some(t) = self.thumbs_bar(r) {
            scene.fill_rounded(t, BAR_W / 2.0, Color::rgba(ink.r, ink.g, ink.b, if held { 150 } else { 80 }));
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

/// A match with up to 40 characters either side, on one line, cut at words.
pub fn snippet(chars: &[Char], a: usize, b: usize) -> String {
    if chars.is_empty() {
        return String::new();
    }
    let (start, end) = (a.saturating_sub(40), (b + 40).min(chars.len() - 1));
    let raw = text::selected(chars, start, end).split_whitespace().collect::<Vec<_>>().join(" ");
    let mut words: Vec<&str> = raw.split(' ').collect();
    // The first and last words may be cut: dropped, unless that leaves too little.
    if start > 0 && words.len() > 3 {
        words.remove(0);
    }
    if end + 1 < chars.len() && words.len() > 3 {
        words.pop();
    }
    let body = words.join(" ");
    format!("{}{body}{}", if start > 0 { "…" } else { "" }, if end + 1 < chars.len() { "…" } else { "" })
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

/// Each document's zoom (and the page it was left at, no longer gone to: a
/// document the user opens starts at its first page, session milestone 1;
/// the page after a restart is kept with the window, keep.rs):
/// ~/.local/state/vergence/reader-places
/// (Close-Up's closeup-pdf-places, taken over once when Reader first looks),
/// owner-only, a line each (page, mode, path), newest first, at most 500.
pub mod places {
    use super::Mode;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    const MOST: usize = 500;

    fn file() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
        let new = base.join("vergence/reader-places");
        let old = base.join("vergence/closeup-pdf-places");
        if !new.exists() && old.exists() {
            let _ = std::fs::rename(&old, &new);
        }
        Some(new)
    }

    pub fn mode_text(m: Mode) -> String {
        match m {
            Mode::Width => "width".into(),
            Mode::Page => "page".into(),
            Mode::Zoom(z) => format!("{z:.3}"),
        }
    }

    pub fn parse_mode(s: &str) -> Option<Mode> {
        match s {
            "width" => Some(Mode::Width),
            "page" => Some(Mode::Page),
            z => z
                .parse::<f32>()
                .ok()
                .filter(|z| z.is_finite())
                .map(|z| Mode::Zoom(z.clamp(super::MIN_ZOOM, super::MAX_ZOOM))),
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
            let text = format!(
                "12\t{}\t/a/b c.pdf\n3\t{}\t/x.pdf\nbad line\n",
                mode_text(Mode::Page),
                mode_text(Mode::Zoom(1.5))
            );
            let all = parse(&text);
            assert_eq!(
                all,
                vec![(12, Mode::Page, "/a/b c.pdf".to_string()), (3, Mode::Zoom(1.5), "/x.pdf".to_string())]
            );
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
        d.shared.links.lock().unwrap().insert(
            0,
            vec![Link { rect: [90.0, 40.0, 200.0, 60.0], action: Action::Uri("https://example.org/".into()) }],
        );
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
    fn selections_across_pages() {
        let s = Selection { anchor: (3, 10), head: (1, 4) };
        assert_eq!(s.span(), ((1, 4), (3, 10)));
        assert_eq!(s.on_page(0, 50), None);
        assert_eq!(s.on_page(1, 50), Some((4, 49)));
        assert_eq!(s.on_page(2, 50), Some((0, 49)));
        assert_eq!(s.on_page(3, 50), Some((0, 10)));
        assert_eq!(s.on_page(3, 5), Some((0, 4)));
    }

    #[test]
    fn two_pages_a_row_the_cover_alone() {
        let mut d = Layout::sized(vec![LETTER; 5]);
        let r = Rect::new(0.0, 0.0, 1300.0, 900.0);
        d.set_view(View::Two, r);
        assert_eq!(d.rows(), vec![0..1, 1..3, 3..5]);
        let spots = d.spots();
        assert_eq!(spots[1].1, spots[2].1, "a spread's pages share a top");
        assert!(spots[1].0 < 0.0 && spots[2].0 > 0.0, "left and right of the middle");
        assert!((spots[0].0 + LETTER.0 / 2.0).abs() < 1e-3, "the cover alone, centered");
        // Fit to the width: a spread fills it.
        assert!((d.zoom(r) - (1300.0 - 2.0 * MARGIN) / (2.0 * LETTER.0 + GAP)).abs() < 1e-4);
        d.go_to(4, r);
        assert_eq!(d.current(r), 3, "the row's first page");
    }

    #[test]
    fn one_page_at_a_time_goes_on_at_its_edges() {
        let mut d = Layout::sized(vec![LETTER; 4]);
        let r = place();
        d.set_view(View::Single, r);
        d.scroll(1e6, 0.0, r);
        assert_eq!(d.current(r), 0, "held at the page's end");
        let (_, last) = d.in_sight(r);
        assert_eq!(last, 0, "only the page shows");
        d.scroll(10.0, 0.0, r);
        assert_eq!(d.current(r), 1, "a scroll past its end: the next page");
        d.scroll(-10.0, 0.0, r);
        assert_eq!(d.current(r), 0, "and back past its start");
    }

    #[test]
    fn turned_pages() {
        let mut d = Layout::sized(vec![LETTER; 2]);
        let r = place();
        d.set_turn(90, r);
        assert_eq!(d.pages[0], (792.0, 612.0));
        // A point and its box turned and back.
        for t in [0, 90, 180, 270] {
            let (x, y) = turn_point(t, 612.0, 792.0, 100.0, 50.0);
            assert_eq!(unturn_point(t, 612.0, 792.0, x, y), (100.0, 50.0), "{t}");
        }
        assert_eq!(turn_box(90, 612.0, 792.0, &[100.0, 50.0, 200.0, 60.0]), [732.0, 100.0, 742.0, 200.0]);
        let img = Image { width: 2, height: 1, rgba: vec![1, 1, 1, 1, 2, 2, 2, 2] };
        let t = turn_image(img, 90);
        assert_eq!((t.width, t.height), (1, 2));
        assert_eq!(&t.rgba[..4], &[1, 1, 1, 1], "the left pixel at the top");
    }

    #[test]
    fn a_match_with_the_words_around_it() {
        let text = "Early one morning in the spring the quick brown fox jumps over the lazy dog and keeps running far away from the farm";
        let chars: Vec<Char> = text
            .chars()
            .enumerate()
            .map(|(i, c)| Char { text: c.to_string(), rect: [i as f32 * 5.0, 0.0, i as f32 * 5.0 + 5.0, 10.0] })
            .collect();
        let at = text.find("lazy").unwrap();
        let s = snippet(&chars, at, at + 3);
        assert!(s.contains("lazy dog"), "{s}");
        assert!(s.starts_with('…') && s.ends_with('…'), "cut both sides: {s}");
        assert!(!s.contains("  "));
        assert_eq!(snippet(&chars[..9], 0, 4), "Early one", "the whole text: no dots");
    }

    #[test]
    fn the_night_look_inverts_lightness_and_keeps_hue() {
        let mut px = vec![255, 255, 255, 255, 0, 0, 0, 255, 200, 30, 30, 255];
        night(&mut px);
        assert_eq!(&px[0..3], &[28, 28, 28], "white paper: soft black");
        assert_eq!(&px[4..7], &[233, 233, 233], "black text: soft white");
        assert!(px[8] > px[9] && px[9] == px[10], "red stays red: {:?}", &px[8..11]);
    }

    #[test]
    fn scroll_bars_follow_and_lead() {
        let mut d = Layout::sized(vec![LETTER; 10]);
        let r = place();
        let t = d.bar(r, Bar::Down).expect("ten pages are taller than the view");
        assert!((t.y - (r.y + BAR_INSET)).abs() < 1e-3, "at the top first");
        assert!(d.bar(r, Bar::Across).is_none(), "fitted to the width: nothing across");
        assert_eq!(d.bar_at(r, r.w - 4.0, 100.0), Some(Bar::Down));
        assert_eq!(d.bar_at(r, 300.0, 100.0), None);
        // Dragged to the bottom: the document's end.
        d.bar_to(r, Bar::Down, r.y + r.h);
        let view = (r.h - 2.0 * MARGIN) / d.zoom(r);
        assert!((d.top - (d.total() - view)).abs() < 1.0, "{} {}", d.top, d.total() - view);
        let t = d.bar(r, Bar::Down).unwrap();
        assert!((t.y + t.h - (r.y + r.h - BAR_INSET)).abs() < 1e-2, "the thumb at the end");
        // Zoomed in: a bar across, dragged to its right end shows the right edge.
        d.set_mode(Mode::Zoom(2.0), None, r);
        assert!(d.bar(r, Bar::Across).is_some());
        d.bar_to(r, Bar::Across, r.x + r.w);
        let room = (d.widest() * 2.0 + 2.0 * MARGIN - r.w) / 2.0;
        assert!((d.pan_x + room).abs() < 1e-2, "the right edge in view: pan {}", d.pan_x);
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
