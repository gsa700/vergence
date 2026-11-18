// Display geometry, checked without a GPU (testing method, layer 1,
// 2026-11-18). A native window on the screen is a picture the toolkit drew
// (vergence_render::pixel_size), its bar a picture the server drew the same
// way, both placed by edges_px and cropped by shown_px (decor.rs), sized and
// put by the grid (grid.rs) and read back by points_of (native.rs). These
// are the rules the seams of 2026-11-17/18 broke; each test walks every
// clean scale and a range of places and sizes.

use smithay::utils::{Logical, Point, Size};

use crate::decor::{edges_px, shown_px};
use crate::grid::{snap_at, snap_len, step_of};
use crate::native::points_of;

/// The slider's clean stops that have a grid, and some that do not.
const GRID_SCALES: [f64; 9] = [1.0, 1.25, 4.0 / 3.0, 1.5, 1.2, 15.0 / 8.0, 2.0, 0.75, 3.0];
const OFF_GRID: [f64; 3] = [173.0 / 120.0, 1.075, 3.2833333333333333];

/// The scale as the server has it: in 120ths (scale_in_120ths).
fn s120(s: f64) -> f64 {
    crate::scale_in_120ths(s)
}

/// The toolkit's picture size for `points` at `scale`.
fn picture_px(points: (i32, i32), scale: f64) -> (i32, i32) {
    let scene = vergence_scene::Scene::new(points.0 as f32, points.1 as f32);
    let (w, h) = vergence_render::pixel_size(&scene, scale as f32).expect("a picture");
    (w as i32, h as i32)
}

fn sizes() -> impl Iterator<Item = (i32, i32)> {
    [(320, 160), (580, 420), (773, 511), (1173, 765), (1001, 999)].into_iter()
}

fn places() -> impl Iterator<Item = (i32, i32)> {
    (0..12).map(|i| (i * 7 + 1, i * 5 + 3)).chain([(2879, 1500), (-17, 40)])
}

/// On the grid, a window's picture is exactly the pixels it covers: nothing
/// cropped, nothing short, wherever it is.
#[test]
fn on_the_grid_a_picture_is_the_pixels_it_covers() {
    for scale in GRID_SCALES.map(s120) {
        let step = step_of(scale);
        for size in sizes() {
            let size: Size<i32, Logical> = (snap_len(size.0, step), snap_len(size.1, step)).into();
            let pic = picture_px((size.w, size.h), scale);
            for (x, y) in places() {
                let at: Point<i32, Logical> = (snap_at(x, 0, step), snap_at(y, 0, step)).into();
                let cover = edges_px(at, size, scale);
                assert_eq!((pic.0, pic.1), (cover.size.w, cover.size.h), "{}x{} pt at {},{} scale {scale}: picture {pic:?}, covers {:?}", size.w, size.h, at.x, at.y, cover.size);
                let shown = shown_px(pic, cover.size);
                assert_eq!((shown.size.w as i32, shown.size.h as i32), pic, "nothing cropped");
            }
        }
    }
}

/// The bar and the body meet on one pixel row and share both side edges,
/// on the grid or off it (e24e617's rule: each edge rounded on its own).
#[test]
fn bar_and_body_meet_wherever_the_window_is() {
    for scale in GRID_SCALES.iter().chain(OFF_GRID.iter()).map(|s| s120(*s)) {
        let step = step_of(scale);
        for bar_h in [36, 28, 22] {
            let bar_h = snap_len(bar_h, step);
            for (w, h) in sizes() {
                for (x, y) in places() {
                    let size: Size<i32, Logical> = (w, h).into();
                    let body = edges_px((x, y).into(), size, scale);
                    let bar = edges_px((x, y - bar_h).into(), (w, bar_h).into(), scale);
                    assert_eq!(bar.loc.y + bar.size.h, body.loc.y, "bar meets body at {x},{y} scale {scale}");
                    assert_eq!((bar.loc.x, bar.size.w), (body.loc.x, body.size.w), "same sides at {x},{y} scale {scale}");
                }
            }
        }
    }
}

/// Bar heights on the grid are whole pixels (the dialog's 28 pt was 37.33).
#[test]
fn bar_heights_on_the_grid_are_whole_pixels() {
    for scale in GRID_SCALES.map(s120) {
        let step = step_of(scale);
        for bar_h in [36, 28, 22, 44] {
            let px = snap_len(bar_h, step) as f64 * scale;
            assert!((px - px.round()).abs() < 1e-9, "{bar_h} pt bar at {scale}: {px} px");
        }
    }
}

/// A picture's pixels read back as the points it was drawn for (points_of,
/// the inverse of the toolkit's rounding up), on the grid or not. Below
/// scale 1 two sizes in points can be drawn as the same pixels: then it
/// reads back as one that is drawn as those pixels.
#[test]
fn picture_sizes_read_back_as_their_points() {
    for scale in GRID_SCALES.iter().chain(OFF_GRID.iter()).map(|s| s120(*s)) {
        for points in [1, 22, 28, 320, 420, 579, 580, 773, 1173, 2880] {
            let px = picture_px((points, points), scale).0;
            let back = points_of(px, scale);
            if scale >= 1.0 {
                assert_eq!(back, points, "{points} pt at {scale} drawn as {px} px");
            } else {
                assert_eq!(picture_px((back, back), scale).0, px, "{points} pt at {scale} drawn as {px} px, read as {back}");
            }
        }
    }
}

/// Two windows that touch in points touch in pixels: no gap, no overlap.
#[test]
fn neighbours_touch_in_pixels() {
    for scale in GRID_SCALES.iter().chain(OFF_GRID.iter()).map(|s| s120(*s)) {
        for (x, w) in [(0, 1440), (13, 577), (100, 773)] {
            let a = edges_px((x, 50).into(), (w, 300).into(), scale);
            let b = edges_px((x + w, 50).into(), (400, 300).into(), scale);
            assert_eq!(a.loc.x + a.size.w, b.loc.x, "{x}+{w} at {scale}");
        }
    }
}
