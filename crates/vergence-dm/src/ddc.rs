//! DDC/CI, our own (brightness milestone 1, step 5; Dave 2026-11-27, 3 a):
//! an external monitor's brightness, its "Luminance" (VCP code 0x10), read
//! and set over the I²C line its DRM connector names as its own
//! (`/sys/class/drm/card0-DP-1/ddc -> i2c-9`, opened as `/dev/i2c-9`, which
//! needs the `i2c-dev` module and root: vdm does it).
//!
//! The protocol (VESA DDC/CI 1.1, MCCS): the monitor answers at I²C address
//! 0x37; the host writes `0x51, 0x80 | length, payload..., checksum` and the
//! monitor answers `0x6E, 0x80 | length, payload..., checksum`, each checksum
//! the XOR of the bytes before it with the destination's address (0x6E
//! writing, 0x50 reading). Get VCP feature is payload `0x01 code`, its reply
//! `0x02 result code type max_hi max_lo now_hi now_lo`; Set VCP feature is
//! `0x03 code value_hi value_lo`, with no reply. A monitor needs a pause
//! after each message (40 ms after a request before its reply, 50 ms after a
//! set), and some miss a message now and then: a read is tried three times.
//! Only code 0x10 is ever asked or set: nothing else of a monitor's is ours
//! to change.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The monitor's brightness, as MCCS numbers it.
pub const LUMINANCE: u8 = 0x10;
/// The monitor's I²C address for DDC/CI.
const ADDRESS: u16 = 0x37;
const HOST: u8 = 0x51;
const MONITOR: u8 = 0x6E;
const REPLY_AFTER: Duration = Duration::from_millis(40);
const AFTER_SET: Duration = Duration::from_millis(50);
const TRIES: usize = 3;

/// A Get VCP feature request for `code`, as written to the monitor.
pub fn get_request(code: u8) -> Vec<u8> {
    frame(&[0x01, code])
}

/// A Set VCP feature request.
pub fn set_request(code: u8, value: u16) -> Vec<u8> {
    frame(&[0x03, code, (value >> 8) as u8, value as u8])
}

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut f = vec![HOST, 0x80 | payload.len() as u8];
    f.extend_from_slice(payload);
    let check = f.iter().fold(MONITOR, |a, b| a ^ b);
    f.push(check);
    f
}

/// A Get VCP feature reply: (now, max) for `code`, or why it is not one.
pub fn parse_reply(code: u8, r: &[u8]) -> Result<(u16, u16), &'static str> {
    if r.len() < 11 {
        return Err("short");
    }
    if r[0] != MONITOR {
        return Err("not from the monitor");
    }
    let len = (r[1] & 0x7f) as usize;
    if r[1] & 0x80 == 0 || len != 8 || r.len() < 3 + len {
        return Err("not a feature reply");
    }
    let check = r[..2 + len].iter().fold(0x50, |a, b| a ^ b);
    if check != r[2 + len] {
        return Err("checksum");
    }
    if r[2] != 0x02 || r[4] != code {
        return Err("another reply");
    }
    if r[3] != 0 {
        return Err("the monitor does not have it");
    }
    let max = u16::from_be_bytes([r[6], r[7]]);
    let now = u16::from_be_bytes([r[8], r[9]]);
    if max == 0 {
        return Err("no range");
    }
    Ok((now.min(max), max))
}

/// The I²C line of a connector with a monitor on it: `/dev/i2c-N` from its
/// sysfs `ddc` link, under `root`.
pub fn bus_of(root: &Path, card_connector: &str) -> Option<PathBuf> {
    let dir = root.join("sys/class/drm").join(card_connector);
    let status = std::fs::read_to_string(dir.join("status")).unwrap_or_default();
    if status.trim() != "connected" {
        return None;
    }
    let link = std::fs::read_link(dir.join("ddc")).ok()?;
    let name = link.file_name()?.to_string_lossy().into_owned();
    let n = name.strip_prefix("i2c-")?;
    n.parse::<u32>().ok()?;
    Some(root.join("dev").join(name))
}

#[cfg(target_os = "linux")]
mod io {
    use super::*;
    use std::ffi::{c_int, c_ulong};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;

    const I2C_SLAVE: c_ulong = 0x0703;

    unsafe extern "C" {
        fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    }

    pub fn open(bus: &Path) -> Result<std::fs::File, String> {
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(bus)
            .map_err(|e| format!("{}: {e}", bus.display()))?;
        // SAFETY: a valid descriptor; I2C_SLAVE takes the address as its argument.
        if unsafe { ioctl(f.as_raw_fd(), I2C_SLAVE, ADDRESS as c_ulong) } != 0 {
            return Err(format!("{}: {}", bus.display(), std::io::Error::last_os_error()));
        }
        Ok(f)
    }

    pub fn get(bus: &Path, code: u8) -> Result<(u16, u16), String> {
        let mut f = open(bus)?;
        let mut last = String::new();
        for _ in 0..TRIES {
            if let Err(e) = f.write_all(&get_request(code)) {
                last = e.to_string();
                std::thread::sleep(REPLY_AFTER);
                continue;
            }
            std::thread::sleep(REPLY_AFTER);
            let mut r = [0u8; 11];
            match f.read_exact(&mut r) {
                Ok(()) => match parse_reply(code, &r) {
                    Ok(v) => return Ok(v),
                    Err(why) => last = why.to_string(),
                },
                Err(e) => last = e.to_string(),
            }
            std::thread::sleep(REPLY_AFTER);
        }
        Err(format!("{}: no DDC/CI answer ({last})", bus.display()))
    }

    pub fn set(bus: &Path, code: u8, value: u16) -> Result<(), String> {
        let mut f = open(bus)?;
        f.write_all(&set_request(code, value)).map_err(|e| format!("{}: {e}", bus.display()))?;
        std::thread::sleep(AFTER_SET);
        Ok(())
    }
}

/// A monitor's brightness now and its range, read over its I²C line.
pub fn get_luminance(bus: &Path) -> Result<(u16, u16), String> {
    #[cfg(target_os = "linux")]
    {
        io::get(bus, LUMINANCE)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = bus;
        Err("DDC/CI is Linux-only for now".into())
    }
}

/// Set a monitor's brightness, `value` in its own range.
pub fn set_luminance(bus: &Path, value: u16) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        io::set(bus, LUMINANCE, value)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (bus, value);
        Err("DDC/CI is Linux-only for now".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_as_the_standard_writes_them() {
        // Get VCP 0x10: 51 82 01 10 and the checksum 6E^51^82^01^10 = AC.
        assert_eq!(get_request(0x10), vec![0x51, 0x82, 0x01, 0x10, 0xAC]);
        // Set VCP 0x10 to 50: 51 84 03 10 00 32, checksum 6E^51^84^03^10^00^32 = 9A.
        assert_eq!(set_request(0x10, 50), vec![0x51, 0x84, 0x03, 0x10, 0x00, 0x32, 0x9A]);
    }

    fn reply(result: u8, code: u8, max: u16, now: u16) -> Vec<u8> {
        let mut r = vec![0x6E, 0x88, 0x02, result, code, 0x00];
        r.extend_from_slice(&max.to_be_bytes());
        r.extend_from_slice(&now.to_be_bytes());
        let check = r.iter().fold(0x50, |a, b| a ^ b);
        r.push(check);
        r
    }

    #[test]
    fn replies_read_and_checked() {
        assert_eq!(parse_reply(0x10, &reply(0, 0x10, 100, 40)), Ok((40, 100)));
        assert_eq!(parse_reply(0x10, &reply(0, 0x10, 255, 300)), Ok((255, 255)), "now never above max");
        assert_eq!(parse_reply(0x10, &reply(1, 0x10, 100, 40)), Err("the monitor does not have it"));
        assert_eq!(parse_reply(0x10, &reply(0, 0x12, 100, 40)), Err("another reply"));
        assert_eq!(parse_reply(0x10, &reply(0, 0x10, 0, 0)), Err("no range"));
        let mut bad = reply(0, 0x10, 100, 40);
        bad[9] ^= 1;
        assert_eq!(parse_reply(0x10, &bad), Err("checksum"));
        assert_eq!(parse_reply(0x10, &[0x6E, 0x80]), Err("short"));
        let mut other = reply(0, 0x10, 100, 40);
        other[0] = 0x6F;
        assert_eq!(parse_reply(0x10, &other), Err("not from the monitor"));
        // A null message (the monitor busy): 6E 80 BE.
        assert!(parse_reply(0x10, &[0x6E, 0x80, 0xBE, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_connectors_bus_from_its_ddc_link() {
        let root = std::env::temp_dir().join(format!("vergence-ddc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dp = root.join("sys/class/drm/card0-DP-1");
        std::fs::create_dir_all(&dp).unwrap();
        std::fs::write(dp.join("status"), "connected\n").unwrap();
        std::os::unix::fs::symlink("../../../i2c-9", dp.join("ddc")).unwrap();
        assert_eq!(bus_of(&root, "card0-DP-1"), Some(root.join("dev/i2c-9")));
        std::fs::write(dp.join("status"), "disconnected\n").unwrap();
        assert_eq!(bus_of(&root, "card0-DP-1"), None, "nothing on it");
        let _ = std::fs::remove_dir_all(&root);
    }
}
