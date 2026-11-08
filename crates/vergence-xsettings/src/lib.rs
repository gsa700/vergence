//! The XSETTINGS wire format (freedesktop.org XSETTINGS specification 0.5):
//! the bytes a settings manager puts in its `_XSETTINGS_SETTINGS` property,
//! read by GTK (theme, fonts, window scale, DPI...).
//!
//! Encoding and decoding only, no X connection: vergd (milestone 4) and the
//! phase 2 display server (which must serve XSETTINGS to X11 apps under
//! Xwayland) both use it.
//!
//! Layout: byte order (0 = LSB first, 1 = MSB first), 3 unused bytes, serial
//! (u32), number of settings (u32), then per setting: type (0 integer,
//! 1 string, 2 colour), 1 unused byte, name length (u16), the name padded to
//! 4 bytes, the serial it last changed in (u32), and the value: i32; or
//! length (u32) and bytes padded to 4; or red, blue, green, alpha (u16 each,
//! in that order).

use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i32),
    Str(String),
    /// Red, green, blue, alpha (stored on the wire as red, blue, green, alpha).
    Color(u16, u16, u16, u16),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{i}"),
            Value::Str(s) => write!(f, "{s:?}"),
            Value::Color(r, g, b, a) => write!(f, "rgba({r}, {g}, {b}, {a})"),
        }
    }
}

/// One setting and the serial it last changed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    pub value: Value,
    pub last_change: u32,
}

/// A complete settings set, sorted by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub serial: u32,
    pub settings: BTreeMap<String, Setting>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Truncated,
    BadByteOrder(u8),
    BadType(u8),
    BadUtf8,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated => write!(f, "XSETTINGS data ends early"),
            Error::BadByteOrder(b) => write!(f, "XSETTINGS byte order {b} is not 0 or 1"),
            Error::BadType(t) => write!(f, "XSETTINGS setting type {t} is not 0, 1 or 2"),
            Error::BadUtf8 => write!(f, "XSETTINGS name or string is not UTF-8"),
        }
    }
}

impl std::error::Error for Error {}

fn pad4(n: usize) -> usize {
    (4 - n % 4) % 4
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
    msb: bool,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], Error> {
        let end = self.at.checked_add(n).ok_or(Error::Truncated)?;
        let s = self.b.get(self.at..end).ok_or(Error::Truncated)?;
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        let s: [u8; 2] = self.take(2)?.try_into().unwrap();
        Ok(if self.msb { u16::from_be_bytes(s) } else { u16::from_le_bytes(s) })
    }
    fn u32(&mut self) -> Result<u32, Error> {
        let s: [u8; 4] = self.take(4)?.try_into().unwrap();
        Ok(if self.msb { u32::from_be_bytes(s) } else { u32::from_le_bytes(s) })
    }
    fn string(&mut self, n: usize) -> Result<String, Error> {
        let s = String::from_utf8(self.take(n)?.to_vec()).map_err(|_| Error::BadUtf8)?;
        self.take(pad4(n))?;
        Ok(s)
    }
}

impl Settings {
    /// Decode a `_XSETTINGS_SETTINGS` property, in either byte order.
    pub fn decode(b: &[u8]) -> Result<Settings, Error> {
        let msb = match b.first() {
            Some(0) => false,
            Some(1) => true,
            Some(&x) => return Err(Error::BadByteOrder(x)),
            None => return Err(Error::Truncated),
        };
        let mut r = Reader { b, at: 4, msb };
        let serial = r.u32()?;
        let n = r.u32()?;
        let mut settings = BTreeMap::new();
        for _ in 0..n {
            let kind = r.u8()?;
            r.u8()?;
            let name_len = r.u16()? as usize;
            let name = r.string(name_len)?;
            let last_change = r.u32()?;
            let value = match kind {
                0 => Value::Int(r.u32()? as i32),
                1 => {
                    let len = r.u32()? as usize;
                    Value::Str(r.string(len)?)
                }
                2 => {
                    let (red, blue, green, alpha) = (r.u16()?, r.u16()?, r.u16()?, r.u16()?);
                    Value::Color(red, green, blue, alpha)
                }
                t => return Err(Error::BadType(t)),
            };
            settings.insert(name, Setting { value, last_change });
        }
        Ok(Settings { serial, settings })
    }

    /// Encode in little-endian byte order (any order is valid).
    pub fn encode(&self) -> Vec<u8> {
        let mut o = vec![0u8, 0, 0, 0];
        o.extend(self.serial.to_le_bytes());
        o.extend((self.settings.len() as u32).to_le_bytes());
        let put_str = |o: &mut Vec<u8>, s: &str| {
            o.extend(s.as_bytes());
            o.extend(std::iter::repeat_n(0u8, pad4(s.len())));
        };
        for (name, s) in &self.settings {
            let kind = match s.value {
                Value::Int(_) => 0u8,
                Value::Str(_) => 1,
                Value::Color(..) => 2,
            };
            o.push(kind);
            o.push(0);
            o.extend((name.len() as u16).to_le_bytes());
            put_str(&mut o, name);
            o.extend(s.last_change.to_le_bytes());
            match &s.value {
                Value::Int(i) => o.extend(i.to_le_bytes()),
                Value::Str(v) => {
                    o.extend((v.len() as u32).to_le_bytes());
                    put_str(&mut o, v);
                }
                Value::Color(r, g, b, a) => {
                    for c in [r, b, g, a] {
                        o.extend(c.to_le_bytes());
                    }
                }
            }
        }
        o
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.settings.get(name).map(|s| &s.value)
    }

    /// Set a value; if it differs, bump the serial and record the change.
    /// Returns true if anything changed. Call [`Settings::bump`] once per
    /// batch instead when setting several values together.
    pub fn set(&mut self, name: &str, value: Value) -> bool {
        if self.get(name) == Some(&value) {
            return false;
        }
        self.serial = self.serial.wrapping_add(1);
        self.settings.insert(name.to_string(), Setting { value, last_change: self.serial });
        true
    }
}

// ---- the settings file (~/.config/vergence/settings.toml), shared by vergd and the server ----

/// The settings as TOML: `"Net/ThemeName" = "Adwaita"`, integers as
/// integers, colours as `[red, green, blue, alpha]`.
pub fn to_toml(s: &Settings) -> String {
    let mut t = toml::Table::new();
    for (name, v) in &s.settings {
        let x = match &v.value {
            Value::Int(i) => toml::Value::Integer(*i as i64),
            Value::Str(s) => toml::Value::String(s.clone()),
            Value::Color(r, g, b, a) => {
                toml::Value::Array([r, g, b, a].iter().map(|c| toml::Value::Integer(**c as i64)).collect())
            }
        };
        t.insert(name.clone(), x);
    }
    let mut top = toml::Table::new();
    top.insert("settings".into(), toml::Value::Table(t));
    toml::to_string(&top).expect("settings serialize")
}

pub fn from_toml(text: &str) -> Result<Settings, String> {
    let top: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let mut s = Settings::default();
    let Some(t) = top.get("settings") else { return Ok(s) };
    let t = t.as_table().ok_or_else(|| "[settings] must be a table".to_string())?;
    for (name, x) in t {
        let v = match x {
            toml::Value::Integer(i) => Value::Int(i32::try_from(*i).map_err(|_| format!("{name}: {i} is too large"))?),
            toml::Value::Boolean(b) => Value::Int(*b as i32),
            toml::Value::String(v) => Value::Str(v.clone()),
            toml::Value::Array(a) if a.len() == 4 => {
                let c: Vec<u16> = a
                    .iter()
                    .map(|c| c.as_integer().and_then(|i| u16::try_from(i).ok()))
                    .collect::<Option<_>>()
                    .ok_or_else(|| format!("{name}: a colour is four numbers 0-65535"))?;
                Value::Color(c[0], c[1], c[2], c[3])
            }
            _ => return Err(format!("{name}: must be a number, text, or [r, g, b, a]")),
        };
        s.set(name, v);
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Settings {
        let mut s = Settings::default();
        s.set("Net/ThemeName", Value::Str("Adwaita".into()));
        s.set("Gdk/WindowScalingFactor", Value::Int(2));
        s.set("Xft/DPI", Value::Int(96 * 2 * 1024));
        s.set("Gtk/ColorPalette", Value::Str("black:white".into()));
        s.set("Test/Colour", Value::Color(1, 2, 3, 0xFFFF));
        s.set("Net/CursorBlinkTime", Value::Int(-1));
        s
    }

    #[test]
    fn round_trip() {
        let s = sample();
        let b = s.encode();
        assert_eq!(b.len() % 4, 0);
        assert_eq!(Settings::decode(&b).unwrap(), s);
    }

    #[test]
    fn decodes_big_endian() {
        // One integer setting "A" = 5, serial 7, written MSB first by hand.
        let b = [
            1, 0, 0, 0, // MSB first
            0, 0, 0, 7, // serial
            0, 0, 0, 1, // one setting
            0, 0, 0, 1, // integer, name length 1
            b'A', 0, 0, 0, // name, padded
            0, 0, 0, 3, // last change
            0, 0, 0, 5, // value
        ];
        let s = Settings::decode(&b).unwrap();
        assert_eq!(s.serial, 7);
        assert_eq!(s.get("A"), Some(&Value::Int(5)));
        assert_eq!(s.settings["A"].last_change, 3);
    }

    #[test]
    fn colour_wire_order_is_red_blue_green_alpha() {
        let mut s = Settings::default();
        s.set("C", Value::Color(0x1111, 0x2222, 0x3333, 0x4444));
        let b = s.encode();
        let tail = &b[b.len() - 8..];
        assert_eq!(tail, [0x11, 0x11, 0x33, 0x33, 0x22, 0x22, 0x44, 0x44]);
    }

    #[test]
    fn set_bumps_serial_only_on_change() {
        let mut s = sample();
        let before = s.serial;
        assert!(!s.set("Gdk/WindowScalingFactor", Value::Int(2)));
        assert_eq!(s.serial, before);
        assert!(s.set("Gdk/WindowScalingFactor", Value::Int(1)));
        assert_eq!(s.serial, before + 1);
        assert_eq!(s.settings["Gdk/WindowScalingFactor"].last_change, before + 1);
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert_eq!(Settings::decode(&[]), Err(Error::Truncated));
        assert_eq!(Settings::decode(&[2, 0, 0, 0]), Err(Error::BadByteOrder(2)));
        let mut b = sample().encode();
        b.truncate(b.len() - 3);
        assert_eq!(Settings::decode(&b), Err(Error::Truncated));
        // A huge declared count with no data.
        let huge = [0, 0, 0, 0, 1, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(Settings::decode(&huge), Err(Error::Truncated));
        let mut t = vec![0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 9, 0, 1, 0, b'A', 0, 0, 0, 0, 0, 0, 0];
        t.extend([0; 4]);
        assert_eq!(Settings::decode(&t), Err(Error::BadType(9)));
    }
}
