// `vergence-harness colors`: how much of a picture is in color? (Edit syntax
// milestone 1, step 8.) Syntax colors make a code file's text colored where
// it was gray or black: the display suite counts the pixels whose channels
// differ by more than 60 (a colored ink, not a gray or the text's own),
// optionally in a region, to tell a colored file from a plain one.
//
//     vergence-harness colors <picture.png> [X Y W H]
//
// Prints "colored N of M luma L" (L: the region's mean brightness, 0 to 255,
// for how much of something bright shows through a dark see-through window;
// frost milestone 1); exits 0 (a count, not a judgment: the suite judges).

use crate::same::load;

pub fn run(args: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    let p = load(&args[0])?;
    let nums: Vec<i32> = args[1..].iter().map(|a| a.parse()).collect::<Result<_, _>>()?;
    let (x0, y0, w, h) = match nums.as_slice() {
        [x, y, w, h] => (*x, *y, *w, *h),
        [] => (0, 0, p.w, p.h),
        _ => return Err("colors: X Y W H, or no region".into()),
    };
    let (mut colored, mut all, mut luma) = (0u64, 0u64, 0f64);
    for y in y0.max(0)..(y0 + h).min(p.h) {
        for x in x0.max(0)..(x0 + w).min(p.w) {
            let Some([r, g, b, _]) = p.at(x, y) else { continue };
            all += 1;
            luma += 0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64;
            if r.max(g).max(b) - r.min(g).min(b) > 60 {
                colored += 1;
            }
        }
    }
    println!("colored {colored} of {all} luma {:.1}", if all > 0 { luma / all as f64 } else { 0.0 });
    Ok(true)
}
