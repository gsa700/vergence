// The shell's pseudo-terminal, straight from the operating system: open a
// master, start the shell on its slave as a new session with the slave as its
// controlling terminal, read and write the master, set the window size.

use std::{
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
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
        let fd = unsafe { posix_openpt(O_RDWR | O_NOCTTY) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let master = unsafe { File::from_raw_fd(fd) };
        if unsafe { grantpt(fd) } != 0 || unsafe { unlockpt(fd) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let name = unsafe { ptsname(fd) };
        if name.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let name = unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy().into_owned();
        let slave = std::fs::OpenOptions::new().read(true).write(true).open(&name)?;
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
