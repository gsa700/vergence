// `vergence-harness same`: do two pictures hold the same pixels? (Headless
// milestone 1, step 4.) Two uses:
//
//   - what a program drew against what the screen shows: a window's own
//     picture (`vctl picture <i>`) against the region of a shot it is shown
//     in, `--at X Y` its first pixel there. On the pixel grid the server
//     shows a picture one for one, so nothing may differ but the rounded
//     corners (`--corners R`, pixels) and a strip under the bar (`--top N`);
//   - the toolkit on the GPU against its CPU reference: the same program's
//     pictures from a window drawn each way (VERGENCE_RENDER=cpu), compared
//     within the render tests' tolerance (`--tolerance 3`).
//
//     vergence-harness same <a.png> <b.png> [--at X Y] [--corners R] [--top N] [--tolerance T]
//
// `b` is compared whole, against `a` from (X, Y). Prints how many pixels
// differ by more than T on any channel, the largest difference and the
// first such pixel; exits 1 if any do.

use std::path::Path;

pub(crate) struct Pic {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) rgba: Vec<u8>,
}

impl Pic {
    pub(crate) fn at(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return None;
        }
        let i = (y as usize * self.w as usize + x as usize) * 4;
        Some([self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]])
    }
}

pub(crate) fn load(path: &str) -> Result<Pic, Box<dyn std::error::Error>> {
    let data = std::fs::read(Path::new(path)).map_err(|e| format!("{path}: {e}"))?;
    let p = vergence_decode::decode_bytes(&data, None).map_err(|e| format!("{path}: {e}"))?;
    let f = p.frames.into_iter().next().ok_or("no picture")?;
    Ok(Pic { w: p.width as i32, h: p.height as i32, rgba: f.rgba })
}

/// Is (x, y) of a w x h picture inside a rounded corner of radius r?
fn in_corner(x: i32, y: i32, w: i32, h: i32, r: i32) -> bool {
    if r <= 0 {
        return false;
    }
    let near_x = x < r || x >= w - r;
    let near_y = y < r || y >= h - r;
    near_x && near_y
}

pub struct Outcome {
    pub compared: usize,
    pub differ: usize,
    pub largest: u8,
    pub first: Option<(i32, i32, [u8; 4], [u8; 4])>,
}

fn compare(a: &Pic, b: &Pic, at: (i32, i32), corners: i32, top: i32, tolerance: u8) -> Outcome {
    let mut o = Outcome { compared: 0, differ: 0, largest: 0, first: None };
    for y in top.max(0)..b.h {
        for x in 0..b.w {
            if in_corner(x, y, b.w, b.h, corners) {
                continue;
            }
            let pb = b.at(x, y).unwrap_or([0; 4]);
            // Fully clear pixels of the picture show what is behind them.
            if pb[3] == 0 {
                continue;
            }
            o.compared += 1;
            let pa = a.at(at.0 + x, at.1 + y).unwrap_or([0; 4]);
            let d = (0..3).map(|k| (pa[k] as i32 - pb[k] as i32).unsigned_abs() as u8).max().unwrap_or(0);
            o.largest = o.largest.max(d);
            if d > tolerance {
                o.differ += 1;
                o.first.get_or_insert((x, y, pa, pb));
            }
        }
    }
    o
}

pub fn run(args: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    let usage = "same <a.png> <b.png> [--at X Y] [--corners R] [--top N] [--tolerance T]";
    let (Some(pa), Some(pb)) = (args.first(), args.get(1)) else { return Err(usage.into()) };
    let num = |name: &str, k: usize| -> Option<i32> { args.iter().position(|a| a == name).and_then(|i| args.get(i + 1 + k)).and_then(|v| v.parse().ok()) };
    let at = (num("--at", 0).unwrap_or(0), num("--at", 1).unwrap_or(0));
    let corners = num("--corners", 0).unwrap_or(0);
    let top = num("--top", 0).unwrap_or(0);
    let tolerance = num("--tolerance", 0).unwrap_or(0).clamp(0, 255) as u8;
    let a = load(pa)?;
    let b = load(pb)?;
    if at.0 < 0 || at.1 < 0 || at.0 + b.w > a.w || at.1 + b.h > a.h {
        println!("FAIL {pb} ({}x{}) at {},{} is not inside {pa} ({}x{})", b.w, b.h, at.0, at.1, a.w, a.h);
        return Ok(false);
    }
    let o = compare(&a, &b, at, corners, top, tolerance);
    let first = o.first.map_or(String::new(), |(x, y, pa, pb)| format!("; first at {x},{y}: {pa:?} against {pb:?}"));
    let ok = o.differ == 0 && o.compared > 0;
    println!("{} {}x{} at {},{}: {} of {} pixels differ by more than {tolerance} (largest {}){first}", if ok { "PASS" } else { "FAIL" }, b.w, b.h, at.0, at.1, o.differ, o.compared, o.largest);
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pic(w: i32, h: i32, f: impl Fn(i32, i32) -> [u8; 4]) -> Pic {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&f(x, y));
            }
        }
        Pic { w, h, rgba }
    }

    #[test]
    fn a_picture_shown_one_for_one_is_the_same_and_one_pixel_off_is_not() {
        let window = pic(20, 10, |x, y| [(x * 10) as u8, (y * 20) as u8, 7, 255]);
        let screen = pic(60, 40, |x, y| if (5..25).contains(&x) && (8..18).contains(&y) { [((x - 5) * 10) as u8, ((y - 8) * 20) as u8, 7, 255] } else { [1, 1, 1, 255] });
        let o = compare(&screen, &window, (5, 8), 0, 0, 0);
        assert_eq!((o.differ, o.compared), (0, 200));
        let off = compare(&screen, &window, (6, 8), 0, 0, 0);
        assert!(off.differ > 100);
        // Within a tolerance, a near miss passes.
        let near = pic(20, 10, |x, y| [(x * 10) as u8 + 2, (y * 20) as u8, 7, 255]);
        assert_eq!(compare(&screen, &near, (5, 8), 0, 0, 3).differ, 0);
        assert!(compare(&screen, &near, (5, 8), 0, 0, 1).differ > 0);
    }

    #[test]
    fn corners_and_the_top_strip_are_left_out() {
        assert!(in_corner(0, 0, 20, 10, 3) && in_corner(19, 9, 20, 10, 3));
        assert!(!in_corner(10, 0, 20, 10, 3) && !in_corner(0, 5, 20, 10, 3));
        let window = pic(20, 10, |_, _| [9, 9, 9, 255]);
        let screen = pic(20, 10, |x, y| if y < 2 || in_corner(x, y, 20, 10, 3) { [200, 0, 0, 255] } else { [9, 9, 9, 255] });
        assert_eq!(compare(&screen, &window, (0, 0), 3, 2, 0).differ, 0);
    }
}
