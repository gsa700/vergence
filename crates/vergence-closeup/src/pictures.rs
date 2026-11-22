//! What Close-Up does with a decoded picture's pixels: a pyramid of
//! halvings (the renderer samples a picture by its nearest pixel, so a
//! large photo drawn small is drawn from the level just larger than it is
//! shown, not from every eighth pixel of the whole), quarter turns, and the
//! pictures of a folder in the order File Viewer lists them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use vergence_scene::Image;

/// The smallest level kept (either side, in pixels).
const SMALLEST: u32 = 256;
/// The largest side a level drawn may have (GPUs' texture limit is 8192 at the least).
pub const MAX_SIDE: u32 = 8192;

/// One picture's levels: the whole first, then each half the one before.
pub fn pyramid(full: Image) -> Vec<Arc<Image>> {
    let mut levels = vec![Arc::new(full)];
    loop {
        let last = levels.last().unwrap();
        if last.width / 2 < SMALLEST || last.height / 2 < SMALLEST {
            break;
        }
        let half = halve(last);
        levels.push(Arc::new(half));
    }
    levels
}

/// A picture half as wide and half as tall, each pixel the mean of four
/// (by premultiplied color, so a transparent pixel's color does not bleed).
pub fn halve(img: &Image) -> Image {
    let (w, h) = ((img.width / 2).max(1), (img.height / 2).max(1));
    let mut out = vec![0u8; w as usize * h as usize * 4];
    let at = |x: u32, y: u32| ((y.min(img.height - 1) * img.width + x.min(img.width - 1)) as usize) * 4;
    for y in 0..h {
        for x in 0..w {
            let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let i = at(x * 2 + dx, y * 2 + dy);
                let pa = img.rgba[i + 3] as u32;
                r += img.rgba[i] as u32 * pa;
                g += img.rgba[i + 1] as u32 * pa;
                b += img.rgba[i + 2] as u32 * pa;
                a += pa;
            }
            let o = ((y * w + x) as usize) * 4;
            if a > 0 {
                out[o] = (r / a) as u8;
                out[o + 1] = (g / a) as u8;
                out[o + 2] = (b / a) as u8;
            }
            out[o + 3] = (a / 4) as u8;
        }
    }
    Image { width: w, height: h, rgba: out }
}

/// The level to draw a picture from when it is shown `px_w` pixels wide:
/// the smallest at least that wide, within the GPU's limit.
pub fn level_for(levels: &[Arc<Image>], px_w: f32) -> Option<&Arc<Image>> {
    let fits = levels.iter().filter(|l| l.width <= MAX_SIDE && l.height <= MAX_SIDE);
    let mut best = None;
    for l in fits {
        match best {
            None => best = Some(l),
            Some(_) if l.width as f32 >= px_w => best = Some(l),
            _ => {}
        }
    }
    best
}

/// A picture turned a quarter clockwise (or, `left`, counter-clockwise).
pub fn turn(img: &Image, left: bool) -> Image {
    let (w, h) = (img.width, img.height);
    let mut out = vec![0u8; img.rgba.len()];
    for y in 0..h {
        for x in 0..w {
            // Clockwise: (x, y) goes to (h-1-y, x) in a picture h wide.
            let (nx, ny) = if left { (y, w - 1 - x) } else { (h - 1 - y, x) };
            let s = ((y * w + x) as usize) * 4;
            let d = ((ny * h + nx) as usize) * 4;
            out[d..d + 4].copy_from_slice(&img.rgba[s..s + 4]);
        }
    }
    Image { width: h, height: w, rgba: out }
}

/// The part of a picture `w` x `h` from (x, y).
pub fn crop(img: &Image, x: u32, y: u32, w: u32, h: u32) -> Image {
    let (x, y) = (x.min(img.width.saturating_sub(1)), y.min(img.height.saturating_sub(1)));
    let (w, h) = (w.min(img.width - x).max(1), h.min(img.height - y).max(1));
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for row in y..y + h {
        let s = ((row * img.width + x) * 4) as usize;
        out.extend_from_slice(&img.rgba[s..s + w as usize * 4]);
    }
    Image { width: w, height: h, rgba: out }
}

/// Where an edited picture that is not a PNG is saved: "`<name> (edited).png`"
/// beside it, numbered if taken.
pub fn copy_name(file: &Path) -> PathBuf {
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "picture".into());
    let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
    for n in 1.. {
        let name = if n == 1 { format!("{stem} (edited).png") } else { format!("{stem} (edited {n}).png") };
        let p = dir.join(name);
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

/// Writes a picture (straight RGBA, as decoded) as PNG: beside the file
/// and moved into place, so a failure leaves the old one; a file replaced
/// keeps its permissions, a new one is the user's alone.
pub fn save_png(img: &Image, to: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mode = std::fs::metadata(to).map(|m| m.permissions().mode() & 0o777).unwrap_or(0o600);
    let tmp = to.with_extension("png.part");
    let res = (|| -> std::io::Result<()> {
        let file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(mode).open(&tmp)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().and_then(|mut w| w.write_image_data(&img.rgba)).map_err(std::io::Error::other)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
        std::fs::rename(&tmp, to)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// Names in the order people expect: without case, numbers by their value
/// (as File Viewer and the Open dialog sort).
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n.push(c);
                    x.next();
                }
                let mut m = String::new();
                while let Some(d) = y.peek().copied().filter(char::is_ascii_digit) {
                    m.push(d);
                    y.next();
                }
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// The pictures in a file's folder (by their names' types), in order,
/// hidden ones left out unless the file itself is one.
pub fn siblings(file: &Path) -> Vec<PathBuf> {
    let Some(dir) = file.parent() else { return vec![file.to_path_buf()] };
    let hidden_ok = file.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let Some(name) = p.file_name().and_then(|n| n.to_str()) else { return false };
                    (hidden_ok || !name.starts_with('.'))
                        && vergence_types::of_name(name).is_some_and(|m| m.starts_with("image/"))
                        && p.is_file()
                })
                .take(20_000)
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| {
        natural(
            &a.file_name().unwrap_or_default().to_string_lossy(),
            &b.file_name().unwrap_or_default().to_string_lossy(),
        )
    });
    if !out.iter().any(|p| p == file) {
        out.push(file.to_path_buf());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32) -> Image {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend([x as u8, y as u8, 0, 255]);
            }
        }
        Image { width: w, height: h, rgba }
    }

    #[test]
    fn a_pyramid_halves_down_to_its_smallest() {
        let p = pyramid(img(2000, 1100));
        let sizes: Vec<(u32, u32)> = p.iter().map(|l| (l.width, l.height)).collect();
        assert_eq!(sizes, vec![(2000, 1100), (1000, 550), (500, 275)]);
        assert_eq!(level_for(&p, 600.0).map(|l| l.width), Some(1000), "the smallest at least as wide as shown");
        assert_eq!(level_for(&p, 300.0).map(|l| l.width), Some(500));
        assert_eq!(level_for(&p, 4000.0).map(|l| l.width), Some(2000), "never more than the whole");
        assert_eq!(pyramid(img(100, 50)).len(), 1);
    }

    #[test]
    fn halving_means_four_pixels_and_keeps_clear_ones_clear() {
        let mut i = img(2, 2);
        i.rgba = vec![255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0];
        let h = halve(&i);
        assert_eq!(h.rgba, vec![255, 0, 0, 127], "red, half covered: no black bleeds in");
    }

    #[test]
    fn a_quarter_turn_each_way_and_back() {
        let i = img(3, 2);
        let r = turn(&i, false);
        assert_eq!((r.width, r.height), (2, 3));
        // Clockwise: the top-left pixel goes to the top right.
        assert_eq!(&r.rgba[4..8], &i.rgba[0..4]);
        assert_eq!(turn(&r, true).rgba, i.rgba);
    }

    #[test]
    fn names_sort_naturally() {
        let mut v = vec!["img10.png", "IMG2.png", "img1.png"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(v, vec!["img1.png", "IMG2.png", "img10.png"]);
    }

    #[test]
    fn crop_takes_the_part() {
        let img = Image { width: 4, height: 3, rgba: (0..48u8).collect() };
        let c = super::crop(&img, 1, 1, 2, 2);
        assert_eq!((c.width, c.height), (2, 2));
        assert_eq!(&c.rgba[..4], &img.rgba[(1 * 4 + 1) * 4..(1 * 4 + 1) * 4 + 4]);
        assert_eq!(&c.rgba[8..12], &img.rgba[(2 * 4 + 1) * 4..(2 * 4 + 1) * 4 + 4]);
    }

    #[test]
    fn saved_and_read_back() {
        let dir = std::env::temp_dir().join(format!("closeup-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = Image { width: 3, height: 2, rgba: (0..24u8).map(|v| v * 10).collect() };
        let to = dir.join("a.png");
        super::save_png(&img, &to).unwrap();
        let d = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&to).unwrap()));
        let mut r = d.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        r.next_frame(&mut buf).unwrap();
        assert_eq!(buf, img.rgba);
        assert_eq!(super::copy_name(&dir.join("photo.jpg")).file_name().unwrap(), "photo (edited).png");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
