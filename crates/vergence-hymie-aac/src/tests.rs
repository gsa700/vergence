//! The AAC decoder against its fixtures (vergence-hymie's fixtures/README.md):
//! gapless length as FFmpeg's edit list says, the tones at their level and
//! frequency and in place, seeking equal to reading through, FFmpeg's own
//! decode close by (when given it), and damage an error, never a panic.

use super::*;
use vergence_hymie::{Format, format_of};

const AAC44: &[u8] = include_bytes!("../../vergence-hymie/fixtures/aac44.m4a");
const AAC48: &[u8] = include_bytes!("../../vergence-hymie/fixtures/aac48.m4a");
const BURST: &[u8] = include_bytes!("../../vergence-hymie/fixtures/aacburst.m4a");
const TAGGED: &[u8] = include_bytes!("../../vergence-hymie/fixtures/aac.m4a");

/// The fixtures: name, bytes, rate, channels, the samples given to the encoder.
const FIXTURES: [(&str, &[u8], u32, u16, u64); 4] = [
    ("aac44", AAC44, 44_100, 2, 66_150),
    ("aac48", AAC48, 48_000, 1, 48_000),
    ("aacburst", BURST, 48_000, 2, 48_000),
    ("aac", TAGGED, 44_100, 1, 22_050),
];

/// All the samples of a track, read in pieces of `chunk` frames.
fn all(t: &mut dyn Track, chunk: usize) -> Vec<f32> {
    let ch = t.channels() as usize;
    let mut out = Vec::new();
    let mut buf = vec![0f32; chunk * ch];
    loop {
        let n = t.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n * ch]);
    }
    out
}

fn all_lossy(t: &mut dyn Track) -> usize {
    let mut buf = vec![0f32; 1024 * t.channels() as usize];
    let mut n = 0;
    while let Ok(k) = t.read(&mut buf) {
        if k == 0 {
            break;
        }
        n += k;
    }
    n
}

#[test]
fn aac_is_gapless_by_its_edit_list() {
    for (name, data, rate, ch, samples) in FIXTURES {
        assert_eq!(format_of(data), Some(Format::Mp4), "{name}");
        let mut t = open(data.to_vec()).unwrap();
        assert_eq!((t.rate(), t.channels(), t.bits(), t.frames()), (rate, ch, 0, samples), "{name}");
        // Read in pieces that do not divide a frame: exactly the samples
        // the encoder was given (its 1024 priming samples dropped).
        assert_eq!(all(t.as_mut(), 1000).len() as u64, samples * ch as u64, "{name}");
    }
    // vergence-hymie alone still refuses them, for the helper to send on.
    assert!(matches!(vergence_hymie::open(AAC44.to_vec()), Err(Error::NeedsAac)));
    assert_eq!(vergence_hymie::tags(AAC44).title.as_deref(), Some("AAC Stereo"));
}

/// Upward zero crossings of channel `c` of interleaved `s`.
fn crossings(s: &[f32], ch: usize, c: usize) -> usize {
    s.chunks_exact(ch).map(|f| f[c]).collect::<Vec<_>>().windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
}

#[test]
fn aac_decodes_the_tones_in_place() {
    // (fixture, channels, channel, frequency, rate); every tone is a sine at
    // 0.5 starting at sample 0 (fixtures/README.md).
    for (data, ch, c, hz, rate) in
        [(AAC44, 2, 0, 440.0, 44_100.0), (AAC44, 2, 1, 660.0, 44_100.0), (AAC48, 1, 0, 1000.0, 48_000.0)]
    {
        let mut t = open(data.to_vec()).unwrap();
        let s = all(t.as_mut(), 4096);
        let n = s.len() / ch;
        // The level by RMS, not the peak: FFmpeg's encoder overshoots (0.75
        // at the 440 Hz onset, 0.62 within the 660 Hz tone), and FFmpeg's own
        // decoder gives the same samples. A sine at 0.5 is 0.354 RMS.
        let middle = &s[2048 * ch..(n - 2048) * ch];
        let level =
            (middle.chunks_exact(ch).map(|f| (f[c] as f64).powi(2)).sum::<f64>() / (middle.len() / ch) as f64).sqrt();
        assert!((level - 0.5 / 2f64.sqrt()).abs() < 0.01, "{hz} Hz: RMS level {level}");
        let seconds = n as f64 / rate;
        let k = crossings(&s, ch, c) as f64;
        assert!((k - hz * seconds).abs() <= 2.0, "{hz} Hz: {k} crossings in {seconds} s");
        // Gapless puts every sample where it was: the decoded sound is
        // closest to the sine as it was given, not to it a sample early or late.
        let rms = |shift: f64| {
            ((0..n)
                .map(|i| {
                    (s[i * ch + c] as f64 - 0.5 * (std::f64::consts::TAU * hz * (i as f64 + shift) / rate).sin())
                        .powi(2)
                })
                .sum::<f64>()
                / n as f64)
                .sqrt()
        };
        let (early, here, late) = (rms(-1.0), rms(0.0), rms(1.0));
        eprintln!("{hz} Hz: RMS level {level:.4}, RMS from the sine {early:.2e} / {here:.2e} / {late:.2e}");
        assert!(here < 0.03 && here < early && here < late, "{hz} Hz: RMS from the sine {early} / {here} / {late}");
    }
}

#[test]
fn aac_seeks_to_the_exact_sample() {
    // These fixtures have no PNS (noise is random by design), so a seek,
    // decoding one frame before from nothing, gives exactly the samples a
    // straight read does (Mp4Track.cs's promise). aac.m4a was encoded with
    // FFmpeg's default, PNS on, and is left out.
    for (name, data, _, ch, samples) in &FIXTURES[..3] {
        let (ch, samples) = (*ch, *samples);
        let ch = ch as usize;
        let mut t = open(data.to_vec()).unwrap();
        let whole = all(t.as_mut(), 999);
        for to in [0u64, 1, 1023, 1024, 1025, 2047, 2048, 5000, samples / 2 + 7, samples - 1] {
            t.seek(to).unwrap();
            let mut buf = vec![0f32; ch * 3000];
            let n = t.read(&mut buf).unwrap();
            assert_eq!(n as u64, 3000.min(samples - to), "{name}: seek to {to}");
            assert!(buf[..n * ch] == whole[to as usize * ch..(to as usize + n) * ch], "{name}: seek to {to}");
        }
        t.seek(samples).unwrap();
        assert_eq!(t.read(&mut [0f32; 64]).unwrap(), 0, "{name}: the end");
        t.seek(u64::MAX).unwrap();
        assert_eq!(t.read(&mut [0f32; 64]).unwrap(), 0, "{name}: past the end");
    }
}

/// The fixtures against FFmpeg's decode of them (its float AAC decoder,
/// gapless as here), when HYMIE_AAC_REFERENCE names a folder of
/// <fixture>.f32 files made by `ffmpeg -i <fixture>.m4a -f f32le
/// <fixture>.f32`: the same length and nearly the same samples (two
/// decoders of a lossy format, each with its own transform, are not bit for
/// bit the same).
#[test]
fn aac_matches_ffmpeg_when_given_its_decode() {
    let Some(dir) = std::env::var_os("HYMIE_AAC_REFERENCE") else { return };
    for (name, data, ..) in &FIXTURES[..3] {
        let raw = std::fs::read(std::path::Path::new(&dir).join(format!("{name}.f32"))).unwrap();
        let reference: Vec<f32> = raw.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        let mut t = open(data.to_vec()).unwrap();
        let ours = all(t.as_mut(), 4096);
        assert_eq!(ours.len(), reference.len(), "{name}: length");
        let (mut diff, mut sig, mut worst) = (0f64, 0f64, 0f32);
        for (a, b) in ours.iter().zip(&reference) {
            diff += ((a - b) as f64).powi(2);
            sig += (*b as f64).powi(2);
            worst = worst.max((a - b).abs());
        }
        let n = ours.len() as f64;
        let rms = (diff / n).sqrt();
        let (dbfs, below) = (20.0 * rms.max(1e-30).log10(), 10.0 * (sig / diff.max(1e-30)).log10());
        eprintln!(
            "{name}: RMS difference {rms:.2e} ({dbfs:.1} dBFS), largest {worst:.2e}, {below:.1} dB below the signal"
        );
        assert!(dbfs < -60.0, "{name}: RMS difference {rms} ({dbfs:.1} dBFS)");
    }
}

/// Each fixture's frames straight through the decoder: how many channels
/// used short windows, M/S, intensity stereo, TNS and pulses.
fn tools_used(data: &[u8], asc: [u8; 2]) -> [usize; 5] {
    // The frames by the file's own sample table (stsz sizes, stsc runs,
    // stco chunk offsets), read here directly from these small fixtures.
    let find = |kind: &[u8]| data.windows(4).position(|w| w == kind).unwrap() + 4;
    let be = |at: usize| u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
    let stsz = find(b"stsz");
    let sizes: Vec<usize> = (0..be(stsz + 8)).map(|i| be(stsz + 12 + i * 4)).collect();
    let stco = find(b"stco");
    let stsc = find(b"stsc");
    let runs: Vec<(usize, usize)> =
        (0..be(stsc + 4)).map(|i| (be(stsc + 8 + i * 12), be(stsc + 12 + i * 12))).collect();
    let chunks: Vec<usize> = (0..be(stco + 4)).map(|i| be(stco + 8 + i * 4)).collect();
    let mut dec = AacDecoder::from_config(&asc).unwrap();
    let mut pcm = vec![0f32; 2048];
    let mut sample = 0;
    for (c, &start) in chunks.iter().enumerate() {
        let per = runs.iter().rev().find(|r| r.0 <= c + 1).unwrap().1;
        let mut at = start;
        for _ in 0..per {
            let Some(&size) = sizes.get(sample) else { break };
            dec.decode_frame(&data[at..at + size], &mut pcm).unwrap();
            at += size;
            sample += 1;
        }
    }
    assert_eq!(sample, sizes.len());
    dec.seen
}

#[test]
fn the_fixtures_reach_the_tools() {
    let burst = tools_used(BURST, [0x11, 0x90]);
    let stereo = tools_used(AAC44, [0x12, 0x10]);
    let mono = tools_used(AAC48, [0x11, 0x88]);
    eprintln!("short windows, M/S, intensity, TNS, pulses: aacburst {burst:?}, aac44 {stereo:?}, aac48 {mono:?}");
    // FFmpeg's encoder writes no pulses, and PNS is off: those two are left
    // to the damage tests.
    assert!(burst[0] > 0 && burst[1] > 0 && burst[2] > 0 && burst[3] > 0, "aacburst: {burst:?}");
}

#[test]
fn only_aac_lc_is_taken() {
    // AudioSpecificConfigs: object type (5 bits), rate index (4), channels (4), GASpecificConfig (3).
    let asc = |object: u32, rate: u32, channels: u32| {
        let v = object << 11 | rate << 7 | channels << 3;
        [(v >> 8) as u8, v as u8]
    };
    assert!(AacDecoder::from_config(&asc(2, 4, 2)).is_ok());
    assert!(AacDecoder::from_config(&asc(2, 3, 1)).is_ok());
    for (object, rate, channels) in
        [(5, 4, 2), (29, 4, 2), (1, 4, 2), (4, 4, 2), (2, 4, 6), (2, 4, 0), (2, 12, 2), (2, 15, 2)]
    {
        assert!(AacDecoder::from_config(&asc(object, rate, channels)).is_err(), "{object} {rate} {channels}");
    }
    // Implicit SBR signalled after the config (0x2b7, type 5, flag 1).
    let mut sbr = asc(2, 4, 2).to_vec();
    sbr.extend_from_slice(&[0x56, 0xe5, 0x80]);
    assert!(matches!(AacDecoder::from_config(&sbr), Err(Error::Unsupported(_))));
    assert!(AacDecoder::from_config(&[]).is_err());
}

#[test]
fn damaged_aac_is_not_a_panic() {
    // Cut anywhere (and AAC48 at every frame's worth of bytes, about).
    for (_, data, ..) in FIXTURES {
        for cut in (0..data.len()).step_by(if data == AAC48 { 61 } else { 499 }) {
            if let Ok(mut t) = open(data[..cut].to_vec()) {
                let _ = all_lossy(t.as_mut());
            }
        }
    }
    // Every byte of the moov flipped in turn (the sample tables, the esds),
    // and the frames' bytes every so often.
    for data in [AAC48, BURST] {
        let moov = data.windows(4).position(|w| w == b"moov").unwrap() - 4;
        let every = if data == AAC48 { 1 } else { 5 };
        for at in (moov..data.len()).step_by(every).chain((0..moov).step_by(41)) {
            let mut d = data.to_vec();
            d[at] ^= 0xa5;
            if let Ok(mut t) = open(d) {
                let _ = all_lossy(t.as_mut());
                let _ = t.seek(20_000);
                let _ = all_lossy(t.as_mut());
            }
        }
    }
    // Handfuls of random bytes in the frames.
    let mut x: u32 = 7;
    let mut rand = || {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        x >> 8
    };
    for data in [AAC44, AAC48, BURST] {
        let mdat = data.windows(4).position(|w| w == b"mdat").unwrap() + 4;
        let moov = data.windows(4).position(|w| w == b"moov").unwrap() - 4;
        for _ in 0..40 {
            let mut d = data.to_vec();
            for _ in 0..16 {
                let at = mdat + rand() as usize % (moov - mdat);
                d[at] = rand() as u8;
            }
            if let Ok(mut t) = open(d) {
                let _ = all_lossy(t.as_mut());
                let _ = t.seek(rand() as u64 % 70_000);
                let _ = all_lossy(t.as_mut());
            }
        }
    }
    // Frames of nothing but noise, straight into the decoder, mono and stereo.
    for asc in [[0x12, 0x10], [0x12, 0x08]] {
        let mut dec = AacDecoder::from_config(&asc).unwrap();
        let mut pcm = vec![0f32; 2048];
        for len in 0..3000 {
            let frame: Vec<u8> = (0..len % 700).map(|_| rand() as u8).collect();
            let _ = dec.decode_frame(&frame, &mut pcm);
            assert!(pcm.iter().all(|v| v.is_finite()));
        }
    }
}
