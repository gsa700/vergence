//! Reader's commands: their numbers (cmd), what each does (`run`, the
//! document's own in `pdf_run`), Back and Forward, the Open dialog, and the
//! list the app menu shows. The window itself is in main.rs.

use std::path::Path;

use vergence_ui::{AppCommand as C, Role};
use vergence_widgets::{Program, Window};

use crate::tool::remember_tool;
use crate::{APP_ID, Content, Viewer, info, pdf};

pub(crate) mod cmd {
    pub const NEW_WINDOW: u32 = 1;
    pub const OPEN: u32 = 2;
    pub const CLOSE_TAB: u32 = 3;
    pub const ZOOM_IN: u32 = 4;
    pub const ZOOM_OUT: u32 = 5;
    pub const FIT: u32 = 6;
    pub const ACTUAL: u32 = 7;
    pub const TURN_RIGHT: u32 = 8;
    pub const TURN_LEFT: u32 = 9;
    pub const NEXT: u32 = 10;
    pub const PREVIOUS: u32 = 11;
    pub const OPEN_WITH: u32 = 14;
    pub const FIT_PAGE: u32 = 15;
    pub const SIDEBAR: u32 = 16;
    pub const COPY: u32 = 17;
    pub const SELECT_ALL: u32 = 18;
    pub const FIND: u32 = 19;
    pub const FIND_NEXT: u32 = 20;
    pub const FIND_PREVIOUS: u32 = 21;
    pub const BACK: u32 = 22;
    pub const FORWARD: u32 = 23;
    pub const GO_TO_PAGE: u32 = 24;
    pub const VIEW_CONTINUOUS: u32 = 25;
    pub const VIEW_SINGLE: u32 = 26;
    pub const VIEW_TWO: u32 = 27;
    pub const NIGHT: u32 = 28;
    pub const INFO: u32 = 29;
    pub const TOOL_TEXT: u32 = 30;
    pub const TOOL_HAND: u32 = 31;
    pub const SETTINGS: u32 = 40;
    pub const ABOUT: u32 = 41;
}

impl Viewer {
    /// Before a jump (a link, the outline, find, the page field): the page
    /// now, to come back to.
    pub(crate) fn mark(&mut self) {
        let r = self.area.get();
        let Some(t) = self.front_mut() else { return };
        let Content::Pdf(d) = &t.content else { return };
        let here = d.current(r);
        if t.back.last() != Some(&here) {
            t.back.push(here);
            if t.back.len() > 100 {
                t.back.remove(0);
            }
        }
        t.forward.clear();
    }

    /// Back (or forward) to the page gone from.
    fn go_back(&mut self, forward: bool) {
        let r = self.area.get();
        let Some(t) = self.front_mut() else { return };
        let Content::Pdf(d) = &mut t.content else { return };
        let here = d.current(r);
        let (from, to) = if forward { (&mut t.forward, &mut t.back) } else { (&mut t.back, &mut t.forward) };
        if let Some(p) = from.pop() {
            to.push(here);
            d.go_to(p, r);
        }
    }

    /// A document command (zoom, pages, text); false if none is open or the
    /// command is not one of these.
    pub(crate) fn pdf_run(&mut self, id: u32) -> bool {
        let r = self.area.get();
        let Some(d) = self.pdf_mut() else { return false };
        let z = d.zoom(r);
        match id {
            cmd::ZOOM_IN => d.set_mode(pdf::Mode::Zoom(z * 1.25), None, r),
            cmd::ZOOM_OUT => d.set_mode(pdf::Mode::Zoom(z / 1.25), None, r),
            cmd::FIT => d.set_mode(pdf::Mode::Width, None, r),
            cmd::FIT_PAGE => d.set_mode(pdf::Mode::Page, None, r),
            cmd::ACTUAL => d.set_mode(pdf::Mode::Zoom(1.0), None, r),
            cmd::NEXT => {
                let c = d.current(r);
                d.go_to(c + 1, r);
            }
            cmd::PREVIOUS => {
                let c = d.current(r);
                d.go_to(c.saturating_sub(1), r);
            }
            cmd::SIDEBAR => self.sidebar = !self.sidebar,
            cmd::TURN_RIGHT => {
                let t = (d.turn + 90) % 360;
                d.set_turn(t, r);
            }
            cmd::TURN_LEFT => {
                let t = (d.turn + 270) % 360;
                d.set_turn(t, r);
            }
            cmd::VIEW_CONTINUOUS => d.set_view(pdf::View::Continuous, r),
            cmd::VIEW_SINGLE => d.set_view(pdf::View::Single, r),
            cmd::VIEW_TWO => d.set_view(pdf::View::Two, r),
            cmd::NIGHT => d.night = !d.night,
            cmd::BACK | cmd::FORWARD => {
                self.go_back(id == cmd::FORWARD);
                self.follow_thumb();
                return true;
            }
            cmd::GO_TO_PAGE => {
                self.page_key += 1;
                self.page_focus = true;
                self.page_field = None;
                return true;
            }
            cmd::COPY | cmd::SELECT_ALL | cmd::FIND | cmd::FIND_NEXT | cmd::FIND_PREVIOUS => {
                return self.pdf_text_command(id);
            }
            _ => return false,
        }
        self.follow_thumb();
        true
    }

    /// Copy, Select All and Find.
    fn pdf_text_command(&mut self, id: u32) -> bool {
        // The first match gone to: where reading was is kept for Back.
        if matches!(id, cmd::FIND_NEXT | cmd::FIND_PREVIOUS)
            && self.front_tab().is_some_and(|t| matches!(&t.content, Content::Pdf(d) if d.found.is_none()))
        {
            self.mark();
        }
        let r = self.area.get();
        let finding = self.finding.is_some();
        let (mut copy, mut find) = (None, None);
        let Some(d) = self.pdf_mut() else { return false };
        match id {
            cmd::COPY => copy = d.selected_text(),
            cmd::SELECT_ALL => {
                let c = d.current(r);
                d.select_page(c);
            }
            cmd::FIND => {
                if !finding {
                    // Starts with the selection's text when it is short.
                    let q = d.selected_text().filter(|t| t.len() <= 80 && !t.contains('\n')).unwrap_or_default();
                    d.start_search(&q);
                    find = Some(q);
                }
            }
            cmd::FIND_NEXT | cmd::FIND_PREVIOUS => {
                let step = if id == cmd::FIND_NEXT { 1 } else { -1 };

                let next = match d.found {
                    Some(i) => i as isize + step,
                    // The first match at or after the place in view.
                    None => {
                        let c = d.current(r);
                        let se = d.shared.search.lock().unwrap_or_else(|e| e.into_inner());
                        let i = se.matches.iter().position(|m| m.0 >= c).unwrap_or(0) as isize;
                        drop(se);
                        if step < 0 { i - 1 } else { i }
                    }
                };
                d.show_match(next, r);
            }
            _ => return false,
        }
        if copy.is_some() {
            self.copy_out = copy;
        }
        if find.is_some() {
            self.finding = find;
        }
        true
    }

    fn open_dialog(&mut self, window: &mut Window) {
        let inbox = self.shared.borrow().inbox.clone();
        let waker = self.waker.clone();
        let dir = self.front_tab().and_then(|t| t.path.parent().map(Path::to_path_buf));
        let mut d = vergence_widgets::filedialog::FileDialog::open(APP_ID, "Open", self.theme(), move |paths| {
            if !paths.is_empty() {
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push(paths);
                if let Some(w) = &waker {
                    w.wake();
                }
            }
        })
        .multiple(true)
        .filters(vec![
            vergence_widgets::filedialog::Filter::of_types("PDF documents", &["application/pdf"]),
            vergence_widgets::filedialog::Filter::new("All files", &[]),
        ]);
        if let Some(dir) = dir {
            d = d.folder(&dir);
        }
        window.open_window(vergence_widgets::window(d));
    }

    /// The front file with the app chosen for it (a file that is not a PDF).
    pub(crate) fn open_with(&self) {
        let Some(t) = self.front_tab() else { return };
        let _ = std::process::Command::new("vergence-open").arg(&t.path).spawn();
    }

    pub(crate) fn run(&mut self, id: u32, window: &mut Window) {
        if self.pdf_run(id) {
            if let Some(t) = self.copy_out.take() {
                window.copy(&t);
            }
            self.place_soon(window);
            window.redraw();
            return;
        }
        match id {
            cmd::NEW_WINDOW => {
                let w = Viewer::new(self.shared.clone());
                window.open_window(vergence_widgets::window(w));
            }
            cmd::OPEN => self.open_dialog(window),
            cmd::CLOSE_TAB => {
                let id = self.front_tab().map(|t| t.id);
                match id {
                    Some(id) => self.close_tab(id, window),
                    None => window.close(),
                }
            }
            cmd::OPEN_WITH => self.open_with(),
            cmd::TOOL_TEXT | cmd::TOOL_HAND => {
                self.hand = id == cmd::TOOL_HAND;
                remember_tool(self.hand);
            }
            cmd::INFO => {
                if let Some(t) = self.front_tab()
                    && let Content::Pdf(d) = &t.content
                {
                    let meta = d.shared.meta.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
                    let w = info::Info::new(&t.path, &d.sizes, &meta, self.theme());
                    window.open_window(vergence_widgets::window(w));
                }
            }
            cmd::SETTINGS => self.open_settings(window),
            cmd::ABOUT => {
                let about = vergence_widgets::about::About::new(
                    APP_ID,
                    "Reader",
                    env!("CARGO_PKG_VERSION"),
                    "PDF documents: pages, the outline, links, find and selection, read and drawn apart from the app by Vergence's own PDF engine.",
                    self.theme(),
                );
                window.open_window(vergence_widgets::window(about));
            }
            _ => {}
        }
        window.redraw();
    }

    pub(crate) fn on_commands(&self) -> Option<(bool, Vec<C>)> {
        let mut list = vec![
            C::role(cmd::OPEN, Role::Open).featured(),
            C::role(cmd::NEW_WINDOW, Role::NewWindow),
            C::role(cmd::CLOSE_TAB, Role::CloseWindow),
        ];
        if let Some(Content::Pdf(d)) = self.front_tab().map(|t| &t.content) {
            list.extend([
                C::role(cmd::ZOOM_IN, Role::ZoomIn).featured(),
                C::role(cmd::ZOOM_OUT, Role::ZoomOut).featured(),
                C::own(cmd::FIT, "View", "Fit to the width", "0").checked(d.mode == pdf::Mode::Width),
                C::own(cmd::FIT_PAGE, "View", "The whole page", "W").checked(d.mode == pdf::Mode::Page),
                C::own(cmd::SIDEBAR, "View", "Sidebar", "S").checked(self.sidebar).featured(),
                C::role(cmd::COPY, Role::Copy),
                C::role(cmd::SELECT_ALL, Role::SelectAll),
                C::role(cmd::FIND, Role::Find).featured(),
                C::role(cmd::FIND_NEXT, Role::FindNext),
                C::own(cmd::ACTUAL, "View", "Actual size", "1"),
                C::own(cmd::NEXT, "Go", "Next page", "Right").featured(),
                C::own(cmd::PREVIOUS, "Go", "Previous page", "Left"),
                C::own(cmd::BACK, "Go", "Back", "Ctrl+["),
                C::own(cmd::FORWARD, "Go", "Forward", "Ctrl+]"),
                C::own(cmd::GO_TO_PAGE, "Go", "Go to Page…", "Ctrl+Alt+G"),
                C::own(cmd::VIEW_CONTINUOUS, "View", "Continuous", "").checked(d.view == pdf::View::Continuous),
                C::own(cmd::VIEW_SINGLE, "View", "One page", "").checked(d.view == pdf::View::Single),
                C::own(cmd::VIEW_TWO, "View", "Two pages", "").checked(d.view == pdf::View::Two),
                C::own(cmd::TURN_LEFT, "View", "Turn left", "Ctrl+L"),
                C::own(cmd::TURN_RIGHT, "View", "Turn right", "Ctrl+R").featured(),
                C::own(cmd::NIGHT, "View", "Night look", "N").checked(d.night),
                C::own(cmd::TOOL_TEXT, "Tools", "Text", "T").checked(!self.hand),
                C::own(cmd::TOOL_HAND, "Tools", "Hand", "H").checked(self.hand),
                C::own(cmd::INFO, "File", "Info", "Ctrl+I"),
            ]);
        }
        list.extend([C::role(cmd::SETTINGS, Role::Settings), C::role(cmd::ABOUT, Role::About)]);
        Some((false, list))
    }
}
