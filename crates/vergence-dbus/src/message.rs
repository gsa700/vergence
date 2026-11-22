//! D-Bus messages: the fixed header, the header fields, and the body.

use crate::value::{self, Error, Reader, Value, Writer};

/// Largest message accepted. The specification allows 128 MiB; the session
/// bus carries nothing near that (a notification with an image is a few
/// hundred KiB).
pub const MAX_MESSAGE: usize = 8 * 1024 * 1024;

pub const METHOD_CALL: u8 = 1;
pub const METHOD_RETURN: u8 = 2;
pub const ERROR: u8 = 3;
pub const SIGNAL: u8 = 4;

pub const NO_REPLY_EXPECTED: u8 = 0x1;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Message {
    /// Big-endian ('B') rather than little-endian ('l').
    pub big: bool,
    pub kind: u8,
    pub flags: u8,
    pub serial: u32,
    pub path: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
    pub error_name: Option<String>,
    pub reply_serial: Option<u32>,
    pub destination: Option<String>,
    pub sender: Option<String>,
    /// The body's signature ("" when there is no body).
    pub signature: String,
    pub unix_fds: u32,
    /// The marshalled body, in this message's byte order.
    pub body: Vec<u8>,
}

/// The total length of the message whose first 16 bytes are `head`, once
/// those 16 bytes are there.
pub fn frame_len(head: &[u8]) -> Result<Option<usize>, Error> {
    if head.len() < 16 {
        return Ok(None);
    }
    let big = match head[0] {
        b'l' => false,
        b'B' => true,
        _ => return Err(Error::BadValue("byte order")),
    };
    let word = |i: usize| {
        let mut b: [u8; 4] = head[i..i + 4].try_into().unwrap();
        if big {
            b.reverse();
        }
        u32::from_le_bytes(b) as usize
    };
    let body = word(4);
    let fields = word(12);
    let total = (16 + fields).div_ceil(8) * 8 + body;
    if body > MAX_MESSAGE || fields > MAX_MESSAGE || total > MAX_MESSAGE {
        return Err(Error::BadValue("message length"));
    }
    Ok(Some(total))
}

impl Message {
    pub fn method_call(destination: &str, path: &str, interface: &str, member: &str, args: &[Value]) -> Message {
        let (signature, body) = value::body(args, false);
        Message {
            kind: METHOD_CALL,
            destination: Some(destination.into()),
            path: Some(path.into()),
            interface: Some(interface.into()),
            member: Some(member.into()),
            signature,
            body,
            ..Default::default()
        }
    }

    /// A reply to `call`, from `sender`.
    pub fn method_return(call: &Message, sender: &str, args: &[Value]) -> Message {
        let (signature, body) = value::body(args, false);
        Message {
            kind: METHOD_RETURN,
            flags: NO_REPLY_EXPECTED,
            reply_serial: Some(call.serial),
            destination: call.sender.clone(),
            sender: Some(sender.into()),
            signature,
            body,
            ..Default::default()
        }
    }

    pub fn error(call: &Message, sender: &str, name: &str, text: &str) -> Message {
        let (signature, body) = value::body(&[Value::Str(text.into())], false);
        Message {
            kind: ERROR,
            flags: NO_REPLY_EXPECTED,
            error_name: Some(name.into()),
            reply_serial: Some(call.serial),
            destination: call.sender.clone(),
            sender: Some(sender.into()),
            signature,
            body,
            ..Default::default()
        }
    }

    pub fn signal(sender: &str, path: &str, interface: &str, member: &str, args: &[Value]) -> Message {
        let (signature, body) = value::body(args, false);
        Message {
            kind: SIGNAL,
            flags: NO_REPLY_EXPECTED,
            sender: Some(sender.into()),
            path: Some(path.into()),
            interface: Some(interface.into()),
            member: Some(member.into()),
            signature,
            body,
            ..Default::default()
        }
    }

    /// Parse one whole message (exactly `frame_len` bytes).
    pub fn parse(data: &[u8]) -> Result<Message, Error> {
        let total = frame_len(data)?.ok_or(Error::Short)?;
        if data.len() != total {
            return Err(Error::BadValue("message length"));
        }
        let big = data[0] == b'B';
        let mut r = Reader::at(data, 1, big);
        let kind = r.u8()?;
        let flags = r.u8()?;
        if r.u8()? != 1 {
            return Err(Error::BadValue("protocol version"));
        }
        let body_len = r.u32()? as usize;
        let serial = r.u32()?;
        if serial == 0 {
            return Err(Error::BadValue("serial"));
        }
        let fields = match r.value("a(yv)")? {
            Value::Array(_, items) => items,
            _ => unreachable!(),
        };
        let mut m = Message { big, kind, flags, serial, ..Default::default() };
        for f in fields {
            let Value::Struct(mut pair) = f else { continue };
            let (Some(Value::Variant(v)), Some(Value::Byte(code))) = (pair.pop(), pair.pop()) else { continue };
            let s = || v.as_str().map(str::to_owned).ok_or(Error::BadValue("header field"));
            match code {
                1 => m.path = Some(s()?),
                2 => m.interface = Some(s()?),
                3 => m.member = Some(s()?),
                4 => m.error_name = Some(s()?),
                5 => m.reply_serial = Some(v.as_u32().ok_or(Error::BadValue("reply serial"))?),
                6 => m.destination = Some(s()?),
                7 => m.sender = Some(s()?),
                8 => m.signature = s()?,
                9 => m.unix_fds = v.as_u32().ok_or(Error::BadValue("unix fds"))?,
                _ => {} // Unknown fields are ignored (and not passed on).
            }
        }
        r.align(8)?;
        m.body = data.get(r.pos..r.pos + body_len).ok_or(Error::Short)?.to_vec();
        let need = |ok: bool, what| if ok { Ok(()) } else { Err(Error::BadValue(what)) };
        match kind {
            METHOD_CALL => need(m.path.is_some() && m.member.is_some(), "method call fields")?,
            METHOD_RETURN => need(m.reply_serial.is_some(), "reply serial")?,
            ERROR => need(m.reply_serial.is_some() && m.error_name.is_some(), "error fields")?,
            SIGNAL => need(m.path.is_some() && m.interface.is_some() && m.member.is_some(), "signal fields")?,
            _ => return Err(Error::BadValue("message type")),
        }
        Ok(m)
    }

    /// The message on the wire, in its own byte order.
    pub fn serialize(&self) -> Vec<u8> {
        let mut w = Writer::new(self.big);
        w.u8(if self.big { b'B' } else { b'l' });
        w.u8(self.kind);
        w.u8(self.flags);
        w.u8(1);
        w.u32(self.body.len() as u32);
        w.u32(self.serial);
        let mut fields = Vec::new();
        let mut field =
            |code: u8, v: Value| fields.push(Value::Struct(vec![Value::Byte(code), Value::Variant(Box::new(v))]));
        if let Some(p) = &self.path {
            field(1, Value::Path(p.clone()));
        }
        for (code, v) in
            [(2, &self.interface), (3, &self.member), (4, &self.error_name), (6, &self.destination), (7, &self.sender)]
        {
            if let Some(s) = v {
                field(code, Value::Str(s.clone()));
            }
        }
        if let Some(s) = self.reply_serial {
            field(5, Value::U32(s));
        }
        if !self.signature.is_empty() {
            field(8, Value::Sig(self.signature.clone()));
        }
        if self.unix_fds > 0 {
            field(9, Value::U32(self.unix_fds));
        }
        w.value(&Value::Array("(yv)".into(), fields));
        w.align(8);
        w.buf.extend_from_slice(&self.body);
        w.buf
    }

    /// The body's values.
    pub fn args(&self) -> Result<Vec<Value>, Error> {
        Reader::new(&self.body, self.big).values(&self.signature)
    }

    /// "interface.member" for logs.
    pub fn method(&self) -> String {
        format!("{}.{}", self.interface.as_deref().unwrap_or("?"), self.member.as_deref().unwrap_or("?"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_in_both_byte_orders() {
        let mut call = Message::method_call(
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "Notify",
            &[Value::Str("firefox".into()), Value::U32(0), Value::strings(["a"]), Value::I32(-1)],
        );
        call.serial = 7;
        call.sender = Some(":1.4".into());
        for big in [false, true] {
            let mut m = call.clone();
            m.big = big;
            // The body must be in the message's own byte order.
            m.body = value::body(&call.args().unwrap(), big).1;
            let bytes = m.serialize();
            assert_eq!(frame_len(&bytes[..16]).unwrap(), Some(bytes.len()));
            let back = Message::parse(&bytes).unwrap();
            assert_eq!(back, m);
            assert_eq!(back.args().unwrap(), call.args().unwrap());
        }
    }

    #[test]
    fn replies_and_signals() {
        let mut call =
            Message::method_call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "Hello", &[]);
        call.serial = 1;
        call.sender = Some(":1.9".into());
        let mut r = Message::method_return(&call, "org.freedesktop.DBus", &[Value::Str(":1.9".into())]);
        r.serial = 1;
        let back = Message::parse(&r.serialize()).unwrap();
        assert_eq!((back.reply_serial, back.destination.as_deref()), (Some(1), Some(":1.9")));
        let mut e = Message::error(&call, "org.freedesktop.DBus", "org.freedesktop.DBus.Error.ServiceUnknown", "no");
        e.serial = 2;
        assert_eq!(
            Message::parse(&e.serialize()).unwrap().error_name.as_deref(),
            Some("org.freedesktop.DBus.Error.ServiceUnknown")
        );
        let mut s = Message::signal(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameAcquired",
            &[Value::Str(":1.9".into())],
        );
        s.serial = 3;
        assert_eq!(Message::parse(&s.serialize()).unwrap(), s);
    }

    /// A byte array in a variant in an a{sv} (the portal's current_folder):
    /// its bytes come back, not some of the header's (2026-11-15).
    #[test]
    fn byte_arrays_in_options_round_trip() {
        let folder: Vec<Value> = b"/home/u/docs\0".iter().map(|b| Value::Byte(*b)).collect();
        let opts = Value::Array(
            "{sv}".into(),
            vec![
                Value::Entry(
                    Box::new(Value::Str("handle_token".into())),
                    Box::new(Value::Variant(Box::new(Value::Str("t1".into())))),
                ),
                Value::Entry(
                    Box::new(Value::Str("multiple".into())),
                    Box::new(Value::Variant(Box::new(Value::Bool(true)))),
                ),
                Value::Entry(
                    Box::new(Value::Str("accept_label".into())),
                    Box::new(Value::Variant(Box::new(Value::Str("_Attach".into())))),
                ),
                Value::Entry(
                    Box::new(Value::Str("filters".into())),
                    Box::new(Value::Variant(Box::new(Value::Array(
                        "(sa(us))".into(),
                        vec![
                            Value::Struct(vec![
                                Value::Str("Markdown".into()),
                                Value::Array(
                                    "(us)".into(),
                                    vec![Value::Struct(vec![Value::U32(0), Value::Str("*.md".into())])],
                                ),
                            ]),
                            Value::Struct(vec![
                                Value::Str("Pictures".into()),
                                Value::Array(
                                    "(us)".into(),
                                    vec![Value::Struct(vec![Value::U32(1), Value::Str("image/*".into())])],
                                ),
                            ]),
                        ],
                    )))),
                ),
                Value::Entry(
                    Box::new(Value::Str("current_folder".into())),
                    Box::new(Value::Variant(Box::new(Value::Array("y".into(), folder)))),
                ),
            ],
        );
        let args = vec![Value::Str(String::new()), Value::Str("Pick".into()), opts];
        let mut m = Message::method_call(
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.FileChooser",
            "OpenFile",
            &args,
        );
        m.serial = 7;
        m.sender = Some(":1.2".into());
        let back = Message::parse(&m.serialize()).unwrap();
        assert_eq!(back.args().unwrap(), args);
    }

    #[test]
    fn bad_headers_are_rejected() {
        let mut m = Message::signal("a.b", "/", "a.b", "C", &[]);
        m.serial = 1;
        let good = m.serialize();
        let mut bad = good.clone();
        bad[0] = b'x';
        assert!(Message::parse(&bad).is_err());
        let mut bad = good.clone();
        bad[3] = 2; // protocol version
        assert!(Message::parse(&bad).is_err());
        let mut bad = good.clone();
        bad[8..12].copy_from_slice(&0u32.to_le_bytes()); // serial 0
        assert!(Message::parse(&bad).is_err());
        // A signal without a member.
        let mut m2 = m.clone();
        m2.member = None;
        assert!(Message::parse(&m2.serialize()).is_err());
        // A huge declared body.
        let mut head = good[..16].to_vec();
        head[4..8].copy_from_slice(&(MAX_MESSAGE as u32 + 1).to_le_bytes());
        assert!(frame_len(&head).is_err());
    }
}
