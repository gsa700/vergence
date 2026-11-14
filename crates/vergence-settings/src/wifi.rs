// The Wi-Fi page (settings milestone 1, step 4): the setting Off / On /
// When no cable is connected (desktop.toml `[network] wifi`, the server's
// network.rs applies it), the network in use with its signal, the networks
// in range, strongest first (rescanned every few seconds while the page is
// open and Wi-Fi is on), joining one with its password in the page, the
// known networks with Forget, and an Advanced part (signal, channel and
// band, the access point, the addresses). Through vergence-net's own
// client of wpa_supplicant's control socket, the `wheel` group's: no root.
//
// The password is held in the field only until Join sends it, then cleared;
// it is never written anywhere by this app (wpa_supplicant keeps it in its
// own file, root's).

use std::path::PathBuf;
use std::time::Duration;

use vergence_net::{ifaces, wpa};

use super::*;

/// How often the networks are scanned again while the page is open.
const RESCAN: Duration = Duration::from_secs(10);

/// The setting's words in desktop.toml, in the choice's order.
pub const WIFI_WORDS: [&str; 3] = ["off", "on", "auto"];
const WIFI_NAMES: [&str; 3] = ["Off", "On", "When no cable is connected"];

/// What the page knows.
#[derive(Default)]
pub struct Wifi {
    /// The Wi-Fi interface and wpa_supplicant's socket for it.
    pub iface: Option<String>,
    socket: Option<PathBuf>,
    /// Why there is nothing to show, in words, with what to do.
    problem: Option<(String, String)>,
    status: wpa::Status,
    signal: Option<wpa::Signal>,
    nets: Vec<wpa::Bss>,
    known: Vec<wpa::Known>,
    /// The Wi-Fi interface as the system sees it (addresses).
    link: Option<ifaces::Iface>,
    /// A cable is connected (for When no cable is connected).
    cable: bool,
    /// The network whose password is being typed.
    asking: Option<Vec<u8>>,
    pub password: String,
    show_password: bool,
    /// A network just added and being joined: its id and name.
    joining: Option<(u32, Vec<u8>)>,
    /// What happened last, in words.
    said: String,
    /// The thread that hears wpa_supplicant's events has started.
    started: bool,
    /// Every interface, for the Network page.
    pub all: Vec<ifaces::Iface>,
    /// The Network page's cables: each one's byte counters at the last sample
    /// and its rates in and out since (bits a second), the last two minutes.
    cables: std::collections::HashMap<String, Cable>,
    cable_sampler: bool,
    /// The network in use's signal every two seconds while the page is open
    /// (dBm, oldest first; the last two minutes), and for which network.
    history: std::collections::VecDeque<i32>,
    history_of: Option<String>,
    /// The Wi-Fi interface's byte counters at the last sample, and the rates
    /// in and out since then (bits a second).
    last_bytes: Option<(std::time::Instant, u64, u64)>,
    rates: Option<(f64, f64)>,
}

/// A cable's traffic for the Network page.
#[derive(Default)]
struct Cable {
    last: Option<(std::time::Instant, u64, u64)>,
    rx: std::collections::VecDeque<f64>,
    tx: std::collections::VecDeque<f64>,
}

/// Samples kept for the signal chart: two minutes at one every two seconds.
const HISTORY: usize = 60;
/// How often the signal and the traffic are sampled while the page is open.
const SAMPLE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub enum WifiMsg {
    Setting(usize),
    /// Time to scan again; an event from wpa_supplicant.
    Tick,
    Event(wpa::Event),
    /// Time to sample the signal and the traffic.
    Sample,
    /// Time to sample the cables' traffic (the Network page).
    CableSample,
    Pick(usize),
    Password(String),
    ShowPassword(bool),
    Join,
    Cancel,
    Forget(u32),
    /// Join a known network by itself, or only when chosen.
    Auto(u32, bool),
    /// The card's power saving (desktop.toml `[network] wifi-power-save`; the server applies it).
    PowerSave(bool),
}

/// The thread: wpa_supplicant's events, and a tick every RESCAN.
fn listen(mail: Mail) {
    loop {
        let wifi: Vec<String> = ifaces::list().unwrap_or_default().into_iter().filter(|f| f.kind == ifaces::Kind::WiFi).map(|f| f.name).collect();
        let sock = wpa::sockets().into_iter().find(|(n, _)| wifi.contains(n)).map(|(_, p)| p);
        let Some(sock) = sock else {
            std::thread::sleep(RESCAN);
            if !mail.send(Message::Wifi(WifiMsg::Tick)) {
                return;
            }
            continue;
        };
        let Ok(events) = wpa::Events::attach(&sock) else {
            std::thread::sleep(Duration::from_secs(3));
            continue;
        };
        let mut last = std::time::Instant::now();
        let mut sampled = std::time::Instant::now();
        loop {
            match events.next(Duration::from_secs(1)) {
                Ok(Some(e)) => {
                    let gone = e == wpa::Event::Terminating;
                    if !mail.send(Message::Wifi(WifiMsg::Event(e))) {
                        return;
                    }
                    if gone {
                        break;
                    }
                }
                Ok(None) => {}
                Err(_) => break,
            }
            if sampled.elapsed() >= SAMPLE {
                sampled = std::time::Instant::now();
                if !mail.send(Message::Wifi(WifiMsg::Sample)) {
                    return;
                }
            }
            if last.elapsed() >= RESCAN {
                last = std::time::Instant::now();
                if !mail.send(Message::Wifi(WifiMsg::Tick)) {
                    return;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Signal bars, 0 to 4, drawn.
fn bars_mark<M: 'static>(n: u8) -> Element<M> {
    drawing(move |scene: &mut Scene, _: &mut Fonts, r: Rect, t: &Theme| {
        let (x0, base) = (r.x + (r.w - 18.0) / 2.0, r.y + r.h / 2.0 + 6.0);
        for i in 0..4u8 {
            let h = 3.0 + i as f32 * 3.0;
            let c = if i < n { t.text } else { Color::rgba(t.dim.r, t.dim.g, t.dim.b, 90) };
            scene.fill_rounded(Rect::new(x0 + i as f32 * 5.0, base - h, 3.0, h), 1.0, c);
        }
    })
    .width(Length::Fixed(22.0))
    .height(Length::Fixed(16.0))
}

/// The color a signal's strength wears: green when good, amber when fair,
/// red when weak (the system colors macOS and iOS use for the same).
fn strength_color(bars: u8) -> Color {
    match bars {
        3..=4 => Color::rgb(0x30, 0xb0, 0x4f),
        2 => Color::rgb(0xf0, 0x9a, 0x0a),
        _ => Color::rgb(0xe5, 0x3e, 0x35),
    }
}

/// Signal bars in the strength's color (the network in use).
fn bars_colored<M: 'static>(n: u8) -> Element<M> {
    drawing(move |scene: &mut Scene, _: &mut Fonts, r: Rect, t: &Theme| {
        let (x0, base) = (r.x + (r.w - 18.0) / 2.0, r.y + r.h / 2.0 + 6.0);
        let on = strength_color(n);
        for i in 0..4u8 {
            let h = 3.0 + i as f32 * 3.0;
            let c = if i < n { on } else { Color::rgba(t.dim.r, t.dim.g, t.dim.b, 90) };
            scene.fill_rounded(Rect::new(x0 + i as f32 * 5.0, base - h, 3.0, h), 1.0, c);
        }
    })
    .width(Length::Fixed(22.0))
    .height(Length::Fixed(16.0))
}

/// The signal over the last two minutes: a line from -90 dBm (bottom) to
/// -30 dBm (top), in the latest sample's strength color, with faint lines
/// where the bars change.
fn signal_chart<M: 'static>(samples: Vec<i32>) -> Element<M> {
    drawing(move |scene: &mut Scene, _: &mut Fonts, r: Rect, t: &Theme| {
        let y_of = |dbm: i32| r.y + r.h - ((dbm.clamp(-90, -30) + 90) as f32 / 60.0) * r.h;
        let faint = Color::rgba(t.dim.r, t.dim.g, t.dim.b, 50);
        for edge in [-55, -67, -75] {
            let y = y_of(edge);
            scene.line(Point::new(r.x, y), Point::new(r.x + r.w, y), 1.0, faint);
        }
        if samples.len() < 2 {
            return;
        }
        let color = strength_color(wpa::bars(*samples.last().unwrap_or(&-90)));
        let step = r.w / (HISTORY - 1) as f32;
        // The newest at the right edge.
        let x0 = r.x + r.w - step * (samples.len() - 1) as f32;
        for (i, pair) in samples.windows(2).enumerate() {
            let a = Point::new(x0 + step * i as f32, y_of(pair[0]));
            let b = Point::new(x0 + step * (i + 1) as f32, y_of(pair[1]));
            scene.line(a, b, 1.8, color);
        }
    })
    .width(Length::Fixed(180.0))
    .height(Length::Fixed(30.0))
}

/// Traffic over the last two minutes: in (the accent) and out (fainter),
/// from 0 to the larger of 1 Mb/s and the most seen, the newest at the right.
fn traffic_chart<M: 'static>(rx: Vec<f64>, tx: Vec<f64>) -> Element<M> {
    drawing(move |scene: &mut Scene, _: &mut Fonts, r: Rect, t: &Theme| {
        let faint = Color::rgba(t.dim.r, t.dim.g, t.dim.b, 50);
        scene.line(Point::new(r.x, r.y + r.h), Point::new(r.x + r.w, r.y + r.h), 1.0, faint);
        let top = rx.iter().chain(tx.iter()).copied().fold(1e6f64, f64::max) * 1.1;
        let step = r.w / (HISTORY - 1) as f32;
        let y_of = |v: f64| r.y + r.h - (v / top).clamp(0.0, 1.0) as f32 * r.h;
        let draw = |scene: &mut Scene, s: &[f64], c: Color| {
            if s.len() < 2 {
                return;
            }
            let x0 = r.x + r.w - step * (s.len() - 1) as f32;
            for (i, p) in s.windows(2).enumerate() {
                scene.line(Point::new(x0 + step * i as f32, y_of(p[0])), Point::new(x0 + step * (i + 1) as f32, y_of(p[1])), 1.6, c);
            }
        };
        draw(scene, &tx, Color::rgba(t.accent.r, t.accent.g, t.accent.b, 110));
        draw(scene, &rx, t.accent);
    })
    .width(Length::Fixed(180.0))
    .height(Length::Fixed(30.0))
}

/// A rate in bits a second, in words: "2.4 Mb/s", "860 kb/s".
fn rate_text(bits: f64) -> String {
    if bits >= 1e6 {
        format!("{:.1} Mb/s", bits / 1e6)
    } else if bits >= 1e3 {
        format!("{:.0} kb/s", bits / 1e3)
    } else {
        format!("{bits:.0} b/s")
    }
}

/// A signal in a word, by its bars.
fn signal_word(dbm: i32) -> &'static str {
    match wpa::bars(dbm) {
        4 => "excellent",
        3 => "good",
        2 => "fair",
        1 => "weak",
        _ => "very weak",
    }
}

/// wpa_supplicant's state in a few words.
fn state_words(s: &str) -> &'static str {
    match s {
        "COMPLETED" => "Connected",
        "SCANNING" => "Looking for networks",
        "ASSOCIATING" | "ASSOCIATED" | "AUTHENTICATING" | "4WAY_HANDSHAKE" | "GROUP_HANDSHAKE" => "Joining",
        "DISCONNECTED" => "Not connected",
        "INACTIVE" => "Not connected",
        "INTERFACE_DISABLED" => "Turned off",
        _ => "Not connected",
    }
}

impl App {
    /// Whether the machine has a Wi-Fi device now (the sidebar shows the Wi-Fi
    /// page then, as Touchpad and Touchscreen once the machine has had one;
    /// Dave, 2026-11-14). A USB Wi-Fi adapter plugged in makes it appear.
    pub fn has_wifi(&self) -> bool {
        self.wifi.all.iter().any(|f| f.kind == ifaces::Kind::WiFi)
    }

    /// The interfaces read again (at start; the Wi-Fi thread's tick keeps them current).
    pub fn read_interfaces(&mut self) {
        self.wifi.all = ifaces::list().unwrap_or_default();
    }

    fn wifi_setting(&self) -> usize {
        let w = self.desktop.get("network").and_then(|n| n.get("wifi")).and_then(|v| v.as_str()).unwrap_or("auto");
        WIFI_WORDS.iter().position(|x| *x == w.trim().to_ascii_lowercase()).unwrap_or(2)
    }

    /// Whether Wi-Fi is wanted now (the server's rule, network.rs).
    fn wifi_wanted(&self) -> bool {
        match self.wifi_setting() {
            0 => false,
            1 => true,
            _ => !self.wifi.cable,
        }
    }

    /// Read everything again (the page opened, a scan finished, a tick).
    pub fn wifi_read(&mut self) {
        // The Network page's cables are sampled whether or not there is Wi-Fi.
        if self.page == Page::Network && !self.wifi.cable_sampler {
            self.wifi.cable_sampler = true;
            let mail = self.mail.clone();
            let _ = std::thread::Builder::new().name("cables".into()).spawn(move || {
                loop {
                    std::thread::sleep(SAMPLE);
                    if !mail.send(Message::Wifi(WifiMsg::CableSample)) {
                        return;
                    }
                }
            });
        }
        if !self.wifi.started {
            self.wifi.started = true;
            let mail = self.mail.clone();
            let _ = std::thread::Builder::new().name("wifi".into()).spawn(move || listen(mail));
        }
        let all = ifaces::list().unwrap_or_default();
        self.wifi.all = all.clone();
        self.wifi.cable = all.iter().any(ifaces::is_cable);
        let link = all.iter().find(|f| f.kind == ifaces::Kind::WiFi).cloned();
        self.wifi.iface = link.as_ref().map(|f| f.name.clone());
        self.wifi.link = link;
        self.wifi.problem = None;
        let Some(iface) = self.wifi.iface.clone() else {
            self.wifi.socket = None;
            self.wifi.problem = Some(("This computer has no Wi-Fi device".into(), "Plug in a Wi-Fi adapter, or use a network cable.".into()));
            return;
        };
        self.wifi.socket = wpa::find(&iface);
        let Some(sock) = self.wifi.socket.clone() else {
            self.wifi.problem = Some(if Path::new("/run/NetworkManager").is_dir() {
                ("Wi-Fi is NetworkManager's on this computer, for now".into(), "Vergence will speak to NetworkManager in a later milestone; until then, use its own tools.".into())
            } else {
                (format!("wpa_supplicant is not running for {iface}"), "Run sudo tools/install.sh once (it enables wpa_supplicant where there is a Wi-Fi card), or start it by hand.".into())
            });
            return;
        };
        let client = match wpa::Client::open(&sock) {
            Ok(c) => c,
            Err(wpa::Error::Denied) => {
                self.wifi.problem = Some(("This account may not use Wi-Fi".into(), "wpa_supplicant's control socket is the wheel group's: add the account to wheel, then log in again.".into()));
                return;
            }
            Err(e) => {
                self.wifi.problem = Some((format!("Wi-Fi: {e}"), String::new()));
                return;
            }
        };
        self.wifi.status = client.status().unwrap_or_default();
        self.wifi.signal = client.signal().ok().flatten();
        self.wifi.known = client.networks().unwrap_or_default();
        self.wifi.nets = if self.wifi_wanted() { wpa::by_network(client.scan_results().unwrap_or_default()) } else { Vec::new() };
    }

    fn wifi_client(&self) -> Option<wpa::Client> {
        self.wifi.socket.as_deref().and_then(|s| wpa::Client::open(s).ok())
    }

    pub fn wifi_handle(&mut self, m: WifiMsg) {
        match m {
            WifiMsg::Setting(i) => {
                self.save_table("network", |t| {
                    t.insert("wifi".into(), WIFI_WORDS[i.min(2)].into());
                });
                self.wifi.said = String::new();
                // The server applies it within a few seconds.
                self.wifi_read();
            }
            WifiMsg::Tick => {
                if !matches!(self.page, Page::WiFi | Page::Network) {
                    self.read_interfaces();
                }
                if self.page == Page::Network {
                    self.wifi_read();
                }
                if self.page == Page::WiFi {
                    if self.wifi_wanted()
                        && let Some(c) = self.wifi_client()
                    {
                        let _ = c.scan();
                    }
                    self.wifi_read();
                }
            }
            WifiMsg::CableSample => {
                if self.page != Page::Network {
                    return;
                }
                self.wifi.all = ifaces::list().unwrap_or_default();
                let now = std::time::Instant::now();
                for f in self.wifi.all.iter().filter(|f| matches!(f.kind, ifaces::Kind::Wired | ifaces::Kind::Phone)) {
                    let (Some(rx), Some(tx)) = (f.rx_bytes, f.tx_bytes) else { continue };
                    let c = self.wifi.cables.entry(f.name.clone()).or_default();
                    if let Some((then, rx0, tx0)) = c.last {
                        let secs = now.duration_since(then).as_secs_f64().max(0.1);
                        c.rx.push_back(ifaces::delta(rx0, rx) as f64 * 8.0 / secs);
                        c.tx.push_back(ifaces::delta(tx0, tx) as f64 * 8.0 / secs);
                        while c.rx.len() > HISTORY {
                            c.rx.pop_front();
                            c.tx.pop_front();
                        }
                    }
                    c.last = Some((now, rx, tx));
                }
            }
            WifiMsg::Sample => {
                if self.page != Page::WiFi {
                    return;
                }
                let Some(c) = self.wifi_client() else { return };
                let status = c.status().unwrap_or_default();
                if !status.connected() {
                    self.wifi.history.clear();
                    self.wifi.rates = None;
                    return;
                }
                if self.wifi.history_of != status.ssid {
                    self.wifi.history.clear();
                    self.wifi.history_of = status.ssid.clone();
                }
                if let Ok(Some(g)) = c.signal() {
                    self.wifi.history.push_back(g.rssi);
                    while self.wifi.history.len() > HISTORY {
                        self.wifi.history.pop_front();
                    }
                    self.wifi.signal = Some(g);
                }
                let iface = self.wifi.iface.clone();
                let counters = ifaces::list().ok().and_then(|all| all.into_iter().find(|f| Some(&f.name) == iface.as_ref())).and_then(|f| Some((f.rx_bytes?, f.tx_bytes?)));
                if let Some((rx, tx)) = counters {
                    let now = std::time::Instant::now();
                    if let Some((then, rx0, tx0)) = self.wifi.last_bytes {
                        let secs = now.duration_since(then).as_secs_f64().max(0.1);
                        self.wifi.rates = Some((ifaces::delta(rx0, rx) as f64 * 8.0 / secs, ifaces::delta(tx0, tx) as f64 * 8.0 / secs));
                    }
                    self.wifi.last_bytes = Some((now, rx, tx));
                }
            }
            WifiMsg::Event(e) => {
                match &e {
                    wpa::Event::Connected { .. } => {
                        if let Some((_, name)) = self.wifi.joining.take() {
                            if let Some(c) = self.wifi_client() {
                                let _ = c.enable_auto();
                                let _ = c.save();
                            }
                            self.wifi.said = format!("Joined {}.", wpa::ssid_text(&name));
                        }
                    }
                    wpa::Event::WrongKey { id } => {
                        if let Some((jid, name)) = self.wifi.joining.clone()
                            && id.is_none_or(|i| i == jid)
                        {
                            // The failed try is forgotten; ask again.
                            if let Some(c) = self.wifi_client() {
                                let _ = c.forget(jid);
                                let _ = c.enable_auto();
                            }
                            self.wifi.joining = None;
                            self.wifi.said = format!("Wrong password for {}.", wpa::ssid_text(&name));
                            self.wifi.asking = Some(name);
                        }
                    }
                    _ => {}
                }
                if matches!(self.page, Page::WiFi | Page::Network) && !matches!(e, wpa::Event::Other(_)) {
                    self.wifi_read();
                }
            }
            WifiMsg::Pick(i) => {
                let Some(b) = self.wifi.nets.get(i).cloned() else { return };
                self.wifi.said = String::new();
                if let Some(k) = self.wifi.known.iter().find(|k| k.ssid_raw == b.ssid_raw) {
                    if let Some(c) = self.wifi_client() {
                        match c.select(k.id) {
                            Ok(()) => {
                                self.wifi.joining = Some((k.id, b.ssid_raw.clone()));
                                self.wifi.said = format!("Joining {}\u{2026}", b.ssid);
                            }
                            Err(e) => self.wifi.said = format!("Can't join {}: {e}", b.ssid),
                        }
                    }
                    return;
                }
                match b.security {
                    wpa::Security::Open => self.wifi_join(b.ssid_raw.clone(), None),
                    wpa::Security::Password => {
                        self.wifi.asking = Some(b.ssid_raw.clone());
                        self.wifi.password.clear();
                        self.wifi.show_password = false;
                    }
                    wpa::Security::Enterprise => self.wifi.said = format!("{} asks for a user name as well: not supported here yet.", b.ssid),
                    wpa::Security::Wep => self.wifi.said = format!("{} uses WEP, which anyone nearby can break: not joined.", b.ssid),
                }
            }
            WifiMsg::Password(p) => self.wifi.password = p.chars().take(64).collect(),
            WifiMsg::ShowPassword(on) => self.wifi.show_password = on,
            WifiMsg::Join => {
                if let Some(name) = self.wifi.asking.clone() {
                    let password = std::mem::take(&mut self.wifi.password);
                    if let Err(e) = wpa::psk_arg(&password) {
                        self.wifi.password = password;
                        self.wifi.said = e.to_string();
                        return;
                    }
                    self.wifi.asking = None;
                    self.wifi_join(name, Some(password));
                }
            }
            WifiMsg::Cancel => {
                self.wifi.asking = None;
                self.wifi.password.clear();
            }
            WifiMsg::PowerSave(on) => self.save_table("network", |t| {
                t.insert("wifi-power-save".into(), on.into());
            }),
            WifiMsg::Auto(id, auto) => {
                if let Some(c) = self.wifi_client()
                    && let Err(e) = c.set_auto(id, auto)
                {
                    self.wifi.said = format!("Can't change it: {e}");
                }
                self.wifi_read();
            }
            WifiMsg::Forget(id) => {
                if let Some(c) = self.wifi_client() {
                    let name = self.wifi.known.iter().find(|k| k.id == id).map(|k| k.ssid.clone()).unwrap_or_default();
                    self.wifi.said = match c.forget(id) {
                        Ok(()) => format!("Forgot {name}."),
                        Err(e) => format!("Can't forget {name}: {e}"),
                    };
                }
                self.wifi_read();
            }
        }
    }

    fn wifi_join(&mut self, name: Vec<u8>, password: Option<String>) {
        let Some(c) = self.wifi_client() else { return };
        // WPA3 too where the network takes it (its 6 GHz access points).
        let keys = self.wifi.nets.iter().find(|b| b.ssid_raw == name).map(|b| b.keys).unwrap_or_default();
        match c.join(&name, password.as_deref(), keys) {
            Ok(id) => {
                self.wifi.joining = Some((id, name.clone()));
                self.wifi.said = format!("Joining {}\u{2026}", wpa::ssid_text(&name));
            }
            Err(e) => self.wifi.said = format!("Can't join {}: {e}", wpa::ssid_text(&name)),
        }
    }

    pub fn wifi_page(&self) -> Element<Message> {
        let w = &self.wifi;
        let about = "Wireless networks: which one this computer uses, and when Wi-Fi is on.";
        let mut items = Vec::new();
        let names: Vec<String> = WIFI_NAMES.iter().map(|s| s.to_string()).collect();
        let setting_row = setting(
            "Wi-Fi",
            "When no cable is connected: on only while no network cable is plugged in.",
            choice(names, Some(self.wifi_setting()), |i| Message::Wifi(WifiMsg::Setting(i))).width(Length::Fixed(230.0)).name("Wi-Fi"),
        );
        if let Some((what, how)) = &w.problem {
            if w.iface.is_some() {
                items.push(self.group(None, vec![setting_row]));
            }
            items.push(self.group(None, vec![setting(what, how, space())]));
            return self.page_layout("Wi-Fi", about, items);
        }
        let wanted = self.wifi_wanted();
        let now = if !wanted {
            if self.wifi_setting() == 2 { "Off: a network cable is connected.".to_string() } else { "Off.".to_string() }
        } else if w.status.connected() {
            format!("Connected to {}.", w.status.ssid.clone().unwrap_or_default())
        } else {
            format!("{}.", state_words(&w.status.state))
        };
        let mut top = vec![setting_row];
        if wanted && w.status.connected() {
            // The network in use with its numbers up front (Dave, 2026-11-14:
            // "some of the connection statistics right up front"); the rest
            // stays in Advanced.
            let n = w.signal.as_ref().map_or(0, |s| wpa::bars(s.rssi));
            let band = w.status.freq.and_then(wpa::band_channel).map(|(b, ch)| format!("In use. {b}, channel {ch}.")).unwrap_or_else(|| "In use.".into());
            top.push(setting(&w.status.ssid.clone().unwrap_or_default(), &band, bars_colored(n)));
            let value = |t: String| label(t).dim();
            // Signal and Speed always there while connected (a dash until read),
            // so the rows never move under the pointer.
            let chart: Vec<i32> = w.history.iter().copied().collect();
            let strength = w.signal.as_ref().map_or_else(|| "\u{2014}".to_string(), |g| format!("{} dBm, {}", g.rssi, signal_word(g.rssi)));
            top.push(setting(
                "Signal",
                "The last two minutes.",
                row(vec![signal_chart(chart), value(strength).width(Length::Fixed(130.0)).align(Align::End)]).spacing(12.0).align(Align::Center),
            ));
            let speed = w.signal.as_ref().and_then(|g| g.link_mbps).map_or_else(|| "\u{2014}".to_string(), |m| format!("{m} Mb/s"));
            top.push(setting("Speed", "", value(speed)));
            let v4 = w.link.as_ref().and_then(|f| f.addrs.iter().find(|(a, _)| a.is_ipv4())).map(|(a, _)| a.to_string()).or_else(|| w.status.ip.clone());
            top.push(setting("Address", "", value(v4.unwrap_or_else(|| "Waiting for one".into()))));
            // Always there while connected, so the page never shifts under the pointer.
            let traffic = w.rates.map_or_else(|| "Measuring\u{2026}".to_string(), |(rx, tx)| format!("in {}, out {}", rate_text(rx), rate_text(tx)));
            top.push(setting("Traffic", "Now, in and out.", value(traffic)));
        }
        items.push(self.group(None, top));
        // Said by the card above while connected.
        if !(wanted && w.status.connected()) {
            items.push(label(now).size(12.0).dim());
        }
        // The password, asked in the page.
        if let Some(name) = &w.asking {
            let mut f = field(&w.password, |p| Message::Wifi(WifiMsg::Password(p))).placeholder("Password").on_submit(Message::Wifi(WifiMsg::Join)).width(Length::Fixed(240.0)).name("Password").autofocus();
            if !w.show_password {
                f = f.secret();
            }
            items.push(self.group(
                Some(&format!("Join {}", wpa::ssid_text(name))),
                vec![
                    setting("Password", "", f),
                    setting("Show the password", "", switch(w.show_password, |on| Message::Wifi(WifiMsg::ShowPassword(on))).name("Show the password")),
                    panel(
                        row(vec![
                            space().width(Length::FILL),
                            button(label("Cancel").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(Message::Wifi(WifiMsg::Cancel)),
                            button(label("Join").size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(4.0, 14.0)).on_press(Message::Wifi(WifiMsg::Join)),
                        ])
                        .spacing(8.0)
                        .width(Length::FILL),
                    )
                    .padding(Padding::hv(8.0, 12.0))
                    .width(Length::FILL),
                ],
            ));
        }
        if !w.said.is_empty() {
            items.push(label(w.said.clone()).size(12.0));
        }
        // The networks in range.
        if wanted {
            let mut rows = Vec::new();
            for (i, b) in w.nets.iter().enumerate() {
                let current = w.status.connected() && w.status.ssid.as_deref() == Some(b.ssid.as_str());
                let known = w.known.iter().any(|k| k.ssid_raw == b.ssid_raw);
                let lock = match b.security {
                    wpa::Security::Open => "Open",
                    wpa::Security::Password => "Password",
                    wpa::Security::Enterprise => "User name and password",
                    wpa::Security::Wep => "WEP",
                };
                let detail = if known { format!("{lock}, known") } else { lock.to_string() };
                let action: Element<Message> = if current {
                    label("Connected").dim()
                } else {
                    button(label("Join").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(Message::Wifi(WifiMsg::Pick(i))).name(&format!("Join {}", b.ssid))
                };
                rows.push(setting(&b.ssid, &detail, row(vec![bars_mark(wpa::bars(b.signal)), action]).spacing(10.0).align(Align::Center)));
            }
            if rows.is_empty() {
                rows.push(setting("Looking for networks\u{2026}", "", space()));
            }
            items.push(self.group(Some("Networks"), rows));
        }
        // The known ones.
        if !w.known.is_empty() {
            let rows = w
                .known
                .iter()
                .map(|k| {
                    let note = if k.current {
                        "In use."
                    } else if k.temp_disabled {
                        "Turned away for now (a wrong password?)."
                    } else if k.manual {
                        "Joined only when you choose it."
                    } else {
                        "Joined by itself when in range."
                    };
                    let id = k.id;
                    setting(
                        &k.ssid,
                        note,
                        row(vec![
                            label("Join automatically").size(12.0).dim(),
                            switch(!k.manual, move |on| Message::Wifi(WifiMsg::Auto(id, on))).name(&format!("Join {} automatically", k.ssid)),
                            button(label("Forget").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(Message::Wifi(WifiMsg::Forget(k.id))).name(&format!("Forget {}", k.ssid)),
                        ])
                        .spacing(8.0)
                        .align(Align::Center),
                    )
                })
                .collect();
            items.push(self.group(Some("Known networks"), rows));
        }
        // Advanced: the live detail.
        let power_save = self.desktop.get("network").and_then(|n| n.get("wifi-power-save")).and_then(|v| v.as_bool()).unwrap_or(true);
        let mut adv = vec![
            setting(
                "Power saving",
                "On: the card naps between packets, saving battery. Off: quicker replies (pings, remote sessions), a little more power. Linux only for now.",
                switch(power_save, |on| Message::Wifi(WifiMsg::PowerSave(on))).name("Power saving"),
            ),
            setting("Device", w.iface.as_deref().unwrap_or(""), space()),
            setting("State", &w.status.state, space()),
        ];
        if let Some(s) = &w.signal {
            adv.push(setting("Signal", &format!("{} dBm{}", s.rssi, s.link_mbps.map(|m| format!(", {m} Mb/s")).unwrap_or_default()), space()));
        }
        if let Some((band, ch)) = w.status.freq.and_then(wpa::band_channel) {
            adv.push(setting("Channel", &format!("{ch} ({band}, {} MHz)", w.status.freq.unwrap_or(0)), space()));
        }
        if let Some(b) = &w.status.bssid {
            adv.push(setting("Access point", b, space()));
        }
        if let Some(link) = &w.link {
            let addrs: Vec<String> = link.addrs.iter().map(|(a, p)| format!("{a}/{p}")).collect();
            adv.push(setting("Addresses", &if addrs.is_empty() { "None".to_string() } else { addrs.join(", ") }, space()));
            if let Some(m) = &link.mac {
                adv.push(setting("Hardware address", m, space()));
            }
        }
        items.push(self.advanced("wifi", adv));
        self.page_layout("Wi-Fi", about, items)
    }
}

/// Addresses as shown: IPv4 first, then IPv6 but link-local ones (Advanced has every one).
fn shown_addrs(f: &ifaces::Iface) -> String {
    let mut v4: Vec<String> = Vec::new();
    let mut v6: Vec<String> = Vec::new();
    for (a, _) in &f.addrs {
        match a {
            std::net::IpAddr::V4(x) => v4.push(x.to_string()),
            std::net::IpAddr::V6(x) if (x.segments()[0] & 0xffc0) != 0xfe80 => v6.push(x.to_string()),
            _ => {}
        }
    }
    v4.extend(v6);
    v4.join(", ")
}

impl App {
    /// The Network page (settings milestone 1, step 5): the computer's
    /// connections by the names people use: each network cable ("Ethernet"),
    /// a phone sharing its connection by USB ("iPhone (USB)"), and Wi-Fi in
    /// one line with its own page. Read only: addresses are dhcpcd's.
    pub fn network_page(&self) -> Element<Message> {
        let w = &self.wifi;
        let about = "The computer's connections: network cables, a phone by USB, and Wi-Fi.";
        let links: Vec<&ifaces::Iface> = w.all.iter().filter(|f| matches!(f.kind, ifaces::Kind::Wired | ifaces::Kind::Phone) && !ifaces::is_virtual(&f.name) && f.mac.is_some()).collect();
        let ethernets = links.iter().filter(|f| f.kind == ifaces::Kind::Wired).count();
        let mut items = Vec::new();
        for f in &links {
            let phone = f.kind == ifaces::Kind::Phone;
            let title = if phone {
                "iPhone (USB)".to_string()
            } else if ethernets > 1 {
                format!("Ethernet ({})", f.name)
            } else {
                "Ethernet".to_string()
            };
            let addrs = shown_addrs(f);
            let speed = match (f.speed_mbps, f.full_duplex) {
                (Some(sp), fd) => {
                    let s = if sp >= 1000 && sp % 1000 == 0 { format!("{} Gb/s", sp / 1000) } else { format!("{sp} Mb/s") };
                    match fd {
                        Some(true) => format!("{s}, full duplex"),
                        Some(false) => format!("{s}, half duplex"),
                        None => s,
                    }
                }
                _ => String::new(),
            };
            let (state, detail) = match (f.carrier, phone) {
                (true, _) if !addrs.is_empty() && !speed.is_empty() => (format!("Connected at {speed}"), addrs),
                (true, _) if !addrs.is_empty() => ("Connected".to_string(), addrs),
                (true, _) => ("Connected".to_string(), "Waiting for an address.".to_string()),
                (false, false) => ("Cable not connected".to_string(), String::new()),
                (false, true) => ("Not sharing its connection".to_string(), "On the iPhone: trust this computer, and turn on Personal Hotspot.".to_string()),
            };
            let mut rows = vec![setting(&state, &detail, space())];
            // Its traffic while connected: always shown then (a dash until two
            // samples), so the page does not shift.
            if f.carrier {
                let c = w.cables.get(&f.name);
                let rx: Vec<f64> = c.map(|c| c.rx.iter().copied().collect()).unwrap_or_default();
                let tx: Vec<f64> = c.map(|c| c.tx.iter().copied().collect()).unwrap_or_default();
                let now = match (rx.last(), tx.last()) {
                    (Some(r), Some(t)) => format!("in {}, out {}", rate_text(*r), rate_text(*t)),
                    _ => "Measuring\u{2026}".to_string(),
                };
                rows.push(setting(
                    "Traffic",
                    "The last two minutes: in, and out (fainter).",
                    row(vec![traffic_chart(rx, tx), label(now).dim().width(Length::Fixed(170.0)).align(Align::End)]).spacing(12.0).align(Align::Center),
                ));
            }
            items.push(self.group(Some(&title), rows));
        }
        if links.is_empty() {
            items.push(self.group(None, vec![setting("No network cable port", "", space())]));
        }
        // Wi-Fi in a line.
        let wifi_line = if let Some((what, _)) = &w.problem {
            what.clone()
        } else if w.status.connected() {
            format!("Connected to {}.", w.status.ssid.clone().unwrap_or_default())
        } else if !self.wifi_wanted() {
            "Off.".to_string()
        } else {
            "Not connected.".to_string()
        };
        // Only where there is Wi-Fi (Dave, 2026-11-14: hidden like the touchscreen's).
        if self.has_wifi() {
            items.push(self.group(
                Some("Wi-Fi"),
                vec![setting(&wifi_line, "", button(label("Wi-Fi").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(Message::Go(Page::WiFi)).name("Open Wi-Fi"))],
            ));
        }
        if !links.iter().any(|f| f.kind == ifaces::Kind::Phone) {
            items.push(label("An iPhone by cable shows here once it shares its connection: plug it in, trust this computer, and turn on Personal Hotspot.").size(11.0).dim().wrap().width(Length::FILL));
        }
        // Advanced: every interface, as the system has it.
        let adv = w
            .all
            .iter()
            .map(|f| {
                let addrs: Vec<String> = f.addrs.iter().map(|(a, p)| format!("{a}/{p}")).collect();
                let state = format!(
                    "{:?}, {}{}{}{}",
                    f.kind,
                    if f.up { "up" } else { "down" },
                    if f.carrier { ", carrier" } else { "" },
                    f.mac.as_deref().map(|m| format!(", {m}")).unwrap_or_default(),
                    if addrs.is_empty() { String::new() } else { format!(", {}", addrs.join(" ")) }
                );
                setting(&f.name, &state, space())
            })
            .collect();
        items.push(self.advanced("network", adv));
        self.page_layout("Network", about, items)
    }
}
