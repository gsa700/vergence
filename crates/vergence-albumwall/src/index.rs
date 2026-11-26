//! The index: what a scan read of each file, so a launch with nothing
//! changed opens no file (the Avalonia app's LibraryIndex; question 4: our
//! own file, no SQLite).
//!
//! **It is only a cache.** A file counts as unchanged when its path, size
//! and modified time all match what is kept; Rescan reads every file and
//! rebuilds it. Pictures beside the tracks are kept the same way (their
//! size in pixels). One file per library under
//! `~/.cache/vergence/albumwall/`, written whole and renamed over the old
//! one; a file of another version, or damaged, is read as empty (the next
//! scan reads everything again, which is all a lost cache costs).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::library::TrackFacts;

const MAGIC: &[u8; 4] = b"AWIX";
/// Up when what a scan keeps of a file changes, or what Hymie reads of one
/// (a file refused before may be read now): every file is read again.
/// 2: MP3s whose ID3 tag ended just inside the first 256 KB were refused.
/// 3: AAC tracks said 16 bits (lossy files keep no depth).
pub const VERSION: u32 = 3;

/// A file's facts and the size and time they were read at.
#[derive(Debug, Clone, PartialEq)]
pub struct Kept {
    pub size: u64,
    pub modified: i64,
    pub facts: TrackFacts,
}

/// A picture's size in pixels and the size and time it was measured at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeptArt {
    pub size: u64,
    pub modified: i64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Index {
    pub tracks: HashMap<PathBuf, Kept>,
    pub art: HashMap<PathBuf, KeptArt>,
}

impl Index {
    /// A file's facts if it is unchanged.
    pub fn find(&self, path: &Path, size: u64, modified: i64) -> Option<&TrackFacts> {
        self.tracks.get(path).filter(|k| k.size == size && k.modified == modified).map(|k| &k.facts)
    }

    pub fn find_art(&self, path: &Path, size: u64, modified: i64) -> Option<(u32, u32)> {
        self.art.get(path).filter(|k| k.size == size && k.modified == modified).map(|k| (k.width, k.height))
    }

    /// Whether any file under `root` is kept (a library that held music last time).
    pub fn has_files_under(&self, root: &Path) -> bool {
        self.tracks.keys().any(|p| p.starts_with(root))
    }

    /// The file's bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(64 + self.tracks.len() * 160);
        b.extend_from_slice(MAGIC);
        b.extend_from_slice(&VERSION.to_le_bytes());
        b.extend_from_slice(&(self.tracks.len() as u64).to_le_bytes());
        for (path, k) in &self.tracks {
            let f = &k.facts;
            text(&mut b, &path.to_string_lossy());
            b.extend_from_slice(&k.size.to_le_bytes());
            b.extend_from_slice(&k.modified.to_le_bytes());
            b.push(f.readable as u8);
            for s in [&f.album_artist, &f.album, &f.title, &f.artist] {
                text(&mut b, s);
            }
            b.extend_from_slice(&f.tag_year.to_le_bytes());
            b.extend_from_slice(&f.disc.to_le_bytes());
            b.extend_from_slice(&f.number.to_le_bytes());
            b.extend_from_slice(&f.millis.to_le_bytes());
            b.extend_from_slice(&f.rate.to_le_bytes());
            b.extend_from_slice(&f.bits.to_le_bytes());
            b.extend_from_slice(&f.bitrate.to_le_bytes());
            let (has, w, h) = f.cover.map_or((0u8, 0, 0), |(w, h)| (1, w, h));
            b.push(has);
            b.extend_from_slice(&w.to_le_bytes());
            b.extend_from_slice(&h.to_le_bytes());
        }
        b.extend_from_slice(&(self.art.len() as u64).to_le_bytes());
        for (path, a) in &self.art {
            text(&mut b, &path.to_string_lossy());
            b.extend_from_slice(&a.size.to_le_bytes());
            b.extend_from_slice(&a.modified.to_le_bytes());
            b.extend_from_slice(&a.width.to_le_bytes());
            b.extend_from_slice(&a.height.to_le_bytes());
        }
        b
    }

    /// From the file's bytes; empty if they are of another version or damaged.
    pub fn decode(b: &[u8]) -> Index {
        Index::try_decode(b).unwrap_or_default()
    }

    fn try_decode(b: &[u8]) -> Option<Index> {
        let mut r = Reader { b, at: 0 };
        if r.take(4)? != MAGIC || r.u32()? != VERSION {
            return None;
        }
        let mut index = Index::default();
        let n = r.u64()?;
        for _ in 0..n {
            let path = PathBuf::from(r.text()?);
            let size = r.u64()?;
            let modified = r.u64()? as i64;
            let readable = r.take(1)?[0] != 0;
            let (album_artist, album, title, artist) = (r.text()?, r.text()?, r.text()?, r.text()?);
            let tag_year = r.u32()? as i32;
            let (disc, number) = (r.u32()?, r.u32()?);
            let millis = r.u64()?;
            let rate = r.u32()?;
            let bits = u16::from_le_bytes(r.take(2)?.try_into().ok()?);
            let bitrate = r.u32()?;
            let has = r.take(1)?[0] != 0;
            let (w, h) = (r.u32()?, r.u32()?);
            let facts = TrackFacts {
                readable,
                album_artist,
                album,
                tag_year,
                disc,
                number,
                title,
                artist,
                millis,
                rate,
                bits,
                bitrate,
                cover: has.then_some((w, h)),
            };
            index.tracks.insert(path, Kept { size, modified, facts });
        }
        let n = r.u64()?;
        for _ in 0..n {
            let path = PathBuf::from(r.text()?);
            let (size, modified) = (r.u64()?, r.u64()? as i64);
            let (width, height) = (r.u32()?, r.u32()?);
            index.art.insert(path, KeptArt { size, modified, width, height });
        }
        (r.at == b.len()).then_some(index)
    }

    pub fn load(path: &Path) -> Index {
        std::fs::read(path).map(|b| Index::decode(&b)).unwrap_or_default()
    }

    /// Written whole: a new file beside it renamed over it.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("new");
        std::fs::write(&tmp, self.encode())?;
        std::fs::rename(&tmp, path)
    }
}

/// Where a library's index is kept: `~/.cache/vergence/albumwall/` (or
/// under `$XDG_CACHE_HOME`), named by its folder.
pub fn path_for(folder: &Path) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    // FNV-1a of the folder's path: a name of its own for each library.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in folder.to_string_lossy().bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(base.join(format!("vergence/albumwall/index-{hash:016x}")))
}

fn text(b: &mut Vec<u8>, s: &str) {
    b.extend_from_slice(&(s.len() as u32).to_le_bytes());
    b.extend_from_slice(s.as_bytes());
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn text(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        Some(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Index {
        let mut i = Index::default();
        let facts = TrackFacts {
            readable: true,
            album_artist: "Toto".into(),
            album: "IV".into(),
            tag_year: 1982,
            disc: 1,
            number: 3,
            title: "Rosanna".into(),
            artist: "Toto".into(),
            millis: 331_000,
            rate: 44100,
            bits: 16,
            bitrate: 900,
            cover: Some((600, 600)),
        };
        i.tracks.insert("/m/Toto/1982 - IV/03.flac".into(), Kept { size: 30_000_000, modified: 17, facts });
        i.tracks.insert("/m/x/broken.mp3".into(), Kept { size: 3, modified: 4, facts: TrackFacts::unreadable() });
        i.art.insert("/m/Toto/1982 - IV/back.jpg".into(), KeptArt { size: 5, modified: 6, width: 1270, height: 1000 });
        i
    }

    #[test]
    fn the_index_round_trips() {
        let i = sample();
        assert_eq!(Index::decode(&i.encode()), i);
    }

    #[test]
    fn a_changed_file_is_not_found() {
        let i = sample();
        let p = Path::new("/m/Toto/1982 - IV/03.flac");
        assert!(i.find(p, 30_000_000, 17).is_some());
        assert!(i.find(p, 30_000_000, 18).is_none());
        assert!(i.find(p, 30_000_001, 17).is_none());
        assert!(i.has_files_under(Path::new("/m/Toto")));
        assert!(!i.has_files_under(Path::new("/other")));
    }

    #[test]
    fn another_version_or_a_damaged_file_reads_as_empty() {
        let mut b = sample().encode();
        assert_eq!(Index::decode(&b[..b.len() - 1]), Index::default());
        b[4] = 99;
        assert_eq!(Index::decode(&b), Index::default());
        assert_eq!(Index::decode(b"nonsense"), Index::default());
    }
}
