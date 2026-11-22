// The Lock Screen and Login pages (VDM milestone 1, step 8).
//
// Lock Screen: desktop.toml [lock], which the server's energy.rs reads:
// screens-off (lock when the screens go off, default on), after (minutes
// with no input, 0: not on its own), before-sleep (default on).
//
// Login: what vdm says on its power socket ("login-state"): whether the
// login screen comes up at boot, and automatic login, with these rules
// (review M6): off unless turned on, only where exactly one person
// can log in, never with any encrypted storage, checked at every boot
// (see docs/vdm-milestone-1.md). Every change goes through an elevation
// prompt: administrators only, with their own password, every time, turning
// it off too (see docs/elevation-milestone-1.md). The page asks vdm, the
// server shows the prompt, vdm does it.

use super::*;

const LOCK_CHOICES: [(&str, i64); 8] = [
    ("Not on its own", 0),
    ("1 minute", 1),
    ("2 minutes", 2),
    ("5 minutes", 5),
    ("10 minutes", 10),
    ("15 minutes", 15),
    ("30 minutes", 30),
    ("1 hour", 60),
];

/// `[lock]`'s switches and minutes, with their defaults.
pub fn lock_settings(d: &toml::Table) -> (bool, i64, bool) {
    let t = d.get("lock").and_then(|n| n.as_table());
    let on = |k: &str| t.and_then(|t| t.get(k)).and_then(|v| v.as_bool()).unwrap_or(true);
    let after = t.and_then(|t| t.get("after")).and_then(|v| v.as_integer()).unwrap_or(0);
    (on("screens-off"), after, on("before-sleep"))
}

/// The grace after the screens' timeout before the lock needs the password (see docs/apps-roadmap.md).
const GRACE_CHOICES: [(&str, i64); 4] =
    [("At once", 0), ("After 10 seconds", 10), ("After 30 seconds", 30), ("After 1 minute", 60)];

fn lock_grace(d: &toml::Table) -> i64 {
    d.get("lock").and_then(|n| n.get("grace")).and_then(|v| v.as_integer()).unwrap_or(0)
}

#[derive(Clone, Debug)]
pub enum LockMsg {
    ScreensOff(bool),
    After(String),
    Grace(String),
    BeforeSleep(bool),
    AutologinOff,
    AutologinOn(String),
    LoginScreen(bool),
    /// The prompt's outcome came back (from the asking thread).
    Elevated(Result<String, String>),
}

/// vdm's power socket.
fn vdm_socket() -> &'static str {
    vergence_dm::paths::POWER_SOCKET
}

/// One line to vdm, its one-line answer.
pub fn ask_vdm(line: &str) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Write};
    let mut s =
        std::os::unix::net::UnixStream::connect(vdm_socket()).map_err(|_| "vdm is not running here".to_string())?;
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    s.write_all(format!("{line}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut answer = String::new();
    BufReader::new(&s).read_line(&mut answer).map_err(|e| e.to_string())?;
    Ok(answer.trim().to_string())
}

/// What vdm said about logging in.
#[derive(Clone, Debug, PartialEq)]
pub struct LoginState {
    pub login_screen: bool,
    pub autologin: Option<String>,
    /// The one person automatic login could be for, or why it cannot be on.
    pub allowed: Result<String, String>,
}

pub fn parse_login_state(line: &str) -> Option<LoginState> {
    let mut st = LoginState { login_screen: false, autologin: None, allowed: Err(String::new()) };
    let mut seen = false;
    for part in line.split('\t') {
        match part.split_once('=') {
            Some(("login-screen", v)) => {
                st.login_screen = v == "on";
                seen = true;
            }
            Some(("autologin", v)) => st.autologin = (v != "-").then(|| v.to_string()),
            Some(("allowed", v)) => {
                st.allowed = match v.strip_prefix("no: ") {
                    Some(why) => Err(why.to_string()),
                    None => Ok(v.to_string()),
                }
            }
            _ => {}
        }
    }
    seen.then_some(st)
}

impl App {
    /// Ask vdm for an action through the elevation prompt, in a thread (the
    /// person answers at the prompt); the outcome comes back as a message.
    fn elevate(&mut self, words: Vec<String>) {
        self.status = "Waiting for your answer at the prompt…".into();
        let mail = self.mail.clone();
        let _ = std::thread::Builder::new().name("elevate".into()).spawn(move || {
            let w: Vec<&str> = words.iter().map(String::as_str).collect();
            mail.send(Message::Lock(LockMsg::Elevated(vergence_dm::elevate::request(&w))));
        });
    }

    pub fn login_read(&mut self) {
        self.login = match ask_vdm("login-state") {
            Ok(line) => parse_login_state(&line).ok_or_else(|| format!("vdm said: {line}")),
            Err(e) => Err(e),
        };
    }

    pub fn lock_message(&mut self, m: LockMsg) {
        match m {
            LockMsg::ScreensOff(on) => self.save_table("lock", |t| {
                t.insert("screens-off".into(), on.into());
            }),
            LockMsg::BeforeSleep(on) => self.save_table("lock", |t| {
                t.insert("before-sleep".into(), on.into());
            }),
            LockMsg::Grace(label) => {
                let n = GRACE_CHOICES.iter().find(|(l, _)| *l == label).map(|(_, n)| *n).unwrap_or(0);
                self.save_table("lock", |t| {
                    t.insert("grace".into(), n.into());
                })
            }
            LockMsg::After(label) => {
                let m = LOCK_CHOICES.iter().find(|(l, _)| *l == label).map(|(_, m)| *m).unwrap_or(0);
                self.save_table("lock", |t| {
                    t.insert("after".into(), m.into());
                })
            }
            LockMsg::AutologinOff => self.elevate(vec!["autologin.off".into()]),
            LockMsg::AutologinOn(user) => self.elevate(vec!["autologin.on".into(), user]),
            LockMsg::LoginScreen(on) => {
                self.elevate(vec![if on { "login-screen.on" } else { "login-screen.off" }.into()])
            }
            LockMsg::Elevated(r) => {
                self.status = match r {
                    Ok(t) => format!("Done: {t}."),
                    Err(t) => format!("Not done: {t}."),
                };
                self.login_read();
            }
        }
    }

    pub fn lock_page(&self) -> Element<Message> {
        let (screens_off, after, before_sleep) = lock_settings(&self.desktop);
        let list: Vec<String> = LOCK_CHOICES.iter().map(|(l, _)| l.to_string()).collect();
        let at = LOCK_CHOICES.iter().position(|(_, m)| *m == after);
        let options = list.clone();
        let grace_list: Vec<String> = GRACE_CHOICES.iter().map(|(l, _)| l.to_string()).collect();
        let grace_at = GRACE_CHOICES.iter().position(|(_, n)| *n == lock_grace(&self.desktop));
        let grace_options = grace_list.clone();
        self.page_layout(
            "Lock Screen",
            "The lock covers every screen and asks for your password; your apps keep running behind it. Super+L locks at once, as does Lock in the log out dialog.",
            vec![
                self.group(Some("Lock the screen"), vec![
                    setting("When the screens go off", "After the time on the Energy page, and when you come back to this console after it.", switch(screens_off, |on| Message::Lock(LockMsg::ScreensOff(on))).name("Lock when the screens go off")),
                    setting(
                        "Require the password",
                        "After the screens went off by themselves: a touch within this brings them back without it. Super+L, Lock and sleep always lock at once.",
                        choice(grace_options, grace_at, move |i| Message::Lock(LockMsg::Grace(grace_list[i].clone()))).width(Length::Fixed(160.0)).name("Require the password"),
                    ),
                    setting("After no input", "Even with the screens on.", choice(options, at, move |i| Message::Lock(LockMsg::After(list[i].clone()))).width(Length::Fixed(160.0)).name("Lock after no input")),
                    setting(
                        "Before the machine sleeps",
                        if before_sleep { "The lock is on every screen before it sleeps, so it wakes to the lock." } else { "Off: whoever wakes the machine is in your session." },
                        switch(before_sleep, |on| Message::Lock(LockMsg::BeforeSleep(on))).name("Lock before the machine sleeps"),
                    ),
                ]),
                self.status_line(),
            ],
        )
    }

    pub fn login_page(&self) -> Element<Message> {
        let mut groups = Vec::new();
        match &self.login {
            Err(e) => groups.push(self.group(None, vec![setting("Login", e, space())])),
            Ok(st) => {
                groups.push(self.group(Some("At boot"), vec![setting(
                    "The login screen",
                    if st.login_screen { "On: the machine starts at the Vergence login screen. The text consoles stay (Ctrl+Alt+F1)." } else { "Off: the machine starts at a text console; log in there and run vergence." },
                    switch(st.login_screen, |on| Message::Lock(LockMsg::LoginScreen(on))).name("The login screen at boot"),
                )]));
                let (what, control): (String, Element<Message>) = match (&st.autologin, &st.allowed) {
                    (Some(user), Ok(_)) => (
                        format!(
                            "On for {user}: the machine starts straight into {user}'s session, without a password, at boot only."
                        ),
                        button(label("Turn Off…").size(12.0))
                            .padding(Padding::hv(4.0, 12.0))
                            .on_press(Message::Lock(LockMsg::AutologinOff))
                            .name("Turn automatic login off"),
                    ),
                    (Some(user), Err(why)) => (
                        format!("Set for {user}, but not done: {why}."),
                        button(label("Turn Off…").size(12.0))
                            .padding(Padding::hv(4.0, 12.0))
                            .on_press(Message::Lock(LockMsg::AutologinOff))
                            .name("Turn automatic login off"),
                    ),
                    (None, Ok(user)) => (
                        format!("Off. It can be on here, for {user}."),
                        button(label("Turn On…").size(12.0))
                            .padding(Padding::hv(4.0, 12.0))
                            .on_press(Message::Lock(LockMsg::AutologinOn(user.clone())))
                            .name("Turn automatic login on"),
                    ),
                    (None, Err(why)) => (format!("Off, and it cannot be on here: {why}."), label("Off")),
                };
                groups.push(
                    self.group(Some("Automatic login"), vec![setting("Log in without a password", &what, control)]),
                );
                groups.push(self.group(None, vec![setting(
                    "The rules",
                    "Automatic login is only ever allowed where exactly one person can log in and nothing on the machine is encrypted; both are checked again at every boot. The lock still asks for the password. Every change here asks an administrator's password first.",
                    space(),
                )]));
            }
        }
        groups.push(self.status_line());
        self.page_layout("Login", "How this machine lets people in.", groups)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vdms_login_state_is_read() {
        let st = parse_login_state("login-screen=on\tautologin=-\tallowed=derickson").unwrap();
        assert!(st.login_screen);
        assert_eq!(st.autologin, None);
        assert_eq!(st.allowed, Ok("derickson".into()));
        let st = parse_login_state(
            "login-screen=off\tautologin=derickson\tallowed=no: storage here is encrypted (dm-0 (LUKS2)): x",
        )
        .unwrap();
        assert_eq!(st.autologin.as_deref(), Some("derickson"));
        assert!(st.allowed.unwrap_err().starts_with("storage here is encrypted"));
        assert!(parse_login_state("no: nonsense").is_none());
    }
}
