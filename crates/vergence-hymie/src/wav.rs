//! One WAV file open for playing, as Hymie's WavTrack.cs. There is no codec:
//! the samples are stored as they are, so this is only the RIFF chunks around
//! them. `fmt ` says what they are: whole numbers of 8, 16, 24 or 32 bits (8
//! unsigned, the rest signed), or 32- or 64-bit floats, plainly or in
//! WAVE_FORMAT_EXTENSIBLE's dress. `data` is the samples; a size of 0 or
//! 0xFFFFFFFF (a recorder that never came back to fill it in) means to the end
//! of the file. Seeks are a multiply. An `id3 ` chunk is read for the tags.

use crate::tags::{self, Tags};
use crate::{Error, ReplayGain, Track, damaged};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Unsigned8,
    Signed,
    Float,
}

pub struct WavTrack {
    data: Vec<u8>,
    rate: u32,
    channels: u16,
    bits: u16,
    kind: Kind,
    /// Bytes per sample as stored (24-bit is 3, float64 is 8), and per frame.
    per_sample: usize,
    align: usize,
    start: usize,
    frames: u64,
    frame: u64,
    gain: ReplayGain,
}

const PCM: u16 = 1;
const IEEE_FLOAT: u16 = 3;
const EXTENSIBLE: u16 = 0xFFFE;

struct Chunks {
    format: u16,
    channels: u16,
    rate: u32,
    align: u16,
    bits: u16,
    start: usize,
    size: usize,
}

fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([d[at], d[at + 1]])
}

fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

/// The chunks walked: the format, the samples' place, and the tags.
fn chunks(data: &[u8], tags: &mut Tags) -> Result<Chunks, Error> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(Error::Unsupported(
            if data.starts_with(b"RF64") { "RF64, not RIFF WAV." } else { "Not a RIFF WAVE file." }.into(),
        ));
    }
    let mut c = Chunks { format: 0, channels: 0, rate: 0, align: 0, bits: 0, start: 0, size: 0 };
    let mut have_format = false;
    let mut at = 12usize;
    loop {
        if at + 8 > data.len() {
            return Err(damaged("No samples in this WAV."));
        }
        let id = &data[at..at + 4];
        let mut size = u32_at(data, at + 4) as usize;
        let body = at + 8;
        if id == b"data" {
            if !have_format {
                return Err(damaged("WAV samples before their format."));
            }
            if size == 0 || size == u32::MAX as usize || body + size > data.len() {
                size = data.len() - body;
            }
            c.start = body;
            c.size = size;
            return Ok(c);
        }
        let end = body.saturating_add(size).min(data.len());
        if id == b"fmt " && size >= 16 && body + 16 <= data.len() {
            c.format = u16_at(data, body);
            c.channels = u16_at(data, body + 2);
            c.rate = u32_at(data, body + 4);
            c.align = u16_at(data, body + 12);
            c.bits = u16_at(data, body + 14);
            // EXTENSIBLE carries the real format in the first two bytes of its sub-format GUID.
            if c.format == EXTENSIBLE && size >= 40 && body + 26 <= data.len() {
                c.format = u16_at(data, body + 24);
            }
            have_format = true;
        } else if id.eq_ignore_ascii_case(b"id3 ") {
            tags::read_id3(&data[..end], body, tags);
        }
        // Chunks are padded to an even length.
        at = body.saturating_add(size).saturating_add(size & 1);
    }
}

impl WavTrack {
    pub fn new(data: Vec<u8>) -> Result<WavTrack, Error> {
        let mut t = Tags::default();
        let c = chunks(&data, &mut t)?;
        if c.channels == 0 || c.channels > 8 || c.rate == 0 || c.align == 0 {
            return Err(damaged("A WAV format that makes no sense."));
        }
        let per_sample = (c.align / c.channels) as usize;
        let (kind, bits) = match (c.format, c.bits, per_sample) {
            (PCM, 8, 1) => (Kind::Unsigned8, 8),
            (PCM, 9..=32, 2..=4) => (Kind::Signed, c.bits),
            (IEEE_FLOAT, 32, 4) | (IEEE_FLOAT, 64, 8) => (Kind::Float, 0),
            _ => {
                return Err(Error::Unsupported(format!("WAV format {}, {}-bit: not plain samples.", c.format, c.bits)));
            }
        };
        Ok(WavTrack {
            rate: c.rate,
            channels: c.channels,
            bits,
            kind,
            per_sample,
            align: c.align as usize,
            start: c.start,
            frames: (c.size / c.align as usize) as u64,
            frame: 0,
            gain: ReplayGain::from_tags(&t.values),
            data,
        })
    }
}

pub fn tags(data: &[u8]) -> Tags {
    let mut t = Tags::default();
    let _ = chunks(data, &mut t);
    t
}

impl Track for WavTrack {
    fn rate(&self) -> u32 {
        self.rate
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn bits(&self) -> u16 {
        self.bits
    }
    fn frames(&self) -> u64 {
        self.frames
    }
    fn gain(&self) -> ReplayGain {
        self.gain
    }

    fn seek(&mut self, frame: u64) -> Result<(), Error> {
        self.frame = frame.min(self.frames);
        Ok(())
    }

    fn read(&mut self, into: &mut [f32]) -> Result<usize, Error> {
        let ch = self.channels as usize;
        let want = ((into.len() / ch) as u64).min(self.frames - self.frame) as usize;
        if want == 0 {
            return Ok(0);
        }
        let from = self.start + self.frame as usize * self.align;
        let src = &self.data[from..from + want * self.align];
        let n = want * ch;
        let s = self.per_sample;
        // A sample's bytes at its place in the frame (the frame may hold more than channels times the size).
        let at = |i: usize| (i / ch) * self.align + (i % ch) * s;
        match (self.kind, s) {
            (Kind::Unsigned8, _) => (0..n).for_each(|i| into[i] = (src[at(i)] as f32 - 128.0) / 128.0),
            (Kind::Signed, 2) => {
                (0..n).for_each(|i| into[i] = i16::from_le_bytes([src[at(i)], src[at(i) + 1]]) as f32 / 32768.0)
            }
            (Kind::Signed, 3) => (0..n).for_each(|i| {
                let a = at(i);
                into[i] = (((src[a] as i32) << 8) | ((src[a + 1] as i32) << 16) | ((src[a + 2] as i32) << 24)) as f32
                    / 2_147_483_648.0;
            }),
            (Kind::Signed, 4) => (0..n).for_each(|i| {
                into[i] =
                    i32::from_le_bytes(src[at(i)..at(i) + 4].try_into().unwrap_or([0; 4])) as f32 / 2_147_483_648.0
            }),
            (Kind::Float, 4) => {
                (0..n).for_each(|i| into[i] = f32::from_le_bytes(src[at(i)..at(i) + 4].try_into().unwrap_or([0; 4])))
            }
            (Kind::Float, 8) => (0..n)
                .for_each(|i| into[i] = f64::from_le_bytes(src[at(i)..at(i) + 8].try_into().unwrap_or([0; 8])) as f32),
            _ => return Err(damaged("A WAV sample size that makes no sense.")),
        }
        self.frame += want as u64;
        Ok(want)
    }
}
