//! Edit's documents in tabs: a document (DocTab), reading files into tabs,
//! the tab in front, files changed on disk read again, the tab events.
//! The window itself (EditWindow, Shared, the Program wiring) is in main.rs.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use vergence_ui::TabEvent;
use vergence_widgets::{TextDoc, Window};

use crate::{EditWindow, cmd, files};

/// A file's modification time and length: changed on disk if these differ.
pub(crate) fn stamp(p: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// One document in a tab.
/// A file read as running text rather than code: an untitled document, a
/// name with no extension (not a dotfile), or a prose extension.
pub(crate) fn is_prose(path: Option<&std::path::Path>) -> bool {
    let Some(path) = path else { return true };
    // A dotfile (.zshrc, .gitignore) is configuration, not prose.
    if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')) {
        return false;
    }
    match path.extension().and_then(|e| e.to_str()) {
        None => true,
        Some(e) => matches!(
            e.to_ascii_lowercase().as_str(),
            "md" | "markdown" | "txt" | "text" | "rst" | "adoc" | "asciidoc" | "org" | "tex"
        ),
    }
}

pub(crate) struct DocTab {
    pub(crate) id: u32,
    pub(crate) doc: TextDoc,
    pub(crate) path: Option<PathBuf>,
    /// "Untitled", "Untitled 2", … for a document never saved.
    pub(crate) untitled: u32,
    pub(crate) read_only: Option<String>,
    pub(crate) bom: bool,
    /// Tab inserts spaces (from the file's own indenting, or the status line).
    pub(crate) spaces: Option<bool>,
    /// The file as last read or written (step 8: changes on disk are noticed).
    pub(crate) disk: Option<(SystemTime, u64)>,
    /// The file changed on disk while this tab had changes of its own.
    pub(crate) changed_on_disk: bool,
    /// The text's language, for its colors (None: plain text), and whether
    /// it was chosen in View > Language (then a new name does not change it).
    pub(crate) lang: Option<vergence_syntax::LangId>,
    pub(crate) lang_chosen: bool,
}

/// A document's language: by its file's extension, else its first line
/// (a `#!` line, a language's own opening). None: plain text.
pub(crate) fn language_of(path: Option<&Path>, doc: &TextDoc) -> Option<vergence_syntax::LangId> {
    let d = doc.borrow();
    let first = if d.buffer().line_count() > 0 { d.buffer().line(0) } else { "" };
    vergence_syntax::detect(path, first)
}

impl DocTab {
    pub(crate) fn name(&self) -> String {
        match &self.path {
            Some(p) => files::name(p),
            None if self.untitled <= 1 => "Untitled".into(),
            None => format!("Untitled {}", self.untitled),
        }
    }

    pub(crate) fn modified(&self) -> bool {
        self.doc.borrow().modified()
    }
}

impl EditWindow {
    pub(crate) fn new_tab_id(&self) -> u32 {
        let mut s = self.shared.borrow_mut();
        s.next_tab += 1;
        s.next_tab
    }

    pub(crate) fn untitled_tab(&self) -> DocTab {
        // The lowest number no open Untitled here has.
        let n = (1..).find(|n| !self.tabs.iter().any(|t| t.path.is_none() && t.untitled == *n)).unwrap_or(1);
        DocTab {
            id: self.new_tab_id(),
            doc: TextDoc::new(""),
            path: None,
            untitled: n,
            read_only: None,
            bom: false,
            spaces: None,
            disk: None,
            changed_on_disk: false,
            lang: None,
            lang_chosen: false,
        }
    }

    /// A tab for `path`, read in; None (and a notice) if it cannot be.
    pub(crate) fn file_tab(&mut self, path: &Path) -> Option<DocTab> {
        match files::load(path) {
            Ok(l) => {
                let doc = TextDoc::new(&l.text);
                doc.borrow_mut().mark_saved();
                let spaces = doc.borrow().buffer().uses_tabs().map(|tabs| !tabs);
                let lang = language_of(Some(path), &doc);
                Some(DocTab {
                    id: self.new_tab_id(),
                    doc,
                    path: Some(path.to_path_buf()),
                    untitled: 0,
                    read_only: l.read_only,
                    bom: l.bom,
                    spaces,
                    disk: stamp(path),
                    changed_on_disk: false,
                    lang,
                    lang_chosen: false,
                })
            }
            Err(e) => {
                self.notice = Some(e);
                None
            }
        }
    }

    /// Opens files in tabs here (one already open comes to the front instead).
    pub(crate) fn open_files(&mut self, paths: &[PathBuf]) {
        for p in paths {
            if let Some(t) = self.tabs.iter().find(|t| t.path.as_deref() == Some(p.as_path())) {
                self.front = t.id;
                continue;
            }
            if let Some(t) = self.file_tab(p) {
                // An untouched empty Untitled gives way to the first file.
                let lone = self.tabs.len() == 1
                    && self.tabs[0].path.is_none()
                    && !self.tabs[0].modified()
                    && self.tabs[0].doc.borrow().buffer().bytes() == 0;
                if lone {
                    self.tabs.clear();
                }
                self.front = t.id;
                self.tabs.push(t);
            }
        }
        self.apply_options();
    }

    pub(crate) fn front_tab(&self) -> Option<&DocTab> {
        self.tabs.iter().find(|t| t.id == self.front).or(self.tabs.first())
    }

    pub(crate) fn front_index(&self) -> usize {
        self.tabs.iter().position(|t| t.id == self.front).unwrap_or(0)
    }

    pub(crate) fn doc(&self) -> Option<TextDoc> {
        self.front_tab().map(|t| t.doc.clone())
    }

    /// Open files changed on disk: one without changes of its own is read
    /// again (the caret kept where it was); one with them asks (a strip).
    pub(crate) fn check_disk(&mut self) -> bool {
        let mut changed = false;
        for i in 0..self.tabs.len() {
            let Some(p) = self.tabs[i].path.clone() else { continue };
            let now = stamp(&p);
            if now.is_none() || now == self.tabs[i].disk || self.tabs[i].changed_on_disk {
                continue;
            }
            if self.tabs[i].modified() {
                self.tabs[i].changed_on_disk = true;
            } else {
                self.reload(i);
            }
            changed = true;
        }
        changed
    }

    /// Reads tab `i`'s file again, the caret kept on its line.
    pub(crate) fn reload(&mut self, i: usize) {
        let Some(p) = self.tabs[i].path.clone() else { return };
        match files::load(&p) {
            Ok(l) => {
                let t = &mut self.tabs[i];
                let mut d = t.doc.borrow_mut();
                let c = d.caret();
                d.set_text(&l.text);
                d.mark_saved();
                d.set_selection(c, c);
                drop(d);
                t.bom = l.bom;
                t.read_only = l.read_only;
                t.disk = stamp(&p);
                t.changed_on_disk = false;
            }
            Err(e) => self.notice = Some(e),
        }
    }

    pub(crate) fn remove_tab(&mut self, id: u32) -> Option<DocTab> {
        let i = self.tabs.iter().position(|t| t.id == id)?;
        let t = self.tabs.remove(i);
        if self.front == id
            && let Some(n) = self.tabs.get(i.min(self.tabs.len().saturating_sub(1)))
        {
            self.front = n.id;
        }
        Some(t)
    }

    pub(crate) fn step_tab(&mut self, by: isize) {
        let n = self.tabs.len() as isize;
        if n > 1 {
            let i = (self.front_index() as isize + by).rem_euclid(n) as usize;
            self.front = self.tabs[i].id;
        }
    }

    pub(crate) fn on_tab(&mut self, event: TabEvent, window: &mut Window) {
        match event {
            TabEvent::Chosen(id) => {
                self.front = id;
                if self.find.is_some() {
                    self.refind();
                }
            }
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
        self.keep_soon(window);
    }
}
