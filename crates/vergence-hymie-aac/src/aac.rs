//! AAC-LC decoded in Rust: the codec core, Hymie's Aac.cs carried over.
//!
//! PORTED FROM FAAD2 (github.com/knik0/faad2, libfaad at 521d105),
//! GPL-2.0-or-later, by way of Hymie's C# port. Code from FAAD2 is copyright
//! (c) Nero AG, www.nero.com. Its LC path only, function by function, the C
//! names kept (in snake_case, as FAAD2 has them): syntax.c (the bitstream),
//! huffman.c, specrec.c (inverse quantization), pns.c, ms.c, is.c, pulse.c,
//! tns.c, filtbank.c and mdct.c, in the floating-point build. Left out: SBR
//! and PS (HE-AAC), MAIN prediction, LTP, SSR, the error-resilient and DRM
//! profiles, DRC, and coupling channels: none is in an iTunes AAC-LC file,
//! and a stream that needs one is refused. The FFT is a plain radix-2 one in
//! place of FAAD2's FFTPACK, the same transform with the same sign. The
//! tables are in tables.rs, generated from FAAD2's (through AacTables.cs).
//!
//! Unlike FAAD2, the first frame is put out, not dropped: FFmpeg puts it out,
//! and the container's priming (vergence-hymie's mp4.rs) is counted from
//! there, so the same samples are cut either way.
//!
//! What changed in the carrying: no `unsafe`, and every index into a table
//! or buffer is either held in range by a check before it or read with
//! `get`, so a damaged frame is an error, never a panic (the C# caught
//! IndexOutOfRangeException for that); quantized values are i32 here, so an
//! escape of 2^15 or more no longer wraps around as the C#'s short (and
//! FAAD2's int16_t) did: it is out of the inverse-quantization table and
//! refused.

use crate::tables::*;
use vergence_hymie::{Error, FrameDecoder};

pub const FRAME_LENGTH: usize = 1024;
const NSHORT: usize = FRAME_LENGTH / 8;

const ONLY_LONG_SEQUENCE: usize = 0;
const LONG_START_SEQUENCE: usize = 1;
const EIGHT_SHORT_SEQUENCE: usize = 2;
const LONG_STOP_SEQUENCE: usize = 3;

const ZERO_HCB: u8 = 0;
const FIRST_PAIR_HCB: u8 = 5;
const NOISE_HCB: u8 = 13;
const INTENSITY_HCB2: u8 = 14;
const INTENSITY_HCB: u8 = 15;

const ID_SCE: u32 = 0;
const ID_CPE: u32 = 1;
const ID_CCE: u32 = 2;
const ID_LFE: u32 = 3;
const ID_DSE: u32 = 4;
const ID_PCE: u32 = 5;
const ID_FIL: u32 = 6;
const ID_END: u32 = 7;

const MAX_SFB: usize = 51;
const TNS_MAX_ORDER: usize = 20;
/// Sections per window group, at most: 8 * 15 for short windows.
const MAX_SECTIONS: usize = 8 * 15;

fn damaged(m: &str) -> Error {
    Error::Damaged(m.into())
}

fn unsupported(m: impl Into<String>) -> Error {
    Error::Unsupported(m.into())
}

const SAMPLE_RATES: [u32; 13] =
    [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
const NUM_SWB_1024_WINDOW: [usize; 12] = [41, 41, 47, 49, 49, 51, 47, 47, 43, 43, 43, 40];
const NUM_SWB_128_WINDOW: [usize; 12] = [12, 12, 12, 14, 14, 14, 15, 15, 15, 15, 15, 15];
static SWB_OFFSET_1024_WINDOW: [&[u16]; 12] = [
    &SWB_OFFSET1024_96,
    &SWB_OFFSET1024_96,
    &SWB_OFFSET1024_64,
    &SWB_OFFSET1024_48,
    &SWB_OFFSET1024_48,
    &SWB_OFFSET1024_32,
    &SWB_OFFSET1024_24,
    &SWB_OFFSET1024_24,
    &SWB_OFFSET1024_16,
    &SWB_OFFSET1024_16,
    &SWB_OFFSET1024_16,
    &SWB_OFFSET1024_8,
];
static SWB_OFFSET_128_WINDOW: [&[u16]; 12] = [
    &SWB_OFFSET128_96,
    &SWB_OFFSET128_96,
    &SWB_OFFSET128_64,
    &SWB_OFFSET128_48,
    &SWB_OFFSET128_48,
    &SWB_OFFSET128_48,
    &SWB_OFFSET128_24,
    &SWB_OFFSET128_24,
    &SWB_OFFSET128_16,
    &SWB_OFFSET128_16,
    &SWB_OFFSET128_16,
    &SWB_OFFSET128_8,
];

/// One AAC-LC stream's decoder, mono or stereo.
pub struct AacDecoder {
    sample_rate: u32,
    channels: usize,
    sf_index: usize,

    // Per channel, from frame to frame: the second half of the last
    // transform and its window's shape; and PNS's random generator.
    overlap: Vec<[f32; FRAME_LENGTH]>,
    window_shape_prev: Vec<usize>,
    r1: u32,
    r2: u32,

    ics1: Box<Ics>,
    ics2: Box<Ics>,
    spec_data1: Box<[i32; FRAME_LENGTH]>,
    spec_data2: Box<[i32; FRAME_LENGTH]>,
    spec_coef1: Box<[f32; FRAME_LENGTH]>,
    spec_coef2: Box<[f32; FRAME_LENGTH]>,
    time_out: Vec<[f32; FRAME_LENGTH]>,
    transf_buf: Box<[f32; 2 * FRAME_LENGTH]>,
    mdct2048: Mdct,
    mdct256: Mdct,
    /// What the frames used, for the tests to see that the fixtures reach
    /// each tool: short-window channels, M/S, intensity stereo, TNS, pulses.
    #[cfg(test)]
    pub(crate) seen: [usize; 5],
}

impl AacDecoder {
    fn new(sf_index: usize, channels: usize) -> AacDecoder {
        AacDecoder {
            sample_rate: SAMPLE_RATES[sf_index],
            channels,
            sf_index,
            overlap: vec![[0.0; FRAME_LENGTH]; channels],
            window_shape_prev: vec![0; channels],
            r1: 0x2bb431ea,
            r2: 0x206155b7,
            ics1: Box::new(Ics::new()),
            ics2: Box::new(Ics::new()),
            spec_data1: Box::new([0; FRAME_LENGTH]),
            spec_data2: Box::new([0; FRAME_LENGTH]),
            spec_coef1: Box::new([0.0; FRAME_LENGTH]),
            spec_coef2: Box::new([0.0; FRAME_LENGTH]),
            time_out: vec![[0.0; FRAME_LENGTH]; channels],
            transf_buf: Box::new([0.0; 2 * FRAME_LENGTH]),
            mdct2048: Mdct::new(2048),
            mdct256: Mdct::new(256),
            #[cfg(test)]
            seen: [0; 5],
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// From an AudioSpecificConfig (ISO 14496-3 1.6.2.1), as an MP4's esds
    /// carries it. Only AAC-LC, mono or stereo, 1024-sample frames.
    pub fn from_config(asc: &[u8]) -> Result<AacDecoder, Error> {
        let mut ld = BitReader::new(asc);
        let mut object_type = ld.get_bits(5);
        if object_type == 31 {
            object_type = 32 + ld.get_bits(6);
        }
        let sf_index = ld.get_bits(4) as usize;
        if sf_index == 15 {
            return Err(unsupported("An explicit AAC sample rate is not supported."));
        }
        let channel_configuration = ld.get_bits(4);
        if object_type == 5 || object_type == 29 {
            return Err(unsupported("HE-AAC (SBR) is not supported."));
        }
        if object_type != 2 {
            return Err(unsupported(format!("AAC object type {object_type} is not supported, only LC (2).")));
        }
        if sf_index >= 12 {
            return Err(damaged("AAC sample rate index out of range."));
        }
        // GASpecificConfig
        if ld.get1bit() != 0 {
            return Err(unsupported("960-sample AAC frames are not supported."));
        }
        if ld.get1bit() != 0 {
            ld.get_bits(14); // dependsOnCoreCoder: coreCoderDelay
        }
        ld.get1bit(); // extensionFlag, 0 for LC
        if !(1..=2).contains(&channel_configuration) {
            return Err(unsupported(format!("AAC channel configuration {channel_configuration} is not supported.")));
        }
        // A backward-compatible SBR signal after the config (sync word
        // 0x2b7): an HE-AAC stream that would play at twice the rate.
        if ld.bits_left() >= 16 && ld.show_bits(11) == 0x2b7 {
            ld.get_bits(11);
            let ext = ld.get_bits(5);
            if ext == 5 && ld.get1bit() != 0 {
                return Err(unsupported("HE-AAC (SBR) is not supported."));
            }
        }
        Ok(AacDecoder::new(sf_index, channel_configuration as usize))
    }

    /// Forgets the past (the overlap and window shapes), for a seek: the
    /// frame after the next one decoded is then exact.
    pub fn reset(&mut self) {
        for o in &mut self.overlap {
            o.fill(0.0);
        }
        self.window_shape_prev.fill(0);
    }

    /// One raw_data_block (an MP4 sample) into `pcm`: 1024 frames,
    /// interleaved, as floats from -1 to 1.
    pub fn decode_frame(&mut self, data: &[u8], pcm: &mut [f32]) -> Result<(), Error> {
        if pcm.len() < FRAME_LENGTH * self.channels {
            return Err(damaged("AAC output buffer too small."));
        }
        let mut ld = BitReader::new(data);
        let mut channels = 0;
        loop {
            let id = ld.get_bits(3);
            if id == ID_END {
                break;
            }
            match id {
                ID_SCE | ID_LFE => {
                    if channels + 1 > self.channels {
                        return Err(damaged("More AAC channels than configured."));
                    }
                    self.single_lfe_channel_element(&mut ld, channels)?;
                    channels += 1;
                }
                ID_CPE => {
                    if channels + 2 > self.channels {
                        return Err(damaged("More AAC channels than configured."));
                    }
                    self.channel_pair_element(&mut ld, channels)?;
                    channels += 2;
                }
                ID_CCE => return Err(unsupported("AAC coupling channels are not supported.")),
                ID_DSE => data_stream_element(&mut ld),
                ID_PCE => program_config_element(&mut ld),
                ID_FIL => fill_element(&mut ld)?,
                _ => unreachable!("three bits"),
            }
            if ld.error {
                return Err(damaged("An AAC frame ends early."));
            }
        }
        if channels != self.channels {
            return Err(damaged("An AAC frame without all its channels."));
        }

        const SCALE: f32 = 1.0 / 32768.0; // FAAD2's floats are on the 16-bit scale
        for (c, t) in self.time_out.iter().enumerate() {
            for (i, v) in t.iter().enumerate() {
                pcm[i * self.channels + c] = v * SCALE;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ elements

    fn single_lfe_channel_element(&mut self, ld: &mut BitReader, channel: usize) -> Result<(), Error> {
        ld.get_bits(4); // element_instance_tag
        let ics = &mut *self.ics1;
        ics.reset();
        self.spec_data1.fill(0);
        individual_channel_stream(ld, ics, false, &mut self.spec_data1, self.sf_index)?;
        #[cfg(test)]
        count(&mut self.seen, ics);
        if ics.is_used {
            return Err(damaged("Intensity stereo in a single channel."));
        }
        // reconstruct_single_channel
        quant_to_spec(ics, &self.spec_data1, &mut self.spec_coef1)?;
        pns_decode(ics, None, &mut self.spec_coef1, None, false, &mut self.r1, &mut self.r2);
        tns_decode_frame(ics, &mut self.spec_coef1, self.sf_index);
        let fb =
            FilterBank { transf_buf: &mut self.transf_buf, mdct2048: &mut self.mdct2048, mdct256: &mut self.mdct256 };
        let (Some(time_out), Some(overlap), Some(prev)) =
            (self.time_out.get_mut(channel), self.overlap.get_mut(channel), self.window_shape_prev.get_mut(channel))
        else {
            return Err(damaged("AAC channel out of range."));
        };
        fb.ifilter_bank(ics.window_sequence, ics.window_shape, *prev, &self.spec_coef1, time_out, overlap);
        *prev = ics.window_shape;
        Ok(())
    }

    fn channel_pair_element(&mut self, ld: &mut BitReader, channel: usize) -> Result<(), Error> {
        self.spec_data1.fill(0);
        self.spec_data2.fill(0);
        let ics1 = &mut *self.ics1;
        let ics2 = &mut *self.ics2;
        ics1.reset();
        ics2.reset();
        ld.get_bits(4); // element_instance_tag
        let common_window = ld.get1bit() != 0;
        if common_window {
            ics_info(ld, ics1, self.sf_index)?;
            ics1.ms_mask_present = ld.get_bits(2) as usize;
            if ics1.ms_mask_present == 3 {
                return Err(damaged("Bad AAC M/S mask."));
            }
            if ics1.ms_mask_present == 1 {
                for g in 0..ics1.num_window_groups {
                    for sfb in 0..ics1.max_sfb {
                        ics1.ms_used[g][sfb] = ld.get1bit() as u8;
                    }
                }
            }
            ics2.clone_from(ics1);
        } else {
            ics1.ms_mask_present = 0;
        }
        individual_channel_stream(ld, ics1, common_window, &mut self.spec_data1, self.sf_index)?;
        individual_channel_stream(ld, ics2, common_window, &mut self.spec_data2, self.sf_index)?;
        #[cfg(test)]
        {
            count(&mut self.seen, ics1);
            count(&mut self.seen, ics2);
        }

        // reconstruct_channel_pair
        quant_to_spec(ics1, &self.spec_data1, &mut self.spec_coef1)?;
        quant_to_spec(ics2, &self.spec_data2, &mut self.spec_coef2)?;
        let pair = ics1.ms_mask_present != 0;
        pns_decode(
            ics1,
            Some(&*ics2),
            &mut self.spec_coef1,
            Some(&mut *self.spec_coef2),
            pair,
            &mut self.r1,
            &mut self.r2,
        );
        ms_decode(ics1, ics2, &mut self.spec_coef1, &mut self.spec_coef2);
        is_decode(ics1, ics2, &self.spec_coef1, &mut self.spec_coef2);
        tns_decode_frame(ics1, &mut self.spec_coef1, self.sf_index);
        tns_decode_frame(ics2, &mut self.spec_coef2, self.sf_index);
        if channel + 2 > self.channels {
            return Err(damaged("AAC channel out of range."));
        }
        let (t1, t2) = self.time_out[channel..channel + 2].split_at_mut(1);
        let (o1, o2) = self.overlap[channel..channel + 2].split_at_mut(1);
        let mut fb =
            FilterBank { transf_buf: &mut self.transf_buf, mdct2048: &mut self.mdct2048, mdct256: &mut self.mdct256 };
        fb.ifilter_bank_mut(
            ics1.window_sequence,
            ics1.window_shape,
            self.window_shape_prev[channel],
            &self.spec_coef1,
            &mut t1[0],
            &mut o1[0],
        );
        fb.ifilter_bank(
            ics2.window_sequence,
            ics2.window_shape,
            self.window_shape_prev[channel + 1],
            &self.spec_coef2,
            &mut t2[0],
            &mut o2[0],
        );
        self.window_shape_prev[channel] = ics1.window_shape;
        self.window_shape_prev[channel + 1] = ics2.window_shape;
        Ok(())
    }
}

impl FrameDecoder for AacDecoder {
    fn rate(&self) -> u32 {
        self.sample_rate
    }
    fn channels(&self) -> u16 {
        self.channels as u16
    }
    fn frame_length(&self) -> usize {
        FRAME_LENGTH
    }
    /// One frame before a seek's own: an AAC frame needs the one before it
    /// only through the transform's overlap (Mp4Track.cs's _preroll).
    fn preroll(&self) -> usize {
        1
    }
    fn decode(&mut self, frame: &[u8], out: &mut [f32]) -> Result<usize, Error> {
        self.decode_frame(frame, out)?;
        Ok(FRAME_LENGTH)
    }
    fn reset(&mut self) {
        AacDecoder::reset(self);
    }
}

#[cfg(test)]
fn count(seen: &mut [usize; 5], ics: &Ics) {
    let used = [
        ics.window_sequence == EIGHT_SHORT_SEQUENCE,
        ics.ms_mask_present != 0,
        ics.is_used,
        ics.tns_data_present,
        ics.pulse_data_present,
    ];
    for (s, u) in seen.iter_mut().zip(used) {
        *s += usize::from(u);
    }
}

fn data_stream_element(ld: &mut BitReader) {
    ld.get_bits(4); // element_instance_tag
    let byte_aligned = ld.get1bit() != 0;
    let mut count = ld.get_bits(8) as usize;
    if count == 255 {
        count += ld.get_bits(8) as usize;
    }
    if byte_aligned {
        ld.byte_align();
    }
    ld.skip(count * 8);
}

/// Read and passed over (14496-4 5.6.4.1.2.1.3: a PCE in an access unit is ignored).
fn program_config_element(ld: &mut BitReader) {
    ld.get_bits(4 + 2 + 4); // tag, object type, sf index
    let (front, side, back) = (ld.get_bits(4) as usize, ld.get_bits(4) as usize, ld.get_bits(4) as usize);
    let (lfe, assoc, cc) = (ld.get_bits(2) as usize, ld.get_bits(3) as usize, ld.get_bits(4) as usize);
    if ld.get1bit() != 0 {
        ld.get_bits(4);
    }
    if ld.get1bit() != 0 {
        ld.get_bits(4);
    }
    if ld.get1bit() != 0 {
        ld.get_bits(3);
    }
    ld.skip((front + side + back) * 5 + lfe * 4 + assoc * 4 + cc * 5);
    ld.byte_align();
    let comment = ld.get_bits(8) as usize;
    ld.skip(comment * 8);
}

/// Passed over whole: DRC is not applied, and SBR data here is refused.
fn fill_element(ld: &mut BitReader) -> Result<(), Error> {
    let mut count = ld.get_bits(4) as usize;
    if count == 15 {
        count += ld.get_bits(8) as usize;
        count -= 1;
    }
    if count > 0 && matches!(ld.show_bits(4), 13 | 14) {
        return Err(unsupported("HE-AAC (SBR) is not supported."));
    }
    ld.skip(count * 8);
    Ok(())
}

// ------------------------------------------------------------ the channel stream

fn ics_info(ld: &mut BitReader, ics: &mut Ics, sf_index: usize) -> Result<(), Error> {
    if ld.get1bit() != 0 {
        return Err(damaged("AAC ics_reserved_bit set."));
    }
    ics.window_sequence = ld.get_bits(2) as usize;
    ics.window_shape = ld.get1bit() as usize;
    if ics.window_sequence == EIGHT_SHORT_SEQUENCE {
        ics.max_sfb = ld.get_bits(4) as usize;
        ics.scale_factor_grouping = ld.get_bits(7);
    } else {
        ics.max_sfb = ld.get_bits(6) as usize;
    }
    window_grouping_info(ics, sf_index)?;
    if ics.max_sfb > ics.num_swb {
        return Err(damaged("AAC max_sfb out of range."));
    }
    if ics.window_sequence != EIGHT_SHORT_SEQUENCE && ld.get1bit() != 0 {
        return Err(unsupported("AAC prediction (MAIN or LTP) is not supported."));
    }
    Ok(())
}

fn individual_channel_stream(
    ld: &mut BitReader,
    ics: &mut Ics,
    common_window: bool,
    spec_data: &mut [i32; FRAME_LENGTH],
    sf_index: usize,
) -> Result<(), Error> {
    // side_info
    ics.global_gain = ld.get_bits(8) as i32;
    if !common_window {
        ics_info(ld, ics, sf_index)?;
    }
    section_data(ld, ics)?;
    decode_scale_factors(ld, ics)?;
    ics.pulse_data_present = ld.get1bit() != 0;
    if ics.pulse_data_present {
        pulse_data(ld, ics)?;
    }
    ics.tns_data_present = ld.get1bit() != 0;
    if ics.tns_data_present {
        tns_data(ld, ics);
    }
    if ld.get1bit() != 0 {
        return Err(unsupported("AAC gain control (SSR) is not supported."));
    }

    spectral_data(ld, ics, spec_data)?;
    if ics.pulse_data_present {
        if ics.window_sequence == EIGHT_SHORT_SEQUENCE {
            return Err(damaged("AAC pulse data in short blocks."));
        }
        pulse_decode(ics, spec_data)?;
    }
    Ok(())
}

fn window_grouping_info(ics: &mut Ics, sf: usize) -> Result<(), Error> {
    let (Some(&num_swb_long), Some(&num_swb_short)) = (NUM_SWB_1024_WINDOW.get(sf), NUM_SWB_128_WINDOW.get(sf)) else {
        return Err(damaged("AAC sample rate index out of range."));
    };
    match ics.window_sequence {
        ONLY_LONG_SEQUENCE | LONG_START_SEQUENCE | LONG_STOP_SEQUENCE => {
            ics.num_windows = 1;
            ics.num_window_groups = 1;
            ics.window_group_length[0] = 1;
            ics.num_swb = num_swb_long;
            if ics.max_sfb > ics.num_swb {
                return Err(damaged("AAC max_sfb out of range."));
            }
            let table = SWB_OFFSET_1024_WINDOW[sf];
            for (i, &o) in table.iter().enumerate().take(ics.num_swb) {
                ics.sect_sfb_offset[0][i] = o as usize;
                ics.swb_offset[i] = o as usize;
            }
            ics.sect_sfb_offset[0][ics.num_swb] = FRAME_LENGTH;
            ics.swb_offset[ics.num_swb] = FRAME_LENGTH;
            ics.swb_offset_max = FRAME_LENGTH;
            Ok(())
        }
        EIGHT_SHORT_SEQUENCE => {
            ics.num_windows = 8;
            ics.num_window_groups = 1;
            ics.window_group_length[0] = 1;
            ics.num_swb = num_swb_short;
            if ics.max_sfb > ics.num_swb {
                return Err(damaged("AAC max_sfb out of range."));
            }
            let table = SWB_OFFSET_128_WINDOW[sf];
            for (i, &o) in table.iter().enumerate().take(ics.num_swb) {
                ics.swb_offset[i] = o as usize;
            }
            ics.swb_offset[ics.num_swb] = NSHORT;
            ics.swb_offset_max = NSHORT;
            for i in 0..ics.num_windows - 1 {
                if ics.scale_factor_grouping & (1 << (6 - i)) == 0 {
                    ics.num_window_groups += 1;
                    ics.window_group_length[ics.num_window_groups - 1] = 1;
                } else {
                    ics.window_group_length[ics.num_window_groups - 1] += 1;
                }
            }
            for g in 0..ics.num_window_groups {
                let mut offset = 0;
                for i in 0..ics.num_swb {
                    let width = if i + 1 == ics.num_swb {
                        NSHORT - table[i] as usize
                    } else {
                        (table[i + 1] - table[i]) as usize
                    };
                    ics.sect_sfb_offset[g][i] = offset;
                    offset += width * ics.window_group_length[g];
                }
                ics.sect_sfb_offset[g][ics.num_swb] = offset;
            }
            Ok(())
        }
        _ => Err(damaged("Bad AAC window sequence.")),
    }
}

fn section_data(ld: &mut BitReader, ics: &mut Ics) -> Result<(), Error> {
    let (sect_bits, sect_lim) =
        if ics.window_sequence == EIGHT_SHORT_SEQUENCE { (3, MAX_SECTIONS) } else { (5, MAX_SFB) };
    let sect_esc_val = (1 << sect_bits) - 1;

    for g in 0..ics.num_window_groups {
        let (mut k, mut i) = (0usize, 0usize);
        while k < ics.max_sfb {
            if ld.error {
                return Err(damaged("AAC section data ends early."));
            }
            if i >= sect_lim {
                return Err(damaged("Too many AAC sections."));
            }
            let cb = ld.get_bits(4) as u8;
            if cb == 12 {
                return Err(damaged("AAC codebook 12."));
            }
            ics.sect_cb[g][i] = cb;
            if cb == INTENSITY_HCB2 || cb == INTENSITY_HCB {
                ics.is_used = true;
            }

            let mut sect_len = 0usize;
            let mut sect_len_incr = ld.get_bits(sect_bits) as usize;
            while sect_len_incr == sect_esc_val {
                sect_len += sect_len_incr;
                if sect_len > sect_lim {
                    return Err(damaged("AAC section too long."));
                }
                sect_len_incr = ld.get_bits(sect_bits) as usize;
            }
            sect_len += sect_len_incr;
            // Checked before it is kept (the C# kept it in a byte first).
            if sect_len > sect_lim || k + sect_len > sect_lim {
                return Err(damaged("AAC section too long."));
            }
            ics.sect_start[g][i] = k;
            ics.sect_end[g][i] = k + sect_len;
            for sfb in k..k + sect_len {
                ics.sfb_cb[g][sfb] = cb;
            }
            k += sect_len;
            i += 1;
        }
        ics.num_sec[g] = i;
        if k != ics.max_sfb {
            return Err(damaged("AAC sections do not add up."));
        }
    }
    Ok(())
}

fn decode_scale_factors(ld: &mut BitReader, ics: &mut Ics) -> Result<(), Error> {
    let mut scale_factor = ics.global_gain;
    let mut is_position = 0i32;
    let mut noise_pcm_flag = true;
    let mut noise_energy = ics.global_gain - 90;

    for g in 0..ics.num_window_groups {
        for sfb in 0..ics.max_sfb {
            match ics.sfb_cb[g][sfb] {
                ZERO_HCB => ics.scale_factors[g][sfb] = 0,
                INTENSITY_HCB | INTENSITY_HCB2 => {
                    is_position += huffman_scale_factor(ld)? - 60;
                    ics.scale_factors[g][sfb] = is_position;
                }
                NOISE_HCB => {
                    let t = if noise_pcm_flag {
                        noise_pcm_flag = false;
                        ld.get_bits(9) as i32 - 256
                    } else {
                        huffman_scale_factor(ld)? - 60
                    };
                    noise_energy += t;
                    ics.scale_factors[g][sfb] = noise_energy;
                }
                _ => {
                    scale_factor += huffman_scale_factor(ld)? - 60;
                    if !(0..=255).contains(&scale_factor) {
                        return Err(damaged("AAC scalefactor out of range."));
                    }
                    ics.scale_factors[g][sfb] = scale_factor;
                }
            }
            // A damaged frame can run the positions on and on; they are
            // clamped where they are used, and kept from overflowing here.
            is_position = is_position.clamp(-1 << 20, 1 << 20);
            noise_energy = noise_energy.clamp(-1 << 20, 1 << 20);
        }
    }
    Ok(())
}

fn pulse_data(ld: &mut BitReader, ics: &mut Ics) -> Result<(), Error> {
    ics.number_pulse = ld.get_bits(2) as usize;
    ics.pulse_start_sfb = ld.get_bits(6) as usize;
    if ics.pulse_start_sfb > ics.num_swb {
        return Err(damaged("AAC pulse_start_sfb out of range."));
    }
    for i in 0..ics.number_pulse + 1 {
        ics.pulse_offset[i] = ld.get_bits(5) as usize;
        ics.pulse_amp[i] = ld.get_bits(4) as i32;
    }
    Ok(())
}

fn pulse_decode(ics: &Ics, spec_data: &mut [i32; FRAME_LENGTH]) -> Result<(), Error> {
    let mut k = ics.swb_offset[ics.pulse_start_sfb].min(ics.swb_offset_max);
    for i in 0..=ics.number_pulse {
        k += ics.pulse_offset[i];
        if k >= FRAME_LENGTH {
            return Err(damaged("AAC pulse out of range."));
        }
        if spec_data[k] > 0 {
            spec_data[k] += ics.pulse_amp[i];
        } else {
            spec_data[k] -= ics.pulse_amp[i];
        }
    }
    Ok(())
}

fn tns_data(ld: &mut BitReader, ics: &mut Ics) {
    let (n_filt_bits, length_bits, order_bits) =
        if ics.window_sequence == EIGHT_SHORT_SEQUENCE { (1, 4, 3) } else { (2, 6, 5) };

    for w in 0..ics.num_windows {
        let mut start_coef_bits = 3;
        ics.tns_n_filt[w] = ld.get_bits(n_filt_bits) as usize;
        if ics.tns_n_filt[w] != 0 {
            ics.tns_coef_res[w] = ld.get1bit() as usize;
            if ics.tns_coef_res[w] != 0 {
                start_coef_bits = 4;
            }
        }
        for filt in 0..ics.tns_n_filt[w] {
            ics.tns_length[w][filt] = ld.get_bits(length_bits) as usize;
            ics.tns_order[w][filt] = ld.get_bits(order_bits) as usize;
            if ics.tns_order[w][filt] != 0 {
                ics.tns_direction[w][filt] = ld.get1bit() != 0;
                ics.tns_coef_compress[w][filt] = ld.get1bit() as usize;
                let coef_bits = start_coef_bits - ics.tns_coef_compress[w][filt] as u32;
                for i in 0..ics.tns_order[w][filt] {
                    ics.tns_coef[w][filt][i] = ld.get_bits(coef_bits) as u8;
                }
            }
        }
    }
}

fn spectral_data(ld: &mut BitReader, ics: &Ics, spec_data: &mut [i32; FRAME_LENGTH]) -> Result<(), Error> {
    let mut groups = 0;
    for g in 0..ics.num_window_groups {
        let mut p = groups * NSHORT;
        for i in 0..ics.num_sec[g] {
            let sect_cb = ics.sect_cb[g][i];
            let inc = if sect_cb >= FIRST_PAIR_HCB { 2 } else { 4 };
            let from = ics.sect_sfb_offset[g][ics.sect_start[g][i]];
            let to = ics.sect_sfb_offset[g][ics.sect_end[g][i]];
            match sect_cb {
                ZERO_HCB | NOISE_HCB | INTENSITY_HCB | INTENSITY_HCB2 => p += to.saturating_sub(from),
                _ => {
                    let mut k = from;
                    while k < to {
                        let Some(sp) = spec_data.get_mut(p..p + inc) else {
                            return Err(damaged("AAC spectral data out of range."));
                        };
                        huffman_spectral_data(sect_cb, ld, sp)?;
                        p += inc;
                        k += inc;
                    }
                }
            }
        }
        groups += ics.window_group_length[g];
    }
    Ok(())
}

// ------------------------------------------------------------ Huffman

/// A table's entry, or a damaged frame where the codeword leads outside it.
fn at<T: Copy>(table: &[T], i: usize) -> Result<T, Error> {
    table.get(i).copied().ok_or_else(|| damaged("A bad AAC Huffman codeword."))
}

fn huffman_scale_factor(ld: &mut BitReader) -> Result<i32, Error> {
    let t = &HCB_SF;
    let mut offset = 0usize;
    while at(t, offset * 2 + 1)? != 0 {
        let b = ld.get1bit() as usize;
        offset += at(t, offset * 2 + b)? as usize;
    }
    Ok(at(t, offset * 2)? as i32)
}

const HCB_N: [u32; 12] = [0, 5, 5, 0, 5, 0, 5, 0, 5, 0, 6, 5];

fn hcb_table(cb: u8) -> &'static [u8] {
    match cb {
        1 => &HCB1_1,
        2 => &HCB2_1,
        4 => &HCB4_1,
        6 => &HCB6_1,
        8 => &HCB8_1,
        10 => &HCB10_1,
        11 => &HCB11_1,
        _ => &[],
    }
}

fn hcb_2_quad_table(cb: u8) -> &'static [i8] {
    match cb {
        1 => &HCB1_2,
        2 => &HCB2_2,
        4 => &HCB4_2,
        _ => &[],
    }
}

fn hcb_2_pair_table(cb: u8) -> &'static [i8] {
    match cb {
        6 => &HCB6_2,
        8 => &HCB8_2,
        10 => &HCB10_2,
        11 => &HCB11_2,
        _ => &[],
    }
}

fn hcb_bin_table(cb: u8) -> &'static [i8] {
    match cb {
        5 => &HCB5,
        7 => &HCB7,
        9 => &HCB9,
        _ => &[],
    }
}

fn huffman_sign_bits(ld: &mut BitReader, sp: &mut [i32]) {
    for v in sp {
        if *v != 0 && ld.get1bit() != 0 {
            *v = -*v;
        }
    }
}

fn huffman_get_escape(ld: &mut BitReader, sp: &mut i32) -> Result<(), Error> {
    let x = *sp;
    let neg = match x {
        -16 => true,
        16 => false,
        _ => return Ok(()),
    };
    let mut i = 4;
    while i < 16 {
        if ld.get1bit() == 0 {
            break;
        }
        i += 1;
    }
    if i >= 16 {
        return Err(damaged("Bad AAC escape."));
    }
    let off = ld.get_bits(i) as i32;
    let j = off | (1 << i);
    *sp = if neg { -j } else { j };
    Ok(())
}

/// huffman_2step_quad and huffman_2step_pair: a first lookup of a few bits,
/// and the rest of a long codeword looked up from there. `n` is the
/// table's row width (5 for quads, 3 for pairs).
fn huffman_2step(cb: u8, ld: &mut BitReader, sp: &mut [i32], table: &[i8], n: usize) -> Result<(), Error> {
    let root = hcb_table(cb);
    let root_bits = HCB_N[cb as usize];
    let cw = ld.show_bits(root_bits) as usize;
    let mut offset = at(root, cw * 2)? as usize;
    let extra_bits = at(root, cw * 2 + 1)? as u32;
    if extra_bits != 0 {
        ld.flush_bits(root_bits as usize);
        offset += ld.show_bits(extra_bits) as usize;
        let len = at(table, offset * n)? as u8 as usize;
        ld.flush_bits(len.saturating_sub(root_bits as usize));
    } else {
        let len = at(table, offset * n)? as u8 as usize;
        ld.flush_bits(len);
    }
    for (i, v) in sp.iter_mut().enumerate().take(n - 1) {
        *v = at(table, offset * n + 1 + i)? as i32;
    }
    Ok(())
}

/// huffman_binary_quad and huffman_binary_pair: a bit at a time down a
/// binary tree, until a leaf (its first field not zero).
fn huffman_binary(ld: &mut BitReader, sp: &mut [i32], table: &[i8], n: usize) -> Result<(), Error> {
    let mut offset = 0usize;
    while at(table, offset * n)? == 0 {
        let b = ld.get1bit() as usize;
        offset = offset.wrapping_add_signed(at(table, offset * n + 1 + b)? as isize);
    }
    for (i, v) in sp.iter_mut().enumerate().take(n - 1) {
        *v = at(table, offset * n + 1 + i)? as i32;
    }
    Ok(())
}

fn huffman_spectral_data(cb: u8, ld: &mut BitReader, sp: &mut [i32]) -> Result<(), Error> {
    match cb {
        1 | 2 => huffman_2step(cb, ld, sp, hcb_2_quad_table(cb), 5),
        3 => {
            huffman_binary(ld, sp, &HCB3, 5)?;
            huffman_sign_bits(ld, &mut sp[..4]);
            Ok(())
        }
        4 => {
            huffman_2step(cb, ld, sp, hcb_2_quad_table(cb), 5)?;
            huffman_sign_bits(ld, &mut sp[..4]);
            Ok(())
        }
        5 => huffman_binary(ld, sp, hcb_bin_table(cb), 3),
        6 => huffman_2step(cb, ld, sp, hcb_2_pair_table(cb), 3),
        7 | 9 => {
            huffman_binary(ld, sp, hcb_bin_table(cb), 3)?;
            huffman_sign_bits(ld, &mut sp[..2]);
            Ok(())
        }
        8 | 10 => {
            huffman_2step(cb, ld, sp, hcb_2_pair_table(cb), 3)?;
            huffman_sign_bits(ld, &mut sp[..2]);
            Ok(())
        }
        11 => {
            huffman_2step(11, ld, sp, hcb_2_pair_table(11), 3)?;
            huffman_sign_bits(ld, &mut sp[..2]);
            huffman_get_escape(ld, &mut sp[0])?;
            huffman_get_escape(ld, &mut sp[1])
        }
        _ => Err(damaged("An AAC codebook that does not exist.")),
    }
}

// ------------------------------------------------------------ reconstruction

/// iq_table: x^(4/3) for each quantized magnitude.
fn iq_table() -> &'static [f32; 8192] {
    static T: std::sync::OnceLock<Box<[f32; 8192]>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = Box::new([0.0f32; 8192]);
        for (i, v) in t.iter_mut().enumerate() {
            *v = (i as f64).powf(4.0 / 3.0) as f32;
        }
        t
    })
}

/// pow2sf_tab: 2^(i - 25).
fn pow2sf_tab(i: usize) -> f32 {
    2f64.powi(i as i32 - 25) as f32
}

/// 2^(i/4), as FAAD2 writes it.
#[allow(clippy::excessive_precision)]
const POW2_TABLE: [f32; 4] =
    [1.0, 1.1892071150027210667174999705605, std::f32::consts::SQRT_2, 1.6817928305074290860622509524664];

fn iquant(q: i32, iq: &[f32; 8192]) -> Result<f32, Error> {
    match iq.get(q.unsigned_abs() as usize) {
        Some(&v) => Ok(if q < 0 { -v } else { v }),
        None => Err(damaged("AAC quantized value out of range.")),
    }
}

fn is_intensity_band(ics: &Ics, g: usize, sfb: usize) -> bool {
    matches!(ics.sfb_cb[g][sfb], INTENSITY_HCB | INTENSITY_HCB2)
}

fn is_intensity(ics: &Ics, g: usize, sfb: usize) -> i32 {
    match ics.sfb_cb[g][sfb] {
        INTENSITY_HCB => 1,
        INTENSITY_HCB2 => -1,
        _ => 0,
    }
}

fn is_noise(ics: &Ics, g: usize, sfb: usize) -> bool {
    ics.sfb_cb[g][sfb] == NOISE_HCB
}

fn invert_intensity(ics: &Ics, g: usize, sfb: usize) -> i32 {
    if ics.ms_mask_present == 1 { 1 - 2 * ics.ms_used[g][sfb] as i32 } else { 1 }
}

/// quant_to_spec: dequantisation and scaling, and for short blocks the deinterleaving.
fn quant_to_spec(
    ics: &Ics,
    quant_data: &[i32; FRAME_LENGTH],
    spec_data: &mut [f32; FRAME_LENGTH],
) -> Result<(), Error> {
    let iq = iq_table();
    let (mut k, mut gindex) = (0usize, 0usize);
    if ics.num_swb == 0 {
        spec_data.fill(0.0);
    }
    for g in 0..ics.num_window_groups {
        let (mut j, mut gincrease) = (0usize, 0usize);
        let win_inc = ics.swb_offset[ics.num_swb];
        for sfb in 0..ics.num_swb {
            let mut wa = gindex + j;
            let mut scale_factor = ics.scale_factors[g][sfb];
            let width = ics.swb_offset[sfb + 1].saturating_sub(ics.swb_offset[sfb]);
            if width + 3 >= FRAME_LENGTH {
                return Err(damaged("AAC band too wide."));
            }
            if is_intensity_band(ics, g, sfb) || is_noise(ics, g, sfb) {
                scale_factor = 0;
            }
            let (exp, frac) = ((scale_factor >> 2) as usize, (scale_factor & 3) as usize);
            if exp >= 64 {
                return Err(damaged("AAC scalefactor out of range."));
            }
            let scf = pow2sf_tab(exp) * POW2_TABLE[frac];
            for _ in 0..ics.window_group_length[g] {
                let mut bin = 0;
                while bin < width {
                    let wb = wa + bin;
                    if wb + 4 > FRAME_LENGTH || k + 4 > FRAME_LENGTH {
                        return Err(damaged("AAC spectrum out of range."));
                    }
                    for x in 0..4 {
                        spec_data[wb + x] = iquant(quant_data[k + x], iq)? * scf;
                    }
                    gincrease += 4;
                    k += 4;
                    bin += 4;
                }
                wa += win_inc;
            }
            j += width;
        }
        gindex += gincrease;
    }
    Ok(())
}

fn ms_decode(ics: &Ics, icsr: &Ics, l_spec: &mut [f32; FRAME_LENGTH], r_spec: &mut [f32; FRAME_LENGTH]) {
    if ics.ms_mask_present < 1 {
        return;
    }
    let mut group = 0;
    for g in 0..ics.num_window_groups {
        for _ in 0..ics.window_group_length[g] {
            for sfb in 0..ics.max_sfb {
                if (ics.ms_used[g][sfb] != 0 || ics.ms_mask_present == 2)
                    && !is_intensity_band(icsr, g, sfb)
                    && !is_noise(ics, g, sfb)
                {
                    for i in ics.swb_offset[sfb]..ics.swb_offset[sfb + 1].min(ics.swb_offset_max) {
                        let k = group * NSHORT + i;
                        if k >= FRAME_LENGTH {
                            break;
                        }
                        let tmp = l_spec[k] - r_spec[k];
                        l_spec[k] += r_spec[k];
                        r_spec[k] = tmp;
                    }
                }
            }
            group += 1;
        }
    }
}

fn is_decode(ics: &Ics, icsr: &Ics, l_spec: &[f32; FRAME_LENGTH], r_spec: &mut [f32; FRAME_LENGTH]) {
    let mut group = 0;
    for g in 0..icsr.num_window_groups {
        for _ in 0..icsr.window_group_length[g] {
            for sfb in 0..icsr.max_sfb {
                if !is_intensity_band(icsr, g, sfb) {
                    continue;
                }
                let scale_factor = icsr.scale_factors[g][sfb].clamp(-120, 120);
                let scale = 0.5f64.powf(0.25 * scale_factor as f64) as f32;
                for i in icsr.swb_offset[sfb]..icsr.swb_offset[sfb + 1].min(ics.swb_offset_max) {
                    let k = group * NSHORT + i;
                    if k >= FRAME_LENGTH {
                        break;
                    }
                    r_spec[k] = l_spec[k] * scale;
                    if is_intensity(icsr, g, sfb) != invert_intensity(ics, g, sfb) {
                        r_spec[k] = -r_spec[k];
                    }
                }
            }
            group += 1;
        }
    }
}

/// ne_rng: two polycounters, XORed (common.c).
fn ne_rng(r1: &mut u32, r2: &mut u32) -> u32 {
    let (t3, t4) = (*r1, *r2);
    let t1 = ((t3 & 0xF5).count_ones() & 1) << 31;
    let t2 = ((t4 >> 25) & 0x63).count_ones() & 1;
    *r1 = (t3 >> 1) | t1;
    *r2 = t4.wrapping_add(t4) | t2;
    *r1 ^ *r2
}

fn gen_rand_vector(spec: &mut [f32], scale_factor: i32, r1: &mut u32, r2: &mut u32) {
    let mut energy = 0.0f32;
    let scale_factor = scale_factor.clamp(-120, 120);
    for v in spec.iter_mut() {
        let tmp = ne_rng(r1, r2) as i32 as f32;
        *v = tmp;
        energy += tmp * tmp;
    }
    if energy > 0.0 {
        let mut scale = 1.0 / energy.sqrt();
        scale *= 2f64.powf(0.25 * scale_factor as f64) as f32;
        for v in spec.iter_mut() {
            *v *= scale;
        }
    }
}

fn pns_decode(
    ics_left: &Ics,
    ics_right: Option<&Ics>,
    spec_left: &mut [f32; FRAME_LENGTH],
    mut spec_right: Option<&mut [f32; FRAME_LENGTH]>,
    channel_pair: bool,
    r1: &mut u32,
    r2: &mut u32,
) {
    let mut group = 0;
    for g in 0..ics_left.num_window_groups {
        for _ in 0..ics_left.window_group_length[g] {
            let base = group * NSHORT;
            for sfb in 0..ics_left.max_sfb {
                let (mut r1_dep, mut r2_dep) = (0u32, 0u32);
                if is_noise(ics_left, g, sfb) {
                    let begin = (base + ics_left.swb_offset[sfb]).min(FRAME_LENGTH);
                    let end = (base + ics_left.swb_offset[sfb + 1]).min(FRAME_LENGTH).max(begin);
                    r1_dep = *r1;
                    r2_dep = *r2;
                    gen_rand_vector(&mut spec_left[begin..end], ics_left.scale_factors[g][sfb], r1, r2);
                }
                if let (Some(icsr), Some(spec_right)) = (ics_right, spec_right.as_deref_mut())
                    && is_noise(icsr, g, sfb)
                {
                    let begin = (base + icsr.swb_offset[sfb]).min(FRAME_LENGTH);
                    let end = (base + icsr.swb_offset[sfb + 1]).min(FRAME_LENGTH).max(begin);
                    if channel_pair
                        && is_noise(ics_left, g, sfb)
                        && ((ics_left.ms_mask_present == 1 && ics_left.ms_used[g][sfb] != 0)
                            || ics_left.ms_mask_present == 2)
                    {
                        // The same noise as the left (FAAD2: correlated by the M/S mask).
                        gen_rand_vector(
                            &mut spec_right[begin..end],
                            icsr.scale_factors[g][sfb],
                            &mut r1_dep,
                            &mut r2_dep,
                        );
                    } else {
                        gen_rand_vector(&mut spec_right[begin..end], icsr.scale_factors[g][sfb], r1, r2);
                    }
                }
            }
            group += 1;
        }
    }
}

// ------------------------------------------------------------ TNS

const TNS_SBF_MAX: [[usize; 2]; 13] = [
    [31, 9],
    [31, 9],
    [34, 10],
    [40, 14],
    [42, 14],
    [51, 14],
    [46, 14],
    [46, 14],
    [42, 14],
    [42, 14],
    [42, 14],
    [39, 14],
    [39, 14],
];
static ALL_TNS_COEFS: [&[f32; 16]; 4] = [&TNS_COEF0_3, &TNS_COEF0_4, &TNS_COEF1_3, &TNS_COEF1_4];

fn tns_decode_frame(ics: &Ics, spec: &mut [f32; FRAME_LENGTH], sf_index: usize) {
    if !ics.tns_data_present {
        return;
    }
    let mut lpc = [0f32; TNS_MAX_ORDER + 1];
    let max_tns =
        TNS_SBF_MAX.get(sf_index).map_or(0, |r| r[if ics.window_sequence == EIGHT_SHORT_SEQUENCE { 1 } else { 0 }]);
    for w in 0..ics.num_windows {
        let mut bottom = ics.num_swb;
        for f in 0..ics.tns_n_filt[w] {
            let top = bottom;
            bottom = top.saturating_sub(ics.tns_length[w][f]);
            let tns_order = ics.tns_order[w][f].min(TNS_MAX_ORDER);
            if tns_order == 0 {
                continue;
            }
            tns_decode_coef(
                tns_order,
                ics.tns_coef_res[w] + 3,
                ics.tns_coef_compress[w][f],
                &ics.tns_coef[w][f],
                &mut lpc,
            );

            let start = ics.swb_offset[bottom.min(max_tns).min(ics.max_sfb)].min(ics.swb_offset_max);
            let end = ics.swb_offset[top.min(max_tns).min(ics.max_sfb)].min(ics.swb_offset_max);
            if end <= start {
                continue;
            }
            let base = w * NSHORT;
            if base + end > FRAME_LENGTH {
                continue;
            }
            tns_ar_filter(&mut spec[base + start..base + end], ics.tns_direction[w][f], &lpc, tns_order);
        }
    }
}

fn tns_decode_coef(
    order: usize,
    coef_res_bits: usize,
    coef_compress: usize,
    coef: &[u8; 32],
    a: &mut [f32; TNS_MAX_ORDER + 1],
) {
    let mut tmp2 = [0f32; TNS_MAX_ORDER + 1];
    let mut b = [0f32; TNS_MAX_ORDER + 1];
    let table_index = 2 * usize::from(coef_compress != 0) + usize::from(coef_res_bits != 3);
    let tns_coef = ALL_TNS_COEFS[table_index];
    for i in 0..order {
        tmp2[i] = tns_coef[coef[i] as usize & 15];
    }
    a[0] = 1.0;
    for m in 1..=order {
        a[m] = tmp2[m - 1];
        for i in 1..m {
            b[i] = a[i] + a[m] * a[m - i];
        }
        a[1..m].copy_from_slice(&b[1..m]);
    }
}

/// tns_ar_filter over `spectrum`, upward, or downward when `down` (FAAD2
/// starts at end - 1 with inc = -1).
fn tns_ar_filter(spectrum: &mut [f32], down: bool, lpc: &[f32; TNS_MAX_ORDER + 1], order: usize) {
    let mut state = [0f32; 2 * TNS_MAX_ORDER];
    let mut state_index = 0usize;
    let size = spectrum.len();
    for n in 0..size {
        let at = if down { size - 1 - n } else { n };
        let mut y = 0.0f32;
        for j in 0..order {
            y += state[state_index + j] * lpc[j + 1];
        }
        y = spectrum[at] - y;
        state_index = if state_index == 0 { order - 1 } else { state_index - 1 };
        state[state_index] = y;
        state[state_index + order] = y;
        spectrum[at] = y;
    }
}

// ------------------------------------------------------------ filterbank

static LONG_WINDOW: [&[f32; 1024]; 2] = [&SINE_LONG1024, &KBD_LONG1024];
static SHORT_WINDOW: [&[f32; 128]; 2] = [&SINE_SHORT128, &KBD_SHORT128];

/// What ifilter_bank works with, borrowed from the decoder apart from the
/// channel's own buffers.
struct FilterBank<'a> {
    transf_buf: &'a mut [f32; 2 * FRAME_LENGTH],
    mdct2048: &'a mut Mdct,
    mdct256: &'a mut Mdct,
}

impl FilterBank<'_> {
    fn ifilter_bank(
        mut self,
        window_sequence: usize,
        window_shape: usize,
        window_shape_prev: usize,
        freq_in: &[f32; FRAME_LENGTH],
        time_out: &mut [f32; FRAME_LENGTH],
        overlap: &mut [f32; FRAME_LENGTH],
    ) {
        self.ifilter_bank_mut(window_sequence, window_shape, window_shape_prev, freq_in, time_out, overlap);
    }

    fn ifilter_bank_mut(
        &mut self,
        window_sequence: usize,
        window_shape: usize,
        window_shape_prev: usize,
        freq_in: &[f32; FRAME_LENGTH],
        time_out: &mut [f32; FRAME_LENGTH],
        overlap: &mut [f32; FRAME_LENGTH],
    ) {
        const NLONG: usize = FRAME_LENGTH;
        const TRANS: usize = NSHORT / 2;
        const NFLAT_LS: usize = (NLONG - NSHORT) / 2;
        let tb = &mut *self.transf_buf;
        tb.fill(0.0);
        let window_long = LONG_WINDOW[window_shape & 1];
        let window_long_prev = LONG_WINDOW[window_shape_prev & 1];
        let window_short = SHORT_WINDOW[window_shape & 1];
        let window_short_prev = SHORT_WINDOW[window_shape_prev & 1];

        match window_sequence {
            ONLY_LONG_SEQUENCE => {
                self.mdct2048.imdct(freq_in, tb);
                for i in 0..NLONG {
                    time_out[i] = overlap[i] + tb[i] * window_long_prev[i];
                }
                for i in 0..NLONG {
                    overlap[i] = tb[NLONG + i] * window_long[NLONG - 1 - i];
                }
            }
            LONG_START_SEQUENCE => {
                self.mdct2048.imdct(freq_in, tb);
                for i in 0..NLONG {
                    time_out[i] = overlap[i] + tb[i] * window_long_prev[i];
                }
                overlap[..NFLAT_LS].copy_from_slice(&tb[NLONG..NLONG + NFLAT_LS]);
                for i in 0..NSHORT {
                    overlap[NFLAT_LS + i] = tb[NLONG + NFLAT_LS + i] * window_short[NSHORT - i - 1];
                }
                overlap[NFLAT_LS + NSHORT..].fill(0.0);
            }
            EIGHT_SHORT_SEQUENCE => {
                for w in 0..8 {
                    let x_in = &freq_in[w * NSHORT..(w + 1) * NSHORT];
                    self.mdct256.imdct(x_in, &mut tb[2 * NSHORT * w..2 * NSHORT * (w + 1)]);
                }
                time_out[..NFLAT_LS].copy_from_slice(&overlap[..NFLAT_LS]);
                let ws = window_short;
                for i in 0..NSHORT {
                    time_out[NFLAT_LS + i] = overlap[NFLAT_LS + i] + tb[i] * window_short_prev[i];
                    time_out[NFLAT_LS + NSHORT + i] = overlap[NFLAT_LS + NSHORT + i]
                        + tb[NSHORT + i] * ws[NSHORT - 1 - i]
                        + tb[NSHORT * 2 + i] * ws[i];
                    time_out[NFLAT_LS + 2 * NSHORT + i] = overlap[NFLAT_LS + NSHORT * 2 + i]
                        + tb[NSHORT * 3 + i] * ws[NSHORT - 1 - i]
                        + tb[NSHORT * 4 + i] * ws[i];
                    time_out[NFLAT_LS + 3 * NSHORT + i] = overlap[NFLAT_LS + NSHORT * 3 + i]
                        + tb[NSHORT * 5 + i] * ws[NSHORT - 1 - i]
                        + tb[NSHORT * 6 + i] * ws[i];
                    if i < TRANS {
                        time_out[NFLAT_LS + 4 * NSHORT + i] = overlap[NFLAT_LS + NSHORT * 4 + i]
                            + tb[NSHORT * 7 + i] * ws[NSHORT - 1 - i]
                            + tb[NSHORT * 8 + i] * ws[i];
                    }
                }
                for i in 0..NSHORT {
                    if i >= TRANS {
                        overlap[NFLAT_LS + 4 * NSHORT + i - NLONG] =
                            tb[NSHORT * 7 + i] * ws[NSHORT - 1 - i] + tb[NSHORT * 8 + i] * ws[i];
                    }
                    overlap[NFLAT_LS + 5 * NSHORT + i - NLONG] =
                        tb[NSHORT * 9 + i] * ws[NSHORT - 1 - i] + tb[NSHORT * 10 + i] * ws[i];
                    overlap[NFLAT_LS + 6 * NSHORT + i - NLONG] =
                        tb[NSHORT * 11 + i] * ws[NSHORT - 1 - i] + tb[NSHORT * 12 + i] * ws[i];
                    overlap[NFLAT_LS + 7 * NSHORT + i - NLONG] =
                        tb[NSHORT * 13 + i] * ws[NSHORT - 1 - i] + tb[NSHORT * 14 + i] * ws[i];
                    overlap[NFLAT_LS + 8 * NSHORT + i - NLONG] = tb[NSHORT * 15 + i] * ws[NSHORT - 1 - i];
                }
                overlap[NFLAT_LS + NSHORT..].fill(0.0);
            }
            LONG_STOP_SEQUENCE => {
                self.mdct2048.imdct(freq_in, tb);
                time_out[..NFLAT_LS].copy_from_slice(&overlap[..NFLAT_LS]);
                for i in 0..NSHORT {
                    time_out[NFLAT_LS + i] = overlap[NFLAT_LS + i] + tb[NFLAT_LS + i] * window_short_prev[i];
                }
                for i in 0..NFLAT_LS {
                    time_out[NFLAT_LS + NSHORT + i] = overlap[NFLAT_LS + NSHORT + i] + tb[NFLAT_LS + NSHORT + i];
                }
                for i in 0..NLONG {
                    overlap[i] = tb[NLONG + i] * window_long[NLONG - 1 - i];
                }
            }
            _ => {} // two bits, all four handled; ics_info refuses nothing else
        }
    }
}

// ------------------------------------------------------------ state

/// ic_stream: one channel's side information for one frame.
#[derive(Clone)]
struct Ics {
    max_sfb: usize,
    num_swb: usize,
    num_window_groups: usize,
    num_windows: usize,
    window_sequence: usize,
    window_shape: usize,
    scale_factor_grouping: u32,
    window_group_length: [usize; 8],
    swb_offset: [usize; 52],
    swb_offset_max: usize,
    sect_sfb_offset: [[usize; MAX_SECTIONS + 1]; 8],
    sect_cb: [[u8; MAX_SECTIONS]; 8],
    sect_start: [[usize; MAX_SECTIONS]; 8],
    sect_end: [[usize; MAX_SECTIONS]; 8],
    sfb_cb: [[u8; MAX_SECTIONS]; 8],
    num_sec: [usize; 8],
    global_gain: i32,
    scale_factors: [[i32; MAX_SFB]; 8],
    ms_mask_present: usize,
    ms_used: [[u8; MAX_SFB]; 8],
    is_used: bool,
    pulse_data_present: bool,
    tns_data_present: bool,
    number_pulse: usize,
    pulse_start_sfb: usize,
    pulse_offset: [usize; 4],
    pulse_amp: [i32; 4],
    tns_n_filt: [usize; 8],
    tns_coef_res: [usize; 8],
    tns_length: [[usize; 4]; 8],
    tns_order: [[usize; 4]; 8],
    tns_direction: [[bool; 4]; 8],
    tns_coef_compress: [[usize; 4]; 8],
    tns_coef: [[[u8; 32]; 4]; 8],
}

impl Ics {
    fn new() -> Ics {
        Ics {
            max_sfb: 0,
            num_swb: 0,
            num_window_groups: 0,
            num_windows: 0,
            window_sequence: 0,
            window_shape: 0,
            scale_factor_grouping: 0,
            window_group_length: [0; 8],
            swb_offset: [0; 52],
            swb_offset_max: 0,
            sect_sfb_offset: [[0; MAX_SECTIONS + 1]; 8],
            sect_cb: [[0; MAX_SECTIONS]; 8],
            sect_start: [[0; MAX_SECTIONS]; 8],
            sect_end: [[0; MAX_SECTIONS]; 8],
            sfb_cb: [[0; MAX_SECTIONS]; 8],
            num_sec: [0; 8],
            global_gain: 0,
            scale_factors: [[0; MAX_SFB]; 8],
            ms_mask_present: 0,
            ms_used: [[0; MAX_SFB]; 8],
            is_used: false,
            pulse_data_present: false,
            tns_data_present: false,
            number_pulse: 0,
            pulse_start_sfb: 0,
            pulse_offset: [0; 4],
            pulse_amp: [0; 4],
            tns_n_filt: [0; 8],
            tns_coef_res: [0; 8],
            tns_length: [[0; 4]; 8],
            tns_order: [[0; 4]; 8],
            tns_direction: [[false; 4]; 8],
            tns_coef_compress: [[0; 4]; 8],
            tns_coef: [[[0; 32]; 4]; 8],
        }
    }

    /// element sce = {0}: a fresh one for every element.
    fn reset(&mut self) {
        *self = Ics::new();
    }
}

/// MSB-first bits from a byte slice, as FAAD2's bitfile reads them: past
/// the end it gives zeros and says so.
struct BitReader<'a> {
    data: &'a [u8],
    /// Bits from the start.
    pos: usize,
    limit: usize,
    error: bool,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> BitReader<'a> {
        BitReader { data, pos: 0, limit: data.len() * 8, error: false }
    }

    fn bits_left(&self) -> usize {
        self.limit.saturating_sub(self.pos)
    }

    fn bit(&self, p: usize) -> u32 {
        if p < self.limit { (self.data[p >> 3] >> (7 - (p & 7))) as u32 & 1 } else { 0 }
    }

    /// The next `n` bits (at most 32), without moving on. The end is on a
    /// byte boundary, so the bytes past it, read as zeros, give the zeros.
    fn show_bits(&self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let first = self.pos >> 3;
        let mut word = 0u64;
        for k in 0..5 {
            let b = first.checked_add(k).and_then(|i| self.data.get(i)).copied().unwrap_or(0);
            word = word << 8 | b as u64;
        }
        let used = (self.pos & 7) as u32;
        ((word << (24 + used)) >> (64 - n)) as u32
    }

    fn flush_bits(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n);
        if self.pos > self.limit {
            self.error = true;
        }
    }

    fn get_bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let r = self.show_bits(n);
        self.flush_bits(n as usize);
        r
    }

    fn get1bit(&mut self) -> u32 {
        let r = self.bit(self.pos);
        self.flush_bits(1);
        r
    }

    fn skip(&mut self, n: usize) {
        self.flush_bits(n);
    }

    fn byte_align(&mut self) {
        let r = self.pos & 7;
        if r != 0 {
            self.flush_bits(8 - r);
        }
    }
}

/// The IMDCT (mdct.c, floating point): pre-twiddle, a complex FFT of N/4
/// points, post-twiddle, reorder.
struct Mdct {
    n: usize,
    /// FAAD2's float table: sqrt(2/N)·(cos, sin)(2π(k + 1/8)/N).
    sincos: &'static [f32],
    re: Vec<f32>,
    im: Vec<f32>,
    fft: Fft,
}

impl Mdct {
    fn new(n: usize) -> Mdct {
        Mdct {
            n,
            sincos: if n == 2048 { &MDCT_TAB2048 } else { &MDCT_TAB256 },
            re: vec![0.0; n / 4],
            im: vec![0.0; n / 4],
            fft: Fft::new(n / 4),
        }
    }

    /// N/2 coefficients from `x_in` into N samples in `x_out`.
    fn imdct(&mut self, x_in: &[f32], x_out: &mut [f32]) {
        let (n2, n4, n8) = (self.n >> 1, self.n >> 2, self.n >> 3);
        let sc = self.sincos;
        let (re, im) = (&mut self.re, &mut self.im);
        for k in 0..n4 {
            // ComplexMult(&IM(Z1[k]), &RE(Z1[k]), X_in[2k], X_in[N2-1-2k], RE(sincos[k]), IM(sincos[k]))
            let (x1, x2, c1, c2) = (x_in[2 * k], x_in[n2 - 1 - 2 * k], sc[2 * k], sc[2 * k + 1]);
            im[k] = x1 * c1 + x2 * c2;
            re[k] = x2 * c1 - x1 * c2;
        }
        self.fft.backward(re, im);
        for k in 0..n4 {
            // ComplexMult(&IM(Z1[k]), &RE(Z1[k]), IM(x), RE(x), RE(sincos[k]), IM(sincos[k]))
            let (x1, x2, c1, c2) = (im[k], re[k], sc[2 * k], sc[2 * k + 1]);
            im[k] = x1 * c1 + x2 * c2;
            re[k] = x2 * c1 - x1 * c2;
        }
        let o = x_out;
        let mut k = 0;
        while k < n8 {
            o[2 * k] = im[n8 + k];
            o[2 + 2 * k] = im[n8 + 1 + k];
            o[1 + 2 * k] = -re[n8 - 1 - k];
            o[3 + 2 * k] = -re[n8 - 2 - k];
            o[n4 + 2 * k] = re[k];
            o[n4 + 2 + 2 * k] = re[1 + k];
            o[n4 + 1 + 2 * k] = -im[n4 - 1 - k];
            o[n4 + 3 + 2 * k] = -im[n4 - 2 - k];
            o[n2 + 2 * k] = re[n8 + k];
            o[n2 + 2 + 2 * k] = re[n8 + 1 + k];
            o[n2 + 1 + 2 * k] = -im[n8 - 1 - k];
            o[n2 + 3 + 2 * k] = -im[n8 - 2 - k];
            o[n2 + n4 + 2 * k] = -im[k];
            o[n2 + n4 + 2 + 2 * k] = -im[1 + k];
            o[n2 + n4 + 1 + 2 * k] = re[n4 - 1 - k];
            o[n2 + n4 + 3 + 2 * k] = re[n4 - 2 - k];
            k += 2;
        }
    }
}

/// An unnormalised complex FFT of a power-of-two size, with FFTPACK's
/// backward sign (cfftb: exp(+i 2 pi j k / n)).
struct Fft {
    n: usize,
    reverse: Vec<usize>,
    cos: Vec<f32>,
    sin: Vec<f32>,
}

impl Fft {
    fn new(n: usize) -> Fft {
        let bits = n.trailing_zeros();
        let reverse = (0..n)
            .map(|i| {
                let (mut r, mut x) = (0, i);
                for _ in 0..bits {
                    r = (r << 1) | (x & 1);
                    x >>= 1;
                }
                r
            })
            .collect();
        let angle = |i: usize| 2.0 * std::f64::consts::PI * i as f64 / n as f64;
        Fft {
            n,
            reverse,
            cos: (0..n / 2).map(|i| angle(i).cos() as f32).collect(),
            sin: (0..n / 2).map(|i| angle(i).sin() as f32).collect(),
        }
    }

    fn backward(&self, re: &mut [f32], im: &mut [f32]) {
        let n = self.n;
        for i in 0..n {
            let j = self.reverse[i];
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut size = 2;
        while size <= n {
            let half = size >> 1;
            let step = n / size;
            let mut start = 0;
            while start < n {
                for k in 0..half {
                    let (wr, wi) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
                start += size;
            }
            size <<= 1;
        }
    }
}
