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
}

#[derive(Debug, Clone)]
pub enum WifiMsg {
    Setting(usize),
    /// Time to scan again; an event from wpa_supplicant.
    Tick,
    Event(wpa::Event),
    Pick(usize),
    Password(String),
    ShowPassword(bool),
    Join,
    Cancel,
    Forget(u32),
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
        if !self.wifi.started {
            self.wifi.started = true;
            let mail = self.mail.clone();
            let _ = std::thread::Builder::new().name("wifi".into()).spawn(move || listen(mail));
        }
        let all = ifaces::list().unwrap_or_default();
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
                if self.page == Page::WiFi {
                    if self.wifi_wanted()
                        && let Some(c) = self.wifi_client()
                    {
                        let _ = c.scan();
                    }
                    self.wifi_read();
                }
            }
            WifiMsg::Event(e) => {
                match &e {
                    wpa::Event::Connected { .. } => {
                        if let Some((_, name)) = self.wifi.joining.take() {
                            if let Some(c) = self.wifi_client() {
                                let _ = c.enable_all();
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
                                let _ = c.enable_all();
                            }
                            self.wifi.joining = None;
                            self.wifi.said = format!("Wrong password for {}.", wpa::ssid_text(&name));
                            self.wifi.asking = Some(name);
                        }
                    }
                    _ => {}
                }
                if self.page == Page::WiFi && !matches!(e, wpa::Event::Other(_)) {
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
        match c.join(&name, password.as_deref()) {
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
            let n = w.signal.as_ref().map_or(0, |s| wpa::bars(s.rssi));
            top.push(setting(&w.status.ssid.clone().unwrap_or_default(), "In use.", bars_mark(n)));
        }
        items.push(self.group(None, top));
        items.push(label(now).size(12.0).dim());
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
                    let note = if k.current { "In use" } else if k.temp_disabled { "Turned away for now (a wrong password?)" } else { "" };
                    setting(&k.ssid, note, button(label("Forget").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(Message::Wifi(WifiMsg::Forget(k.id))).name(&format!("Forget {}", k.ssid)))
                })
                .collect();
            items.push(self.group(Some("Known networks"), rows));
        }
        // Advanced: the live detail.
        let mut adv = vec![setting("Device", w.iface.as_deref().unwrap_or(""), space()), setting("State", &w.status.state, space())];
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
