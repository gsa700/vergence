// The overview, Super+Tab (overview milestone 1, step 5;
// docs/overview-milestone-1.md). Dave, 2026-11-18: "Super+Tab Mission Control
// style"; minimized windows in a faint row along the bottom (his 1a).
//
// Every window on each screen at once: the screen dimmed, each window itself
// (its bar, tabs, border and content: the scene's own elements, scaled;
// Dave, 2026-11-20: "Could Super+Tab use the actual windows rather than
// images of them?") where vergence-shell's overview.rs puts it (scaled
// alike, none overlapping, roughly where it was), its title under it; the
// minimized ones in a row of their own along the bottom, faint. The dock
// stays where it is (the arrangement keeps to the work area).
//
// Click or Enter brings a window forward (back from minimized first); the
// arrows move the choice; typing narrows the windows by title or app name
// (Backspace takes a letter back); Escape, Super+Tab again, or a click on
// the empty ground closes it. While it is open it has every key (the seat's
// Menu::Overview) and every press (Part::Overview).

use std::hash::{Hash, Hasher};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{Id, Kind, memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement}, solid::SolidColorRenderElement},
            gles::GlesRenderer,
            utils::CommitCounter,
        },
    },
    input::keyboard::keysyms as key,
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::{Logical, Physical, Point, Rectangle, Transform},
};
use vergence_scene::{Color, Point as ScenePoint, Rect as SceneRect, Scene};
use vergence_shell::{Rect, overview};

use crate::{Server, decor::SceneElement, win::Win};

smithay::backend::renderer::element::render_elements! {
    /// What a window is drawn with (decor.rs's scene), to be scaled.
    pub WinParts<=GlesRenderer>;
    Surface = smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<GlesRenderer>,
    Solid = SolidColorRenderElement,
    Memory = MemoryRenderBufferRenderElement<GlesRenderer>,
    Texture = smithay::backend::renderer::element::texture::TextureRenderElement<smithay::backend::renderer::gles::GlesTexture>,
    Shaded = crate::backdrop::Shaded,
    Backdrop = crate::backdrop::Backdrop,
}

/// A window's element scaled into its place in the overview.
pub type Placed = smithay::backend::renderer::element::utils::RelocateRenderElement<smithay::backend::renderer::element::utils::RescaleRenderElement<WinParts>>;

/// Between windows, and from them to the edges: room for a title under each.
const GAP: i32 = 44;
/// The strip at the top for what is typed, and a title's height.
const FIELD_H: i32 = 56;
const LABEL_H: i32 = 24;

/// One window in the overview.
pub struct Entry {
    pub win: Win,
    pub title: String,
    /// Its application's name, for typing.
    pub app: String,
    pub minimized: bool,
    /// Where it is shown (desktop points); empty while narrowed away.
    pub rect: Rectangle<i32, Logical>,
}

/// The open overview.
pub struct Overview {
    pub entries: Vec<Entry>,
    /// The window chosen with the keys, and the one under the pointer.
    pub at: Option<usize>,
    pub hover: Option<usize>,
    /// What has been typed.
    pub filter: String,
    dim: Id,
    outline: [Id; 4],
    commit: CommitCounter,
    /// Each title's picture, by the entry's place and what it was drawn from.
    labels: Vec<Option<(u64, MemoryRenderBuffer, i32)>>,
    field: Option<(u64, MemoryRenderBuffer, i32)>,
}

impl Overview {
    /// The entries shown (those the typing has not narrowed away).
    fn shown(&self, i: usize) -> bool {
        self.entries.get(i).is_some_and(|e| e.rect.size.w > 0)
    }
}

fn r(x: Rectangle<i32, Logical>) -> Rect {
    Rect::new(x.loc.x, x.loc.y, x.size.w, x.size.h)
}

fn rect(x: Rect) -> Rectangle<i32, Logical> {
    Rectangle::new((x.x, x.y).into(), (x.w, x.h).into())
}

impl Server {
    pub fn overview_is_open(&self) -> bool {
        self.overview.is_some()
    }

    /// Super+Tab: open, or close if open.
    pub fn overview_toggle(&mut self) {
        if self.overview.is_some() {
            return self.overview_close();
        }
        self.app_menu_close();
        self.close_menu();
        self.launcher_close();
        self.picker_close();
        let items = self.shelf_items();
        let focused = self.keyboard_window();
        let mut entries: Vec<Entry> = Vec::new();
        // The windows in the order the keyboard last had them, as Alt+Tab.
        let all: Vec<(Win, String)> = items.iter().flat_map(|(item, ws)| ws.iter().map(move |w| (w.clone(), item.name.clone()))).collect();
        let mut order: Vec<Win> = self.input.seat.order_of_use().iter().filter_map(|&id| self.input.window(id)).filter(|w| all.iter().any(|(a, _)| a == w)).collect();
        for (w, _) in &all {
            if !order.contains(w) {
                order.push(w.clone());
            }
        }
        for w in order {
            let app = all.iter().find(|(a, _)| *a == w).map(|(_, n)| n.clone()).unwrap_or_default();
            let title: String = self.window_title(&w).chars().filter(|c| !c.is_control()).take(200).collect();
            let minimized = self.is_minimized(&w);
            entries.push(Entry { win: w, title, app, minimized, rect: Rectangle::default() });
        }
        if entries.is_empty() {
            return;
        }
        tracing::info!("overview: open ({} windows)", entries.len());
        let at = focused.and_then(|f| entries.iter().position(|e| e.win == f)).or(Some(0));
        let n = entries.len();
        self.overview = Some(Overview { entries, at, hover: None, filter: String::new(), dim: Id::new(), outline: [Id::new(), Id::new(), Id::new(), Id::new()], commit: CommitCounter::default(), labels: (0..n).map(|_| None).collect(), field: None });
        self.overview_arrange();
        self.redraw_all();
        // Live: drawn again while it is open.
        let _ = self.loop_handle.insert_source(Timer::from_duration(crate::thumbs::EVERY), |_, _, st| {
            if st.overview.is_none() {
                return TimeoutAction::Drop;
            }
            st.redraw_all();
            TimeoutAction::ToDuration(crate::thumbs::EVERY)
        });
    }

    /// For `vctl overview`: closed, or each window's place (x y w h), title
    /// and whether it is minimized, and which is chosen.
    pub fn overview_report(&self) -> String {
        let Some(ov) = self.overview.as_ref() else { return "closed".into() };
        let mut out = vec![format!("open, {} windows, filter {:?}, chosen {:?}", ov.entries.len(), ov.filter, ov.at)];
        for (i, e) in ov.entries.iter().enumerate() {
            out.push(format!("{i} {} {} {} {} {:?}{}", e.rect.loc.x, e.rect.loc.y, e.rect.size.w, e.rect.size.h, e.title, if e.minimized { " minimized" } else { "" }));
        }
        out.join("\n")
    }

    pub fn overview_close(&mut self) {
        if self.overview.take().is_some() {
            tracing::info!("overview: closed");
            self.redraw_all();
        }
    }

    /// Where each window shows, screen by screen: the open ones in the work
    /// area under the typing strip, the minimized ones in a row along the
    /// bottom of it (a fifth of its height) when there are any.
    fn overview_arrange(&mut self) {
        let screens: Vec<Rectangle<i32, Logical>> = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).collect();
        let Some(ov) = self.overview.as_ref() else { return };
        let words = ov.filter.to_lowercase();
        // Where each window is (a minimized one: where it goes back to).
        let places: Vec<Option<Rectangle<i32, Logical>>> = ov
            .entries
            .iter()
            .map(|e| match self.shelf.minimized.iter().find(|(m, _)| *m == e.win) {
                Some((_, at)) => Some(Rectangle::new(*at, e.win.geometry().size)),
                None => self.space.element_geometry(&e.win),
            })
            .collect();
        let wanted: Vec<bool> = ov.entries.iter().map(|e| words.is_empty() || e.title.to_lowercase().contains(&words) || e.app.to_lowercase().contains(&words)).collect();
        let minimized: Vec<bool> = ov.entries.iter().map(|e| e.minimized).collect();
        let mut rects = vec![Rectangle::<i32, Logical>::default(); ov.entries.len()];
        for screen in &screens {
            let on: Vec<usize> = (0..places.len())
                .filter(|&i| wanted[i])
                .filter(|&i| places[i].is_some_and(|p| screen.contains((p.loc.x + p.size.w / 2, p.loc.y + p.size.h / 2))) || (places[i].is_none() && Some(*screen) == screens.first().copied()))
                .collect();
            let work = self.work_area(*screen);
            let area = Rectangle::<i32, Logical>::new((work.loc.x, work.loc.y + FIELD_H).into(), (work.size.w, work.size.h - FIELD_H).into());
            let (open, gone): (Vec<usize>, Vec<usize>) = on.iter().partition(|&&i| !minimized[i]);
            let strip_h = if gone.is_empty() { 0 } else { area.size.h / 5 };
            let main = Rect::new(area.loc.x, area.loc.y, area.size.w, area.size.h - strip_h);
            let size = |i: usize| places[i].map_or(Rect::new(0, 0, 800, 600), r);
            for (i, p) in open.iter().zip(overview::arrange(&open.iter().map(|&i| size(i)).collect::<Vec<_>>(), main, GAP)) {
                rects[*i] = rect(p);
            }
            if strip_h > 0 {
                let strip = Rect::new(area.loc.x, area.loc.y + area.size.h - strip_h, area.size.w, strip_h);
                // In a row by their order, whatever their places.
                let sizes: Vec<Rect> = gone.iter().enumerate().map(|(k, &i)| { let s = size(i); Rect::new(k as i32 * 10_000, 0, s.w, s.h) }).collect();
                for (i, p) in gone.iter().zip(overview::arrange(&sizes, strip, GAP / 2)) {
                    rects[*i] = rect(p);
                }
            }
        }
        if let Some(ov) = self.overview.as_mut() {
            for (e, rc) in ov.entries.iter_mut().zip(rects) {
                e.rect = rc;
            }
            // The choice stays on one that is shown.
            if !ov.at.is_some_and(|i| ov.shown(i)) {
                ov.at = (0..ov.entries.len()).find(|&i| ov.shown(i));
            }
            ov.commit.increment();
        }
    }

    /// The entry shown at a point.
    fn overview_entry_at(&self, at: Point<f64, Logical>) -> Option<usize> {
        let ov = self.overview.as_ref()?;
        ov.entries.iter().position(|e| e.rect.size.w > 0 && e.rect.to_f64().contains(at))
    }

    pub fn overview_hover(&mut self, at: Point<f64, Logical>) {
        if self.overview.is_none() {
            return;
        }
        let hover = self.overview_entry_at(at);
        if let Some(ov) = self.overview.as_mut()
            && ov.hover != hover
        {
            ov.hover = hover;
            ov.commit.increment();
            self.redraw_all();
        }
    }

    /// A press: on a window, it comes forward; on the ground, the overview closes.
    pub fn overview_press(&mut self, at: Point<f64, Logical>) {
        match self.overview_entry_at(at) {
            Some(i) => self.overview_choose(i),
            None => self.overview_close(),
        }
    }

    fn overview_choose(&mut self, i: usize) {
        let Some(w) = self.overview.as_ref().and_then(|ov| ov.entries.get(i)).map(|e| e.win.clone()) else { return };
        self.overview_close();
        if !smithay::utils::IsAlive::alive(&w) {
            return;
        }
        tracing::info!("overview: chose {:?}", self.window_title(&w));
        if self.is_minimized(&w) {
            self.unminimize(&w);
        } else {
            self.focus_window(&w, smithay::utils::SERIAL_COUNTER.next_serial());
        }
    }

    /// A key while it is open.
    pub fn overview_key(&mut self, sym: u32, typed: Option<char>, mods: vergence_seat::keyboard::Mods) {
        let Some(ov) = self.overview.as_ref() else { return };
        let arrow = match sym {
            key::KEY_Left => Some((-1, 0)),
            key::KEY_Right => Some((1, 0)),
            key::KEY_Up => Some((0, -1)),
            key::KEY_Down => Some((0, 1)),
            _ => None,
        };
        if let Some((dx, dy)) = arrow {
            let shown: Vec<usize> = (0..ov.entries.len()).filter(|&i| ov.shown(i)).collect();
            let places: Vec<Rect> = shown.iter().map(|&i| r(ov.entries[i].rect)).collect();
            let from = ov.at.and_then(|a| shown.iter().position(|&i| i == a)).unwrap_or(0);
            let to = shown.get(overview::step(&places, from, dx, dy)).copied();
            if let Some(ov) = self.overview.as_mut() {
                ov.at = to;
                ov.commit.increment();
            }
            return self.redraw_all();
        }
        match sym {
            key::KEY_Escape => self.overview_close(),
            // Super+Tab again (or Tab alone) closes it.
            key::KEY_Tab | key::KEY_ISO_Left_Tab if mods.logo => self.overview_close(),
            key::KEY_Return | key::KEY_KP_Enter => {
                if let Some(i) = ov.at.filter(|&i| ov.shown(i)) {
                    self.overview_choose(i);
                }
            }
            key::KEY_BackSpace => {
                if let Some(ov) = self.overview.as_mut() {
                    if mods.ctrl { ov.filter.clear() } else { ov.filter.pop(); }
                }
                self.overview_arrange();
                self.redraw_all();
            }
            _ => {
                if let Some(c) = typed.filter(|c| !c.is_control() && !mods.ctrl && !mods.logo)
                    && let Some(ov) = self.overview.as_mut()
                    && ov.filter.chars().count() < 80
                {
                    ov.filter.push(c);
                    self.overview_arrange();
                    self.redraw_all();
                }
            }
        }
    }

    /// The overview's elements on `out`: what is typed, the titles, the
    /// choice's outline, the pictures, the dimmed screen behind them.
    pub fn overview_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        let Some(ov) = self.overview.as_ref() else { return Vec::new() };
        // A window gone meanwhile: arranged again without it.
        if ov.entries.iter().any(|e| !smithay::utils::IsAlive::alive(&e.win)) {
            if let Some(ov) = self.overview.as_mut() {
                ov.entries.retain(|e| smithay::utils::IsAlive::alive(&e.win));
                ov.labels = (0..ov.entries.len()).map(|_| None).collect();
                if ov.entries.is_empty() {
                    self.overview = None;
                    return Vec::new();
                }
            }
            self.overview_arrange();
        }
        let light = self.light_style();
        let accent = self.accent();
        let s = vergence_look::shelf_look(light);
        let px = |x: Rectangle<i32, Logical>| -> Rectangle<i32, Physical> {
            let a = (x.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            let b = (x.loc + x.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
            Rectangle::new(a, (b.x - a.x, b.y - a.y).into())
        };
        let mut elements: Vec<SceneElement> = Vec::new();
        let ink = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
        // What is typed, at the top of each screen's work area, centered.
        {
            let ov = self.overview.as_ref().unwrap();
            let text = if ov.filter.is_empty() { "Type to find a window".to_string() } else { ov.filter.clone() };
            let faint = ov.filter.is_empty();
            let key = { let mut h = std::collections::hash_map::DefaultHasher::new(); (&text, faint, light, scale.to_bits()).hash(&mut h); h.finish() };
            if ov.field.as_ref().map(|(k, ..)| *k) != Some(key) {
                let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
                if let Some(face) = fonts.face("sans-serif", 400, false) {
                    let size = 15.0;
                    let m = fonts.metrics(face, size);
                    let tw = fonts.layout(face, size, &text).1;
                    let (w, h) = ((tw + 36.0).clamp(240.0, 640.0).ceil(), 34.0);
                    let mut scene = Scene::new(w, h);
                    scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), h / 2.0, ink(s.menu));
                    scene.stroke(SceneRect::new(0.0, 0.0, w, h), h / 2.0, vergence_look::EDGE, ink(s.edge));
                    scene.push_clip(SceneRect::new(14.0, 0.0, w - 28.0, h), 0.0);
                    let x = ((w - tw) / 2.0).max(18.0);
                    vergence_render::text(&mut scene, fonts, face, size, if faint { ink(s.menu_ink_off) } else { ink(s.menu_ink) }, ScenePoint::new(x, ((h - (m.ascent + m.descent)) / 2.0 + m.ascent).round()), &text);
                    scene.pop_clip();
                    if let Ok(p) = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) {
                        let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
                        self.overview.as_mut().unwrap().field = Some((key, buffer, w as i32));
                    }
                }
            }
            let work = self.work_area(out);
            if let Some((_, buffer, w)) = self.overview.as_ref().unwrap().field.as_ref() {
                let area = Rectangle::<i32, Logical>::new((work.loc.x + (work.size.w - w) / 2, work.loc.y + (FIELD_H - 34) / 2 + 6).into(), (*w, 34).into());
                let p = px(area);
                let src = Rectangle::<f64, Logical>::from_size((p.size.w as f64, p.size.h as f64).into());
                if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(renderer, p.loc.to_f64(), buffer, None, Some(src), Some(area.size), Kind::Unspecified) {
                    elements.push(SceneElement::Memory(e));
                }
            }
        }
        // The titles, under each window shown on this screen.
        let n = self.overview.as_ref().unwrap().entries.len();
        for i in 0..n {
            let (rc, title, minimized, chosen) = {
                let ov = self.overview.as_ref().unwrap();
                let e = &ov.entries[i];
                (e.rect, e.title.clone(), e.minimized, ov.at == Some(i) || ov.hover == Some(i))
            };
            if rc.size.w <= 0 || !rc.overlaps(out) {
                continue;
            }
            let lw = (rc.size.w + GAP - 8).max(60);
            let key = { let mut h = std::collections::hash_map::DefaultHasher::new(); (&title, minimized, chosen, lw, light, scale.to_bits()).hash(&mut h); h.finish() };
            if self.overview.as_ref().unwrap().labels.get(i).and_then(|l| l.as_ref()).map(|(k, ..)| *k) != Some(key) {
                let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
                if let Some(face) = fonts.face("sans-serif", if chosen { 600 } else { 400 }, false).or_else(|| fonts.face("sans-serif", 400, false)) {
                    let size = 13.0;
                    let m = fonts.metrics(face, size);
                    let mut text = if title.is_empty() { "Untitled".to_string() } else { title.clone() };
                    let room = lw as f32 - 16.0;
                    if fonts.layout(face, size, &text).1 > room {
                        while !text.is_empty() && fonts.layout(face, size, &format!("{text}…")).1 > room {
                            text.pop();
                        }
                        text.push('…');
                    }
                    let tw = fonts.layout(face, size, &text).1;
                    let (w, h) = ((tw + 16.0).ceil(), LABEL_H as f32);
                    let mut scene = Scene::new(w, h);
                    scene.fill_rounded(SceneRect::new(0.0, 0.0, w, h), h / 2.0, if chosen { accent } else { Color { a: 200, ..ink(s.menu) } });
                    let color = if chosen { Color::rgb(0xff, 0xff, 0xff) } else if minimized { ink(s.menu_ink_off) } else { ink(s.menu_ink) };
                    vergence_render::text(&mut scene, fonts, face, size, color, ScenePoint::new(8.0, ((h - (m.ascent + m.descent)) / 2.0 + m.ascent).round()), &text);
                    if let Ok(p) = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) {
                        let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
                        if let Some(slot) = self.overview.as_mut().unwrap().labels.get_mut(i) {
                            *slot = Some((key, buffer, w as i32));
                        }
                    }
                }
            }
            if let Some((_, buffer, w)) = self.overview.as_ref().unwrap().labels.get(i).and_then(|l| l.as_ref()) {
                let area = Rectangle::<i32, Logical>::new((rc.loc.x + (rc.size.w - w) / 2, rc.loc.y + rc.size.h + 8).into(), (*w, LABEL_H).into());
                let p = px(area);
                let src = Rectangle::<f64, Logical>::from_size((p.size.w as f64, p.size.h as f64).into());
                if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(renderer, p.loc.to_f64(), buffer, None, Some(src), Some(area.size), Kind::Unspecified) {
                    elements.push(SceneElement::Memory(e));
                }
            }
        }
        // The chosen window's outline, in the accent.
        let (chosen, outline, commit) = {
            let ov = self.overview.as_ref().unwrap();
            (ov.hover.or(ov.at).and_then(|i| ov.entries.get(i)).map(|e| e.rect).filter(|rc| rc.size.w > 0 && rc.overlaps(out)), ov.outline.clone(), ov.commit)
        };
        if let Some(rc) = chosen {
            let t = 3;
            let a = [accent.r as f32 / 255.0, accent.g as f32 / 255.0, accent.b as f32 / 255.0, 1.0];
            let bars = [
                Rectangle::new((rc.loc.x - t - 2, rc.loc.y - t - 2).into(), (rc.size.w + 2 * t + 4, t).into()),
                Rectangle::new((rc.loc.x - t - 2, rc.loc.y + rc.size.h + 2).into(), (rc.size.w + 2 * t + 4, t).into()),
                Rectangle::new((rc.loc.x - t - 2, rc.loc.y - 2).into(), (t, rc.size.h + 4).into()),
                Rectangle::new((rc.loc.x + rc.size.w + 2, rc.loc.y - 2).into(), (t, rc.size.h + 4).into()),
            ];
            for (id, bar) in outline.iter().zip(bars) {
                elements.push(SceneElement::Solid(SolidColorRenderElement::new(id.clone(), px(bar), commit, a, Kind::Unspecified)));
            }
        }
        // The pictures.
        for i in 0..n {
            let (w, rc, minimized) = {
                let e = &self.overview.as_ref().unwrap().entries[i];
                (e.win.clone(), e.rect, e.minimized)
            };
            if rc.size.w <= 0 || !rc.overlaps(out) {
                continue;
            }
            // A window on this screen is drawn itself (overview_place); a
            // minimized one, or one whose place is on another screen, by its picture.
            let live = !minimized && self.space.element_bbox(&w).is_some_and(|b| b.overlaps(out));
            if live {
                continue;
            }
            if let Some(e) = self.thumb_element(renderer, &w, px(rc), if minimized { 0.55 } else { 1.0 }) {
                elements.push(SceneElement::Thumb(e));
            }
        }
        elements
    }

    /// The screen dimmed behind the overview's windows (decor.rs puts it
    /// behind them and in front of everything else).
    pub fn overview_dim(&self, out: Rectangle<i32, Logical>, scale: f64) -> Option<SceneElement> {
        let ov = self.overview.as_ref()?;
        let shade = if self.light_style() { [0.0, 0.0, 0.0, 0.45] } else { [0.0, 0.0, 0.0, 0.6] };
        let rect = Rectangle::<i32, Physical>::from_size(out.size.to_f64().to_physical(scale).to_i32_round());
        Some(SceneElement::Solid(SolidColorRenderElement::new(ov.dim.clone(), rect, CommitCounter::default(), shade, Kind::Unspecified)))
    }

    /// The windows' elements (front first, `spans` saying each window's)
    /// split: those of windows shown in the overview, each window's scaled
    /// whole into its place (its shape kept, centered), and the rest as they
    /// were (windows narrowed away by typing, the desktop's icons).
    pub fn overview_place(&self, parts: Vec<SceneElement>, spans: &[(Win, usize, usize)], out: Rectangle<i32, Logical>, scale: f64) -> (Vec<SceneElement>, Vec<SceneElement>) {
        use smithay::backend::renderer::element::{
            Element,
            utils::{Relocate, RelocateRenderElement, RescaleRenderElement},
        };
        let Some(ov) = self.overview.as_ref() else { return (Vec::new(), parts) };
        let target = |w: &Win| ov.entries.iter().find(|e| e.win == *w && !e.minimized && e.rect.size.w > 0).map(|e| e.rect);
        let mut slots: Vec<Option<SceneElement>> = parts.into_iter().map(Some).collect();
        let (mut placed, mut rest) = (Vec::new(), Vec::new());
        let k_out = smithay::utils::Scale::from(scale);
        for (w, a, b) in spans {
            let Some(rc) = target(w) else { continue };
            let mine: Vec<SceneElement> = slots[*a..*b].iter_mut().filter_map(Option::take).collect();
            // The window's whole: the union of what it is drawn with.
            let mut bx: Option<Rectangle<i32, Physical>> = None;
            for e in &mine {
                let g = e.geometry(k_out);
                bx = Some(bx.map_or(g, |b| b.merge(g)));
            }
            let Some(bx) = bx.filter(|b| b.size.w > 0 && b.size.h > 0) else { continue };
            let t0 = (rc.loc - out.loc).to_f64().to_physical(scale);
            let ts = rc.size.to_f64().to_physical(scale);
            let k = (ts.w / bx.size.w as f64).min(ts.h / bx.size.h as f64);
            let (sw, sh) = (bx.size.w as f64 * k, bx.size.h as f64 * k);
            let to = Point::<i32, Physical>::from(((t0.x + (ts.w - sw) / 2.0).round() as i32, (t0.y + (ts.h - sh) / 2.0).round() as i32));
            for e in mine {
                let part = match e {
                    SceneElement::Surface(x) => WinParts::Surface(x),
                    SceneElement::Solid(x) => WinParts::Solid(x),
                    SceneElement::Memory(x) => WinParts::Memory(x),
                    SceneElement::Texture(x) => WinParts::Texture(x),
                    SceneElement::Shaded(x) => WinParts::Shaded(x),
                    SceneElement::Backdrop(x) => WinParts::Backdrop(x),
                    _ => continue,
                };
                placed.push(SceneElement::Placed(RelocateRenderElement::from_element(RescaleRenderElement::from_element(part, bx.loc, k), to - bx.loc, Relocate::Relative)));
            }
        }
        rest.extend(slots.into_iter().flatten());
        (placed, rest)
    }
}
