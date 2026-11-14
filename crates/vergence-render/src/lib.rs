//! A scene into pixels (whitepaper §6.10; toolkit milestone 1).
//!
//! Two renderers are planned for one scene format, and they must agree:
//!
//! - **the CPU renderer** ([`Cpu`], cpu.rs), the reference: plain software,
//!   in horizontal bands across the cores. It is what tests compare against,
//!   what draws on a machine without a GPU, and what draws a window that
//!   changes once;
//! - **the GPU renderer** (step 3 of the milestone), for everything else.
//!
//! A scene is in reference units; a renderer is told the scale (pixels per
//! unit) of the monitor it draws for, so the same scene comes out sharp at
//! any density. Glyphs come from `vergence_text`'s cache, rasterized at the
//! exact pixel size and tinted here, so color costs nothing.
//!
//! The result is a [`Pixmap`]: 8-bit RGBA with alpha multiplied in, rows
//! from the top. Colors are blended as they are stored (sRGB values, no
//! linearization), which is what toolkits and terminals do and what the GPU
//! renderer will match.
//!
//! A renderer takes a scene from anyone: it validates before it draws, and
//! refuses sizes that would need an absurd buffer.

pub mod blur;
pub mod cpu;
mod gl;
pub mod gpu;

use std::fmt;

pub use cpu::{Cpu, Order, Target};
pub use gl::preload as preload_gpu_libraries;
pub use gpu::{Gpu, GpuBuffer, GpuError};
use vergence_scene::{Color, Glyph, Point, Scene, SceneError};
use vergence_text::{FaceId, Fonts};

/// The largest side of a picture, in pixels, and the most pixels in one.
pub const MAX_SIDE: u32 = 32_768;
pub const MAX_PIXELS: u64 = 256 << 20;
/// The largest scale (pixels per reference unit).
pub const MAX_SCALE: f32 = 64.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// The scene is not valid.
    Scene(SceneError),
    /// The scale is not a positive, finite, bounded number.
    Scale,
    /// The picture would be larger than a renderer will make.
    TooLarge,
    /// The pixmap given to draw into is not this scene's size at this scale.
    Size,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::Scene(e) => write!(f, "invalid scene: {e}"),
            RenderError::Scale => write!(f, "bad scale"),
            RenderError::TooLarge => write!(f, "the picture would be too large"),
            RenderError::Size => write!(f, "the pixmap is not the scene's size"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<SceneError> for RenderError {
    fn from(e: SceneError) -> Self {
        RenderError::Scene(e)
    }
}

/// The size in pixels of a scene drawn at `scale`: every started pixel counts.
pub fn pixel_size(scene: &Scene, scale: f32) -> Result<(u32, u32), RenderError> {
    if !(scale.is_finite() && scale > 0.0 && scale <= MAX_SCALE) {
        return Err(RenderError::Scale);
    }
    let (w, h) = ((scene.width * scale).ceil(), (scene.height * scale).ceil());
    if !(w >= 1.0 && h >= 1.0 && w <= MAX_SIDE as f32 && h <= MAX_SIDE as f32) || (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(RenderError::TooLarge);
    }
    Ok((w as u32, h as u32))
}

/// A picture: `width` x `height` pixels, four bytes each (red, green, blue,
/// alpha, with alpha multiplied into the colors), rows from the top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pixmap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Pixmap {
    /// A transparent picture.
    pub fn new(width: u32, height: u32) -> Pixmap {
        Pixmap { width, height, data: vec![0; width as usize * height as usize * 4] }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }

    /// A number that changes if any pixel does (FNV-1a), for comparing
    /// pictures in tests and across machines.
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in self.width.to_le_bytes().iter().chain(self.height.to_le_bytes().iter()).chain(self.data.iter()) {
            h = (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// The same picture with alpha not multiplied in, as PNG and most file
    /// formats want it.
    pub fn straight_rgba(&self) -> Vec<u8> {
        let mut out = self.data.clone();
        for p in out.chunks_exact_mut(4) {
            let a = p[3] as u32;
            if a != 0 && a != 255 {
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        out
    }
}

/// Adds one line of text to a scene: laid out by `fonts` in `face` at `size`
/// (reference units to the em), its first baseline origin at `origin`, one
/// glyph run per font the line needs. Returns the line's width.
pub fn text(scene: &mut Scene, fonts: &mut Fonts, face: FaceId, size: f32, color: Color, origin: Point, text: &str) -> f32 {
    let (runs, width) = fonts.layout(face, size, text);
    for run in runs {
        let name = fonts.name(run.face);
        let font = scene.font(&name.family, name.weight, name.italic);
        let glyphs = run.glyphs.iter().map(|g| Glyph { id: g.glyph, x: g.x, y: g.y }).collect();
        scene.glyphs(font, size, color, origin, glyphs);
    }
    width
}
