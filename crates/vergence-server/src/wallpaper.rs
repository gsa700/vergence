// The wallpaper (server milestone 1, 2026-11-07; the settings app writes it).
//
// ~/.config/vergence/desktop.toml:
//
//     [wallpaper]
//     image = "/usr/local/share/backgrounds/xfce/xfce-leaves.svg"   # optional
//     mode = "fill"        # fill | fit | center | stretch
//     color = "#1a1a1a"    # behind the image, and alone without one
//
//     [[wallpaper.monitor]]   # optional: one monitor's own (2026-11-08)
//     id = "3f2a9c0e1b7d4a55"  # its EDID (vergence_profile::monitor_id), any port
//     label = "ASUS VS229"     # for people; not used
//     image = "..."            # each field optional: unset ones come from above;
//     mode = "fit"             # image = "" means none (the color alone)
//     color = "#000000"
//
// Each monitor gets the image rendered at its own native pixels, once per size:
// PNG/JPEG resampled from the source (Catmull-Rom), SVG drawn at that size by the decode helper,
// so it is sharp at any density. Rendering happens on a worker thread; until a
// monitor's picture is ready it shows the one before. The file is checked every
// second and changes apply at once.
//
// Only a monitor whose own picture, fit, color or size changed is drawn again
// (2026-11-08: choosing a picture for one screen used to redraw both). The worker
// keeps the last few decoded pictures, each shrunk to what the largest monitor
// needs (an 8K JPEG is 130 MB decoded and took 1.4 s to draw again each time).
//
// Finished pictures are also kept on disk, in ~/.cache/vergence/wallpaper, one
// file per picture, fit, color and monitor size (raw pixels: reading one back
// takes milliseconds where decoding the 8K JPEG took 2 s), so a restart shows the
// wallpaper at once. The newest CACHE_FILES are kept.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::SystemTime,
};

use serde::Deserialize;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{Kind, memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement}},
            gles::GlesRenderer,
        },
    },
    reexports::calloop::channel::{Sender, channel},
    utils::{Logical, Point, Rectangle, Size, Transform},
};

use crate::Server;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct WallpaperConfig {
    pub image: Option<PathBuf>,
    pub mode: String,
    pub color: String,
    /// Monitors with their own wallpaper.
    pub monitor: Vec<MonitorWallpaper>,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        WallpaperConfig { image: None, mode: "fill".into(), color: "#1a1a1a".into(), monitor: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct MonitorWallpaper {
    pub id: String,
    pub image: Option<PathBuf>,
    pub mode: Option<String>,
    pub color: Option<String>,
}

impl WallpaperConfig {
    /// The wallpaper for the monitor `id`: its own fields over the common ones.
    pub fn for_monitor(&self, id: &str) -> WallpaperConfig {
        let mut c = WallpaperConfig { monitor: Vec::new(), ..self.clone() };
        if let Some(m) = self.monitor.iter().find(|m| m.id == id) {
            if let Some(i) = &m.image {
                c.image = (!i.as_os_str().is_empty()).then(|| i.clone());
            }
            if let Some(mode) = &m.mode {
                c.mode = mode.clone();
            }
            if let Some(color) = &m.color {
                c.color = color.clone();
            }
        }
        c
    }
}

#[derive(Debug, Default, Deserialize)]
struct DesktopFile {
    #[serde(default)]
    wallpaper: WallpaperConfig,
}

pub fn desktop_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/vergence/desktop.toml"))
}

/// "#rrggbb" -> straight RGB 0-1 (else the default dark gray).
pub fn parse_color(s: &str) -> [f32; 3] {
    let h = s.trim().trim_start_matches('#');
    let v = (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok()).flatten().unwrap_or(0x1a1a1a);
    [((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0]
}

/// What a monitor's picture is drawn from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Look {
    image: Option<PathBuf>,
    mode: String,
    color: String,
}

impl Look {
    fn of(c: &WallpaperConfig) -> Look {
        Look { image: c.image.clone(), mode: c.mode.clone(), color: c.color.clone() }
    }
}

struct Job {
    look: Look,
    output: String,
    w: u32,
    h: u32,
}

struct Done {
    look: Look,
    output: String,
    w: u32,
    h: u32,
    rgba: Option<Vec<u8>>,
}

pub struct Wallpaper {
    pub config: WallpaperConfig,
    mtime: Option<SystemTime>,
    generation: u64,
    /// Per output name: its picture (size, what it was drawn from, pixels).
    pictures: HashMap<String, (u32, u32, Look, MemoryRenderBuffer)>,
    requested: HashSet<(String, u32, u32, Look)>,
    jobs: std::sync::mpsc::Sender<Job>,
    /// Per output name: the picture's colors averaged over a 32 x 18 grid
    /// (tint.rs: windows take a little of the wallpaper behind them), and a
    /// number that changes whenever a grid does.
    pub grids: HashMap<String, Vec<[u8; 3]>>,
    pub grid_gen: u64,
}

/// The color grid's size.
pub const GRID_W: usize = 32;
pub const GRID_H: usize = 18;

/// A picture's colors averaged over the grid, every few pixels sampled.
fn grid_of(rgba: &[u8], w: u32, h: u32) -> Vec<[u8; 3]> {
    let (w, h) = (w as usize, h as usize);
    let mut sums = vec![[0u64; 4]; GRID_W * GRID_H];
    let step = ((w * h) / 400_000).max(1).isqrt().max(1);
    for y in (0..h).step_by(step) {
        let gy = y * GRID_H / h.max(1);
        for x in (0..w).step_by(step) {
            let gx = x * GRID_W / w.max(1);
            let i = (y * w + x) * 4;
            if i + 2 < rgba.len() {
                let s = &mut sums[gy * GRID_W + gx];
                s[0] += rgba[i] as u64;
                s[1] += rgba[i + 1] as u64;
                s[2] += rgba[i + 2] as u64;
                s[3] += 1;
            }
        }
    }
    sums.iter().map(|s| if s[3] == 0 { [0, 0, 0] } else { [(s[0] / s[3]) as u8, (s[1] / s[3]) as u8, (s[2] / s[3]) as u8] }).collect()
}

impl Server {
    /// Start the worker and the file watch (called once the event loop exists).
    pub fn start_wallpaper(&mut self) {
        let (done_tx, done_rx) = channel::<Done>();
        let (job_tx, job_rx) = std::sync::mpsc::channel::<Job>();
        std::thread::spawn(move || worker(job_rx, done_tx));
        let _ = self.loop_handle.insert_source(done_rx, |event, _, state| {
            if let smithay::reexports::calloop::channel::Event::Msg(d) = event {
                state.wallpaper_done(d);
            }
        });
        let mut w = Wallpaper {
            config: WallpaperConfig::default(),
            mtime: None,
            generation: 0,
            pictures: HashMap::new(),
            requested: HashSet::new(),
            jobs: job_tx,
            grids: HashMap::new(),
            grid_gen: 0,
        };
        w.reload_if_changed();
        self.wallpaper = Some(w);
        let _ = self.loop_handle.insert_source(
            smithay::reexports::calloop::timer::Timer::from_duration(std::time::Duration::from_secs(1)),
            |_, _, state| {
                let t = std::time::Instant::now();
                if let Some(w) = state.wallpaper.as_mut()
                    && w.reload_if_changed()
                {
                    state.redraw_all();
                }
                if t.elapsed().as_millis() > 20 {
                    tracing::warn!("slow: wallpaper check {} ms", t.elapsed().as_millis());
                }
                smithay::reexports::calloop::timer::TimeoutAction::ToDuration(std::time::Duration::from_secs(1))
            },
        );
    }

    fn wallpaper_done(&mut self, d: Done) {
        let id = self.monitor_id(&d.output);
        let Some(w) = self.wallpaper.as_mut() else { return };
        // Drawn for a look this monitor no longer has: drop it.
        if d.look != Look::of(&w.config.for_monitor(&id)) {
            return;
        }
        match d.rgba {
            Some(rgba) => {
                w.grids.insert(d.output.clone(), grid_of(&rgba, d.w, d.h));
                let buffer = MemoryRenderBuffer::from_slice(&rgba, Fourcc::Abgr8888, (d.w as i32, d.h as i32), 1, Transform::Normal, None);
                w.pictures.insert(d.output, (d.w, d.h, d.look, buffer));
            }
            None => {
                // The color alone: every cell of it.
                let c = parse_color(&d.look.color);
                let rgb = [(c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8];
                w.grids.insert(d.output.clone(), vec![rgb; GRID_W * GRID_H]);
                w.pictures.remove(&d.output);
            }
        }
        w.grid_gen += 1;
        self.redraw_all();
    }

    /// The color behind everything on an output.
    pub fn background_color(&self, output: &str) -> [f32; 4] {
        let id = self.monitor_id(output);
        let c = self.wallpaper.as_ref().map(|w| parse_color(&w.config.for_monitor(&id).color)).unwrap_or([0.1, 0.1, 0.1]);
        [c[0], c[1], c[2], 1.0]
    }

    /// A monitor's identity, from its EDID (as window memory names it, kept on
    /// the output when it is set up), so its wallpaper follows it to any port;
    /// nested outputs are named by their name. (Not from the backend: during a
    /// frame the backend is out of the state, and every monitor fell back to its
    /// name, so per-monitor wallpapers never matched, 2026-11-08.)
    pub fn monitor_id(&self, output: &str) -> String {
        self.space
            .outputs()
            .find(|o| o.name() == output)
            .and_then(|o| o.user_data().get::<crate::memory::MonitorId>().map(|m| m.0.clone()))
            .unwrap_or_else(|| vergence_profile::monitor_id(output.as_bytes()))
    }

    /// The wallpaper element for an output of `px` physical pixels covering `logical`
    /// (asks the worker for a picture of that size if there is none yet).
    pub fn wallpaper_element(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &str,
        px: Size<i32, smithay::utils::Physical>,
        logical: Size<i32, Logical>,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let id = self.monitor_id(output);
        let w = self.wallpaper.as_mut()?;
        let config = w.config.for_monitor(&id);
        config.image.as_ref()?;
        let (pw, ph) = (px.w.max(1) as u32, px.h.max(1) as u32);
        let look = Look::of(&config);
        let current = w.pictures.get(output).filter(|(x, y, l, _)| *x == pw && *y == ph && *l == look);
        if current.is_none() {
            let key = (output.to_string(), pw, ph, look.clone());
            if w.requested.insert(key) {
                let _ = w.jobs.send(Job { look, output: output.into(), w: pw, h: ph });
            }
        }
        // The newest picture for this output, even from an older size, until the new one is ready.
        let (bw, bh, _, buffer) = w.pictures.get(output)?;
        let src = Rectangle::<f64, Logical>::from_size((*bw as f64, *bh as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(renderer, Point::from((0.0, 0.0)), buffer, None, Some(src), Some(logical), Kind::Unspecified).ok()
    }

    /// Draw every output again in full (wallpaper or settings changed).
    pub fn redraw_all(&mut self) {
        if let Some(b) = self.backend.as_mut() {
            b.redraw_all();
        }
    }
}

impl Wallpaper {
    /// Re-read desktop.toml if it changed; true if the wallpaper changed.
    fn reload_if_changed(&mut self) -> bool {
        let path = desktop_path();
        let mtime = path.as_ref().and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        if mtime == self.mtime && self.generation > 0 {
            return false;
        }
        self.mtime = mtime;
        let config = path
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| match toml::from_str::<DesktopFile>(&t) {
                Ok(f) => f.wallpaper,
                Err(e) => {
                    tracing::warn!("desktop.toml: {e}");
                    self.config.clone()
                }
            })
            .unwrap_or_default();
        if config == self.config && self.generation > 0 {
            return false;
        }
        tracing::info!("wallpaper: {:?} {} {}", config.image, config.mode, config.color);
        self.config = config;
        self.generation += 1;
        // Pictures stay: each monitor's is redrawn only if its own look changed.
        self.requested.clear();
        true
    }
}

/// Decoded pictures kept by the worker.
const KEEP_SOURCES: usize = 3;
/// Finished pictures kept on disk.
const CACHE_FILES: usize = 12;
const CACHE_MAGIC: &[u8; 8] = b"VWALL001";

/// The cache file for a look at w x h, named by the picture's path, size and
/// time too, so an edited picture is drawn again.
fn cache_path(look: &Look, w: u32, h: u32) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let image = look.image.as_ref()?;
    let meta = std::fs::metadata(image).ok()?;
    let mut hs = std::collections::hash_map::DefaultHasher::new();
    look.hash(&mut hs);
    (w, h, meta.len()).hash(&mut hs);
    meta.modified().ok()?.hash(&mut hs);
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".cache/vergence/wallpaper").join(format!("{:016x}-{w}x{h}.rgba", hs.finish())))
}

/// A finished picture from the cache: 8-byte magic, width and height (u32 LE),
/// then w x h premultiplied RGBA.
fn cache_read(path: &std::path::Path, w: u32, h: u32) -> Option<Vec<u8>> {
    let data = vergence_types::read_regular(path, 16 + w as u64 * h as u64 * 4).ok()?;
    let want = 16 + w as usize * h as usize * 4;
    if data.len() != want || &data[..8] != CACHE_MAGIC || data[8..12] != w.to_le_bytes() || data[12..16] != h.to_le_bytes() {
        return None;
    }
    // Touch it, so the newest-kept rule counts it as used.
    let _ = std::fs::File::options().append(true).open(path).and_then(|f| f.set_modified(SystemTime::now()));
    Some(data[16..].to_vec())
}

fn cache_write(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) {
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let mut data = Vec::with_capacity(16 + rgba.len());
    data.extend_from_slice(CACHE_MAGIC);
    data.extend_from_slice(&w.to_le_bytes());
    data.extend_from_slice(&h.to_le_bytes());
    data.extend_from_slice(rgba);
    let tmp = path.with_extension("rgba.new");
    if std::fs::write(&tmp, &data).and_then(|_| std::fs::rename(&tmp, path)).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    // Keep the newest CACHE_FILES.
    let mut files: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "rgba"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in files.into_iter().skip(CACHE_FILES) {
        let _ = std::fs::remove_file(p);
    }
}

fn worker(jobs: std::sync::mpsc::Receiver<Job>, done: Sender<Done>) {
    // The last few decoded pictures, newest first, each shrunk to `cover`.
    // (path, shrunk, picture)
    let mut sources: Vec<(PathBuf, bool, Source)> = Vec::new();
    // The largest monitor drawn for so far: rasters are kept at the size that
    // fills it, no bigger.
    let mut cover = (0u32, 0u32);
    while let Ok(job) = jobs.recv() {
        let Some(path) = job.look.image.clone() else { continue };
        let t = std::time::Instant::now();
        let cached = cache_path(&job.look, job.w, job.h);
        if let Some(rgba) = cached.as_deref().and_then(|c| cache_read(c, job.w, job.h)) {
            tracing::info!("wallpaper: {} at {}x{} from the cache in {} ms", job.output, job.w, job.h, t.elapsed().as_millis());
            let _ = done.send(Done { look: job.look, output: job.output, w: job.w, h: job.h, rgba: Some(rgba) });
            continue;
        }
        let grew = job.w > cover.0 || job.h > cover.1;
        cover = (cover.0.max(job.w), cover.1.max(job.h));
        if grew {
            // Kept pictures may now be too small for this monitor.
            sources.clear();
        }
        // "center" shows the picture at its own pixels: never from a shrunk copy.
        let may_shrink = job.look.mode != "center";
        let at = match sources.iter().position(|(p, shrunk, _)| *p == path && (may_shrink || !shrunk)) {
            Some(i) => Some(i),
            None => match Source::load(&path, cover, may_shrink) {
                Some((s, shrunk)) => {
                    sources.retain(|(p, _, _)| *p != path);
                    sources.insert(0, (path.clone(), shrunk, s));
                    sources.truncate(KEEP_SOURCES);
                    Some(0)
                }
                None => {
                    tracing::warn!("wallpaper: cannot read {}", path.display());
                    None
                }
            },
        };
        let loaded = t.elapsed().as_millis();
        let rgba = at.map(|i| sources[i].2.render(job.w, job.h, &job.look.mode, parse_color(&job.look.color)));
        tracing::info!("wallpaper: {} at {}x{} in {} ms ({} ms to read)", job.output, job.w, job.h, t.elapsed().as_millis(), loaded);
        if let (Some(c), Some(px)) = (&cached, &rgba) {
            cache_write(c, job.w, job.h, px);
        }
        let _ = done.send(Done { look: job.look, output: job.output, w: job.w, h: job.h, rgba });
    }
}

enum Source {
    Raster(image::RgbaImage),
}

impl Source {
    /// Read a picture: decoded by the helper (vergence-decode, confined),
    /// never in the server, which sees every key and every window (security
    /// pass 2026-11-15, finding 4). A picture bigger than it needs to be to
    /// fill `cover` comes back shrunk to that size; a drawing (SVG) comes
    /// drawn at that size, with no outside files. Unless shrinking is not
    /// allowed ("center"): then every pixel, a drawing at its own size.
    fn load(path: &std::path::Path, cover: (u32, u32), may_shrink: bool) -> Option<(Source, bool)> {
        let at_least = may_shrink.then_some((cover.0.max(1), cover.1.max(1)));
        let p = match vergence_decode::decode_file(path, at_least) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("wallpaper: {}: {e}", path.display());
                return None;
            }
        };
        let frame = p.frames.into_iter().next()?;
        let img = image::RgbaImage::from_raw(p.width, p.height, frame.rgba)?;
        Some((Source::Raster(img), may_shrink))
    }

    /// The picture for a w x h output: premultiplied RGBA, the color behind.
    fn render(&self, w: u32, h: u32, mode: &str, color: [f32; 3]) -> Vec<u8> {
        let (sw, sh) = match self {
            Source::Raster(i) => (i.width() as f64, i.height() as f64),
        };
        let (fw, fh) = placement(sw, sh, w as f64, h as f64, mode);
        let (x, y) = ((w as f64 - fw) / 2.0, (h as f64 - fh) / 2.0);
        let mut canvas = tiny_skia::Pixmap::new(w, h).expect("non-empty output");
        canvas.fill(tiny_skia::Color::from_rgba(color[0], color[1], color[2], 1.0).unwrap());
        match self {
            Source::Raster(img) => {
                let scaled = image::imageops::resize(img, fw.round().max(1.0) as u32, fh.round().max(1.0) as u32, image::imageops::FilterType::CatmullRom);
                // Premultiply and place it (straight alpha in, premultiplied canvas).
                let mut pm = scaled.into_raw();
                for p in pm.chunks_exact_mut(4) {
                    let a = p[3] as u32;
                    for c in &mut p[..3] {
                        *c = ((*c as u32 * a + 127) / 255) as u8;
                    }
                }
                let (iw, ih) = (fw.round().max(1.0) as u32, fh.round().max(1.0) as u32);
                if let Some(size) = tiny_skia::IntSize::from_wh(iw, ih)
                    && let Some(src) = tiny_skia::Pixmap::from_vec(pm, size)
                {
                    canvas.draw_pixmap(x.round() as i32, y.round() as i32, src.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
                }
            }
        }
        // tiny-skia's pixels are premultiplied RGBA bytes = DRM Abgr8888 (little-endian).
        canvas.take()
    }
}

/// The size of a sw x sh picture on a w x h output in `mode`.
fn placement(sw: f64, sh: f64, w: f64, h: f64, mode: &str) -> (f64, f64) {
    match mode {
        "stretch" => (w, h),
        "center" => (sw, sh),
        "fit" => {
            let s = (w / sw).min(h / sh);
            (sw * s, sh * s)
        }
        _ => {
            let s = (w / sw).max(h / sh);
            (sw * s, sh * s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placements() {
        // A 16:10 picture on a 16:9 output.
        assert_eq!(placement(1600.0, 1000.0, 1920.0, 1080.0, "fill"), (1920.0, 1200.0));
        assert_eq!(placement(1600.0, 1000.0, 1920.0, 1080.0, "fit"), (1728.0, 1080.0));
        assert_eq!(placement(1600.0, 1000.0, 1920.0, 1080.0, "stretch"), (1920.0, 1080.0));
        assert_eq!(placement(1600.0, 1000.0, 1920.0, 1080.0, "center"), (1600.0, 1000.0));
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#ff8000"), [1.0, 128.0 / 255.0, 0.0]);
        assert_eq!(parse_color("nonsense"), parse_color("#1a1a1a"));
    }
}

#[cfg(test)]
mod monitor_tests {
    use super::*;

    #[test]
    fn a_monitor_entry_overrides_all_screens() {
        let text = "[wallpaper]\ncolor = \"#202020\"\nimage = \"/a.svg\"\nmode = \"fill\"\n\n[[wallpaper.monitor]]\ncolor = \"#202020\"\nid = \"a48cece3b53cb939\"\nimage = \"/b.svg\"\nlabel = \"ASUS VS229\"\nmode = \"fit\"\n";
        let f: DesktopFile = toml::from_str(text).unwrap();
        let c = f.wallpaper.for_monitor("a48cece3b53cb939");
        assert_eq!(c.image, Some(PathBuf::from("/b.svg")));
        assert_eq!(c.mode, "fit");
        assert_eq!(f.wallpaper.for_monitor("other").image, Some(PathBuf::from("/a.svg")));
        let c = WallpaperConfig { monitor: vec![MonitorWallpaper { id: "m".into(), image: Some(PathBuf::new()), ..Default::default() }], ..f.wallpaper.clone() };
        assert_eq!(c.for_monitor("m").image, None);
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn cache_round_trip_and_rejects() {
        let dir = std::env::temp_dir().join(format!("vergence-wall-test-{}", std::process::id()));
        let path = dir.join("x-2x1.rgba");
        let px = vec![1, 2, 3, 255, 4, 5, 6, 255];
        cache_write(&path, 2, 1, &px);
        assert_eq!(cache_read(&path, 2, 1), Some(px));
        // Another size, or a damaged file, is not used.
        assert_eq!(cache_read(&path, 1, 2), None);
        std::fs::write(&path, b"short").unwrap();
        assert_eq!(cache_read(&path, 2, 1), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
