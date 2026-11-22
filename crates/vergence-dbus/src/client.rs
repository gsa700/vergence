//! A D-Bus client: a connection to a bus (the system bus, for udisks2:
//! files milestone 1, docs/files-milestone-1.md), blocking, one thread. Authenticates
//! with EXTERNAL (our uid), says Hello, makes method calls and waits for
//! their replies, and keeps the signals that arrive meanwhile for
//! [`Client::next_signal`]. No file descriptors are passed.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::message::{self, Message};
use crate::value::Value;

unsafe extern "C" {
    fn getuid() -> u32;
}

pub struct Client {
    sock: UnixStream,
    serial: u32,
    buf: Vec<u8>,
    signals: VecDeque<Message>,
    /// The unique name the bus gave us.
    pub name: String,
}

/// The system bus's socket: DBUS_SYSTEM_BUS_ADDRESS's `unix:path=`, else
/// the usual places.
pub fn system_bus_path() -> Option<PathBuf> {
    if let Ok(a) = std::env::var("DBUS_SYSTEM_BUS_ADDRESS")
        && let Some(p) = a
            .split(';')
            .find_map(|t| t.strip_prefix("unix:"))
            .and_then(|t| t.split(',').find_map(|kv| kv.strip_prefix("path=")))
    {
        return Some(PathBuf::from(p));
    }
    ["/run/dbus/system_bus_socket", "/var/run/dbus/system_bus_socket"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

fn other(text: impl Into<String>) -> io::Error {
    io::Error::other(text.into())
}

impl Client {
    /// Connects to the system bus.
    pub fn system() -> io::Result<Client> {
        let path = system_bus_path().ok_or_else(|| other("no system bus"))?;
        Client::connect(&path)
    }

    pub fn connect(path: &std::path::Path) -> io::Result<Client> {
        let mut sock = UnixStream::connect(path)?;
        sock.set_read_timeout(Some(Duration::from_secs(5)))?;
        // SAFETY: getuid cannot fail and touches no memory of ours.
        let uid = unsafe { getuid() };
        sock.write_all(
            format!("\0AUTH EXTERNAL {}\r\n", crate::auth::encode_hex(uid.to_string().as_bytes())).as_bytes(),
        )?;
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        while !line.ends_with(b"\r\n") {
            if sock.read(&mut byte)? == 0 || line.len() > 512 {
                return Err(other("the bus closed during authentication"));
            }
            line.push(byte[0]);
        }
        if !line.starts_with(b"OK ") {
            return Err(other(format!("the bus refused: {}", String::from_utf8_lossy(&line).trim())));
        }
        sock.write_all(b"BEGIN\r\n")?;
        let mut c = Client { sock, serial: 0, buf: Vec::new(), signals: VecDeque::new(), name: String::new() };
        let hello = c.call(
            Message::method_call(crate::BUS_NAME, crate::BUS_PATH, crate::BUS_NAME, "Hello", &[]),
            Duration::from_secs(5),
        )?;
        c.name =
            hello.args().ok().and_then(|a| a.first().and_then(|v| v.as_str().map(str::to_string))).unwrap_or_default();
        Ok(c)
    }

    fn send(&mut self, mut m: Message) -> io::Result<u32> {
        self.serial = self.serial.wrapping_add(1).max(1);
        m.serial = self.serial;
        self.sock.write_all(&m.serialize())?;
        Ok(self.serial)
    }

    /// The next message from the bus, waiting at most `timeout`.
    fn read(&mut self, timeout: Duration) -> io::Result<Option<Message>> {
        let until = Instant::now() + timeout;
        loop {
            if let Some(total) = message::frame_len(&self.buf).map_err(|e| other(format!("{e:?}")))?
                && self.buf.len() >= total
            {
                let rest = self.buf.split_off(total);
                let m = Message::parse(&self.buf).map_err(|e| other(format!("{e:?}")));
                self.buf = rest;
                return m.map(Some);
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            self.sock.set_read_timeout(Some(left.max(Duration::from_millis(1))))?;
            let mut chunk = [0u8; 16 * 1024];
            match self.sock.read(&mut chunk) {
                Ok(0) => return Err(other("the bus closed the connection")),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => return Ok(None),
                Err(e) => return Err(e),
            }
        }
    }

    /// Calls a method and waits for its reply; an error reply is an Err
    /// carrying its name and text.
    pub fn call(&mut self, m: Message, timeout: Duration) -> io::Result<Message> {
        let serial = self.send(m)?;
        let until = Instant::now() + timeout;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let Some(r) = self.read(left)? else { return Err(io::Error::new(io::ErrorKind::TimedOut, "no reply")) };
            if r.reply_serial == Some(serial) {
                if r.kind == message::ERROR {
                    let text = r
                        .args()
                        .ok()
                        .and_then(|a| a.first().and_then(|v| v.as_str().map(str::to_string)))
                        .unwrap_or_default();
                    return Err(other(format!("{}: {text}", r.error_name.as_deref().unwrap_or("error"))));
                }
                return Ok(r);
            }
            if r.kind == message::SIGNAL {
                self.signals.push_back(r);
            }
        }
    }

    /// Asks the bus for signals matching `rule` (AddMatch).
    pub fn add_match(&mut self, rule: &str) -> io::Result<()> {
        self.call(
            Message::method_call(
                crate::BUS_NAME,
                crate::BUS_PATH,
                crate::BUS_NAME,
                "AddMatch",
                &[Value::Str(rule.into())],
            ),
            Duration::from_secs(5),
        )
        .map(|_| ())
    }

    /// The next signal, waiting at most `timeout`.
    pub fn next_signal(&mut self, timeout: Duration) -> io::Result<Option<Message>> {
        if let Some(s) = self.signals.pop_front() {
            return Ok(Some(s));
        }
        let until = Instant::now() + timeout;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match self.read(left)? {
                Some(m) if m.kind == message::SIGNAL => return Ok(Some(m)),
                Some(_) => continue,
                None => return Ok(None),
            }
        }
    }
}

/// A value looked up in a dictionary (`a{sv}` or `a{s...}`) by its string key.
pub fn lookup<'a>(dict: &'a Value, key: &str) -> Option<&'a Value> {
    let Value::Array(_, items) = dict else { return None };
    items.iter().find_map(|e| match e {
        Value::Entry(k, v) if k.as_str() == Some(key) || matches!(&**k, Value::Path(p) if p == key) => {
            Some(match &**v {
                Value::Variant(inner) => &**inner,
                other => other,
            })
        }
        _ => None,
    })
}

/// A byte string (`ay`, as udisks gives paths), its ending nul left off.
pub fn bytes_str(v: &Value) -> Option<String> {
    let Value::Array(_, items) = v else { return None };
    let b: Vec<u8> = items
        .iter()
        .filter_map(|x| if let Value::Byte(b) = x { Some(*b) } else { None })
        .take_while(|b| *b != 0)
        .collect();
    Some(String::from_utf8_lossy(&b).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionaries_and_byte_strings_are_read() {
        let d = Value::Array(
            "{sv}".into(),
            vec![Value::Entry(
                Box::new(Value::Str("HintAuto".into())),
                Box::new(Value::Variant(Box::new(Value::Bool(true)))),
            )],
        );
        assert_eq!(lookup(&d, "HintAuto"), Some(&Value::Bool(true)));
        assert_eq!(lookup(&d, "Nope"), None);
        let ay = Value::Array("y".into(), b"/run/media/u/STICK\0".iter().map(|b| Value::Byte(*b)).collect());
        assert_eq!(bytes_str(&ay).as_deref(), Some("/run/media/u/STICK"));
    }

    /// Against a real system bus where there is one (Linux): Hello and a call.
    #[test]
    fn talks_to_the_system_bus_if_there_is_one() {
        let Ok(mut c) = Client::system() else { return };
        assert!(c.name.starts_with(':'), "a unique name: {}", c.name);
        let r = c
            .call(
                Message::method_call(crate::BUS_NAME, crate::BUS_PATH, crate::BUS_NAME, "GetId", &[]),
                Duration::from_secs(5),
            )
            .unwrap();
        assert!(r.args().unwrap()[0].as_str().is_some_and(|s| s.len() == 32));
    }
}
