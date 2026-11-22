// A screen's resolution and refresh rate (settings milestone 2, step 2): the
// monitor's modes from the bus (protocol v9), written as desktop.toml
// `[[displays.screen]] mode = "3840x2160@60"`, which the server sets only if
// the monitor offers it. A new mode asks "Keep this resolution?" and goes back
// by itself after 15 s unless kept (Dave: "1 yes"), closing the dialog
// included, so a mode the monitor cannot show never strands anyone.

use std::time::{Duration, Instant};

use toml::{Table, Value};
use vergence_protocol::bus::{ModeInfo, OutputInfo};

use super::*;

/// How long a new mode lasts unless kept.
pub const KEEP_FOR: Duration = Duration::from_secs(15);
/// The settings window's timer for going back.
pub const REVERT_TIMER: u64 = 0x6d6f6465;

/// The distinct resolutions a screen offers, in the monitor's order.
pub fn resolutions(modes: &[ModeInfo]) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for m in modes {
        if !out.contains(&(m.width, m.height)) {
            out.push((m.width, m.height));
        }
    }
    out
}

/// The refresh rates of one resolution, fastest first.
pub fn rates(modes: &[ModeInfo], size: (u32, u32)) -> Vec<ModeInfo> {
    let mut v: Vec<ModeInfo> = modes.iter().copied().filter(|m| (m.width, m.height) == size).collect();
    v.sort_by(|a, b| b.refresh_mhz.cmp(&a.refresh_mhz));
    v
}

/// "60 Hz", "59.94 Hz (video)": a rate a thousandth below a whole one
/// (59.94, 29.97, 23.976, 119.88) is television's, kept for video that is
/// made at it (Dave, 2026-11-14: "label it").
pub fn rate_words(mhz: u32) -> String {
    if mhz % 1000 == 0 {
        return format!("{} Hz", mhz / 1000);
    }
    let whole = ((mhz as f64 / 1000.0) * 1.001).round();
    let video = whole > 0.0 && ((whole * 1000.0 / 1.001) - mhz as f64).abs() < 15.0;
    let n = format!("{:.3}", mhz as f64 / 1000.0);
    let n = n.trim_end_matches('0').trim_end_matches('.');
    if video { format!("{n} Hz (video)") } else { format!("{n} Hz") }
}

/// The mode as desktop.toml writes it.
pub fn mode_text(m: &ModeInfo) -> String {
    let hz = if m.refresh_mhz % 1000 == 0 { format!("{}", m.refresh_mhz / 1000) } else { format!("{:.3}", m.refresh_mhz as f64 / 1000.0).trim_end_matches('0').to_string() };
    format!("{}x{}@{hz}", m.width, m.height)
}

/// The mode chosen for a screen in desktop.toml, as written.
pub fn chosen(desktop: &Table, id: &str) -> Option<String> {
    desktop
        .get("displays")
        .and_then(|d| d.get("screen"))
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|s| s.get("id").and_then(Value::as_str) == Some(id)))
        .and_then(|e| e.get("mode"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Write (or with None, clear) a screen's chosen mode.
pub fn set_chosen(desktop: &mut Table, id: &str, label: &str, mode: Option<&str>) {
    let displays = desktop.entry("displays").or_insert_with(|| Value::Table(Table::new()));
    let Some(displays) = displays.as_table_mut() else { return };
    let list = displays.entry("screen").or_insert_with(|| Value::Array(Vec::new()));
    let Some(list) = list.as_array_mut() else { return };
    let i = match list.iter().position(|s| s.get("id").and_then(Value::as_str) == Some(id)) {
        Some(i) => i,
        None => {
            let mut t = Table::new();
            t.insert("id".into(), id.into());
            list.push(Value::Table(t));
            list.len() - 1
        }
    };
    let Some(t) = list[i].as_table_mut() else { return };
    t.insert("label".into(), label.into());
    match mode {
        Some(m) => t.insert("mode".into(), m.into()),
        None => t.remove("mode"),
    };
}

/// A new mode waiting to be kept: the screen, and what was chosen before
/// (None: nothing, the monitor's preferred mode).
pub struct Pending {
    pub id: String,
    pub label: String,
    pub before: Option<String>,
}

impl App {
    /// Rows for a screen's page: Resolution and Refresh rate.
    pub fn mode_rows(&self, o: &OutputInfo) -> Vec<Element<Message>> {
        let Some(running) = o.modes.get(o.mode as usize).copied() else { return Vec::new() };
        let sizes = resolutions(&o.modes);
        let size_names: Vec<String> = sizes
            .iter()
            .map(|(w, h)| {
                let best = o.modes.iter().any(|m| (m.width, m.height) == (*w, *h) && m.preferred);
                format!("{w} \u{d7} {h}{}", if best { " (recommended)" } else { "" })
            })
            .collect();
        let at = sizes.iter().position(|s| *s == (running.width, running.height));
        let its_rates = rates(&o.modes, (running.width, running.height));
        let rate_names: Vec<String> = its_rates.iter().map(|m| rate_words(m.refresh_mhz)).collect();
        let rate_at = its_rates.iter().position(|m| m.refresh_mhz == running.refresh_mhz);
        let one = o.modes.len() <= 1;
        let mut rows = vec![setting(
            "Resolution",
            if one { "The only one this screen offers." } else { "The screen's own pixels; the scale above sizes things on them." },
            choice(size_names, at, Message::Resolution).width(Length::Fixed(220.0)).name("Resolution"),
        )];
        if its_rates.len() > 1 {
            rows.push(setting("Refresh rate", "", choice(rate_names, rate_at, Message::Refresh).width(Length::Fixed(140.0)).name("Refresh rate")));
        } else if let Some(r) = its_rates.first() {
            rows.push(setting("Refresh rate", "", label(rate_words(r.refresh_mhz)).dim()));
        }
        rows
    }

    /// The mode picked from the choices: written, and asked about.
    pub fn pick_mode(&mut self, resolution: Option<usize>, refresh: Option<usize>) {
        let Some(o) = self.current_output().cloned() else { return };
        let Some(running) = o.modes.get(o.mode as usize).copied() else { return };
        let sizes = resolutions(&o.modes);
        let size = resolution.and_then(|i| sizes.get(i).copied()).unwrap_or((running.width, running.height));
        let its_rates = rates(&o.modes, size);
        let mode = match refresh {
            Some(i) => its_rates.get(i).copied(),
            // A new resolution: its preferred rate, else its fastest.
            None => its_rates.iter().find(|m| m.preferred).or(its_rates.first()).copied(),
        };
        let Some(mode) = mode else { return };
        if (mode.width, mode.height, mode.refresh_mhz) == (running.width, running.height, running.refresh_mhz) {
            return;
        }
        let label = screen_label(&o);
        let before = chosen(&self.desktop, &o.id);
        let text = mode_text(&mode);
        let (id, l) = (o.id.clone(), label.clone());
        self.save(|d| set_chosen(d, &id, &l, Some(&text)));
        // Asked only once per change: a second change while asking keeps the first "before".
        if self.mode_pending.is_none() {
            self.mode_pending = Some(Pending { id: o.id.clone(), label, before });
        }
        self.mode_ask = true;
    }

    /// Kept, or gone back to what was chosen before.
    pub fn settle_mode(&mut self, keep: bool) {
        let Some(p) = self.mode_pending.take() else { return };
        if !keep {
            let before = p.before.clone();
            self.save(|d| set_chosen(d, &p.id, &p.label, before.as_deref()));
            self.status = format!("{}: back to its earlier resolution.", p.label);
        }
    }
}

/// "Keep this resolution?" over the settings window, counting down; Keep and
/// Revert tell the settings window, which goes back by itself when its own
/// 15 s timer runs out (closing this dialog with its bar's button too).
pub struct KeepDialog {
    pub mail: Mail,
    pub until: Instant,
    /// The settings window's look (light or dark, accent), not the toolkit's default.
    pub theme: Theme,
}

#[derive(Debug, Clone)]
pub enum KeepMsg {
    Keep,
    Revert,
}

impl Program for KeepDialog {
    type Message = KeepMsg;
    fn title(&self) -> String {
        "Keep this resolution?".into()
    }
    fn app_id(&self) -> String {
        "vergence-settings".into()
    }
    fn theme(&self) -> Theme {
        self.theme.clone()
    }
    fn view(&self, _: &Theme) -> Element<KeepMsg> {
        let left = self.until.saturating_duration_since(Instant::now()).as_secs_f32().ceil() as u32;
        panel(column(vec![
            label("Keep this resolution?").size(15.0).weight(600),
            label(format!("The screen goes back to the earlier one in {left} s unless you keep it.")).dim().wrap().width(Length::FILL),
            row(vec![
                space().width(Length::FILL),
                button(label("Go back").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(KeepMsg::Revert).name("Go back"),
                button(label("Keep").size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(4.0, 14.0)).on_press(KeepMsg::Keep).name("Keep"),
            ])
            .spacing(8.0)
            .width(Length::FILL),
        ])
        .spacing(10.0)
        .width(Length::FILL))
        .padding(20.0)
        .radius(0.0)
        .width(Length::FILL)
        .height(Length::FILL)
        .into()
    }
    fn update(&mut self, m: KeepMsg, window: &mut Window) {
        self.mail.send(Message::KeepMode(matches!(m, KeepMsg::Keep)));
        window.close();
    }
    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { kind: vergence_ui::WinKind::Dialog, modal: true, min: (380.0, 150.0), max: (0.0, 0.0), alone: false, frosted: false }
    }
    /// The countdown: a thread wakes the dialog twice a second until the
    /// time is up (a new window has no hook to start a timer of its own).
    fn waker(&mut self, waker: Waker) {
        let until = self.until;
        let _ = std::thread::Builder::new().name("keep-mode".into()).spawn(move || {
            while Instant::now() < until + Duration::from_secs(1) {
                std::thread::sleep(Duration::from_millis(500));
                waker.wake();
            }
        });
    }
    fn woken(&mut self, window: &mut Window) -> bool {
        // Time is up: the settings window has gone back by itself.
        if Instant::now() >= self.until {
            window.close();
        }
        true
    }
    fn scale(&mut self, _: f32) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(w: u32, h: u32, r: u32, p: bool) -> ModeInfo {
        ModeInfo { width: w, height: h, refresh_mhz: r, preferred: p }
    }

    #[test]
    fn modes_sorted_and_written() {
        let modes = [m(3840, 2160, 30000, true), m(3840, 2160, 29970, false), m(2560, 1440, 60000, false), m(1920, 1080, 59940, false), m(1920, 1080, 60000, false)];
        assert_eq!(resolutions(&modes), vec![(3840, 2160), (2560, 1440), (1920, 1080)]);
        let r = rates(&modes, (1920, 1080));
        assert_eq!(r.iter().map(|m| m.refresh_mhz).collect::<Vec<_>>(), vec![60000, 59940], "fastest first");
        assert_eq!(rate_words(60000), "60 Hz");
        assert_eq!(rate_words(59940), "59.94 Hz (video)");
        assert_eq!(rate_words(23976), "23.976 Hz (video)");
        assert_eq!(rate_words(60020), "60.02 Hz");
        assert_eq!(mode_text(&m(1920, 1080, 59940, false)), "1920x1080@59.94");
        assert_eq!(mode_text(&m(3840, 2160, 60000, true)), "3840x2160@60");
        let mut d = Table::new();
        set_chosen(&mut d, "ae05", "HDMI", Some("1920x1080@60"));
        assert_eq!(chosen(&d, "ae05").as_deref(), Some("1920x1080@60"));
        set_chosen(&mut d, "ae05", "HDMI", None);
        assert_eq!(chosen(&d, "ae05"), None);
    }
}
