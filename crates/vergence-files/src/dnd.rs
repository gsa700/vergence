//! Drag and drop in FileView's window: rows dragged out, files dropped into
//! the folder or the Trash, favorites dropped on the sidebar. The window
//! itself is in main.rs.

use std::path::PathBuf;

use vergence_widgets::Window;

use crate::{Ask, SIDEBAR_KIND, Viewer, ops, sidebar};

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

    /// Files dragged here go into the folder shown: moved, or copied with Ctrl.
    pub(crate) fn on_drag_over(&mut self, x: f32, y: f32, kinds: &[String], ctrl: bool) -> u8 {
        // Over the sidebar's favorites: a shortcut made, or a favorite moved;
        // nothing on the disk changes (a copy, so the source keeps its files).
        if !self.trash_window && self.side_rect.get().w > 0.0 && {
            let r = self.side_rect.get();
            x >= r.x && x < r.x + r.w
        } {
            let ours = kinds.iter().any(|k| k == SIDEBAR_KIND || k == "text/uri-list");
            return if ours && self.side_drop_index(x, y).is_some() {
                vergence_ui::DROP_COPY
            } else {
                vergence_ui::DROP_NONE
            };
        }
        if !kinds.iter().any(|k| k == "text/uri-list") || self.here().is_none() || self.job.is_some() {
            return vergence_ui::DROP_NONE;
        }
        // Onto the Trash: thrown away (never copied there).
        if self.in_trash() {
            return vergence_ui::DROP_MOVE;
        }
        if ctrl { vergence_ui::DROP_COPY } else { vergence_ui::DROP_MOVE }
    }

    pub(crate) fn on_dropped(&mut self, x: f32, y: f32, action: u8, items: &[(String, Vec<u8>)], window: &mut Window) {
        // On the sidebar: into the favorites at the place dropped.
        if !self.trash_window && {
            let r = self.side_rect.get();
            r.w > 0.0 && x >= r.x && x < r.x + r.w
        } {
            if let Some(at) = self.side_drop_index(x, y) {
                let moved: Vec<PathBuf> = match items.iter().find(|(k, _)| k == SIDEBAR_KIND) {
                    Some((_, v)) => vec![PathBuf::from(String::from_utf8_lossy(v).into_owned())],
                    None => items
                        .iter()
                        .find(|(k, _)| k == "text/uri-list")
                        .map(|(_, v)| {
                            String::from_utf8_lossy(v)
                                .lines()
                                .filter_map(|l| vergence_types::path_of_link(l.trim()))
                                .collect()
                        })
                        .unwrap_or_default(),
                };
                let moved: Vec<PathBuf> = moved.into_iter().filter(|p| p.is_absolute() && p.exists()).collect();
                if !moved.is_empty() {
                    let favs = sidebar::put(&sidebar::favorites(&self.side_setup()), &moved, at);
                    self.set_favorites(&favs);
                }
            }
            window.redraw();
            return;
        }
        let Some(dir) = self.here() else { return };
        let paths: Vec<PathBuf> = items
            .iter()
            .find(|(k, _)| k == "text/uri-list")
            .map(|(_, v)| {
                String::from_utf8_lossy(v).lines().filter_map(|l| vergence_types::path_of_link(l.trim())).collect()
            })
            .unwrap_or_default();
        if paths.is_empty() {
            return;
        }
        if self.in_trash() {
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
            return;
        }
        let cut = action == vergence_ui::DROP_MOVE;
        let names = ops::clashes(&paths, &dir);
        if names.is_empty() {
            self.start_paste(paths, dir, cut, ops::Clash::KeepBoth);
        } else {
            self.ask = Some(Ask::Clash { sources: paths, dir, cut, names });
        }
        window.redraw();
    }
}
