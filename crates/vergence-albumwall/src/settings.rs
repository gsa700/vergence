//! AlbumWall's choices, as desktop.toml's `[albumwall]` keeps them
//! (question 5: the look and playback choices there, applied live; the
//! library list and the session in ~/.config/vergence/albumwall). A key
//! that is absent is the default, so the file holds only what was chosen,
//! and choosing the default again takes the key away.
//!
//! The names and defaults are the Avalonia app's (Settings.cs): resume on,
//! auto-play off, stop when an album ends, ReplayGain chosen by what is
//! playing until it is chosen here, Light 21, Tint 2, the ink bars, the
//! meters shown. Motion and text size are the desktop's (Settings ›
//! Accessibility), as for every Vergence app: the Avalonia app had its own
//! because other desktops gave it nothing to read (Dave, 2026-11-26).

use crate::colors;
use crate::player::Loudness;

pub const SECTION: &str = "[albumwall]";

/// What happens when an album ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterAlbum {
    Stop,
    NextAlbum,
    RandomAlbum,
    ShuffleSongs,
}

impl AfterAlbum {
    pub const ALL: [AfterAlbum; 4] =
        [AfterAlbum::Stop, AfterAlbum::NextAlbum, AfterAlbum::RandomAlbum, AfterAlbum::ShuffleSongs];

    /// As written in desktop.toml.
    pub fn key(self) -> &'static str {
        match self {
            AfterAlbum::Stop => "stop",
            AfterAlbum::NextAlbum => "next",
            AfterAlbum::RandomAlbum => "random",
            AfterAlbum::ShuffleSongs => "songs",
        }
    }

    fn from_key(k: &str) -> Option<AfterAlbum> {
        AfterAlbum::ALL.into_iter().find(|a| a.key() == k)
    }

    /// The play bar's button: on round the four (MainWindow's play mode).
    pub fn next(self) -> AfterAlbum {
        match self {
            AfterAlbum::Stop => AfterAlbum::NextAlbum,
            AfterAlbum::NextAlbum => AfterAlbum::RandomAlbum,
            AfterAlbum::RandomAlbum => AfterAlbum::ShuffleSongs,
            AfterAlbum::ShuffleSongs => AfterAlbum::Stop,
        }
    }

    /// The status line's word for it, as it is chosen.
    pub fn words(self) -> &'static str {
        match self {
            AfterAlbum::Stop => "Stop when the album ends",
            AfterAlbum::NextAlbum => "Then the next album",
            AfterAlbum::RandomAlbum => "Then a random album",
            AfterAlbum::ShuffleSongs => "Shuffle all songs",
        }
    }
}

/// Every choice, read from desktop.toml's text.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub resume: bool,
    pub auto_play: bool,
    pub after: AfterAlbum,
    /// None until chosen: by album, and by track for a shuffle of songs.
    pub gain: Option<Loudness>,
    pub meters_after_gain: bool,
    pub light: u32,
    pub tint: u32,
    /// An index into colors::CHROMES.
    pub bars: usize,
    pub meters: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            resume: true,
            auto_play: false,
            after: AfterAlbum::Stop,
            gain: None,
            meters_after_gain: false,
            light: colors::DEFAULT_LIGHT,
            tint: colors::DEFAULT_TINT,
            bars: colors::DEFAULT_CHROME,
            meters: true,
        }
    }
}

pub fn gain_key(g: Loudness) -> &'static str {
    match g {
        Loudness::Album => "album",
        Loudness::Track => "track",
        Loudness::Off => "off",
    }
}

impl Settings {
    pub fn from_text(text: &str) -> Settings {
        let d = Settings::default();
        let get = |k: &str| vergence_ui::table_value(text, SECTION, k);
        let flag = |k: &str, default: bool| get(k).map_or(default, |v| v != "false");
        let number = |k: &str| get(k).and_then(|v| v.parse::<u32>().ok());
        Settings {
            resume: flag("resume", d.resume),
            auto_play: flag("auto-play", d.auto_play),
            after: get("after-album").and_then(|v| AfterAlbum::from_key(&v)).unwrap_or(d.after),
            gain: get("replaygain").and_then(|v| match v.as_str() {
                "album" => Some(Loudness::Album),
                "track" => Some(Loudness::Track),
                "off" => Some(Loudness::Off),
                _ => None,
            }),
            meters_after_gain: flag("meters-after-gain", d.meters_after_gain),
            light: number("light").map_or(d.light, |l| l.clamp(colors::MIN_LIGHT, colors::MAX_LIGHT)),
            tint: number("tint").map_or(d.tint, |t| t.min(colors::MAX_TINT)),
            bars: get("bars").and_then(|v| colors::CHROMES.iter().position(|c| c.name == v)).unwrap_or(d.bars),
            meters: flag("meters", d.meters),
        }
    }

    /// The loudness for what is about to play: the choice, else by album
    /// for an album and by track for a shuffle of songs.
    pub fn loudness(&self, shuffle: bool) -> Loudness {
        self.gain.unwrap_or(if shuffle { Loudness::Track } else { Loudness::Album })
    }

    /// Each key and what it would be written as: None where the value is
    /// the default (the key taken away).
    pub fn written(&self) -> Vec<(&'static str, Option<String>)> {
        let d = Settings::default();
        let differs = |b: bool, v: String| b.then_some(v);
        vec![
            ("resume", differs(self.resume != d.resume, self.resume.to_string())),
            ("auto-play", differs(self.auto_play != d.auto_play, self.auto_play.to_string())),
            ("after-album", differs(self.after != d.after, format!("\"{}\"", self.after.key()))),
            ("replaygain", self.gain.map(|g| format!("\"{}\"", gain_key(g)))),
            (
                "meters-after-gain",
                differs(self.meters_after_gain != d.meters_after_gain, self.meters_after_gain.to_string()),
            ),
            ("light", differs(self.light != d.light, self.light.to_string())),
            ("tint", differs(self.tint != d.tint, self.tint.to_string())),
            ("bars", differs(self.bars != d.bars, format!("\"{}\"", colors::CHROMES[self.bars].name))),
            ("meters", differs(self.meters != d.meters, self.meters.to_string())),
        ]
    }

    /// desktop.toml's text with these choices written into `[albumwall]`,
    /// key by key, the rest of the file as it was.
    pub fn write_into(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (k, v) in self.written() {
            out = vergence_widgets::prefs::set_key(&out, SECTION, k, v.as_deref());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_is_the_default() {
        assert_eq!(Settings::from_text(""), Settings::default());
        assert_eq!(Settings::from_text("[albumwall]\n"), Settings::default());
        // The default written back adds nothing.
        assert_eq!(Settings::default().write_into("[other]\nx = 1\n"), "[other]\nx = 1\n");
    }

    #[test]
    fn every_choice_round_trips() {
        let s = Settings {
            resume: false,
            auto_play: true,
            after: AfterAlbum::ShuffleSongs,
            gain: Some(Loudness::Off),
            meters_after_gain: true,
            light: 33,
            tint: 9,
            bars: 0,
            meters: false,
        };
        let text = s.write_into("[appearance]\naccent = \"#123456\"\n");
        assert_eq!(Settings::from_text(&text), s);
        assert!(text.contains("accent = \"#123456\""));
        // And back to the defaults: the keys go.
        let back = Settings::default().write_into(&text);
        assert_eq!(Settings::from_text(&back), Settings::default());
        assert!(!back.contains("light"));
    }

    #[test]
    fn out_of_range_is_held_or_ignored() {
        let s = Settings::from_text("[albumwall]\nlight = 99\ntint = 40\nbars = \"chrome\"\n");
        assert_eq!((s.light, s.tint, s.bars), (colors::MAX_LIGHT, colors::MAX_TINT, colors::DEFAULT_CHROME));
    }

    #[test]
    fn loudness_by_context_until_chosen() {
        let s = Settings::default();
        assert_eq!((s.loudness(false), s.loudness(true)), (Loudness::Album, Loudness::Track));
        let s = Settings { gain: Some(Loudness::Off), ..s };
        assert_eq!((s.loudness(false), s.loudness(true)), (Loudness::Off, Loudness::Off));
    }
}
