//! The sidebar's actions and its view in FileView's window: its lines, a row
//! pressed, its menus, drops among the favorites. sidebar.rs has the sidebar's
//! lines and settings; the window itself is in main.rs.

use std::path::{Path, PathBuf};

use vergence_scene::Rect;
use vergence_ui::MenuLine;
use vergence_widgets::files::{self, Entry, Place};
use vergence_widgets::toolbar::Icon;
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Theme, button, column, drawing, label, panel, row, scroll, space,
    stack,
};

use crate::{Msg, SECTION, Showing, Viewer, cmd, ops, sidebar};

/// A section's fold mark: down when open, right when folded.
pub(crate) fn fold_mark(open: bool) -> Element<Msg> {
    drawing(move |scene, _, r: Rect, t: &Theme| {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let pts = if open { [(-3.0, -1.5), (0.0, 1.5), (3.0, -1.5)] } else { [(-1.5, -3.0), (1.5, 0.0), (-1.5, 3.0)] };
        let p = |(dx, dy): (f32, f32)| vergence_scene::Point::new(cx + dx, cy + dy);
        scene.line(p(pts[0]), p(pts[1]), 1.4, t.dim);
        scene.line(p(pts[1]), p(pts[2]), 1.4, t.dim);
    })
    .width(Length::Fixed(12.0))
    .height(Length::Fixed(12.0))
}

impl Viewer {
    pub(crate) fn side_setup(&self) -> sidebar::Setup {
        sidebar::setup(&self.desktop, SECTION)
    }

    /// The sidebar's Locations: the computer, its disks, the Trash, the network's places.
    fn locations(&self) -> Vec<Place> {
        let mine: Vec<PathBuf> =
            files::home().into_iter().chain(files::user_dirs().into_iter().map(|(_, p)| p)).collect();
        self.places().into_iter().filter(|p| p.path.as_ref().is_some_and(|x| !mine.contains(x))).collect()
    }

    pub(crate) fn side_lines(&self) -> Vec<sidebar::Line> {
        sidebar::lines(&self.side_setup(), &self.locations(), &files::recent_files())
    }

    pub(crate) fn set_favorites(&mut self, favs: &[PathBuf]) {
        let items: Vec<String> = favs.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        self.write_key("favorites", Some(&sidebar::list_value(&items)));
    }

    /// A sidebar row pressed: a folder gone to, a file opened, Recent shown.
    pub(crate) fn side_go(&mut self, i: usize) {
        let Some(sidebar::Line::Item(_, item)) = self.side_lines().get(i).cloned() else { return };
        match item.path {
            None => self.go(Showing::Recent),
            Some(p) if item.file => self.open_file(&p),
            Some(p) => self.go(Showing::Folder(p)),
        }
    }

    /// A sidebar row's menu (Open in New Tab, Remove from Sidebar, Show in its
    /// folder; see docs/files-milestone-3.md); the Recent heading's: Hide Recent.
    pub(crate) fn side_menu(&self, line: &sidebar::Line) -> Vec<MenuLine> {
        use sidebar::{Line, Section};
        match line {
            Line::Head(Section::Recent) => vec![MenuLine::item(cmd::SIDE_HIDE_RECENT, "Hide Recent", "")],
            Line::Head(_) => Vec::new(),
            Line::Item(sec, item) => {
                let mut v = Vec::new();
                if !item.file {
                    v.push(MenuLine::item(cmd::SIDE_NEW_TAB, "Open in New Tab", ""));
                }
                if item.path.as_ref().is_some_and(|p| p.parent().is_some()) {
                    v.push(MenuLine::item(cmd::SIDE_SHOW, "Show in Its Folder", ""));
                }
                if *sec == Section::Favorites {
                    v.push(MenuLine::Separator);
                    v.push(MenuLine::item(cmd::SIDE_REMOVE, "Remove from Sidebar", ""));
                }
                if *sec == Section::Locations && item.path.as_deref().is_some_and(files::ejectable) {
                    v.push(MenuLine::Separator);
                    v.push(MenuLine::item(cmd::SIDE_EJECT, "Eject", ""));
                }
                v
            }
        }
    }

    /// Where a drop on the sidebar at window point `y` goes among the favorites.
    pub(crate) fn side_drop_index(&self, x: f32, y: f32) -> Option<usize> {
        let r = self.side_rect.get();
        if x < r.x || x >= r.x + r.w || y < r.y || y >= r.y + r.h {
            return None;
        }
        let lines = self.side_lines();
        sidebar::drop_index(&lines, sidebar::line_at(&lines, y - r.y + self.side_scroll))
    }

    /// The sidebar: its sections, each row of a fixed height (drops are
    /// placed by where they are), in a place that remembers where it was drawn.
    pub(crate) fn sidebar_view(&self, t: &Theme) -> Element<Msg> {
        use sidebar::{Line, Section};
        let here = self.front_tab().and_then(|t| t.dir().map(Path::to_path_buf));
        let recent_here = self.front_tab().is_some_and(|t| t.showing == Showing::Recent);
        let folded = self.side_setup().folded;
        // The disks that can be let go of (read once for the whole sidebar).
        let ejectable: Vec<PathBuf> =
            self.locations().into_iter().filter_map(|p| p.path).filter(|p| files::ejectable(p)).collect();
        let mut rows: Vec<Element<Msg>> = Vec::new();
        for (i, line) in self.side_lines().into_iter().enumerate() {
            match line {
                Line::Head(sec) => {
                    let open = !folded.contains(&sec);
                    let head = button(
                        row(vec![
                            label(sec.name()).size(11.0).dim().weight(600),
                            space().width(Length::FILL),
                            fold_mark(open),
                        ])
                        .align(Align::Center)
                        .width(Length::FILL),
                    )
                    .look(ButtonLook::Plain)
                    .padding(Padding { top: 6.0, bottom: 2.0, left: 10.0, right: 6.0 })
                    .height(Length::Fixed(sidebar::HEAD_H))
                    .width(Length::FILL)
                    .on_press(Msg::SideFold(sec))
                    .on_menu(Msg::SideMenu(i))
                    .name(format!("{} ({})", sec.name(), if open { "open" } else { "folded" }));
                    rows.push(head);
                }
                Line::Item(sec, item) => {
                    let chosen = match &item.path {
                        None => recent_here,
                        Some(p) => !item.file && Some(p) == here.as_ref(),
                    };
                    let icon = match &item.place {
                        Some(p) => files::place_icon(p),
                        None => item
                            .path
                            .clone()
                            .and_then(Entry::of)
                            .map_or(("mimes", "text"), |e| files::own_icon_name(e.kind())),
                    };
                    let mark = drawing(move |scene, _, r: Rect, _t: &Theme| {
                        let side = r.w.min(r.h);
                        if let Some(img) = files::own_icon(icon.0, icon.1, (side * 2.5).ceil() as u32) {
                            let id = scene.image(img);
                            scene.draw_image(id, Rect::new(r.x, r.y + (r.h - side) / 2.0, side, side), 1.0);
                        }
                    })
                    .width(Length::Fixed(18.0))
                    .height(Length::Fixed(18.0));
                    let mut cells = vec![mark, label(item.name.clone()).size(13.0).elide().width(Length::FILL)];
                    // A drive or a disk image: its eject button at the row's end, as Finder's.
                    if sec == Section::Locations
                        && item.path.as_deref().is_some_and(|p| ejectable.contains(&p.to_path_buf()))
                    {
                        cells.push(
                            button(vergence_widgets::toolbar::icon(Icon::Eject, 13.0, false))
                                .look(ButtonLook::Plain)
                                .padding(Padding::all(0.0))
                                .width(Length::Fixed(20.0))
                                .height(Length::Fixed(20.0))
                                .on_press(Msg::SideEject(i))
                                .name(format!("Eject {}", item.name)),
                        );
                    }
                    let line = row(cells).spacing(8.0).align(Align::Center).width(Length::FILL);
                    let mut b = button(line)
                        .look(if chosen { ButtonLook::Chosen } else { ButtonLook::Plain })
                        .padding(Padding::hv(0.0, 8.0))
                        .height(Length::Fixed(sidebar::ROW_H))
                        .width(Length::FILL)
                        .on_press(Msg::SideGo(i))
                        .on_menu(Msg::SideMenu(i))
                        .name(item.name.clone());
                    if sec == Section::Favorites {
                        b = b.on_drag(Msg::SideDrag(i));
                    }
                    rows.push(b);
                }
            }
        }
        let rect = self.side_rect.clone();
        let place = drawing(move |_, _, r: Rect, _| rect.set(r)).width(Length::FILL).height(Length::FILL);
        // On the folder's own ground, the same color as the main view, as
        // Explorer's, a line between them.
        let _ = t;
        let list = panel(scroll(column(rows).width(Length::FILL)).name("Sidebar"))
            .radius(0.0)
            .padding(sidebar::PAD)
            .width(Length::FILL)
            .height(Length::FILL);
        stack(vec![place, list]).width(Length::Fixed(self.side_w)).height(Length::FILL)
    }

    fn places(&self) -> Vec<Place> {
        let mut p = files::places();
        if let Some(trash) = ops::trash_dir().map(|d| d.join("files")) {
            // After the computer and its disks, before the network's places.
            let at = p.iter().position(|x| x.network).unwrap_or(p.len());
            p.insert(at, Place { name: "Trash".into(), path: Some(trash), network: false });
        }
        p
    }
}
