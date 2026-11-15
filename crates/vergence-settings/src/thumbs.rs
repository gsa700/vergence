// Picture previews for the wallpaper grid and the screen tiles: decoded
// apart by vergence-decode (files milestone 1, step 4), drawn once,
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
/// (cropped) as the server draws "fill", so the page draws it 1:1.
pub fn grid_image(t: &Thumb, scale: f32) -> vergence_scene::Image {
    let (w, h) = (((GRID_W * scale).round() as u32).max(1), ((GRID_H * scale).round() as u32).max(1));
    match image::RgbaImage::from_raw(t.w, t.h, t.rgba.clone()) {
        Some(img) => {
            let out = image::DynamicImage::ImageRgba8(img).resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8();
            vergence_scene::Image { width: w, height: h, rgba: out.into_raw() }
        }
        None => vergence_scene::Image { width: t.w, height: t.h, rgba: t.rgba.clone() },
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

/// A W x H preview, filling (cropped), as the server draws "fill". The
/// picture is decoded apart, by the helper (vergence-decode: limited, and in
/// Capsicum on FreeBSD; files milestone 1), shrunk there to at least W x H.
fn draw(path: &Path) -> Option<Thumb> {
    let p = match vergence_decode::decode_file(path, Some((W, H))) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("vergence-settings: {}: {e}", path.display());
            return None;
        }
    };
    let frame = p.frames.into_iter().next()?;
    let img = image::RgbaImage::from_raw(p.width, p.height, frame.rgba)?;
    let filled = image::DynamicImage::ImageRgba8(img).resize_to_fill(W, H, image::imageops::FilterType::Triangle).to_rgba8();
    Some(Thumb { w: W, h: H, rgba: filled.into_raw() })
}
