//! One ALAC or AAC track in an MP4 (.m4a) open for playing, and any MP4's
//! tags: Hymie's Mp4Track.cs carried over, around the codec cores (alac.rs
//! here; AAC's in crates/vergence-hymie-aac, behind [`FrameDecoder`]).
//!
//! The MP4 is read for what playing needs and nothing else: the first audio
//! track's sample table (where each frame is and how long), its decoder
//! configuration (the 'alac' box's magic cookie for ALAC, the esds for AAC),
//! its edit list, and the iTunes tags that carry the gapless information and
//! ReplayGain. An ALAC frame is as long as its cookie says, 4096 as a rule,
//! the last one shorter; an AAC frame is 1024 samples. AAC (FAAD2, GPL) has
//! its own crate and helper: this crate holds no AAC code, and an AAC track
//! is played only when the caller hands in a decoder ([`MakeAac`], which only
//! the AAC helper does); otherwise it is refused with [`Error::NeedsAac`],
//! and its tags and cover are still read for a preview.
//!
//! GAPLESS, AS FFMPEG DOES IT (libavformat/mov.c, n9.0.2, which is what mpv
//! played through), so a record plays the same on either engine. An encoder
//! starts with priming samples and ends with padding:
//!   * the edit list, when there is one, says where the music starts in the
//!     media (its media time: so many samples are dropped) and how long it is;
//!   * iTunes' iTunSMPB tag says the same in its own words, priming, padding
//!     and length, and its priming is what is dropped when it is there;
//!   * its padding counts only when nothing else has cut the length down: with
//!     an edit list that already ends at the music, it is not cut twice.
//!
//! With neither, nothing is dropped. The length is the track's own, the
//! shorter of its header's and its sample table's.
//!
//! SEEKS ARE EXACT. An ALAC frame depends on nothing before it, so decoding
//! starts at the frame that holds the sample. An AAC frame depends on the
//! one before it only through the overlap of the transform, so decoding
//! starts one frame before (the decoder's preroll), from nothing, and from
//! there the samples are a straight decode's (but for PNS noise, random by
//! design).
//!
//! What changed in the carrying: the whole file is in memory (no TrackBytes
//! waiting for it to arrive); every read of a box is bounds-checked (a field
//! past the file reads as zero); the box walk has a nesting limit and a box
//! count limit, and going past either is an error (Marginal's security review
//! 2026-10-31, A1: a crafted file nested 3,000 deep overflowed the stack; the
//! C#'s MaxDepth guarded Walk but not the 'wave' boxes inside a sample
//! description); sample sizes are unsigned (a size of 2^31 or more was
//! negative in the C#); a sample-to-chunk run of no samples is passed over (in
//! the C# such runs made the table's build quadratic) and a first chunk of 0
//! does not index before the chunk table.

use crate::alac::AlacDecoder;
use crate::tags::Tags;
use crate::{Error, ReplayGain, Track, damaged};

/// A codec core that turns an MP4's frames (its samples, in MP4's words)
/// into sound, one at a time: ALAC's here, AAC's in vergence-hymie-aac. It
/// says nothing of how a codec works, so the MP4 reader can serve a decoder
/// it does not contain (and whose license it does not share).
pub trait FrameDecoder: Send {
    fn rate(&self) -> u32;
    fn channels(&self) -> u16;
    /// Bits per sample as stored; 0 for a lossy codec.
    fn bits(&self) -> u16 {
        0
    }
    /// Samples (per channel) in a whole frame: 1024 for AAC.
    fn frame_length(&self) -> usize;
    /// Frames decoded and thrown away before a seek's own, so that the
    /// first one kept is exact: 1 for AAC (the transform's overlap), 0 for ALAC.
    fn preroll(&self) -> usize {
        0
    }
    /// One frame's bytes into `out` (frame_length * channels floats from -1
    /// to 1, interleaved). Returns the samples per channel written: a whole
    /// frame as a rule, fewer in a short last frame. An `Unsupported` error
    /// on the first frame refuses the track (what shows only inside the
    /// frames, SBR in an AAC fill element for one); anything else is a
    /// damaged frame, played as silence.
    fn decode(&mut self, frame: &[u8], out: &mut [f32]) -> Result<usize, Error>;
    /// Forgets the past (the overlap from frame to frame), for a seek.
    fn reset(&mut self);
}

/// Makes the decoder for an MP4's AAC track from its AudioSpecificConfig
/// (ISO 14496-3 1.6.2.1, the bytes of the esds' DecoderSpecificInfo).
pub type MakeAac<'a> = &'a dyn Fn(&[u8]) -> Result<Box<dyn FrameDecoder>, Error>;

/// ALAC behind the trait: every frame stands alone.
struct Alac(AlacDecoder);

impl FrameDecoder for Alac {
    fn rate(&self) -> u32 {
        self.0.sample_rate
    }
    fn channels(&self) -> u16 {
        self.0.channels as u16
    }
    fn bits(&self) -> u16 {
        self.0.bit_depth as u16
    }
    fn frame_length(&self) -> usize {
        self.0.frame_length
    }
    fn decode(&mut self, frame: &[u8], out: &mut [f32]) -> Result<usize, Error> {
        self.0.decode_frame(frame, out)
    }
    fn reset(&mut self) {}
}

/// The largest frame a decoder may claim (ALAC's cookie says its own; real
/// ones are 4096): more is a crafted file asking for memory.
const MAX_FRAME_LENGTH: usize = 1 << 20;

/// How deep boxes are followed. A real file goes six or seven deep
/// (moov/udta/meta/ilst/©nam/data, or moov/trak/mdia/minf/stbl/stsd/alac,
/// and a 'wave' or two inside that); deeper is a crafted file.
const MAX_DEPTH: u32 = 16;

/// How many boxes are looked at in all: a real moov has a few hundred.
const MAX_BOXES: u32 = 100_000;

/// A field of the file, read big-endian; zero where the file ends (where the
/// C# would have thrown).
fn be16(d: &[u8], i: u64) -> u32 {
    let b = |k: u64| i.checked_add(k).and_then(|j| d.get(usize::try_from(j).ok()?)).copied().unwrap_or(0) as u32;
    b(0) << 8 | b(1)
}

fn be32(d: &[u8], i: u64) -> u32 {
    be16(d, i) << 16 | be16(d, i.saturating_add(2))
}

fn be64(d: &[u8], i: u64) -> u64 {
    (be32(d, i) as u64) << 32 | be32(d, i.saturating_add(4)) as u64
}

fn byte(d: &[u8], i: u64) -> u8 {
    usize::try_from(i).ok().and_then(|j| d.get(j)).copied().unwrap_or(0)
}

/// The bytes of [from, to), clamped to the file.
fn span(d: &[u8], from: u64, to: u64) -> &[u8] {
    let len = d.len() as u64;
    let (a, b) = (from.min(len) as usize, to.min(len) as usize);
    if a < b { &d[a..b] } else { &[] }
}

/// a * b / c, rounded to nearest with halves away from zero, exactly:
/// FFmpeg's av_rescale, which a double would round differently at times.
fn rescale(a: i64, b: i64, c: i64) -> i64 {
    if c <= 0 {
        return a;
    }
    let (a, b, c) = (a as i128, b as i128, c as i128);
    let r = (a * b + if a < 0 { -(c / 2) } else { c / 2 }) / c;
    r.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

/// What the walk collects (Mp4Track.cs's Movie): the movie's and the first
/// audio track's facts, and the tags.
#[derive(Default)]
struct Movie {
    movie_timescale: i64,
    track_timescale: i64,
    mdhd_duration: i64,
    stts_total: i64,
    edit_media_time: Option<i64>,
    edits_duration: i64,
    asc: Option<Vec<u8>>,
    alac_cookie: Option<Vec<u8>>,
    sizes: Option<Vec<u32>>,
    fixed_size: u32,
    sample_count: u32,
    chunk_offsets: Option<Vec<u64>>,
    stsc: Option<Vec<(u32, u32)>>,
    smpb: Option<(i32, i32, i64)>,
    have_audio: bool,
    tags: Tags,
}

/// The walk's state: the file, and the boxes looked at so far.
struct Walker<'a> {
    d: &'a [u8],
    boxes: u32,
}

impl Walker<'_> {
    /// A box's size, type and header length at `at`, the size as the file
    /// says (0 means "to the end").
    fn box_header(&mut self, at: u64, end: u64) -> Result<(u64, [u8; 4], u64), Error> {
        self.boxes += 1;
        if self.boxes > MAX_BOXES {
            return Err(damaged("Too many boxes in this MP4."));
        }
        let mut size = be32(self.d, at) as u64;
        let t = [byte(self.d, at + 4), byte(self.d, at + 5), byte(self.d, at + 6), byte(self.d, at + 7)];
        let mut header = 8;
        if size == 1 {
            size = be64(self.d, at + 8);
            header = 16;
        } else if size == 0 {
            size = end.saturating_sub(at);
        }
        Ok((size, t, header))
    }

    /// The moov box, wherever it is among the top-level boxes.
    fn find_moov(&mut self) -> Result<(u64, u64), Error> {
        let len = self.d.len() as u64;
        let mut at = 0u64;
        loop {
            if at.saturating_add(8) > len {
                return Err(damaged("No moov box in this MP4."));
            }
            let (size, t, header) = self.box_header(at, len)?;
            if size < header {
                return Err(damaged("A broken MP4 box."));
            }
            if &t == b"moov" {
                return Ok((at + header, len.min(at.saturating_add(size))));
            }
            at = at.saturating_add(size);
        }
    }

    /// A full box's entry count (after its version and flags), when that
    /// many entries of `entry` bytes fit in the box; a broken file otherwise.
    fn count(&self, body: u64, body_end: u64, entry: u64) -> Result<usize, Error> {
        let count = be32(self.d, body + 4) as u64;
        if 8 + count * entry > body_end.saturating_sub(body) {
            return Err(damaged("A broken MP4 sample table."));
        }
        Ok(count as usize)
    }

    /// The boxes inside [start, end), those that matter, recursively. `trak`
    /// says whether this is inside a track, whose boxes go to `movie` (a
    /// track's own Movie, kept only if it is the first audio track).
    fn walk(&mut self, start: u64, end: u64, movie: &mut Movie, trak: bool, depth: u32) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            return Err(damaged("MP4 boxes nested too deep."));
        }
        let d = self.d;
        let mut at = start;
        while at.saturating_add(8) <= end {
            let (size, t, header) = self.box_header(at, end)?;
            if size < header || at.saturating_add(size) > end {
                break;
            }
            let (body, body_end) = (at + header, at + size);
            match &t {
                b"mvhd" => {
                    let version = byte(d, body);
                    movie.movie_timescale = be32(d, body + if version == 1 { 20 } else { 12 }) as i64;
                }
                b"trak" => {
                    if !movie.have_audio {
                        let mut t = Movie { movie_timescale: movie.movie_timescale, ..Movie::default() };
                        let walked = self.walk(body, body_end, &mut t, true, depth + 1);
                        if t.have_audio && (t.asc.is_some() || t.alac_cookie.is_some()) {
                            movie.have_audio = true;
                            movie.track_timescale = t.track_timescale;
                            movie.mdhd_duration = t.mdhd_duration;
                            movie.stts_total = t.stts_total;
                            movie.edit_media_time = t.edit_media_time;
                            movie.edits_duration = t.edits_duration;
                            movie.asc = t.asc;
                            movie.alac_cookie = t.alac_cookie;
                            movie.sizes = t.sizes;
                            movie.fixed_size = t.fixed_size;
                            movie.sample_count = t.sample_count;
                            movie.chunk_offsets = t.chunk_offsets;
                            movie.stsc = t.stsc;
                        }
                        walked?;
                    }
                }
                b"mdia" | b"minf" | b"stbl" | b"edts" | b"udta" | b"ilst" => {
                    self.walk(body, body_end, movie, trak, depth + 1)?
                }
                b"meta" => {
                    // A full box in MP4 (version and flags first), not in QuickTime.
                    let from = if be32(d, body) == 0 { body + 4 } else { body };
                    self.walk(from, body_end, movie, trak, depth + 1)?;
                }
                b"mdhd" if trak => {
                    let version = byte(d, body);
                    movie.track_timescale = be32(d, body + if version == 1 { 20 } else { 12 }) as i64;
                    movie.mdhd_duration =
                        if version == 1 { be64(d, body + 24) as i64 } else { be32(d, body + 16) as i64 };
                }
                b"hdlr" if trak => {
                    if span(d, body + 8, body + 12) == b"soun" {
                        movie.have_audio = true;
                    }
                }
                b"elst" if trak => {
                    let version = byte(d, body);
                    let count = be32(d, body + 4);
                    let step = if version == 1 { 20 } else { 12 };
                    let mut e = body + 8;
                    let mut found = false;
                    let mut i = 0;
                    while i < count && e + step <= body_end {
                        let (segment, media_time) = if version == 1 {
                            (be64(d, e) as i64, be64(d, e + 8) as i64)
                        } else {
                            (be32(d, e) as i64, be32(d, e + 4) as i32 as i64)
                        };
                        e += step;
                        movie.edits_duration = movie.edits_duration.wrapping_add(segment);
                        if !found && media_time != -1 {
                            movie.edit_media_time = Some(media_time);
                            found = true;
                        }
                        i += 1;
                    }
                }
                b"stsd" if trak => self.sample_descriptions(body + 8, body_end, movie, depth + 1)?,
                b"stts" if trak => {
                    let count = be32(d, body + 4) as u64;
                    let mut total: i64 = 0;
                    let (mut i, mut e) = (0, body + 8);
                    while i < count && e + 8 <= body_end {
                        total = total.wrapping_add(be32(d, e) as i64 * be32(d, e + 4) as i64);
                        i += 1;
                        e += 8;
                    }
                    movie.stts_total = total;
                }
                b"stsz" if trak => {
                    // Every count in the sample tables is held to the bytes
                    // there are before anything is made from it (Hymie's
                    // security review 2026-10-26, finding 3: a 148-byte file
                    // claiming 2^31 samples asked for 8 GB).
                    let fixed_size = be32(d, body + 4) as u64;
                    let sample_count = be32(d, body + 8) as u64;
                    let len = d.len() as u64;
                    if fixed_size > i32::MAX as u64
                        || sample_count > len
                        || (fixed_size == 0 && 12 + 4 * sample_count > body_end - body)
                        || sample_count * fixed_size > len
                    {
                        return Err(damaged("A broken MP4 sample size table."));
                    }
                    movie.fixed_size = fixed_size as u32;
                    movie.sample_count = sample_count as u32;
                    movie.sizes = Some(if fixed_size == 0 {
                        (0..sample_count).map(|i| be32(d, body + 12 + i * 4)).collect()
                    } else {
                        Vec::new()
                    });
                }
                b"stsc" if trak => {
                    let count = self.count(body, body_end, 12)? as u64;
                    movie.stsc =
                        Some((0..count).map(|i| (be32(d, body + 8 + i * 12), be32(d, body + 12 + i * 12))).collect());
                }
                b"stco" | b"co64" if trak => {
                    let wide = &t == b"co64";
                    let count = self.count(body, body_end, if wide { 8 } else { 4 })? as u64;
                    movie.chunk_offsets = Some(
                        (0..count)
                            .map(|i| if wide { be64(d, body + 8 + i * 8) } else { be32(d, body + 8 + i * 4) as u64 })
                            .collect(),
                    );
                }
                b"----" => self.freeform(body, body_end, movie),
                // The tags a preview shows (Hymie read them with TagLib#).
                b"\xa9nam" => self.text_item(body, body_end, movie, "TITLE"),
                b"\xa9ART" => self.text_item(body, body_end, movie, "ARTIST"),
                b"\xa9alb" => self.text_item(body, body_end, movie, "ALBUM"),
                b"covr" => self.cover(body, body_end, movie),
                _ => {}
            }
            at += size;
        }
        Ok(())
    }

    /// The first audio sample description: an mp4a's esds, for its
    /// AudioSpecificConfig, or an alac's magic cookie.
    fn sample_descriptions(&mut self, start: u64, end: u64, trak: &mut Movie, depth: u32) -> Result<(), Error> {
        let mut at = start;
        while at.saturating_add(8) <= end {
            let (size, t, header) = self.box_header(at, end)?;
            if size < header || at.saturating_add(size) > end {
                break;
            }
            if &t == b"mp4a" || &t == b"alac" {
                let body = at + header;
                let version = be16(self.d, body + 8); // QuickTime sound description version
                let children = body
                    + 28
                    + match version {
                        1 => 16,
                        2 => 36,
                        _ => 0,
                    };
                return if &t == b"mp4a" {
                    self.esds(children, at + size, trak, depth + 1)
                } else {
                    self.alac_cookie(children, at + size, trak, depth + 1)
                };
            }
            at += size;
        }
        Ok(())
    }

    /// The ALAC magic cookie (its ALACSpecificConfig, and a channel layout
    /// after it when there is one): in an 'alac' full box after the sample
    /// entry, or, as older encoders wrote it, in one inside a 'wave' box
    /// (ALACMagicCookieDescription.txt). 'wave' inside 'wave' is followed
    /// only to the nesting limit (the C# followed it without one).
    fn alac_cookie(&mut self, start: u64, end: u64, trak: &mut Movie, depth: u32) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            return Err(damaged("MP4 boxes nested too deep."));
        }
        let mut at = start;
        while at.saturating_add(8) <= end {
            let (size, t, header) = self.box_header(at, end)?;
            if size < header || at.saturating_add(size) > end {
                break;
            }
            if &t == b"wave" {
                return self.alac_cookie(at + header, at + size, trak, depth + 1);
            }
            if &t == b"alac" {
                let body = at + header + 4; // version and flags
                if body < at + size {
                    trak.alac_cookie = Some(span(self.d, body, at + size).to_vec());
                }
                return Ok(());
            }
            at += size;
        }
        Ok(())
    }

    fn esds(&mut self, start: u64, end: u64, trak: &mut Movie, depth: u32) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            return Err(damaged("MP4 boxes nested too deep."));
        }
        let d = self.d;
        let mut at = start;
        while at.saturating_add(8) <= end {
            let (size, t, header) = self.box_header(at, end)?;
            if size < header || at.saturating_add(size) > end {
                break;
            }
            if &t == b"wave" {
                return self.esds(at + header, at + size, trak, depth + 1);
            }
            if &t == b"esds" {
                let mut p = at + header + 4; // version and flags
                let e = at + size;
                if descriptor(d, &mut p, e).0 != Some(0x03) {
                    return Ok(());
                }
                p += 2; // ES_ID
                let flags = byte(d, p);
                p += 1;
                if flags & 0x80 != 0 {
                    p += 2;
                }
                if flags & 0x40 != 0 {
                    p += 1 + byte(d, p) as u64;
                }
                if flags & 0x20 != 0 {
                    p += 2;
                }
                if descriptor(d, &mut p, e).0 != Some(0x04) {
                    return Ok(());
                }
                let oti = byte(d, p);
                if !matches!(oti, 0x40 | 0x66 | 0x67 | 0x68) {
                    return Ok(()); // MPEG-4 audio, MPEG-2 AAC
                }
                p += 13;
                let (tag, length) = descriptor(d, &mut p, e);
                if tag != Some(0x05) || p + length as u64 > e {
                    return Ok(());
                }
                trak.asc = Some(span(d, p, p + length as u64).to_vec());
                return Ok(());
            }
            at += size;
        }
        Ok(())
    }

    /// A freeform iTunes tag (mean, name, data), as FFmpeg's mov_read_custom
    /// reads one: iTunSMPB for gapless, the rest kept by name (ReplayGain).
    fn freeform(&self, start: u64, end: u64, movie: &mut Movie) {
        let d = self.d;
        let (mut mean, mut key, mut value): (Option<String>, Option<String>, Option<String>) = (None, None, None);
        let mut at = start;
        let mut i = 0;
        while i < 3 && end.saturating_sub(at) > 12 {
            let len = be32(d, at) as u64;
            let tag = span(d, at + 4, at + 8);
            let mut p = at + 12; // length, tag, version and flags
            if len < 12 || at + len > end {
                break;
            }
            let mut n = len - 12;
            if tag == b"data" && n > 4 {
                p += 4;
                n -= 4;
            } else if tag != b"mean" && tag != b"name" {
                break;
            }
            let text = String::from_utf8_lossy(span(d, p, p + n)).into_owned();
            match tag {
                b"mean" => mean = Some(text),
                b"name" => key = Some(text),
                _ => value = Some(text),
            }
            at += len;
            i += 1;
        }
        let (Some(_), Some(key), Some(value)) = (mean, key, value) else { return };
        if key == "iTunSMPB" {
            // int.TryParse(..., HexNumber): eight hex digits, the top bit the sign.
            let parts: Vec<&str> = value.split(' ').filter(|p| !p.is_empty()).collect();
            if parts.len() >= 4
                && let (Ok(priming), Ok(remainder), Ok(samples)) = (
                    u32::from_str_radix(parts[1], 16),
                    u32::from_str_radix(parts[2], 16),
                    u64::from_str_radix(parts[3], 16),
                )
            {
                movie.smpb = Some((priming as i32, remainder as i32, samples as i64));
            }
        }
        if key != "cdec" {
            movie.tags.put_value(&key, value);
        }
    }

    /// The 'data' boxes inside an ilst item: their type (1 UTF-8, 2 UTF-16,
    /// 13 JPEG, 14 PNG, 0 implicit) and value.
    fn data_atoms(&self, start: u64, end: u64) -> Vec<(u32, &[u8])> {
        let d = self.d;
        let mut out = Vec::new();
        let mut at = start;
        while at.saturating_add(16) <= end {
            let len = be32(d, at) as u64;
            if len < 16 || at.saturating_add(len) > end {
                break;
            }
            if span(d, at + 4, at + 8) == b"data" {
                // type (version and flags), locale, value
                out.push((be32(d, at + 8) & 0x00FF_FFFF, span(d, at + 16, at + len)));
            }
            at += len;
        }
        out
    }

    fn text_item(&self, start: u64, end: u64, movie: &mut Movie, which: &str) {
        for (kind, value) in self.data_atoms(start, end) {
            let text = match kind {
                1 => String::from_utf8_lossy(value).into_owned(),
                2 => String::from_utf16_lossy(
                    &value.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect::<Vec<_>>(),
                ),
                _ => continue,
            };
            movie.tags.put_text(which, text);
            return;
        }
    }

    /// covr: the cover, by the data type (13 JPEG, 14 PNG), or by its first
    /// bytes when the type says nothing (0); always the front cover.
    fn cover(&self, start: u64, end: u64, movie: &mut Movie) {
        for (kind, value) in self.data_atoms(start, end) {
            let mime = match kind {
                13 => "image/jpeg",
                14 => "image/png",
                0 if value.starts_with(&[0xFF, 0xD8, 0xFF]) => "image/jpeg",
                0 if value.starts_with(b"\x89PNG") => "image/png",
                _ => continue,
            };
            movie.tags.put_picture(3, mime.to_string(), value);
        }
    }
}

/// An MPEG-4 descriptor's tag and length (up to four bytes of seven bits);
/// no tag where the box ends.
fn descriptor(d: &[u8], p: &mut u64, end: u64) -> (Option<u8>, u32) {
    let mut length = 0u32;
    if *p >= end {
        return (None, 0);
    }
    let tag = byte(d, *p);
    *p += 1;
    let mut i = 0;
    while i < 4 && *p < end {
        let b = byte(d, *p);
        *p += 1;
        length = length << 7 | (b & 0x7F) as u32;
        if b & 0x80 == 0 {
            break;
        }
        i += 1;
    }
    (Some(tag), length)
}

/// The movie's facts and tags, as far as they could be read: an error stops
/// the walk where it happened, and what came before it is kept.
fn parse(d: &[u8]) -> (Movie, Result<(), Error>) {
    let mut w = Walker { d, boxes: 0 };
    let mut movie = Movie::default();
    let result = w.find_moov().and_then(|(start, end)| w.walk(start, end, &mut movie, false, 0));
    (movie, result)
}

/// Where every frame is, from the chunk offsets and the sample-to-chunk runs.
fn sample_table(m: &Movie, chunks: &[u64], sizes: &[u32]) -> (Vec<u64>, Vec<u32>) {
    let count = if m.fixed_size != 0 { m.sample_count as usize } else { sizes.len() };
    let mut offsets = Vec::with_capacity(count);
    let mut out_sizes = Vec::with_capacity(count);
    let whole = [(1u32, count as u32)];
    let stsc: &[(u32, u32)] = m.stsc.as_deref().unwrap_or(&whole);
    for r in 0..stsc.len() {
        if offsets.len() >= count {
            break;
        }
        let (first, per_chunk) = stsc[r];
        if per_chunk == 0 {
            continue; // no samples: nothing to place (the C# walked its chunks anyway)
        }
        let first_chunk = (first as usize).saturating_sub(1);
        let last_chunk = if r + 1 < stsc.len() { (stsc[r + 1].0 as usize).saturating_sub(1) } else { chunks.len() };
        let mut c = first_chunk;
        while c < last_chunk && c < chunks.len() && offsets.len() < count {
            let mut at = chunks[c];
            let mut s = 0;
            while s < per_chunk && offsets.len() < count {
                let size = if m.fixed_size != 0 { m.fixed_size } else { sizes[offsets.len()] };
                offsets.push(at);
                out_sizes.push(size);
                at = at.saturating_add(size as u64);
                s += 1;
            }
            c += 1;
        }
    }
    (offsets, out_sizes)
}

/// One ALAC or AAC track in an MP4 open for playing.
pub struct Mp4Track {
    data: Vec<u8>,
    codec: Box<dyn FrameDecoder>,
    rate: u32,
    channels: u16,
    gain: ReplayGain,
    /// Samples in a whole frame.
    frame_length: usize,
    /// Frames decoded before a seek's own.
    preroll: usize,
    /// Each frame's place in the file, and size.
    offsets: Vec<u64>,
    sizes: Vec<u32>,
    /// Decoded samples dropped at the start; the decoded sample the music ends before.
    pub(crate) skip: i64,
    pub(crate) end: i64,

    pcm: Vec<f32>,
    /// The next frame to decode.
    frame: usize,
    /// The decoded sample pcm starts at; its frames, and those handed out.
    pcm_at: i64,
    pcm_frames: usize,
    pcm_offset: usize,
    /// The first decoded sample to hand out.
    from: i64,
}

impl Mp4Track {
    /// An ALAC track; an AAC one is refused with [`Error::NeedsAac`] (the
    /// tests' way in; `open` goes through `new_with`).
    #[cfg(test)]
    pub fn new(data: Vec<u8>) -> Result<Mp4Track, Error> {
        Mp4Track::new_with(data, &|_| Err(Error::NeedsAac))
    }

    /// An ALAC track, or an AAC one by the decoder `aac` makes.
    pub fn new_with(data: Vec<u8>, aac: MakeAac) -> Result<Mp4Track, Error> {
        let (movie, walked) = parse(&data);
        walked?;
        let (Some(sizes), Some(chunks)) = (&movie.sizes, &movie.chunk_offsets) else {
            return Err(Error::Unsupported("No AAC or ALAC track in this MP4 (another codec).".into()));
        };
        let mut codec: Box<dyn FrameDecoder> = match (&movie.alac_cookie, &movie.asc) {
            (Some(cookie), _) => Box::new(Alac(AlacDecoder::from_cookie(cookie)?)),
            (None, Some(asc)) => aac(asc)?,
            (None, None) => return Err(Error::Unsupported("No AAC or ALAC track in this MP4 (another codec).".into())),
        };
        let rate = codec.rate();
        let channels = codec.channels();
        let frame_length = codec.frame_length();
        let preroll = codec.preroll();
        if !(1..=MAX_FRAME_LENGTH).contains(&frame_length) || !(1..=crate::MAX_CHANNELS).contains(&channels) {
            return Err(damaged(format!("{frame_length}-sample frames of {channels} channels: not a track.")));
        }
        let (offsets, sizes) = sample_table(&movie, chunks, sizes);
        let mut pcm = vec![0.0; frame_length * channels as usize];

        // The first frame decoded now and forgotten (Mp4Track.cs): what shows
        // only inside the frames (SBR in a fill element, a coupling channel)
        // is refused here, not played as silence; a damaged frame is played
        // as it comes.
        if let (Some(&at), Some(&size)) = (offsets.first(), sizes.first()) {
            let bytes = span(&data, at, at.saturating_add(size as u64));
            if let Err(Error::Unsupported(m)) = codec.decode(bytes, &mut pcm) {
                return Err(Error::Unsupported(m));
            }
            codec.reset();
        }

        // The length and the cuts, in decoded samples (FFmpeg's st->duration,
        // skip_samples and first_discard_sample).
        let to_samples = |t: i64| {
            if movie.track_timescale == rate as i64 { t } else { rescale(t, rate as i64, movie.track_timescale) }
        };
        let decoded = offsets.len() as i64 * frame_length as i64;
        let stts_total = movie.stts_total;
        let mut duration = if movie.mdhd_duration > 0 {
            movie.mdhd_duration.min(if stts_total > 0 { stts_total } else { i64::MAX })
        } else {
            stts_total
        };
        let (mut origin, mut skip) = (0i64, 0i64);
        if let Some(media_time) = movie.edit_media_time {
            origin = media_time;
            skip = media_time;
            if movie.edits_duration > 0 && movie.movie_timescale > 0 {
                duration = duration.min(rescale(movie.edits_duration, movie.track_timescale, movie.movie_timescale));
            }
        }
        let mut first_discard = -1i64; // in the timeline that starts at `origin`
        if let Some((priming, remainder, samples)) = movie.smpb {
            if priming > 0 && priming < 16384 {
                skip = priming as i64;
            }
            if remainder > 0 && duration > remainder as i64 && duration > samples {
                first_discard = duration - remainder as i64;
            }
        }
        if first_discard < 0 {
            first_discard = duration;
        }

        let skip = to_samples(skip);
        let end = decoded.min(to_samples(origin.saturating_add(first_discard)));
        Ok(Mp4Track {
            gain: ReplayGain::from_tags(&movie.tags.values),
            codec,
            rate,
            channels,
            frame_length,
            preroll,
            offsets,
            sizes,
            skip,
            end,
            pcm,
            frame: 0,
            pcm_at: 0,
            pcm_frames: 0,
            pcm_offset: 0,
            from: skip,
            data,
        })
    }

    /// The next frame into pcm. False at the end.
    fn decode_next(&mut self) -> bool {
        if self.frame >= self.offsets.len() {
            return false;
        }
        let at = self.offsets[self.frame];
        let size = self.sizes[self.frame] as u64;
        if at.saturating_add(size) > self.data.len() as u64 {
            return false;
        }
        let bytes = &self.data[at as usize..(at + size) as usize];
        let frames = match self.codec.decode(bytes, &mut self.pcm) {
            Ok(n) => n.min(self.frame_length),
            Err(_) => {
                // A damaged frame still takes its place in time.
                self.pcm.fill(0.0);
                self.frame_length
            }
        };
        self.pcm_at = self.frame as i64 * self.frame_length as i64;
        self.pcm_frames = frames;
        self.pcm_offset = 0;
        self.frame += 1;
        true
    }
}

impl Track for Mp4Track {
    fn rate(&self) -> u32 {
        self.rate
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn bits(&self) -> u16 {
        self.codec.bits()
    }
    fn frames(&self) -> u64 {
        (self.end - self.skip).max(0) as u64
    }
    fn gain(&self) -> ReplayGain {
        self.gain
    }

    fn read(&mut self, into: &mut [f32]) -> Result<usize, Error> {
        let channels = self.channels as usize;
        let want = into.len() / channels;
        let mut done = 0;
        while done < want {
            if self.pcm_offset >= self.pcm_frames {
                if self.pcm_at + self.pcm_frames as i64 >= self.end && self.pcm_frames > 0 {
                    break;
                }
                if !self.decode_next() {
                    break;
                }
                continue;
            }
            let s = self.pcm_at + self.pcm_offset as i64;
            if s >= self.end {
                self.pcm_offset = self.pcm_frames;
                break;
            }
            if s < self.from {
                self.pcm_offset += ((self.from - s) as u64).min((self.pcm_frames - self.pcm_offset) as u64) as usize;
                continue;
            }
            let n = ((want - done).min(self.pcm_frames - self.pcm_offset) as i64).min(self.end - s) as usize;
            into[done * channels..(done + n) * channels]
                .copy_from_slice(&self.pcm[self.pcm_offset * channels..(self.pcm_offset + n) * channels]);
            self.pcm_offset += n;
            done += n;
        }
        Ok(done)
    }

    /// Mp4Track.cs's SeekNow: decoding starts at the frame that holds the
    /// sample (an ALAC frame needs nothing from before it), or the decoder's
    /// preroll before it, the overlap from before forgotten (AAC).
    fn seek(&mut self, to: u64) -> Result<(), Error> {
        let target = (to.min(i64::MAX as u64 / 2) as i64).saturating_add(self.skip);
        let frame = (self.offsets.len() as i64).min(target / self.frame_length as i64).max(0) as usize;
        let from = frame.saturating_sub(self.preroll);
        self.codec.reset();
        self.frame = from;
        self.pcm_at = from as i64 * self.frame_length as i64;
        self.pcm_frames = 0;
        self.pcm_offset = 0;
        self.from = self.skip.max(target);
        Ok(())
    }
}

/// The tags alone, of any MP4 (AAC too): the ilst's title, artist, album,
/// cover and freeform values, as far as the file could be read.
pub fn tags(data: &[u8]) -> Tags {
    parse(data).0.tags
}

/// Whether this is an ISO base media file of sound (ftyp at 4) rather than
/// a picture (HEIF, AVIF), as vergence-types tells them apart.
pub fn sniff(data: &[u8]) -> bool {
    if data.len() < 12 || &data[4..8] != b"ftyp" {
        return false;
    }
    !matches!(&data[8..12], b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"msf1" | b"avif" | b"avis")
}
