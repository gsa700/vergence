//! ALAC (Apple Lossless) decoded in Rust: the codec core, Hymie's Alac.cs
//! carried over.
//!
//! PORTED FROM APPLE'S OWN DECODER (github.com/macosforge/alac, at c38887c,
//! 2016-05-11), Copyright (c) 2011 Apple Inc., Apache License 2.0
//! (licenses/alac-LICENSE.txt). Alac.cs ported it function by function from
//! codec/: ALACDecoder.cpp (the frame and its channel elements), ag_dec.c
//! (the adaptive Golomb decoder, dyn_decomp), dp_dec.c (the dynamic
//! predictor, unpc_block), matrix_dec.c (the stereo unmix) and
//! ALACBitUtilities.c (the bit reader). The C names are kept here in
//! snake_case, so this reads beside the C and the C#; the magic cookie is read
//! as ALACMagicCookieDescription.txt describes it.
//!
//! What Alac.cs changed from the C, kept here: only unpc_block's general loop
//! (Apple's unrolled 4- and 8-coefficient cases are the same sums in wrapping
//! arithmetic); floats from -1 to 1 out, each sample what Apple's output
//! routine would store, sign-extended at the stream's depth and scaled by
//! 1 / 2^(depth-1), so an ALAC file and its FLAC source give the same floats;
//! channels past two in WAV's order; a frame that ends before its channels or
//! holds an element Apple's decoder refuses is damaged (the track plays
//! silence in its place).
//!
//! What changed in the carrying to Rust: no `unsafe`, and nothing a damaged
//! frame holds can panic. The C reads a few bytes past a frame by design, so
//! Alac.cs copied each frame into a buffer with 8 zero bytes after it and let
//! C#'s bounds checks catch a read past those; here every byte is read
//! through [`Input::at`], which gives zero past the end and remembers that it
//! did, and a frame that read past its padding is damaged. All 32-bit
//! arithmetic wraps, and shifts by a count the data chooses are masked to
//! 0..31 (`wrapping_shl`/`wrapping_shr`), as C#'s are: the same results as
//! Alac.cs wherever the C itself is undefined.
//!
//! Each frame is independent of the one before, which is why a seek needs no
//! pre-roll (Mp4Track, mp4.rs).

// Indexed loops and the C's argument lists, so the code reads beside it.
#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

use crate::{Error, damaged};
use std::cell::Cell;

// ALACAudioTypes.h, ALACBitUtilities.h
const ID_SCE: u8 = 0;
const ID_CPE: u8 = 1;
const ID_CCE: u8 = 2;
const ID_LFE: u8 = 3;
const ID_DSE: u8 = 4;
const ID_PCE: u8 = 5;
const ID_FIL: u8 = 6;
const ID_END: u8 = 7;
const MAX_CHANNELS: usize = 8;

// aglib.h
const QB_SHIFT: u32 = 9;
const QB: u32 = 1 << QB_SHIFT;
const MMUL_SHIFT: u32 = 2;
const MDEN_SHIFT: u32 = QB_SHIFT - MMUL_SHIFT - 1;
const MOFF: u32 = 1 << (MDEN_SHIFT - 2);
const BITOFF: u32 = 24;
const MAX_PREFIX_16: u32 = 9;
const MAX_PREFIX_32: u32 = 9;
const MAX_DATATYPE_BITS_16: u32 = 16;

// ag_dec.c
const N_MAX_MEAN_CLAMP: u32 = 0xffff;
const N_MEAN_CLAMP_VAL: u32 = 0xffff;

/// Bytes of zeros after a frame's own, for the C's reads past its end: the
/// bit reader looks three bytes ahead, the Golomb decoder five.
const PADDING: usize = 8;

/// Where each channel, in the order the elements carry them, goes in the
/// output: ALAC's layouts (C L R ..., ALACMagicCookieDescription.txt) put in
/// WAV's (L R C LFE ...).
const CHANNEL_ORDER: [&[usize]; MAX_CHANNELS] = [
    &[0],                      // C
    &[0, 1],                   // L R
    &[2, 0, 1],                // C L R
    &[2, 0, 1, 3],             // C L R Cs
    &[2, 0, 1, 3, 4],          // C L R Ls Rs
    &[2, 0, 1, 4, 5, 3],       // C L R Ls Rs LFE
    &[2, 0, 1, 4, 5, 6, 3],    // C L R Ls Rs Cs LFE
    &[2, 6, 7, 0, 1, 4, 5, 3], // C Lc Rc L R Ls Rs LFE
];

/// One ALAC stream's decoder, made from its magic cookie (AlacDecoder).
pub struct AlacDecoder {
    pub frame_length: usize,
    pub bit_depth: u32,
    pub channels: usize,
    pub sample_rate: u32,

    pb: u32,
    mb: u32,
    kb: u32,
    max_run: u32,
    order: &'static [usize],
    scale: f32,

    mix_buffer_u: Vec<i32>,
    mix_buffer_v: Vec<i32>,
    predictor: Vec<i32>,
    shift_buffer: Vec<u16>,
    coefs_u: [i16; 32],
    coefs_v: [i16; 32],
}

/// A frame's bytes with zeros after them: what Alac.cs's padded copy was.
/// A read past the padding gives zero too, and is remembered, so that the
/// frame is called damaged where C# would have thrown.
struct Input<'a> {
    data: &'a [u8],
    past: Cell<bool>,
}

impl Input<'_> {
    fn at(&self, i: usize) -> u8 {
        match self.data.get(i) {
            Some(b) => *b,
            None => {
                if i >= self.data.len() + PADDING {
                    self.past.set(true);
                }
                0
            }
        }
    }
}

/// A big-endian bit reader over one frame (BitBuffer): a byte and a bit
/// within it.
#[derive(Clone, Copy)]
struct BitBuffer {
    cur: usize,
    bit_index: u32,
    end: usize,
    byte_size: usize,
}

fn be32(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

impl AlacDecoder {
    fn new(frame_length: usize, bit_depth: u32, pb: u32, mb: u32, kb: u32, channels: usize, max_run: u32, sample_rate: u32) -> AlacDecoder {
        // The predictor writes its first coefficients' worth of samples
        // whatever the frame's length, so the buffers hold at least that.
        let size = frame_length.max(32);
        AlacDecoder {
            frame_length,
            bit_depth,
            channels,
            sample_rate,
            pb,
            mb,
            kb,
            max_run,
            order: CHANNEL_ORDER[channels - 1],
            scale: 1.0 / (1u64 << (bit_depth - 1)) as f32,
            mix_buffer_u: vec![0; size],
            mix_buffer_v: vec![0; size],
            predictor: vec![0; size],
            shift_buffer: vec![0; 2 * size],
            coefs_u: [0; 32],
            coefs_v: [0; 32],
        }
    }

    /// From the magic cookie (ALACSpecificConfig), as an MP4's 'alac' box
    /// carries it: ALACDecoder::Init, which also takes the older cookies that
    /// wrap it in 'frma' and 'alac' atoms.
    pub fn from_cookie(mut cookie: &[u8]) -> Result<AlacDecoder, Error> {
        if cookie.len() >= 12 && &cookie[4..8] == b"frma" {
            cookie = &cookie[12..];
        }
        if cookie.len() >= 12 && &cookie[4..8] == b"alac" {
            cookie = &cookie[12..];
        }
        if cookie.len() < 24 {
            return Err(damaged("The ALAC magic cookie is too short."));
        }
        let frame_length = be32(cookie, 0);
        let compatible_version = cookie[4];
        let bit_depth = cookie[5] as u32;
        let (pb, mb, kb) = (cookie[6] as u32, cookie[7] as u32, cookie[8] as u32);
        let channels = cookie[9] as usize;
        let max_run = (cookie[10] as u32) << 8 | cookie[11] as u32;
        let sample_rate = be32(cookie, 20);

        if compatible_version > 0 {
            return Err(Error::Unsupported(format!("ALAC version {compatible_version} is not supported.")));
        }
        // The only depths Apple's decoder writes out; any other would be
        // decoded and then dropped on the floor.
        if !matches!(bit_depth, 16 | 20 | 24 | 32) {
            return Err(Error::Unsupported(format!("{bit_depth}-bit ALAC is not supported.")));
        }
        if !(1..=MAX_CHANNELS).contains(&channels) {
            return Err(Error::Unsupported(format!("ALAC with {channels} channels is not supported.")));
        }
        if frame_length == 0 || frame_length > 1 << 20 {
            return Err(damaged(format!("An ALAC frame length of {frame_length} samples.")));
        }
        if sample_rate == 0 || sample_rate > 1_000_000 {
            return Err(damaged(format!("An ALAC sample rate of {sample_rate}.")));
        }
        Ok(AlacDecoder::new(frame_length as usize, bit_depth, pb, mb, kb, channels, max_run, sample_rate))
    }

    /// One ALAC packet (an MP4 sample) into `pcm` (at least frame_length *
    /// channels long), interleaved, as floats from -1 to 1. Returns its
    /// length in samples: frame_length, or fewer for a partial frame (the last).
    pub fn decode_frame(&mut self, frame: &[u8], pcm: &mut [f32]) -> Result<usize, Error> {
        if pcm.len() < self.frame_length * self.channels {
            return Err(damaged("The ALAC output buffer is too small."));
        }
        let input = Input { data: frame, past: Cell::new(false) };
        let mut bits = BitBuffer { cur: 0, bit_index: 0, end: frame.len(), byte_size: frame.len() };
        let n = self.decode(&input, &mut bits, pcm)?;
        if input.past.get() {
            return Err(damaged("A damaged ALAC frame: it reads past its end."));
        }
        Ok(n)
    }

    // ------------------------------------------------------------ ALACDecoder.cpp

    /// ALACDecoder::Decode: the frame's elements, one after another, until
    /// every channel has been decoded or the frame ends.
    fn decode(&mut self, input: &Input, bits: &mut BitBuffer, pcm: &mut [f32]) -> Result<usize, Error> {
        let num_channels = self.channels;
        let mut num_samples = self.frame_length;
        let mut channel_index = 0;
        let mut shift_bits = BitBuffer { cur: 0, bit_index: 0, end: bits.end, byte_size: bits.byte_size };

        'elements: while channel_index < num_channels {
            if bits.cur >= bits.end {
                return Err(damaged("A damaged ALAC frame: it ends before its channels."));
            }
            if input.past.get() {
                return Err(damaged("A damaged ALAC frame: it reads past its end."));
            }

            let pb = self.pb;
            let tag = bit_buffer_read_small(input, bits, 3);
            match tag {
                ID_SCE | ID_LFE => {
                    // mono/LFE channel
                    bit_buffer_read_small(input, bits, 4); // element instance tag
                    if bit_buffer_read(input, bits, 12) != 0 {
                        return Err(damaged("A damaged ALAC frame: unused header bits set."));
                    }

                    // the 1-bit "partial frame" flag, 2-bit "shift-off" flag & 1-bit "escape" flag
                    let header_byte = bit_buffer_read(input, bits, 4);
                    let partial_frame = header_byte >> 3;
                    let mut bytes_shifted = (header_byte >> 1) & 0x3;
                    if bytes_shifted == 3 {
                        return Err(damaged("A damaged ALAC frame: 3 bytes shifted."));
                    }
                    let shift = bytes_shifted * 8;
                    let escape_flag = header_byte & 0x1;
                    let chan_bits = self.bit_depth as i32 - (bytes_shifted * 8) as i32;

                    if partial_frame != 0 {
                        num_samples = self.partial_frame(input, bits)?;
                    }

                    if escape_flag == 0 {
                        // compressed frame, read rest of parameters (no mixing for mono)
                        bit_buffer_read(input, bits, 8); // mixBits
                        bit_buffer_read(input, bits, 8); // mixRes

                        let header_byte = bit_buffer_read(input, bits, 8);
                        let mode_u = header_byte >> 4;
                        let den_shift_u = header_byte & 0xf;

                        let header_byte = bit_buffer_read(input, bits, 8);
                        let pb_factor_u = header_byte >> 5;
                        let num_u = (header_byte & 0x1f) as usize;
                        for i in 0..num_u {
                            self.coefs_u[i] = bit_buffer_read(input, bits, 16) as i16;
                        }

                        // if shift active, skip the shift buffer but remember where it starts
                        if bytes_shifted != 0 {
                            shift_bits = *bits;
                            bit_buffer_advance(bits, (bytes_shifted as usize * 8).saturating_mul(num_samples))?;
                        }

                        let ag_params = AgParams::new(self.mb, pb.wrapping_mul(pb_factor_u) / 4, self.kb, self.max_run);
                        dyn_decomp(&ag_params, input, bits, &mut self.predictor, num_samples, chan_bits)?;

                        if mode_u != 0 {
                            // the special "numActive == 31" mode, in place
                            unpc_block_31_in_place(&mut self.predictor, num_samples, chan_bits);
                        }
                        unpc_block(&self.predictor, &mut self.mix_buffer_u, num_samples, &mut self.coefs_u, num_u, chan_bits, den_shift_u);
                    } else {
                        // uncompressed frame, copied into the mix buffer to use the common output code
                        uncompressed(input, bits, &mut self.mix_buffer_u, None, num_samples, chan_bits);
                        bytes_shifted = 0;
                    }

                    // now read the shifted values into the shift buffer
                    if bytes_shifted != 0 {
                        for i in 0..num_samples {
                            self.shift_buffer[i] = bit_buffer_read(input, &mut shift_bits, shift) as u16;
                        }
                    }

                    // the samples out, as the copyPredictorTo* routines would store them
                    let o = self.order[channel_index];
                    let shifted = self.bit_depth >= 24 && bytes_shifted != 0;
                    for i in 0..num_samples {
                        let mut val = self.mix_buffer_u[i];
                        if shifted {
                            val = val.wrapping_shl(shift) | self.shift_buffer[i] as i32;
                        }
                        pcm[i * num_channels + o] = self.output(val);
                    }

                    channel_index += 1;
                }

                ID_CPE => {
                    // if decoding this pair would take us over the max channels limit, bail
                    if channel_index + 2 > num_channels {
                        break 'elements;
                    }

                    // stereo channel pair
                    bit_buffer_read_small(input, bits, 4); // element instance tag
                    if bit_buffer_read(input, bits, 12) != 0 {
                        return Err(damaged("A damaged ALAC frame: unused header bits set."));
                    }

                    let header_byte = bit_buffer_read(input, bits, 4);
                    let partial_frame = header_byte >> 3;
                    let mut bytes_shifted = (header_byte >> 1) & 0x3;
                    if bytes_shifted == 3 {
                        return Err(damaged("A damaged ALAC frame: 3 bytes shifted."));
                    }
                    let shift = bytes_shifted * 8;
                    let escape_flag = header_byte & 0x1;
                    let chan_bits = self.bit_depth as i32 - (bytes_shifted * 8) as i32 + 1;

                    if partial_frame != 0 {
                        num_samples = self.partial_frame(input, bits)?;
                    }

                    let (mix_bits, mix_res): (u32, i32);
                    if escape_flag == 0 {
                        // compressed frame, read rest of parameters
                        mix_bits = bit_buffer_read(input, bits, 8);
                        mix_res = bit_buffer_read(input, bits, 8) as u8 as i8 as i32;

                        let header_byte = bit_buffer_read(input, bits, 8);
                        let mode_u = header_byte >> 4;
                        let den_shift_u = header_byte & 0xf;

                        let header_byte = bit_buffer_read(input, bits, 8);
                        let pb_factor_u = header_byte >> 5;
                        let num_u = (header_byte & 0x1f) as usize;
                        for i in 0..num_u {
                            self.coefs_u[i] = bit_buffer_read(input, bits, 16) as i16;
                        }

                        let header_byte = bit_buffer_read(input, bits, 8);
                        let mode_v = header_byte >> 4;
                        let den_shift_v = header_byte & 0xf;

                        let header_byte = bit_buffer_read(input, bits, 8);
                        let pb_factor_v = header_byte >> 5;
                        let num_v = (header_byte & 0x1f) as usize;
                        for i in 0..num_v {
                            self.coefs_v[i] = bit_buffer_read(input, bits, 16) as i16;
                        }

                        // if shift active, skip the interleaved shifted values but remember where they start
                        if bytes_shifted != 0 {
                            shift_bits = *bits;
                            bit_buffer_advance(bits, (bytes_shifted as usize * 8 * 2).saturating_mul(num_samples))?;
                        }

                        // decompress and run predictor for "left" channel
                        let ag = AgParams::new(self.mb, pb.wrapping_mul(pb_factor_u) / 4, self.kb, self.max_run);
                        dyn_decomp(&ag, input, bits, &mut self.predictor, num_samples, chan_bits)?;
                        if mode_u != 0 {
                            unpc_block_31_in_place(&mut self.predictor, num_samples, chan_bits);
                        }
                        unpc_block(&self.predictor, &mut self.mix_buffer_u, num_samples, &mut self.coefs_u, num_u, chan_bits, den_shift_u);

                        // decompress and run predictor for "right" channel
                        let ag = AgParams::new(self.mb, pb.wrapping_mul(pb_factor_v) / 4, self.kb, self.max_run);
                        dyn_decomp(&ag, input, bits, &mut self.predictor, num_samples, chan_bits)?;
                        if mode_v != 0 {
                            unpc_block_31_in_place(&mut self.predictor, num_samples, chan_bits);
                        }
                        unpc_block(&self.predictor, &mut self.mix_buffer_v, num_samples, &mut self.coefs_v, num_v, chan_bits, den_shift_v);
                    } else {
                        // uncompressed frame, copied into the mix buffers to use the common output code
                        uncompressed(input, bits, &mut self.mix_buffer_u, Some(&mut self.mix_buffer_v), num_samples, self.bit_depth as i32);
                        mix_bits = 0;
                        mix_res = 0;
                        bytes_shifted = 0;
                    }

                    // now read the shifted values into the shift buffer
                    if bytes_shifted != 0 {
                        for i in (0..num_samples * 2).step_by(2) {
                            self.shift_buffer[i] = bit_buffer_read(input, &mut shift_bits, shift) as u16;
                            self.shift_buffer[i + 1] = bit_buffer_read(input, &mut shift_bits, shift) as u16;
                        }
                    }

                    // un-mix the data and convert to output format (unmix16/20/24/32);
                    // mixRes = 0 means just interleave, which uncompressed frames use
                    let ol = self.order[channel_index];
                    let or = self.order[channel_index + 1];
                    let shifted = self.bit_depth >= 24 && bytes_shifted != 0;
                    for j in 0..num_samples {
                        let k = 2 * j;
                        let (u, v) = (self.mix_buffer_u[j], self.mix_buffer_v[j]);
                        let (mut l, mut r);
                        if mix_res != 0 {
                            // matrixed stereo
                            l = u.wrapping_add(v).wrapping_sub(mix_res.wrapping_mul(v).wrapping_shr(mix_bits));
                            r = l.wrapping_sub(v);
                        } else {
                            // conventional separated stereo
                            l = u;
                            r = v;
                        }
                        if shifted {
                            l = l.wrapping_shl(shift) | self.shift_buffer[k] as i32;
                            r = r.wrapping_shl(shift) | self.shift_buffer[k + 1] as i32;
                        }
                        pcm[j * num_channels + ol] = self.output(l);
                        pcm[j * num_channels + or] = self.output(r);
                    }

                    channel_index += 2;
                }

                ID_CCE | ID_PCE => {
                    // unsupported element: Apple's decoder bails
                    return Err(damaged(format!("An ALAC frame with a {} element.", if tag == ID_CCE { "coupling channel" } else { "program config" })));
                }

                ID_DSE => {
                    // data stream element: parse but ignore
                    data_stream_element(input, bits)?;
                }

                ID_FIL => {
                    // fill element: parse but ignore
                    fill_element(input, bits)?;
                }

                _ => {
                    // ID_END: the frame's end, before all the channels it should have
                    debug_assert_eq!(tag, ID_END);
                    bit_buffer_byte_align(bits)?;
                    break 'elements;
                }
            }
        }

        // channels the frame did not have are silent
        for c in channel_index..num_channels {
            let o = self.order[c];
            for i in 0..num_samples {
                pcm[i * num_channels + o] = 0.0;
            }
        }
        Ok(num_samples)
    }

    /// The length of a partial frame, which overrides the cookie's.
    fn partial_frame(&self, input: &Input, bits: &mut BitBuffer) -> Result<usize, Error> {
        let mut n = bit_buffer_read(input, bits, 16) << 16;
        n |= bit_buffer_read(input, bits, 16);
        if n as usize > self.frame_length {
            return Err(damaged(format!("A damaged ALAC frame: {n} samples, more than {}.", self.frame_length)));
        }
        Ok(n as usize)
    }

    /// A sample as Apple's output routines store it (the low bit_depth bits:
    /// a 16-bit sample is cast to int16_t, a 24-bit one written as 3 bytes)
    /// and as a float.
    fn output(&self, val: i32) -> f32 {
        let s = 32 - self.bit_depth;
        (val.wrapping_shl(s) >> s) as f32 * self.scale
    }
}

/// An escaped (uncompressed) element's samples, one channel or a pair
/// interleaved, `chan_bits` each: bit_buffer_read reads at most 16 bits at a
/// time, so a wider sample is read in two.
fn uncompressed(input: &Input, bits: &mut BitBuffer, u: &mut [i32], mut v: Option<&mut [i32]>, num_samples: usize, chan_bits: i32) {
    let shift = (32 - chan_bits) as u32;
    if chan_bits <= 16 {
        let n = chan_bits.max(0) as u32;
        for i in 0..num_samples {
            let val = bit_buffer_read(input, bits, n) as i32;
            u[i] = val.wrapping_shl(shift).wrapping_shr(shift);
            if let Some(v) = v.as_deref_mut() {
                let val = bit_buffer_read(input, bits, n) as i32;
                v[i] = val.wrapping_shl(shift).wrapping_shr(shift);
            }
        }
    } else {
        let extra_bits = (chan_bits - 16) as u32;
        for i in 0..num_samples {
            let val = bit_buffer_read(input, bits, 16) as i32;
            let val = val.wrapping_shl(16).wrapping_shr(shift);
            u[i] = val | bit_buffer_read(input, bits, extra_bits) as i32;
            if let Some(v) = v.as_deref_mut() {
                let val = bit_buffer_read(input, bits, 16) as i32;
                let val = val.wrapping_shl(16).wrapping_shr(shift);
                v[i] = val | bit_buffer_read(input, bits, extra_bits) as i32;
            }
        }
    }
}

/// ALACDecoder::FillElement: filler, skipped.
fn fill_element(input: &Input, bits: &mut BitBuffer) -> Result<(), Error> {
    // 4-bit count or (4-bit + 8-bit count) if 4-bit count == 15
    let mut count = bit_buffer_read_small(input, bits, 4) as usize;
    if count == 15 {
        count = 14 + bit_buffer_read_small(input, bits, 8) as usize;
    }
    bit_buffer_advance(bits, count * 8)?;
    if bits.cur > bits.end {
        return Err(damaged("A damaged ALAC frame: a fill element past its end."));
    }
    Ok(())
}

/// ALACDecoder::DataStreamElement: not wanted, skipped.
fn data_stream_element(input: &Input, bits: &mut BitBuffer) -> Result<(), Error> {
    bit_buffer_read_small(input, bits, 4); // element instance tag
    let data_byte_align_flag = bit_buffer_read_one(input, bits);
    // 8-bit count or (8-bit + 8-bit count) if 8-bit count == 255
    let mut count = bit_buffer_read_small(input, bits, 8) as usize;
    if count == 255 {
        count += bit_buffer_read_small(input, bits, 8) as usize;
    }
    if data_byte_align_flag != 0 {
        bit_buffer_byte_align(bits)?;
    }
    bit_buffer_advance(bits, count * 8)?;
    if bits.cur > bits.end {
        return Err(damaged("A damaged ALAC frame: a data element past its end."));
    }
    Ok(())
}

// ------------------------------------------------------------ ag_dec.c

/// set_ag_params, with fullwidth and sectorwidth, which decoding never
/// reads, left out.
struct AgParams {
    mb0: u32,
    pb: u32,
    kb: u32,
    wb: u32,
    #[allow(dead_code)] // kept as the C has it; dyn_decomp never reads it
    max_run: u32,
}

impl AgParams {
    fn new(m: u32, p: u32, k: u32, maxrun: u32) -> AgParams {
        AgParams { mb0: m, pb: p, kb: k, wb: 1u32.wrapping_shl(k).wrapping_sub(1), max_run: maxrun }
    }
}

/// The leading zeros of a 32-bit word (32 for none): lead, the loop in the C
/// that its own comment says a count-leading-zeros instruction should be.
fn lead(m: u32) -> u32 {
    m.leading_zeros()
}

fn lg3a(x: u32) -> i32 {
    31 - lead(x.wrapping_add(3)) as i32
}

fn read32bit(input: &Input, at: usize) -> u32 {
    (input.at(at) as u32) << 24 | (input.at(at.saturating_add(1)) as u32) << 16 | (input.at(at.saturating_add(2)) as u32) << 8 | input.at(at.saturating_add(3)) as u32
}

fn get_stream_bits(input: &Input, in_at: usize, bit_offset: u32, num_bits: u32) -> u32 {
    let byte_offset = in_at.saturating_add((bit_offset / 8) as usize);
    let load1 = read32bit(input, byte_offset);
    let off = bit_offset & 7;
    let mut result;
    if num_bits + off > 32 {
        result = load1 << off;
        let mut load2 = input.at(byte_offset.saturating_add(4)) as u32;
        let load2_shift = 8i32 - (num_bits + off) as i32 + 32;
        load2 = load2.wrapping_shr(load2_shift as u32);
        result = result.wrapping_shr(32u32.wrapping_sub(num_bits));
        result |= load2;
    } else {
        result = load1.wrapping_shr(32 - num_bits - off);
    }
    // a shift by 32 does nothing in C# (as on x86), so the C's guard stays
    if num_bits != 32 {
        result &= !0xffff_ffffu32.wrapping_shl(num_bits);
    }
    result
}

/// dyn_get: a run length's Golomb code.
fn dyn_get(input: &Input, in_at: usize, bit_pos: &mut u32, m: u32, k: u32) -> u32 {
    let mut temp_bits = *bit_pos;
    let result;
    let mut stream_long = read32bit(input, in_at.saturating_add((temp_bits >> 3) as usize));
    stream_long <<= temp_bits & 7;

    // the number of bits in the prefix
    let mut pre = lead(!stream_long);
    if pre >= MAX_PREFIX_16 {
        pre = MAX_PREFIX_16;
        temp_bits = temp_bits.wrapping_add(pre);
        stream_long <<= pre;
        result = stream_long >> (32 - MAX_DATATYPE_BITS_16);
        temp_bits = temp_bits.wrapping_add(MAX_DATATYPE_BITS_16);
    } else {
        // all of the bits fit within the word loaded
        temp_bits = temp_bits.wrapping_add(pre + 1);
        stream_long = stream_long.wrapping_shl(pre + 1);
        let v = stream_long.wrapping_shr(32u32.wrapping_sub(k));
        temp_bits = temp_bits.wrapping_add(k);
        let mut r = pre.wrapping_mul(m).wrapping_add(v).wrapping_sub(1);
        if v < 2 {
            r = r.wrapping_sub(v.wrapping_sub(1));
            temp_bits = temp_bits.wrapping_sub(1);
        }
        result = r;
    }
    *bit_pos = temp_bits;
    result
}

/// dyn_get_32bit: a sample's Golomb code, escaped to `max_bits` raw bits.
fn dyn_get_32bit(input: &Input, in_at: usize, bit_pos: &mut u32, m: u32, k: u32, max_bits: u32) -> u32 {
    let mut temp_bits = *bit_pos;
    let mut stream_long = read32bit(input, in_at.saturating_add((temp_bits >> 3) as usize));
    stream_long <<= temp_bits & 7;

    // the number of bits in the prefix
    let mut result = lead(!stream_long);
    if result >= MAX_PREFIX_32 {
        result = get_stream_bits(input, in_at, temp_bits.wrapping_add(MAX_PREFIX_32), max_bits);
        temp_bits = temp_bits.wrapping_add(MAX_PREFIX_32).wrapping_add(max_bits);
    } else {
        // all of the bits fit within the word loaded
        temp_bits = temp_bits.wrapping_add(result + 1);
        if k != 1 {
            stream_long = stream_long.wrapping_shl(result + 1);
            let v = stream_long.wrapping_shr(32u32.wrapping_sub(k));
            temp_bits = temp_bits.wrapping_add(k).wrapping_sub(1);
            result = result.wrapping_mul(m);
            if v >= 2 {
                result = result.wrapping_add(v - 1);
                temp_bits = temp_bits.wrapping_add(1);
            }
        }
    }
    *bit_pos = temp_bits;
    result
}

/// dyn_decomp: one channel's prediction residuals, adaptive Golomb coded,
/// with runs of zeros.
fn dyn_decomp(p: &AgParams, input: &Input, bitstream: &mut BitBuffer, pc: &mut [i32], num_samples: usize, max_size: i32) -> Result<(), Error> {
    let in_at = bitstream.cur;
    let start_pos = bitstream.bit_index;
    let max_pos = (bitstream.byte_size as u64 * 8).min(u32::MAX as u64) as u32;
    let mut bit_pos = start_pos;
    // A negative width cannot come from a good cookie; as an unsigned count
    // it is what C#'s cast to byte-sized shifts made of it.
    let max_bits = max_size as u32;

    let mut mb = p.mb0;
    let pb = p.pb;
    let kb = p.kb;
    let wb = p.wb;
    let mut zmode: u32 = 0;
    let mut c = 0usize;
    let mut out_at = 0usize;

    while c < num_samples {
        // bail if we've run off the end of the buffer
        if bit_pos >= max_pos || input.past.get() {
            return Err(damaged("A damaged ALAC frame: its residuals run past its end."));
        }

        let m = mb >> QB_SHIFT;
        let mut k = lg3a(m) as u32;
        k = k.min(kb);
        let m = 1u32.wrapping_shl(k).wrapping_sub(1);

        let mut n = dyn_get_32bit(input, in_at, &mut bit_pos, m, k, max_bits);

        // least significant bit is sign bit
        {
            let ndecode = n.wrapping_add(zmode);
            let mut multiplier = -((ndecode & 1) as i32);
            multiplier |= 1;
            pc[out_at] = (ndecode.wrapping_add(1) >> 1).wrapping_mul(multiplier as u32) as i32;
            out_at += 1;
        }

        c += 1;

        mb = pb.wrapping_mul(n.wrapping_add(zmode)).wrapping_add(mb).wrapping_sub(pb.wrapping_mul(mb) >> QB_SHIFT);

        // update mean tracking
        if n > N_MAX_MEAN_CLAMP {
            mb = N_MEAN_CLAMP_VAL;
        }

        zmode = 0;

        if (mb << MMUL_SHIFT) < QB && c < num_samples {
            zmode = 1;
            let k = lead(mb).wrapping_sub(BITOFF).wrapping_add(mb.wrapping_add(MOFF) >> MDEN_SHIFT);
            let mz = 1u32.wrapping_shl(k).wrapping_sub(1) & wb;

            n = dyn_get(input, in_at, &mut bit_pos, mz, k);

            if c as u64 + n as u64 > num_samples as u64 {
                return Err(damaged("A damaged ALAC frame: a run of zeros past its end."));
            }

            for _ in 0..n {
                pc[out_at] = 0;
                out_at += 1;
                c += 1;
            }

            if n >= 65535 {
                zmode = 0;
            }

            mb = 0;
        }
    }

    bit_buffer_advance(bitstream, bit_pos.wrapping_sub(start_pos) as usize)?;
    if bitstream.cur > bitstream.end {
        return Err(damaged("A damaged ALAC frame: its residuals run past its end."));
    }
    Ok(())
}

// ------------------------------------------------------------ dp_dec.c

fn sign_of_int(i: i32) -> i32 {
    let negishift = ((i.wrapping_neg() as u32) >> 31) as i32;
    negishift | (i >> 31)
}

/// unpc_block's "numActive == 31" short circuit, which runs in place (in and
/// out the same array, as the decoder calls it).
fn unpc_block_31_in_place(buf: &mut [i32], num: usize, chan_bits: i32) {
    let chan_shift = (32 - chan_bits) as u32;
    let mut prev = buf[0];
    for j in 1..num {
        let d = buf[j].wrapping_add(prev);
        prev = d.wrapping_shl(chan_shift).wrapping_shr(chan_shift);
        buf[j] = prev;
    }
}

/// unpc_block: the dynamic predictor run backwards, from residuals `pc1` to
/// samples `output`, adapting its coefficients as it goes. Apple's unrolled
/// cases for 4 and 8 coefficients are this loop, written out. (Its in-place
/// use, numActive 31, is unpc_block_31_in_place.)
fn unpc_block(pc1: &[i32], output: &mut [i32], num: usize, coefs: &mut [i16; 32], num_active: usize, chan_bits: i32, den_shift: u32) {
    let chan_shift = (32 - chan_bits) as u32;
    let den_half = 1i32.wrapping_shl(den_shift.wrapping_sub(1));

    output[0] = pc1[0];
    if num_active == 0 {
        // just copy if numActive == 0
        if num > 1 {
            output[1..num].copy_from_slice(&pc1[1..num]);
        }
        return;
    }
    if num_active == 31 {
        // short-circuit if numActive == 31
        let mut prev = output[0];
        for j in 1..num {
            let d = pc1[j].wrapping_add(prev);
            prev = d.wrapping_shl(chan_shift).wrapping_shr(chan_shift);
            output[j] = prev;
        }
        return;
    }

    for j in 1..=num_active {
        let d = pc1[j].wrapping_add(output[j - 1]);
        output[j] = d.wrapping_shl(chan_shift).wrapping_shr(chan_shift);
    }

    let lim = num_active + 1;

    // general case
    for j in lim..num {
        let mut sum1: i32 = 0;
        let pout = j - 1;
        let top = output[j - lim];

        for k in 0..num_active {
            sum1 = sum1.wrapping_add((coefs[k] as i32).wrapping_mul(output[pout - k].wrapping_sub(top)));
        }

        let mut del = pc1[j];
        let mut del0 = del;
        let sg = sign_of_int(del);
        del = del.wrapping_add(top).wrapping_add(sum1.wrapping_add(den_half).wrapping_shr(den_shift));
        output[j] = del.wrapping_shl(chan_shift).wrapping_shr(chan_shift);

        if sg > 0 {
            for k in (0..num_active).rev() {
                let dd = top.wrapping_sub(output[pout - k]);
                let sgn = sign_of_int(dd);
                coefs[k] = coefs[k].wrapping_sub(sgn as i16);
                del0 = del0.wrapping_sub(((num_active - k) as i32).wrapping_mul(sgn.wrapping_mul(dd).wrapping_shr(den_shift)));
                if del0 <= 0 {
                    break;
                }
            }
        } else if sg < 0 {
            for k in (0..num_active).rev() {
                let dd = top.wrapping_sub(output[pout - k]);
                let sgn = sign_of_int(dd);
                coefs[k] = coefs[k].wrapping_add(sgn as i16);
                del0 = del0.wrapping_sub(((num_active - k) as i32).wrapping_mul(sgn.wrapping_neg().wrapping_mul(dd).wrapping_shr(den_shift)));
                if del0 >= 0 {
                    break;
                }
            }
        }
    }
}

// ------------------------------------------------------------ ALACBitUtilities.c

/// Up to 16 bits.
fn bit_buffer_read(input: &Input, bits: &mut BitBuffer, num_bits: u32) -> u32 {
    let c = bits.cur;
    let mut return_bits = (input.at(c) as u32) << 16 | (input.at(c.saturating_add(1)) as u32) << 8 | input.at(c.saturating_add(2)) as u32;
    return_bits <<= bits.bit_index;
    return_bits &= 0x00FF_FFFF;
    bits.bit_index += num_bits;
    return_bits = return_bits.wrapping_shr(24u32.wrapping_sub(num_bits));
    bits.cur = bits.cur.saturating_add((bits.bit_index >> 3) as usize);
    bits.bit_index &= 7;
    return_bits
}

/// Up to 8 bits.
fn bit_buffer_read_small(input: &Input, bits: &mut BitBuffer, num_bits: u32) -> u8 {
    let c = bits.cur;
    let mut return_bits = ((input.at(c) as u32) << 8 | input.at(c.saturating_add(1)) as u32) as u16;
    return_bits = return_bits.wrapping_shl(bits.bit_index);
    bits.bit_index += num_bits;
    return_bits >>= 16 - num_bits;
    bits.cur = bits.cur.saturating_add((bits.bit_index >> 3) as usize);
    bits.bit_index &= 7;
    return_bits as u8
}

/// One bit.
fn bit_buffer_read_one(input: &Input, bits: &mut BitBuffer) -> u8 {
    let return_bits = (input.at(bits.cur) >> (7 - bits.bit_index)) & 1;
    bits.bit_index += 1;
    bits.cur = bits.cur.saturating_add((bits.bit_index >> 3) as usize);
    bits.bit_index &= 7;
    return_bits
}

fn bit_buffer_byte_align(bits: &mut BitBuffer) -> Result<(), Error> {
    if bits.bit_index == 0 {
        return Ok(());
    }
    bit_buffer_advance(bits, (8 - bits.bit_index) as usize)
}

fn bit_buffer_advance(bits: &mut BitBuffer, num_bits: usize) -> Result<(), Error> {
    if num_bits == 0 {
        return Ok(());
    }
    // a damaged count would wrap: past the end either way
    if num_bits > (bits.byte_size + PADDING) * 8 {
        return Err(damaged("A damaged ALAC frame: a skip past its end."));
    }
    let total = bits.bit_index as usize + num_bits;
    bits.cur = bits.cur.saturating_add(total >> 3);
    bits.bit_index = (total & 7) as u32;
    Ok(())
}
