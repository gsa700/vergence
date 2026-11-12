//! ALSA, on Linux without a sound server (Void on Hubble): its kernel
//! interface, spoken here directly as the wire protocols are, with no
//! alsa-lib between. Sizes and numbers measured from Hubble's
//! <sound/asound.h> (2026-11-11, x86_64: `long` is 8 bytes).
//!
//! - **Outputs** are a card's playback PCM devices (`/proc/asound/cardN/
//!   pcmDp/info`): the analog one (speakers and headphones, which the card
//!   switches itself) and one per HDMI or DisplayPort connector.
//! - **Volume and mute** are the card's mixer controls, through
//!   `/dev/snd/controlCN` (group `audio`): "Master Playback Volume" and
//!   "Master Playback Switch" for the analog output; an HDMI output has no
//!   volume, only its "IEC958 Playback Switch" (by connector index).
//! - **The default** is per user, `~/.asoundrc` (`defaults.pcm.card`,
//!   `defaults.pcm.device`, `defaults.ctl.card`): written only when it does
//!   not exist or we wrote it (our first line), never over a file of the
//!   user's own.
//! - **The test tone** goes to `/dev/snd/pcmCNDMp` with the PCM ioctls:
//!   hardware parameters (16-bit, interleaved, the channels and rate asked,
//!   the rest left to the kernel to choose), prepare, write, drain.

use std::{
    fs::{self, OpenOptions},
    io,
    os::fd::AsRawFd,
    path::PathBuf,
};

use crate::{Output, Sound};

unsafe extern "C" {
    fn ioctl(fd: i32, request: std::ffi::c_ulong, arg: *mut u8) -> i32;
}

// Control interface (sound/asound.h, measured on Hubble).
const ELEM_LIST: std::ffi::c_ulong = 0xc050_5510;
const ELEM_INFO: std::ffi::c_ulong = 0xc110_5511;
const ELEM_READ: std::ffi::c_ulong = 0xc4c8_5512;
const ELEM_WRITE: std::ffi::c_ulong = 0xc4c8_5513;
const ID_SIZE: usize = 64; // numid, iface, device, subdevice, name[44] at 16, index at 60
const INFO_SIZE: usize = 272; // id; type at 64; count at 72; value (min, max, step: long) at 80
const VALUE_SIZE: usize = 1224; // id; value (long[128]) at 72
const IFACE_MIXER: u32 = 2;

// PCM interface.
const HW_PARAMS: std::ffi::c_ulong = 0xc260_4111;
const PREPARE: std::ffi::c_ulong = 0x4140;
const DRAIN: std::ffi::c_ulong = 0x4144;
const WRITEI_FRAMES: std::ffi::c_ulong = 0x4018_4150;
const HW_PARAMS_SIZE: usize = 608; // flags; masks[3] at 4 (32 bytes each); intervals at 260 (12 each, params 8-19); rmask at 512
const MASKS_AT: usize = 4;
const INTERVALS_AT: usize = 260;
const RMASK_AT: usize = 512;
const PARAM_ACCESS: usize = 0;
const PARAM_FORMAT: usize = 1;
const PARAM_SUBFORMAT: usize = 2;
const PARAM_FIRST_INTERVAL: usize = 8;
const PARAM_LAST_INTERVAL: usize = 19;
const PARAM_CHANNELS: usize = 10;
const PARAM_RATE: usize = 11;
const ACCESS_RW_INTERLEAVED: usize = 3;
const FORMAT_S16_LE: usize = 2;
const SUBFORMAT_STD: usize = 0;
const RATE: u32 = 48_000;

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_ne_bytes());
}
fn get_u32(b: &[u8], at: usize) -> u32 {
    u32::from_ne_bytes(b[at..at + 4].try_into().unwrap())
}
fn get_i64(b: &[u8], at: usize) -> i64 {
    i64::from_ne_bytes(b[at..at + 8].try_into().unwrap())
}
fn put_i64(b: &mut [u8], at: usize, v: i64) {
    b[at..at + 8].copy_from_slice(&v.to_ne_bytes());
}

fn call(f: &fs::File, request: std::ffi::c_ulong, buf: &mut [u8]) -> io::Result<()> {
    // SAFETY: `buf` is the size the request's number encodes (measured above).
    if unsafe { ioctl(f.as_raw_fd(), request, buf.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// A mixer control by name and index, as found on a card.
#[derive(Clone)]
struct Control {
    id: [u8; ID_SIZE],
    name: String,
    index: u32,
}

/// A card's mixer controls.
fn controls(card: u32) -> io::Result<(fs::File, Vec<Control>)> {
    let f = OpenOptions::new().read(true).write(true).open(format!("/dev/snd/controlC{card}"))?;
    // First how many, then that many ids.
    let mut list = [0u8; 80];
    call(&f, ELEM_LIST, &mut list)?;
    let count = get_u32(&list, 12) as usize;
    let mut ids = vec![0u8; count * ID_SIZE];
    put_u32(&mut list, 0, 0); // offset
    put_u32(&mut list, 4, count as u32); // space
    list[16..24].copy_from_slice(&(ids.as_mut_ptr() as u64).to_ne_bytes());
    call(&f, ELEM_LIST, &mut list)?;
    let used = (get_u32(&list, 8) as usize).min(count);
    let out = (0..used)
        .map(|i| {
            let mut id = [0u8; ID_SIZE];
            id.copy_from_slice(&ids[i * ID_SIZE..(i + 1) * ID_SIZE]);
            let name = String::from_utf8_lossy(id[16..60].split(|c| *c == 0).next().unwrap_or(&[])).into_owned();
            let index = get_u32(&id, 60);
            Control { id, name, index }
        })
        .filter(|c| get_u32(&c.id, 4) == IFACE_MIXER)
        .collect();
    Ok((f, out))
}

fn find<'a>(cs: &'a [Control], name: &str, index: u32) -> Option<&'a Control> {
    cs.iter().find(|c| c.name == name && c.index == index)
}

/// A control's range (integer controls) and its first value.
fn read(f: &fs::File, c: &Control) -> io::Result<(i64, i64, i64)> {
    let mut info = vec![0u8; INFO_SIZE];
    info[..ID_SIZE].copy_from_slice(&c.id);
    call(f, ELEM_INFO, &mut info)?;
    let (min, max) = (get_i64(&info, 80), get_i64(&info, 88));
    let mut v = vec![0u8; VALUE_SIZE];
    v[..ID_SIZE].copy_from_slice(&c.id);
    call(f, ELEM_READ, &mut v)?;
    Ok((min, max, get_i64(&v, 72)))
}

/// Sets every channel of a control (both sides of a stereo volume) to `value`.
fn write(f: &fs::File, c: &Control, value: i64) -> io::Result<()> {
    let mut info = vec![0u8; INFO_SIZE];
    info[..ID_SIZE].copy_from_slice(&c.id);
    call(f, ELEM_INFO, &mut info)?;
    let channels = get_u32(&info, 72).clamp(1, 128) as usize;
    let mut v = vec![0u8; VALUE_SIZE];
    v[..ID_SIZE].copy_from_slice(&c.id);
    for ch in 0..channels {
        put_i64(&mut v, 72 + ch * 8, value);
    }
    call(f, ELEM_WRITE, &mut v)
}

/// The cards, by number, from /proc/asound/cards (" 0 [PCH ]: HDA-Intel - HDA Intel PCH").
fn cards() -> Vec<(u32, String)> {
    let text = fs::read_to_string("/proc/asound/cards").unwrap_or_default();
    text.lines().filter_map(crate::alsa_card_line).collect()
}

/// A card's playback devices: (device number, its name), from /proc/asound.
fn playback_devices(card: u32) -> Vec<(u32, String)> {
    let mut out: Vec<(u32, String)> = fs::read_dir(format!("/proc/asound/card{card}"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let dev = name.strip_prefix("pcm")?.strip_suffix('p')?.parse().ok()?;
            let info = fs::read_to_string(e.path().join("info")).ok()?;
            let pcm_name = info.lines().find_map(|l| l.strip_prefix("name: ")).unwrap_or("").to_string();
            Some((dev, pcm_name))
        })
        .collect();
    out.sort();
    out
}

fn asoundrc() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".asoundrc"))
}

const MARK: &str = "# Written by Vergence's sound settings (vergence-sound); edit freely, or remove the file to go back to ALSA's default.";

pub struct Alsa;

impl Alsa {
    pub fn open() -> Option<Alsa> {
        (!cards().is_empty()).then_some(Alsa)
    }

    fn parse(id: &str) -> io::Result<(u32, u32)> {
        id.strip_prefix("hw:").and_then(|r| r.split_once(',')).and_then(|(c, d)| Some((c.parse().ok()?, d.parse().ok()?))).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no output {id}")))
    }

    /// The default (card, device): ours from ~/.asoundrc, else ALSA's own, 0,0.
    fn default() -> (u32, u32) {
        asoundrc().and_then(|p| fs::read_to_string(p).ok()).map(|t| crate::asoundrc_default(&t)).unwrap_or((0, 0))
    }

    /// Whether `pcm_name` is an HDMI or DisplayPort device, and which of them (from 0).
    fn hdmi_index(card: u32, dev: u32, pcm_name: &str) -> Option<u32> {
        if !pcm_name.contains("HDMI") && !pcm_name.contains("DP") {
            return None;
        }
        let hdmi: Vec<u32> = playback_devices(card).into_iter().filter(|(_, n)| n.contains("HDMI") || n.contains("DP")).map(|(d, _)| d).collect();
        hdmi.iter().position(|d| *d == dev).map(|p| p as u32)
    }
}

impl Sound for Alsa {
    fn system(&self) -> &'static str {
        "ALSA"
    }

    fn outputs(&mut self) -> io::Result<Vec<Output>> {
        let default = Self::default();
        let mut out = Vec::new();
        for (card, card_name) in cards() {
            let (f, cs) = match controls(card) {
                Ok(x) => x,
                Err(_) => continue,
            };
            for (dev, pcm_name) in playback_devices(card) {
                let (name, volume, muted) = match Self::hdmi_index(card, dev, &pcm_name) {
                    Some(k) => {
                        let muted = find(&cs, "IEC958 Playback Switch", k).and_then(|c| read(&f, c).ok()).is_some_and(|(_, _, v)| v == 0);
                        (format!("HDMI or DisplayPort {}", k + 1), 100, muted)
                    }
                    None => {
                        let (speaker, phones) = (find(&cs, "Speaker Playback Volume", 0).is_some(), find(&cs, "Headphone Playback Volume", 0).is_some());
                        let name = match (speaker, phones) {
                            (true, true) => "Speakers and headphones".to_string(),
                            (true, false) => "Speakers".to_string(),
                            (false, true) => "Headphones".to_string(),
                            _ => pcm_name.clone(),
                        };
                        let volume = find(&cs, "Master Playback Volume", 0).and_then(|c| read(&f, c).ok()).map(|(min, max, v)| crate::range_percent(min, max, v)).unwrap_or(100);
                        let muted = find(&cs, "Master Playback Switch", 0).and_then(|c| read(&f, c).ok()).is_some_and(|(_, _, v)| v == 0);
                        (name, volume, muted)
                    }
                };
                out.push(Output { id: format!("hw:{card},{dev}"), name, detail: format!("{card_name}: {pcm_name}"), volume, muted, default: (card, dev) == default });
            }
        }
        Ok(out)
    }

    fn set_volume(&mut self, id: &str, volume: u8) -> io::Result<()> {
        let (card, dev) = Self::parse(id)?;
        let pcm_name = playback_devices(card).into_iter().find(|(d, _)| *d == dev).map(|(_, n)| n).unwrap_or_default();
        if Self::hdmi_index(card, dev, &pcm_name).is_some() {
            return Err(io::Error::new(io::ErrorKind::Unsupported, format!("{id} has no volume of its own (HDMI: the monitor sets it)")));
        }
        let (f, cs) = controls(card)?;
        let c = find(&cs, "Master Playback Volume", 0).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no Master volume on this card"))?;
        let (min, max, _) = read(&f, c)?;
        write(&f, c, crate::percent_range(min, max, volume))
    }

    fn set_muted(&mut self, id: &str, muted: bool) -> io::Result<()> {
        let (card, dev) = Self::parse(id)?;
        let pcm_name = playback_devices(card).into_iter().find(|(d, _)| *d == dev).map(|(_, n)| n).unwrap_or_default();
        let (f, cs) = controls(card)?;
        let c = match Self::hdmi_index(card, dev, &pcm_name) {
            Some(k) => find(&cs, "IEC958 Playback Switch", k),
            None => find(&cs, "Master Playback Switch", 0),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no mute switch for {id}")))?;
        // A switch is on when sound passes.
        write(&f, c, if muted { 0 } else { 1 })
    }

    fn set_default(&mut self, id: &str) -> io::Result<()> {
        let (card, dev) = Self::parse(id)?;
        let path = asoundrc().ok_or_else(|| io::Error::other("no HOME"))?;
        if let Ok(old) = fs::read_to_string(&path)
            && !old.starts_with(MARK)
        {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} is your own; not changed (add defaults.pcm.card {card} and defaults.pcm.device {dev} yourself)", path.display())));
        }
        fs::write(&path, format!("{MARK}\ndefaults.pcm.card {card}\ndefaults.pcm.device {dev}\ndefaults.ctl.card {card}\n"))
    }

    fn tone(&mut self, id: &str, channels: u32, on: &[u32]) -> io::Result<String> {
        let (card, dev) = Self::parse(id)?;
        let f = OpenOptions::new().write(true).open(format!("/dev/snd/pcmC{card}D{dev}p"))?;
        let mut p = vec![0u8; HW_PARAMS_SIZE];
        // Everything open, then ours fixed: interleaved 16-bit, the channels and rate.
        for m in 0..3 {
            for w in 0..8 {
                put_u32(&mut p, MASKS_AT + m * 32 + w * 4, u32::MAX);
            }
        }
        for i in PARAM_FIRST_INTERVAL..=PARAM_LAST_INTERVAL {
            let at = INTERVALS_AT + (i - PARAM_FIRST_INTERVAL) * 12;
            put_u32(&mut p, at, 0);
            put_u32(&mut p, at + 4, u32::MAX);
            put_u32(&mut p, at + 8, 0);
        }
        let only = |p: &mut [u8], mask: usize, bit: usize| {
            for w in 0..8 {
                put_u32(p, MASKS_AT + mask * 32 + w * 4, if w == bit / 32 { 1 << (bit % 32) } else { 0 });
            }
        };
        only(&mut p, PARAM_ACCESS, ACCESS_RW_INTERLEAVED);
        only(&mut p, PARAM_FORMAT, FORMAT_S16_LE);
        only(&mut p, PARAM_SUBFORMAT, SUBFORMAT_STD);
        let exactly = |p: &mut [u8], param: usize, v: u32| {
            let at = INTERVALS_AT + (param - PARAM_FIRST_INTERVAL) * 12;
            put_u32(p, at, v);
            put_u32(p, at + 4, v);
            put_u32(p, at + 8, 1 << 2); // integer
        };
        exactly(&mut p, PARAM_CHANNELS, channels);
        exactly(&mut p, PARAM_RATE, RATE);
        put_u32(&mut p, RMASK_AT, u32::MAX);
        call(&f, HW_PARAMS, &mut p).map_err(|e| io::Error::new(e.kind(), format!("{id} would not take {channels} channels of 16-bit at {RATE} Hz: {e}")))?;
        call(&f, PREPARE, &mut [])?;
        let samples = crate::tone_samples(channels, on, RATE);
        let frames = samples.len() / channels as usize;
        let mut done = 0usize;
        while done < frames {
            // struct snd_xferi { long result; void *buf; unsigned long frames; }
            let mut x = [0u8; 24];
            let buf = samples[done * channels as usize..].as_ptr() as u64;
            x[8..16].copy_from_slice(&buf.to_ne_bytes());
            x[16..24].copy_from_slice(&((frames - done) as u64).to_ne_bytes());
            call(&f, WRITEI_FRAMES, &mut x)?;
            let wrote = get_i64(&x, 0);
            if wrote <= 0 {
                return Err(io::Error::other("the device took nothing"));
            }
            done += wrote as usize;
        }
        call(&f, DRAIN, &mut [])?;
        Ok(format!("440 Hz for 1.5 s on {id}, {channels} channels at {RATE} Hz, heard on {on:?}"))
    }
}
