//! What vergence-net sees: `cargo run -p vergence-net --example vnet [scan]`.
//! The interfaces and their addresses; for each interface wpa_supplicant
//! runs for, its state, signal and known networks, and with `scan`, the
//! networks in range (it waits up to 10 s for the scan to finish).

use std::time::{Duration, Instant};
use vergence_net::{ifaces, wpa};

fn main() {
    match ifaces::list() {
        Ok(all) => {
            for f in all {
                let addrs: Vec<String> = f.addrs.iter().map(|(a, p)| format!("{a}/{p}")).collect();
                println!(
                    "{:<10} {:?}{}{} {} {}",
                    f.name,
                    f.kind,
                    if f.up { " up" } else { " down" },
                    if f.carrier { " carrier" } else { "" },
                    f.mac.unwrap_or_default(),
                    addrs.join(" ")
                );
            }
        }
        Err(e) => println!("interfaces: {e}"),
    }
    let socks = wpa::sockets();
    if socks.is_empty() {
        println!("wpa_supplicant: not running (no control socket in {})", wpa::DIRS.join(" or "));
    }
    let scan = std::env::args().any(|a| a == "scan");
    for (name, path) in socks {
        println!("-- wpa_supplicant for {name}");
        let c = match wpa::Client::open(&path) {
            Ok(c) => c,
            Err(e) => {
                println!("  {e}");
                continue;
            }
        };
        match c.status() {
            Ok(s) => println!("  state {} ssid {:?} freq {:?} ip {:?}", s.state, s.ssid, s.freq, s.ip),
            Err(e) => println!("  status: {e}"),
        }
        if let Ok(Some(g)) = c.signal() {
            println!("  signal {} dBm ({} bars), {:?} Mb/s, {:?}", g.rssi, wpa::bars(g.rssi), g.link_mbps, g.freq.and_then(wpa::band_channel));
        }
        for k in c.networks().unwrap_or_default() {
            println!("  known {} {:?}{}{}", k.id, k.ssid, if k.current { " current" } else { "" }, if k.disabled { " disabled" } else { "" });
        }
        if scan {
            let events = wpa::Events::attach(&path);
            if let Err(e) = c.scan() {
                println!("  scan: {e}");
            }
            if let Ok(ev) = &events {
                let start = Instant::now();
                while start.elapsed() < Duration::from_secs(10) {
                    if let Ok(Some(wpa::Event::ScanResults)) = ev.next(Duration::from_millis(500)) {
                        break;
                    }
                }
            }
            for b in wpa::by_network(c.scan_results().unwrap_or_default()) {
                println!("  {:>4} dBm {} {:?} {:?} {}", b.signal, "|".repeat(wpa::bars(b.signal) as usize), b.security, wpa::band_channel(b.freq), b.ssid);
            }
        }
    }
}
