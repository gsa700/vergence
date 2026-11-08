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
//     mode = "fit"             # image = "" means none (the colour alone)
//     color = "#000000"
//
// Each monitor gets the image rendered at its own native pixels, once per size:
// PNG/JPEG resampled from the source (Catmull-Rom), SVG drawn at that size by resvg,
// so it is sharp at any density. Rendering happens on a worker thread; until a
// monitor's picture is ready it shows the colour. The file is checked every second
// and changes apply at once.

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

/// "#rrggbb" -> straight RGB 0-1 (else the default dark grey).
pub fn parse_color(s: &str) -> [f32; 3] {
    let h = s.trim().trim_start_matches('#');
    let v = (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok()).flatten().unwrap_or(0x1a1a1a);
    [((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0]
}

struct Job {
    generation: u64,
    config: WallpaperConfig,
    output: String,
    w: u32,
    h: u32,
}

struct Done {
    generation: u64,
    output: String,
    w: u32,
    h: u32,
    rgba: Option<Vec<u8>>,
}

pub struct Wallpaper {
    pub config: WallpaperConfig,
    mtime: Option<SystemTime>,
    generation: u64,
    /// Per output name: its picture (size, generation, pixels).
    pictures: HashMap<String, (u32, u32, u64, MemoryRenderBuffer)>,
    requested: HashSet<(String, u32, u32, u64)>,
    jobs: std::sync::mpsc::Sender<Job>,
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
        let Some(w) = self.wallpaper.as_mut() else { return };
        if d.generation != w.generation {
            return;
        }
        match d.rgba {
            Some(rgba) => {
                let buffer = MemoryRenderBuffer::from_slice(&rgba, Fourcc::Abgr8888, (d.w as i32, d.h as i32), 1, Transform::Normal, None);
                w.pictures.insert(d.output, (d.w, d.h, d.generation, buffer));
            }
            None => {
                w.pictures.remove(&d.output);
            }
        }
        self.redraw_all();
    }

    /// The colour behind everything on an output.
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
        let current = w.pictures.get(output).filter(|(x, y, g, _)| *x == pw && *y == ph && *g == w.generation);
        if current.is_none() {
            let key = (output.to_string(), pw, ph, w.generation);
            if w.requested.insert(key) {
                let _ = w.jobs.send(Job { generation: w.generation, config, output: output.into(), w: pw, h: ph });
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
        self.requested.clear();
        true
    }
}

fn worker(jobs: std::sync::mpsc::Receiver<Job>, done: Sender<Done>) {
    // Decoded sources, kept while the same image is asked for.
    let mut source: Option<(PathBuf, Source)> = None;
    while let Ok(job) = jobs.recv() {
        let Some(path) = job.config.image.clone() else { continue };
        if source.as_ref().map(|(p, _)| p) != Some(&path) {
            source = Source::load(&path).map(|s| (path.clone(), s));
            if source.is_none() {
                tracing::warn!("wallpaper: cannot read {}", path.display());
            }
        }
        let t = std::time::Instant::now();
        let rgba = source.as_ref().map(|(_, s)| s.render(job.w, job.h, &job.config.mode, parse_color(&job.config.color)));
        tracing::info!("wallpaper: {} at {}x{} in {} ms", job.output, job.w, job.h, t.elapsed().as_millis());
        let _ = done.send(Done { generation: job.generation, output: job.output, w: job.w, h: job.h, rgba });
    }
}

enum Source {
    Raster(image::RgbaImage),
    Svg(resvg::usvg::Tree),
}

impl Source {
    fn load(path: &std::path::Path) -> Option<Source> {
        let is_svg = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"));
        if is_svg {
            let data = std::fs::read(path).ok()?;
            let opt = resvg::usvg::Options { resources_dir: path.parent().map(|p| p.to_path_buf()), ..Default::default() };
            return resvg::usvg::Tree::from_data(&data, &opt).ok().map(Source::Svg);
        }
        image::open(path).ok().map(|i| Source::Raster(i.to_rgba8()))
    }

    /// The picture for a w x h output: premultiplied RGBA, the colour behind.
    fn render(&self, w: u32, h: u32, mode: &str, color: [f32; 3]) -> Vec<u8> {
        let (sw, sh) = match self {
            Source::Raster(i) => (i.width() as f64, i.height() as f64),
            Source::Svg(t) => (t.size().width() as f64, t.size().height() as f64),
        };
        let (fw, fh) = placement(sw, sh, w as f64, h as f64, mode);
        let (x, y) = ((w as f64 - fw) / 2.0, (h as f64 - fh) / 2.0);
        let mut canvas = resvg::tiny_skia::Pixmap::new(w, h).expect("non-empty output");
        canvas.fill(resvg::tiny_skia::Color::from_rgba(color[0], color[1], color[2], 1.0).unwrap());
        match self {
            Source::Svg(tree) => {
                let ts = resvg::tiny_skia::Transform::from_row((fw / sw) as f32, 0.0, 0.0, (fh / sh) as f32, x as f32, y as f32);
                resvg::render(tree, ts, &mut canvas.as_mut());
            }
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
                if let Some(size) = resvg::tiny_skia::IntSize::from_wh(iw, ih)
                    && let Some(src) = resvg::tiny_skia::Pixmap::from_vec(pm, size)
                {
                    canvas.draw_pixmap(x.round() as i32, y.round() as i32, src.as_ref(), &resvg::tiny_skia::PixmapPaint::default(), resvg::tiny_skia::Transform::identity(), None);
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
