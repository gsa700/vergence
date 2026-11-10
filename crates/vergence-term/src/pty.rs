// The shell's pseudo-terminal, straight from the operating system: open a
// master, start the shell on its slave as a new session with the slave as its
// controlling terminal, read and write the master, set the window size.

use std::{
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    os::unix::fs::OpenOptionsExt,
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
};

unsafe extern "C" {
    fn posix_openpt(flags: i32) -> i32;
    fn grantpt(fd: i32) -> i32;
    fn unlockpt(fd: i32) -> i32;
    fn ptsname(fd: i32) -> *const std::ffi::c_char;
    fn setsid() -> i32;
    fn ioctl(fd: i32, request: std::ffi::c_ulong, ...) -> i32;
}

const O_RDWR: i32 = 2;
// Close-on-exec: the terminal's end of a pty must not be handed down to the
// programs it starts. Without it every shell held its own terminal's master
// (and those of the tabs opened before it), so a killed terminal's programs
// never saw the hangup and ran on (found in the stress test, 2026-11-09).
#[cfg(target_os = "freebsd")]
const O_CLOEXEC: i32 = 0x0010_0000;
#[cfg(not(target_os = "freebsd"))]
const O_CLOEXEC: i32 = 0o2_000_000;
#[cfg(target_os = "freebsd")]
const O_NOCTTY: i32 = 0x8000;
#[cfg(not(target_os = "freebsd"))]
const O_NOCTTY: i32 = 0o400;
#[cfg(target_os = "freebsd")]
const TIOCSCTTY: std::ffi::c_ulong = 0x2000_7461; // _IO('t', 97)
#[cfg(not(target_os = "freebsd"))]
const TIOCSCTTY: std::ffi::c_ulong = 0x540e;
#[cfg(target_os = "freebsd")]
const TIOCSWINSZ: std::ffi::c_ulong = 0x8008_7467; // _IOW('t', 103, struct winsize)
#[cfg(not(target_os = "freebsd"))]
const TIOCSWINSZ: std::ffi::c_ulong = 0x5414;

#[repr(C)]
struct Winsize {
    rows: u16,
    cols: u16,
    xpixel: u16,
    ypixel: u16,
}

pub struct Pty {
    master: File,
    child: Child,
}

impl Pty {
    /// Start `shell` (a login-style interactive shell) on a new terminal of
    /// cols x rows.
    pub fn spawn(shell: &str, cols: u16, rows: u16) -> std::io::Result<Pty> {
        let fd = unsafe { posix_openpt(O_RDWR | O_NOCTTY | O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let master = unsafe { File::from_raw_fd(fd) };
        if unsafe { grantpt(fd) } != 0 || unsafe { unlockpt(fd) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // ptsname answers in one buffer shared by the whole process, so two
        // threads asking at once could read each other's name, or half of
        // one: a shell then started on the wrong terminal, or the open below
        // failed on "/dev/pts/" (seen on FreeBSD, 2026-11-09). One at a time,
        // and the name is copied before the next may ask.
        static NAMING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let name = {
            let _one = NAMING.lock().unwrap_or_else(|e| e.into_inner());
            let name = unsafe { ptsname(fd) };
            if name.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy().into_owned()
        };
        // O_NOCTTY: if we are a session leader with no terminal (started by a
        // launcher that called setsid), opening the slave would make it OUR
        // controlling terminal, and the shell's TIOCSCTTY would then fail (EPERM).
        let slave = std::fs::OpenOptions::new().read(true).write(true).custom_flags(O_NOCTTY).open(&name)?;
        set_size(fd, cols, rows);
        let mut cmd = Command::new(shell);
        cmd.stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave))
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env("TERM_PROGRAM", "vergence-term");
        unsafe {
            cmd.pre_exec(|| {
                // A new session, with the terminal (now fd 0) as its controlling terminal.
                if setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if ioctl(0, TIOCSCTTY, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        Ok(Pty { master, child })
    }

    /// A handle for reading the shell's output on another thread.
    pub fn reader(&self) -> std::io::Result<File> {
        self.master.try_clone()
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.master.write_all(bytes);
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        set_size(self.master.as_raw_fd(), cols, rows);
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn set_size(fd: i32, cols: u16, rows: u16) {
    let ws = Winsize { rows, cols, xpixel: 0, ypixel: 0 };
    unsafe { ioctl(fd, TIOCSWINSZ, &ws as *const Winsize) };
}

/// Read the shell's output until it ends, handing each chunk to `f`.
pub fn read_loop(mut r: File, mut f: impl FnMut(Vec<u8>) -> bool) {
    let mut buf = vec![0u8; 65536];
    loop {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if !f(buf[..n].to_vec()) {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// A shell started in a terminal has its own three descriptors and no
    /// others: not its terminal's master, and not an earlier terminal's.
    #[test]
    fn a_shell_inherits_no_terminal_masters() {
        let _earlier = super::Pty::spawn("/bin/sh", 80, 24).unwrap();
        let mut p = super::Pty::spawn("/bin/sh", 80, 24).unwrap();
        let r = p.reader().unwrap();
        // Each descriptor from 3 to 12 that can be written to is reported.
        p.write(b"for fd in 3 4 5 6 7 8 9 10 11 12; do if (: >&$fd) 2>/dev/null; then echo held-$fd; fi; done; echo checked-$((40+2)); exit\n");
        let mut all = Vec::new();
        super::read_loop(r, |b| {
            all.extend(b);
            true
        });
        let text = String::from_utf8_lossy(&all);
        assert!(text.contains("checked-42"), "{text}");
        let held: Vec<&str> = text.lines().filter(|l| l.starts_with("held-")).collect();
        assert!(held.is_empty(), "descriptors handed down to the shell: {held:?}");
    }

    #[test]
    fn a_shell_echoes_through_the_terminal() {
        let mut p = super::Pty::spawn("/bin/sh", 80, 24).unwrap();
        let r = p.reader().unwrap();
        p.write(b"echo vergence-$((40+2)); exit\n");
        let mut all = Vec::new();
        super::read_loop(r, |b| {
            all.extend(b);
            true
        });
        let text = String::from_utf8_lossy(&all);
        assert!(text.contains("vergence-42"), "{text}");
    }
}
