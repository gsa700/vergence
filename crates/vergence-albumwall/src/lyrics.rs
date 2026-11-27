//! A song's lyrics as lines, timed or not (milestone 2, step 4; the Avalonia
//! app's Domain/Lyrics.cs). Most lyrics are plain for now and must read well
//! alone; timed ones (LRC: "[01:23.45]words") light up line by line.
//!
//! Where they come from, first found wins:
//! 1. a `.lrc` file beside the track, same name (Marginal writes one when it
//!    has timing; other taggers do too);
//! 2. the track's own tag (LYRICS / UNSYNCEDLYRICS in FLAC and Ogg, USLT in
//!    MP3, ©lyr in M4A), read in Hymie's confined helper as every tag is;
//! 3. for a music server's song, the server's structured lyrics
//!    (getLyricsBySongId, OpenSubsonic), timed ones preferred.
//!
//! LRC as found in the wild: several stamps on one line ("[00:12.00][01:30.00]
//! chorus"), minutes past 99, hundredths or thousandths or none, a colon where
//! the dot should be, an `[offset:+250]` in milliseconds (positive = earlier),
//! and `[ar:]`/`[ti:]`/`[al:]`/`[by:]` headers to skip. A line with no stamp
//! inside timed lyrics is a header or a credit and is dropped.

use std::path::Path;

/// One line: when it starts, in seconds (None in plain lyrics), and its
/// words ("" for a gap).
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub at: Option<f64>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lyrics {
    pub lines: Vec<Line>,
    /// Every line has a time, in order.
    pub timed: bool,
}

/// Lyrics as kept: one kind of line ending, nothing around them.
fn tidy(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n").trim().to_string()
}

/// "[mm:ss]", "[mm:ss.xx]", "[mm:ss.xxx]", "[mm:ss:xx]" at the start of `s`:
/// the time in seconds and the length of the stamp.
fn stamp(s: &str) -> Option<(f64, usize)> {
    let b = s.as_bytes();
    if b.first() != Some(&b'[') {
        return None;
    }
    let digits = |from: usize, most: usize| {
        let n = b[from..].iter().take(most).take_while(|c| c.is_ascii_digit()).count();
        (n > 0).then_some(n)
    };
    let m = digits(1, 3)?;
    let mut i = 1 + m;
    if b.get(i) != Some(&b':') {
        return None;
    }
    i += 1;
    let sec = digits(i, 2)?;
    let (minutes, seconds): (u64, u64) = (s[1..1 + m].parse().ok()?, s[i..i + sec].parse().ok()?);
    i += sec;
    let mut ms = 0u64;
    if matches!(b.get(i), Some(b'.' | b':')) {
        let f = digits(i + 1, 3)?;
        let frac = &s[i + 1..i + 1 + f];
        ms = match f {
            1 => frac.parse::<u64>().ok()? * 100,
            2 => frac.parse::<u64>().ok()? * 10,
            _ => frac.parse().ok()?,
        };
        i += 1 + f;
    }
    if b.get(i) != Some(&b']') {
        return None;
    }
    Some(((minutes * 60 + seconds) as f64 + ms as f64 / 1000.0, i + 1))
}

/// "[ar:Artist]", "[offset:+250]": a tag of letters, not a time, alone on its line.
fn header(line: &str) -> Option<(&str, &str)> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    let (key, value) = inner.split_once(':')?;
    (!key.is_empty() && key.chars().all(|c| c.is_ascii_alphabetic() || c == '#') && !value.contains(']'))
        .then_some((key, value))
}

impl Lyrics {
    pub fn parse(text: &str) -> Option<Lyrics> {
        let text = tidy(text);
        if text.is_empty() {
            return None;
        }
        let mut offset = 0.0;
        let mut timed: Vec<Line> = Vec::new();
        let mut plain: Vec<String> = Vec::new();
        for raw in text.split('\n') {
            let line = raw.trim();
            if let Some((key, value)) = header(line) {
                if key.eq_ignore_ascii_case("offset")
                    && let Ok(ms) = value.trim().trim_start_matches('+').parse::<i64>()
                {
                    offset = ms as f64 / 1000.0;
                }
                continue;
            }
            let mut stamps = Vec::new();
            let mut rest = line;
            while let Some((at, len)) = stamp(rest) {
                stamps.push(at);
                rest = &rest[len..];
            }
            if stamps.is_empty() {
                plain.push(line.to_string());
            } else {
                for at in stamps {
                    timed.push(Line { at: Some(at), text: rest.trim().to_string() });
                }
            }
        }
        if !timed.is_empty() {
            // [offset:+n] means the lyrics come n ms earlier. A stable sort keeps
            // a line's repeats in the order written.
            for l in &mut timed {
                l.at = l.at.map(|t| (t - offset).max(0.0));
            }
            timed.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal));
            return Some(Lyrics { lines: timed, timed: true });
        }
        // Plain: the verses' blank lines kept, but not a run of them.
        let mut kept: Vec<Line> = Vec::new();
        for l in plain {
            if !l.is_empty() || kept.last().is_some_and(|k| !k.text.is_empty()) {
                kept.push(Line { at: None, text: l });
            }
        }
        while kept.last().is_some_and(|k| k.text.is_empty()) {
            kept.pop();
        }
        (!kept.is_empty()).then_some(Lyrics { lines: kept, timed: false })
    }

    /// The line being sung at `seconds` in timed lyrics: the last one started.
    pub fn line_at(&self, seconds: f64) -> Option<usize> {
        if !self.timed {
            return None;
        }
        self.lines.iter().rposition(|l| l.at.is_some_and(|t| t <= seconds))
    }

    /// The lyrics of a file on this computer: the .lrc beside it, else its
    /// tag (LYRICS, then UNSYNCEDLYRICS), read in Hymie's helper. Not for
    /// the window's thread.
    pub fn for_file(path: &Path) -> Option<Lyrics> {
        let lrc = path.with_extension("lrc");
        if lrc != path
            && let Ok(meta) = std::fs::metadata(&lrc)
            && meta.is_file()
            && meta.len() <= MAX_LRC
            && let Ok(bytes) = std::fs::read(&lrc)
            && let Some(l) = Lyrics::parse(&String::from_utf8_lossy(&bytes))
        {
            return Some(l);
        }
        let mut scanner = vergence_hymie::helper::Scanner::new().ok()?;
        let facts = scanner.facts(path).ok()?;
        ["LYRICS", "UNSYNCEDLYRICS"].iter().find_map(|k| facts.tags.values.get(*k).and_then(|v| Lyrics::parse(v)))
    }

    /// A server's structured lyrics (getLyricsBySongId's `lyricsList`):
    /// the first timed set with words, else the first plain one.
    pub fn from_structured(list: &vergence_http::json::Value) -> Option<Lyrics> {
        let sets = list.get("structuredLyrics").map(|s| s.items()).unwrap_or_default();
        let synced = |s: &vergence_http::json::Value| s.get("synced").and_then(|v| v.bool()).unwrap_or(false);
        let mut ordered: Vec<_> = sets.into_iter().collect();
        // Timed first, keeping the server's order otherwise.
        ordered.sort_by_key(|s| !synced(s));
        for set in ordered {
            let timed = synced(set);
            let offset = set.get("offset").and_then(|v| v.number()).unwrap_or(0.0);
            let lines: Vec<Line> = set
                .get("line")
                .map(|l| l.items())
                .unwrap_or_default()
                .into_iter()
                .map(|l| Line {
                    at: if timed {
                        l.get("start").and_then(|v| v.number()).map(|ms| ((ms - offset) / 1000.0).max(0.0))
                    } else {
                        None
                    },
                    text: l.get("value").and_then(|v| v.text()).unwrap_or_default().trim().to_string(),
                })
                .collect();
            if lines.iter().all(|l| l.text.is_empty()) {
                continue;
            }
            let all_timed = timed && lines.iter().all(|l| l.at.is_some());
            let lines = if all_timed { lines } else { lines.into_iter().map(|l| Line { at: None, ..l }).collect() };
            return Some(Lyrics { lines, timed: all_timed });
        }
        None
    }
}

/// The largest .lrc read: more is not a song's words.
const MAX_LRC: u64 = 1 << 20;

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(l: &Lyrics) -> Vec<&str> {
        l.lines.iter().map(|x| x.text.as_str()).collect()
    }
    fn times(l: &Lyrics) -> Vec<f64> {
        l.lines.iter().map(|x| (x.at.unwrap() * 1000.0).round() / 1000.0).collect()
    }

    #[test]
    fn nothing_is_no_lyrics() {
        assert_eq!(Lyrics::parse(""), None);
        assert_eq!(Lyrics::parse("  \n \r\n"), None);
    }

    #[test]
    fn plain_lyrics_keep_their_lines_and_gaps() {
        let l = Lyrics::parse("First line\r\nSecond line\n\n\n\nAfter a gap\n\n").unwrap();
        assert!(!l.timed);
        assert_eq!(texts(&l), ["First line", "Second line", "", "After a gap"]);
        assert!(l.lines.iter().all(|x| x.at.is_none()));
        assert_eq!(l.line_at(10.0), None);
    }

    #[test]
    fn timed_lyrics_are_in_order_with_headers_dropped() {
        let l = Lyrics::parse("[ar:Someone]\n[ti:A song]\n[00:12.50]One\n[00:05.00]Zero\n[01:02.345]Two").unwrap();
        assert!(l.timed);
        assert_eq!(texts(&l), ["Zero", "One", "Two"]);
        assert_eq!(times(&l), [5.0, 12.5, 62.345]);
        assert_eq!(
            (l.line_at(4.0), l.line_at(5.0), l.line_at(30.0), l.line_at(999.0)),
            (None, Some(0), Some(1), Some(2))
        );
    }

    #[test]
    fn several_stamps_on_a_line_are_that_line_each_time() {
        let l = Lyrics::parse("[00:10.00][01:30.00]Chorus\n[00:50.00]Verse").unwrap();
        assert_eq!(texts(&l), ["Chorus", "Verse", "Chorus"]);
        assert_eq!(times(&l), [10.0, 50.0, 90.0]);
    }

    #[test]
    fn an_offset_moves_every_line_earlier_when_positive() {
        let l = Lyrics::parse("[offset:+250]\n[00:10.00]A").unwrap();
        assert_eq!(times(&l), [9.75]);
        let l = Lyrics::parse("[offset:-500]\n[00:10.00]A").unwrap();
        assert_eq!(times(&l), [10.5]);
    }

    #[test]
    fn stamps_as_written_in_the_wild_are_read() {
        let l = Lyrics::parse(
            "[00:01]No fraction\n[00:02:50]Colon for a dot\n[120:00.00]Past an hour and a half\n[00:03.5]Tenths",
        )
        .unwrap();
        assert_eq!(times(&l), [1.0, 2.5, 3.5, 7200.0]);
    }

    #[test]
    fn a_line_without_a_stamp_inside_timed_lyrics_is_dropped() {
        let l = Lyrics::parse("Lyrics by someone\n[00:01.00]A\n[00:02.00]B").unwrap();
        assert!(l.timed);
        assert_eq!(texts(&l), ["A", "B"]);
    }

    #[test]
    fn brackets_that_are_not_stamps_are_words() {
        let l = Lyrics::parse("[Chorus]\nla la [x:y] la\n[1:2:3:4]").unwrap();
        assert!(!l.timed);
        assert_eq!(texts(&l), ["[Chorus]", "la la [x:y] la", "[1:2:3:4]"]);
    }

    #[test]
    fn a_servers_timed_set_is_preferred() {
        let v = vergence_http::json::parse(
            br#"{"structuredLyrics":[
                {"synced":false,"line":[{"value":"plain one"}]},
                {"synced":true,"offset":100,"line":[{"start":1100,"value":"Hello "},{"start":2600,"value":"world"}]}]}"#,
        )
        .unwrap();
        let l = Lyrics::from_structured(&v).unwrap();
        assert!(l.timed);
        assert_eq!(texts(&l), ["Hello", "world"]);
        assert_eq!(times(&l), [1.0, 2.5]);
        let v = vergence_http::json::parse(
            br#"{"structuredLyrics":{"synced":false,"line":[{"value":"a"},{"value":"b"}]}}"#,
        )
        .unwrap();
        let l = Lyrics::from_structured(&v).unwrap();
        assert!(!l.timed);
        assert_eq!(texts(&l), ["a", "b"]);
        assert_eq!(Lyrics::from_structured(&vergence_http::json::parse(br#"{}"#).unwrap()), None);
    }

    #[test]
    fn an_lrc_beside_the_file_comes_first() {
        let dir = std::env::temp_dir().join(format!("aw-lrc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let track = dir.join("01 A Song.flac");
        std::fs::write(&track, b"not a real flac").unwrap();
        std::fs::write(dir.join("01 A Song.lrc"), "[00:01.00]From the file beside it\r\n").unwrap();
        let l = Lyrics::for_file(&track).unwrap();
        assert!(l.timed);
        assert_eq!(texts(&l), ["From the file beside it"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn random_text_never_panics() {
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        let pieces = ["[", "]", ":", ".", "0", "12", "999", "offset", "+", "-", "\n", "\r", "é", "x"];
        for _ in 0..20_000 {
            let mut s = String::new();
            for _ in 0..(x % 24) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                s.push_str(pieces[(x % pieces.len() as u64) as usize]);
            }
            let _ = Lyrics::parse(&s);
        }
    }
}
