//! Hymie's AAC-LC decoder, ported from C# (Spindle's
//! src/Spindle.Codecs/Aac.cs at 005f289) to Rust, and what its helper
//! program needs to play an AAC .m4a.
//!
//! LICENSE: GPL-2.0-or-later, unlike the rest of Vergence (Apache-2.0 OR
//! GPL-2.0-or-later). The decoder is ported from FAAD2 (github.com/knik0/faad2,
//! libfaad at 521d105, GPL-2.0-or-later; licenses/faad2-COPYING.txt), its LC
//! path only. Code from FAAD2 is copyright (c) Nero AG, www.nero.com. So this
//! crate is used ONLY by its own helper program,
//! libexec/vergence/vergence-hymie-aac, never linked into vergence-hymie or
//! any app (Dave, 2026-11-21). It depends on vergence-hymie (dual-licensed,
//! so it may be combined into this GPL program) for the MP4 reader, the
//! track, and the helper's protocol and confinement; vergence-hymie holds no
//! AAC code, only the [`FrameDecoder`] trait this decoder plugs into.

mod aac;
mod tables;

pub use aac::AacDecoder;
use vergence_hymie::{Error, FrameDecoder, Track};

/// The AAC decoder for an MP4 track's AudioSpecificConfig: vergence-hymie's
/// [`vergence_hymie::MakeAac`].
pub fn make(asc: &[u8]) -> Result<Box<dyn FrameDecoder>, Error> {
    Ok(Box::new(AacDecoder::from_config(asc)?))
}

/// A track from a file's bytes as vergence-hymie opens it, AAC in an MP4
/// played too.
pub fn open(data: Vec<u8>) -> Result<Box<dyn Track>, Error> {
    vergence_hymie::open_with(data, &make)
}

#[cfg(test)]
mod tests;
