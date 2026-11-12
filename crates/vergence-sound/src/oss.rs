//! FreeBSD's OSS (sound(4), snd_hda): a `pcm` device per output group the
//! codec reports, each with a mixer, `/dev/mixerN` (mode 0666: volume and
//! mute need no privilege). The default device is the sysctl
//! `hw.snd.default_unit`, which only root may set: [`Oss::set_default`]
//! asks the root helper ([`helper_main`], `libexec/vergence/sound-default`)
//! when the system refuses us.
//!
//! What each device is: `dev.pcm.N.%desc` (the driver's words),
//! `dev.pcm.N.%location` (the codec pins it drives, "nid=33,26") and
//! `dev.pcm.N.%parent` (the codec, "hdaa0"); each pin's configuration,
//! `dev.hdaaM.nidK_config`, names it ([`crate::pin_name`]).
//! `dev.pcm.N.mode`: 1 mixer, 2 play, 4 record; only devices that play are
//! outputs.

use std::{
    ffi::{CString, OsString},
    fs::OpenOptions,
    io,
    os::fd::AsRawFd,
    path::PathBuf,
};

use crate::{Output, Sound, level_of, level_percent, location_pins, pin_name};

unsafe extern "C" {
    fn sysctlbyname(name: *const std::ffi::c_char, old: *mut u8, oldlen: *mut usize, new: *const u8, newlen: usize) -> i32;
    fn ioctl(fd: i32, request: std::ffi::c_ulong, arg: *mut i32) -> i32;
}

/// FreeBSD's ioctl numbers for the mixer: _IOR('M', n, int) and
/// _IOWR('M', n, int) (sys/ioccom.h: in 0x80000000, out 0x40000000, the
/// size in bits 16-28, the group in 8-15).
const fn mixer_read(n: u32) -> std::ffi::c_ulong {
    (0x4000_0000 | (4 << 16) | ((b'M' as u32) << 8) | n) as std::ffi::c_ulong
}
const fn mixer_write(n: u32) -> std::ffi::c_ulong {
    (0xC000_0000 | (4 << 16) | ((b'M' as u32) << 8) | n) as std::ffi::c_ulong
}
/// sys/soundcard.h: the master level, and the mask of muted controls.
const SOUND_MIXER_VOLUME: u32 = 0;
const SOUND_MIXER_MUTE: u32 = 28;
/// dev.pcm.N.mode's play bit.
const MODE_PLAY: i32 = 2;
/// Playback (sys/soundcard.h): _IOWR('P', n, int), and SNDCTL_DSP_SYNC _IO('P', 1).
const fn dsp(n: u32) -> std::ffi::c_ulong {
    (0xC000_0000 | (4 << 16) | ((b'P' as u32) << 8) | n) as std::ffi::c_ulong
}
const SNDCTL_DSP_SPEED: std::ffi::c_ulong = dsp(2);
const SNDCTL_DSP_SETFMT: std::ffi::c_ulong = dsp(5);
const SNDCTL_DSP_CHANNELS: std::ffi::c_ulong = dsp(6);
const SNDCTL_DSP_SYNC: std::ffi::c_ulong = (0x2000_0000 | ((b'P' as u32) << 8) | 1) as std::ffi::c_ulong;
const AFMT_S16_LE: i32 = 0x10;
const RATE: i32 = 48_000;

fn sysctl_bytes(name: &str) -> Option<Vec<u8>> {
    let c = CString::new(name).ok()?;
    let mut len = 0usize;
    // SAFETY: a size query (no buffer), then a buffer of that size.
    if unsafe { sysctlbyname(c.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null(), 0) } != 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    if unsafe { sysctlbyname(c.as_ptr(), buf.as_mut_ptr(), &mut len, std::ptr::null(), 0) } != 0 {
        return None;
    }
    buf.truncate(len);
    Some(buf)
}

fn sysctl_string(name: &str) -> Option<String> {
    let b = sysctl_bytes(name)?;
    Some(String::from_utf8_lossy(b.split(|c| *c == 0).next().unwrap_or(&[])).into_owned())
}

fn sysctl_int(name: &str) -> Option<i32> {
    let b = sysctl_bytes(name)?;
    (b.len() >= 4).then(|| i32::from_ne_bytes(b[..4].try_into().unwrap()))
}

fn set_sysctl_int(name: &str, value: i32) -> io::Result<()> {
    let c = CString::new(name).map_err(io::Error::other)?;
    let v = value.to_ne_bytes();
    // SAFETY: a write of four bytes from `v`, no old value asked for.
    if unsafe { sysctlbyname(c.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), v.as_ptr(), 4) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The root helper's name, in `PREFIX/libexec/vergence` (tools/install.sh
/// installs it setuid root, runnable by the `video` group only).
pub const HELPER: &str = "sound-default";

/// The helper: in the prefix our program runs from (`PREFIX/bin/..`), else
/// /usr/local's (a development build run from target/).
fn helper_path() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.parent()?.join("libexec/vergence").join(HELPER)));
    beside.into_iter().chain([PathBuf::from("/usr/local/libexec/vergence").join(HELPER)]).find(|p| p.is_file())
}

/// Ask the helper to make `unit` the default output.
fn run_helper(unit: u32) -> io::Result<()> {
    let refused = |why: &str| io::Error::new(io::ErrorKind::PermissionDenied, why.to_string());
    let Some(path) = helper_path() else {
        return Err(refused("the sound helper (libexec/vergence/sound-default) is not installed; sudo tools/install.sh puts it there"));
    };
    let out = match std::process::Command::new(&path).arg(unit.to_string()).env_clear().output() {
        Ok(o) => o,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(refused("only members of the video group may choose the output (sudo pw groupmod video -m $USER, then log in again)")),
        Err(e) => return Err(e),
    };
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr);
    Err(io::Error::other(said.trim().trim_start_matches("sound-default: ").to_string()))
}

/// The root helper's whole job (sound milestone 1, Decided 1): make the
/// output numbered in its one argument the default, and nothing else. It
/// runs setuid root, so it trusts nothing it is given: exactly one argument,
/// one or two decimal digits, naming a device that exists and plays; no
/// file is opened, no environment read, nothing else written. Returns the
/// exit status.
pub fn helper_main(args: Vec<OsString>) -> i32 {
    let fail = |why: String| {
        eprintln!("sound-default: {why}");
        1
    };
    let [arg] = args.as_slice() else {
        return fail("usage: sound-default UNIT (the N of pcmN)".into());
    };
    let Some(unit) = arg.to_str().filter(|a| (1..=2).contains(&a.len()) && a.bytes().all(|b| b.is_ascii_digit())).and_then(|a| a.parse::<u32>().ok()) else {
        return fail("the unit is a number, the N of pcmN".into());
    };
    if !Oss::units().contains(&unit) {
        return fail(format!("no output pcm{unit}"));
    }
    match set_sysctl_int("hw.snd.default_unit", unit as i32) {
        Ok(()) => 0,
        Err(e) => fail(format!("setting hw.snd.default_unit: {e}")),
    }
}

pub struct Oss;

impl Oss {
    /// OSS, if this machine has sound devices.
    pub fn open() -> Option<Oss> {
        sysctl_string("dev.pcm.0.%desc").map(|_| Oss)
    }

    /// The numbers of the devices that play.
    fn units() -> Vec<u32> {
        (0..64).filter(|n| sysctl_string(&format!("dev.pcm.{n}.%desc")).is_some()).filter(|n| sysctl_int(&format!("dev.pcm.{n}.mode")).is_some_and(|m| m & MODE_PLAY != 0)).collect()
    }

    fn unit(id: &str) -> io::Result<u32> {
        id.strip_prefix("pcm").and_then(|n| n.parse().ok()).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no output {id}")))
    }

    /// One of the device's mixer values, read or written through /dev/mixerN.
    fn mixer(unit: u32, request: std::ffi::c_ulong, value: i32) -> io::Result<i32> {
        let f = OpenOptions::new().read(true).write(true).open(format!("/dev/mixer{unit}"))?;
        let mut v = value;
        // SAFETY: the mixer ioctls read or write one int, `v`.
        if unsafe { ioctl(f.as_raw_fd(), request, &mut v) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(v)
    }

    /// The device's name from the first output pin it drives, else the driver's words.
    fn name(unit: u32, detail: &str) -> String {
        let parent = sysctl_string(&format!("dev.pcm.{unit}.%parent")).unwrap_or_default();
        let pins = location_pins(&sysctl_string(&format!("dev.pcm.{unit}.%location")).unwrap_or_default());
        let codec = parent.strip_prefix("hdaa").unwrap_or("");
        let from_pins = pins.iter().find_map(|nid| pin_name(&sysctl_string(&format!("dev.hdaa.{codec}.nid{nid}_config"))?));
        // A GPU's digital out is HDMI or DisplayPort.
        let hdmi = detail.contains("HDMI") || detail.contains("DP");
        match from_pins {
            Some(n) if hdmi && n.ends_with("igital out") => "HDMI or DisplayPort".into(),
            Some(n) => n,
            None if hdmi => "HDMI or DisplayPort".into(),
            None => detail.to_string(),
        }
    }
}

impl Sound for Oss {
    fn system(&self) -> &'static str {
        "OSS"
    }

    fn outputs(&mut self) -> io::Result<Vec<Output>> {
        let default = sysctl_int("hw.snd.default_unit").unwrap_or(-1);
        let mut out = Vec::new();
        for unit in Self::units() {
            let detail = sysctl_string(&format!("dev.pcm.{unit}.%desc")).unwrap_or_default();
            let volume = Self::mixer(unit, mixer_read(SOUND_MIXER_VOLUME), 0).map(level_percent).unwrap_or(0);
            let muted = Self::mixer(unit, mixer_read(SOUND_MIXER_MUTE), 0).is_ok_and(|mask| mask & (1 << SOUND_MIXER_VOLUME) != 0);
            out.push(Output { id: format!("pcm{unit}"), name: Self::name(unit, &detail), detail, volume, adjustable: true, muted, default: unit as i32 == default });
        }
        Ok(out)
    }

    fn set_volume(&mut self, id: &str, volume: u8) -> io::Result<()> {
        Self::mixer(Self::unit(id)?, mixer_write(SOUND_MIXER_VOLUME), level_of(volume)).map(|_| ())
    }

    fn set_muted(&mut self, id: &str, muted: bool) -> io::Result<()> {
        let unit = Self::unit(id)?;
        let mask = Self::mixer(unit, mixer_read(SOUND_MIXER_MUTE), 0)?;
        let bit = 1 << SOUND_MIXER_VOLUME;
        Self::mixer(unit, mixer_write(SOUND_MIXER_MUTE), if muted { mask | bit } else { mask & !bit }).map(|_| ())
    }

    fn set_default(&mut self, id: &str) -> io::Result<()> {
        let unit = Self::unit(id)?;
        if !Self::units().contains(&unit) {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("no output {id}")));
        }
        match set_sysctl_int("hw.snd.default_unit", unit as i32) {
            // Not root (the usual case): the helper does it.
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => run_helper(unit),
            r => r,
        }
    }

    fn tone(&mut self, id: &str, channels: u32, on: &[u32]) -> io::Result<String> {
        use std::io::Write;
        let unit = Self::unit(id)?;
        let mut f = OpenOptions::new().write(true).open(format!("/dev/dsp{unit}"))?;
        let set = |f: &std::fs::File, request: std::ffi::c_ulong, value: i32| -> io::Result<i32> {
            let mut v = value;
            // SAFETY: these ioctls read and write one int, `v`.
            if unsafe { ioctl(f.as_raw_fd(), request, &mut v) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(v)
        };
        if set(&f, SNDCTL_DSP_SETFMT, AFMT_S16_LE)? != AFMT_S16_LE {
            return Err(io::Error::other("the device takes no 16-bit samples"));
        }
        let got = set(&f, SNDCTL_DSP_CHANNELS, channels as i32)? as u32;
        let rate = set(&f, SNDCTL_DSP_SPEED, RATE)? as u32;
        let highest = on.iter().copied().max().unwrap_or(1);
        if got < highest {
            // The virtual channels mix in fewer: say how to reach the higher outputs.
            let mix = sysctl_string(&format!("dev.pcm.{unit}.play.vchanformat")).unwrap_or_default();
            return Err(io::Error::other(format!(
                "{id} takes {got} channels here (its virtual channels mix in {mix}); for channel {highest}: sudo sysctl dev.pcm.{unit}.play.vchanformat=s16le:{channels}.0"
            )));
        }
        let samples = crate::tone_samples(got, on, rate);
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        f.write_all(&bytes)?;
        // Wait until it has been heard.
        let _ = set(&f, SNDCTL_DSP_SYNC, 0);
        Ok(format!("440 Hz for 1.5 s on {id}, {got} channels at {rate} Hz, heard on {on:?}"))
    }
}
