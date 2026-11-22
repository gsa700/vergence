//! The Windows page (Page::Windows): the title bar's layout, snapping and
//! switching (desktop.toml [windows]), X11 applications; window places are
//! places.rs.
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

/// Snapping and switching (windows milestone 1), as shown and as desktop.toml
/// `[windows]` writes them; the first of each is the default unless marked.
const SNAP_MODES: [(&str, &str); 3] = [("Off", "off"), ("While Ctrl is held", "ctrl"), ("Always", "always")];
const TOP_EDGE: [(&str, &str); 2] = [("Maximizes", "maximize"), ("Top half", "half")];
const BETWEEN_SCREENS: [(&str, &str); 2] = [("Never snap", "never"), ("After a short press", "press")];
const ALT_TAB: [(&str, &str); 2] = [("Every screen", "all"), ("This screen", "screen")];

impl App {
    /// desktop.toml `[windows] key` as a place in `list` (`default` if unset or unknown).
    fn win_word(&self, key: &str, list: &[(&str, &str)], default: usize) -> usize {
        let word = match self.desktop.get("windows").and_then(|t| t.get(key)) {
            // `snap = true` / `false` from before the three choices.
            Some(toml::Value::Boolean(true)) => "always".to_string(),
            Some(toml::Value::Boolean(false)) => "off".to_string(),
            Some(toml::Value::String(w)) => w.trim().to_ascii_lowercase(),
            _ => return default,
        };
        list.iter().position(|(_, w)| *w == word).unwrap_or(default)
    }

    fn win_flag(&self, key: &str, default: bool) -> bool {
        self.desktop.get("windows").and_then(|t| t.get(key)).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    pub(crate) fn windows(&self) -> Element<Message> {
        let l = winbar::parse(&theme::get(&self.xsettings, winbar::KEY));
        let side = |left: bool, text: &'static str| {
            let selected = l.left == left;
            button(label(text))
                .look(if selected { ButtonLook::Chosen } else { ButtonLook::Normal })
                .padding(Padding::hv(5.0, 16.0))
                .on_press(Message::Layout(winbar::Layout { left, ..l }))
        };
        self.page_layout(
            "Windows",
            "The title bar: which side its controls are on, and which buttons it has. Snapping windows to the screen's edges, the window keys, and Alt+Tab.",
            vec![
                self.group(None, vec![preview(l, self.line_color())]),
                self.group(Some("Title bar"), vec![
                    setting("Controls", "Left, as on a Mac; right, as on Windows.", row(vec![side(true, "Left"), side(false, "Right")]).spacing(6.0)),
                    setting("Minimize", "Hides the window until it is chosen again.", switch(l.minimize, move |on| Message::Layout(winbar::Layout { minimize: on, ..l })).name("Minimize")),
                    setting("Maximize", "Fills the screen. Double-clicking the title bar does too.", switch(l.maximize, move |on| Message::Layout(winbar::Layout { maximize: on, ..l })).name("Maximize")),
                    setting("Close", "Always there.", label("On").dim()),
                ]),
                self.snapping(),
                self.places_group(),
                self.group(Some("Older applications"), vec![setting(
                    "X11 applications",
                    "Through Xwayland, for programs with no Wayland support. Turning it on starts it at once; turning it off takes effect the next time Vergence starts.",
                    switch(self.desktop.get("x11").and_then(|t| t.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(false), Message::X11).name("X11 applications"),
                )]),
                self.status_line(),
            ],
        )
    }

    /// Windows > Snapping and switching (windows milestone 1, step 6).
    fn snapping(&self) -> Element<Message> {
        let names = |list: &[(&str, &str)]| list.iter().map(|(n, _)| n.to_string()).collect::<Vec<_>>();
        let pick = |key: &'static str,
                    list: &'static [(&'static str, &'static str)],
                    default: usize,
                    width: f32,
                    name: &str,
                    enabled: bool| {
            let c =
                choice(names(list), Some(self.win_word(key, list, default)), move |i| Message::WinWord(key, list[i].1))
                    .width(Length::Fixed(width))
                    .name(name);
            if enabled { c } else { c.disabled() }
        };
        let flag = |key: &'static str, default: bool, name: &str, enabled: bool| {
            let s = switch(self.win_flag(key, default), move |on| Message::WinFlag(key, on)).name(name);
            if enabled { s } else { s.disabled() }
        };
        let snapping = self.win_word("snap", &SNAP_MODES, 2) != 0;
        self.group(Some("Snapping and switching"), vec![
            setting(
                "Snap a window dragged to an edge",
                "Left or right: half the screen. A corner: a quarter. The top: all of it. With Ctrl, only when Ctrl is held during the drag.",
                pick("snap", &SNAP_MODES, 2, 180.0, "Snap a window dragged to an edge", true),
            ),
            setting("The top edge", "What dragging a window to the top of the screen does.", pick("top-edge", &TOP_EDGE, 0, 180.0, "The top edge", snapping)),
            setting(
                "Edges between screens",
                "Where one screen meets another, a window crosses; it can snap there after the pointer has pressed against the edge.",
                pick("between-screens", &BETWEEN_SCREENS, 0, 180.0, "Edges between screens", snapping),
            ),
            setting("Show where it will go", "A tint over the part of the screen the window will take.", flag("preview", true, "Show where it will go", snapping)),
            setting("Room between snapped windows", "A little space round them and between them.", flag("gaps", true, "Room between snapped windows", true)),
            setting(
                "Layouts on the maximize button",
                "Resting the pointer on the button shows them. A right click on it, or on the title bar, always does.",
                flag("layouts-hover", true, "Layouts on the maximize button", true),
            ),
            setting(
                "Window keys",
                "Super with the arrows: left and right halves, Up to maximize, Down back or to minimize. Whether or not the buttons are shown.",
                flag("window-keys", true, "Window keys", true),
            ),
            setting("Alt+Tab shows", "The windows it steps through, the most recently used first.", pick("alt-tab", &ALT_TAB, 0, 180.0, "Alt+Tab shows", true)),
            setting("Alt+` (the key above Tab)", "The same, among the focused application's windows only.", flag("app-switch", true, "Alt+backtick", true)),
            setting(
                "Click the desktop to move windows aside",
                "A click on the desktop's bare ground slides the windows to the sides; another click, or a click on a window, brings them back.",
                flag("desktop-aside", true, "Click the desktop to move windows aside", true),
            ),
        ])
    }

    /// This page's messages, as main.rs's handle routes them here.
    pub(crate) fn windows_message(&mut self, m: Message) {
        match m {
            Message::Layout(l) => self.set_x(winbar::KEY, &winbar::format(l)),
            Message::X11(on) => self.save_table("x11", |t| {
                t.insert("enabled".into(), on.into());
            }),
            Message::WinWord(key, word) => self.save_table("windows", |t| {
                t.insert(key.into(), word.into());
            }),
            Message::WinFlag(key, on) => self.save_table("windows", |t| {
                t.insert(key.into(), on.into());
            }),
            _ => {}
        }
    }
}

/// A preview of a title bar in the chosen layout, in the server's colors.
fn preview(l: winbar::Layout, line: Color) -> Element<Message> {
    let d = drawing(move |scene: &mut Scene, fonts: &mut Fonts, r: Rect, t: &Theme| {
        let w = 420.0f32.min(r.w - 28.0);
        let win = Rect::new(r.x + (r.w - w) / 2.0, r.y + 14.0, w, 88.0);
        let bar = Rect::new(win.x, win.y, win.w, 32.0);
        scene.fill(bar, Color::rgb(51, 61, 82));
        scene.fill(Rect::new(win.x, win.y + 32.0, win.w, 56.0), rgb(0x1e1e2e));
        scene.stroke(win, 0.0, 1.0, line);
        // The buttons: close in red, the others gray; on the chosen side.
        let close = rgb(0xc74d47);
        let other = Color::rgba(255, 255, 255, 140);
        let mut order = Vec::new();
        if l.minimize {
            order.push(other);
        }
        if l.maximize {
            order.push(other);
        }
        if l.left {
            order.insert(0, close);
        } else {
            order.push(close);
        }
        let dots_w = order.len() as f32 * 18.0 - 6.0;
        let mut x = if l.left { bar.x + 10.0 } else { bar.x + bar.w - 10.0 - dots_w };
        for c in order {
            scene.fill_rounded(Rect::new(x, bar.y + 14.0, 12.0, 12.0), 6.0, c);
            x += 18.0;
        }
        // The tabs: on the other side.
        let Some(f) = fonts.face(&t.font, 400, false).or_else(|| fonts.face("sans-serif", 400, false)) else { return };
        let ascent = fonts.metrics(f, 11.0).ascent;
        let tabs = ["~/src/vergence", "htop"];
        let widths: Vec<f32> = tabs.iter().map(|s| fonts.layout(f, 11.0, s).1 + 24.0).collect();
        let total: f32 = widths.iter().sum::<f32>() + 4.0 * tabs.len() as f32 + 12.0;
        let mut tx = if l.left { bar.x + 10.0 + dots_w + 14.0 } else { bar.x + bar.w - 10.0 - dots_w - 14.0 - total };
        for (k, s) in tabs.iter().enumerate() {
            let tab = Rect::new(tx, bar.y + 6.0, widths[k], 26.0);
            if k == 0 {
                scene.fill_rounded(Rect::new(tab.x, tab.y, tab.w, tab.h + 6.0), 6.0, rgb(0x1e1e2e));
            }
            text(scene, fonts, f, 11.0, WHITE, Point::new(tab.x + 12.0, (tab.y + 4.0 + ascent).round()), s);
            tx += widths[k] + 4.0;
        }
        text(scene, fonts, f, 13.0, WHITE, Point::new(tx + 2.0, (bar.y + 10.0 + ascent).round()), "+");
    });
    d.height(Length::Fixed(116.0)).name("Title bar preview")
}
