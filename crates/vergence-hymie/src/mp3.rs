//! MP3 (MPEG-1, 2 and 2.5 Layer III) decoded in Rust: the codec core, Hymie's
//! Mp3.cs carried over. Mp3.cs is minimp3 (github.com/lieff/minimp3,
//! minimp3.h at ea99364) ported line for line, pointers and all; minimp3 is
//! lieff's, dedicated to the public domain (CC0 1.0), and so is this port of
//! it. The functions keep minimp3's names (snake_case, as in the C) so that
//! a fix can be carried between the C, the C# and this; a comment names the
//! function where the name alone does not.
//!
//! What changed in the carrying: pointers became slices and offsets into
//! them, with no `unsafe`. Where the C# walks past a buffer on damaged input
//! (the Huffman reader looks ahead of the bits it uses, and nothing but the
//! bit counts keeps it in its buffer), reads here past the end give zeros
//! instead, and the main-data buffer keeps the C#'s 64 bytes of slack so that
//! a good file reads the same bytes either way. Shifts by a count the data
//! chooses are wrapping, as C#'s are (the count masked to 0..31). Left out,
//! as in Mp3.cs: Layers I and II, the SIMD paths, the 16-bit output. The
//! tables are in mp3_tables.rs, generated from Mp3Tables.cs by a script.
//!
//! What a track needs around this (the Xing/LAME tag, gapless trimming, ID3,
//! seeking) is Mp3Track's, below the decoder (Mp3Track.cs).

// Indexed loops and minimp3's constants as the C writes them, so the code
// reads beside it.
#![allow(clippy::needless_range_loop, clippy::excessive_precision)]

use crate::mp3_tables::*;
use crate::tags::{self, Tags};
use crate::{Error, ReplayGain, Track, damaged};

pub const MAX_SAMPLES_PER_FRAME: usize = 1152 * 2;

const MAX_FREE_FORMAT_FRAME_SIZE: usize = 2304; // more than ISO spec's
const MAX_FRAME_SYNC_MATCHES: usize = 10;
const MAX_L3_FRAME_PAYLOAD_BYTES: usize = MAX_FREE_FORMAT_FRAME_SIZE;
const MAX_BIT_RESERVOIR_BYTES: usize = 511;
const SHORT_BLOCK_TYPE: u8 = 2;
const STOP_BLOCK_TYPE: u8 = 3;
const HDR_SIZE: usize = 4;
const BITS_DEQUANTIZER_OUT: i32 = -1;
const MAX_SCF: i32 = 255 + BITS_DEQUANTIZER_OUT * 4 - 210;
const MAX_SCFI: i32 = (MAX_SCF + 3) & !3;
/// The main-data buffer: the reservoir, the largest frame, and the slack the
/// Huffman reader may look into (Mp3.cs's 64).
const MAIN_DATA_SIZE: usize = MAX_BIT_RESERVOIR_BYTES + MAX_L3_FRAME_PAYLOAD_BYTES + 64;

#[derive(Debug, Clone, Copy, Default)]
pub struct FrameInfo {
    pub frame_bytes: usize,
    pub frame_offset: usize,
    pub channels: usize,
    pub hz: u32,
    pub layer: u32,
    pub bitrate_kbps: u32,
}

/// The bit reader (minimp3's bs_t): `buf` in place of its pointer.
struct Bs<'a> {
    buf: &'a [u8],
    pos: i32,
    limit: i32,
}

impl<'a> Bs<'a> {
    /// bs_init.
    fn new(buf: &'a [u8], bytes: usize) -> Bs<'a> {
        Bs { buf, pos: 0, limit: (bytes.min(buf.len()) * 8) as i32 }
    }

    fn byte(&self, i: usize) -> u32 {
        self.buf.get(i).copied().unwrap_or(0) as u32
    }

    /// get_bits.
    fn get_bits(&mut self, n: i32) -> u32 {
        let s = (self.pos & 7) as u32;
        let mut shl = n + s as i32;
        let mut p = (self.pos >> 3) as usize;
        self.pos += n;
        if self.pos > self.limit {
            return 0;
        }
        let mut cache: u32 = 0;
        let mut next = self.byte(p) & (255 >> s);
        p += 1;
        loop {
            shl -= 8;
            if shl <= 0 {
                break;
            }
            cache |= next.wrapping_shl(shl as u32);
            next = self.byte(p);
            p += 1;
        }
        cache | next.wrapping_shr((-shl) as u32)
    }
}

/// gr_info_t. `sfbtab` is the row of the band-width table it points into.
#[derive(Clone, Copy, Default)]
struct GrInfo {
    sfbtab: &'static [u8],
    part_23_length: u16,
    big_values: u16,
    scalefac_compress: u16,
    global_gain: u8,
    block_type: u8,
    mixed_block_flag: u8,
    n_long_sfb: u8,
    n_short_sfb: u8,
    table_select: [u8; 3],
    region_count: [u8; 3],
    subblock_gain: [u8; 3],
    preflag: u8,
    scalefac_scale: u8,
    count1_table: u8,
    scfsi: u8,
}

/// One decoder: mp3dec_t (what carries from one frame to the next) and
/// mp3dec_scratch_t (one frame's work), as in Mp3.cs's Mp3Decoder.
pub struct Mp3Decoder {
    mdct_overlap: [f32; 2 * 9 * 32],
    qmf_state: [f32; 15 * 2 * 32],
    reserv: i32,
    free_format_bytes: usize,
    header: [u8; HDR_SIZE],
    reserv_buf: [u8; MAX_BIT_RESERVOIR_BYTES],
    // The scratch: the main-data bit reader's place, and its buffers.
    bs_pos: i32,
    bs_limit: i32,
    maindata: [u8; MAIN_DATA_SIZE],
    gr_info: [GrInfo; 4],
    grbuf: [f32; 2 * 576],
    scf: [f32; 40],
    syn: [f32; (18 + 15) * 2 * 32],
    ist_pos: [u8; 2 * 39 + 8],
}

impl Default for Mp3Decoder {
    fn default() -> Self {
        Mp3Decoder::new()
    }
}

// ------------------------------------------------------------ the header
//
// The HDR_ macros, on a header's four bytes (zeros past the data: no header).

/// The four header bytes at `at`, zeros where the data ends.
pub fn h4(d: &[u8], at: usize) -> [u8; 4] {
    let mut h = [0u8; 4];
    for (k, b) in h.iter_mut().enumerate() {
        *b = at.checked_add(k).and_then(|i| d.get(i)).copied().unwrap_or(0);
    }
    h
}

pub fn hdr_is_mono(h: [u8; 4]) -> bool {
    h[3] & 0xC0 == 0xC0
}
fn hdr_is_ms_stereo(h: [u8; 4]) -> bool {
    h[3] & 0xE0 == 0x60
}
fn hdr_is_free_format(h: [u8; 4]) -> bool {
    h[2] & 0xF0 == 0
}
fn hdr_is_crc(h: [u8; 4]) -> bool {
    h[1] & 1 == 0
}
fn hdr_test_padding(h: [u8; 4]) -> bool {
    h[2] & 0x2 != 0
}
pub fn hdr_test_mpeg1(h: [u8; 4]) -> bool {
    h[1] & 0x8 != 0
}
fn hdr_test_not_mpeg25(h: [u8; 4]) -> bool {
    h[1] & 0x10 != 0
}
fn hdr_test_i_stereo(h: [u8; 4]) -> bool {
    h[3] & 0x10 != 0
}
fn hdr_test_ms_stereo(h: [u8; 4]) -> bool {
    h[3] & 0x20 != 0
}
pub fn hdr_get_layer(h: [u8; 4]) -> u32 {
    ((h[1] >> 1) & 3) as u32
}
fn hdr_get_bitrate(h: [u8; 4]) -> u32 {
    (h[2] >> 4) as u32
}
fn hdr_get_sample_rate(h: [u8; 4]) -> u32 {
    ((h[2] >> 2) & 3) as u32
}
fn hdr_get_my_sample_rate(h: [u8; 4]) -> u32 {
    hdr_get_sample_rate(h) + (((h[1] >> 3) & 1) as u32 + ((h[1] >> 4) & 1) as u32) * 3
}
fn hdr_is_frame_576(h: [u8; 4]) -> bool {
    h[1] & 14 == 2
}
fn hdr_is_layer_1(h: [u8; 4]) -> bool {
    h[1] & 6 == 6
}

pub fn hdr_valid(h: [u8; 4]) -> bool {
    h[0] == 0xff
        && ((h[1] & 0xF0) == 0xf0 || (h[1] & 0xFE) == 0xe2)
        && hdr_get_layer(h) != 0
        && hdr_get_bitrate(h) != 15
        && hdr_get_sample_rate(h) != 3
}

pub fn hdr_compare(h1: [u8; 4], h2: [u8; 4]) -> bool {
    hdr_valid(h2)
        && (h1[1] ^ h2[1]) & 0xFE == 0
        && (h1[2] ^ h2[2]) & 0x0C == 0
        && !(hdr_is_free_format(h1) ^ hdr_is_free_format(h2))
}

pub fn hdr_bitrate_kbps(h: [u8; 4]) -> u32 {
    // Layer 0 is no header (hdr_valid): its index would be before the table.
    let i = hdr_test_mpeg1(h) as usize * 45
        + (hdr_get_layer(h) as usize).wrapping_sub(1).wrapping_mul(15)
        + hdr_get_bitrate(h) as usize;
    2 * HALF_RATE.get(i).copied().unwrap_or(0) as u32
}

pub fn hdr_sample_rate_hz(h: [u8; 4]) -> u32 {
    // Index 3 is reserved (hdr_valid refuses it): read as 32 kHz, not past the table.
    const HZ: [u32; 4] = [44100, 48000, 32000, 32000];
    HZ[hdr_get_sample_rate(h) as usize] >> (!hdr_test_mpeg1(h)) as u32 >> (!hdr_test_not_mpeg25(h)) as u32
}

pub fn hdr_frame_samples(h: [u8; 4]) -> usize {
    if hdr_is_layer_1(h) { 384 } else { 1152 >> hdr_is_frame_576(h) as usize }
}

pub fn hdr_frame_bytes(h: [u8; 4], free_format_size: usize) -> usize {
    let mut frame_bytes =
        hdr_frame_samples(h) * hdr_bitrate_kbps(h) as usize * 125 / hdr_sample_rate_hz(h).max(1) as usize;
    if hdr_is_layer_1(h) {
        frame_bytes &= !3; // slot align
    }
    if frame_bytes != 0 { frame_bytes } else { free_format_size }
}

pub fn hdr_padding(h: [u8; 4]) -> usize {
    if hdr_test_padding(h) { if hdr_is_layer_1(h) { 4 } else { 1 } } else { 0 }
}

// ------------------------------------------------------------ Layer III

/// L3_read_side_info: the granules' side info into `gr`; returns
/// main_data_begin, or -1 for a frame that cannot be.
fn l3_read_side_info(bs: &mut Bs, gr: &mut [GrInfo; 4], hdr: [u8; 4]) -> i32 {
    let mut tables: u32;
    let mut scfsi: u32 = 0;
    let main_data_begin: i32;
    let mut part_23_sum: i32 = 0;
    let mut sr_idx = hdr_get_my_sample_rate(hdr) as usize;
    sr_idx -= (sr_idx != 0) as usize;
    let mut gr_count = if hdr_is_mono(hdr) { 1 } else { 2 };

    if hdr_test_mpeg1(hdr) {
        gr_count *= 2;
        main_data_begin = bs.get_bits(9) as i32;
        scfsi = bs.get_bits(7 + gr_count);
    } else {
        main_data_begin = (bs.get_bits(8 + gr_count) >> gr_count) as i32;
    }

    for g in gr.iter_mut().take(gr_count as usize) {
        if hdr_is_mono(hdr) {
            scfsi <<= 4;
        }
        g.part_23_length = bs.get_bits(12) as u16;
        part_23_sum += g.part_23_length as i32;
        g.big_values = bs.get_bits(9) as u16;
        if g.big_values > 288 {
            return -1;
        }
        g.global_gain = bs.get_bits(8) as u8;
        g.scalefac_compress = bs.get_bits(if hdr_test_mpeg1(hdr) { 4 } else { 9 }) as u16;
        g.sfbtab = &SCF_LONG[sr_idx * 23..];
        g.n_long_sfb = 22;
        g.n_short_sfb = 0;
        if bs.get_bits(1) != 0 {
            g.block_type = bs.get_bits(2) as u8;
            if g.block_type == 0 {
                return -1;
            }
            g.mixed_block_flag = bs.get_bits(1) as u8;
            g.region_count[0] = 7;
            g.region_count[1] = 255;
            if g.block_type == SHORT_BLOCK_TYPE {
                scfsi &= 0x0F0F;
                if g.mixed_block_flag == 0 {
                    g.region_count[0] = 8;
                    g.sfbtab = &SCF_SHORT[sr_idx * 40..];
                    g.n_long_sfb = 0;
                    g.n_short_sfb = 39;
                } else {
                    g.sfbtab = &SCF_MIXED[sr_idx * 40..];
                    g.n_long_sfb = if hdr_test_mpeg1(hdr) { 8 } else { 6 };
                    g.n_short_sfb = 30;
                }
            }
            tables = bs.get_bits(10);
            tables <<= 5;
            g.subblock_gain[0] = bs.get_bits(3) as u8;
            g.subblock_gain[1] = bs.get_bits(3) as u8;
            g.subblock_gain[2] = bs.get_bits(3) as u8;
        } else {
            g.block_type = 0;
            g.mixed_block_flag = 0;
            tables = bs.get_bits(15);
            g.region_count[0] = bs.get_bits(4) as u8;
            g.region_count[1] = bs.get_bits(3) as u8;
            g.region_count[2] = 255;
        }
        g.table_select[0] = (tables >> 10) as u8;
        g.table_select[1] = ((tables >> 5) & 31) as u8;
        g.table_select[2] = (tables & 31) as u8;
        g.preflag = if hdr_test_mpeg1(hdr) { bs.get_bits(1) as u8 } else { (g.scalefac_compress >= 500) as u8 };
        g.scalefac_scale = bs.get_bits(1) as u8;
        g.count1_table = bs.get_bits(1) as u8;
        g.scfsi = ((scfsi >> 12) & 15) as u8;
        scfsi <<= 4;
    }

    if part_23_sum + bs.pos > bs.limit + main_data_begin * 8 {
        return -1;
    }

    main_data_begin
}

/// L3_read_scalefactors: `scf` and `ist_pos` from their starts, the band
/// counts from `scf_count` (a row of g_scf_partitions).
fn l3_read_scalefactors(
    scf: &mut [u8],
    ist_pos: &mut [u8],
    scf_size: &[u8; 4],
    scf_count: &[u8],
    bitbuf: &mut Bs,
    mut scfsi: i32,
) {
    let mut at = 0;
    let mut i = 0;
    while i < 4 && scf_count[i] != 0 {
        let cnt = scf_count[i] as usize;
        if scfsi & 8 != 0 {
            scf[at..at + cnt].copy_from_slice(&ist_pos[at..at + cnt]);
        } else {
            let bits = scf_size[i] as i32;
            if bits == 0 {
                scf[at..at + cnt].fill(0);
                ist_pos[at..at + cnt].fill(0);
            } else {
                let max_scf = if scfsi < 0 { (1 << bits) - 1 } else { -1 };
                for k in 0..cnt {
                    let s = bitbuf.get_bits(bits) as i32;
                    ist_pos[at + k] = if s == max_scf { 255 } else { s as u8 };
                    scf[at + k] = s as u8;
                }
            }
        }
        at += cnt;
        i += 1;
        scfsi = scfsi.wrapping_mul(2);
    }
    scf[at] = 0;
    scf[at + 1] = 0;
    scf[at + 2] = 0;
}

/// L3_ldexp_q2: y times 2 to the exp_q2/4.
fn l3_ldexp_q2(mut y: f32, mut exp_q2: i32) -> f32 {
    loop {
        let e = exp_q2.min(30 * 4);
        y *= EXP_FRAC[(e & 3) as usize] * (1i32 << 30).wrapping_shr((e >> 2) as u32) as f32;
        exp_q2 -= e;
        if exp_q2 <= 0 {
            break;
        }
    }
    y
}

/// L3_decode_scalefactors: one channel's scale factors, as gains, into `scf`.
fn l3_decode_scalefactors(hdr: [u8; 4], ist_pos: &mut [u8], bs: &mut Bs, gr: &GrInfo, scf: &mut [f32; 40], ch: usize) {
    let mut scf_partition = 28 * ((gr.n_short_sfb != 0) as usize + (gr.n_long_sfb == 0) as usize);
    let mut scf_size = [0u8; 4];
    let mut iscf = [0u8; 44]; // 40 in C, which the three zeros after the last band can run past
    let scf_shift = gr.scalefac_scale as i32 + 1;
    let mut scfsi = gr.scfsi as i32;

    if hdr_test_mpeg1(hdr) {
        let part = SCFC_DECODE[gr.scalefac_compress as usize & 15];
        scf_size[0] = part >> 2;
        scf_size[1] = scf_size[0];
        scf_size[2] = part & 3;
        scf_size[3] = scf_size[2];
    } else {
        let ist = (hdr_test_i_stereo(hdr) && ch != 0) as usize;
        let mut sfc = (gr.scalefac_compress >> ist) as i32;
        let mut k = ist * 3 * 4;
        // sfc is at most 511 (9 bits), which the three rows of g_mod use up
        // before k passes the table.
        while sfc >= 0 {
            let mut modprod = 1;
            for i in (0..4).rev() {
                scf_size[i] = (sfc / modprod % MOD[k + i] as i32) as u8;
                modprod *= MOD[k + i] as i32;
            }
            sfc -= modprod;
            k += 4;
        }
        scf_partition += k;
        scfsi = -16;
    }
    l3_read_scalefactors(&mut iscf, ist_pos, &scf_size, &SCF_PARTITIONS[scf_partition..], bs, scfsi);

    let (n_long, n_short) = (gr.n_long_sfb as usize, gr.n_short_sfb as usize);
    if n_short != 0 {
        let sh = 3 - scf_shift;
        for i in (0..n_short).step_by(3) {
            iscf[n_long + i] = iscf[n_long + i].wrapping_add(gr.subblock_gain[0] << sh);
            iscf[n_long + i + 1] = iscf[n_long + i + 1].wrapping_add(gr.subblock_gain[1] << sh);
            iscf[n_long + i + 2] = iscf[n_long + i + 2].wrapping_add(gr.subblock_gain[2] << sh);
        }
    } else if gr.preflag != 0 {
        for i in 0..10 {
            iscf[11 + i] = iscf[11 + i].wrapping_add(PREAMP[i]);
        }
    }

    let gain_exp = gr.global_gain as i32 + BITS_DEQUANTIZER_OUT * 4 - 210 - if hdr_is_ms_stereo(hdr) { 2 } else { 0 };
    let gain = l3_ldexp_q2((1 << (MAX_SCFI / 4)) as f32, MAX_SCFI - gain_exp);
    for i in 0..n_long + n_short {
        scf[i] = l3_ldexp_q2(gain, (iscf[i] as i32) << scf_shift);
    }
}

/// L3_pow_43: |x| to the 4/3, from the table and an interpolation.
fn l3_pow_43(mut x: i32) -> f32 {
    let mut mult = 256;
    if x < 129 {
        return POW43[(16 + x) as usize];
    }
    if x < 1024 {
        mult = 16;
        x <<= 3;
    }
    let sign = (2 * x) & 64;
    let frac = ((x & 63) - sign) as f32 / ((x & !63) + sign) as f32;
    POW43[(16 + ((x + sign) >> 6)) as usize] * (1.0 + frac * ((4.0 / 3.0) + frac * (2.0 / 9.0))) * mult as f32
}

/// L3_huffman: one channel's spectrum into `dst` (576 values), from the bits
/// at bs.pos up to `layer3gr_limit`. PEEK_BITS(n) is cache >> (32 - n),
/// FLUSH_BITS(n) is cache <<= n, sh += n, and CHECK_BITS refills the cache
/// from `next` (an index into bs.buf in place of bs_next_ptr).
fn l3_huffman(dst: &mut [f32], bs: &mut Bs, gr_info: &GrInfo, scf: &[f32; 40], layer3gr_limit: i32) {
    let tab = |i: i64| -> i32 { if i < 0 { 0 } else { TABS.get(i as usize).copied().unwrap_or(0) as i32 } };
    let mut one = 0.0f32;
    let mut ireg = 0;
    let mut big_val_cnt = gr_info.big_values as i32;
    let sfb = gr_info.sfbtab;
    let mut sfb_at = 0usize;
    let mut scf_at = 0usize;
    let mut d = 0usize;
    let mut next = (bs.pos / 8) as usize;
    let mut cache: u32 = ((((bs.byte(next) * 256) + bs.byte(next + 1)) * 256 + bs.byte(next + 2)) * 256
        + bs.byte(next + 3))
        << (bs.pos & 7);
    let mut sh: i32 = (bs.pos & 7) - 8;
    next += 4;
    let mut np: i32;
    let sfb_next = |at: &mut usize| -> i32 {
        let v = sfb.get(*at).copied().unwrap_or(0) as i32;
        *at += 1;
        v
    };
    let scf_next = |at: &mut usize| -> f32 {
        let v = scf.get(*at).copied().unwrap_or(0.0);
        *at += 1;
        v
    };
    macro_rules! flush {
        ($n:expr) => {{
            let n = $n as i32;
            cache = cache.wrapping_shl(n as u32);
            sh += n;
        }};
    }
    macro_rules! check {
        () => {
            while sh >= 0 {
                cache |= bs.byte(next).wrapping_shl(sh as u32);
                next += 1;
                sh -= 8;
            }
        };
    }
    // *dst = v, within the channel's 576 whatever the bits say.
    fn put(dst: &mut [f32], i: usize, v: f32) {
        if let Some(x) = dst.get_mut(i) {
            *x = v;
        }
    }

    while big_val_cnt > 0 {
        let tab_num = gr_info.table_select.get(ireg).copied().unwrap_or(0) as usize;
        let mut sfb_cnt = gr_info.region_count.get(ireg).copied().unwrap_or(0) as i32;
        ireg += 1;
        let codebook = TAB_INDEX[tab_num] as i64;
        let linbits = LINBITS[tab_num] as u32;
        loop {
            np = sfb_next(&mut sfb_at) / 2;
            let mut pairs_to_decode = big_val_cnt.min(np);
            one = scf_next(&mut scf_at);
            loop {
                let mut w: u32 = 5;
                let mut leaf = tab(codebook + cache.wrapping_shr(32 - w) as i64);
                while leaf < 0 {
                    flush!(w);
                    w = (leaf & 7) as u32;
                    leaf = tab(codebook + cache.wrapping_shr(32 - w) as i64 - (leaf >> 3) as i64);
                }
                flush!(leaf >> 8);

                for _ in 0..2 {
                    let mut lsb = leaf & 0x0F;
                    if linbits != 0 && lsb == 15 {
                        lsb += cache.wrapping_shr(32 - linbits) as i32;
                        flush!(linbits);
                        check!();
                        put(dst, d, one * l3_pow_43(lsb) * if (cache as i32) < 0 { -1.0 } else { 1.0 });
                    } else {
                        put(dst, d, POW43[(16 + lsb - 16 * (cache >> 31) as i32) as usize] * one);
                    }
                    flush!((lsb != 0) as i32);
                    d += 1;
                    leaf >>= 4;
                }
                check!();
                // `!= 0` in the C: the same for any pair count a table allows,
                // and no four-billion-pair loop if one did not.
                pairs_to_decode -= 1;
                if pairs_to_decode <= 0 {
                    break;
                }
            }
            big_val_cnt -= np;
            if big_val_cnt <= 0 {
                break;
            }
            sfb_cnt -= 1;
            if sfb_cnt < 0 {
                break;
            }
        }
    }

    np = 1 - big_val_cnt;
    loop {
        let codebook_count1: &[u8] = if gr_info.count1_table != 0 { &TAB33 } else { &TAB32 };
        let c1 = |i: usize| codebook_count1.get(i).copied().unwrap_or(0) as i32;
        let mut leaf = c1((cache >> (32 - 4)) as usize);
        if leaf & 8 == 0 {
            leaf = c1(((leaf >> 3) as u32).wrapping_add((cache << 4).wrapping_shr(32 - (leaf & 3) as u32)) as usize);
        }
        flush!(leaf & 7);
        if (next as i64 * 8 - 24 + sh as i64) > layer3gr_limit as i64 {
            break;
        }
        macro_rules! sign_bit {
            ($k:expr) => {
                if leaf & (128 >> $k) != 0 {
                    put(dst, d + $k, if (cache as i32) < 0 { -one } else { one });
                    flush!(1);
                }
            };
        }
        np -= 1;
        if np == 0 {
            np = sfb_next(&mut sfb_at) / 2;
            if np == 0 {
                break;
            }
            one = scf_next(&mut scf_at);
        }
        sign_bit!(0);
        sign_bit!(1);
        np -= 1;
        if np == 0 {
            np = sfb_next(&mut sfb_at) / 2;
            if np == 0 {
                break;
            }
            one = scf_next(&mut scf_at);
        }
        sign_bit!(2);
        sign_bit!(3);
        check!();
        d += 4;
    }

    bs.pos = layer3gr_limit;
}

/// L3_midside_stereo: left at buf[left..], right 576 after it.
fn l3_midside_stereo(buf: &mut [f32], left: usize, n: usize) {
    for i in left..left + n {
        let a = buf[i];
        let b = buf[i + 576];
        buf[i] = a + b;
        buf[i + 576] = a - b;
    }
}

fn l3_intensity_stereo_band(buf: &mut [f32], left: usize, n: usize, kl: f32, kr: f32) {
    for i in left..left + n {
        buf[i + 576] = buf[i] * kr;
        buf[i] *= kl;
    }
}

/// L3_stereo_top_band: the last band with sound in the right channel, for
/// each of the three short windows (or the one long one).
fn l3_stereo_top_band(right: &[f32], sfb: &[u8], nbands: usize, max_band: &mut [i32; 3]) {
    *max_band = [-1; 3];
    let mut r = 0;
    for i in 0..nbands {
        let mut k = 0;
        while k < sfb[i] as usize {
            if right[r + k] != 0.0 || right[r + k + 1] != 0.0 {
                max_band[i % 3] = i as i32;
                break;
            }
            k += 2;
        }
        r += sfb[i] as usize;
    }
}

fn l3_stereo_process(buf: &mut [f32], ist_pos: &[u8], sfb: &[u8], hdr: [u8; 4], max_band: &[i32; 3], mpeg2_sh: u32) {
    let max_pos: u32 = if hdr_test_mpeg1(hdr) { 7 } else { 64 };
    let mut left = 0;
    let mut i = 0;
    while sfb[i] != 0 {
        let ipos = ist_pos[i] as u32;
        if i as i32 > max_band[i % 3] && ipos < max_pos {
            let s = if hdr_test_ms_stereo(hdr) { std::f32::consts::SQRT_2 } else { 1.0 };
            let (kl, kr);
            if hdr_test_mpeg1(hdr) {
                kl = PAN[2 * ipos as usize];
                kr = PAN[2 * ipos as usize + 1];
            } else {
                let k = l3_ldexp_q2(1.0, (((ipos + 1) >> 1) << mpeg2_sh) as i32);
                (kl, kr) = if ipos & 1 != 0 { (k, 1.0) } else { (1.0, k) };
            }
            l3_intensity_stereo_band(buf, left, sfb[i] as usize, kl * s, kr * s);
        } else if hdr_test_ms_stereo(hdr) {
            l3_midside_stereo(buf, left, sfb[i] as usize);
        }
        left += sfb[i] as usize;
        i += 1;
    }
}

/// L3_intensity_stereo: `gr` is this granule's two channels; `ist_pos` the
/// right channel's.
fn l3_intensity_stereo(buf: &mut [f32], ist_pos: &mut [u8], gr: &[GrInfo], hdr: [u8; 4]) {
    let mut max_band = [0i32; 3];
    let n_sfb = gr[0].n_long_sfb as usize + gr[0].n_short_sfb as usize;
    let max_blocks = if gr[0].n_short_sfb != 0 { 3 } else { 1 };

    l3_stereo_top_band(&buf[576..], gr[0].sfbtab, n_sfb, &mut max_band);
    if gr[0].n_long_sfb != 0 {
        let m = max_band[0].max(max_band[1]).max(max_band[2]);
        max_band = [m; 3];
    }
    for i in 0..max_blocks {
        let default_pos = if hdr_test_mpeg1(hdr) { 3 } else { 0 };
        let itop = n_sfb - max_blocks + i;
        let prev = itop - max_blocks;
        ist_pos[itop] = if max_band[i] >= prev as i32 { default_pos } else { ist_pos[prev] };
    }
    l3_stereo_process(buf, ist_pos, gr[0].sfbtab, hdr, &max_band, (gr[1].scalefac_compress & 1) as u32);
}

/// L3_reorder: short blocks' values from window order into band order.
fn l3_reorder(grbuf: &mut [f32], scratch: &mut [f32], sfb: &[u8]) {
    let mut src = 0;
    let mut dst = 0;
    let mut b = 0;
    loop {
        let len = sfb[b] as usize;
        if len == 0 {
            break;
        }
        for _ in 0..len {
            scratch[dst] = grbuf[src];
            scratch[dst + 1] = grbuf[src + len];
            scratch[dst + 2] = grbuf[src + 2 * len];
            dst += 3;
            src += 1;
        }
        src += 2 * len;
        b += 3;
    }
    grbuf[..dst].copy_from_slice(&scratch[..dst]);
}

fn l3_antialias(grbuf: &mut [f32], nbands: i32) {
    let mut g = 0;
    for _ in 0..nbands.max(0) {
        for i in 0..8 {
            let u = grbuf[g + 18 + i];
            let d = grbuf[g + 17 - i];
            grbuf[g + 18 + i] = u * AA[i] - d * AA[8 + i];
            grbuf[g + 17 - i] = u * AA[8 + i] + d * AA[i];
        }
        g += 18;
    }
}

fn l3_dct3_9(y: &mut [f32; 9]) {
    let (mut s0, mut s1, mut s2, mut s3, mut s4, mut s5, mut s6, mut s7, mut s8);
    let (mut t0, mut t2, mut t4);

    s0 = y[0];
    s2 = y[2];
    s4 = y[4];
    s6 = y[6];
    s8 = y[8];
    t0 = s0 + s6 * 0.5;
    s0 -= s6;
    t4 = (s4 + s2) * 0.93969262;
    t2 = (s8 + s2) * 0.76604444;
    s6 = (s4 - s8) * 0.17364818;
    s4 += s8 - s2;

    s2 = s0 - s4 * 0.5;
    y[4] = s4 + s0;
    s8 = t0 - t2 + s6;
    s0 = t0 - t4 + t2;
    s4 = t0 + t4 - s6;

    s1 = y[1];
    s3 = y[3];
    s5 = y[5];
    s7 = y[7];

    s3 *= 0.86602540;
    t0 = (s5 + s1) * 0.98480775;
    t4 = (s5 - s7) * 0.34202014;
    t2 = (s1 + s7) * 0.64278761;
    s1 = (s1 - s5 - s7) * 0.86602540;

    s5 = t0 - s3 - t2;
    s7 = t4 - s3 - t0;
    s3 = t4 + s3 - t2;

    y[0] = s4 - s7;
    y[1] = s2 + s1;
    y[2] = s0 - s3;
    y[3] = s8 + s5;
    y[5] = s8 - s5;
    y[6] = s0 + s3;
    y[7] = s2 - s1;
    y[8] = s4 + s7;
}

fn l3_imdct36(grbuf: &mut [f32], overlap: &mut [f32], window: &[f32], nbands: usize) {
    let mut co = [0f32; 9];
    let mut si = [0f32; 9];
    for j in 0..nbands {
        let g = &mut grbuf[j * 18..j * 18 + 18];
        let o = &mut overlap[j * 9..j * 9 + 9];
        co[0] = -g[0];
        si[0] = g[17];
        for i in 0..4 {
            si[8 - 2 * i] = g[4 * i + 1] - g[4 * i + 2];
            co[1 + 2 * i] = g[4 * i + 1] + g[4 * i + 2];
            si[7 - 2 * i] = g[4 * i + 4] - g[4 * i + 3];
            co[2 + 2 * i] = -(g[4 * i + 3] + g[4 * i + 4]);
        }
        l3_dct3_9(&mut co);
        l3_dct3_9(&mut si);

        si[1] = -si[1];
        si[3] = -si[3];
        si[5] = -si[5];
        si[7] = -si[7];

        for i in 0..9 {
            let ovl = o[i];
            let sum = co[i] * TWID9[9 + i] + si[i] * TWID9[i];
            o[i] = co[i] * TWID9[i] - si[i] * TWID9[9 + i];
            g[i] = ovl * window[i] - sum * window[9 + i];
            g[17 - i] = ovl * window[9 + i] + sum * window[i];
        }
    }
}

fn l3_idct3(x0: f32, x1: f32, x2: f32) -> [f32; 3] {
    let m1 = x1 * 0.86602540;
    let a1 = x0 - x2 * 0.5;
    [a1 + m1, x0 + x2, a1 - m1]
}

/// L3_imdct12: `x` read every third value (x[0], x[3], .. x[15]).
fn l3_imdct12(x: &[f32], dst: &mut [f32], overlap: &mut [f32]) {
    let co = l3_idct3(-x[0], x[6] + x[3], x[12] + x[9]);
    let mut si = l3_idct3(x[15], x[12] - x[9], x[6] - x[3]);
    si[1] = -si[1];

    for i in 0..3 {
        let ovl = overlap[i];
        let sum = co[i] * TWID3[3 + i] + si[i] * TWID3[i];
        overlap[i] = co[i] * TWID3[i] - si[i] * TWID3[3 + i];
        dst[i] = ovl * TWID3[2 - i] - sum * TWID3[5 - i];
        dst[5 - i] = ovl * TWID3[5 - i] + sum * TWID3[2 - i];
    }
}

fn l3_imdct_short(grbuf: &mut [f32], overlap: &mut [f32], nbands: usize) {
    let mut tmp = [0f32; 18];
    for b in 0..nbands {
        let g = &mut grbuf[b * 18..b * 18 + 18];
        let o = &mut overlap[b * 9..b * 9 + 9];
        tmp.copy_from_slice(g);
        g[..6].copy_from_slice(&o[..6]);
        let (o_first, o_rest) = o.split_at_mut(6);
        l3_imdct12(&tmp, &mut g[6..12], o_rest);
        l3_imdct12(&tmp[1..], &mut g[12..18], o_rest);
        l3_imdct12(&tmp[2..], o_first, o_rest);
    }
}

fn l3_change_sign(grbuf: &mut [f32]) {
    let mut g = 18;
    for _ in (0..32).step_by(2) {
        for i in (1..18).step_by(2) {
            grbuf[g + i] = -grbuf[g + i];
        }
        g += 36;
    }
}

fn l3_imdct_gr(grbuf: &mut [f32], overlap: &mut [f32], block_type: u8, n_long_bands: usize) {
    if n_long_bands != 0 {
        l3_imdct36(grbuf, overlap, &MDCT_WINDOW, n_long_bands);
    }
    let (g, o) = (&mut grbuf[18 * n_long_bands..], &mut overlap[9 * n_long_bands..]);
    if block_type == SHORT_BLOCK_TYPE {
        l3_imdct_short(g, o, 32 - n_long_bands);
    } else {
        l3_imdct36(g, o, &MDCT_WINDOW[if block_type == STOP_BLOCK_TYPE { 18 } else { 0 }..], 32 - n_long_bands);
    }
}

// ------------------------------------------------------------ synthesis

/// mp3d_DCT_II: the 32-band DCT on `n` of a channel's 18 rows.
fn mp3d_dct_ii(grbuf: &mut [f32], n: usize) {
    let mut t = [0f32; 4 * 8];
    for k in 0..n {
        let mut y = k;
        for i in 0..8 {
            let x0 = grbuf[y + i * 18];
            let x1 = grbuf[y + (15 - i) * 18];
            let x2 = grbuf[y + (16 + i) * 18];
            let x3 = grbuf[y + (31 - i) * 18];
            let t0 = x0 + x3;
            let t1 = x1 + x2;
            let t2 = (x1 - x2) * SEC[3 * i];
            let t3 = (x0 - x3) * SEC[3 * i + 1];
            t[i] = t0 + t1;
            t[i + 8] = (t0 - t1) * SEC[3 * i + 2];
            t[i + 16] = t3 + t2;
            t[i + 24] = (t3 - t2) * SEC[3 * i + 2];
        }
        for x in t.as_chunks_mut::<8>().0 {
            let (mut x0, mut x1, mut x2, mut x3, mut x4, mut x5, mut x6, mut x7) =
                (x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]);
            let mut xt = x0 - x7;
            x0 += x7;
            x7 = x1 - x6;
            x1 += x6;
            x6 = x2 - x5;
            x2 += x5;
            x5 = x3 - x4;
            x3 += x4;
            x4 = x0 - x3;
            x0 += x3;
            x3 = x1 - x2;
            x1 += x2;
            x[0] = x0 + x1;
            x[4] = (x0 - x1) * 0.70710677;
            x5 += x6;
            x6 = (x6 + x7) * 0.70710677;
            x7 += xt;
            x3 = (x3 + x4) * 0.70710677;
            x5 -= x7 * 0.198912367; // rotate by PI/8
            x7 += x5 * 0.382683432;
            x5 -= x7 * 0.198912367;
            x0 = xt - x6;
            xt += x6;
            x[1] = (xt + x7) * 0.50979561;
            x[2] = (x4 + x3) * 0.54119611;
            x[3] = (x0 - x5) * 0.60134488;
            x[5] = (x0 + x5) * 0.89997619;
            x[6] = (x4 - x3) * 1.30656302;
            x[7] = (xt - x7) * 2.56291556;
        }
        for i in 0..7 {
            grbuf[y] = t[i];
            grbuf[y + 18] = t[2 * 8 + i] + t[3 * 8 + i] + t[3 * 8 + i + 1];
            grbuf[y + 2 * 18] = t[8 + i] + t[8 + i + 1];
            grbuf[y + 3 * 18] = t[2 * 8 + i + 1] + t[3 * 8 + i] + t[3 * 8 + i + 1];
            y += 4 * 18;
        }
        grbuf[y] = t[7];
        grbuf[y + 18] = t[2 * 8 + 7] + t[3 * 8 + 7];
        grbuf[y + 2 * 18] = t[8 + 7];
        grbuf[y + 3 * 18] = t[3 * 8 + 7];
    }
}

fn mp3d_scale_pcm(sample: f32) -> f32 {
    sample * (1.0 / 32768.0)
}

/// mp3d_synth_pair: pcm[p] and pcm[p + 16 * nch] from the history at lins[z..].
fn mp3d_synth_pair(pcm: &mut [f32], p: usize, nch: usize, lins: &[f32], z: usize) {
    let zz = |i: usize| lins[z + i];
    let mut a = (zz(14 * 64) - zz(0)) * 29.0;
    a += (zz(64) + zz(13 * 64)) * 213.0;
    a += (zz(12 * 64) - zz(2 * 64)) * 459.0;
    a += (zz(3 * 64) + zz(11 * 64)) * 2037.0;
    a += (zz(10 * 64) - zz(4 * 64)) * 5153.0;
    a += (zz(5 * 64) + zz(9 * 64)) * 6574.0;
    a += (zz(8 * 64) - zz(6 * 64)) * 37489.0;
    a += zz(7 * 64) * 75038.0;
    pcm[p] = mp3d_scale_pcm(a);

    let zz = |i: usize| lins[z + 2 + i];
    let mut a = zz(14 * 64) * 104.0;
    a += zz(12 * 64) * 1567.0;
    a += zz(10 * 64) * 9727.0;
    a += zz(8 * 64) * 64019.0;
    a += zz(6 * 64) * -9975.0;
    a += zz(4 * 64) * -45.0;
    a += zz(2 * 64) * 146.0;
    a += zz(0) * -5.0;
    pcm[p + 16 * nch] = mp3d_scale_pcm(a);
}

/// mp3d_synth: two rows of subband values (grbuf[xl..], the right channel
/// 576 on) through the synthesis window into pcm[dstl..], interleaved;
/// `lins` is the history from lins[l..].
fn mp3d_synth(grbuf: &[f32], xl: usize, pcm: &mut [f32], dstl: usize, nch: usize, lins: &mut [f32], l: usize) {
    let xr = xl + 576 * (nch - 1);
    let dstr = dstl + (nch - 1);
    let zlin = l + 15 * 64;
    let mut w = 0;
    let mut a = [0f32; 4];
    let mut b = [0f32; 4];

    lins[zlin + 4 * 15] = grbuf[xl + 18 * 16];
    lins[zlin + 4 * 15 + 1] = grbuf[xr + 18 * 16];
    lins[zlin + 4 * 15 + 2] = grbuf[xl];
    lins[zlin + 4 * 15 + 3] = grbuf[xr];

    lins[zlin + 4 * 31] = grbuf[xl + 1 + 18 * 16];
    lins[zlin + 4 * 31 + 1] = grbuf[xr + 1 + 18 * 16];
    lins[zlin + 4 * 31 + 2] = grbuf[xl + 1];
    lins[zlin + 4 * 31 + 3] = grbuf[xr + 1];

    mp3d_synth_pair(pcm, dstr, nch, lins, l + 4 * 15 + 1);
    mp3d_synth_pair(pcm, dstr + 32 * nch, nch, lins, l + 4 * 15 + 64 + 1);
    mp3d_synth_pair(pcm, dstl, nch, lins, l + 4 * 15);
    mp3d_synth_pair(pcm, dstl + 32 * nch, nch, lins, l + 4 * 15 + 64);

    for i in (0..15).rev() {
        lins[zlin + 4 * i] = grbuf[xl + 18 * (31 - i)];
        lins[zlin + 4 * i + 1] = grbuf[xr + 18 * (31 - i)];
        lins[zlin + 4 * i + 2] = grbuf[xl + 1 + 18 * (31 - i)];
        lins[zlin + 4 * i + 3] = grbuf[xr + 1 + 18 * (31 - i)];
        lins[zlin + 4 * (i + 16)] = grbuf[xl + 1 + 18 * (1 + i)];
        lins[zlin + 4 * (i + 16) + 1] = grbuf[xr + 1 + 18 * (1 + i)];
        // zlin[4 * (i - 16) + 2] in the C: 64 before zlin + 4i.
        lins[zlin + 4 * i + 2 - 64] = grbuf[xl + 18 * (1 + i)];
        lins[zlin + 4 * i + 3 - 64] = grbuf[xr + 18 * (1 + i)];

        // S0(0) S2(1) S1(2) S2(3) S1(4) S2(5) S1(6) S2(7)
        for k in 0..8 {
            let w0 = WIN[w];
            let w1 = WIN[w + 1];
            w += 2;
            let vz = zlin + 4 * i - k * 64;
            let vy = zlin + 4 * i - (15 - k) * 64;
            for j in 0..4 {
                let (z, y) = (lins[vz + j], lins[vy + j]);
                if k == 0 {
                    b[j] = z * w1 + y * w0;
                    a[j] = z * w0 - y * w1;
                } else if k & 1 != 0 {
                    b[j] += z * w1 + y * w0;
                    a[j] += y * w1 - z * w0;
                } else {
                    b[j] += z * w1 + y * w0;
                    a[j] += z * w0 - y * w1;
                }
            }
        }

        pcm[dstr + (15 - i) * nch] = mp3d_scale_pcm(a[1]);
        pcm[dstr + (17 + i) * nch] = mp3d_scale_pcm(b[1]);
        pcm[dstl + (15 - i) * nch] = mp3d_scale_pcm(a[0]);
        pcm[dstl + (17 + i) * nch] = mp3d_scale_pcm(b[0]);
        pcm[dstr + (47 - i) * nch] = mp3d_scale_pcm(a[3]);
        pcm[dstr + (49 + i) * nch] = mp3d_scale_pcm(b[3]);
        pcm[dstl + (47 - i) * nch] = mp3d_scale_pcm(a[2]);
        pcm[dstl + (49 + i) * nch] = mp3d_scale_pcm(b[2]);
    }
}

fn mp3d_synth_granule(
    qmf_state: &mut [f32; 960],
    grbuf: &mut [f32; 1152],
    nbands: usize,
    nch: usize,
    pcm: &mut [f32],
    lins: &mut [f32],
) {
    for i in 0..nch {
        mp3d_dct_ii(&mut grbuf[576 * i..], nbands);
    }

    lins[..15 * 64].copy_from_slice(qmf_state);

    for i in (0..nbands).step_by(2) {
        mp3d_synth(grbuf, i, pcm, 32 * nch * i, nch, lins, i * 64);
    }
    if nch == 1 {
        for i in (0..15 * 64).step_by(2) {
            qmf_state[i] = lins[nbands * 64 + i];
        }
    } else {
        qmf_state.copy_from_slice(&lins[nbands * 64..nbands * 64 + 15 * 64]);
    }
}

// ------------------------------------------------------------ framing

fn mp3d_match_frame(hdr: &[u8], frame_bytes: usize) -> bool {
    let mut i = 0usize;
    for nmatch in 0..MAX_FRAME_SYNC_MATCHES {
        let h = h4(hdr, i);
        i += hdr_frame_bytes(h, frame_bytes) + hdr_padding(h);
        if i + HDR_SIZE > hdr.len() {
            return nmatch > 0;
        }
        if !hdr_compare(h4(hdr, 0), h4(hdr, i)) {
            return false;
        }
    }
    true
}

/// mp3d_find_frame: the first frame in `mp3` that the frames after it agree
/// with. Returns its offset and its length with padding, or (mp3.len(), 0)
/// for none.
pub fn mp3d_find_frame(mp3: &[u8], free_format_bytes: &mut usize) -> (usize, usize) {
    let n = mp3.len();
    let mut i = 0;
    while i + HDR_SIZE < n {
        let h = h4(mp3, i);
        if hdr_valid(h) {
            let mut frame_bytes = hdr_frame_bytes(h, *free_format_bytes);
            let mut frame_and_padding = frame_bytes + hdr_padding(h);

            let mut k = HDR_SIZE;
            while frame_bytes == 0 && k < MAX_FREE_FORMAT_FRAME_SIZE && i + 2 * k + HDR_SIZE < n {
                if hdr_compare(h, h4(mp3, i + k)) {
                    let fb = k - hdr_padding(h);
                    let nextfb = fb + hdr_padding(h4(mp3, i + k));
                    if !(i + k + nextfb + HDR_SIZE > n || !hdr_compare(h, h4(mp3, i + k + nextfb))) {
                        frame_and_padding = k;
                        frame_bytes = fb;
                        *free_format_bytes = fb;
                    }
                }
                k += 1;
            }
            if (frame_bytes != 0 && i + frame_and_padding <= n && mp3d_match_frame(&mp3[i..], frame_bytes))
                || (i == 0 && frame_and_padding == n)
            {
                return (i, frame_and_padding);
            }
            *free_format_bytes = 0;
        }
        i += 1;
    }
    (n, 0)
}

impl Mp3Decoder {
    pub fn new() -> Mp3Decoder {
        Mp3Decoder {
            mdct_overlap: [0.0; 2 * 9 * 32],
            qmf_state: [0.0; 15 * 2 * 32],
            reserv: 0,
            free_format_bytes: 0,
            header: [0; HDR_SIZE],
            reserv_buf: [0; MAX_BIT_RESERVOIR_BYTES],
            bs_pos: 0,
            bs_limit: 0,
            maindata: [0; MAIN_DATA_SIZE],
            gr_info: [GrInfo::default(); 4],
            grbuf: [0.0; 2 * 576],
            scf: [0.0; 40],
            syn: [0.0; (18 + 15) * 2 * 32],
            ist_pos: [0; 2 * 39 + 8],
        }
    }

    /// mp3dec_init, and the memset(dec, 0) a lost sync does: the next frame
    /// starts from nothing.
    pub fn reset(&mut self) {
        self.mdct_overlap.fill(0.0);
        self.qmf_state.fill(0.0);
        self.reserv = 0;
        self.free_format_bytes = 0;
        self.header = [0; HDR_SIZE];
        self.reserv_buf.fill(0);
    }

    fn l3_save_reservoir(&mut self) {
        let mut pos = ((self.bs_pos as u32).wrapping_add(7) / 8) as i32;
        let mut remains = (self.bs_limit as u32 / 8) as i32 - pos;
        if remains > MAX_BIT_RESERVOIR_BYTES as i32 {
            pos += remains - MAX_BIT_RESERVOIR_BYTES as i32;
            remains = MAX_BIT_RESERVOIR_BYTES as i32;
        }
        if remains > 0 {
            let (pos, remains) = (pos as usize, remains as usize);
            self.reserv_buf[..remains].copy_from_slice(&self.maindata[pos..pos + remains]);
        }
        // Never below zero: the bits read stop at the limit (Mp3.cs would
        // hand a negative count to its next copy and throw).
        self.reserv = remains.max(0);
    }

    /// L3_restore_reservoir: main data begins `main_data_begin` bytes back in
    /// the reservoir and goes on with this frame's bytes after its side info.
    fn l3_restore_reservoir(&mut self, bs: &Bs, main_data_begin: i32) -> bool {
        let frame_bytes = ((bs.limit - bs.pos) / 8).max(0) as usize;
        let bytes_have = self.reserv.min(main_data_begin).max(0) as usize;
        let from = (self.reserv - main_data_begin).max(0) as usize;
        self.maindata[..bytes_have].copy_from_slice(&self.reserv_buf[from..from + bytes_have]);
        let start = (bs.pos / 8) as usize;
        let frame_bytes = frame_bytes.min(bs.buf.len().saturating_sub(start)).min(MAIN_DATA_SIZE - bytes_have);
        self.maindata[bytes_have..bytes_have + frame_bytes].copy_from_slice(&bs.buf[start..start + frame_bytes]);
        self.bs_pos = 0;
        self.bs_limit = ((bytes_have + frame_bytes) * 8) as i32;
        self.reserv >= main_data_begin
    }

    /// L3_decode: granule `gr0` (its first channel's index in gr_info).
    fn l3_decode(&mut self, gr0: usize, nch: usize) {
        let hdr = self.header;
        {
            let mut bs = Bs { buf: &self.maindata, pos: self.bs_pos, limit: self.bs_limit };
            for ch in 0..nch {
                let gr = self.gr_info[gr0 + ch];
                let layer3gr_limit = bs.pos + gr.part_23_length as i32;
                l3_decode_scalefactors(hdr, &mut self.ist_pos[39 * ch..], &mut bs, &gr, &mut self.scf, ch);
                l3_huffman(&mut self.grbuf[576 * ch..576 * ch + 576], &mut bs, &gr, &self.scf, layer3gr_limit);
            }
            self.bs_pos = bs.pos;
        }

        if hdr_test_i_stereo(hdr) {
            // gr_info[gr0 + 1] even for a mono frame that says intensity
            // stereo (damaged): within the four, as in the C.
            l3_intensity_stereo(&mut self.grbuf, &mut self.ist_pos[39..], &self.gr_info[gr0..gr0 + 2], hdr);
        } else if hdr_is_ms_stereo(hdr) {
            l3_midside_stereo(&mut self.grbuf, 0, 576);
        }

        for ch in 0..nch {
            let gr = self.gr_info[gr0 + ch];
            let mut aa_bands = 31;
            let n_long_bands =
                (if gr.mixed_block_flag != 0 { 2 } else { 0 }) << (hdr_get_my_sample_rate(hdr) == 2) as usize;
            let g = &mut self.grbuf[576 * ch..576 * ch + 576];

            if gr.n_short_sfb != 0 {
                aa_bands = n_long_bands as i32 - 1;
                l3_reorder(&mut g[n_long_bands * 18..], &mut self.syn, &gr.sfbtab[gr.n_long_sfb as usize..]);
            }

            l3_antialias(g, aa_bands);
            l3_imdct_gr(g, &mut self.mdct_overlap[9 * 32 * ch..9 * 32 * (ch + 1)], gr.block_type, n_long_bands);
            l3_change_sign(g);
        }
    }

    /// mp3dec_decode_frame: the next frame from `mp3`, into `pcm` (room for
    /// MAX_SAMPLES_PER_FRAME floats, interleaved), or with `pcm` None only
    /// the frame found and described. Returns the samples per channel
    /// decoded: 0 when no frame was found (info.frame_bytes says how much to
    /// skip), or when one was but could not be decoded (its reservoir is from
    /// before a seek, or it is not Layer III).
    pub fn decode_frame(&mut self, mp3: &[u8], pcm: Option<&mut [f32]>) -> (usize, FrameInfo) {
        let mut info = FrameInfo::default();
        let n = mp3.len();
        let mut i = 0;
        let mut frame_size = 0;

        if n > 4 && self.header[0] == 0xff && hdr_compare(self.header, h4(mp3, 0)) {
            frame_size = hdr_frame_bytes(h4(mp3, 0), self.free_format_bytes) + hdr_padding(h4(mp3, 0));
            if frame_size != n && (frame_size + HDR_SIZE > n || !hdr_compare(h4(mp3, 0), h4(mp3, frame_size))) {
                frame_size = 0;
            }
        }
        if frame_size == 0 {
            self.reset();
            (i, frame_size) = mp3d_find_frame(mp3, &mut self.free_format_bytes);
            if frame_size == 0 || i + frame_size > n {
                info.frame_bytes = i;
                return (0, info);
            }
        }

        let hdr = h4(mp3, i);
        self.header = hdr;
        info.frame_bytes = i + frame_size;
        info.frame_offset = i;
        info.channels = if hdr_is_mono(hdr) { 1 } else { 2 };
        info.hz = hdr_sample_rate_hz(hdr);
        info.layer = 4 - hdr_get_layer(hdr);
        info.bitrate_kbps = hdr_bitrate_kbps(hdr);

        let Some(pcm) = pcm else {
            return (hdr_frame_samples(hdr), info);
        };
        if pcm.len() < MAX_SAMPLES_PER_FRAME {
            return (0, info);
        }

        // A frame shorter than its header (a free-format guess) has no bits.
        let body = mp3.get(i + HDR_SIZE..i + frame_size).unwrap_or(&[]);
        let mut bs_frame = Bs::new(body, body.len());
        if hdr_is_crc(hdr) {
            bs_frame.get_bits(16);
        }

        if info.layer != 3 {
            return (0, info); // Layers I and II are not ported
        }

        let main_data_begin = l3_read_side_info(&mut bs_frame, &mut self.gr_info, hdr);
        if main_data_begin < 0 || bs_frame.pos > bs_frame.limit {
            self.reset();
            return (0, info);
        }
        let success = self.l3_restore_reservoir(&bs_frame, main_data_begin);
        if success {
            let granules = if hdr_test_mpeg1(hdr) { 2 } else { 1 };
            let ch = info.channels;
            for igr in 0..granules {
                self.grbuf.fill(0.0);
                self.l3_decode(igr * ch, ch);
                mp3d_synth_granule(
                    &mut self.qmf_state,
                    &mut self.grbuf,
                    18,
                    ch,
                    &mut pcm[576 * ch * igr..],
                    &mut self.syn,
                );
            }
        }
        self.l3_save_reservoir();
        (if success { hdr_frame_samples(self.header) } else { 0 }, info)
    }
}

// ------------------------------------------------------------ the track
//
// Mp3Track.cs: one MP3 open for playing, held whole in memory (as FlacTrack
// is; Hymie's waits for bytes arriving, Prepare and Interrupt have no part
// here, and a seek is done at once).
//
// GAPLESS, AS FFMPEG DOES IT (libavformat/mp3dec.c, n9.0.2, which is what mpv
// plays MP3 through), so a record plays the same on either engine. An encoder
// pads both ends of the music, and LAME writes how much in the Info tag, a
// frame of its own before the first audio frame: `start_pad` at the front and
// `end_pad` at the end, read only when the tag says LAME, Lavf or Lavc. The
// first start_pad + 529 decoded samples are dropped (529 is the decoder's own
// delay), and the track is then frames x samples-per-frame - start_pad -
// end_pad samples long. No tag, no trimming, as there. The Info frame itself
// is not audio and is not decoded.
//
// SEEKS ARE EXACT. The frame that holds the sample is found by its position
// in an index of frame starts (headers only), and decoding starts ten frames
// before it with nothing kept from before: the bit reservoir refills from
// those frames' bytes and the filters settle, so from the frame asked for
// on, the samples are the ones a straight decode gives.
//
// REPLAYGAIN as FFmpeg keeps it: the ID3 TXXX tags if they have a gain, else
// the LAME tag's (only when its CRC is right).

/// Frames decoded and dropped before a seek's.
const PREROLL: usize = 10;

/// The Xing/Info tag's facts (Mp3Track.cs's Info).
struct Info {
    skip: bool,
    lame: bool,
    frames: i64,
    start_pad: i64,
    end_pad: i64,
    gain: [i32; 4],
}

/// One MP3 file open for playing.
pub struct Mp3Track {
    data: Vec<u8>,
    rate: u32,
    channels: u16,
    gain: ReplayGain,
    /// The encoder's padding, as the LAME tag says (0, 0 without one): kept
    /// as Hymie keeps them, read by the tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub start_pad: i64,
    #[cfg_attr(not(test), allow(dead_code))]
    pub end_pad: i64,
    /// Where the audio ends (before ID3v1 or APE).
    end: usize,
    /// Samples per frame.
    spf: i64,
    /// Stream samples dropped at the start.
    skip: i64,
    /// Samples played, when the Info tag says (-1 when not), or else an
    /// estimate from the bitrate.
    total: i64,
    estimate: i64,

    decoder: Box<Mp3Decoder>,
    pcm: Vec<f32>,
    /// Decoded in pcm, and handed out; the stream sample pcm starts at.
    pcm_frames: usize,
    pcm_offset: usize,
    pcm_at: i64,
    /// The next byte to decode, the stream sample its frame starts at.
    at: usize,
    stream_at: i64,
    /// The first stream sample to hand out (the skip, or a seek's).
    from: i64,
    /// Where frame n starts, as far as known.
    frames: Vec<usize>,
    scan_free_format: usize,
}

impl Mp3Track {
    pub fn new(data: Vec<u8>) -> Result<Mp3Track, Error> {
        // The ID3v2 tags in front, however many.
        let mut tags = Tags::default();
        let at = tags::read_id3(&data, 0, &mut tags).min(data.len());

        // The first frame, which may be the Info tag's.
        let mut free = 0;
        let (off, first_bytes) = mp3d_find_frame(&data[at..], &mut free);
        if first_bytes == 0 {
            return Err(damaged("No MP3 frames."));
        }
        let first = at + off;
        let h = h4(&data, first);
        if hdr_get_layer(h) != 1 {
            return Err(Error::Unsupported("MPEG audio Layer I or II, not Layer III.".into()));
        }
        let rate = hdr_sample_rate_hz(h);
        let channels = if hdr_is_mono(h) { 1 } else { 2 };
        let spf: i64 = if hdr_test_mpeg1(h) { 1152 } else { 576 };
        let mut estimate = ((data.len() - first) as f64 / first_bytes.max(1) as f64 * spf as f64) as i64;

        let info = info_tag(&data, first, first_bytes, h);
        let start = if info.skip { first + first_bytes } else { first };
        let (mut skip, mut total) = (0, -1);
        if info.lame {
            skip = info.start_pad + 528 + 1;
            if info.frames > 0 {
                total = (info.frames * spf - info.start_pad - info.end_pad).max(0);
            }
        }
        if info.frames > 0 {
            estimate = info.frames * spf - info.start_pad - info.end_pad;
        }

        let raw = crate::replaygain::raw(&tags.values);
        let any = raw[0] != i32::MIN || raw[2] != i32::MIN;
        let g = if any { raw } else { info.gain };
        let gain = ReplayGain::from_raw(g[0], g[1], g[2], g[3]);

        let end = audio_end(&data, start);
        Ok(Mp3Track {
            rate,
            channels,
            gain,
            start_pad: info.start_pad,
            end_pad: info.end_pad,
            end,
            spf,
            skip,
            total,
            estimate: estimate.max(0),
            decoder: Box::new(Mp3Decoder::new()),
            pcm: vec![0.0; MAX_SAMPLES_PER_FRAME],
            pcm_frames: 0,
            pcm_offset: 0,
            pcm_at: 0,
            at: start,
            stream_at: 0,
            from: skip,
            frames: vec![start],
            scan_free_format: 0,
            data,
        })
    }

    /// The next frame into pcm. False at the end.
    fn decode_next(&mut self) -> bool {
        loop {
            let limit = self.end;
            if self.at >= limit {
                return false;
            }
            let (samples, info) = self.decoder.decode_frame(&self.data[self.at..limit], Some(&mut self.pcm[..]));

            if info.channels == 0 {
                // No frame here: what is skipped is not audio.
                if info.frame_bytes == 0 {
                    return false;
                }
                self.at += info.frame_bytes;
                continue;
            }

            let index = (self.stream_at / self.spf) as usize;
            if index == self.frames.len() {
                self.frames.push(self.at + info.frame_offset);
            }
            self.at += info.frame_bytes;
            self.pcm_at = self.stream_at;
            self.pcm_offset = 0;
            self.pcm_frames = self.spf as usize;
            // A frame that could not be decoded (its reservoir is from before
            // a seek, or the file is damaged) still takes its place in time.
            if samples == 0 || info.channels != self.channels as usize {
                self.pcm.fill(0.0);
            }
            self.stream_at += self.spf;
            return true;
        }
    }

    /// The index grown to frame `n`, by headers alone. False if the frames
    /// end before it.
    fn index_to(&mut self, n: usize) -> bool {
        while self.frames.len() <= n {
            let at = self.frames[self.frames.len() - 1];
            let limit = self.end;
            if at + 4 > limit {
                return false;
            }
            let h = h4(&self.data, at);
            if !hdr_valid(h) {
                // Not a frame where one should be: find the next, as the decoder would.
                let (skip, _) = mp3d_find_frame(&self.data[at..limit], &mut self.scan_free_format);
                if skip >= limit - at {
                    return false;
                }
                let last = self.frames.len() - 1;
                self.frames[last] = at + skip;
                continue;
            }
            let size = hdr_frame_bytes(h, self.scan_free_format) + hdr_padding(h);
            if size <= 4 || at + size >= limit {
                return false;
            }
            self.frames.push(at + size);
        }
        true
    }
}

/// The Xing/Info tag (and LAME's inside it) or the VBRI tag, read in the
/// order FFmpeg reads them (mp3_parse_vbr_tags, mp3_parse_info_tag):
/// Mp3Track.cs's InfoTag.
fn info_tag(data: &[u8], frame: usize, frame_bytes: usize, h: [u8; 4]) -> Info {
    let none = [i32::MIN, 0, i32::MIN, 0];
    let (lsf, mono) = (!hdr_test_mpeg1(h), hdr_is_mono(h));
    let end = (frame + frame_bytes).min(data.len());
    let mut r = frame
        + 4
        + if lsf {
            if mono { 9 } else { 17 }
        } else if mono {
            17
        } else {
            32
        };
    let be32 = |i: usize| -> u32 {
        if i + 4 <= end { u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) } else { 0 }
    };
    let be16 = |i: usize| -> u32 { if i + 2 <= end { (data[i] as u32) << 8 | data[i + 1] as u32 } else { 0 } };

    let mut frames: i64 = 0;
    let mut header_file_size: u32 = 0;
    let mut lame = false;
    let (mut start_pad, mut end_pad) = (0, 0);
    let mut gain = none;
    let tag = be32(r);
    if tag == 0x58696E67 || tag == 0x496E666F {
        // "Xing", "Info"
        r += 4;
        let flags = be32(r);
        r += 4;
        if flags & 1 != 0 {
            frames = be32(r) as i64;
            r += 4;
        }
        if flags & 2 != 0 {
            header_file_size = be32(r);
            r += 4;
        }
        let file_size = data.len() as i64 - (frame as i64 + 4);
        if file_size > 0 && header_file_size > 0 {
            let hs = header_file_size as i64;
            let (min, delta) = (file_size.min(hs), file_size.max(hs) - file_size.min(hs));
            if file_size > hs && delta > min >> 4 {
                frames = 0; // a concatenated file
            }
        }
        if flags & 4 != 0 {
            r += 100; // the seek table, which exact seeking does not need
        }
        if flags & 8 != 0 {
            r += 4; // VBR quality
        }
        let version = if r + 9 <= end { be32(r) } else { 0 };
        r += 9;
        r += 2; // tag revision and VBR method, lowpass
        let peak = ((be32(r) as i64 * 100000 + (1 << 22)) >> 23) as i32; // av_rescale, to nearest
        r += 4;
        let rg = be16(r) as i32;
        r += 2;
        let ag = be16(r) as i32;
        r += 2;
        let (mut track_gain, mut album_gain) = (i32::MIN, i32::MIN);
        if rg >> 13 == 1 {
            track_gain = (rg & 0x1FF) * 10000;
            if rg & 0x200 != 0 {
                track_gain = -track_gain;
            }
        }
        if ag >> 13 == 2 {
            album_gain = (ag & 0x1FF) * 10000;
            if ag & 0x200 != 0 {
                album_gain = -album_gain;
            }
        }
        r += 2; // encoding flags, bitrate
        let delays =
            if r + 3 <= end { (data[r] as i64) << 16 | (data[r + 1] as i64) << 8 | data[r + 2] as i64 } else { 0 };
        r += 3;
        if version == 0x4C414D45 || version == 0x4C617666 || version == 0x4C617663 {
            // "LAME", "Lavf", "Lavc"
            lame = true;
            start_pad = delays >> 12;
            end_pad = delays & 4095;
        }
        r += 1 + 1 + 2 + 4 + 2; // misc, MP3 gain, preset, music length, music CRC
        if r + 2 <= end && be16(r) == crc16(&data[frame..r]) {
            gain = [track_gain, peak, album_gain, 0];
        }
    } else if be32(frame + 4 + 32) == 0x56425249 && be16(frame + 4 + 36) == 1 {
        // "VBRI", version 1
        header_file_size = be32(frame + 4 + 42);
        frames = be32(frame + 4 + 46) as i64;
    }
    // Without a count or a size, the frame was audio after all.
    let skip = frames != 0 || header_file_size != 0;
    Info { skip, lame, frames, start_pad, end_pad, gain }
}

/// CRC-16 with the polynomial 0xA001 (FFmpeg's ff_crcA001_update): the
/// LAME tag's check over the 190 bytes before it.
pub(crate) fn crc16(p: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for &b in p {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc
}

/// The end of the audio: before an ID3v1 tag, and an APEv2 or Lyrics3v2
/// tag before that (Mp3Track.cs's AudioEnd).
fn audio_end(data: &[u8], start: usize) -> usize {
    let len = data.len() as i64;
    let mut end = len;
    let at = |i: i64, s: &[u8]| i >= 0 && i + s.len() as i64 <= len && &data[i as usize..i as usize + s.len()] == s;
    let le32 = |i: i64| {
        i32::from_le_bytes([data[i as usize], data[i as usize + 1], data[i as usize + 2], data[i as usize + 3]])
    };
    if end >= 128 && at(end - 128, b"TAG") {
        end -= 128;
    }
    if at(end - 32, b"APETAGEX") {
        let size = le32(end - 32 + 12) as i64;
        let flags = le32(end - 32 + 20) as u32;
        end -= size + if flags & 0x80000000 != 0 { 32 } else { 0 };
    } else if at(end - 9, b"LYRICS200") && end >= 15 {
        // int.TryParse on the six digits before it.
        if let Some(lyrics) = std::str::from_utf8(&data[(end - 15) as usize..(end - 9) as usize])
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
        {
            end -= lyrics + 15;
        }
    }
    end.clamp(start as i64, len) as usize
}

/// The tags alone: the ID3v2 tags at the start.
pub fn tags(data: &[u8]) -> Tags {
    let mut t = Tags::default();
    tags::read_id3(data, 0, &mut t);
    t
}

/// Whether MPEG audio starts here (after any ID3v2 tags): a frame that the
/// frames after it agree with, found within the first few kilobytes, as
/// mp3d_find_frame finds the first frame. Layers I and II count, so that
/// opening one says why it is not played.
pub fn sniff(data: &[u8]) -> bool {
    let at = tags::id3_size(data).min(data.len());
    let window = &data[at..data.len().min(at + 2 * 4100)];
    let (_, bytes) = mp3d_find_frame(window, &mut 0);
    bytes != 0
}

impl Track for Mp3Track {
    fn rate(&self) -> u32 {
        self.rate
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn bits(&self) -> u16 {
        0 // lossy: no depth of its own
    }
    fn frames(&self) -> u64 {
        (if self.total >= 0 { self.total } else { self.estimate }) as u64
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
                if self.total >= 0 && self.stream_at - self.skip >= self.total {
                    break;
                }
                if !self.decode_next() {
                    break;
                }
                continue;
            }
            let s = self.pcm_at + self.pcm_offset as i64;
            if s < self.from {
                self.pcm_offset += ((self.from - s) as usize).min(self.pcm_frames - self.pcm_offset);
                continue;
            }
            let mut n = (want - done).min(self.pcm_frames - self.pcm_offset) as i64;
            if self.total >= 0 {
                n = n.min(self.total - (s - self.skip));
            }
            if n <= 0 {
                self.pcm_offset = self.pcm_frames;
                self.stream_at = self.skip + self.total;
                break;
            }
            let n = n as usize;
            into[done * channels..(done + n) * channels]
                .copy_from_slice(&self.pcm[self.pcm_offset * channels..(self.pcm_offset + n) * channels]);
            self.pcm_offset += n;
            done += n;
        }
        Ok(done)
    }

    /// Mp3Track.cs's SeekNow: the frame that holds the sample found in the
    /// index, and decoding started PREROLL frames before it from nothing.
    fn seek(&mut self, to: u64) -> Result<(), Error> {
        let target = (to.min(i64::MAX as u64 / 2) as i64).saturating_add(self.skip);
        let frame = (target / self.spf) as usize;
        let mut from = frame.saturating_sub(PREROLL);
        if !self.index_to(from) {
            from = self.frames.len().saturating_sub(1); // past the end: as far as known
        }

        self.decoder.reset();
        self.at = self.frames[from];
        self.stream_at = from as i64 * self.spf;
        self.pcm_frames = 0;
        self.pcm_offset = 0;
        self.from = self.skip.max(target);
        Ok(())
    }
}
