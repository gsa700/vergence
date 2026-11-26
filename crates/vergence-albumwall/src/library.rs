//! AlbumWall's library rules (ported from the Avalonia app's Model.cs and
//! LibraryScanner.cs; docs/albumwall-milestone-1.md, step 2): what an album
//! is, its year, the order of its tracks and of the wall, its cover.
//!
//! - **An album is (album artist, album)**, from the tags: a set of discs in
//!   folders of their own is one album, ordered by disc then track. Folder
//!   names are not identity.
//! - **The year** comes from a "YYYY - " folder name, which is authoritative
//!   in these libraries (a FLAC's DATE is often a reissue's), else the tag;
//!   the earliest across an album's folders, so a box set's year does not
//!   change with the order the folders were listed in.
//! - **The wall's order**: the album artist with a leading "The ", "A " or
//!   "An " left out, then the year, then the title, ignoring case.
//! - **Missing tags**: the album artist falls back to the track's artist,
//!   then "Unknown Artist"; the album to "Unknown Album"; the title to the
//!   file's name; the track's artist to the album artist; the disc to 1.
//! - **The cover**: a picture embedded in ANY of the album's tracks beats a
//!   sidecar (a 1,305-byte black Folder.jpg scattered by some tool hid good
//!   embedded covers on Hambench); a sidecar (cover.jpg, cover.jpeg,
//!   cover.png, folder.jpg, folder.jpeg, front.jpg, any case) only when no
//!   track has one. The back of the sleeve: back.jpg, back.jpeg, back.png.
//!
//! The rules take the facts of files already read ([`TrackFacts`]) and a
//! way to find sidecars, so they are tested without a disk or the helper.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The formats a library holds: what Hymie plays (keep the two the same).
pub const AUDIO_EXTENSIONS: [&str; 4] = ["flac", "mp3", "m4a", "wav"];
/// The front of the sleeve beside the tracks, in the order looked for.
pub const ART_NAMES: [&str; 6] = ["cover.jpg", "cover.jpeg", "cover.png", "folder.jpg", "folder.jpeg", "front.jpg"];
/// The back of the sleeve.
pub const BACK_NAMES: [&str; 3] = ["back.jpg", "back.jpeg", "back.png"];

/// Whether `path` is a music file a library holds, by its extension.
pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(e)))
}

/// Whether a change to `path` could change what a scan finds (the watcher asks).
pub fn is_library_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    is_audio(path) || ART_NAMES.iter().chain(BACK_NAMES.iter()).any(|n| n.eq_ignore_ascii_case(name))
}

/// What a scan keeps of one file (the index stores exactly this).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackFacts {
    /// False for a file opened and found not to be music Hymie plays: kept
    /// all the same, so it is not opened again on every scan.
    pub readable: bool,
    pub album_artist: String,
    pub album: String,
    pub tag_year: i32,
    pub disc: u32,
    pub number: u32,
    pub title: String,
    pub artist: String,
    pub millis: u64,
    pub rate: u32,
    pub bits: u16,
    /// kbit/s, the whole file over its length (art included): the Statistics page's.
    pub bitrate: u32,
    /// A cover is embedded, and its size ((0, 0) unknown).
    pub cover: Option<(u32, u32)>,
}

impl TrackFacts {
    /// A file that is not music Hymie plays.
    pub fn unreadable() -> TrackFacts {
        TrackFacts::default()
    }

    /// The facts of the file at `path` (`size` bytes) from what Hymie read
    /// of it, with the fallbacks for missing tags.
    pub fn from_hymie(path: &Path, size: u64, f: &vergence_hymie::facts::Facts) -> TrackFacts {
        let value = |k: &str| f.tags.values.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
        let artist_tag = f.tags.artist.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let album_artist = value("ALBUMARTIST")
            .or_else(|| value("ALBUM ARTIST"))
            .or(artist_tag)
            .unwrap_or("Unknown Artist")
            .to_string();
        let album = f.tags.album.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("Unknown Album");
        let title = f
            .tags
            .title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
        let millis = (f.seconds() * 1000.0).round() as u64;
        TrackFacts {
            readable: true,
            album: album.to_string(),
            tag_year: value("DATE").or_else(|| value("YEAR")).and_then(year_of).unwrap_or(0),
            disc: value("DISCNUMBER").and_then(leading_number).unwrap_or(0).max(1),
            number: value("TRACKNUMBER").and_then(leading_number).unwrap_or(0),
            title,
            artist: artist_tag.map(str::to_string).unwrap_or_else(|| album_artist.clone()),
            album_artist,
            millis,
            rate: f.rate,
            bits: f.bits,
            bitrate: size.saturating_mul(8).checked_div(millis).unwrap_or(0) as u32,
            cover: f.cover,
        }
    }
}

/// "3/10" is 3; "07" is 7.
fn leading_number(s: &str) -> Option<u32> {
    let digits: String = s.trim().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The year in a date: its first four digits ("1982", "1982-05-01").
fn year_of(s: &str) -> Option<i32> {
    let digits: String = s.trim().chars().take_while(char::is_ascii_digit).take(4).collect();
    (digits.len() == 4).then(|| digits.parse().ok()).flatten().filter(|y| *y > 0)
}

/// The year a "YYYY - " folder name gives ("1978 - Toto").
pub fn folder_year(name: &str) -> Option<i32> {
    let b = name.as_bytes();
    (b.len() >= 7 && b[..4].iter().all(u8::is_ascii_digit) && &b[4..7] == b" - ")
        .then(|| name[..4].parse().ok())
        .flatten()
}

/// A picture beside or in the tracks, measured, with what it came from
/// (the decoded cover's cache keys on its size and time, so a sleeve
/// replaced in place is decoded again).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Art {
    pub path: PathBuf,
    /// The picture is in this track, not a file of its own.
    pub embedded: bool,
    /// Its size in pixels ((0, 0): unknown, never "small").
    pub width: u32,
    pub height: u32,
    /// The size and time of the file it came from.
    pub size: u64,
    pub modified: i64,
}

/// One track of an album.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub disc: u32,
    pub number: u32,
    pub title: String,
    pub artist: String,
    pub millis: u64,
    pub path: PathBuf,
    pub rate: u32,
    pub bits: u16,
    pub bitrate: u32,
    pub size: u64,
}

/// An album on the wall.
#[derive(Debug, Clone, PartialEq)]
pub struct Album {
    pub album_artist: String,
    pub title: String,
    pub year: i32,
    pub tracks: Vec<Track>,
    pub front: Option<Art>,
    pub back: Option<Art>,
    /// Every folder its tracks are in (a set of discs spans several).
    pub folders: BTreeSet<PathBuf>,
    /// What the search box looks in: "album artist\ntitle", lower case.
    pub search: String,
}

impl Album {
    /// The wall's order: the artist without a leading article, the year, the title.
    pub fn sort_key(&self) -> String {
        sort_key(&self.album_artist, self.year, &self.title)
    }

    pub fn discs(&self) -> usize {
        self.tracks.iter().map(|t| t.disc).collect::<BTreeSet<_>>().len()
    }

    pub fn millis(&self) -> u64 {
        self.tracks.iter().map(|t| t.millis).sum()
    }
}

/// "The Doobie Brothers" sorts under D; "Theatre of Tragedy" under T.
pub fn sort_key(album_artist: &str, year: i32, title: &str) -> String {
    let mut a = album_artist;
    for article in ["The ", "A ", "An "] {
        if a.len() >= article.len()
            && a.is_char_boundary(article.len())
            && a[..article.len()].eq_ignore_ascii_case(article)
        {
            a = &a[article.len()..];
            break;
        }
    }
    format!("{a} {year:04} {title}")
}

/// One file read, as the scan hands it over.
#[derive(Debug, Clone)]
pub struct Seen {
    pub path: PathBuf,
    pub size: u64,
    pub modified: i64,
    pub facts: TrackFacts,
}

/// The albums of a library from its files' facts, in the wall's order.
/// `sidecar(folder, names)` finds the first of `names` in a folder (any
/// case) and measures it.
pub fn group(files: &[Seen], mut sidecar: impl FnMut(&Path, &[&str]) -> Option<Art>) -> Vec<Album> {
    let mut albums: Vec<Album> = Vec::new();
    let mut by_key: HashMap<(String, String), usize> = HashMap::new();
    // A folder's sidecars are looked for once, however many tracks it holds.
    let mut fronts: HashMap<PathBuf, Option<Art>> = HashMap::new();
    let mut backs: HashMap<PathBuf, Option<Art>> = HashMap::new();
    for f in files.iter().filter(|f| f.facts.readable) {
        let t = &f.facts;
        let key = (t.album_artist.clone(), t.album.clone());
        let i = *by_key.entry(key).or_insert_with(|| {
            albums.push(Album {
                album_artist: t.album_artist.clone(),
                title: t.album.clone(),
                year: 0,
                tracks: Vec::new(),
                front: None,
                back: None,
                folders: BTreeSet::new(),
                search: String::new(),
            });
            albums.len() - 1
        });
        let album = &mut albums[i];
        let folder = f.path.parent().map(Path::to_path_buf).unwrap_or_default();
        album.folders.insert(folder.clone());
        let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let year = folder_year(&name).unwrap_or(t.tag_year);
        if year > 0 && (album.year == 0 || year < album.year) {
            album.year = year;
        }
        album.tracks.push(Track {
            disc: t.disc,
            number: t.number,
            title: t.title.clone(),
            artist: t.artist.clone(),
            millis: t.millis,
            path: f.path.clone(),
            rate: t.rate,
            bits: t.bits,
            bitrate: t.bitrate,
            size: f.size,
        });
        // The back first, on its own: an album whose front is settled still
        // gets its back from a later folder.
        if album.back.is_none() {
            album.back = backs.entry(folder.clone()).or_insert_with(|| sidecar(&folder, &BACK_NAMES)).clone();
        }
        if album.front.as_ref().is_some_and(|a| a.embedded) {
            continue;
        }
        if let Some((width, height)) = t.cover {
            album.front =
                Some(Art { path: f.path.clone(), embedded: true, width, height, size: f.size, modified: f.modified });
        } else if album.front.is_none() {
            album.front = fronts.entry(folder.clone()).or_insert_with(|| sidecar(&folder, &ART_NAMES)).clone();
        }
    }
    for a in &mut albums {
        a.tracks.sort_by_key(|t| (t.disc, t.number));
        a.search = format!("{}\n{}", a.album_artist, a.title).to_lowercase();
    }
    albums.sort_by_cached_key(|a| a.sort_key().to_uppercase());
    albums
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(album_artist: &str, album: &str, disc: u32, number: u32) -> TrackFacts {
        TrackFacts {
            readable: true,
            album_artist: album_artist.into(),
            album: album.into(),
            disc,
            number,
            title: format!("Track {number}"),
            artist: album_artist.into(),
            millis: 1000,
            ..TrackFacts::default()
        }
    }

    fn seen(path: &str, f: TrackFacts) -> Seen {
        Seen { path: PathBuf::from(path), size: 100, modified: 1, facts: f }
    }

    fn no_sidecars(_: &Path, _: &[&str]) -> Option<Art> {
        None
    }

    #[test]
    fn a_leading_article_is_left_out() {
        for (artist, sorts_as) in [
            ("The Doobie Brothers", "Doobie Brothers"),
            ("A Flock of Seagulls", "Flock of Seagulls"),
            ("An Emotional Fish", "Emotional Fish"),
            ("THE BEATLES", "BEATLES"),
            ("Theatre of Tragedy", "Theatre of Tragedy"),
            ("Them", "Them"),
        ] {
            assert!(sort_key(artist, 1970, "X").starts_with(&format!("{sorts_as} ")), "{artist}");
        }
    }

    #[test]
    fn an_artists_albums_go_by_year_then_title() {
        assert!(sort_key("Toto", 1978, "Toto") < sort_key("Toto", 1982, "IV"));
        assert!(sort_key("Toto", 1982, "A") < sort_key("Toto", 1982, "B"));
    }

    #[test]
    fn albums_come_in_the_walls_order_a_leading_article_ignored() {
        let files = [
            seen("/m/Toto/1978 - Toto/01.flac", facts("Toto", "Toto", 1, 1)),
            seen(
                "/m/The Doobie Brothers/1973 - The Captain/01.flac",
                facts("The Doobie Brothers", "The Captain", 1, 1),
            ),
            seen("/m/ABBA/1976 - Arrival/01.flac", facts("ABBA", "Arrival", 1, 1)),
        ];
        let titles: Vec<String> = group(&files, no_sidecars).into_iter().map(|a| a.album_artist).collect();
        assert_eq!(titles, ["ABBA", "The Doobie Brothers", "Toto"]);
    }

    #[test]
    fn the_folders_year_beats_the_tags_reissue_year() {
        let mut f = facts("Toto", "Toto", 1, 1);
        f.tag_year = 2014;
        let albums = group(&[seen("/m/Toto/1978 - Toto/01.flac", f.clone())], no_sidecars);
        assert_eq!(albums[0].year, 1978);
        let albums = group(&[seen("/m/Toto/Toto/01.flac", f)], no_sidecars);
        assert_eq!(albums[0].year, 2014);
    }

    #[test]
    fn a_box_set_takes_its_earliest_folders_year_whatever_the_order() {
        let files = [
            seen("/m/Toto/1984 - Isolation/01.flac", facts("Toto", "All In", 3, 1)),
            seen("/m/Toto/1978 - Toto/01.flac", facts("Toto", "All In", 1, 1)),
        ];
        assert_eq!(group(&files, no_sidecars)[0].year, 1978);
    }

    #[test]
    fn discs_in_folders_of_their_own_are_one_album_in_order() {
        let files = [
            seen("/m/LZ/2014 - LZ I (Disc 2)/02.flac", facts("Led Zeppelin", "Led Zeppelin I", 2, 2)),
            seen("/m/LZ/2014 - LZ I (Disc 1)/02.flac", facts("Led Zeppelin", "Led Zeppelin I", 1, 2)),
            seen("/m/LZ/2014 - LZ I (Disc 2)/01.flac", facts("Led Zeppelin", "Led Zeppelin I", 2, 1)),
            seen("/m/LZ/2014 - LZ I (Disc 1)/01.flac", facts("Led Zeppelin", "Led Zeppelin I", 1, 1)),
        ];
        let albums = group(&files, no_sidecars);
        assert_eq!(albums.len(), 1);
        let order: Vec<(u32, u32)> = albums[0].tracks.iter().map(|t| (t.disc, t.number)).collect();
        assert_eq!(order, [(1, 1), (1, 2), (2, 1), (2, 2)]);
        assert_eq!(albums[0].discs(), 2);
        assert_eq!(albums[0].folders.len(), 2);
    }

    #[test]
    fn a_cover_in_any_track_beats_a_sidecar_found_first() {
        let mut second = facts("X", "Y", 1, 2);
        second.cover = Some((600, 600));
        let files = [seen("/m/X/Y/01.flac", facts("X", "Y", 1, 1)), seen("/m/X/Y/02.flac", second)];
        let black = |dir: &Path, names: &[&str]| {
            (names[0] == "cover.jpg").then(|| Art {
                path: dir.join("Folder.jpg"),
                embedded: false,
                width: 32,
                height: 32,
                size: 1305,
                modified: 1,
            })
        };
        let front = group(&files, black).remove(0).front.unwrap();
        assert!(front.embedded);
        assert_eq!(front.path, Path::new("/m/X/Y/02.flac"));
        assert_eq!((front.width, front.height), (600, 600));
    }

    #[test]
    fn a_sidecar_front_and_back_are_found() {
        let files = [seen("/m/X/Y/01.flac", facts("X", "Y", 1, 1))];
        let found = |dir: &Path, names: &[&str]| {
            Some(Art { path: dir.join(names[0]), embedded: false, width: 1000, height: 1270, size: 5, modified: 1 })
        };
        let a = group(&files, found).remove(0);
        assert_eq!(a.front.unwrap().path, Path::new("/m/X/Y/cover.jpg"));
        assert_eq!(a.back.unwrap().path, Path::new("/m/X/Y/back.jpg"));
    }

    #[test]
    fn a_loose_file_takes_its_performer_and_file_name_and_a_broken_one_is_passed_over() {
        let mut hymie = vergence_hymie::facts::Facts { rate: 44100, frames: 44100 * 3, ..Default::default() };
        hymie.tags.artist = Some("  Someone ".into());
        let f = TrackFacts::from_hymie(Path::new("/m/loose/a song.mp3"), 300_000, &hymie);
        assert_eq!(
            (f.album_artist.as_str(), f.album.as_str(), f.title.as_str(), f.artist.as_str(), f.disc),
            ("Someone", "Unknown Album", "a song", "Someone", 1)
        );
        assert_eq!(f.millis, 3000);
        assert_eq!(f.bitrate, 800);
        let files = [seen("/m/loose/a song.mp3", f), seen("/m/loose/broken.mp3", TrackFacts::unreadable())];
        let albums = group(&files, no_sidecars);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].tracks.len(), 1);
    }

    #[test]
    fn numbers_and_years_come_from_their_tags() {
        let mut hymie = vergence_hymie::facts::Facts::default();
        for (k, v) in [("ALBUMARTIST", "Toto"), ("TRACKNUMBER", "03/10"), ("DISCNUMBER", "2/2"), ("DATE", "1982-04-08")]
        {
            hymie.tags.values.insert(k.into(), v.into());
        }
        let f = TrackFacts::from_hymie(Path::new("/m/a.flac"), 1, &hymie);
        assert_eq!((f.album_artist.as_str(), f.number, f.disc, f.tag_year), ("Toto", 3, 2, 1982));
        assert_eq!(f.artist, "Toto");
    }

    #[test]
    fn folder_years_need_the_dash() {
        assert_eq!(folder_year("1978 - Toto"), Some(1978));
        assert_eq!(folder_year("2014 - Led Zeppelin I (Disc 1 of 2)"), Some(2014));
        assert_eq!(folder_year("1978 Toto"), None);
        assert_eq!(folder_year("Toto"), None);
    }

    #[test]
    fn library_files_by_name_any_case() {
        assert!(is_library_file(Path::new("/m/a/01.FLAC")));
        assert!(is_library_file(Path::new("/m/a/Folder.jpg")));
        assert!(is_library_file(Path::new("/m/a/back.png")));
        assert!(!is_library_file(Path::new("/m/a/notes.txt")));
        assert!(!is_library_file(Path::new("/m/a/song.ogg")));
    }
}
