//! FileView's window drawn: the toolbar strip, the question strips, the
//! folder as a list or icons, the foot; what is visible (hidden files, the
//! narrowing, a search below). The sidebar is drawn in sidebar_actions.rs, the
//! preview pane in pane.rs. The window itself is in main.rs.

use vergence_scene::Rect;
use vergence_widgets::files::{self, Entry, GroupBy, ListLook, RowSize};
use vergence_widgets::toolbar::{Icon, flexible, group, toggle, tool, toolbar};
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Theme, button, column, drawing, field, label, panel, row, rule,
    scroll, space,
};

use crate::{Ask, Msg, ROW_TOOL, SECTION, Showing, Viewer, cmd, ops, search};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewKind {
    List,
    Icons,
}

impl Viewer {
    /// The front tab's entries as shown: hidden ones and the narrowing left out, sorted.
    pub(crate) fn visible(&self) -> Vec<&Entry> {
        let Some(t) = self.front_tab() else { return Vec::new() };
        let n = self.narrow.to_lowercase();
        let source = if self.search.is_some() { &self.search_entries } else { &t.entries };
        // What is in the Trash is all shown: a hidden file thrown away is still there.
        let all = self.hidden || self.in_trash();
        let mut v: Vec<&Entry> = source
            .iter()
            .filter(|e| all || !e.name.starts_with('.'))
            .filter(|e| n.is_empty() || e.name.to_lowercase().contains(&n))
            .collect();
        let group = if self.shown_view() == ViewKind::List { self.list_look().group } else { GroupBy::None };
        files::sort(&mut v, self.sort.0, self.sort.1, t.showing == Showing::Recent && self.search.is_none(), group);
        v.truncate(files::MAX_ROWS);
        v
    }

    /// The narrowing words gone, and any search below with them.
    pub(crate) fn clear_narrow(&mut self) {
        self.narrow.clear();
        self.search = None;
        self.search_entries.clear();
    }

    /// Searches below the folder shown for the narrowing words (again, when
    /// they change); off when they are emptied.
    pub(crate) fn search_below(&mut self) {
        self.search = None;
        self.search_entries.clear();
        self.search_done = false;
        let (Some(here), false) = (self.here(), self.narrow.trim().is_empty()) else { return };
        let w = self.thumb_waker.clone();
        self.search = Some(search::Search::start(&here, self.narrow.trim(), self.hidden, move || {
            if let Some(w) = w.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                w.wake();
            }
        }));
    }

    /// How the list looks, from the settings (and the widths being pulled).
    pub(crate) fn list_look(&self) -> ListLook {
        let key = |k: &str| vergence_ui::table_value(&self.desktop, SECTION, k);
        ListLook {
            rows: key("row-size").map(|v| RowSize::parse(&v)).unwrap_or_default(),
            widths: self.widths,
            kind: true,
            stripes: true,
            // By kind unless said (files milestone 3); the older switch's "false" kept.
            group: match (key("group-by"), key("group-by-kind").as_deref()) {
                (Some(g), _) => GroupBy::parse(&g),
                (None, Some("false")) => GroupBy::None,
                _ => GroupBy::Kind,
            },
        }
    }

    /// The view as shown: the Trash is always a list,
    /// whatever the window's view is for folders.
    pub(crate) fn shown_view(&self) -> ViewKind {
        if self.in_trash() { ViewKind::List } else { self.view }
    }

    /// The strips above the folder: a question, a name being typed, a job's progress.
    fn strips(&self, t: &Theme) -> Vec<Element<Msg>> {
        let strip = |items: Vec<Element<Msg>>| -> Element<Msg> {
            panel(row(items).spacing(8.0).align(Align::Center).width(Length::FILL))
                .color(t.card)
                .radius(8.0)
                .padding(Padding::hv(6.0, 10.0))
                .width(Length::FILL)
        };
        let b = |text: &str, m: Msg, accent: bool| -> Element<Msg> {
            button(label(text).size(12.0))
                .look(if accent { ButtonLook::Accent } else { ButtonLook::Plain })
                .padding(Padding::hv(3.0, 10.0))
                .on_press(m)
                .name(text)
        };
        let mut out = Vec::new();
        match &self.ask {
            Some(Ask::Clash { names, .. }) => {
                let what = if names.len() == 1 {
                    format!("“{}” is already here.", names[0])
                } else {
                    format!("{} items of these names are already here.", names.len())
                };
                out.push(strip(vec![
                    label(what).size(12.0).weight(600).wrap().width(Length::FILL),
                    b("Cancel", Msg::Answer(None), false),
                    b("Skip", Msg::Answer(Some(ops::Clash::Skip)), false),
                    b("Keep both", Msg::Answer(Some(ops::Clash::KeepBoth)), false),
                    b("Replace", Msg::Answer(Some(ops::Clash::Replace)), true),
                ]));
            }
            Some(Ask::Forever(paths)) => {
                let what = if paths.len() == 1 {
                    format!("Delete “{}” for good? It cannot be undone.", self.shown_name(&paths[0]))
                } else {
                    format!("Delete {} items for good? They cannot be undone.", paths.len())
                };
                out.push(strip(vec![
                    label(what).size(12.0).weight(600).wrap().width(Length::FILL),
                    b("Cancel", Msg::Confirm(false), false),
                    b("Delete", Msg::Confirm(true), true),
                ]));
            }
            Some(Ask::EmptyTrash) => {
                out.push(strip(vec![
                    label("Empty the Trash? What is in it cannot be got back.")
                        .size(12.0)
                        .weight(600)
                        .wrap()
                        .width(Length::FILL),
                    b("Cancel", Msg::Confirm(false), false),
                    b("Empty Trash", Msg::Confirm(true), true),
                ]));
            }
            Some(Ask::Extract { why, .. }) => {
                out.push(strip(vec![
                    label(format!("{why} Extract it anyway?")).size(12.0).weight(600).wrap().width(Length::FILL),
                    b("Cancel", Msg::Confirm(false), false),
                    b("Extract anyway", Msg::Confirm(true), true),
                ]));
            }
            None => {}
        }
        if let Some(n) = self.naming.as_ref().filter(|n| n.rename.is_none() || self.renaming_row().is_none()) {
            out.push(strip(vec![
                label(if n.rename.is_some() {
                    "New name"
                } else if n.file {
                    "New file"
                } else {
                    "New folder"
                })
                .size(12.0),
                field(n.text.clone(), Msg::NameTyped)
                    .on_submit(Msg::NameDone)
                    .autofocus()
                    .width(Length::FILL)
                    .name("Name"),
                b("Cancel", Msg::NameCancel, false),
                b(if n.rename.is_some() { "Rename" } else { "Make" }, Msg::NameDone, true),
            ]));
        }
        if let Some(job) = &self.job {
            use std::sync::atomic::Ordering::Relaxed;
            let (done, total) = (job.progress.done.load(Relaxed), job.progress.total.load(Relaxed).max(1));
            let part = (done as f32 / total as f32).clamp(0.0, 1.0);
            let now = job.progress.now.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let bar = drawing(move |scene, _, r: Rect, t: &Theme| {
                let r = Rect::new(r.x, r.y + (r.h - 6.0) / 2.0, r.w, 6.0);
                scene.fill_rounded(r, 3.0, t.line);
                scene.fill_rounded(Rect::new(r.x, r.y, r.w * part, r.h), 3.0, t.accent);
            })
            .width(Length::Fixed(160.0))
            .height(Length::Fixed(16.0));
            out.push(strip(vec![
                label(format!(
                    "{} · {:.0}% · {} of {}",
                    job.words,
                    part * 100.0,
                    files::size_words(done),
                    files::size_words(total)
                ))
                .size(12.0),
                bar,
                label(now).size(11.0).dim().elide().width(Length::FILL),
                b("Stop", Msg::StopJob, false),
            ]));
        }
        out
    }

    fn tool_strip(&self, t: &Theme) -> Element<Msg> {
        let tab = self.front_tab();
        let can_back = tab.is_some_and(|t| !t.back.is_empty());
        let can_forward = tab.is_some_and(|t| !t.forward.is_empty());
        // The Trash is a place of its own: nothing above it to go up to.
        let can_up = !self.in_trash() && tab.and_then(|t| t.dir()).is_some_and(|d| d.parent().is_some());
        let mut items = vec![group(vec![
            tool(Icon::Back, "Back", can_back.then_some(Msg::Run(cmd::BACK))),
            tool(Icon::Forward, "Forward", can_forward.then_some(Msg::Run(cmd::FORWARD))),
            tool(Icon::Up, "Up", can_up.then_some(Msg::Run(cmd::UP))),
        ])];
        // The Trash window goes nowhere: its name is in its title bar.
        if self.trash_window {
            items.clear();
        }
        // The path (see docs/files-milestone-3.md): the folder's real path in a
        // box drawn as the field is, since the folder's name is right above it
        // in the tab; a click on it, or Ctrl+L,
        // makes it the field to type a path in (Enter goes there, Escape lets
        // go). Not a field all the time: the folder's keys (Backspace goes
        // up, Delete trashes) must not act while a path is being typed.
        if !self.trash_window {
            match &self.path_edit {
                // A key of its own: the box stood in its place, and a field
                // taken for the same widget would not take the keyboard.
                Some(text) => items.push(
                    field(text.clone(), Msg::PathTyped)
                        .on_submit(Msg::PathGo)
                        .autofocus()
                        .key(0x7061_7468)
                        .width(Length::FILL)
                        .name("Path"),
                ),
                None => {
                    let shown = match tab.map(|t| &t.showing) {
                        Some(Showing::Folder(dir)) => dir.display().to_string(),
                        _ => "Recent".to_string(),
                    };
                    let edge = if t.high_contrast { t.text } else { t.line };
                    items.push(
                        button(
                            panel(label(shown).elide().width(Length::FILL))
                                .color(t.page)
                                .edge(edge, 1.0)
                                .padding(Padding::hv(0.0, 8.0))
                                .align(Align::Center)
                                .width(Length::FILL)
                                .height(Length::Fixed(26.0)),
                        )
                        .look(ButtonLook::Plain)
                        .padding(Padding::all(0.0))
                        .width(Length::FILL)
                        .on_press(Msg::PathEdit)
                        .name("Path"),
                    );
                }
            }
        } else {
            items.push(flexible());
        }
        let any = tab.is_some_and(|t| !t.chosen.is_empty());
        let trash = self.in_trash();
        // The views and the panes, then what is done here, then the filter
        // at the right (files milestone 3, step 2: Explorer's order).
        if !trash {
            items.push(group(vec![
                toggle(Icon::List, "As a list", self.view == ViewKind::List, Msg::Run(cmd::LIST)),
                toggle(Icon::Grid, "As icons", self.view == ViewKind::Icons, Msg::Run(cmd::ICONS)),
            ]));
        }
        if !self.trash_window {
            let mut panes = vec![toggle(Icon::Pane, "Preview pane", self.pane, Msg::Run(cmd::PANE))];
            if !trash {
                panes.push(toggle(Icon::Hidden, "Hidden files", self.hidden, Msg::Run(cmd::HIDDEN)));
            }
            items.push(group(panes));
        }
        if trash {
            // The chosen ones put back or deleted; the whole Trash emptied
            // (right click and each row have them too).
            let full = tab.is_some_and(|t| !t.entries.is_empty());
            items.push(
                button(label("Put Back").size(12.0))
                    .padding(Padding::hv(3.0, 10.0))
                    .on_press_if(any, Msg::Run(cmd::RESTORE))
                    .name("Put Back"),
            );
            items.push(
                button(label("Delete Forever…").size(12.0))
                    .padding(Padding::hv(3.0, 10.0))
                    .on_press_if(any, Msg::Run(cmd::DELETE))
                    .name("Delete Forever"),
            );
            items.push(
                button(label("Empty Trash…").size(12.0))
                    .padding(Padding::hv(3.0, 10.0))
                    .on_press_if(full, Msg::Run(cmd::EMPTY_TRASH))
                    .name("Empty Trash"),
            );
        } else {
            items.push(group(vec![
                tool(Icon::NewFolder, "New folder", self.here().is_some().then_some(Msg::Run(cmd::NEW_FOLDER))),
                tool(Icon::Trash, "Move to the Trash", any.then_some(Msg::Run(cmd::TRASH))),
            ]));
        }
        // With words typed: search the folders below too.
        if !self.narrow.trim().is_empty() && self.here().is_some() {
            let on = self.search.is_some();
            items.push(
                button(label("In subfolders").size(11.5))
                    .look(if on { ButtonLook::Chosen } else { ButtonLook::Plain })
                    .padding(Padding::hv(4.0, 8.0))
                    .on_press(Msg::SearchBelow)
                    .name("Search in subfolders"),
            );
        }
        items.push(
            field(self.narrow.clone(), Msg::Narrow)
                .placeholder(if trash { "Narrow the Trash" } else { "Narrow this folder" })
                .width(Length::Fixed(200.0))
                .name("Narrow"),
        );
        toolbar(items, t)
    }

    fn folder_view(&self, t: &Theme) -> Element<Msg> {
        let Some(tab) = self.front_tab() else { return space() };
        let visible = self.visible();
        let chosen = |_: usize, e: &Entry| tab.chosen.contains(&e.path);
        let click = |k: usize| Msg::Click(k);
        let in_view = tab
            .cursor
            .as_ref()
            .and_then(|c| visible.iter().position(|e| &e.path == c))
            .map(|k| (k, tab.turn << 32 | k as u64));
        let drag = |k: usize| Msg::DragRow(k);
        let menu = |k: usize| Msg::RowMenu(k);
        let look = self.list_look();
        let folder_size = |e: &Entry| self.folder_size(e);
        let grip: fn(usize, vergence_widgets::Touch) -> Option<Msg> = |c, t| Some(Msg::Grip(c, t));
        // The icon view's thumbnails: this folder's pictures asked for, in
        // their order; what other folders asked and has not begun is dropped.
        if self.shown_view() == ViewKind::Icons {
            self.thumbs.forget_waiting();
            for e in visible.iter().filter(|e| !e.dir && vergence_thumbs::has_thumbnail(e.mime)) {
                self.thumbs.want(&e.path, e.changed);
            }
        }
        let thumb = |e: &Entry| self.thumbs.get(&e.path, e.changed).flatten();
        // The Trash: where each came from under its name, when it was
        // deleted, and its own Put Back and Delete Forever, shown on the row
        // under the pointer and on the chosen ones (no hover on a touch screen).
        let trash = self.in_trash();
        let under = |e: &Entry| tab.from.get(&e.path).and_then(|p| p.parent()).map(files::tilde);
        let row_tools = |k: usize, e: &Entry| -> Element<Msg> {
            let small = |which: Icon, name: &str, m: Msg| {
                button(vergence_widgets::toolbar::icon(which, 14.0, false))
                    .look(ButtonLook::Plain)
                    .padding(Padding::all(0.0))
                    .width(Length::Fixed(ROW_TOOL))
                    .height(Length::Fixed(ROW_TOOL))
                    .on_press(m)
                    .name(name)
            };
            let tools = row(vec![
                small(Icon::PutBack, "Put Back", Msg::RowPutBack(k)),
                small(Icon::Remove, "Delete Forever", Msg::RowForever(k)),
            ])
            .spacing(4.0)
            .align(Align::Center);
            if tab.chosen.contains(&e.path) { tools } else { tools.shown_on_hover() }
        };
        let v = files::View {
            entries: &visible,
            chosen: &chosen,
            in_view,
            click: &click,
            drag: Some(&drag),
            folders: trash || tab.showing == Showing::Recent || self.search.is_some(),
            thumb: if self.shown_view() == ViewKind::Icons { Some(&thumb) } else { None },
            menu: Some(&menu),
            look: &look,
            folder_size: Some(&folder_size),
            grip: Some(grip),
            under: if trash { Some(&under) } else { None },
            changed_heading: trash.then_some("Deleted"),
            trailing: if trash { Some((&row_tools, 2.0 * ROW_TOOL + 4.0)) } else { None },
            rename: self.renaming_row().and_then(|row| {
                self.naming.as_ref().map(|n| files::Rename {
                    row,
                    text: n.text.clone(),
                    typed: Msg::NameTyped,
                    done: Msg::NameDone,
                })
            }),
        };
        let mut body: Vec<Element<Msg>> = Vec::new();
        if let Some(p) = &tab.problem {
            body.push(label(p.clone()).size(12.0).wrap().width(Length::FILL));
        }
        if visible.is_empty() && tab.problem.is_none() {
            let why = if self.search.is_some() && !self.search_done {
                "Searching…"
            } else if self.search.is_some() {
                "Nothing here or below has that in its name."
            } else if !self.narrow.is_empty() {
                "Nothing here has that in its name."
            } else if tab.showing == Showing::Recent {
                "Nothing opened or saved yet."
            } else if trash {
                "The Trash is empty."
            } else {
                "This folder is empty."
            };
            body.push(panel(label(why).size(12.0).dim()).padding(12.0));
        }
        let list = match self.shown_view() {
            ViewKind::List => {
                let (head, rows) = files::list(&v, self.sort, Msg::Sort);
                let mut all = vec![head, rule()];
                all.extend(body);
                all.push(scroll(column(rows).spacing(1.0).width(Length::FILL)).name("Folder"));
                column(all).spacing(2.0).width(Length::FILL).height(Length::FILL)
            }
            ViewKind::Icons => {
                body.push(scroll(panel(files::icons(&v)).padding(6.0).width(Length::FILL)).name("Folder"));
                column(body).spacing(2.0).width(Length::FILL).height(Length::FILL)
            }
        };
        // The Trash window's folder is a page of its own; FileView's shares
        // its page with the sidebar (view()).
        let p = panel(list).padding(4.0).width(Length::FILL).height(Length::FILL).on_menu(Msg::GroundMenu);
        if self.trash_window { p.color(t.page).radius(8.0).edge(t.line, 1.0) } else { p.radius(0.0) }
    }

    pub(crate) fn window_view(&self, t: &Theme) -> Element<Msg> {
        self.sync_peek();
        let mut middle = if self.trash_window {
            vec![self.folder_view(t)]
        } else {
            // The sidebar and the folder on one page, a line between them that
            // is pulled to change the sidebar's width (see docs/files-milestone-3.md).
            let rect = self.divider_rect.clone();
            let divider = drawing(move |scene, _, r: Rect, t: &Theme| {
                rect.set(r);
                scene.fill(Rect::new(r.x + r.w / 2.0 - 0.5, r.y, 1.0, r.h), t.line);
            })
            .on_touch(|touch| Some(Msg::SideGrip(touch)))
            .cursor(vergence_ui::Cursor::ColResize)
            .width(Length::Fixed(7.0))
            .height(Length::FILL)
            .name("Sidebar width");
            let edge = if t.high_contrast { t.text } else { t.line };
            vec![
                panel(
                    row(vec![self.sidebar_view(t), divider, self.folder_view(t)])
                        .width(Length::FILL)
                        .height(Length::FILL),
                )
                .color(t.page)
                .radius(8.0)
                .edge(edge, 1.0)
                .width(Length::FILL)
                .height(Length::FILL),
            ]
        };
        if self.pane && !self.trash_window {
            middle.push(self.pane_view(t));
        }
        let middle: Element<Msg> = row(middle).spacing(8.0).width(Length::FILL).height(Length::FILL);
        let mut all = vec![self.tool_strip(t)];
        if let Some(n) = &self.notice {
            all.push(
                panel(
                    row(vec![
                        label(n.clone()).size(12.0).wrap().width(Length::FILL),
                        button(label("OK").size(12.0))
                            .padding(Padding::hv(2.0, 10.0))
                            .on_press(Msg::DismissNotice)
                            .name("OK"),
                    ])
                    .spacing(8.0)
                    .align(Align::Center)
                    .width(Length::FILL),
                )
                .padding(Padding::hv(6.0, 12.0))
                .width(Length::FILL),
            );
        }
        let strips = self.strips(t);
        if !strips.is_empty() {
            all.push(
                panel(column(strips).spacing(4.0).width(Length::FILL))
                    .padding(Padding { top: 8.0, bottom: 0.0, left: 8.0, right: 8.0 })
                    .radius(0.0)
                    .width(Length::FILL),
            );
        }
        all.push(
            panel(middle)
                .padding(Padding { top: 8.0, bottom: 8.0, left: 8.0, right: 8.0 })
                .radius(0.0)
                .width(Length::FILL)
                .height(Length::FILL),
        );
        // The foot: how many, how many chosen.
        let n = self.visible().len();
        let chosen = self.front_tab().map_or(0, |t| t.chosen.len());
        let mut foot = if chosen > 1 {
            format!("{chosen} of {n} chosen")
        } else {
            format!("{n} item{}", if n == 1 { "" } else { "s" })
        };
        if self.search.is_some() {
            foot = format!(
                "{foot} {} subfolders{}",
                if self.search_done { "found in" } else { "found so far in" },
                if self.search_entries.len() >= search::MAX { " (the first 5000)" } else { "" }
            );
        }
        all.push(rule());
        // How many and what is chosen at the left; the disk's free space at
        // the right (files milestone 3, step 2).
        let free = if self.trash_window { None } else { self.free_here() };
        let free = free.map(|n| format!("{} free", files::size_words(n))).unwrap_or_default();
        all.push(
            panel(
                row(vec![label(foot).size(12.0).dim(), space().width(Length::FILL), label(free).size(12.0).dim()])
                    .width(Length::FILL),
            )
            .radius(0.0)
            .padding(Padding { top: 3.0, bottom: 3.0, left: 12.0, right: 12.0 })
            .width(Length::FILL),
        );
        column(all).width(Length::FILL).height(Length::FILL)
    }
}
