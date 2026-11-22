// The pointer image (server milestone 1, stage 2).
//
// The pointer is the same physical size and the same shape on every monitor, like
// everything else in Vergence. XCURSOR_SIZE (default 24) is in points. Each shape
// is taken from the theme's LARGEST image only and shrunk on the CPU (area
// average) to exactly the pixels an output needs, then drawn 1:1. Themes draw
// different designs at different sizes (Adwaita: a thin hinted arrow at 24/32 px,
// a fat outlined one at 48-96 px; picking the nearest image per monitor showed a
// different arrow on each, 2026-11-07), and GPU resampling at a fractional factor
// is soft. Shapes come from the theme by name, as clients ask with
// wp-cursor-shape-v1; a client that sets its own cursor surface gets that drawn.

use std::{collections::HashMap, time::Duration};

use smithay::{
    backend::allocator::Fourcc,
    backend::renderer::{
        element::{
            Kind,
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
        },
        gles::GlesRenderer,
    },
    input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData},
    reexports::wayland_server::Resource,
    utils::{Logical, Physical, Point, Rectangle, Size, Transform},
    wayland::compositor::with_states,
};
use xcursor::{CursorTheme, parser::Image};

/// What the pointer draws on one output.
pub enum Drawn {
    Theme(MemoryRenderBufferRenderElement<GlesRenderer>),
    Surface(WaylandSurfaceRenderElement<GlesRenderer>),
}

/// A shape as the theme has it: the frames of its largest image.
struct Source {
    frames: Vec<Image>,
}

/// One frame shrunk to an output's pixels.
struct Scaled {
    buffer: MemoryRenderBuffer,
    delay: u32,
}

pub struct Pointer {
    theme: CursorTheme,
    pub theme_name: String,
    /// The pointer's size in points.
    size: u32,
    pub status: CursorImageStatus,
    /// Each shape loaded so far (None: the theme lacks it).
    sources: HashMap<CursorIcon, Option<Source>>,
    /// Shrunk frames by shape and pixel size (one entry per monitor density).
    scaled: HashMap<(CursorIcon, i32, i32), Vec<Scaled>>,
}

impl Pointer {
    /// The pointer in the desktop's cursor theme: settings.toml's
    /// Gtk/CursorThemeName (the settings app's Theme page), else XCURSOR_THEME,
    /// else Vergence's own (install.sh, vergence-harness cursors; 2026-11-18)
    /// where it is installed, else Adwaita.
    pub fn load() -> Pointer {
        let theme_name = configured_theme()
            .or_else(|| std::env::var("XCURSOR_THEME").ok())
            .unwrap_or_else(|| if own_theme_installed() { "Vergence".into() } else { "Adwaita".into() });
        // Apps started from now on draw their own pointers in it too.
        unsafe { std::env::set_var("XCURSOR_THEME", &theme_name) };
        let size = std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(24);
        tracing::info!("pointer: theme {theme_name}, {size} points");
        Pointer {
            theme: CursorTheme::load(&theme_name),
            theme_name,
            size,
            status: CursorImageStatus::default_named(),
            sources: HashMap::new(),
            scaled: HashMap::new(),
        }
    }

    /// The pointer's elements for an output at `scale`, with the pointer at `at`
    /// (physical, on that output). Front-most first.
    pub fn elements(
        &mut self,
        renderer: &mut GlesRenderer,
        at: Point<f64, Physical>,
        scale: f64,
        time: Duration,
    ) -> Vec<Drawn> {
        // A client's cursor surface that has gone away: back to the arrow.
        if let CursorImageStatus::Surface(s) = &self.status
            && !s.is_alive()
        {
            self.status = CursorImageStatus::default_named();
        }
        match self.status.clone() {
            CursorImageStatus::Hidden => Vec::new(),
            CursorImageStatus::Surface(surface) => {
                let hotspot = with_states(&surface, |states| {
                    states
                        .data_map
                        .get::<CursorImageSurfaceData>()
                        .map(|d| d.lock().unwrap().hotspot)
                        .unwrap_or_default()
                });
                let loc = (at - hotspot.to_f64().to_physical(scale)).to_i32_round();
                render_elements_from_surface_tree(renderer, &surface, loc, scale, 1.0, Kind::Cursor)
                    .into_iter()
                    .map(Drawn::Surface)
                    .collect()
            }
            CursorImageStatus::Named(icon) => self.themed(renderer, icon, at, scale, time),
        }
    }

    fn themed(
        &mut self,
        renderer: &mut GlesRenderer,
        icon: CursorIcon,
        at: Point<f64, Physical>,
        scale: f64,
        time: Duration,
    ) -> Vec<Drawn> {
        let icon = self.resolve(icon);
        let Some(Some(src)) = self.sources.get(&icon) else { return Vec::new() };
        let first = &src.frames[0];
        // The image's nominal size covers `size` points. Its logical size in whole
        // points, and its pixels exactly as Smithay will place that size on this
        // output, so the buffer is shown 1:1.
        let points_per_px = self.size as f64 / first.size as f64;
        let logical: Size<i32, Logical> = (
            ((first.width as f64 * points_per_px).round() as i32).max(1),
            ((first.height as f64 * points_per_px).round() as i32).max(1),
        )
            .into();
        let px = logical.to_f64().to_physical(scale);
        let (pw, ph) = ((px.w.round() as i32).max(1), (px.h.round() as i32).max(1));
        let hot: Point<f64, Physical> =
            (first.xhot as f64 * pw as f64 / first.width as f64, first.yhot as f64 * ph as f64 / first.height as f64)
                .into();

        let frames = self.scaled.entry((icon, pw, ph)).or_insert_with(|| {
            tracing::info!("pointer: {icon:?} from {}x{} to {pw}x{ph} px ({:.4})", first.width, first.height, scale);
            src.frames
                .iter()
                .map(|f| Scaled {
                    buffer: MemoryRenderBuffer::from_slice(
                        &shrink(&f.pixels_rgba, f.width as usize, f.height as usize, pw as usize, ph as usize),
                        // R,G,B,A bytes in memory = DRM Abgr8888 (little-endian).
                        Fourcc::Abgr8888,
                        (pw, ph),
                        1,
                        Transform::Normal,
                        None,
                    ),
                    delay: f.delay,
                })
                .collect()
        });
        let frame = pick(frames, time);
        // The whole buffer as the source: with only a size given, Smithay crops the
        // source to that size too (24x24 of 79x79 showed the arrow's top-left, enlarged).
        let whole = Rectangle::<f64, Logical>::from_size((pw as f64, ph as f64).into());
        match MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            at - hot,
            &frame.buffer,
            None,
            Some(whole),
            Some(logical),
            Kind::Cursor,
        ) {
            Ok(e) => vec![Drawn::Theme(e)],
            Err(e) => {
                tracing::warn!("pointer image: {e:?}");
                Vec::new()
            }
        }
    }

    /// `icon` if the theme has it, else the arrow; loads what it needs.
    fn resolve(&mut self, icon: CursorIcon) -> CursorIcon {
        for i in [icon, CursorIcon::Default] {
            if !self.sources.contains_key(&i) {
                let loaded = self.load_shape(i);
                if loaded.is_none() {
                    tracing::info!("pointer: theme {} has no {i:?}", self.theme_name);
                }
                self.sources.insert(i, loaded);
            }
        }
        if matches!(self.sources.get(&icon), Some(Some(_))) { icon } else { CursorIcon::Default }
    }

    fn load_shape(&self, icon: CursorIcon) -> Option<Source> {
        let path = std::iter::once(icon.name())
            .chain(icon.alt_names().iter().copied())
            .find_map(|n| self.theme.load_icon(n))?;
        let data = std::fs::read(&path).ok()?;
        let images = xcursor::parser::parse_xcursor(&data)?;
        let largest = images.iter().map(|i| i.size).max()?;
        let frames: Vec<Image> = images.into_iter().filter(|i| i.size == largest).collect();
        Some(Source { frames })
    }
}

impl Pointer {
    pub fn size_points(&self) -> u32 {
        self.size
    }

    /// The arrow for Xwayland's root window (X apps that set no pointer of their
    /// own), shrunk to `px` pixels like the server's own: RGBA, size, hotspot.
    pub fn x_default(&mut self, px: u32) -> Option<(Vec<u8>, u16, u16, u16, u16)> {
        let icon = self.resolve(CursorIcon::Default);
        let first = &self.sources.get(&icon)?.as_ref()?.frames[0];
        let f = px.max(1) as f64 / first.size as f64;
        let (w, h) =
            (((first.width as f64 * f).round() as usize).max(1), ((first.height as f64 * f).round() as usize).max(1));
        let pixels = shrink(&first.pixels_rgba, first.width as usize, first.height as usize, w, h);
        let hx = (first.xhot as f64 * w as f64 / first.width as f64).round() as u16;
        let hy = (first.yhot as f64 * h as f64 / first.height as f64).round() as u16;
        Some((pixels, w as u16, h as u16, hx, hy))
    }
}

/// The frame of an animation to show at `time` (the first if still).
/// Is Vergence's cursor theme where Xcursor finds themes (~/.icons, the data
/// directories' icons)?
fn own_theme_installed() -> bool {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let data = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    home.map(|h| h.join(".icons"))
        .into_iter()
        .chain(data.split(':').filter(|d| !d.is_empty()).map(|d| std::path::Path::new(d).join("icons")))
        .any(|d| d.join("Vergence/cursors/default").exists())
}

/// Gtk/CursorThemeName from ~/.config/vergence/settings.toml.
fn configured_theme() -> Option<String> {
    let path = std::path::Path::new(&std::env::var_os("HOME")?).join(".config/vergence/settings.toml");
    let s = vergence_xsettings::from_toml(&std::fs::read_to_string(path).ok()?).ok()?;
    match s.get("Gtk/CursorThemeName") {
        Some(vergence_xsettings::Value::Str(t)) if !t.is_empty() => Some(t.clone()),
        _ => None,
    }
}

fn pick(frames: &[Scaled], time: Duration) -> &Scaled {
    let total: u32 = frames.iter().map(|f| f.delay).sum();
    if total == 0 || frames.len() == 1 {
        return &frames[0];
    }
    let mut ms = (time.as_millis() % total as u128) as u32;
    for f in frames {
        if ms < f.delay {
            return f;
        }
        ms -= f.delay;
    }
    &frames[0]
}

/// Resize premultiplied RGBA by area averaging (each output pixel is the mean of the
/// source area it covers; also right for enlarging, as a box filter). Separable.
fn shrink(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    // Weights of source cells [0, n) covering destination cell i of m.
    fn spans(n: usize, m: usize) -> Vec<Vec<(usize, f32)>> {
        let step = n as f64 / m as f64;
        (0..m)
            .map(|i| {
                let (a, b) = (i as f64 * step, (i + 1) as f64 * step);
                let mut v = Vec::new();
                let mut j = a.floor() as usize;
                while (j as f64) < b && j < n {
                    let w = (b.min(j as f64 + 1.0) - a.max(j as f64)) / step;
                    if w > 0.0 {
                        v.push((j, w as f32));
                    }
                    j += 1;
                }
                v
            })
            .collect()
    }
    let (hx, hy) = (spans(sw, dw), spans(sh, dh));
    // Horizontal pass: sh rows of dw pixels.
    let mut mid = vec![0f32; dw * sh * 4];
    for y in 0..sh {
        for (x, span) in hx.iter().enumerate() {
            for &(j, w) in span {
                for c in 0..4 {
                    mid[(y * dw + x) * 4 + c] += src[(y * sw + j) * 4 + c] as f32 * w;
                }
            }
        }
    }
    // Vertical pass.
    let mut out = vec![0u8; dw * dh * 4];
    for (y, span) in hy.iter().enumerate() {
        for x in 0..dw {
            for c in 0..4 {
                let v: f32 = span.iter().map(|&(j, w)| mid[(j * dw + x) * 4 + c] * w).sum();
                out[(y * dw + x) * 4 + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::shrink;

    #[test]
    fn shrink_averages_areas() {
        // 4x1 -> 2x1: pairs averaged, all channels.
        let src = [0, 0, 0, 0, 200, 100, 50, 255, 255, 255, 255, 255, 255, 255, 255, 255];
        assert_eq!(shrink(&src, 4, 1, 2, 1), vec![100, 50, 25, 128, 255, 255, 255, 255]);
    }

    #[test]
    fn shrink_keeps_size_and_handles_fractions() {
        let src: Vec<u8> = (0..96 * 96 * 4).map(|i| (i % 251) as u8).collect();
        assert_eq!(shrink(&src, 96, 96, 96, 96), src);
        let out = shrink(&[255; 96 * 96 * 4], 96, 96, 26, 26);
        assert_eq!(out.len(), 26 * 26 * 4);
        assert!(out.iter().all(|&v| v == 255), "a uniform image stays uniform at a fractional factor");
    }
}
