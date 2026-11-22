//! The power key while a Vergence session runs (menus milestone 1, step 3;
//! Dave, 2026-11-16): it opens the session dialog (Log Out, Sleep, Restart,
//! Shut Down) instead of the system shutting down at once. vdm, the one
//! part that runs as root and knows when a session is up, holds the key
//! from the system's own handling while one is, and lets it go when none is
//! (the login screen and the bare console keep the system's way):
//!
//! - logind (systemd's, or elogind): a "handle-power-key" inhibitor, held by
//!   a `systemd-inhibit` (or `elogind-inhibit`) child that ends when vdm
//!   closes its input; the server sees the key itself (XF86PowerOff);
//! - acpid (Void without elogind): its handler shuts down whatever is
//!   running, so install.sh puts a test of [`FLAG`] before its shutdown,
//!   and vdm makes the flag while a session is up; the server sees the key;
//! - FreeBSD: the kernel acts on the button (`hw.acpi.power_button_state`);
//!   vdm sets it to NONE, keeping the old state in [`kept_file`] so a vdm
//!   that died holding it puts it back when it starts again, and tells the
//!   sessions of the button devd reports ([`is_power_button`]), since the
//!   server never sees it as a key.

use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};

/// While it is there, Void's acpid handler leaves the power button to the
/// session (install.sh's guard).
#[cfg(target_os = "linux")]
pub const FLAG: &str = "/run/vdm/power-key";

/// The power button's state before vdm took it (FreeBSD).
pub fn kept_file() -> PathBuf {
    std::path::Path::new(crate::paths::STATE_DIR).join("power-button-state")
}

/// The key, held from the system's own handling.
pub struct Hold {
    inhibitor: Option<(Child, ChildStdin)>,
    #[cfg(target_os = "freebsd")]
    was: Option<String>,
}

/// The inhibitor program, where there is one.
#[cfg(target_os = "linux")]
fn inhibit_program() -> Option<&'static str> {
    ["/usr/bin/systemd-inhibit", "/bin/systemd-inhibit", "/usr/bin/elogind-inhibit", "/bin/elogind-inhibit"]
        .into_iter()
        .find(|p| std::path::Path::new(p).is_file())
}

#[cfg(target_os = "freebsd")]
fn sysctl_get() -> Option<String> {
    let out = Command::new("/sbin/sysctl").args(["-n", "hw.acpi.power_button_state"]).output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !v.is_empty()).then_some(v)
}

#[cfg(target_os = "freebsd")]
fn sysctl_set(v: &str) -> bool {
    Command::new("/sbin/sysctl")
        .arg(format!("hw.acpi.power_button_state={v}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A state of the button worth putting back: a sleep state, not NONE.
#[cfg(target_os = "freebsd")]
fn restorable(v: &str) -> bool {
    matches!(v, "S1" | "S2" | "S3" | "S4" | "S5")
}

/// At vdm's start: a power button a vdm before it held and never let go
/// (it died) is the system's again.
pub fn recover() {
    #[cfg(target_os = "freebsd")]
    if let Ok(v) = std::fs::read_to_string(kept_file()) {
        let v = v.trim();
        if restorable(v) {
            sysctl_set(v);
        }
        let _ = std::fs::remove_file(kept_file());
    }
    #[cfg(target_os = "linux")]
    let _ = std::fs::remove_file(FLAG);
}

/// Hold the key from the system: what was done, and what to undo (`release`).
pub fn hold() -> (Hold, String) {
    let mut done = Vec::new();
    #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
    let mut inhibitor = None;
    #[cfg(target_os = "linux")]
    {
        if let Some(p) = inhibit_program() {
            let child = Command::new(p)
                .args([
                    "--what=handle-power-key",
                    "--who=Vergence",
                    "--why=The power key asks first (Log Out, Sleep, Restart, Shut Down)",
                    "--mode=block",
                    "cat",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            match child {
                Ok(mut c) => match c.stdin.take() {
                    Some(input) => {
                        inhibitor = Some((c, input));
                        done.push("logind's power key inhibited".to_string());
                    }
                    None => {
                        let _ = c.kill();
                    }
                },
                Err(e) => done.push(format!("{p}: {e}")),
            }
        }
        if std::fs::write(FLAG, b"").is_ok() {
            done.push(format!("{FLAG} made"));
        }
    }
    #[cfg(target_os = "freebsd")]
    let was = match sysctl_get() {
        Some(v) if restorable(&v) => {
            let _ = std::fs::write(kept_file(), &v);
            if sysctl_set("NONE") {
                done.push(format!("hw.acpi.power_button_state {v} -> NONE"));
                Some(v)
            } else {
                let _ = std::fs::remove_file(kept_file());
                done.push("hw.acpi.power_button_state could not be set".into());
                None
            }
        }
        _ => None,
    };
    (
        Hold {
            inhibitor,
            #[cfg(target_os = "freebsd")]
            was,
        },
        done.join("; "),
    )
}

/// Give the key back to the system.
pub fn release(h: Hold) {
    if let Some((mut child, input)) = h.inhibitor {
        // Its input closed, `cat` ends and the inhibitor with it.
        drop(input);
        let _ = child.kill();
        let _ = child.wait();
    }
    #[cfg(target_os = "linux")]
    let _ = std::fs::remove_file(FLAG);
    #[cfg(target_os = "freebsd")]
    if let Some(v) = h.was {
        sysctl_set(&v);
        let _ = std::fs::remove_file(kept_file());
    }
}

/// Whether a line from devd's pipe is the power button pressed:
/// `!system=ACPI subsystem=Button type=\_SB_.PWRB notify=0x00`.
pub fn is_power_button(line: &str) -> bool {
    let words: Vec<&str> = line.trim_start_matches('!').split_whitespace().collect();
    words.contains(&"system=ACPI") && words.contains(&"subsystem=Button") && words.contains(&"notify=0x00")
}

/// devd's pipe, where it has one.
pub const DEVD_PIPE: &str = "/var/run/devd.pipe";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devd_says_the_power_button() {
        assert!(is_power_button("!system=ACPI subsystem=Button type=\\_SB_.PWRB notify=0x00"));
        assert!(!is_power_button("!system=ACPI subsystem=Button type=\\_SB_.SLPB notify=0x01"), "the sleep button");
        assert!(!is_power_button("!system=ACPI subsystem=ACAD type=\\_SB_.AC notify=0x00"));
        assert!(!is_power_button("+uhid0 at bus=0"));
    }
}
