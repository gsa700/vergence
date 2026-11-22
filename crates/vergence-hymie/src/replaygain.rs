//! ReplayGain from a track's tags, for every format: the four tags are the
//! same words in FLAC's Vorbis comments, ID3's TXXX frames and MP4's freeform
//! atoms. Read the way mpv read them (through FFmpeg's demuxer,
//! libavformat/replaygain.c), as Hymie does (Spindle ReplayGain.cs): a gain
//! counts only with a peak above zero beside it, an album without its own
//! gain takes the track's, and with neither there is no ReplayGain at all.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ReplayGain {
    pub present: bool,
    pub track_gain: f32,
    pub track_peak: f32,
    pub album_gain: f32,
    pub album_peak: f32,
}

/// A missing gain, as FFmpeg keeps it.
const MISSING: i32 = i32::MIN;

impl ReplayGain {
    /// From tags whose keys are upper case.
    pub fn from_tags(tags: &HashMap<String, String>) -> ReplayGain {
        let [tg, tp, ag, ap] = raw(tags);
        ReplayGain::from_raw(tg, tp, ag, ap)
    }

    /// From the four values as FFmpeg keeps them, in hundred-thousandths.
    pub fn from_raw(tg: i32, tp: i32, ag: i32, ap: i32) -> ReplayGain {
        let track = tg != MISSING && tp != 0;
        let album = ag != MISSING && ap != 0;
        if !track && !album {
            return ReplayGain::default();
        }
        let mut r = ReplayGain { present: true, track_gain: 0.0, track_peak: 1.0, album_gain: 0.0, album_peak: 1.0 };
        if track {
            r.track_gain = tg as f32 / 100_000.0;
            r.album_gain = r.track_gain;
            r.track_peak = tp as f32 / 100_000.0;
            r.album_peak = r.track_peak;
        }
        if album {
            r.album_gain = ag as f32 / 100_000.0;
            r.album_peak = ap as f32 / 100_000.0;
        }
        r
    }

    /// The multiplier, by album or by track: mpv's compute_replaygain,
    /// clipping prevention on, no pre-amp, no fallback gain.
    pub fn multiplier(&self, album: bool) -> f32 {
        if !self.present {
            return 1.0;
        }
        let (gain, peak) = if album { (self.album_gain, self.album_peak) } else { (self.track_gain, self.track_peak) };
        10f32.powf(gain / 20.0).min(1.0 / peak)
    }
}

/// The four values FFmpeg keeps (ff_replaygain_export; Hymie's
/// ReplayGain.Raw), in hundred-thousandths, a missing gain as i32::MIN and a
/// missing peak as 0. An MP3 takes its LAME tag's when these have no gain.
pub(crate) fn raw(tags: &HashMap<String, String>) -> [i32; 4] {
    let get = |k: &str, missing: i32| tags.get(k).map_or(missing, |v| parse(v, missing));
    [
        get("REPLAYGAIN_TRACK_GAIN", MISSING),
        get("REPLAYGAIN_TRACK_PEAK", 0),
        get("REPLAYGAIN_ALBUM_GAIN", MISSING),
        get("REPLAYGAIN_ALBUM_PEAK", 0),
    ]
}

/// FFmpeg's parse_value: a whole number, then up to five decimals, in
/// hundred-thousandths ("-7.53 dB" is -753000; what follows is ignored).
fn parse(text: &str, missing: i32) -> i32 {
    let s = text.trim_start_matches([' ', '\t']).as_bytes();
    let negative = s.first() == Some(&b'-');
    let mut i = usize::from(matches!(s.first(), Some(b'-' | b'+')));
    let mut whole: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        whole = (whole * 10 + (s[i] - b'0') as i64).min(i32::MAX as i64);
        i += 1;
    }
    let mut fraction: i64 = 0;
    if i < s.len() && s[i] == b'.' {
        let mut scale = 10_000;
        let mut j = i + 1;
        while scale > 0 && j < s.len() && s[j].is_ascii_digit() {
            fraction += scale * (s[j] - b'0') as i64;
            scale /= 10;
            j += 1;
        }
    }
    if whole > (i32::MAX as i64 - fraction) / 100_000 {
        return missing;
    }
    let value = whole * 100_000 + fraction;
    (if negative { -value } else { value }) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gains_are_read_as_ffmpeg_reads_them() {
        assert_eq!(parse("-7.53 dB", MISSING), -753_000);
        assert_eq!(parse("+1.000001", MISSING), 100_000);
        assert_eq!(parse("0.988525", 0), 98_852);
        let tags: HashMap<String, String> = [("REPLAYGAIN_TRACK_GAIN", "-6.00 dB"), ("REPLAYGAIN_TRACK_PEAK", "0.5")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let g = ReplayGain::from_tags(&tags);
        assert!(g.present && g.album_gain == g.track_gain, "the album takes the track's");
        assert!((g.multiplier(false) - 0.501).abs() < 0.001);
        assert!(!ReplayGain::from_tags(&HashMap::new()).present);
    }
}
