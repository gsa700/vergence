// Alt+Tab and Alt+` (docs/windows-milestone-1.md, steps 1 and 5): switching
// windows by the order they were last used.
//
// The seat (vergence-seat keyboard.rs) decides which keys switch and keeps the
// order of use; this takes a snapshot of the windows when switching begins and
// steps through it, so windows opening or closing meanwhile do not move the
// selection under the hand. The windows are the dock's (shelf.rs
// `shelf_items`): real windows, minimized ones included, no menus or tooltips;
// Alt+` keeps to the dock item the focused window belongs to, so it agrees
// with the dock about what one application is.
//
// The list (step 5): once Alt has been held 0.15 s after the first Tab (so a
// quick Alt+Tab switches without a flash), a frosted panel in the middle of
// the screen with the focused window: a row for each window, its
// application's icon (the dock's) and its title, the minimized ones faint,
// the one stepped to shaded. A click on a row chooses it.

use std::{
    hash::{Hash, Hasher},
    sync::Arc,
    time::{Duration, Instant},
};

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
    utils::{IsAlive, Logical, Point, Rectangle, Transform},
};
use vergence_scene::{Color, Image, Point as ScenePoint, Rect as SceneRect, Scene};
use vergence_seat::keyboard::Switch;

use crate::{Server, decor::SceneElement, win::Win};

/// How long Alt is held after the first Tab before the list shows.
const SHOW_AFTER: Duration = Duration::from_millis(150);
/// A row's height, the panel's padding, the icon's size, in points.
const ROW_H: i32 = 40;
const PAD: i32 = 8;
const ICON: i32 = 28;

/// A row of the list: its application's icon name, the window's title, minimized.
struct Row {
    icon: String,
    title: String,
    minimized: bool,
}

/// Switching in progress: the windows in the order of use, and the one stepped to.
pub struct Switching {
    list: Vec<Win>,
    at: usize,
    rows: Vec<Row>,
    since: Instant,
    /// The screen the list is shown on (the focused window's, else the pointer's).
    screen: Option<Rectangle<i32, Logical>>,
    /// The first row shown, when there are more than fit.
    top: usize,
    picture: Option<(u64, MemoryRenderBuffer)>,
    backdrop: crate::backdrop::Slot,
}

impl Switching {
    fn step(&mut self, back: bool) {
        let n = self.list.len();
        self.at = if back { (self.at + n - 1) % n } else { (self.at + 1) % n };
    }

    fn shown(&self) -> bool {
        self.since.elapsed() >= SHOW_AFTER
    }

    /// The panel: as wide as a third of the screen within limits, as tall as its
    /// rows within three fifths of the screen; in the screen's middle.
    fn panel(&self) -> Option<(Rectangle<i32, Logical>, usize)> {
        let s = self.screen?;
        let w = (s.size.w * 2 / 5).clamp(360, 640).min(s.size.w - 32);
        let fit = ((s.size.h * 3 / 5 - 2 * PAD) / ROW_H).max(1) as usize;
        let rows = self.list.len().min(fit);
        let h = rows as i32 * ROW_H + 2 * PAD;
        Some((Rectangle::new((s.loc.x + (s.size.w - w) / 2, s.loc.y + (s.size.h - h) / 2).into(), (w, h).into()), rows))
    }

    /// Keep the one stepped to among the rows shown.
    fn keep_in_view(&mut self) {
        let Some((_, rows)) = self.panel() else { return };
        if self.at < self.top {
            self.top = self.at;
        } else if self.at >= self.top + rows {
            self.top = self.at + 1 - rows;
        }
    }
}

impl Server {
    pub fn switch_key(&mut self, s: Switch) {
        match s {
            Switch::Begin { app, back } => {
                let list = self.switch_list(app);
                if list.is_empty() {
                    self.input.seat.switch_ended();
                    self.input.switching = None;
                    return;
                }
                // With a window focused it is first in the order: the first step is
                // the one before it. With none (a click on the desktop), the first.
                let focused = self.keyboard_window();
                let rows = self.switch_rows(&list);
                let screen = focused.as_ref().and_then(|f| self.space.element_geometry(f)).and_then(|g| {
                    let mid = (g.loc.x + g.size.w / 2, g.loc.y + g.size.h / 2);
                    self.space.outputs().filter_map(|o| self.space.output_geometry(o)).find(|o| o.contains(mid))
                });
                let screen = screen.or_else(|| self.output_at_pointer());
                let mut sw = Switching { list, at: 0, rows, since: Instant::now(), screen, top: 0, picture: None, backdrop: None };
                if focused.is_some_and(|f| sw.list[0] == f) {
                    sw.step(back);
                } else if back {
                    sw.at = sw.list.len() - 1;
                }
                sw.keep_in_view();
                self.input.switching = Some(sw);
            }
            Switch::Step { back } => {
                if let Some(sw) = self.input.switching.as_mut() {
                    sw.step(back);
                    sw.keep_in_view();
                }
            }
            Switch::Choose => {
                if let Some(sw) = self.input.switching.take() {
                    self.switch_to(sw.list[sw.at].clone(), sw.shown());
                }
            }
            Switch::Cancel => {
                if self.input.switching.take().is_some_and(|sw| sw.shown()) {
                    self.redraw_all();
                }
            }
        }
    }

    fn switch_to(&mut self, w: Win, was_shown: bool) {
        if was_shown {
            self.redraw_all();
        }
        if !w.alive() {
            return;
        }
        tracing::info!("switch: {:?}", self.window_title(&w));
        if self.is_minimized(&w) {
            self.unminimize(&w);
        } else {
            self.focus_window(&w, smithay::utils::SERIAL_COUNTER.next_serial());
        }
    }

    /// The windows to switch among, the most recently used first; those never
    /// used (restored at the session's start, say) after them, as the dock has them.
    fn switch_list(&mut self, app: bool) -> Vec<Win> {
        let items = self.shelf_items();
        let focused = self.keyboard_window();
        let windows: Vec<Win> = if app {
            let Some(f) = focused else { return Vec::new() };
            match items.into_iter().find(|(_, ws)| ws.contains(&f)) {
                Some((_, ws)) => ws,
                None => return Vec::new(),
            }
        } else {
            items.into_iter().flat_map(|(_, ws)| ws).collect()
        };
        let mut list: Vec<Win> = self.input.seat.order_of_use().iter().filter_map(|&id| self.input.window(id)).filter(|w| windows.contains(w)).collect();
        for w in windows {
            if !list.contains(&w) {
                list.push(w);
            }
        }
        list
    }

    /// Each window's row: its dock item's icon, its title (no control
    /// characters, at most 200), and whether it is minimized.
    fn switch_rows(&mut self, list: &[Win]) -> Vec<Row> {
        let items = self.shelf_items();
        list.iter()
            .map(|w| {
                let icon = items.iter().find(|(_, ws)| ws.contains(w)).map(|(i, _)| i.icon.clone()).unwrap_or_default();
                let title: String = self.window_title(w).chars().filter(|c| !c.is_control()).take(200).collect();
                Row { icon, title, minimized: self.is_minimized(w) }
            })
            .collect()
    }

    /// Is `pos` on the shown list (dispatch.rs: a press there is the list's)?
    pub fn switcher_contains(&self, pos: Point<f64, Logical>) -> bool {
        self.input.switching.as_ref().filter(|sw| sw.shown()).and_then(|sw| sw.panel()).is_some_and(|(r, _)| r.to_f64().contains(pos))
    }

    /// A click on the list: the row under it is chosen.
    pub fn switch_click(&mut self, pos: Point<f64, Logical>) {
        let Some(sw) = self.input.switching.as_ref() else { return };
        let Some((panel, rows)) = sw.panel() else { return };
        let row = ((pos.y - (panel.loc.y + PAD) as f64) / ROW_H as f64).floor();
        if row < 0.0 || row as usize >= rows {
            return;
        }
        let i = sw.top + row as usize;
        let Some(sw) = self.input.switching.take() else { return };
        self.input.seat.switch_ended();
        if let Some(w) = sw.list.get(i).cloned() {
            self.switch_to(w, true);
        }
    }

    /// The list's picture on `out`, once it shows.
    pub fn switcher_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        let Some((panel, rows)) = self.input.switching.as_ref().filter(|sw| sw.shown()).and_then(|sw| sw.panel()) else { return Vec::new() };
        if !panel.overlaps(out) {
            return Vec::new();
        }
        let light = self.light_style();
        let s = vergence_look::shelf_look(light);
        let frost = self.frost_program(renderer);
        let pixels = (ICON as f64 * scale).round() as u32;
        // The rows shown, each with its icon once it has come.
        let (top, at) = {
            let sw = self.input.switching.as_ref().unwrap();
            (sw.top, sw.at)
        };
        let shown: Vec<(usize, String, String, bool)> = self.input.switching.as_ref().unwrap().rows.iter().enumerate().skip(top).take(rows).map(|(i, r)| (i, r.icon.clone(), r.title.clone(), r.minimized)).collect();
        let icons: Vec<Option<Arc<crate::shelf::Icon>>> = shown.iter().map(|(_, icon, ..)| self.shelf_icon(icon, pixels)).collect();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (top, at, rows, panel.size.w, panel.size.h, scale.to_bits(), light, frost.is_some()).hash(&mut hasher);
        for ((i, _, title, min), icon) in shown.iter().zip(&icons) {
            (i, title, min, icon.is_some()).hash(&mut hasher);
        }
        let stamp = hasher.finish();
        let color = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
        if self.input.switching.as_ref().unwrap().picture.as_ref().map(|(k, _)| *k) != Some(stamp) {
            let (w, h) = (panel.size.w as f32, panel.size.h as f32);
            let mut scene = Scene::new(w, h);
            if frost.is_none() {
                scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, color(s.menu));
            }
            scene.stroke(SceneRect::new(0.0, 0.0, w, h), vergence_look::RADIUS, vergence_look::EDGE, color(s.edge));
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            let face = fonts.face("sans-serif", 400, false);
            for (k, ((i, _, title, minimized), icon)) in shown.iter().zip(&icons).enumerate() {
                let y = (PAD + k as i32 * ROW_H) as f32;
                let row = SceneRect::new(PAD as f32 / 2.0, y, w - PAD as f32, ROW_H as f32);
                if *i == at {
                    scene.fill_rounded(row, vergence_look::RADIUS, color(s.hover));
                }
                let square = SceneRect::new(PAD as f32 + 6.0, y + (ROW_H - ICON) as f32 / 2.0, ICON as f32, ICON as f32);
                let alpha = if *minimized { 0.45 } else { 1.0 };
                match icon {
                    Some(icon) => {
                        let (iw, ih) = (icon.width as f32 / scale as f32, icon.height as f32 / scale as f32);
                        let picture = scene.image(Image { width: icon.width, height: icon.height, rgba: icon.rgba.clone() });
                        scene.draw_image(picture, SceneRect::new(square.x + (square.w - iw) / 2.0, square.y + (square.h - ih) / 2.0, iw, ih), alpha);
                    }
                    None => {
                        scene.fill_rounded(square, vergence_look::RADIUS, color(s.tile));
                    }
                }
                if let Some(face) = face {
                    let size = 14.0;
                    let m = fonts.metrics(face, size);
                    let x = square.x + square.w + 12.0;
                    let room = w - x - PAD as f32 - 8.0;
                    // Cut with an ellipsis to the room there is.
                    let mut text = if *minimized { format!("{title} (minimized)") } else { title.clone() };
                    if fonts.layout(face, size, &text).1 > room {
                        while !text.is_empty() && fonts.layout(face, size, &format!("{text}…")).1 > room {
                            text.pop();
                        }
                        text.push('…');
                    }
                    // Faint for a minimized window, except on the shaded row, where the
                    // faint ink would not read (its icon stays faint).
                    let ink = if *minimized && *i != at { s.menu_ink_off } else { s.menu_ink };
                    scene.push_clip(SceneRect::new(x, y, room, ROW_H as f32), 0.0);
                    vergence_render::text(&mut scene, fonts, face, size, color(ink), ScenePoint::new(x, (y + (ROW_H as f32 - (m.ascent + m.descent)) / 2.0 + m.ascent).round()), &text);
                    scene.pop_clip();
                }
            }
            match vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) {
                Ok(p) => {
                    let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
                    self.input.switching.as_mut().unwrap().picture = Some((stamp, buffer));
                }
                Err(e) => {
                    tracing::warn!("switch: the list could not be drawn: {e}");
                    return Vec::new();
                }
            }
        }
        let a = (panel.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let b = (panel.loc + panel.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
        let mut elements = Vec::new();
        if let Some((_, buffer)) = &self.input.switching.as_ref().unwrap().picture
            && let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(panel.size), Kind::Unspecified)
        {
            elements.push(SceneElement::Memory(el));
        }
        if let Some(program) = frost {
            let look = crate::backdrop::Look {
                tint: crate::backdrop::unit(s.menu),
                strength: vergence_look::frost_strength(self.look_setting("frost")),
                ink: crate::backdrop::unit(s.menu_ink),
                need: 5.0,
                radius: vergence_look::frost_radius(self.look_setting("frost")) * scale as f32,
                corner: vergence_look::RADIUS * scale as f32,
                bottom: true,
            };
            let dst = Rectangle::<i32, smithay::utils::Physical>::new(a, (b.x - a.x, b.y - a.y).into());
            let mut slot = self.input.switching.as_mut().unwrap().backdrop.take();
            if let Some(e) = crate::backdrop::element(renderer, &mut slot, dst, program, &look) {
                elements.push(SceneElement::Backdrop(e));
            }
            self.input.switching.as_mut().unwrap().backdrop = slot;
        }
        elements
    }

    /// For `vctl used`'s neighbor `vctl switching`: what the list holds and shows.
    pub fn switching_report(&self) -> String {
        match &self.input.switching {
            None => "not switching".into(),
            Some(sw) => {
                let mut out = vec![format!("at {} of {}, shown {}, panel {:?}", sw.at, sw.list.len(), sw.shown(), sw.panel().map(|(r, n)| (r, n)))];
                for (i, r) in sw.rows.iter().enumerate() {
                    out.push(format!("{}{}: {}{}", if i == sw.at { "> " } else { "  " }, i, r.title, if r.minimized { " (minimized)" } else { "" }));
                }
                out.join("\n")
            }
        }
    }
}
