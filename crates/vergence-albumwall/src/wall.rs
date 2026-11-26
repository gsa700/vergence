//! The wall's arithmetic (the Avalonia app's ComputeLayout and AlbumVm;
//! docs/albumwall-milestone-1.md, step 4): how many covers a row holds and
//! how large they are, and the color a tile shows until its cover comes.
//!
//! **A row fills the window exactly.** There is no size control (it replaced
//! a cover-size slider, which left a ragged strip at the right at almost
//! every width): the column count is the one nearest a 185-point cover, and
//! the covers stretch or shrink to fill the row, between 132 and 268 points;
//! past those the column count changes instead. Resizing the window scales
//! the art rather than only reflowing it.

use crate::colors::{Rgb, from_hsl};

/// The size a cover would ideally be, in points.
pub const TARGET: f32 = 185.0;
/// How far a cover may be squeezed or stretched before the column count changes.
pub const MIN: f32 = 132.0;
pub const MAX: f32 = 268.0;
/// Between covers, across and down.
pub const GAP: f32 = 15.0;
/// Around the wall: left and right, and above.
pub const PAD_SIDE: f32 = 26.0;
pub const PAD_TOP: f32 = 20.0;
/// Under a cover: the gap, the title (11 points) and the artist (10).
pub const TITLE: f32 = 11.0;
pub const ARTIST: f32 = 10.0;
pub const UNDER_GAP: f32 = 5.0;

/// For `available` points of width: the columns and the cover's size.
pub fn columns(available: f32) -> (usize, f32) {
    if available <= 0.0 {
        return (1, TARGET);
    }
    let mut cols = ((available + GAP) / (TARGET + GAP)).round().max(1.0) as usize;
    let cover = |cols: usize| ((available - (cols - 1) as f32 * GAP) / cols as f32).floor();
    let mut c = cover(cols);
    // At the extremes, give up filling the row exactly rather than show
    // covers the wrong size.
    if c < MIN && cols > 1 {
        cols -= 1;
        c = cover(cols);
    }
    (cols, c.clamp(MIN, MAX))
}

/// A tile's height for its cover's size: the cover, then the two lines
/// under it (a line is about 1.3 of its text's size; `text` the screen's
/// text factor).
pub fn tile_height(cover: f32, text: f32) -> f32 {
    cover + UNDER_GAP + (TITLE * text * 1.3).ceil() + UNDER_GAP + (ARTIST * text * 1.3).ceil()
}

/// The color a tile shows until its cover comes (and for good when there
/// is none): its own, from the album's names, so the wall is not a grid of
/// gray squares while covers load (AlbumVm.PlaceholderBrushFor: FNV-1a over
/// the names' UTF-16).
pub fn placeholder(album_artist: &str, title: &str) -> Rgb {
    let mut h: u32 = 2_166_136_261;
    for c in album_artist.encode_utf16().chain(title.encode_utf16()) {
        h ^= c as u32;
        h = h.wrapping_mul(16_777_619);
    }
    let hue = (h % 360) as f64;
    let sat = 0.26 + ((h >> 9) % 22) as f64 / 100.0;
    let light = 0.30 + ((h >> 17) % 16) as f64 / 100.0;
    from_hsl(hue, sat, light)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_fills_the_width_exactly() {
        for available in [600.0, 900.0, 1392.0, 2000.0, 3788.0] {
            let (cols, cover) = columns(available);
            let used = cols as f32 * cover + (cols - 1) as f32 * GAP;
            assert!(available - used < cols as f32, "{available}: {cols} x {cover} uses {used}");
            assert!((MIN..=MAX).contains(&cover));
        }
        // The Avalonia app's default window: 1444 wide, the wall 26 in on each side.
        assert_eq!(columns(1444.0 - 52.0), (7, 186.0));
    }

    #[test]
    fn a_narrow_window_keeps_covers_their_size() {
        let (cols, cover) = columns(200.0);
        assert_eq!(cols, 1);
        assert!((MIN..=MAX).contains(&cover));
        assert_eq!(columns(0.0), (1, TARGET));
    }

    #[test]
    fn placeholders_are_the_albums_own_and_steady() {
        let a = placeholder("Toto", "IV");
        assert_eq!(a, placeholder("Toto", "IV"));
        assert_ne!(a, placeholder("Toto", "Isolation"));
    }
}
