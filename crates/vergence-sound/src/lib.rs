//! Sound (docs/sound-milestone-1.md): the machine's outputs by the names
//! people know, which one sound goes to, and each one's volume and mute.
//! Used by the settings app's Sound page and the server's volume keys.
//!
//! One interface, [`Sound`], and a back end per system, chosen at run time:
//! OSS on FreeBSD ([`oss`], step 1); ALSA on Linux without a sound server
//! (step 2); with one (PipeWire or PulseAudio, the platform's standard on
//! Fedora), the PulseAudio protocol both speak ([`pulse`], our own client,
//! 2026-11-20).
//! Volume and mute need no privilege on either; FreeBSD's default output
//! needs root: the helper `sound-default` (step 5, [`oss::helper_main`]).

use std::io;

#[cfg(target_os = "freebsd")]
pub mod oss;
#[cfg(target_os = "linux")]
pub mod alsa;
#[cfg(target_os = "linux")]
pub mod pulse;

/// One output, as the page shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// Stable while the system's devices stay as they are ("pcm1" on OSS).
    pub id: String,
    /// What people call it: "Front headphones", "Internal speaker", "HDMI".
    pub name: String,
    /// What the system calls it ("Realtek ALC294 (Front Analog)").
    pub detail: String,
    /// 0 to 100.
    pub volume: u8,
    /// Whether the volume can be set here (an HDMI output's is the monitor's).
    pub adjustable: bool,
    pub muted: bool,
    /// Where sound goes when a program does not choose.
    pub default: bool,
}

/// A sound system we can read and set.
pub trait Sound {
    /// Which system this is, for the page and the log ("OSS").
    fn system(&self) -> &'static str;
    fn outputs(&mut self) -> io::Result<Vec<Output>>;
    /// 0 to 100; more is taken as 100.
    fn set_volume(&mut self, id: &str, volume: u8) -> io::Result<()>;
    fn set_muted(&mut self, id: &str, muted: bool) -> io::Result<()>;
    /// Where sound goes from now on. On FreeBSD this needs root.
    fn set_default(&mut self, id: &str) -> io::Result<()>;
    /// A test tone ([`tone_samples`]) on the output, in a stream of
    /// `channels` channels, heard on those numbered in `on` (from 1).
    /// Returns what the device took, to tell the user.
    fn tone(&mut self, id: &str, channels: u32, on: &[u32]) -> io::Result<String>;
}

/// The test tone: 440 Hz for 1.5 s at a tenth of full scale (about -20 dB,
/// heard plainly without being harsh at full volume), with 30 ms ramps at
/// each end so it starts and stops without a click; interleaved 16-bit
/// samples, the tone in the channels numbered in `on` (from 1), silence in
/// the others.
pub fn tone_samples(channels: u32, on: &[u32], rate: u32) -> Vec<i16> {
    const HZ: f64 = 440.0;
    const SECONDS: f64 = 1.5;
    const LEVEL: f64 = 0.1;
    const RAMP: f64 = 0.03;
    let frames = (rate as f64 * SECONDS) as usize;
    let mut out = Vec::with_capacity(frames * channels as usize);
    for i in 0..frames {
        let t = i as f64 / rate as f64;
        let ramp = (t / RAMP).min((SECONDS - t) / RAMP).clamp(0.0, 1.0);
        let v = ((2.0 * std::f64::consts::PI * HZ * t).sin() * LEVEL * ramp * i16::MAX as f64) as i16;
        for c in 1..=channels {
            out.push(if on.contains(&c) { v } else { 0 });
        }
    }
    out
}

/// The sound system of this machine, if we know how to speak to it.
pub fn open() -> Option<Box<dyn Sound>> {
    #[cfg(target_os = "freebsd")]
    {
        oss::Oss::open().map(|o| Box::new(o) as Box<dyn Sound>)
    }
    #[cfg(target_os = "linux")]
    {
        // A sound server owns the devices when one runs: we ask it, as
        // every program's sound goes through it.
        if pipewire_running() {
            return pulse::Pulse::open().map(|p| Box::new(p) as Box<dyn Sound>);
        }
        alsa::Alsa::open().map(|a| Box::new(a) as Box<dyn Sound>)
    }
    #[cfg(not(any(target_os = "freebsd", target_os = "linux")))]
    {
        None
    }
}

/// Sound played as it is written, to the default output (preview milestone
/// 1: a preview's Play). The output may play at another rate than asked
/// (an ALSA device that takes only its own) or with fewer channels: the
/// writer converts to [`Stream::rate`] and [`Stream::channels`].
pub trait Stream: Send {
    fn rate(&self) -> u32;
    fn channels(&self) -> u32;
    /// Interleaved 16-bit samples, a whole number of frames; blocks while
    /// the output is full, so writing paces the playing.
    fn write(&mut self, samples: &[i16]) -> io::Result<()>;
}

/// A stream to the default output, at `rate` and `channels` if it takes them.
/// `VERGENCE_SOUND_OFF=1` refuses it (the display suite presses Play
/// without making a sound on the machine it runs on).
pub fn stream(rate: u32, channels: u32) -> io::Result<Box<dyn Stream>> {
    match std::env::var("VERGENCE_SOUND_OFF").as_deref() {
        // Taken at the real rate and heard nowhere: the display suite plays
        // without making a sound.
        Ok("pace") => return Ok(Box::new(Paced { rate, channels: channels.max(1) })),
        Ok(_) => return Err(io::Error::other("sound is off (VERGENCE_SOUND_OFF)")),
        Err(_) => {}
    }
    #[cfg(target_os = "freebsd")]
    {
        oss::stream(rate, channels)
    }
    #[cfg(target_os = "linux")]
    {
        if pipewire_running() { pulse::stream(rate, channels) } else { alsa::stream(rate, channels) }
    }
    #[cfg(not(any(target_os = "freebsd", target_os = "linux")))]
    {
        let _ = (rate, channels);
        Err(io::Error::other("no sound system here"))
    }
}

/// `VERGENCE_SOUND_OFF=pace`: a stream that takes samples at their rate and plays them nowhere.
struct Paced {
    rate: u32,
    channels: u32,
}

impl Stream for Paced {
    fn rate(&self) -> u32 {
        self.rate
    }
    fn channels(&self) -> u32 {
        self.channels
    }
    fn write(&mut self, samples: &[i16]) -> io::Result<()> {
        let frames = samples.len() as f64 / self.channels as f64;
        std::thread::sleep(std::time::Duration::from_secs_f64(frames / self.rate.max(1) as f64));
        Ok(())
    }
}

/// Whether PipeWire (or PulseAudio) has the user's sound: its socket is in
/// the runtime directory.
#[cfg(target_os = "linux")]
pub fn pipewire_running() -> bool {
    std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).is_some_and(|d| d.join("pipewire-0").exists() || d.join("pulse/native").exists())
}

/// An output's name from a codec pin's configuration, as FreeBSD's driver
/// prints it ("device=Headphones conn=Jack ctype=1/8 loc=Front ..."):
/// "Front headphones", "Internal speaker", "Rear line out", "HDMI". None
/// for pins that are not outputs.
pub fn pin_name(config: &str) -> Option<String> {
    let field = |key: &str| config.split_whitespace().find_map(|w| w.strip_prefix(key)).unwrap_or("");
    let what = match field("device=") {
        "Headphones" => "headphones",
        "Speaker" => "speaker",
        "Line-out" => "line out",
        "Digital-out" | "SPDIF-out" => "digital out",
        _ => return None,
    };
    if field("conn=") == "None" {
        return None;
    }
    // HDMI and DisplayPort show as digital out on the GPU's codec.
    let place = match field("loc=") {
        "Internal" => "Internal",
        "Front" => "Front",
        "Rear" => "Rear",
        "Left" => "Left",
        "Right" => "Right",
        "Top" => "Top",
        "Bottom" => "Bottom",
        _ => "",
    };
    let name = if place.is_empty() { what.to_string() } else { format!("{place} {what}") };
    let mut c = name.chars();
    Some(c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default())
}

/// An ALSA card from a line of /proc/asound/cards
/// (" 0 [PCH            ]: HDA-Intel - HDA Intel PCH"): its number and name.
pub fn alsa_card_line(line: &str) -> Option<(u32, String)> {
    let (num, rest) = line.trim_start().split_once(' ')?;
    let num = num.parse().ok()?;
    let name = rest.split_once(" - ").map(|(_, n)| n.trim()).unwrap_or(rest.trim());
    Some((num, name.to_string()))
}

/// The default card and device from an ~/.asoundrc (0, 0 if it names none).
pub fn asoundrc_default(text: &str) -> (u32, u32) {
    let get = |key: &str| text.lines().find_map(|l| l.trim().strip_prefix(key).and_then(|v| v.trim().parse().ok())).unwrap_or(0);
    (get("defaults.pcm.card"), get("defaults.pcm.device"))
}

/// An ALSA volume control's value as 0 to 100, from its range.
pub fn range_percent(min: i64, max: i64, v: i64) -> u8 {
    if max <= min {
        return 100;
    }
    (((v - min) * 100 + (max - min) / 2) / (max - min)).clamp(0, 100) as u8
}

/// 0 to 100 as a value in an ALSA control's range.
pub fn percent_range(min: i64, max: i64, percent: u8) -> i64 {
    min + ((max - min) * percent.min(100) as i64 + 50) / 100
}

/// The pins a device drives, from its location ("nid=33,26").
pub fn location_pins(location: &str) -> Vec<u32> {
    location.strip_prefix("nid=").map(|l| l.split(',').filter_map(|n| n.trim().parse().ok()).collect()).unwrap_or_default()
}

/// OSS's level: left in the low byte, right in the next, each 0 to 100.
/// The louder side is the output's volume.
pub fn level_percent(level: i32) -> u8 {
    let (left, right) = (level & 0xff, (level >> 8) & 0xff);
    left.max(right).clamp(0, 100) as u8
}

/// The same level on both sides.
pub fn level_of(percent: u8) -> i32 {
    let p = percent.min(100) as i32;
    p | (p << 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_named_as_people_know_them() {
        // kepler's (FreeBSD's snd_hda, Realtek ALC294).
        assert_eq!(pin_name("0x02211020 as=2 seq=0 device=Headphones conn=Jack ctype=1/8 loc=Front color=Black misc=0").as_deref(), Some("Front headphones"));
        assert_eq!(pin_name("0x90170110 as=1 seq=0 device=Speaker conn=Fixed ctype=Analog loc=Internal color=Unknown misc=1").as_deref(), Some("Internal speaker"));
        assert_eq!(pin_name("0x01014010 as=1 seq=0 device=Line-out conn=Jack ctype=1/8 loc=Rear color=Green misc=0").as_deref(), Some("Rear line out"));
        assert_eq!(pin_name("0x18560010 as=1 seq=0 device=Digital-out conn=Jack ctype=Digital loc=0x18 color=Unknown misc=0").as_deref(), Some("Digital out"));
        // Inputs and unconnected pins are no outputs.
        assert_eq!(pin_name("0x02a11030 as=3 seq=0 device=Mic conn=Jack ctype=1/8 loc=Front color=Black misc=0"), None);
        assert_eq!(pin_name("0x411111f0 as=15 seq=0 device=Speaker conn=None ctype=1/8 loc=Rear color=Black misc=1"), None);
    }

    #[test]
    fn a_location_names_its_pins() {
        assert_eq!(location_pins("nid=33,26"), vec![33, 26]);
        assert_eq!(location_pins("nid=3"), vec![3]);
        assert_eq!(location_pins("something else"), Vec::<u32>::new());
    }

    #[test]
    fn the_tone_is_where_it_was_asked_and_quiet() {
        let s = tone_samples(4, &[3, 4], 48_000);
        assert_eq!(s.len(), 72_000 * 4, "1.5 s of four channels");
        // Channels 1 and 2 silent, 3 and 4 the tone, never past a tenth of full scale.
        assert!(s.chunks(4).all(|f| f[0] == 0 && f[1] == 0 && f[2] == f[3]));
        let peak = s.iter().map(|v| v.unsigned_abs()).max().unwrap();
        assert!(peak > 3000 && peak <= (i16::MAX as u16) / 10 + 1, "peak {peak}");
        // It starts and ends at silence (the ramps).
        assert_eq!((s[2], s[s.len() - 2]), (0, 0));
    }

    #[test]
    fn alsa_s_words_are_read() {
        assert_eq!(alsa_card_line(" 0 [PCH            ]: HDA-Intel - HDA Intel PCH"), Some((0, "HDA Intel PCH".into())));
        assert_eq!(alsa_card_line("                      HDA Intel PCH at 0xb1230000 irq 142"), None);
        assert_eq!(asoundrc_default("# ours\ndefaults.pcm.card 1\ndefaults.pcm.device 3\ndefaults.ctl.card 1\n"), (1, 3));
        assert_eq!(asoundrc_default(""), (0, 0));
        // Hubble's Master: 0 to 87.
        assert_eq!((range_percent(0, 87, 87), range_percent(0, 87, 0), range_percent(0, 87, 44)), (100, 0, 51));
        assert_eq!((percent_range(0, 87, 100), percent_range(0, 87, 0), percent_range(0, 87, 50)), (87, 0, 44));
        assert_eq!(range_percent(0, 87, percent_range(0, 87, 37)), 37);
    }

    #[test]
    fn levels_go_both_ways() {
        assert_eq!(level_of(100), 0x6464);
        assert_eq!(level_percent(0x6464), 100);
        assert_eq!(level_percent(0x3250), 80, "the louder side");
        assert_eq!(level_of(250), level_of(100), "at most 100");
        assert_eq!(level_percent(level_of(37)), 37);
    }
}
