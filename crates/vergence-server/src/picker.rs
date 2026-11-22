// The dock's window picker (overview milestone 1, step 3;
// docs/overview-milestone-1.md): the live tiles as the dock's window picker,
// shown when the pointer rests on an icon, as Windows does.
//
// The pointer resting on a running application's icon opens a frosted strip
// beside the icon, on the side the windows are: a tile for each of the
// application's windows, its live picture (thumbs.rs) over its title, a
// minimized one faint. A click on a tile brings that window forward (back
// from minimized first); the × at the end of the title row of the tile under
// the pointer closes it. Moving to another running app's icon while it is
// open shows that app's at once. It goes when the pointer has left both the
// icon and the strip for a moment, at a press anywhere else, and when the
// app has no windows left.
//
// Drawn as the tiles' cards are (tiles.rs): the strip's chrome painted once
// on the CPU and kept while nothing in it changes, the frost behind it, the
// pictures in front, drawn again 15 times a second while it is open.

use std::hash::{Hash, Hasher};
use std::time::Duration;

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
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::{Logical, Physical, Point, Rectangle, Transform},
};
use vergence_scene::{Color, Point as ScenePoint, Rect as SceneRect, Scene};
use vergence_shell::{Edge, running};

use crate::{Server, decor::SceneElement, win::Win};

/// Each tile's picture room and its title row, the strip's padding and the
/// space between tiles, in points.
const THUMB_W: i32 = 200;
const THUMB_H: i32 = 124;
const TITLE_H: i32 = 24;
const PAD: i32 = 10;
const GAP: i32 = 8;
/// From the dock to the strip.
const OFF: i32 = 8;
/// How long the pointer may be off the icon and the strip before it goes.
const AWAY: Duration = Duration::from_millis(400);

/// The open picker.
pub struct Picker {
    /// The dock item's key, and its place on the dock.
    pub key: String,
    pub place: usize,
    /// Its windows, in the dock's order.
    pub windows: Vec<Win>,
    /// Where the strip is, each tile's picture and its × (desktop points).
    pub rect: Rectangle<i32, Logical>,
    pub thumbs: Vec<Rectangle<i32, Logical>>,
    pub closes: Vec<Rectangle<i32, Logical>>,
    /// The tile under the pointer, and whether it is on its ×.
    pub hover: Option<usize>,
    pub on_close: bool,
    /// The chrome's picture and what it was drawn from; the frost's slot.
    picture: Option<(u64, MemoryRenderBuffer)>,
    frost: crate::backdrop::Slot,
}

impl Server {
    /// Open the picker for the dock item at `place`, if it has windows.
    pub fn picker_open(&mut self, place: usize) {
        let items = self.shelf_items();
        let Some((item, windows)) = items.into_iter().nth(place) else { return };
        if windows.is_empty() {
            return;
        }
        if self.picker.as_ref().is_some_and(|p| p.key == item.key && p.windows == windows) {
            return;
        }
        let first = self.picker.is_none();
        tracing::info!("picker: {} ({} windows)", item.key, windows.len());
        self.picker = Some(Picker {
            key: item.key,
            place,
            windows,
            rect: Rectangle::default(),
            thumbs: Vec::new(),
            closes: Vec::new(),
            hover: None,
            on_close: false,
            picture: None,
            frost: None,
        });
        self.shelf.named = false;
        self.redraw_all();
        if first {
            // Live: drawn again while it is open.
            let _ = self.loop_handle.insert_source(Timer::from_duration(crate::thumbs::EVERY), |_, _, st| {
                if st.picker.is_none() {
                    return TimeoutAction::Drop;
                }
                st.redraw_all();
                TimeoutAction::ToDuration(crate::thumbs::EVERY)
            });
        }
    }

    /// For `vctl picker`: closed, or the app and each tile's picture (x y w h).
    pub fn picker_report(&self) -> String {
        let Some(p) = self.picker.as_ref() else { return "closed".into() };
        let mut out = vec![format!(
            "open {} at {} {} {} {}, {} windows",
            p.key,
            p.rect.loc.x,
            p.rect.loc.y,
            p.rect.size.w,
            p.rect.size.h,
            p.windows.len()
        )];
        for (i, t) in p.thumbs.iter().enumerate() {
            out.push(format!("{i} {} {} {} {}", t.loc.x, t.loc.y, t.size.w, t.size.h));
        }
        out.join("\n")
    }

    pub fn picker_close(&mut self) {
        if self.picker.take().is_some() {
            self.redraw_all();
        }
    }

    /// Whether the open picker is at a point.
    pub fn picker_at(&self, at: Point<f64, Logical>) -> bool {
        self.picker.as_ref().is_some_and(|p| p.rect.to_f64().contains(at))
    }

    /// The dock icon slot the picker belongs to.
    fn picker_slot(&self) -> Option<Rectangle<i32, Logical>> {
        let p = self.picker.as_ref()?;
        let dock = self.shelf.dock?;
        let upright = self.shelf.config.placement().upright();
        running::slot(crate::shelf::rect(dock), p.place, upright)
            .map(|r| Rectangle::new((r.x, r.y).into(), (r.w, r.h).into()))
    }

    /// The pointer moved: the tile under it; off the icon and the strip a
    /// moment, the picker goes.
    pub fn picker_hover(&mut self, at: Point<f64, Logical>) {
        let Some(p) = self.picker.as_ref() else { return };
        let hover = p
            .thumbs
            .iter()
            .position(|r| r.to_f64().contains(at))
            .or_else(|| p.closes.iter().position(|r| r.to_f64().contains(at)));
        let on_close = p.closes.iter().any(|r| r.to_f64().contains(at));
        if (hover, on_close) != (p.hover, p.on_close) {
            if let Some(p) = self.picker.as_mut() {
                (p.hover, p.on_close) = (hover, on_close);
            }
            self.redraw_all();
        }
        let on_icon = self.picker_slot().is_some_and(|s| s.to_f64().contains(at));
        if !self.picker_at(at) && !on_icon {
            let _ = self.loop_handle.insert_source(Timer::from_duration(AWAY), |_, _, st| {
                let at = st.pointer_location();
                let on_icon = st.picker_slot().is_some_and(|s| s.to_f64().contains(at));
                if !st.picker_at(at) && !on_icon {
                    st.picker_close();
                }
                TimeoutAction::Drop
            });
        }
    }

    /// A press on the picker: on a tile's ×, that window closes; on a tile,
    /// it comes forward.
    pub fn picker_press(&mut self, at: Point<f64, Logical>) {
        let Some(p) = self.picker.as_ref() else { return };
        if let Some(i) = p.closes.iter().position(|r| r.to_f64().contains(at)) {
            let Some(w) = p.windows.get(i).cloned() else { return };
            self.close_window(&w);
            return;
        }
        let Some(i) = p.thumbs.iter().position(|r| r.to_f64().contains(at)) else { return };
        let Some(w) = p.windows.get(i).cloned() else { return };
        self.picker_close();
        if !smithay::utils::IsAlive::alive(&w) {
            return;
        }
        tracing::info!("picker: chose {:?}", self.window_title(&w));
        if self.is_minimized(&w) {
            self.unminimize(&w);
        } else {
            self.focus_window(&w, smithay::utils::SERIAL_COUNTER.next_serial());
        }
    }

    /// The picker's elements on `out`: the pictures, the chrome, the frost.
    pub fn picker_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        out: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Vec<SceneElement> {
        let Some(key) = self.picker.as_ref().map(|p| p.key.clone()) else { return Vec::new() };
        // The app's windows now (one may have closed or opened).
        let items = self.shelf_items();
        let Some(place) = items.iter().position(|(i, _)| i.key == key) else {
            self.picker = None;
            return Vec::new();
        };
        let windows = items[place].1.clone();
        if windows.is_empty() {
            self.picker = None;
            return Vec::new();
        }
        let Some(dock) = self.shelf.dock else { return Vec::new() };
        let screen = self.shelf_screen().unwrap_or(out);
        if !screen.overlaps(out) {
            return Vec::new();
        }
        let upright = self.shelf.config.placement().upright();
        let Some(slot) = running::slot(crate::shelf::rect(dock), place, upright) else { return Vec::new() };
        let side = self.shelf_side(dock);
        // The tiles' size: as asked, smaller when they would not fit the screen.
        let n = windows.len() as i32;
        let along = |tw: i32| 2 * PAD + n * tw + (n - 1) * GAP;
        let room = if matches!(side, Edge::Left | Edge::Right) { screen.size.h - 16 } else { screen.size.w - 16 };
        let across_tiles = matches!(side, Edge::Top | Edge::Bottom);
        let (tw, th) = {
            let need = if across_tiles { along(THUMB_W) } else { 2 * PAD + n * (THUMB_H + TITLE_H) + (n - 1) * GAP };
            if need <= room {
                (THUMB_W, THUMB_H)
            } else {
                let k = room as f64 / need as f64;
                (((THUMB_W as f64 * k) as i32).max(60), ((THUMB_H as f64 * k) as i32).max(38))
            }
        };
        // Tiles in a row for a dock along the bottom or top, in a column beside a side dock.
        let (w, h) = if across_tiles {
            (along(tw), 2 * PAD + th + TITLE_H)
        } else {
            (2 * PAD + tw, 2 * PAD + n * (th + TITLE_H) + (n - 1) * GAP)
        };
        let middle = (slot.y + slot.h / 2 - h / 2, slot.x + slot.w / 2 - w / 2);
        let (x, y) = match side {
            Edge::Left => (dock.loc.x - OFF - w, middle.0),
            Edge::Right => (dock.loc.x + dock.size.w + OFF, middle.0),
            Edge::Top => (middle.1, dock.loc.y - OFF - h),
            Edge::Bottom => (middle.1, dock.loc.y + dock.size.h + OFF),
        };
        let (x, y) = (
            x.min(screen.loc.x + screen.size.w - w).max(screen.loc.x),
            y.min(screen.loc.y + screen.size.h - h).max(screen.loc.y),
        );
        let area: Rectangle<i32, Logical> = Rectangle::new((x, y).into(), (w, h).into());
        let tile_at = |i: i32| -> Point<i32, Logical> {
            if across_tiles {
                (x + PAD + i * (tw + GAP), y + PAD).into()
            } else {
                (x + PAD, y + PAD + i * (th + TITLE_H + GAP)).into()
            }
        };
        let thumbs: Vec<Rectangle<i32, Logical>> =
            (0..n).map(|i| Rectangle::new(tile_at(i), (tw, th).into())).collect();
        let closes: Vec<Rectangle<i32, Logical>> = (0..n)
            .map(|i| {
                let t = tile_at(i);
                Rectangle::new((t.x + tw - TITLE_H, t.y + th).into(), (TITLE_H, TITLE_H).into())
            })
            .collect();
        let titles: Vec<String> = windows
            .iter()
            .map(|w| self.window_title(w).chars().filter(|c| !c.is_control()).take(200).collect())
            .collect();
        let minimized: Vec<bool> = windows.iter().map(|w| self.is_minimized(w)).collect();
        let light = self.light_style();
        let frost = self.frost_program(renderer);
        let accent = self.accent();
        let p = self.picker.as_mut().expect("open");
        p.place = place;
        p.windows = windows.clone();
        p.rect = area;
        p.thumbs = thumbs.clone();
        p.closes = closes;
        let (hover, on_close) = (p.hover.filter(|i| *i < windows.len()), p.on_close);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (
            &titles,
            &minimized,
            hover,
            on_close,
            w,
            h,
            tw,
            th,
            across_tiles,
            frost.is_some(),
            light,
            (accent.r, accent.g, accent.b),
        )
            .hash(&mut hasher);
        let key = hasher.finish() ^ scale.to_bits();
        if p.picture.as_ref().map(|(k, _)| *k) != Some(key) {
            let s = vergence_look::shelf_look(light);
            let ink = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            let Some(face) = fonts.face("sans-serif", 400, false) else { return Vec::new() };
            let (wf, hf) = (w as f32, h as f32);
            let mut scene = Scene::new(wf, hf);
            if frost.is_none() {
                scene.fill_rounded(SceneRect::new(0.0, 0.0, wf, hf), vergence_look::RADIUS, ink(s.menu));
            }
            scene.stroke(SceneRect::new(0.0, 0.0, wf, hf), vergence_look::RADIUS, vergence_look::EDGE, ink(s.edge));
            let size = 12.0;
            let m = fonts.metrics(face, size);
            for i in 0..n as usize {
                let t = tile_at(i as i32);
                let (tx, ty) = ((t.x - x) as f32, (t.y - y) as f32);
                // The tile under the pointer: shaded behind its picture and title.
                if hover == Some(i) {
                    scene.fill_rounded(
                        SceneRect::new(tx - 4.0, ty - 4.0, tw as f32 + 8.0, (th + TITLE_H) as f32 + 4.0),
                        vergence_look::RADIUS,
                        ink(s.hover),
                    );
                }
                // The picture's room, a faint ground where the picture does not fill it.
                scene.fill_rounded(
                    SceneRect::new(tx, ty, tw as f32, th as f32),
                    4.0,
                    Color { a: 26, ..ink(s.menu_ink) },
                );
                let close_w = if hover == Some(i) { TITLE_H as f32 } else { 0.0 };
                let text_ink = if minimized[i] { ink(s.menu_ink_off) } else { ink(s.menu_ink) };
                scene.push_clip(SceneRect::new(tx, ty + th as f32, tw as f32 - close_w, TITLE_H as f32), 0.0);
                let baseline = (ty + th as f32 + (TITLE_H as f32 - (m.ascent + m.descent)) / 2.0 + m.ascent).round();
                let title = if titles[i].is_empty() { "Untitled".to_string() } else { titles[i].clone() };
                vergence_render::text(
                    &mut scene,
                    fonts,
                    face,
                    size,
                    text_ink,
                    ScenePoint::new(tx + 2.0, baseline),
                    &title,
                );
                scene.pop_clip();
                if hover == Some(i) {
                    // The ×, shaded while the pointer is on it.
                    let (cx, cy) = (tx + tw as f32 - TITLE_H as f32, ty + th as f32);
                    if on_close {
                        scene.fill_rounded(
                            SceneRect::new(cx + 3.0, cy + 3.0, TITLE_H as f32 - 6.0, TITLE_H as f32 - 6.0),
                            4.0,
                            ink(s.hover),
                        );
                    }
                    let (mx, my, r) = (cx + TITLE_H as f32 / 2.0, cy + TITLE_H as f32 / 2.0, 4.0);
                    scene.line(ScenePoint::new(mx - r, my - r), ScenePoint::new(mx + r, my + r), 1.4, ink(s.menu_ink));
                    scene.line(ScenePoint::new(mx + r, my - r), ScenePoint::new(mx - r, my + r), 1.4, ink(s.menu_ink));
                }
            }
            let Ok(pic) = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0))
            else {
                return Vec::new();
            };
            let buffer = MemoryRenderBuffer::from_slice(
                &pic.data,
                Fourcc::Abgr8888,
                (pic.width as i32, pic.height as i32),
                1,
                Transform::Normal,
                None,
            );
            if let Some(p) = self.picker.as_mut() {
                p.picture = Some((key, buffer));
            }
        }
        let px = |r: Rectangle<i32, Logical>| -> Rectangle<i32, Physical> {
            let a = (r.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            let b = (r.loc + r.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            Rectangle::new(a, (b.x - a.x, b.y - a.y).into())
        };
        let mut elements: Vec<SceneElement> = Vec::new();
        // The pictures in front.
        for (i, w) in windows.iter().enumerate() {
            let alpha = if minimized[i] { 0.55 } else { 1.0 };
            if let Some(e) = self.thumb_element(renderer, w, px(thumbs[i]), alpha) {
                elements.push(SceneElement::Thumb(e));
            }
        }
        // Then the chrome.
        let whole = px(area);
        if let Some((_, buffer)) = self.picker.as_ref().and_then(|p| p.picture.as_ref()) {
            let src = Rectangle::<f64, Logical>::from_size((whole.size.w as f64, whole.size.h as f64).into());
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                whole.loc.to_f64(),
                buffer,
                None,
                Some(src),
                Some(area.size),
                Kind::Unspecified,
            ) {
                elements.push(SceneElement::Memory(e));
            }
        }
        // Behind it all, the frost.
        if let Some(program) = frost {
            let look = self.panel_frost(scale);
            if let Some(p) = self.picker.as_mut() {
                let mut slot = p.frost.take();
                if let Some(e) = crate::backdrop::element(renderer, &mut slot, whole, program, &look) {
                    elements.push(SceneElement::Backdrop(e));
                }
                p.frost = slot;
            }
        }
        elements
    }
}
