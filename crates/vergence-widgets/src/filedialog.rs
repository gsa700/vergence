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
use std::time::{Duration, Instant, SystemTime};

use vergence_scene::{Color, Rect};
use vergence_ui::{Input, Window};

use crate::{Align, ButtonLook, Element, Length, Padding, Program, Theme, button, column, drawing, field, label, panel, row, rule, scroll, space};

/// The most rows listed (a folder of more is narrowed by typing).
const MAX_ROWS: usize = 2000;
const MAX_RECENT: usize = 16;
const PREVIEW_BYTES: usize = 8 * 1024;
const PREVIEW_LINES: usize = 40;
const DOUBLE: Duration = Duration::from_millis(450);

/// Which files a dialog shows: a name ("Text files") and patterns
/// ("*.txt"); a filter with no patterns shows everything.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    pub name: String,
    pub patterns: Vec<String>,
}

impl Filter {
    pub fn new(name: &str, patterns: &[&str]) -> Filter {
        Filter { name: name.into(), patterns: patterns.iter().map(|p| p.to_string()).collect() }
    }

    fn admits(&self, name: &str) -> bool {
        self.patterns.is_empty() || self.patterns.iter().any(|p| glob(p, name))
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    Name,
    Size,
    Changed,
}

#[derive(Clone, Debug)]
struct Entry {
    name: String,
    path: PathBuf,
    dir: bool,
    size: u64,
    changed: Option<SystemTime>,
}

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
            turn: Rc::new(Cell::new(0)),
        };
        d.read();
        d
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

    /// Reads what it shows again.
    fn read(&mut self) {
        self.problem = None;
        self.selected = None;
        self.entries = match &self.showing {
            Showing::Folder(dir) => match fs::read_dir(dir) {
                Ok(rd) => rd
                    .flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let path = e.path();
                        // Follow links: a link to a folder is a folder.
                        let m = fs::metadata(&path).ok();
                        Some(Entry { dir: m.as_ref().is_some_and(|m| m.is_dir()), size: m.as_ref().map_or(0, |m| m.len()), changed: m.and_then(|m| m.modified().ok()), name, path })
                    })
                    .collect(),
                Err(e) => {
                    self.problem = Some(format!("This folder cannot be read: {}", plain(&e)));
                    Vec::new()
                }
            },
            Showing::Recent => recent_files()
                .into_iter()
                .filter_map(|p| {
                    let m = fs::metadata(&p).ok()?;
                    Some(Entry { name: p.file_name()?.to_string_lossy().into_owned(), dir: m.is_dir(), size: m.len(), changed: m.modified().ok(), path: p })
                })
                .collect(),
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
            .filter(|e| e.dir || filter.is_none_or(|f| f.admits(&e.name)))
            .filter(|e| narrow.is_empty() || e.name.to_lowercase().contains(&narrow))
            .collect();
        let (by, up) = self.sort;
        let recent = self.showing == Showing::Recent;
        v.sort_by(|a, b| {
            // Folders first (the recent files keep their order unless sorted).
            let o = b.dir.cmp(&a.dir).then_with(|| match by {
                SortBy::Name if recent => std::cmp::Ordering::Equal,
                SortBy::Name => natural(&a.name, &b.name),
                SortBy::Size => a.size.cmp(&b.size),
                SortBy::Changed => a.changed.cmp(&b.changed),
            });
            if up || b.dir != a.dir { o } else { o.reverse() }
        });
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

    fn places(&self) -> Vec<(String, Showing)> {
        let mut out = Vec::new();
        if let Some(h) = home() {
            out.push(("Home".to_string(), Showing::Folder(h.clone())));
            for name in ["Desktop", "Documents", "Downloads", "Pictures", "Music", "Videos"] {
                let p = h.join(name);
                if p.is_dir() {
                    out.push((name.to_string(), Showing::Folder(p)));
                }
            }
        }
        out.push(("Recent".into(), Showing::Recent));
        out.push(("Computer".into(), Showing::Folder(PathBuf::from("/"))));
        for m in mounted_disks() {
            let name = m.file_name().map_or_else(|| m.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
            out.push((name, Showing::Folder(m)));
        }
        out
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
        let places: Vec<Element<FileMsg>> = self
            .places()
            .into_iter()
            .map(|(name, s)| {
                let here = s == self.showing;
                let m = match &s {
                    Showing::Recent => FileMsg::Recent,
                    Showing::Folder(p) => FileMsg::Go(p.clone()),
                };
                button(label(name.clone()).size(12.0).width(Length::FILL)).look(if here { ButtonLook::Chosen } else { ButtonLook::Plain }).padding(Padding::hv(5.0, 10.0)).on_press(m).width(Length::FILL).name(name)
            })
            .collect();
        let sidebar = panel(scroll(column(places).spacing(1.0).width(Length::FILL))).color(t.card).radius(8.0).padding(6.0).width(Length::Fixed(160.0)).height(Length::FILL);

        // The list: headings, then the rows.
        let recent = self.showing == Showing::Recent;
        let heading = |text: &str, by: SortBy, w: Length, end: bool| -> Element<FileMsg> {
            let mark = if self.sort.0 == by { if self.sort.1 { " ▲" } else { " ▼" } } else { "" };
            button(label(format!("{text}{mark}")).size(11.0).dim().align(if end { Align::End } else { Align::Start }).width(Length::FILL)).look(ButtonLook::Plain).padding(Padding::hv(2.0, 6.0)).on_press(FileMsg::Sort(by)).width(w).name(format!("Sort by {text}"))
        };
        let size_w = Length::Fixed(80.0);
        let when_w = Length::Fixed(130.0);
        let head = row(vec![space().width(Length::Fixed(22.0)), heading("Name", SortBy::Name, Length::FILL, false), heading("Size", SortBy::Size, size_w, true), heading("Changed", SortBy::Changed, when_w, false)]).spacing(4.0).align(Align::Center).width(Length::FILL);
        let visible = self.visible();
        let mut rows: Vec<Element<FileMsg>> = Vec::with_capacity(visible.len());
        for (k, e) in visible.iter().enumerate() {
            let chosen = self.selected == Some(k);
            let mut name_col: Vec<Element<FileMsg>> = vec![label(e.name.clone()).size(12.0).width(Length::FILL)];
            if recent && let Some(p) = e.path.parent() {
                name_col.push(label(tilde(p)).size(10.0).dim().width(Length::FILL));
            }
            let mut b = button(
                row(vec![
                    kind_icon(e.dir),
                    column(name_col).width(Length::FILL),
                    label(if e.dir { String::new() } else { size_words(e.size) }).size(11.0).dim().align(Align::End).width(size_w),
                    label(e.changed.map(when).unwrap_or_default()).size(11.0).dim().width(when_w),
                ])
                .spacing(4.0)
                .align(Align::Center)
                .width(Length::FILL),
            )
            .look(if chosen { ButtonLook::Chosen } else { ButtonLook::Plain })
            .padding(Padding::hv(3.0, 6.0))
            .on_press(FileMsg::Click(k))
            .width(Length::FILL)
            .name(e.name.clone());
            if chosen {
                // A new row each time the choice moves, so it is scrolled to.
                b = b.key(self.turn.get() << 32 | k as u64).keep_in_view();
            }
            rows.push(b);
        }
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
            Mode::Open => foot.push(field(self.typed.clone(), FileMsg::Typed).placeholder("Type to narrow the list, or a path").on_submit(FileMsg::Submit).autofocus().width(Length::FILL).name("Find or go to")),
        }
        if self.filters.len() > 1 {
            let names: Vec<String> = self.filters.iter().map(|f| f.name.clone()).collect();
            foot.push(crate::choice(names, Some(self.filter), FileMsg::Filter).width(Length::Fixed(150.0)).name("Kind of file"));
        }
        if self.mode == Mode::Save && self.dir().is_some() {
            foot.push(button(label("New folder").size(12.0)).padding(Padding::hv(4.0, 10.0)).on_press(FileMsg::NewFolder).name("New folder"));
        }
        foot.push(button(label("Cancel").size(12.0)).padding(Padding::hv(4.0, 12.0)).on_press(FileMsg::Cancel).name("Cancel"));
        let go = if self.mode == Mode::Save { "Save" } else { "Open" };
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
            FileMsg::Click(k) => {
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
        vergence_ui::Style { kind: vergence_ui::WinKind::Dialog, modal: true, min: (640.0, 400.0), max: (0.0, 0.0) }
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

/// A small folder or page, drawn.
fn kind_icon<M: 'static>(dir: bool) -> Element<M> {
    drawing(move |scene, _, r: Rect, t: &Theme| {
        let (x, y) = (r.x + 2.0, r.y + (r.h - 14.0) / 2.0);
        if dir {
            let c = t.accent;
            scene.fill_rounded(Rect::new(x, y + 1.0, 7.0, 4.0), 1.5, c);
            scene.fill_rounded(Rect::new(x, y + 3.0, 16.0, 11.0), 2.0, c);
        } else {
            scene.fill_rounded(Rect::new(x + 2.0, y, 12.0, 15.0), 2.0, t.dim);
            scene.fill_rounded(Rect::new(x + 3.0, y + 1.0, 10.0, 13.0), 1.5, t.page);
            for k in 0..3 {
                scene.fill(Rect::new(x + 5.0, y + 4.0 + k as f32 * 3.0, 6.0, 1.0), Color { a: 160, ..t.dim });
            }
        }
    })
    .width(Length::Fixed(20.0))
    .height(Length::Fixed(18.0))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute())
}

/// A path as shown: ~ for Home.
fn tilde(p: &Path) -> String {
    match home().and_then(|h| p.strip_prefix(&h).ok().map(Path::to_path_buf)) {
        Some(r) if r.as_os_str().is_empty() => "~".into(),
        Some(r) => format!("~/{}", r.display()),
        None => p.display().to_string(),
    }
}

fn looks_like_path(s: &str) -> bool {
    s.starts_with('/') || s.starts_with('~')
}

/// A typed path made whole: ~ is Home; anything else is from `dir`.
fn expand(s: &str, dir: Option<&Path>) -> PathBuf {
    if s == "~" {
        return home().unwrap_or_else(|| PathBuf::from("/"));
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return home().unwrap_or_else(|| PathBuf::from("/")).join(rest);
    }
    let p = PathBuf::from(s);
    if p.is_absolute() { p } else { dir.map_or(p.clone(), |d| d.join(&p)) }
}

/// Names in the order people expect: without case, numbers by their value.
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n = String::new();
                let mut m = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n.push(c);
                    x.next();
                }
                while let Some(d) = y.peek().copied().filter(char::is_ascii_digit) {
                    m.push(d);
                    y.next();
                }
                let (n2, m2) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n2.len().cmp(&m2.len()).then_with(|| n2.cmp(m2));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

fn size_words(n: u64) -> String {
    match n {
        0..1000 => format!("{n} bytes"),
        1000..1_000_000 => format!("{:.0} KB", n as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1} MB", n as f64 / 1e6),
        _ => format!("{:.1} GB", n as f64 / 1e9),
    }
}

/// When a file changed, in words: "Today 14:03", "Yesterday 09:12",
/// "Nov 3", "2025-11-03".
fn when(t: SystemTime) -> String {
    let secs = |t: SystemTime| t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let (Some(at), Some(now)) = (local(secs(t)), local(secs(SystemTime::now()))) else { return String::new() };
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let day = |d: &Local| (d.year, d.yday);
    if day(&at) == day(&now) {
        format!("Today {:02}:{:02}", at.hour, at.min)
    } else if (at.year == now.year && at.yday + 1 == now.yday) || local(secs(SystemTime::now()) - 86_400).is_some_and(|y| day(&y) == day(&at)) {
        format!("Yesterday {:02}:{:02}", at.hour, at.min)
    } else if at.year == now.year {
        format!("{} {}", MONTHS[at.month.min(11) as usize], at.mday)
    } else {
        format!("{}-{:02}-{:02}", at.year, at.month + 1, at.mday)
    }
}

struct Local {
    year: i32,
    month: i32,
    mday: i32,
    yday: i32,
    hour: i32,
    min: i32,
}

/// The local time of `secs` since 1970, by localtime_r(3) (glibc's and
/// FreeBSD's struct tm agree on these fields and their order).
fn local(secs: i64) -> Option<Local> {
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: i64,
        zone: *const std::ffi::c_char,
    }
    unsafe extern "C" {
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }
    let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
    // SAFETY: localtime_r reads the time and fills the struct we own.
    let r = unsafe { localtime_r(&secs, &mut tm) };
    (!r.is_null()).then_some(Local { year: tm.year + 1900, month: tm.mon, mday: tm.mday, yday: tm.yday, hour: tm.hour, min: tm.min })
}

/// The start of a file, if it is text.
fn text_start(p: &Path) -> Option<String> {
    use std::io::Read;
    let mut buf = vec![0u8; PREVIEW_BYTES];
    let n = fs::File::open(p).ok()?.read(&mut buf).ok()?;
    buf.truncate(n);
    if buf.contains(&0) {
        return None;
    }
    // Cut at a character's edge.
    let s = match std::str::from_utf8(&buf) {
        Ok(s) => s.to_string(),
        Err(e) if e.valid_up_to() + 4 >= buf.len() => String::from_utf8_lossy(&buf[..e.valid_up_to()]).into_owned(),
        Err(_) => return None,
    };
    Some(s)
}

/// Disks mounted where removable and extra disks go.
fn mounted_disks() -> Vec<PathBuf> {
    let text = fs::read_to_string("/proc/self/mounts").ok().or_else(|| {
        // FreeBSD: mount -p prints the table in fstab's form.
        std::process::Command::new("mount").arg("-p").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    });
    let mut out: Vec<PathBuf> = text
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .map(|m| m.replace("\\040", " "))
        .filter(|m| ["/media/", "/mnt/", "/run/media/"].iter().any(|p| m.starts_with(p)) || m == "/mnt")
        .map(PathBuf::from)
        .collect();
    out.sort();
    out.dedup();
    out
}

fn plain(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "you do not have permission.".into(),
        std::io::ErrorKind::NotFound => "it is not there.".into(),
        std::io::ErrorKind::AlreadyExists => "something of that name is already here.".into(),
        _ => {
            let s = e.to_string();
            format!("{}.", s.split(" (os error").next().unwrap_or(&s).trim_end_matches('.'))
        }
    }
}

// --- What is remembered ---

fn state_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| home().map(|h| h.join(".local/state")))?;
    Some(base.join("vergence/file-dialog"))
}

/// The state file's lines: "folder <app>\t<path>" and "file\t<path>", newest files first.
fn state_lines() -> Vec<(String, String)> {
    let Some(p) = state_path() else { return Vec::new() };
    fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter(|(_, v)| v.starts_with('/') && v.len() <= 4096)
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .take(256)
        .collect()
}

fn remembered_folder(app: &str) -> Option<PathBuf> {
    let key = format!("folder {app}");
    state_lines().into_iter().find(|(k, _)| *k == key).map(|(_, v)| PathBuf::from(v))
}

fn recent_files() -> Vec<PathBuf> {
    state_lines().into_iter().filter(|(k, _)| k == "file").map(|(_, v)| PathBuf::from(v)).collect()
}

/// Keeps `app`'s folder and puts `file` at the top of the recent files.
fn remember(app: &str, dir: &Path, file: &Path) {
    let Some(path) = state_path() else { return };
    let key = format!("folder {app}");
    let (dir, file) = (dir.to_string_lossy().into_owned(), file.to_string_lossy().into_owned());
    if dir.contains(['\t', '\n']) || file.contains(['\t', '\n']) {
        return;
    }
    let old = state_lines();
    let mut lines = vec![(key.clone(), dir), ("file".to_string(), file.clone())];
    lines.extend(old.iter().filter(|(k, _)| *k != key && k.starts_with("folder ")).cloned());
    lines.extend(old.iter().filter(|(k, v)| k == "file" && *v != file).take(MAX_RECENT - 1).cloned());
    let text: String = lines.iter().map(|(k, v)| format!("{k}\t{v}\n")).collect();
    if let Some(d) = path.parent() {
        let _ = fs::create_dir_all(d);
    }
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("new");
    let written = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).and_then(|mut f| f.write_all(text.as_bytes()));
    if written.is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sort_naturally_and_patterns_match_without_case() {
        let mut v = vec!["file 10", "File 2", "file 1", "alpha", "Beta"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(v, vec!["alpha", "Beta", "file 1", "File 2", "file 10"]);
        assert!(glob("*.txt", "Notes.TXT"));
        assert!(glob("read?e", "readme"));
        assert!(!glob("*.md", "notes.txt"));
        assert!(Filter::new("All files", &[]).admits("anything"));
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
