//! The display list: what a window looks like, as data in reference units, drawn by whoever
//! renders it.
//!
//! Whitepaper §6.6, §6.10; toolkit milestone 1, step 1. Used by every crate that draws:
//! vergence-render (draws it), vergence-ui and vergence-widgets (build it), the server (draws
//! windows from their scenes, display lists milestone 1), vergence-bar and the apps. No
//! dependencies.
//!
//! A [`Scene`] is a list of drawing commands in **reference units** (points at 96 to the inch,
//! whitepaper Contract 1), with the window's top-left corner at 0,0 and y downwards. It holds no
//! pixels of its own making and names no scale: whoever renders it decides the density, so the
//! same scene is sharp on every monitor, can be drawn again at any size, and can be sent to
//! another machine as it is.
//!
//! The commands ([`Command`]):
//!
//! - [`Command::Fill`] and [`Command::Stroke`]: a rectangle, corners rounded or not, filled or
//!   outlined;
//! - [`Command::Line`]: a straight line of a given width, round ends;
//! - [`Command::Glyphs`]: glyphs of one font at one size in one color, each placed by the
//!   toolkit's text code. The font is named by family, weight and slant in the scene's font
//!   table, so a renderer anywhere can find it;
//! - [`Command::Image`]: a picture from the scene's image table, stretched to a rectangle;
//! - [`Command::PushClip`] / [`Command::PopClip`]: everything between them is kept inside a
//!   (rounded) rectangle;
//! - [`Command::Cells`]: a grid of equal cells, each with its own ground, ink and glyph: a
//!   terminal's screen in one command of sixteen bytes a cell, where fills and glyph runs would
//!   take two commands for every cell whose colors are its own.
//!
//! Layers with their own opacity, gradients and paths come when a tool needs them.
//!
//! Start reading: [`Command`], then [`Scene`] and [`Scene::validate`], then wire.rs (the
//! encoding, [`Scene::decode`]).
//!
//! Rule: **a scene is untrusted input wherever it is read.** [`Scene::validate`] checks every
//! number, index and count against the limits below, and [`Scene::decode`] never returns a
//! scene that fails it. A renderer may rely on a validated scene: indices are in range, numbers
//! are finite and bounded, clips are balanced.

pub mod wire;

use std::fmt;
use std::sync::Arc;

/// The wire format's version (wire.rs).
pub const VERSION: u16 = 1;

/// Largest coordinate or length, in reference units, either sign: far
/// beyond any wall of monitors, small enough that sums stay exact in f32.
pub const MAX_COORD: f32 = 1_000_000.0;
/// Largest font size, in reference units.
pub const MAX_FONT_SIZE: f32 = 4096.0;
pub const MAX_COMMANDS: usize = 1 << 20;
/// Glyphs in one run, and in a whole scene.
pub const MAX_RUN_GLYPHS: usize = 1 << 16;
pub const MAX_SCENE_GLYPHS: usize = 1 << 22;
pub const MAX_FONTS: usize = 4096;
/// Longest font family name, in bytes.
pub const MAX_FAMILY: usize = 256;
pub const MAX_IMAGES: usize = 1024;
/// Largest image side, in pixels, and the most pixel bytes in one scene.
pub const MAX_IMAGE_SIDE: u32 = 16_384;
pub const MAX_IMAGE_BYTES: usize = 256 << 20;
/// How deep clips may nest.
pub const MAX_CLIP_DEPTH: usize = 64;
/// The largest blur radius, in points, and how deep blurred groups nest.
pub const MAX_BLUR: f32 = 64.0;
pub const MAX_BLUR_DEPTH: usize = 4;
/// Cells in one row of a grid. (The cells of a whole scene count with its
/// glyphs, against [`MAX_SCENE_GLYPHS`].)
pub const MAX_GRID_COLUMNS: usize = 1 << 12;

/// A color: sRGB, alpha not multiplied in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }
}

/// A point, in reference units.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Point {
        Point { x, y }
    }
}

/// A rectangle, in reference units: top-left corner and size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
}

/// A font, named so that any renderer can find it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FontRef {
    pub family: String,
    /// 1 to 1000; 400 is regular, 700 bold.
    pub weight: u16,
    pub italic: bool,
}

/// A picture: `width` x `height` pixels, four bytes each (red, green, blue,
/// alpha; sRGB, alpha not multiplied in), rows from the top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// One glyph of a run: its number in the font, and where its origin (on the
/// baseline) sits relative to the run's own origin.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Glyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// One cell of a [`Command::Cells`] grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cell {
    /// What is behind the cell; alpha 0 for nothing (what was drawn before
    /// the grid shows).
    pub ground: Color,
    /// The color of the glyph, and of the underline.
    pub ink: Color,
    /// The glyph's number in the font `font` (0 is a font's own "no such
    /// character" box, and is drawn like any other). Looked at only with
    /// [`CELL_GLYPH`].
    pub glyph: u32,
    /// An index into [`Scene::fonts`]. Looked at only with [`CELL_GLYPH`].
    pub font: u16,
    /// [`CELL_UNDERLINE`], [`CELL_GLYPH`]; no other bit may be set.
    pub flags: u16,
}

/// A line along the cell's foot, in its ink.
pub const CELL_UNDERLINE: u16 = 1;
/// The cell has a glyph.
pub const CELL_GLYPH: u16 = 2;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// A filled rectangle; `radius` rounds its corners (0: square).
    Fill {
        rect: Rect,
        radius: f32,
        color: Color,
    },
    /// A rectangle's outline, `width` wide, drawn inside `rect`.
    Stroke {
        rect: Rect,
        radius: f32,
        width: f32,
        color: Color,
    },
    /// A straight line with round ends.
    Line {
        from: Point,
        to: Point,
        width: f32,
        color: Color,
    },
    /// Glyphs of the font `font` (an index into [`Scene::fonts`]) at `size`
    /// reference units to the em, the first baseline origin at `origin`.
    Glyphs {
        font: u32,
        size: f32,
        color: Color,
        origin: Point,
        glyphs: Vec<Glyph>,
    },
    /// The picture `image` (an index into [`Scene::images`]) stretched to
    /// `rect`, at `opacity` (0 to 1).
    Image {
        image: u32,
        rect: Rect,
        opacity: f32,
    },
    /// Until the matching [`Command::PopClip`], nothing is drawn outside
    /// this rectangle (and the clips around it).
    PushClip {
        rect: Rect,
        radius: f32,
    },
    PopClip,
    /// Until the matching [`Command::PopBlur`], what is drawn is cut to
    /// `rect` and blurred by `radius` points (a Gaussian, the radius as
    /// Skia's: AlbumWall's tracks behind the lyrics, as the Avalonia app's
    /// BlurEffect). The renderers draw the group as a picture
    /// (vergence-render's layers.rs). Bus v43.
    PushBlur {
        rect: Rect,
        radius: f32,
    },
    PopBlur,
    /// A grid of cells, `columns` to the row, rows downwards from `origin`
    /// (the first cell's top-left corner), each `cell_w` by `cell_h`. Drawn
    /// a row at a time: the grounds, neighbors of one color as a single
    /// rectangle (so no seam shows between them at a scale where a cell's
    /// edge is not on a pixel's); the underlines, one unit thick and one
    /// unit above the row's foot; then the glyphs at `size`, each with its
    /// origin at its cell's left edge, `baseline` below the row's top. A
    /// glyph wider than its cell is drawn over the next.
    Cells {
        origin: Point,
        cell_w: f32,
        cell_h: f32,
        baseline: f32,
        size: f32,
        columns: u32,
        cells: Vec<Cell>,
    },
}

/// What is wrong with a scene. `at` is the command's place in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneError {
    /// A number that is not finite, or outside its range.
    BadNumber { at: usize, what: &'static str },
    /// A font or image index with nothing behind it.
    BadIndex { at: usize, what: &'static str },
    /// More of something than a scene may hold.
    TooMany(&'static str),
    /// A font table entry that cannot be right.
    BadFont(usize),
    /// An image whose bytes do not match its size, or that is too large.
    BadImage(usize),
    /// A PopClip without a PushClip, a clip left open, or clips nested too deep.
    Clip { at: usize },
    /// The window size is not a positive, finite, bounded number.
    BadSize,
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneError::BadNumber { at, what } => write!(f, "command {at}: bad {what}"),
            SceneError::BadIndex { at, what } => write!(f, "command {at}: no such {what}"),
            SceneError::TooMany(what) => write!(f, "too many {what}"),
            SceneError::BadFont(i) => write!(f, "font {i} is not valid"),
            SceneError::BadImage(i) => write!(f, "image {i} is not valid"),
            SceneError::Clip { at } => write!(f, "command {at}: clips do not match"),
            SceneError::BadSize => write!(f, "bad scene size"),
        }
    }
}

impl std::error::Error for SceneError {}

/// A window's picture. Build one with the methods below, or read one with
/// [`Scene::decode`].
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The window's size, in reference units.
    pub width: f32,
    pub height: f32,
    pub fonts: Vec<FontRef>,
    /// Shared, not copied: a program that shows the same picture frame
    /// after frame hands the scene the same pixels each time.
    pub images: Vec<Arc<Image>>,
    pub commands: Vec<Command>,
}

impl Scene {
    pub fn new(width: f32, height: f32) -> Scene {
        Scene { width, height, fonts: Vec::new(), images: Vec::new(), commands: Vec::new() }
    }

    /// The index of this font in the scene's table, added if it is new.
    pub fn font(&mut self, family: &str, weight: u16, italic: bool) -> u32 {
        if let Some(i) = self.fonts.iter().position(|f| f.family == family && f.weight == weight && f.italic == italic)
        {
            return i as u32;
        }
        self.fonts.push(FontRef { family: family.to_string(), weight, italic });
        (self.fonts.len() - 1) as u32
    }

    /// Adds a picture (its own, or one shared with the program) and
    /// returns its index.
    pub fn image(&mut self, image: impl Into<Arc<Image>>) -> u32 {
        self.images.push(image.into());
        (self.images.len() - 1) as u32
    }

    pub fn fill(&mut self, rect: Rect, color: Color) -> &mut Self {
        self.commands.push(Command::Fill { rect, radius: 0.0, color });
        self
    }

    pub fn fill_rounded(&mut self, rect: Rect, radius: f32, color: Color) -> &mut Self {
        self.commands.push(Command::Fill { rect, radius, color });
        self
    }

    pub fn stroke(&mut self, rect: Rect, radius: f32, width: f32, color: Color) -> &mut Self {
        self.commands.push(Command::Stroke { rect, radius, width, color });
        self
    }

    pub fn line(&mut self, from: Point, to: Point, width: f32, color: Color) -> &mut Self {
        self.commands.push(Command::Line { from, to, width, color });
        self
    }

    pub fn glyphs(&mut self, font: u32, size: f32, color: Color, origin: Point, glyphs: Vec<Glyph>) -> &mut Self {
        self.commands.push(Command::Glyphs { font, size, color, origin, glyphs });
        self
    }

    pub fn draw_image(&mut self, image: u32, rect: Rect, opacity: f32) -> &mut Self {
        self.commands.push(Command::Image { image, rect, opacity });
        self
    }

    pub fn push_clip(&mut self, rect: Rect, radius: f32) -> &mut Self {
        self.commands.push(Command::PushClip { rect, radius });
        self
    }

    pub fn pop_clip(&mut self) -> &mut Self {
        self.commands.push(Command::PopClip);
        self
    }

    pub fn push_blur(&mut self, rect: Rect, radius: f32) -> &mut Self {
        self.commands.push(Command::PushBlur { rect, radius });
        self
    }

    pub fn pop_blur(&mut self) -> &mut Self {
        self.commands.push(Command::PopBlur);
        self
    }

    /// Everything drawn from command `from` on made `opacity` as opaque
    /// (its colors' alpha, its pictures' opacity): an element faded, its
    /// children with it, with nothing sent but the colors (the toolkit's
    /// `opacity`). Shapes that overlap show through each other a little,
    /// which a fade of text and fills does not show.
    pub fn fade_from(&mut self, from: usize, opacity: f32) -> &mut Self {
        let k = opacity.clamp(0.0, 1.0);
        if k >= 1.0 {
            return self;
        }
        let a = |c: &mut Color| c.a = (c.a as f32 * k).round() as u8;
        for c in self.commands.iter_mut().skip(from) {
            match c {
                Command::Fill { color, .. }
                | Command::Stroke { color, .. }
                | Command::Line { color, .. }
                | Command::Glyphs { color, .. } => a(color),
                Command::Image { opacity, .. } => *opacity *= k,
                Command::Cells { cells, .. } => {
                    for cell in cells {
                        a(&mut cell.ground);
                        a(&mut cell.ink);
                    }
                }
                Command::PushClip { .. } | Command::PopClip | Command::PushBlur { .. } | Command::PopBlur => {}
            }
        }
        self
    }

    /// Moves everything drawn so far by (dx, dy): a scene made for a part
    /// of a window put in its place (the toolkit's fallback bar above a
    /// program's scene). Glyphs and cells move with their origins.
    pub fn translate(&mut self, dx: f32, dy: f32) -> &mut Self {
        let r = |r: &mut Rect| {
            r.x += dx;
            r.y += dy;
        };
        let p = |p: &mut Point| {
            p.x += dx;
            p.y += dy;
        };
        for c in &mut self.commands {
            match c {
                Command::Fill { rect, .. }
                | Command::Stroke { rect, .. }
                | Command::Image { rect, .. }
                | Command::PushClip { rect, .. }
                | Command::PushBlur { rect, .. } => r(rect),
                Command::Line { from, to, .. } => {
                    p(from);
                    p(to);
                }
                Command::Glyphs { origin, .. } | Command::Cells { origin, .. } => p(origin),
                Command::PopClip | Command::PopBlur => {}
            }
        }
        self
    }

    /// A grid of cells ([`Command::Cells`]); `cells` holds whole rows of `columns`.
    #[allow(clippy::too_many_arguments)]
    pub fn cells(
        &mut self,
        origin: Point,
        cell_w: f32,
        cell_h: f32,
        baseline: f32,
        size: f32,
        columns: u32,
        cells: Vec<Cell>,
    ) -> &mut Self {
        self.commands.push(Command::Cells { origin, cell_w, cell_h, baseline, size, columns, cells });
        self
    }

    /// Checks everything a renderer relies on. A scene from [`Scene::decode`]
    /// has already passed; one built in the same process should be checked
    /// before it is handed on.
    pub fn validate(&self) -> Result<(), SceneError> {
        if !positive(self.width) || !positive(self.height) {
            return Err(SceneError::BadSize);
        }
        if self.fonts.len() > MAX_FONTS {
            return Err(SceneError::TooMany("fonts"));
        }
        for (i, f) in self.fonts.iter().enumerate() {
            if f.family.is_empty()
                || f.family.len() > MAX_FAMILY
                || f.weight == 0
                || f.weight > 1000
                || f.family.chars().any(char::is_control)
            {
                return Err(SceneError::BadFont(i));
            }
        }
        if self.images.len() > MAX_IMAGES {
            return Err(SceneError::TooMany("images"));
        }
        let mut image_bytes = 0usize;
        for (i, im) in self.images.iter().enumerate() {
            let want = image_len(im.width, im.height).ok_or(SceneError::BadImage(i))?;
            if im.rgba.len() != want {
                return Err(SceneError::BadImage(i));
            }
            image_bytes += want;
            if image_bytes > MAX_IMAGE_BYTES {
                return Err(SceneError::TooMany("image bytes"));
            }
        }
        if self.commands.len() > MAX_COMMANDS {
            return Err(SceneError::TooMany("commands"));
        }
        // The open clips and blurs, innermost last: each closed by its own kind.
        let mut open: Vec<bool> = Vec::new();
        let mut depth = 0usize;
        let mut blurs = 0usize;
        let mut glyph_total = 0usize;
        for (at, c) in self.commands.iter().enumerate() {
            let bad = |what| Err(SceneError::BadNumber { at, what });
            match c {
                Command::Fill { rect, radius, .. } => {
                    if !rect_ok(rect) {
                        return bad("rectangle");
                    }
                    if !length(*radius) {
                        return bad("radius");
                    }
                }
                Command::Stroke { rect, radius, width, .. } => {
                    if !rect_ok(rect) {
                        return bad("rectangle");
                    }
                    if !length(*radius) {
                        return bad("radius");
                    }
                    if !length(*width) {
                        return bad("width");
                    }
                }
                Command::Line { from, to, width, .. } => {
                    if !coord(from.x) || !coord(from.y) || !coord(to.x) || !coord(to.y) {
                        return bad("point");
                    }
                    if !length(*width) {
                        return bad("width");
                    }
                }
                Command::Glyphs { font, size, origin, glyphs, .. } => {
                    if *font as usize >= self.fonts.len() {
                        return Err(SceneError::BadIndex { at, what: "font" });
                    }
                    if !(size.is_finite() && *size > 0.0 && *size <= MAX_FONT_SIZE) {
                        return bad("font size");
                    }
                    if !coord(origin.x) || !coord(origin.y) {
                        return bad("point");
                    }
                    if glyphs.len() > MAX_RUN_GLYPHS {
                        return Err(SceneError::TooMany("glyphs in a run"));
                    }
                    glyph_total += glyphs.len();
                    if glyph_total > MAX_SCENE_GLYPHS {
                        return Err(SceneError::TooMany("glyphs"));
                    }
                    if glyphs.iter().any(|g| !coord(g.x) || !coord(g.y)) {
                        return bad("glyph position");
                    }
                }
                Command::Image { image, rect, opacity } => {
                    if *image as usize >= self.images.len() {
                        return Err(SceneError::BadIndex { at, what: "image" });
                    }
                    if !rect_ok(rect) {
                        return bad("rectangle");
                    }
                    if !(opacity.is_finite() && (0.0..=1.0).contains(opacity)) {
                        return bad("opacity");
                    }
                }
                Command::PushClip { rect, radius } => {
                    if !rect_ok(rect) {
                        return bad("rectangle");
                    }
                    if !length(*radius) {
                        return bad("radius");
                    }
                    depth += 1;
                    if depth > MAX_CLIP_DEPTH {
                        return Err(SceneError::Clip { at });
                    }
                    open.push(false);
                }
                Command::PopClip => {
                    if depth == 0 || open.pop() != Some(false) {
                        return Err(SceneError::Clip { at });
                    }
                    depth -= 1;
                }
                Command::PushBlur { rect, radius } => {
                    if !rect_ok(rect) {
                        return bad("rectangle");
                    }
                    if !(radius.is_finite() && (0.0..=MAX_BLUR).contains(radius)) {
                        return bad("blur radius");
                    }
                    blurs += 1;
                    if blurs > MAX_BLUR_DEPTH {
                        return Err(SceneError::Clip { at });
                    }
                    open.push(true);
                }
                Command::PopBlur => {
                    if open.pop() != Some(true) {
                        return Err(SceneError::Clip { at });
                    }
                    blurs -= 1;
                }
                Command::Cells { origin, cell_w, cell_h, baseline, size, columns, cells } => {
                    if !coord(origin.x) || !coord(origin.y) {
                        return bad("point");
                    }
                    if !positive(*cell_w) || !positive(*cell_h) {
                        return bad("cell size");
                    }
                    if !coord(*baseline) {
                        return bad("baseline");
                    }
                    if !(size.is_finite() && *size > 0.0 && *size <= MAX_FONT_SIZE) {
                        return bad("font size");
                    }
                    let columns = *columns as usize;
                    if columns == 0 || columns > MAX_GRID_COLUMNS {
                        return bad("columns");
                    }
                    if cells.len() % columns != 0 {
                        return bad("cells: not whole rows");
                    }
                    glyph_total += cells.len();
                    if glyph_total > MAX_SCENE_GLYPHS {
                        return Err(SceneError::TooMany("glyphs"));
                    }
                    // The far corner is a coordinate like any other.
                    let rows = cells.len() / columns;
                    if !coord(origin.x + columns as f32 * cell_w) || !coord(origin.y + rows as f32 * cell_h) {
                        return bad("grid size");
                    }
                    for cell in cells {
                        if cell.flags & CELL_GLYPH != 0 && cell.font as usize >= self.fonts.len() {
                            return Err(SceneError::BadIndex { at, what: "font" });
                        }
                        if cell.flags & !(CELL_UNDERLINE | CELL_GLYPH) != 0 {
                            return bad("cell flags");
                        }
                    }
                }
            }
        }
        if depth != 0 || !open.is_empty() {
            return Err(SceneError::Clip { at: self.commands.len() });
        }
        Ok(())
    }
}

/// The byte length of a `width` x `height` picture, if its size is allowed.
pub(crate) fn image_len(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 || width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return None;
    }
    let n = width as usize * height as usize * 4;
    (n <= MAX_IMAGE_BYTES).then_some(n)
}

fn coord(v: f32) -> bool {
    v.is_finite() && v.abs() <= MAX_COORD
}

/// A size, radius or width: finite, not negative, bounded.
fn length(v: f32) -> bool {
    v.is_finite() && (0.0..=MAX_COORD).contains(&v)
}

fn positive(v: f32) -> bool {
    v.is_finite() && v > 0.0 && v <= MAX_COORD
}

fn rect_ok(r: &Rect) -> bool {
    coord(r.x) && coord(r.y) && length(r.w) && length(r.h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_translated_scene_moves_whole() {
        let mut s = sample();
        let before = s.commands.clone();
        s.translate(0.0, 30.0);
        for (a, b) in before.iter().zip(&s.commands) {
            match (a, b) {
                (Command::Fill { rect: ra, .. }, Command::Fill { rect: rb, .. })
                | (Command::Stroke { rect: ra, .. }, Command::Stroke { rect: rb, .. })
                | (Command::PushClip { rect: ra, .. }, Command::PushClip { rect: rb, .. })
                | (Command::Image { rect: ra, .. }, Command::Image { rect: rb, .. }) => {
                    assert_eq!((rb.x, rb.y, rb.w, rb.h), (ra.x, ra.y + 30.0, ra.w, ra.h));
                }
                (Command::Line { from: fa, to: ta, .. }, Command::Line { from: fb, to: tb, .. }) => {
                    assert_eq!((fb.y, tb.y), (fa.y + 30.0, ta.y + 30.0))
                }
                (Command::Glyphs { origin: oa, glyphs: ga, .. }, Command::Glyphs { origin: ob, glyphs: gb, .. }) => {
                    assert_eq!((ob.x, ob.y), (oa.x, oa.y + 30.0));
                    assert_eq!(ga, gb, "glyphs stay where they are on their origin");
                }
                (Command::PushBlur { rect: ra, .. }, Command::PushBlur { rect: rb, .. }) => {
                    assert_eq!((rb.x, rb.y), (ra.x, ra.y + 30.0));
                }
                (Command::PopClip, Command::PopClip)
                | (Command::PopBlur, Command::PopBlur)
                | (Command::Cells { .. }, Command::Cells { .. }) => {}
                other => panic!("the commands changed kind: {other:?}"),
            }
        }
        assert!(s.validate().is_ok());
    }

    #[test]
    fn blurred_groups_nest_with_clips_or_are_refused() {
        let r = Rect::new(1.0, 1.0, 10.0, 10.0);
        let mut ok = Scene::new(20.0, 20.0);
        ok.push_blur(r, 4.0).push_clip(r, 2.0).fill(r, Color::rgb(1, 2, 3)).pop_clip().pop_blur();
        assert!(ok.validate().is_ok());
        let back = Scene::decode(&ok.encode().expect("encodes")).expect("its bytes read back");
        assert_eq!(back.commands, ok.commands);
        // A clip closed by the blur's end, a blur left open, a radius too large.
        let mut crossed = Scene::new(20.0, 20.0);
        crossed.push_blur(r, 4.0).push_clip(r, 0.0).pop_blur().pop_clip();
        assert!(crossed.validate().is_err());
        let mut open = Scene::new(20.0, 20.0);
        open.push_blur(r, 4.0);
        assert!(open.validate().is_err());
        let mut huge = Scene::new(20.0, 20.0);
        huge.push_blur(r, MAX_BLUR + 1.0).pop_blur();
        assert!(huge.validate().is_err());
    }

    #[test]
    fn a_fade_scales_alpha_from_its_start() {
        let r = Rect::new(0.0, 0.0, 4.0, 4.0);
        let mut s = Scene::new(10.0, 10.0);
        s.fill(r, Color::rgba(10, 10, 10, 200)).fill(r, Color::rgba(10, 10, 10, 200));
        s.fade_from(1, 0.25);
        let alphas: Vec<u8> = s
            .commands
            .iter()
            .filter_map(|c| if let Command::Fill { color, .. } = c { Some(color.a) } else { None })
            .collect();
        assert_eq!(alphas, [200, 50]);
    }

    fn sample() -> Scene {
        let mut s = Scene::new(640.0, 400.0);
        let mono = s.font("Noto Sans Mono", 400, false);
        let bold = s.font("Noto Sans Mono", 700, false);
        assert_eq!(s.font("Noto Sans Mono", 400, false), mono);
        let pic = s.image(Image { width: 2, height: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 128] });
        s.fill(Rect::new(0.0, 0.0, 640.0, 400.0), Color::rgb(46, 52, 54));
        s.fill_rounded(Rect::new(8.0, 4.0, 200.0, 28.0), 7.0, Color::rgba(255, 255, 255, 20));
        s.push_clip(Rect::new(0.0, 36.0, 640.0, 364.0), 0.0);
        s.glyphs(
            mono,
            13.5,
            Color::rgb(211, 215, 207),
            Point::new(6.0, 52.0),
            vec![Glyph { id: 71, x: 0.0, y: 0.0 }, Glyph { id: 72, x: 8.1, y: 0.0 }],
        );
        s.glyphs(bold, 13.5, Color::rgb(252, 233, 79), Point::new(6.0, 70.0), vec![]);
        s.draw_image(pic, Rect::new(600.0, 40.0, 32.0, 16.0), 0.5);
        s.pop_clip();
        s.stroke(Rect::new(0.5, 0.5, 639.0, 399.0), 0.0, 1.0, Color::rgb(80, 90, 110));
        s.line(Point::new(12.0, 12.0), Point::new(22.0, 22.0), 1.25, Color::rgb(235, 235, 235));
        s
    }

    #[test]
    fn a_built_scene_is_valid() {
        assert_eq!(sample().validate(), Ok(()));
    }

    #[test]
    fn numbers_must_be_finite_and_bounded() {
        for bad in [f32::NAN, f32::INFINITY, -f32::INFINITY, MAX_COORD * 2.0] {
            let mut s = sample();
            s.fill(Rect::new(bad, 0.0, 1.0, 1.0), Color::default());
            assert!(matches!(s.validate(), Err(SceneError::BadNumber { what: "rectangle", .. })), "{bad}");
            let mut s = sample();
            s.line(Point::new(0.0, 0.0), Point::new(1.0, bad), 1.0, Color::default());
            assert!(matches!(s.validate(), Err(SceneError::BadNumber { what: "point", .. })), "{bad}");
        }
        let mut s = sample();
        s.fill(Rect::new(0.0, 0.0, -1.0, 1.0), Color::default());
        assert!(s.validate().is_err(), "a negative width");
        let mut s = sample();
        s.fill_rounded(Rect::new(0.0, 0.0, 1.0, 1.0), -0.5, Color::default());
        assert!(matches!(s.validate(), Err(SceneError::BadNumber { what: "radius", .. })));
        let mut s = sample();
        s.draw_image(0, Rect::new(0.0, 0.0, 1.0, 1.0), 1.5);
        assert!(matches!(s.validate(), Err(SceneError::BadNumber { what: "opacity", .. })));
        assert_eq!(Scene::new(0.0, 10.0).validate(), Err(SceneError::BadSize));
        assert_eq!(Scene::new(10.0, f32::NAN).validate(), Err(SceneError::BadSize));
    }

    #[test]
    fn indices_must_point_at_something() {
        let mut s = sample();
        s.glyphs(2, 12.0, Color::default(), Point::default(), vec![]);
        assert!(matches!(s.validate(), Err(SceneError::BadIndex { what: "font", .. })));
        let mut s = sample();
        s.draw_image(1, Rect::new(0.0, 0.0, 1.0, 1.0), 1.0);
        assert!(matches!(s.validate(), Err(SceneError::BadIndex { what: "image", .. })));
        let mut s = sample();
        s.glyphs(0, 0.0, Color::default(), Point::default(), vec![]);
        assert!(matches!(s.validate(), Err(SceneError::BadNumber { what: "font size", .. })));
    }

    #[test]
    fn clips_must_match() {
        let mut s = sample();
        s.pop_clip();
        assert!(matches!(s.validate(), Err(SceneError::Clip { .. })));
        let mut s = sample();
        s.push_clip(Rect::new(0.0, 0.0, 1.0, 1.0), 0.0);
        assert!(matches!(s.validate(), Err(SceneError::Clip { .. })));
        let mut s = Scene::new(10.0, 10.0);
        for _ in 0..=MAX_CLIP_DEPTH {
            s.push_clip(Rect::new(0.0, 0.0, 1.0, 1.0), 0.0);
        }
        for _ in 0..=MAX_CLIP_DEPTH {
            s.pop_clip();
        }
        assert!(matches!(s.validate(), Err(SceneError::Clip { .. })), "nested too deep");
    }

    #[test]
    fn fonts_and_images_are_checked() {
        let mut s = sample();
        s.fonts.push(FontRef { family: String::new(), weight: 400, italic: false });
        assert_eq!(s.validate(), Err(SceneError::BadFont(2)));
        let mut s = sample();
        s.fonts[0].weight = 1001;
        assert_eq!(s.validate(), Err(SceneError::BadFont(0)));
        let mut s = sample();
        s.fonts[1].family = "Mono\nSans".into();
        assert_eq!(s.validate(), Err(SceneError::BadFont(1)));
        let mut s = sample();
        Arc::make_mut(&mut s.images[0]).rgba.pop();
        assert_eq!(s.validate(), Err(SceneError::BadImage(0)));
        let mut s = sample();
        s.images.push(Image { width: 0, height: 4, rgba: vec![] }.into());
        assert_eq!(s.validate(), Err(SceneError::BadImage(1)));
        let mut s = sample();
        s.images.push(Image { width: MAX_IMAGE_SIDE + 1, height: 1, rgba: vec![] }.into());
        assert_eq!(s.validate(), Err(SceneError::BadImage(1)));
    }

    #[test]
    fn a_run_cannot_hold_too_many_glyphs() {
        let mut s = sample();
        s.glyphs(0, 12.0, Color::default(), Point::default(), vec![Glyph::default(); MAX_RUN_GLYPHS + 1]);
        assert_eq!(s.validate(), Err(SceneError::TooMany("glyphs in a run")));
    }

    #[test]
    fn a_grid_of_cells_is_checked() {
        let grid = |change: &dyn Fn(&mut Command)| {
            let mut s = Scene::new(100.0, 100.0);
            let font = s.font("Mono", 400, false) as u16;
            let cells = vec![
                Cell { glyph: 5, font, flags: CELL_GLYPH, ..Cell::default() },
                Cell { flags: CELL_UNDERLINE, ..Cell::default() },
                Cell::default(),
                Cell::default(),
            ];
            s.cells(Point::new(1.0, 2.0), 8.0, 16.0, 12.0, 13.0, 2, cells);
            change(s.commands.last_mut().unwrap());
            s.validate()
        };
        assert_eq!(grid(&|_| ()), Ok(()));
        let cells_of = |c: &mut Command, f: &dyn Fn(&mut Vec<Cell>)| {
            if let Command::Cells { cells, .. } = c {
                f(cells)
            }
        };
        // A font that is not in the table matters only where there is a glyph.
        assert_eq!(grid(&|c| cells_of(c, &|cells| cells[1].font = 9)), Ok(()));
        assert!(matches!(
            grid(&|c| cells_of(c, &|cells| cells[0].font = 9)),
            Err(SceneError::BadIndex { what: "font", .. })
        ));
        assert!(matches!(
            grid(&|c| cells_of(c, &|cells| cells[2].flags = 4)),
            Err(SceneError::BadNumber { what: "cell flags", .. })
        ));
        assert!(matches!(
            grid(&|c| cells_of(c, &|cells| cells.truncate(3))),
            Err(SceneError::BadNumber { what: "cells: not whole rows", .. })
        ));
        let set = |c: &mut Command, f: &dyn Fn(&mut f32, &mut f32, &mut f32, &mut u32)| {
            if let Command::Cells { cell_w, baseline, size, columns, .. } = c {
                f(cell_w, baseline, size, columns)
            }
        };
        assert!(matches!(
            grid(&|c| set(c, &|w, _, _, _| *w = 0.0)),
            Err(SceneError::BadNumber { what: "cell size", .. })
        ));
        assert!(matches!(
            grid(&|c| set(c, &|w, _, _, _| *w = f32::NAN)),
            Err(SceneError::BadNumber { what: "cell size", .. })
        ));
        assert!(matches!(
            grid(&|c| set(c, &|_, b, _, _| *b = f32::INFINITY)),
            Err(SceneError::BadNumber { what: "baseline", .. })
        ));
        assert!(matches!(
            grid(&|c| set(c, &|_, _, s, _| *s = 0.0)),
            Err(SceneError::BadNumber { what: "font size", .. })
        ));
        assert!(matches!(grid(&|c| set(c, &|_, _, _, n| *n = 0)), Err(SceneError::BadNumber { what: "columns", .. })));
        assert!(matches!(
            grid(&|c| set(c, &|w, _, _, _| *w = MAX_COORD)),
            Err(SceneError::BadNumber { what: "grid size", .. })
        ));
    }
}
