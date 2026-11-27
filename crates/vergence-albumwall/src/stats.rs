//! A library's numbers for Preferences › Library (LibraryStats.cs): the
//! totals, the file types, the lossy tracks by bitrate, the lossless ones
//! by rate and depth, and the cover art. Worked out from the albums when a
//! library has been read, and kept beside its index so the numbers of a
//! library not on the wall can be shown "as of" when it was last read.

use std::path::{Path, PathBuf};

use crate::library::Album;

/// The side under which a cover counts as small (the `art:small` search's).
pub const SMALL_COVER: u32 = 300;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TypeRow {
    /// The extension, in capitals ("FLAC").
    pub kind: String,
    pub tracks: usize,
    pub bytes: u64,
    pub millis: u64,
    /// The middle track's bitrate in kbps (0: none reported).
    pub bitrate: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Band {
    pub label: String,
    pub tracks: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stats {
    pub albums: usize,
    pub tracks: usize,
    pub artists: usize,
    pub millis: u64,
    pub bytes: u64,
    pub types: Vec<TypeRow>,
    pub lossy: Vec<Band>,
    pub lossless: Vec<Band>,
    pub embedded: usize,
    pub sidecar: usize,
    pub missing: usize,
    pub small: usize,
}

const BANDS: [(u32, &str); 5] =
    [(320, "320 and up"), (256, "256 – 319"), (192, "192 – 255"), (128, "128 – 191"), (1, "under 128")];

impl Stats {
    pub fn of(albums: &[Album]) -> Stats {
        let tracks: Vec<_> = albums.iter().flat_map(|a| a.tracks.iter()).collect();
        let mut kinds: Vec<(String, Vec<&crate::library::Track>)> = Vec::new();
        for t in &tracks {
            let k = t.path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
            let k = if k.is_empty() { "(none)".to_string() } else { k };
            match kinds.iter_mut().find(|(n, _)| *n == k) {
                Some((_, v)) => v.push(t),
                None => kinds.push((k, vec![t])),
            }
        }
        let mut types: Vec<TypeRow> = kinds
            .into_iter()
            .map(|(kind, v)| {
                let mut rates: Vec<u32> = v.iter().map(|t| t.bitrate).filter(|b| *b > 0).collect();
                rates.sort_unstable();
                TypeRow {
                    kind,
                    tracks: v.len(),
                    bytes: v.iter().map(|t| t.size).sum(),
                    millis: v.iter().map(|t| t.millis).sum(),
                    bitrate: rates.get(rates.len() / 2).copied().unwrap_or(0),
                }
            })
            .collect();
        types.sort_by_key(|t| std::cmp::Reverse(t.tracks));

        let lossy: Vec<u32> = tracks.iter().filter(|t| t.bits == 0 && t.bitrate > 0).map(|t| t.bitrate).collect();
        let lossy = BANDS
            .iter()
            .enumerate()
            .map(|(i, (from, label))| Band {
                label: label.to_string(),
                tracks: lossy.iter().filter(|b| **b >= *from && (i == 0 || **b < BANDS[i - 1].0)).count(),
            })
            .filter(|b| b.tracks > 0)
            .collect();

        let mut formats: Vec<((u32, u16), usize)> = Vec::new();
        for t in tracks.iter().filter(|t| t.bits > 0) {
            match formats.iter_mut().find(|(k, _)| *k == (t.rate, t.bits)) {
                Some((_, n)) => *n += 1,
                None => formats.push(((t.rate, t.bits), 1)),
            }
        }
        formats.sort_by_key(|f| std::cmp::Reverse(f.1));
        let lossless = formats
            .into_iter()
            .map(|((rate, bits), n)| Band { label: format!("{} kHz / {bits}-bit", khz(rate)), tracks: n })
            .collect();

        let mut artists: Vec<&str> = albums.iter().map(|a| a.album_artist.as_str()).collect();
        artists.sort_unstable();
        artists.dedup();
        Stats {
            albums: albums.len(),
            tracks: tracks.len(),
            artists: artists.len(),
            millis: tracks.iter().map(|t| t.millis).sum(),
            bytes: tracks.iter().map(|t| t.size).sum(),
            types,
            lossy,
            lossless,
            embedded: albums.iter().filter(|a| a.front.as_ref().is_some_and(|f| f.embedded)).count(),
            sidecar: albums.iter().filter(|a| a.front.as_ref().is_some_and(|f| !f.embedded)).count(),
            missing: albums.iter().filter(|a| a.front.is_none()).count(),
            small: albums
                .iter()
                .filter(|a| {
                    a.front.as_ref().is_some_and(|f| f.width > 0 && f.height > 0 && f.width.min(f.height) < SMALL_COVER)
                })
                .count(),
        }
    }

    /// Kept as a few lines of our own: "key\tvalue..." per line.
    pub fn to_text(&self, as_of: i64) -> String {
        let mut s = format!(
            "# AlbumWall for Vergence: a library's numbers\nasof\t{as_of}\ntotals\t{}\t{}\t{}\t{}\t{}\nart\t{}\t{}\t{}\t{}\n",
            self.albums,
            self.tracks,
            self.artists,
            self.millis,
            self.bytes,
            self.embedded,
            self.sidecar,
            self.missing,
            self.small
        );
        for t in &self.types {
            s.push_str(&format!("type\t{}\t{}\t{}\t{}\t{}\n", t.kind, t.tracks, t.bytes, t.millis, t.bitrate));
        }
        for b in &self.lossy {
            s.push_str(&format!("lossy\t{}\t{}\n", b.tracks, b.label));
        }
        for b in &self.lossless {
            s.push_str(&format!("lossless\t{}\t{}\n", b.tracks, b.label));
        }
        s
    }

    /// The numbers and when they were worked out (seconds since 1970).
    pub fn from_text(text: &str) -> Option<(Stats, i64)> {
        let mut s = Stats::default();
        let mut as_of = None;
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let f: Vec<&str> = line.split('\t').collect();
            let n = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            match f.first().copied() {
                Some("asof") => as_of = f.get(1).and_then(|v| v.parse().ok()),
                Some("totals") => {
                    (s.albums, s.tracks, s.artists) = (n(1) as usize, n(2) as usize, n(3) as usize);
                    (s.millis, s.bytes) = (n(4), n(5));
                }
                Some("art") => {
                    (s.embedded, s.sidecar, s.missing, s.small) =
                        (n(1) as usize, n(2) as usize, n(3) as usize, n(4) as usize)
                }
                Some("type") if f.len() == 6 => s.types.push(TypeRow {
                    kind: f[1].to_string(),
                    tracks: n(2) as usize,
                    bytes: n(3),
                    millis: n(4),
                    bitrate: n(5) as u32,
                }),
                Some("lossy") if f.len() == 3 => s.lossy.push(Band { label: f[2].to_string(), tracks: n(1) as usize }),
                Some("lossless") if f.len() == 3 => {
                    s.lossless.push(Band { label: f[2].to_string(), tracks: n(1) as usize })
                }
                _ => {}
            }
        }
        Some((s, as_of?))
    }
}

/// Where a library's numbers are kept: beside its index.
pub fn path_for(folder: &Path) -> Option<PathBuf> {
    crate::index::path_for(folder).map(|p| p.with_extension("stats"))
}

/// The numbers kept for `folder`, and when.
pub fn load(folder: &Path) -> Option<(Stats, i64)> {
    Stats::from_text(&std::fs::read_to_string(path_for(folder)?).ok()?)
}

/// Kept for `folder`, as of now; written whole and renamed over.
pub fn save(folder: &Path, s: &Stats) {
    let Some(p) = path_for(folder) else { return };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("stats-new");
    if std::fs::write(&tmp, s.to_text(now)).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// 44100 as "44.1", 48000 as "48".
fn khz(rate: u32) -> String {
    let k = rate as f64 / 1000.0;
    let s = format!("{k:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

/// "4.2 GB", "310 MB", "12 KB" (LibraryNumbers.Bytes: powers of 1024).
pub fn bytes(b: u64) -> String {
    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;
    if b >= GB {
        let s = format!("{:.1}", b as f64 / GB as f64);
        format!("{} GB", s.strip_suffix(".0").unwrap_or(&s))
    } else if b >= MB {
        format!("{:.0} MB", b as f64 / MB as f64)
    } else {
        format!("{:.0} KB", b as f64 / 1024.0)
    }
}

/// "3 d 4 h", "5 h 12 min", "40 min" (LibraryNumbers.Spoken).
pub fn spoken(millis: u64) -> String {
    let m = millis / 60_000;
    let (d, h, min) = (m / 1440, (m / 60) % 24, m % 60);
    if d >= 1 {
        format!("{d} d {h} h")
    } else if m >= 60 {
        format!("{} h {min} min", m / 60)
    } else {
        format!("{min} min")
    }
}

/// "12.5%", "<0.1%" (LibraryNumbers.Share).
pub fn share(part: usize, whole: usize) -> String {
    if whole == 0 {
        return String::new();
    }
    if part as u64 * 1000 / whole as u64 == 0 {
        return "<0.1%".into();
    }
    let s = format!("{:.1}", 100.0 * part as f64 / whole as f64);
    format!("{}%", s.strip_suffix(".0").unwrap_or(&s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Art, Track};

    fn track(path: &str, bits: u16, bitrate: u32) -> Track {
        Track {
            disc: 1,
            number: 1,
            title: String::new(),
            artist: String::new(),
            millis: 200_000,
            path: path.into(),
            rate: 44_100,
            bits,
            bitrate,
            size: 10 << 20,
        }
    }

    fn album(artist: &str, tracks: Vec<Track>, front: Option<Art>) -> Album {
        Album { album_artist: artist.into(), tracks, front, ..Default::default() }
    }

    fn art(embedded: bool, side: u32) -> Option<Art> {
        Some(Art { embedded, width: side, height: side, ..Default::default() })
    }

    #[test]
    fn the_numbers() {
        let albums = vec![
            album("A", vec![track("1.flac", 16, 900), track("2.flac", 24, 1400)], art(true, 1000)),
            album("A", vec![track("3.mp3", 0, 320), track("4.mp3", 0, 128)], art(false, 200)),
            album("B", vec![track("5.mp3", 0, 96)], None),
        ];
        let s = Stats::of(&albums);
        assert_eq!((s.albums, s.tracks, s.artists), (3, 5, 2));
        assert_eq!(s.types[0].kind, "MP3");
        assert_eq!((s.types[0].tracks, s.types[0].bitrate), (3, 128));
        assert_eq!(
            s.lossy.iter().map(|b| (b.label.as_str(), b.tracks)).collect::<Vec<_>>(),
            [("320 and up", 1), ("128 – 191", 1), ("under 128", 1)]
        );
        assert_eq!(s.lossless.len(), 2);
        assert_eq!(s.lossless[0].label, "44.1 kHz / 16-bit");
        assert_eq!((s.embedded, s.sidecar, s.missing, s.small), (1, 1, 1, 1));
    }

    #[test]
    fn kept_and_read_back() {
        let albums = vec![album("A", vec![track("1.flac", 16, 900), track("2.m4a", 0, 256)], art(true, 600))];
        let s = Stats::of(&albums);
        let (back, when) = Stats::from_text(&s.to_text(1_764_000_000)).unwrap();
        assert_eq!((back, when), (s, 1_764_000_000));
    }

    #[test]
    fn words() {
        assert_eq!(bytes(5 * (1 << 30) + (1 << 29)), "5.5 GB");
        assert_eq!(bytes(3 << 30), "3 GB");
        assert_eq!(bytes(310 << 20), "310 MB");
        assert_eq!(spoken(3 * 86_400_000 + 4 * 3_600_000), "3 d 4 h");
        assert_eq!(spoken(5 * 3_600_000 + 12 * 60_000), "5 h 12 min");
        assert_eq!(spoken(40 * 60_000), "40 min");
        assert_eq!(share(1, 8), "12.5%");
        assert_eq!(share(1, 4000), "<0.1%");
        assert_eq!(share(1, 2), "50%");
    }
}
