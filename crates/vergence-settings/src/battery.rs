// The Battery page (Dave, 2026-11-14: "yes, add a battery page", after
// "this thing eats the battery"): the charge, time left, the power drawn now
// with a chart of the last two minutes, which GPUs are awake, and in
// Advanced the battery's health and make. From vergence-power (Linux's
// /sys, FreeBSD's ACPI sysctls). Shown in the sidebar only where there is a
// battery.

use std::collections::VecDeque;
use std::time::Duration;

use vergence_power::history::{self, Record};
use vergence_power::{Gpu, Power, State, duration_text};

use super::*;

/// Samples kept: two minutes at one every two seconds.
const HISTORY: usize = 60;
const SAMPLE: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct BatteryPage {
    pub power: Power,
    gpus: Vec<Gpu>,
    /// Watts drawn (or taken in), oldest first, while the page is open.
    history: VecDeque<f64>,
    started: bool,
    /// The kept history (the server writes it): the last day or week.
    range: usize,
    records: Vec<Record>,
    records_read: Option<std::time::Instant>,
}

/// The history's ranges, in seconds, and their names.
const RANGES: [(u64, &str, &str); 2] =
    [(24 * 3600, "Last 24 hours", "24 hours ago"), (7 * 24 * 3600, "Last 7 days", "7 days ago")];

impl App {
    /// Whether the machine has a battery (the sidebar shows the page then).
    pub fn has_battery(&self) -> bool {
        !self.battery.power.batteries.is_empty()
    }

    pub fn battery_read(&mut self) {
        if !self.battery.started {
            self.battery.started = true;
            let mail = self.mail.clone();
            let _ = std::thread::Builder::new().name("battery".into()).spawn(move || {
                loop {
                    std::thread::sleep(SAMPLE);
                    if !mail.send(Message::BatterySample) {
                        return;
                    }
                }
            });
        }
        self.battery.power = vergence_power::power();
        self.battery.gpus = vergence_power::gpus();
        // The kept history, once a minute (the server writes a line a minute).
        if self.battery.records_read.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
            self.battery.records_read = Some(std::time::Instant::now());
            let since = history::now().saturating_sub(RANGES[1].0);
            self.battery.records = history::path().map(|p| history::read(&p, since)).unwrap_or_default();
        }
    }

    pub fn battery_range(&mut self, i: usize) {
        self.battery.range = i.min(RANGES.len() - 1);
    }

    pub fn battery_sample(&mut self) {
        if self.page == Page::Energy {
            // Only "(now)" on the Energy page follows the plug.
            self.battery.power = vergence_power::power();
            return;
        }
        if self.page != Page::Battery {
            return;
        }
        self.battery_read();
        let w = self.battery.power.batteries.first().and_then(|b| b.watts);
        if let Some(w) = w {
            self.battery.history.push_back(w);
            while self.battery.history.len() > HISTORY {
                self.battery.history.pop_front();
            }
        }
    }

    pub fn battery_page(&self) -> Element<Message> {
        let about = "How much charge is left, how long it lasts, and what is drawing power.";
        let p = &self.battery.power;
        let mut items = Vec::new();
        let value = |t: String| label(t).dim();
        for b in &p.batteries {
            let pct = b.percent.unwrap_or(0);
            let state = match b.state {
                State::Charging => "charging",
                State::Discharging => "on battery",
                State::Full => "full",
                State::NotCharging => "plugged in, not charging",
                State::Unknown => "",
            };
            let mains = match (b.state, p.on_mains) {
                (State::Discharging, _) | (_, None) => String::new(),
                (_, Some(true)) => ", on power".into(),
                (_, Some(false)) => String::new(),
            };
            let headline = if state.is_empty() { format!("{pct}%") } else { format!("{pct}%, {state}{mains}") };
            let mut rows = vec![setting(&headline, "", battery_mark(pct, b.state == State::Charging))];
            let left = match (b.state, b.minutes) {
                (State::Discharging, Some(m)) => Some(("Time left", format!("About {}", duration_text(m)))),
                (State::Charging, Some(m)) => Some(("Full in", format!("About {}", duration_text(m)))),
                (State::Discharging, None) | (State::Charging, None) => {
                    Some(("Time left", "Working it out\u{2026}".into()))
                }
                _ => None,
            };
            if let Some((name, text)) = left {
                rows.push(setting(name, "", value(text)));
            }
            // Always there, so the page never shifts under the pointer.
            let now = match (b.watts, b.state) {
                (Some(w), State::Charging) => format!("{w:.1} W going in"),
                (Some(w), _) if w > 0.05 => format!("{w:.1} W"),
                _ => "\u{2014}".into(),
            };
            let chart: Vec<f64> = self.battery.history.iter().copied().collect();
            rows.push(setting(
                "Drawing now",
                if b.state == State::Discharging {
                    "The last two minutes."
                } else {
                    "Shown on battery: on power the charger feeds the machine."
                },
                row(vec![power_chart(chart), value(now).width(Length::Fixed(120.0)).align(Align::End)])
                    .spacing(12.0)
                    .align(Align::Center),
            ));
            let title = if p.batteries.len() > 1 { b.name.clone() } else { "Battery".into() };
            items.push(self.group(Some(&title), rows));
        }
        // The kept history: the charge over a day or a week, and what it comes to.
        if !p.batteries.is_empty() {
            let (span, _, ago) = RANGES[self.battery.range];
            let to = history::now();
            let from = to.saturating_sub(span);
            let shown: Vec<Record> = self.battery.records.iter().copied().filter(|r| r.t >= from).collect();
            let names: Vec<String> = RANGES.iter().map(|(_, n, _)| n.to_string()).collect();
            let sum = history::summary(&shown);
            let words = if sum.on_battery < 60 {
                if shown.is_empty() {
                    "Nothing kept yet: a line is written every minute.".to_string()
                } else {
                    "Not on battery in this time.".to_string()
                }
            } else {
                let mut w = format!("On battery {}, {}% used", duration_text((sum.on_battery / 60) as u32), sum.used);
                if let Some(a) = sum.avg_watts {
                    w.push_str(&format!(", {a:.1} W on average"));
                }
                w.push('.');
                w
            };
            let chart = panel(
                column(vec![
                    charge_chart(shown, from, to),
                    row(vec![label(ago).size(10.0).dim(), space().width(Length::FILL), label("now").size(10.0).dim()])
                        .width(Length::FILL),
                ])
                .spacing(2.0)
                .width(Length::FILL),
            )
            .padding(Padding::hv(6.0, 12.0))
            .width(Length::FILL);
            items.push(self.group(
                Some("History"),
                vec![
                        setting(
                            "Charge",
                            &words,
                            choice(names, Some(self.battery.range), Message::BatteryRange)
                                .width(Length::Fixed(150.0))
                                .name("History range"),
                        ),
                        chart,
                    ],
            ));
        }
        // The GPUs: an awake discrete one is the usual drain.
        if !self.battery.gpus.is_empty() {
            let several = self.battery.gpus.len() > 1;
            let rows = self
                .battery
                .gpus
                .iter()
                .map(|g| {
                    let name = if g.driver.is_empty() {
                        format!("{} graphics", g.vendor)
                    } else {
                        format!("{} graphics ({})", g.vendor, g.driver)
                    };
                    let (word, note) = match g.awake {
                        Some(true)
                            if several && g.vendor == "NVIDIA"
                                || several && g.vendor == "AMD" && g.driver != "amdgpu-igpu" =>
                        {
                            ("Awake", "Draws several watts while awake; it sleeps when nothing uses it.")
                        }
                        Some(true) => ("Awake", "Draws the screens."),
                        Some(false) => ("Asleep", "Uses next to no power."),
                        None => ("\u{2014}", "The system does not say whether it sleeps."),
                    };
                    setting(&name, note, value(word.into()))
                })
                .collect();
            items.push(self.group(Some("Graphics"), rows));
        }
        // Advanced: the battery's health and make.
        let mut adv = Vec::new();
        for b in &p.batteries {
            if let Some(h) = b.health() {
                // In the battery's own units: a charge-based one's watt-hours
                // would be worked out at today's voltage, which sags as it runs down.
                let detail = match (b.full_mah, b.design_mah, b.full_wh, b.design_wh) {
                    (Some(f), Some(d), _, _) => format!("Holds {f:.0} of {d:.0} mAh when new."),
                    (_, _, Some(f), Some(d)) => format!("Holds {f:.1} of {d:.1} Wh when new."),
                    _ => String::new(),
                };
                adv.push(setting(&format!("Health: {h}%"), &detail, space()));
            } else if let Some(f) = b.full_wh {
                adv.push(setting(
                    "Holds when full",
                    &format!("{f:.1} Wh (its design capacity is not reported)."),
                    space(),
                ));
            }
            if let Some(c) = b.cycles {
                adv.push(setting("Charge cycles", &c.to_string(), space()));
            }
            if let Some(v) = b.volts {
                adv.push(setting("Voltage", &format!("{v:.2} V"), space()));
            }
            if !b.model.is_empty() || !b.technology.is_empty() {
                adv.push(setting(
                    "Battery",
                    &[b.model.as_str(), b.technology.as_str()]
                        .iter()
                        .filter(|s| !s.is_empty())
                        .copied()
                        .collect::<Vec<_>>()
                        .join(", "),
                    space(),
                ));
            }
        }
        for g in &self.battery.gpus {
            adv.push(setting(&format!("GPU {}", g.slot), &format!("{} {}", g.vendor, g.driver), space()));
        }
        items.push(self.advanced("battery", adv));
        self.page_layout("Battery", about, items)
    }
}

/// A battery drawn, filled to its charge in green, amber below 20%, red
/// below 10%, with a bolt while charging.
fn battery_mark<M: 'static>(pct: u8, charging: bool) -> Element<M> {
    drawing(move |scene: &mut Scene, _: &mut Fonts, r: Rect, t: &Theme| {
        let (w, h) = (44.0f32, 20.0f32);
        let (x, y) = (r.x + r.w - w - 6.0, r.y + (r.h - h) / 2.0);
        scene.stroke(Rect::new(x, y, w, h), 4.0, 1.5, t.text);
        scene.fill_rounded(Rect::new(x + w + 1.0, y + h / 2.0 - 3.5, 3.0, 7.0), 1.5, t.text);
        let color = match pct {
            0..=9 => Color::rgb(0xe5, 0x3e, 0x35),
            10..=19 => Color::rgb(0xf0, 0x9a, 0x0a),
            _ => Color::rgb(0x30, 0xb0, 0x4f),
        };
        let fill = (w - 5.0) * pct.min(100) as f32 / 100.0;
        if fill > 0.5 {
            scene.fill_rounded(Rect::new(x + 2.5, y + 2.5, fill, h - 5.0), 2.0, color);
        }
        if charging {
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            let ink = t.text;
            scene.line(Point::new(cx + 2.0, cy - 6.0), Point::new(cx - 2.5, cy + 0.5), 1.6, ink);
            scene.line(Point::new(cx - 2.5, cy + 0.5), Point::new(cx + 2.5, cy - 0.5), 1.6, ink);
            scene.line(Point::new(cx + 2.5, cy - 0.5), Point::new(cx - 2.0, cy + 6.0), 1.6, ink);
        }
    })
    .width(Length::Fixed(60.0))
    .height(Length::Fixed(26.0))
}

/// The charge from `from` to `to` (seconds since 1970): 0 to 100%, a line
/// broken where the machine slept or was off, a green band under the
/// stretches spent charging.
fn charge_chart<M: 'static>(records: Vec<Record>, from: u64, to: u64) -> Element<M> {
    drawing(move |scene: &mut Scene, _: &mut Fonts, r: Rect, t: &Theme| {
        let faint = Color::rgba(t.dim.r, t.dim.g, t.dim.b, 45);
        for pct in [0.0f32, 50.0, 100.0] {
            let y = r.y + r.h - pct / 100.0 * r.h;
            scene.line(Point::new(r.x, y), Point::new(r.x + r.w, y), 1.0, faint);
        }
        let span = to.saturating_sub(from).max(1) as f32;
        let x_of = |s: u64| r.x + (s.saturating_sub(from) as f32 / span).clamp(0.0, 1.0) * r.w;
        let y_of = |p: u8| r.y + r.h - p.min(100) as f32 / 100.0 * r.h;
        let green = Color::rgba(0x30, 0xb0, 0x4f, 70);
        for pair in records.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if b.t <= a.t || b.t - a.t > history::GAP {
                continue;
            }
            if a.state == State::Charging {
                let (xa, xb) = (x_of(a.t), x_of(b.t));
                scene.fill(Rect::new(xa, r.y, (xb - xa).max(1.0), r.h), green);
            }
            scene.line(Point::new(x_of(a.t), y_of(a.percent)), Point::new(x_of(b.t), y_of(b.percent)), 1.8, t.accent);
        }
    })
    .width(Length::FILL)
    .height(Length::Fixed(90.0))
}

/// Watts over the last two minutes, from 0 (bottom) to the larger of 10 W
/// and the most seen, the newest at the right.
fn power_chart<M: 'static>(samples: Vec<f64>) -> Element<M> {
    drawing(move |scene: &mut Scene, _: &mut Fonts, r: Rect, t: &Theme| {
        let faint = Color::rgba(t.dim.r, t.dim.g, t.dim.b, 50);
        scene.line(Point::new(r.x, r.y + r.h), Point::new(r.x + r.w, r.y + r.h), 1.0, faint);
        if samples.len() < 2 {
            return;
        }
        let top = samples.iter().copied().fold(10.0f64, f64::max) * 1.1;
        let y_of = |w: f64| r.y + r.h - (w / top).clamp(0.0, 1.0) as f32 * r.h;
        let step = r.w / (HISTORY - 1) as f32;
        let x0 = r.x + r.w - step * (samples.len() - 1) as f32;
        let ink = t.accent;
        for (i, pair) in samples.windows(2).enumerate() {
            scene.line(
                Point::new(x0 + step * i as f32, y_of(pair[0])),
                Point::new(x0 + step * (i + 1) as f32, y_of(pair[1])),
                1.8,
                ink,
            );
        }
    })
    .width(Length::Fixed(180.0))
    .height(Length::Fixed(30.0))
}
