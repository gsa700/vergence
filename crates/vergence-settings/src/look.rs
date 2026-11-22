//! The look every page shares: the intro, the page's layout, groups, the
//! Advanced part, a setting's row, the status line and the theme's colors.
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

impl App {
    pub(crate) fn status_line(&self) -> Element<Message> {
        if self.status.is_empty() {
            space()
        } else {
            label(self.status.clone()).size(11.0).dim().wrap().width(Length::FILL)
        }
    }

    // The theme's colors, for the few places that are drawn by hand.
    fn palette(&self) -> vergence_look::Palette {
        if appearance_text(&self.desktop, "color-scheme") == "dark" {
            vergence_look::PALETTE_DARK
        } else {
            vergence_look::PALETTE_LIGHT
        }
    }

    pub(crate) fn text_color(&self) -> Color {
        let c = self.palette().text;
        Color::rgb(c.0, c.1, c.2)
    }

    pub(crate) fn line_color(&self) -> Color {
        let c = self.palette().line;
        Color::rgb(c.0, c.1, c.2)
    }

    pub(crate) fn card_color(&self) -> Color {
        let c = self.palette().card;
        Color::rgb(c.0, c.1, c.2)
    }

    /// A line or two saying what a page is for, plain text over its groups:
    /// no heading card, since the sidebar already says which page it is
    /// (see docs/settings-milestone-1.md). A sub-page with no sidebar entry (adding pictures) keeps its
    /// name in bold.
    pub(crate) fn intro(&self, title: Option<&str>, description: &str) -> Element<Message> {
        let text = label(description).size(12.0).dim().wrap().width(Length::FILL);
        match title {
            Some(t) => column(vec![label(t).size(15.0).weight(700), text]).spacing(4.0).width(Length::FILL),
            None => text,
        }
    }

    /// A page: what it is for, then its groups, scrolling as one; its name
    /// (the sidebar's) names the scroll for Contract 3.
    pub(crate) fn page_layout(&self, title: &str, description: &str, items: Vec<Element<Message>>) -> Element<Message> {
        let mut all = vec![self.intro(None, description)];
        all.extend(items);
        // The page scrolls up under the frosted bar (look milestone 1's content under the bar).
        let top = self.top.get();
        scroll(
            panel(column(all).spacing(16.0).width(Length::FILL))
                .padding(Padding { top: 22.0 + top, bottom: 22.0, left: 22.0, right: 22.0 })
                .width(Length::FILL),
        )
        .name(title)
    }

    /// A page's Advanced part (settings milestone 1, step 2): every other
    /// option and live status, behind "Advanced" at the page's end; a click
    /// opens it, and it stays open while the app runs. `key` names it.
    pub(crate) fn advanced(&self, key: &'static str, rows: Vec<Element<Message>>) -> Element<Message> {
        self.disclosure(key, "Advanced", rows)
    }

    /// A titled section that opens and closes (closed until opened, per page
    /// key), as Advanced is; Sound's other outputs use it too.
    pub(crate) fn disclosure(&self, key: &'static str, title: &str, rows: Vec<Element<Message>>) -> Element<Message> {
        let open = self.advanced.contains(key);
        let head = button(
            row(vec![chevron(open), label(title).size(12.0).dim().weight(600)]).spacing(4.0).align(Align::Center),
        )
        .look(ButtonLook::Plain)
        .padding(Padding::hv(2.0, 4.0))
        .on_press(Message::Advanced(key))
        .name(&if open { format!("Close {title}") } else { format!("Open {title}") });
        let mut parts = vec![row(vec![head, space().width(Length::FILL)]).width(Length::FILL)];
        if open {
            parts.push(self.group(None, rows));
        }
        column(parts).spacing(6.0).width(Length::FILL)
    }

    /// A group of settings in a rounded card, a hairline between rows.
    pub(crate) fn group(&self, heading: Option<&str>, rows: Vec<Element<Message>>) -> Element<Message> {
        let mut inner = Vec::new();
        for (i, r) in rows.into_iter().enumerate() {
            if i > 0 {
                inner.push(panel(rule()).padding(Padding::hv(0.0, 12.0)).width(Length::FILL));
            }
            inner.push(r);
        }
        let card = panel(column(inner).width(Length::FILL))
            .color(self.card_color())
            .radius(10.0)
            .padding(2.0)
            .width(Length::FILL);
        match heading {
            Some(h) => column(vec![label(h).size(12.0).dim().weight(600), card]).spacing(6.0).width(Length::FILL),
            None => card,
        }
    }
}

/// `name` cut to at most `max` characters, with an ellipsis when cut (about a
/// 152-point tile's width at 11 points).
pub(crate) fn short_name(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_string();
    }
    let mut s: String = name.chars().take(max - 1).collect();
    s.push('\u{2026}');
    s
}

/// One setting: its name and a line about it on the left, its control on the right.
pub(crate) fn setting(name: &str, detail: &str, control: Element<Message>) -> Element<Message> {
    let mut left = vec![label(name)];
    if !detail.is_empty() {
        left.push(label(detail).size(11.0).dim().wrap().width(Length::FILL));
    }
    panel(
        row(vec![column(left).spacing(2.0).width(Length::FILL), control])
            .spacing(16.0)
            .align(Align::Center)
            .width(Length::FILL),
    )
    .padding(Padding::hv(8.0, 12.0))
    .width(Length::FILL)
}

pub(crate) fn percent(v: f32) -> Element<Message> {
    label(format!("{:.0}%", v * 100.0)).width(Length::Fixed(48.0)).align(Align::End)
}
