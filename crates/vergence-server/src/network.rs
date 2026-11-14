// Wi-Fi follows the cable (settings milestone 1, step 4; Dave, 2026-11-14:
// "what if we enable wpa on all, but only turn it on when there is no
// hardline detected?"). ~/.config/vergence/desktop.toml:
//
//     [network]
//     wifi = "auto"   # or "on", "off"; auto: on only while no cable is connected
//
// A thread looks every two seconds at the wired links (vergence-net's
// interfaces: an Ethernet port, or a phone's USB tethering, that is up and
// has a carrier; virtual ones such as bridges and tunnels left out) and,
// once that has held for a few seconds, tells each Wi-Fi interface's
// wpa_supplicant DISCONNECT (it neither joins nor scans) or RECONNECT. It
// does so only when what is wanted changes, or wpa_supplicant comes back,
// so nothing is fought over between changes. All of it on the thread:
// wpa_supplicant's socket can be slow, and the server never waits on it.
//
// The first time Wi-Fi comes on for want of a cable while wpa_supplicant
// knows no network, a notification says so and opens the settings app's
// Wi-Fi page; never again (~/.local/state/vergence/network.toml, 0600).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use smithay::reexports::calloop::channel::{Event, Sender, channel};
use vergence_net::{ifaces, wpa};

use crate::state::Server;

/// How often the links are looked at.
const EVERY: Duration = Duration::from_secs(2);
/// How long a cable's coming or going must hold before Wi-Fi follows it
/// (a cable being pushed in flaps).
const HOLD: Duration = Duration::from_secs(4);

/// What the setting says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiSetting {
    Off,
    On,
    Auto,
}

impl WifiSetting {
    pub fn from_word(w: &str) -> WifiSetting {
        match w.trim().to_ascii_lowercase().as_str() {
            "off" => WifiSetting::Off,
            "on" => WifiSetting::On,
            _ => WifiSetting::Auto,
        }
    }

    /// Whether Wi-Fi is wanted, with a cable connected or not.
    pub fn wants(self, cable: bool) -> bool {
        match self {
            WifiSetting::Off => false,
            WifiSetting::On => true,
            WifiSetting::Auto => !cable,
        }
    }
}

/// desktop.toml `[network] country`: the radio rules wpa_supplicant follows
/// (6 GHz needs one); none: the system's.
fn country() -> Option<String> {
    let text = crate::wallpaper::desktop_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let table = text.parse::<toml::Table>().unwrap_or_default();
    table.get("network").and_then(|n| n.get("country")).and_then(|v| v.as_str()).map(|c| c.trim().to_ascii_uppercase()).filter(|c| wpa::country_ok(c))
}

fn setting() -> WifiSetting {
    let text = crate::wallpaper::desktop_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let table = text.parse::<toml::Table>().unwrap_or_default();
    table.get("network").and_then(|n| n.get("wifi")).and_then(|v| v.as_str()).map_or(WifiSetting::Auto, WifiSetting::from_word)
}

fn state_path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("vergence/network.toml"))
}

fn note_shown() -> bool {
    state_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| t.parse::<toml::Table>().ok()).and_then(|t| t.get("no-cable-note-shown")?.as_bool()).unwrap_or(false)
}

fn mark_note_shown() {
    use std::{io::Write, os::unix::fs::{DirBuilderExt, OpenOptionsExt}};
    let Some(path) = state_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir);
    }
    let beside = path.with_extension("toml.new");
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&beside)
        .and_then(|mut f| f.write_all(b"no-cable-note-shown = true\n"))
        .and_then(|_| std::fs::rename(&beside, &path));
    if let Err(e) = written {
        tracing::warn!("network: can't keep that the note was shown: {e}");
    }
}

/// How long Wi-Fi may be on without joining anything before it says what is in range.
const UNJOINED: Duration = Duration::from_secs(20);

/// What the thread tells the server.
enum Said {
    /// Wi-Fi came on for want of a cable and knows no network: say so, once.
    NoCable,
    /// Wi-Fi is on, has joined nothing, and these are in range (Dave,
    /// 2026-11-14: "it could notify: Networks Available"): the words to say.
    Available(String),
}

/// The note's words for the networks in range: the known ones joined only
/// when chosen first (a phone's hotspot), then how many others.
pub fn available_words(manual_in_range: &[String], others: usize) -> String {
    let mut parts: Vec<String> = manual_in_range.to_vec();
    if others > 0 {
        let other = if parts.is_empty() { "" } else { " other" };
        parts.push(if others == 1 { format!("one{other} network") } else { format!("{others}{other} networks") });
    }
    let one = parts.len() == 1 && (others <= 1 || !manual_in_range.is_empty());
    let list = match parts.as_slice() {
        [] => return String::new(),
        [only] => only.clone(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    };
    let mut text = format!("{list} {} in range.", if one { "is" } else { "are" });
    if let Some(c) = text.get(0..1) {
        text.replace_range(0..1, &c.to_uppercase());
    }
    text
}

/// The thread: watch the links, apply the setting.
fn watch(tx: Sender<Said>) {
    let mut seen: Option<(bool, Instant)> = None;
    let mut applied: Option<(bool, Vec<String>)> = None;
    let mut first = true;
    // Wi-Fi on and joined to nothing since; said so already.
    let mut unjoined: Option<Instant> = None;
    let mut told = false;
    // wpa_supplicant readied (WPA3, the country) per socket, and when the
    // known networks were last looked at for WPA3.
    let mut tuned: Vec<(String, Option<String>)> = Vec::new();
    let mut upgraded_at: Option<Instant> = None;
    loop {
        let all = ifaces::list().unwrap_or_default();
        let cable = all.iter().any(ifaces::is_cable);
        let now = Instant::now();
        match seen {
            Some((c, _)) if c == cable => {}
            _ => seen = Some((cable, now)),
        }
        // At start the links are as they are; later a change must hold.
        let held = first || seen.is_some_and(|(_, since)| now.duration_since(since) >= HOLD);
        first = false;
        let wifi: Vec<String> = all.iter().filter(|f| f.kind == ifaces::Kind::WiFi).map(|f| f.name.clone()).collect();
        let socks: Vec<(String, PathBuf)> = wpa::sockets().into_iter().filter(|(n, _)| wifi.contains(n)).collect();
        if held && !socks.is_empty() {
            let set = setting();
            let want = set.wants(cable);
            let names: Vec<String> = socks.iter().map(|(n, _)| n.clone()).collect();
            if applied.as_ref() != Some(&(want, names.clone())) {
                let mut ok = true;
                let mut none_known = true;
                for (name, path) in &socks {
                    let done = wpa::Client::open(path).and_then(|c| {
                        if want {
                            c.reconnect()?
                        } else {
                            c.disconnect()?;
                            let _ = c.keep_manual_now();
                        }
                        if c.networks().is_ok_and(|n| !n.is_empty()) {
                            none_known = false;
                        }
                        Ok(())
                    });
                    match done {
                        Ok(()) => tracing::info!("network: Wi-Fi {} on {name} ({set:?}, cable {})", if want { "on" } else { "off" }, if cable { "connected" } else { "not connected" }),
                        Err(e) => {
                            ok = false;
                            tracing::warn!("network: Wi-Fi on {name}: {e}");
                        }
                    }
                }
                if ok {
                    // Wi-Fi came on because the cable went (not by being set On).
                    let for_want_of_cable = want && set == WifiSetting::Auto && applied.as_ref().is_none_or(|(w, _)| !w);
                    if for_want_of_cable && none_known && !note_shown() {
                        mark_note_shown();
                        told = true;
                        if tx.send(Said::NoCable).is_err() {
                            return;
                        }
                    }
                    applied = Some((want, names));
                }
            }
        }
        // Networks joined only when chosen (a phone's hotspot) are kept from
        // being joined by themselves once left (vergence-net's keep_manual).
        if applied.as_ref().is_some_and(|(w, _)| *w) {
            for (_, path) in &socks {
                if let Ok(true) = wpa::Client::open(path).and_then(|c| c.keep_manual()) {
                    tracing::info!("network: a network joined only when chosen was left; kept from joining by itself");
                }
            }
        }
        let on = applied.as_ref().is_some_and(|(w, _)| *w);
        // WPA3 and the country, once per wpa_supplicant (and again if the
        // country setting changes); the known networks every half minute.
        let want_country = country();
        for (name, path) in &socks {
            if tuned.iter().any(|(n, c)| n == name && *c == want_country) {
                continue;
            }
            match wpa::Client::open(path).and_then(|c| c.tune(want_country.as_deref())) {
                Ok(()) => {
                    tracing::info!("network: wpa_supplicant for {name}: WPA3 handshakes on, country {}", want_country.as_deref().unwrap_or("the system's"));
                    tuned.retain(|(n, _)| n != name);
                    tuned.push((name.clone(), want_country.clone()));
                    upgraded_at = None;
                }
                Err(e) => tracing::warn!("network: wpa_supplicant for {name}: {e}"),
            }
        }
        if on && upgraded_at.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(30)) {
            upgraded_at = Some(now);
            for (name, path) in &socks {
                let Ok(c) = wpa::Client::open(path) else { continue };
                let scan = c.scan_results().unwrap_or_default();
                match c.upgrade_known(&scan) {
                    Ok(ids) if !ids.is_empty() => {
                        tracing::info!("network: {name}: known networks {ids:?} take WPA3 too; given it (6 GHz)");
                        if let Ok(st) = c.status()
                            && st.id.is_some_and(|i| ids.contains(&i))
                        {
                            let _ = c.reassociate();
                        }
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("network: {name}: WPA3 for known networks: {e}"),
                }
            }
        }
        // On, joined to nothing for a while, networks in range: say so once.
        let client = socks.first().and_then(|(_, p)| wpa::Client::open(p).ok());
        match (&client, on) {
            (Some(c), true) if !c.status().is_ok_and(|s| s.connected()) => {
                let since = *unjoined.get_or_insert(now);
                if !told && now.duration_since(since) >= UNJOINED {
                    let nets = wpa::by_network(c.scan_results().unwrap_or_default());
                    if !nets.is_empty() {
                        told = true;
                        let known = c.networks().unwrap_or_default();
                        let manual: Vec<String> = nets.iter().filter(|b| known.iter().any(|k| k.manual && k.ssid_raw == b.ssid_raw)).map(|b| b.ssid.clone()).take(3).collect();
                        let words = available_words(&manual, nets.len() - manual.len());
                        if tx.send(Said::Available(words)).is_err() {
                            return;
                        }
                    }
                }
            }
            _ => {
                unjoined = None;
                told = false;
            }
        }
        if socks.is_empty() {
            // wpa_supplicant gone: apply again when it comes back.
            tuned.clear();
            applied = None;
        }
        std::thread::sleep(EVERY);
    }
}

impl Server {
    /// Start the thread (called once the event loop exists).
    pub fn start_network(&mut self) {
        // Only the session on the real screens: a nested server (a test, with
        // a throwaway home and its own settings) must not switch the
        // machine's Wi-Fi.
        if !std::env::args().any(|a| a == "--drm") {
            tracing::info!("network: nested, so Wi-Fi is left to the session");
            return;
        }
        let (tx, rx) = channel::<Said>();
        let _ = self.loop_handle.insert_source(rx, |event, _, state| {
            match event {
                Event::Msg(Said::NoCable) => state.network_no_cable_note(),
                Event::Msg(Said::Available(words)) => state.network_available_note(words),
                _ => {}
            }
        });
        if let Err(e) = std::thread::Builder::new().name("network".into()).spawn(move || watch(tx)) {
            tracing::warn!("network: can't start the watch: {e}");
        }
    }

    fn network_no_cable_note(&mut self) {
        let id = self.notify(
            crate::notify::Origin::Server,
            crate::notify::Request {
                app: "Settings".into(),
                replaces: 0,
                summary: "No network cable: Wi-Fi is on".into(),
                body: "Choose a network to join.".into(),
                urgency: 1,
                timeout_ms: -1,
                actions: vec![("default".into(), String::new()), ("wifi".into(), "Choose a network".into())],
            },
        );
        self.network_note = Some(id);
    }

    fn network_available_note(&mut self, words: String) {
        let id = self.notify(
            crate::notify::Origin::Server,
            crate::notify::Request {
                app: "Settings".into(),
                replaces: self.network_note.unwrap_or(0),
                summary: "Wi-Fi networks available".into(),
                body: words,
                urgency: 0,
                timeout_ms: -1,
                actions: vec![("default".into(), String::new()), ("wifi".into(), "Choose a network".into())],
            },
        );
        self.network_note = Some(id);
    }

    /// A notification of the server's own was clicked (notify.rs).
    pub fn server_note_action(&mut self, id: u32, _key: &str) {
        if self.network_note == Some(id) {
            self.network_note = None;
            if let Err(e) = crate::spawn(std::process::Command::new("vergence-settings").args(["--page", "wifi"])) {
                tracing::warn!("network: can't open the settings app: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_for_what_is_in_range() {
        assert_eq!(available_words(&["OMGiphone".into()], 0), "OMGiphone is in range.");
        assert_eq!(available_words(&["OMGiphone".into()], 3), "OMGiphone and 3 other networks are in range.");
        assert_eq!(available_words(&["A".into(), "B".into()], 1), "A, B and one other network are in range.");
        assert_eq!(available_words(&["A".into(), "B".into()], 0), "A and B are in range.");
        assert_eq!(available_words(&[], 1), "One network is in range.");
        assert_eq!(available_words(&[], 4), "4 networks are in range.");
    }

    #[test]
    fn the_setting() {
        assert_eq!(WifiSetting::from_word("off"), WifiSetting::Off);
        assert_eq!(WifiSetting::from_word(" On "), WifiSetting::On);
        assert_eq!(WifiSetting::from_word("anything"), WifiSetting::Auto);
        assert!(WifiSetting::Auto.wants(false) && !WifiSetting::Auto.wants(true));
        assert!(WifiSetting::On.wants(true) && !WifiSetting::Off.wants(false));
    }
}
