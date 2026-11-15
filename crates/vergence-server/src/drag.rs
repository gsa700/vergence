// Drag and drop between native windows (files milestone 1, step 8; bus
// v16). A native window starts a drag while the pointer's left button is
// held on it (`DragStart`, carrying the data: file drags are a few paths);
// the seat goes on sending the pointer to it, and the server follows the
// pointer beside that: the native window under it hears `DragOver` (and
// `DragLeave`) and answers what a drop would do (`DragAccept`); the dock's
// app icons take files themselves (opened with that app). The left button's
// release drops: the window under the pointer gets `DragDrop` with the data,
// if it said it would take it; the source hears `DragEnded` with what was
// done. A ghost of the drag's words follows the pointer.
//
// Not yet: drags from and to Wayland and X11 programs (Smithay's drag
// grab knows only Wayland surfaces), nor drops on the desktop (step 9).

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Rectangle, Transform};
use vergence_protocol::bus::ServerMsg;
use vergence_protocol::window::{DRAG_CTRL, DRAG_SHIFT, DROP_COPY, DROP_NONE, WinEvent};

use crate::{Server, decor::SceneElement, win::Win};

/// Where a drag is.
#[derive(Clone, PartialEq)]
enum Over {
    Nothing,
    Native(Win),
    /// A dock icon that can open the files: its desktop entry's id.
    Dock(String),
}

pub struct NativeDrag {
    source: Win,
    label: String,
    items: Vec<(String, Vec<u8>)>,
    over: Over,
    /// What a drop where it is would do.
    action: u8,
    /// The ghost's picture (for its scale and words).
    ghost: Option<((String, u64, bool), MemoryRenderBuffer, (i32, i32), (f32, f32))>,
}

/// The files a drag carries (its text/uri-list's file:// lines).
fn files_of(items: &[(String, Vec<u8>)]) -> Vec<String> {
    items
        .iter()
        .find(|(k, _)| k == "text/uri-list")
        .map(|(_, v)| String::from_utf8_lossy(v).lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(vergence_types::path_of_link).map(|p| p.to_string_lossy().into_owned()).collect())
        .unwrap_or_default()
}

impl Server {
    /// A native window starts a drag: only while the left button is held
    /// on it (§7.3: no drags from the background).
    pub fn native_drag_start(&mut self, w: Win, label: String, items: Vec<(String, Vec<u8>)>) {
        let held = self.input.seat.buttons_held();
        let under = self.target_at(self.pointer_location());
        let on_it = matches!(under, Some(vergence_seat::Target::Native(id)) if self.input.wins.get(&id) == Some(&w));
        if !held || !on_it {
            return tracing::debug!("drag: refused (no button held on the window)");
        }
        tracing::info!("drag: {label:?} ({} kinds)", items.len());
        self.native_drag = Some(NativeDrag { source: w, label, items, over: Over::Nothing, action: DROP_NONE, ghost: None });
        let pos = self.pointer_location();
        self.native_drag_motion(pos);
    }

    fn drag_mods(&self) -> u8 {
        self.seat.get_keyboard().map(|k| k.modifier_state()).map_or(0, |m| (if m.ctrl { DRAG_CTRL } else { 0 }) | (if m.shift { DRAG_SHIFT } else { 0 }))
    }

    /// The pointer moved during a drag: whatever is under it hears.
    pub fn native_drag_motion(&mut self, pos: Point<f64, Logical>) {
        let Some(drag) = self.native_drag.as_ref() else { return };
        let (kinds, has_files) = (drag.items.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(), !files_of(&drag.items).is_empty());
        let before = drag.over.clone();
        let under = self.target_at(pos);
        let now = match under {
            Some(vergence_seat::Target::Native(id)) => self.input.wins.get(&id).cloned().map_or(Over::Nothing, Over::Native),
            Some(vergence_seat::Target::Server(vergence_seat::Part::Dock)) if has_files => match self.dock_item_at(pos) {
                Some(item) if self.shelf.apps.iter().any(|e| e.id == item.key) => Over::Dock(item.key),
                _ => Over::Nothing,
            },
            _ => Over::Nothing,
        };
        if before != now
            && let Over::Native(Win::Native(n)) = &before
        {
            self.bus_send(n.client, ServerMsg::Window(WinEvent::DragLeave { window: n.id }));
        }
        let mods = self.drag_mods();
        match &now {
            Over::Native(Win::Native(n)) => {
                let loc = self.space.element_location(&Win::Native(n.clone())).unwrap_or_default();
                let (x, y) = (pos.x - loc.x as f64, pos.y - loc.y as f64);
                self.bus_send(n.client, ServerMsg::Window(WinEvent::DragOver { window: n.id, x, y, kinds, mods }));
            }
            _ => {}
        }
        if let Some(d) = self.native_drag.as_mut() {
            if before != now {
                d.action = if matches!(now, Over::Dock(_)) { DROP_COPY } else { DROP_NONE };
            }
            d.over = now;
        }
        self.redraw_all();
    }

    /// A window's answer to DragOver.
    pub fn native_drag_accept(&mut self, w: &Win, action: u8) {
        if let Some(d) = self.native_drag.as_mut()
            && d.over == Over::Native(w.clone())
        {
            d.action = action;
        }
    }

    /// The left button let go: dropped where the pointer is.
    pub fn native_drag_drop(&mut self, pos: Point<f64, Logical>) {
        let Some(drag) = self.native_drag.take() else { return };
        let mut done = DROP_NONE;
        match &drag.over {
            Over::Native(Win::Native(n)) if drag.action != DROP_NONE => {
                let loc = self.space.element_location(&Win::Native(n.clone())).unwrap_or_default();
                let (x, y) = (pos.x - loc.x as f64, pos.y - loc.y as f64);
                self.bus_send(n.client, ServerMsg::Window(WinEvent::DragDrop { window: n.id, x, y, action: drag.action, items: drag.items.clone() }));
                done = drag.action;
            }
            Over::Native(Win::Native(n)) => self.bus_send(n.client, ServerMsg::Window(WinEvent::DragLeave { window: n.id })),
            Over::Dock(key) => {
                let files = files_of(&drag.items);
                match self.open_items(&files, key) {
                    Ok(()) => done = DROP_COPY,
                    Err(e) => tracing::info!("drag: the dock's {key} could not open them: {e}"),
                }
            }
            _ => {}
        }
        if let Win::Native(s) = &drag.source {
            self.bus_send(s.client, ServerMsg::Window(WinEvent::DragEnded { window: s.id, action: done }));
        }
        tracing::info!("drag: dropped ({})", ["nothing done", "copied", "moved"][done.min(2) as usize]);
        self.redraw_all();
    }

    /// A drag whose button is no longer held (its release missed): ended.
    pub fn native_drag_check(&mut self) {
        if self.native_drag.is_some() && !self.input.seat.buttons_held() {
            let pos = self.pointer_location();
            if let Some(d) = self.native_drag.as_mut() {
                // Nothing was let go over: nothing is done.
                d.action = DROP_NONE;
                if matches!(d.over, Over::Dock(_)) {
                    d.over = Over::Nothing;
                }
            }
            self.native_drag_drop(pos);
        }
    }

    /// The ghost under the pointer: the drag's words in a rounded box, the
    /// accent's while a drop there would do something.
    pub fn native_drag_ghost(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64, light: bool) -> Option<SceneElement> {
        let pos = self.pointer_location();
        let accent = self.accent();
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        let d = self.native_drag.as_mut()?;
        let active = d.action != DROP_NONE;
        let key = (d.label.clone(), scale.to_bits(), active);
        if d.ghost.as_ref().is_none_or(|(k, ..)| *k != key) {
            let face = fonts.face("sans-serif", 400, false);
            let tw = face.map_or(80.0, |f| fonts.layout(f, 13.0, &d.label).1);
            let (w, h) = ((tw + 28.0).clamp(60.0, 360.0), 30.0f32);
            let style = vergence_look::bar_look(light);
            let mut scene = vergence_scene::Scene::new(w, h);
            let (fill, ink) = if active { (vergence_scene::Color::rgba(accent.r, accent.g, accent.b, 230), vergence_scene::Color::rgb(255, 255, 255)) } else { (vergence_scene::Color::rgba(style.front.0, style.front.1, style.front.2, 225), vergence_scene::Color::rgb(style.ink.0, style.ink.1, style.ink.2)) };
            vergence_bar::draw_ghost(&mut scene, fonts, w, h, &d.label, face, 13.0, fill, ink);
            let p = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, vergence_scene::Color::rgba(0, 0, 0, 0)).ok()?;
            let buffer = MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None);
            d.ghost = Some((key, buffer, (p.width as i32, p.height as i32), (w, h)));
        }
        let (_, buffer, px, (w, h)) = d.ghost.as_ref()?;
        let at = Point::<f64, Logical>::from((pos.x + 14.0, pos.y + 14.0));
        let r = Rectangle::<f64, Logical>::new(at, (*w as f64, *h as f64).into());
        if !r.overlaps(out.to_f64()) {
            return None;
        }
        let whole = Rectangle::<f64, Logical>::from_size((px.0 as f64, px.1 as f64).into());
        let loc = (at - out.loc.to_f64()).to_physical(scale).to_i32_round::<i32>();
        MemoryRenderBufferRenderElement::from_buffer(renderer, loc.to_f64(), buffer, None, Some(whole), Some((w.round() as i32, h.round() as i32).into()), Kind::Unspecified).ok().map(SceneElement::Memory)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_drags_files_are_its_uri_lists_file_lines() {
        let items = vec![("text/plain".to_string(), b"/x".to_vec()), ("text/uri-list".to_string(), b"# a comment\r\nfile:///home/u/a%20b.txt\r\nhttps://example.org\r\nfile:///tmp/c\r\n".to_vec())];
        assert_eq!(super::files_of(&items), vec!["/home/u/a b.txt".to_string(), "/tmp/c".to_string()]);
        assert!(super::files_of(&[]).is_empty());
    }
}
