//! Vergence's Hardware app: a page for each part of the machine, what it is and what it is doing
//! now.
//!
//! Tools milestone 1, docs/tools-milestone-1.md; details on the CPU, GPU, network, memory and
//! the rest, as listed in docs/apps-roadmap.md. Each page charts the last minute. Read-only; the
//! readings are vergence-sysinfo's (processors, memory, file systems, disks, temperatures),
//! vergence-net's (interfaces and their traffic) and vergence-power's (batteries, mains, the GPUs
//! and whether they are awake), once a second. The window is on vergence-widgets and
//! vergence-ui; the charts are vergence-widgets' chart.rs.
//!
//! Start reading: this one file, from `main` to the program's state and its `view`, which
//! builds a page from the latest readings.
//!
//! Rule: read-only; the app reads what the system offers to any user and changes nothing.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use vergence_sysinfo::{DiskIo, FileSystem, Memory, Temp, Ticks, bytes_text};
use vergence_ui::{AppCommand as C, BarStyle, Role};
use vergence_widgets::chart::{chart, meter};
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Program, Theme, Waker, Window, button, column, label, panel, row,
    rule, scroll, space, vrule,
};

const APP_ID: &str = "vergence-hw";
/// Samples kept for the charts: a minute.
const SLOTS: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Computer,
    Processors,
    Memory,
    Storage,
    Network,
    Graphics,
    Power,
}

const PAGES: [(Page, &str); 7] = [
    (Page::Computer, "This computer"),
    (Page::Processors, "Processors"),
    (Page::Memory, "Memory"),
    (Page::Storage, "Storage"),
    (Page::Network, "Network"),
    (Page::Graphics, "Graphics"),
    (Page::Power, "Power"),
];

#[derive(Debug, Clone)]
enum Msg {
    Go(Page),
}

fn push(h: &mut VecDeque<f64>, v: f64) {
    h.push_back(v);
    while h.len() > SLOTS {
        h.pop_front();
    }
}

struct Hardware {
    page: Page,
    host: String,
    system: String,
    model: String,
    mhz: Option<u32>,
    ticks: Vec<Ticks>,
    loads: Vec<f32>,
    cpu_hist: VecDeque<f64>,
    temps: Vec<Temp>,
    mem: Memory,
    mem_hist: VecDeque<f64>,
    fs: Vec<FileSystem>,
    io: Vec<DiskIo>,
    io_hist: (VecDeque<f64>, VecDeque<f64>),
    /// Interfaces: name, kind word, addresses, speed; their rates' history.
    ifaces: Vec<vergence_net::ifaces::Iface>,
    net_prev: HashMap<String, (u64, u64)>,
    net_hist: HashMap<String, (VecDeque<f64>, VecDeque<f64>)>,
    gpus: Vec<vergence_power::Gpu>,
    power: vergence_power::Power,
    tick: u64,
    desktop: String,
}

impl Hardware {
    fn new() -> Hardware {
        let (host, system) = vergence_sysinfo::system();
        let desktop = vergence_config::effective_text_with(vergence_widgets::prefs::desktop_path().as_deref());
        let mut h = Hardware {
            page: Page::Computer,
            host,
            system,
            model: String::new(),
            mhz: None,
            ticks: Vec::new(),
            loads: Vec::new(),
            cpu_hist: VecDeque::new(),
            temps: Vec::new(),
            mem: Memory::default(),
            mem_hist: VecDeque::new(),
            fs: Vec::new(),
            io: Vec::new(),
            io_hist: (VecDeque::new(), VecDeque::new()),
            ifaces: Vec::new(),
            net_prev: HashMap::new(),
            net_hist: HashMap::new(),
            gpus: Vec::new(),
            power: Default::default(),
            tick: 0,
            desktop,
        };
        h.sample();
        h
    }

    /// One second's readings.
    fn sample(&mut self) {
        let cpu = vergence_sysinfo::cpu();
        if !self.ticks.is_empty() && self.ticks.len() == cpu.ticks.len() {
            self.loads = vergence_sysinfo::load(&self.ticks, &cpu.ticks);
            let avg = self.loads.iter().sum::<f32>() / self.loads.len().max(1) as f32;
            push(&mut self.cpu_hist, avg as f64 * 100.0);
        }
        (self.model, self.mhz, self.ticks) = (cpu.model, cpu.mhz, cpu.ticks);
        self.temps = vergence_sysinfo::temps();
        self.mem = vergence_sysinfo::memory();
        if self.mem.total > 0 {
            push(&mut self.mem_hist, self.mem.used() as f64 / self.mem.total as f64 * 100.0);
        }
        let io = vergence_sysinfo::disk_io();
        if !self.io.is_empty() {
            let sum = |v: &[DiskIo]| v.iter().fold((0u64, 0u64), |a, d| (a.0 + d.read, a.1 + d.written));
            let (a, b) = (sum(&self.io), sum(&io));
            push(&mut self.io_hist.0, b.0.saturating_sub(a.0) as f64);
            push(&mut self.io_hist.1, b.1.saturating_sub(a.1) as f64);
        }
        self.io = io;
        if let Ok(list) = vergence_net::ifaces::list() {
            for f in &list {
                let (rx, tx) = (f.rx_bytes.unwrap_or(0), f.tx_bytes.unwrap_or(0));
                if let Some((prx, ptx)) = self.net_prev.insert(f.name.clone(), (rx, tx)) {
                    let h = self.net_hist.entry(f.name.clone()).or_default();
                    push(&mut h.0, vergence_net::ifaces::delta(prx, rx) as f64 * 8.0);
                    push(&mut h.1, vergence_net::ifaces::delta(ptx, tx) as f64 * 8.0);
                }
            }
            self.ifaces = list
                .into_iter()
                .filter(|f| {
                    f.kind != vergence_net::ifaces::Kind::Loopback && !vergence_net::ifaces::is_virtual(&f.name)
                })
                .collect();
        }
        // The slower ones every five seconds.
        if self.tick % 5 == 0 {
            self.fs = vergence_sysinfo::file_systems();
            self.gpus = vergence_power::gpus();
            self.power = vergence_power::power();
        }
        self.tick += 1;
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

    fn line(name: &str, value: String) -> Element<Msg> {
        panel(
            row(vec![label(name).width(Length::FILL), label(value).dim().align(Align::End)])
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

    fn page_view(&self, t: &Theme) -> Vec<Element<Msg>> {
        let cpu_now = self.cpu_hist.back().copied().unwrap_or(0.0);
        match self.page {
            Page::Computer => {
                vec![self.group(
                    t,
                    None,
                    vec![
                            Hardware::line("Name", self.host.clone()),
                            Hardware::line("System", self.system.clone()),
                            Hardware::line(
                                "Processor",
                                if self.model.is_empty() { "—".into() } else { self.model.clone() },
                            ),
                            Hardware::line("Threads", self.ticks.len().to_string()),
                            Hardware::line("Memory", bytes_text(self.mem.total)),
                            Hardware::line(
                                "Graphics",
                                self.gpus
                                    .iter()
                                    .map(|g| format!("{} ({})", g.vendor, g.driver))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            ),
                        ],
                )]
            }
            Page::Processors => {
                let mut cores: Vec<Element<Msg>> = Vec::new();
                for (i, l) in self.loads.iter().enumerate() {
                    cores.push(
                        row(vec![
                            label(format!("{}", i + 1)).size(11.0).dim().width(Length::Fixed(24.0)),
                            meter(*l as f64, 8.0).width(Length::FILL),
                            label(format!("{:.0}%", l * 100.0))
                                .size(11.0)
                                .dim()
                                .align(Align::End)
                                .width(Length::Fixed(40.0)),
                        ])
                        .spacing(8.0)
                        .align(Align::Center)
                        .width(Length::FILL),
                    );
                }
                let mut v = vec![self.group(
                    t,
                    None,
                    vec![
                            Hardware::line("Processor", self.model.clone()),
                            Hardware::line("Busy now", format!("{cpu_now:.0}%")),
                            Hardware::line(
                                "Clock",
                                self.mhz.map_or("—".into(), |m| format!("{:.2} GHz", m as f64 / 1000.0)),
                            ),
                            Hardware::wide(
                                column(vec![
                                    label("The last minute").size(11.0).dim(),
                                    chart(vec![self.cpu_hist.iter().copied().collect()], Some(100.0), SLOTS, 70.0),
                                ])
                                .spacing(4.0)
                                .width(Length::FILL),
                            ),
                        ],
                )];
                v.push(self.group(
                    t,
                    Some("Each thread"),
                    vec![Hardware::wide(column(cores).spacing(5.0).width(Length::FILL))],
                ));
                if !self.temps.is_empty() {
                    v.push(self.group(
                        t,
                        Some("Temperatures"),
                        self.temps.iter().map(|tp| Hardware::line(&tp.what, format!("{:.0} °C", tp.celsius))).collect(),
                    ));
                }
                v
            }
            Page::Memory => {
                let m = &self.mem;
                let mut rows = vec![
                    Hardware::wide(
                        column(vec![
                            row(vec![
                                label(format!("{} used of {}", bytes_text(m.used()), bytes_text(m.total)))
                                    .width(Length::FILL),
                            ])
                            .width(Length::FILL),
                            meter(if m.total > 0 { m.used() as f64 / m.total as f64 } else { 0.0 }, 10.0)
                                .width(Length::FILL),
                        ])
                        .spacing(6.0)
                        .width(Length::FILL),
                    ),
                    Hardware::line("Free for programs", bytes_text(m.available)),
                    Hardware::line("Cache", bytes_text(m.cached)),
                ];
                if m.swap_total > 0 {
                    rows.push(Hardware::line(
                        "Swap used",
                        format!("{} of {}", bytes_text(m.swap_total - m.swap_free), bytes_text(m.swap_total)),
                    ));
                }
                rows.push(Hardware::wide(
                    column(vec![
                        label("Used, the last minute").size(11.0).dim(),
                        chart(vec![self.mem_hist.iter().copied().collect()], Some(100.0), SLOTS, 70.0),
                    ])
                    .spacing(4.0)
                    .width(Length::FILL),
                ));
                vec![self.group(t, None, rows)]
            }
            Page::Storage => {
                let mut v: Vec<Element<Msg>> = self
                    .fs
                    .iter()
                    .map(|f| {
                        let used = f.total.saturating_sub(f.free);
                        self.group(
                            t,
                            Some(&f.mount),
                            vec![Hardware::wide(
                                column(vec![
                                    row(vec![
                                        label(format!("{} free of {}", bytes_text(f.free), bytes_text(f.total)))
                                            .width(Length::FILL),
                                        label(f.source.clone()).size(11.0).dim(),
                                    ])
                                    .spacing(8.0)
                                    .width(Length::FILL),
                                    meter(used as f64 / f.total.max(1) as f64, 10.0).width(Length::FILL),
                                ])
                                .spacing(6.0)
                                .width(Length::FILL),
                            )],
                        )
                    })
                    .collect();
                if !self.io.is_empty() {
                    let (r, w) =
                        (self.io_hist.0.back().copied().unwrap_or(0.0), self.io_hist.1.back().copied().unwrap_or(0.0));
                    v.push(self.group(
                        t,
                        Some("Disks, all together"),
                        vec![
                            Hardware::line("Reading", format!("{}/s", bytes_text(r as u64))),
                            Hardware::line("Writing", format!("{}/s", bytes_text(w as u64))),
                            Hardware::wide(chart(
                                vec![
                                    self.io_hist.0.iter().copied().collect(),
                                    self.io_hist.1.iter().copied().collect(),
                                ],
                                None,
                                SLOTS,
                                60.0,
                            )),
                        ],
                    ));
                }
                v
            }
            Page::Network => self
                .ifaces
                .iter()
                .map(|f| {
                    let h = self.net_hist.get(&f.name);
                    let rate = |v: Option<&VecDeque<f64>>| rate_text(v.and_then(|q| q.back().copied()).unwrap_or(0.0));
                    let mut rows =
                        vec![Hardware::line("Connected", if f.carrier { "Yes".into() } else { "No".into() })];
                    for (a, p) in &f.addrs {
                        rows.push(Hardware::line(
                            if a.is_ipv4() { "Address" } else { "IPv6 address" },
                            format!("{a}/{p}"),
                        ));
                    }
                    if let Some(s) = f.speed_mbps {
                        rows.push(Hardware::line(
                            "Link",
                            if s >= 1000 { format!("{} Gb/s", s as f64 / 1000.0) } else { format!("{s} Mb/s") },
                        ));
                    }
                    rows.push(Hardware::line("In", rate(h.map(|h| &h.0))));
                    rows.push(Hardware::line("Out", rate(h.map(|h| &h.1))));
                    if let Some(h) = h {
                        rows.push(Hardware::wide(chart(
                            vec![h.0.iter().copied().collect(), h.1.iter().copied().collect()],
                            None,
                            SLOTS,
                            50.0,
                        )));
                    }
                    self.group(t, Some(&format!("{} ({})", f.name, kind_word(f))), rows)
                })
                .collect(),
            Page::Graphics => {
                let mut v: Vec<Element<Msg>> = self
                    .gpus
                    .iter()
                    .map(|g| {
                        self.group(
                            t,
                            Some(&g.vendor),
                            vec![
                                Hardware::line("Driver", g.driver.clone()),
                                Hardware::line("Slot", g.slot.clone()),
                                Hardware::line(
                                    "Awake",
                                    match g.awake {
                                        Some(true) => "Yes".into(),
                                        Some(false) => "Asleep".into(),
                                        None => "—".into(),
                                    },
                                ),
                            ],
                        )
                    })
                    .collect();
                let gpu_temps: Vec<Element<Msg>> = self
                    .temps
                    .iter()
                    .filter(|tp| {
                        ["nouveau", "amdgpu", "radeon", "i915", "nvidia"].iter().any(|n| tp.what.starts_with(n))
                    })
                    .map(|tp| Hardware::line(&tp.what, format!("{:.0} °C", tp.celsius)))
                    .collect();
                if !gpu_temps.is_empty() {
                    v.push(self.group(t, Some("Temperatures"), gpu_temps));
                }
                v
            }
            Page::Power => {
                let mut v = vec![self.group(
                    t,
                    None,
                    vec![Hardware::line(
                        "Mains power",
                        match self.power.on_mains {
                            Some(true) => "Connected".into(),
                            Some(false) => "Not connected".into(),
                            None => "—".into(),
                        },
                    )],
                )];
                for b in &self.power.batteries {
                    let mut rows = vec![Hardware::wide(
                        column(vec![
                            label(format!("{}%  {:?}", b.percent.unwrap_or(0), b.state)),
                            meter(b.percent.unwrap_or(0) as f64 / 100.0, 10.0).width(Length::FILL),
                        ])
                        .spacing(6.0)
                        .width(Length::FILL),
                    )];
                    if let Some(w) = b.watts {
                        rows.push(Hardware::line("Power", format!("{w:.1} W")));
                    }
                    if let Some(m) = b.minutes {
                        rows.push(Hardware::line("Time left", vergence_power::duration_text(m)));
                    }
                    if let Some(h) = b.health() {
                        rows.push(Hardware::line("Health", format!("{h}% of new")));
                    }
                    if let Some(c) = b.cycles {
                        rows.push(Hardware::line("Cycles", c.to_string()));
                    }
                    if !b.model.is_empty() {
                        rows.push(Hardware::line("Model", b.model.clone()));
                    }
                    v.push(self.group(t, Some(&b.name), rows));
                }
                v
            }
        }
    }
}

fn kind_word(f: &vergence_net::ifaces::Iface) -> &'static str {
    match f.kind {
        vergence_net::ifaces::Kind::WiFi => "Wi-Fi",
        vergence_net::ifaces::Kind::Wired => "cable",
        vergence_net::ifaces::Kind::Phone => "phone",
        vergence_net::ifaces::Kind::Loopback => "this computer",
    }
}

/// Bits a second in words.
fn rate_text(bits: f64) -> String {
    if bits >= 1e9 {
        format!("{:.1} Gb/s", bits / 1e9)
    } else if bits >= 1e6 {
        format!("{:.1} Mb/s", bits / 1e6)
    } else if bits >= 1e3 {
        format!("{:.0} kb/s", bits / 1e3)
    } else {
        format!("{bits:.0} b/s")
    }
}

impl Program for Hardware {
    type Message = Msg;

    fn title(&self) -> String {
        "Hardware".into()
    }

    fn app_id(&self) -> String {
        APP_ID.into()
    }

    fn theme(&self) -> Theme {
        Theme::from_desktop(&self.desktop, false, "[hardware]")
    }

    fn bar_style(&self) -> Option<BarStyle> {
        Some(vergence_ui::bar_style_setting(&self.desktop, "[hardware]", BarStyle::Light))
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let mut side: Vec<Element<Msg>> = Vec::new();
        for (p, name) in PAGES {
            if p == Page::Power && self.power.batteries.is_empty() && self.power.on_mains.is_none() {
                continue;
            }
            let here = p == self.page;
            side.push(
                button(label(name).width(Length::FILL))
                    .look(if here { ButtonLook::Accent } else { ButtonLook::Plain })
                    .padding(Padding::hv(6.0, 10.0))
                    .on_press(Msg::Go(p))
                    .width(Length::FILL)
                    .name(name),
            );
        }
        let sidebar = panel(column(side).spacing(2.0).width(Length::FILL))
            .color(t.sidebar)
            .padding(Padding { top: 12.0 + t.top, bottom: 12.0, left: 10.0, right: 10.0 })
            .radius(0.0)
            .width(Length::Fixed(190.0))
            .height(Length::FILL);
        let title = PAGES.iter().find(|(p, _)| *p == self.page).map_or("", |(_, n)| n);
        let mut all = vec![label(title).size(18.0).weight(700)];
        all.extend(self.page_view(t));
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
        .into()
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        match m {
            Msg::Go(p) => self.page = p,
        }
        window.redraw();
    }

    /// A reading once a second, from a thread that only wakes the window.
    fn waker(&mut self, waker: Waker) {
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                waker.wake();
            }
        });
    }

    fn woken(&mut self, _window: &mut Window) -> bool {
        self.sample();
        true
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        Some((false, vec![C::role(1, Role::About)]))
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        if id == 1 {
            let about = vergence_widgets::about::About::new(
                APP_ID,
                "Hardware",
                env!("CARGO_PKG_VERSION"),
                "What this computer is and what it is doing now: processors, memory, storage, network, graphics and power.",
                self.theme(),
            );
            window.open_window(vergence_widgets::window(about));
        }
    }

    fn under_bar(&self) -> bool {
        true
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { kind: vergence_ui::WinKind::Sidebar, min: (640.0, 420.0), ..Default::default() }
    }

    fn bar(&self) -> Option<vergence_ui::Bar> {
        Some(vergence_ui::Bar::Tools { buttons: Vec::new(), from: 190.0 })
    }
}

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("vergence-hw {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if let Err(e) = vergence_widgets::run(Hardware::new(), (860, 640)) {
        eprintln!("vergence-hw: {e}");
        std::process::exit(1);
    }
}
