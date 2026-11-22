//! FileView's commands: their numbers (cmd), what each does (`run`), the
//! right-click menus with Open With, opening what is chosen, and the list the
//! app menu shows. The window itself is in main.rs.

use std::path::{Path, PathBuf};

use vergence_ui::{AppCommand as C, MenuLine, Role};
use vergence_widgets::files::{self, Entry, GroupBy, RowSize, SortBy};
use vergence_widgets::{Program, Window};

use crate::{
    APP_ID, Ask, Naming, Showing, ViewKind, Viewer, home, notify, open_files, open_files_with, ops, props, sidebar,
};

pub(crate) mod cmd {
    pub const NEW_WINDOW: u32 = 1;
    pub const NEW_TAB: u32 = 2;
    pub const CLOSE: u32 = 3;
    pub const CLOSE_TAB: u32 = 17;
    pub const NEXT_TAB: u32 = 14;
    pub const PREV_TAB: u32 = 15;
    pub const BACK: u32 = 4;
    pub const FORWARD: u32 = 5;
    pub const UP: u32 = 6;
    pub const LIST: u32 = 7;
    pub const ICONS: u32 = 8;
    pub const PANE: u32 = 9;
    pub const HIDDEN: u32 = 10;
    pub const PREVIEW: u32 = 11;
    pub const OPEN: u32 = 12;
    pub const SELECT_ALL: u32 = 13;
    pub const RELOAD: u32 = 16;
    pub const CUT: u32 = 20;
    pub const COPY: u32 = 21;
    pub const PASTE: u32 = 22;
    pub const DUPLICATE: u32 = 23;
    pub const RENAME: u32 = 24;
    pub const TRASH: u32 = 25;
    pub const DELETE: u32 = 26;
    pub const NEW_FOLDER: u32 = 27;
    pub const UNDO: u32 = 28;
    pub const RESTORE: u32 = 29;
    pub const EMPTY_TRASH: u32 = 30;
    pub const EJECT: u32 = 31;
    pub const SETTINGS: u32 = 40;
    pub const ABOUT: u32 = 41;
    // The right-click menus' own (backup milestone 1, step 1).
    pub const SHOW_IN_FOLDER: u32 = 50;
    pub const MAKE_LINK: u32 = 51;
    pub const COMPRESS: u32 = 52;
    pub const INFO: u32 = 53;
    pub const VERSIONS: u32 = 54;
    pub const COPY_PATH: u32 = 55;
    pub const TERMINAL: u32 = 56;
    pub const NEW_FILE: u32 = 57;
    pub const SORT_NAME: u32 = 58;
    pub const SORT_SIZE: u32 = 59;
    pub const SORT_CHANGED: u32 = 60;
    pub const SORT_KIND: u32 = 61;
    pub const ROWS_SMALL: u32 = 62;
    pub const ROWS_MEDIUM: u32 = 63;
    pub const ROWS_LARGE: u32 = 64;
    // Group by (files milestone 3, step 4).
    pub const GROUP_KIND: u32 = 65;
    pub const GROUP_DATE: u32 = 66;
    pub const GROUP_SIZE: u32 = 67;
    pub const GROUP_NONE: u32 = 68;
    // A sidebar row's menu (step 3), for the row it was opened on.
    pub const SIDE_NEW_TAB: u32 = 70;
    pub const SIDE_REMOVE: u32 = 71;
    pub const SIDE_SHOW: u32 = 72;
    pub const SIDE_HIDE_RECENT: u32 = 73;
    pub const SIDE_EJECT: u32 = 74;
    pub const MOUNT_IMAGE: u32 = 75;
    /// Open With's applications: this, plus their place in the list.
    pub const OPEN_WITH: u32 = 1000;
    pub const MAX_OPEN_WITH: usize = 16;
}

impl Viewer {
    /// Opens a file with the app chosen for its type, and puts it among the recent ones.
    pub(crate) fn open_file(&mut self, p: &Path) {
        if let Some(dir) = p.parent() {
            files::remember(APP_ID, dir, p);
        }
        if let Err(e) = open_files(&[p.to_path_buf()]) {
            self.notice = Some(e);
        }
    }

    /// Opens what is chosen: one folder here; files with their apps.
    pub(crate) fn open_chosen(&mut self) {
        let Some(t) = self.front_tab() else { return };
        let chosen: Vec<Entry> = t.entries.iter().filter(|e| t.chosen.contains(&e.path)).cloned().collect();
        if let [one] = chosen.as_slice()
            && one.dir
        {
            let p = one.path.clone();
            // The Trash window shows only the Trash: a folder in it opens in FileView.
            if self.trash_window {
                let _ = crate::start(std::process::Command::new("vergence-files").arg(&p));
                return;
            }
            self.close_peek();
            return self.go(Showing::Folder(p));
        }
        // A disk image opened is mounted, as a drive plugged in (step 7).
        if let [one] = chosen.as_slice()
            && files::is_disk_image(one)
            && !self.trash_window
        {
            let p = one.path.clone();
            self.mount_image(p);
            return;
        }
        let paths: Vec<PathBuf> = chosen.iter().filter(|e| !e.dir).map(|e| e.path.clone()).collect();
        // Among the recent files (the sidebar's Recent), as the other apps put theirs.
        for p in &paths {
            if let Some(dir) = p.parent() {
                files::remember(APP_ID, dir, p);
            }
        }
        if !paths.is_empty()
            && let Err(e) = open_files(&paths)
        {
            self.notice = Some(e);
        }
    }

    pub(crate) fn run(&mut self, id: u32, window: &mut Window) {
        match id {
            // New windows and tabs open at home (files milestone 3, item 7).
            cmd::NEW_WINDOW => {
                let mut w = Viewer::new(self.shared.clone());
                w.new_tab(Showing::Folder(home()));
                window.open_window(vergence_widgets::window(w));
            }
            cmd::NEW_TAB if !self.trash_window => self.new_tab(Showing::Folder(home())),
            cmd::CLOSE => window.close(),
            cmd::CLOSE_TAB => {
                if self.trash_window {
                    window.close();
                } else if let Some(id) = self.front_tab().map(|t| t.id) {
                    self.close_tab(id, window);
                }
            }
            cmd::NEXT_TAB | cmd::PREV_TAB => {
                let n = self.tabs.len() as isize;
                if n > 1 {
                    let i = self.tabs.iter().position(|t| t.id == self.front).unwrap_or(0) as isize;
                    self.front = self.tabs[(i + if id == cmd::NEXT_TAB { 1 } else { -1 }).rem_euclid(n) as usize].id;
                    self.tab_changed();
                }
            }
            cmd::BACK => self.back(),
            cmd::FORWARD => self.forward(),
            cmd::UP => self.up(),
            cmd::LIST => {
                self.view = ViewKind::List;
                self.write_key("view", None);
            }
            cmd::ICONS => {
                self.view = ViewKind::Icons;
                self.write_key("view", Some("\"icons\""));
            }
            cmd::PANE => {
                self.pane = !self.pane;
                self.write_key("preview-pane", if self.pane { Some("true") } else { None });
            }
            cmd::HIDDEN => {
                self.hidden = !self.hidden;
                self.write_key("hidden", if self.hidden { Some("true") } else { None });
            }
            cmd::PREVIEW => self.toggle_peek(window),
            cmd::OPEN => self.open_chosen(),
            cmd::SELECT_ALL => {
                let all: Vec<PathBuf> = self.visible().iter().map(|e| e.path.clone()).collect();
                if let Some(t) = self.front_mut() {
                    t.chosen = all;
                }
            }
            cmd::RELOAD => {
                if let Some(t) = self.front_mut() {
                    t.read();
                }
            }
            cmd::CUT | cmd::COPY => {
                let paths = self.chosen_paths();
                if !paths.is_empty() {
                    window.copy(&paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n"));
                    self.shared.borrow_mut().clip = Some((paths, id == cmd::CUT));
                }
            }
            cmd::PASTE => self.paste(),
            cmd::DUPLICATE => {
                let paths = self.chosen_paths();
                if let Some(dir) = paths.first().and_then(|p| p.parent()).map(Path::to_path_buf) {
                    self.start_paste(paths, dir, false, ops::Clash::KeepBoth);
                }
            }
            // F2 (Explorer's key) and the menu's Rename: the name a field in its row.
            cmd::RENAME => {
                if let [one] = self.chosen_paths().as_slice() {
                    self.start_rename(one.clone());
                }
            }
            cmd::NEW_FOLDER => {
                if self.here().is_some() {
                    self.naming = Some(Naming { rename: None, text: "New folder".into(), file: false });
                }
            }
            cmd::TRASH => self.trash_chosen(),
            cmd::DELETE => {
                let paths = self.chosen_paths();
                if !paths.is_empty() {
                    self.ask = Some(Ask::Forever(paths));
                }
            }
            cmd::UNDO => {
                let last = self.shared.borrow_mut().undo.pop();
                if let Some(d) = last {
                    // Only what went wrong is said: what was done shows itself
                    // (Dave, 2026-11-18: "I know I deleted a file").
                    if let Err(why) = ops::undo(&d) {
                        self.notice = Some(format!("It could not be undone: {why}"));
                    }
                    self.read_all();
                }
            }
            cmd::RESTORE => {
                self.notice = None;
                let names: Vec<(String, String)> = self
                    .chosen_paths()
                    .iter()
                    .filter_map(|p| p.file_name().map(|n| (n.to_string_lossy().into_owned(), self.shown_name(p))))
                    .collect();
                // Only what went wrong is said (Dave, 2026-11-18).
                for (n, shown) in &names {
                    if let Err(e) = ops::restore(n) {
                        self.notice = Some(format!("“{shown}” could not be put back: {e}"));
                    }
                }
                self.read_all();
            }
            cmd::EMPTY_TRASH => self.ask = Some(Ask::EmptyTrash),
            cmd::EJECT => {
                if let Some(here) = self.here().filter(|_| self.here_ejectable()) {
                    let name = here.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let image = files::is_image_mount(&here);
                    for t in self.tabs.iter_mut().filter(|t| t.dir().is_some_and(|d| d.starts_with(&here))) {
                        t.go(Showing::Folder(home()));
                    }
                    match files::eject(&here) {
                        Ok(told) => {
                            if !image && !told {
                                notify(&format!("Safe to remove “{name}”"), "The drive can be pulled out now.");
                            }
                            *self.ejectable.borrow_mut() = None;
                            if let Some(h) = files::home() {
                                self.go(Showing::Folder(h));
                            }
                        }
                        Err(why) => self.notice = Some(format!("“{name}” could not be ejected: {why}")),
                    }
                }
            }
            cmd::NEW_FILE => {
                if self.here().is_some() {
                    self.naming = Some(Naming { rename: None, text: "New file.txt".into(), file: true });
                }
            }
            cmd::MAKE_LINK => self.make_link(),
            cmd::COMPRESS => self.compress(),
            cmd::INFO => {
                // A Properties window for each chosen (a few at most), or the folder's.
                let paths = match self.chosen_paths() {
                    none if none.is_empty() => self.here().into_iter().collect(),
                    some => some,
                };
                for p in paths.into_iter().take(8) {
                    window.open_window(vergence_widgets::window(props::Properties::new(
                        p,
                        self.shared.clone(),
                        self.theme(),
                    )));
                }
            }
            cmd::COPY_PATH => {
                let paths = self.chosen_paths();
                if !paths.is_empty() {
                    window.copy(&paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n"));
                }
            }
            cmd::TERMINAL => {
                // A folder opens there; a file, in its folder; nothing chosen, here.
                let at = match self.chosen_paths().as_slice() {
                    [one] if one.is_dir() => Some(one.clone()),
                    [one] => one.parent().map(Path::to_path_buf),
                    _ => self.here(),
                };
                if let Some(d) = at
                    && let Err(e) = crate::start(std::process::Command::new("vergence-term").current_dir(&d))
                {
                    self.notice = Some(format!("The Terminal could not be opened: {}", files::plain(&e)));
                }
            }
            cmd::SHOW_IN_FOLDER => {
                if let [one] = self.chosen_paths().as_slice()
                    && let Some(dir) = one.parent()
                {
                    let one = one.clone();
                    self.search = None;
                    self.search_entries.clear();
                    self.clear_narrow();
                    self.go(Showing::Folder(dir.to_path_buf()));
                    self.choose_only(one);
                }
            }
            cmd::SORT_NAME => self.sort = (SortBy::Name, true),
            cmd::SORT_SIZE => self.sort = (SortBy::Size, false),
            cmd::SORT_CHANGED => self.sort = (SortBy::Changed, false),
            cmd::SORT_KIND => self.sort = (SortBy::Kind, true),
            cmd::ROWS_SMALL | cmd::ROWS_MEDIUM | cmd::ROWS_LARGE => {
                let size = match id {
                    cmd::ROWS_SMALL => RowSize::Small,
                    cmd::ROWS_LARGE => RowSize::Large,
                    _ => RowSize::Medium,
                };
                // Medium is the default: no key.
                let words = format!("\"{}\"", size.word());
                self.write_key("row-size", (size != RowSize::Medium).then_some(words.as_str()));
            }
            cmd::GROUP_KIND | cmd::GROUP_DATE | cmd::GROUP_SIZE | cmd::GROUP_NONE => {
                let g = match id {
                    cmd::GROUP_DATE => GroupBy::Date,
                    cmd::GROUP_SIZE => GroupBy::Size,
                    cmd::GROUP_NONE => GroupBy::None,
                    _ => GroupBy::Kind,
                };
                // Kind is the default: no key; the older switch taken away.
                let words = format!("\"{}\"", g.word());
                self.write_key("group-by-kind", None);
                self.write_key("group-by", (g != GroupBy::Kind).then_some(words.as_str()));
            }
            id if (cmd::OPEN_WITH..cmd::OPEN_WITH + cmd::MAX_OPEN_WITH as u32).contains(&id) => {
                if let Some(app) = self.open_with.get((id - cmd::OPEN_WITH) as usize).cloned()
                    && let Err(e) = open_files_with(&self.chosen_paths(), Some(&app))
                {
                    self.notice = Some(format!("It could not be opened: {e}"));
                }
            }
            cmd::SIDE_NEW_TAB | cmd::SIDE_SHOW | cmd::SIDE_REMOVE => {
                if let Some(sidebar::Line::Item(_, item)) = self.side_target.take() {
                    match (id, item.path) {
                        (cmd::SIDE_NEW_TAB, None) => self.new_tab(Showing::Recent),
                        (cmd::SIDE_NEW_TAB, Some(p)) => self.new_tab(Showing::Folder(p)),
                        (cmd::SIDE_SHOW, Some(p)) => {
                            if let Some(dir) = p.parent() {
                                self.go(Showing::Folder(dir.to_path_buf()));
                                self.choose_only(p.clone());
                            }
                        }
                        (cmd::SIDE_REMOVE, Some(p)) => {
                            let favs: Vec<PathBuf> =
                                sidebar::favorites(&self.side_setup()).into_iter().filter(|f| *f != p).collect();
                            self.set_favorites(&favs);
                        }
                        _ => {}
                    }
                }
            }
            cmd::SIDE_HIDE_RECENT => self.write_key("recent", Some("false")),
            cmd::MOUNT_IMAGE => {
                if let [one] = self.chosen_paths().as_slice() {
                    self.mount_image(one.clone());
                }
            }
            cmd::SIDE_EJECT => {
                if let Some(sidebar::Line::Item(_, item)) = self.side_target.take()
                    && let Some(p) = item.path
                {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let image = files::is_image_mount(&p);
                    // Tabs on it go home first: on FreeBSD a look into the folder
                    // after the eject would have the automounter mount it again.
                    for t in self.tabs.iter_mut().filter(|t| t.dir().is_some_and(|d| d.starts_with(&p))) {
                        t.go(Showing::Folder(home()));
                    }
                    match files::eject(&p) {
                        Ok(told) => {
                            // A drive: safe to pull out (the server says so itself when it
                            // ejected); a disk image: put away, nothing to say.
                            if !image && !told {
                                notify(&format!("Safe to remove “{name}”"), "The drive can be pulled out now.");
                            }
                            *self.ejectable.borrow_mut() = None;
                            // Tabs showing it go home.
                            for t in self.tabs.iter_mut().filter(|t| t.dir().is_some_and(|d| d.starts_with(&p))) {
                                t.go(Showing::Folder(home()));
                            }
                        }
                        Err(why) => self.notice = Some(format!("“{name}” could not be ejected: {why}")),
                    }
                }
            }
            cmd::SETTINGS => self.open_settings(window),
            cmd::ABOUT => {
                let about = vergence_widgets::about::About::new(
                    APP_ID,
                    "FileView",
                    env!("CARGO_PKG_VERSION"),
                    "Vergence's file manager: your folders as a list or icons, Preview on the space bar, files opened with the apps chosen for them.",
                    self.theme(),
                );
                window.open_window(vergence_widgets::window(about));
            }
            _ => {}
        }
        self.want_picture();
        self.keep();
        window.redraw();
    }

    /// A chosen item's menu (Dave: "a blend of macOS and Windows with Linux
    /// flavoring"): every line always there, faint when it cannot be done.
    pub(crate) fn item_menu(&mut self) -> Vec<MenuLine> {
        let chosen = self.chosen_paths();
        let one = chosen.len() == 1;
        let any = !chosen.is_empty();
        let trash = self.in_trash();
        let results = self.search.is_some() || self.front_tab().is_some_and(|t| t.showing == Showing::Recent);
        let mut lines = vec![MenuLine::item(cmd::OPEN, "Open", "Enter")];
        // A disk image: mounted, as a drive plugged in (step 7).
        let image =
            one && !trash && self.visible().iter().any(|e| Some(&e.path) == chosen.first() && files::is_disk_image(e));
        if image {
            lines.push(MenuLine::item(cmd::MOUNT_IMAGE, "Mount Disk Image", ""));
        }
        // Open With: the applications for the files' type, when they share one.
        self.open_with.clear();
        let files: Vec<&PathBuf> = chosen.iter().filter(|p| !p.is_dir()).collect();
        let mimes: Vec<String> = files.iter().map(|p| vergence_types::of_file(p).to_string()).collect();
        if !files.is_empty() && files.len() == chosen.len() && mimes.iter().all(|m| *m == mimes[0]) {
            let entries = vergence_shell::apps::installed();
            let ids = vergence_shell::open::apps_for(&mimes[0], &entries, &vergence_types::lists());
            let mut subs = Vec::new();
            for id in ids.into_iter().take(cmd::MAX_OPEN_WITH) {
                let name = entries
                    .iter()
                    .find(|e| vergence_shell::open::desktop_id(&e.id) == id)
                    .map(|e| e.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| id.trim_end_matches(".desktop").to_string());
                subs.push(MenuLine::item(
                    cmd::OPEN_WITH + self.open_with.len() as u32,
                    &name.chars().take(60).collect::<String>(),
                    "",
                ));
                self.open_with.push(id);
            }
            if subs.is_empty() {
                subs.push(
                    MenuLine::item(cmd::OPEN_WITH + cmd::MAX_OPEN_WITH as u32, "No applications for this type", "")
                        .enabled(false),
                );
            }
            lines.push(MenuLine::Sub { label: "Open With".into(), lines: subs });
        }
        if results {
            lines.push(MenuLine::item(cmd::SHOW_IN_FOLDER, "Show in Folder", "").enabled(one));
        }
        lines.push(MenuLine::Separator);
        lines.push(MenuLine::item(cmd::CUT, "Cut", "Ctrl+X").enabled(any));
        lines.push(MenuLine::item(cmd::COPY, "Copy", "Ctrl+C").enabled(any));
        lines.push(MenuLine::item(cmd::PASTE, "Paste", "Ctrl+V").enabled(self.here().is_some()));
        lines.push(MenuLine::item(cmd::DUPLICATE, "Duplicate", "Ctrl+D").enabled(any && !trash));
        lines.push(MenuLine::item(cmd::MAKE_LINK, "Make Link", "").enabled(one && !trash));
        lines.push(MenuLine::Separator);
        lines.push(MenuLine::item(cmd::RENAME, "Rename…", "F2").enabled(one && !trash));
        if trash {
            lines.push(MenuLine::item(cmd::RESTORE, "Put Back", "").enabled(any));
            lines.push(MenuLine::item(cmd::DELETE, "Delete Forever…", "Delete").enabled(any));
        } else {
            lines.push(MenuLine::item(cmd::TRASH, "Move to the Trash", "Delete").enabled(any));
        }
        lines.push(MenuLine::Separator);
        lines.push(MenuLine::item(cmd::COMPRESS, "Compress", "").enabled(any && !trash));
        lines.push(MenuLine::item(cmd::INFO, "Properties", "Ctrl+I").enabled(any));
        // Backup milestone 1, step 4: versions from snapshots and the backup copy.
        lines.push(MenuLine::item(cmd::VERSIONS, "Earlier Versions…", "").enabled(false));
        lines.push(MenuLine::Separator);
        lines.push(MenuLine::item(cmd::COPY_PATH, "Copy Path", "").enabled(any));
        lines.push(MenuLine::item(cmd::TERMINAL, "Open in Terminal", "").enabled(one));
        lines
    }

    /// The menu of the ground around the rows: the folder's own.
    pub(crate) fn ground_menu(&mut self) -> Vec<MenuLine> {
        let here = self.here().is_some();
        let trash = self.in_trash();
        let (by, _) = self.sort;
        let mut lines = vec![
            MenuLine::item(cmd::NEW_FOLDER, "New Folder", "Ctrl+Shift+N").enabled(here && !trash),
            MenuLine::item(cmd::NEW_FILE, "New File", "").enabled(here && !trash),
            MenuLine::item(cmd::PASTE, "Paste", "Ctrl+V").enabled(here && !trash),
            MenuLine::Separator,
            MenuLine::Sub {
                label: "Sort By".into(),
                lines: vec![
                    MenuLine::item(cmd::SORT_NAME, "Name", "").checked(by == SortBy::Name),
                    MenuLine::item(cmd::SORT_SIZE, "Size", "").checked(by == SortBy::Size),
                    MenuLine::item(cmd::SORT_CHANGED, "Date Changed", "").checked(by == SortBy::Changed),
                    MenuLine::item(cmd::SORT_KIND, "Kind", "").checked(by == SortBy::Kind),
                ],
            },
            MenuLine::Sub {
                label: "Group By".into(),
                lines: vec![
                    MenuLine::item(cmd::GROUP_KIND, "Kind", "").checked(self.list_look().group == GroupBy::Kind),
                    MenuLine::item(cmd::GROUP_DATE, "Date", "").checked(self.list_look().group == GroupBy::Date),
                    MenuLine::item(cmd::GROUP_SIZE, "Size", "").checked(self.list_look().group == GroupBy::Size),
                    MenuLine::item(cmd::GROUP_NONE, "None", "").checked(self.list_look().group == GroupBy::None),
                ],
            },
            MenuLine::item(cmd::HIDDEN, "Show Hidden Files", "Ctrl+H").checked(self.hidden),
        ];
        if trash {
            lines.push(MenuLine::Separator);
            lines.push(MenuLine::item(cmd::EMPTY_TRASH, "Empty Trash…", ""));
        }
        lines.push(MenuLine::Separator);
        lines.push(MenuLine::item(cmd::TERMINAL, "Open in Terminal", "").enabled(here));
        lines.push(MenuLine::item(cmd::INFO, "Properties", "Ctrl+I").enabled(here));
        lines
    }

    pub(crate) fn command_list(&self) -> Option<(bool, Vec<C>)> {
        if self.trash_window {
            return Some((
                false,
                vec![
                    C::role(cmd::CLOSE, Role::CloseWindow),
                    C::own(cmd::OPEN, "File", "Open", "Enter"),
                    C::own(cmd::PREVIEW, "File", "Preview", "Space").checked(self.peek_open()),
                    C::own(cmd::RESTORE, "File", "Put Back", "").featured(),
                    C::own(cmd::DELETE, "File", "Delete Forever…", "Delete"),
                    C::own(cmd::EMPTY_TRASH, "File", "Empty Trash…", "").featured(),
                    C::own(cmd::INFO, "File", "Properties", "Ctrl+I"),
                    C::role(cmd::SELECT_ALL, Role::SelectAll),
                    C::own(cmd::ROWS_SMALL, "View", "Small rows", "").checked(self.list_look().rows == RowSize::Small),
                    C::own(cmd::ROWS_MEDIUM, "View", "Medium rows", "")
                        .checked(self.list_look().rows == RowSize::Medium),
                    C::own(cmd::ROWS_LARGE, "View", "Large rows", "").checked(self.list_look().rows == RowSize::Large),
                    C::own(cmd::RELOAD, "View", "Read again", "Ctrl+R"),
                    C::role(cmd::SETTINGS, Role::Settings),
                    C::role(cmd::ABOUT, Role::About),
                ],
            ));
        }
        Some((
            false,
            vec![
                C::role(cmd::NEW_WINDOW, Role::NewWindow).featured(),
                C::role(cmd::NEW_TAB, Role::NewTab),
                C::role(cmd::CLOSE_TAB, Role::CloseTab),
                C::role(cmd::CLOSE, Role::CloseWindow),
                C::role(cmd::NEXT_TAB, Role::NextTab),
                C::role(cmd::PREV_TAB, Role::PreviousTab),
                C::own(cmd::OPEN, "File", "Open", "Enter").featured(),
                C::own(cmd::PREVIEW, "File", "Preview", "Space").checked(self.peek_open()),
                C::role(cmd::SELECT_ALL, Role::SelectAll),
                C::role(cmd::UNDO, Role::Undo),
                C::role(cmd::CUT, Role::Cut),
                C::role(cmd::COPY, Role::Copy),
                C::role(cmd::PASTE, Role::Paste),
                C::own(cmd::DUPLICATE, "Edit", "Duplicate", "Ctrl+D"),
                C::own(cmd::RENAME, "Edit", "Rename…", "F2"),
                C::own(cmd::NEW_FOLDER, "File", "New folder", "Ctrl+Shift+N").featured(),
                C::own(cmd::INFO, "File", "Properties", "Ctrl+I"),
                C::own(cmd::TRASH, "File", "Move to the Trash", "Delete"),
                C::own(cmd::DELETE, "File", "Delete for good…", "Shift+Delete"),
                C::own(cmd::EMPTY_TRASH, "File", "Empty Trash…", ""),
                C::own(cmd::EJECT, "File", "Eject", "").enabled(self.here_ejectable()),
                C::own(cmd::LIST, "View", "As a list", "Ctrl+1").checked(self.view == ViewKind::List),
                C::own(cmd::ICONS, "View", "As icons", "Ctrl+2").checked(self.view == ViewKind::Icons),
                C::own(cmd::PANE, "View", "Preview pane", "").checked(self.pane).featured(),
                C::own(cmd::HIDDEN, "View", "Hidden files", "Ctrl+H").checked(self.hidden),
                C::own(cmd::ROWS_SMALL, "View", "Small rows", "").checked(self.list_look().rows == RowSize::Small),
                C::own(cmd::ROWS_MEDIUM, "View", "Medium rows", "").checked(self.list_look().rows == RowSize::Medium),
                C::own(cmd::ROWS_LARGE, "View", "Large rows", "").checked(self.list_look().rows == RowSize::Large),
                C::own(cmd::GROUP_KIND, "View", "Group by kind", "").checked(self.list_look().group == GroupBy::Kind),
                C::own(cmd::GROUP_DATE, "View", "Group by date", "").checked(self.list_look().group == GroupBy::Date),
                C::own(cmd::GROUP_SIZE, "View", "Group by size", "").checked(self.list_look().group == GroupBy::Size),
                C::own(cmd::GROUP_NONE, "View", "No groups", "").checked(self.list_look().group == GroupBy::None),
                C::own(cmd::RELOAD, "View", "Read again", "Ctrl+R"),
                C::own(cmd::BACK, "Go", "Back", "Alt+Left"),
                C::own(cmd::FORWARD, "Go", "Forward", "Alt+Right"),
                C::own(cmd::UP, "Go", "Up", "Alt+Up"),
                C::role(cmd::SETTINGS, Role::Settings),
                C::role(cmd::ABOUT, Role::About),
            ],
        ))
    }
}
