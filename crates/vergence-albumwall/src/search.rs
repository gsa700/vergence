//! The search box (the Avalonia app's ApplyFilter and ArtQuery): live, over
//! album artist and title, lower case; and three maintenance queries for
//! the albums whose artwork wants attention, typed in whole:
//!
//! - `art:missing`: no cover at all;
//! - `art:small`: shorter than 300 pixels on its shorter side (the panel's
//!   sleeve at 100%: below it a cover is enlarged somewhere);
//! - `art:nonsquare`: its sides more than 5% apart.
//!
//! An unmeasured cover (0 x 0) matches none of them: unknown is not small.
//! Deliberately not in a menu: a query for whoever keeps the files, not
//! something a listener needs to see. The app fetches no art; better art
//! belongs in the files, put there once with a tagger.

use crate::library::Album;

/// A cover shorter than this on its shorter side is "small".
pub const SMALL_ART: u32 = 300;

/// The query as the search box's text means it: trimmed, lower case.
pub fn normalize(text: &str) -> String {
    text.trim().to_lowercase()
}

/// One of the art queries, if `query` (normalized) is one.
fn art_query(query: &str) -> Option<fn(&Album) -> bool> {
    match query {
        "art:missing" => Some(|a| a.front.is_none()),
        "art:small" => Some(|a| a.front.as_ref().is_some_and(|f| f.width > 0 && f.width.min(f.height) < SMALL_ART)),
        "art:nonsquare" => Some(|a| {
            a.front.as_ref().is_some_and(|f| {
                f.width > 0 && (f.width.abs_diff(f.height) as f64) > 0.05 * f.width.max(f.height) as f64
            })
        }),
        _ => None,
    }
}

/// The albums found, as their indexes in wall order: every one for an
/// empty query.
pub fn found(albums: &[Album], text: &str) -> Vec<usize> {
    let query = normalize(text);
    if query.is_empty() {
        return (0..albums.len()).collect();
    }
    match art_query(&query) {
        Some(wanted) => (0..albums.len()).filter(|&i| wanted(&albums[i])).collect(),
        None => (0..albums.len()).filter(|&i| albums[i].search.contains(&query)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Art;

    fn album(artist: &str, title: &str, art: Option<(u32, u32)>) -> Album {
        Album {
            album_artist: artist.into(),
            title: title.into(),
            search: format!("{artist}\n{title}").to_lowercase(),
            front: art.map(|(width, height)| Art {
                path: "cover.jpg".into(),
                embedded: false,
                width,
                height,
                size: 1,
                modified: 1,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn artist_and_title_in_any_case_and_the_art_queries() {
        let albums = vec![
            album("Alice in Chains", "Dirt", Some((1200, 1200))),
            album("AC/DC", "Back in Black", None),
            album("Tool", "Lateralus", Some((250, 250))),
            album("Rush", "Moving Pictures", Some((1000, 1270))),
            album("Yes", "Fragile", Some((0, 0))),
        ];
        assert_eq!(found(&albums, ""), vec![0, 1, 2, 3, 4]);
        // "in" is in "Alice in Chains", "Back in Black" and "Moving Pictures".
        assert_eq!(found(&albums, "  IN  "), vec![0, 1, 3]);
        assert_eq!(found(&albums, "lateralus"), vec![2]);
        // Across the two: no album has "chains dirt" in one line.
        assert_eq!(found(&albums, "chains dirt"), Vec::<usize>::new());
        assert_eq!(found(&albums, "art:missing"), vec![1]);
        assert_eq!(found(&albums, "ART:small"), vec![2], "unmeasured (0 x 0) is not small");
        assert_eq!(found(&albums, "art:nonsquare"), vec![3]);
    }
}
