//! The About page and its sub-pages (Page::About, LicenseText, Notices):
//! the version, copyright, the license texts and the notices of the code
//! Vergence is built with.
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

// About: copyright, license and notices, compiled in so they always match this
// build (also installed under share/doc/vergence by tools/install.sh).
const COPYRIGHT: &str = "\u{a9} 2026 David Erickson";
const LICENSE_APACHE: &str = include_str!("../../../LICENSE-APACHE");
const LICENSE_GPL: &str = include_str!("../../../LICENSE-GPL");
const NOTICES: &str = include_str!("../../../THIRD-PARTY-NOTICES.md");

/// A license text in paragraphs: blank lines separate them, and the line breaks
/// inside one become spaces so the label can wrap it.
fn paragraphs(text: &str) -> Vec<String> {
    text.split("\n\n").map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty()).collect()
}

/// (crate, version, license) from the notices file's summary table.
fn notices_summary() -> Vec<(String, String, String)> {
    let Some(start) = NOTICES.find("## Summary") else { return Vec::new() };
    NOTICES[start..]
        .lines()
        .skip_while(|l| !l.starts_with("|---"))
        .skip(1)
        .take_while(|l| l.starts_with('|'))
        .filter_map(|l| {
            let cells: Vec<&str> = l.trim_matches('|').split('|').map(str::trim).collect();
            (cells.len() == 3).then(|| (cells[0].to_string(), cells[1].to_string(), cells[2].to_string()))
        })
        .collect()
}

/// Where tools/install.sh put the license files: `<prefix>/share/doc/vergence`
/// beside `<prefix>/bin`. None when running from a checkout.
fn installed_doc_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?.join("share/doc/vergence");
    dir.join("THIRD-PARTY-NOTICES.md").is_file().then_some(dir)
}

/// What does not change while Settings runs, read once (the About page is
/// drawn often; mind the battery): the system, the machine, the processor,
/// graphics and memory.
struct Facts {
    system: String,
    machine: String,
    firmware: String,
    processor: String,
    cores: String,
    graphics: String,
    memory: String,
}

fn facts() -> &'static Facts {
    static FACTS: std::sync::OnceLock<Facts> = std::sync::OnceLock::new();
    FACTS.get_or_init(|| {
        let (_, uname) = vergence_sysinfo::system();
        let os = vergence_sysinfo::os_name();
        // Linux: "Void Linux, kernel 6.18.65_1"; FreeBSD names its own release.
        let system = match uname.strip_prefix("Linux ") {
            Some(kernel) if !os.is_empty() => format!("{os}, kernel {kernel}"),
            _ => uname,
        };
        let m = vergence_sysinfo::machine();
        let cpu = vergence_sysinfo::cpu();
        let topo = vergence_sysinfo::topology();
        let cores = if topo.threads() > 0 {
            vergence_sysinfo::cores_text(&topo, &cpu.model)
        } else {
            format!("{} Threads", cpu.ticks.len())
        };
        let graphics = vergence_power::gpus()
            .iter()
            .map(|g| format!("{} ({})", g.vendor, g.driver))
            .collect::<Vec<_>>()
            .join(", ");
        Facts {
            system,
            machine: m.name(),
            firmware: m.bios,
            processor: cpu.model,
            cores,
            graphics,
            memory: installed_memory(vergence_sysinfo::memory().total),
        }
    })
}

/// The memory as sold ("64 GB"): the system reports a little less than is
/// fitted (the firmware and the graphics keep some), so the smallest usual
/// size at or above it, in GiB.
fn installed_memory(bytes: u64) -> String {
    if bytes == 0 {
        return String::new();
    }
    let gib = bytes as f64 / (1u64 << 30) as f64;
    const SIZES: [u32; 18] = [1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512];
    match SIZES.iter().find(|&&s| s as f64 >= gib) {
        Some(s) => format!("{s} GB"),
        None => format!("{:.0} GB", gib.ceil()),
    }
}

/// A screen in words: "LG ULTRAFINE, 3840 × 2160 at 60 Hz, 133%".
fn screen_text(o: &OutputInfo) -> String {
    let mut t = screen_label(o);
    if let Some(m) = o.modes.get(o.mode as usize) {
        t += &format!(", {} \u{d7} {}", m.width, m.height);
        if m.refresh_mhz > 0 {
            t += &format!(" at {:.0} Hz", m.refresh_mhz as f64 / 1000.0);
        }
    }
    if o.scale > 0.0 {
        t += &format!(", {:.0}%", o.scale * 100.0);
    }
    t
}

/// The Vergence mark (the logo Dave chose 2026-11-27; masters on techbench in
/// ~/Documents/Projects/Vergence/Logo): a V of one stroke with round ends and
/// a round point, and a focal dot below it, on a 64 grid, `size` across. Its
/// colors follow light or dark as the masters' two versions do.
fn logo_mark<M: 'static>(size: f32) -> Element<M> {
    drawing(move |scene: &mut Scene, _: &mut Fonts, r: Rect, theme: &Theme| {
        let k = size / 64.0;
        let (x, y) = (r.x + (r.w - size) / 2.0, r.y + (r.h - size) / 2.0);
        let p = |gx: f32, gy: f32| Point::new(x + gx * k, y + gy * k);
        let (v, dot) = if theme.dark {
            (Color::rgb(0xc9, 0xc4, 0xff), Color::rgb(0xff, 0xff, 0xff))
        } else {
            (Color::rgb(0x2d, 0x2f, 0x5c), Color::rgb(0x5b, 0x56, 0xb0))
        };
        // Lines have round ends, so the two meeting at the point make the
        // master's round join.
        scene.line(p(14.0, 10.5), p(32.0, 41.5), 7.5 * k, v);
        scene.line(p(32.0, 41.5), p(50.0, 10.5), 7.5 * k, v);
        let d = 4.8 * k;
        scene.fill_rounded(Rect::new(x + 32.0 * k - d, y + 52.5 * k - d, 2.0 * d, 2.0 * d), d, dot);
    })
    .width(Length::Fixed(size))
    .height(Length::Fixed(size))
}

impl App {
    /// The lockup at the top of About: the mark and "vergence" in Inter 600,
    /// in the masters' proportions (mark 64, text 44 starting at 72).
    fn lockup(&self) -> Element<Message> {
        let ink = if self.theme().dark { Color::rgb(0xe9, 0xe7, 0xff) } else { Color::rgb(0x1a, 0x1d, 0x3a) };
        row(vec![logo_mark(48.0), label("vergence").size(33.0).weight(600).color(ink)])
            .spacing(6.0)
            .align(Align::Center)
    }

    /// The summary's lines, name and value, as shown and as copied.
    fn summary(&self) -> Vec<(&'static str, String)> {
        let f = facts();
        let session = match (std::env::var("VERGENCE_BUS"), std::env::var("WAYLAND_DISPLAY")) {
            (Ok(_), Ok(w)) => format!("Vergence on {w}"),
            (_, Ok(w)) => format!("another Wayland desktop ({w})"),
            _ => "an X11 desktop".to_string(),
        };
        let mut v = vec![
            // The Settings app's own version, as before: which number names
            // Vergence itself (workspace 0.4.0-pre.3 vs the package's pre16)
            // is control's to settle with the release (Dave, 2026-11-26).
            ("Settings app", format!("version {}", env!("CARGO_PKG_VERSION"))),
            ("Session", session),
        ];
        v.push(("System", f.system.clone()));
        let computer =
            if f.firmware.is_empty() { f.machine.clone() } else { format!("{}, firmware {}", f.machine, f.firmware) };
        for (name, value) in [
            ("Computer", computer),
            ("Processor", f.processor.clone()),
            ("Cores", f.cores.clone()),
            ("Graphics", f.graphics.clone()),
            ("Memory", f.memory.clone()),
        ] {
            if !value.trim().is_empty() {
                v.push((name, value));
            }
        }
        for o in &self.outputs {
            v.push(("Screen", screen_text(o)));
        }
        if let Some(s) = vergence_sysinfo::uptime() {
            v.push(("Up for", vergence_sysinfo::uptime_text(s)));
        }
        v
    }

    /// The summary as plain text, a line each ("Processor: ..."), for a bug
    /// report or a post (Dave: built so it could be shared).
    pub(crate) fn summary_text(&self) -> String {
        self.summary().into_iter().map(|(n, v)| format!("{n}: {v}\n")).collect()
    }

    pub(crate) fn about(&self) -> Element<Message> {
        let mut rows: Vec<Element<Message>> =
            self.summary().into_iter().map(|(n, v)| setting(n, "", label(v).dim())).collect();
        rows.push(setting(
            "Share",
            "These lines as plain text, for a bug report or a question.",
            row(vec![
                button(label(if self.about_copied { "Copied" } else { "Copy" }).size(12.0))
                    .padding(Padding::hv(4.0, 12.0))
                    .on_press(Message::CopySummary)
                    .name("Copy the summary"),
                button(label("More in Hardware\u{2026}").size(12.0))
                    .padding(Padding::hv(4.0, 12.0))
                    .on_press(Message::OpenHardware)
                    .name("Open Hardware"),
            ])
            .spacing(8.0),
        ));
        self.page_layout_headed(
            "About",
            Some(self.lockup()),
            "Vergence: every app the right size, in the right place and readable, on any monitor setup.",
            vec![
                self.group(Some("This computer"), rows),
                self.group(
                    None,
                    vec![
                        setting("Built with", "", label("vergence-ui, Vergence's own toolkit")),
                        setting("Copyright", "", label(COPYRIGHT)),
                        setting(
                            "License",
                            "Apache License 2.0, or GNU GPL version 2 or later, at your choice. No warranty.",
                            button(label("Read").size(12.0))
                                .padding(Padding::hv(4.0, 12.0))
                                .on_press(Message::Go(Page::LicenseText))
                                .name("Read the license"),
                        ),
                        setting(
                            "Third-party software",
                            &format!("{} crates, each under its own license", notices_summary().len()),
                            button(label("List").size(12.0))
                                .padding(Padding::hv(4.0, 12.0))
                                .on_press(Message::Go(Page::Notices))
                                .name("List third-party software"),
                        ),
                    ],
                ),
            ],
        )
    }

    fn back_to_about(&self) -> Element<Message> {
        row(vec![
            button(label("\u{2039} About").size(12.0))
                .padding(Padding::hv(4.0, 12.0))
                .on_press(Message::Go(Page::About))
                .name("Back to About"),
            space().width(Length::FILL),
        ])
        .width(Length::FILL)
    }

    /// Both license texts, a paragraph to a label so they wrap to the window.
    pub(crate) fn license_page(&self) -> Element<Message> {
        let text = |t: &'static str| -> Element<Message> {
            let paras: Vec<Element<Message>> =
                paragraphs(t).into_iter().map(|p| label(p).size(11.0).wrap().width(Length::FILL)).collect();
            panel(column(paras).spacing(8.0).width(Length::FILL)).padding(Padding::hv(12.0, 14.0)).width(Length::FILL)
        };
        self.page_layout(
            "License",
            "Vergence is yours to use under either license below, at your choice: Apache-2.0 OR GPL-2.0-or-later.",
            vec![
                self.back_to_about(),
                self.group(Some("Apache License 2.0 (LICENSE-APACHE)"), vec![text(LICENSE_APACHE)]),
                self.group(Some("GNU General Public License, version 2 (LICENSE-GPL)"), vec![text(LICENSE_GPL)]),
            ],
        )
    }

    /// The crates Vergence is built with, from THIRD-PARTY-NOTICES.md's summary.
    pub(crate) fn notices_page(&self) -> Element<Message> {
        let rows: Vec<Element<Message>> = notices_summary()
            .into_iter()
            .map(|(name, version, license)| setting(&format!("{name} {version}"), "", label(license).size(11.0).dim()))
            .collect();
        let full = match installed_doc_dir() {
            Some(d) => {
                format!("Their full license and notice texts are in {}.", d.join("THIRD-PARTY-NOTICES.md").display())
            }
            None => "Their full license and notice texts are in THIRD-PARTY-NOTICES.md, beside the source.".to_string(),
        };
        self.page_layout(
            "Third-party software",
            &format!("Vergence is built with this code from other projects, each under its own license. {full}"),
            vec![self.back_to_about(), self.group(None, rows)],
        )
    }
}

#[cfg(test)]
mod about_tests {
    use super::*;

    #[test]
    fn memory_as_sold() {
        assert_eq!(installed_memory(65_600_000_000), "64 GB");
        assert_eq!(installed_memory(16_400_000_000), "16 GB");
        assert_eq!(installed_memory(7_900_000_000), "8 GB");
        assert_eq!(installed_memory(0), "");
    }

    #[test]
    fn notices_summary_reads_the_table() {
        let rows = notices_summary();
        assert!(rows.len() > 100, "{} rows", rows.len());
        assert!(rows.iter().any(|(n, _, _)| n == "smithay"));
        assert!(rows.iter().all(|(n, v, l)| !n.is_empty() && !v.is_empty() && !l.is_empty()));
    }

    #[test]
    fn license_texts_split_into_paragraphs() {
        let a = paragraphs(LICENSE_APACHE);
        let g = paragraphs(LICENSE_GPL);
        assert!(a.len() > 20 && g.len() > 20);
        assert!(a.iter().any(|p| p.starts_with("Apache License")));
        assert!(g.iter().any(|p| p.starts_with("GNU GENERAL PUBLIC LICENSE")));
        assert!(a.iter().chain(&g).all(|p| !p.contains('\n')));
    }
}
