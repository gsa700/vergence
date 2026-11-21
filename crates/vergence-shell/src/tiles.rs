//! Live tiles on the dock (dock milestone 1): where they stand, and the
//! desktop's own clock tile. The tiles themselves are
//! vergence_protocol::tile; the server draws them.

use crate::{Rect, running};
use vergence_protocol::tile::{Card, Control, ControlKind, Tile, TileState};

/// The place of tile `index` on a dock with `items` applications: after
/// them (applications and tiles keep their own parts; Dave, 2026-11-16).
pub fn slot(dock: Rect, items: usize, index: usize, upright: bool) -> Option<Rect> {
    running::slot(dock, items + index, upright)
}

/// Which of `count` tiles is at a point, if any.
pub fn tile_at(dock: Rect, items: usize, count: usize, upright: bool, x: i32, y: i32) -> Option<usize> {
    (0..count).find(|i| slot(dock, items, *i, upright).is_some_and(|s| x >= s.x && x < s.right() && y >= s.y && y < s.bottom()))
}

/// A moment in local time, as the clock shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    /// 0 to 11.
    pub month: i32,
    pub mday: i32,
    /// 0 is Sunday.
    pub wday: i32,
    pub hour: i32,
    pub min: i32,
    pub sec: i32,
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// How the clock shows the time (desktop.toml [clock], the settings app's
/// Date & Time page).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockLook {
    pub h24: bool,
    pub date: bool,
}

impl Default for ClockLook {
    fn default() -> Self {
        ClockLook { h24: true, date: true }
    }
}

/// The clock's tile for a local time and the same moment in UTC: the time
/// on the dock with the day under it (or, without the date on a 12-hour
/// clock, AM or PM); the card has the whole date, the time large, and UTC.
/// A click opens Date & Time.
pub fn clock(local: Local, utc: Local, look: ClockLook) -> Tile {
    let day = DAYS[local.wday.rem_euclid(7) as usize];
    let month = MONTHS[local.month.rem_euclid(12) as usize];
    let half = if local.hour < 12 { "AM" } else { "PM" };
    let hm = |t: Local| {
        if look.h24 {
            format!("{:02}:{:02}", t.hour, t.min)
        } else {
            format!("{}:{:02}", (t.hour + 11) % 12 + 1, t.min)
        }
    };
    let label = match (look.date, look.h24) {
        (true, _) => format!("{} {}", &day[..3], local.mday),
        (false, false) => half.into(),
        (false, true) => String::new(),
    };
    Tile {
        id: "clock".into(),
        value: hm(local),
        label,
        group: "System".into(),
        card: Card {
            title: format!("{day}, {month} {}, {}", local.mday, local.year),
            big: if look.h24 { hm(local) } else { format!("{} {half}", hm(local)) },
            rows: vec![("UTC".into(), format!("{:02}:{:02}", utc.hour, utc.min))],
            ..Card::default()
        },
        page: "date & time".into(),
        ..Tile::default()
    }
}

/// The default sound output as the Sound tile shows it (read from
/// vergence-sound by the server).
#[derive(Debug, Clone, PartialEq)]
pub struct SoundNow {
    pub name: String,
    pub volume: u8,
    pub muted: bool,
    pub adjustable: bool,
}

/// The Sound tile: the volume, or Muted; the card has the output, a
/// volume slider and a mute switch. A click opens Sound. None: no sound
/// system of ours, or no output; no tile.
pub fn sound(now: Option<&SoundNow>) -> Option<Tile> {
    let o = now?;
    let mut controls = Vec::new();
    if o.adjustable {
        controls.push(Control { id: "volume".into(), label: "Volume".into(), kind: ControlKind::Slider { value: o.volume as f64, min: 0.0, max: 100.0, step: 5.0 } });
    }
    controls.push(Control { id: "mute".into(), label: "Mute".into(), kind: ControlKind::Switch(o.muted) });
    Some(Tile {
        id: "sound".into(),
        value: if o.muted { "Off".into() } else if o.adjustable { format!("{}%", o.volume) } else { "On".into() },
        label: if o.muted { "Muted".into() } else { "Sound".into() },
        group: "System".into(),
        card: Card {
            title: "Sound".into(),
            rows: vec![("Output".into(), o.name.clone())],
            controls,
            note: if o.adjustable { String::new() } else { "The volume is set on the device.".into() },
            ..Card::default()
        },
        page: "sound".into(),
        ..Tile::default()
    })
}

/// A rate in bits a second, short: "840 kb/s", "12 Mb/s".
pub fn rate_text(bytes_per_s: f64) -> String {
    let bits = bytes_per_s * 8.0;
    match bits {
        b if b >= 1e9 => format!("{:.1} Gb/s", b / 1e9),
        b if b >= 1e7 => format!("{:.0} Mb/s", b / 1e6),
        b if b >= 1e6 => format!("{:.1} Mb/s", b / 1e6),
        b if b >= 1e3 => format!("{:.0} kb/s", b / 1e3),
        b => format!("{b:.0} b/s"),
    }
}

/// The interface the machine reaches the network by: up, with a carrier
/// and an address that is not link-local, a cable before Wi-Fi before a
/// phone; loopback and virtual interfaces never.
pub fn main_iface(ifaces: &[vergence_net::ifaces::Iface]) -> Option<&vergence_net::ifaces::Iface> {
    use vergence_net::ifaces::Kind;
    let usable = |f: &&vergence_net::ifaces::Iface| {
        f.kind != Kind::Loopback
            && !vergence_net::ifaces::is_virtual(&f.name)
            && f.up
            && f.carrier
            && f.addrs.iter().any(|(a, _)| match a {
                std::net::IpAddr::V4(v) => !v.is_link_local(),
                std::net::IpAddr::V6(v) => (v.segments()[0] & 0xffc0) != 0xfe80,
            })
    };
    let rank = |f: &vergence_net::ifaces::Iface| match f.kind {
        Kind::Wired => 0,
        Kind::WiFi => 1,
        Kind::Phone => 2,
        Kind::Loopback => 3,
    };
    ifaces.iter().filter(usable).min_by_key(|f| rank(f))
}

/// The Network tile: how the machine is connected ("Wired", "Wi-Fi",
/// "Phone"), with the rate coming in under it; "Offline" in the problem's
/// color when nothing reaches the network. The card has the interface, its
/// address, the link's speed and the rates in and out, with a chart of
/// what came in (`history`, bytes a second, oldest first). A click opens
/// Network.
pub fn network(ifaces: &[vergence_net::ifaces::Iface], rate_in: f64, rate_out: f64, history: &[f32]) -> Tile {
    use vergence_net::ifaces::Kind;
    let mut t = Tile { id: "network".into(), group: "System".into(), page: "network".into(), ..Tile::default() };
    match main_iface(ifaces) {
        Some(f) => {
            t.value = match f.kind {
                Kind::WiFi => "Wi-Fi",
                Kind::Phone => "Phone",
                _ => "Wired",
            }
            .into();
            t.label = rate_text(rate_in);
            let addr = f.addrs.iter().find(|(a, _)| a.is_ipv4()).or_else(|| f.addrs.first()).map(|(a, p)| format!("{a}/{p}")).unwrap_or_default();
            t.card.title = format!("Network: {}", f.name);
            t.card.rows = vec![("Address".into(), addr)];
            if let Some(mbps) = f.speed_mbps {
                t.card.rows.push(("Link".into(), if mbps >= 1000 { format!("{} Gb/s", mbps as f64 / 1000.0) } else { format!("{mbps} Mb/s") }));
            }
            t.card.rows.push(("In".into(), rate_text(rate_in)));
            t.card.rows.push(("Out".into(), rate_text(rate_out)));
            t.card.chart = history.to_vec();
            t.card.note = "Coming in, the last minute.".into();
        }
        None => {
            t.value = "Offline".into();
            t.label = "Net".into();
            t.state = TileState::Problem;
            t.card.title = "Network".into();
            t.card.rows = vec![("Connection".into(), "none".into())];
            t.card.note = "No cable, Wi-Fi or phone with an address.".into();
        }
    }
    t
}

/// The Power tile, on a machine with a battery: its charge, and whether it
/// is charging; a warning under 20 %, a problem under 10 % while running
/// down. None without a battery. A click opens Battery.
pub fn power(p: &vergence_power::Power) -> Option<Tile> {
    use vergence_power::State;
    let b = p.batteries.first()?;
    let percent = b.percent?;
    let state = match b.state {
        State::Charging => "Charging",
        State::Discharging => "On battery",
        State::Full => "Full",
        State::NotCharging => "Plugged in",
        State::Unknown => "Battery",
    };
    let draining = b.state == State::Discharging;
    let mut rows = vec![("Charge".to_string(), format!("{percent} %")), ("Now".to_string(), state.to_string())];
    if let Some(m) = b.minutes {
        rows.push((if draining { "Time left" } else { "Until full" }.into(), vergence_power::duration_text(m)));
    }
    if let Some(w) = b.watts.filter(|w| *w > 0.0) {
        rows.push(("Power".into(), format!("{w:.1} W")));
    }
    Some(Tile {
        id: "power".into(),
        value: format!("{percent}%"),
        label: match b.state {
            State::Discharging => "Battery",
            State::Charging => "Charging",
            State::Full => "Full",
            State::NotCharging => "Plugged in",
            State::Unknown => "Battery",
        }
        .into(),
        state: match (draining, percent) {
            (true, p) if p < 10 => TileState::Problem,
            (true, p) if p < 20 => TileState::Warning,
            _ => TileState::Fine,
        },
        group: "System".into(),
        card: Card { title: "Battery".into(), rows, ..Card::default() },
        page: "battery".into(),
    })
}

/// The tiles that stand on the dock when the user has not said: the
/// desktop's own. Every other tile starts in the drawer.
pub const DOCKED: [&str; 4] = ["sound", "network", "power", "clock"];

/// Whether a tile stands on the dock: the user put it there (`docked`,
/// desktop.toml [shelf] tiles, else [`DOCKED`]), or it is in trouble and
/// tiles in trouble come out (`trouble_out`).
pub fn on_dock(docked: Option<&[String]>, id: &str, state: TileState, trouble_out: bool) -> bool {
    let chosen = match docked {
        Some(d) => d.iter().any(|x| x == id),
        None => DOCKED.contains(&id),
    };
    chosen || (trouble_out && state != TileState::Fine)
}

/// The drawer's groups in their order; a group not named here comes after
/// them, by name.
pub const GROUPS: [&str; 3] = ["System", "Lab", "Background apps"];

/// The drawer's grid: four tiles a row, each 60 by 52 points, 4 apart,
/// 10 in from the drawer's edge, a heading of 18 over each section.
pub const DRAWER_COLS: usize = 4;
pub const CELL_W: i32 = 60;
pub const CELL_H: i32 = 52;
const CELL_GAP: i32 = 4;
const DRAWER_PAD: i32 = 10;
const HEADING: i32 = 20;
pub const DRAWER_W: i32 = 2 * DRAWER_PAD + DRAWER_COLS as i32 * CELL_W + (DRAWER_COLS as i32 - 1) * CELL_GAP;

/// The drawer's headings (name, top), cells (tile, place) and height.
pub type DrawerLayout = (Vec<(String, i32)>, Vec<(usize, Rect)>, i32);

/// Where everything in the drawer stands, from its top-left corner: each
/// section's heading and where it is written, each tile's cell (by its
/// place in the list given), and the drawer's height. Tiles needing
/// attention come first under their own heading, then each group in
/// [`GROUPS`]'s order, then the others by name; within a section, in the
/// order given.
pub fn drawer_layout(tiles: &[(String, TileState)]) -> DrawerLayout {
    let mut sections: Vec<(String, Vec<usize>)> = Vec::new();
    let trouble: Vec<usize> = (0..tiles.len()).filter(|i| tiles[*i].1 != TileState::Fine).collect();
    if !trouble.is_empty() {
        sections.push(("Needs attention".into(), trouble));
    }
    let mut names: Vec<String> = GROUPS.iter().map(|g| g.to_string()).collect();
    let mut others: Vec<String> = tiles.iter().map(|(g, _)| if g.is_empty() { "Other".to_string() } else { g.clone() }).filter(|g| !GROUPS.contains(&g.as_str())).collect();
    others.sort();
    others.dedup();
    names.extend(others);
    for name in names {
        let these: Vec<usize> = (0..tiles.len()).filter(|i| tiles[*i].1 == TileState::Fine && (tiles[*i].0 == name || (tiles[*i].0.is_empty() && name == "Other"))).collect();
        if !these.is_empty() {
            sections.push((name, these));
        }
    }
    let (mut headings, mut cells) = (Vec::new(), Vec::new());
    let mut y = DRAWER_PAD;
    for (name, these) in sections {
        headings.push((name, y));
        y += HEADING;
        for (k, i) in these.iter().enumerate() {
            let (col, row) = ((k % DRAWER_COLS) as i32, (k / DRAWER_COLS) as i32);
            cells.push((*i, Rect::new(DRAWER_PAD + col * (CELL_W + CELL_GAP), y + row * (CELL_H + CELL_GAP), CELL_W, CELL_H)));
        }
        let rows = these.len().div_ceil(DRAWER_COLS) as i32;
        y += rows * (CELL_H + CELL_GAP) + 4;
    }
    (headings, cells, y + DRAWER_PAD - 4)
}

#[repr(C)]
struct Tm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    wday: i32,
    yday: i32,
    isdst: i32,
    gmtoff: i64,
    zone: *const std::ffi::c_char,
}

unsafe extern "C" {
    fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    fn gmtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
}

/// `secs` since 1970 in local time and in UTC, by localtime_r(3) and
/// gmtime_r(3) (glibc's and FreeBSD's struct tm agree on these fields).
pub fn now_at(secs: i64) -> Option<(Local, Local)> {
    let blank = || Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
    let (mut l, mut u) = (blank(), blank());
    // SAFETY: each reads the time and fills a struct we own.
    let ok = unsafe { !localtime_r(&secs, &mut l).is_null() && !gmtime_r(&secs, &mut u).is_null() };
    let take = |t: &Tm| Local { year: t.year + 1900, month: t.mon, mday: t.mday, wday: t.wday, hour: t.hour, min: t.min, sec: t.sec };
    ok.then(|| (take(&l), take(&u)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_stand_after_the_applications() {
        let strip = Rect::new(1232, 0, 48, 720);
        assert_eq!(slot(strip, 3, 0, true), Some(Rect::new(1232, 192, 48, 48)));
        assert_eq!(tile_at(strip, 3, 2, true, 1250, 200), Some(0));
        assert_eq!(tile_at(strip, 3, 2, true, 1250, 250), Some(1));
        assert_eq!(tile_at(strip, 3, 2, true, 1250, 150), None, "an application's place");
        assert_eq!(tile_at(strip, 3, 2, true, 1250, 300), None, "past the last tile");
    }

    #[test]
    fn the_clock_says_the_time_and_the_day() {
        let local = Local { year: 2026, month: 10, mday: 16, wday: 1, hour: 8, min: 5, sec: 0 };
        let utc = Local { hour: 14, ..local };
        let t = clock(local, utc, ClockLook::default());
        assert_eq!((t.value.as_str(), t.label.as_str()), ("08:05", "Mon 16"));
        let pm = Local { hour: 18, ..local };
        let t = clock(pm, utc, ClockLook { h24: false, date: false });
        assert_eq!((t.value.as_str(), t.label.as_str(), t.card.big.as_str()), ("6:05", "PM", "6:05 PM"));
        let midnight = Local { hour: 0, ..local };
        assert_eq!(clock(midnight, utc, ClockLook { h24: false, date: true }).value, "12:05");
        assert_eq!(clock(local, utc, ClockLook { h24: true, date: false }).label, "");
        assert_eq!(t.card.title, "Monday, November 16, 2026");
        assert_eq!(t.card.rows, [("UTC".to_string(), "14:05".to_string())]);
        assert_eq!(t.page, "date & time");
    }

    #[test]
    fn utc_at_the_start_of_1970() {
        let (_, u) = now_at(0).unwrap();
        assert_eq!((u.year, u.month, u.mday, u.wday, u.hour, u.min), (1970, 0, 1, 4, 0, 0));
    }

    #[test]
    fn the_sound_tile_says_the_volume_or_muted() {
        let mut o = SoundNow { name: "Speakers".into(), volume: 65, muted: false, adjustable: true };
        let t = sound(Some(&o)).unwrap();
        assert_eq!((t.value.as_str(), t.label.as_str(), t.card.controls.len()), ("65%", "Sound", 2));
        o.muted = true;
        assert_eq!(sound(Some(&o)).unwrap().value, "Off");
        o.adjustable = false;
        o.muted = false;
        let t = sound(Some(&o)).unwrap();
        assert_eq!((t.value.as_str(), t.card.controls.len()), ("On", 1), "only mute where the device sets the volume");
        assert!(sound(None).is_none());
    }

    #[test]
    fn rates_read_short() {
        assert_eq!(rate_text(0.0), "0 b/s");
        assert_eq!(rate_text(105_000.0), "840 kb/s");
        assert_eq!(rate_text(150_000.0), "1.2 Mb/s");
        assert_eq!(rate_text(1_500_000.0), "12 Mb/s");
        assert_eq!(rate_text(150_000_000.0), "1.2 Gb/s");
    }

    #[test]
    fn the_network_tile_finds_the_connection() {
        use vergence_net::ifaces::{Iface, Kind};
        let iface = |name: &str, kind: Kind, carrier: bool, addr: &str| Iface { name: name.into(), kind, up: true, carrier, addrs: vec![(addr.parse().unwrap(), 24)], ..Iface::default() };
        let all = vec![iface("lo", Kind::Loopback, true, "127.0.0.1"), iface("wlan0", Kind::WiFi, true, "192.168.1.5"), iface("em0", Kind::Wired, true, "192.168.1.4"), iface("em1", Kind::Wired, false, "169.254.1.1")];
        assert_eq!(main_iface(&all).map(|f| f.name.as_str()), Some("em0"), "a cable first");
        let t = network(&all, 1_500_000.0, 1000.0, &[1.0, 2.0]);
        assert_eq!((t.value.as_str(), t.label.as_str(), t.state), ("Wired", "12 Mb/s", TileState::Fine));
        assert_eq!(t.card.chart.len(), 2);
        let none = network(&all[..1], 0.0, 0.0, &[]);
        assert_eq!((none.value.as_str(), none.state), ("Offline", TileState::Problem));
        let link_local = vec![iface("em1", Kind::Wired, true, "169.254.1.1")];
        assert!(main_iface(&link_local).is_none(), "no address the network gave");
    }

    #[test]
    fn the_power_tile_warns_when_low() {
        use vergence_power::{Battery, Power, State};
        let p = |percent: u8, state: State| Power { on_mains: None, batteries: vec![Battery { percent: Some(percent), state, minutes: Some(95), ..Battery::default() }] };
        let t = power(&p(64, State::Discharging)).unwrap();
        assert_eq!((t.value.as_str(), t.label.as_str(), t.state), ("64%", "Battery", TileState::Fine));
        assert!(t.card.rows.iter().any(|(k, v)| k == "Time left" && v == "1 h 35 min"));
        assert_eq!(power(&p(15, State::Discharging)).unwrap().state, TileState::Warning);
        assert_eq!(power(&p(8, State::Discharging)).unwrap().state, TileState::Problem);
        assert_eq!(power(&p(8, State::Charging)).unwrap().state, TileState::Fine, "charging is no problem");
        assert!(power(&Power { on_mains: Some(true), batteries: vec![] }).is_none());
    }

    #[test]
    fn what_stands_on_the_dock() {
        assert!(on_dock(None, "clock", TileState::Fine, true));
        assert!(!on_dock(None, "ups", TileState::Fine, true), "a program's tile starts in the drawer");
        assert!(on_dock(None, "ups", TileState::Warning, true), "until it is in trouble");
        assert!(!on_dock(None, "ups", TileState::Problem, false), "unless trouble stays in the drawer");
        let mine = vec!["ups".to_string()];
        assert!(on_dock(Some(&mine), "ups", TileState::Fine, true));
        assert!(!on_dock(Some(&mine), "clock", TileState::Fine, true), "the user's list, not the default");
    }

    #[test]
    fn the_drawer_puts_trouble_first_then_groups() {
        let t = |g: &str, s: TileState| (g.to_string(), s);
        let tiles = vec![t("Lab", TileState::Fine), t("System", TileState::Fine), t("Lab", TileState::Problem), t("", TileState::Fine), t("Lab", TileState::Fine), t("Zoo", TileState::Fine)];
        let (headings, cells, h) = drawer_layout(&tiles);
        let names: Vec<&str> = headings.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Needs attention", "System", "Lab", "Other", "Zoo"]);
        let order: Vec<usize> = cells.iter().map(|(i, _)| *i).collect();
        assert_eq!(order, [2, 1, 0, 4, 3, 5]);
        assert_eq!(cells[0].1, Rect::new(10, 30, CELL_W, CELL_H));
        assert_eq!(cells[3].1.x, 10 + CELL_W + 4, "the second in its section stands beside the first");
        assert!(h > cells.last().unwrap().1.bottom());
        assert_eq!(DRAWER_W, 272);
        let five: Vec<_> = (0..5).map(|_| t("System", TileState::Fine)).collect();
        let (_, cells, _) = drawer_layout(&five);
        assert_eq!((cells[4].1.x, cells[4].1.y), (10, 30 + CELL_H + 4), "a fifth starts the next row");
        assert_eq!(drawer_layout(&[]).1.len(), 0);
    }
}
