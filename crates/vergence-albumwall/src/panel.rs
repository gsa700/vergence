//! The open album's panel, its words and its measurements (the Avalonia
//! app's PanelRow in WallRows.cs and its template; docs/albumwall-milestone-1.md,
//! step 5): the header line, the track list in two columns, its height.
//!
//! - **The header line**: year · tracks · discs · length · file type ·
//!   format, each only when it says something; "mixed" when the tracks
//!   differ (the file type from the extension, not a guess at the codec).
//! - **Two columns, read down the left and then down the right** (a track
//!   list is a sequence); disc headings are rows in the flow, the left
//!   column never shorter than the right and never ending on a heading.
//! - **Its height** is worked out, not measured, so the wall's rows below
//!   move once, to the right place: the header, the row for Lyrics, the
//!   track list or the 260-point sleeve, whichever is taller, and the
//!   panel's padding (54 at its foot, where it fades into the wall).

use crate::library::Album;

/// The sleeve beside the track list (the play button), in points.
pub const SLEEVE: f32 = 260.0;
/// The arrow pointing at the tile: its width and height.
pub const ARROW_W: f32 = 22.0;
pub const ARROW_H: f32 = 11.0;
/// Above the body (the arrow sits in it), the body's padding, and its foot.
pub const MARGIN: f32 = 10.0;
pub const PAD: f32 = 22.0;
pub const FOOT: f32 = 54.0;
/// Between the track list and the sleeve, and between the two columns.
pub const GAP: f32 = 24.0;
pub const COLUMN_GAP: f32 = 26.0;
/// A track line's height and its number's width, before the text factor.
pub const LINE: f32 = 28.0;
pub const NUMBER: f32 = 30.0;
/// The empty row kept under the header (Lyrics, in milestone 2), its gap above.
pub const BUTTON_ROW: f32 = 30.0;
pub const BUTTON_GAP: f32 = 14.0;
/// Above the track list.
pub const LIST_GAP: f32 = 16.0;

/// One row of the track list.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// "Disc 2 of 3".
    Disc(String),
    /// A track: its place in the album (what playing starts from).
    Track(usize),
}

/// The track list split into a left and a right column.
pub fn columns(album: &Album) -> (Vec<Line>, Vec<Line>) {
    let discs = album.discs();
    let mut lines = Vec::new();
    let mut disc = None;
    for (i, t) in album.tracks.iter().enumerate() {
        if discs > 1 && disc != Some(t.disc) {
            disc = Some(t.disc);
            lines.push(Line::Disc(format!("Disc {} of {discs}", t.disc)));
        }
        lines.push(Line::Track(i));
    }
    let mut half = lines.len().div_ceil(2);
    if half > 0 && half < lines.len() && matches!(lines[half - 1], Line::Disc(_)) {
        half -= 1;
    }
    let right = lines.split_off(half);
    (lines, right)
}

/// "4:05", or "1:02:03" past an hour.
pub fn duration(millis: u64) -> String {
    let s = millis / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// The line under the title and artist.
pub fn header_line(album: &Album) -> String {
    let mut parts = Vec::new();
    if album.year > 0 {
        parts.push(album.year.to_string());
    }
    let n = album.tracks.len();
    parts.push(if n == 1 { "1 track".into() } else { format!("{n} tracks") });
    if album.discs() > 1 {
        parts.push(format!("{} discs", album.discs()));
    }
    parts.push(duration(album.millis()));
    let mut types: Vec<String> =
        album.tracks.iter().filter_map(|t| t.path.extension().map(|e| e.to_string_lossy().to_uppercase())).collect();
    types.sort();
    types.dedup();
    match types.len() {
        0 => {}
        1 => parts.push(types.remove(0)),
        _ => parts.push(format!("mixed {}", types.join(", "))),
    }
    let mut specs: Vec<String> = album
        .tracks
        .iter()
        .filter(|t| t.rate > 0)
        .map(|t| {
            let khz = khz(t.rate);
            // A lossy file keeps no depth (Hymie says 0): its rate alone.
            if t.bits > 0 { format!("{}/{khz}", t.bits) } else { khz }
        })
        .collect();
    specs.dedup();
    specs.sort();
    specs.dedup();
    match specs.len() {
        0 => {}
        1 => parts.push(format!("{} kHz", specs[0])),
        _ => parts.push(format!("mixed {} kHz", specs.join(", "))),
    }
    parts.join("   ·   ")
}

/// 44100 as "44.1", 48000 as "48".
fn khz(rate: u32) -> String {
    let k = rate as f64 / 1000.0;
    if (k - k.round()).abs() < 0.05 { format!("{}", k.round() as u64) } else { format!("{k:.1}") }
}

/// The rows of a column, in points: tracks a line each, a disc heading its
/// gap above, its line and its gap below.
fn column_height(lines: &[Line], text: f32) -> f32 {
    lines
        .iter()
        .map(|l| match l {
            Line::Track(_) => (LINE * text).ceil(),
            Line::Disc(_) => 14.0 + (11.0 * text * 1.3).ceil() + 5.0,
        })
        .sum()
}

/// The track list's height: the taller column.
pub fn list_height(album: &Album, text: f32) -> f32 {
    let (left, right) = columns(album);
    column_height(&left, text).max(column_height(&right, text))
}

/// The lyrics view's height in place of the track list: the list's, or
/// down to the sleeve's foot if that is more, so the panel keeps its height.
pub fn words_height(album: &Album, text: f32) -> f32 {
    list_height(album, text).max(SLEEVE - header_height(text) - LIST_GAP)
}

/// The header: title, artist, the header line, the row kept for Lyrics.
pub fn header_height(text: f32) -> f32 {
    (21.0 * text * 1.3).ceil()
        + 3.0
        + (14.0 * text * 1.3).ceil()
        + 3.0
        + 4.0
        + (11.0 * text * 1.3).ceil()
        + BUTTON_GAP
        + (BUTTON_ROW * text).round()
}

/// The panel's whole height: the margin the arrow sits in, the body's
/// padding, the taller of the header and track list or the sleeve, and the
/// foot where it fades.
pub fn height(album: &Album, text: f32) -> f32 {
    let (left, right) = columns(album);
    let list = column_height(&left, text).max(column_height(&right, text));
    let words = header_height(text) + LIST_GAP + list;
    MARGIN + PAD + words.max(SLEEVE) + FOOT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Track;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn album(discs: &[usize], ext: &str, rate: u32, bits: u16) -> Album {
        let mut tracks = Vec::new();
        for (d, n) in discs.iter().enumerate() {
            for i in 0..*n {
                tracks.push(Track {
                    disc: d as u32 + 1,
                    number: i as u32 + 1,
                    title: format!("T{i}"),
                    artist: "A".into(),
                    millis: 240_000,
                    path: PathBuf::from(format!("/m/{d}{i}.{ext}")),
                    rate,
                    bits,
                    bitrate: 900,
                    size: 1,
                });
            }
        }
        Album {
            album_artist: "A".into(),
            title: "B".into(),
            year: 1982,
            tracks,
            front: None,
            back: None,
            folders: BTreeSet::new(),
            search: String::new(),
        }
    }

    #[test]
    fn the_header_line_says_what_the_files_are() {
        assert_eq!(
            header_line(&album(&[10], "flac", 44100, 16)),
            "1982   ·   10 tracks   ·   40:00   ·   FLAC   ·   16/44.1 kHz"
        );
        let two = album(&[8, 8], "mp3", 44100, 0);
        assert_eq!(header_line(&two), "1982   ·   16 tracks   ·   2 discs   ·   1:04:00   ·   MP3   ·   44.1 kHz");
        let mut mixed = album(&[2], "flac", 96000, 24);
        mixed.tracks[1].rate = 44100;
        mixed.tracks[1].bits = 16;
        assert!(header_line(&mixed).ends_with("mixed 16/44.1, 24/96 kHz"), "{}", header_line(&mixed));
    }

    #[test]
    fn two_columns_read_down_and_never_end_on_a_heading() {
        let (l, r) = columns(&album(&[10], "flac", 44100, 16));
        assert_eq!((l.len(), r.len()), (5, 5));
        assert_eq!(l[0], Line::Track(0));
        assert_eq!(r[0], Line::Track(5));
        // 5 + 5 tracks and two headings: 12 lines; the left would end on
        // "Disc 2", so it gives the heading to the right.
        let (l, r) = columns(&album(&[5, 5], "flac", 44100, 16));
        assert!(!matches!(l.last(), Some(Line::Disc(_))));
        assert_eq!(r[0], Line::Disc("Disc 2 of 2".into()));
        assert!(l.len() >= r.len() - 1);
    }

    #[test]
    fn a_panel_is_at_least_the_sleeve_tall() {
        let short = height(&album(&[2], "flac", 44100, 16), 1.0);
        assert_eq!(short, MARGIN + PAD + SLEEVE + FOOT);
        let long = height(&album(&[40], "flac", 44100, 16), 1.0);
        assert!(long > short);
        assert_eq!(duration(3_723_000), "1:02:03");
        assert_eq!(duration(245_000), "4:05");
    }
}
