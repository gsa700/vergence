// The frost over what is behind something the server draws (look milestone
// 1, step 5): the dock now; Wayland and X11 windows' bars and the menus next.
//
// A framebuffer effect, as Smithay names it: before the element is drawn,
// the renderer has already painted everything behind it this frame (Smithay
// widens the damage to the whole area behind a framebuffer effect whenever
// what is there changed, and then calls `capture_framebuffer`). The capture
// copies that region of the framebuffer into a texture of our own; drawing
// runs vergence-render's frost over it (blur, tint, the tint raised where
// what stands on it needs). Which way up the copy comes depends on the
// frame: drawing to the screen, the framebuffer's rows run from the bottom
// (the copy is drawn flipped); drawing into a texture (a screenshot), the
// frame is flipped already and its rows run from the top (copied the other
// way, the top of the screen was frosted into a dock at the bottom).
//
// The texture is made by the scene builder (which has the renderer) and kept
// while the size stays the same; the element only copies into it.

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement},
        gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformValue, ffi},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{Buffer as BufferCoords, Physical, Rectangle, Scale, Transform, user_data::UserDataMap},
};

/// What a backdrop frost is made with: the tint (sRGB, 0 to 1) and the
/// frost's own strength, the ink that stands on it and the contrast to keep,
/// the blur's reach and the corners' radius in pixels, and whether the bottom
/// corners are rounded too (a dock, a menu; a bar rounds its top ones only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub tint: [f32; 3],
    pub strength: f32,
    pub ink: [f32; 3],
    pub need: f32,
    pub radius: f32,
    pub corner: f32,
    pub bottom: bool,
    /// No lighter than the tint (a light bar's frost; decor.rs).
    pub cap: bool,
}

pub struct Backdrop {
    id: Id,
    commit: CommitCounter,
    dst: Rectangle<i32, Physical>,
    texture: GlesTexture,
    program: GlesTexProgram,
    uniforms: Vec<Uniform<'static>>,
}

impl Backdrop {
    /// A backdrop frost over `dst` (physical pixels of the output), drawn
    /// with `program` (the frost's) into `texture` (dst's size).
    pub fn new(
        id: Id,
        commit: CommitCounter,
        dst: Rectangle<i32, Physical>,
        texture: GlesTexture,
        program: GlesTexProgram,
        look: &Look,
    ) -> Backdrop {
        let (w, h) = (dst.size.w.max(1) as f32, dst.size.h.max(1) as f32);
        let uniforms = vec![
            Uniform::new("texel", UniformValue::_2f(1.0 / w, 1.0 / h)),
            Uniform::new("radius", UniformValue::_1f(look.radius)),
            Uniform::new("tint", UniformValue::_4f(look.tint[0], look.tint[1], look.tint[2], look.strength)),
            Uniform::new("ink", UniformValue::_3f(look.ink[0], look.ink[1], look.ink[2])),
            Uniform::new("need", UniformValue::_1f(look.need)),
            Uniform::new("corner", UniformValue::_1f(look.corner)),
            Uniform::new("bottom", UniformValue::_1f(if look.bottom { 1.0 } else { 0.0 })),
            // Said every time: a program's uniforms keep their last value, and a
            // native bar may have inverted (decor.rs).
            Uniform::new("invert", UniformValue::_1f(0.0)),
            Uniform::new("cap", UniformValue::_1f(if look.cap { 1.0 } else { 0.0 })),
            // Set every time: a program keeps the last value given (a sidebar
            // window's split or a glass strip's clear would carry over).
            Uniform::new("split", UniformValue::_1f(0.0)),
            Uniform::new("clear", UniformValue::_1f(0.0)),
        ];
        Backdrop { id, commit, dst, texture, program, uniforms }
    }
}

impl Element for Backdrop {
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
        if commit == Some(self.commit) {
            DamageSet::default()
        } else {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
        }
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        1.0
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }

    fn is_framebuffer_effect(&self) -> bool {
        true
    }
}

impl RenderElement<GlesRenderer> for Backdrop {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let turn = if rows_from_top(frame) { Transform::Normal } else { Transform::Flipped180 };
        frame.render_texture_from_to(
            &self.texture,
            src,
            dst,
            damage,
            opaque_regions,
            turn,
            1.0,
            Some(&self.program),
            &self.uniforms,
        )
    }

    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        _cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        let texture = self.texture.tex_id();
        let from_top = rows_from_top(frame);
        frame.with_context(|gl| {
            // SAFETY: GL calls in the frame's own context on a texture we made
            // of dst's size; the binding is put back to none after.
            unsafe {
                let mut viewport = [0i32; 4];
                gl.GetIntegerv(ffi::VIEWPORT, viewport.as_mut_ptr());
                let top = viewport[1] + viewport[3];
                gl.BindTexture(ffi::TEXTURE_2D, texture);
                let y = if from_top { viewport[1] + dst.loc.y } else { top - dst.loc.y - dst.size.h };
                gl.CopyTexSubImage2D(ffi::TEXTURE_2D, 0, 0, 0, dst.loc.x, y, dst.size.w, dst.size.h);
                gl.BindTexture(ffi::TEXTURE_2D, 0);
            }
        })
    }
}

/// Whether this frame's framebuffer rows run from the output's top (into a
/// texture, as a screenshot is drawn) rather than from its bottom (the
/// screen): read from the frame's projection, whose vertical scale is
/// negative when the output's top is the framebuffer's last row. (The
/// frame's transformation says Normal either way.)
fn rows_from_top(frame: &GlesFrame<'_, '_>) -> bool {
    frame.projection()[4] > 0.0
}

/// A surface's frost texture, kept while its size holds: the element's id,
/// the texture its capture goes into, and its commit, moved on whenever the
/// texture is new. (Smithay captures what is behind a framebuffer effect when
/// that changed, or when the element reports damage of its own; a menu that
/// opens over a still desktop changes nothing behind it, so without the new
/// commit its texture stayed empty: a menu and a Wayland
/// bar frosted only their tint.)
/// The element's id, its texture, its commit, and the look it was last
/// drawn with: a new look (the dock's opacity, the Frost step, light or
/// dark) is a new commit, or the screen kept the old frost until something
/// behind it changed (the dock's opacity slider seemed to do nothing).
pub type Slot = Option<(Id, GlesTexture, CommitCounter, Look, Ctx)>;

/// The GL context a texture or program was made on. After a GPU hang the
/// server draws with a new context (drm.rs renew_renderer); everything made on
/// the old one is dead there (textures warn "from different renderer context",
/// programs fail glUseProgram), so each cache keeps the context its things were
/// made on and makes them again when it differs (kepler, 2026-11-27: windows
/// flashing and the desktop's icons gone after a hang).
pub type Ctx = smithay::backend::renderer::ContextId<GlesTexture>;

/// The renderer's context now.
pub fn ctx(renderer: &GlesRenderer) -> Ctx {
    use smithay::backend::renderer::Renderer;
    renderer.context_id()
}

/// The backdrop frost over `dst` for a surface whose texture is kept in
/// `slot` (made or remade here when the size changed). None if the texture
/// cannot be made.
pub fn element(
    renderer: &mut GlesRenderer,
    slot: &mut Slot,
    dst: Rectangle<i32, Physical>,
    program: GlesTexProgram,
    look: &Look,
) -> Option<Backdrop> {
    use smithay::backend::renderer::{Offscreen, Texture};
    if dst.size.w <= 0 || dst.size.h <= 0 {
        return None;
    }
    let now = ctx(renderer);
    let fresh = slot
        .as_ref()
        .is_none_or(|(_, t, _, _, c)| (t.width() as i32, t.height() as i32) != (dst.size.w, dst.size.h) || *c != now);
    if fresh {
        match Offscreen::<GlesTexture>::create_buffer(
            renderer,
            smithay::backend::allocator::Fourcc::Abgr8888,
            (dst.size.w, dst.size.h).into(),
        ) {
            Ok(t) => {
                let (id, mut commit) =
                    slot.take().map_or_else(|| (Id::new(), CommitCounter::default()), |(id, _, c, ..)| (id, c));
                commit.increment();
                *slot = Some((id, t, commit, *look, now));
            }
            Err(e) => {
                tracing::warn!("frost: no texture for {dst:?}: {e:?}");
                return None;
            }
        }
    }
    if let Some((_, _, commit, was, _)) = slot.as_mut()
        && *was != *look
    {
        commit.increment();
        *was = *look;
    }
    let (id, texture, commit, ..) = slot.as_ref()?;
    Some(Backdrop::new(id.clone(), *commit, dst, texture.clone(), program, look))
}

/// sRGB 0-255 as 0-1, for a look.
pub fn unit(c: vergence_look::Rgb) -> [f32; 3] {
    [c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0]
}

/// A texture drawn through one of our shader programs (the frost of a
/// window's own top strip, a picture with rounded corners), with an id that
/// stays the window's and a commit that moves only when the picture, its
/// place or the shader's numbers change. Smithay's own TextureShaderElement
/// takes a new id each time it is made: every frame looked new, so the
/// screen was drawn again at every refresh (25% of a core while idle).
pub struct Shaded {
    id: Id,
    commit: CommitCounter,
    dst: Rectangle<i32, Physical>,
    src: Rectangle<f64, BufferCoords>,
    texture: GlesTexture,
    program: GlesTexProgram,
    uniforms: Vec<Uniform<'static>>,
}

/// What a window keeps for one Shaded element: its id, its commit, and what
/// it was last made of (to know when to move the commit).
#[derive(Default)]
pub struct ShadedSlot {
    id: Option<Id>,
    commit: CommitCounter,
    last: u64,
}

impl Shaded {
    /// `texture` drawn from `src` (its pixels) to `dst` (the output's).
    /// `content` changes with what the texture holds (a native window's
    /// picture serial): the texture's own number may be the same for a new
    /// picture, GL reusing a freed one.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        slot: &mut ShadedSlot,
        dst: Rectangle<i32, Physical>,
        src: Rectangle<f64, BufferCoords>,
        texture: GlesTexture,
        content: u64,
        program: GlesTexProgram,
        uniforms: Vec<Uniform<'static>>,
    ) -> Shaded {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (
            texture.tex_id(),
            content,
            dst.loc.x,
            dst.loc.y,
            dst.size.w,
            dst.size.h,
            src.loc.x.to_bits(),
            src.loc.y.to_bits(),
            src.size.w.to_bits(),
            src.size.h.to_bits(),
        )
            .hash(&mut h);
        for u in &uniforms {
            u.name.hash(&mut h);
            format!("{:?}", u.value).hash(&mut h);
        }
        let made = h.finish();
        if slot.last != made {
            slot.last = made;
            slot.commit.increment();
        }
        let id = slot.id.get_or_insert_with(Id::new).clone();
        Shaded { id, commit: slot.commit, dst, src, texture, program, uniforms }
    }
}

impl Element for Shaded {
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
        self.src
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        if commit == Some(self.commit) {
            DamageSet::default()
        } else {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
        }
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        1.0
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for Shaded {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        frame.render_texture_from_to(
            &self.texture,
            src,
            dst,
            damage,
            opaque_regions,
            Transform::Normal,
            1.0,
            Some(&self.program),
            &self.uniforms,
        )
    }
}
