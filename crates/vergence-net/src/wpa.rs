//! A client of wpa_supplicant's control socket (settings milestone 1,
//! step 3): the text protocol wpa_cli speaks, over a Unix datagram socket.
//!
//! wpa_supplicant keeps one socket per interface in its control directory:
//! `/run/wpa_supplicant/<iface>` on Void (its default config), and
//! `/var/run/wpa_supplicant/<iface>` on FreeBSD, both for the `wheel`
//! group, so a member of it needs no root. A client binds a socket of its
//! own (wpa_supplicant answers to the sender's address), sends a command,
//! and reads the reply. Events come to a client that sent `ATTACH`, on a
//! socket of their own here ([`Events`]), as wpa_cli does, so a reply is
//! never mistaken for an event.
//!
//! What goes to wpa_supplicant is built here from checked parts only: a
//! network name as hex, a password quoted after its length and characters
//! are checked, network ids as numbers. What comes back is untrusted:
//! names are decoded from wpa_supplicant's escapes, control characters
//! dropped, lengths limited.

use std::io;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Where wpa_supplicant keeps its control sockets: Void's, then FreeBSD's.
pub const DIRS: [&str; 2] = ["/run/wpa_supplicant", "/var/run/wpa_supplicant"];

/// How long a command may take: a scan's results come as an event, so
/// every command answers at once; this is for a wpa_supplicant that hangs.
const TIMEOUT: Duration = Duration::from_secs(3);

/// The largest reply read (scan results with many networks are long).
const MAX_REPLY: usize = 64 * 1024;

/// A known network's `id_str` when it is joined only when chosen, and when
/// it is joined by itself again.
pub const MANUAL: &str = "vergence-manual";
const AUTO: &str = "vergence-auto";

/// The longest network name (IEEE 802.11: 32 bytes).
pub const MAX_SSID: usize = 32;

#[derive(Debug)]
pub enum Error {
    /// No control socket: wpa_supplicant is not running for this interface.
    NotRunning,
    /// The socket is there, but this account may not use it (not in the
    /// group its control directory belongs to).
    Denied,
    /// wpa_supplicant did not answer in time.
    Timeout,
    /// It answered something other than what the command expects.
    Failed(String),
    /// A network name or password that cannot be sent (and why).
    BadInput(&'static str),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotRunning => write!(f, "wpa_supplicant is not running"),
            Error::Denied => write!(f, "this account may not use wpa_supplicant's control socket"),
            Error::Timeout => write!(f, "wpa_supplicant did not answer"),
            Error::Failed(r) => write!(f, "wpa_supplicant said {r:?}"),
            Error::BadInput(why) => write!(f, "{why}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        match e.kind() {
            io::ErrorKind::PermissionDenied => Error::Denied,
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => Error::NotRunning,
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Error::Timeout,
            _ => Error::Io(e),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// The control sockets present: (interface, socket), in the directories'
/// order, each interface once.
pub fn sockets() -> Vec<(String, PathBuf)> {
    sockets_in(&DIRS.map(PathBuf::from))
}

fn sockets_in(dirs: &[PathBuf]) -> Vec<(String, PathBuf)> {
    use std::os::unix::fs::FileTypeExt;
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut found: Vec<(String, PathBuf)> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_socket()))
            .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.path())))
            // Its global socket and Wi-Fi Direct's (p2p-dev-wlp4s0) are no interface's.
            .filter(|(n, _)| !n.starts_with("global") && !n.starts_with("p2p-"))
            .collect();
        found.sort();
        for (n, p) in found {
            if !out.iter().any(|(m, _)| *m == n) {
                out.push((n, p));
            }
        }
    }
    out
}

/// An interface's control socket, if wpa_supplicant runs for it.
pub fn find(iface: &str) -> Option<PathBuf> {
    sockets().into_iter().find(|(n, _)| n == iface).map(|(_, p)| p)
}

/// A client's own socket: in the runtime directory (the user's, 0700),
/// named for this process; removed when the client goes.
fn local_path() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute() && p.is_dir()).unwrap_or_else(std::env::temp_dir);
    dir.join(format!("vergence-wpa-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

/// A connection to one interface's wpa_supplicant, for commands.
pub struct Client {
    sock: UnixDatagram,
    local: PathBuf,
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.local);
    }
}

impl Client {
    /// Connect to the control socket at `path` (see [`find`]).
    pub fn open(path: &Path) -> Result<Client> {
        if !path.exists() {
            return Err(Error::NotRunning);
        }
        let local = local_path();
        let _ = std::fs::remove_file(&local);
        let sock = UnixDatagram::bind(&local)?;
        let client = Client { sock, local };
        client.sock.connect(path)?;
        client.sock.set_read_timeout(Some(TIMEOUT))?;
        client.sock.set_write_timeout(Some(TIMEOUT))?;
        Ok(client)
    }

    /// Send a command, read its reply. Unsolicited event lines (`<N>...`)
    /// that reach this socket are passed over.
    pub fn request(&self, cmd: &str) -> Result<String> {
        self.sock.send(cmd.as_bytes())?;
        let mut buf = vec![0u8; MAX_REPLY];
        loop {
            let n = self.sock.recv(&mut buf)?;
            let reply = String::from_utf8_lossy(&buf[..n]).into_owned();
            if is_event(&reply) {
                continue;
            }
            return Ok(reply);
        }
    }

    /// A command whose reply is OK.
    fn ok(&self, cmd: &str) -> Result<()> {
        let r = self.request(cmd)?;
        if r.trim_end() == "OK" { Ok(()) } else { Err(Error::Failed(clean(r.trim_end(), 80))) }
    }

    pub fn ping(&self) -> Result<()> {
        let r = self.request("PING")?;
        if r.trim_end() == "PONG" { Ok(()) } else { Err(Error::Failed(clean(r.trim_end(), 80))) }
    }

    /// Where the interface stands: joined or not, to what.
    pub fn status(&self) -> Result<Status> {
        Ok(parse_status(&self.request("STATUS")?))
    }

    /// The signal of the network joined, if any.
    pub fn signal(&self) -> Result<Option<Signal>> {
        Ok(parse_signal(&self.request("SIGNAL_POLL")?))
    }

    /// Start a scan; its results are announced as [`Event::ScanResults`].
    /// A scan already going is as good.
    pub fn scan(&self) -> Result<()> {
        let r = self.request("SCAN")?;
        match r.trim_end() {
            "OK" | "FAIL-BUSY" => Ok(()),
            other => Err(Error::Failed(clean(other, 80))),
        }
    }

    /// The networks in range as last scanned, one entry per access point.
    pub fn scan_results(&self) -> Result<Vec<Bss>> {
        Ok(parse_scan_results(&self.request("SCAN_RESULTS")?))
    }

    /// The networks wpa_supplicant knows (its configuration), each with
    /// whether it is joined only when chosen ([`MANUAL`]).
    pub fn networks(&self) -> Result<Vec<Known>> {
        let mut all = parse_networks(&self.request("LIST_NETWORKS")?);
        for k in &mut all {
            k.manual = self.request(&format!("GET_NETWORK {} id_str", k.id)).is_ok_and(|r| r.trim() == format!("\"{MANUAL}\""));
        }
        Ok(all)
    }

    /// Whether a known network is joined by itself when in range, or only
    /// when chosen (Dave, 2026-11-14: a phone's hotspot, as Windows does it).
    /// The mark is the network's `id_str` in wpa_supplicant's own file, so
    /// it outlives everything here; a network marked so is kept disabled
    /// while it is not the one in use ([`Client::keep_manual`]), so turning
    /// it off for the network in use does not drop it.
    pub fn set_auto(&self, id: u32, auto: bool) -> Result<()> {
        let mark = if auto { AUTO } else { MANUAL };
        self.ok(&format!("SET_NETWORK {id} id_str \"{mark}\""))?;
        if auto {
            self.ok(&format!("ENABLE_NETWORK {id}"))?;
        } else {
            self.keep_manual_now()?;
        }
        self.save()
    }

    /// Let every known network be joined again (SELECT_NETWORK disables
    /// the others), but those joined only when chosen.
    pub fn enable_auto(&self) -> Result<()> {
        for k in self.networks()? {
            if k.disabled && !k.manual {
                self.ok(&format!("ENABLE_NETWORK {}", k.id))?;
            }
        }
        Ok(())
    }

    /// Keep the networks joined only when chosen disabled while they are
    /// not in use, once the interface has settled on a network: while one
    /// is being joined (it is not yet the one in use) nothing is touched,
    /// or choosing a hotspot would be undone. True if any changed (saved).
    pub fn keep_manual(&self) -> Result<bool> {
        if !self.status()?.connected() {
            return Ok(false);
        }
        self.keep_manual_now()
    }

    /// The same at once (Wi-Fi just turned off, or the mark just set).
    pub fn keep_manual_now(&self) -> Result<bool> {
        let mut changed = false;
        for k in self.networks()? {
            if k.manual && !k.current && !k.disabled {
                self.ok(&format!("DISABLE_NETWORK {}", k.id))?;
                changed = true;
            }
        }
        if changed {
            self.save()?;
        }
        Ok(changed)
    }

    /// Add a network by its name and password (None for an open one), and
    /// join it now. Its id comes back. The others are left as they are;
    /// once [`Event::Connected`] comes, [`Client::enable_auto`] lets them be
    /// joined again later, and [`Client::save`] keeps the new one. On
    /// [`Event::WrongKey`] the caller may [`Client::forget`] it.
    pub fn join(&self, ssid: &[u8], password: Option<&str>, keys: Keys) -> Result<u32> {
        let ssid_arg = ssid_arg(ssid)?;
        let psk = password.map(psk_arg).transpose()?;
        let r = self.request("ADD_NETWORK")?;
        let id: u32 = r.trim().parse().map_err(|_| Error::Failed(clean(r.trim_end(), 80)))?;
        let set = || -> Result<()> {
            self.ok(&format!("SET_NETWORK {id} ssid {ssid_arg}"))?;
            match &psk {
                Some(p) => {
                    self.ok(&format!("SET_NETWORK {id} psk {p}"))?;
                    if let Some((mgmt, pmf)) = keys.settings() {
                        self.ok(&format!("SET_NETWORK {id} key_mgmt {mgmt}"))?;
                        self.ok(&format!("SET_NETWORK {id} ieee80211w {pmf}"))?;
                    }
                }
                None => self.ok(&format!("SET_NETWORK {id} key_mgmt NONE"))?,
            }
            self.ok(&format!("ENABLE_NETWORK {id}"))?;
            self.ok(&format!("SELECT_NETWORK {id}"))
        };
        if let Err(e) = set() {
            let _ = self.ok(&format!("REMOVE_NETWORK {id}"));
            return Err(e);
        }
        Ok(id)
    }

    /// Ready wpa_supplicant for WPA3 everywhere, and the country's radio
    /// rules (6 GHz needs a country): both WPA3 handshakes (sae_pwe 2: 6 GHz
    /// needs the newer, hash-to-element), and the country if one is given.
    /// Kept in its file by the next save.
    pub fn tune(&self, country: Option<&str>) -> Result<()> {
        self.ok("SET sae_pwe 2")?;
        if let Some(c) = country {
            if !country_ok(c) {
                return Err(Error::BadInput("a country is two capital letters, as US"));
            }
            self.ok(&format!("SET country {c}"))?;
        }
        Ok(())
    }

    /// Known networks joined as WPA2 alone whose access points in range take
    /// WPA3 too are given it (and its management frame protection), so their
    /// 6 GHz access points can be used. The ids changed come back (saved).
    pub fn upgrade_known(&self, in_range: &[Bss]) -> Result<Vec<u32>> {
        let nets = by_network(in_range.to_vec());
        let mut changed = Vec::new();
        for k in self.networks()? {
            let Some(b) = nets.iter().find(|b| b.ssid_raw == k.ssid_raw) else { continue };
            let Some((mgmt, pmf)) = b.keys.settings() else { continue };
            let now = self.request(&format!("GET_NETWORK {} key_mgmt", k.id))?;
            // An open network (NONE) or one already with SAE is left alone.
            if now.contains("SAE") || now.contains("NONE") || now.starts_with("FAIL") {
                continue;
            }
            self.ok(&format!("SET_NETWORK {} key_mgmt {mgmt}", k.id))?;
            self.ok(&format!("SET_NETWORK {} ieee80211w {pmf}", k.id))?;
            changed.push(k.id);
        }
        if !changed.is_empty() {
            self.save()?;
        }
        Ok(changed)
    }

    /// The card's power saving on or off (wpa_supplicant's `SET ps`, which
    /// its nl80211 driver passes to the kernel: Linux). Off: quicker
    /// replies, a little more power. Not kept by wpa_supplicant: set it
    /// again after the card resets. FreeBSD's wpa_supplicant refuses it.
    pub fn set_power_save(&self, on: bool) -> Result<()> {
        self.ok(if on { "SET ps 1" } else { "SET ps 0" })
    }

    /// Join the network in use again (after its settings changed).
    pub fn reassociate(&self) -> Result<()> {
        self.ok("REASSOCIATE")
    }

    /// Join a network wpa_supplicant already knows.
    pub fn select(&self, id: u32) -> Result<()> {
        self.ok(&format!("SELECT_NETWORK {id}"))
    }

    /// Let every known network be joined again (SELECT_NETWORK disables the others).
    pub fn enable_all(&self) -> Result<()> {
        self.ok("ENABLE_NETWORK all")
    }

    /// Forget a known network, and keep that.
    pub fn forget(&self, id: u32) -> Result<()> {
        self.ok(&format!("REMOVE_NETWORK {id}"))?;
        self.save()
    }

    /// Write the known networks to wpa_supplicant's file (`update_config=1`).
    pub fn save(&self) -> Result<()> {
        self.ok("SAVE_CONFIG")
    }

    pub fn disconnect(&self) -> Result<()> {
        self.ok("DISCONNECT")
    }

    pub fn reconnect(&self) -> Result<()> {
        self.ok("RECONNECT")
    }
}

/// A connection that hears wpa_supplicant's events.
pub struct Events {
    client: Client,
}

impl Drop for Events {
    fn drop(&mut self) {
        let _ = self.client.sock.send(b"DETACH");
    }
}

impl Events {
    pub fn attach(path: &Path) -> Result<Events> {
        let client = Client::open(path)?;
        client.ok("ATTACH")?;
        Ok(Events { client })
    }

    /// The next event; None if none came within `wait`.
    pub fn next(&self, wait: Duration) -> Result<Option<Event>> {
        self.client.sock.set_read_timeout(Some(wait.max(Duration::from_millis(1))))?;
        let mut buf = vec![0u8; 4096];
        loop {
            let n = match self.client.sock.recv(&mut buf) {
                Ok(n) => n,
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => return Ok(None),
                Err(e) => return Err(e.into()),
            };
            let text = String::from_utf8_lossy(&buf[..n]);
            if is_event(&text) {
                return Ok(Some(parse_event(&text)));
            }
        }
    }
}

/// What wpa_supplicant tells an attached client.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A scan finished: read [`Client::scan_results`].
    ScanResults,
    /// Joined a network.
    Connected { bssid: String, id: Option<u32> },
    Disconnected { bssid: String, reason: Option<u32> },
    /// The network's password was refused.
    WrongKey { id: Option<u32> },
    /// None of the known networks is in range.
    NotFound,
    /// wpa_supplicant is stopping.
    Terminating,
    /// Anything else, cleaned.
    Other(String),
}

fn is_event(text: &str) -> bool {
    text.starts_with('<') && text[1..].find('>').is_some_and(|i| text[1..1 + i].bytes().all(|b| b.is_ascii_digit()))
}

/// `key=value` out of an event's words.
fn word<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.split_whitespace().find_map(|w| w.strip_prefix(key)?.strip_prefix('='))
}

pub fn parse_event(text: &str) -> Event {
    let body = text.find('>').map_or(text, |i| &text[i + 1..]).trim_end();
    let mac = |s: Option<&str>| s.filter(|m| is_mac(m)).unwrap_or("").to_string();
    if body.starts_with("CTRL-EVENT-SCAN-RESULTS") {
        Event::ScanResults
    } else if body.starts_with("CTRL-EVENT-CONNECTED") {
        // "CTRL-EVENT-CONNECTED - Connection to aa:bb:cc:dd:ee:ff completed [id=0 id_str=]"
        let bssid = mac(body.split_whitespace().find(|w| is_mac(w)));
        let id = word(&body.replace('[', " "), "id").and_then(|v| v.parse().ok());
        Event::Connected { bssid, id }
    } else if body.starts_with("CTRL-EVENT-DISCONNECTED") {
        Event::Disconnected { bssid: mac(word(body, "bssid")), reason: word(body, "reason").and_then(|v| v.parse().ok()) }
    } else if body.starts_with("CTRL-EVENT-SSID-TEMP-DISABLED") && word(body, "reason") == Some("WRONG_KEY") {
        Event::WrongKey { id: word(body, "id").and_then(|v| v.parse().ok()) }
    } else if body.starts_with("CTRL-EVENT-NETWORK-NOT-FOUND") {
        Event::NotFound
    } else if body.starts_with("CTRL-EVENT-TERMINATING") {
        Event::Terminating
    } else {
        Event::Other(clean(body, 200))
    }
}

fn is_mac(s: &str) -> bool {
    s.len() == 17 && s.split(':').count() == 6 && s.split(':').all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Text from wpa_supplicant as shown: control characters dropped, at most `max` characters.
fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// Undo wpa_supplicant's escapes (printf_encode: `\\`, `\"`, `\e`, `\n`,
/// `\r`, `\t`, `\xHH`) into the name's bytes, at most [`MAX_SSID`].
pub fn unescape(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() && out.len() < MAX_SSID {
        if b[i] == b'\\' && i + 1 < b.len() {
            match b[i + 1] {
                b'x' if i + 3 < b.len() && b[i + 2].is_ascii_hexdigit() && b[i + 3].is_ascii_hexdigit() => {
                    out.push(u8::from_str_radix(&s[i + 2..i + 4], 16).unwrap_or(b'?'));
                    i += 4;
                    continue;
                }
                b'n' => out.push(b'\n'),
                b'r' => out.push(b'\r'),
                b't' => out.push(b'\t'),
                b'e' => out.push(0x1b),
                c => out.push(c),
            }
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// A network name's bytes as shown: UTF-8 where it is, control characters dropped.
pub fn ssid_text(raw: &[u8]) -> String {
    clean(&String::from_utf8_lossy(raw), MAX_SSID)
}

/// A network name as sent: hex, which wpa_supplicant takes for any bytes.
pub fn ssid_arg(ssid: &[u8]) -> Result<String> {
    if ssid.is_empty() || ssid.len() > MAX_SSID {
        return Err(Error::BadInput("a network name is 1 to 32 bytes"));
    }
    Ok(ssid.iter().map(|b| format!("{b:02x}")).collect())
}

/// A password as sent: 8 to 63 printable ASCII characters, quoted (WPA's
/// passphrase), or the 64 hex digits of a raw key.
pub fn psk_arg(password: &str) -> Result<String> {
    if password.len() == 64 && password.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(password.to_string());
    }
    if !password.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err(Error::BadInput("a Wi-Fi password is plain letters, digits, spaces and punctuation"));
    }
    if !(8..=63).contains(&password.len()) {
        return Err(Error::BadInput("a Wi-Fi password is 8 to 63 characters"));
    }
    Ok(format!("\"{password}\""))
}

/// The interface's state (STATUS).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    /// wpa_supplicant's word: COMPLETED (joined), SCANNING, ASSOCIATING,
    /// 4WAY_HANDSHAKE, DISCONNECTED, INACTIVE, INTERFACE_DISABLED...
    pub state: String,
    pub ssid: Option<String>,
    pub bssid: Option<String>,
    pub freq: Option<u32>,
    /// The known network's id.
    pub id: Option<u32>,
    pub key_mgmt: Option<String>,
    pub ip: Option<String>,
    /// The interface's own hardware address.
    pub address: Option<String>,
}

impl Status {
    pub fn connected(&self) -> bool {
        self.state == "COMPLETED"
    }
}

pub fn parse_status(text: &str) -> Status {
    let mut s = Status::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k {
            "wpa_state" => s.state = clean(v, 32),
            "ssid" => s.ssid = Some(ssid_text(&unescape(v))),
            "bssid" if is_mac(v) => s.bssid = Some(v.to_string()),
            "freq" => s.freq = v.parse().ok(),
            "id" => s.id = v.parse().ok(),
            "key_mgmt" => s.key_mgmt = Some(clean(v, 32)),
            "ip_address" => s.ip = v.parse::<std::net::IpAddr>().ok().map(|a| a.to_string()),
            "address" if is_mac(v) => s.address = Some(v.to_string()),
            _ => {}
        }
    }
    s
}

/// The joined network's signal (SIGNAL_POLL).
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    /// dBm.
    pub rssi: i32,
    pub link_mbps: Option<u32>,
    pub freq: Option<u32>,
}

pub fn parse_signal(text: &str) -> Option<Signal> {
    let get = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('='));
    let rssi = get("RSSI")?.parse().ok()?;
    Some(Signal { rssi, link_mbps: get("LINKSPEED").and_then(|v| v.parse().ok()), freq: get("FREQUENCY").and_then(|v| v.parse().ok()) })
}

/// How a network is protected, from its scan flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    Open,
    /// WPA, WPA2 or WPA3 with a password (PSK or SAE).
    Password,
    /// WPA with a user name and password (802.1X, EAP): not joined from the page yet.
    Enterprise,
    /// WEP: not joined (broken since 2001).
    Wep,
}

/// A network in range: one access point of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Bss {
    pub bssid: String,
    pub freq: u32,
    /// dBm.
    pub signal: i32,
    pub security: Security,
    /// Which password handshakes it takes (by_network: across every access
    /// point of the network).
    pub keys: Keys,
    /// Its name as shown, and its bytes as sent back to join it.
    pub ssid: String,
    pub ssid_raw: Vec<u8>,
}

/// The password handshakes a network takes: WPA2's (PSK), WPA3's (SAE), or
/// both. 6 GHz takes WPA3 only, so a network joined as WPA2 alone never
/// uses its 6 GHz access points (found on Hubble, 2026-11-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Keys {
    pub psk: bool,
    pub sae: bool,
}

impl Keys {
    pub fn of(flags: &str) -> Keys {
        Keys { psk: flags.contains("PSK"), sae: flags.contains("SAE") }
    }

    /// wpa_supplicant's key_mgmt and ieee80211w (management frame
    /// protection: optional with both, required for WPA3 alone), or None
    /// for WPA2 alone (its defaults).
    pub fn settings(self) -> Option<(&'static str, &'static str)> {
        match (self.psk, self.sae) {
            (true, true) => Some(("WPA-PSK SAE", "1")),
            (false, true) => Some(("SAE", "2")),
            _ => None,
        }
    }
}

pub fn security_of(flags: &str) -> Security {
    if flags.contains("EAP") {
        Security::Enterprise
    } else if flags.contains("PSK") || flags.contains("SAE") {
        Security::Password
    } else if flags.contains("WEP") {
        Security::Wep
    } else {
        Security::Open
    }
}

/// SCAN_RESULTS: a header line, then "bssid\tfrequency\tsignal\tflags\tssid".
pub fn parse_scan_results(text: &str) -> Vec<Bss> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let mut f = line.splitn(5, '\t');
        let (Some(bssid), Some(freq), Some(signal), Some(flags)) = (f.next(), f.next(), f.next(), f.next()) else { continue };
        let raw = unescape(f.next().unwrap_or(""));
        let (Ok(freq), Ok(signal)) = (freq.parse(), signal.parse()) else { continue };
        if !is_mac(bssid) {
            continue;
        }
        out.push(Bss { bssid: bssid.to_string(), freq, signal, security: security_of(flags), keys: Keys::of(flags), ssid: ssid_text(&raw), ssid_raw: raw });
    }
    out
}

/// One entry per network name, the strongest access point's, strongest
/// first; hidden networks (no name) left out.
pub fn by_network(mut all: Vec<Bss>) -> Vec<Bss> {
    all.sort_by(|a, b| b.signal.cmp(&a.signal));
    let mut out: Vec<Bss> = Vec::new();
    for b in all {
        if b.ssid_raw.is_empty() {
            continue;
        }
        match out.iter_mut().find(|o| o.ssid_raw == b.ssid_raw) {
            // The handshakes of every access point of it.
            Some(o) => {
                o.keys.psk |= b.keys.psk;
                o.keys.sae |= b.keys.sae;
            }
            None => out.push(b),
        }
    }
    out
}

/// A country as wpa_supplicant takes it: two capital letters (ISO 3166).
pub fn country_ok(c: &str) -> bool {
    c.len() == 2 && c.bytes().all(|b| b.is_ascii_uppercase())
}

/// A network wpa_supplicant knows.
#[derive(Debug, Clone, PartialEq)]
pub struct Known {
    pub id: u32,
    pub ssid: String,
    pub ssid_raw: Vec<u8>,
    pub current: bool,
    pub disabled: bool,
    /// Turned away for now (a wrong password, say).
    pub temp_disabled: bool,
    /// Joined only when chosen, never by itself ([`Client::set_auto`]).
    pub manual: bool,
}

/// LIST_NETWORKS: a header, then "id\tssid\tbssid\tflags".
pub fn parse_networks(text: &str) -> Vec<Known> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.splitn(4, '\t').collect();
        let Some(Ok(id)) = f.first().map(|v| v.parse()) else { continue };
        let raw = unescape(f.get(1).copied().unwrap_or(""));
        let flags = f.get(3).copied().unwrap_or("");
        out.push(Known { id, ssid: ssid_text(&raw), ssid_raw: raw, current: flags.contains("[CURRENT]"), disabled: flags.contains("[DISABLED]"), temp_disabled: flags.contains("[TEMP-DISABLED]"), manual: false });
    }
    out
}

/// Bars for a signal in dBm, 0 to 4.
pub fn bars(dbm: i32) -> u8 {
    match dbm {
        d if d >= -55 => 4,
        d if d >= -67 => 3,
        d if d >= -75 => 2,
        d if d >= -85 => 1,
        _ => 0,
    }
}

/// The band and channel of a frequency in MHz.
pub fn band_channel(freq: u32) -> Option<(&'static str, u32)> {
    match freq {
        2484 => Some(("2.4 GHz", 14)),
        2412..=2472 => Some(("2.4 GHz", (freq - 2407) / 5)),
        5955..=7115 => Some(("6 GHz", (freq - 5950) / 5)),
        5160..=5885 => Some(("5 GHz", (freq - 5000) / 5)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const SCAN: &str = "bssid / frequency / signal level / flags / ssid\n\
        aa:bb:cc:dd:ee:01\t5180\t-48\t[WPA2-PSK-CCMP][ESS]\tHome\n\
        aa:bb:cc:dd:ee:02\t2437\t-71\t[WPA2-PSK-CCMP][ESS]\tHome\n\
        aa:bb:cc:dd:ee:03\t2462\t-80\t[ESS]\tCaf\\xc3\\xa9 \\\"free\\\"\n\
        aa:bb:cc:dd:ee:04\t5745\t-60\t[WPA2-EAP-CCMP][ESS]\tWork\n\
        aa:bb:cc:dd:ee:05\t2412\t-66\t[WPA3-SAE-CCMP][ESS]\t\n\
        aa:bb:cc:dd:ee:06\t6115\t-58\t[RSN-SAE-CCMP][ESS]\tBell\\x07\\x1b[31m\n\
        garbage line\n";

    #[test]
    fn scan_results_are_read() {
        let all = parse_scan_results(SCAN);
        assert_eq!(all.len(), 6);
        assert_eq!(all[0].ssid, "Home");
        assert_eq!(all[0].security, Security::Password);
        assert_eq!(all[2].ssid, "Café \"free\"");
        assert_eq!(all[2].security, Security::Open);
        assert_eq!(all[3].security, Security::Enterprise);
        assert_eq!(all[5].ssid, "Bell[31m", "control characters dropped");
        assert_eq!(all[5].ssid_raw, b"Bell\x07\x1b[31m", "the bytes kept to join it");
        let nets = by_network(all);
        assert_eq!(nets.iter().map(|b| b.ssid.as_str()).collect::<Vec<_>>(), ["Home", "Bell[31m", "Work", "Café \"free\""]);
        assert_eq!(nets[0].bssid, "aa:bb:cc:dd:ee:01", "the strongest access point of Home");
    }

    #[test]
    fn names_never_run_long() {
        let long = "\\x41".repeat(100);
        assert_eq!(unescape(&long).len(), MAX_SSID);
    }

    #[test]
    fn status_and_signal() {
        let s = parse_status("bssid=aa:bb:cc:dd:ee:01\nfreq=5180\nssid=Home\nid=0\nmode=station\npairwise_cipher=CCMP\nkey_mgmt=WPA2-PSK\nwpa_state=COMPLETED\nip_address=192.168.1.20\naddress=11:22:33:44:55:66\n");
        assert!(s.connected());
        assert_eq!((s.ssid.as_deref(), s.freq, s.id, s.ip.as_deref()), (Some("Home"), Some(5180), Some(0), Some("192.168.1.20")));
        assert!(!parse_status("wpa_state=SCANNING\n").connected());
        assert_eq!(parse_status("ip_address=not an address\n").ip, None);
        let g = parse_signal("RSSI=-52\nLINKSPEED=866\nNOISE=9999\nFREQUENCY=5180\n").unwrap();
        assert_eq!((g.rssi, g.link_mbps, g.freq), (-52, Some(866), Some(5180)));
        assert_eq!(parse_signal("FAIL\n"), None);
    }

    #[test]
    fn known_networks() {
        let k = parse_networks("network id / ssid / bssid / flags\n0\tHome\tany\t[CURRENT]\n1\tOld\tany\t[DISABLED]\n2\tCaf\\xc3\\xa9\tany\t[TEMP-DISABLED]\n");
        assert_eq!(k.len(), 3);
        assert!(k[0].current && !k[0].disabled);
        assert!(k[1].disabled);
        assert_eq!(k[2].ssid, "Café");
        assert!(k[2].temp_disabled);
    }

    #[test]
    fn events() {
        assert_eq!(parse_event("<2>CTRL-EVENT-SCAN-RESULTS "), Event::ScanResults);
        assert_eq!(
            parse_event("<3>CTRL-EVENT-CONNECTED - Connection to aa:bb:cc:dd:ee:01 completed [id=0 id_str=]"),
            Event::Connected { bssid: "aa:bb:cc:dd:ee:01".into(), id: Some(0) }
        );
        assert_eq!(parse_event("<3>CTRL-EVENT-DISCONNECTED bssid=aa:bb:cc:dd:ee:01 reason=3 locally_generated=1"), Event::Disconnected { bssid: "aa:bb:cc:dd:ee:01".into(), reason: Some(3) });
        assert_eq!(parse_event("<3>CTRL-EVENT-SSID-TEMP-DISABLED id=4 ssid=\"Home\" auth_failures=1 duration=10 reason=WRONG_KEY"), Event::WrongKey { id: Some(4) });
        assert_eq!(parse_event("<3>CTRL-EVENT-NETWORK-NOT-FOUND"), Event::NotFound);
        assert_eq!(parse_event("<3>Trying to associate with \x1b[1mx"), Event::Other("Trying to associate with [1mx".into()));
        assert!(is_event("<3>x"));
        assert!(!is_event("OK\n"));
        assert!(!is_event("<a>x"));
    }

    #[test]
    fn what_is_sent_is_checked() {
        assert_eq!(ssid_arg(b"Home").unwrap(), "486f6d65");
        assert!(ssid_arg(b"").is_err());
        assert!(ssid_arg(&[b'a'; 33]).is_err());
        assert_eq!(psk_arg("correct horse").unwrap(), "\"correct horse\"");
        assert_eq!(psk_arg("say \"hi\" now").unwrap(), "\"say \"hi\" now\"", "wpa_supplicant takes up to the last quote");
        assert!(psk_arg("short").is_err());
        assert!(psk_arg(&"x".repeat(64)).is_err(), "64 characters that are not hex");
        assert_eq!(psk_arg(&"a1".repeat(32)).unwrap(), "a1".repeat(32), "a raw key, unquoted");
        assert!(psk_arg("line\nbreak!!").is_err());
        assert!(psk_arg("pässword1").is_err());
        // A password can never smuggle a second command: there is no newline in one.
        assert!(psk_arg("abcdefgh\nSAVE_CONFIG").is_err());
    }

    #[test]
    fn bands_and_bars() {
        assert_eq!(band_channel(2412), Some(("2.4 GHz", 1)));
        assert_eq!(band_channel(2484), Some(("2.4 GHz", 14)));
        assert_eq!(band_channel(5180), Some(("5 GHz", 36)));
        assert_eq!(band_channel(5745), Some(("5 GHz", 149)));
        assert_eq!(band_channel(5955), Some(("6 GHz", 1)));
        assert_eq!(band_channel(6115), Some(("6 GHz", 33)));
        assert_eq!(band_channel(900), None);
        assert_eq!((bars(-40), bars(-60), bars(-70), bars(-80), bars(-90)), (4, 3, 2, 1, 0));
    }

    /// A fake wpa_supplicant: a datagram socket that answers commands from
    /// a table and keeps what it was sent; ATTACH makes it send `events` to
    /// the attached client.
    struct Fake {
        dir: PathBuf,
        path: PathBuf,
        sent: Arc<Mutex<Vec<String>>>,
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fake(answers: Vec<(&'static str, &'static str)>, events: Vec<&'static str>) -> Fake {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("vergence-net-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wlan0");
        let sock = UnixDatagram::bind(&path).unwrap();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let log = sent.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok((n, from)) = sock.recv_from(&mut buf) {
                let cmd = String::from_utf8_lossy(&buf[..n]).into_owned();
                log.lock().unwrap().push(cmd.clone());
                let Some(to) = from.as_pathname().map(Path::to_path_buf) else { continue };
                let reply = answers.iter().find(|(c, _)| cmd.starts_with(c)).map_or("UNKNOWN COMMAND\n", |(_, r)| r);
                // An event slipped in before the reply: the client must pass over it.
                if cmd == "STATUS" {
                    let _ = sock.send_to(b"<3>CTRL-EVENT-BSS-ADDED 1 aa:bb:cc:dd:ee:01", &to);
                }
                let _ = sock.send_to(reply.as_bytes(), &to);
                if cmd == "ATTACH" {
                    for e in &events {
                        let _ = sock.send_to(e.as_bytes(), &to);
                    }
                }
            }
        });
        Fake { dir, path, sent }
    }

    #[test]
    fn a_client_talks_to_a_fake_wpa_supplicant() {
        let f = fake(vec![("PING", "PONG\n"), ("STATUS", "wpa_state=COMPLETED\nssid=Home\n"), ("SCAN_RESULTS", SCAN), ("SCAN", "FAIL-BUSY\n"), ("LIST_NETWORKS", "network id / ssid / bssid / flags\n0\tHome\tany\t[CURRENT]\n")], vec![]);
        let c = Client::open(&f.path).unwrap();
        c.ping().unwrap();
        assert_eq!(c.status().unwrap().ssid.as_deref(), Some("Home"));
        c.scan().unwrap();
        assert_eq!(by_network(c.scan_results().unwrap()).len(), 4);
        assert_eq!(c.networks().unwrap()[0].id, 0);
        let local = c.local.clone();
        drop(c);
        assert!(!local.exists(), "its own socket is removed");
    }

    #[test]
    fn joining_sends_checked_commands_in_order() {
        let f = fake(vec![("ADD_NETWORK", "3\n"), ("SET_NETWORK", "OK\n"), ("ENABLE_NETWORK", "OK\n"), ("SELECT_NETWORK", "OK\n")], vec![]);
        let c = Client::open(&f.path).unwrap();
        assert_eq!(c.join(b"Home", Some("correct horse"), Keys::default()).unwrap(), 3);
        assert_eq!(
            *f.sent.lock().unwrap(),
            ["ADD_NETWORK", "SET_NETWORK 3 ssid 486f6d65", "SET_NETWORK 3 psk \"correct horse\"", "ENABLE_NETWORK 3", "SELECT_NETWORK 3"]
        );
        f.sent.lock().unwrap().clear();
        c.join(b"Cafe", None, Keys::default()).unwrap();
        assert_eq!(f.sent.lock().unwrap()[2], "SET_NETWORK 3 key_mgmt NONE");
        // A bad password is refused before anything is sent.
        f.sent.lock().unwrap().clear();
        assert!(matches!(c.join(b"Home", Some("short"), Keys::default()), Err(Error::BadInput(_))));
        assert!(f.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn a_refused_setting_removes_the_half_made_network() {
        let f = fake(vec![("ADD_NETWORK", "5\n"), ("SET_NETWORK 5 ssid", "OK\n"), ("SET_NETWORK 5 psk", "FAIL\n"), ("REMOVE_NETWORK", "OK\n")], vec![]);
        let c = Client::open(&f.path).unwrap();
        assert!(matches!(c.join(b"Home", Some("correct horse"), Keys::default()), Err(Error::Failed(_))));
        assert_eq!(f.sent.lock().unwrap().last().map(String::as_str), Some("REMOVE_NETWORK 5"));
    }

    #[test]
    fn a_network_joined_only_when_chosen_is_kept_disabled_when_not_in_use() {
        let f = fake(
            vec![
                ("LIST_NETWORKS", "network id / ssid / bssid / flags\n0\tHome\tany\t[CURRENT]\n1\tPhone\tany\t\n2\tOld\tany\t[DISABLED]\n"),
                ("GET_NETWORK 1 id_str", "\"vergence-manual\"\n"),
                ("GET_NETWORK", "FAIL\n"),
                ("SET_NETWORK", "OK\n"),
                ("DISABLE_NETWORK", "OK\n"),
                ("ENABLE_NETWORK", "OK\n"),
                ("SAVE_CONFIG", "OK\n"),
            ],
            vec![],
        );
        let c = Client::open(&f.path).unwrap();
        let k = c.networks().unwrap();
        assert_eq!(k.iter().map(|k| k.manual).collect::<Vec<_>>(), [false, true, false]);
        assert!(c.keep_manual_now().unwrap());
        let sent = f.sent.lock().unwrap().clone();
        assert!(sent.contains(&"DISABLE_NETWORK 1".to_string()));
        assert!(!sent.iter().any(|s| s == "DISABLE_NETWORK 0"), "the network in use is never dropped");
        f.sent.lock().unwrap().clear();
        // After choosing: the others come back, but not the manual one.
        c.enable_auto().unwrap();
        let sent = f.sent.lock().unwrap().clone();
        assert!(sent.contains(&"ENABLE_NETWORK 2".to_string()));
        assert!(!sent.iter().any(|s| s == "ENABLE_NETWORK 1"));
        f.sent.lock().unwrap().clear();
        c.set_auto(1, true).unwrap();
        let sent = f.sent.lock().unwrap().clone();
        assert_eq!(sent[0], "SET_NETWORK 1 id_str \"vergence-auto\"");
        assert_eq!(sent[1], "ENABLE_NETWORK 1");
    }

    #[test]
    fn wpa3_where_the_network_takes_it() {
        // Hubble's dNET1: WPA2+WPA3 on 2.4 and 5 GHz, WPA3 alone on 6 GHz.
        let scan = "bssid / frequency / signal level / flags / ssid\n\
            aa:bb:cc:dd:ee:01\t6135\t-38\t[WPA2-SAE-CCMP][SAE-H2E][ESS]\tdNET1\n\
            aa:bb:cc:dd:ee:02\t5745\t-44\t[WPA2-PSK+SAE-CCMP][ESS]\tdNET1\n\
            aa:bb:cc:dd:ee:03\t2437\t-60\t[WPA2-PSK-CCMP][ESS]\tOld\n";
        let nets = by_network(parse_scan_results(scan));
        assert_eq!(nets[0].keys, Keys { psk: true, sae: true }, "across its access points");
        assert_eq!(nets[0].keys.settings(), Some(("WPA-PSK SAE", "1")));
        assert_eq!(nets[1].keys.settings(), None, "WPA2 alone keeps the defaults");
        assert_eq!(Keys { psk: false, sae: true }.settings(), Some(("SAE", "2")));
        let f = fake(
            vec![
                ("LIST_NETWORKS", "network id / ssid / bssid / flags\n0\tdNET1\tany\t[CURRENT]\n1\tOld\tany\t\n"),
                ("GET_NETWORK 0 key_mgmt", "WPA-PSK WPA-EAP\n"),
                ("GET_NETWORK 1 key_mgmt", "WPA-PSK WPA-EAP\n"),
                ("GET_NETWORK", "FAIL\n"),
                ("SET", "OK\n"),
                ("ADD_NETWORK", "2\n"),
                ("ENABLE_NETWORK", "OK\n"),
                ("SELECT_NETWORK", "OK\n"),
                ("SAVE_CONFIG", "OK\n"),
            ],
            vec![],
        );
        let c = Client::open(&f.path).unwrap();
        assert_eq!(c.upgrade_known(&parse_scan_results(scan)).unwrap(), vec![0], "only the network that takes WPA3");
        let sent = f.sent.lock().unwrap().clone();
        assert!(sent.contains(&"SET_NETWORK 0 key_mgmt WPA-PSK SAE".to_string()));
        assert!(sent.contains(&"SET_NETWORK 0 ieee80211w 1".to_string()));
        assert!(!sent.iter().any(|s| s.starts_with("SET_NETWORK 1")));
        f.sent.lock().unwrap().clear();
        c.tune(Some("US")).unwrap();
        assert_eq!(*f.sent.lock().unwrap(), ["SET sae_pwe 2", "SET country US"]);
        assert!(matches!(c.tune(Some("us; x")), Err(Error::BadInput(_))));
        f.sent.lock().unwrap().clear();
        c.set_power_save(false).unwrap();
        assert_eq!(*f.sent.lock().unwrap(), ["SET ps 0"]);
        f.sent.lock().unwrap().clear();
        c.join(b"dNET1", Some("correct horse"), nets[0].keys).unwrap();
        let sent = f.sent.lock().unwrap().clone();
        assert!(sent.contains(&"SET_NETWORK 2 key_mgmt WPA-PSK SAE".to_string()));
    }

    #[test]
    fn events_arrive_after_attach() {
        let f = fake(vec![("ATTACH", "OK\n"), ("DETACH", "OK\n")], vec!["<2>CTRL-EVENT-SCAN-RESULTS ", "<3>CTRL-EVENT-SSID-TEMP-DISABLED id=3 ssid=\"Home\" auth_failures=1 duration=10 reason=WRONG_KEY"]);
        let e = Events::attach(&f.path).unwrap();
        assert_eq!(e.next(Duration::from_secs(2)).unwrap(), Some(Event::ScanResults));
        assert_eq!(e.next(Duration::from_secs(2)).unwrap(), Some(Event::WrongKey { id: Some(3) }));
        assert_eq!(e.next(Duration::from_millis(50)).unwrap(), None);
    }

    #[test]
    fn no_socket_is_not_running() {
        assert!(matches!(Client::open(Path::new("/nonexistent/wpa/wlan0")), Err(Error::NotRunning)));
        let f = fake(vec![], vec![]);
        let _p2p = UnixDatagram::bind(f.dir.join("p2p-dev-wlan0")).unwrap();
        let found = sockets_in(&[f.dir.clone()]);
        assert_eq!(found, vec![("wlan0".to_string(), f.path.clone())]);
    }
}
