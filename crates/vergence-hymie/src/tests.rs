//! The decoders against their fixtures (fixtures/README.md): FLAC decoded
//! whole matches the MD5 its STREAMINFO keeps; seeking lands on the exact
//! sample; tags and the cover are read; a WAV written here reads back the
//! same; peaks follow the sound; damaged files are refused, never a panic.

use super::*;

const TONE16: &[u8] = include_bytes!("../fixtures/tone16.flac");
const TONE24: &[u8] = include_bytes!("../fixtures/tone24.flac");

#[test]
fn flac_decodes_to_the_md5_it_keeps() {
    assert_eq!(flac_check(TONE16), Ok(Some(true)), "16-bit stereo, LPC");
    assert_eq!(flac_check(TONE24), Ok(Some(true)), "24-bit mono");
}

#[test]
fn flac_opens_with_its_facts_tags_and_cover() {
    let t = open(TONE16.to_vec()).unwrap();
    assert_eq!((t.rate(), t.channels(), t.bits(), t.frames()), (44_100, 2, 16, 66_150));
    assert!((t.seconds() - 1.5).abs() < 1e-9);
    assert!(t.gain().present && (t.gain().track_gain + 6.0).abs() < 1e-6);
    let tags = tags(TONE16);
    assert_eq!(tags.title.as_deref(), Some("Test Tone"));
    assert_eq!(tags.artist.as_deref(), Some("Vergence"));
    assert_eq!(tags.album.as_deref(), Some("Hymie Fixtures"));
    let pic = tags.picture.expect("the cover");
    assert_eq!(pic.mime, "image/png");
    assert!(pic.data.starts_with(b"\x89PNG"));
    let t24 = open(TONE24.to_vec()).unwrap();
    assert_eq!((t24.rate(), t24.channels(), t24.bits(), t24.frames()), (48_000, 1, 24, 48_000));
}

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

#[test]
fn flac_seeks_to_the_exact_sample() {
    let mut t = open(TONE16.to_vec()).unwrap();
    let whole = all(t.as_mut(), 1000);
    assert_eq!(whole.len(), 66_150 * 2);
    for to in [1u64, 4095, 4096, 33_333, 66_149] {
        t.seek(to).unwrap();
        let mut buf = vec![0f32; 2 * 64];
        let n = t.read(&mut buf).unwrap();
        assert!(n > 0);
        assert_eq!(&buf[..n * 2], &whole[to as usize * 2..(to as usize + n) * 2], "seek to {to}");
    }
    t.seek(66_150).unwrap();
    assert_eq!(t.read(&mut [0f32; 64]).unwrap(), 0, "the end");
}

/// A WAV of `samples` (interleaved, 16-bit) with an id3 chunk holding a title.
fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let mut id3 = b"ID3\x03\x00\x00".to_vec();
    let frame = [&b"TIT2"[..], &[0, 0, 0, 6, 0, 0], &b"\x00Hello"[..]].concat();
    let size = frame.len() as u32;
    id3.extend_from_slice(&[(size >> 21) as u8 & 0x7f, (size >> 14) as u8 & 0x7f, (size >> 7) as u8 & 0x7f, size as u8 & 0x7f]);
    id3.extend_from_slice(&frame);
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut w = b"RIFF\0\0\0\0WAVE".to_vec();
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    w.extend_from_slice(&(channels * 2).to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"id3 ");
    w.extend_from_slice(&(id3.len() as u32).to_le_bytes());
    w.extend_from_slice(&id3);
    if id3.len() % 2 == 1 {
        w.push(0);
    }
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    let riff = (w.len() - 8) as u32;
    w[4..8].copy_from_slice(&riff.to_le_bytes());
    w
}

#[test]
fn a_wav_reads_back_its_samples_and_title() {
    let samples: Vec<i16> = (0..2000).map(|i| ((i * 37) % 65_536 - 32_768) as i16).collect();
    let w = wav(22_050, 2, &samples);
    let mut t = open(w.clone()).unwrap();
    assert_eq!((t.rate(), t.channels(), t.bits(), t.frames()), (22_050, 2, 16, 1000));
    let got = all(t.as_mut(), 300);
    assert_eq!(got.len(), 2000);
    for (g, s) in got.iter().zip(&samples) {
        assert_eq!(*g, *s as f32 / 32768.0);
    }
    t.seek(500).unwrap();
    let mut buf = [0f32; 2];
    t.read(&mut buf).unwrap();
    assert_eq!(buf[0], samples[1000] as f32 / 32768.0);
    assert_eq!(tags(&w).title.as_deref(), Some("Hello"));
}

#[test]
fn peaks_follow_the_sound() {
    let mut t = open(TONE16.to_vec()).unwrap();
    let mut told = 0;
    let p = peaks(t.as_mut(), 100, |done| {
        told = done.len();
        true
    })
    .unwrap();
    assert_eq!(told, 100, "the last telling is the whole");
    // ffmpeg's sines at 1/8, times 0.8: every slice swings to about ±0.1 on both channels.
    for s in &p {
        for c in s {
            assert!(c.0 < -0.095 && c.1 > 0.095 && c.1 < 0.101, "{c:?}");
        }
    }
}

#[test]
fn damaged_files_are_refused_not_a_panic() {
    assert!(matches!(open(b"not sound at all".to_vec()), Err(Error::Unsupported(_))));
    // Every truncation and a byte flipped every so often: an error or wrong
    // sound, never a panic.
    for cut in (0..TONE16.len()).step_by(997) {
        if let Ok(mut t) = open(TONE16[..cut].to_vec()) {
            let _ = all_lossy(t.as_mut());
        }
    }
    for at in (0..TONE24.len()).step_by(211) {
        let mut d = TONE24.to_vec();
        d[at] ^= 0x5a;
        if let Ok(mut t) = open(d) {
            let _ = all_lossy(t.as_mut());
            let _ = t.seek(20_000);
        }
    }
    let w = wav(8000, 1, &[1, 2, 3]);
    for cut in 0..w.len() {
        if let Ok(mut t) = open(w[..cut].to_vec()) {
            let _ = all_lossy(t.as_mut());
        }
    }
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

// ------------------------------------------------------------ MP3

const TONE44: &[u8] = include_bytes!("../fixtures/tone44.mp3");
const TONE22: &[u8] = include_bytes!("../fixtures/tone22.mp3");
const BURST48: &[u8] = include_bytes!("../fixtures/burst48.mp3");

/// The fixtures: bytes, rate, channels, the samples given to lame.
const MP3S: [(&str, &[u8], u32, u16, u64); 3] = [("tone44", TONE44, 44_100, 2, 66_150), ("tone22", TONE22, 22_050, 1, 22_050), ("burst48", BURST48, 48_000, 2, 48_000)];

#[test]
fn mp3_is_gapless_by_its_lame_tag() {
    for (name, data, rate, ch, samples) in MP3S {
        assert_eq!(format_of(data), Some(Format::Mp3), "{name}");
        let mut t = open(data.to_vec()).unwrap();
        assert_eq!((t.rate(), t.channels(), t.bits(), t.frames()), (rate, ch, 0, samples), "{name}");
        // Read in pieces that do not divide a frame: exactly the samples lame was given.
        assert_eq!(all(t.as_mut(), 1000).len() as u64, samples * ch as u64, "{name}");
    }
    let t = mp3::Mp3Track::new(TONE44.to_vec()).unwrap();
    assert_eq!((t.start_pad, t.end_pad), (576, 1242), "the LAME tag's delay and padding");
    let tags = tags(TONE44);
    assert_eq!(tags.title.as_deref(), Some("Test Tone MP3"));
    assert_eq!(tags.artist.as_deref(), Some("Vergence"));
    assert_eq!(tags.album.as_deref(), Some("Hymie Fixtures"));
    assert_eq!(super::tags(TONE22).title.as_deref(), Some("Mono Tone"));
}

/// Upward zero crossings of channel `c` of interleaved `s`.
fn crossings(s: &[f32], ch: usize, c: usize) -> usize {
    s.chunks_exact(ch).map(|f| f[c]).collect::<Vec<_>>().windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
}

#[test]
fn mp3_decodes_the_tones_in_place() {
    // (fixture, channel, frequency, seconds); every tone is a sine at 0.5
    // starting at sample 0 (fixtures/README.md).
    for (data, ch, c, hz, rate) in [(TONE44, 2, 0, 440.0, 44_100.0), (TONE44, 2, 1, 660.0, 44_100.0), (TONE22, 1, 0, 1000.0, 22_050.0)] {
        let mut t = open(data.to_vec()).unwrap();
        let s = all(t.as_mut(), 4096);
        let n = s.len() / ch;
        let peak = s.chunks_exact(ch).map(|f| f[c].abs()).fold(0.0, f32::max);
        assert!((0.47..0.53).contains(&peak), "{hz} Hz: peak {peak}");
        let seconds = n as f64 / rate;
        let k = crossings(&s, ch, c) as f64;
        assert!((k - hz * seconds).abs() <= 2.0, "{hz} Hz: {k} crossings in {seconds} s");
        // Gapless puts every sample where it was: the decoded sound is
        // closest to the sine as it was given, not to it a sample early or
        // late (lame's own noise on these tones is about 0.018 RMS).
        let rms = |shift: f64| ((0..n).map(|i| (s[i * ch + c] as f64 - 0.5 * (std::f64::consts::TAU * hz * (i as f64 + shift) / rate).sin()).powi(2)).sum::<f64>() / n as f64).sqrt();
        let (early, here, late) = (rms(-1.0), rms(0.0), rms(1.0));
        assert!(here < 0.025 && here < early && here < late, "{hz} Hz: RMS from the sine {early} / {here} / {late}");
    }
}

#[test]
fn mp3_seeks_to_the_exact_sample() {
    for (name, data, _, ch, samples) in MP3S {
        let ch = ch as usize;
        let mut t = open(data.to_vec()).unwrap();
        let whole = all(t.as_mut(), 999);
        for to in [0u64, 1, 575, 576, 1151, 1152, 5000, 11_520, samples / 2 + 7, samples - 1] {
            t.seek(to).unwrap();
            let mut buf = vec![0f32; ch * 3000];
            let n = t.read(&mut buf).unwrap();
            assert_eq!(n as u64, 3000.min(samples - to), "{name}: seek to {to}");
            // Exactly the same: decoding starts ten frames back, from nothing.
            assert_eq!(&buf[..n * ch], &whole[to as usize * ch..(to as usize + n) * ch], "{name}: seek to {to}");
        }
        t.seek(samples).unwrap();
        assert_eq!(t.read(&mut [0f32; 64]).unwrap(), 0, "{name}: the end");
        t.seek(u64::MAX).unwrap();
        assert_eq!(t.read(&mut [0f32; 64]).unwrap(), 0, "{name}: past the end");
    }
}

/// An ID3v2.3 tag of TXXX frames (description, value).
fn id3_txxx(values: &[(&str, &str)]) -> Vec<u8> {
    let mut frames = Vec::new();
    for (k, v) in values {
        let body = [&[0u8][..], k.as_bytes(), &[0], v.as_bytes()].concat();
        frames.extend_from_slice(b"TXXX");
        frames.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frames.extend_from_slice(&[0, 0]);
        frames.extend_from_slice(&body);
    }
    let size = frames.len();
    let mut tag = b"ID3\x03\x00\x00".to_vec();
    tag.extend_from_slice(&[(size >> 21) as u8 & 0x7f, (size >> 14) as u8 & 0x7f, (size >> 7) as u8 & 0x7f, size as u8 & 0x7f]);
    tag.extend_from_slice(&frames);
    tag
}

#[test]
fn mp3_replaygain_from_the_lame_tag_or_id3() {
    // lame wrote a track gain of -8.2 dB but no peak: no ReplayGain, as FFmpeg has it.
    assert!(!open(TONE44.to_vec()).unwrap().gain().present);
    // The peak put in (0.5, 9.23 fixed point) and the tag's CRC made again.
    let info = TONE44.windows(4).position(|w| w == b"Info").unwrap();
    let (frame, lame) = (info - 36, info + 120);
    assert_eq!(&TONE44[lame..lame + 4], b"LAME");
    let mut d = TONE44.to_vec();
    d[lame + 11..lame + 15].copy_from_slice(&0x0040_0000u32.to_be_bytes());
    let crc = mp3::crc16(&d[frame..lame + 34]) as u16;
    d[lame + 34..lame + 36].copy_from_slice(&crc.to_be_bytes());
    let g = open(d.clone()).unwrap().gain();
    assert!(g.present && (g.track_gain + 8.2).abs() < 1e-5 && (g.track_peak - 0.5).abs() < 1e-5, "{g:?}");
    // A wrong CRC: the LAME tag's gain is not believed.
    d[lame + 35] ^= 1;
    assert!(!open(d.clone()).unwrap().gain().present);
    d[lame + 35] ^= 1;
    // TXXX gains in an ID3 tag before the first win over the LAME tag's.
    let mut tagged = id3_txxx(&[("REPLAYGAIN_TRACK_GAIN", "-3.00 dB"), ("REPLAYGAIN_TRACK_PEAK", "0.9")]);
    tagged.extend_from_slice(&d);
    let mut t = open(tagged.clone()).unwrap();
    let g = t.gain();
    assert!(g.present && (g.track_gain + 3.0).abs() < 1e-5 && (g.track_peak - 0.9).abs() < 1e-5, "{g:?}");
    assert_eq!(all(t.as_mut(), 4096).len(), 66_150 * 2, "two ID3 tags before the frames");
}

/// Each frame header from the first on, by its length.
fn frame_starts(data: &[u8]) -> Vec<usize> {
    let mut at = tags::id3_size(data);
    let mut out = Vec::new();
    while mp3::hdr_valid(mp3::h4(data, at)) {
        out.push(at);
        let h = mp3::h4(data, at);
        at += mp3::hdr_frame_bytes(h, 0) + mp3::hdr_padding(h);
    }
    out
}

#[test]
fn mp3_refuses_layer_ii_and_damage_is_not_a_panic() {
    // The same frames called Layer II (the same length at the same rate):
    // MPEG audio, refused for what it is.
    let starts = frame_starts(TONE44);
    assert_eq!(starts.len(), 60, "the Info frame and 59 of audio");
    let mut mp2 = TONE44.to_vec();
    for &at in &starts {
        mp2[at + 1] = (mp2[at + 1] & !0x06) | 0x04;
    }
    assert!(matches!(open(mp2), Err(Error::Unsupported(_))));
    // No frames after the tags.
    let mut none = id3_txxx(&[("X", "y")]);
    none.extend_from_slice(&[0u8; 5000]);
    assert!(matches!(open(none), Err(Error::Unsupported(_))));

    // Every truncation, bytes flipped one at a time, and handfuls of random
    // bytes in the audio: an error or wrong sound, never a panic.
    for (_, data, ..) in MP3S {
        for cut in (0..data.len()).step_by(397) {
            if let Ok(mut t) = open(data[..cut].to_vec()) {
                let _ = all_lossy(t.as_mut());
            }
        }
    }
    for at in (0..TONE22.len()).step_by(7) {
        let mut d = TONE22.to_vec();
        d[at] ^= 0xa5;
        if let Ok(mut t) = open(d) {
            let _ = all_lossy(t.as_mut());
            let _ = t.seek(9000);
            let _ = all_lossy(t.as_mut());
        }
    }
    let mut x: u32 = 1;
    let mut rand = || {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        x >> 8
    };
    let first = frame_starts(BURST48)[1];
    for _ in 0..150 {
        let mut d = BURST48.to_vec();
        for _ in 0..12 {
            let at = first + rand() as usize % (d.len() - first);
            d[at] = rand() as u8;
        }
        if let Ok(mut t) = open(d) {
            let _ = all_lossy(t.as_mut());
            let _ = t.seek(rand() as u64 % 60_000);
            let _ = all_lossy(t.as_mut());
        }
    }
    // Pure noise after a frame header, frame after frame.
    let mut d = TONE44[..starts[2]].to_vec();
    for _ in 0..40 {
        let h = &TONE44[starts[2]..starts[2] + 4];
        d.extend_from_slice(h);
        for _ in 0..413 {
            d.push(rand() as u8);
        }
    }
    if let Ok(mut t) = open(d) {
        let _ = all_lossy(t.as_mut());
    }
}

/// The fixtures against FFmpeg's decode of them (its float decoder, gapless
/// as here), when HYMIE_MP3_REFERENCE names a folder of <fixture>.f32 files
/// made by `ffmpeg -i <fixture>.mp3 -f f32le <fixture>.f32`: the same length
/// and almost the same samples (both are minimp3-like float decoders, not
/// bit for bit the same).
#[test]
fn mp3_matches_ffmpeg_when_given_its_decode() {
    let Some(dir) = std::env::var_os("HYMIE_MP3_REFERENCE") else { return };
    for (name, data, ..) in MP3S {
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
        let (rms, snr) = ((diff / n).sqrt(), 10.0 * (sig / diff.max(1e-30)).log10());
        eprintln!("{name}: RMS difference {rms:.2e}, largest {worst:.2e}, {snr:.1} dB below the signal");
        assert!(rms < 1e-4, "{name}: RMS difference {rms}");
    }
}
