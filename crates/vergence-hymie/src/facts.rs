//! A file's facts for a music library (AlbumWall's scan, docs/albumwall-milestone-1.md
//! step 2): its rate, channels, bits, length, ReplayGain, tags and the
//! size of its cover, from only the parts of the file that hold them.
//!
//! A library of thousands of tracks cannot send every byte of every file to
//! the helper (a first scan from a network share would take hours); the
//! tags and the facts sit at a file's ends. So the program reading a library
//! sends the head and the tail and the file's size, and this says either
//! the facts or how much more of each end it needs: a FLAC's metadata
//! blocks, an MP3's ID3 tags, an MP4's moov box, a WAV's chunks before and
//! after its samples. Nothing is read from the bytes in the program that
//! sends them: the walk to find what is needed runs here, in the helper.
//!
//! The decoders are opened on a buffer of the file's real size with its two
//! ends filled in and zeros between: their constructors read headers, not
//! samples, and are hardened against garbage in any case. An AAC track's
//! facts come from its container ([`crate::mp4::aac_facts`]).

use crate::{Error, Format, ReplayGain, Tags, format_of};

/// What a scan keeps of a file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Facts {
    pub rate: u32,
    pub channels: u16,
    /// Bits per sample as stored (0 for floating point; 16 for AAC).
    pub bits: u16,
    pub frames: u64,
    pub gain: ReplayGain,
    /// The words (title, artist, album and every value), without the picture.
    pub tags: Tags,
    /// A cover is kept in the file, and its size in pixels if its header
    /// could be read ((0, 0): unknown, never "small").
    pub cover: Option<(u32, u32)>,
}

impl Facts {
    pub fn seconds(&self) -> f64 {
        if self.rate == 0 { 0.0 } else { self.frames as f64 / self.rate as f64 }
    }
}

impl Facts {
    /// On the helper's pipe: the numbers, the cover, the words.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&self.rate.to_le_bytes());
        b.extend_from_slice(&self.channels.to_le_bytes());
        b.extend_from_slice(&self.bits.to_le_bytes());
        b.extend_from_slice(&self.frames.to_le_bytes());
        b.push(self.gain.present as u8);
        for v in [self.gain.track_gain, self.gain.track_peak, self.gain.album_gain, self.gain.album_peak] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let (has, w, h) = self.cover.map_or((0u8, 0, 0), |(w, h)| (1, w, h));
        b.push(has);
        b.extend_from_slice(&w.to_le_bytes());
        b.extend_from_slice(&h.to_le_bytes());
        let text = |b: &mut Vec<u8>, s: &str| {
            b.extend_from_slice(&(s.len() as u32).to_le_bytes());
            b.extend_from_slice(s.as_bytes());
        };
        for s in [&self.tags.title, &self.tags.artist, &self.tags.album] {
            text(&mut b, s.as_deref().unwrap_or(""));
        }
        let values: Vec<(&String, &String)> = self.tags.values.iter().take(MAX_VALUES).collect();
        b.extend_from_slice(&(values.len() as u32).to_le_bytes());
        for (k, v) in values {
            text(&mut b, k);
            text(&mut b, v);
        }
        b
    }

    pub(crate) fn decode(b: &[u8]) -> Option<Facts> {
        let mut r = Reader { b, at: 0 };
        let rate = r.u32()?;
        let channels = u16::from_le_bytes(r.take(2)?.try_into().ok()?);
        let bits = u16::from_le_bytes(r.take(2)?.try_into().ok()?);
        let frames = u64::from_le_bytes(r.take(8)?.try_into().ok()?);
        let present = r.take(1)?[0] != 0;
        let mut g = [0f32; 4];
        for v in &mut g {
            *v = f32::from_le_bytes(r.take(4)?.try_into().ok()?);
        }
        let has = r.take(1)?[0] != 0;
        let (w, h) = (r.u32()?, r.u32()?);
        let word = |s: String| Some(s).filter(|s| !s.is_empty());
        let mut tags = Tags::default();
        tags.title = word(r.text()?);
        tags.artist = word(r.text()?);
        tags.album = word(r.text()?);
        for _ in 0..(r.u32()? as usize).min(MAX_VALUES) {
            let (k, v) = (r.text()?, r.text()?);
            tags.values.insert(k, v);
        }
        Some(Facts {
            rate,
            channels,
            bits,
            frames,
            gain: ReplayGain { present, track_gain: g[0], track_peak: g[1], album_gain: g[2], album_peak: g[3] },
            tags,
            cover: has.then_some((w, h)),
        })
    }
}

/// The most tag values sent for one file.
const MAX_VALUES: usize = 512;

/// Reading an answer, a field at a time.
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

    fn text(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        Some(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

/// How much of each end of the file is wanted, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Want {
    pub head: u64,
    pub tail: u64,
}

/// What a scan sends first: enough for most files.
pub const HEAD: u64 = 256 << 10;
pub const TAIL: u64 = 128 << 10;
/// The most of either end asked for; past this the whole file is sent.
pub const MOST: u64 = 64 << 20;
/// Beyond what a structure needs, for what follows it (an MP3's first
/// frame and its Info tag after the ID3 tags).
const MARGIN: u64 = 16 << 10;

/// The facts of a file of `size` bytes from its first bytes (`head`) and
/// its last (`tail`; empty when `head` is the whole file), or what more is
/// wanted first.
pub fn facts(size: u64, head: &[u8], tail: &[u8]) -> Result<Result<Facts, Want>, Error> {
    if size > crate::MAX_FILE {
        return Err(Error::Unsupported("Larger than Hymie reads.".into()));
    }
    let whole = head.len() as u64 >= size;
    if !whole {
        if head.len() as u64 + tail.len() as u64 > size {
            return Err(crate::damaged("The ends overlap."));
        }
        if let Some(want) = wanted(size, head, tail)? {
            return Ok(Err(want));
        }
    }
    let data: Vec<u8> = if whole {
        head[..size as usize].to_vec()
    } else {
        let mut d = vec![0u8; size as usize];
        d[..head.len()].copy_from_slice(head);
        let from = size as usize - tail.len();
        d[from..].copy_from_slice(tail);
        d
    };
    let mut tags = crate::tags(&data);
    let cover = tags.take_picture().map(|p| image_size(&repair(&p.data)).unwrap_or((0, 0)));
    // AAC from its container (its decoder is not here); everything else by
    // opening it, the buffer handed over, not copied (a 2 GB WAV's zeros
    // twice would pass the helper's 4 GB).
    if format_of(&data) == Some(Format::Mp4)
        && let Some((rate, channels, frames)) = crate::mp4::aac_facts(&data)
    {
        let gain = ReplayGain::from_tags(&tags.values);
        return Ok(Ok(Facts { rate, channels, bits: 16, frames, gain, tags, cover }));
    }
    let t = crate::open(data)?;
    Ok(Ok(Facts {
        rate: t.rate(),
        channels: t.channels(),
        bits: t.bits(),
        frames: t.frames(),
        gain: t.gain(),
        tags,
        cover,
    }))
}

/// The two ends together, as far as they reach: the byte at `at`, if sent.
struct Ends<'a> {
    size: u64,
    head: &'a [u8],
    tail: &'a [u8],
}

impl Ends<'_> {
    /// `n` bytes at `at`, if they lie inside one of the ends.
    fn get(&self, at: u64, n: u64) -> Option<&[u8]> {
        let end = at.checked_add(n)?;
        if end <= self.head.len() as u64 {
            return Some(&self.head[at as usize..end as usize]);
        }
        let tail_from = self.size - self.tail.len() as u64;
        if at >= tail_from && end <= self.size {
            return Some(&self.tail[(at - tail_from) as usize..(end - tail_from) as usize]);
        }
        None
    }

    /// Whether the span [at, at + n) is all sent.
    fn has(&self, at: u64, n: u64) -> bool {
        n == 0 || self.get(at, n).is_some()
    }

    /// The ends grown to hold [at, at + n): the head if it starts in the
    /// file's first half, else the tail.
    fn want(&self, at: u64, n: u64) -> Want {
        let end = at.saturating_add(n).min(self.size);
        let (head, tail) = (self.head.len() as u64, self.tail.len() as u64);
        if at < self.size / 2 {
            Want { head: head.max(end + MARGIN).min(self.size), tail }
        } else {
            Want { head, tail: tail.max(self.size - at).min(self.size) }
        }
    }
}

/// None if the ends hold what this format's facts and tags need, else what
/// more is wanted. The walk reads only the structure's own lengths.
fn wanted(size: u64, head: &[u8], tail: &[u8]) -> Result<Option<Want>, Error> {
    let e = Ends { size, head, tail };
    // ID3v2 tags in front of the stream (MP3's, and some FLACs').
    // id3_size reads the tags' own lengths; a tag running past the head is
    // asked for whole (with room for the next one's header).
    let id3 = crate::tags::id3_size(head) as u64;
    if id3 > 0 && !e.has(0, id3 + 10) {
        return Ok(Some(e.want(0, id3 + 10)));
    }
    match format_of(head) {
        Some(Format::Flac) => Ok(flac_wanted(&e, id3)),
        Some(Format::Mp3) => Ok((!e.has(0, id3 + MARGIN)).then(|| e.want(0, id3 + MARGIN))),
        Some(Format::Mp4) => Ok(boxes_wanted(&e, 0, |t| t == b"moov")),
        Some(Format::Wav) => Ok(boxes_wanted(&e, 12, |t| t != b"data")),
        None => Err(Error::Unsupported("Not a sound file Hymie plays.".into())),
    }
}

/// FLAC: every metadata block from "fLaC" to the last.
fn flac_wanted(e: &Ends, from: u64) -> Option<Want> {
    let mut at = from + 4;
    loop {
        let Some(h) = e.get(at, 4) else { return Some(e.want(at, 4)) };
        let last = h[0] & 0x80 != 0;
        let len = (h[1] as u64) << 16 | (h[2] as u64) << 8 | h[3] as u64;
        if !e.has(at + 4, len) {
            return Some(e.want(at, 4 + len));
        }
        at += 4 + len;
        if last || at >= e.size {
            return None;
        }
    }
}

/// MP4's top-level boxes (the moov wanted whole) or a WAV's chunks (every
/// one but the samples wanted whole), from `start`: each box's header, and
/// the bodies `keep` names.
fn boxes_wanted(e: &Ends, start: u64, keep: impl Fn(&[u8]) -> bool) -> Option<Want> {
    let wav = start == 12;
    let mut at = start;
    let mut seen = 0;
    while at + 8 <= e.size {
        seen += 1;
        if seen > 1000 {
            return None;
        }
        let Some(h) = e.get(at, 8) else { return Some(e.want(at, 16)) };
        let kind = [h[4], h[5], h[6], h[7]];
        let (mut len, mut body_at) = if wav {
            // A RIFF chunk: its id, then a little-endian length (padded to even).
            let n = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as u64;
            (8 + n + (n & 1), at + 8)
        } else {
            (u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64, at + 8)
        };
        let id: [u8; 4] = if wav { [h[0], h[1], h[2], h[3]] } else { kind };
        if !wav && len == 1 {
            let Some(l) = e.get(at + 8, 8) else { return Some(e.want(at, 16)) };
            len = u64::from_be_bytes(l.try_into().unwrap_or([0; 8]));
            body_at = at + 16;
        } else if !wav && len == 0 {
            len = e.size - at;
        }
        if len < body_at - at {
            return None;
        }
        if keep(&id) && !e.has(at, len.min(e.size - at)) {
            return Some(e.want(at, len));
        }
        at = at.saturating_add(len);
    }
    None
}

/// A cover's size in pixels from its header: PNG, GIF, BMP, JPEG (AlbumWall's
/// ImageSize.Read). None if it is none of those or the size makes no sense.
pub fn image_size(d: &[u8]) -> Option<(u32, u32)> {
    let be32 = |i: usize| u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    let le32 = |i: usize| i32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    let sane = |w: i64, h: i64| ((1..65536).contains(&w) && (1..65536).contains(&h)).then_some((w as u32, h as u32));
    if d.len() >= 24 && d.starts_with(&[0x89, b'P', b'N', b'G']) {
        return sane(be32(16) as i64, be32(20) as i64);
    }
    if d.len() >= 10 && d.starts_with(b"GIF") {
        return sane((d[6] as u16 | (d[7] as u16) << 8) as i64, (d[8] as u16 | (d[9] as u16) << 8) as i64);
    }
    if d.len() >= 26 && d.starts_with(b"BM") {
        return sane(le32(18) as i64, (le32(22) as i64).abs());
    }
    if d.len() >= 4 && d[0] == 0xFF && d[1] == 0xD8 {
        let mut i = 2;
        while i + 9 < d.len() {
            if d[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = d[i + 1];
            if marker == 0xFF {
                i += 1;
                continue;
            }
            if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
                let h = (d[i + 5] as i64) << 8 | d[i + 6] as i64;
                let w = (d[i + 7] as i64) << 8 | d[i + 8] as i64;
                return sane(w, h);
            }
            if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
                i += 2;
                continue;
            }
            i += 2 + ((d[i + 2] as usize) << 8 | d[i + 3] as usize);
        }
    }
    None
}

/// A picture whose first bytes a tagger cut off, put back (AlbumWall's
/// ImageSize.Repair): a PNG from its IHDR, a JPEG from its JFIF or its
/// quantization table.
pub fn repair(d: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    use std::borrow::Cow;
    if d.len() < 16 {
        return Cow::Borrowed(d);
    }
    let with = |front: &[u8]| Cow::Owned([front, d].concat());
    if d[..7] == [0x00, 0x00, 0x0D, b'I', b'H', b'D', b'R'] {
        return with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00]);
    }
    if d[..6] == [0x10, b'J', b'F', b'I', b'F', 0x00] {
        return with(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]);
    }
    if d.len() > 68 && d[0] == 0x43 && d[1] == 0x00 && d[66] == 0xFF {
        return with(&[0xFF, 0xD8, 0xFF, 0xDB, 0x00]);
    }
    Cow::Borrowed(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flac() -> &'static [u8] {
        include_bytes!("../fixtures/tone16.flac")
    }

    /// The facts asked for from the ends, as a scan asks: more until enough.
    fn by_ends(data: &[u8]) -> (Facts, u32) {
        let size = data.len() as u64;
        let (mut head, mut tail) = (HEAD.min(size), TAIL.min(size.saturating_sub(HEAD)));
        for round in 0..6 {
            let h = &data[..head as usize];
            let t = if head >= size { &[][..] } else { &data[(size - tail) as usize..] };
            match facts(size, h, t).unwrap() {
                Ok(f) => return (f, round),
                Err(w) => {
                    (head, tail) = (w.head.min(size), w.tail.min(size - w.head.min(size)));
                }
            }
        }
        panic!("never enough");
    }

    #[test]
    fn a_whole_file_gives_the_same_facts_as_opening_it() {
        let d = flac();
        let f = facts(d.len() as u64, d, &[]).unwrap().unwrap();
        let t = crate::open(d.to_vec()).unwrap();
        assert_eq!((f.rate, f.channels, f.bits, f.frames), (t.rate(), t.channels(), t.bits(), t.frames()));
    }

    #[test]
    fn every_fixture_has_its_facts_whole_and_by_its_ends() {
        for (name, d) in [
            ("tone16.flac", &include_bytes!("../fixtures/tone16.flac")[..]),
            ("tone24.flac", &include_bytes!("../fixtures/tone24.flac")[..]),
            ("tone44.mp3", &include_bytes!("../fixtures/tone44.mp3")[..]),
            ("alac16.m4a", &include_bytes!("../fixtures/alac16.m4a")[..]),
            ("aac44.m4a", &include_bytes!("../fixtures/aac44.m4a")[..]),
        ] {
            let whole = facts(d.len() as u64, d, &[]).unwrap().unwrap();
            assert!(whole.rate > 0 && whole.frames > 0, "{name}: {whole:?}");
            if let Ok(t) = crate::open(d.to_vec()) {
                assert_eq!((whole.rate, whole.channels, whole.frames), (t.rate(), t.channels(), t.frames()), "{name}");
            }
            let (ends, _) = by_ends(d);
            assert_eq!(ends, whole, "{name}");
        }
    }

    #[test]
    fn an_aac_track_is_measured_from_its_container() {
        let d = include_bytes!("../fixtures/aac44.m4a");
        let f = facts(d.len() as u64, d, &[]).unwrap().unwrap();
        assert_eq!((f.rate, f.bits), (44100, 16));
        assert!(f.seconds() > 0.1 && f.seconds() < 60.0, "{}", f.seconds());
    }

    #[test]
    fn a_big_flac_is_read_from_its_ends() {
        // The tone with ten megabytes of zeros after its frames: the facts come
        // from the head, the frames never sent.
        let mut d = flac().to_vec();
        let t = crate::open(d.clone()).unwrap();
        d.resize(d.len() + (10 << 20), 0);
        let (f, rounds) = by_ends(&d);
        assert_eq!((f.rate, f.frames), (t.rate(), t.frames()));
        assert_eq!(rounds, 0);
    }

    #[test]
    fn a_long_metadata_chain_is_asked_for() {
        // A FLAC with a 600 KB PADDING block before the rest of its metadata:
        // more of the head is asked for, once.
        let d = flac();
        let mut big = d[..4].to_vec();
        let pad = 600 << 10;
        big.extend_from_slice(&[0x01, (pad >> 16) as u8, (pad >> 8) as u8, pad as u8]);
        big.resize(big.len() + pad, 0);
        big.extend_from_slice(&d[4..]);
        big.resize(big.len() + (4 << 20), 0);
        let (f, rounds) = by_ends(&big);
        assert_eq!(rounds, 1);
        assert!(f.frames > 0);
    }

    #[test]
    fn id3_gives_the_album_artist_track_disc_and_date() {
        let frame = |id: &[u8; 4], text: &str| {
            let mut f = id.to_vec();
            f.extend_from_slice(&(text.len() as u32 + 1).to_be_bytes());
            f.extend_from_slice(&[0, 0, 0]); // flags, then Latin-1
            f.extend_from_slice(text.as_bytes());
            f
        };
        let body: Vec<u8> =
            [frame(b"TPE2", "Toto"), frame(b"TRCK", "3/10"), frame(b"TPOS", "2/2"), frame(b"TYER", "1982")].concat();
        let n = body.len() as u32;
        let mut tag = b"ID3\x03\x00\x00".to_vec();
        tag.extend_from_slice(&[
            (n >> 21 & 0x7F) as u8,
            (n >> 14 & 0x7F) as u8,
            (n >> 7 & 0x7F) as u8,
            (n & 0x7F) as u8,
        ]);
        tag.extend_from_slice(&body);
        let mut t = Tags::default();
        crate::tags::read_id3(&tag, 0, &mut t);
        let v = |k: &str| t.values.get(k).cloned();
        assert_eq!(v("ALBUMARTIST").as_deref(), Some("Toto"));
        assert_eq!(v("TRACKNUMBER").as_deref(), Some("3/10"));
        assert_eq!(v("DISCNUMBER").as_deref(), Some("2/2"));
        assert_eq!(v("DATE").as_deref(), Some("1982"));
    }

    #[test]
    fn facts_cross_the_pipe_unchanged() {
        let d = flac();
        let mut f = facts(d.len() as u64, d, &[]).unwrap().unwrap();
        f.cover = Some((600, 598));
        f.tags.values.insert("ALBUMARTIST".into(), "Toto".into());
        assert_eq!(Facts::decode(&f.encode()), Some(f));
        assert_eq!(Facts::decode(&[1, 2, 3]), None);
    }

    #[test]
    fn image_sizes_come_from_headers() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        png.extend_from_slice(&600u32.to_be_bytes());
        png.extend_from_slice(&598u32.to_be_bytes());
        png.extend_from_slice(&[8, 2, 0, 0, 0, 0xDE, 0xAD, 0xBE, 0xEF]); // depth, color, ..., the CRC
        assert_eq!(image_size(&png), Some((600, 598)));
        // A JPEG: SOI, an APP0 of 16 bytes, then SOF0 with 500 x 400.
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        jpg.extend_from_slice(&[0; 14]);
        jpg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0x90, 0x01, 0xF4, 0x03]);
        assert_eq!(image_size(&jpg), Some((500, 400)));
        assert_eq!(image_size(b"not a picture at all"), None);
        // A PNG missing its first nine bytes is put back.
        assert_eq!(image_size(&repair(&png[9..])), Some((600, 598)));
    }
}
