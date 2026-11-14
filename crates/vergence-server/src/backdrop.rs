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
// frame is flipped already and its rows run from the top (found on Hubble:
// the first try frosted the top of the screen into a dock at the bottom).
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
    pub fn new(id: Id, commit: CommitCounter, dst: Rectangle<i32, Physical>, texture: GlesTexture, program: GlesTexProgram, look: &Look) -> Backdrop {
        let (w, h) = (dst.size.w.max(1) as f32, dst.size.h.max(1) as f32);
        let uniforms = vec![
            Uniform::new("texel", UniformValue::_2f(1.0 / w, 1.0 / h)),
            Uniform::new("radius", UniformValue::_1f(look.radius)),
            Uniform::new("tint", UniformValue::_4f(look.tint[0], look.tint[1], look.tint[2], look.strength)),
            Uniform::new("ink", UniformValue::_3f(look.ink[0], look.ink[1], look.ink[2])),
            Uniform::new("need", UniformValue::_1f(look.need)),
            Uniform::new("corner", UniformValue::_1f(look.corner)),
            Uniform::new("bottom", UniformValue::_1f(if look.bottom { 1.0 } else { 0.0 })),
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
        frame.render_texture_from_to(&self.texture, src, dst, damage, opaque_regions, turn, 1.0, Some(&self.program), &self.uniforms)
    }

    fn capture_framebuffer(&self, frame: &mut GlesFrame<'_, '_>, _src: Rectangle<f64, BufferCoords>, dst: Rectangle<i32, Physical>, _cache: &UserDataMap) -> Result<(), GlesError> {
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
/// frame's transformation says Normal either way: found on Hubble.)
fn rows_from_top(frame: &GlesFrame<'_, '_>) -> bool {
    frame.projection()[4] > 0.0
}
