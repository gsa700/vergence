//! A sound server's outputs, through the PulseAudio native protocol, which
//! PulseAudio and PipeWire (pipewire-pulse) both speak: Linux with a sound
//! server running (Fedora on apollo, the platform's standard). Our own
//! client of the wire protocol (Dave, 2026-11-20: "build the pulse
//! client"), as with ALSA's kernel interface: no libpulse between.
//!
//! - **The socket** is `$PULSE_SERVER` when it names one (`unix:/path`),
//!   else `$XDG_RUNTIME_DIR/pulse/native`; the server checks our uid by the
//!   socket. The cookie (`~/.config/pulse/cookie`) is sent when there is
//!   one; pipewire-pulse does not ask for it.
//! - **Outputs** are the server's sinks: a sink's active port names it
//!   ("Speakers", "Headphones"), its description says the rest.
//! - **Volume** is the sink's, one value for every channel; 100 is the
//!   server's normal volume (no boost past it here). **Mute** and **the
//!   default** are the server's own (`SET_SINK_MUTE`, `SET_DEFAULT_SINK`),
//!   so every program's sound follows, and the server remembers them.
//! - **The test tone** is a playback stream to the sink, written as the
//!   server asks for it, drained, then deleted.
//!
//! The wire, as protocol-native.c and tagstruct.c have it: a packet is a
//! 20-byte header (length, channel, offset high and low, flags; big-endian)
//! and its payload. Commands go on channel 0xffffffff as a "tagstruct", a
//! run of values each led by its type's letter; sound goes on its stream's
//! channel as raw samples. We speak version 32 of it.

use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

use crate::{Output, Sound};

const VERSION: u32 = 32;
const CONTROL: u32 = 0xffff_ffff;
const INVALID: u32 = 0xffff_ffff;
/// The server's normal volume (PA_VOLUME_NORM).
const NORM: u32 = 0x1_0000;

// Commands (pulsecore/native-common.h).
const ERROR: u32 = 0;
const REPLY: u32 = 2;
const CREATE_PLAYBACK_STREAM: u32 = 3;
const DELETE_PLAYBACK_STREAM: u32 = 4;
const AUTH: u32 = 8;
const SET_CLIENT_NAME: u32 = 9;
const DRAIN_PLAYBACK_STREAM: u32 = 12;
const GET_SERVER_INFO: u32 = 20;
const GET_SINK_INFO: u32 = 21;
const GET_SINK_INFO_LIST: u32 = 22;
const SET_SINK_VOLUME: u32 = 36;
const SET_SINK_MUTE: u32 = 39;
const SET_DEFAULT_SINK: u32 = 44;
const REQUEST: u32 = 61;

/// Signed 16-bit little-endian samples (PA_SAMPLE_S16LE).
const S16LE: u8 = 3;

// The tags (pulsecore/tagstruct.h).
const T_STRING: u8 = b't';
const T_NULL: u8 = b'N';
const T_U32: u8 = b'L';
const T_U8: u8 = b'B';
const T_U64: u8 = b'R';
const T_S64: u8 = b'r';
const T_SPEC: u8 = b'a';
const T_ARB: u8 = b'x';
const T_TRUE: u8 = b'1';
const T_FALSE: u8 = b'0';
const T_TIME: u8 = b'T';
const T_USEC: u8 = b'U';
const T_MAP: u8 = b'm';
const T_CVOL: u8 = b'v';
const T_PROPS: u8 = b'P';
const T_VOL: u8 = b'V';
const T_FORMAT: u8 = b'f';

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("sound server: {what}"))
}

/// A tagstruct being written.
#[derive(Default)]
pub(crate) struct Put(pub(crate) Vec<u8>);

impl Put {
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.push(T_U32);
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.extend_from_slice(&[T_U8, v]);
        self
    }
    fn bool(&mut self, v: bool) -> &mut Self {
        self.0.push(if v { T_TRUE } else { T_FALSE });
        self
    }
    fn str(&mut self, s: Option<&str>) -> &mut Self {
        match s {
            Some(s) => {
                self.0.push(T_STRING);
                self.0.extend(s.bytes().filter(|b| *b != 0));
                self.0.push(0);
            }
            None => self.0.push(T_NULL),
        }
        self
    }
    fn arbitrary(&mut self, b: &[u8]) -> &mut Self {
        self.0.push(T_ARB);
        self.0.extend_from_slice(&(b.len() as u32).to_be_bytes());
        self.0.extend_from_slice(b);
        self
    }
    fn spec(&mut self, channels: u8, rate: u32) -> &mut Self {
        self.0.extend_from_slice(&[T_SPEC, S16LE, channels]);
        self.0.extend_from_slice(&rate.to_be_bytes());
        self
    }
    /// The usual channel positions for a count (mono; front left and right; then aux).
    fn map(&mut self, channels: u8) -> &mut Self {
        self.0.extend_from_slice(&[T_MAP, channels]);
        for c in 0..channels {
            // PA_CHANNEL_POSITION_MONO 0, FRONT_LEFT 1, FRONT_RIGHT 2, AUX0 12...
            self.0.push(match (channels, c) {
                (1, _) => 0,
                (_, 0) => 1,
                (_, 1) => 2,
                (_, c) => 12 + (c - 2),
            });
        }
        self
    }
    fn cvolume(&mut self, channels: u8, v: u32) -> &mut Self {
        self.0.extend_from_slice(&[T_CVOL, channels]);
        for _ in 0..channels {
            self.0.extend_from_slice(&v.to_be_bytes());
        }
        self
    }
    fn props(&mut self, pairs: &[(&str, &str)]) -> &mut Self {
        self.0.push(T_PROPS);
        for (k, v) in pairs {
            self.str(Some(k));
            let mut value = v.as_bytes().to_vec();
            value.push(0);
            self.u32(value.len() as u32);
            self.arbitrary(&value);
        }
        self.0.push(T_NULL);
        self
    }
}

/// A tagstruct being read.
pub(crate) struct Get<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Get<'a> {
    pub(crate) fn new(b: &'a [u8]) -> Get<'a> {
        Get { b, at: 0 }
    }
    fn done(&self) -> bool {
        self.at >= self.b.len()
    }
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        let s = self.b.get(self.at..self.at + n).ok_or_else(|| bad("message cut short"))?;
        self.at += n;
        Ok(s)
    }
    fn tag(&mut self, want: u8) -> io::Result<()> {
        match self.take(1)?[0] {
            t if t == want => Ok(()),
            t => Err(bad(&format!("expected {:?}, found {:?}", want as char, t as char))),
        }
    }
    fn be32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> io::Result<u32> {
        self.tag(T_U32)?;
        self.be32()
    }
    fn u8(&mut self) -> io::Result<u8> {
        self.tag(T_U8)?;
        Ok(self.take(1)?[0])
    }
    fn bool(&mut self) -> io::Result<bool> {
        match self.take(1)?[0] {
            T_TRUE => Ok(true),
            T_FALSE => Ok(false),
            t => Err(bad(&format!("expected a yes or no, found {:?}", t as char))),
        }
    }
    fn str(&mut self) -> io::Result<Option<String>> {
        match self.take(1)?[0] {
            T_NULL => Ok(None),
            T_STRING => {
                let rest = &self.b[self.at..];
                let end = rest.iter().position(|b| *b == 0).ok_or_else(|| bad("a string without its end"))?;
                let s = String::from_utf8_lossy(&rest[..end]).into_owned();
                self.at += end + 1;
                Ok(Some(s))
            }
            t => Err(bad(&format!("expected a string, found {:?}", t as char))),
        }
    }
    fn cvolume(&mut self) -> io::Result<Vec<u32>> {
        self.tag(T_CVOL)?;
        let n = self.take(1)?[0] as usize;
        (0..n).map(|_| self.be32()).collect()
    }
    fn props(&mut self) -> io::Result<Vec<(String, String)>> {
        self.tag(T_PROPS)?;
        let mut out = Vec::new();
        loop {
            let Some(key) = self.str()? else { return Ok(out) };
            let len = self.u32()? as usize;
            self.tag(T_ARB)?;
            let n = self.be32()? as usize;
            if n != len {
                return Err(bad("a property's length disagrees"));
            }
            let v = self.take(n)?;
            out.push((key, String::from_utf8_lossy(v.strip_suffix(&[0]).unwrap_or(v)).into_owned()));
        }
    }
    /// Any one value, not looked at.
    fn skip(&mut self) -> io::Result<()> {
        let t = *self.b.get(self.at).ok_or_else(|| bad("message cut short"))?;
        match t {
            T_STRING | T_NULL => {
                self.str()?;
            }
            T_PROPS => {
                self.props()?;
            }
            T_CVOL => {
                self.cvolume()?;
            }
            T_FORMAT => {
                self.at += 1;
                self.u8()?;
                self.props()?;
            }
            _ => {
                self.at += 1;
                let n = match t {
                    T_U32 | T_VOL => 4,
                    T_U8 => 1,
                    T_U64 | T_S64 | T_USEC => 8,
                    T_TIME => 8,
                    T_SPEC => 6,
                    T_TRUE | T_FALSE => 0,
                    T_MAP => self.b.get(self.at).map(|c| 1 + *c as usize).ok_or_else(|| bad("message cut short"))?,
                    T_ARB => {
                        4 + self
                            .b
                            .get(self.at..self.at + 4)
                            .map(|l| u32::from_be_bytes(l.try_into().unwrap()) as usize)
                            .ok_or_else(|| bad("message cut short"))?
                    }
                    other => return Err(bad(&format!("unknown tag {:?}", other as char))),
                };
                self.take(n)?;
            }
        }
        Ok(())
    }
    fn skip_n(&mut self, n: usize) -> io::Result<()> {
        (0..n).try_for_each(|_| self.skip())
    }
}

/// A sink as the list gives it (protocol version 32).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sink {
    pub(crate) index: u32,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) volume: Vec<u32>,
    pub(crate) muted: bool,
    /// The active port's description ("Speakers", "Headphones").
    pub(crate) port: Option<String>,
}

/// One sink's entry, from where it starts in a reply.
pub(crate) fn read_sink(g: &mut Get) -> io::Result<Sink> {
    let index = g.u32()?;
    let name = g.str()?.unwrap_or_default();
    let description = g.str()?.unwrap_or_default();
    g.skip_n(3)?; // sample spec, channel map, owner module
    let volume = g.cvolume()?;
    let muted = g.bool()?;
    g.skip_n(5)?; // monitor source and its name, latency, driver, flags
    g.skip_n(2)?; // (13) properties, configured latency
    g.skip_n(4)?; // (15) base volume, state, volume steps, card
    // (16) ports: name, description, priority, (24) available; the active one's name.
    let ports = g.u32()?;
    let mut list = Vec::new();
    for _ in 0..ports {
        let pname = g.str()?.unwrap_or_default();
        let pdesc = g.str()?.unwrap_or_default();
        g.skip_n(2)?;
        list.push((pname, pdesc));
    }
    let active = g.str()?;
    let port = active.and_then(|a| list.into_iter().find(|(n, _)| *n == a).map(|(_, d)| d)).filter(|d| !d.is_empty());
    // (21) formats.
    let formats = g.u8()?;
    g.skip_n(formats as usize)?;
    Ok(Sink { index, name, description, volume, muted, port })
}

/// 0..100 from a sink's channel volumes (their mean against the normal).
pub(crate) fn percent(v: &[u32]) -> u8 {
    if v.is_empty() {
        return 0;
    }
    let mean = v.iter().map(|x| *x as u64).sum::<u64>() / v.len() as u64;
    ((mean * 100 + NORM as u64 / 2) / NORM as u64).min(100) as u8
}

pub struct Pulse {
    sock: UnixStream,
    tag: u32,
    /// What the server called itself ("PulseAudio (on PipeWire 1.6.11)").
    pub server: String,
}

fn socket_path() -> Option<PathBuf> {
    if let Some(s) = std::env::var_os("PULSE_SERVER") {
        let s = s.to_string_lossy().into_owned();
        // Only a local socket: never a network server.
        return s.strip_prefix("unix:").map(PathBuf::from);
    }
    std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("pulse/native"))
}

fn cookie() -> Vec<u8> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let places = [
        std::env::var_os("PULSE_COOKIE").map(PathBuf::from),
        home.as_ref().map(|h| h.join(".config/pulse/cookie")),
        home.as_ref().map(|h| h.join(".pulse-cookie")),
    ];
    places
        .into_iter()
        .flatten()
        .find_map(|p| std::fs::read(p).ok().filter(|c| c.len() == 256))
        .unwrap_or_else(|| vec![0; 256])
}

impl Pulse {
    pub fn open() -> Option<Pulse> {
        match Pulse::connect() {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("vergence-sound: {e}");
                None
            }
        }
    }

    fn connect() -> io::Result<Pulse> {
        let path = socket_path().ok_or_else(|| bad("no socket named"))?;
        let sock = UnixStream::connect(&path)?;
        sock.set_read_timeout(Some(Duration::from_secs(3)))?;
        sock.set_write_timeout(Some(Duration::from_secs(3)))?;
        let mut p = Pulse { sock, tag: 0, server: String::new() };
        let r = p.ask(AUTH, |t| {
            t.u32(VERSION).arbitrary(&cookie());
        })?;
        let theirs = Get::new(&r).u32()? & 0xffff;
        if theirs < VERSION {
            return Err(bad(&format!("the server speaks version {theirs}, older than ours")));
        }
        p.ask(SET_CLIENT_NAME, |t| {
            t.props(&[("application.name", "Vergence"), ("application.id", "vergence")]);
        })?;
        let (name, version, _) = p.server_info()?;
        p.server = format!("{name} {version}");
        Ok(p)
    }

    fn send(&mut self, channel: u32, payload: &[u8]) -> io::Result<()> {
        let mut head = [0u8; 20];
        head[..4].copy_from_slice(&(payload.len() as u32).to_be_bytes());
        head[4..8].copy_from_slice(&channel.to_be_bytes());
        let mut all = head.to_vec();
        all.extend_from_slice(payload);
        self.sock.write_all(&all)
    }

    /// The next control packet's payload (sound sent to us, which we never
    /// ask for, is read and dropped).
    fn read_control(&mut self) -> io::Result<Vec<u8>> {
        loop {
            let mut head = [0u8; 20];
            self.sock.read_exact(&mut head)?;
            let len = u32::from_be_bytes(head[..4].try_into().unwrap()) as usize;
            let channel = u32::from_be_bytes(head[4..8].try_into().unwrap());
            if len > 16 << 20 {
                return Err(bad("a packet too large"));
            }
            let mut body = vec![0; len];
            self.sock.read_exact(&mut body)?;
            if channel == CONTROL {
                return Ok(body);
            }
        }
    }

    /// A command and its reply's values (after the command and tag); the
    /// server's own messages in between (requests, events) go to `other`.
    fn ask_with(
        &mut self,
        command: u32,
        args: impl FnOnce(&mut Put),
        mut other: impl FnMut(&mut Pulse, u32, &[u8]) -> io::Result<()>,
    ) -> io::Result<Vec<u8>> {
        self.tag = self.tag.wrapping_add(1) & 0x7fff_ffff;
        let tag = self.tag;
        let mut t = Put::default();
        t.u32(command).u32(tag);
        args(&mut t);
        self.send(CONTROL, &t.0)?;
        loop {
            let body = self.read_control()?;
            let mut g = Get::new(&body);
            let cmd = g.u32()?;
            let their = g.u32()?;
            let rest = body[g.at..].to_vec();
            match cmd {
                REPLY if their == tag => return Ok(rest),
                ERROR if their == tag => {
                    let code = Get::new(&rest).u32().unwrap_or(0);
                    return Err(io::Error::other(format!("sound server refused (error {code})")));
                }
                _ => other(self, cmd, &rest)?,
            }
        }
    }

    fn ask(&mut self, command: u32, args: impl FnOnce(&mut Put)) -> io::Result<Vec<u8>> {
        self.ask_with(command, args, |_, _, _| Ok(()))
    }

    /// The server's name, version and default sink's name.
    fn server_info(&mut self) -> io::Result<(String, String, String)> {
        let r = self.ask(GET_SERVER_INFO, |_| {})?;
        let mut g = Get::new(&r);
        let name = g.str()?.unwrap_or_default();
        let version = g.str()?.unwrap_or_default();
        g.skip_n(3)?; // user, host, sample spec
        let default = g.str()?.unwrap_or_default();
        Ok((name, version, default))
    }

    pub(crate) fn sinks(&mut self) -> io::Result<Vec<Sink>> {
        let r = self.ask(GET_SINK_INFO_LIST, |_| {})?;
        let mut g = Get::new(&r);
        let mut out = Vec::new();
        while !g.done() {
            out.push(read_sink(&mut g)?);
        }
        Ok(out)
    }

    fn sink(&mut self, name: &str) -> io::Result<Sink> {
        let r = self.ask(GET_SINK_INFO, |t| {
            t.u32(INVALID).str(Some(name));
        })?;
        read_sink(&mut Get::new(&r))
    }
}

impl Sound for Pulse {
    fn system(&self) -> &'static str {
        "PulseAudio protocol"
    }

    fn outputs(&mut self) -> io::Result<Vec<Output>> {
        let (_, _, default) = self.server_info()?;
        Ok(self
            .sinks()?
            .into_iter()
            .map(|s| Output {
                name: s.port.clone().unwrap_or_else(|| s.description.clone()),
                detail: s.description.clone(),
                volume: percent(&s.volume),
                adjustable: true,
                muted: s.muted,
                default: s.name == default,
                id: s.name,
            })
            .collect())
    }

    fn set_volume(&mut self, id: &str, volume: u8) -> io::Result<()> {
        let s = self.sink(id)?;
        let v = (volume.min(100) as u64 * NORM as u64 / 100) as u32;
        let channels = s.volume.len().clamp(1, 32) as u8;
        self.ask(SET_SINK_VOLUME, |t| {
            t.u32(s.index).str(None).cvolume(channels, v);
        })?;
        Ok(())
    }

    fn set_muted(&mut self, id: &str, muted: bool) -> io::Result<()> {
        let s = self.sink(id)?;
        self.ask(SET_SINK_MUTE, |t| {
            t.u32(s.index).str(None).bool(muted);
        })?;
        Ok(())
    }

    fn set_default(&mut self, id: &str) -> io::Result<()> {
        self.ask(SET_DEFAULT_SINK, |t| {
            t.str(Some(id));
        })?;
        Ok(())
    }

    fn tone(&mut self, id: &str, channels: u32, on: &[u32]) -> io::Result<String> {
        let s = self.sink(id)?;
        let channels = channels.clamp(1, 8) as u8;
        let rate = 48_000;
        let samples = crate::tone_samples(channels as u32, on, rate);
        let bytes: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
        let r = self.ask(CREATE_PLAYBACK_STREAM, |t| {
            t.spec(channels, rate).map(channels).u32(s.index).str(None);
            // Buffer sizes left to the server; started at once; no sync group.
            t.u32(INVALID).bool(false).u32(INVALID).u32(INVALID).u32(INVALID).u32(0);
            t.cvolume(channels, NORM);
            // (12) no remap, no remix, fix format, rate, channels, no move, variable rate.
            for _ in 0..7 {
                t.bool(false);
            }
            // (13) muted, adjust latency, properties.
            t.bool(false).bool(false).props(&[("media.name", "Test tone"), ("media.role", "event")]);
            // (14) volume set, early requests; (15) muted set, don't inhibit
            // suspend, fail on suspend; (17) relative volume; (18) passthrough.
            for _ in 0..7 {
                t.bool(false);
            }
            // (21) no formats: the sample spec says it.
            t.u8(0);
        })?;
        let mut g = Get::new(&r);
        let channel = g.u32()?;
        g.u32()?; // its sink input
        let mut missing = g.u32()? as usize;
        let mut sent = 0;
        let frame = channels as usize * 2;
        while sent < bytes.len() {
            if missing == 0 {
                // Wait for the server to ask for more.
                let body = self.read_control()?;
                let mut g = Get::new(&body);
                if g.u32()? == REQUEST {
                    g.u32()?;
                    if g.u32()? == channel {
                        missing += g.u32()? as usize;
                    }
                }
                continue;
            }
            let n = (missing.min(bytes.len() - sent).min(64 * 1024) / frame * frame).max(frame.min(bytes.len() - sent));
            self.send(channel, &bytes[sent..sent + n])?;
            sent += n;
            missing = missing.saturating_sub(n);
        }
        self.sock.set_read_timeout(Some(Duration::from_secs(5)))?;
        let drained = self.ask_with(
            DRAIN_PLAYBACK_STREAM,
            |t| {
                t.u32(channel);
            },
            |_, _, _| Ok(()),
        );
        let _ = self.ask(DELETE_PLAYBACK_STREAM, |t| {
            t.u32(channel);
        });
        self.sock.set_read_timeout(Some(Duration::from_secs(3)))?;
        drained?;
        Ok(format!("{channels} channels at {rate} Hz, through {}", self.server))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink entry as pipewire-pulse sends it at version 32.
    fn sink_bytes() -> Vec<u8> {
        let mut t = Put::default();
        t.u32(53).str(Some("alsa_output.pci.analog-stereo")).str(Some("Built-in Audio Analog Stereo"));
        t.spec(2, 48_000).map(2).u32(INVALID);
        t.cvolume(2, NORM * 2 / 5).bool(false);
        t.u32(54).str(Some("alsa_output.pci.analog-stereo.monitor"));
        t.0.push(T_USEC);
        t.0.extend_from_slice(&0u64.to_be_bytes());
        t.str(Some("PipeWire")).u32(0x1);
        t.props(&[("device.class", "sound")]);
        t.0.push(T_USEC);
        t.0.extend_from_slice(&0u64.to_be_bytes());
        t.0.push(T_VOL);
        t.0.extend_from_slice(&NORM.to_be_bytes());
        t.u32(0).u32(65537).u32(57);
        t.u32(2);
        t.str(Some("analog-output-speaker")).str(Some("Speakers")).u32(100).u32(0);
        t.str(Some("analog-output-headphones")).str(Some("Headphones")).u32(200).u32(1);
        t.str(Some("analog-output-speaker"));
        t.u8(1);
        t.0.push(T_FORMAT);
        t.u8(1).props(&[]);
        t.0
    }

    #[test]
    fn a_sink_is_read() {
        let b = sink_bytes();
        let mut g = Get::new(&b);
        let s = read_sink(&mut g).unwrap();
        assert!(g.done(), "the whole entry read");
        assert_eq!(s.index, 53);
        assert_eq!(s.port.as_deref(), Some("Speakers"), "named by its active port");
        assert_eq!(percent(&s.volume), 40);
        assert!(!s.muted);
        // Two in a row, as the list sends them.
        let two = [b.clone(), b].concat();
        let mut g = Get::new(&two);
        read_sink(&mut g).unwrap();
        read_sink(&mut g).unwrap();
        assert!(g.done());
    }

    #[test]
    fn values_round_trip() {
        let mut t = Put::default();
        t.u32(7).str(Some("é")).str(None).bool(true).cvolume(2, 5).props(&[("a", "b")]).u8(9);
        let mut g = Get::new(&t.0);
        assert_eq!(g.u32().unwrap(), 7);
        assert_eq!(g.str().unwrap().as_deref(), Some("é"));
        assert_eq!(g.str().unwrap(), None);
        assert!(g.bool().unwrap());
        assert_eq!(g.cvolume().unwrap(), vec![5, 5]);
        assert_eq!(g.props().unwrap(), vec![("a".to_string(), "b".to_string())]);
        assert_eq!(g.u8().unwrap(), 9);
        assert!(g.done());
    }

    #[test]
    fn short_or_strange_messages_are_errors_not_panics() {
        let b = sink_bytes();
        for cut in 0..b.len() {
            let _ = read_sink(&mut Get::new(&b[..cut]));
        }
        let mut junk = 0x9e37_79b9u32;
        for _ in 0..2000 {
            let bytes: Vec<u8> = (0..64)
                .map(|_| {
                    junk ^= junk << 13;
                    junk ^= junk >> 17;
                    junk ^= junk << 5;
                    junk as u8
                })
                .collect();
            let _ = read_sink(&mut Get::new(&bytes));
            let mut g = Get::new(&bytes);
            while g.skip().is_ok() && !g.done() {}
        }
    }

    #[test]
    fn volume_as_percent() {
        assert_eq!(percent(&[NORM, NORM]), 100);
        assert_eq!(percent(&[NORM * 3]), 100, "past normal shows as 100");
        assert_eq!(percent(&[0]), 0);
        assert_eq!(percent(&[]), 0);
    }
}

/// Sound to the default sink, on a connection of its own: written as the
/// server asks for more (its REQUEST packets), so writing paces the playing.
pub struct PulseStream {
    p: Pulse,
    channel: u32,
    missing: usize,
    rate: u32,
    channels: u32,
}

pub fn stream(rate: u32, channels: u32) -> io::Result<Box<dyn crate::Stream>> {
    let mut p = Pulse::connect()?;
    let sink = p.sink("@DEFAULT_SINK@")?;
    let ch = channels.clamp(1, 8) as u8;
    let r = p.ask(CREATE_PLAYBACK_STREAM, |t| {
        t.spec(ch, rate).map(ch).u32(sink.index).str(None);
        t.u32(INVALID).bool(false).u32(INVALID).u32(INVALID).u32(INVALID).u32(0);
        t.cvolume(ch, NORM);
        for _ in 0..7 {
            t.bool(false);
        }
        t.bool(false).bool(false).props(&[("media.name", "Preview"), ("media.role", "music")]);
        for _ in 0..7 {
            t.bool(false);
        }
        t.u8(0);
    })?;
    let mut g = Get::new(&r);
    let channel = g.u32()?;
    g.u32()?; // its sink input
    let missing = g.u32()? as usize;
    Ok(Box::new(PulseStream { p, channel, missing, rate, channels: ch as u32 }))
}

impl crate::Stream for PulseStream {
    fn rate(&self) -> u32 {
        self.rate
    }
    fn channels(&self) -> u32 {
        self.channels
    }
    fn write(&mut self, samples: &[i16]) -> io::Result<()> {
        let bytes: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
        let frame = self.channels as usize * 2;
        let mut sent = 0;
        while sent < bytes.len() {
            if self.missing == 0 {
                // Wait for the server to ask for more.
                let body = self.p.read_control()?;
                let mut g = Get::new(&body);
                if g.u32()? == REQUEST {
                    g.u32()?;
                    if g.u32()? == self.channel {
                        self.missing += g.u32()? as usize;
                    }
                }
                continue;
            }
            let n = (self.missing.min(bytes.len() - sent).min(64 * 1024) / frame * frame)
                .max(frame.min(bytes.len() - sent));
            self.p.send(self.channel, &bytes[sent..sent + n])?;
            sent += n;
            self.missing = self.missing.saturating_sub(n);
        }
        Ok(())
    }
}

impl Drop for PulseStream {
    fn drop(&mut self) {
        let channel = self.channel;
        let _ = self.p.ask(DELETE_PLAYBACK_STREAM, |t| {
            t.u32(channel);
        });
    }
}
