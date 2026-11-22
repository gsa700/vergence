//! The preview in FileView's window: the preview pane at the right, the
//! pictures decoded for it, and the link to Space's preview window
//! (preview.rs). The window itself is in main.rs.

use std::path::Path;
use std::sync::Arc;

use vergence_scene::{Image, Rect};
use vergence_widgets::files::{self, Entry, Kind};
use vergence_widgets::{
    ButtonLook, Element, Length, Padding, Theme, Window, button, column, drawing, label, panel, scroll, space,
};

use crate::{Msg, PANE_W, Tab, Viewer, files_theme, preview};

impl Viewer {
    /// Asks the worker for the cursor's picture (if it is one, or a PDF's
    /// first page, and not asked already).
    pub(crate) fn want_picture(&self) {
        let Some(e) = self.cursor_entry() else { return };
        if e.dir || !vergence_thumbs::has_thumbnail(e.mime) || self.asked.borrow().as_ref() == Some(&e.path) {
            return;
        }
        let pdf = e.mime == Some("application/pdf");
        *self.asked.borrow_mut() = Some(e.path.clone());
        let (out, waker, path) = (self.picture.clone(), self.waker.clone(), e.path.clone());
        std::thread::spawn(move || {
            let img = if pdf {
                vergence_thumbs::pdf_page(&path, 1200).map(Arc::new)
            } else {
                vergence_decode::decode_file(&path, Some((1600, 1200))).ok().and_then(|p| {
                    let f = p.frames.into_iter().next()?;
                    Some(Arc::new(Image { width: p.width, height: p.height, rgba: f.rgba }))
                })
            };
            *out.lock().unwrap_or_else(|e| e.into_inner()) = Some((path, img));
            if let Some(w) = waker {
                w.wake();
            }
        });
    }

    /// The decoded picture of `path`, if the worker has it.
    pub(crate) fn picture_of(&self, path: &Path) -> Option<Arc<Image>> {
        let p = self.picture.lock().unwrap_or_else(|e| e.into_inner());
        p.as_ref().filter(|(q, _)| q == path).and_then(|(_, img)| img.clone())
    }

    /// A picture fitted into its place.
    fn fitted(img: Arc<Image>) -> Element<Msg> {
        drawing(move |scene, _, r: Rect, _t: &Theme| {
            let s = (r.w / img.width as f32).min(r.h / img.height as f32).min(4.0);
            let (w, h) = (img.width as f32 * s, img.height as f32 * s);
            let id = scene.image(img.clone());
            scene.draw_image(id, Rect::new(r.x + (r.w - w) / 2.0, r.y + (r.h - h) / 2.0, w, h), 1.0);
        })
        .width(Length::FILL)
        .height(Length::FILL)
    }

    /// What a file is, in a few lines: name, kind, size, when it changed.
    fn facts(e: &Entry) -> Vec<Element<Msg>> {
        let kind = if e.dir {
            "Folder".to_string()
        } else {
            vergence_types::describe(e.mime.unwrap_or(vergence_types::of_file(&e.path)))
        };
        let mut lines =
            vec![label(e.name.clone()).size(13.0).weight(600).wrap().width(Length::FILL), label(kind).size(11.0).dim()];
        if !e.dir {
            lines.push(label(files::size_words(e.size)).size(11.0).dim());
        } else {
            let n = std::fs::read_dir(&e.path).map(|r| r.count()).unwrap_or(0);
            lines.push(label(format!("{n} item{}", if n == 1 { "" } else { "s" })).size(11.0).dim());
        }
        if let Some(c) = e.changed {
            lines.push(label(format!("Changed {}", files::when(c))).size(11.0).dim());
        }
        lines
    }

    /// The preview of a file: its picture, the start of its text, or its mark.
    fn look_at(&self, e: &Entry, big: bool, t: &Theme) -> Element<Msg> {
        if !e.dir && vergence_thumbs::has_thumbnail(e.mime) {
            if let Some(img) = self.picture_of(&e.path) {
                return Viewer::fitted(img);
            }
            return panel(label("…").size(14.0).dim()).width(Length::FILL).height(Length::FILL);
        }
        if !e.dir
            && e.kind() == Kind::Text
            && let Some(text) = files::text_start(&e.path)
        {
            let lines: Vec<Element<Msg>> = text
                .lines()
                .take(if big { 60 } else { 24 })
                .map(|l| {
                    label(if l.is_empty() {
                        " ".to_string()
                    } else {
                        l.replace('\t', "    ").chars().take(120).collect()
                    })
                    .size(if big { 12.0 } else { 10.0 })
                    .color(t.dim)
                })
                .collect();
            return panel(column(lines).spacing(1.0).width(Length::FILL))
                .color(t.card)
                .radius(6.0)
                .padding(8.0)
                .width(Length::FILL)
                .height(Length::FILL);
        }
        panel(
            files::entry_icon_sized(e, if big { 8.0 } else { 5.0 }).width(Length::FILL).height(Length::Fixed(if big {
                150.0
            } else {
                90.0
            })),
        )
        .width(Length::FILL)
        .height(Length::FILL)
    }

    pub(crate) fn pane_view(&self, t: &Theme) -> Element<Msg> {
        let mut parts: Vec<Element<Msg>> = Vec::new();
        match self.cursor_entry() {
            Some(e) => {
                parts.push(panel(self.look_at(&e, false, t)).width(Length::FILL).height(Length::Fixed(200.0)));
                parts.extend(Viewer::facts(&e));
                if !e.dir {
                    parts.push(space().height(Length::Fixed(6.0)));
                    parts.push(
                        button(label("Open").size(12.0))
                            .look(ButtonLook::Accent)
                            .padding(Padding::hv(4.0, 12.0))
                            .on_press(Msg::OpenChosen)
                            .name("Open"),
                    );
                    if matches!(e.kind(), Kind::Picture | Kind::Text) || e.mime == Some("application/pdf") {
                        parts.push(
                            button(label("Open in Close-Up").size(12.0))
                                .padding(Padding::hv(4.0, 12.0))
                                .on_press(Msg::OpenInCloseUp)
                                .name("Open in Close-Up"),
                        );
                    }
                }
            }
            None => {
                let n = self.visible().len();
                parts.push(label(self.front_tab().map(Tab::name).unwrap_or_default()).size(13.0).weight(600));
                parts.push(label(format!("{n} item{}", if n == 1 { "" } else { "s" })).size(11.0).dim());
            }
        }
        panel(scroll(column(parts).spacing(4.0).width(Length::FILL)))
            .color(t.card)
            .radius(8.0)
            .padding(10.0)
            .width(Length::Fixed(PANE_W))
            .height(Length::FILL)
    }

    pub(crate) fn peek_open(&self) -> bool {
        self.peek.as_ref().is_some_and(|l| l.borrow().open)
    }

    /// Space: the preview opened over this window for the cursor's file, or closed.
    pub(crate) fn toggle_peek(&mut self, window: &mut Window) {
        if self.peek_open() {
            return self.close_peek();
        }
        let Some(e) = self.cursor_entry() else { return };
        let link = preview::link(e);
        window.open_window(vergence_widgets::window(preview::Peek::new(link.clone(), files_theme(&self.desktop))));
        self.peek = Some(link);
    }

    pub(crate) fn close_peek(&mut self) {
        if let Some(l) = self.peek.take() {
            l.borrow_mut().close = true;
            if let Some(w) = &self.waker {
                w.wake();
            }
        }
    }

    /// The preview follows the cursor: its file given to it when it changed.
    pub(crate) fn sync_peek(&self) {
        let Some(l) = &self.peek else { return };
        let now = self.cursor_entry();
        let mut link = l.borrow_mut();
        if !link.open || link.entry.as_ref().map(|e| &e.path) == now.as_ref().map(|e| &e.path) {
            return;
        }
        link.entry = now;
        link.serial += 1;
        drop(link);
        if let Some(w) = &self.waker {
            w.wake();
        }
    }
}
