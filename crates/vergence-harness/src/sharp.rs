// `vergence-harness sharper`: is one shot's region sharper than another's?
// (Display lists milestone 1, step 7.) The same picture drawn sharp or
// enlarged changes brightness by the same total across an edge, but sharp
// in fewer, larger steps: so the sum of the squared steps over the sum of
// the steps (the typical step across an edge) is larger when sharp.
//
//     vergence-harness sharper <a.png> <b.png> <x> <y> <w> <h> [--by R]
//
// The region in both shots' pixels. Prints each one's measure; exits 1
// unless a's is at least R (default 1.25) times b's, or the region has
// almost no edges.

use crate::same::{Pic, load};

fn luma(c: [u8; 4]) -> f64 {
    0.2126 * c[0] as f64 + 0.7152 * c[1] as f64 + 0.0722 * c[2] as f64
}

/// (the typical step across an edge, the sum of the steps).
pub fn measure(p: &Pic, x: i32, y: i32, w: i32, h: i32) -> (f64, f64) {
    let (mut sum, mut squares) = (0.0, 0.0);
    for yy in y..y + h {
        for xx in x..x + w {
            let Some(c) = p.at(xx, yy) else { continue };
            for (dx, dy) in [(1, 0), (0, 1)] {
                let Some(o) = p.at(xx + dx, yy + dy) else { continue };
                let d = (luma(o) - luma(c)).abs();
                // Below this is noise and gradients, not edges.
                if d > 2.0 {
                    sum += d;
                    squares += d * d;
                }
            }
        }
    }
    (if sum > 0.0 { squares / sum } else { 0.0 }, sum)
}

pub fn run(args: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    let (a, b) = (load(&args[0])?, load(&args[1])?);
    let n: Vec<i32> = args[2..6].iter().map(|s| s.parse()).collect::<Result<_, _>>()?;
    let by = match args.iter().position(|s| s == "--by") {
        Some(i) => args.get(i + 1).ok_or("--by needs a number")?.parse::<f64>()?,
        None => 1.25,
    };
    let (sa, ta) = measure(&a, n[0], n[1], n[2], n[3]);
    let (sb, tb) = measure(&b, n[0], n[1], n[2], n[3]);
    if ta < 1000.0 || tb < 1000.0 {
        println!("FAIL almost no edges in the region ({ta:.0}, {tb:.0})");
        return Ok(false);
    }
    let ok = sa >= by * sb;
    println!(
        "{} sharpness {sa:.1} against {sb:.1} ({:.2}x, {by:.2}x wanted)",
        if ok { "PASS" } else { "FAIL" },
        sa / sb.max(1e-9)
    );
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pic(w: i32, h: i32, f: impl Fn(i32, i32) -> u8) -> Pic {
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = f(x, y);
                data.extend_from_slice(&[v, v, v, 255]);
            }
        }
        Pic { w, h, rgba: data }
    }

    #[test]
    fn a_hard_edge_is_sharper_than_a_soft_one() {
        // Stripes 8 pixels wide: hard, and the same ramped over 4 pixels.
        let hard = pic(64, 64, |x, _| if (x / 8) % 2 == 0 { 0 } else { 255 });
        let soft = pic(64, 64, |x, _| {
            let p = x % 16;
            let v = match p {
                0..=5 => 0.0,
                6..=9 => (p - 5) as f64 / 5.0 * 255.0,
                10..=13 => 255.0,
                _ => (16 - p) as f64 / 3.0 * 255.0,
            };
            v as u8
        });
        let (sh, _) = measure(&hard, 0, 0, 64, 64);
        let (ss, _) = measure(&soft, 0, 0, 64, 64);
        assert!(sh > 2.0 * ss, "{sh} against {ss}");
    }
}
