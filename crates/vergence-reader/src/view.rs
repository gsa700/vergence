//! Reader's window drawn: the toolbar, the page view and where reading is,
//! the password and problem pages, the status line. The sidebar and the
//! Find bar are drawn in sidebar.rs. The window itself is in main.rs.

use vergence_scene::Color;
use vergence_widgets::{
    Align, ButtonLook, Element, Length, Padding, Theme, button, column, drawing, field, label, panel, row, rule, vrule,
};

use crate::{Content, Msg, Tab, Viewer, cmd, pdf, update::openable};

impl Viewer {
    fn pdf_view(&self, d: &pdf::Pdf) -> Element<Msg> {
        let layout = d.0.clone();
        let scale = self.scale;
        let area = self.area.clone();
        let waker = self.waker.clone();
        let ground = match self.setting("ground").as_deref() {
            Some("light") => Some(Color::rgb(0xe4, 0xe4, 0xe6)),
            Some("window") => None,
            _ => Some(Color::rgb(0x2a, 0x2a, 0x2c)),
        };
        let held = self.bar_drag.map(|b| b.0);
        drawing(move |scene, _, r, theme| {
            if area.get() != r {
                area.set(r);
                if let Some(w) = &waker {
                    w.wake();
                }
            }
            layout.draw(scene, r, scale.max(0.25), ground.unwrap_or(theme.page), theme.accent, held);
        })
        .on_touch(|t| Some(Msg::Touch(t)))
        .cursor(match (self.hand, self.drag.is_some() && !self.selecting) {
            // Over a scroll bar (or holding one): the arrow, whatever the tool.
            _ if self.over_bar || self.bar_drag.is_some() => vergence_ui::Cursor::Default,
            // Over a link, not dragging: the pointing hand, with either tool.
            (_, false) if self.over_link && !self.selecting => vergence_ui::Cursor::Pointer,
            // The open hand; closed while it holds the pages (Dave, 2026-11-20).
            (true, true) => vergence_ui::Cursor::Grabbing,
            (true, false) => vergence_ui::Cursor::Grab,
            _ => vergence_ui::Cursor::Text,
        })
        .name("Pages")
        .value(self.where_now(d))
        .width(Length::FILL)
        .height(Length::FILL)
    }

    /// Where reading is, in words, for the page view's meaning (agents, the
    /// magnifier, a screen reader): "Page 3 of 253, continuous, 129%
    /// (width)", with the view, the turn and the night look when not the
    /// usual.
    fn where_now(&self, d: &pdf::Pdf) -> String {
        let r = self.area.get();
        let c = d.current(r);
        let mut s = match d.view {
            pdf::View::Two if c + 1 < d.pages.len() && c > 0 => {
                format!("Pages {} and {} of {}", c + 1, c + 2, d.pages.len())
            }
            _ => format!("Page {} of {}", c + 1, d.pages.len()),
        };
        s.push_str(match d.view {
            pdf::View::Continuous => ", continuous",
            pdf::View::Single => ", one page",
            pdf::View::Two => ", two pages",
        });
        let mode = match d.mode {
            pdf::Mode::Width => " (width)",
            pdf::Mode::Page => " (page)",
            pdf::Mode::Zoom(_) => "",
        };
        s.push_str(&format!(", {:.0}%{mode}", d.zoom(r) * 100.0));
        if d.turn != 0 {
            s.push_str(&format!(", turned {}°", d.turn));
        }
        if d.night {
            s.push_str(", night look");
        }
        s
    }

    /// The toolbar: the sidebar; back and forward; the page (a field to
    /// type one into) with previous and next; zoom (out, in, the width, the
    /// whole page, actual size); the views (continuous, one page, two);
    /// turning; Find and Open at the end.
    fn toolbar(&self, t: &Tab, th: &Theme) -> Element<Msg> {
        use vergence_widgets::toolbar::{Icon, flexible, group, toggle, tool, toolbar};
        let Content::Pdf(d) = &t.content else {
            return toolbar(vec![flexible(), tool(Icon::Open, "Open…", Some(Msg::Run(cmd::OPEN)))], th);
        };
        let r = self.area.get();
        let c = d.current(r);
        let shown = self.page_field.clone().unwrap_or_else(|| (c + 1).to_string());
        let mut page = field(shown, Msg::PageField)
            .on_submit(Msg::GoToPage)
            .name("Page number")
            .width(Length::Fixed(48.0))
            .key(0x5041_4745_0000 + self.page_key);
        if self.page_focus {
            page = page.autofocus();
        }
        let items = vec![
            toggle(Icon::Pane, "Sidebar", self.sidebar, Msg::Run(cmd::SIDEBAR)),
            group(vec![
                toggle(Icon::TextSelect, "Text tool", !self.hand, Msg::Run(cmd::TOOL_TEXT)),
                toggle(Icon::Hand, "Hand tool", self.hand, Msg::Run(cmd::TOOL_HAND)),
            ]),
            group(vec![
                tool(Icon::Back, "Back", (!t.back.is_empty()).then_some(Msg::Run(cmd::BACK))),
                tool(Icon::Forward, "Forward", (!t.forward.is_empty()).then_some(Msg::Run(cmd::FORWARD))),
            ]),
            group(vec![
                tool(Icon::Previous, "Previous page", (c > 0).then_some(Msg::Run(cmd::PREVIOUS))),
                page,
                label(format!("of {}", d.pages.len())).size(12.0).dim(),
                tool(Icon::Next, "Next page", (c + 1 < d.pages.len()).then_some(Msg::Run(cmd::NEXT))),
            ]),
            group(vec![
                tool(Icon::ZoomOut, "Zoom out", Some(Msg::Run(cmd::ZOOM_OUT))),
                tool(Icon::ZoomIn, "Zoom in", Some(Msg::Run(cmd::ZOOM_IN))),
                toggle(Icon::FitWidth, "Fit to the width", d.mode == pdf::Mode::Width, Msg::Run(cmd::FIT)),
                toggle(Icon::Fit, "The whole page", d.mode == pdf::Mode::Page, Msg::Run(cmd::FIT_PAGE)),
                tool(Icon::Actual, "Actual size", Some(Msg::Run(cmd::ACTUAL))),
            ]),
            group(vec![
                toggle(Icon::Continuous, "Continuous", d.view == pdf::View::Continuous, Msg::Run(cmd::VIEW_CONTINUOUS)),
                toggle(Icon::OnePage, "One page", d.view == pdf::View::Single, Msg::Run(cmd::VIEW_SINGLE)),
                toggle(Icon::TwoPages, "Two pages", d.view == pdf::View::Two, Msg::Run(cmd::VIEW_TWO)),
            ]),
            group(vec![
                tool(Icon::TurnLeft, "Turn left", Some(Msg::Run(cmd::TURN_LEFT))),
                tool(Icon::TurnRight, "Turn right", Some(Msg::Run(cmd::TURN_RIGHT))),
            ]),
            flexible(),
            tool(Icon::Info, "Info", Some(Msg::Run(cmd::INFO))),
            toggle(
                Icon::Search,
                "Find",
                self.finding.is_some(),
                if self.finding.is_some() { Msg::FindDone } else { Msg::Run(cmd::FIND) },
            ),
            tool(Icon::Open, "Open…", Some(Msg::Run(cmd::OPEN))),
        ];
        toolbar(items, th)
    }

    fn status(&self, t: &Tab) -> Element<Msg> {
        let mut items: Vec<Element<Msg>> = vec![label(t.name()).size(12.0).width(Length::FILL)];
        if let Content::Pdf(d) = &t.content {
            let r = self.area.get();
            items.push(label(format!("Page {} of {}", d.current(r) + 1, d.pages.len())).size(12.0).dim());
            let mode = match d.mode {
                pdf::Mode::Width => " (width)",
                pdf::Mode::Page => " (page)",
                pdf::Mode::Zoom(_) => "",
            };
            items.push(label(format!("{:.0}%{mode}", d.zoom(r) * 100.0)).size(12.0).dim());
            if let Some(why) = d.shared.failure(d.current(r)) {
                items.push(label(why).size(12.0).dim());
            }
            if let Some(u) = &self.pending_uri {
                let shown: String =
                    if u.chars().count() > 60 { u.chars().take(57).chain("…".chars()).collect() } else { u.clone() };
                if openable(u) {
                    items.insert(1, label(format!("Open {shown}?")).size(12.0));
                    items.insert(
                        2,
                        button(label("Open").size(12.0))
                            .look(ButtonLook::Accent)
                            .padding(Padding::hv(2.0, 10.0))
                            .on_press(Msg::OpenUri)
                            .name("Open the address"),
                    );
                    items.insert(
                        3,
                        button(label("Cancel").size(12.0))
                            .padding(Padding::hv(2.0, 10.0))
                            .on_press(Msg::CancelUri)
                            .name("Cancel"),
                    );
                } else {
                    items.insert(1, label(format!("Reader does not open links like {shown}")).size(12.0));
                    items.insert(
                        2,
                        button(label("OK").size(12.0))
                            .padding(Padding::hv(2.0, 10.0))
                            .on_press(Msg::CancelUri)
                            .name("OK"),
                    );
                }
            }
        }
        panel(row(items).spacing(14.0).align(Align::Center).width(Length::FILL))
            .radius(0.0)
            .padding(Padding { top: 3.0, bottom: 3.0, left: 12.0, right: 12.0 })
            .width(Length::FILL)
    }

    pub(crate) fn on_view(&self, t: &Theme) -> Element<Msg> {
        let Some(tab) = self.front_tab() else {
            return panel(label("Nothing open: Ctrl+O opens a PDF document.").size(13.0).dim())
                .padding(Padding { top: t.top + 24.0, bottom: 24.0, left: 24.0, right: 24.0 })
                .width(Length::FILL)
                .height(Length::FILL);
        };
        let body: Element<Msg> = match &tab.content {
            Content::Loading => {
                panel(label("Opening…").size(13.0).dim()).padding(24.0).width(Length::FILL).height(Length::FILL)
            }
            Content::Pdf(d) if self.sidebar => {
                row(vec![self.sidebar_view(d), vrule(), self.pdf_view(d)]).width(Length::FILL).height(Length::FILL)
            }
            Content::Pdf(d) => self.pdf_view(d),
            Content::Locked { password, wrong, .. } => {
                let mut items: Vec<Element<Msg>> = vec![
                    label(format!("“{}” is protected by a password.", tab.name()))
                        .size(13.0)
                        .wrap()
                        .width(Length::FILL),
                    field(password.clone(), Msg::Password)
                        .secret()
                        .autofocus()
                        .placeholder("Password")
                        .on_submit(Msg::Unlock)
                        .name("Password")
                        .width(Length::Fixed(280.0)),
                ];
                if *wrong {
                    items.push(label("That password does not open it.").size(12.0).dim());
                }
                items.push(
                    button(label("Open").size(12.0))
                        .look(ButtonLook::Accent)
                        .padding(Padding::hv(4.0, 12.0))
                        .on_press(Msg::Unlock)
                        .name("Open"),
                );
                panel(column(items).spacing(12.0).width(Length::Fixed(420.0)))
                    .padding(24.0)
                    .width(Length::FILL)
                    .height(Length::FILL)
            }
            Content::Problem { why, elsewhere } => {
                let mut items: Vec<Element<Msg>> = vec![label(why.clone()).size(13.0).wrap().width(Length::FILL)];
                if *elsewhere {
                    items.push(
                        button(label("Open with its app").size(12.0))
                            .look(ButtonLook::Accent)
                            .padding(Padding::hv(4.0, 12.0))
                            .on_press(Msg::OpenWith)
                            .name("Open with its app"),
                    );
                }
                panel(column(items).spacing(12.0).width(Length::Fixed(420.0)))
                    .padding(24.0)
                    .width(Length::FILL)
                    .height(Length::FILL)
            }
        };
        let mut all = vec![self.toolbar(tab, t), panel(body).radius(0.0).width(Length::FILL).height(Length::FILL)];
        if let (Some(q), Content::Pdf(d)) = (&self.finding, &tab.content) {
            all.push(rule());
            all.push(self.find_bar(q, d));
        }
        all.push(rule());
        all.push(self.status(tab));
        column(all).width(Length::FILL).height(Length::FILL)
    }
}
