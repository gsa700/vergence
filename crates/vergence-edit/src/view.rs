//! Edit's window drawn: the strips above the text (notices, read-only,
//! changed on disk, Go to line, the find bar), the text, the status line.
//! The window itself is in main.rs.

use vergence_widgets::edit::Ending;
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Theme, button, choice, column, field, label, panel, row, rule, space,
    text_area,
};

use crate::{EditWindow, Msg};

impl EditWindow {
    pub(crate) fn on_view(&self, t: &Theme) -> Element<Msg> {
        // The page as drawn now (tinted with the wallpaper, if it is), for the tab in front.
        self.page_drawn.set(Some((t.page.r as u32) << 16 | (t.page.g as u32) << 8 | t.page.b as u32));
        let Some(tab) = self.front_tab() else { return space() };
        // Strips above the text (notices, the find bar), on a shade over the
        // window's ground so its opacity shows through as everywhere else.
        let shade = vergence_scene::Color { a: if t.dark { 20 } else { 14 }, ..t.text };
        let mut parts: Vec<Element<Msg>> = Vec::new();
        let strip = |text: String, dismiss: bool| -> Element<Msg> {
            let mut items: Vec<Element<Msg>> = vec![label(text).size(12.0).wrap().width(Length::FILL)];
            if dismiss {
                items.push(
                    button(label("OK").size(12.0))
                        .padding(Padding::hv(2.0, 10.0))
                        .on_press(Msg::DismissNotice)
                        .name("OK"),
                );
            }
            panel(row(items).spacing(8.0).align(Align::Center).width(Length::FILL))
                .padding(Padding::hv(6.0, 12.0))
                .width(Length::FILL)
        };
        if let Some(n) = &self.notice {
            parts.push(strip(n.clone(), true));
        }
        if let Some(r) = &tab.read_only {
            parts.push(strip(r.clone(), false));
        }
        // Changed on disk while it has changes here: which to keep.
        if tab.changed_on_disk {
            parts.push(
                panel(
                    row(vec![
                        label(format!("“{}” changed on disk.", tab.name())).size(12.0).wrap().width(Length::FILL),
                        button(label("Keep mine").size(12.0))
                            .padding(Padding::hv(2.0, 10.0))
                            .on_press(Msg::KeepMine)
                            .name("Keep mine"),
                        button(label("Reload").size(12.0))
                            .look(ButtonLook::Accent)
                            .padding(Padding::hv(2.0, 10.0))
                            .on_press(Msg::Reload)
                            .name("Reload from disk"),
                    ])
                    .spacing(8.0)
                    .align(Align::Center)
                    .width(Length::FILL),
                )
                .padding(Padding::hv(6.0, 12.0))
                .width(Length::FILL),
            );
        }
        if let Some(g) = &self.goto {
            parts.push(
                panel(
                    row(vec![
                        label("Go to line").size(12.0),
                        field(g.clone(), Msg::GotoTyped)
                            .placeholder("Line, or line:column")
                            .on_submit(Msg::GotoGo)
                            .key(self.focus_turn)
                            .autofocus()
                            .width(Length::Fixed(200.0))
                            .name("Line"),
                        label(format!("of {}", grouped(tab.doc.borrow().buffer().line_count()))).size(12.0).dim(),
                        space().width(Length::FILL),
                        button(label("×").size(12.0))
                            .look(ButtonLook::Plain)
                            .padding(Padding::hv(2.0, 8.0))
                            .on_press(Msg::GotoClose)
                            .name("Close"),
                    ])
                    .spacing(8.0)
                    .align(Align::Center)
                    .width(Length::FILL),
                )
                .padding(Padding::hv(6.0, 12.0))
                .width(Length::FILL),
            );
        }
        if let Some(f) = &self.find {
            parts.push(self.find_bar(f, t));
        }
        // With strips, they go under the server's bar (padding is a panel's,
        // not a column's) and the text below them; without, the text itself
        // starts under the bar and scrolls up beneath it, frosted, as the
        // terminal's does (the text area keeps its first line below the bar).
        let mut all: Vec<Element<Msg>> = Vec::new();
        if !parts.is_empty() {
            all.push(
                panel(column(std::mem::take(&mut parts)).spacing(1.0).width(Length::FILL))
                    .color(shade)
                    .padding(Padding { top: t.top, ..Padding::default() })
                    .radius(0.0)
                    .width(Length::FILL),
            );
        }
        // The area's id changes with focus_turn, so it takes the keyboard back
        // when the find bar closes (its state is the document's, not the id's).
        let area =
            text_area(&tab.doc).key((self.focus_turn << 32) | tab.id as u64).on_change(Msg::Changed).name(tab.name());
        all.push(if self.find.is_none() && self.goto.is_none() { area.autofocus() } else { area });
        // The status line: where the caret is, how Tab indents, the line endings.
        let d = tab.doc.borrow();
        let (l, c) = d.line_column();
        let (a, b) = d.selection();
        let mut status: Vec<Element<Msg>> = vec![space().width(Length::FILL)];
        // Words: a word count in the status line (see docs/editor-milestone-1.md).
        let words = grouped(d.word_count());
        status.push(
            label(if a != b {
                format!("{} of {words} words", grouped(d.selected_words()))
            } else {
                format!("{words} words")
            })
            .size(12.0)
            .dim(),
        );
        if a != b {
            let n = d.selected_text().chars().count();
            status.push(label(format!("{} characters selected", grouped(n))).size(12.0).dim());
        }
        status.push(label(format!("Line {l}, Column {c}")).size(12.0).dim());
        let o = d.options();
        let indent = if o.spaces { format!("Spaces: {}", o.tab_width) } else { format!("Tab width: {}", o.tab_width) };
        let ending = if d.buffer().ending() == Ending::CrLf { "CRLF" } else { "LF" };
        let small = |text: String, m: Msg, name: &str| -> Element<Msg> {
            button(label(text).size(12.0).dim())
                .look(ButtonLook::Plain)
                .padding(Padding::hv(1.0, 6.0))
                .on_press(m)
                .name(name)
        };
        status.push(small(indent, Msg::ToggleSpaces, "Indent with spaces or tabs"));
        status.push(small(ending.into(), Msg::ToggleEnding, "Line endings"));
        // The language: a list to choose another (as View > Language).
        let mut names = vec!["Plain Text".to_string()];
        names.extend(vergence_syntax::languages().iter().map(|l| l.name.clone()));
        let at = tab.lang.map_or(0, |l| l.0 as usize + 1);
        status.push(choice(names, Some(at), Msg::Language).name("Language"));
        status.push(label("UTF-8").size(12.0).dim());
        all.push(rule());
        all.push(
            panel(row(status).spacing(14.0).align(Align::Center).width(Length::FILL))
                .radius(0.0)
                .padding(Padding { top: 3.0, bottom: 3.0, left: 12.0, right: 12.0 })
                .width(Length::FILL),
        );
        column(all).width(Length::FILL).height(Length::FILL)
    }
}

/// 1234567 as "1,234,567".
pub(crate) fn grouped(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
