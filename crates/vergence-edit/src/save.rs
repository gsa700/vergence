//! Edit's saving and closing: saving a tab, the file dialog (Open, Save
//! as) and the name it suggests, closing tabs with the "Save changes?"
//! question. The window itself is in main.rs.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Program, Theme, Waker, Window, button, column, label, panel, row,
    space,
};

use crate::{APP_ID, DocTab, EditWindow, files, stamp};

/// An answer to "Save changes?".
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Answer {
    Save,
    Discard,
    Cancel,
}

/// What a file dialog was opened for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Picking {
    Open,
    /// Save tab `tab` under a new name; then close it (the close question's Save).
    SaveAs {
        tab: u32,
        then_close: bool,
    },
}

/// A file dialog open over the window: what for, and where its answer lands.
pub(crate) struct Pick {
    pub(crate) what: Picking,
    pub(crate) answer: Rc<RefCell<Option<Vec<PathBuf>>>>,
}

/// A question open over the window, and what it leads to.
pub(crate) struct Asking {
    tab: u32,
    pub(crate) answer: Rc<Cell<Option<Answer>>>,
}

impl EditWindow {
    /// Saves tab `id`. False if it could not be (a notice says why).
    pub(crate) fn save(&mut self, id: u32) -> bool {
        let Some(t) = self.tabs.iter().find(|t| t.id == id) else { return false };
        if t.read_only.is_some() {
            self.notice = Some(format!("{} is open read-only.", t.name()));
            return false;
        }
        let Some(path) = t.path.clone() else { return false };
        let text = t.doc.borrow().text();
        match files::save(&path, &text, t.bom) {
            Ok(()) => {
                t.doc.borrow_mut().mark_saved();
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                    t.disk = stamp(&path);
                    t.changed_on_disk = false;
                }
                true
            }
            Err(e) => {
                self.notice = Some(e);
                false
            }
        }
    }

    /// Opens the file dialog: Open, or Save as for tab `tab`.
    pub(crate) fn pick(&mut self, what: Picking, window: &mut Window) {
        if self.picking.is_some() {
            return;
        }
        let answer: Rc<RefCell<Option<Vec<PathBuf>>>> = Rc::new(RefCell::new(None));
        let (slot, waker) = (answer.clone(), self.waker.clone());
        let done = move |paths: Vec<PathBuf>| {
            *slot.borrow_mut() = Some(paths);
            if let Some(w) = &waker {
                w.wake();
            }
        };
        let theme = self.theme();
        // The dialog starts in the document's own folder, if it has one.
        let here = match what {
            Picking::Open => self.front_tab(),
            Picking::SaveAs { tab, .. } => self.tabs.iter().find(|t| t.id == tab),
        }
        .and_then(|t| t.path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)));
        let dialog = match what {
            Picking::Open => vergence_widgets::filedialog::FileDialog::open(APP_ID, "Open", theme, done).filters(vec![
                vergence_widgets::filedialog::Filter::new("All files", &[]),
                // Every kind of text: scripts, Markdown, TOML, source code (files milestone 1).
                vergence_widgets::filedialog::Filter::of_types("Text files", &["text/plain"]),
            ]),
            Picking::SaveAs { tab, .. } => {
                let name = self.tabs.iter().find(|t| t.id == tab).map_or_else(|| "Untitled.txt".into(), suggested_name);
                vergence_widgets::filedialog::FileDialog::save(APP_ID, "Save as", &name, theme, done)
            }
        };
        let dialog = match here {
            Some(d) => dialog.folder(&d),
            None => dialog,
        };
        self.picking = Some(Pick { what, answer });
        window.open_window(vergence_widgets::window(dialog));
    }

    /// The file dialog answered (nothing: cancelled).
    pub(crate) fn picked(&mut self, what: Picking, paths: Vec<PathBuf>, window: &mut Window) {
        match what {
            Picking::Open => self.open_files(&paths),
            Picking::SaveAs { tab, then_close } => {
                let Some(p) = paths.into_iter().next() else {
                    // Cancelled: a close waiting on it stops.
                    if then_close {
                        self.closing.clear();
                        self.closing_window = false;
                    }
                    return;
                };
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                    t.path = Some(p);
                    t.read_only = None;
                }
                if self.save(tab) && then_close {
                    self.closing.retain(|t| *t != tab);
                    self.remove_tab(tab);
                    self.go_on_closing(window);
                } else if then_close {
                    self.closing.clear();
                    self.closing_window = false;
                }
                self.apply_options();
            }
        }
    }

    /// Saves tab `id`, asking where first if it has never been saved.
    pub(crate) fn save_or_ask(&mut self, id: u32, window: &mut Window) {
        let untitled = self.tabs.iter().any(|t| t.id == id && t.path.is_none());
        if untitled {
            self.pick(Picking::SaveAs { tab: id, then_close: false }, window);
        } else {
            self.save(id);
        }
    }

    /// Closes tab `id` (asking first if it has changes); the window goes
    /// with its last tab.
    pub(crate) fn close_tab(&mut self, id: u32, window: &mut Window) {
        if !self.closing.contains(&id) {
            self.closing.push(id);
        }
        self.go_on_closing(window);
    }

    /// Works through the tabs to close: those without changes go at once;
    /// the first with changes is asked about.
    pub(crate) fn go_on_closing(&mut self, window: &mut Window) {
        if self.asking.is_some() {
            return;
        }
        while let Some(&id) = self.closing.first() {
            match self.tabs.iter().find(|t| t.id == id) {
                Some(t) if t.modified() => {
                    self.front = id;
                    let answer = Rc::new(Cell::new(None));
                    let q = SaveQuestion {
                        name: t.name(),
                        answer: answer.clone(),
                        waker: self.waker.clone(),
                        theme: self.theme(),
                    };
                    self.asking = Some(Asking { tab: id, answer });
                    window.open_window(vergence_widgets::window(q));
                    return;
                }
                _ => {
                    self.closing.remove(0);
                    self.remove_tab(id);
                }
            }
        }
        if self.tabs.is_empty() || self.closing_window {
            self.closing_window = false;
            window.close();
        }
    }

    /// The question about a tab was answered.
    pub(crate) fn answered(&mut self, a: Answer, window: &mut Window) {
        let Some(asking) = self.asking.take() else { return };
        match a {
            Answer::Cancel => {
                self.closing.clear();
                self.closing_window = false;
            }
            // Never saved: where to, first; the closing goes on after.
            Answer::Save if self.tabs.iter().any(|t| t.id == asking.tab && t.path.is_none()) => {
                self.pick(Picking::SaveAs { tab: asking.tab, then_close: true }, window);
            }
            Answer::Save if !self.save(asking.tab) => {
                // Not saved: stop, and say why.
                self.closing.clear();
                self.closing_window = false;
            }
            _ => {
                self.closing.retain(|t| *t != asking.tab);
                self.remove_tab(asking.tab);
                self.go_on_closing(window);
            }
        }
    }
}

/// The name the Save dialog suggests: the file's own, or for a new
/// document its first words (as TextEdit does), else "Untitled".
pub(crate) fn suggested_name(t: &DocTab) -> String {
    if let Some(p) = &t.path {
        return files::name(p);
    }
    let doc = t.doc.borrow();
    let first = doc.buffer().lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let clean: String = first
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control())
        .collect();
    let words: Vec<&str> = clean.split_whitespace().collect();
    let mut name = String::new();
    for w in words {
        if name.chars().count() + w.chars().count() > 40 {
            break;
        }
        if !name.is_empty() {
            name.push(' ');
        }
        name.push_str(w);
    }
    let name = name.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    if name.is_empty() { format!("{}.txt", t.name()) } else { format!("{name}.txt") }
}

/// "Save changes?", a modal dialog over the window. The answer goes back
/// through `answer` and a wake of the window.
struct SaveQuestion {
    name: String,
    answer: Rc<Cell<Option<Answer>>>,
    waker: Option<Waker>,
    theme: Theme,
}

impl Program for SaveQuestion {
    type Message = Answer;
    fn title(&self) -> String {
        "Save changes?".into()
    }
    fn app_id(&self) -> String {
        APP_ID.into()
    }
    fn theme(&self) -> Theme {
        self.theme.clone()
    }
    fn view(&self, _: &Theme) -> Element<Answer> {
        let b = |text: &str, a: Answer, look: ButtonLook| -> Element<Answer> {
            button(label(text).size(12.0)).look(look).padding(Padding::hv(4.0, 12.0)).on_press(a).name(text)
        };
        panel(
            column(vec![
                label(format!("Save the changes to “{}”?", self.name))
                    .size(15.0)
                    .weight(600)
                    .wrap()
                    .width(Length::FILL),
                label("If you don't save them, they are lost.").dim().wrap().width(Length::FILL),
                row(vec![
                    b("Don't save", Answer::Discard, ButtonLook::Normal),
                    space().width(Length::FILL),
                    b("Cancel", Answer::Cancel, ButtonLook::Normal),
                    b("Save", Answer::Save, ButtonLook::Accent),
                ])
                .spacing(8.0)
                .align(Align::Center)
                .width(Length::FILL),
            ])
            .spacing(10.0)
            .width(Length::FILL),
        )
        .padding(20.0)
        .radius(0.0)
        .width(Length::FILL)
        .height(Length::FILL)
    }
    fn update(&mut self, a: Answer, window: &mut Window) {
        self.answer.set(Some(a));
        window.close();
    }
    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style {
            kind: vergence_ui::WinKind::Dialog,
            modal: true,
            min: (420.0, 150.0),
            max: (0.0, 0.0),
            alone: false,
            frosted: false,
        }
    }
}

impl Drop for SaveQuestion {
    /// Answered or closed, the window hears of it.
    fn drop(&mut self) {
        if let Some(w) = &self.waker {
            w.wake();
        }
    }
}
