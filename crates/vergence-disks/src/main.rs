//! Vergence's Disks app: the disks, their partitions and file systems, and ZFS pools.
//!
//! Disks milestone 1, step 1 (docs/disks-milestone-1.md): seeing only, as any user may. The
//! readings are this crate's library (sysfs and udev on Linux, GEOM on FreeBSD, `zpool` for
//! pools), read again every five seconds; Open and Eject go through the toolkit's drive actions
//! (the server's, drives milestone 1). Health (behind Advanced) and changes (Erase, Rename,
//! Check, Scrub) come with vdm's actions in later steps.
//!
//! Start reading: `Disks::view`, then `disk_page` and `pool_page`.
//!
//! Rule: read-only; nothing here opens a device or reads a disk's bytes.

use std::path::Path;
use std::time::Duration;

use vergence_disks::{Disk, Kind, Part, Pool, Segment, size_text};
use vergence_scene::Color;
use vergence_ui::{AppCommand as C, BarStyle, Role};
use vergence_widgets::chart::meter;
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Program, Theme, Waker, Window, button, column, label, panel, row,
    rule, scroll, space, vrule,
};

const APP_ID: &str = "vergence-disks";
const SIDEBAR: f32 = 220.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sel {
    Disk(usize),
    Pool(usize),
}

#[derive(Debug, Clone)]
enum Msg {
    Pick(Sel),
    Part(usize),
    Open(String),
    Eject(String),
}

struct Disks {
    disks: Vec<Disk>,
    pools: Vec<Pool>,
    /// What is chosen, kept by name across readings (disks come and go).
    sel: Option<(bool, String)>,
    part: Option<String>,
    /// The last action's outcome, shown under the page's title ("Safe to remove" comes from
    /// the server; this is for a refusal).
    note: String,
    desktop: String,
}

impl Disks {
    fn new() -> Disks {
        let desktop = vergence_config::effective_text_with(vergence_widgets::prefs::desktop_path().as_deref());
        let mut d = Disks { disks: Vec::new(), pools: Vec::new(), sel: None, part: None, note: String::new(), desktop };
        d.read();
        d
    }

    fn read(&mut self) {
        self.disks = vergence_disks::disks();
        self.pools = vergence_disks::pools();
        if self.current().is_none() {
            self.sel = self.disks.first().map(|d| (false, d.name.clone()));
            self.part = None;
        }
    }

    fn current(&self) -> Option<Sel> {
        let (pool, name) = self.sel.as_ref()?;
        if *pool {
            self.pools.iter().position(|p| &p.name == name).map(Sel::Pool)
        } else {
            self.disks.iter().position(|d| &d.name == name).map(Sel::Disk)
        }
    }

    fn group(&self, t: &Theme, title: Option<&str>, rows: Vec<Element<Msg>>) -> Element<Msg> {
        let mut inner = Vec::new();
        for (i, r) in rows.into_iter().enumerate() {
            if i > 0 {
                inner.push(panel(rule()).padding(Padding::hv(0.0, 12.0)).width(Length::FILL));
            }
            inner.push(r);
        }
        let card = panel(column(inner).width(Length::FILL)).color(t.card).radius(10.0).padding(2.0).width(Length::FILL);
        match title {
            Some(h) => column(vec![label(h).size(12.0).dim().weight(600), card]).spacing(6.0).width(Length::FILL),
            None => card,
        }
    }

    fn line(name: &str, value: impl Into<String>) -> Element<Msg> {
        panel(
            row(vec![label(name).width(Length::FILL), label(value.into()).dim().align(Align::End)])
                .spacing(16.0)
                .align(Align::Center)
                .width(Length::FILL),
        )
        .padding(Padding::hv(8.0, 12.0))
        .width(Length::FILL)
    }

    fn wide(child: Element<Msg>) -> Element<Msg> {
        panel(child).padding(Padding::hv(10.0, 12.0)).width(Length::FILL)
    }

    /// The partition bar: each segment a button as wide as its share of the disk (a small one
    /// still wide enough to press), tinted by its file system, the chosen one ringed.
    fn bar(&self, t: &Theme, d: &Disk) -> Element<Msg> {
        let segs = d.segments();
        let total = d.size.max(1) as f64;
        let cells: Vec<Element<Msg>> = segs
            .iter()
            .map(|s| {
                let (title, size, fill, msg, chosen) = match s {
                    Segment::Part(i) => {
                        let p = &d.parts[*i];
                        let title = if !p.label.is_empty() {
                            p.label.clone()
                        } else if !p.fs.is_empty() {
                            p.fs.clone()
                        } else {
                            p.name.clone()
                        };
                        let chosen = self.part.as_deref() == Some(p.name.as_str());
                        (title, p.size, fs_color(&p.fs, t), Some(Msg::Part(*i)), chosen)
                    }
                    Segment::Free { size, .. } => {
                        ("Free".to_string(), *size, Color::rgba(t.dim.r, t.dim.g, t.dim.b, 25), None, false)
                    }
                };
                let share = size as f64 / total;
                let weight = ((share * 1000.0).round() as u16).max(40);
                let edge = chosen.then_some((t.accent, 2.0));
                // Words only where there is room for them (seen on kepler: EFI, boot and swap
                // side by side wrote over each other); the list under the bar names every one.
                let words: Vec<Element<Msg>> = if share >= 0.12 {
                    vec![label(title.clone()).size(11.0).weight(600), label(size_text(size)).size(10.0).dim()]
                } else {
                    Vec::new()
                };
                let mut b = button(column(words).spacing(2.0).width(Length::FILL))
                    .name(title)
                    .look(ButtonLook::Custom { fill: Some(fill), edge, radius: 6.0 })
                    .padding(Padding::hv(6.0, 8.0))
                    .width(Length::Fill(weight))
                    .height(Length::Fixed(54.0));
                if let Some(m) = msg {
                    b = b.on_press(m);
                }
                b
            })
            .collect();
        row(cells).spacing(3.0).width(Length::FILL)
    }

    /// Every partition by name under the bar, each a row to choose it by: its tint, its label
    /// or name and type, its file system and size.
    fn part_list(&self, t: &Theme, d: &Disk) -> Element<Msg> {
        let mut order: Vec<usize> = (0..d.parts.len()).collect();
        order.sort_by_key(|&i| d.parts[i].start);
        let rows: Vec<Element<Msg>> = order
            .into_iter()
            .map(|i| {
                let p = &d.parts[i];
                let chosen = self.part.as_deref() == Some(p.name.as_str());
                let name = if p.label.is_empty() { p.name.clone() } else { format!("{} ({})", p.label, p.name) };
                let right = [p.fs.as_str(), &size_text(p.size)]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                button(
                    row(vec![
                        panel(space())
                            .color(fs_color(&p.fs, t))
                            .radius(3.0)
                            .width(Length::Fixed(12.0))
                            .height(Length::Fixed(12.0)),
                        column(vec![label(name.clone()), label(p.ptype.clone()).size(11.0).dim()])
                            .spacing(1.0)
                            .width(Length::FILL),
                        label(right).dim().align(Align::End),
                    ])
                    .spacing(10.0)
                    .align(Align::Center)
                    .width(Length::FILL),
                )
                .look(if chosen { ButtonLook::Chosen } else { ButtonLook::Plain })
                .padding(Padding::hv(7.0, 12.0))
                .on_press(Msg::Part(i))
                .width(Length::FILL)
                .name(name)
            })
            .collect();
        self.group(t, Some("Partitions"), rows)
    }

    fn part_rows(p: &Part, external: bool) -> Vec<Element<Msg>> {
        let mut rows = Vec::new();
        if let Some((total, free)) = p.usage {
            let used = total.saturating_sub(free);
            rows.push(Disks::wide(
                column(vec![
                    label(format!("{} used of {}, {} free", size_text(used), size_text(total), size_text(free))),
                    meter(used as f64 / total.max(1) as f64, 10.0).width(Length::FILL),
                ])
                .spacing(6.0)
                .width(Length::FILL),
            ));
        }
        let mut add = |k: &str, v: &str| {
            if !v.is_empty() {
                rows.push(Disks::line(k, v.to_string()));
            }
        };
        add("File system", &p.fs);
        add("Label", &p.label);
        add("Type", &p.ptype);
        add("Size", &size_text(p.size));
        add("Mounted at", p.mount.as_deref().unwrap_or(""));
        add("In use", &p.role);
        add("Device", &format!("/dev/{}", p.name));
        add("UUID", &p.uuid);
        if p.mount.is_none() && p.role.is_empty() && !p.fs.is_empty() {
            add("Mounted at", "Not mounted");
        }
        if let Some(m) = &p.mount {
            let mut buttons = vec![
                button(label("Open"))
                    .look(ButtonLook::Normal)
                    .padding(Padding::hv(5.0, 14.0))
                    .on_press(Msg::Open(m.clone()))
                    .name("Open"),
            ];
            if external {
                buttons.push(
                    button(label("Eject"))
                        .look(ButtonLook::Normal)
                        .padding(Padding::hv(5.0, 14.0))
                        .on_press(Msg::Eject(m.clone()))
                        .name("Eject"),
                );
            }
            rows.push(Disks::wide(row(buttons).spacing(8.0)));
        }
        rows
    }

    fn disk_page(&self, t: &Theme, d: &Disk) -> Vec<Element<Msg>> {
        let mut v = Vec::new();
        let what = match d.rotational {
            Some(true) => "hard disk",
            Some(false) if d.bus != "USB" => "solid-state disk",
            _ if d.kind == Some(Kind::Image) => "disk image",
            _ => "disk",
        };
        v.push(label(format!("{} {} {}, {}", size_text(d.size), d.bus, what, d.name).replace("  ", " ")).dim());
        if !d.parts.is_empty() {
            v.push(self.bar(t, d));
            v.push(self.part_list(t, d));
            let chosen = self.part.as_ref().and_then(|n| d.parts.iter().find(|p| &p.name == n));
            match chosen {
                Some(p) => v.push(self.group(
                    t,
                    Some(&format!("Partition {}", p.index)),
                    Disks::part_rows(p, d.kind == Some(Kind::External)),
                )),
                None => v.push(label("Choose a partition to see its details.").dim()),
            }
        } else if let Some(w) = &d.whole {
            v.push(self.group(t, Some("The whole disk"), Disks::part_rows(w, d.kind == Some(Kind::External))));
        } else {
            v.push(
                label(
                    "No partitions and no file system: this disk is blank, or holds something this system cannot read.",
                )
                .dim()
                .wrap(),
            );
        }
        let mut rows = Vec::new();
        let mut add = |k: &str, v: String| {
            if !v.is_empty() {
                rows.push(Disks::line(k, v));
            }
        };
        add("Model", d.model.clone());
        add("Serial number", d.serial.clone());
        add("Firmware", d.firmware.clone());
        add("Connection", d.bus.clone());
        add(
            "Kind",
            match d.rotational {
                Some(true) => "Spinning disk".into(),
                Some(false) => "Solid state".into(),
                None => String::new(),
            },
        );
        add(
            "Partition table",
            match d.scheme.as_str() {
                "GPT" => "GUID (GPT)".into(),
                "MBR" => "Master boot record (MBR)".into(),
                "" if d.whole.is_some() => "None (one file system on the whole disk)".into(),
                s => s.to_string(),
            },
        );
        add("Capacity", format!("{} ({} bytes)", size_text(d.size), group_digits(d.size)));
        add("Temperature", d.temp.map(|c| format!("{c:.0} °C")).unwrap_or_default());
        add("Device", format!("/dev/{}", d.name));
        v.push(self.group(t, Some("This disk"), rows));
        v
    }

    fn pool_page(&self, t: &Theme, p: &Pool) -> Vec<Element<Msg>> {
        let mut v = vec![label(format!("ZFS pool, {}", health_word(&p.health))).dim()];
        let used = p.alloc;
        v.push(self.group(
            t,
            None,
            vec![
                Disks::wide(
                    column(vec![
                        label(format!("{} used of {}, {} free", size_text(used), size_text(p.size), size_text(p.free))),
                        meter(used as f64 / p.size.max(1) as f64, 10.0).width(Length::FILL),
                    ])
                    .spacing(6.0)
                    .width(Length::FILL),
                ),
                Disks::line("Health", health_word(&p.health)),
                Disks::line("Errors", if p.errors.is_empty() { "—".into() } else { p.errors.clone() }),
                Disks::line("Last check", if p.scan.is_empty() { "Never scrubbed".into() } else { p.scan.clone() }),
            ],
        ));
        let members: Vec<Element<Msg>> = p.members.iter().map(|(m, s)| Disks::line(m, health_word(s))).collect();
        if !members.is_empty() {
            v.push(self.group(t, Some("Disks in the pool"), members));
        }
        v
    }
}

/// A file system's tint for the bar: light enough under the page's text in either scheme.
fn fs_color(fs: &str, t: &Theme) -> Color {
    let a = if t.dark { 90 } else { 70 };
    let (r, g, b) = match fs {
        "Btrfs" | "ext4" | "ext3" | "ext2" | "XFS" | "ext2/3/4" | "F2FS" => (66, 133, 244),
        "ZFS" | "UFS" => (52, 168, 83),
        "FAT32" | "FAT16" | "FAT12" | "exFAT" => (240, 150, 40),
        "NTFS" => (0, 150, 200),
        "Swap" => (150, 150, 150),
        s if s.starts_with("Encrypted") => (150, 90, 200),
        _ => (t.dim.r, t.dim.g, t.dim.b),
    };
    Color::rgba(r, g, b, a)
}

fn health_word(h: &str) -> String {
    match h {
        "ONLINE" => "Healthy".into(),
        "DEGRADED" => "Degraded: a disk is missing or failing".into(),
        "FAULTED" => "Faulted".into(),
        "OFFLINE" => "Offline".into(),
        "UNAVAIL" => "Not available".into(),
        "REMOVED" => "Removed".into(),
        "SUSPENDED" => "Suspended: I/O failures".into(),
        o => o.to_string(),
    }
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Program for Disks {
    type Message = Msg;

    fn title(&self) -> String {
        "Disks".into()
    }

    fn app_id(&self) -> String {
        APP_ID.into()
    }

    fn theme(&self) -> Theme {
        Theme::from_desktop(&self.desktop, false, "[disks]")
    }

    fn bar_style(&self) -> Option<BarStyle> {
        Some(vergence_ui::bar_style_setting(&self.desktop, "[disks]", BarStyle::Light))
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let cur = self.current();
        let mut side: Vec<Element<Msg>> = Vec::new();
        let groups = [(Kind::Internal, "Internal"), (Kind::External, "External"), (Kind::Image, "Disk images")];
        let entry = |title: String, sub: String, sel: Sel| -> Element<Msg> {
            let here = cur == Some(sel);
            button(
                column(vec![label(title.clone()).width(Length::FILL), label(sub).size(11.0).dim()])
                    .spacing(1.0)
                    .width(Length::FILL),
            )
            .look(if here { ButtonLook::Accent } else { ButtonLook::Plain })
            .padding(Padding::hv(6.0, 10.0))
            .on_press(Msg::Pick(sel))
            .width(Length::FILL)
            .name(title)
        };
        for (kind, heading) in groups {
            let items: Vec<(usize, &Disk)> =
                self.disks.iter().enumerate().filter(|(_, d)| d.kind == Some(kind)).collect();
            if items.is_empty() {
                continue;
            }
            side.push(panel(label(heading).size(11.0).dim().weight(600)).padding(Padding::hv(8.0, 10.0)));
            for (i, d) in items {
                let title = if d.model.is_empty() { d.name.clone() } else { d.model.clone() };
                side.push(entry(title, format!("{} {}", size_text(d.size), d.bus).trim().to_string(), Sel::Disk(i)));
            }
        }
        if !self.pools.is_empty() {
            side.push(panel(label("Pools").size(11.0).dim().weight(600)).padding(Padding::hv(8.0, 10.0)));
            for (i, p) in self.pools.iter().enumerate() {
                side.push(entry(p.name.clone(), format!("{} ZFS", size_text(p.size)), Sel::Pool(i)));
            }
        }
        let sidebar = panel(scroll(column(side).spacing(2.0).width(Length::FILL)))
            .color(t.sidebar)
            .padding(Padding { top: 6.0 + t.top, bottom: 12.0, left: 10.0, right: 10.0 })
            .radius(0.0)
            .width(Length::Fixed(SIDEBAR))
            .height(Length::FILL);
        let (title, body) = match cur {
            Some(Sel::Disk(i)) => {
                let d = &self.disks[i];
                (if d.model.is_empty() { d.name.clone() } else { d.model.clone() }, self.disk_page(t, d))
            }
            Some(Sel::Pool(i)) => (self.pools[i].name.clone(), self.pool_page(t, &self.pools[i])),
            None => ("Disks".into(), vec![label("No disks found.").dim()]),
        };
        let mut all = vec![label(title.clone()).size(18.0).weight(700)];
        if !self.note.is_empty() {
            all.push(label(self.note.clone()).dim().wrap());
        }
        all.extend(body);
        let page = scroll(
            panel(column(all).spacing(16.0).width(Length::FILL))
                .padding(Padding { top: 18.0 + t.top, bottom: 22.0, left: 22.0, right: 22.0 })
                .width(Length::FILL),
        )
        .name(title);
        row(vec![
            sidebar,
            vrule().color(t.line),
            page.width(Length::FILL).height(Length::FILL),
            space().width(Length::Fixed(0.0)),
        ])
        .width(Length::FILL)
        .height(Length::FILL)
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        match m {
            Msg::Pick(s) => {
                self.sel = Some(match s {
                    Sel::Disk(i) => (false, self.disks[i].name.clone()),
                    Sel::Pool(i) => (true, self.pools[i].name.clone()),
                });
                self.part = None;
                self.note.clear();
            }
            Msg::Part(i) => {
                if let Some(Sel::Disk(d)) = self.current() {
                    self.part = self.disks[d].parts.get(i).map(|p| p.name.clone());
                }
            }
            Msg::Open(path) => {
                if let Err(e) = std::process::Command::new("vergence-files").arg(&path).spawn() {
                    self.note = format!("Could not open FileView: {e}");
                }
            }
            Msg::Eject(path) => {
                self.note = match vergence_widgets::files::eject(Path::new(&path)) {
                    Ok(_) => String::new(),
                    Err(e) => format!("Could not eject: {e}"),
                };
                self.read();
            }
        }
        window.redraw();
    }

    /// A reading every five seconds, from a thread that only wakes the window.
    fn waker(&mut self, waker: Waker) {
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(5));
                waker.wake();
            }
        });
    }

    fn woken(&mut self, _window: &mut Window) -> bool {
        let before = (self.disks.clone(), self.pools.clone());
        self.read();
        before != (self.disks.clone(), self.pools.clone())
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        Some((false, vec![C::role(1, Role::About)]))
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        if id == 1 {
            let about = vergence_widgets::about::About::new(
                APP_ID,
                "Disks",
                env!("CARGO_PKG_VERSION"),
                "The disks in this computer and plugged into it: their partitions, file systems, how full they are, and ZFS pools.",
                self.theme(),
            );
            window.open_window(vergence_widgets::window(about));
        }
    }

    fn under_bar(&self) -> bool {
        true
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { kind: vergence_ui::WinKind::Sidebar, min: (680.0, 440.0), ..Default::default() }
    }

    fn bar(&self) -> Option<vergence_ui::Bar> {
        Some(vergence_ui::Bar::Tools { buttons: Vec::new(), from: SIDEBAR as f64 })
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("vergence-disks {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    // `--list`: what the window would show, as text (for checking a machine over ssh).
    if args.iter().any(|a| a == "--list") {
        for d in vergence_disks::disks() {
            println!(
                "{} {:?} {} {} {} scheme={} temp={:?} serial={} fw={}",
                d.name,
                d.kind,
                d.bus,
                d.model,
                size_text(d.size),
                d.scheme,
                d.temp,
                d.serial,
                d.firmware
            );
            for p in d.parts.iter().chain(d.whole.iter()) {
                println!(
                    "  {} #{} start={} {} fs={} label={:?} type={} mount={:?} usage={:?} role={:?} aliases={:?}",
                    p.name,
                    p.index,
                    p.start,
                    size_text(p.size),
                    p.fs,
                    p.label,
                    p.ptype,
                    p.mount,
                    p.usage,
                    p.role,
                    p.aliases
                );
            }
        }
        for p in vergence_disks::pools() {
            println!(
                "pool {} {} {} alloc={} health={} members={:?} scan={:?} errors={:?}",
                p.name,
                size_text(p.size),
                p.size,
                p.alloc,
                p.health,
                p.members,
                p.scan,
                p.errors
            );
        }
        return;
    }
    if let Err(e) = vergence_widgets::run(Disks::new(), (900, 640)) {
        eprintln!("vergence-disks: {e}");
        std::process::exit(1);
    }
}
