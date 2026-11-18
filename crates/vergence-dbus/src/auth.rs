//! The server side of D-Bus authentication (SASL, EXTERNAL only).
//!
//! The client sends one nul byte, then text lines ending in "\r\n":
//! `AUTH EXTERNAL <hex of its uid in decimal>` (or `AUTH EXTERNAL` and then
//! `DATA ...`), optionally `NEGOTIATE_UNIX_FD`, then `BEGIN`, after which
//! the stream carries binary messages. The uid claimed must be the one the
//! kernel reports for the socket's peer.

/// Longest authentication exchange accepted before giving up.
const MAX_AUTH: usize = 16 * 1024;

#[derive(Debug, PartialEq, Eq)]
enum State {
    WaitingNul,
    WaitingAuth,
    WaitingData,
    Authenticated,
}

pub struct Auth {
    state: State,
    buf: Vec<u8>,
    peer_uid: u32,
    guid: String,
    seen: usize,
    unix_fds: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Send these bytes back (maybe nothing) and wait for more.
    Continue(Vec<u8>),
    /// Send these bytes back; authentication is over, and the rest are the
    /// first bytes of binary messages.
    Done(Vec<u8>, Vec<u8>),
    /// Give up on this client.
    Fail,
}

impl Auth {
    pub fn new(peer_uid: u32, guid: &str) -> Auth {
        Auth { state: State::WaitingNul, buf: Vec::new(), peer_uid, guid: guid.into(), seen: 0, unix_fds: false }
    }

    /// Whether the client asked to pass file descriptors (and was agreed).
    pub fn unix_fds(&self) -> bool {
        self.unix_fds
    }

    pub fn feed(&mut self, data: &[u8]) -> Step {
        self.seen += data.len();
        if self.seen > MAX_AUTH {
            return Step::Fail;
        }
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        if self.state == State::WaitingNul {
            match self.buf.first() {
                None => return Step::Continue(out),
                Some(0) => {
                    self.buf.remove(0);
                    self.state = State::WaitingAuth;
                }
                Some(_) => return Step::Fail,
            }
        }
        while let Some(i) = self.buf.windows(2).position(|w| w == b"\r\n") {
            let line: Vec<u8> = self.buf.drain(..i + 2).take(i).collect();
            let Ok(line) = String::from_utf8(line) else { return Step::Fail };
            let mut words = line.split(' ');
            let cmd = words.next().unwrap_or("");
            let arg = words.collect::<Vec<_>>();
            match (&self.state, cmd) {
                (State::WaitingAuth, "AUTH") => match (arg.first().copied(), arg.get(1).copied()) {
                    (Some("EXTERNAL"), Some(hex)) => self.check(hex, &mut out),
                    (Some("EXTERNAL"), None) => {
                        out.extend_from_slice(b"DATA\r\n");
                        self.state = State::WaitingData;
                    }
                    _ => out.extend_from_slice(b"REJECTED EXTERNAL\r\n"),
                },
                (State::WaitingData, "DATA") => match arg.first().copied() {
                    // No data: the client asks to be known by its socket credentials.
                    None | Some("") => self.accept(&mut out),
                    Some(hex) => self.check(hex, &mut out),
                },
                (State::WaitingData, "CANCEL") | (State::WaitingAuth, "CANCEL") => {
                    out.extend_from_slice(b"REJECTED EXTERNAL\r\n");
                    self.state = State::WaitingAuth;
                }
                // File descriptors may be passed (2026-11-17: GLib sends one
                // even when refused, then ends the connection, and with it
                // Firefox, on its portal OpenDirectory).
                (State::Authenticated, "NEGOTIATE_UNIX_FD") => {
                    self.unix_fds = true;
                    out.extend_from_slice(b"AGREE_UNIX_FD\r\n");
                }
                (State::Authenticated, "BEGIN") => return Step::Done(out, std::mem::take(&mut self.buf)),
                (_, "ERROR") => {
                    out.extend_from_slice(b"REJECTED EXTERNAL\r\n");
                    self.state = State::WaitingAuth;
                }
                _ => out.extend_from_slice(b"ERROR\r\n"),
            }
        }
        Step::Continue(out)
    }

    fn check(&mut self, hex: &str, out: &mut Vec<u8>) {
        if decode_hex(hex).and_then(|b| String::from_utf8(b).ok()).and_then(|s| s.parse::<u32>().ok()) == Some(self.peer_uid) {
            self.accept(out);
        } else {
            out.extend_from_slice(b"REJECTED EXTERNAL\r\n");
            self.state = State::WaitingAuth;
        }
    }

    fn accept(&mut self, out: &mut Vec<u8>) {
        out.extend_from_slice(format!("OK {}\r\n", self.guid).as_bytes());
        self.state = State::Authenticated;
    }
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

pub fn encode_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn external_with_initial_response() {
        let mut a = Auth::new(1001, GUID);
        let hex = encode_hex(b"1001");
        let step = a.feed(format!("\0AUTH EXTERNAL {hex}\r\n").as_bytes());
        assert_eq!(step, Step::Continue(format!("OK {GUID}\r\n").into_bytes()));
        assert!(!a.unix_fds());
        assert_eq!(a.feed(b"NEGOTIATE_UNIX_FD\r\n"), Step::Continue(b"AGREE_UNIX_FD\r\n".to_vec()));
        assert!(a.unix_fds());
        // BEGIN and the first message arrive together.
        assert_eq!(a.feed(b"BEGIN\r\nl\x01\x00\x01"), Step::Done(Vec::new(), b"l\x01\x00\x01".to_vec()));
    }

    #[test]
    fn external_with_data_and_empty_data() {
        let mut a = Auth::new(1001, GUID);
        assert_eq!(a.feed(b"\0AUTH EXTERNAL\r\n"), Step::Continue(b"DATA\r\n".to_vec()));
        assert_eq!(a.feed(b"DATA\r\n"), Step::Continue(format!("OK {GUID}\r\n").into_bytes()));
        let mut a = Auth::new(1001, GUID);
        a.feed(b"\0AUTH EXTERNAL\r\n");
        let step = a.feed(format!("DATA {}\r\n", encode_hex(b"1001")).as_bytes());
        assert_eq!(step, Step::Continue(format!("OK {GUID}\r\n").into_bytes()));
    }

    #[test]
    fn wrong_uid_other_mechanisms_and_garbage() {
        let mut a = Auth::new(1001, GUID);
        let step = a.feed(format!("\0AUTH EXTERNAL {}\r\n", encode_hex(b"0")).as_bytes());
        assert_eq!(step, Step::Continue(b"REJECTED EXTERNAL\r\n".to_vec()));
        assert_eq!(a.feed(b"AUTH DBUS_COOKIE_SHA1 x\r\n"), Step::Continue(b"REJECTED EXTERNAL\r\n".to_vec()));
        assert_eq!(a.feed(b"AUTH\r\n"), Step::Continue(b"REJECTED EXTERNAL\r\n".to_vec()));
        // BEGIN before OK is an error, not a way in.
        assert_eq!(a.feed(b"BEGIN\r\n"), Step::Continue(b"ERROR\r\n".to_vec()));
        assert_eq!(Auth::new(1001, GUID).feed(b"AUTH"), Step::Fail); // no leading nul
        let mut a = Auth::new(1001, GUID);
        assert_eq!(a.feed(&vec![b'x'; MAX_AUTH + 1]), Step::Fail);
    }
}
