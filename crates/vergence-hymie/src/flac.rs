//! FLAC, read for playing: Hymie's Flac.cs and FlacTrack.cs, which are
//! Deadwax's decoder (written from RFC 9639; it decoded a whole FLAC
//! library the same as FFmpeg, 2026-10-20). decode_frame, subframe, residual,
//! the bit reader and the CRCs are as they are there, so a fix can be carried
//! over line for line; what playing adds is the tags and finding the frame
//! that holds a given sample (seeking). The checks from Hymie's security
//! review (2026-10-26) are kept: STREAMINFO at least 34 bytes, every frame's
//! channels as STREAMINFO says, no frame longer than its largest block.

use crate::tags::{self, Tags};
use crate::{Error, ReplayGain, Track, damaged};

/// STREAMINFO (RFC 9639 §8.2).
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub max_block: u32,
    pub max_frame: u32,
    pub rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub samples: u64,
    /// The MD5 of the decoded samples (all zero: not computed by the encoder).
    pub md5: [u8; 16],
}

/// STREAMINFO, the tags, and where the first frame starts.
pub fn read_metadata(data: &[u8], tags: &mut Tags) -> Result<(StreamInfo, usize), Error> {
    let mut at = tags::id3_size(data);
    if data.len() < at + 4 + 4 + 34 || &data[at..at + 4] != b"fLaC" {
        return Err(damaged("Not a FLAC stream."));
    }
    at += 4;
    let mut info = None;
    loop {
        if at + 4 > data.len() {
            return Err(damaged("The metadata ends early."));
        }
        let last = data[at] & 0x80 != 0;
        let kind = data[at] & 0x7F;
        let size = (data[at + 1] as usize) << 16 | (data[at + 2] as usize) << 8 | data[at + 3] as usize;
        at += 4;
        if at + size > data.len() {
            return Err(damaged("A metadata block runs past the end."));
        }
        let block = &data[at..at + size];
        match kind {
            0 => {
                if size < 34 {
                    return Err(damaged("STREAMINFO is too short."));
                }
                let mut r = BitReader::new(block);
                // The smallest block and frame are read past: playing needs the largest.
                let _ = r.read(16)?;
                let max_block = r.read(16)?;
                let _ = r.read(24)?;
                info = Some(StreamInfo {
                    max_block,
                    max_frame: r.read(24)?,
                    rate: r.read(20)?,
                    channels: r.read(3)? as u16 + 1,
                    bits: r.read(5)? as u16 + 1,
                    samples: (r.read(4)? as u64) << 32 | r.read(32)? as u64,
                    md5: block[18..34].try_into().unwrap_or([0; 16]),
                });
            }
            4 => tags::vorbis_comments(block, tags),
            6 => tags::flac_picture(block, tags),
            _ => {}
        }
        at += size;
        if last {
            break;
        }
    }
    info.map(|i| (i, at)).ok_or_else(|| damaged("No STREAMINFO."))
}

/// The tags alone.
pub fn tags(data: &[u8]) -> Tags {
    let mut t = Tags::default();
    let _ = read_metadata(data, &mut t);
    t
}

/// The frame or sample number's extra bytes, coded as UTF-8 extends it.
fn extra_bytes(first: u32) -> Option<u32> {
    match first {
        0..0x80 => Some(0),
        0x80..0xC0 => None,
        0xC0..0xE0 => Some(1),
        0xE0..0xF0 => Some(2),
        0xF0..0xF8 => Some(3),
        0xF8..0xFC => Some(4),
        0xFC..0xFE => Some(5),
        0xFE => Some(6),
        _ => None,
    }
}

fn block_size(code: u32, r: &mut BitReader) -> Result<usize, Error> {
    Ok(match code {
        1 => 192,
        2..=5 => 576 << (code - 2),
        6 => r.read(8)? as usize + 1,
        7 => r.read(16)? as usize + 1,
        8.. => 256 << (code - 8),
        _ => return Err(damaged("A reserved block size code.")),
    })
}

/// Whether a frame header starts at data[at] (sync, no reserved values, a
/// matching CRC-8), and if so its first sample and its block size: found
/// without decoding the frames before it, for seeking.
pub fn try_header(data: &[u8], at: usize, info: &StreamInfo) -> Option<(u64, usize)> {
    if at + 6 > data.len() || data[at] != 0xFF || data[at + 1] & 0xFE != 0xF8 {
        return None;
    }
    let mut r = BitReader::new(&data[at..data.len().min(at + 16)]);
    let go = |r: &mut BitReader| -> Result<Option<(u64, usize)>, Error> {
        r.read(15)?;
        let variable = r.read(1)? == 1;
        let size_code = r.read(4)?;
        let rate_code = r.read(4)?;
        let channel_code = r.read(4)?;
        let depth_code = r.read(3)?;
        if r.read(1)? != 0 || size_code == 0 || rate_code == 15 || channel_code > 10 || depth_code == 3 {
            return Ok(None);
        }
        let first = r.read(8)?;
        let Some(extra) = extra_bytes(first) else { return Ok(None) };
        let mut number = if extra == 0 { first as u64 } else { (first & (0x3F >> extra)) as u64 };
        for _ in 0..extra {
            let b = r.read(8)?;
            if b & 0xC0 != 0x80 {
                return Ok(None);
            }
            number = number << 6 | (b & 0x3F) as u64;
        }
        let size = block_size(size_code, r)?;
        match rate_code {
            12 => {
                r.read(8)?;
            }
            13 | 14 => {
                r.read(16)?;
            }
            _ => {}
        }
        let header_end = r.byte_position();
        if r.read(8)? != crc8(&data[at..at + header_end]) as u32 {
            return Ok(None);
        }
        Ok(Some((if variable { number } else { number * info.max_block as u64 }, size)))
    };
    go(&mut r).ok().flatten()
}

/// One frame at data[start..end]: its samples into channels[c][..block],
/// growing them when a frame is longer. Returns the frame's length in bytes
/// and its block size. Indexed loops as in Hymie, so a fix carries over.
#[allow(clippy::needless_range_loop)]
pub fn decode_frame(
    data: &[u8],
    start: usize,
    end: usize,
    info: &StreamInfo,
    channels: &mut Vec<Vec<i64>>,
) -> Result<(usize, usize), Error> {
    let frame = &data[start..end];
    let mut r = BitReader::new(frame);
    if r.read(14)? != 0b11111111111110 || r.read(1)? != 0 {
        return Err(damaged(format!("No frame sync at byte {start}.")));
    }
    r.read(1)?; // blocking strategy: fixed or variable, it reads the same
    let size_code = r.read(4)?;
    let rate_code = r.read(4)?;
    let channel_code = r.read(4)?;
    let depth_code = r.read(3)?;
    if r.read(1)? != 0 {
        return Err(damaged("A reserved frame-header bit is set."));
    }
    let first = r.read(8)?;
    let extra = extra_bytes(first).ok_or_else(|| damaged("A malformed frame number."))?;
    for _ in 0..extra {
        if r.read(8)? & 0xC0 != 0x80 {
            return Err(damaged("A malformed frame number."));
        }
    }
    let block = block_size(size_code, &mut r)?;
    match rate_code {
        12 => {
            r.read(8)?;
        }
        13 | 14 => {
            r.read(16)?;
        }
        15 => return Err(damaged("A forbidden sample rate code.")),
        _ => {}
    }
    let bits: u32 = match depth_code {
        0 => info.bits as u32,
        1 => 8,
        2 => 12,
        4 => 16,
        5 => 20,
        6 => 24,
        7 => 32,
        _ => return Err(damaged("A reserved sample size code.")),
    };
    let header_end = r.byte_position();
    if r.read(8)? != crc8(&frame[..header_end]) as u32 {
        return Err(damaged(format!("Frame header CRC-8 mismatch at byte {start}.")));
    }
    let count = match channel_code {
        0..8 => channel_code as usize + 1,
        8..=10 => 2,
        _ => return Err(damaged("A reserved channel assignment.")),
    };
    if channels.len() < count {
        channels.resize(count, Vec::new());
    }
    for ch in channels.iter_mut().take(count) {
        if ch.len() < block {
            ch.resize(block, 0);
        }
    }
    for (c, ch) in channels.iter_mut().enumerate().take(count) {
        // The side channel carries one bit more (RFC 9639 §9.2).
        let side = (channel_code == 8 && c == 1) || (channel_code == 9 && c == 0) || (channel_code == 10 && c == 1);
        subframe(&mut r, &mut ch[..block], bits + side as u32)?;
    }
    match channel_code {
        8 => (0..block).for_each(|i| channels[1][i] = channels[0][i].wrapping_sub(channels[1][i])),
        9 => (0..block).for_each(|i| channels[0][i] = channels[0][i].wrapping_add(channels[1][i])),
        10 => {
            for i in 0..block {
                let mid = channels[0][i].wrapping_shl(1) | (channels[1][i] & 1);
                let s = channels[1][i];
                channels[0][i] = mid.wrapping_add(s) >> 1;
                channels[1][i] = mid.wrapping_sub(s) >> 1;
            }
        }
        _ => {}
    }
    r.align_to_byte();
    let crc_at = r.byte_position();
    if r.read(16)? != crc16(&frame[..crc_at]) as u32 {
        return Err(damaged(format!("Frame CRC-16 mismatch at byte {start}.")));
    }
    Ok((r.byte_position(), block))
}

fn subframe(r: &mut BitReader, s: &mut [i64], bits: u32) -> Result<(), Error> {
    let n = s.len();
    if r.read(1)? != 0 {
        return Err(damaged("A subframe's first bit is not zero."));
    }
    let kind = r.read(6)?;
    let wasted = if r.read(1)? == 1 { r.read_unary()? as u32 + 1 } else { 0 };
    if wasted >= bits {
        return Err(damaged("More wasted bits than the sample has."));
    }
    let bits = bits - wasted;
    match kind {
        0 => {
            let v = r.read_signed(bits)?;
            s.fill(v);
        }
        1 => {
            for x in s.iter_mut() {
                *x = r.read_signed(bits)?;
            }
        }
        8..=12 => {
            let order = (kind - 8) as usize;
            if order > n {
                return Err(damaged("A predictor order longer than the block."));
            }
            for x in s.iter_mut().take(order) {
                *x = r.read_signed(bits)?;
            }
            residual(r, s, order)?;
            for i in order..n {
                s[i] = s[i].wrapping_add(match order {
                    0 => 0,
                    // Wrapping: a damaged file gives wrong sound, never a crash.
                    1 => s[i - 1],
                    2 => s[i - 1].wrapping_mul(2).wrapping_sub(s[i - 2]),
                    3 => s[i - 1].wrapping_mul(3).wrapping_sub(s[i - 2].wrapping_mul(3)).wrapping_add(s[i - 3]),
                    _ => s[i - 1]
                        .wrapping_mul(4)
                        .wrapping_sub(s[i - 2].wrapping_mul(6))
                        .wrapping_add(s[i - 3].wrapping_mul(4))
                        .wrapping_sub(s[i - 4]),
                });
            }
        }
        32.. => {
            let order = (kind - 31) as usize;
            if order > n {
                return Err(damaged("A predictor order longer than the block."));
            }
            for x in s.iter_mut().take(order) {
                *x = r.read_signed(bits)?;
            }
            let precision = r.read(4)? + 1;
            if precision == 16 {
                return Err(damaged("An invalid coefficient precision."));
            }
            let shift = r.read_signed(5)?;
            if shift < 0 {
                return Err(damaged("A negative predictor shift."));
            }
            let mut coef = [0i64; 32];
            for c in coef.iter_mut().take(order) {
                *c = r.read_signed(precision)?;
            }
            residual(r, s, order)?;
            for i in order..n {
                let mut sum: i64 = 0;
                for j in 0..order {
                    sum = sum.wrapping_add(coef[j].wrapping_mul(s[i - 1 - j]));
                }
                s[i] = s[i].wrapping_add(sum >> shift);
            }
        }
        _ => return Err(damaged(format!("A reserved subframe type ({kind})."))),
    }
    if wasted > 0 {
        for x in s.iter_mut() {
            *x = x.wrapping_shl(wasted);
        }
    }
    Ok(())
}

/// The residual into s[order..], to which the prediction is added after.
fn residual(r: &mut BitReader, s: &mut [i64], order: usize) -> Result<(), Error> {
    let n = s.len();
    let method = r.read(2)?;
    if method > 1 {
        return Err(damaged("A reserved residual coding method."));
    }
    let param_bits = if method == 0 { 4 } else { 5 };
    let escape = (1u32 << param_bits) - 1;
    let partition_order = r.read(4)?;
    let parts = 1usize << partition_order;
    if (n >> partition_order) < order || (n & (parts - 1) != 0 && partition_order > 0) {
        return Err(damaged("A partition order the block cannot have."));
    }
    let mut i = order;
    for p in 0..parts {
        let count = (n >> partition_order) - if p == 0 { order } else { 0 };
        let k = r.read(param_bits)?;
        if k == escape {
            let raw = r.read(5)?;
            for _ in 0..count {
                s[i] = r.read_signed(raw)?;
                i += 1;
            }
        } else {
            for _ in 0..count {
                s[i] = r.read_rice(k)?;
                i += 1;
            }
        }
    }
    Ok(())
}

/// Bits read back, most significant first, through a 64-bit cache.
pub struct BitReader<'a> {
    data: &'a [u8],
    next: usize,
    cache: u64,
    cache_bits: u32,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> BitReader<'a> {
        BitReader { data, next: 0, cache: 0, cache_bits: 0 }
    }

    /// The byte the next unread bit is in (or the next byte, at a boundary).
    pub fn byte_position(&self) -> usize {
        self.next - (self.cache_bits / 8) as usize
    }

    fn refill(&mut self) {
        while self.cache_bits <= 56 && self.next < self.data.len() {
            self.cache |= (self.data[self.next] as u64) << (56 - self.cache_bits);
            self.next += 1;
            self.cache_bits += 8;
        }
    }

    fn drop_bits(&mut self, bits: u32) {
        self.cache = if bits >= 64 { 0 } else { self.cache << bits };
        self.cache_bits -= bits;
    }

    /// 0 to 32 bits as an unsigned number.
    pub fn read(&mut self, bits: u32) -> Result<u32, Error> {
        if bits == 0 {
            return Ok(0);
        }
        if self.cache_bits < bits {
            self.refill();
        }
        if self.cache_bits < bits {
            return Err(damaged("The frame ends early."));
        }
        let v = (self.cache >> (64 - bits)) as u32;
        self.drop_bits(bits);
        Ok(v)
    }

    pub fn read_signed(&mut self, bits: u32) -> Result<i64, Error> {
        if bits == 0 {
            return Ok(0);
        }
        // A side channel of 32-bit audio is 33 bits wide.
        let v: i64 = if bits > 32 {
            (self.read(bits - 32)? as i64) << 32 | self.read(32)? as i64
        } else {
            self.read(bits)? as i64
        };
        Ok(if v & (1i64 << (bits - 1)) != 0 { v - (1i64 << bits) } else { v })
    }

    pub fn read_unary(&mut self) -> Result<u64, Error> {
        let mut zeros = 0u64;
        loop {
            if self.cache_bits == 0 {
                self.refill();
                if self.cache_bits == 0 {
                    return Err(damaged("The frame ends early."));
                }
            }
            let lead = if self.cache == 0 { 64 } else { self.cache.leading_zeros() };
            if lead >= self.cache_bits {
                zeros += self.cache_bits as u64;
                self.cache = 0;
                self.cache_bits = 0;
                continue;
            }
            zeros += lead as u64;
            self.drop_bits(lead + 1);
            return Ok(zeros);
        }
    }

    pub fn read_rice(&mut self, k: u32) -> Result<i64, Error> {
        let q = self.read_unary()?;
        if q > (u64::MAX >> (k + 1)) {
            return Err(damaged("A residual too large."));
        }
        let folded = (q << k) | self.read(k)? as u64;
        Ok(if folded & 1 == 0 { (folded >> 1) as i64 } else { -((folded >> 1) as i64) - 1 })
    }

    pub fn align_to_byte(&mut self) {
        let b = self.cache_bits & 7;
        self.drop_bits(b);
    }
}

/// CRC-8 over a frame header (x^8 + x^2 + x + 1) and CRC-16 over the whole
/// frame (x^16 + x^15 + x^2 + 1), both from zero, most significant bit first.
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

pub fn crc16(data: &[u8]) -> u16 {
    static TABLE: std::sync::OnceLock<[u16; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u16; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = (i as u16) << 8;
            for _ in 0..8 {
                c = if c & 0x8000 != 0 { (c << 1) ^ 0x8005 } else { c << 1 };
            }
            *e = c;
        }
        t
    });
    let mut crc: u16 = 0;
    for &b in data {
        crc = (crc << 8) ^ t[((crc >> 8) as u8 ^ b) as usize];
    }
    crc
}

/// One FLAC file open for playing, held whole in memory.
pub struct FlacTrack {
    data: Vec<u8>,
    info: StreamInfo,
    frames_start: usize,
    gain: ReplayGain,
    scale: f32,
    /// The next frame to decode, the last frame decoded, its length, how much handed out.
    at: usize,
    channels: Vec<Vec<i64>>,
    frame_len: usize,
    frame_off: usize,
}

impl FlacTrack {
    pub fn new(data: Vec<u8>) -> Result<FlacTrack, Error> {
        let mut t = Tags::default();
        let (info, frames_start) = read_metadata(&data, &mut t)?;
        if info.channels > 8 {
            return Err(damaged("More channels than FLAC has."));
        }
        let scale = 1.0 / (1u64 << (info.bits - 1)) as f32;
        let channels = vec![vec![0i64; info.max_block.max(1) as usize]; info.channels as usize];
        Ok(FlacTrack {
            gain: ReplayGain::from_tags(&t.values),
            scale,
            at: frames_start,
            frames_start,
            channels,
            frame_len: 0,
            frame_off: 0,
            info,
            data,
        })
    }

    /// False at the end: what follows the last frame is not another frame
    /// (an ID3v1 tag left by a tagger, or padding).
    fn decode_next(&mut self) -> Result<bool, Error> {
        if self.at >= self.data.len() || try_header(&self.data, self.at, &self.info).is_none() {
            self.at = self.data.len();
            return Ok(false);
        }
        let code = (self.data[self.at + 3] >> 4) as u16;
        if (if code < 8 { code + 1 } else { 2 }) != self.info.channels {
            return Err(damaged(format!(
                "The frame at byte {} has a different number of channels from STREAMINFO.",
                self.at
            )));
        }
        let at = self.at;
        let (len, block) = decode_frame(&self.data, self.at, self.data.len(), &self.info, &mut self.channels)?;
        if self.info.max_block > 0 && block > self.info.max_block as usize {
            return Err(damaged(format!("The frame at byte {at} is longer than STREAMINFO's largest block.")));
        }
        self.at += len;
        self.frame_len = block;
        self.frame_off = 0;
        Ok(true)
    }

    /// The first frame header at or after `from` that the next header agrees
    /// with (its first sample this one's plus its size), so that a sync
    /// pattern inside a frame's audio is not taken for a frame.
    fn find_frame(&self, from: usize) -> Option<(usize, u64, usize)> {
        let d = &self.data;
        let limit = if self.info.max_frame > 0 { self.info.max_frame as usize + 64 } else { 1 << 20 };
        let stop = d.len().saturating_sub(1);
        for i in from..stop {
            if d[i] != 0xFF {
                continue;
            }
            let Some((first, size)) = try_header(d, i, &self.info) else { continue };
            if self.info.samples > 0 && first + size as u64 >= self.info.samples {
                return Some((i, first, size));
            }
            let within = stop.min(i + limit);
            for j in i + 2..within {
                if d[j] == 0xFF && try_header(d, j, &self.info).is_some_and(|(next, _)| next == first + size as u64) {
                    return Some((i, first, size));
                }
            }
        }
        None
    }
}

impl Track for FlacTrack {
    fn rate(&self) -> u32 {
        self.info.rate
    }
    fn channels(&self) -> u16 {
        self.info.channels
    }
    fn bits(&self) -> u16 {
        self.info.bits
    }
    fn frames(&self) -> u64 {
        self.info.samples
    }
    fn gain(&self) -> ReplayGain {
        self.gain
    }

    fn read(&mut self, into: &mut [f32]) -> Result<usize, Error> {
        let ch = self.info.channels as usize;
        let want = into.len() / ch;
        let mut done = 0;
        while done < want {
            if self.frame_off >= self.frame_len {
                if !self.decode_next()? {
                    break;
                }
                continue;
            }
            let n = (want - done).min(self.frame_len - self.frame_off);
            for i in 0..n {
                let o = (done + i) * ch;
                for c in 0..ch {
                    into[o + c] = self.channels[c][self.frame_off + i] as f32 * self.scale;
                }
            }
            self.frame_off += n;
            done += n;
        }
        Ok(done)
    }

    /// The frame holding the sample found by its header, without decoding
    /// what comes before (a binary search over the bytes, each guess checked
    /// by the header that follows it), decoded, and what precedes the sample skipped.
    fn seek(&mut self, to: u64) -> Result<(), Error> {
        self.frame_len = 0;
        self.frame_off = 0;
        self.at = self.frames_start;
        if to == 0 {
            return Ok(());
        }
        let total = self.info.samples;
        let (mut lo, mut hi) = (self.frames_start, self.data.len());
        let (mut from, mut from_sample) = (self.frames_start, 0u64);
        let mut tries = 0;
        while tries < 48 && hi - lo > 16 && total > 0 {
            tries += 1;
            let mid = (lo + hi) / 2;
            let Some((at, first, size)) = self.find_frame(mid) else {
                hi = mid;
                continue;
            };
            if first > to {
                hi = mid;
                continue;
            }
            from = at;
            from_sample = first;
            if first + size as u64 > to {
                break;
            }
            lo = at + 1;
        }
        self.at = from;
        let mut skip = to - from_sample;
        while skip > 0 && self.decode_next()? {
            if skip < self.frame_len as u64 {
                self.frame_off = skip as usize;
                return Ok(());
            }
            skip -= self.frame_len as u64;
            // A frame skipped whole is used up: it is not handed out again.
            // (Hymie's FlacTrack.SeekNow leaves it unread, so a seek to the
            // end of a frame plays it again; found while porting.)
            self.frame_off = self.frame_len;
        }
        Ok(())
    }
}

#[allow(clippy::needless_range_loop)]
/// The whole file decoded and checked against STREAMINFO's MD5 (each
/// sample little-endian, signed, in whole bytes, the channels interleaved):
/// Some(true) a match, Some(false) a mismatch, None no MD5 to check by.
pub fn check(data: &[u8]) -> Result<Option<bool>, Error> {
    let mut t = Tags::default();
    let (info, mut at) = read_metadata(data, &mut t)?;
    if info.md5 == [0; 16] {
        return Ok(None);
    }
    let width = info.bits.div_ceil(8) as usize;
    let mut pcm = Vec::with_capacity(info.samples as usize * info.channels as usize * width);
    let mut ch: Vec<Vec<i64>> = Vec::new();
    while at < data.len() && try_header(data, at, &info).is_some() {
        let (len, block) = decode_frame(data, at, data.len(), &info, &mut ch)?;
        for i in 0..block {
            for c in ch.iter().take(info.channels as usize) {
                pcm.extend_from_slice(&c[i].to_le_bytes()[..width]);
            }
        }
        at += len;
    }
    Ok(Some(crate::md5::hex(&pcm) == info.md5.iter().map(|b| format!("{b:02x}")).collect::<String>()))
}
