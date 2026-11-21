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
