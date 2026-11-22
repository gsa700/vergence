//! A document's Info window (Reader milestone 1, step 4): the file (its
//! name, folder and size), its pages (how many, the first one's size in
//! millimeters and inches) and what the document says about itself (its
//! Info dictionary: title, author, subject, keywords, the application that
//! made it and what made the PDF, its dates, the PDF version, encryption),
//! read by the helper. Ctrl+I or the toolbar's Info.

use std::path::Path;

use vergence_widgets::{Align, Element, Length, Padding, Program, Theme, Window, column, label, panel, row, scroll};

pub struct Info {
    title: String,
    rows: Vec<(String, String)>,
    theme: Theme,
}

/// A size in bytes as people read it.
fn bytes(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.0} KB", n as f64 / 1024.0),
        n => format!("{n} bytes"),
    }
}

/// A page size in points as millimeters and inches, with its paper's name
/// when it is a common one.
pub fn page_size(w: f32, h: f32) -> String {
    let (mm_w, mm_h) = (w * 25.4 / 72.0, h * 25.4 / 72.0);
    let named = [("A4", 210.0, 297.0), ("A5", 148.0, 210.0), ("A3", 297.0, 420.0), ("Letter", 215.9, 279.4), ("Legal", 215.9, 355.6), ("Tabloid", 279.4, 431.8)]
        .iter()
        .find(|(_, a, b)| ((mm_w - a).abs() < 2.0 && (mm_h - b).abs() < 2.0) || ((mm_w - b).abs() < 2.0 && (mm_h - a).abs() < 2.0))
        .map(|(n, ..)| format!("{n}, "))
        .unwrap_or_default();
    format!("{named}{:.0} × {:.0} mm ({:.2} × {:.2} in)", mm_w, mm_h, w / 72.0, h / 72.0)
}

/// What the document's own names are called here.
fn label_of(key: &str) -> &str {
    match key {
        "Creator" => "Made with",
        "Producer" => "PDF made by",
        "CreationDate" => "Created",
        "ModDate" => "Modified",
        "Version" => "PDF version",
        other => other,
    }
}

impl Info {
    pub fn new(path: &Path, pages: &[(f32, f32)], meta: &[(String, String)], theme: Theme) -> Info {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut rows = vec![("File".to_string(), name.clone()), ("Folder".to_string(), path.parent().map(Path::to_path_buf).unwrap_or_default().display().to_string())];
        if let Ok(m) = std::fs::metadata(path) {
            rows.push(("Size".into(), bytes(m.len())));
        }
        rows.push(("Pages".into(), pages.len().to_string()));
        if let Some(&(w, h)) = pages.first() {
            let same = pages.iter().all(|p| (p.0 - w).abs() < 1.0 && (p.1 - h).abs() < 1.0);
            rows.push((if same { "Page size" } else { "First page" }.into(), page_size(w, h)));
        }
        rows.extend(meta.iter().map(|(k, v)| (label_of(k).to_string(), v.clone())));
        Info { title: format!("{name} Info"), rows, theme }
    }
}

impl Program for Info {
    type Message = ();

    fn title(&self) -> String {
        self.title.clone()
    }

    fn app_id(&self) -> String {
        "vergence-reader".into()
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    fn view(&self, t: &Theme) -> Element<()> {
        let lines: Vec<Element<()>> = self
            .rows
            .iter()
            .map(|(k, v)| row(vec![label(k.clone()).size(12.0).dim().align(Align::End).width(Length::Fixed(96.0)), label(v.clone()).size(12.0).wrap().width(Length::FILL)]).spacing(12.0).width(Length::FILL))
            .collect();
        panel(scroll(column(lines).spacing(7.0).width(Length::FILL))).padding(Padding { top: t.top + 14.0, bottom: 16.0, left: 18.0, right: 18.0 }).width(Length::FILL).height(Length::FILL)
    }

    fn update(&mut self, _: (), _window: &mut Window) {}

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { kind: vergence_ui::WinKind::Dialog, modal: false, min: (440.0, 380.0), max: (0.0, 0.0), alone: false, frosted: false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_sizes_by_name() {
        assert!(page_size(595.0, 842.0).starts_with("A4, 210 × 297 mm"), "{}", page_size(595.0, 842.0));
        assert!(page_size(612.0, 792.0).starts_with("Letter, 216 × 279 mm (8.50 × 11.00 in)"), "{}", page_size(612.0, 792.0));
        assert!(page_size(792.0, 612.0).starts_with("Letter"), "turned the other way, still Letter");
        assert!(page_size(300.0, 300.0).starts_with("106 × 106 mm"));
        assert_eq!(bytes(1536), "2 KB");
        assert_eq!(bytes(5 << 20), "5.0 MB");
    }
}
