// `vergence-harness zoomed`: is a shot the magnifier's picture of another?
// (Meaning milestone 1, step 9.) The magnifier scales the screen about its
// point of interest P by L, so the pixel at q in the zoomed shot shows what
// was at P + (q - P) / L in the plain one. Compared where the plain shot is
// flat around that place (edges are smoothed differently when scaled), and
// passed when almost every such pixel agrees.
//
//     vergence-harness zoomed <plain.png> <zoomed.png> <px> <py> <level> [--tolerance T]
//
// P in the shots' pixels. Prints how many compared pixels differ by more
// than T (default 10) on any channel; exits 1 if more than 0.5% do, or
// fewer than 10,000 could be compared.

use crate::same::{Pic, load};

fn flat(p: &Pic, x: i32, y: i32) -> Option<[u8; 4]> {
    let c = p.at(x, y)?;
    for dy in -2..=2 {
        for dx in -2..=2 {
            let o = p.at(x + dx, y + dy)?;
            if (0..3).any(|k| (o[k] as i32 - c[k] as i32).abs() > 2) {
                return None;
            }
        }
    }
    Some(c)
}

/// (compared, differ, first difference).
pub fn compare(
    plain: &Pic,
    zoomed: &Pic,
    p: (f64, f64),
    level: f64,
    tolerance: u8,
) -> (usize, usize, Option<(i32, i32)>) {
    let (mut compared, mut differ, mut first) = (0, 0, None);
    for y in 0..zoomed.h {
        for x in 0..zoomed.w {
            let sx = p.0 + (x as f64 + 0.5 - p.0) / level - 0.5;
            let sy = p.1 + (y as f64 + 0.5 - p.1) / level - 0.5;
            let Some(want) = flat(plain, sx.round() as i32, sy.round() as i32) else { continue };
            let Some(got) = zoomed.at(x, y) else { continue };
            compared += 1;
            if (0..3).any(|k| (got[k] as i32 - want[k] as i32).unsigned_abs() > tolerance as u32) {
                differ += 1;
                first.get_or_insert((x, y));
            }
        }
    }
    (compared, differ, first)
}

pub fn run(args: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    let usage = "zoomed <plain.png> <zoomed.png> <px> <py> <level> [--tolerance T]";
    let (Some(pa), Some(pb)) = (args.first(), args.get(1)) else { return Err(usage.into()) };
    let num = |i: usize| -> Result<f64, Box<dyn std::error::Error>> {
        Ok(args.get(i).ok_or(usage)?.parse::<f64>().map_err(|_| usage)?)
    };
    let (px, py, level) = (num(2)?, num(3)?, num(4)?);
    let tolerance = args
        .iter()
        .position(|a| a == "--tolerance")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(10u8);
    let (plain, zoomed) = (load(pa)?, load(pb)?);
    if plain.w != zoomed.w || plain.h != zoomed.h || level <= 1.0 {
        println!("FAIL the shots differ in size, or the level is not above 1");
        return Ok(false);
    }
    let (compared, differ, first) = compare(&plain, &zoomed, (px, py), level, tolerance);
    let ok = compared >= 10_000 && (differ as f64) <= compared as f64 * 0.005;
    let first = first.map_or(String::new(), |(x, y)| format!("; first at {x},{y}"));
    println!(
        "{} zoomed {level} about {px},{py}: {differ} of {compared} flat pixels differ by more than {tolerance}{first}",
        if ok { "PASS" } else { "FAIL" }
    );
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
    fn a_picture_scaled_about_a_point_is_its_zoom() {
        // Squares of 40 pixels in two colors.
        let square = |x: i32, y: i32| if (x / 40 + y / 40) % 2 == 0 { [200, 40, 160, 255] } else { [20, 20, 20, 255] };
        let plain = pic(400, 300, square);
        let (p, l) = ((130.0, 90.0), 2.0);
        let zoomed = pic(400, 300, |x, y| {
            let sx = (p.0 + (x as f64 + 0.5 - p.0) / l - 0.5).round() as i32;
            let sy = (p.1 + (y as f64 + 0.5 - p.1) / l - 0.5).round() as i32;
            square(sx, sy)
        });
        let (c, d, _) = compare(&plain, &zoomed, p, l, 10);
        assert!(c > 50_000 && d == 0, "{c} {d}");
        let (_, wrong, _) = compare(&plain, &zoomed, (200.0, 150.0), l, 10);
        assert!(wrong > c / 10, "about another point it is not: {wrong}");
        let (_, unscaled, _) = compare(&plain, &plain, p, l, 10);
        assert!(unscaled > c / 10, "the plain picture is not its own zoom: {unscaled}");
    }
}
