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
    fn signal(signum: i32, handler: usize) -> usize;
    fn ioctl(fd: i32, request: std::ffi::c_ulong, ...) -> i32;
    fn tcgetpgrp(fd: i32) -> i32;
    fn tcgetattr(fd: i32, termios: *mut u8) -> i32;
}

/// struct termios's c_lflag (after c_iflag, c_oflag and c_cflag, each a
/// 32-bit tcflag_t on Linux and FreeBSD) and its ECHO bit (8 on both);
/// ICANON's differs: 2 on Linux's common layout, 0x100 on FreeBSD and on
/// Linux's powerpc (alpha too, which Rust does not build for). Any other system (macOS's tcflag_t is 64 bits)
/// must be added here, read from its headers: a wrong bit would read every
/// password prompt as none (the security review of 2026-11-22).
const LFLAG_AT: usize = 12;
const ECHO: u32 = 0o10;
#[cfg(all(target_os = "linux", not(any(target_arch = "powerpc", target_arch = "powerpc64"))))]
const ICANON: u32 = 0o2;
#[cfg(any(target_os = "freebsd", all(target_os = "linux", any(target_arch = "powerpc", target_arch = "powerpc64"))))]
const ICANON: u32 = 0x100;
#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
compile_error!("vergence-term: struct termios's layout is not known for this system (pty.rs)");

const O_RDWR: i32 = 2;
// Close-on-exec: the terminal's end of a pty must not be handed down to the
// programs it starts. Without it every shell held its own terminal's master
// (and those of the tabs opened before it), so a killed terminal's programs
// never saw the hangup and ran on (found in the stress test).
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
    /// Start `program` with `args` (the user's shell, or what `-e` named) on
    /// a new terminal of cols x rows.
    pub fn spawn(program: &str, args: &[String], cols: u16, rows: u16, cwd: Option<&str>) -> std::io::Result<Pty> {
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
        // failed on "/dev/pts/" (seen on FreeBSD). One at a time,
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
        let mut cmd = Command::new(program);
        // Where it starts: a restored tab's directory, if it is still there.
        if let Some(dir) = cwd.filter(|d| std::path::Path::new(d).is_dir()) {
            cmd.current_dir(dir);
        }
        cmd.args(args)
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave))
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env("TERM_PROGRAM", "vergence-term");
        unsafe {
            cmd.pre_exec(|| {
                // The signals a terminal's programs are ended and interrupted by are
                // theirs to take: if this terminal was started ignoring any of them
                // (under `nohup`, by a launcher), its programs are not to be born
                // ignoring them too. One that ignored the hangup would run on after
                // the terminal was gone (as under a server started with nohup).
                // Hangup, interrupt, quit, terminate;
                // 0 is the default action.
                for number in [1, 2, 3, 15] {
                    signal(number, 0);
                }
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

    /// The program's process id (its working directory is read by it, program.rs).
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The terminal's foreground process group: the program the user is
    /// typing to now (the shell itself, or what it started). Asked of the
    /// master (POSIX: Linux and FreeBSD answer there for the slave).
    pub fn foreground(&self) -> Option<u32> {
        let group = unsafe { tcgetpgrp(self.master.as_raw_fd()) };
        (group > 0).then_some(group as u32)
    }

    /// Whether a password is being asked (terminal agent milestone 1: an
    /// agent may not type then): input read a line at a time (canonical) and
    /// not echoed, as `sudo`, `ssh` and `read -s` read it. Echo off alone is
    /// not enough: a shell's line editor (zsh's zle, bash's readline,
    /// FreeBSD sh's libedit) turns it off while a command is typed and echoes
    /// itself, but reads a key at a time (not canonical). Asked of the
    /// master: Linux answers a terminal-mode request on a pty's master for its
    /// slave (tty_mode_ioctl uses the linked tty), and FreeBSD's pts takes it
    /// on either side. None: not known.
    pub fn asks_password(&self) -> Option<bool> {
        // Larger than either system's struct termios (Linux 60 bytes, FreeBSD 44).
        let mut t = [0u8; 128];
        // SAFETY: a buffer larger than the struct, and our own descriptor.
        if unsafe { tcgetattr(self.master.as_raw_fd(), t.as_mut_ptr()) } != 0 {
            return None;
        }
        let lflag = u32::from_ne_bytes([t[LFLAG_AT], t[LFLAG_AT + 1], t[LFLAG_AT + 2], t[LFLAG_AT + 3]]);
        Some(lflag & ECHO == 0 && lflag & ICANON != 0)
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
        let _earlier = super::Pty::spawn("/bin/sh", &[], 80, 24, None).unwrap();
        let mut p = super::Pty::spawn("/bin/sh", &[], 80, 24, None).unwrap();
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

    /// The password state, seen from the master (Linux and FreeBSD alike):
    /// a line read without echo while `sleep` holds the terminal after
    /// `stty -echo`, and not once echo is back. (A shell's line editor, while
    /// it waits for a command, reads keys without echo but not a line: not a
    /// password either.)
    #[test]
    fn a_password_prompt_is_seen_from_the_master() {
        let mut p = super::Pty::spawn("/bin/sh", &[], 80, 24, None).unwrap();
        let r = p.reader().unwrap();
        std::thread::spawn(move || super::read_loop(r, |_| true));
        let wait_for = |want: bool, p: &super::Pty| {
            for _ in 0..150 {
                if p.asks_password() == Some(want) {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            false
        };
        assert!(wait_for(false, &p), "a shell waiting for a command asks no password");
        p.write(b"stty -echo; sleep 2; stty echo\n");
        assert!(wait_for(true, &p), "a line without echo is a password's");
        assert!(wait_for(false, &p), "and not once echo is back");
        p.write(b"exit\n");
    }

    #[test]
    fn a_shell_echoes_through_the_terminal() {
        let mut p = super::Pty::spawn("/bin/sh", &[], 80, 24, None).unwrap();
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

    /// `-e`: a program with its arguments, no shell between, on the terminal.
    #[test]
    fn a_program_runs_with_its_arguments() {
        // The program waits a moment after writing. On FreeBSD, when the
        // session leader that has the pty as its controlling terminal exits,
        // the terminal is revoked and output its master has not read yet is
        // thrown away, even if another descriptor of the slave is open (Linux
        // keeps it). A program quicker than the reading thread (here, under a
        // loaded test run) lost all its output: the test failed now and then
        // on FreeBSD. Shells and real programs live longer than
        // their first output; a one-line `-e` command's window closes as it
        // exits anyway.
        let p = super::Pty::spawn(
            "/bin/sh",
            &["-c".into(), "echo ran-$((40+2)) \"$0\"; sleep 0.5".into(), "two words".into()],
            80,
            24,
            None,
        )
        .unwrap();
        let r = p.reader().unwrap();
        let mut all = Vec::new();
        super::read_loop(r, |b| {
            all.extend(b);
            true
        });
        let text = String::from_utf8_lossy(&all);
        assert!(text.contains("ran-42 two words"), "{text}");
    }

    /// When the terminal goes (its end of the pty is closed, as when the
    /// terminal's process is killed), a program on it is ended by the
    /// system's hangup even if it never reads or writes the terminal and so
    /// never meets an error of its own: nothing is left running behind a
    /// dead terminal.
    #[test]
    fn a_program_that_ignores_its_terminal_ends_with_it() {
        use std::os::unix::fs::PermissionsExt;
        let script = std::env::temp_dir().join(format!("vergence-term-quiet-{}.sh", std::process::id()));
        std::fs::write(&script, "#!/bin/sh\nwhile :; do sleep 1; done\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Started as under `nohup`, or from a launcher that ignores hangups: what a
        // process ignores, the programs it starts ignore too, unless told otherwise.
        const SIGHUP: i32 = 1;
        // SAFETY: the disposition of one signal, put back below; 1 is SIG_IGN.
        let before = unsafe { super::signal(SIGHUP, 1) };
        let p = super::Pty::spawn(script.to_str().unwrap(), &[], 80, 24, None);
        // SAFETY: as above.
        unsafe { super::signal(SIGHUP, before) };
        let p = p.unwrap();
        // The terminal's end is closed and nothing else is done: no kill, as when
        // the terminal itself is killed.
        let mut p = std::mem::ManuallyDrop::new(p);
        // SAFETY: the file is read out once and `p` is never dropped, so it is closed once.
        drop(unsafe { std::ptr::read(&p.master) });
        let started = std::time::Instant::now();
        let ended = loop {
            if let Some(status) = p.child.try_wait().unwrap() {
                break Some(status);
            }
            if started.elapsed() > std::time::Duration::from_secs(5) {
                break None;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let _ = std::fs::remove_file(&script);
        if ended.is_none() {
            let _ = p.child.kill();
            let _ = p.child.wait();
        }
        assert!(ended.is_some(), "the program was still running five seconds after its terminal was closed");
    }
}
