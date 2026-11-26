//! A track's words and cover art: what a preview shows (title, artist,
//! album, the front cover), and the TXXX / Vorbis comment values ReplayGain
//! is read from. ID3v2 as Hymie reads it (Spindle Id3.cs, after FFmpeg's
//! libavformat/id3v2.c: versions 2.2 to 2.4, one tag after another,
//! unsynchronisation undone, compressed and encrypted frames left alone),
//! with the text and picture frames added; Vorbis comments as RFC 9639 §8.6;
//! FLAC's PICTURE block as §8.8. MP4's ilst atoms are read in mp4.rs, into
//! the same Tags.

use std::collections::HashMap;

/// A picture kept in a file: its type as stored (image/jpeg, image/png) and
/// its bytes, still encoded (a preview decodes it in the picture helper).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Picture {
    pub mime: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// The front cover, else the first picture.
    pub picture: Option<Picture>,
    /// Every key as upper case, the first of a repeated key kept (TXXX's
    /// descriptions, Vorbis comments' names).
    pub values: HashMap<String, String>,
    /// Whether `picture` is a front cover (type 3), which a later one may not replace.
    front: bool,
}

/// The largest picture kept: more is not a cover.
const PICTURE_MAX: usize = 16 << 20;

impl Tags {
    pub(crate) fn put_text(&mut self, which: &str, value: String) {
        let value = value.trim().to_string();
        if value.is_empty() {
            return;
        }
        let slot = match which {
            "TITLE" => &mut self.title,
            "ARTIST" => &mut self.artist,
            "ALBUM" => &mut self.album,
            _ => return,
        };
        if slot.is_none() {
            *slot = Some(value);
        }
    }

    pub(crate) fn put_value(&mut self, key: &str, value: String) {
        let key = key.to_ascii_uppercase();
        if key.is_empty() {
            return;
        }
        self.put_text(&key, value.clone());
        self.values.entry(key).or_insert(value);
    }

    /// The picture taken out (a library keeps only its size).
    pub(crate) fn take_picture(&mut self) -> Option<Picture> {
        self.front = false;
        self.picture.take()
    }

    /// A picture of `kind` (3 the front cover): kept if none is yet, or it
    /// is the front cover and the one kept is not.
    pub(crate) fn put_picture(&mut self, kind: u32, mime: String, data: &[u8]) {
        if data.is_empty() || data.len() > PICTURE_MAX {
            return;
        }
        if self.picture.is_none() || (kind == 3 && !self.front) {
            self.front = kind == 3;
            self.picture = Some(Picture { mime, data: data.to_vec() });
        }
    }
}

/// How many bytes at the start are ID3v2 tags (one after another).
pub fn id3_size(data: &[u8]) -> usize {
    let mut at = 0;
    while let Some(n) = id3_tag_size(data, at) {
        at += n;
    }
    at
}

/// The size of the tag starting at `at`, from its first ten bytes, if one does.
fn id3_tag_size(data: &[u8], at: usize) -> Option<usize> {
    let h = data.get(at..at + 10)?;
    if &h[..3] != b"ID3" || !(2..=4).contains(&h[3]) {
        return None;
    }
    let size = synchsafe(&h[6..10])?;
    Some(10 + size + if h[5] & 0x10 != 0 { 10 } else { 0 })
}

fn synchsafe(b: &[u8]) -> Option<usize> {
    if b.iter().take(4).any(|x| *x >= 0x80) {
        return None;
    }
    Some((b[0] as usize) << 21 | (b[1] as usize) << 14 | (b[2] as usize) << 7 | b[3] as usize)
}

/// Every ID3v2 tag from `at` on, read into `tags`; returns the bytes they take.
pub fn read_id3(data: &[u8], mut at: usize, tags: &mut Tags) -> usize {
    let start = at;
    while let Some(total) = id3_tag_size(data, at) {
        let size = total - 10 - if data[at + 5] & 0x10 != 0 { 10 } else { 0 };
        if let Some(tag) = data.get(at..at + 10 + size) {
            read_tag(tag, tags);
        }
        at += total;
    }
    at - start
}

fn read_tag(tag: &[u8], tags: &mut Tags) {
    let (version, flags) = (tag[3], tag[5]);
    let whole;
    let mut body: &[u8] = &tag[10..];
    if flags & 0x80 != 0 && version < 4 {
        whole = unsync(body);
        body = &whole;
    }
    let mut at = 0usize;
    if flags & 0x40 != 0 && version >= 3 && body.len() >= 4 {
        // The extended header: its size, which in 2.3 leaves itself out.
        at = if version == 3 {
            (u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize).saturating_add(4)
        } else {
            synchsafe(&body[..4]).unwrap_or(usize::MAX)
        };
    }
    let (id_len, head_len) = if version == 2 { (3, 6) } else { (4, 10) };
    while at.saturating_add(head_len) <= body.len() && body[at] != 0 {
        let id = &body[at..at + id_len];
        let size = match version {
            2 => (body[at + 3] as usize) << 16 | (body[at + 4] as usize) << 8 | body[at + 5] as usize,
            3 => u32::from_be_bytes([body[at + 4], body[at + 5], body[at + 6], body[at + 7]]) as usize,
            _ => synchsafe(&body[at + 4..at + 8]).unwrap_or(0),
        };
        let format_flags = if version == 2 { 0 } else { body[at + 9] };
        at += head_len;
        if size == 0 || at + size > body.len() {
            break;
        }
        let mut frame: &[u8] = &body[at..at + size];
        let undone;
        at += size;
        if version == 3 && format_flags & 0xC0 != 0 {
            continue; // compressed or encrypted
        }
        if version == 4 {
            if format_flags & 0x0C != 0 {
                continue;
            }
            if format_flags & 0x01 != 0 {
                if frame.len() < 4 {
                    continue;
                }
                frame = &frame[4..];
            }
            if format_flags & 0x02 != 0 || flags & 0x80 != 0 {
                undone = unsync(frame);
                frame = &undone;
            }
        }
        match id {
            b"TXXX" | b"TXX" => txxx(frame, tags),
            b"TIT2" | b"TT2" => tags.put_text("TITLE", text_frame(frame)),
            b"TPE1" | b"TP1" => tags.put_text("ARTIST", text_frame(frame)),
            b"TALB" | b"TAL" => tags.put_text("ALBUM", text_frame(frame)),
            // What a library groups and orders by (AlbumWall's scan), under the
            // Vorbis comment names: the album artist, track and disc ("n/count"),
            // the date (2.4's recording time, 2.3's year, 2.2's TYE).
            b"TPE2" | b"TP2" => tags.put_value("ALBUMARTIST", text_frame(frame)),
            b"TRCK" | b"TRK" => tags.put_value("TRACKNUMBER", text_frame(frame)),
            b"TPOS" | b"TPA" => tags.put_value("DISCNUMBER", text_frame(frame)),
            b"TDRC" | b"TYER" | b"TYE" | b"TDOR" => tags.put_value("DATE", text_frame(frame)),
            b"APIC" => apic(frame, tags, false),
            b"PIC" => apic(frame, tags, true),
            _ => {}
        }
    }
}

/// A text frame: an encoding, then the text (the first of several values).
fn text_frame(frame: &[u8]) -> String {
    let Some((&enc, rest)) = frame.split_first() else { return String::new() };
    let wide = matches!(enc, 1 | 2);
    text(&rest[..terminator(rest, wide)], enc)
}

/// A TXXX: an encoding, a description, a value.
fn txxx(frame: &[u8], tags: &mut Tags) {
    if frame.len() < 2 {
        return;
    }
    let (enc, rest) = (frame[0], &frame[1..]);
    let wide = matches!(enc, 1 | 2);
    let end = terminator(rest, wide);
    let key = text(&rest[..end], enc);
    let after = rest.len().min(end + if wide { 2 } else { 1 });
    let value_bytes = &rest[after..];
    let value = text(&value_bytes[..terminator(value_bytes, wide)], enc);
    tags.put_value(&key, value);
}

/// APIC (2.3, 2.4): an encoding, a MIME type (Latin-1, ended by 0), the
/// picture's type, a description, the data. PIC (2.2): an encoding, a
/// three-letter format, the type, a description, the data.
fn apic(frame: &[u8], tags: &mut Tags, v22: bool) {
    let Some((&enc, rest)) = frame.split_first() else { return };
    let (mime, rest) = if v22 {
        let Some(f) = rest.get(..3) else { return };
        (if f.eq_ignore_ascii_case(b"PNG") { "image/png" } else { "image/jpeg" }.to_string(), &rest[3..])
    } else {
        let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
        let m = String::from_utf8_lossy(&rest[..end]).into_owned();
        (m, rest.get(end + 1..).unwrap_or(&[]))
    };
    let Some((&kind, rest)) = rest.split_first() else { return };
    let wide = matches!(enc, 1 | 2);
    let end = terminator(rest, wide);
    let data = rest.get(end + if wide { 2 } else { 1 }..).unwrap_or(&[]);
    tags.put_picture(kind as u32, mime, data);
}

fn terminator(s: &[u8], wide: bool) -> usize {
    if !wide {
        return s.iter().position(|b| *b == 0).unwrap_or(s.len());
    }
    let mut i = 0;
    while i + 1 < s.len() {
        if s[i] == 0 && s[i + 1] == 0 {
            return i;
        }
        i += 2;
    }
    s.len() & !1
}

fn text(s: &[u8], encoding: u8) -> String {
    let utf16 = |s: &[u8], big: bool| -> String {
        let units: Vec<u16> = s
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| if big { u16::from_be_bytes(*c) } else { u16::from_le_bytes(*c) })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match encoding {
        0 => s.iter().map(|&b| b as char).collect(),
        1 if s.len() >= 2 && s[0] == 0xFE && s[1] == 0xFF => utf16(&s[2..], true),
        1 if s.len() >= 2 && s[0] == 0xFF && s[1] == 0xFE => utf16(&s[2..], false),
        1 => utf16(s, false),
        2 => utf16(s, true),
        _ => String::from_utf8_lossy(s).into_owned(),
    }
}

/// Unsynchronisation undone: every FF 00 back to FF.
fn unsync(s: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        o.push(s[i]);
        if s[i] == 0xFF && i + 1 < s.len() && s[i + 1] == 0 {
            i += 1;
        }
        i += 1;
    }
    o
}

/// A VORBIS_COMMENT block: little-endian lengths, a vendor string, then
/// "KEY=value" entries. A damaged block gives what it can.
pub fn vorbis_comments(b: &[u8], tags: &mut Tags) {
    let mut at = 0usize;
    let take = |at: &mut usize| -> Option<usize> {
        let n = u32::from_le_bytes(b.get(*at..*at + 4)?.try_into().ok()?) as usize;
        *at += 4;
        (*at + n <= b.len()).then_some(n)
    };
    let Some(vendor) = take(&mut at) else { return };
    at += vendor;
    // A count, not a length, but the range check still holds for any real
    // one: every entry takes at least its own four length bytes (Hymie's rule).
    let Some(count) = take(&mut at) else { return };
    for _ in 0..count {
        let Some(len) = take(&mut at) else { return };
        let entry = String::from_utf8_lossy(&b[at..at + len]).into_owned();
        at += len;
        if let Some(eq) = entry.find('=').filter(|&e| e > 0) {
            tags.put_value(&entry[..eq], entry[eq + 1..].to_string());
        }
    }
}

/// FLAC's PICTURE block (RFC 9639 §8.8): big-endian fields, the type, the
/// MIME type, a description, sizes, then the data.
pub fn flac_picture(b: &[u8], tags: &mut Tags) {
    let be = |at: usize| -> Option<usize> { Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize) };
    let Some(kind) = be(0) else { return };
    let Some(mime_len) = be(4) else { return };
    let Some(mime) = b.get(8..8 + mime_len) else { return };
    let at = 8 + mime_len;
    let Some(desc_len) = be(at) else { return };
    let at = at + 4 + desc_len + 16;
    let Some(len) = be(at) else { return };
    let Some(data) = b.get(at + 4..at + 4 + len) else { return };
    tags.put_picture(kind as u32, String::from_utf8_lossy(mime).into_owned(), data);
}
