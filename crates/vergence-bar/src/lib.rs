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
pub const STANDARD: Sizes =
    Sizes { bar_h: BAR_H, item_h: TAB_H, button_w: BUTTON_W, plus_w: PLUS_W, side: SIDE, gap: GAP };
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
    /// The app button (app settings milestone 1): the app's icon and a
    /// chevron; it opens the app menu. GNOME's layout calls it `appmenu`.
    AppMenu,
}

impl WinButton {
    /// Its width at a size of controls: the app button holds an icon and a chevron.
    pub fn width(self, z: &Sizes) -> f32 {
        match self {
            WinButton::AppMenu => z.button_w + 14.0,
            _ => z.button_w,
        }
    }
}

/// The layout with the app button in it (app settings milestone 1): where
/// the layout puts `appmenu`, else at the end opposite the window controls
/// (controls on the left: far right; on the right: far left, as Windows 95
/// had its system menu; on both sides or none: far right). Every window has
/// one: it is not optional, or the uniformity is gone.
pub fn with_app_menu((mut left, mut right): (Vec<WinButton>, Vec<WinButton>)) -> (Vec<WinButton>, Vec<WinButton>) {
    if !left.contains(&WinButton::AppMenu) && !right.contains(&WinButton::AppMenu) {
        if right.is_empty() || !left.is_empty() {
            right.push(WinButton::AppMenu);
        } else {
            left.insert(0, WinButton::AppMenu);
        }
    }
    (left, right)
}

/// The layout without the app button: for bars whose program has no app
/// menu to open yet (the toolkit's own bar under other desktops).
pub fn without_app_menu((mut left, mut right): (Vec<WinButton>, Vec<WinButton>)) -> (Vec<WinButton>, Vec<WinButton>) {
    left.retain(|b| *b != WinButton::AppMenu);
    right.retain(|b| *b != WinButton::AppMenu);
    (left, right)
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
    // No app button where the toolkit draws its own bar: its menu is the server's.
    without_app_menu(decoration_layout(
        &host_layout().or_else(|| layout_setting(&text)).unwrap_or_else(|| DEFAULT_LAYOUT.into()),
    ))
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
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")));
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
                "appmenu" => Some(WinButton::AppMenu),
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
    /// One of the program's own buttons ([`Layout::tools_sized`]), by its place.
    Tool(usize),
    /// Bare bar: dragging it moves the window.
    Bar,
}

/// The icon of a program's own bar button (window styles milestone 1: the
/// settings app's back and forward).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolIcon {
    Back,
    Forward,
    /// Its name as text ([`Look::tool_names`]), at the bar's far end
    /// (preview milestone 1: "Open with Reader").
    Label,
}

/// The width of a text button showing `name` (a measure from its letters,
/// so the layout needs no fonts; the text is drawn centered in it).
pub fn label_width(name: &str) -> f32 {
    (name.chars().count() as f32 * FONT_SIZE * 0.56 + 24.0).clamp(48.0, 320.0)
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
    /// The program's own buttons ([`Layout::tools_sized`]), left to right.
    pub tools: Vec<Rect>,
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
        // The tabs stand at the bar's bottom edge, joined to the page (Dave,
        // 2026-11-10); the window's buttons in the bar's middle, as in a plain
        // bar (Dave, 2026-11-21: "it feels like the close minimize are closer
        // to the bottom of the titlebar": at the tabs' height they had 9
        // points above and 3 below).
        let (buttons, start, end) = Layout::buttons(width, left, right, &z, ((z.bar_h - z.item_h) / 2.0).round());
        // The tabs share what is left after +, down to a least width.
        let room = (end - start - z.plus_w - z.gap).max(0.0);
        let n = count.max(1) as f32;
        let tab_w = ((room - z.gap * n) / n).clamp(TAB_MIN, TAB_W);
        let controls_left = !left.is_empty();
        // The tabs a touch taller than the buttons, reaching up from the
        // bottom edge, so they stand apart from the centered controls (Dave,
        // 2026-11-21: "it is nice that the tab is a bit taller, I think it
        // sets them off the controls"): 3 points at Standard and Large, 2 at Compact.
        let tab_h = z.item_h + ((z.bar_h - z.item_h) * 0.4).round();
        let top = z.bar_h - tab_h;
        let (mut tabs, mut closes) = (Vec::new(), Vec::new());
        let mut x = start;
        for i in 0..count {
            let tab = Rect::new(x, top, tab_w, tab_h);
            let close = (tab_w >= CLOSE_NEEDS || (i == shown && tab_w >= 44.0)).then(|| {
                let cx = if controls_left { tab.x + 4.0 } else { tab.x + tab_w - 24.0 };
                Rect::new(cx, top + (tab_h - 20.0) / 2.0, 20.0, 20.0)
            });
            tabs.push(tab);
            closes.push(close);
            x += tab_w + z.gap;
        }
        Layout {
            buttons,
            tabs,
            closes,
            plus: Rect::new(x, top, z.plus_w, tab_h),
            title: None,
            tools: Vec::new(),
            controls_left,
            sizes: z,
        }
    }

    /// The bar of a window without tabs: its buttons, and between them the
    /// room its title is written in. The whole of it moves the window.
    pub fn plain(width: f32, left: &[WinButton], right: &[WinButton]) -> Layout {
        Layout::plain_sized(width, left, right, STANDARD)
    }

    /// [`Layout::plain`] at a size of controls.
    pub fn plain_sized(width: f32, left: &[WinButton], right: &[WinButton], z: Sizes) -> Layout {
        // No tabs to stand with: everything in the bar's middle (Dave,
        // 2026-11-14: "the buttons never got centered").
        let top = ((z.bar_h - z.item_h) / 2.0).round();
        let (buttons, start, end) = Layout::buttons(width, left, right, &z, top);
        let title = Rect::new(start + 6.0, top, (end - start - 12.0).max(0.0), z.item_h);
        Layout {
            buttons,
            tabs: Vec::new(),
            closes: Vec::new(),
            plus: Rect::new(end, top, 0.0, z.item_h),
            title: Some(title),
            tools: Vec::new(),
            controls_left: !left.is_empty(),
            sizes: z,
        }
    }

    /// The bar of a window with `count` buttons of its own (back and forward,
    /// say) and the title after them, both starting `from` points in (the
    /// width of a sidebar the content has on the left), or after the window's
    /// buttons if they reach further.
    pub fn tools_sized(
        width: f32,
        left: &[WinButton],
        right: &[WinButton],
        z: Sizes,
        from: f32,
        tools: &[Option<f32>],
    ) -> Layout {
        // Centered, as in a plain bar. Icon buttons from `from` on, after
        // the window's buttons on the left; text buttons (their widths given)
        // at the far end, before the window's buttons on the right.
        let top = ((z.bar_h - z.item_h) / 2.0).round();
        let (buttons, start, mut end) = Layout::buttons(width, left, right, &z, top);
        let mut rects = vec![Rect::new(0.0, top, 0.0, z.item_h); tools.len()];
        for (i, t) in tools.iter().enumerate().rev() {
            if let Some(w) = *t
                && end - w > start
            {
                end -= w;
                rects[i] = Rect::new(end, top, w, z.item_h);
                end -= z.gap;
            }
        }
        let mut x = start.max(from + z.side);
        for (i, t) in tools.iter().enumerate() {
            if t.is_none() && x + z.button_w <= end {
                rects[i] = Rect::new(x, top, z.button_w, z.item_h);
                x += z.button_w + z.gap;
            }
        }
        let title = Rect::new(x + 6.0, top, (end - x - 12.0).max(0.0), z.item_h);
        Layout {
            buttons,
            tabs: Vec::new(),
            closes: Vec::new(),
            plus: Rect::new(end, top, 0.0, z.item_h),
            title: Some(title),
            tools: rects,
            controls_left: !left.is_empty(),
            sizes: z,
        }
    }

    /// The window's buttons on their sides, and where the room between them
    /// starts and ends.
    fn buttons(
        width: f32,
        left: &[WinButton],
        right: &[WinButton],
        z: &Sizes,
        button_y: f32,
    ) -> (Vec<(WinButton, Rect)>, f32, f32) {
        let mut buttons = Vec::new();
        let mut x = z.side;
        for b in left {
            let w = b.width(z);
            buttons.push((*b, Rect::new(x, button_y, w, z.item_h)));
            x += w + z.gap;
        }
        let mut end = width - z.side;
        for b in right.iter().rev() {
            end -= b.width(z);
            buttons.push((*b, Rect::new(end, button_y, b.width(z), z.item_h)));
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
        if let Some(i) = self.tools.iter().position(|r| inside(r, x, y)) {
            return Some(Part::Tool(i));
        }
        Some(Part::Bar)
    }
}

/// A program button's icon, centered in its box: a chevron.
fn tool_icon(scene: &mut Scene, r: &Rect, what: ToolIcon, size: f32, ink: Color) {
    let (cx, cy, h) = (r.x + r.w / 2.0, r.y + r.h / 2.0, size / 2.0);
    let s = match what {
        ToolIcon::Back => 1.0,
        ToolIcon::Forward => -1.0,
        ToolIcon::Label => return,
    };
    let w = h * 0.6;
    scene.line(Point::new(cx + s * w, cy - h), Point::new(cx - s * w, cy), 1.5, ink);
    scene.line(Point::new(cx - s * w, cy), Point::new(cx + s * w, cy + h), 1.5, ink);
}

/// The app button's face: the app's icon on the left, a chevron on the right.
fn app_button(scene: &mut Scene, r: &Rect, picture: Option<&std::sync::Arc<vergence_scene::Image>>, ink: Color) {
    let side = (r.h - 4.0).clamp(12.0, 24.0);
    let at = Rect::new(r.x + 4.0, r.y + (r.h - side) / 2.0, side, side);
    match picture {
        Some(p) => {
            let id = scene.image(p.clone());
            scene.draw_image(id, at, 1.0);
        }
        None => {
            scene.stroke(Rect::new(at.x + 1.0, at.y + 1.0, at.w - 2.0, at.h - 2.0), RADIUS, 1.25, ink);
        }
    }
    let (cx, cy, h) = (r.x + r.w - 9.0, r.y + r.h / 2.0, 2.5);
    scene.line(Point::new(cx - h * 1.4, cy - h * 0.6), Point::new(cx, cy + h * 0.8), 1.25, ink);
    scene.line(Point::new(cx, cy + h * 0.8), Point::new(cx + h * 1.4, cy - h * 0.6), 1.25, ink);
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
    /// Which tabs have edits not saved: marked in the look's `edited` color.
    pub edited: &'a [bool],
    /// The window's title, for a bar without tabs ([`Layout::plain`]).
    pub title: &'a str,
    pub shown: usize,
    /// What the pointer is over.
    pub hover: Option<Part>,
    /// The tab being dragged (the server's, dispatch.rs): left in its place,
    /// faint, while its ghost follows the pointer and a line shows where it
    /// would land (Dave, 2026-11-12).
    pub dragged: Option<usize>,
    /// The program's own buttons' icons and whether each can be pressed
    /// ([`Layout::tools`] gives where).
    pub tools: &'a [(ToolIcon, bool)],
    /// The program's own buttons' names: a [`ToolIcon::Label`] shows its own.
    pub tool_names: &'a [String],
    /// The app's icon for the app button (the one the dock shows), drawn
    /// for this scale; None: a plain square in its place.
    pub app_icon: Option<std::sync::Arc<vergence_scene::Image>>,
    /// The app menu is open from this bar: its button stays lit.
    pub menu_open: bool,
    /// The window's program runs as the superuser (Dave's wish, 2026-11-10:
    /// "a server-drawn mark on windows of root clients"): the bar's ground
    /// goes the alarm red, as a terminal's does while a root tab is in front,
    /// and a bar without tabs shows its title as a red tab, white on red.
    pub root: bool,
}

/// Draws the bar across the top of the scene.
pub fn draw(scene: &mut Scene, fonts: &mut Fonts, layout: &Layout, look: &Look) {
    let l = look.bar;
    let ink = color(if look.focused { l.ink } else { l.ink_off });
    let shape_rest = shape(l.shape, l.shape_rest);
    let tab_rest = shape(l.shape, l.tab_rest);
    let shape_hover = shape(l.shape, l.shape_hover);
    let front_alert = look.root || (!layout.tabs.is_empty() && look.alerts.get(look.shown).copied().unwrap_or(false));
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
        let drawn = Rect::new(
            r.x + vergence_look::BUTTON_INSET_X,
            r.y + vergence_look::BUTTON_INSET_Y,
            r.w - 2.0 * vergence_look::BUTTON_INSET_X,
            r.h - 2.0 * vergence_look::BUTTON_INSET_Y,
        );
        let hovered = look.hover == Some(part) || (*b == WinButton::AppMenu && look.menu_open);
        let fill = match (hovered, *b) {
            (true, WinButton::Close) => Some(CLOSE_HOVER),
            (true, _) => Some(shape_hover),
            (false, _) => look.focused.then_some(shape_rest),
        };
        if let Some(fill) = fill {
            scene.fill_rounded(drawn, RADIUS, fill);
        }
        if *b == WinButton::AppMenu {
            app_button(scene, &drawn, look.app_icon.as_ref(), ink);
            continue;
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
        if look.dragged == Some(i) {
            // Being dragged: an empty slot with its title faint, no close.
            tab_shape(scene, tab, tab_rest);
            if let (Some(face), Some(title)) = (look.font, look.titles.get(i)) {
                let (from, to) = (tab.x + 12.0, tab.x + tab.w - 8.0);
                if to - from > 10.0 {
                    scene.push_clip(Rect::new(from, tab.y, to - from, tab.h), 0.0);
                    let m = fonts.metrics(face, look.font_size);
                    let baseline = tab.y + ((tab.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round() - 1.0;
                    text(
                        scene,
                        fonts,
                        face,
                        look.font_size,
                        Color::rgba(ink.r, ink.g, ink.b, FAINT),
                        Point::new(from, baseline),
                        title,
                    );
                    scene.pop_clip();
                }
            }
            continue;
        }
        let shown = i == look.shown;
        let hovered = matches!(look.hover, Some(Part::Tab(t) | Part::CloseTab(t)) if t == i);
        let alert = look.alerts.get(i).copied().unwrap_or(false);
        let tab_ink = if alert {
            Color::rgb(255, 255, 255)
        } else if shown {
            look.pane_ink
        } else {
            ink
        };
        tab_shape(
            scene,
            tab,
            if alert {
                color(l.alert)
            } else if shown {
                look.pane
            } else if hovered {
                shape_hover
            } else {
                tab_rest
            },
        );
        if alert && hovered {
            tab_shape(scene, tab, shape_hover);
        }
        if !alert && look.edited.get(i).copied().unwrap_or(false) {
            let e = l.edited;
            if !shown {
                tab_shape(scene, tab, Color::rgba(e.0, e.1, e.2, 56));
            }
            scene.fill_rounded(Rect::new(tab.x + 6.0, tab.y + 1.0, (tab.w - 12.0).max(0.0), 3.0), 1.5, color(e));
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
    // The program's own buttons: shaded under the pointer, faint when they cannot be pressed.
    for (i, r) in layout.tools.iter().enumerate() {
        let Some(&(what, enabled)) = look.tools.get(i) else { continue };
        if enabled && look.hover == Some(Part::Tool(i)) {
            let drawn = Rect::new(
                r.x + vergence_look::BUTTON_INSET_X,
                r.y + vergence_look::BUTTON_INSET_Y,
                r.w - 2.0 * vergence_look::BUTTON_INSET_X,
                r.h - 2.0 * vergence_look::BUTTON_INSET_Y,
            );
            scene.fill_rounded(drawn, RADIUS, shape_hover);
        }
        let mark = if enabled { ink } else { Color::rgba(ink.r, ink.g, ink.b, FAINT) };
        if what == ToolIcon::Label {
            // A text button: shaded at rest while the window has the keyboard, its name centered.
            if r.w <= 0.0 {
                continue;
            }
            let drawn = Rect::new(
                r.x + vergence_look::BUTTON_INSET_X,
                r.y + vergence_look::BUTTON_INSET_Y,
                r.w - 2.0 * vergence_look::BUTTON_INSET_X,
                r.h - 2.0 * vergence_look::BUTTON_INSET_Y,
            );
            if look.focused && !(enabled && look.hover == Some(Part::Tool(i))) {
                scene.fill_rounded(drawn, RADIUS, shape_rest);
            }
            if let (Some(face), Some(name)) = (look.font, look.tool_names.get(i)) {
                let w = fonts.layout(face, look.font_size, name).1;
                let m = fonts.metrics(face, look.font_size);
                let baseline = r.y + ((r.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round() - 1.0;
                scene.push_clip(drawn, 0.0);
                text(
                    scene,
                    fonts,
                    face,
                    look.font_size,
                    mark,
                    Point::new((r.x + (r.w - w) / 2.0).max(drawn.x + 4.0), baseline),
                    name,
                );
                scene.pop_clip();
            }
            continue;
        }
        tool_icon(scene, r, what, 12.0, mark);
    }
    if let (Some(room), Some(face)) = (&layout.title, look.font)
        && room.w > 10.0
    {
        // The title, on the tabs' line, cut where its room ends. A superuser's
        // window wears it as a red tab does: white on the alarm red, in a tab's
        // shape just wide enough for it.
        let mut at = room.x;
        let title_ink = if look.root {
            let width = fonts.layout(face, look.font_size, look.title).1;
            let pad = 12.0;
            tab_shape(scene, &Rect::new(room.x, room.y, (width + 2.0 * pad).min(room.w), room.h), color(l.alert));
            at += pad;
            Color::rgb(255, 255, 255)
        } else {
            ink
        };
        scene.push_clip(
            if look.root { Rect::new(at, room.y, (room.x + room.w - at - 8.0).max(0.0), room.h) } else { *room },
            0.0,
        );
        let m = fonts.metrics(face, look.font_size);
        let baseline = room.y + ((room.h - (m.ascent + m.descent)) / 2.0 + m.ascent).round() - 1.0;
        text(scene, fonts, face, look.font_size, title_ink, Point::new(at, baseline), look.title);
        scene.pop_clip();
    }
}

/// The alpha of a dragged tab's title left in its slot.
pub const FAINT: u8 = 90;
/// The insertion line's width, in points.
pub const GAP_LINE: f32 = 3.0;

/// Where the line goes that shows a tab landing before tab `before` of the
/// row (`before` == the count: after the last): its middle, in the window's
/// points; None with no tabs.
pub fn gap_x(layout: &Layout, before: usize) -> Option<f32> {
    let last = layout.tabs.last()?;
    Some(match layout.tabs.get(before) {
        Some(t) if before == 0 => t.x - 2.0,
        Some(t) => {
            let prev = &layout.tabs[before - 1];
            (prev.x + prev.w + t.x) / 2.0
        }
        None => last.x + last.w + 2.0,
    })
}

/// A drag of tab `from` landing at `to` ([`Landing::At`], counted with the
/// dragged tab out of the row): before which tab of the row as it stands the
/// line goes. None: its own place, nothing moves.
pub fn gap_of(from: usize, to: usize) -> Option<usize> {
    match to.cmp(&from) {
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Less => Some(to),
        std::cmp::Ordering::Greater => Some(to + 1),
    }
}

/// The ghost of a dragged tab: the tab, `w` by `h`, in `fill` with its title
/// in `ink`, for the server to draw under the pointer.
#[allow(clippy::too_many_arguments)]
pub fn draw_ghost(
    scene: &mut Scene,
    fonts: &mut Fonts,
    w: f32,
    h: f32,
    title: &str,
    font: Option<FaceId>,
    font_size: f32,
    fill: Color,
    ink: Color,
) {
    scene.fill_rounded(Rect::new(0.0, 0.0, w, h), RADIUS, fill);
    if let Some(face) = font
        && w > 30.0
    {
        scene.push_clip(Rect::new(12.0, 0.0, w - 20.0, h), 0.0);
        let m = fonts.metrics(face, font_size);
        let baseline = ((h - (m.ascent + m.descent)) / 2.0 + m.ascent).round() - 1.0;
        text(scene, fonts, face, font_size, ink, Point::new(12.0, baseline), title);
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
    /// Where along the tab the pointer took hold, from its left edge.
    pub fn grab(&self) -> f32 {
        self.grab
    }

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
        assert_eq!(
            large.at(large.tabs[0].x + 50.0, STANDARD.bar_h + 2.0),
            Some(Part::Tab(0)),
            "below a Standard bar's bottom, still a Large tab"
        );
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

    /// Icon buttons start after the window's buttons on the left; a text
    /// button stands at the far end, before those on the right, and the
    /// title has the room between; a bar too narrow leaves the text button out.
    #[test]
    fn a_text_button_stands_at_the_far_end() {
        let w = label_width("Open with Reader");
        let l = Layout::tools_sized(800.0, &[WinButton::Close], &[], STANDARD, 0.0, &[None, Some(w)]);
        assert!(l.tools[0].x >= l.buttons[0].1.x + l.buttons[0].1.w, "the icon after Close");
        assert_eq!(l.tools[1].x + l.tools[1].w, 800.0 - STANDARD.side, "the text button at the right edge");
        let title = l.title.unwrap();
        assert!(title.x + title.w <= l.tools[1].x, "the title before it");
        assert_eq!(l.at(l.tools[1].x + 5.0, l.tools[1].y + 5.0), Some(Part::Tool(1)));
        let right = Layout::tools_sized(800.0, &[], &[WinButton::Close], STANDARD, 0.0, &[Some(w)]);
        let close = right.buttons[0].1;
        assert!(right.tools[0].x + right.tools[0].w <= close.x, "before Close on the right");
        let narrow = Layout::tools_sized(120.0, &[WinButton::Close], &[], STANDARD, 0.0, &[Some(w)]);
        assert_eq!(narrow.tools[0].w, 0.0, "no room: left out");
    }

    /// The insertion line: between two tabs' edges, before the first, after
    /// the last; a landing at its own place shows none.
    #[test]
    fn the_insertion_line_stands_between_tabs() {
        let l = Layout::new(900.0, 3, 0, &LEFT, &[]);
        let (a, b) = (l.tabs[0], l.tabs[1]);
        let mid = gap_x(&l, 1).unwrap();
        assert!(mid >= a.x + a.w && mid <= b.x, "{mid} between {} and {}", a.x + a.w, b.x);
        assert!(gap_x(&l, 0).unwrap() < a.x);
        let last = l.tabs[2];
        assert!(gap_x(&l, 3).unwrap() > last.x + last.w);
        assert_eq!(gap_x(&Layout::new(900.0, 0, 0, &LEFT, &[]), 0), None);
        assert_eq!(gap_of(1, 1), None);
        assert_eq!(gap_of(2, 0), Some(0), "to the front: before the first");
        assert_eq!(gap_of(0, 2), Some(3), "first to last of three: after the last");
        assert_eq!(gap_of(0, 1), Some(2), "one along: after the next one");
    }

    #[test]
    fn the_layout_string_says_which_side() {
        assert_eq!(decoration_layout("close,minimize:"), (vec![WinButton::Close, WinButton::Minimize], vec![]));
        assert_eq!(
            decoration_layout(":minimize,maximize,close"),
            (vec![], vec![WinButton::Minimize, WinButton::Maximize, WinButton::Close])
        );
        assert_eq!(decoration_layout("menu:close"), (vec![], vec![WinButton::Close]));
        assert_eq!(decoration_layout("close"), (vec![], vec![WinButton::Close]));
        assert_eq!(decoration_layout("appmenu:close"), (vec![WinButton::AppMenu], vec![WinButton::Close]));
    }

    #[test]
    fn the_app_button_goes_opposite_the_controls_unless_the_layout_places_it() {
        use WinButton::*;
        // Controls on the left: far right. On the right: far left (Windows 95's place).
        assert_eq!(
            with_app_menu(decoration_layout("close,minimize,maximize:")),
            (vec![Close, Minimize, Maximize], vec![AppMenu])
        );
        assert_eq!(
            with_app_menu(decoration_layout(":minimize,maximize,close")),
            (vec![AppMenu], vec![Minimize, Maximize, Close])
        );
        // Both sides, or none: far right.
        assert_eq!(with_app_menu(decoration_layout("close:minimize")), (vec![Close], vec![Minimize, AppMenu]));
        assert_eq!(with_app_menu((vec![], vec![])), (vec![], vec![AppMenu]));
        // Where the layout says, once.
        assert_eq!(with_app_menu(decoration_layout("close,appmenu:")), (vec![Close, AppMenu], vec![]));
        assert_eq!(without_app_menu(decoration_layout("appmenu:close")), (vec![], vec![Close]));
        // It is wider than a button, and the room for tabs shrinks by it.
        let (l, r) = with_app_menu(decoration_layout("close,minimize,maximize:"));
        let lay = Layout::new(800.0, 1, 0, &l, &r);
        let (_, app) = lay.buttons.iter().find(|(b, _)| *b == AppMenu).unwrap();
        assert_eq!(app.w, BUTTON_W + 14.0);
        assert_eq!(app.x + app.w, 800.0 - SIDE);
        assert_eq!(lay.at(app.x + 5.0, 20.0), Some(Part::Button(AppMenu)));
        assert_eq!(decoration_layout(""), (vec![], vec![]));
        assert_eq!(
            layout_setting("[settings]\n\"Gtk/DecorationLayout\" = \"close,minimize:\"\n").as_deref(),
            Some("close,minimize:")
        );
        assert_eq!(layout_setting("[settings]\n\"Gtk/FontName\" = \"Sans 10\"\n"), None);
    }

    #[test]
    fn plus_is_always_in_the_window() {
        for width in [1264.0, 820.0, 500.0, 320.0] {
            for count in 1..=40 {
                let l = Layout::new(width, count, 0, &LEFT, &[]);
                assert_eq!(l.tabs.len(), count);
                let narrow = l.tabs[0].w <= TAB_MIN;
                assert!(
                    narrow || l.plus.x + l.plus.w <= width - SIDE + 0.01,
                    "{count} tabs in {width}: + ends at {}",
                    l.plus.x + l.plus.w
                );
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
        assert!(
            title.x >= SIDE && title.x + title.w <= first_button,
            "the title's room ends before the buttons: {title:?}"
        );
        assert_eq!(l.at(title.x + 20.0, 20.0), Some(Part::Bar), "the title's room moves the window");
        assert_eq!(l.at(640.0 - SIDE - 5.0, 20.0), Some(Part::Button(WinButton::Close)));
        assert_eq!(l.at(300.0, BAR_H + 1.0), None);
        // Buttons on the left: the title starts after them.
        let l = Layout::plain(640.0, &LEFT, &[]);
        assert!(l.title.unwrap().x > 2.0 * BUTTON_W);
    }

    /// A superuser's window: its bar's ground is the alarm red, tabs or none,
    /// focused or not; anyone else's, the plain ground.
    #[test]
    fn a_superusers_window_has_a_red_bar() {
        let mut fonts = vergence_text::Fonts::new(vergence_text::db::FontDb::default());
        let l = vergence_look::bar_look(false);
        for focused in [true, false] {
            for root in [true, false] {
                let layout = Layout::plain(640.0, &[], &[WinButton::Close]);
                let look = Look {
                    bar: l,
                    frosted: false,
                    focused,
                    pane: Color::rgb(0, 0, 0),
                    pane_ink: Color::rgb(255, 255, 255),
                    font: None,
                    font_size: FONT_SIZE,
                    titles: &[],
                    alerts: &[],
                    edited: &[],
                    title: "sudo",
                    shown: 0,
                    hover: None,
                    dragged: None,
                    tools: &[],
                    tool_names: &[],
                    app_icon: None,
                    menu_open: false,
                    root,
                };
                let mut scene = Scene::new(640.0, 60.0);
                draw(&mut scene, &mut fonts, &layout, &look);
                let want = match (root, focused) {
                    (true, true) => l.alert_ground,
                    (true, false) => l.alert_ground_off,
                    (false, true) => l.ground,
                    (false, false) => l.ground_off,
                };
                let ground = scene.commands.iter().find_map(|c| match c {
                    vergence_scene::Command::Fill { color, .. } => Some(*color),
                    _ => None,
                });
                assert_eq!(ground, Some(color(want)), "root {root}, focused {focused}");
            }
        }
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
