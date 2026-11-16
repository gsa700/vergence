//! The named actions an elevation prompt may approve (docs/elevation-
//! milestone-1.md): each a fixed thing vdm does with its own code, its
//! arguments checked here, and said in words for the prompt. Nothing a
//! program sends reaches a shell or names a path.

/// One action vdm may carry out after the prompt's yes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Automatic login for this person (VDM milestone 1's rules still apply).
    AutologinOn(String),
    AutologinOff,
    /// The login screen at boot (STATE_DIR/login-screen).
    LoginScreenOn,
    LoginScreenOff,
}

impl Action {
    /// The words a program sent (ToVdm::Act), as an action, or why not.
    pub fn parse(words: &[String]) -> Result<Action, String> {
        let w: Vec<&str> = words.iter().map(String::as_str).collect();
        match w.as_slice() {
            ["autologin.on", user] if valid_name(user) => Ok(Action::AutologinOn(user.to_string())),
            ["autologin.off"] => Ok(Action::AutologinOff),
            ["login-screen.on"] => Ok(Action::LoginScreenOn),
            ["login-screen.off"] => Ok(Action::LoginScreenOff),
            [name, ..] => Err(format!("no such action: {name}")),
            [] => Err("no action".into()),
        }
    }

    /// What the prompt says is asked.
    pub fn describe(&self) -> String {
        match self {
            Action::AutologinOn(u) => format!("Turn on automatic login for {u}: this machine starts into {u}'s session without a password"),
            Action::AutologinOff => "Turn off automatic login".into(),
            Action::LoginScreenOn => "Start this machine at the Vergence login screen".into(),
            Action::LoginScreenOff => "Stop starting this machine at the Vergence login screen (log in on a text console instead)".into(),
        }
    }

    /// Its name in the log.
    pub fn name(&self) -> &'static str {
        match self {
            Action::AutologinOn(_) => "autologin.on",
            Action::AutologinOff => "autologin.off",
            Action::LoginScreenOn => "login-screen.on",
            Action::LoginScreenOff => "login-screen.off",
        }
    }
}

/// An account's name as accounts are named: letters, digits, '.', '_', '-',
/// not first; at most 64.
fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && !n.starts_with('-') && n.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// A program's name for the prompt, from the kernel's pid: what it runs as
/// (Linux /proc/<pid>/comm; FreeBSD ps), our own tools by their names.
pub fn program_name(pid: u32) -> String {
    let comm = if cfg!(target_os = "linux") {
        std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()
    } else {
        std::process::Command::new("/bin/ps").args(["-o", "comm=", "-p", &pid.to_string()]).env_clear().output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let comm = comm.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).unwrap_or_else(|| format!("process {pid}"));
    match comm.as_str() {
        // The kernel keeps 15 characters of a name.
        "vergence-settin" | "vergence-settings" => "System Settings".into(),
        "vdm-elevate" => "vdm-elevate (a terminal)".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn actions_and_their_checks() {
        assert_eq!(Action::parse(&w(&["autologin.on", "derickson"])), Ok(Action::AutologinOn("derickson".into())));
        assert_eq!(Action::parse(&w(&["login-screen.off"])), Ok(Action::LoginScreenOff));
        assert!(Action::parse(&w(&["autologin.on"])).is_err(), "whose");
        assert!(Action::parse(&w(&["autologin.on", "-x"])).is_err());
        assert!(Action::parse(&w(&["autologin.off", "extra"])).is_err());
        assert!(Action::parse(&w(&["rm", "-rf"])).is_err());
        assert!(Action::parse(&[]).is_err());
    }
}
