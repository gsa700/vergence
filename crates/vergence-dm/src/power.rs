//! Sleep, Restart and Shut Down (VDM step 6), carried out by vdm as root for
//! the login screen and for the people with a session at the machine. The
//! request is one line on the power socket ("sleep", "restart",
//! "shut-down"); the answer is one line, "ok" or "no: <why>".

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Power {
    Sleep,
    Restart,
    ShutDown,
}

impl Power {
    pub fn word(self) -> &'static str {
        match self {
            Power::Sleep => "sleep",
            Power::Restart => "restart",
            Power::ShutDown => "shut-down",
        }
    }

    pub fn from_word(w: &str) -> Option<Power> {
        match w {
            "sleep" => Some(Power::Sleep),
            "restart" => Some(Power::Restart),
            "shut-down" => Some(Power::ShutDown),
            _ => None,
        }
    }

    /// The system's own command for it: systemd's where it runs (Fedora);
    /// Void's zzz, reboot and poweroff; FreeBSD's zzz and shutdown.
    pub fn command(self) -> (&'static str, &'static [&'static str]) {
        let systemd = std::path::Path::new("/run/systemd/system").is_dir();
        match (self, systemd, cfg!(target_os = "linux")) {
            (Power::Sleep, true, _) => ("systemctl", &["suspend"]),
            (Power::Restart, true, _) => ("systemctl", &["reboot"]),
            (Power::ShutDown, true, _) => ("systemctl", &["poweroff"]),
            (Power::Sleep, false, _) => ("zzz", &[]),
            (Power::Restart, false, true) => ("reboot", &[]),
            (Power::ShutDown, false, true) => ("poweroff", &[]),
            (Power::Restart, false, false) => ("shutdown", &["-r", "now"]),
            (Power::ShutDown, false, false) => ("shutdown", &["-p", "now"]),
        }
    }
}

/// Ask the running vdm (from the login screen or a session); its answer.
pub fn ask(p: Power) -> Result<(), String> {
    let mut s = UnixStream::connect(crate::paths::POWER_SOCKET).map_err(|e| format!("vdm is not running ({e})"))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    s.write_all(format!("{}\n", p.word()).as_bytes()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line).map_err(|e| e.to_string())?;
    match line.trim() {
        "ok" => Ok(()),
        other => Err(other.strip_prefix("no: ").unwrap_or(other).to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_go_both_ways() {
        for p in [Power::Sleep, Power::Restart, Power::ShutDown] {
            assert_eq!(Power::from_word(p.word()), Some(p));
        }
        assert_eq!(Power::from_word("halt"), None);
    }
}
