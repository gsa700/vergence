// Picture previews for the wallpaper grid and the screen tiles: drawn once,
// kept in ~/.cache/vergence/thumbnails (named by the picture's path, size and
// time), so the page opens with them at once (an 8K JPEG takes a second or more
// to decode).

use std::path::{Path, PathBuf};

/// Preview size in pixels: 16:9, dense enough to be sharp at scale 3.
pub const W: u32 = 560;
pub const H: u32 = 315;

/// A tile in the wallpaper grid, in points.
pub const GRID_W: f32 = 152.0;
pub const GRID_H: f32 = 86.0;

/// The preview made at the grid tile's exact pixel size for `scale`, filled
/// (cropped) as the server draws "fill", so the page copies it 1:1.
pub fn grid_handle(t: &Thumb, scale: f32) -> iced::widget::image::Handle {
    let (w, h) = (((GRID_W * scale).round() as u32).max(1), ((GRID_H * scale).round() as u32).max(1));
    match image::RgbaImage::from_raw(t.w, t.h, t.rgba.clone()) {
        Some(img) => {
            let out = image::DynamicImage::ImageRgba8(img).resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8();
            iced::widget::image::Handle::from_rgba(w, h, out.into_raw())
        }
        None => iced::widget::image::Handle::from_rgba(t.w, t.h, t.rgba.clone()),
    }
}

/// A preview: width, height, straight RGBA.
#[derive(Debug, Clone)]
pub struct Thumb {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

fn cache_path(path: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let meta = std::fs::metadata(path).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    meta.len().hash(&mut h);
    meta.modified().ok()?.hash(&mut h);
    (W, H).hash(&mut h);
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".cache/vergence/thumbnails").join(format!("{:016x}.png", h.finish())))
}

/// The preview if it is already in the cache (never drawn here).
pub fn cached(path: &Path) -> Option<Thumb> {
    let img = image::open(cache_path(path)?).ok()?.to_rgba8();
    Some(Thumb { w: img.width(), h: img.height(), rgba: img.into_raw() })
}

/// The preview: from the cache, else drawn and cached.
pub fn get(path: &Path) -> Option<Thumb> {
    if let Some(t) = cached(path) {
        return Some(t);
    }
    let t = draw(path)?;
    if let Some(c) = cache_path(path) {
        let _ = c.parent().map(std::fs::create_dir_all);
        let tmp = c.with_extension("png.new");
        // The format by name: the ".new" name says nothing.
        if image::save_buffer_with_format(&tmp, &t.rgba, t.w, t.h, image::ExtendedColorType::Rgba8, image::ImageFormat::Png).is_ok() {
            let _ = std::fs::rename(&tmp, &c);
        }
    }
    Some(t)
}

/// A W x H preview, filling (cropped), as the server draws "fill".
fn draw(path: &Path) -> Option<Thumb> {
    let is_svg = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    if !is_svg {
        let img = image::open(path).ok()?;
        let filled = img.resize_to_fill(W, H, image::imageops::FilterType::Triangle).to_rgba8();
        return Some(Thumb { w: W, h: H, rgba: filled.into_raw() });
    }
    let mut canvas = resvg::tiny_skia::Pixmap::new(W, H)?;
    let data = std::fs::read(path).ok()?;
    let opt = resvg::usvg::Options { resources_dir: path.parent().map(|p| p.to_path_buf()), ..Default::default() };
    let tree = resvg::usvg::Tree::from_data(&data, &opt).ok()?;
    let (sw, sh) = (tree.size().width(), tree.size().height());
    let s = (W as f32 / sw).max(H as f32 / sh);
    let ts = resvg::tiny_skia::Transform::from_row(s, 0.0, 0.0, s, (W as f32 - sw * s) / 2.0, (H as f32 - sh * s) / 2.0);
    resvg::render(&tree, ts, &mut canvas.as_mut());
    // tiny-skia is premultiplied; previews are straight alpha (opaque here anyway).
    let mut px = canvas.take();
    for p in px.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Some(Thumb { w: W, h: H, rgba: px })
}
