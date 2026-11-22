//! Hymie, AlbumWall's audio engine, ported to Rust (preview milestone 1,
//! docs/preview-milestone-1.md): its decoders already exist and are proven,
//! so Vergence reuses them rather than writing new ones. The source is Spindle's
//! `src/Spindle.Codecs` (C#, at 005f289), which played bit-identical to mpv
//! (2026-10-26). Each decoder keeps Hymie's structure, so a fix in one can be
//! carried to the other.
//!
//! A [`Track`] is one file open for playing: its rate, channels and length,
//! its ReplayGain, interleaved samples as floats from -1 to 1, and seeking to
//! an exact frame. [`open`] chooses the decoder by the file's first bytes.
//! [`Tags`] gives the words and the cover art a preview shows. [`peaks`]
//! decodes a whole track into a waveform's min/max per slice.
//!
//! The formats and where each came from:
//! - WAV and FLAC (Deadwax's decoder, written from RFC 9639): our own.
//! - MP3, MPEG-1, 2 and 2.5 Layer III: minimp3 (CC0), gapless by the LAME
//!   tag as FFmpeg does it.
//! - ALAC in an MP4 (.m4a): Apple's own decoder (Copyright (c) 2011 Apple
//!   Inc., Apache-2.0, licenses/alac-LICENSE.txt) by way of Hymie's Alac.cs;
//!   the MP4 container, gapless by its edit list and iTunSMPB as FFmpeg does
//!   it, and its iTunes tags (ilst atoms): our own.
//! - AAC (FAAD2, GPL-2.0-or-later) lives in its own crate and helper program
//!   (crates/vergence-hymie-aac, libexec/vergence/vergence-hymie-aac), so
//!   this one stays under Vergence's license: here an AAC .m4a is refused
//!   with [`Error::NeedsAac`] (its tags and cover are still read), and the
//!   client ([`helper::Sound`]) hands such a file to the AAC helper. That
//!   helper reuses this crate's MP4 reader through [`open_with`] and the
//!   [`FrameDecoder`] trait, which say nothing of AAC's own workings.
//!
//! Files are parsed in a confined helper, never in an app's own process
//! (whitepaper §6.10): this crate is that helper's library.

mod alac;
mod flac;
pub mod helper;
mod md5;
mod mp3;
mod mp3_tables;
mod mp4;
mod replaygain;
mod tags;
mod wav;

pub use mp4::{FrameDecoder, MakeAac};
pub use replaygain::ReplayGain;

/// One waveform slice: the lowest and highest sample of two channels.
pub type Peak = [(f32, f32); 2];
pub use tags::{Picture, Tags};

/// Why a file is not played.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A file these decoders do not play, though nothing is wrong with it.
    Unsupported(String),
    /// A damaged or malformed file.
    Damaged(String),
    /// The file could not be read.
    Io(String),
    /// AAC in an MP4: played by the AAC helper (vergence-hymie-aac), not by
    /// this crate. The helper's client tells it apart (its own answer kind on
    /// the pipe) and starts that helper instead.
    NeedsAac,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unsupported(m) | Error::Damaged(m) | Error::Io(m) => f.write_str(m),
            Error::NeedsAac => {
                f.write_str("AAC in an MP4 is played by the AAC helper (libexec/vergence/vergence-hymie-aac).")
            }
        }
    }
}

impl std::error::Error for Error {}

pub(crate) fn damaged(m: impl Into<String>) -> Error {
    Error::Damaged(m.into())
}

/// The most a track may claim, and the fewest: what an output is made for
/// (Hymie's security review 2026-10-26, finding 4: a WAV claiming
/// 3,221,225,472 Hz took the app down).
pub const MAX_RATE: u32 = 768_000;
pub const MAX_CHANNELS: u16 = 8;

/// The largest file opened: held whole in memory while it plays.
pub const MAX_FILE: u64 = 2 << 30;

/// One track open for playing, whatever its format.
pub trait Track: Send {
    fn rate(&self) -> u32;
    fn channels(&self) -> u16;
    /// Bits per sample as stored (0 for floating point).
    fn bits(&self) -> u16;
    /// Frames from the first to the last, after anything the format says to drop.
    fn frames(&self) -> u64;
    fn gain(&self) -> ReplayGain;
    /// The next frames, interleaved, into `into` (a whole number of frames
    /// long). Returns the frames written; fewer than asked only at the end.
    fn read(&mut self, into: &mut [f32]) -> Result<usize, Error>;
    /// Puts the cursor on `frame`, exactly, for the next read.
    fn seek(&mut self, frame: u64) -> Result<(), Error>;
    /// The track's length in seconds.
    fn seconds(&self) -> f64 {
        if self.rate() == 0 { 0.0 } else { self.frames() as f64 / self.rate() as f64 }
    }
}

/// A file's bytes, whole (at most [`MAX_FILE`]).
pub fn read_file(path: &std::path::Path) -> Result<Vec<u8>, Error> {
    use std::io::Read;
    let f = std::fs::File::open(path).map_err(|e| Error::Io(e.to_string()))?;
    let mut data = Vec::new();
    f.take(MAX_FILE + 1).read_to_end(&mut data).map_err(|e| Error::Io(e.to_string()))?;
    if data.len() as u64 > MAX_FILE {
        return Err(Error::Unsupported("The file is too large to play.".into()));
    }
    Ok(data)
}

/// The format of a file by its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Wav,
    Flac,
    /// MPEG audio: Layer III is played, Layers I and II are refused by name.
    Mp3,
    /// An MP4 (ftyp at byte 4, not a HEIF or AVIF picture): ALAC is played,
    /// AAC only through [`open_with`] (the AAC helper), its tags read either way.
    Mp4,
}

pub fn format_of(data: &[u8]) -> Option<Format> {
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return Some(Format::Wav);
    }
    let at = tags::id3_size(data);
    if data.len() >= at + 4 && &data[at..at + 4] == b"fLaC" {
        return Some(Format::Flac);
    }
    if mp4::sniff(data) {
        return Some(Format::Mp4);
    }
    if mp3::sniff(data) {
        return Some(Format::Mp3);
    }
    None
}

/// A track from a file's bytes, the decoder chosen by them. AAC in an MP4
/// is refused with [`Error::NeedsAac`].
pub fn open(data: Vec<u8>) -> Result<Box<dyn Track>, Error> {
    open_with(data, &|_| Err(Error::NeedsAac))
}

/// As [`open`], with `aac` making the decoder for an MP4's AAC track from
/// its AudioSpecificConfig (the esds' bytes). Only the AAC helper passes
/// one: the decoder is GPL code that this crate never contains.
pub fn open_with(data: Vec<u8>, aac: MakeAac) -> Result<Box<dyn Track>, Error> {
    let track: Box<dyn Track> = match format_of(&data) {
        Some(Format::Wav) => Box::new(wav::WavTrack::new(data)?),
        Some(Format::Flac) => Box::new(flac::FlacTrack::new(data)?),
        Some(Format::Mp3) => Box::new(mp3::Mp3Track::new(data)?),
        Some(Format::Mp4) => Box::new(mp4::Mp4Track::new_with(data, aac)?),
        None => return Err(Error::Unsupported("Not a sound file Hymie plays.".into())),
    };
    if !(1..=MAX_RATE).contains(&track.rate()) || !(1..=MAX_CHANNELS).contains(&track.channels()) {
        return Err(damaged(format!("{} Hz, {} channels: no output plays that.", track.rate(), track.channels())));
    }
    Ok(track)
}

/// A file's words and cover art, without decoding its sound.
pub fn tags(data: &[u8]) -> Tags {
    match format_of(data) {
        Some(Format::Flac) => flac::tags(data),
        Some(Format::Wav) => wav::tags(data),
        Some(Format::Mp3) => mp3::tags(data),
        Some(Format::Mp4) => mp4::tags(data),
        None => Tags::default(),
    }
}

/// A FLAC file decoded whole and checked against the MD5 it keeps of its
/// own samples: Some(true) they match, None it keeps none.
pub fn flac_check(data: &[u8]) -> Result<Option<bool>, Error> {
    flac::check(data)
}

/// A waveform: for each of `slices` equal parts of the track, the lowest and
/// highest sample of each channel (min, max), mixed to at most two channels.
/// `told` hears the slices done so far, as they come (a preview draws them
/// left to right); returning false stops the work.
pub fn peaks(track: &mut dyn Track, slices: usize, mut told: impl FnMut(&[Peak]) -> bool) -> Result<Vec<Peak>, Error> {
    let frames = track.frames();
    let ch = track.channels().max(1) as usize;
    let slices = slices.max(1);
    let mut out = vec![[(0.0f32, 0.0f32); 2]; slices];
    if frames == 0 {
        return Ok(out);
    }
    track.seek(0)?;
    let mut buf = vec![0f32; 4096 * ch];
    let mut at: u64 = 0;
    let mut told_up_to = 0;
    loop {
        let n = track.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for f in 0..n {
            let slice = (((at + f as u64) as u128 * slices as u128) / frames as u128).min(slices as u128 - 1) as usize;
            for c in 0..ch.min(2) {
                // A third channel and more go with the second's side.
                let v = buf[f * ch + c];
                let p = &mut out[slice][c];
                p.0 = p.0.min(v);
                p.1 = p.1.max(v);
            }
            if ch == 1 {
                out[slice][1] = out[slice][0];
            }
        }
        at += n as u64;
        let done = ((at as u128 * slices as u128) / frames as u128).min(slices as u128) as usize;
        if done > told_up_to + slices / 50 || done == slices {
            told_up_to = done;
            if !told(&out[..done]) {
                break;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
