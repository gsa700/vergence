// The Date & Time page (dock milestone 1, step 4): what the clock tile's
// click opens. The time zone as the system has it, and how the dock's clock
// shows the time: desktop.toml [clock] 24-hour (default on) and date
// (default on); the server's clock tile reads the same keys.

use super::*;

/// The system's time zone by name: Linux points /etc/localtime into the
/// zoneinfo tree; FreeBSD copies the file there and writes the name in
/// /var/db/zoneinfo (tzsetup). $TZ first, as programs see it.
pub fn time_zone() -> Option<String> {
    if let Ok(tz) = std::env::var("TZ")
        && !tz.is_empty()
    {
        return Some(tz.trim_start_matches(':').to_string());
    }
    if let Ok(target) = std::fs::read_link("/etc/localtime") {
        let t = target.to_string_lossy();
        if let Some(i) = t.find("zoneinfo/") {
            return Some(t[i + "zoneinfo/".len()..].to_string());
        }
    }
    std::fs::read_to_string("/var/db/zoneinfo").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn table(d: &toml::Table) -> Option<&toml::Table> {
    d.get("clock").and_then(|n| n.as_table())
}

/// [clock] 24-hour and date, with their defaults (both on).
pub fn clock_look(d: &toml::Table) -> (bool, bool) {
    let t = table(d);
    let get = |k: &str| t.and_then(|t| t.get(k)).and_then(|v| v.as_bool()).unwrap_or(true);
    (get("24-hour"), get("date"))
}

#[derive(Clone, Debug)]
pub enum ClockMsg {
    TwentyFour(bool),
    Date(bool),
}

impl App {
    pub fn clock_message(&mut self, m: ClockMsg) {
        let (key, on) = match m {
            ClockMsg::TwentyFour(on) => ("24-hour", on),
            ClockMsg::Date(on) => ("date", on),
        };
        self.save_table("clock", |t| {
            t.insert(key.into(), on.into());
        });
    }

    pub fn datetime_page(&self) -> Element<Message> {
        let (h24, date) = clock_look(&self.desktop);
        let zone = time_zone().unwrap_or_else(|| "not known".into());
        self.page_layout(
            "Date & Time",
            "The clock on the dock, and the time zone this computer keeps.",
            vec![
                self.group(Some("The dock's clock"), vec![
                    setting("24-hour clock", if h24 { "18:42" } else { "6:42, with AM or PM in its card" }, switch(h24, |on| Message::Clock(ClockMsg::TwentyFour(on))).name("24-hour clock")),
                    setting("Show the date", "The day under the time, as \"Mon 16\".", switch(date, |on| Message::Clock(ClockMsg::Date(on))).name("Show the date")),
                ]),
                self.group(Some("Time"), vec![setting(
                    "Time zone",
                    "Set for the whole system (tzsetup on FreeBSD, the system's own tool on Linux); every user sees the same.",
                    label(&zone),
                )]),
                self.status_line(),
            ],
        )
    }
}
