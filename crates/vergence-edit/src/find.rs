//! Edit's find bar and Go to line: what is looked for and found, choosing
//! matches, replacing, the bar drawn. The window itself is in main.rs.

use vergence_widgets::{Align, ButtonLook, Element, Length, Padding, Theme, button, column, field, label, panel, row};

use crate::{EditWindow, Msg, grouped};

/// The most matches lit and counted.
const MAX_MATCHES: usize = 10_000;

/// The find bar (step 5): what is looked for, with what it is replaced,
/// how, and what was found in the document in front.
#[derive(Default)]
pub(crate) struct Find {
    pub(crate) needle: String,
    pub(crate) with: String,
    pub(crate) case: bool,
    pub(crate) whole: bool,
    pub(crate) replace: bool,
    pub(crate) matches: Vec<(vergence_widgets::edit::Pos, vergence_widgets::edit::Pos)>,
    /// The document's revision and tab the matches were found in.
    pub(crate) found_in: (u32, u64),
    /// Where the caret was when the bar opened: typing searches from there.
    origin: vergence_widgets::edit::Pos,
    /// After Replace all: how many.
    pub(crate) replaced: Option<usize>,
}

impl EditWindow {
    /// Go to line: "12", or "12:5" for a column too.
    pub(crate) fn go_to(&mut self, what: &str) -> bool {
        let Some(doc) = self.doc() else { return false };
        let mut parts = what.trim().splitn(2, [':', ',', ' ']);
        let Some(line) = parts.next().and_then(|l| l.trim().parse::<usize>().ok()).filter(|l| *l > 0) else {
            return false;
        };
        let col = parts.next().and_then(|c| c.trim().parse::<usize>().ok()).unwrap_or(1).max(1);
        let mut d = doc.borrow_mut();
        let line = (line - 1).min(d.buffer().line_count() - 1);
        // The column in characters, as the status line counts them.
        let text = d.buffer().line(line).to_string();
        let byte = text.char_indices().nth(col - 1).map_or(text.len(), |(b, _)| b);
        let p = vergence_widgets::edit::Pos::new(line, byte);
        d.set_selection(p, p);
        true
    }

    /// Opens the find bar (with Replace, if `replace`): the selection, if on
    /// one line, is what it looks for.
    pub(crate) fn open_find(&mut self, replace: bool) {
        let Some(doc) = self.doc() else { return };
        let (sel, (a, _)) = {
            let d = doc.borrow();
            (d.selected_text(), d.selection())
        };
        let f = self.find.get_or_insert_with(Find::default);
        if !sel.is_empty() && !sel.contains('\n') {
            f.needle = sel;
        }
        f.replace |= replace;
        f.origin = a;
        f.found_in = (0, u64::MAX);
        self.focus_turn += 1;
        self.refind();
    }

    pub(crate) fn close_find(&mut self) {
        if self.find.take().is_some() {
            for t in &self.tabs {
                t.doc.borrow_mut().set_marks(Vec::new());
            }
            self.focus_turn += 1;
        }
    }

    /// Finds again in the document in front, if it or what is looked for changed.
    pub(crate) fn refind(&mut self) {
        let Some(t) = self.front_tab() else { return };
        let (id, doc) = (t.id, t.doc.clone());
        let Some(f) = self.find.as_mut() else { return };
        let rev = doc.borrow().revision();
        if f.found_in == (id, rev) {
            return;
        }
        f.matches = doc.borrow().find_all(&f.needle, f.case, f.whole, MAX_MATCHES);
        f.found_in = (id, rev);
        doc.borrow_mut().set_marks(f.matches.clone());
    }

    /// Chooses the next match after the selection (or before, `back`), round the ends.
    pub(crate) fn find_step(&mut self, back: bool) {
        self.refind();
        let Some(doc) = self.doc() else { return };
        let Some(f) = self.find.as_ref() else { return };
        if f.matches.is_empty() {
            return;
        }
        let (a, b) = doc.borrow().selection();
        let k = if back {
            f.matches.iter().rposition(|m| m.0 < a).unwrap_or(f.matches.len() - 1)
        } else {
            f.matches.iter().position(|m| m.0 >= b && (m.0, m.1) != (a, b)).unwrap_or(0)
        };
        let (ma, mb) = f.matches[k];
        doc.borrow_mut().set_selection(ma, mb);
    }

    /// Typing in the find field: the first match from where the bar opened.
    pub(crate) fn find_from_origin(&mut self) {
        self.refind();
        let Some(doc) = self.doc() else { return };
        let Some(f) = self.find.as_ref() else { return };
        let hit = f.matches.iter().find(|m| m.0 >= f.origin).or(f.matches.first()).copied();
        match hit {
            Some((a, b)) => doc.borrow_mut().set_selection(a, b),
            None => {
                let o = f.origin;
                doc.borrow_mut().set_selection(o, o);
            }
        }
    }

    /// The match chosen, if the selection is one.
    pub(crate) fn chosen_match(&self) -> bool {
        let (Some(doc), Some(f)) = (self.doc(), self.find.as_ref()) else { return false };
        let sel = doc.borrow().selection();
        f.matches.iter().any(|m| (m.0, m.1) == sel)
    }
}

impl EditWindow {
    pub(crate) fn find_bar(&self, f: &Find, t: &Theme) -> Element<Msg> {
        let small = |text: &str, m: Msg, on: bool, name: &str| -> Element<Msg> {
            button(label(text).size(12.0))
                .look(if on { ButtonLook::Chosen } else { ButtonLook::Plain })
                .padding(Padding::hv(3.0, 8.0))
                .on_press(m)
                .name(name)
        };
        let count = match (f.replaced, f.needle.is_empty(), f.matches.len()) {
            (Some(n), _, _) => format!("Replaced {}", grouped(n)),
            (_, true, _) => String::new(),
            (_, _, 0) => "Not found".into(),
            (_, _, n) => {
                let at = self.doc().and_then(|d| {
                    let sel = d.borrow().selection();
                    f.matches.iter().position(|m| (m.0, m.1) == sel)
                });
                let n = if n >= MAX_MATCHES { format!("{}+", grouped(n)) } else { grouped(n) };
                match at {
                    Some(k) => format!("{} of {n}", grouped(k + 1)),
                    None => format!("{n} found"),
                }
            }
        };
        let find_field = field(f.needle.clone(), Msg::FindTyped)
            .placeholder("Find")
            .on_submit(Msg::FindNext)
            .key(self.focus_turn)
            .autofocus()
            .width(Length::FILL)
            .name("Find");
        let mut rows: Vec<Element<Msg>> = vec![
            row(vec![
                find_field,
                label(count).size(12.0).dim().width(Length::Fixed(96.0)),
                small("‹", Msg::FindPrev, false, "Previous match"),
                small("›", Msg::FindNext, false, "Next match"),
                small("Aa", Msg::FindCase, f.case, "Match case"),
                small("Whole words", Msg::FindWhole, f.whole, "Whole words"),
                small("Replace", Msg::FindReplace, f.replace, "Show Replace"),
                small("×", Msg::FindClose, false, "Close"),
            ])
            .spacing(4.0)
            .align(Align::Center)
            .width(Length::FILL),
        ];
        if f.replace {
            let editable = self.front_tab().is_some_and(|t| t.read_only.is_none());
            rows.push(
                row(vec![
                    field(f.with.clone(), Msg::WithTyped)
                        .placeholder("Replace with")
                        .on_submit(Msg::ReplaceOne)
                        .width(Length::FILL)
                        .name("Replace with"),
                    button(label("Replace").size(12.0))
                        .padding(Padding::hv(3.0, 10.0))
                        .on_press_if(editable, Msg::ReplaceOne)
                        .name("Replace"),
                    button(label("Replace all").size(12.0))
                        .padding(Padding::hv(3.0, 10.0))
                        .on_press_if(editable && !f.matches.is_empty(), Msg::ReplaceAll)
                        .name("Replace all"),
                ])
                .spacing(6.0)
                .align(Align::Center)
                .width(Length::FILL),
            );
        }
        let _ = t;
        panel(column(rows).spacing(6.0).width(Length::FILL))
            .radius(0.0)
            .padding(Padding::hv(6.0, 10.0))
            .width(Length::FILL)
    }
}
