//! The text area by itself (editor milestone 1, step 2): a file to edit,
//! with line numbers, wrapping, indenting, undo and the clipboard, and a
//! status line. Nothing is saved.
//! `cargo run --release -p vergence-widgets --example varea [FILE]`.
use vergence_widgets::{Align, Element, Length, Padding, Program, TextDoc, Theme, Window, column, label, panel, row, run, space, text_area};

#[derive(Debug, Clone)]
enum Msg {
    Changed,
}

struct Area {
    doc: TextDoc,
    name: String,
}

impl Program for Area {
    type Message = Msg;
    fn title(&self) -> String {
        let dot = if self.doc.borrow().modified() { "• " } else { "" };
        format!("{dot}{}", self.name)
    }
    fn app_id(&self) -> String {
        "varea".into()
    }
    fn view(&self, theme: &Theme) -> Element<Msg> {
        let d = self.doc.borrow();
        let (l, c) = d.line_column();
        let (a, b) = d.selection();
        let chosen = if a == b { String::new() } else { format!("{} chosen   ", d.selected_text().chars().count()) };
        let status = row(vec![
            space().width(Length::FILL),
            label(format!("{chosen}Line {l}, Column {c}   {} lines   Spaces: {}", d.buffer().line_count(), d.options().tab_width)).size(12.0).dim(),
        ])
        .align(Align::Center)
        .width(Length::FILL);
        let status = panel(status).padding(Padding { top: 4.0, bottom: 4.0, left: 12.0, right: 12.0 }).radius(0.0).width(Length::FILL);
        panel(column(vec![text_area(&self.doc).on_change(Msg::Changed).name("Text"), status]).width(Length::FILL).height(Length::FILL))
            .padding(Padding { top: theme.top, ..Padding::default() })
            .radius(0.0)
            .width(Length::FILL)
            .height(Length::FILL)
    }
    fn update(&mut self, _: Msg, _: &mut Window) {}
}

fn main() -> std::io::Result<()> {
    let (name, text) = match std::env::args().nth(1) {
        Some(path) => {
            let text = std::fs::read_to_string(&path)?;
            (std::path::Path::new(&path).file_name().map_or(path.clone(), |n| n.to_string_lossy().into_owned()), text)
        }
        None => ("Untitled".into(), "A text area.\n\n    Type, select, undo (Ctrl+Z), indent lines with Tab and Shift+Tab.\n    Long lines wrap at the window's edge, and the line numbers count each line once however many rows it takes.\n".into()),
    };
    run(Area { doc: TextDoc::new(&text), name }, (720, 520))
}
