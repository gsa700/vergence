// Window pictures (overview milestone 1, step 1; docs/overview-milestone-1.md):
// a small live picture of any window, for the dock's picker, Alt+Tab's tiles
// and Super+Tab's overview.
//
// A window's picture is its own drawing, the same elements the screen gets
// (a native window's picture, a Wayland or X11 window's surfaces), drawn at
// a smaller scale into a texture of its own, offscreen. It is not drawn
// straight into the scene: a window and its picture shown in the same frame
// (Alt+Tab over the windows) would be two elements of one id, and the damage
// tracker draws an id once. The texture is kept per window while something
// asks for it and dropped when nothing has for a while; it is drawn again
// at most `EVERY` while asked for, so a picker shows the window live. A
// minimized window is off the screen, but its surfaces keep their last
// buffer: its picture is the window as it was when it went.
//
// The texture is drawn into the scene by a `ThumbElement`, with an id that
// stays the window's and a commit that moves when the picture is drawn again.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use smithay::{
    backend::renderer::{
        Bind, Offscreen,
        damage::OutputDamageTracker,
        element::{AsRenderElements, Element, Id, Kind, RenderElement, memory::MemoryRenderBufferRenderElement, surface::WaylandSurfaceRenderElement, texture::TextureRenderElement},
        gles::{GlesError, GlesFrame, GlesRenderer, GlesTexture},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size, Transform, user_data::UserDataMap},
};

use crate::decor::SceneElement;
use crate::state::Server;
use crate::win::Win;

/// How often a shown picture is drawn again: 15 times a second.
pub const EVERY: Duration = Duration::from_millis(66);
/// A picture nothing has asked for in this long is let go.
const KEEP: Duration = Duration::from_secs(10);

/// One window's picture: its texture and size in pixels, the scale it was
/// drawn at (pixels per point of the window), when it was drawn and last
/// asked for, and its element's id and commit.
pub struct Thumb {
    texture: GlesTexture,
    px: Size<i32, Physical>,
    drawn: Instant,
    asked: Instant,
    id: Id,
    commit: CommitCounter,
}

/// Every window's picture, by window.
#[derive(Default)]
pub struct Thumbs {
    pub by_window: HashMap<Win, Thumb>,
}

/// The size a window's content is, in points (a native window's picture
/// from its top, under its bar if it draws there; another window's
/// geometry), or None if it has nothing to show.
fn content_size(w: &Win) -> Option<Size<i32, Logical>> {
    if let Some(n) = w.native() {
        return n.picture().map(|p| p.logical).filter(|s| s.w > 0 && s.h > 0);
    }
    let g = w.geometry();
    (g.size.w > 0 && g.size.h > 0).then_some(g.size)
}

/// The scale that fits `content` (points) into `room` (pixels), keeping its
/// shape, never above one pixel a point (a small window is not blown up
/// blurred), and the picture's size in pixels at that scale (at least 1x1).
pub fn fit(content: Size<i32, Logical>, room: Size<i32, Physical>) -> (f64, Size<i32, Physical>) {
    let k = (room.w as f64 / content.w.max(1) as f64).min(room.h as f64 / content.h.max(1) as f64).clamp(0.0, 1.0);
    let px = Size::from((((content.w as f64 * k).round() as i32).max(1), ((content.h as f64 * k).round() as i32).max(1)));
    (k, px)
}

/// Window `w` drawn at `k` pixels a point into a new texture `px` big.
fn draw(renderer: &mut GlesRenderer, w: &Win, k: f64, px: Size<i32, Physical>) -> Result<GlesTexture, String> {
    let mut elements: Vec<SceneElement> = Vec::new();
    let size_pt = Size::<i32, Logical>::from((px.w, px.h));
    if let Some(n) = w.native() {
        let p = n.picture().ok_or("no picture")?;
        // All of its pixels (as decor.rs: given with the size, or the size is not used).
        let whole = Rectangle::<f64, Logical>::from_size((p.px.w as f64, p.px.h as f64).into());
        match &p.content {
            crate::native::Content::Memory(buffer) => {
                let e = MemoryRenderBufferRenderElement::from_buffer(renderer, (0.0, 0.0), buffer, None, Some(whole), Some(size_pt), Kind::Unspecified).map_err(|e| format!("{e:?}"))?;
                elements.push(SceneElement::Memory(e));
            }
            crate::native::Content::Gpu(_) => {
                let (t, _, _, _) = crate::decor::native_texture(renderer, w).ok_or("no GPU picture")?;
                elements.push(SceneElement::Texture(TextureRenderElement::from_texture_buffer((0.0, 0.0), &t, None, Some(whole), Some(size_pt), Kind::Unspecified)));
            }
        }
    } else {
        // From the window's geometry's corner (a client's own shadow left out).
        let at: Point<i32, Physical> = (Point::<i32, Logical>::from((0, 0)) - w.geometry().loc).to_f64().to_physical(k).to_i32_round();
        let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = w.render_elements(renderer, at, Scale::from(k), 1.0);
        elements.extend(surfaces.into_iter().map(SceneElement::Surface));
    }
    let buf = Size::<i32, BufferCoords>::from((px.w, px.h));
    let mut tex: GlesTexture = renderer.create_buffer(smithay::backend::allocator::Fourcc::Abgr8888, buf).map_err(|e| e.to_string())?;
    {
        let mut fb = renderer.bind(&mut tex).map_err(|e| e.to_string())?;
        let mut damage = OutputDamageTracker::new(px, 1.0, Transform::Normal);
        damage.render_output(renderer, &mut fb, 0, &elements, [0.0, 0.0, 0.0, 0.0]).map_err(|e| format!("{e:?}"))?;
    }
    Ok(tex)
}

impl Server {
    /// Window `w`'s picture fitted into `room` pixels, drawn (again, if it is
    /// older than `EVERY` or its size changed) and kept; its texture and size.
    /// None if the window has nothing to show or it cannot be drawn.
    pub fn thumb(&mut self, renderer: &mut GlesRenderer, w: &Win, room: Size<i32, Physical>) -> Option<(GlesTexture, Size<i32, Physical>)> {
        let content = content_size(w)?;
        let (k, px) = fit(content, room);
        let now = Instant::now();
        let fresh = self.thumbs.by_window.get(w).is_some_and(|t| t.px == px && now.duration_since(t.drawn) < EVERY);
        if !fresh {
            match draw(renderer, w, k, px) {
                Ok(texture) => {
                    let (id, mut commit) = self.thumbs.by_window.remove(w).map_or_else(|| (Id::new(), CommitCounter::default()), |t| (t.id, t.commit));
                    commit.increment();
                    self.thumbs.by_window.insert(w.clone(), Thumb { texture, px, drawn: now, asked: now, id, commit });
                }
                Err(e) => {
                    tracing::debug!("thumbs: {:?}: {e}", self.window_title(w));
                    // An older picture is still better than none.
                    if !self.thumbs.by_window.contains_key(w) {
                        return None;
                    }
                }
            }
        }
        let t = self.thumbs.by_window.get_mut(w)?;
        t.asked = now;
        Some((t.texture.clone(), t.px))
    }

    /// Window `w`'s picture as an element filling `dst` (its shape kept,
    /// centered in it), faded to `alpha`. None if it has none.
    pub fn thumb_element(&mut self, renderer: &mut GlesRenderer, w: &Win, dst: Rectangle<i32, Physical>, alpha: f32) -> Option<ThumbElement> {
        let (texture, px) = self.thumb(renderer, w, dst.size)?;
        let t = self.thumbs.by_window.get(w)?;
        let at = Point::<i32, Physical>::from((dst.loc.x + (dst.size.w - px.w) / 2, dst.loc.y + (dst.size.h - px.h) / 2));
        Some(ThumbElement { id: t.id.clone(), commit: t.commit, dst: Rectangle::new(at, px), texture, alpha })
    }

    /// Pictures of windows that are gone, or that nothing has asked for in a
    /// while, let go.
    pub fn thumbs_prune(&mut self) {
        let now = Instant::now();
        self.thumbs.by_window.retain(|w, t| smithay::utils::IsAlive::alive(w) && now.duration_since(t.asked) < KEEP);
    }
}

/// A window's picture in the scene.
pub struct ThumbElement {
    id: Id,
    commit: CommitCounter,
    dst: Rectangle<i32, Physical>,
    texture: GlesTexture,
    alpha: f32,
}

impl Element for ThumbElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.dst
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        Rectangle::from_size((self.dst.size.w as f64, self.dst.size.h as f64).into())
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        if commit == Some(self.commit) { DamageSet::default() } else { DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)]) }
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for ThumbElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        frame.render_texture_from_to(&self.texture, src, dst, damage, opaque_regions, Transform::Normal, self.alpha, None, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_keeps_its_window_shape_inside_its_room() {
        // A wide window into a square: as wide as the room, shorter.
        let (k, px) = fit(Size::from((1000, 500)), Size::from((200, 200)));
        assert!((k - 0.2).abs() < 1e-9);
        assert_eq!((px.w, px.h), (200, 100));
        // A tall one: as tall as the room.
        let (_, px) = fit(Size::from((400, 800)), Size::from((300, 200)));
        assert_eq!((px.w, px.h), (100, 200));
        // A small window is not blown up.
        let (k, px) = fit(Size::from((100, 50)), Size::from((400, 400)));
        assert!((k - 1.0).abs() < 1e-9);
        assert_eq!((px.w, px.h), (100, 50));
        // Never nothing, however small the room.
        let (_, px) = fit(Size::from((4000, 10)), Size::from((10, 10)));
        assert!(px.w >= 1 && px.h >= 1);
    }
}
