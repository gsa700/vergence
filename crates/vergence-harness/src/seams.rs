// `vergence-harness seams`: is a test-pattern window (vergence-ui's
// testpattern example) shown whole, on the pixels it should be? Testing
// method, layer 2 (2026-11-18), for tools/display-check.sh.
//
//     vergence-harness seams <shot.png> --scale S --window X Y W H [--bar H]
//
// X Y W H are the window's body in points from the screen's corner (as
// `vctl windows` gives them, less the screen's place), S the screen's
// scale. Each edge is where Smithay puts it (rounded on its own,
// decor.rs edges_px). Checked, away from the rounded corners:
//   - the pattern's one-pixel lines (top red, bottom blue, left yellow,
//     right cyan) are on the first and last rows and columns: the picture
//     was shown whole, not cropped or squeezed;
//   - the body's magenta is just inside them;
//   - with --bar, the bar's first and last columns match each other (its
//     outline on both sides, or on neither), the seam of 2026-11-18.
// Prints one line per check and exits 1 if any fails.

use std::path::Path;

type Rgb = [u8; 3];

const BODY: Rgb = [200, 40, 160];
const TOP: Rgb = [255, 0, 0];
const BOTTOM: Rgb = [0, 0, 255];
const LEFT: Rgb = [255, 255, 0];
const RIGHT: Rgb = [0, 255, 255];

/// How far a pixel may be from the color it should be (each channel):
/// the screen's color handling and the frost's dithering.
const SLACK: i32 = 12;

struct Shot {
    w: i32,
    h: i32,
    rgba: Vec<u8>,
}

impl Shot {
    fn at(&self, x: i32, y: i32) -> Option<Rgb> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return None;
        }
        let i = (y as usize * self.w as usize + x as usize) * 4;
        Some([self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]])
    }
}

fn near(a: Rgb, b: Rgb, slack: i32) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (*x as i32 - *y as i32).abs() <= slack)
}

/// The pixels a run of points covers: each end rounded on its own.
pub fn edges(at: f64, len: f64, scale: f64) -> (i32, i32) {
    ((at * scale).round() as i32, ((at + len) * scale).round() as i32)
}

struct Checks {
    failed: usize,
}

impl Checks {
    fn check(&mut self, ok: bool, what: &str) {
        println!("{} {what}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            self.failed += 1;
        }
    }
}

/// Of the pixels along a line, how many are `want`, and the first that is not.
fn along(shot: &Shot, points: impl Iterator<Item = (i32, i32)>, want: Rgb) -> (usize, usize, Option<(i32, i32, Rgb)>) {
    let (mut good, mut all, mut first_bad) = (0, 0, None);
    for (x, y) in points {
        all += 1;
        match shot.at(x, y) {
            Some(c) if near(c, want, SLACK) => good += 1,
            Some(c) => {
                first_bad.get_or_insert((x, y, c));
            }
            None => {
                first_bad.get_or_insert((x, y, [0, 0, 0]));
            }
        }
    }
    (good, all, first_bad)
}

pub fn run(args: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    let usage = || "seams <shot.png> --scale S --window X Y W H [--bar H]".to_string();
    let path = args.first().ok_or_else(usage)?;
    let num = |name: &str, k: usize| -> Option<f64> { args.iter().position(|a| a == name).and_then(|i| args.get(i + 1 + k)).and_then(|v| v.parse().ok()) };
    let scale = num("--scale", 0).filter(|s| s.is_finite() && *s > 0.0).ok_or_else(usage)?;
    let (x, y, w, h) = match (num("--window", 0), num("--window", 1), num("--window", 2), num("--window", 3)) {
        (Some(x), Some(y), Some(w), Some(h)) if w > 0.0 && h > 0.0 => (x, y, w, h),
        _ => return Err(usage().into()),
    };
    let bar = num("--bar", 0);
    let data = std::fs::read(Path::new(path)).map_err(|e| format!("{path}: {e}"))?;
    let pic = vergence_decode::decode_bytes(&data, None).map_err(|e| format!("{path}: {e}"))?;
    let frame = pic.frames.into_iter().next().ok_or("no picture")?;
    let shot = Shot { w: pic.width as i32, h: pic.height as i32, rgba: frame.rgba };

    let (l, r) = edges(x, w, scale);
    let (t, b) = edges(y, h, scale);
    println!("window {w}x{h} pt at {x},{y}, scale {scale:.4}: pixels {l}..{r} x {t}..{b} ({} x {})", r - l, b - t);
    // Away from the rounded corners (5 pt, and a little more).
    let inset = (8.0 * scale).ceil() as i32;
    let rows = || (t + inset)..(b - inset);
    let cols = || (l + inset)..(r - inset);
    let mut c = Checks { failed: 0 };
    let mut line = |c: &mut Checks, name: &str, points: Vec<(i32, i32)>, want: Rgb| {
        let (good, all, bad) = along(&shot, points.into_iter(), want);
        let note = bad.map_or(String::new(), |(x, y, c)| format!("; first wrong at {x},{y}: {c:?}"));
        c.check(good == all, &format!("{name}: {good}/{all}{note}"));
    };
    line(&mut c, "top line on the first row", cols().map(|x| (x, t)).collect(), TOP);
    line(&mut c, "bottom line on the last row", cols().map(|x| (x, b - 1)).collect(), BOTTOM);
    line(&mut c, "left line on the first column", rows().map(|y| (l, y)).collect(), LEFT);
    line(&mut c, "right line on the last column", rows().map(|y| (r - 1, y)).collect(), RIGHT);
    line(&mut c, "body inside the top line", cols().map(|x| (x, t + 1)).collect(), BODY);
    line(&mut c, "body inside the bottom line", cols().map(|x| (x, b - 2)).collect(), BODY);
    line(&mut c, "body inside the left line", rows().map(|y| (l + 1, y)).collect(), BODY);
    line(&mut c, "body inside the right line", rows().map(|y| (r - 2, y)).collect(), BODY);
    // Nothing of the pattern outside its pixels (a picture shown too big).
    let outside: Vec<(i32, i32)> = cols().map(|x| (x, b)).chain(rows().map(|y| (r, y))).collect();
    let spill = outside.iter().filter(|(x, y)| shot.at(*x, *y).is_some_and(|p| near(p, BOTTOM, SLACK) || near(p, RIGHT, SLACK) || near(p, BODY, SLACK))).count();
    c.check(spill == 0, &format!("nothing of it past its last row and column: {spill} pixels"));
    if let Some(bar) = bar {
        let (bt, bb) = edges(y - bar, bar, scale);
        c.check(bb == t, &format!("bar ends where the body starts: row {bb} and {t}"));
        // The bar's sides, row by row through its middle: its first and last
        // columns alike, and each unlike the column outside it.
        let mid = (bt + 2)..(bb - 2);
        let mut same = 0;
        let mut all = 0;
        for yy in mid {
            all += 1;
            if let (Some(a), Some(z)) = (shot.at(l, yy), shot.at(r - 1, yy))
                && near(a, z, 24)
            {
                same += 1;
            }
        }
        c.check(all > 0 && same * 10 >= all * 9, &format!("bar's first and last columns alike: {same}/{all} rows"));
    }
    println!("{} of the checks failed", c.failed);
    Ok(c.failed == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: i32, h: i32, f: impl Fn(i32, i32) -> Rgb) -> Shot {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let c = f(x, y);
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        Shot { w, h, rgba }
    }

    #[test]
    fn edges_round_each_end() {
        assert_eq!(edges(63.0, 579.0, 4.0 / 3.0), (84, 856));
        assert_eq!(edges(1.0, 2.0, 4.0 / 3.0), (1, 4));
    }

    #[test]
    fn a_whole_pattern_is_found_and_a_cropped_one_is_not() {
        let (l, r, t, b) = (10, 50, 20, 60);
        let pattern = |crop: i32| {
            picture(80, 80, move |x, y| {
                if x < l || x >= r || y < t || y >= b - crop {
                    [9, 9, 9]
                } else if y == t {
                    TOP
                } else if y == b - 1 {
                    BOTTOM
                } else if x == l {
                    LEFT
                } else if x == r - 1 {
                    RIGHT
                } else {
                    BODY
                }
            })
        };
        let whole = pattern(0);
        assert_eq!(along(&whole, (12..48).map(|x| (x, b - 1)), BOTTOM).0, 36);
        let cropped = pattern(1);
        assert_eq!(along(&cropped, (12..48).map(|x| (x, b - 1)), BOTTOM).0, 0);
    }
}
