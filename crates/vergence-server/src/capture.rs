// Screenshots (docs/screenshots-milestone-1.md, step 1): the server takes
// them, since it has every pixel. A request names what to take (a screen,
// an area of the desktop in points, a window alone) and is answered on the
// next frame drawn (drm.rs `run_captures`, which has the renderer): a screen
// as it is shown (its own pixels, unmagnified, the pointer left out unless
// asked); an area as each screen it touches shows its part, at the densest
// of their scales (a less dense screen's part enlarged to it); a window
// alone, its bar, border and corners, drawn at the densest scale of the
// screens it is on (a window drawn from its scene is drawn again at that
// density, sharp), on a clear ground, with a soft shadow around it unless
// asked not to (Dave, 2026-11-20: as the Mac; Vergence's windows have no
// shadow of their own, so the picture gets one).
//
// Never while the screen is locked or at the login screen, and the
// password prompt (elevate.rs) is left out of what is taken.
//
// Files: ~/Pictures/Screenshots/<date>/Screenshot <date> at <h.m.s>.png
// (Dave, 2026-11-20: "~/Pictures/Screenshots/$DATE"), a second in the same
// second numbered " (2)", every screen at once named by screen; the folder
// 0700 and the file 0600, written beside it and moved into place, off the
// main thread. Super+Shift+3 takes the screen under the pointer (Dave,
// 2026-11-20); every screen is a setting (step 8).

use std::path::PathBuf;

use smithay::utils::{Logical, Rectangle};

use crate::{Server, win::Win};

/// What to take.
#[derive(Clone)]
pub enum What {
    /// A screen, by its name; None for the screen under the pointer.
    Screen(Option<String>),
    /// Every screen, each its own file.
    Screens,
    /// An area of the desktop, in points.
    Area(Rectangle<i32, Logical>),
    /// A window alone.
    Window(Win),
}

/// Where it goes.
#[derive(Clone)]
pub enum Dest {
    /// ~/Pictures/Screenshots/<date>/.
    Folder,
    /// A file named by the caller (vctl, tests).
    Path(PathBuf),
    /// The clipboard, as image/png (Ctrl with the keys, step 3).
    Clipboard,
}

#[derive(Clone)]
pub struct Request {
    pub what: What,
    pub dest: Dest,
    /// Draw the pointer into it.
    pub pointer: bool,
    /// A window's soft shadow around it.
    pub shadow: bool,
}

/// What was taken: its pixels (RGBA, rows from the top) and where it goes.
pub struct Taken {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub path: PathBuf,
}

impl Server {
    /// Asks for a capture on the next frame; the files it will be written
    /// to, or why not.
    pub fn capture_ask(&mut self, req: Request) -> Result<Vec<PathBuf>, String> {
        if self.lock.on {
            return Err("the screen is locked".into());
        }
        if crate::greeter() {
            return Err("not at the login screen".into());
        }
        if self.backend.is_none() {
            return Err("no screens to take".into());
        }
        let screens: Vec<String> = match &req.what {
            What::Screens => self.space.outputs().map(|o| o.name()).collect(),
            _ => vec![String::new()],
        };
        let paths: Vec<PathBuf> = match &req.dest {
            // One picture to the clipboard (every screen: the first).
            Dest::Clipboard => vec![PathBuf::new()],
            Dest::Path(p) => screens.iter().map(|s| if s.is_empty() || screens.len() == 1 { p.clone() } else { with_suffix(p, s) }).collect(),
            Dest::Folder => {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
                let (l, _) = vergence_shell::tiles::now_at(now).ok_or("no local time")?;
                let dir = folder(&l);
                let stem = format!("Screenshot {:04}-{:02}-{:02} at {:02}.{:02}.{:02}", l.year, l.month + 1, l.mday, l.hour, l.min, l.sec);
                // Names given lately count as taken: their files may still be
                // being written.
                screens
                    .iter()
                    .map(|s| {
                        let stem = if s.is_empty() || screens.len() == 1 { stem.clone() } else { format!("{stem} ({s})") };
                        let p = free_name(&dir, &stem, &self.capture_given);
                        self.capture_given.push(p.clone());
                        p
                    })
                    .collect()
            }
        };
        let n = self.capture_given.len();
        if n > 64 {
            self.capture_given.drain(..n - 64);
        }
        self.capture_pending.push((req, paths.clone()));
        self.redraw_all();
        Ok(paths)
    }
}

/// ~/Pictures/Screenshots/<date>.
pub fn folder(l: &vergence_shell::tiles::Local) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join("Pictures/Screenshots").join(format!("{:04}-{:02}-{:02}", l.year, l.month + 1, l.mday))
}

/// `<stem>.png` in `dir`, or `<stem> (2).png` and on if taken (on disk or
/// by a capture still waiting).
fn free_name(dir: &std::path::Path, stem: &str, waiting: &[PathBuf]) -> PathBuf {
    for n in 1.. {
        let name = if n == 1 { format!("{stem}.png") } else { format!("{stem} ({n}).png") };
        let p = dir.join(name);
        if !p.exists() && !waiting.contains(&p) {
            return p;
        }
    }
    unreachable!()
}

fn with_suffix(p: &std::path::Path, s: &str) -> PathBuf {
    let stem = p.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
    p.with_file_name(format!("{stem} ({s}).png"))
}

/// A capture as PNG bytes (for the clipboard).
pub fn png_bytes(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(rgba)).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Writes a capture's PNG off the main thread: the folder made 0700 if new,
/// the file 0600, written beside it and moved into place.
pub fn save(t: Taken, told: impl FnOnce(Result<PathBuf, String>) + Send + 'static) {
    std::thread::spawn(move || {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let res = (|| -> Result<PathBuf, Box<dyn std::error::Error>> {
            let dir = t.path.parent().ok_or("no folder")?;
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
            let tmp = t.path.with_extension("png.part");
            let file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), t.w, t.h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header()?.write_image_data(&t.rgba)?;
            std::fs::rename(&tmp, &t.path)?;
            Ok(t.path.clone())
        })();
        told(res.map_err(|e| e.to_string()));
    });
}

/// Pastes `src` (sw x sh, RGBA) into `dst` (dw wide) at (x, y), enlarged by
/// `k` (1 for one to one; more for a less dense screen's part of an area),
/// smoothly. Pixels outside `dst` are skipped.
#[allow(clippy::too_many_arguments)]
pub fn paste(dst: &mut [u8], dw: usize, dh: usize, src: &[u8], sw: usize, sh: usize, x: i64, y: i64, k: f64) {
    let (tw, th) = ((sw as f64 * k).round() as i64, (sh as f64 * k).round() as i64);
    for ty in 0..th {
        let dy = y + ty;
        if dy < 0 || dy >= dh as i64 {
            continue;
        }
        for tx in 0..tw {
            let dx = x + tx;
            if dx < 0 || dx >= dw as i64 {
                continue;
            }
            let px = if (k - 1.0).abs() < 1e-9 {
                let o = (ty as usize * sw + tx as usize) * 4;
                [src[o], src[o + 1], src[o + 2], src[o + 3]]
            } else {
                // Bilinear, from the pixel centers.
                let fx = ((tx as f64 + 0.5) / k - 0.5).clamp(0.0, (sw - 1) as f64);
                let fy = ((ty as f64 + 0.5) / k - 0.5).clamp(0.0, (sh - 1) as f64);
                let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
                let (x1, y1) = ((x0 + 1).min(sw - 1), (y0 + 1).min(sh - 1));
                let (ax, ay) = (fx - x0 as f64, fy - y0 as f64);
                let at = |xx: usize, yy: usize, c: usize| src[(yy * sw + xx) * 4 + c] as f64;
                let mut out = [0u8; 4];
                for (c, v) in out.iter_mut().enumerate() {
                    let top = at(x0, y0, c) * (1.0 - ax) + at(x1, y0, c) * ax;
                    let bot = at(x0, y1, c) * (1.0 - ax) + at(x1, y1, c) * ax;
                    *v = (top * (1.0 - ay) + bot * ay).round() as u8;
                }
                out
            };
            let o = (dy as usize * dw + dx as usize) * 4;
            dst[o..o + 4].copy_from_slice(&px);
        }
    }
}

/// A window's picture (RGBA, alpha multiplied in) with a soft shadow under
/// it: a margin of `r` pixels around, the shadow its shape blurred and
/// moved down a little, at most 40% dark. Returns the new size and pixels.
pub fn with_shadow(w: usize, h: usize, rgba: &[u8], r: usize) -> (usize, usize, Vec<u8>) {
    let (nw, nh) = (w + 2 * r, h + 2 * r);
    let drop = r / 4;
    // The window's alpha, placed in the margin and moved down.
    let mut a = vec![0f32; nw * nh];
    for y in 0..h {
        for x in 0..w {
            let yy = y + r + drop;
            if yy < nh {
                a[yy * nw + x + r] = rgba[(y * w + x) * 4 + 3] as f32 / 255.0;
            }
        }
    }
    // Three box blurs, each way, about a Gaussian of radius r/2.
    let rad = (r / 3).max(1);
    for _ in 0..3 {
        a = box_blur(&a, nw, nh, rad, true);
        a = box_blur(&a, nw, nh, rad, false);
    }
    let mut out = vec![0u8; nw * nh * 4];
    for (i, v) in a.iter().enumerate() {
        out[i * 4 + 3] = (v * 0.4 * 255.0).round().min(255.0) as u8;
    }
    // The window over its shadow (both alpha multiplied in: over is
    // src + dst * (1 - src alpha)).
    for y in 0..h {
        for x in 0..w {
            let s = &rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
            let o = ((y + r) * nw + x + r) * 4;
            let ia = 1.0 - s[3] as f32 / 255.0;
            for c in 0..4 {
                out[o + c] = (s[c] as f32 + out[o + c] as f32 * ia).round().min(255.0) as u8;
            }
        }
    }
    (nw, nh, out)
}

fn box_blur(a: &[f32], w: usize, h: usize, r: usize, across: bool) -> Vec<f32> {
    let mut out = vec![0f32; a.len()];
    let (n, lines) = if across { (w, h) } else { (h, w) };
    let at = |line: usize, i: usize| if across { line * w + i } else { i * w + line };
    let span = (2 * r + 1) as f32;
    for line in 0..lines {
        let mut sum = 0f32;
        for i in 0..=r.min(n - 1) {
            sum += a[at(line, i)];
        }
        for i in 0..n {
            out[at(line, i)] = sum / span;
            if i + r + 1 < n {
                sum += a[at(line, i + r + 1)];
            }
            if i >= r {
                sum -= a[at(line, i - r)];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_count_up_when_taken() {
        let dir = std::env::temp_dir().join(format!("vergence-capture-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = free_name(&dir, "Screenshot 2026-11-20 at 23.01.19", &[]);
        assert_eq!(first.file_name().unwrap(), "Screenshot 2026-11-20 at 23.01.19.png");
        let second = free_name(&dir, "Screenshot 2026-11-20 at 23.01.19", std::slice::from_ref(&first));
        assert_eq!(second.file_name().unwrap(), "Screenshot 2026-11-20 at 23.01.19 (2).png");
        std::fs::write(&second, b"x").unwrap();
        let third = free_name(&dir, "Screenshot 2026-11-20 at 23.01.19", std::slice::from_ref(&first));
        assert_eq!(third.file_name().unwrap(), "Screenshot 2026-11-20 at 23.01.19 (3).png");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn links_come_back_as_their_paths() {
        let p = std::path::Path::new("/home/d/Pictures/Screenshots/2026-11-20/Screenshot 2026-11-20 at 23.07.44 (2).png");
        let link = file_link(p);
        assert!(!link.contains(' '));
        assert_eq!(vergence_types::path_of_link(&link).as_deref(), Some(p));
    }

    #[test]
    fn a_thumbnail_fits_and_has_its_frame_and_shadow() {
        let rgba: Vec<u8> = std::iter::repeat_n([200u8, 200, 200, 255], 1920 * 1080).flatten().collect();
        let (w, h, px) = thumb_pixels(1920, 1080, &rgba, 2.0);
        // 400x225 picture (200 pt wide at 2x), a 6 px frame, a 20 px shadow each side.
        assert_eq!((w, h), (400 + 12 + 40, 225 + 12 + 40));
        assert_eq!(px.len(), w * h * 4);
        // The frame is white, the picture its own gray.
        assert_eq!(&px[((20 + 2) * w + 20 + 100) * 4..((20 + 2) * w + 20 + 100) * 4 + 3], &[255, 255, 255]);
        assert_eq!(px[((20 + 100) * w + 20 + 100) * 4], 200);
    }

    #[test]
    fn the_folder_is_the_day() {
        let l = vergence_shell::tiles::Local { year: 2026, month: 10, mday: 20, wday: 5, hour: 23, min: 1, sec: 19 };
        assert!(folder(&l).ends_with("Pictures/Screenshots/2026-11-20"));
    }

    #[test]
    fn pasting_one_to_one_and_enlarged() {
        let src: Vec<u8> = (0..4u8).flat_map(|i| [i * 60, 0, 0, 255]).collect(); // 2x2
        let mut dst = vec![0u8; 4 * 4 * 4];
        paste(&mut dst, 4, 4, &src, 2, 2, 1, 1, 1.0);
        assert_eq!(&dst[(4 + 1) * 4..(4 + 1) * 4 + 4], &[0, 0, 0, 255]);
        assert_eq!(&dst[(2 * 4 + 2) * 4..(2 * 4 + 2) * 4 + 4], &[180, 0, 0, 255]);
        let mut big = vec![0u8; 4 * 4 * 4];
        paste(&mut big, 4, 4, &src, 2, 2, 0, 0, 2.0);
        // Corners keep their colors; every pixel is covered.
        assert_eq!(big[0], 0);
        assert_eq!(big[(3 * 4 + 3) * 4], 180);
        assert!(big.chunks(4).all(|p| p[3] == 255));
    }

    #[test]
    fn a_shadow_goes_around_and_below() {
        let (w, h) = (20, 10);
        let rgba = vec![255u8; w * h * 4];
        let (nw, nh, out) = with_shadow(w, h, &rgba, 12);
        assert_eq!((nw, nh), (44, 34));
        // The window itself unchanged in the middle.
        assert_eq!(&out[((12 + 5) * nw + 12 + 10) * 4..((12 + 5) * nw + 12 + 10) * 4 + 4], &[255, 255, 255, 255]);
        // Darker below than above, nothing at the far corners.
        let alpha = |x: usize, y: usize| out[(y * nw + x) * 4 + 3];
        assert!(alpha(22, nh - 6) > alpha(22, 5), "{} {}", alpha(22, nh - 6), alpha(22, 5));
        assert_eq!(alpha(0, 0), 0);
    }
}

// Step 2: the keys and the picker. Super+Shift+3 takes the screen under the
// pointer; Super+Shift+4 opens the picker over every screen: a crosshair,
// a drag takes the area dragged out (its size in points beside it while
// dragging), Space turns it into choosing a window (the window under the
// pointer lit; a click takes it, Alt+click without its shadow) and back,
// Escape lets go. Ctrl with either: to the clipboard (step 3; a file until
// then). Super+Shift+5's bar is step 5.

/// The picker while it is open.
pub struct Pick {
    pub clip: bool,
    /// Choosing a window (Space) rather than dragging out an area.
    pub window: bool,
    /// Where the drag started, and where the pointer is.
    pub start: Option<smithay::utils::Point<f64, Logical>>,
    pub now: smithay::utils::Point<f64, Logical>,
    /// Its overlay's picture, by what it shows.
    picture: Option<(u64, smithay::backend::renderer::element::memory::MemoryRenderBuffer, (i32, i32))>,
}

impl Server {
    /// Super+Shift+3, 4 or 5 (vergence-seat KeyOut::Capture).
    pub fn capture_key(&mut self, key: vergence_seat::keyboard::CaptureKey, clip: bool) {
        use vergence_seat::keyboard::CaptureKey;
        match key {
            CaptureKey::Screen => {
                let what = if self.capture_every_screen() { What::Screens } else { What::Screen(None) };
                self.capture_now(what, clip);
            }
            CaptureKey::Area => {
                if self.lock.on || crate::greeter() {
                    return;
                }
                let now = self.pointer_location();
                self.capture_pick = Some(Pick { clip, window: false, start: None, now, picture: None });
                self.redraw_all();
            }
            // The screenshot bar comes with step 5; until then, as Super+Shift+4.
            CaptureKey::Bar => self.capture_key(CaptureKey::Area, clip),
        }
    }

    /// Settings > Screenshots: Super+Shift+3 takes every screen (step 8;
    /// desktop.toml `[screenshots] every-screen`).
    fn capture_every_screen(&self) -> bool {
        false
    }

    fn capture_now(&mut self, what: What, clip: bool) {
        let req = Request { what, dest: if clip { Dest::Clipboard } else { Dest::Folder }, pointer: false, shadow: true };
        if let Err(e) = self.capture_ask(req) {
            tracing::info!("screenshot: {e}");
        }
    }

    pub fn capture_picking(&self) -> bool {
        self.capture_pick.is_some()
    }

    /// A key while the picker is open: Space switches area and window,
    /// Escape lets go.
    pub fn capture_pick_key(&mut self, sym: u32) {
        use xkbcommon::xkb::keysyms as k;
        let Some(p) = self.capture_pick.as_mut() else { return };
        match sym {
            k::KEY_Escape => self.capture_pick = None,
            k::KEY_space if p.start.is_none() => p.window = !p.window,
            _ => return,
        }
        self.redraw_all();
    }

    pub fn capture_pick_hover(&mut self, pos: smithay::utils::Point<f64, Logical>) {
        if let Some(p) = self.capture_pick.as_mut() {
            p.now = pos;
        }
    }

    /// The window a click would take: the front one under the pointer
    /// (not the desktop's icons).
    pub fn capture_pick_window(&self) -> Option<Win> {
        let p = self.capture_pick.as_ref()?;
        // Front first; its bar counts as the window.
        self.space.elements().rev().find(|w| !self.is_desktop(w) && !self.is_minimized(w) && self.space.element_geometry(w).is_some_and(|g| {
            let bar = if w.native().is_some_and(|n| n.inset() > 0) { 0 } else { self.bar_h(w) };
            Rectangle::new((g.loc.x, g.loc.y - bar).into(), (g.size.w, g.size.h + bar).into()).to_f64().contains(p.now)
        })).cloned()
    }

    pub fn capture_pick_press(&mut self, pos: smithay::utils::Point<f64, Logical>) {
        let Some(p) = self.capture_pick.as_mut() else { return };
        p.now = pos;
        if !p.window {
            p.start = Some(pos);
            return;
        }
        let clip = p.clip;
        let alt = self.seat.get_keyboard().is_some_and(|k| k.modifier_state().alt);
        let w = self.capture_pick_window();
        self.capture_pick = None;
        self.redraw_all();
        if let Some(w) = w {
            let req = Request { what: What::Window(w), dest: if clip { Dest::Clipboard } else { Dest::Folder }, pointer: false, shadow: !alt };
            if let Err(e) = self.capture_ask(req) {
                tracing::info!("screenshot: {e}");
            }
        }
    }

    pub fn capture_pick_release(&mut self, pos: smithay::utils::Point<f64, Logical>) {
        let Some(p) = self.capture_pick.as_ref() else { return };
        let Some(start) = p.start else { return };
        let clip = p.clip;
        self.capture_pick = None;
        self.redraw_all();
        let r = drag_rect(start, pos);
        // A click without a drag takes nothing (as the Mac).
        if r.size.w >= 3 && r.size.h >= 3 {
            self.capture_now(What::Area(r), clip);
        }
    }

    /// The picker's overlay on screen `out`: the area being dragged out with
    /// its size, or the window a click would take, lit.
    pub fn capture_pick_element(&mut self, renderer: &mut smithay::backend::renderer::gles::GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Option<crate::decor::SceneElement> {
        use smithay::backend::renderer::element::{
            Kind,
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
        };
        let p = self.capture_pick.as_ref()?;
        let (r, label) = if p.window {
            let w = self.capture_pick_window()?;
            // Its bar too (drawn above its geometry by the server).
            let g = self.space.element_geometry(&w)?;
            let bar = if w.native().is_some_and(|n| n.inset() > 0) { 0 } else { self.bar_h(&w) };
            (Rectangle::new((g.loc.x, g.loc.y - bar).into(), (g.size.w, g.size.h + bar).into()), None)
        } else {
            let start = p.start?;
            let r = drag_rect(start, p.now);
            (r, Some(format!("{} \u{00d7} {}", r.size.w, r.size.h)))
        };
        let clipped = r.intersection(out)?;
        let a = self.accent();
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (clipped.loc.x, clipped.loc.y, clipped.size.w, clipped.size.h, r.size.w, r.size.h, scale.to_bits(), p.window, (a.r, a.g, a.b)).hash(&mut h);
            h.finish()
        };
        let (w, h) = (clipped.size.w as f32, clipped.size.h as f32);
        if self.capture_pick.as_ref().and_then(|p| p.picture.as_ref()).is_none_or(|(k, ..)| *k != key) {
            let mut scene = vergence_scene::Scene::new(w, h);
            let rect = vergence_scene::Rect::new(0.0, 0.0, w, h);
            if p.window {
                scene.fill(rect, vergence_scene::Color::rgba(a.r, a.g, a.b, 70));
            } else {
                scene.fill(rect, vergence_scene::Color::rgba(255, 255, 255, 40));
                scene.stroke(rect, 0.0, 1.0, vergence_scene::Color::rgba(255, 255, 255, 200));
            }
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            // The size, inside the area's lower right corner.
            if let (Some(label), Some(face)) = (label, fonts.face("sans-serif", 500, false)) {
                let size = 12.0;
                let (_, tw) = fonts.layout(face, size, &label);
                let m = fonts.metrics(face, size);
                let (cw, ch) = (tw + 12.0, m.ascent + m.descent + 6.0);
                if cw + 4.0 < w && ch + 4.0 < h {
                    let card = vergence_scene::Rect::new((w - cw - 4.0).round(), (h - ch - 4.0).round(), cw, ch);
                    scene.fill_rounded(card, 4.0, vergence_scene::Color::rgba(0, 0, 0, 160));
                    vergence_render::text(&mut scene, fonts, face, size, vergence_scene::Color::rgb(255, 255, 255), vergence_scene::Point::new(card.x + 6.0, (card.y + 3.0 + m.ascent).round()), &label);
                }
            }
            let pic = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, vergence_scene::Color::rgba(0, 0, 0, 0)).ok()?;
            let buffer = MemoryRenderBuffer::from_slice(&pic.data, smithay::backend::allocator::Fourcc::Abgr8888, (pic.width as i32, pic.height as i32), 1, smithay::utils::Transform::Normal, None);
            self.capture_pick.as_mut()?.picture = Some((key, buffer, (pic.width as i32, pic.height as i32)));
        }
        let (_, buffer, px) = self.capture_pick.as_ref()?.picture.as_ref()?;
        let whole = Rectangle::<f64, Logical>::from_size((px.0 as f64, px.1 as f64).into());
        let loc = (clipped.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        MemoryRenderBufferRenderElement::from_buffer(renderer, loc.to_f64(), buffer, None, Some(whole), Some(clipped.size), Kind::Unspecified).ok().map(crate::decor::SceneElement::Memory)
    }
}

/// The rectangle between two corners, in whole points.
fn drag_rect(a: smithay::utils::Point<f64, Logical>, b: smithay::utils::Point<f64, Logical>) -> Rectangle<i32, Logical> {
    let (x0, y0) = (a.x.min(b.x).round() as i32, a.y.min(b.y).round() as i32);
    let (x1, y1) = (a.x.max(b.x).round() as i32, a.y.max(b.y).round() as i32);
    Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into())
}

// Step 4: the floating thumbnail (Dave, 2026-11-20: as the Mac's, "anchor
// it to Main"). After each capture a small picture of it floats in the
// bottom-right corner of the main screen's work area (the dock's screen),
// whatever was captured, in front of everything: about 5 s, kept while the
// pointer is over it; a click opens the file in Close-Up; a drag takes the
// file to any app that takes files (the server's own drag, drag.rs); a
// flick to the right sends it away. A capture to the clipboard shows it
// too, with nothing to open or drag. It is not a notification: Do Not
// Disturb and the cards' settings don't touch it. The next capture's
// thumbnail takes its place.

/// A path as a file:// link, every byte but the unreserved ones and '/'
/// escaped (a screenshot's name has spaces).
pub fn file_link(p: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::from("file://");
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// The longest side of the thumbnail's picture, in points.
const SHOT_W: f64 = 200.0;
const SHOT_H: f64 = 130.0;
/// Its white frame and shadow, and its margin from the corner, in points.
const SHOT_FRAME: f64 = 3.0;
const SHOT_SHADOW: f64 = 10.0;
const SHOT_MARGIN: i32 = 12;
/// How long it floats.
const SHOT_TIME: std::time::Duration = std::time::Duration::from_secs(5);

/// The thumbnail while it floats.
pub struct Shot {
    /// The file (None: it went to the clipboard).
    pub path: Option<PathBuf>,
    buffer: smithay::backend::renderer::element::memory::MemoryRenderBuffer,
    px: (i32, i32),
    /// Where it is, in points (shadow included).
    pub rect: Rectangle<i32, Logical>,
    until: std::time::Instant,
    hovered: bool,
    /// Where a press on it started.
    press: Option<smithay::utils::Point<f64, Logical>>,
}

/// A capture made small for its thumbnail, at `scale` (the main screen's):
/// averaged down to fit SHOT_W x SHOT_H points, a white frame, a shadow.
pub fn thumb_pixels(w: usize, h: usize, rgba: &[u8], scale: f64) -> (usize, usize, Vec<u8>) {
    let k = ((SHOT_W * scale) / w as f64).min((SHOT_H * scale) / h as f64).min(1.0);
    let (tw, th) = (((w as f64 * k).round() as usize).max(1), ((h as f64 * k).round() as usize).max(1));
    let small = shrink(w, h, rgba, tw, th);
    let f = (SHOT_FRAME * scale).round() as usize;
    let (fw, fh) = (tw + 2 * f, th + 2 * f);
    let mut framed = vec![255u8; fw * fh * 4];
    for y in 0..th {
        for x in 0..tw {
            let s = &small[(y * tw + x) * 4..(y * tw + x) * 4 + 4];
            let o = ((y + f) * fw + x + f) * 4;
            // Over white (a window's clear corners show white, as framed).
            let ia = 255 - s[3] as u32;
            for c in 0..3 {
                framed[o + c] = (s[c] as u32 + 255 * ia / 255).min(255) as u8;
            }
            framed[o + 3] = 255;
        }
    }
    with_shadow(fw, fh, &framed, (SHOT_SHADOW * scale).round() as usize)
}

/// Area-averaged shrinking (premultiplied pixels average right).
fn shrink(w: usize, h: usize, rgba: &[u8], tw: usize, th: usize) -> Vec<u8> {
    if (tw, th) == (w, h) {
        return rgba.to_vec();
    }
    let mut out = vec![0u8; tw * th * 4];
    for ty in 0..th {
        let (y0, y1) = (ty * h / th, ((ty + 1) * h / th).max(ty * h / th + 1).min(h));
        for tx in 0..tw {
            let (x0, x1) = (tx * w / tw, ((tx + 1) * w / tw).max(tx * w / tw + 1).min(w));
            let mut sum = [0u64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let o = (y * w + x) * 4;
                    for c in 0..4 {
                        sum[c] += rgba[o + c] as u64;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            let o = (ty * tw + tx) * 4;
            for c in 0..4 {
                out[o + c] = (sum[c] / n) as u8;
            }
        }
    }
    out
}

impl Server {
    /// The main screen and its scale, for the thumbnail.
    pub fn shot_screen(&self) -> Option<(Rectangle<i32, Logical>, f64)> {
        let geo = self.shelf_screen().or_else(|| self.space.outputs().next().and_then(|o| self.space.output_geometry(o)))?;
        let scale = self.space.outputs().find(|o| self.space.output_geometry(o) == Some(geo)).map_or(1.0, |o| o.current_scale().fractional_scale());
        Some((geo, scale))
    }

    /// A capture was saved (or put on the clipboard): its thumbnail floats.
    pub fn shot_show(&mut self, path: Option<PathBuf>, w: usize, h: usize, rgba: Vec<u8>, scale: f64) {
        use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
        let Some((geo, _)) = self.shot_screen() else { return };
        let area = self.work_area(geo);
        let (pw, ph) = ((w as f64 / scale).round() as i32, (h as f64 / scale).round() as i32);
        // The shadow's margin sits over the margin from the corner.
        let shadow = (SHOT_SHADOW).round() as i32;
        let x = area.loc.x + area.size.w - pw - SHOT_MARGIN + shadow;
        let y = area.loc.y + area.size.h - ph - SHOT_MARGIN + shadow;
        let buffer = MemoryRenderBuffer::from_slice(&rgba, smithay::backend::allocator::Fourcc::Abgr8888, (w as i32, h as i32), 1, smithay::utils::Transform::Normal, None);
        self.capture_shot = Some(Shot { path, buffer, px: (w as i32, h as i32), rect: Rectangle::new((x, y).into(), (pw, ph).into()), until: std::time::Instant::now() + SHOT_TIME, hovered: false, press: None });
        self.shot_timer();
        self.redraw_all();
    }

    /// Looks again when it is due to go; kept while hovered or held.
    fn shot_timer(&mut self) {
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        let _ = self.loop_handle.insert_source(Timer::from_duration(SHOT_TIME + std::time::Duration::from_millis(50)), |_, _, st| {
            let now = std::time::Instant::now();
            match st.capture_shot.as_mut() {
                Some(s) if s.hovered || s.press.is_some() => {
                    s.until = now + std::time::Duration::from_secs(2);
                    return TimeoutAction::ToDuration(std::time::Duration::from_secs(2));
                }
                Some(s) if s.until > now => return TimeoutAction::ToDuration(s.until - now),
                Some(_) => {
                    st.capture_shot = None;
                    st.redraw_all();
                }
                None => {}
            }
            TimeoutAction::Drop
        });
    }

    pub fn shot_at(&self, pos: smithay::utils::Point<f64, Logical>) -> bool {
        self.capture_shot.as_ref().is_some_and(|s| s.rect.to_f64().contains(pos))
    }

    /// The pointer moved: hovered or not; held and moved, a drag starts (or
    /// a flick to the right sends it away).
    pub fn shot_hover(&mut self, pos: smithay::utils::Point<f64, Logical>) {
        let Some(s) = self.capture_shot.as_mut() else { return };
        s.hovered = s.rect.to_f64().contains(pos);
        let Some(from) = s.press else { return };
        let (dx, dy) = (pos.x - from.x, pos.y - from.y);
        if dx > 30.0 && dy.abs() < dx / 2.0 {
            // Flicked away.
            self.capture_shot = None;
            self.redraw_all();
        } else if dx.hypot(dy) > 8.0 && dx <= 0.0 {
            let path = s.path.clone();
            s.press = None;
            if let Some(p) = path {
                self.capture_shot = None;
                let uri = format!("{}\r\n", file_link(&p));
                let label = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let plain = p.to_string_lossy().into_owned().into_bytes();
                self.server_drag_start(label, vec![("text/uri-list".into(), uri.into_bytes()), ("text/plain".into(), plain)]);
                self.redraw_all();
            }
        }
    }

    pub fn shot_press(&mut self, pos: smithay::utils::Point<f64, Logical>) {
        if let Some(s) = self.capture_shot.as_mut() {
            s.press = Some(pos);
        }
    }

    /// Let go without a drag: a click opens it in Close-Up.
    pub fn shot_release(&mut self, pos: smithay::utils::Point<f64, Logical>) {
        let Some(s) = self.capture_shot.as_mut() else { return };
        let Some(from) = s.press.take() else { return };
        if (pos.x - from.x).hypot(pos.y - from.y) > 8.0 {
            return;
        }
        let Some(path) = s.path.clone() else { return };
        self.capture_shot = None;
        self.redraw_all();
        if let Err(e) = crate::spawn(std::process::Command::new("vergence-closeup").arg(&path)) {
            tracing::warn!("screenshot: Close-Up did not start: {e}");
        }
    }

    /// The thumbnail on screen `out` (the main screen's).
    pub fn shot_element(&mut self, renderer: &mut smithay::backend::renderer::gles::GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Option<crate::decor::SceneElement> {
        use smithay::backend::renderer::element::{Kind, memory::MemoryRenderBufferRenderElement};
        let s = self.capture_shot.as_ref()?;
        if !s.rect.overlaps(out) {
            return None;
        }
        let whole = Rectangle::<f64, Logical>::from_size((s.px.0 as f64, s.px.1 as f64).into());
        let loc = (s.rect.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        MemoryRenderBufferRenderElement::from_buffer(renderer, loc.to_f64(), &s.buffer, None, Some(whole), Some(s.rect.size), Kind::Unspecified).ok().map(crate::decor::SceneElement::Memory)
    }
}
