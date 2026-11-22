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
// the screen with the focused window. Since overview milestone 1, step 4
// (Dave, 2026-11-18: "Alt+Tab live tiles"), a tile for each window: its live
// picture (thumbs.rs) over its application's icon and title, the minimized
// ones faint with their last picture, the one stepped to outlined in the
// accent; more than fit across wrap to more rows, and when those would not
// fit the screen the tiles shrink. A click on a tile chooses it.

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
/// A tile's picture room and title row, the space between tiles, the
/// panel's padding, the title's icon, in points (at full size).
const TILE_W: i32 = 220;
const TILE_H: i32 = 138;
const TITLE_H: i32 = 28;
const GAP: i32 = 12;
const PAD: i32 = 14;
const ICON: i32 = 18;

/// The list's panel, each tile's place, and the picture room's size.
type Layout = (Rectangle<i32, Logical>, Vec<Rectangle<i32, Logical>>, (i32, i32));

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

    /// The panel and each tile's whole place (picture over title), the
    /// picture room's size: rows as wide as nine tenths of the screen allow,
    /// centered, the panel in the screen's middle; the tiles shrunk (not
    /// below a third) until the rows fit four fifths of its height.
    fn layout(&self) -> Option<Layout> {
        let s = self.screen?;
        let n = self.list.len().max(1) as i32;
        let (max_w, max_h) = (s.size.w * 9 / 10, s.size.h * 4 / 5);
        let mut k = 1.0f64;
        let (tw, th, per_row, rows) = loop {
            let (tw, th) = ((TILE_W as f64 * k).round() as i32, (TILE_H as f64 * k).round() as i32);
            let per_row = ((max_w - 2 * PAD + GAP) / (tw + GAP)).clamp(1, n);
            let rows = (n + per_row - 1) / per_row;
            let h = 2 * PAD + rows * (th + TITLE_H) + (rows - 1) * GAP;
            if h <= max_h || k < 0.34 {
                break (tw, th, per_row, rows);
            }
            k *= 0.9;
        };
        let w = 2 * PAD + per_row * tw + (per_row - 1) * GAP;
        let h = 2 * PAD + rows * (th + TITLE_H) + (rows - 1) * GAP;
        let panel = Rectangle::new((s.loc.x + (s.size.w - w) / 2, s.loc.y + (s.size.h - h) / 2).into(), (w, h).into());
        let tiles = (0..n)
            .map(|i| {
                let (row, col) = (i / per_row, i % per_row);
                // The last row, if short, centered.
                let in_row = if row == rows - 1 { n - row * per_row } else { per_row };
                let row_w = in_row * tw + (in_row - 1) * GAP;
                let x0 = panel.loc.x + (w - row_w) / 2;
                Rectangle::new(
                    (x0 + col * (tw + GAP), panel.loc.y + PAD + row * (th + TITLE_H + GAP)).into(),
                    (tw, th + TITLE_H).into(),
                )
            })
            .collect();
        Some((panel, tiles, (tw, th)))
    }

    /// The panel alone.
    fn panel(&self) -> Option<Rectangle<i32, Logical>> {
        self.layout().map(|(p, ..)| p)
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
                let mut sw =
                    Switching { list, at: 0, rows, since: Instant::now(), screen, picture: None, backdrop: None };
                if focused.is_some_and(|f| sw.list[0] == f) {
                    sw.step(back);
                } else if back {
                    sw.at = sw.list.len() - 1;
                }
                self.input.switching = Some(sw);
                // Live tiles: drawn again while the list is up (thumbs.rs).
                let _ = self.loop_handle.insert_source(
                    smithay::reexports::calloop::timer::Timer::from_duration(crate::thumbs::EVERY),
                    |_, _, st| {
                        if st.input.switching.is_none() {
                            return smithay::reexports::calloop::timer::TimeoutAction::Drop;
                        }
                        if st.input.switching.as_ref().is_some_and(|sw| sw.shown()) {
                            st.redraw_all();
                        }
                        smithay::reexports::calloop::timer::TimeoutAction::ToDuration(crate::thumbs::EVERY)
                    },
                );
            }
            Switch::Step { back } => {
                if let Some(sw) = self.input.switching.as_mut() {
                    sw.step(back);
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
            let Some(ref f) = focused else { return Vec::new() };
            match items.into_iter().find(|(_, ws)| ws.contains(f)) {
                Some((_, ws)) => ws,
                None => return Vec::new(),
            }
        } else {
            items.into_iter().flat_map(|(_, ws)| ws).collect()
        };
        // `alt-tab = "screen"`: only those on the focused window's screen (by their middle).
        let windows: Vec<Win> = match focused
            .as_ref()
            .and_then(|f| self.space.element_geometry(f))
            .filter(|_| self.snap.config.alt_tab_screen && !app)
        {
            Some(g) => {
                let mid = (g.loc.x + g.size.w / 2, g.loc.y + g.size.h / 2);
                let screen =
                    self.space.outputs().filter_map(|o| self.space.output_geometry(o)).find(|o| o.contains(mid));
                windows
                    .into_iter()
                    .filter(|w| {
                        let Some(screen) = screen else { return true };
                        // A minimized window counts on the screen it goes back to.
                        let at = self.shelf.minimized.iter().find(|(m, _)| m == w).map(|(_, p)| *p);
                        let geo = self.space.element_geometry(w);
                        let mid = match (at, geo) {
                            (Some(p), _) => (p.x + w.geometry().size.w / 2, p.y + w.geometry().size.h / 2),
                            (None, Some(g)) => (g.loc.x + g.size.w / 2, g.loc.y + g.size.h / 2),
                            _ => return true,
                        };
                        screen.contains(mid)
                    })
                    .collect()
            }
            None => windows,
        };
        let mut list: Vec<Win> = self
            .input
            .seat
            .order_of_use()
            .iter()
            .filter_map(|&id| self.input.window(id))
            .filter(|w| windows.contains(w))
            .collect();
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
        self.input
            .switching
            .as_ref()
            .filter(|sw| sw.shown())
            .and_then(|sw| sw.panel())
            .is_some_and(|r| r.to_f64().contains(pos))
    }

    /// A click on the list: the tile under it is chosen.
    pub fn switch_click(&mut self, pos: Point<f64, Logical>) {
        let Some(sw) = self.input.switching.as_ref() else { return };
        let Some((_, tiles, _)) = sw.layout() else { return };
        let Some(i) = tiles.iter().position(|t| t.to_f64().contains(pos)) else { return };
        let Some(sw) = self.input.switching.take() else { return };
        self.input.seat.switch_ended();
        if let Some(w) = sw.list.get(i).cloned() {
            self.switch_to(w, true);
        }
    }

    /// The list's picture on `out`, once it shows: the live pictures in
    /// front, the chrome (tiles' grounds, outline, icons, titles), the frost.
    pub fn switcher_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        out: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Vec<SceneElement> {
        let Some((panel, tiles, (tw, th))) =
            self.input.switching.as_ref().filter(|sw| sw.shown()).and_then(|sw| sw.layout())
        else {
            return Vec::new();
        };
        if !panel.overlaps(out) {
            return Vec::new();
        }
        let light = self.light_style();
        let s = vergence_look::shelf_look(light);
        let frost = self.frost_program(renderer);
        let accent = self.accent();
        let pixels = (ICON as f64 * scale).round() as u32;
        let at = self.input.switching.as_ref().unwrap().at;
        let rows: Vec<(String, String, bool)> = self
            .input
            .switching
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .map(|r| (r.icon.clone(), r.title.clone(), r.minimized))
            .collect();
        let list: Vec<Win> = self.input.switching.as_ref().unwrap().list.clone();
        let icons: Vec<Option<Arc<crate::shelf::Icon>>> =
            rows.iter().map(|(icon, ..)| self.shelf_icon(icon, pixels)).collect();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (
            at,
            panel.size.w,
            panel.size.h,
            tw,
            th,
            scale.to_bits(),
            light,
            frost.is_some(),
            (accent.r, accent.g, accent.b),
        )
            .hash(&mut hasher);
        for ((_, title, min), icon) in rows.iter().zip(&icons) {
            (title, min, icon.is_some()).hash(&mut hasher);
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
            for (i, (((_, title, minimized), icon), tile)) in rows.iter().zip(&icons).zip(&tiles).enumerate() {
                let (x, y) = ((tile.loc.x - panel.loc.x) as f32, (tile.loc.y - panel.loc.y) as f32);
                // The picture's room: a faint ground where the picture does not fill it.
                scene.fill_rounded(
                    SceneRect::new(x, y, tw as f32, th as f32),
                    5.0,
                    Color { a: 26, ..color(s.menu_ink) },
                );
                // The one stepped to: shaded, outlined in the accent.
                if i == at {
                    let r = SceneRect::new(x - 5.0, y - 5.0, tw as f32 + 10.0, (th + TITLE_H) as f32 + 5.0);
                    scene.fill_rounded(r, vergence_look::RADIUS + 2.0, Color { a: 60, ..color(s.hover) });
                    scene.stroke(r, vergence_look::RADIUS + 2.0, 2.5, accent);
                }
                let alpha = if *minimized { 0.45 } else { 1.0 };
                let square =
                    SceneRect::new(x + 2.0, y + th as f32 + (TITLE_H - ICON) as f32 / 2.0, ICON as f32, ICON as f32);
                match icon {
                    Some(icon) => {
                        let (iw, ih) = (icon.width as f32 / scale as f32, icon.height as f32 / scale as f32);
                        let picture =
                            scene.image(Image { width: icon.width, height: icon.height, rgba: icon.rgba.clone() });
                        scene.draw_image(
                            picture,
                            SceneRect::new(square.x + (square.w - iw) / 2.0, square.y + (square.h - ih) / 2.0, iw, ih),
                            alpha,
                        );
                    }
                    None => {
                        scene.fill_rounded(square, 4.0, color(s.tile));
                    }
                }
                if let Some(face) = face {
                    let size = 12.5;
                    let m = fonts.metrics(face, size);
                    let tx = square.x + ICON as f32 + 6.0;
                    let room = x + tw as f32 - tx;
                    let mut text = if title.is_empty() { "Untitled".to_string() } else { title.clone() };
                    if fonts.layout(face, size, &text).1 > room {
                        while !text.is_empty() && fonts.layout(face, size, &format!("{text}…")).1 > room {
                            text.pop();
                        }
                        text.push('…');
                    }
                    let ink = if *minimized && i != at { s.menu_ink_off } else { s.menu_ink };
                    scene.push_clip(SceneRect::new(tx, y + th as f32, room, TITLE_H as f32), 0.0);
                    vergence_render::text(
                        &mut scene,
                        fonts,
                        face,
                        size,
                        color(ink),
                        ScenePoint::new(
                            tx,
                            (y + th as f32 + (TITLE_H as f32 - (m.ascent + m.descent)) / 2.0 + m.ascent).round(),
                        ),
                        &text,
                    );
                    scene.pop_clip();
                }
            }
            match vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) {
                Ok(p) => {
                    let buffer = MemoryRenderBuffer::from_slice(
                        &p.data,
                        Fourcc::Abgr8888,
                        (p.width as i32, p.height as i32),
                        1,
                        Transform::Normal,
                        None,
                    );
                    self.input.switching.as_mut().unwrap().picture = Some((stamp, buffer));
                }
                Err(e) => {
                    tracing::warn!("switch: the list could not be drawn: {e}");
                    return Vec::new();
                }
            }
        }
        let px = |r: Rectangle<i32, Logical>| -> Rectangle<i32, smithay::utils::Physical> {
            let a = (r.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            let b = (r.loc + r.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            Rectangle::new(a, (b.x - a.x, b.y - a.y).into())
        };
        let mut elements = Vec::new();
        // The live pictures in front.
        for (i, w) in list.iter().enumerate() {
            let Some(tile) = tiles.get(i) else { break };
            let room = Rectangle::new(tile.loc, (tw, th).into());
            let alpha = if rows.get(i).is_some_and(|r| r.2) { 0.5 } else { 1.0 };
            if let Some(e) = self.thumb_element(renderer, w, px(room), alpha) {
                elements.push(SceneElement::Thumb(e));
            }
        }
        let whole = px(panel);
        let src = Rectangle::<f64, Logical>::from_size((whole.size.w as f64, whole.size.h as f64).into());
        if let Some((_, buffer)) = &self.input.switching.as_ref().unwrap().picture
            && let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                whole.loc.to_f64(),
                buffer,
                None,
                Some(src),
                Some(panel.size),
                Kind::Unspecified,
            )
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
                cap: false,
            };
            let mut slot = self.input.switching.as_mut().unwrap().backdrop.take();
            if let Some(e) = crate::backdrop::element(renderer, &mut slot, whole, program, &look) {
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
                let mut out =
                    vec![format!("at {} of {}, shown {}, panel {:?}", sw.at, sw.list.len(), sw.shown(), sw.panel())];
                for (i, r) in sw.rows.iter().enumerate() {
                    out.push(format!(
                        "{}{}: {}{}",
                        if i == sw.at { "> " } else { "  " },
                        i,
                        r.title,
                        if r.minimized { " (minimized)" } else { "" }
                    ));
                }
                out.join("\n")
            }
        }
    }
}
