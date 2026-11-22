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
