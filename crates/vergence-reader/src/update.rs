//! Reader's messages (`on_update`): the pointer on the pages (the Text and
//! Hand tools, links, the scroll bars), the thumbnails, the sidebar, Find
//! and the page field; links followed or asked about. The window itself
//! is in main.rs.

use vergence_widgets::{Touch, Window};

use crate::{Content, Msg, Side, Viewer, pdf};

/// Addresses a document's links may open (through vergence-open, after asking).
pub(crate) fn openable(uri: &str) -> bool {
    let lower = uri.to_ascii_lowercase();
    ["http://", "https://", "mailto:", "ftp://"].iter().any(|s| lower.starts_with(s))
}

impl Viewer {
    /// A press on the page view's scroll bars (window-relative `x`, `y`):
    /// on a thumb it is held where taken; on a track the thumb's middle
    /// goes there. True if it was on a bar.
    fn bar_press(&mut self, x: f32, y: f32) -> bool {
        let r = self.area.get();
        let Some(d) = self.pdf_mut() else { return false };
        let (ax, ay) = (r.x + x, r.y + y);
        let Some(which) = d.bar_at(r, ax, ay) else { return false };
        let Some(t) = d.bar(r, which) else { return false };
        let (at, start, len) = if which == pdf::Bar::Down { (ay, t.y, t.h) } else { (ax, t.x, t.w) };
        let grab = if at >= start && at <= start + len { at - start } else { len / 2.0 };
        d.bar_to(r, which, at - grab);
        self.bar_drag = Some((which, grab));
        self.follow_thumb();
        true
    }

    /// A link followed: a place in the document, or an address asked about.
    fn follow(&mut self, action: vergence_pdf::outline::Action) {
        let r = self.area.get();
        match action {
            vergence_pdf::outline::Action::GoTo(t) => {
                self.mark();
                if let Some(d) = self.pdf_mut() {
                    d.go_to_target(t, r);
                }
                self.follow_thumb();
            }
            vergence_pdf::outline::Action::Uri(u) => self.pending_uri = Some(u),
        }
    }

    pub(crate) fn on_update(&mut self, m: Msg, window: &mut Window) {
        match m {
            Msg::Touch(Touch::Press { x, y }) if self.bar_press(x, y) => {}
            // Passing over the page view: on a scroll bar, the arrow.
            Msg::Touch(Touch::Move { x, y, pressed: false }) => {
                let r = self.area.get();
                let pdf = self.front_tab().and_then(|t| match &t.content {
                    Content::Pdf(d) => Some(d),
                    _ => None,
                });
                let over = pdf.is_some_and(|d| d.bar_at(r, r.x + x, r.y + y).is_some());
                let link = !over && pdf.is_some_and(|d| d.link_at(r, r.x + x, r.y + y).is_some());
                if over == self.over_bar && link == self.over_link {
                    return;
                }
                self.over_bar = over;
                self.over_link = link;
            }
            Msg::Touch(Touch::Leave) if self.bar_drag.is_none() && self.drag.is_none() => {
                self.over_bar = false;
                self.over_link = false;
                self.press_at = None;
                self.selecting = false;
            }
            Msg::Touch(Touch::Move { x, y, pressed: true }) if self.bar_drag.is_some() => {
                let r = self.area.get();
                if let (Some((which, grab)), Some(d)) = (self.bar_drag, self.pdf_mut()) {
                    let at = if which == pdf::Bar::Down { r.y + y } else { r.x + x };
                    d.bar_to(r, which, at - grab);
                }
                self.follow_thumb();
                self.place_soon(window);
            }
            Msg::Touch(Touch::Release { .. } | Touch::Leave) if self.bar_drag.is_some() => self.bar_drag = None,
            Msg::Touch(Touch::Press { x, y }) => {
                // The Text tool selects; the Hand moves the pages.
                self.drag = Some((x, y));
                self.press_at = Some(((x, y), false));
                let r = self.area.get();
                // Each tool does one thing (Dave: "as is the pointer sort of
                // floats between both functions"): the Text tool selects,
                // from the nearest character on the page pressed, and never
                // moves the pages; the Hand only moves them.
                let hand = self.hand;
                let spot = self.front_tab().and_then(|t| match &t.content {
                    Content::Pdf(d) if !hand => {
                        d.text_at(r, r.x + x, r.y + y).or_else(|| d.spot_near(r, r.x + x, r.y + y))
                    }
                    _ => None,
                });
                self.selecting = !hand;
                if let Some(d) = self.pdf_mut() {
                    d.selection = spot.map(|s| pdf::Selection { anchor: s, head: s });
                }
            }
            Msg::Touch(Touch::Move { x, y, pressed }) => {
                let r = self.area.get();
                let moved_far = |press_at: &mut Option<((f32, f32), bool)>| {
                    if let Some(((px, py), moved)) = press_at
                        && ((x - *px).powi(2) + (y - *py).powi(2)).sqrt() > 4.0
                    {
                        *moved = true;
                    }
                };
                if pressed && self.selecting {
                    moved_far(&mut self.press_at);
                    if let Some(d) = self.pdf_mut()
                        && let Some(spot) = d.spot_near(r, r.x + x, r.y + y)
                    {
                        // Begun beside the pages: it starts where the drag first reaches text.
                        match &mut d.selection {
                            Some(sel) => sel.head = spot,
                            None => d.selection = Some(pdf::Selection { anchor: spot, head: spot }),
                        }
                    }
                } else if pressed
                    && self.hand
                    && let Some((sx, sy)) = self.drag
                {
                    if let Some(d) = self.pdf_mut() {
                        d.scroll(sy - y, x - sx, r);
                    }
                    self.drag = Some((x, y));
                    moved_far(&mut self.press_at);
                    self.follow_thumb();
                    self.place_soon(window);
                }
            }
            Msg::Touch(Touch::Release { .. }) => {
                self.drag = None;
                let was_selecting = std::mem::take(&mut self.selecting);
                // A click (no drag) clears a selection begun by it.
                if was_selecting
                    && matches!(self.press_at, Some((_, false)))
                    && let Some(d) = self.pdf_mut()
                {
                    d.selection = None;
                }
                // A click on a page (no drag): a link there is followed.
                if let Some(((x, y), false)) = self.press_at.take() {
                    let r = self.area.get();
                    let action = self.front_tab().and_then(|t| match &t.content {
                        Content::Pdf(d) => d.link_at(r, r.x + x, r.y + y),
                        _ => None,
                    });
                    if let Some(a) = action {
                        self.follow(a);
                        self.place_soon(window);
                    }
                }
            }
            Msg::Touch(Touch::Leave) => {
                self.drag = None;
                self.press_at = None;
                self.selecting = false;
            }
            Msg::Run(c) => return self.run(c, window),
            Msg::Password(p) => {
                if let Some(t) = self.front_mut()
                    && let Content::Locked { password, .. } = &mut t.content
                {
                    *password = p;
                }
            }
            Msg::Unlock => self.unlock(),
            Msg::ThumbTouch(Touch::Press { x, y })
                if x >= self.thumb_area.get().w - pdf::BAR_HIT
                    && self.front_tab().is_some_and(
                        |t| matches!(&t.content, Content::Pdf(d) if d.thumbs_bar(self.thumb_area.get()).is_some()),
                    ) =>
            {
                let tr = self.thumb_area.get();
                let thumb = match &self.front_tab().map(|t| &t.content) {
                    Some(Content::Pdf(d)) => d.thumbs_bar(tr),
                    _ => None,
                };
                if let Some(t) = thumb {
                    let at = tr.y + y;
                    // On the thumb: held where taken; on the track: the thumb's middle there.
                    let grab = if at >= t.y && at <= t.y + t.h { at - t.y } else { t.h / 2.0 };
                    self.thumb_bar_drag = Some(grab);
                    if let Some(d) = self.pdf_mut() {
                        d.thumbs_bar_to(tr, at - grab);
                    }
                }
            }
            Msg::ThumbTouch(Touch::Move { y, pressed: true, .. }) if self.thumb_bar_drag.is_some() => {
                let tr = self.thumb_area.get();
                if let (Some(grab), Some(d)) = (self.thumb_bar_drag, self.pdf_mut()) {
                    d.thumbs_bar_to(tr, tr.y + y - grab);
                }
            }
            Msg::ThumbTouch(Touch::Release { .. } | Touch::Leave) if self.thumb_bar_drag.is_some() => {
                self.thumb_bar_drag = None
            }
            Msg::ThumbTouch(Touch::Press { y, .. }) => {
                self.mark();
                let (r, tr) = (self.area.get(), self.thumb_area.get());
                if let Some(d) = self.pdf_mut()
                    && let Some(p) = d.thumb_at(tr, tr.y + y)
                {
                    d.go_to(p, r);
                }
                self.place_soon(window);
            }
            Msg::ThumbTouch(_) => return,
            Msg::SideTab(side) => self.side = side,
            Msg::Result(i) => {
                if self.front_tab().is_some_and(|t| matches!(&t.content, Content::Pdf(d) if d.found != Some(i))) {
                    self.mark();
                }
                let r = self.area.get();
                if let Some(d) = self.pdf_mut() {
                    d.show_match(i as isize, r);
                }
                self.follow_thumb();
                self.place_soon(window);
            }
            Msg::Outline(i) => {
                let target = self.front_tab().and_then(|t| match &t.content {
                    Content::Pdf(d) => d
                        .shared
                        .outline
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .as_ref()
                        .and_then(|o| o.get(i))
                        .and_then(|e| e.target),
                    _ => None,
                });
                if let Some(t) = target {
                    self.follow(vergence_pdf::outline::Action::GoTo(t));
                }
                self.place_soon(window);
            }
            Msg::OpenUri => {
                if let Some(u) = self.pending_uri.take()
                    && openable(&u)
                {
                    let _ = std::process::Command::new("vergence-open").arg(u).spawn();
                }
            }
            Msg::CancelUri => self.pending_uri = None,
            Msg::FindText(q) => {
                if let Some(d) = self.pdf_mut() {
                    d.start_search(&q);
                }
                if self.sidebar && !q.trim().is_empty() {
                    self.side = Side::Results;
                }
                self.finding = Some(q);
            }
            Msg::FindDone => {
                self.finding = None;
                if let Some(d) = self.pdf_mut() {
                    d.start_search("");
                }
            }
            Msg::PageField(text) => self.page_field = Some(text.chars().filter(char::is_ascii_digit).take(6).collect()),
            Msg::GoToPage => {
                let r = self.area.get();
                if let Some(n) = self.page_field.take().and_then(|t| t.parse::<usize>().ok()) {
                    self.mark();
                    if let Some(d) = self.pdf_mut() {
                        d.go_to(n.saturating_sub(1), r);
                    }
                }
                // A new field: the keyboard goes back to the pages.
                self.page_key += 1;
                self.page_focus = false;
                self.follow_thumb();
                self.place_soon(window);
            }
            Msg::OpenWith => self.open_with(),
        }
        window.redraw();
    }
}
