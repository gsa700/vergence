//! Vergence's text editor (editor milestone 1, docs/editor-milestone-1.md;
//! Dave: "a native text editor for Vergence. Like TextEdit on Mac or GNOME
//! Edit. Autocomplete and line numbers required."). Plain text for now
//! ("other formats later md rtf etc"), on the toolkit's text area.
//!
//! - **Windows and tabs**: one process for all its windows (hand.rs); a
//!   document a tab, in the bar the server draws; tabs dragged between
//!   windows or out into their own. A tab shows its file's name, with a dot
//!   while it has changes not saved.
//! - **Files** (files.rs): UTF-8, line endings and a final newline kept as
//!   found; saving writes a new file and renames it over the old, so a
//!   crash never leaves half a file. Text that is not UTF-8 opens read-only.
//! - **Closing with changes asks first**: Save, Don't save, Cancel.
//! - **Its settings** (Ctrl+,): font, text size, line numbers, wrapping,
//!   tabs, the current line's shade, its look; desktop.toml `[editor]`.

mod files;
mod hand;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use vergence_ui::{AppCommand as C, Bar, BarStyle, BarTab, Role, TabEvent};
use vergence_widgets::edit::Ending;
use vergence_widgets::prefs::{Choice, Control, Part, Prefs, Row, Rows};
use vergence_widgets::{Align, AreaOptions, ButtonLook, Element, Input, Length, Padding, Program, TextDoc, Theme, Waker, Window, button, column, label, panel, row, space, text_area};

const APP_ID: &str = "vergence-edit";
const SECTION: &str = "[editor]";

mod cmd {
    pub const NEW_WINDOW: u32 = 1;
    pub const NEW_TAB: u32 = 2;
    pub const SAVE: u32 = 4;
    pub const SAVE_AS: u32 = 5;
    pub const CLOSE_TAB: u32 = 6;
    pub const UNDO: u32 = 10;
    pub const REDO: u32 = 11;
    pub const CUT: u32 = 12;
    pub const COPY: u32 = 13;
    pub const PASTE: u32 = 14;
    pub const SELECT_ALL: u32 = 15;
    pub const ZOOM_IN: u32 = 20;
    pub const ZOOM_OUT: u32 = 21;
    pub const ZOOM_RESET: u32 = 22;
    pub const LINE_NUMBERS: u32 = 23;
    pub const WRAP: u32 = 24;
    pub const NEXT_TAB: u32 = 30;
    pub const PREV_TAB: u32 = 31;
    pub const SETTINGS: u32 = 40;
    pub const ABOUT: u32 = 41;
}

/// What all the editor's windows share (they run on one thread).
struct Shared {
    /// Each open window's toolkit number and the means to wake it.
    windows: Vec<(u32, Waker)>,
    /// Tabs on their way to another window: (that window, where in its row, the tab).
    transfers: Vec<(u32, u32, DocTab)>,
    /// Files other starts handed over (hand.rs).
    inbox: Arc<Mutex<Vec<Vec<PathBuf>>>>,
    /// The waker the hand-over thread calls: the first window's.
    listener: Arc<Mutex<Option<Waker>>>,
    /// The ids tabs are given, across windows (so a joined tab keeps its id).
    next_tab: u32,
}

type SharedRef = Rc<RefCell<Shared>>;

/// One document in a tab.
struct DocTab {
    id: u32,
    doc: TextDoc,
    path: Option<PathBuf>,
    /// "Untitled", "Untitled 2", … for a document never saved.
    untitled: u32,
    read_only: Option<String>,
    bom: bool,
    /// Tab inserts spaces (from the file's own indenting, or the status line).
    spaces: Option<bool>,
}

impl DocTab {
    fn name(&self) -> String {
        match &self.path {
            Some(p) => files::name(p),
            None if self.untitled <= 1 => "Untitled".into(),
            None => format!("Untitled {}", self.untitled),
        }
    }

    fn modified(&self) -> bool {
        self.doc.borrow().modified()
    }
}

#[derive(Debug, Clone)]
enum Msg {
    Changed,
    DismissNotice,
    ToggleSpaces,
    ToggleEnding,
}

/// An answer to "Save changes?".
#[derive(Debug, Clone, Copy, PartialEq)]
enum Answer {
    Save,
    Discard,
    Cancel,
}

/// A question open over the window, and what it leads to.
struct Asking {
    tab: u32,
    answer: Rc<Cell<Option<Answer>>>,
}

struct EditWindow {
    shared: SharedRef,
    tabs: Vec<DocTab>,
    front: u32,
    me: Option<u32>,
    waker: Option<Waker>,
    desktop: String,
    stamp: Option<SystemTime>,
    /// Ctrl+= and Ctrl+−, for this window while it is open.
    zoom: f32,
    prefs_open: Rc<Cell<bool>>,
    asking: Option<Asking>,
    /// Tabs still to close (each asked about first if changed), and whether
    /// the window closes after them.
    closing: Vec<u32>,
    closing_window: bool,
    notice: Option<String>,
}

/// ~/.config/vergence/desktop.toml and when it last changed.
fn desktop_text() -> (String, Option<SystemTime>) {
    match vergence_widgets::prefs::desktop_path() {
        Some(p) => (std::fs::read_to_string(&p).unwrap_or_default(), std::fs::metadata(&p).and_then(|m| m.modified()).ok()),
        None => (String::new(), None),
    }
}

impl EditWindow {
    fn new(shared: SharedRef, tabs: Vec<DocTab>) -> EditWindow {
        let (desktop, stamp) = desktop_text();
        let front = tabs.first().map_or(0, |t| t.id);
        let mut w = EditWindow { shared, tabs, front, me: None, waker: None, desktop, stamp, zoom: 1.0, prefs_open: Rc::new(Cell::new(false)), asking: None, closing: Vec::new(), closing_window: false, notice: None };
        if w.tabs.is_empty() {
            let t = w.untitled_tab();
            w.front = t.id;
            w.tabs.push(t);
        }
        w.apply_options();
        w
    }

    fn setting(&self, key: &str) -> Option<String> {
        vergence_ui::table_value(&self.desktop, SECTION, key)
    }

    fn flag(&self, key: &str, default: bool) -> bool {
        match self.setting(key).as_deref() {
            Some("true") => true,
            Some("false") => false,
            _ => default,
        }
    }

    /// How a document's area shows and edits, from the settings.
    fn options_for(&self, t: &DocTab) -> AreaOptions {
        let size = self.setting("text-size").and_then(|s| s.parse::<f32>().ok()).filter(|s| (6.0..=72.0).contains(s));
        let base = size.unwrap_or_else(|| self.theme().text_size);
        AreaOptions {
            font: self.setting("font").filter(|f| !f.is_empty()).unwrap_or_else(|| "monospace".into()),
            size: Some((base * self.zoom).clamp(6.0, 96.0)),
            line_numbers: self.flag("line-numbers", true),
            wrap: self.flag("wrap", true),
            tab_width: self.setting("tab-width").and_then(|s| s.parse().ok()).unwrap_or(4),
            spaces: t.spaces.unwrap_or_else(|| self.flag("spaces", true)),
            current_line: self.flag("current-line", true),
            editable: t.read_only.is_none(),
        }
    }

    fn apply_options(&mut self) {
        for i in 0..self.tabs.len() {
            let o = self.options_for(&self.tabs[i]);
            self.tabs[i].doc.borrow_mut().set_options(o);
        }
    }

    /// Joins the window list once both its number and its waker are known
    /// (the toolkit gives the number first); the first window also takes
    /// what other starts hand over.
    fn register(&mut self) {
        let (Some(id), Some(w)) = (self.me, self.waker.clone()) else { return };
        let mut s = self.shared.borrow_mut();
        if s.windows.iter().any(|(i, _)| *i == id) {
            return;
        }
        s.windows.push((id, w.clone()));
        let mut l = s.listener.lock().unwrap_or_else(|e| e.into_inner());
        if l.is_none() {
            *l = Some(w);
        }
    }

    fn new_tab_id(&self) -> u32 {
        let mut s = self.shared.borrow_mut();
        s.next_tab += 1;
        s.next_tab
    }

    fn untitled_tab(&self) -> DocTab {
        // The lowest number no open Untitled here has.
        let n = (1..).find(|n| !self.tabs.iter().any(|t| t.path.is_none() && t.untitled == *n)).unwrap_or(1);
        DocTab { id: self.new_tab_id(), doc: TextDoc::new(""), path: None, untitled: n, read_only: None, bom: false, spaces: None }
    }

    /// A tab for `path`, read in; None (and a notice) if it cannot be.
    fn file_tab(&mut self, path: &Path) -> Option<DocTab> {
        match files::load(path) {
            Ok(l) => {
                let doc = TextDoc::new(&l.text);
                doc.borrow_mut().mark_saved();
                let spaces = doc.borrow().buffer().uses_tabs().map(|tabs| !tabs);
                Some(DocTab { id: self.new_tab_id(), doc, path: Some(path.to_path_buf()), untitled: 0, read_only: l.read_only, bom: l.bom, spaces })
            }
            Err(e) => {
                self.notice = Some(e);
                None
            }
        }
    }

    /// Opens files in tabs here (one already open comes to the front instead).
    fn open_files(&mut self, paths: &[PathBuf]) {
        for p in paths {
            if let Some(t) = self.tabs.iter().find(|t| t.path.as_deref() == Some(p.as_path())) {
                self.front = t.id;
                continue;
            }
            if let Some(t) = self.file_tab(p) {
                // An untouched empty Untitled gives way to the first file.
                let lone = self.tabs.len() == 1 && self.tabs[0].path.is_none() && !self.tabs[0].modified() && self.tabs[0].doc.borrow().buffer().bytes() == 0;
                if lone {
                    self.tabs.clear();
                }
                self.front = t.id;
                self.tabs.push(t);
            }
        }
        self.apply_options();
    }

    fn front_tab(&self) -> Option<&DocTab> {
        self.tabs.iter().find(|t| t.id == self.front).or(self.tabs.first())
    }

    fn front_index(&self) -> usize {
        self.tabs.iter().position(|t| t.id == self.front).unwrap_or(0)
    }

    fn doc(&self) -> Option<TextDoc> {
        self.front_tab().map(|t| t.doc.clone())
    }

    /// Saves tab `id`. False if it could not be (a notice says why).
    fn save(&mut self, id: u32) -> bool {
        let Some(t) = self.tabs.iter().find(|t| t.id == id) else { return false };
        if t.read_only.is_some() {
            self.notice = Some(format!("{} is open read-only.", t.name()));
            return false;
        }
        let Some(path) = t.path.clone() else {
            self.notice = Some("Saving a new document needs Save as…, which comes with the file dialog (editor milestone 1, step 4).".into());
            return false;
        };
        let text = t.doc.borrow().text();
        match files::save(&path, &text, t.bom) {
            Ok(()) => {
                t.doc.borrow_mut().mark_saved();
                true
            }
            Err(e) => {
                self.notice = Some(e);
                false
            }
        }
    }

    /// Closes tab `id` (asking first if it has changes); the window goes
    /// with its last tab.
    fn close_tab(&mut self, id: u32, window: &mut Window) {
        if !self.closing.contains(&id) {
            self.closing.push(id);
        }
        self.go_on_closing(window);
    }

    /// Works through the tabs to close: those without changes go at once;
    /// the first with changes is asked about.
    fn go_on_closing(&mut self, window: &mut Window) {
        if self.asking.is_some() {
            return;
        }
        while let Some(&id) = self.closing.first() {
            match self.tabs.iter().find(|t| t.id == id) {
                Some(t) if t.modified() => {
                    self.front = id;
                    let answer = Rc::new(Cell::new(None));
                    let q = SaveQuestion { name: t.name(), answer: answer.clone(), waker: self.waker.clone(), theme: self.theme() };
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

    fn remove_tab(&mut self, id: u32) -> Option<DocTab> {
        let i = self.tabs.iter().position(|t| t.id == id)?;
        let t = self.tabs.remove(i);
        if self.front == id
            && let Some(n) = self.tabs.get(i.min(self.tabs.len().saturating_sub(1)))
        {
            self.front = n.id;
        }
        Some(t)
    }

    /// The question about a tab was answered.
    fn answered(&mut self, a: Answer, window: &mut Window) {
        let Some(asking) = self.asking.take() else { return };
        match a {
            Answer::Cancel => {
                self.closing.clear();
                self.closing_window = false;
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

    fn open_settings(&mut self, window: &mut Window) {
        if self.prefs_open.get() {
            return;
        }
        let waker = self.waker.clone();
        let prefs = Prefs::new("Text Editor", APP_ID, SECTION, false, Box::new(EditRows::new()), self.prefs_open.clone()).on_write(move || {
            if let Some(w) = &waker {
                w.wake();
            }
        });
        window.open_window(vergence_widgets::window(prefs));
    }

    /// desktop.toml again, if it changed.
    fn reread(&mut self) -> bool {
        let (text, stamp) = desktop_text();
        if stamp == self.stamp {
            return false;
        }
        (self.desktop, self.stamp) = (text, stamp);
        self.apply_options();
        true
    }

    fn write_key(&mut self, key: &str, value: Option<&str>) {
        let Some(path) = vergence_widgets::prefs::desktop_path() else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let new = vergence_widgets::prefs::set_key(&text, SECTION, key, value);
        if new != text {
            let beside = path.with_extension("toml.new");
            if std::fs::write(&beside, &new).and_then(|_| std::fs::rename(&beside, &path)).is_ok() {
                self.reread();
            }
        }
    }

    fn toggle_setting(&mut self, key: &str, default: bool) {
        let now = self.flag(key, default);
        // Back to the default is the key taken away.
        let value = if !now == default { None } else { Some(if now { "false" } else { "true" }) };
        self.write_key(key, value);
    }

    fn set_zoom(&mut self, z: f32) {
        self.zoom = z.clamp(0.5, 4.0);
        self.apply_options();
    }

    fn step_tab(&mut self, by: isize) {
        let n = self.tabs.len() as isize;
        if n > 1 {
            let i = (self.front_index() as isize + by).rem_euclid(n) as usize;
            self.front = self.tabs[i].id;
        }
    }

    /// The menu's commands, as the app's own keys too.
    fn run(&mut self, id: u32, window: &mut Window) {
        let doc = self.doc();
        match id {
            cmd::NEW_WINDOW => window.open_window(vergence_widgets::window(EditWindow::new(self.shared.clone(), Vec::new()))),
            cmd::NEW_TAB => {
                let t = self.untitled_tab();
                self.front = t.id;
                self.tabs.push(t);
                self.apply_options();
            }
            cmd::SAVE => {
                self.save(self.front);
            }
            cmd::SAVE_AS => self.notice = Some("Save as… comes with the file dialog (editor milestone 1, step 4).".into()),
            cmd::CLOSE_TAB => self.close_tab(self.front, window),
            cmd::UNDO => {
                if let Some(d) = doc {
                    d.borrow_mut().undo();
                }
            }
            cmd::REDO => {
                if let Some(d) = doc {
                    d.borrow_mut().redo();
                }
            }
            cmd::CUT | cmd::COPY => {
                if let Some(d) = doc {
                    let text = d.borrow().selected_text();
                    if !text.is_empty() {
                        window.copy(&text);
                        if id == cmd::CUT {
                            d.borrow_mut().insert("");
                        }
                    }
                }
            }
            cmd::PASTE => window.paste(),
            cmd::SELECT_ALL => {
                if let Some(d) = doc {
                    d.borrow_mut().select_all();
                }
            }
            cmd::ZOOM_IN => self.set_zoom(self.zoom * 1.1),
            cmd::ZOOM_OUT => self.set_zoom(self.zoom / 1.1),
            cmd::ZOOM_RESET => self.set_zoom(1.0),
            cmd::LINE_NUMBERS => self.toggle_setting("line-numbers", true),
            cmd::WRAP => self.toggle_setting("wrap", true),
            cmd::NEXT_TAB => self.step_tab(1),
            cmd::PREV_TAB => self.step_tab(-1),
            cmd::SETTINGS => self.open_settings(window),
            cmd::ABOUT => {
                let about = vergence_widgets::about::About::new(APP_ID, "Text Editor", env!("CARGO_PKG_VERSION"), "A plain-text editor: tabs, line numbers, wrapping, undo; files saved whole or not at all.", self.theme());
                window.open_window(vergence_widgets::window(about));
            }
            _ => {}
        }
    }
}

impl Program for EditWindow {
    type Message = Msg;

    fn title(&self) -> String {
        match self.front_tab() {
            Some(t) if t.modified() => format!("{} •", t.name()),
            Some(t) => t.name(),
            None => "Text Editor".into(),
        }
    }

    fn app_id(&self) -> String {
        APP_ID.into()
    }

    /// Light under App default, as a page of paper.
    fn theme(&self) -> Theme {
        Theme::from_desktop(&self.desktop, false, SECTION)
    }

    fn bar_style(&self) -> Option<BarStyle> {
        Some(vergence_ui::bar_style_setting(&self.desktop, SECTION, BarStyle::Light))
    }

    fn under_bar(&self) -> bool {
        true
    }

    fn bar(&self) -> Option<Bar> {
        let tabs = self.tabs.iter().map(|t| BarTab { id: t.id, title: t.name(), closable: true, alert: false, edited: t.modified() }).collect();
        Some(Bar::Tabs { tabs, front: self.front_tab().map_or(0, |t| t.id), plus: true, accent: None, menu: Vec::new() })
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let Some(tab) = self.front_tab() else { return space() };
        let mut parts: Vec<Element<Msg>> = Vec::new();
        let strip = |text: String, dismiss: bool| -> Element<Msg> {
            let mut items: Vec<Element<Msg>> = vec![label(text).size(12.0).wrap().width(Length::FILL)];
            if dismiss {
                items.push(button(label("OK").size(12.0)).padding(Padding::hv(2.0, 10.0)).on_press(Msg::DismissNotice).name("OK"));
            }
            panel(row(items).spacing(8.0).align(Align::Center).width(Length::FILL)).color(t.card).radius(0.0).padding(Padding::hv(6.0, 12.0)).width(Length::FILL)
        };
        if let Some(n) = &self.notice {
            parts.push(strip(n.clone(), true));
        }
        if let Some(r) = &tab.read_only {
            parts.push(strip(r.clone(), false));
        }
        parts.push(text_area(&tab.doc).key(tab.id as u64).autofocus().on_change(Msg::Changed).name(tab.name()));
        // The status line: where the caret is, how Tab indents, the line endings.
        let d = tab.doc.borrow();
        let (l, c) = d.line_column();
        let (a, b) = d.selection();
        let mut status: Vec<Element<Msg>> = vec![space().width(Length::FILL)];
        // Words (Dave, 2026-11-15: "a word counter in there somewhere").
        let words = grouped(d.word_count());
        status.push(label(if a != b { format!("{} of {words} words", grouped(d.selected_words())) } else { format!("{words} words") }).size(12.0).dim());
        if a != b {
            let n = d.selected_text().chars().count();
            status.push(label(format!("{} characters selected", grouped(n))).size(12.0).dim());
        }
        status.push(label(format!("Line {l}, Column {c}")).size(12.0).dim());
        let o = d.options();
        let indent = if o.spaces { format!("Spaces: {}", o.tab_width) } else { format!("Tab width: {}", o.tab_width) };
        let ending = if d.buffer().ending() == Ending::CrLf { "CRLF" } else { "LF" };
        let small = |text: String, m: Msg, name: &str| -> Element<Msg> { button(label(text).size(12.0).dim()).look(ButtonLook::Plain).padding(Padding::hv(1.0, 6.0)).on_press(m).name(name) };
        status.push(small(indent, Msg::ToggleSpaces, "Indent with spaces or tabs"));
        status.push(small(ending.into(), Msg::ToggleEnding, "Line endings"));
        status.push(label("UTF-8").size(12.0).dim());
        parts.push(panel(row(status).spacing(14.0).align(Align::Center).width(Length::FILL)).color(t.page).radius(0.0).padding(Padding { top: 3.0, bottom: 3.0, left: 12.0, right: 12.0 }).width(Length::FILL));
        // Below the server's bar (padding is a panel's, not a column's).
        panel(column(parts).width(Length::FILL).height(Length::FILL)).padding(Padding { top: t.top, ..Padding::default() }).radius(0.0).width(Length::FILL).height(Length::FILL)
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        match m {
            Msg::Changed => {}
            Msg::DismissNotice => self.notice = None,
            Msg::ToggleSpaces => {
                let i = self.front_index();
                if let Some(t) = self.tabs.get_mut(i) {
                    let now = t.doc.borrow().options().spaces;
                    t.spaces = Some(!now);
                }
                self.apply_options();
            }
            Msg::ToggleEnding => {
                if let Some(d) = self.doc() {
                    let mut d = d.borrow_mut();
                    let e = if d.buffer().ending() == Ending::Lf { Ending::CrLf } else { Ending::Lf };
                    d.set_ending(e);
                }
            }
        }
        window.redraw();
    }

    /// Keys the text area left: the app's own.
    fn input(&mut self, input: &Input, window: &mut Window) -> bool {
        const PAGE_UP: u32 = 0xff55;
        const PAGE_DOWN: u32 = 0xff56;
        let Input::Key { sym, pressed: true, modifiers: m, .. } = input else { return false };
        if !(m.ctrl || m.logo) || m.alt {
            return false;
        }
        let letter = char::from_u32(*sym).map(|c| c.to_ascii_lowercase());
        let id = match (letter, *sym) {
            (Some('s'), _) if m.shift => cmd::SAVE_AS,
            (Some('s'), _) => cmd::SAVE,
            (Some('n'), _) => cmd::NEW_WINDOW,
            (Some('t'), _) => cmd::NEW_TAB,
            (Some('w'), _) => cmd::CLOSE_TAB,
            (Some('=' | '+'), _) => cmd::ZOOM_IN,
            (Some('-'), _) => cmd::ZOOM_OUT,
            (Some('0'), _) => cmd::ZOOM_RESET,
            (Some(','), _) => cmd::SETTINGS,
            (_, PAGE_DOWN) => cmd::NEXT_TAB,
            (_, PAGE_UP) => cmd::PREV_TAB,
            _ => return false,
        };
        self.run(id, window);
        true
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        let (undo, redo, selected, editable) = match self.front_tab() {
            Some(t) => {
                let d = t.doc.borrow();
                let (a, b) = d.selection();
                (d.can_undo(), d.can_redo(), a != b, t.read_only.is_none())
            }
            None => (false, false, false, false),
        };
        Some((
            false,
            vec![
                C::role(cmd::NEW_TAB, Role::NewTab).featured(),
                C::role(cmd::NEW_WINDOW, Role::NewWindow),
                C::role(cmd::SAVE, Role::Save).enabled(editable).featured(),
                C::role(cmd::SAVE_AS, Role::SaveAs),
                C::role(cmd::CLOSE_TAB, Role::CloseTab),
                C::role(cmd::UNDO, Role::Undo).enabled(undo && editable),
                C::role(cmd::REDO, Role::Redo).enabled(redo && editable),
                C::role(cmd::CUT, Role::Cut).enabled(selected && editable),
                C::role(cmd::COPY, Role::Copy).enabled(selected),
                C::role(cmd::PASTE, Role::Paste).enabled(editable),
                C::role(cmd::SELECT_ALL, Role::SelectAll),
                C::role(cmd::ZOOM_IN, Role::ZoomIn),
                C::role(cmd::ZOOM_OUT, Role::ZoomOut),
                C::role(cmd::ZOOM_RESET, Role::ZoomReset),
                C::own(cmd::LINE_NUMBERS, "View", "Line numbers", "").checked(self.flag("line-numbers", true)).featured(),
                C::own(cmd::WRAP, "View", "Wrap lines", "").checked(self.flag("wrap", true)),
                C::role(cmd::NEXT_TAB, Role::NextTab),
                C::role(cmd::PREV_TAB, Role::PreviousTab),
                C::role(cmd::SETTINGS, Role::Settings),
                C::role(cmd::ABOUT, Role::About),
            ],
        ))
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        self.run(id, window);
        window.redraw();
    }

    fn tab(&mut self, event: TabEvent, window: &mut Window) {
        match event {
            TabEvent::Chosen(id) => self.front = id,
            TabEvent::Close(id) => self.close_tab(id, window),
            TabEvent::CloseOthers(id) => {
                for t in self.tabs.iter().map(|t| t.id).filter(|t| *t != id).collect::<Vec<_>>() {
                    if !self.closing.contains(&t) {
                        self.closing.push(t);
                    }
                }
                self.go_on_closing(window);
            }
            TabEvent::New => self.run(cmd::NEW_TAB, window),
            TabEvent::Moved { tab, to } => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == tab) {
                    let t = self.tabs.remove(i);
                    self.tabs.insert((to as usize).min(self.tabs.len()), t);
                }
            }
            TabEvent::Out { tab, .. } => {
                if self.tabs.len() > 1
                    && let Some(t) = self.remove_tab(tab)
                {
                    let mut w = EditWindow::new(self.shared.clone(), vec![t]);
                    w.zoom = self.zoom;
                    w.apply_options();
                    window.open_window(vergence_widgets::window(w));
                }
            }
            TabEvent::Join { tab, into, to } => {
                if let Some(t) = self.remove_tab(tab) {
                    let waker = {
                        let mut s = self.shared.borrow_mut();
                        s.transfers.push((into, to, t));
                        s.windows.iter().find(|(id, _)| *id == into).map(|(_, w)| w.clone())
                    };
                    if let Some(w) = waker {
                        w.wake();
                    }
                    if self.tabs.is_empty() {
                        window.close();
                    }
                }
            }
            TabEvent::Menu { .. } => {}
        }
    }

    fn close_asked(&mut self, window: &mut Window) -> bool {
        if !self.tabs.iter().any(DocTab::modified) {
            return true;
        }
        self.closing = self.tabs.iter().map(|t| t.id).collect();
        self.closing_window = true;
        self.go_on_closing(window);
        false
    }

    fn opened(&mut self, id: u32) {
        self.me = Some(id);
        self.register();
    }

    fn closed(&mut self) {
        let mut s = self.shared.borrow_mut();
        s.windows.retain(|(id, _)| Some(*id) != self.me);
        // Hand-overs go to another window now.
        let next = s.windows.first().map(|(_, w)| w.clone());
        *s.listener.lock().unwrap_or_else(|e| e.into_inner()) = next.clone();
        if let Some(w) = next {
            w.wake();
        }
    }

    fn waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
        self.register();
    }

    fn woken(&mut self, window: &mut Window) -> bool {
        let mut changed = self.reread();
        // A question answered.
        if let Some(a) = self.asking.as_ref().and_then(|q| q.answer.take()) {
            self.answered(a, window);
            changed = true;
        } else if self.asking.as_ref().is_some_and(|q| Rc::strong_count(&q.answer) == 1) {
            // The question was closed without an answer: as Cancel.
            self.answered(Answer::Cancel, window);
            changed = true;
        }
        // Tabs joined to this window.
        let mine: Vec<(u32, DocTab)> = {
            let mut s = self.shared.borrow_mut();
            let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut s.transfers).into_iter().partition(|(into, _, _)| Some(*into) == self.me);
            s.transfers = rest;
            mine.into_iter().map(|(_, to, t)| (to, t)).collect()
        };
        for (to, t) in mine {
            self.front = t.id;
            self.tabs.insert((to as usize).min(self.tabs.len()), t);
            changed = true;
        }
        if changed {
            self.apply_options();
        }
        // Files other starts handed over, if this window takes them.
        let listening = {
            let s = self.shared.borrow();
            s.windows.first().map(|(id, _)| Some(*id) == self.me).unwrap_or(false)
        };
        if listening {
            let requests: Vec<Vec<PathBuf>> = std::mem::take(&mut *self.shared.borrow().inbox.lock().unwrap_or_else(|e| e.into_inner()));
            for files in requests {
                if files.is_empty() {
                    window.open_window(vergence_widgets::window(EditWindow::new(self.shared.clone(), Vec::new())));
                } else {
                    self.open_files(&files);
                    changed = true;
                }
            }
        }
        changed
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { min: (360.0, 240.0), ..Default::default() }
    }
}

/// 1234567 as "1,234,567".
fn grouped(n: usize) -> String {
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
        let b = |text: &str, a: Answer, look: ButtonLook| -> Element<Answer> { button(label(text).size(12.0)).look(look).padding(Padding::hv(4.0, 12.0)).on_press(a).name(text) };
        panel(
            column(vec![
                label(format!("Save the changes to “{}”?", self.name)).size(15.0).weight(600).wrap().width(Length::FILL),
                label("If you don't save them, they are lost.").dim().wrap().width(Length::FILL),
                row(vec![b("Don't save", Answer::Discard, ButtonLook::Normal), space().width(Length::FILL), b("Cancel", Answer::Cancel, ButtonLook::Normal), b("Save", Answer::Save, ButtonLook::Accent)]).spacing(8.0).align(Align::Center).width(Length::FILL),
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
        vergence_ui::Style { kind: vergence_ui::WinKind::Dialog, modal: true, min: (420.0, 150.0), max: (0.0, 0.0) }
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

/// Its settings window's rows.
struct EditRows {
    /// Monospaced families first, then the rest (fc-list, once).
    fonts: Vec<String>,
}

impl EditRows {
    fn new() -> EditRows {
        let list = |args: &[&str]| -> Vec<String> {
            let out = std::process::Command::new("fc-list").args(args).output();
            let mut v: Vec<String> = out.ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default().lines().filter_map(|l| l.split(',').next().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect();
            v.sort_by_key(|n| n.to_lowercase());
            v.dedup();
            v
        };
        let mut fonts = list(&[":spacing=mono", "family"]);
        let rest: Vec<String> = list(&[":", "family"]).into_iter().filter(|f| !fonts.contains(f)).collect();
        fonts.extend(rest);
        EditRows { fonts }
    }
}

impl Rows for EditRows {
    fn parts(&self, text: &str) -> Vec<Part> {
        let fonts: Vec<Choice> = self.fonts.iter().map(|f| Choice::text(f, f)).collect();
        let sizes: Vec<Choice> = [8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28].iter().map(|s| Choice::number(&s.to_string(), *s)).collect();
        let tabs: Vec<Choice> = [2, 3, 4, 8].iter().map(|s| Choice::number(&s.to_string(), *s)).collect();
        let opacity = vergence_ui::opacity_setting(text, "[appearance]");
        let locked = vergence_ui::desktop_pick(text).map(|light| format!("{} for every app: Settings > Appearance > Style", if light { "Light" } else { "Dark" }));
        let styles = vec![Choice::text("Dark", "dark"), Choice::text("Light", "light")];
        vec![
            Part {
                title: "Text".into(),
                rows: vec![
                    Row::new("font", "Font", "", Control::Choose { choices: fonts, desktop: Some("Monospace".into()), app_default: 0 }),
                    Row::new("text-size", "Text size", "Ctrl+= and Ctrl+− change it for a while.", Control::Choose { choices: sizes, desktop: Some("Desktop default".into()), app_default: 0 }),
                    Row::new("line-numbers", "Line numbers", "", Control::Switch { default: true }),
                    Row::new("wrap", "Wrap lines", "Long lines go on below at the window's edge; off, the text scrolls sideways.", Control::Switch { default: true }),
                    Row::new("current-line", "Shade the current line", "", Control::Switch { default: true }),
                ],
            },
            Part {
                title: "Indenting".into(),
                rows: vec![
                    Row::new("tab-width", "Tab width", "How many spaces wide a tab is.", Control::Choose { choices: tabs, desktop: Some("4".into()), app_default: 0 }),
                    Row::new("spaces", "Indent with spaces", "Tab inserts spaces; a file indented with tabs keeps its tabs.", Control::Switch { default: true }),
                ],
            },
            Part {
                title: "On this desktop".into(),
                rows: vec![
                    Row::new("style", "Style", "While the desktop's Style is App default.", Control::Choose { choices: styles.clone(), desktop: Some("Built-in (Light)".into()), app_default: 0 }).locked(locked.clone()),
                    Row::new("bar-style", "Title bar", "", Control::Choose { choices: styles, desktop: Some("Built-in (Light)".into()), app_default: 0 }).locked(locked),
                    Row::new("opacity", "Opacity", "30 to 100: how much of the window's ground shows; the text stays as it is.", Control::Percent { min: 30, desktop: format!("Desktop default ({:.0})", opacity * 100.0), now: (opacity * 100.0).round() as u32 }),
                    Row::new("", "Window places", "Where the desktop puts the editor's windows, for each set of screens.", Control::Button { label: "Forget…".into(), id: 1 }),
                ],
            },
        ]
    }

    fn button(&self, id: u32) {
        if id == 1 {
            let _ = std::process::Command::new("vergence-settings").args(["--page", "windows", "--forget", APP_ID]).spawn();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("vergence-edit {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let paths: Vec<PathBuf> = args.iter().filter(|a| !a.starts_with("--")).map(|a| cwd.join(a)).collect();
    // Another editor running takes the files.
    if hand::hand_over(&paths) {
        return;
    }
    let inbox = Arc::new(Mutex::new(Vec::new()));
    let listener: Arc<Mutex<Option<Waker>>> = Arc::new(Mutex::new(None));
    let wake = listener.clone();
    hand::listen(inbox.clone(), move || {
        if let Some(w) = wake.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            w.wake();
        }
    });
    let shared = Rc::new(RefCell::new(Shared { windows: Vec::new(), transfers: Vec::new(), inbox, listener, next_tab: 0 }));
    let mut w = EditWindow::new(shared, Vec::new());
    w.open_files(&paths);
    if let Err(e) = vergence_widgets::run(w, (760, 560)) {
        eprintln!("vergence-edit: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> SharedRef {
        Rc::new(RefCell::new(Shared { windows: Vec::new(), transfers: Vec::new(), inbox: Arc::new(Mutex::new(Vec::new())), listener: Arc::new(Mutex::new(None)), next_tab: 0 }))
    }

    #[test]
    fn untitled_documents_are_numbered_and_a_file_replaces_an_empty_one() {
        let mut w = EditWindow::new(shared(), Vec::new());
        assert_eq!(w.tabs.len(), 1);
        assert_eq!(w.tabs[0].name(), "Untitled");
        let t = w.untitled_tab();
        assert_eq!(t.name(), "Untitled 2");
        w.tabs.push(t);
        w.tabs.remove(0);
        assert_eq!(w.untitled_tab().name(), "Untitled", "a number given back is used again");
        w.tabs.clear();
        let fresh = w.untitled_tab();
        w.tabs.push(fresh);
        let dir = std::env::temp_dir().join(format!("vergence-edit-main-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("tabbed.txt");
        std::fs::write(&p, "a\n\tb\n\tc\n").unwrap();
        w.open_files(std::slice::from_ref(&p));
        assert_eq!(w.tabs.len(), 1, "the empty Untitled gave way");
        assert_eq!(w.tabs[0].name(), "tabbed.txt");
        assert!(!w.tabs[0].doc.borrow().options().spaces, "a file indented with tabs keeps tabs");
        assert!(!w.tabs[0].modified());
        // Opening it again brings the same tab forward.
        w.open_files(std::slice::from_ref(&p));
        assert_eq!(w.tabs.len(), 1);
        w.tabs[0].doc.borrow_mut().insert("x");
        assert!(w.tabs[0].modified());
        assert!(w.save(w.tabs[0].id));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "xa\n\tb\n\tc\n");
        assert!(!w.tabs[0].modified());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn closing_removes_tabs_and_moves_the_front() {
        let mut w = EditWindow::new(shared(), Vec::new());
        let (a, b) = (w.untitled_tab(), w.untitled_tab());
        let (ida, idb) = (a.id, b.id);
        w.tabs.push(a);
        w.tabs.push(b);
        w.front = ida;
        w.remove_tab(ida);
        assert_eq!(w.front, idb, "the next tab comes to the front");
        w.step_tab(1);
        assert_eq!(w.front, w.tabs[0].id, "and round");
    }
}
