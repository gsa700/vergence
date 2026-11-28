//! The screens' brightness (brightness milestone 1, docs/brightness-milestone-1.md):
//! which screens have a control, how bright each is, and vdm setting one.
//!
//! A built-in panel's backlight is read by anyone (Linux sysfs, FreeBSD
//! backlight(9)) but only root may change it, and there is no logind on
//! Vergence's systems to ask, so vdm does it as a named request on its power
//! socket, "brightness <connector> <percent>", for someone with a session at
//! the machine (as Sleep; Dave, 2026-11-27, question 2 a). vdm finds the
//! control again from the connector's name itself: the asker names a screen,
//! never a path.
//!
//! Levels are percents, 0 to 100, mapped onto the device's own range; a
//! backlight is never set to 0 (a black panel looks broken): 0 is the
//! lowest step, [`FLOOR`] percent of the device's range.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The lowest a backlight goes, in percent of its range.
pub const FLOOR: u32 = 1;

/// A screen's brightness control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    /// The screen's connector as the display server names its output ("eDP-1").
    pub connector: String,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A panel's backlight: Linux, its sysfs directory (`brightness`,
    /// `max_brightness`); FreeBSD, its /dev/backlight node.
    Backlight { path: PathBuf, max: u32 },
}

/// The connector names a screen may have: letters, digits and dashes, short.
pub fn valid_connector(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A connector of a built-in panel (where a backlight belongs).
pub fn is_internal(connector: &str) -> bool {
    ["eDP", "LVDS", "DSI"].iter().any(|p| connector.starts_with(p))
}

/// The screens with a brightness control, found under `root` (`/` on a
/// machine; a stand-in directory in tests).
pub fn controls_in(root: &Path) -> Vec<Control> {
    #[cfg(target_os = "linux")]
    {
        linux::controls(root)
    }
    #[cfg(target_os = "freebsd")]
    {
        freebsd::controls(root)
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        let _ = root;
        Vec::new()
    }
}

/// The machine's controls.
pub fn controls() -> Vec<Control> {
    controls_in(Path::new("/"))
}

/// How bright a screen is now, in percent.
pub fn level(c: &Control) -> Option<u32> {
    match &c.kind {
        Kind::Backlight { path, max } => backlight_raw(path).map(|raw| to_percent(raw, *max)),
    }
}

/// Set a screen's brightness directly (vdm, as root; tests on a stand-in).
pub fn set(c: &Control, percent: u32) -> Result<(), String> {
    match &c.kind {
        Kind::Backlight { path, max } => set_backlight_raw(path, to_raw(percent, *max)),
    }
}

/// vdm's half: the request's words checked, the control found again from
/// the connector's name, and set. The answer for the asker.
pub fn set_named(connector: &str, percent: &str) -> Result<(), String> {
    if !valid_connector(connector) {
        return Err("not a screen".into());
    }
    let percent: u32 = percent.parse().ok().filter(|p| *p <= 100).ok_or("not a level from 0 to 100")?;
    if let Some(c) = controls().into_iter().find(|c| c.connector == connector) {
        return set(&c, percent);
    }
    // An external screen: its monitor over DDC/CI, in the monitor's own range
    // (a monitor at 0 is dim, not dark: no floor).
    let (_, bus) = externals_in(Path::new("/"))
        .into_iter()
        .find(|(c, _)| c == connector)
        .ok_or("that screen has no brightness control")?;
    let (_, max) = crate::ddc::get_luminance(&bus)?;
    crate::ddc::set_luminance(&bus, ((percent * max as u32 + 50) / 100) as u16)
}

/// vdm's half of `brightness-list`: each external screen whose monitor
/// answers DDC/CI, and its level in percent ("DP-1=40 HDMI-A-1=75").
pub fn list_externals() -> String {
    externals_in(Path::new("/"))
        .into_iter()
        .filter_map(|(c, bus)| {
            crate::ddc::get_luminance(&bus)
                .ok()
                .map(|(now, max)| format!("{c}={}", (now as u32 * 100 + max as u32 / 2) / max as u32))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The external screens with a monitor on them and an I²C line for DDC/CI:
/// (connector, `/dev/i2c-N`). Whether the monitor answers is known only by
/// asking it (vdm, as root).
pub fn externals_in(root: &Path) -> Vec<(String, PathBuf)> {
    let mut entries: Vec<String> = std::fs::read_dir(root.join("sys/class/drm"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    entries
        .into_iter()
        .filter_map(|name| {
            let (card, conn) = name.split_once('-')?;
            if !card.starts_with("card") || !valid_connector(conn) || is_internal(conn) {
                return None;
            }
            crate::ddc::bus_of(root, &name).map(|bus| (conn.to_string(), bus))
        })
        .collect()
}

/// One line to vdm on its power socket, one line back.
fn ask_vdm(line: &str, wait: Duration) -> Result<String, String> {
    let mut s = UnixStream::connect(crate::paths::POWER_SOCKET).map_err(|e| format!("vdm is not running ({e})"))?;
    let _ = s.set_read_timeout(Some(wait));
    s.write_all(format!("{line}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut answer = String::new();
    BufReader::new(&s).read_line(&mut answer).map_err(|e| e.to_string())?;
    let answer = answer.trim();
    match answer.strip_prefix("ok") {
        Some(rest) => Ok(rest.trim().to_string()),
        None => Err(answer.strip_prefix("no: ").unwrap_or(answer).to_string()),
    }
}

/// Ask vdm to set a screen's brightness (the server, the settings app).
pub fn ask(connector: &str, percent: u32) -> Result<(), String> {
    ask_vdm(&format!("brightness {connector} {}", percent.min(100)), Duration::from_secs(5)).map(|_| ())
}

/// Ask vdm for the external screens' levels (DDC/CI: slow, a second or more
/// with several monitors; never during a frame).
pub fn ask_externals() -> Result<Vec<(String, u32)>, String> {
    let text = ask_vdm("brightness-list", Duration::from_secs(10))?;
    Ok(text
        .split_whitespace()
        .filter_map(|w| {
            let (c, l) = w.split_once('=')?;
            Some((c.to_string(), l.parse::<u32>().ok()?.min(100))).filter(|(c, _)| valid_connector(c))
        })
        .collect())
}

/// A device level as a percent of its range, the floor counted as 0.
pub fn to_percent(raw: u32, max: u32) -> u32 {
    if max == 0 {
        return 0;
    }
    let floor = floor_raw(max);
    if raw <= floor {
        return 0;
    }
    (((raw - floor) as u64 * 100 + (max - floor) as u64 / 2) / (max - floor).max(1) as u64).min(100) as u32
}

/// A percent as a device level, never below the floor.
pub fn to_raw(percent: u32, max: u32) -> u32 {
    let floor = floor_raw(max);
    let p = percent.min(100) as u64;
    (floor as u64 + ((max - floor) as u64 * p + 50) / 100) as u32
}

fn floor_raw(max: u32) -> u32 {
    ((max as u64 * FLOOR as u64).div_ceil(100) as u32).max(1).min(max)
}

fn backlight_raw(path: &Path) -> Option<u32> {
    #[cfg(target_os = "freebsd")]
    {
        freebsd::get(path)
    }
    #[cfg(not(target_os = "freebsd"))]
    {
        std::fs::read_to_string(path.join("brightness")).ok()?.trim().parse().ok()
    }
}

fn set_backlight_raw(path: &Path, raw: u32) -> Result<(), String> {
    #[cfg(target_os = "freebsd")]
    {
        freebsd::set(path, raw)
    }
    #[cfg(not(target_os = "freebsd"))]
    {
        std::fs::write(path.join("brightness"), format!("{raw}\n")).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    /// A backlight's own kind: the firmware's before the platform's before
    /// the GPU driver's raw register (where more than one drives one panel).
    fn rank(dir: &Path) -> u8 {
        match std::fs::read_to_string(dir.join("type")).unwrap_or_default().trim() {
            "firmware" => 0,
            "platform" => 1,
            _ => 2,
        }
    }

    fn max_of(dir: &Path) -> Option<u32> {
        std::fs::read_to_string(dir.join("max_brightness")).ok()?.trim().parse().ok().filter(|m| *m > 0)
    }

    pub fn controls(root: &Path) -> Vec<Control> {
        let drm = root.join("sys/class/drm");
        let mut out: Vec<Control> = Vec::new();
        let mut internal: Vec<String> = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(&drm).into_iter().flatten().flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            // card1-eDP-1: the connector's name after the card's.
            let Some((card, conn)) = name.split_once('-') else { continue };
            if !card.starts_with("card") || !valid_connector(conn) {
                continue;
            }
            if is_internal(conn) {
                internal.push(conn.to_string());
            }
            // The panel's backlight under its own connector (i915, amdgpu, xe).
            let mut found: Vec<PathBuf> = std::fs::read_dir(e.path())
                .into_iter()
                .flatten()
                .flatten()
                .map(|d| d.path())
                .filter(|d| max_of(d).is_some() && d.join("brightness").is_file())
                .collect();
            found.sort_by_key(|d| rank(d));
            if let Some(path) = found.into_iter().next() {
                let max = max_of(&path).unwrap_or(1);
                out.push(Control { connector: conn.to_string(), kind: Kind::Backlight { path, max } });
            }
        }
        // A backlight that is not under a connector (ACPI's, a vendor's), for
        // the one built-in panel when there is exactly one and it has none yet.
        if internal.len() == 1 && !out.iter().any(|c| c.connector == internal[0]) {
            let mut lights: Vec<PathBuf> = std::fs::read_dir(root.join("sys/class/backlight"))
                .into_iter()
                .flatten()
                .flatten()
                .map(|d| d.path())
                .filter(|d| max_of(d).is_some())
                .collect();
            lights.sort();
            lights.sort_by_key(|d| rank(d));
            if let Some(path) = lights.into_iter().next() {
                let max = max_of(&path).unwrap_or(1);
                out.push(Control { connector: internal[0].clone(), kind: Kind::Backlight { path, max } });
            }
        }
        out
    }
}

/// FreeBSD's backlight(9): /dev/backlight/backlightN, levels 0 to 100,
/// read and set with ioctls. Written to its header (sys/backlight.h);
/// not yet tried on a FreeBSD laptop (the lab has none, 2026-11-27).
#[cfg(target_os = "freebsd")]
mod freebsd {
    use super::*;
    use std::ffi::{c_int, c_ulong};
    use std::os::fd::AsRawFd;

    #[repr(C)]
    struct Props {
        brightness: u32,
        nlevels: u32,
        levels: [u32; 100],
    }

    // _IOWR('G', 0 | 1, struct backlight_props): 0xC0000000 | 408 << 16 | 'G' << 8 | n.
    const GET: c_ulong = 0xC198_4700;
    const UPDATE: c_ulong = 0xC198_4701;

    unsafe extern "C" {
        fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    }

    fn props(path: &Path, write: bool) -> Result<(std::fs::File, Props), String> {
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut p = Props { brightness: 0, nlevels: 0, levels: [0; 100] };
        // SAFETY: a valid descriptor and a Props of the header's layout and size.
        if unsafe { ioctl(f.as_raw_fd(), GET, &mut p as *mut Props) } != 0 {
            return Err(format!("{}: {}", path.display(), std::io::Error::last_os_error()));
        }
        Ok((f, p))
    }

    pub fn get(path: &Path) -> Option<u32> {
        props(path, false).ok().map(|(_, p)| p.brightness)
    }

    pub fn set(path: &Path, raw: u32) -> Result<(), String> {
        let (f, mut p) = props(path, true)?;
        p.brightness = raw.min(100);
        // SAFETY: as in props.
        if unsafe { ioctl(f.as_raw_fd(), UPDATE, &mut p as *mut Props) } != 0 {
            return Err(format!("{}: {}", path.display(), std::io::Error::last_os_error()));
        }
        Ok(())
    }

    /// The first backlight, for the built-in panel: backlight(9) does not say
    /// which connector it lights, and a FreeBSD laptop has one panel.
    pub fn controls(root: &Path) -> Vec<Control> {
        let mut nodes: Vec<PathBuf> = std::fs::read_dir(root.join("dev/backlight"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|d| d.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("backlight")))
            .collect();
        nodes.sort();
        let Some(path) = nodes.into_iter().next() else { return Vec::new() };
        // The panel's connector, from drm-kmod's sysctl-free view: the server
        // names it; vdm and the server agree on "eDP-1" for the usual laptop.
        vec![Control { connector: "eDP-1".into(), kind: Kind::Backlight { path, max: 100 } }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percents_and_device_levels() {
        // Hubble's panel: 0 to 120000.
        assert_eq!(to_raw(0, 120000), 1200, "the floor, never black");
        assert_eq!(to_raw(100, 120000), 120000);
        assert_eq!(to_percent(120000, 120000), 100);
        assert_eq!(to_percent(1200, 120000), 0);
        assert_eq!(to_percent(0, 120000), 0);
        for p in [0, 1, 5, 37, 50, 99, 100] {
            assert_eq!(to_percent(to_raw(p, 120000), 120000), p, "{p}% there and back");
        }
        // A backlight of a few steps.
        assert_eq!(to_raw(0, 7), 1);
        assert_eq!(to_raw(100, 7), 7);
        assert_eq!(to_percent(to_raw(50, 7), 7), 50);
        assert_eq!(to_percent(5, 0), 0);
    }

    #[test]
    fn connector_names() {
        assert!(valid_connector("eDP-1"));
        assert!(valid_connector("HDMI-A-2"));
        assert!(!valid_connector(""));
        assert!(!valid_connector("../../etc"));
        assert!(!valid_connector("eDP 1"));
        assert!(!valid_connector(&"x".repeat(33)));
        assert!(is_internal("eDP-1") && is_internal("LVDS-1") && !is_internal("DP-1"));
    }

    #[cfg(target_os = "linux")]
    fn standin(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("vergence-brightness-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[cfg(target_os = "linux")]
    fn light(dir: &Path, kind: &str, max: u32, now: u32) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("type"), kind).unwrap();
        std::fs::write(dir.join("max_brightness"), format!("{max}\n")).unwrap();
        std::fs::write(dir.join("brightness"), format!("{now}\n")).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_panel_found_under_its_connector_and_set() {
        let root = standin("connector");
        light(&root.join("sys/class/drm/card1-eDP-1/intel_backlight"), "raw", 120000, 42352);
        std::fs::create_dir_all(root.join("sys/class/drm/card1-HDMI-A-1")).unwrap();
        std::fs::create_dir_all(root.join("sys/class/drm/card1")).unwrap();
        let c = controls_in(&root);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].connector, "eDP-1");
        assert_eq!(level(&c[0]), Some(35));
        set(&c[0], 80).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("sys/class/drm/card1-eDP-1/intel_backlight/brightness")).unwrap(),
            "96240\n"
        );
        assert_eq!(level(&c[0]), Some(80));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_backlight_elsewhere_lights_the_one_panel() {
        let root = standin("elsewhere");
        std::fs::create_dir_all(root.join("sys/class/drm/card0-eDP-1")).unwrap();
        light(&root.join("sys/class/backlight/acpi_video0"), "firmware", 15, 15);
        light(&root.join("sys/class/backlight/amdgpu_bl0"), "raw", 255, 128);
        let c = controls_in(&root);
        assert_eq!(c.len(), 1, "{c:?}");
        assert!(
            matches!(&c[0].kind, Kind::Backlight { path, max: 15 } if path.ends_with("acpi_video0")),
            "the firmware's first"
        );
        // Two panels: which backlight is whose is not known, so neither gets one.
        std::fs::create_dir_all(root.join("sys/class/drm/card0-eDP-2")).unwrap();
        assert!(controls_in(&root).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn no_panel_no_control() {
        let root = standin("none");
        std::fs::create_dir_all(root.join("sys/class/drm/card0-DP-1")).unwrap();
        light(&root.join("sys/class/backlight/stray"), "raw", 10, 5);
        assert!(controls_in(&root).is_empty(), "a backlight with no panel to light lights nothing");
        let _ = std::fs::remove_dir_all(&root);
    }
}
