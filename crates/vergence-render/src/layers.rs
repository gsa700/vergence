//! Blurred groups ([`Command::PushBlur`] … [`Command::PopBlur`]): what is
//! drawn between them, cut to the group's rectangle and blurred, as the
//! Avalonia app's BlurEffect on AlbumWall's tracks behind the lyrics (Dave,
//! 2026-11-26: "what about the blur we built for terminal?").
//!
//! **Both renderers draw a group as a picture.** Before a scene is prepared,
//! each group is drawn on its own by the CPU renderer (the reference, always
//! here), blurred, and put in the scene as an [`Command::Image`] of exactly
//! its pixels. So the GPU renderer never meets a command it could refuse: a
//! refusal there draws every window on the CPU from then on (2026-11-26,
//! twice). The pictures are kept, by the group's contents and the scale: a
//! group that does not change (the tracks behind words that scroll) is
//! drawn once, and the GPU keeps its texture, as for any picture it is
//! given again.
//!
//! The blur: a Gaussian of standard deviation `radius × 0.577 + 0.5` pixels
//! (the radius as Skia, and so Avalonia, turns it into one), as three box
//! blurs each way, on colors with alpha multiplied in so a soft edge does not
//! darken.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use vergence_scene::{Color, Command, Image, Rect, Scene};

use crate::Fonts;

/// Pictures of groups kept, the most recent last.
static KEPT: Mutex<Vec<(u64, Arc<Image>)>> = Mutex::new(Vec::new());
const KEEP: usize = 8;

/// The scene with each blurred group made a picture; the scene itself
/// (borrowed) when it has none.
pub fn flatten<'a>(scene: &'a Scene, fonts: &mut Fonts, scale: f32) -> Cow<'a, Scene> {
    if !scene.commands.iter().any(|c| matches!(c, Command::PushBlur { .. })) {
        return Cow::Borrowed(scene);
    }
    let mut out = Scene::new(scene.width, scene.height);
    out.fonts = scene.fonts.clone();
    out.images = scene.images.clone();
    let mut at = 0;
    while at < scene.commands.len() {
        match &scene.commands[at] {
            Command::PushBlur { rect, radius } => {
                // Its commands, to the matching PopBlur (a valid scene has one).
                let mut depth = 0;
                let mut end = at + 1;
                while end < scene.commands.len() {
                    match scene.commands[end] {
                        Command::PushBlur { .. } => depth += 1,
                        Command::PopBlur if depth == 0 => break,
                        Command::PopBlur => depth -= 1,
                        _ => {}
                    }
                    end += 1;
                }
                if let Some((image, place)) = group(scene, &scene.commands[at + 1..end], *rect, *radius, fonts, scale) {
                    let index = out.image(image);
                    out.draw_image(index, place, 1.0);
                }
                at = end + 1;
            }
            Command::PopBlur => at += 1,
            c => {
                out.commands.push(c.clone());
                at += 1;
            }
        }
    }
    Cow::Owned(out)
}

/// One group as a picture: drawn, cut to `rect`, blurred by `radius`
/// points, on the device's pixel grid; and where it goes, in points.
fn group(
    scene: &Scene,
    commands: &[Command],
    rect: Rect,
    radius: f32,
    fonts: &mut Fonts,
    scale: f32,
) -> Option<(Arc<Image>, Rect)> {
    let sigma = radius * scale * 0.577_35 + 0.5;
    // Room for the blur to spread past the rectangle.
    let margin = (sigma * 3.0).ceil();
    let x0 = (rect.x * scale - margin).floor();
    let y0 = (rect.y * scale - margin).floor();
    let x1 = ((rect.x + rect.w) * scale + margin).ceil();
    let y1 = ((rect.y + rect.h) * scale + margin).ceil();
    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
    if w == 0 || h == 0 || (w as u64 * h as u64) > crate::MAX_PIXELS {
        return None;
    }
    let place = Rect::new(x0 / scale, y0 / scale, w as f32 / scale, h as f32 / scale);
    let key = {
        let mut hasher = DefaultHasher::new();
        format!("{commands:?}").hash(&mut hasher);
        for f in &scene.fonts {
            (&f.family, f.weight, f.italic).hash(&mut hasher);
        }
        for im in &scene.images {
            (Arc::as_ptr(im) as usize).hash(&mut hasher);
        }
        [rect.x, rect.y, rect.w, rect.h, radius, scale].map(f32::to_bits).hash(&mut hasher);
        hasher.finish()
    };
    {
        let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = kept.iter().position(|(k, _)| *k == key) {
            let hit = kept.remove(i);
            let image = hit.1.clone();
            kept.push(hit);
            return Some((image, place));
        }
    }
    // The group alone, moved so its picture's corner is the origin, cut to its rectangle.
    let mut sub = Scene::new(w as f32 / scale, h as f32 / scale);
    sub.fonts = scene.fonts.clone();
    sub.images = scene.images.clone();
    sub.push_clip(rect, 0.0);
    sub.commands.extend(commands.iter().cloned());
    sub.pop_clip();
    sub.translate(-place.x, -place.y);
    let mut pixmap = crate::cpu::Cpu::with_threads(2).render(&sub, fonts, scale, Color::rgba(0, 0, 0, 0)).ok()?;
    let (pw, ph) = (pixmap.width as usize, pixmap.height as usize);
    blur(&mut pixmap.data, pw, ph, sigma);
    // Alpha multiplied in, as the renderer draws; a picture's is not.
    for p in pixmap.data.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    let image = Arc::new(Image { width: pw as u32, height: ph as u32, rgba: pixmap.data });
    let place = Rect::new(place.x, place.y, pw as f32 / scale, ph as f32 / scale);
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    kept.push((key, image.clone()));
    while kept.len() > KEEP {
        kept.remove(0);
    }
    Some((image, place))
}

/// The box widths of three passes that together approximate a Gaussian of
/// standard deviation `sigma` (Kovesi's way).
fn boxes(sigma: f32) -> [usize; 3] {
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let mut lo = ideal.floor() as i32;
    if lo % 2 == 0 {
        lo -= 1;
    }
    let lo = lo.max(1);
    let hi = lo + 2;
    let m = ((12.0 * sigma * sigma - 3.0 * (lo * lo) as f32 - 12.0 * lo as f32 - 9.0) / (-4.0 * lo as f32 - 4.0))
        .round() as i32;
    let pick = |i: i32| if i < m { lo as usize } else { hi as usize };
    [pick(0), pick(1), pick(2)]
}

/// Blurs RGBA pixels (alpha multiplied in) in place.
pub(crate) fn blur(data: &mut [u8], w: usize, h: usize, sigma: f32) {
    if sigma <= 0.5 || w == 0 || h == 0 {
        return;
    }
    let mut a = data.to_vec();
    let mut b = vec![0u8; data.len()];
    for size in boxes(sigma) {
        let r = size / 2;
        box_pass(&a, &mut b, w, h, r, true);
        box_pass(&b, &mut a, w, h, r, false);
    }
    data.copy_from_slice(&a);
}

/// One box blur of radius `r` along rows (`across`) or columns; outside the
/// picture counts as transparent.
fn box_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, across: bool) {
    let (lines, len) = if across { (h, w) } else { (w, h) };
    let at = |line: usize, i: usize| if across { (line * w + i) * 4 } else { (i * w + line) * 4 };
    let width = (2 * r + 1) as u32;
    for line in 0..lines {
        let mut sum = [0u32; 4];
        for i in 0..r.min(len) {
            let p = at(line, i);
            for c in 0..4 {
                sum[c] += src[p + c] as u32;
            }
        }
        for i in 0..len {
            if i + r < len {
                let p = at(line, i + r);
                for c in 0..4 {
                    sum[c] += src[p + c] as u32;
                }
            }
            let q = at(line, i);
            for c in 0..4 {
                dst[q + c] = ((sum[c] + width / 2) / width) as u8;
            }
            if i >= r {
                let p = at(line, i - r);
                for c in 0..4 {
                    sum[c] -= src[p + c] as u32;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_boxes_make_the_gaussians_spread() {
        // The variance of three boxes of widths n is the sum of (n^2 - 1) / 12.
        for sigma in [1.0f32, 2.5, 4.5, 10.0] {
            let v: f32 = boxes(sigma).iter().map(|&n| ((n * n - 1) as f32) / 12.0).sum();
            assert!((v.sqrt() - sigma).abs() < 0.6, "sigma {sigma}: {}", v.sqrt());
        }
    }

    #[test]
    fn a_blur_keeps_what_there_is_and_softens_an_edge() {
        let (w, h) = (60, 40);
        let mut px = vec![0u8; w * h * 4];
        for y in 10..30 {
            for x in 20..40 {
                px[(y * w + x) * 4..][..4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let before: u64 = px.iter().map(|&v| v as u64).sum();
        blur(&mut px, w, h, 3.0);
        let after: u64 = px.iter().map(|&v| v as u64).sum();
        // Room all round: what there was is kept, to rounding.
        assert!((after as f64 - before as f64).abs() < before as f64 * 0.02, "{after} of {before}");
        let a = |x: usize| px[(20 * w + x) * 4 + 3];
        assert!(a(10) < 10 && a(19) > 60 && a(19) < 200 && a(30) > 240, "{} {} {}", a(10), a(19), a(30));
    }
}
