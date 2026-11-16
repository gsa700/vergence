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

/// Open VT `n` for reading and writing, never as the opener's controlling
/// terminal (O_NOCTTY): vdm runs as a session leader under systemd, and a
/// plain open made the login screen's VT vdm's own terminal, so the login
/// screen could not take it (Techbench, 2026-11-16: EPERM from TIOCSCTTY).
pub fn open(n: u32) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    #[cfg(target_os = "linux")]
    const O_NOCTTY: i32 = 0o400;
    #[cfg(not(target_os = "linux"))]
    const O_NOCTTY: i32 = 0x8000;
    std::fs::OpenOptions::new().read(true).write(true).custom_flags(O_NOCTTY).open(tty_path(n))
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

/// The VT on the screen now (to go back to it when the login screen stops).
/// Linux: VT_GETSTATE 0x5603 on /dev/tty0 (struct vt_stat: v_active first,
/// a u16); FreeBSD: VT_GETACTIVE _IOR('v', 7, int) on /dev/ttyv0.
pub fn active() -> Option<u32> {
    use std::os::fd::AsRawFd;
    #[cfg(target_os = "linux")]
    {
        let f = std::fs::File::open("/dev/tty0").ok()?;
        let mut st = [0u16; 3];
        // SAFETY: a struct vt_stat (three u16) we own.
        (unsafe { ioctl(f.as_raw_fd(), 0x5603, st.as_mut_ptr()) } == 0).then_some(st[0] as u32)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let f = std::fs::File::open("/dev/ttyv0").ok()?;
        let mut n: c_int = 0;
        // SAFETY: an int we own.
        (unsafe { ioctl(f.as_raw_fd(), 0x4004_7607, &mut n as *mut c_int) } == 0).then_some(n as u32)
    }
}

/// Put VT `n` on the screen, from any process with a VT open (vdm).
pub fn switch_to(n: u32) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let f = open(n)?;
    activate(f.as_raw_fd(), n)
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
