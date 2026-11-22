//! Reader's keys and the wheel: the shortcuts (`on_input`) and the pages
//! scrolled, turned and zoomed (`pdf_input`). The window itself is in
//! main.rs.

use vergence_widgets::{Input, Program, Window};

use crate::{Content, Msg, Side, Viewer, cmd, pdf};

impl Viewer {
    /// The wheel and keys: the wheel scrolls (Ctrl: zooms about the
    /// pointer); arrows, Page Up/Down and Space scroll; Home and End go to
    /// the first and last page; Left and Right go a page when it fits whole.
    fn pdf_input(&mut self, input: &Input, window: &mut Window) -> bool {
        let r = self.area.get();
        let ctrl = self.ctrl;
        if self.pdf_mut().is_none() {
            return false;
        }
        match input {
            Input::Scroll { x, y, dx, dy } => {
                let tr = self.thumb_area.get();
                if self.sidebar
                    && self.side == Side::Pages
                    && *x >= tr.x
                    && *x <= tr.x + tr.w
                    && *y >= tr.y
                    && *y <= tr.y + tr.h
                {
                    let Some(d) = self.pdf_mut() else { return false };
                    d.thumb_top += dy;
                    d.clamp_thumbs(tr);
                    window.redraw();
                    return true;
                }
                let Some(d) = self.pdf_mut() else { return false };
                if *x < r.x || *x > r.x + r.w || *y < r.y || *y > r.y + r.h {
                    return false;
                }
                if ctrl {
                    let z = d.zoom(r);
                    let k = 1.0015f32.powf(-dy.clamp(-200.0, 200.0) * 10.0);
                    d.set_mode(pdf::Mode::Zoom(z * k), Some(*y), r);
                } else {
                    d.scroll(*dy, *dx, r);
                }
            }
            Input::Key { sym, pressed: true, modifiers: m, text, .. } if !m.ctrl && !m.alt => {
                if text.as_deref() == Some("s") {
                    self.sidebar = !self.sidebar;
                    window.redraw();
                    return true;
                }
                if let Some(c) = match text.as_deref() {
                    Some("t") => Some(cmd::TOOL_TEXT),
                    Some("h") => Some(cmd::TOOL_HAND),
                    _ => None,
                } {
                    self.run(c, window);
                    return true;
                }
                if text.as_deref() == Some("n") {
                    self.pdf_run(cmd::NIGHT);
                    window.redraw();
                    return true;
                }
                let Some(d) = self.pdf_mut() else { return false };
                const LEFT: u32 = 0xff51;
                const UP: u32 = 0xff52;
                const RIGHT: u32 = 0xff53;
                const DOWN: u32 = 0xff54;
                const PAGE_UP: u32 = 0xff55;
                const PAGE_DOWN: u32 = 0xff56;
                const HOME: u32 = 0xff50;
                const END: u32 = 0xff57;
                let screen = r.h - 2.0 * pdf::MARGIN - 40.0;
                let command = match (*sym, text.as_deref().unwrap_or("")) {
                    (UP, _) => {
                        d.scroll(-48.0, 0.0, r);
                        None
                    }
                    (DOWN, _) => {
                        d.scroll(48.0, 0.0, r);
                        None
                    }
                    (PAGE_UP, _) => {
                        d.scroll(-screen, 0.0, r);
                        None
                    }
                    (PAGE_DOWN, _) => {
                        d.scroll(screen, 0.0, r);
                        None
                    }
                    (0x20, _) => {
                        d.scroll(if m.shift { -screen } else { screen }, 0.0, r);
                        None
                    }
                    (HOME, _) => {
                        d.go_to(0, r);
                        None
                    }
                    (END, _) => {
                        let last = d.pages.len() - 1;
                        d.go_to(last, r);
                        None
                    }
                    (LEFT | RIGHT, _) if d.page_fits(r) => {
                        let c = d.current(r);
                        d.go_to(if *sym == LEFT { c.saturating_sub(1) } else { c + 1 }, r);
                        None
                    }
                    (LEFT, _) => {
                        d.scroll(0.0, 48.0, r);
                        None
                    }
                    (RIGHT, _) => {
                        d.scroll(0.0, -48.0, r);
                        None
                    }
                    (_, "+" | "=") => Some(cmd::ZOOM_IN),
                    (_, "-") => Some(cmd::ZOOM_OUT),
                    (_, "0") => Some(cmd::FIT),
                    (_, "1") => Some(cmd::ACTUAL),
                    (_, "w" | "W") => Some(cmd::FIT_PAGE),
                    _ => return false,
                };
                if let Some(c) = command {
                    self.pdf_run(c);
                }
            }
            _ => return false,
        }
        self.follow_thumb();
        window.redraw();
        true
    }

    pub(crate) fn on_input(&mut self, input: &Input, window: &mut Window) -> bool {
        if let Input::Key { sym, pressed, modifiers, .. } = input {
            self.ctrl = if matches!(*sym, 0xffe3 | 0xffe4) { *pressed } else { modifiers.ctrl };
        }
        if let Input::Focus(false) = input {
            self.ctrl = false;
        }
        if self.front_tab().is_some_and(|t| matches!(t.content, Content::Locked { .. })) {
            return false;
        }
        if let Input::Key { sym, pressed: true, modifiers: m, .. } = input {
            const ESCAPE: u32 = 0xff1b;
            const F3: u32 = 0xffc0;
            const KP_ADD: u32 = 0xffab;
            const KP_SUB: u32 = 0xffad;
            let c = match *sym {
                0x63 | 0x43 if m.ctrl => Some(cmd::COPY),
                0x61 | 0x41 if m.ctrl => Some(cmd::SELECT_ALL),
                0x66 | 0x46 if m.ctrl => Some(cmd::FIND),
                0x67 | 0x47 if m.ctrl => Some(if m.shift { cmd::FIND_PREVIOUS } else { cmd::FIND_NEXT }),
                0x6f | 0x4f if m.ctrl => Some(cmd::OPEN),
                0x6e | 0x4e if m.ctrl => Some(cmd::NEW_WINDOW),
                0x77 | 0x57 if m.ctrl => Some(cmd::CLOSE_TAB),
                0x2c if m.ctrl => Some(cmd::SETTINGS),
                0x3d | 0x2b | KP_ADD if m.ctrl => Some(cmd::ZOOM_IN),
                0x2d | KP_SUB if m.ctrl => Some(cmd::ZOOM_OUT),
                0x30 if m.ctrl => Some(cmd::FIT),
                0x67 | 0x47 if m.ctrl && m.alt => Some(cmd::GO_TO_PAGE),
                0x72 | 0x52 if m.ctrl => Some(cmd::TURN_RIGHT),
                0x69 | 0x49 if m.ctrl => Some(cmd::INFO),
                0x6c | 0x4c if m.ctrl => Some(cmd::TURN_LEFT),
                0x5b if m.ctrl => Some(cmd::BACK),
                0x5d if m.ctrl => Some(cmd::FORWARD),
                0xff51 if m.alt => Some(cmd::BACK),
                0xff53 if m.alt => Some(cmd::FORWARD),
                F3 => Some(if m.shift { cmd::FIND_PREVIOUS } else { cmd::FIND_NEXT }),
                ESCAPE if self.page_focus => {
                    self.page_key += 1;
                    self.page_focus = false;
                    self.page_field = None;
                    window.redraw();
                    return true;
                }
                ESCAPE if self.is_pdf() => {
                    if self.finding.is_some() {
                        self.update(Msg::FindDone, window);
                    } else if let Some(d) = self.pdf_mut() {
                        d.selection = None;
                        window.redraw();
                    }
                    return true;
                }
                _ => None,
            };
            if let Some(c) = c {
                self.run(c, window);
                return true;
            }
        }
        if self.pdf_input(input, window) {
            self.place_soon(window);
            return true;
        }
        false
    }
}
