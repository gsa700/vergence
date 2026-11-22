// Accessibility > Zoom (meaning milestone 1, step 7): the magnifier the
// server draws, desktop.toml [magnifier] (the server's zoom.rs reads it).

use super::*;

/// The levels offered, with their names.
const LEVELS: [(&str, f64); 7] =
    [("1.5×", 1.5), ("2×", 2.0), ("3×", 3.0), ("4×", 4.0), ("6×", 6.0), ("8×", 8.0), ("12×", 12.0)];
const STYLES: [&str; 2] = ["Full screen", "Lens"];

#[derive(Debug, Clone)]
pub enum ZoomMsg {
    On(bool),
    Level(f64),
    Lens(bool),
    Follow(&'static str, bool),
    /// The server's focus ring: [appearance] focus-ring.
    FocusRing(bool),
}

fn table(d: &toml::Table) -> Option<&toml::Table> {
    d.get("magnifier").and_then(|m| m.as_table())
}

fn flag(d: &toml::Table, key: &str, default: bool) -> bool {
    table(d).and_then(|t| t.get(key)).and_then(|v| v.as_bool()).unwrap_or(default)
}

impl App {
    pub fn zoom_message(&mut self, m: ZoomMsg) {
        if let ZoomMsg::FocusRing(on) = m {
            return self.save_table("appearance", |t| {
                t.insert("focus-ring".into(), on.into());
            });
        }
        self.save_table("magnifier", |t| match m {
            ZoomMsg::On(on) => {
                t.insert("on".into(), on.into());
            }
            ZoomMsg::Level(l) => {
                t.insert("level".into(), l.into());
            }
            ZoomMsg::Lens(lens) => {
                t.insert("style".into(), if lens { "lens" } else { "screen" }.into());
            }
            ZoomMsg::Follow(key, on) => {
                t.insert(key.into(), on.into());
            }
            ZoomMsg::FocusRing(_) => {}
        });
    }

    /// The focus ring's row, for the Seeing group.
    pub fn focus_ring_row(&self) -> Element<Message> {
        let on =
            self.desktop.get("appearance").and_then(|a| a.get("focus-ring")).and_then(|v| v.as_bool()).unwrap_or(false);
        setting(
            "Show the focus ring",
            "A clear ring in the accent around the control the keyboard is on, the same in every Vergence app, the magnifier included.",
            switch(on, |on| Message::Zoom(ZoomMsg::FocusRing(on))).name("Show the focus ring"),
        )
    }

    pub fn zoom_group(&self) -> Element<Message> {
        let d = &self.desktop;
        let on = flag(d, "on", false);
        let level = table(d)
            .and_then(|t| t.get("level"))
            .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
            .unwrap_or(2.0);
        let at = LEVELS.iter().position(|(_, l)| (*l - level).abs() < 0.01);
        let lens = table(d).and_then(|t| t.get("style")).and_then(|v| v.as_str()) == Some("lens");
        let follow = |key: &'static str, title: &str, what: &str| {
            setting(
                title,
                what,
                switch(flag(d, key, true), move |on| Message::Zoom(ZoomMsg::Follow(key, on))).name(title),
            )
        };
        self.group(Some("Zoom"), vec![
            setting("Zoom", "Magnify what is on the screen. Keys: Super+Alt+8 on and off, Super+Alt+= and Super+Alt+- to zoom in and out, Super+Alt+L the lens.", switch(on, |on| Message::Zoom(ZoomMsg::On(on))).name("Zoom")),
            setting("Level", "How much larger.", choice(LEVELS.iter().map(|(n, _)| n.to_string()).collect(), at, |i| Message::Zoom(ZoomMsg::Level(LEVELS[i].1))).width(Length::Fixed(120.0)).name("Zoom level")),
            setting("Style", "The whole screen, or a lens around what you are working on.", choice(STYLES.iter().map(|s| s.to_string()).collect(), Some(lens as usize), |i| Message::Zoom(ZoomMsg::Lens(i == 1))).width(Length::Fixed(140.0)).name("Zoom style")),
            follow("follow-pointer", "Follow the pointer", "Where the pointer moves."),
            follow("follow-focus", "Follow the keyboard focus", "The control the keyboard is on, in Vergence's own apps."),
            follow("follow-caret", "Follow the text caret", "Where you type, in Vergence's own apps."),
        ])
    }
}
