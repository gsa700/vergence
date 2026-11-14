// Snapping by dragging (docs/windows-milestone-1.md, step 2).
//
// While a window is moved by the pointer, the pointer at a screen's edge aims
// it at a part of that screen (vergence-shell snap.rs decides which); letting
// go there puts it there. Not by default (Dave, 2026-11-14: "I am really not a
// snap window person"): by default only while Ctrl is held at some time during
// the drag ("maybe drag with a modifier like control+drag"); desktop.toml
// `[windows] snap = "ctrl" | "always" | "off"` (true and false read as always
// and off), and the other choices sit beside it there (the settings app's
// Windows page, step 6). The maximize button and the title bar carry the same
// layouts in a menu (menu.rs, `open_window_menu`), opened by a right click, a
// long press on the bar, or the pointer resting 0.7 s on the button
// (`layouts-hover`). The screen's part is taken from its work area, the one maximize
// uses, so the dock counts as the edge exactly as its own settings say.
//
// A snapped window keeps where it was before (`Snapped::before`), apart from
// `Server::restore`, which stays the maximized windows' alone (several parts
// of the server read it as "maximized"): dragging it away brings it out at
// that size under the pointer, double-clicking its bar maximizes it, and from
// there it goes back to where it was before it was snapped. Window memory
// records the part and the place before (memory.rs); a window that comes back
// snapped is put in its part when it is first shown (opened.rs).

use std::time::{Duration, Instant};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            },
            gles::GlesRenderer,
        },
    },
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{IsAlive, Logical, Point, Rectangle, Transform},
};
use vergence_seat::keyboard::WindowKey;
use vergence_shell::snap::{Arrow, KeyAct, Standing, Zone, key_act, zone_at, zone_rect_gapped};

use crate::{Server, decor::SceneElement, win::Win};

/// How long the pointer rests at an edge before the preview shows.
const DWELL: Duration = Duration::from_millis(150);
/// How long it must press against an edge shared with another screen, where
/// the settings let that snap at all.
const PRESS: Duration = Duration::from_millis(500);

/// When a drag to an edge snaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapMode {
    Off,
    /// Only if Ctrl is held at some time during the drag (the default).
    Ctrl,
    Always,
}

/// How long the pointer rests on a maximize button before its layouts show.
const HOVER: Duration = Duration::from_millis(700);

/// desktop.toml `[windows]`: the choices, with their defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapConfig {
    /// `snap`: when dragging to an edge snaps (default "ctrl").
    pub mode: SnapMode,
    /// `top-edge = "half"`: the top edge gives the top half (default "maximize").
    pub top_half: bool,
    /// `between-screens = "press"`: edges with a screen beyond snap after a
    /// press (default "never").
    pub between: bool,
    /// `preview`: show where it will go (default true).
    pub preview: bool,
    /// `layouts-hover`: the pointer resting on a maximize button opens its
    /// layouts (default true; a right click always does).
    pub layouts_hover: bool,
    /// `gaps`: snapped windows keep a little room round and between them
    /// (default true; vergence-shell `snap::GAP`).
    pub gaps: bool,
    /// `window-keys`: Super with the arrows (default true).
    pub window_keys: bool,
    /// `app-switch`: Alt+` among one application's windows (default true).
    pub app_switch: bool,
    /// `alt-tab = "screen"`: Alt+Tab lists only the windows on the focused
    /// window's screen (default "all").
    pub alt_tab_screen: bool,
}

impl SnapConfig {
    fn gap(&self) -> i32 {
        if self.gaps { vergence_shell::snap::GAP } else { 0 }
    }
}

impl Default for SnapConfig {
    fn default() -> SnapConfig {
        SnapConfig { mode: SnapMode::Ctrl, top_half: false, between: false, preview: true, layouts_hover: true, gaps: true, window_keys: true, app_switch: true, alt_tab_screen: false }
    }
}

/// The `[windows]` table of desktop.toml's text.
pub fn config_from(text: &str) -> SnapConfig {
    let d = SnapConfig::default();
    let Some(t) = text.parse::<toml::Table>().ok().and_then(|t| t.get("windows").and_then(|w| w.as_table()).cloned()) else { return d };
    let flag = |k: &str, or: bool| t.get(k).and_then(|v| v.as_bool()).unwrap_or(or);
    let word = |k: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mode = match t.get("snap") {
        Some(toml::Value::Boolean(true)) => SnapMode::Always,
        Some(toml::Value::Boolean(false)) => SnapMode::Off,
        Some(toml::Value::String(s)) if s == "always" => SnapMode::Always,
        Some(toml::Value::String(s)) if s == "off" => SnapMode::Off,
        _ => d.mode,
    };
    SnapConfig { mode, top_half: word("top-edge") == "half", between: word("between-screens") == "press", preview: flag("preview", d.preview), layouts_hover: flag("layouts-hover", d.layouts_hover), gaps: flag("gaps", d.gaps), window_keys: flag("window-keys", d.window_keys), app_switch: flag("app-switch", d.app_switch), alt_tab_screen: word("alt-tab") == "screen" }
}

fn load_config() -> SnapConfig {
    let Some(home) = std::env::var_os("HOME") else { return SnapConfig::default() };
    config_from(&std::fs::read_to_string(std::path::Path::new(&home).join(".config/vergence/desktop.toml")).unwrap_or_default())
}

/// A snapped window: its part of the screen, and where it was before.
#[derive(Debug, Clone, Copy)]
pub struct Snapped {
    pub zone: Zone,
    pub before: Rectangle<i32, Logical>,
}

/// Where a dragged window is aimed: the part, its rectangle (bar included),
/// the screen, since when, and whether the edge is shared with another screen.
#[derive(Debug, Clone, Copy)]
struct Aim {
    zone: Zone,
    rect: Rectangle<i32, Logical>,
    screen: Rectangle<i32, Logical>,
    since: Instant,
    shared: bool,
}

impl Aim {
    /// Letting go now snaps: at once at an outer edge, after a press at a shared one.
    fn ready(&self) -> bool {
        !self.shared || self.since.elapsed() >= PRESS
    }
    /// The preview shows.
    fn shown(&self) -> bool {
        self.since.elapsed() >= if self.shared { PRESS } else { DWELL }
    }
}

pub struct Snap {
    pub config: SnapConfig,
    aim: Option<Aim>,
    /// Ctrl has been held during this drag (with the "ctrl" mode it snaps).
    pub ctrl_seen: bool,
    /// The maximize button the pointer rests on, and since when.
    hover: Option<(Win, Instant)>,
    pub snapped: std::collections::HashMap<Win, Snapped>,
    /// Parts window memory had for windows not yet shown (opened.rs applies them).
    pub recalled: std::collections::HashMap<Win, Zone>,
    picture: Option<((i32, i32, u64, bool, (u8, u8, u8)), MemoryRenderBuffer, (i32, i32))>,
}

impl Default for Snap {
    fn default() -> Snap {
        Snap { config: load_config(), aim: None, ctrl_seen: false, hover: None, snapped: Default::default(), recalled: Default::default(), picture: None }
    }
}

fn shell_rect(r: Rectangle<i32, Logical>) -> vergence_shell::Rect {
    vergence_shell::Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h)
}

fn rectangle(r: vergence_shell::Rect) -> Rectangle<i32, Logical> {
    Rectangle::new((r.x, r.y).into(), (r.w, r.h).into())
}

impl Server {
    /// desktop.toml changed (or the session started): the choices again.
    pub fn snap_reload(&mut self) {
        let c = load_config();
        if c != self.snap.config {
            tracing::info!("snap: {c:?}");
            self.snap.config = c;
        }
        // The seat filters these keys: it is told whether they are the server's.
        self.input.seat.set_window_keys(c.window_keys, c.app_switch);
    }

    /// The pointer moved while a window is moved by it: aim, or stop aiming.
    pub fn snap_track(&mut self, pos: Point<f64, Logical>) {
        let c = self.snap.config;
        if self.input.keys.as_ref().is_some_and(|k| k.mods().ctrl) {
            self.snap.ctrl_seen = true;
        }
        let on = match c.mode {
            SnapMode::Off => false,
            SnapMode::Ctrl => self.snap.ctrl_seen,
            SnapMode::Always => true,
        };
        let aim = if on { self.snap_aim(pos, c) } else { None };
        let was = self.snap.aim.map(|a| (a.zone, a.screen, a.shown()));
        match (self.snap.aim, aim) {
            // Still the same part of the same screen: it keeps its time.
            (Some(a), Some((zone, screen, ..))) if a.zone == zone && a.screen == screen => {}
            (_, Some((zone, screen, rect, shared))) => self.snap.aim = Some(Aim { zone, rect, screen, since: Instant::now(), shared }),
            (_, None) => self.snap.aim = None,
        }
        if was.is_some_and(|(.., shown)| shown) && self.snap.aim.map(|a| (a.zone, a.screen)) != was.map(|(z, s, _)| (z, s)) {
            self.redraw_all();
        }
    }

    fn snap_aim(&self, pos: Point<f64, Logical>, c: SnapConfig) -> Option<(Zone, Rectangle<i32, Logical>, Rectangle<i32, Logical>, bool)> {
        let screens: Vec<Rectangle<i32, Logical>> = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).collect();
        let screen = *screens.iter().find(|g| g.to_f64().contains(pos))?;
        let all: Vec<vergence_shell::Rect> = screens.iter().map(|g| shell_rect(*g)).collect();
        let (zone, shared) = zone_at(pos.x, pos.y, shell_rect(screen), &all)?;
        if shared && !c.between {
            return None;
        }
        let (r, _) = zone_rect_gapped(zone, shell_rect(self.work_area(screen)), c.top_half, c.gap());
        Some((zone, screen, rectangle(r), shared))
    }

    /// The move ended: if it was let go aimed at a part, the window goes there,
    /// and back later to `from`, where the drag took it from (at its size).
    pub fn snap_release(&mut self, w: &Win, from: Point<i32, Logical>) {
        let Some(aim) = self.snap.aim.take() else { return };
        if aim.shown() {
            self.redraw_all();
        }
        if aim.ready() && w.alive() {
            if !self.snap.snapped.contains_key(w)
                && let Some(geo) = self.space.element_geometry(w)
            {
                self.snap_apply_from(w, aim.zone, aim.screen, Some(Rectangle::new(from, geo.size)));
            } else {
                self.snap_apply(w, aim.zone, aim.screen);
            }
        }
    }

    /// The move ended some other way (the window went): nothing is aimed.
    pub fn snap_cancel(&mut self) {
        if self.snap.aim.take().is_some_and(|a| a.shown()) {
            self.redraw_all();
        }
    }

    /// Put a window in `zone` of `screen` (its work area), remembering where it
    /// was. The top edge without `top_half` maximizes instead.
    pub fn snap_apply(&mut self, w: &Win, zone: Zone, screen: Rectangle<i32, Logical>) {
        self.snap_apply_from(w, zone, screen, None);
    }

    /// The same, with where it goes back to given (else where it is now).
    fn snap_apply_from(&mut self, w: &Win, zone: Zone, screen: Rectangle<i32, Logical>, from: Option<Rectangle<i32, Logical>>) {
        self.snap.snapped.retain(|w, _| w.alive());
        let area = self.work_area(screen);
        let (r, max) = zone_rect_gapped(zone, shell_rect(area), self.snap.config.top_half, self.snap.config.gap());
        if max {
            if !self.restore.contains_key(w) {
                self.toggle_maximize(w);
            }
            return;
        }
        // Its place before: kept from a first snap through later ones, and from
        // before it was maximized if it was.
        let before = self.snap.snapped.get(w).map(|s| s.before).or_else(|| self.restore.remove(w)).or(from).or_else(|| self.space.element_geometry(w));
        let Some(before) = before else { return };
        let r = rectangle(r);
        let top = if self.has_title(w) { self.bar_h_in(screen) } else { 0 };
        let content = Rectangle::new((r.loc.x, r.loc.y + top).into(), (r.size.w, r.size.h - top).into());
        self.set_window_rect(w, content, false);
        self.set_tiled(w, Some(zone));
        self.snap.snapped.insert(w.clone(), Snapped { zone, before });
        tracing::info!("snap: {:?} to {} at {:?}", self.window_title(w), zone.as_str(), content);
        self.remember(w);
    }

    /// The place a snapped window had before, taking it out of its part (its
    /// size and place are the caller's to set). None if it was not snapped.
    pub fn unsnap(&mut self, w: &Win) -> Option<Rectangle<i32, Logical>> {
        let s = self.snap.snapped.remove(w)?;
        self.set_tiled(w, None);
        Some(s.before)
    }

    /// Tell a Wayland window which of its edges touch a neighbor or the screen
    /// (xdg-shell's tiled states: GTK and others drop shadows and grips there).
    /// Native windows' frames are the server's, so there is nothing to tell them.
    pub fn set_tiled(&mut self, w: &Win, zone: Option<Zone>) {
        let Some(t) = w.toplevel() else { return };
        use xdg_toplevel::State as S;
        let (l, r, top, b) = match zone {
            Some(Zone::Left) => (true, false, true, true),
            Some(Zone::Right) => (false, true, true, true),
            Some(Zone::TopLeft) => (true, false, true, false),
            Some(Zone::TopRight) => (false, true, true, false),
            Some(Zone::BottomLeft) => (true, false, false, true),
            Some(Zone::BottomRight) => (false, true, false, true),
            Some(Zone::Top) => (true, true, true, false),
            None => (false, false, false, false),
        };
        t.with_pending_state(|s| {
            for (on, st) in [(l, S::TiledLeft), (r, S::TiledRight), (top, S::TiledTop), (b, S::TiledBottom)] {
                if on {
                    s.states.set(st);
                } else {
                    s.states.unset(st);
                }
            }
        });
        t.send_pending_configure();
    }

    /// A window window memory had snapped, now shown: into its part of the
    /// screen its middle is on.
    pub fn snap_recalled(&mut self, w: &Win) {
        let Some(zone) = self.snap.recalled.remove(w) else { return };
        let Some(geo) = self.space.element_geometry(w) else { return };
        let mid = (geo.loc.x + geo.size.w / 2, geo.loc.y + geo.size.h / 2);
        let screen = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).find(|g| g.contains(mid)).or_else(|| self.output_at_pointer());
        if let Some(screen) = screen {
            self.snap_apply(w, zone, screen);
        }
    }

    /// The preview: where the dragged window will go, a rounded tint in the
    /// accent with a line round it, over the windows.
    pub fn snap_preview_element(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64, light: bool) -> Option<SceneElement> {
        if !self.is_moving() {
            self.snap.aim = None;
            return None;
        }
        let aim = self.snap.aim.filter(|a| a.shown() && self.snap.config.preview)?;
        // Where the window will go; without gaps a little inside, so its line
        // shows against the screen's edge.
        let inset = if self.snap.config.gaps { 0 } else { 6 };
        let r = Rectangle::<i32, Logical>::new((aim.rect.loc.x + inset, aim.rect.loc.y + inset).into(), (aim.rect.size.w - 2 * inset, aim.rect.size.h - 2 * inset).into());
        if !r.overlaps(out) || r.size.w <= 0 || r.size.h <= 0 {
            return None;
        }
        let a = self.accent();
        let key = (r.size.w, r.size.h, scale.to_bits(), light, (a.r, a.g, a.b));
        if self.snap.picture.as_ref().is_none_or(|(k, ..)| *k != key) {
            let (w, h) = (r.size.w as f32, r.size.h as f32);
            let mut scene = vergence_scene::Scene::new(w, h);
            let rect = vergence_scene::Rect::new(0.0, 0.0, w, h);
            scene.fill_rounded(rect, vergence_look::RADIUS, vergence_scene::Color::rgba(a.r, a.g, a.b, if light { 48 } else { 64 }));
            scene.stroke(rect, vergence_look::RADIUS, 2.0, vergence_scene::Color::rgba(a.r, a.g, a.b, 220));
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            match vergence_render::Cpu::new().render(&scene, fonts, scale as f32, vergence_scene::Color::rgba(0, 0, 0, 0)) {
                Ok(p) => {
                    let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
                    self.snap.picture = Some((key, buffer, (p.width as i32, p.height as i32)));
                }
                Err(e) => {
                    tracing::warn!("snap preview: {e}");
                    return None;
                }
            }
        }
        let (_, buffer, px) = self.snap.picture.as_ref()?;
        let whole = Rectangle::<f64, Logical>::from_size((px.0 as f64, px.1 as f64).into());
        let loc = (r.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        MemoryRenderBufferRenderElement::from_buffer(renderer, loc.to_f64(), buffer, None, Some(whole), Some(r.size), Kind::Unspecified).ok().map(SceneElement::Memory)
    }

    /// Super with an arrow, for the window with the keyboard (vergence-shell
    /// `key_act` decides; docs/windows-milestone-1.md, "Keys for windows").
    pub fn window_key(&mut self, k: WindowKey) {
        let Some(w) = self.keyboard_window() else { return };
        if self.is_minimized(&w) {
            return;
        }
        let screens: Vec<Rectangle<i32, Logical>> = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).collect();
        // The screen most of it is on (its place before, if maximized or snapped,
        // would put a window back on a screen it has left by keys).
        let Some(geo) = self.space.element_geometry(&w) else { return };
        let at = screens.iter().enumerate().max_by_key(|(_, g)| g.intersection(geo).map_or(0, |i| i.size.w as i64 * i.size.h as i64)).map(|(i, _)| i);
        let Some(at) = at else { return };
        let standing = match (self.restore.contains_key(&w), self.snap.snapped.get(&w)) {
            (true, _) => Standing::Maximized,
            (_, Some(s)) => Standing::Snapped(s.zone),
            _ => Standing::Free,
        };
        let arrow = match k {
            WindowKey::Left => Arrow::Left,
            WindowKey::Right => Arrow::Right,
            WindowKey::Up => Arrow::Up,
            WindowKey::Down => Arrow::Down,
        };
        let all: Vec<vergence_shell::Rect> = screens.iter().map(|g| shell_rect(*g)).collect();
        let act = key_act(standing, arrow, at, &all);
        tracing::info!("window key {k:?}: {:?} {standing:?} -> {act:?}", self.window_title(&w));
        match act {
            KeyAct::Snap(zone, i) => self.snap_apply(&w, zone, screens[i]),
            KeyAct::Maximize => {
                if !self.restore.contains_key(&w) {
                    self.toggle_maximize(&w);
                }
            }
            KeyAct::Restore => {
                if self.restore.contains_key(&w) {
                    self.toggle_maximize(&w);
                } else if let Some(before) = self.unsnap(&w) {
                    self.set_window_rect(&w, before, false);
                    self.remember(&w);
                }
            }
            KeyAct::Minimize => self.minimize(&w),
            KeyAct::Nothing => {}
        }
    }

    /// A choice in a window's menu (menu.rs).
    pub fn layout_act(&mut self, w: &Win, act: crate::menu::LayoutAct) {
        use crate::menu::LayoutAct;
        match act {
            LayoutAct::Snap(zone) => {
                let Some(geo) = self.space.element_geometry(w) else { return };
                let screens: Vec<Rectangle<i32, Logical>> = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).collect();
                let screen = screens.iter().max_by_key(|g| g.intersection(geo).map_or(0, |i| i.size.w as i64 * i.size.h as i64)).copied();
                if let Some(screen) = screen {
                    self.snap_apply(w, zone, screen);
                }
            }
            LayoutAct::Maximize => {
                if !self.restore.contains_key(w) {
                    self.toggle_maximize(w);
                }
            }
            LayoutAct::Restore => {
                if self.restore.contains_key(w) {
                    self.toggle_maximize(w);
                } else if let Some(before) = self.unsnap(w) {
                    self.set_window_rect(w, before, false);
                    self.remember(w);
                }
            }
            LayoutAct::Minimize => self.minimize(w),
            LayoutAct::Close => self.close_window(w),
        }
        self.redraw_all();
    }

    /// Each frame: the pointer resting on a maximize button opens its layouts
    /// after `HOVER`; a menu opened that way closes once the pointer has left
    /// both the button and the menu (with the gap between them to cross).
    pub fn layouts_tick(&mut self) {
        let at = self.pointer_location();
        let on_button = match self.hit(at) {
            crate::decor::Hit::Maximize(w) => Some(w),
            _ => None,
        };
        if self.menu_by_hover() {
            if on_button.is_none() && !self.menu_near(at, vergence_look::SHELF_POPUP_GAP as i32 + 4) {
                self.close_menu();
                self.redraw_all();
            }
            self.snap.hover = None;
            return;
        }
        let quiet = self.snap.config.layouts_hover && !self.menu_is_open() && !self.input.seat.buttons_held() && !self.is_moving();
        match on_button.filter(|_| quiet) {
            Some(w) => match &self.snap.hover {
                Some((hw, since)) if *hw == w => {
                    if since.elapsed() >= HOVER {
                        self.snap.hover = None;
                        self.open_window_menu(&w, true);
                        self.set_menu_by_hover();
                        self.redraw_all();
                    }
                }
                _ => self.snap.hover = Some((w, Instant::now())),
            },
            None => self.snap.hover = None,
        }
    }

    /// For `vctl snap`: the choices, what is aimed, and the snapped windows.
    pub fn snap_report(&self) -> String {
        let mut out = vec![format!("{:?}", self.snap.config)];
        out.push(match self.snap.aim {
            Some(a) => format!("aimed: {} {:?} shared {} shown {} ready {}", a.zone.as_str(), a.rect, a.shared, a.shown(), a.ready()),
            None => "aimed: nothing".into(),
        });
        for (w, s) in &self.snap.snapped {
            if w.alive() {
                out.push(format!("snapped: {:?} {} (before {:?})", self.window_title(w), s.zone.as_str(), s.before));
            }
        }
        out.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapping_is_off_unless_desktop_toml_says_so() {
        assert_eq!(config_from(""), SnapConfig::default());
        assert_eq!(SnapConfig::default().mode, SnapMode::Ctrl, "only with Ctrl held unless the settings say otherwise");
        assert_eq!(config_from("[windows]\npreview = false\n").mode, SnapMode::Ctrl);
        let c = config_from("[windows]\nsnap = true\ntop-edge = \"half\"\nbetween-screens = \"press\"\npreview = false\nlayouts-hover = false\n");
        assert_eq!(c, SnapConfig { mode: SnapMode::Always, top_half: true, between: true, preview: false, layouts_hover: false, gaps: true, window_keys: true, app_switch: true, alt_tab_screen: false });
        let c = config_from("[windows]\nwindow-keys = false\napp-switch = false\nalt-tab = \"screen\"\n");
        assert!(!c.window_keys && !c.app_switch && c.alt_tab_screen);
        assert!(!config_from("[windows]\ngaps = false\n").gaps);
        assert_eq!(config_from("[windows]\nsnap = false\n").mode, SnapMode::Off);
        assert_eq!(config_from("[windows]\nsnap = \"off\"\n").mode, SnapMode::Off);
        assert_eq!(config_from("[windows]\nsnap = \"always\"\n").mode, SnapMode::Always);
        // Words it does not know leave the defaults.
        let c = config_from("[windows]\nsnap = \"sometimes\"\ntop-edge = \"sideways\"\n");
        assert!(c.mode == SnapMode::Ctrl && !c.top_half && !c.between && c.preview && c.layouts_hover);
    }
}
