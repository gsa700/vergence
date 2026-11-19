//! The overview's arrangement (overview milestone 1, step 5;
//! docs/overview-milestone-1.md): every window of a screen shown at once,
//! scaled down so none overlap, Mission Control's idea (Dave, 2026-11-18).
//!
//! Pure: windows in (their places on the screen), places out. Every window
//! is scaled by the same amount, so their sizes still say which is which;
//! never above its own size. The windows go into rows by where their middles
//! are, top to bottom, and along each row by where they are across, so each
//! stays roughly where it was. The number of rows is the one that lets the
//! scale be largest. Rows are centered across the area, the rows together
//! down it, and each window in the middle of its row's height.

use crate::Rect;

/// Where each of `windows` goes inside `area`, in the same order, with at
/// least `gap` points between any two and between them and the area's edge.
/// An empty area or no windows: nothing.
pub fn arrange(windows: &[Rect], area: Rect, gap: i32) -> Vec<Rect> {
    let n = windows.len();
    if n == 0 || area.w <= 2 * gap || area.h <= 2 * gap {
        return Vec::new();
    }
    let inner = Rect::new(area.x + gap, area.y + gap, area.w - 2 * gap, area.h - 2 * gap);
    // Top to bottom by their middles (then left to right, then the order given).
    let mut by_y: Vec<usize> = (0..n).collect();
    by_y.sort_by_key(|&i| (windows[i].y + windows[i].h / 2, windows[i].x + windows[i].w / 2, i));
    let mut best: Option<(f64, Vec<Vec<usize>>)> = None;
    for rows_n in 1..=n {
        // Split the windows into this many rows, as evenly as they go.
        let mut rows: Vec<Vec<usize>> = Vec::with_capacity(rows_n);
        let mut taken = 0;
        for r in 0..rows_n {
            let count = (n - taken) / (rows_n - r);
            let mut row: Vec<usize> = by_y[taken..taken + count].to_vec();
            row.sort_by_key(|&i| (windows[i].x + windows[i].w / 2, i));
            rows.push(row);
            taken += count;
        }
        let scale = scale_for(windows, &rows, inner, gap);
        if best.as_ref().is_none_or(|(s, _)| scale > *s + 1e-9) {
            best = Some((scale, rows));
        }
    }
    let Some((scale, rows)) = best else { return Vec::new() };
    let size = |i: usize| (((windows[i].w as f64 * scale).round() as i32).max(1), ((windows[i].h as f64 * scale).round() as i32).max(1));
    let heights: Vec<i32> = rows.iter().map(|row| row.iter().map(|&i| size(i).1).max().unwrap_or(0)).collect();
    let total_h = heights.iter().sum::<i32>() + gap * (rows.len() as i32 - 1);
    let mut out = vec![Rect::new(0, 0, 0, 0); n];
    let mut y = inner.y + (inner.h - total_h).max(0) / 2;
    for (row, h) in rows.iter().zip(&heights) {
        let row_w = row.iter().map(|&i| size(i).0).sum::<i32>() + gap * (row.len() as i32 - 1);
        let mut x = inner.x + (inner.w - row_w).max(0) / 2;
        for &i in row {
            let (w, wh) = size(i);
            out[i] = Rect::new(x, y + (h - wh) / 2, w, wh);
            x += w + gap;
        }
        y += h + gap;
    }
    out
}

/// The largest scale (at most 1) at which `rows` of `windows` fit `inner`.
fn scale_for(windows: &[Rect], rows: &[Vec<usize>], inner: Rect, gap: i32) -> f64 {
    let mut scale = 1.0f64;
    for row in rows {
        let w: i64 = row.iter().map(|&i| windows[i].w.max(1) as i64).sum();
        let room = inner.w - gap * (row.len() as i32 - 1);
        if room <= 0 {
            return 0.0;
        }
        scale = scale.min(room as f64 / w as f64);
    }
    let h: i64 = rows.iter().map(|row| row.iter().map(|&i| windows[i].h.max(1) as i64).max().unwrap_or(0)).sum();
    let room = inner.h - gap * (rows.len() as i32 - 1);
    if room <= 0 {
        return 0.0;
    }
    // A row's height is set by its tallest window; rounding up a point per row
    // keeps the rounded sizes inside the room.
    let rows_n = rows.len() as f64;
    scale.min((room as f64 - rows_n) / h as f64).max(0.0)
}

/// The tile under a point, if any (the arrangement's places, in order).
pub fn at(places: &[Rect], x: i32, y: i32) -> Option<usize> {
    places.iter().position(|r| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
}

/// The place to move to from `from` with an arrow: the nearest whose middle
/// lies that way (`dx`, `dy` one of the four directions), else `from`.
pub fn step(places: &[Rect], from: usize, dx: i32, dy: i32) -> usize {
    let Some(f) = places.get(from) else { return from };
    let mid = |r: &Rect| (r.x + r.w / 2, r.y + r.h / 2);
    let (fx, fy) = mid(f);
    places
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != from)
        .filter_map(|(i, r)| {
            let (x, y) = mid(r);
            let (ax, ay) = (x - fx, y - fy);
            // That way: more along the arrow than across it.
            let along = ax * dx + ay * dy;
            let across = (ax * dy - ay * dx).abs();
            (along > 0 && along >= across / 2).then_some((along as i64 + 2 * across as i64, i))
        })
        .min()
        .map_or(from, |(_, i)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlap(a: &Rect, b: &Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn windows_never_overlap_and_stay_inside() {
        let area = Rect::new(0, 40, 1920, 990);
        // Five windows piled on one another, as they often are.
        let windows = [Rect::new(100, 100, 1200, 800), Rect::new(150, 150, 1200, 800), Rect::new(600, 200, 900, 700), Rect::new(0, 0, 1920, 1030), Rect::new(1400, 600, 400, 300)];
        let out = arrange(&windows, area, 24);
        assert_eq!(out.len(), 5);
        for (i, a) in out.iter().enumerate() {
            assert!(a.x >= area.x + 24 && a.y >= area.y + 24 && a.x + a.w <= area.x + area.w - 24 && a.y + a.h <= area.y + area.h - 24, "{i} inside: {a:?}");
            for b in &out[i + 1..] {
                assert!(!overlap(a, b), "{a:?} and {b:?}");
            }
        }
    }

    #[test]
    fn each_keeps_its_shape_and_all_one_scale() {
        let windows = [Rect::new(0, 0, 1000, 500), Rect::new(0, 0, 500, 1000), Rect::new(0, 0, 800, 600)];
        let out = arrange(&windows, Rect::new(0, 0, 1600, 900), 20);
        let scales: Vec<f64> = out.iter().zip(&windows).map(|(o, w)| o.w as f64 / w.w as f64).collect();
        for (o, w) in out.iter().zip(&windows) {
            let (a, b) = (o.w as f64 / o.h as f64, w.w as f64 / w.h as f64);
            assert!((a - b).abs() / b < 0.02, "shape {o:?} of {w:?}");
        }
        assert!(scales.iter().all(|s| (s - scales[0]).abs() < 0.01), "one scale: {scales:?}");
    }

    #[test]
    fn a_lone_small_window_is_not_blown_up() {
        let out = arrange(&[Rect::new(300, 300, 400, 300)], Rect::new(0, 0, 1920, 1080), 24);
        assert_eq!((out[0].w, out[0].h), (400, 300));
        // In the middle of the area.
        assert_eq!((out[0].x, out[0].y), (760, 390));
    }

    #[test]
    fn windows_keep_their_rough_places() {
        // Two side by side stay side by side, left on the left.
        let out = arrange(&[Rect::new(1000, 100, 800, 600), Rect::new(0, 100, 800, 600)], Rect::new(0, 0, 1920, 1080), 24);
        assert!(out[1].x < out[0].x);
        // Many: the top one is in the top row.
        let windows: Vec<Rect> = (0..6).map(|i| Rect::new((i % 3) * 600, (i / 3) * 500, 560, 460)).collect();
        let out = arrange(&windows, Rect::new(0, 0, 1920, 1080), 24);
        assert!(out[0].y < out[3].y && out[0].x < out[1].x && out[1].x < out[2].x);
    }

    #[test]
    fn the_arrows_move_to_the_nearest_that_way() {
        let places = [Rect::new(0, 0, 100, 100), Rect::new(200, 0, 100, 100), Rect::new(0, 200, 100, 100), Rect::new(200, 200, 100, 100)];
        assert_eq!(step(&places, 0, 1, 0), 1);
        assert_eq!(step(&places, 0, 0, 1), 2);
        assert_eq!(step(&places, 3, -1, 0), 2);
        assert_eq!(step(&places, 0, -1, 0), 0, "nothing that way: stays");
        assert_eq!(at(&places, 250, 250), Some(3));
        assert_eq!(at(&places, 150, 150), None);
    }

    #[test]
    fn nothing_in_nothing_out() {
        assert!(arrange(&[], Rect::new(0, 0, 100, 100), 10).is_empty());
        assert!(arrange(&[Rect::new(0, 0, 10, 10)], Rect::new(0, 0, 10, 10), 10).is_empty());
    }
}
