//! The Open and Save dialog (editor milestone 1, step 4; the first part of
//! docs/apps-roadmap.md item 5), the same in every native app: a modal
//! dialog over the app's window.
//!
//! - **Places** on the left: Home and its usual folders, the whole
//!   computer, disks mounted under /media, /mnt or /run/media, and Recent
//!   (the files last opened or saved through the dialog, in any app).
//! - **The folder**: its path as buttons to go back up it, Back and Up,
//!   the files in it with their size and when they changed, folders first,
//!   sorted by any heading (names in natural order: "file 2" before
//!   "file 10"). Hidden files on Ctrl+H.
//! - **Typing** in Open filters the list as you type; a path ("/", "~")
//!   goes there with Enter. In Save the name is typed, and a file of that
//!   name already there asks before it is replaced. New folder in Save.
//! - **A look before opening**: a text file chosen shows its first lines
//!   at the side, and a folder how much is in it.
//! - **Keys**: Up and Down choose, Enter opens or saves, Backspace in an
//!   empty Open field goes up, Alt+Up up, Alt+Left back, Escape cancels.
//! - **Remembered** (~/.local/state/vergence/file-dialog, owner only): each
//!   app's last folder, and the recent files.

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use vergence_ui::{Input, Window};

use crate::{Align, ButtonLook, Element, Length, Padding, Program, Theme, button, column, field, label, panel, row, rule, scroll, space};

/// The most rows listed (a folder of more is narrowed by typing).
const MAX_ROWS: usize = 2000;
const PREVIEW_LINES: usize = 40;
const DOUBLE: Duration = Duration::from_millis(450);

/// Which files a dialog shows: a name ("Text files"), patterns ("*.txt")
/// and file types ("text/plain", which takes in every kind of text: a
/// script, TOML, Markdown; files milestone 1). A file is shown if a pattern
/// or a type admits it; a filter with neither shows everything.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    pub name: String,
    pub patterns: Vec<String>,
    pub types: Vec<String>,
}

impl Filter {
    pub fn new(name: &str, patterns: &[&str]) -> Filter {
        Filter { name: name.into(), patterns: patterns.iter().map(|p| p.to_string()).collect(), types: Vec::new() }
    }

    /// A filter by file types (vergence-types names; `image/*` for every picture).
    pub fn of_types(name: &str, types: &[&str]) -> Filter {
        Filter { name: name.into(), patterns: Vec::new(), types: types.iter().map(|t| t.to_string()).collect() }
    }

    /// Whether a file of this name (and type, by its name) is shown.
    fn admits(&self, name: &str, mime: Option<&str>) -> bool {
        (self.patterns.is_empty() && self.types.is_empty())
            || self.patterns.iter().any(|p| glob(p, name))
            || mime.is_some_and(|m| {
                self.types.iter().any(|t| match t.strip_suffix("/*") {
                    Some(top) => m.split('/').next() == Some(top),
                    None => vergence_types::is_a(m, t),
                })
            })
    }
}

/// `*` and `?` patterns, without case.
fn glob(p: &str, s: &str) -> bool {
    fn go(p: &[char], s: &[char]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], s) || (!s.is_empty() && go(p, &s[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &s[1..]),
            (Some(a), Some(b)) => a.to_lowercase().eq(b.to_lowercase()) && go(&p[1..], &s[1..]),
            _ => false,
        }
    }
    let (p, s): (Vec<char>, Vec<char>) = (p.chars().collect(), s.chars().collect());
    go(&p, &s)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Open,
    Save,
    /// Choose a folder (the portal's `directory`, files milestone 1): only
    /// folders are listed; the one chosen, or the one shown, is the answer.
    Folder,
}

pub use crate::files::{Kind, SortBy, kind_icon};
use crate::files::{Entry, expand, home, looks_like_path, plain, remember, remembered_folder, size_words, text_start, when};

#[derive(Clone, Debug)]
pub enum FileMsg {
    Typed(String),
    Submit,
    Click(usize),
    Go(PathBuf),
    Recent,
    Back,
    Up,
    Sort(SortBy),
    Filter(usize),
    Hidden,
    NewFolder,
    FolderName(String),
    MakeFolder,
    Replace,
    NotReplace,
    Cancel,
}

/// What the dialog shows in its list: a folder, or the recent files.
#[derive(Clone, Debug, PartialEq)]
enum Showing {
    Folder(PathBuf),
    Recent,
}

/// The dialog. `done` hears the chosen files, or nothing if it was cancelled.
pub struct FileDialog {
    mode: Mode,
    app_id: String,
    title: String,
    theme: Theme,
    showing: Showing,
    entries: Vec<Entry>,
    problem: Option<String>,
    back: Vec<Showing>,
    sort: (SortBy, bool),
    hidden: bool,
    typed: String,
    selected: Option<usize>,
    last_click: Option<(Instant, usize)>,
    filters: Vec<Filter>,
    filter: usize,
    replace: Option<PathBuf>,
    new_folder: Option<String>,
    done: Option<Box<dyn FnOnce(Vec<PathBuf>)>>,
    /// Several files may be chosen (Ctrl+click adds or takes one away,
    /// Shift+click a run of them): those marked so far.
    multiple: bool,
    marked: Vec<PathBuf>,
    /// The modifiers of the last press, for a row's click.
    mods: vergence_ui::Modifiers,
    /// The Open/Save button's word, if not "Open"/"Save"/"Choose".
    accept: Option<String>,
    /// Bumped when the choice moves, so the chosen row is a new one and is scrolled to.
    turn: Rc<Cell<u64>>,
}

impl FileDialog {
    /// An Open dialog for `app_id`, titled `title` ("Open").
    pub fn open(app_id: &str, title: &str, theme: Theme, done: impl FnOnce(Vec<PathBuf>) + 'static) -> FileDialog {
        FileDialog::new(Mode::Open, app_id, title, String::new(), theme, Box::new(done))
    }

    /// A Save dialog, the name `name` suggested.
    pub fn save(app_id: &str, title: &str, name: &str, theme: Theme, done: impl FnOnce(Vec<PathBuf>) + 'static) -> FileDialog {
        FileDialog::new(Mode::Save, app_id, title, name.to_string(), theme, Box::new(done))
    }

    fn new(mode: Mode, app_id: &str, title: &str, typed: String, theme: Theme, done: Box<dyn FnOnce(Vec<PathBuf>)>) -> FileDialog {
        let start = remembered_folder(app_id).filter(|p| p.is_dir()).or_else(home).unwrap_or_else(|| PathBuf::from("/"));
        let mut d = FileDialog {
            mode,
            app_id: app_id.into(),
            title: title.into(),
            theme,
            showing: Showing::Folder(start),
            entries: Vec::new(),
            problem: None,
            back: Vec::new(),
            sort: (SortBy::Name, true),
            hidden: false,
            typed,
            selected: None,
            last_click: None,
            filters: Vec::new(),
            filter: 0,
            replace: None,
            new_folder: None,
            done: Some(done),
            multiple: false,
            marked: Vec::new(),
            mods: vergence_ui::Modifiers::default(),
            accept: None,
            turn: Rc::new(Cell::new(0)),
        };
        d.read();
        d
    }

    /// A dialog that chooses a folder.
    pub fn choose_folder(app_id: &str, title: &str, theme: Theme, done: impl FnOnce(Vec<PathBuf>) + 'static) -> FileDialog {
        FileDialog::new(Mode::Folder, app_id, title, String::new(), theme, Box::new(done))
    }

    /// In Open, several files may be chosen (Ctrl+click, Shift+click).
    pub fn multiple(mut self, yes: bool) -> FileDialog {
        self.multiple = yes && self.mode == Mode::Open;
        self
    }

    /// The word on the button that answers ("Attach", "Upload").
    pub fn accept(mut self, word: &str) -> FileDialog {
        let w = word.trim().replace('_', "");
        self.accept = (!w.is_empty()).then(|| w.chars().take(24).collect());
        self
    }

    /// Opens in `dir` (the document's own folder, say) rather than the last one.
    pub fn folder(mut self, dir: &Path) -> FileDialog {
        if dir.is_dir() {
            self.showing = Showing::Folder(dir.to_path_buf());
            self.read();
        }
        self
    }

    /// The kinds of file it offers to show; the first is chosen.
    pub fn filters(mut self, filters: Vec<Filter>) -> FileDialog {
        self.filters = filters;
        self.filter = 0;
        self
    }

    fn dir(&self) -> Option<&Path> {
        match &self.showing {
            Showing::Folder(p) => Some(p),
            Showing::Recent => None,
        }
    }

    /// Reads what it shows again (files.rs).
    fn read(&mut self) {
        self.problem = None;
        self.selected = None;
        self.entries = match &self.showing {
            Showing::Folder(dir) => crate::files::read_folder(dir).unwrap_or_else(|why| {
                self.problem = Some(why);
                Vec::new()
            }),
            Showing::Recent => crate::files::recent(),
        };
    }

    /// What the list shows, in order: hidden files and the filter's left
    /// out, the typed text narrowing it in Open.
    fn visible(&self) -> Vec<&Entry> {
        let narrow = if self.mode == Mode::Open && !looks_like_path(&self.typed) { self.typed.to_lowercase() } else { String::new() };
        let filter = self.filters.get(self.filter);
        let mut v: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| self.hidden || !e.name.starts_with('.'))
            .filter(|e| e.dir || (self.mode != Mode::Folder && filter.is_none_or(|f| f.admits(&e.name, e.mime))))
            .filter(|e| narrow.is_empty() || e.name.to_lowercase().contains(&narrow))
            .collect();
        // Folders first (the recent files keep their order unless sorted).
        crate::files::sort(&mut v, self.sort.0, self.sort.1, self.showing == Showing::Recent, crate::files::GroupBy::None);
        v.truncate(MAX_ROWS);
        v
    }

    fn go(&mut self, to: Showing) {
        if to != self.showing {
            self.back.push(std::mem::replace(&mut self.showing, to));
        }
        if self.mode == Mode::Open {
            self.typed.clear();
        }
        self.marked.clear();
        self.replace = None;
        self.read();
    }

    fn choose_row(&mut self, k: usize) {
        self.selected = Some(k);
        self.turn.set(self.turn.get() + 1);
        // In Save, a file chosen gives its name.
        if self.mode == Mode::Save
            && let Some(e) = self.visible().get(k)
            && !e.dir
        {
            self.typed = e.name.clone();
        }
    }

    /// Enter, or a double click on row `k`, or the Open/Save button.
    fn submit(&mut self, window: &mut Window) {
        let typed = self.typed.trim().to_string();
        // A path typed: a folder goes there; a file is the answer.
        if looks_like_path(&typed) {
            let p = expand(&typed, self.dir());
            if p.is_dir() {
                self.go(Showing::Folder(p));
                return;
            }
            if self.mode == Mode::Open && p.is_file() || self.mode == Mode::Save && p.parent().is_some_and(Path::is_dir) {
                return self.answer(p, window);
            }
            self.problem = Some(format!("{typed} does not exist."));
            return;
        }
        let chosen = self.selected.and_then(|k| self.visible().get(k).map(|e| (e.path.clone(), e.dir)));
        match self.mode {
            Mode::Folder => {
                // The folder chosen in the list, else the one shown.
                let p = chosen.filter(|(_, dir)| *dir).map(|(p, _)| p).or_else(|| self.dir().map(Path::to_path_buf));
                if let Some(p) = p {
                    if let Some(parent) = p.parent() {
                        remember(&self.app_id, parent, &p);
                    }
                    if let Some(done) = self.done.take() {
                        done(vec![p]);
                    }
                    window.close();
                }
            }
            Mode::Open if self.marked.len() > 1 => {
                let files: Vec<PathBuf> = self.marked.iter().filter(|p| p.is_file()).cloned().collect();
                if !files.is_empty() {
                    self.answer_all(files, window);
                }
            }
            Mode::Open => {
                let chosen = chosen.or_else(|| {
                    let v = self.visible();
                    (v.len() == 1).then(|| (v[0].path.clone(), v[0].dir))
                });
                match chosen {
                    Some((p, true)) => self.go(Showing::Folder(p)),
                    Some((p, false)) => self.answer(p, window),
                    None => {}
                }
            }
            Mode::Save => {
                if let Some((p, true)) = chosen.clone().filter(|_| typed.is_empty() || self.selected_name() != Some(typed.clone())) {
                    return self.go(Showing::Folder(p));
                }
                if typed.is_empty() {
                    return;
                }
                if typed.contains('/') {
                    self.problem = Some("A name cannot have / in it.".into());
                    return;
                }
                let Some(dir) = self.dir().map(Path::to_path_buf) else {
                    self.problem = Some("Choose a folder to save in.".into());
                    return;
                };
                let p = dir.join(&typed);
                if p.is_dir() {
                    return self.go(Showing::Folder(p));
                }
                self.answer(p, window);
            }
        }
    }

    fn selected_name(&self) -> Option<String> {
        self.selected.and_then(|k| self.visible().get(k).map(|e| e.name.clone()))
    }

    /// The chosen file: asked first if Save would replace one.
    /// Several files at once (Open with `multiple`).
    fn answer_all(&mut self, files: Vec<PathBuf>, window: &mut Window) {
        if let Some(last) = files.last()
            && let Some(dir) = last.parent()
        {
            remember(&self.app_id, dir, last);
        }
        if let Some(done) = self.done.take() {
            done(files);
        }
        window.close();
    }

    fn answer(&mut self, p: PathBuf, window: &mut Window) {
        if self.mode == Mode::Save && p.exists() && self.replace.as_ref() != Some(&p) {
            self.replace = Some(p);
            return;
        }
        if let Some(dir) = p.parent() {
            remember(&self.app_id, dir, &p);
        }
        if let Some(done) = self.done.take() {
            done(vec![p]);
        }
        window.close();
    }

    fn make_folder(&mut self) {
        let (Some(name), Some(dir)) = (self.new_folder.take(), self.dir().map(Path::to_path_buf)) else { return };
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if name.contains('/') {
            self.problem = Some("A name cannot have / in it.".into());
            return;
        }
        match fs::create_dir(dir.join(name)) {
            Ok(()) => self.go(Showing::Folder(dir.join(name))),
            Err(e) => self.problem = Some(format!("The folder could not be made: {}", plain(&e))),
        }
    }

    fn move_choice(&mut self, by: isize) {
        let n = self.visible().len() as isize;
        if n == 0 {
            return;
        }
        let k = match self.selected {
            None if by > 0 => 0,
            None => n - 1,
            Some(k) => (k as isize + by).clamp(0, n - 1),
        };
        self.choose_row(k as usize);
    }

    fn places(&self) -> Vec<crate::files::Place> {
        crate::files::places()
    }

    /// The folder's path as buttons, from Home if it is under it.
    fn crumbs(&self) -> Vec<(String, PathBuf)> {
        let Some(dir) = self.dir() else { return Vec::new() };
        let home = home();
        let (mut at, rest, first) = match home.as_ref().and_then(|h| dir.strip_prefix(h).ok().map(|r| (h.clone(), r.to_path_buf()))) {
            Some((h, r)) => (h, r, "Home".to_string()),
            None => (PathBuf::from("/"), dir.strip_prefix("/").map(Path::to_path_buf).unwrap_or_default(), "Computer".to_string()),
        };
        let mut out = vec![(first, at.clone())];
        for c in rest.components() {
            at = at.join(c);
            out.push((c.as_os_str().to_string_lossy().into_owned(), at.clone()));
        }
        out
    }

    fn preview(&self, e: &Entry, t: &Theme) -> Element<FileMsg> {
        let mut lines: Vec<Element<FileMsg>> = vec![label(e.name.clone()).size(13.0).weight(600).wrap().width(Length::FILL)];
        if e.dir {
            let n = fs::read_dir(&e.path).map(|r| r.flatten().filter(|x| self.hidden || !x.file_name().to_string_lossy().starts_with('.')).count()).unwrap_or(0);
            lines.push(label(format!("Folder · {n} item{}", if n == 1 { "" } else { "s" })).size(11.0).dim());
        } else {
            lines.push(label(format!("{} · {}", size_words(e.size), e.changed.map(when).unwrap_or_default())).size(11.0).dim());
            if let Some(text) = text_start(&e.path) {
                lines.push(space().height(Length::Fixed(6.0)));
                for l in text.lines().take(PREVIEW_LINES) {
                    let l: String = l.replace('\t', "    ").chars().take(80).collect();
                    lines.push(label(if l.is_empty() { " ".into() } else { l }).size(10.0).color(t.dim));
                }
            }
        }
        panel(scroll(column(lines).spacing(2.0).width(Length::FILL))).color(t.card).radius(8.0).padding(10.0).width(Length::Fixed(230.0)).height(Length::FILL)
    }
}

impl Program for FileDialog {
    type Message = FileMsg;

    fn title(&self) -> String {
        self.title.clone()
    }

    fn app_id(&self) -> String {
        self.app_id.clone()
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    fn view(&self, t: &Theme) -> Element<FileMsg> {
        let small = |text: &str, m: FileMsg| -> Element<FileMsg> { button(label(text).size(12.0)).look(ButtonLook::Plain).padding(Padding::hv(3.0, 8.0)).on_press(m).name(text) };
        // The path: Back, Up, then each folder up to this one.
        let mut path: Vec<Element<FileMsg>> = vec![
            button(label("‹").size(16.0)).look(ButtonLook::Plain).padding(Padding::hv(0.0, 8.0)).on_press_if(!self.back.is_empty(), FileMsg::Back).name("Back"),
            button(label("↑").size(14.0)).look(ButtonLook::Plain).padding(Padding::hv(0.0, 8.0)).on_press_if(self.dir().is_some_and(|d| d.parent().is_some()), FileMsg::Up).name("Up"),
        ];
        match &self.showing {
            Showing::Recent => path.push(label("Recent").size(13.0).weight(600)),
            Showing::Folder(_) => {
                let crumbs = self.crumbs();
                let n = crumbs.len();
                for (i, (name, p)) in crumbs.into_iter().enumerate() {
                    if i > 0 {
                        path.push(label("›").size(12.0).dim());
                    }
                    let last = i + 1 == n;
                    path.push(button(label(name.clone()).size(12.0).weight(if last { 600 } else { 400 })).look(ButtonLook::Plain).padding(Padding::hv(3.0, 6.0)).on_press(FileMsg::Go(p)).name(name));
                }
            }
        }
        path.push(space().width(Length::FILL));
        path.push(small(if self.hidden { "Hide hidden files" } else { "Show hidden files" }, FileMsg::Hidden));
        let path = row(path).spacing(2.0).align(Align::Center).width(Length::FILL);

        // The places.
        let places = self.places();
        let sidebar = crate::files::sidebar(&places, self.dir(), |p| p.path.clone().map_or(FileMsg::Recent, FileMsg::Go), t);

        // The list: headings, then the rows.
        let recent = self.showing == Showing::Recent;
        let visible = self.visible();
        let chosen = |k: usize, e: &Entry| self.selected == Some(k) || self.marked.contains(&e.path);
        let click = |k: usize| FileMsg::Click(k);
        let view = crate::files::View { entries: &visible, chosen: &chosen, in_view: self.selected.map(|k| (k, self.turn.get() << 32 | k as u64)), click: &click, drag: None, folders: recent, thumb: None, menu: None, look: &crate::files::ListLook { rows: crate::files::RowSize::Small, kind: false, ..crate::files::ListLook::default() }, folder_size: None, grip: None, under: None, changed_heading: None, trailing: None, rename: None };
        let (head, rows) = crate::files::list(&view, self.sort, FileMsg::Sort);
        let empty = rows.is_empty();
        let mut list: Vec<Element<FileMsg>> = vec![head, rule()];
        if let Some(p) = &self.problem {
            list.push(label(p.clone()).size(12.0).wrap().width(Length::FILL));
        }
        if empty && self.problem.is_none() {
            let why = if !self.typed.is_empty() && self.mode == Mode::Open { "Nothing here has that in its name." } else if recent { "Nothing opened or saved yet." } else { "This folder is empty." };
            list.push(panel(label(why).size(12.0).dim()).padding(12.0));
        }
        list.push(scroll(column(rows).spacing(1.0).width(Length::FILL)));
        let list = panel(column(list).spacing(2.0).width(Length::FILL).height(Length::FILL)).color(t.page).radius(8.0).edge(t.line, 1.0).padding(4.0).width(Length::FILL).height(Length::FILL);
        let mut middle = vec![sidebar, list];
        if let Some(e) = self.selected.and_then(|k| visible.get(k)) {
            middle.push(self.preview(e, t));
        }
        let middle = row(middle).spacing(8.0).width(Length::FILL).height(Length::FILL);

        // The foot: the name or the filter, the kinds, the buttons.
        let mut foot: Vec<Element<FileMsg>> = Vec::new();
        match self.mode {
            Mode::Save => {
                foot.push(label("Name").size(12.0));
                foot.push(field(self.typed.clone(), FileMsg::Typed).on_submit(FileMsg::Submit).autofocus().width(Length::FILL).name("Name"));
            }
            Mode::Open | Mode::Folder => {
                let hint = if self.marked.len() > 1 {
                    format!("{} files chosen", self.marked.len())
                } else if self.mode == Mode::Folder {
                    "Type a path to go there".to_string()
                } else {
                    "Type to narrow the list, or a path".to_string()
                };
                foot.push(field(self.typed.clone(), FileMsg::Typed).placeholder(&hint).on_submit(FileMsg::Submit).autofocus().width(Length::FILL).name("Find or go to"))
            }
        }
        if self.filters.len() > 1 {
            let names: Vec<String> = self.filters.iter().map(|f| f.name.clone()).collect();
            foot.push(crate::choice(names, Some(self.filter), FileMsg::Filter).width(Length::Fixed(150.0)).name("Kind of file"));
        }
        if self.mode == Mode::Save && self.dir().is_some() {
            foot.push(button(label("New folder").size(12.0)).padding(Padding::hv(4.0, 10.0)).on_press(FileMsg::NewFolder).name("New folder"));
        }
        foot.push(button(label("Cancel").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(FileMsg::Cancel).name("Cancel"));
        let word = match self.mode {
            Mode::Save => "Save",
            Mode::Open => "Open",
            Mode::Folder => "Choose",
        };
        let go = self.accept.as_deref().unwrap_or(word);
        foot.push(button(label(go).size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(4.0, 14.0)).on_press(FileMsg::Submit).name(go));
        let mut all = vec![path, middle];
        // A question or a name, in a strip above the foot.
        if let Some(p) = &self.replace {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            all.push(
                panel(
                    row(vec![
                        label(format!("“{name}” is already here. Replace it?")).size(12.0).weight(600).width(Length::FILL),
                        button(label("Cancel").size(12.0)).padding(Padding::hv(3.0, 10.0)).on_press(FileMsg::NotReplace).name("Don't replace"),
                        button(label("Replace").size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(3.0, 10.0)).on_press(FileMsg::Replace).name("Replace"),
                    ])
                    .spacing(8.0)
                    .align(Align::Center)
                    .width(Length::FILL),
                )
                .color(t.card)
                .radius(8.0)
                .padding(Padding::hv(6.0, 10.0))
                .width(Length::FILL),
            );
        }
        if let Some(n) = &self.new_folder {
            all.push(
                panel(
                    row(vec![
                        label("New folder").size(12.0),
                        field(n.clone(), FileMsg::FolderName).on_submit(FileMsg::MakeFolder).autofocus().width(Length::FILL).name("Folder name"),
                        button(label("Make").size(12.0)).look(ButtonLook::Accent).padding(Padding::hv(3.0, 10.0)).on_press(FileMsg::MakeFolder).name("Make the folder"),
                    ])
                    .spacing(8.0)
                    .align(Align::Center)
                    .width(Length::FILL),
                )
                .color(t.card)
                .radius(8.0)
                .padding(Padding::hv(6.0, 10.0))
                .width(Length::FILL),
            );
        }
        all.push(row(foot).spacing(8.0).align(Align::Center).width(Length::FILL));
        panel(column(all).spacing(8.0).width(Length::FILL).height(Length::FILL)).padding(Padding { top: 10.0 + t.top, bottom: 12.0, left: 12.0, right: 12.0 }).radius(0.0).width(Length::FILL).height(Length::FILL)
    }

    fn update(&mut self, m: FileMsg, window: &mut Window) {
        match m {
            FileMsg::Typed(t) => {
                self.typed = t;
                self.replace = None;
                if self.mode == Mode::Open {
                    // The narrowed list's first row is the one Enter opens.
                    self.selected = (!self.typed.is_empty() && !looks_like_path(&self.typed) && !self.visible().is_empty()).then_some(0);
                }
            }
            FileMsg::Submit => self.submit(window),
            FileMsg::Click(k) if self.multiple && (self.mods.ctrl || self.mods.shift) => {
                // Ctrl+click: this file in or out of the marked ones; Shift+click:
                // the files from the last chosen to this one.
                let visible: Vec<(PathBuf, bool)> = self.visible().iter().map(|e| (e.path.clone(), e.dir)).collect();
                if self.marked.is_empty()
                    && let Some((p, false)) = self.selected.and_then(|j| visible.get(j).cloned())
                {
                    self.marked.push(p);
                }
                if self.mods.shift {
                    let from = self.selected.unwrap_or(k);
                    let (a, b) = (from.min(k), from.max(k));
                    for (p, dir) in &visible[a..=b.min(visible.len().saturating_sub(1))] {
                        if !dir && !self.marked.contains(p) {
                            self.marked.push(p.clone());
                        }
                    }
                } else if let Some((p, false)) = visible.get(k).cloned() {
                    match self.marked.iter().position(|m| *m == p) {
                        Some(i) => {
                            self.marked.remove(i);
                        }
                        None => self.marked.push(p),
                    }
                }
                self.selected = Some(k);
                self.turn.set(self.turn.get() + 1);
            }
            FileMsg::Click(k) => {
                self.marked.clear();
                let now = Instant::now();
                let double = self.last_click.is_some_and(|(t, j)| j == k && now.duration_since(t) < DOUBLE);
                self.last_click = Some((now, k));
                self.choose_row(k);
                if double {
                    // A double click opens what was clicked, whatever is typed.
                    if let Some((p, dir)) = self.visible().get(k).map(|e| (e.path.clone(), e.dir)) {
                        if dir {
                            self.go(Showing::Folder(p));
                        } else if self.mode == Mode::Open {
                            self.answer(p, window);
                        } else {
                            self.typed = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            self.submit(window);
                        }
                    }
                }
            }
            FileMsg::Go(p) => self.go(Showing::Folder(p)),
            FileMsg::Recent => self.go(Showing::Recent),
            FileMsg::Back => {
                if let Some(s) = self.back.pop() {
                    self.showing = s;
                    self.read();
                }
            }
            FileMsg::Up => {
                if let Some(p) = self.dir().and_then(Path::parent).map(Path::to_path_buf) {
                    self.go(Showing::Folder(p));
                }
            }
            FileMsg::Sort(by) => {
                self.sort = if self.sort.0 == by { (by, !self.sort.1) } else { (by, by == SortBy::Name) };
                self.selected = None;
            }
            FileMsg::Filter(k) => {
                self.filter = k;
                self.selected = None;
            }
            FileMsg::Hidden => {
                self.hidden = !self.hidden;
                self.selected = None;
            }
            FileMsg::NewFolder => self.new_folder = Some(String::new()),
            FileMsg::FolderName(n) => self.new_folder = Some(n),
            FileMsg::MakeFolder => self.make_folder(),
            FileMsg::Replace => {
                if let Some(p) = self.replace.clone() {
                    self.answer(p, window);
                }
            }
            FileMsg::NotReplace => self.replace = None,
            FileMsg::Cancel => window.close(),
        }
        window.redraw();
    }

    /// The modifiers of each press, for the row it clicks (Ctrl+click, Shift+click).
    fn saw(&mut self, input: &Input) {
        if let Input::Button { pressed: true, modifiers, .. } = input {
            self.mods = *modifiers;
        }
    }

    /// The list's keys, before the field: Up and Down choose, Escape cancels.
    fn key_first(&mut self, input: &Input, window: &mut Window) -> Option<bool> {
        const UP: u32 = 0xff52;
        const DOWN: u32 = 0xff54;
        const LEFT: u32 = 0xff51;
        const PAGE_UP: u32 = 0xff55;
        const PAGE_DOWN: u32 = 0xff56;
        const ESCAPE: u32 = 0xff1b;
        const BACKSPACE: u32 = 0xff08;
        let Input::Key { sym, pressed: true, modifiers: m, .. } = input else { return None };
        match *sym {
            UP if m.alt => self.update(FileMsg::Up, window),
            LEFT if m.alt => self.update(FileMsg::Back, window),
            UP if !m.ctrl => self.move_choice(-1),
            DOWN if !m.ctrl => self.move_choice(1),
            PAGE_UP => self.move_choice(-10),
            PAGE_DOWN => self.move_choice(10),
            ESCAPE => {
                if self.new_folder.take().is_none() && self.replace.take().is_none() {
                    window.close();
                }
            }
            BACKSPACE if self.mode == Mode::Open && self.typed.is_empty() && self.new_folder.is_none() => self.update(FileMsg::Up, window),
            s if m.ctrl && char::from_u32(s).is_some_and(|c| c.eq_ignore_ascii_case(&'h')) => self.update(FileMsg::Hidden, window),
            _ => return None,
        }
        Some(true)
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { kind: vergence_ui::WinKind::Dialog, modal: true, min: (640.0, 400.0), max: (0.0, 0.0), alone: false, frosted: false }
    }

    fn opening_size(&self) -> Option<(u32, u32)> {
        Some((860, 540))
    }
}

impl Drop for FileDialog {
    /// Closed without an answer: cancelled.
    fn drop(&mut self) {
        if let Some(done) = self.done.take() {
            done(Vec::new());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::natural;
    use std::time::SystemTime;

    #[test]
    fn names_sort_naturally_and_patterns_match_without_case() {
        let mut v = vec!["file 10", "File 2", "file 1", "alpha", "Beta"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(v, vec!["alpha", "Beta", "file 1", "File 2", "file 10"]);
        assert!(glob("*.txt", "Notes.TXT"));
        assert!(glob("read?e", "readme"));
        assert!(!glob("*.md", "notes.txt"));
        assert!(Filter::new("All files", &[]).admits("anything", None));
        let text = Filter::of_types("Text files", &["text/plain"]);
        assert!(text.admits("notes.md", vergence_types::of_name("notes.md")), "Markdown is text");
        assert!(text.admits("run.sh", vergence_types::of_name("run.sh")), "a script is text");
        assert!(!text.admits("photo.jpg", vergence_types::of_name("photo.jpg")));
        assert!(Filter::of_types("Pictures", &["image/*"]).admits("photo.jpg", vergence_types::of_name("photo.jpg")));
        assert_eq!(Kind::of(vergence_types::of_name("photo.jpg")), Kind::Picture);
        assert_eq!(Kind::of(Some(vergence_types::FOLDER)), Kind::Folder);
        assert_eq!(Kind::of(vergence_types::of_name("a.tar.gz")), Kind::Archive);
        assert_eq!(Kind::of(None), Kind::Other);
    }

    #[test]
    fn sizes_and_paths_in_words() {
        assert_eq!(size_words(512), "512 bytes");
        assert_eq!(size_words(12_345), "12 KB");
        assert_eq!(size_words(3_400_000), "3.4 MB");
        assert!(looks_like_path("~/x") && looks_like_path("/etc") && !looks_like_path("notes"));
        assert_eq!(expand("b.txt", Some(Path::new("/a"))), PathBuf::from("/a/b.txt"));
        assert!(when(SystemTime::now()).starts_with("Today "));
        assert!(when(SystemTime::now() - Duration::from_secs(86_400)).starts_with("Yesterday "));
    }

    #[test]
    fn a_folder_is_listed_folders_first_and_narrowed_by_typing() {
        let d = std::env::temp_dir().join(format!("vergence-files-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sub")).unwrap();
        for n in ["b.txt", "a.md", ".hidden", "file 10.txt", "file 2.txt"] {
            fs::write(d.join(n), "x").unwrap();
        }
        let mut f = FileDialog::open("test", "Open", Theme::default(), |_| {}).folder(&d);
        let names = |f: &FileDialog| f.visible().iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&f), vec!["sub", "a.md", "b.txt", "file 2.txt", "file 10.txt"]);
        f.hidden = true;
        assert_eq!(names(&f).len(), 6);
        f.hidden = false;
        f.typed = "FILE".into();
        assert_eq!(names(&f), vec!["file 2.txt", "file 10.txt"]);
        f.typed.clear();
        f.filters = vec![Filter::new("Markdown", &["*.md"])];
        assert_eq!(names(&f), vec!["sub", "a.md"], "a filter leaves folders in");
        f.sort = (SortBy::Name, false);
        f.filters.clear();
        assert_eq!(names(&f)[0], "sub", "folders stay first whichever way");
        assert_eq!(names(&f)[1], "file 10.txt");
        // Crumbs from the root for a folder not under Home.
        if !d.starts_with(home().unwrap_or_default()) {
            assert_eq!(f.crumbs()[0].0, "Computer");
        }
        fs::remove_dir_all(&d).unwrap();
    }
}
