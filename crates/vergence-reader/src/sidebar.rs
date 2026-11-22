//! Reader's sidebar and Find bar: the pages' thumbnails, the outline and
//! find's results, the thumbnails kept on the current page. The window
//! itself is in main.rs.

use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, button, column, drawing, field, label, panel, row, rule, scroll,
};

use crate::{Msg, Viewer, cmd, pdf};

/// What the sidebar shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Side {
    Pages,
    Contents,
    Results,
}

impl Viewer {
    /// The sidebar's thumbnails kept on the current page.
    pub(crate) fn follow_thumb(&mut self) {
        let (r, tr) = (self.area.get(), self.thumb_area.get());
        if let Some(d) = self.pdf_mut() {
            let c = d.current(r);
            d.show_thumb(c, tr);
        }
    }

    /// The sidebar: the pages' thumbnails, the outline, or find's results.
    pub(crate) fn sidebar_view(&self, d: &pdf::Pdf) -> Element<Msg> {
        let outline = d.shared.outline.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let has_outline = outline.as_ref().is_some_and(|o| !o.is_empty());
        let searching = self.finding.as_ref().is_some_and(|q| !q.trim().is_empty());
        let side = match self.side {
            Side::Contents if !has_outline => Side::Pages,
            Side::Results if !searching => Side::Pages,
            s => s,
        };
        let contents = side == Side::Contents;
        let tab = |text: &str, on: bool, m: Msg| {
            button(label(text).size(12.0))
                .look(if on { ButtonLook::Chosen } else { ButtonLook::Plain })
                .padding(Padding::hv(3.0, 10.0))
                .on_press(m)
                .name(text)
        };
        let mut tabs = vec![tab("Pages", side == Side::Pages, Msg::SideTab(Side::Pages))];
        if has_outline {
            tabs.push(tab("Contents", contents, Msg::SideTab(Side::Contents)));
        }
        if searching {
            tabs.push(tab("Results", side == Side::Results, Msg::SideTab(Side::Results)));
        }
        let head = panel(row(tabs).spacing(6.0)).radius(0.0).padding(Padding::hv(6.0, 8.0)).width(Length::FILL);
        let body: Element<Msg> = if side == Side::Results {
            let se = d.shared.search.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let items: Vec<Element<Msg>> = se
                .matches
                .iter()
                .zip(se.snippets.iter())
                .enumerate()
                .take(2000)
                .map(|(i, ((page, _), text))| {
                    let look = if d.found == Some(i) { ButtonLook::Chosen } else { ButtonLook::Plain };
                    let head = label(format!("Page {}", page + 1)).size(11.0);
                    let head = if d.found == Some(i) { head } else { head.dim() };
                    let lines = column(vec![head, label(text.clone()).size(12.0).wrap().width(Length::FILL)])
                        .spacing(1.0)
                        .width(Length::FILL);
                    let mut b = button(lines)
                        .look(look)
                        .padding(Padding::hv(4.0, 8.0))
                        .width(Length::FILL)
                        .on_press(Msg::Result(i))
                        .name(format!("Page {}: {text}", page + 1));
                    if d.found == Some(i) {
                        b = b.key(2_000_000 + i as u64).keep_in_view();
                    }
                    b
                })
                .collect();
            if items.is_empty() {
                panel(label(if se.done { "Not found" } else { "Searching…" }).size(12.0).dim())
                    .padding(12.0)
                    .width(Length::FILL)
                    .height(Length::FILL)
            } else {
                scroll(column(items).spacing(1.0).width(Length::FILL))
            }
        } else if contents {
            // The entry for the place in view: the last one starting at or
            // above a quarter of the way down the window.
            let r = self.area.get();
            let (tops, z) = (d.tops(), d.zoom(r));
            let view = d.top + (r.h * 0.25) / z;
            let here = outline.as_ref().and_then(|o| {
                o.iter().rposition(|e| {
                    e.target.is_some_and(|t| tops.get(t.page).is_some_and(|y| y + t.top.unwrap_or(0.0) <= view))
                })
            });
            let items: Vec<Element<Msg>> = outline
                .iter()
                .flat_map(|o| o.iter().enumerate().take(4000))
                .map(|(i, e)| {
                    let indent = 8.0 + 12.0 * e.level.min(6) as f32;
                    let look = if Some(i) == here { ButtonLook::Chosen } else { ButtonLook::Plain };
                    let title = if e.title.is_empty() { "(untitled)".to_string() } else { e.title.clone() };
                    let mut b = button(label(title.clone()).size(12.0).elide().align(Align::Start).width(Length::FILL))
                        .look(look)
                        .padding(Padding { top: 3.0, bottom: 3.0, left: indent, right: 8.0 })
                        .width(Length::FILL)
                        .name(title);
                    if e.target.is_some() {
                        b = b.on_press(Msg::Outline(i));
                    }
                    if Some(i) == here {
                        // A new key each time the place moves: scrolled into sight.
                        b = b.key(1_000_000 + i as u64).keep_in_view();
                    }
                    b
                })
                .collect();
            scroll(column(items).spacing(1.0).width(Length::FILL))
        } else {
            let layout = d.0.clone();
            let (scale, area, tarea) = (self.scale, self.area.clone(), self.thumb_area.clone());
            let waker = self.waker.clone();
            let held = self.thumb_bar_drag.is_some();
            drawing(move |scene, _, r, theme| {
                if tarea.get() != r {
                    tarea.set(r);
                    if let Some(w) = &waker {
                        w.wake();
                    }
                }
                let current = layout.current(area.get());
                layout.draw_thumbs(scene, r, scale, current, theme.page, theme.accent, theme.text, held);
            })
            .on_touch(|t| Some(Msg::ThumbTouch(t)))
            .name("Page thumbnails")
            .value(format!("Page {} of {} marked", d.current(self.area.get()) + 1, d.pages.len()))
            .width(Length::FILL)
            .height(Length::FILL)
        };
        column(vec![head, rule(), body]).width(Length::Fixed(pdf::THUMB_W + 48.0)).height(Length::FILL)
    }

    /// The Find bar: the query, how it is going, previous and next.
    pub(crate) fn find_bar(&self, q: &str, d: &pdf::Pdf) -> Element<Msg> {
        let se = d.shared.search.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let n = se.matches.len();
        let how = if se.query.is_empty() {
            String::new()
        } else if se.done && se.chars_seen == 0 {
            "This PDF has no text to search: its pages are pictures (a scan).".to_string()
        } else if se.done && n == 0 {
            "Not found".to_string()
        } else {
            let at = d.found.map_or(String::new(), |i| format!("{} of ", i + 1));
            let going =
                if se.done { String::new() } else { format!(" (searching, page {} of {})", se.next, d.pages.len()) };
            format!("{at}{n} found{going}")
        };
        let small = |text: &str, m: Option<Msg>| {
            let b = button(label(text).size(12.0)).padding(Padding::hv(2.0, 10.0)).name(text);
            match m {
                Some(m) => b.on_press(m),
                None => b,
            }
        };
        let row_items: Vec<Element<Msg>> = vec![
            label("Find").size(12.0).dim(),
            field(q.to_string(), Msg::FindText)
                .autofocus()
                .on_submit(Msg::Run(cmd::FIND_NEXT))
                .name("Find")
                .width(Length::Fixed(260.0)),
            small("Previous", (n > 0).then_some(Msg::Run(cmd::FIND_PREVIOUS))),
            small("Next", (n > 0).then_some(Msg::Run(cmd::FIND_NEXT))),
            label(how).size(12.0).dim().elide().width(Length::FILL),
            small("Done", Some(Msg::FindDone)),
        ];
        // A key of its own: its field is new when the bar opens (and so
        // takes the keyboard), not mistaken for what stood in its place.
        panel(row(row_items).spacing(10.0).align(Align::Center).width(Length::FILL))
            .radius(0.0)
            .padding(Padding { top: 4.0, bottom: 4.0, left: 12.0, right: 12.0 })
            .width(Length::FILL)
            .key(0x4649_4e44)
    }
}
