//! The window bar: the window's buttons where the desktop's layout puts
//! them, and either tabs with a + (the terminal: [`Layout::new`]) or the
//! window's title ([`Layout::plain`]). It was the terminal's own until
//! 2026-11-10, then the toolkit's, and since bar milestone 1
//! (docs/bar-milestone-1.md) a crate of its own: the server draws every bar
//! from it (its title bars, native windows' tabs) and vergence-ui draws the
//! same bar where the server draws none. Its sizes and colors are the
//! desktop's one look (vergence-look).
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
/// The size of the bar's text (tab titles, a window's title), in points.
pub const FONT_SIZE: f32 = 13.0;
/// The least a tab shrinks to, a button's box, the + tab's width.
const TAB_MIN: f32 = 28.0;
const BUTTON_W: f32 = 32.0;
const PLUS_W: f32 = 34.0;
const SIDE: f32 = 6.0;
/// The one corner radius (vergence-look).
pub const RADIUS: f32 = vergence_look::RADIUS;
const GAP: f32 = 2.0;
/// A tab narrower than this has no room for its own close button, unless
/// it is the one shown.
const CLOSE_NEEDS: f32 = 72.0;

/// A bar's proportions at one size of controls (look milestone 1, step 8;
/// the presets of the mock-up Dave chose from, 2026-11-13): the bar's
/// height, its tabs' and buttons' height, a button's width, the + tab's,
/// the room at the sides and between items. Text stays FONT_SIZE: text size
/// is a setting of its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sizes {
    pub bar_h: f32,
    pub item_h: f32,
    pub button_w: f32,
    pub plus_w: f32,
    pub side: f32,
    pub gap: f32,
}

pub const COMPACT: Sizes = Sizes { bar_h: 32.0, item_h: 26.0, button_w: 28.0, plus_w: 30.0, side: 4.0, gap: 2.0 };
pub const STANDARD: Sizes = Sizes { bar_h: BAR_H, item_h: TAB_H, button_w: BUTTON_W, plus_w: PLUS_W, side: SIDE, gap: GAP };
pub const LARGE: Sizes = Sizes { bar_h: 44.0, item_h: 36.0, button_w: 38.0, plus_w: 40.0, side: 8.0, gap: 4.0 };

impl Sizes {
    /// The Size of controls setting's steps: 0 Compact, 1 Standard, 2 Large.
    pub const fn of(step: u32) -> Sizes {
        match step {
            0 => COMPACT,
            2 => LARGE,
            _ => STANDARD,
        }
    }
}

const fn color(c: vergence_look::Rgb) -> Color {
    Color::rgb(c.0, c.1, c.2)
}
const fn shape(c: vergence_look::Rgb, alpha: u8) -> Color {
    Color::rgba(c.0, c.1, c.2, alpha)
}
const CLOSE_HOVER: Color = color(vergence_look::CLOSE);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinButton {
    Close,
    Minimize,
    Maximize,
}

/// The button layout when none has been chosen: close, minimize, maximize
/// on the left (Dave, 2026-11-12; it was on the right until then).
pub const DEFAULT_LAYOUT: &str = "close,minimize,maximize:";

/// The desktop's button layout: under GNOME, GNOME's own ([`host_layout`]:
/// a program of ours there looks like its neighbors, Dave 2026-11-12); else
/// Vergence's settings file (`~/.config/vergence/settings.toml`,
/// `"Gtk/DecorationLayout" = "..."`, the settings app's Windows page); else
/// [`DEFAULT_LAYOUT`]. What the terminal's bar follows, and the toolkit's own
/// bar where the server draws none.
pub fn desktop_layout() -> (Vec<WinButton>, Vec<WinButton>) {
    let text = settings_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    decoration_layout(&host_layout().or_else(|| layout_setting(&text)).unwrap_or_else(|| DEFAULT_LAYOUT.into()))
}

/// Whether the desktop is in its light style: under GNOME, GNOME's own
/// ([`host_light`]: anything but "prefer-dark" is light there); else
/// Vergence's desktop file (`~/.config/vergence/desktop.toml`, `[appearance]`
/// `color-scheme = "light"`; "dark", "default" or nothing is dark, as the
/// server reads it). What the toolkit's own bar and border follow; the
/// server's bars follow the server's settings.
pub fn desktop_light() -> bool {
    host_light().unwrap_or_else(|| {
        let text = desktop_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        color_scheme_light(&text)
    })
}

/// `color-scheme = "light"` under `[appearance]` in desktop.toml's text.
pub fn color_scheme_light(text: &str) -> bool {
    let mut section = "";
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line;
        } else if section == "[appearance]"
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == "color-scheme"
        {
            return v.trim().trim_matches('"') == "light";
        }
    }
    false
}

/// GNOME's light or dark (org.gnome.desktop.interface color-scheme), when
/// running under GNOME outside a Vergence session; None elsewhere. Asked
/// with gsettings, as [`host_layout`] is.
pub fn host_light() -> Option<bool> {
    if std::env::var_os("VERGENCE_BUS").is_some() {
        return None;
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if !desktop.split(':').any(|d| d.eq_ignore_ascii_case("GNOME")) {
        return None;
    }
    let out = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'') != "prefer-dark")
}

/// Vergence's desktop file, which holds light or dark.
pub fn desktop_path() -> Option<std::path::PathBuf> {
    settings_path().map(|p| p.with_file_name("desktop.toml"))
}

/// GNOME's button layout ("icon:minimize,close"), when the program runs
/// under GNOME and not in a Vergence session; None elsewhere. Read with
/// GNOME's own gsettings tool (part of every GNOME install; no library of
/// GNOME's is linked), so it is asked each time: callers that follow it
/// live ask again about once a second.
pub fn host_layout() -> Option<String> {
    // The Vergence session's bus (vergence_protocol::BUS_ENV): in a Vergence
    // session our own setting holds, whatever the desktop variable says.
    if std::env::var_os("VERGENCE_BUS").is_some() {
        return None;
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if !desktop.split(':').any(|d| d.eq_ignore_ascii_case("GNOME")) {
        return None;
    }
    let out = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.wm.preferences", "button-layout"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let value = String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_string();
    (!value.is_empty() && value.len() < 256).then_some(value)
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
    /// The proportions it was laid out at.
    pub sizes: Sizes,
}

impl Layout {
    /// The bar of a window `width` wide with `count` tabs, the tab `shown`
    /// being the one in front, at the Standard size of controls.
    pub fn new(width: f32, count: usize, shown: usize, left: &[WinButton], right: &[WinButton]) -> Layout {
        Layout::sized(width, count, shown, left, right, STANDARD)
    }

    /// [`Layout::new`] at a size of controls (look milestone 1, step 8).
    pub fn sized(width: f32, count: usize, shown: usize, left: &[WinButton], right: &[WinButton], z: Sizes) -> Layout {
        let (buttons, start, end) = Layout::buttons(width, left, right, &z);
        // The tabs share what is left after +, down to a least width.
        let room = (end - start - z.plus_w - z.gap).max(0.0);
        let n = count.max(1) as f32;
        let tab_w = ((room - z.gap * n) / n).clamp(TAB_MIN, TAB_W);
        let controls_left = !left.is_empty();
        let top = z.bar_h - z.item_h;
        let (mut tabs, mut closes) = (Vec::new(), Vec::new());
        let mut x = start;
        for i in 0..count {
            let tab = Rect::new(x, top, tab_w, z.item_h);
            let close = (tab_w >= CLOSE_NEEDS || (i == shown && tab_w >= 44.0)).then(|| {
                let cx = if controls_left { tab.x + 4.0 } else { tab.x + tab_w - 24.0 };
                Rect::new(cx, top + (z.item_h - 20.0) / 2.0, 20.0, 20.0)
            });
            tabs.push(tab);
            closes.push(close);
            x += tab_w + z.gap;
        }
        Layout { buttons, tabs, closes, plus: Rect::new(x, top, z.plus_w, z.item_h), title: None, controls_left, sizes: z }
    }

    /// The bar of a window without tabs: its buttons, and between them the
    /// room its title is written in. The whole of it moves the window.
    pub fn plain(width: f32, left: &[WinButton], right: &[WinButton]) -> Layout {
        Layout::plain_sized(width, left, right, STANDARD)
    }

    /// [`Layout::plain`] at a size of controls.
    pub fn plain_sized(width: f32, left: &[WinButton], right: &[WinButton], z: Sizes) -> Layout {
        let (buttons, start, end) = Layout::buttons(width, left, right, &z);
        let top = z.bar_h - z.item_h;
        let title = Rect::new(start + 6.0, top, (end - start - 12.0).max(0.0), z.item_h);
        Layout { buttons, tabs: Vec::new(), closes: Vec::new(), plus: Rect::new(end, top, 0.0, z.item_h), title: Some(title), controls_left: !left.is_empty(), sizes: z }
    }

    /// The window's buttons on their sides, and where the room between them
    /// starts and ends.
    fn buttons(width: f32, left: &[WinButton], right: &[WinButton], z: &Sizes) -> (Vec<(WinButton, Rect)>, f32, f32) {
        let mut buttons = Vec::new();
        // As tall as the tabs and standing where they stand (Dave, 2026-11-10).
        let button_y = z.bar_h - z.item_h;
        let mut x = z.side;
        for b in left {
            buttons.push((*b, Rect::new(x, button_y, z.button_w, z.item_h)));
            x += z.button_w + z.gap;
        }
        let mut end = width - z.side;
        for b in right.iter().rev() {
            end -= z.button_w;
            buttons.push((*b, Rect::new(end, button_y, z.button_w, z.item_h)));
            end -= z.gap;
        }
        (buttons, x + if left.is_empty() { 0.0 } else { z.gap }, end)
    }

    /// What is at a point of the window, if the point is in the bar.
    pub fn at(&self, x: f32, y: f32) -> Option<Part> {
        if !(0.0..self.sizes.bar_h).contains(&y) {
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
    /// The style's colors (vergence_look::bar_look: light or dark).
    pub bar: &'a vergence_look::BarLook,
    /// The bar's ground is a frost drawn under it (the server's, look
    /// milestone 1): leave it out, draw only what stands on it.
    pub frosted: bool,
    pub focused: bool,
    /// The terminal's own ground and text: the shown tab is drawn in them, joined to the pane.
    pub pane: Color,
    pub pane_ink: Color,
    pub font: Option<FaceId>,
    pub font_size: f32,
    pub titles: &'a [String],
    /// Which tabs' programs run as the superuser: drawn in the alarm red,
    /// and the bar's ground goes red while one of them is in front.
    pub alerts: &'a [bool],
    /// The window's title, for a bar without tabs ([`Layout::plain`]).
    pub title: &'a str,
    pub shown: usize,
    /// What the pointer is over.
    pub hover: Option<Part>,
}

/// Draws the bar across the top of the scene.
pub fn draw(scene: &mut Scene, fonts: &mut Fonts, layout: &Layout, look: &Look) {
    let l = look.bar;
    let ink = color(if look.focused { l.ink } else { l.ink_off });
    let shape_rest = shape(l.shape, l.shape_rest);
    let tab_rest = shape(l.shape, l.tab_rest);
    let shape_hover = shape(l.shape, l.shape_hover);
    let front_alert = !layout.tabs.is_empty() && look.alerts.get(look.shown).copied().unwrap_or(false);
    if !look.frosted {
        let ground = match (front_alert, look.focused) {
            (true, true) => l.alert_ground,
            (true, false) => l.alert_ground_off,
            (false, true) => l.ground,
            (false, false) => l.ground_off,
        };
        scene.fill(Rect::new(0.0, 0.0, scene.width, layout.sizes.bar_h), color(ground));
    }
    for (b, r) in &layout.buttons {
        let part = Part::Button(*b);
        // The buttons float on the tabs' line: drawn inset, pressed anywhere in
        // their box. Shaded at rest only while the window has the keyboard;
        // under the pointer lighter (darker in light), the close button red
        // with a white mark.
        let drawn = Rect::new(r.x + vergence_look::BUTTON_INSET_X, r.y + vergence_look::BUTTON_INSET_Y, r.w - 2.0 * vergence_look::BUTTON_INSET_X, r.h - 2.0 * vergence_look::BUTTON_INSET_Y);
        let hovered = look.hover == Some(part);
        let fill = match (hovered, *b) {
            (true, WinButton::Close) => Some(CLOSE_HOVER),
            (true, _) => Some(shape_hover),
            (false, _) => look.focused.then_some(shape_rest),
        };
        if let Some(fill) = fill {
            scene.fill_rounded(drawn, RADIUS, fill);
        }
        let mark = if hovered && *b == WinButton::Close { Color::rgb(255, 255, 255) } else { ink };
        icon(scene, r, part, 10.0, mark);
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
        let alert = look.alerts.get(i).copied().unwrap_or(false);
        let tab_ink = if alert { Color::rgb(255, 255, 255) } else if shown { look.pane_ink } else { ink };
        tab_shape(scene, tab, if alert { color(l.alert) } else if shown { look.pane } else if hovered { shape_hover } else { tab_rest });
        if alert && hovered {
            tab_shape(scene, tab, shape_hover);
        }
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
        tab_shape(scene, &layout.plus, if look.hover == Some(Part::Plus) { shape_hover } else { tab_rest });
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

/// How far a press on a tab moves before it is a drag, in points.
pub const DRAG_START: f32 = 6.0;
/// How far beyond the bar (above, below, or past the window's sides) a
/// dragged tab goes before it would leave the window, in points.
pub const DETACH: f32 = 30.0;

/// A tab being dragged: which one, and where along it the pointer took hold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabDrag {
    pub tab: usize,
    grab: f32,
}

/// Where a dragged tab would go if let go now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// To this place in the row (counted with the dragged tab taken out
    /// first, so its own place means no change).
    At(usize),
    /// Out of the window: a window of its own, or another window's bar.
    Outside,
}

impl TabDrag {
    /// A drag of `tab` taken hold of at `x` (the window's points).
    pub fn new(layout: &Layout, tab: usize, x: f32) -> TabDrag {
        let left = layout.tabs.get(tab).map_or(x, |r| r.x);
        TabDrag { tab, grab: x - left }
    }

    /// Where the dragged tab's left edge is drawn with the pointer at `x`:
    /// with the pointer, kept within the row of tabs.
    pub fn drawn_x(&self, layout: &Layout, x: f32) -> f32 {
        let (Some(first), Some(last)) = (layout.tabs.first(), layout.tabs.last()) else { return x - self.grab };
        (x - self.grab).clamp(first.x, last.x)
    }

    /// Where it would land if let go at (`x`, `y`) of a window `width` wide.
    pub fn landing(&self, layout: &Layout, width: f32, x: f32, y: f32) -> Landing {
        if y < -DETACH || y > layout.sizes.bar_h + DETACH || x < -DETACH || x > width + DETACH {
            return Landing::Outside;
        }
        let Some(me) = layout.tabs.get(self.tab) else { return Landing::At(0) };
        let center = self.drawn_x(layout, x) + me.w / 2.0;
        // A tab that was before the dragged one stays before it until the
        // dragged one's middle is past its middle; one that was after comes
        // before it as soon as the middles meet (at the ends of the row they do).
        let before = layout
            .tabs
            .iter()
            .enumerate()
            .filter(|(i, r)| {
                let mid = r.x + r.w / 2.0;
                (*i < self.tab && mid < center) || (*i > self.tab && mid <= center)
            })
            .count();
        Landing::At(before)
    }
}

/// Move one item of a list to another place (a tab's new place, as
/// [`Landing::At`] counts it).
pub fn reorder<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from < items.len() {
        let item = items.remove(from);
        items.insert(to.min(items.len()), item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each size of controls lays the bar out at its own proportions, and
    /// what is under a point follows: a Large bar's tab is hit lower down.
    #[test]
    fn a_bar_is_laid_out_at_its_size_of_controls() {
        for z in [COMPACT, STANDARD, LARGE] {
            let l = Layout::sized(900.0, 3, 0, &LEFT, &[], z);
            assert_eq!(l.sizes, z);
            assert_eq!(l.tabs[0].y + l.tabs[0].h, z.bar_h, "tabs stand on the bar's bottom edge");
            assert_eq!(l.buttons[0].1.w, z.button_w);
            assert_eq!(l.buttons[0].1.x, z.side);
            assert!(l.at(450.0, z.bar_h - 1.0).is_some());
            assert_eq!(l.at(450.0, z.bar_h + 1.0), None);
        }
        let large = Layout::sized(900.0, 2, 0, &LEFT, &[], LARGE);
        assert_eq!(large.at(large.tabs[0].x + 50.0, STANDARD.bar_h + 2.0), Some(Part::Tab(0)), "below a Standard bar's bottom, still a Large tab");
        assert_eq!(Sizes::of(0), COMPACT);
        assert_eq!(Sizes::of(1), STANDARD);
        assert_eq!(Sizes::of(2), LARGE);
        assert_eq!(Sizes::of(9), STANDARD);
    }

    #[test]
    fn light_is_read_from_the_appearance_section() {
        assert!(color_scheme_light("[wallpaper]\nmode = \"fill\"\n[appearance]\ncolor-scheme = \"light\"\n"));
        assert!(!color_scheme_light("[appearance]\ncolor-scheme = \"dark\"\n"));
        assert!(!color_scheme_light("[appearance]\ncolor-scheme = \"default\"\n"));
        assert!(!color_scheme_light("[other]\ncolor-scheme = \"light\"\n"));
        assert!(!color_scheme_light(""));
    }

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

    fn five() -> Layout {
        Layout::new(1000.0, 5, 0, &[], &[WinButton::Close])
    }

    #[test]
    fn a_tab_dragged_past_its_neighbour_takes_its_place() {
        let l = five();
        let d = TabDrag::new(&l, 0, l.tabs[0].x + 10.0);
        // A small move: still first.
        assert_eq!(d.landing(&l, 1000.0, l.tabs[0].x + 30.0, 20.0), Landing::At(0));
        // Past the second tab's middle: second.
        let past = l.tabs[1].x + l.tabs[1].w / 2.0 + 15.0;
        assert_eq!(d.landing(&l, 1000.0, past, 20.0), Landing::At(1));
        // Far right, still in the bar: last.
        assert_eq!(d.landing(&l, 1000.0, 990.0, 20.0), Landing::At(4));
    }

    #[test]
    fn a_tab_dragged_left_goes_first_and_is_drawn_within_the_row() {
        let l = five();
        let d = TabDrag::new(&l, 3, l.tabs[3].x + 5.0);
        assert_eq!(d.landing(&l, 1000.0, 0.0, 20.0), Landing::At(0));
        assert_eq!(d.drawn_x(&l, -500.0), l.tabs[0].x);
        assert_eq!(d.drawn_x(&l, 5000.0), l.tabs[4].x);
    }

    #[test]
    fn a_tab_dragged_well_away_from_the_bar_leaves_the_window() {
        let l = five();
        let d = TabDrag::new(&l, 2, l.tabs[2].x + 5.0);
        assert_eq!(d.landing(&l, 1000.0, l.tabs[2].x, BAR_H + DETACH - 1.0), Landing::At(2));
        assert_eq!(d.landing(&l, 1000.0, l.tabs[2].x, BAR_H + DETACH + 1.0), Landing::Outside);
        assert_eq!(d.landing(&l, 1000.0, l.tabs[2].x, -DETACH - 1.0), Landing::Outside);
        assert_eq!(d.landing(&l, 1000.0, 1000.0 + DETACH + 1.0, 20.0), Landing::Outside);
    }

    #[test]
    fn reorder_moves_one_item() {
        let mut v = vec!['a', 'b', 'c', 'd'];
        reorder(&mut v, 0, 2);
        assert_eq!(v, ['b', 'c', 'a', 'd']);
        reorder(&mut v, 3, 0);
        assert_eq!(v, ['d', 'b', 'c', 'a']);
        reorder(&mut v, 1, 99);
        assert_eq!(v, ['d', 'c', 'a', 'b']);
        reorder(&mut v, 9, 0);
        assert_eq!(v, ['d', 'c', 'a', 'b']);
    }
}
