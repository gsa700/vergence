//! The bar a toolkit window draws for itself: the window's buttons where
//! the desktop's layout puts them, and either tabs with a + (the terminal:
//! [`Layout::new`]) or the window's title ([`Layout::plain`]). It was the
//! terminal's own until 2026-11-10 and is the toolkit's so that every tool
//! has the same one; its sizes and colors are the desktop's one look
//! (vergence-look), which the server's title bars are drawn with too.
//!
//! Where everything sits is worked out in one place ([`Layout::new`]) and
//! used both to draw the bar and to say what is under the pointer, so the
//! two can never disagree. Tabs share the room there is: each is at most
//! [`TAB_W`] wide and they shrink together as more are opened, so + is
//! always in the window (the first terminal's fixed-width tabs pushed it
//! off at six).

use vergence_render::text;
use vergence_scene::{Color, Point, Rect, Scene};
use vergence_text::{FaceId, Fonts};

/// The bar's height, the tabs' (they stand on its bottom edge), and the
/// most a tab is wide.
pub const BAR_H: f32 = vergence_look::BAR_H;
pub const TAB_H: f32 = vergence_look::BAR_ITEM_H;
pub const TAB_W: f32 = 200.0;
/// The least a tab shrinks to, a button's box, the + tab's width.
const TAB_MIN: f32 = 28.0;
const BUTTON_W: f32 = 32.0;
const BUTTON_H: f32 = TAB_H;
const PLUS_W: f32 = 34.0;
const SIDE: f32 = 6.0;
/// The one corner radius (vergence-look).
pub const RADIUS: f32 = vergence_look::RADIUS;
const GAP: f32 = 2.0;
/// A tab narrower than this has no room for its own close button, unless
/// it is the one shown.
const CLOSE_NEEDS: f32 = 72.0;

const fn color(c: vergence_look::Rgb) -> Color {
    Color::rgb(c.0, c.1, c.2)
}
const fn shape(alpha: u8) -> Color {
    Color::rgba(255, 255, 255, alpha)
}
const BAR_FOCUSED: Color = color(vergence_look::BAR_FOCUSED);
const BAR_UNFOCUSED: Color = color(vergence_look::BAR_UNFOCUSED);
const INK_FOCUSED: Color = color(vergence_look::INK_FOCUSED);
const INK_UNFOCUSED: Color = color(vergence_look::INK_UNFOCUSED);
const CLOSE_HOVER: Color = color(vergence_look::CLOSE);
const SHAPE_REST: Color = shape(vergence_look::SHAPE_REST);
const SHAPE_HOVER: Color = shape(vergence_look::SHAPE_HOVER);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinButton {
    Close,
    Minimize,
    Maximize,
}

/// The desktop's button layout from Vergence's settings file
/// (`~/.config/vergence/settings.toml`, `"Gtk/DecorationLayout" = "..."`,
/// the settings app's Windows page), else minimize, maximize, close on the
/// right: what the terminal's bar follows, and the toolkit's own bar where
/// the server draws none.
pub fn desktop_layout() -> (Vec<WinButton>, Vec<WinButton>) {
    let text = settings_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    decoration_layout(&layout_setting(&text).unwrap_or_else(|| ":minimize,maximize,close".into()))
}

/// Vergence's settings file, which holds the button layout.
pub fn settings_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")));
    base.map(|b| b.join("vergence/settings.toml"))
}

/// The `"Gtk/DecorationLayout"` line's value in settings.toml's text.
pub fn layout_setting(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k.trim().trim_matches('"') == "Gtk/DecorationLayout").then(|| v.trim().trim_matches('"').to_string())
    })
}

/// GTK's decoration layout ("close,minimize:" or ":minimize,maximize,close"):
/// the buttons left of the colon and right of it.
pub fn decoration_layout(layout: &str) -> (Vec<WinButton>, Vec<WinButton>) {
    let side = |s: &str| {
        s.split(',')
            .filter_map(|b| match b.trim() {
                "close" => Some(WinButton::Close),
                "minimize" => Some(WinButton::Minimize),
                "maximize" => Some(WinButton::Maximize),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    match layout.split_once(':') {
        Some((left, right)) => (side(left), side(right)),
        None => (Vec::new(), side(layout)),
    }
}

/// Something in the bar the pointer can be over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Button(WinButton),
    Tab(usize),
    /// The close button of this tab.
    CloseTab(usize),
    Plus,
    /// Bare bar: dragging it moves the window.
    Bar,
}

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

pub struct Layout {
    pub buttons: Vec<(WinButton, Rect)>,
    pub tabs: Vec<Rect>,
    /// Each tab's close button, where the tab has room for one.
    pub closes: Vec<Option<Rect>>,
    /// The + after the tabs; no wider than nothing in a bar without tabs.
    pub plus: Rect,
    /// Where the window's title is written, in a bar without tabs.
    pub title: Option<Rect>,
    /// True if the window's buttons are on the left: tabs then close on their left too.
    pub controls_left: bool,
}

impl Layout {
    /// The bar of a window `width` wide with `count` tabs, the tab `shown`
    /// being the one in front.
    pub fn new(width: f32, count: usize, shown: usize, left: &[WinButton], right: &[WinButton]) -> Layout {
        let (buttons, start, end) = Layout::buttons(width, left, right);
        // The tabs share what is left after +, down to a least width.
        let room = (end - start - PLUS_W - GAP).max(0.0);
        let n = count.max(1) as f32;
        let tab_w = ((room - GAP * n) / n).clamp(TAB_MIN, TAB_W);
        let controls_left = !left.is_empty();
        let top = BAR_H - TAB_H;
        let (mut tabs, mut closes) = (Vec::new(), Vec::new());
        let mut x = start;
        for i in 0..count {
            let tab = Rect::new(x, top, tab_w, TAB_H);
            let close = (tab_w >= CLOSE_NEEDS || (i == shown && tab_w >= 44.0)).then(|| {
                let cx = if controls_left { tab.x + 4.0 } else { tab.x + tab_w - 24.0 };
                Rect::new(cx, top + (TAB_H - 20.0) / 2.0, 20.0, 20.0)
            });
            tabs.push(tab);
            closes.push(close);
            x += tab_w + GAP;
        }
        Layout { buttons, tabs, closes, plus: Rect::new(x, top, PLUS_W, TAB_H), title: None, controls_left }
    }

    /// The bar of a window without tabs: its buttons, and between them the
    /// room its title is written in. The whole of it moves the window.
    pub fn plain(width: f32, left: &[WinButton], right: &[WinButton]) -> Layout {
        let (buttons, start, end) = Layout::buttons(width, left, right);
        let title = Rect::new(start + 6.0, BAR_H - TAB_H, (end - start - 12.0).max(0.0), TAB_H);
        Layout { buttons, tabs: Vec::new(), closes: Vec::new(), plus: Rect::new(end, BAR_H - TAB_H, 0.0, TAB_H), title: Some(title), controls_left: !left.is_empty() }
    }

    /// The window's buttons on their sides, and where the room between them
    /// starts and ends.
    fn buttons(width: f32, left: &[WinButton], right: &[WinButton]) -> (Vec<(WinButton, Rect)>, f32, f32) {
        let mut buttons = Vec::new();
        // As tall as the tabs and standing where they stand (Dave, 2026-11-10).
        let button_y = BAR_H - TAB_H;
        let mut x = SIDE;
        for b in left {
            buttons.push((*b, Rect::new(x, button_y, BUTTON_W, BUTTON_H)));
            x += BUTTON_W + GAP;
        }
        let mut end = width - SIDE;
        for b in right.iter().rev() {
            end -= BUTTON_W;
            buttons.push((*b, Rect::new(end, button_y, BUTTON_W, BUTTON_H)));
            end -= GAP;
        }
        (buttons, x + if left.is_empty() { 0.0 } else { GAP }, end)
    }

    /// What is at a point of the window, if the point is in the bar.
    pub fn at(&self, x: f32, y: f32) -> Option<Part> {
        if !(0.0..BAR_H).contains(&y) {
            return None;
        }
        if let Some((b, _)) = self.buttons.iter().find(|(_, r)| inside(r, x, y)) {
            return Some(Part::Button(*b));
        }
        for (i, tab) in self.tabs.iter().enumerate() {
            if self.closes[i].is_some_and(|c| inside(&c, x, y)) {
                return Some(Part::CloseTab(i));
            }
            if inside(tab, x, y) {
                return Some(Part::Tab(i));
            }
        }
        if inside(&self.plus, x, y) {
            return Some(Part::Plus);
        }
        Some(Part::Bar)
    }
}

/// An icon's lines, centered in a box.
fn icon(scene: &mut Scene, r: &Rect, what: Part, size: f32, ink: Color) {
    let (cx, cy, h) = (r.x + r.w / 2.0, r.y + r.h / 2.0, size / 2.0);
    let mut line = |a: (f32, f32), b: (f32, f32)| {
        scene.line(Point::new(cx + a.0, cy + a.1), Point::new(cx + b.0, cy + b.1), 1.25, ink);
    };
    match what {
        Part::Button(WinButton::Close) | Part::CloseTab(_) => {
            line((-h, -h), (h, h));
            line((h, -h), (-h, h));
        }
        // On the box's bottom edge, as GNOME and Windows draw it.
        Part::Button(WinButton::Minimize) => line((-h, h), (h, h)),
        Part::Button(WinButton::Maximize) => {
            line((-h, -h), (h, -h));
            line((h, -h), (h, h));
            line((h, h), (-h, h));
            line((-h, h), (-h, -h));
        }
        Part::Plus => {
            line((0.0, -h), (0.0, h));
            line((-h, 0.0), (h, 0.0));
        }
        _ => {}
    }
}

/// What the bar is drawn with.
pub struct Look<'a> {
    pub focused: bool,
    /// The terminal's own ground and text: the shown tab is drawn in them, joined to the pane.
    pub pane: Color,
    pub pane_ink: Color,
    pub font: Option<FaceId>,
    pub font_size: f32,
    pub titles: &'a [String],
    /// The window's title, for a bar without tabs ([`Layout::plain`]).
    pub title: &'a str,
    pub shown: usize,
    /// What the pointer is over.
    pub hover: Option<Part>,
}

/// Draws the bar across the top of the scene.
pub fn draw(scene: &mut Scene, fonts: &mut Fonts, layout: &Layout, look: &Look) {
    let ink = if look.focused { INK_FOCUSED } else { INK_UNFOCUSED };
    scene.fill(Rect::new(0.0, 0.0, scene.width, BAR_H), if look.focused { BAR_FOCUSED } else { BAR_UNFOCUSED });
    for (b, r) in &layout.buttons {
        let part = Part::Button(*b);
        // A button has a shape at rest, the fill of a tab that is not shown, and
        // is lighter under the pointer as such a tab is; the close button, red.
        let fill = match (look.hover == Some(part), *b) {
            (true, WinButton::Close) => CLOSE_HOVER,
            (true, _) => SHAPE_HOVER,
            (false, _) => SHAPE_REST,
        };
        scene.fill_rounded(*r, RADIUS, fill);
        icon(scene, r, part, 10.0, ink);
    }
    // A tab is round at the top and square where it meets the pane: drawn taller than the bar and cut off.
    let tab_shape = |scene: &mut Scene, r: &Rect, fill: Color| {
        scene.push_clip(*r, 0.0);
        scene.fill_rounded(Rect::new(r.x, r.y, r.w, r.h + 12.0), RADIUS, fill);
        scene.pop_clip();
    };
    for (i, tab) in layout.tabs.iter().enumerate() {
        let shown = i == look.shown;
        let hovered = matches!(look.hover, Some(Part::Tab(t) | Part::CloseTab(t)) if t == i);
        let tab_ink = if shown { look.pane_ink } else { ink };
        tab_shape(scene, tab, if shown { look.pane } else if hovered { SHAPE_HOVER } else { SHAPE_REST });
        let mut text_from = tab.x + 12.0;
        let mut text_to = tab.x + tab.w - 8.0;
        if let Some(close) = &layout.closes[i] {
            if look.hover == Some(Part::CloseTab(i)) {
                scene.fill_rounded(*close, RADIUS, Color::rgba(tab_ink.r, tab_ink.g, tab_ink.b, 46));
            }
            icon(scene, close, Part::CloseTab(i), 8.0, tab_ink);
            if layout.controls_left {
                text_from = close.x + close.w + 6.0;
            } else {
                text_to = close.x - 4.0;
            }
        }
        if let (Some(face), Some(title)) = (look.font, look.titles.get(i))
            && text_to - text_from > 10.0
        {
            // The title is cut where the tab's room ends.
            scene.push_clip(Rect::new(text_from, tab.y, text_to - text_from, tab.h), 0.0);
            let m = fonts.metrics(face, look.font_size);
            let baseline = tab.y + ((tab.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round() - 1.0;
            text(scene, fonts, face, look.font_size, tab_ink, Point::new(text_from, baseline), title);
            scene.pop_clip();
        }
    }
    if layout.plus.w > 0.0 {
        tab_shape(scene, &layout.plus, if look.hover == Some(Part::Plus) { SHAPE_HOVER } else { SHAPE_REST });
        icon(scene, &layout.plus, Part::Plus, 12.0, ink);
    }
    if let (Some(room), Some(face)) = (&layout.title, look.font)
        && room.w > 10.0
    {
        // The title, on the tabs' line, cut where its room ends.
        scene.push_clip(*room, 0.0);
        let m = fonts.metrics(face, look.font_size);
        let baseline = room.y + ((room.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round() - 1.0;
        text(scene, fonts, face, look.font_size, ink, Point::new(room.x, baseline), look.title);
        scene.pop_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: [WinButton; 2] = [WinButton::Close, WinButton::Minimize];

    #[test]
    fn the_layout_string_says_which_side() {
        assert_eq!(decoration_layout("close,minimize:"), (vec![WinButton::Close, WinButton::Minimize], vec![]));
        assert_eq!(decoration_layout(":minimize,maximize,close"), (vec![], vec![WinButton::Minimize, WinButton::Maximize, WinButton::Close]));
        assert_eq!(decoration_layout("menu:close"), (vec![], vec![WinButton::Close]));
        assert_eq!(decoration_layout("close"), (vec![], vec![WinButton::Close]));
        assert_eq!(decoration_layout(""), (vec![], vec![]));
        assert_eq!(layout_setting("[settings]\n\"Gtk/DecorationLayout\" = \"close,minimize:\"\n").as_deref(), Some("close,minimize:"));
        assert_eq!(layout_setting("[settings]\n\"Gtk/FontName\" = \"Sans 10\"\n"), None);
    }

    #[test]
    fn plus_is_always_in_the_window() {
        for width in [1264.0, 820.0, 500.0, 320.0] {
            for count in 1..=40 {
                let l = Layout::new(width, count, 0, &LEFT, &[]);
                assert_eq!(l.tabs.len(), count);
                let narrow = l.tabs[0].w <= TAB_MIN;
                assert!(narrow || l.plus.x + l.plus.w <= width - SIDE + 0.01, "{count} tabs in {width}: + ends at {}", l.plus.x + l.plus.w);
                assert!(l.tabs.windows(2).all(|t| t[0].x + t[0].w <= t[1].x), "tabs do not overlap");
            }
        }
        // The stress test's case: seven tabs in the 4K window, all with room for a title.
        let l = Layout::new(1264.0, 7, 6, &LEFT, &[]);
        assert!(l.tabs[0].w > 150.0 && l.plus.x + l.plus.w < 1264.0, "tab width {}", l.tabs[0].w);
        // One tab is no wider than a tab should be.
        assert_eq!(Layout::new(1264.0, 1, 0, &LEFT, &[]).tabs[0].w, TAB_W);
    }

    #[test]
    fn what_is_under_the_pointer() {
        let l = Layout::new(820.0, 3, 1, &LEFT, &[WinButton::Maximize]);
        assert_eq!(l.at(10.0, 20.0), Some(Part::Button(WinButton::Close)));
        assert_eq!(l.at(50.0, 20.0), Some(Part::Button(WinButton::Minimize)));
        assert_eq!(l.at(800.0, 20.0), Some(Part::Button(WinButton::Maximize)));
        let tab = l.tabs[1];
        assert_eq!(l.at(tab.x + tab.w / 2.0, 20.0), Some(Part::Tab(1)));
        // Buttons on the left: a tab's close is at its left end.
        assert!(l.controls_left);
        assert_eq!(l.at(tab.x + 10.0, 20.0), Some(Part::CloseTab(1)));
        assert_eq!(l.at(l.plus.x + 5.0, 20.0), Some(Part::Plus));
        assert_eq!(l.at(l.plus.x + l.plus.w + 40.0, 20.0), Some(Part::Bar));
        assert_eq!(l.at(tab.x + 5.0, 2.0), Some(Part::Bar), "above the tabs is bare bar");
        assert_eq!(l.at(400.0, BAR_H + 1.0), None, "below the bar is the terminal");
        // Buttons on the right: close at the tab's right end.
        let r = Layout::new(820.0, 2, 0, &[], &[WinButton::Minimize, WinButton::Maximize, WinButton::Close]);
        let tab = r.tabs[0];
        assert_eq!(r.at(tab.x + tab.w - 10.0, 20.0), Some(Part::CloseTab(0)));
        assert_eq!(r.at(815.0 - 10.0, 20.0), Some(Part::Button(WinButton::Close)));
    }

    #[test]
    fn narrow_tabs_keep_only_the_shown_ones_close() {
        let l = Layout::new(500.0, 8, 3, &LEFT, &[]);
        assert!(l.tabs[0].w < CLOSE_NEEDS);
        assert!(l.closes[3].is_some());
        assert!(l.closes.iter().enumerate().all(|(i, c)| c.is_some() == (i == 3)));
    }

    #[test]
    fn a_bar_without_tabs_has_its_buttons_and_a_title() {
        let l = Layout::plain(640.0, &[], &[WinButton::Minimize, WinButton::Maximize, WinButton::Close]);
        assert!(l.tabs.is_empty() && l.plus.w == 0.0);
        let title = l.title.unwrap();
        let first_button = l.buttons.iter().map(|(_, r)| r.x).fold(f32::MAX, f32::min);
        assert!(title.x >= SIDE && title.x + title.w <= first_button, "the title's room ends before the buttons: {title:?}");
        assert_eq!(l.at(title.x + 20.0, 20.0), Some(Part::Bar), "the title's room moves the window");
        assert_eq!(l.at(640.0 - SIDE - 5.0, 20.0), Some(Part::Button(WinButton::Close)));
        assert_eq!(l.at(300.0, BAR_H + 1.0), None);
        // Buttons on the left: the title starts after them.
        let l = Layout::plain(640.0, &LEFT, &[]);
        assert!(l.title.unwrap().x > 2.0 * BUTTON_W);
    }
}
