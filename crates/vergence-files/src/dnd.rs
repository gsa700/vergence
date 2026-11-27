//! Drag and drop in FileView's window: rows dragged out, files dropped into
//! the folder under the pointer (a folder row, a sidebar place) or the one
//! shown, or the Trash; shortcuts and favorites dropped between sidebar rows. The window
//! itself is in main.rs.

use std::path::{Path, PathBuf};

use vergence_widgets::Window;

use crate::{Ask, Msg, SIDEBAR_KIND, Viewer, ops, sidebar};

impl Viewer {
    /// A row pulled away: what is chosen goes (this row alone if it is not
    /// among them), as file links and as paths.
    pub(crate) fn drag_row(&mut self, k: usize, window: &mut Window) {
        let visible: Vec<PathBuf> = self.visible().iter().map(|e| e.path.clone()).collect();
        let Some(p) = visible.get(k).cloned() else { return };
        let Some(t) = self.front_mut() else { return };
        if !t.chosen.contains(&p) {
            t.chosen = vec![p.clone()];
            t.cursor = Some(p);
        }
        let paths = t.chosen.clone();
        self.dragging = paths.clone();
        let uris: String = paths.iter().map(|p| format!("file://{}\r\n", ops::escape(p))).collect();
        let plain = paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n");
        if uris.len() + plain.len() > vergence_protocol::window::MAX_DRAG_BYTES {
            self.notice = Some("Too many files to drag at once: Cut and Paste them.".into());
            return;
        }
        let label = match paths.as_slice() {
            [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            many => format!("{} items", many.len()),
        };
        window.start_drag(
            &label,
            vec![("text/uri-list".into(), uris.into_bytes()), ("text/plain".into(), plain.into_bytes())],
        );
    }

    /// Where a drop at (x, y) goes, with `on` the menu message of the row
    /// under it and `sources` what is dropped if it is known (a drag from
    /// this window, or the drop itself); the list row and sidebar line it
    /// lands on, for their highlight.
    fn dest(&self, x: f32, y: f32, kinds: &[String], on: Option<&Msg>, sources: &[PathBuf]) -> Landing {
        let r = self.side_rect.get();
        if !self.trash_window && r.w > 0.0 && x >= r.x && x < r.x + r.w {
            // A favorite dragged: moved among the favorites, as before.
            if kinds.iter().any(|k| k == SIDEBAR_KIND) {
                return match self.side_drop_index(x, y) {
                    Some(at) => Landing::to(Dest::Favorites(at)),
                    None => Landing::to(Dest::Nothing),
                };
            }
            if !kinds.iter().any(|k| k == "text/uri-list") || y < r.y || y >= r.y + r.h {
                return Landing::to(Dest::Nothing);
            }
            let lines = self.side_lines();
            return match sidebar::drop_at(&lines, y - r.y + self.side_scroll) {
                sidebar::Drop::Into(i) => {
                    let Some(sidebar::Line::Item(_, item)) = lines.get(i) else { return Landing::to(Dest::Nothing) };
                    let Some(p) = item.path.clone() else { return Landing::to(Dest::Nothing) };
                    let trash = ops::trash_dir().map(|d| d.join("files"));
                    let dest = if trash.as_ref() == Some(&p) { Dest::Trash } else { Dest::Folder(p) };
                    Landing { dest: self.checked(dest, sources), row: None, line: Some(i) }
                }
                sidebar::Drop::Between(at) => Landing::to(Dest::Favorites(at)),
                sidebar::Drop::Nothing => Landing::to(Dest::Nothing),
            };
        }
        if !kinds.iter().any(|k| k == "text/uri-list") || self.job.is_some() {
            return Landing::to(Dest::Nothing);
        }
        // On a folder's row: into that folder (Finder's, Explorer's).
        if let Some(Msg::RowMenu(k)) = on
            && let Some(e) = self.visible().get(*k).filter(|e| e.dir)
            && !self.in_trash()
        {
            let dest = self.checked(Dest::Folder(e.path.clone()), sources);
            let row = (dest != Dest::Nothing).then_some(*k);
            return Landing { dest, row, line: None };
        }
        // Anywhere else: the folder shown (or the Trash, thrown away).
        let Some(here) = self.here() else { return Landing::to(Dest::Nothing) };
        let dest = if self.in_trash() { Dest::Trash } else { Dest::Folder(here) };
        Landing::to(self.checked(dest, sources))
    }

    /// Nothing, where a drop would do nothing (Dave, 2026-11-26: a drop on
    /// a folder row did nothing, with no word why): so the pointer says no
    /// before the button is let go.
    fn checked(&self, dest: Dest, sources: &[PathBuf]) -> Dest {
        match &dest {
            Dest::Folder(d) if pointless(sources, d) => Dest::Nothing,
            Dest::Folder(d) if std::fs::metadata(d).is_ok_and(|m| m.permissions().readonly()) => Dest::Nothing,
            Dest::Trash => {
                let trash = ops::trash_dir();
                if !sources.is_empty() && sources.iter().all(|p| trash.as_ref().is_some_and(|t| p.starts_with(t))) {
                    Dest::Nothing
                } else {
                    dest
                }
            }
            _ => dest,
        }
    }

    /// Files dragged over: into the folder under the pointer, or the one
    /// shown; moved, or copied with Ctrl; onto a favorite's edge, a shortcut.
    pub(crate) fn on_drag_over(&mut self, x: f32, y: f32, kinds: &[String], ctrl: bool, on: Option<Msg>) -> u8 {
        let sources = self.dragging.clone();
        let l = self.dest(x, y, kinds, on.as_ref(), &sources);
        self.drop_on = l.row;
        self.side_drop_on = l.line;
        match l.dest {
            Dest::Nothing => vergence_ui::DROP_NONE,
            // Onto the Trash: thrown away (never copied there).
            Dest::Trash => vergence_ui::DROP_MOVE,
            // A shortcut: nothing on the disk changes (a copy, so the source keeps its files).
            Dest::Favorites(_) => vergence_ui::DROP_COPY,
            Dest::Folder(_) if ctrl => vergence_ui::DROP_COPY,
            Dest::Folder(_) => vergence_ui::DROP_MOVE,
        }
    }

    /// The drag went away: no row lit.
    pub(crate) fn on_drag_left(&mut self) {
        self.drop_on = None;
        self.side_drop_on = None;
    }

    pub(crate) fn on_dropped(
        &mut self,
        x: f32,
        y: f32,
        action: u8,
        items: &[(String, Vec<u8>)],
        on: Option<Msg>,
        window: &mut Window,
    ) {
        self.on_drag_left();
        let kinds: Vec<String> = items.iter().map(|(k, _)| k.clone()).collect();
        let paths: Vec<PathBuf> = items
            .iter()
            .find(|(k, _)| k == "text/uri-list")
            .map(|(_, v)| {
                String::from_utf8_lossy(v).lines().filter_map(|l| vergence_types::path_of_link(l.trim())).collect()
            })
            .unwrap_or_default();
        let dest = self.dest(x, y, &kinds, on.as_ref(), &paths).dest;
        match dest {
            Dest::Nothing => {}
            Dest::Favorites(at) => {
                let moved: Vec<PathBuf> = match items.iter().find(|(k, _)| k == SIDEBAR_KIND) {
                    Some((_, v)) => vec![PathBuf::from(String::from_utf8_lossy(v).into_owned())],
                    None => paths,
                };
                let moved: Vec<PathBuf> = moved.into_iter().filter(|p| p.is_absolute() && p.exists()).collect();
                if !moved.is_empty() {
                    let favs = sidebar::put(&sidebar::favorites(&self.side_setup()), &moved, at);
                    self.set_favorites(&favs);
                }
            }
            Dest::Trash => {
                // What is in the Trash already stays as it is.
                let trash = ops::trash_dir();
                for p in paths.iter().filter(|p| !trash.as_ref().is_some_and(|t| p.starts_with(t))) {
                    if let Err(e) = ops::trash(p) {
                        self.notice = Some(format!(
                            "“{}” could not be moved to the Trash: {e}",
                            p.file_name().unwrap_or_default().to_string_lossy()
                        ));
                        break;
                    }
                }
                self.read_all();
            }
            Dest::Folder(dir) => {
                // A folder never goes into itself or below itself.
                let paths: Vec<PathBuf> = paths.into_iter().filter(|p| !(dir == *p || dir.starts_with(p))).collect();
                if paths.is_empty() {
                    self.notice = Some("A folder cannot go into itself.".into());
                } else {
                    let cut = action == vergence_ui::DROP_MOVE;
                    let names = ops::clashes(&paths, &dir);
                    if names.is_empty() {
                        self.start_paste(paths, dir, cut, ops::Clash::KeepBoth);
                    } else {
                        self.ask = Some(Ask::Clash { sources: paths, dir, cut, names });
                    }
                }
            }
        }
        window.redraw();
    }
}

/// Where a drop goes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Dest {
    Folder(PathBuf),
    Trash,
    /// Among the favorites, at this place (a shortcut, or a favorite moved).
    Favorites(usize),
    Nothing,
}

/// A drop's place, and the list row or sidebar line lit for it.
struct Landing {
    dest: Dest,
    row: Option<usize>,
    line: Option<usize>,
}

impl Landing {
    fn to(dest: Dest) -> Landing {
        Landing { dest, row: None, line: None }
    }
}

/// Whether dropping `sources` into `dest` would do nothing or harm: each is
/// already there, or `dest` is one of them or inside one (a folder into
/// itself). Unknown sources (a drag from elsewhere, before its drop) are
/// not pointless.
pub(crate) fn pointless(sources: &[PathBuf], dest: &Path) -> bool {
    !sources.is_empty()
        && (sources.iter().all(|p| p.parent() == Some(dest))
            || sources.iter().any(|p| dest == p.as_path() || dest.starts_with(p)))
}
