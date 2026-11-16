//! Virtual terminals: which one the login screen takes, and switching to it.
//! Linux: /dev/ttyN, VT_ACTIVATE 0x5606 and VT_WAITACTIVE 0x5607 (linux/vt.h).
//! FreeBSD: /dev/ttyvX (X the hex number from 0, VT X+1), VT_ACTIVATE
//! _IO('v', 5) and VT_WAITACTIVE _IO('v', 6) (sys/consio.h), the number passed
//! by value as on Linux.

use std::ffi::{c_int, c_ulong};

unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

/// The login screen's VT: past the consoles the system gives logins
/// (Void's agetty-tty1..6; FreeBSD's ttyv0..7 in /etc/ttys).
#[cfg(target_os = "linux")]
pub const GREETER_VT: u32 = 7;
#[cfg(not(target_os = "linux"))]
pub const GREETER_VT: u32 = 9;

#[cfg(target_os = "linux")]
const VT_ACTIVATE: c_ulong = 0x5606;
#[cfg(target_os = "linux")]
const VT_WAITACTIVE: c_ulong = 0x5607;
#[cfg(target_os = "linux")]
const TIOCSCTTY: c_ulong = 0x540E;
#[cfg(not(target_os = "linux"))]
const VT_ACTIVATE: c_ulong = 0x2000_7605;
#[cfg(not(target_os = "linux"))]
const VT_WAITACTIVE: c_ulong = 0x2000_7606;
#[cfg(not(target_os = "linux"))]
const TIOCSCTTY: c_ulong = 0x2000_7461;

/// The device of VT `n` (from 1).
pub fn tty_path(n: u32) -> String {
    if cfg!(target_os = "linux") { format!("/dev/tty{n}") } else { format!("/dev/ttyv{:x}", n.saturating_sub(1)) }
}

/// Make VT `n` the one on the screen and wait until it is; `fd` is any open VT.
pub fn activate(fd: c_int, n: u32) -> std::io::Result<()> {
    // SAFETY: both take the number by value.
    if unsafe { ioctl(fd, VT_ACTIVATE, n as c_int) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: as above.
    if unsafe { ioctl(fd, VT_WAITACTIVE, n as c_int) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Make the terminal on `fd` the calling session leader's controlling terminal.
pub fn make_controlling(fd: c_int) -> std::io::Result<()> {
    // SAFETY: TIOCSCTTY takes an int (0: do not steal it from another session).
    if unsafe { ioctl(fd, TIOCSCTTY, 0 as c_int) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vt_devices() {
        if cfg!(target_os = "linux") {
            assert_eq!(tty_path(7), "/dev/tty7");
        } else {
            assert_eq!(tty_path(9), "/dev/ttyv8");
            assert_eq!(tty_path(12), "/dev/ttyvb");
        }
    }
}
