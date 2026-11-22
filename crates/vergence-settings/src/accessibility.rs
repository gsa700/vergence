//! The Accessibility page (Page::Accessibility): contrast, transparency,
//! the size of everything, motion; Zoom is zoom.rs.
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

impl App {
    /// High contrast, motion, transparency and the size of everything, in one
    /// place (settings milestone 1): the same keys as before, so nothing a
    /// program reads changes.
    pub(crate) fn accessibility(&self) -> Element<Message> {
        let contrast = appearance_text(&self.desktop, "contrast") == "high";
        let motion = self
            .desktop
            .get("appearance")
            .and_then(|a| a.get("reduced-motion"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let transparency = self
            .desktop
            .get("appearance")
            .and_then(|a| a.get("reduce-transparency"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        self.page_layout(
            "Accessibility",
            "Contrast, motion and transparency for the whole desktop, and how big everything is.",
            vec![
                self.group(
                    Some("Seeing"),
                    vec![
                        setting(
                            "High contrast",
                            "Apps that support it draw stronger edges and text. The desktop's bars are solid.",
                            switch(contrast, Message::Contrast).name("High contrast"),
                        ),
                        setting(
                            "Reduce transparency",
                            "Bars, the dock and menus are solid: nothing shows through them.",
                            switch(transparency, Message::Transparency).name("Reduce transparency"),
                        ),
                        setting(
                            "Size",
                            &format!(
                                "Everything bigger on every screen, on top of each screen's own scale (Screens): {}%.",
                                (self.size * 100.0).round()
                            ),
                            size_slider(self.size),
                        ),
                        self.focus_ring_row(),
                    ],
                ),
                self.zoom_group(),
                self.group(
                    Some("Motion"),
                    vec![setting(
                        "Reduce motion",
                        "Apps that support it turn off animations, and zooming jumps rather than glides.",
                        switch(motion, Message::Motion).name("Reduce motion"),
                    )],
                ),
                self.group(
                    None,
                    vec![setting(
                        "Fonts",
                        "The interface and monospace fonts and their sizes are in Theme and fonts.",
                        button(label("Theme and fonts"))
                            .on_press(Message::Go(Page::Theme))
                            .name("Open Theme and fonts"),
                    )],
                ),
                self.status_line(),
            ],
        )
    }

    /// This page's messages, as main.rs's handle routes them here.
    pub(crate) fn accessibility_message(&mut self, m: Message) {
        match m {
            Message::Contrast(on) => self.save_appearance(|t| {
                t.insert("contrast".into(), if on { "high" } else { "normal" }.into());
            }),
            Message::Motion(on) => self.save_appearance(|t| {
                t.insert("reduced-motion".into(), on.into());
            }),
            Message::Transparency(on) => self.save_appearance(|t| {
                t.insert("reduce-transparency".into(), on.into());
            }),
            Message::Size(v) => self.size = if (v - 1.0).abs() < 0.035 { 1.0 } else { v },
            Message::SizeDone => {
                let s = (self.size as f64).clamp(0.75, 1.5);
                self.save(|d| disp::set_size(d, s));
            }
            _ => {}
        }
    }
}

/// The size slider, 75% to 150%, sticking to 100% within 3.5%.
fn size_slider(v: f32) -> Element<Message> {
    row(vec![
        slider(0.75, 1.5, v, Message::Size)
            .step(0.01)
            .marks(vec![1.0])
            .on_release(Message::SizeDone)
            .width(Length::Fixed(220.0))
            .name("Size"),
        percent(v),
    ])
    .spacing(8.0)
    .align(Align::Center)
}
