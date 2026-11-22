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

impl App {
    pub(crate) fn about(&self) -> Element<Message> {
        let session = match (std::env::var("VERGENCE_BUS"), std::env::var("WAYLAND_DISPLAY")) {
            (Ok(_), Ok(w)) => format!("Vergence on {w}"),
            (_, Ok(w)) => format!("another Wayland desktop ({w})"),
            _ => "an X11 desktop".to_string(),
        };
        self.page_layout(
            "About",
            "Vergence: every app the right size, in the right place and readable, on any monitor setup.",
            vec![
                self.group(
                    None,
                    vec![
                        setting("Settings app", "", label(format!("version {}", env!("CARGO_PKG_VERSION")))),
                        setting("Session", "", label(session)),
                        setting("Built with", "", label("vergence-ui, Vergence's own toolkit")),
                    ],
                ),
                self.group(
                    None,
                    vec![
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
