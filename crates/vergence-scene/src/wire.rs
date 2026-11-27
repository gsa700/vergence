//! A scene as bytes: for a file, a socket, another machine.
//!
//! Little-endian throughout; `f32` as its IEEE bits. Fixed-size records, no
//! pointers, no offsets: a reader walks it once from the front.
//!
//! ```text
//! "VSCN"  u16 version (1)
//! f32 width  f32 height
//! u32 fonts,  each: u16 weight  u8 italic (0|1)  u32 length + family (UTF-8)
//! u32 images, each: u32 width  u32 height  width*height*4 bytes (RGBA)
//! u32 commands, each: u8 opcode + its fields:
//!   1 Fill      rect  f32 radius  color
//!   2 Stroke    rect  f32 radius  f32 width  color
//!   3 Line      point from  point to  f32 width  color
//!   4 Glyphs    u32 font  f32 size  color  point origin
//!               u32 count, each: u32 id  f32 x  f32 y
//!   5 Image     u32 image  rect  f32 opacity
//!   6 PushClip  rect  f32 radius
//!   7 PopClip
//!   9 PushBlur  rect  f32 radius      (bus v43)
//!  10 PopBlur
//!   8 Cells     point origin  f32 cell_w  f32 cell_h  f32 baseline  f32 size
//!               u32 columns  u32 count, each: color ground  color ink
//!               u32 glyph  u16 font  u16 flags
//! rect = f32 x y w h    point = f32 x y    color = u8 r g b a
//! ```
//!
//! **Version 2, for the native window protocol** (display lists milestone
//! 1, step 1): the same, but each image begins with a u64 key the sender
//! chose and a kind byte: 0, its pixels follow (u32 width, u32 height,
//! RGBA), and the reader keeps them under the key; 1, nothing follows: the
//! picture sent earlier under that key. So a frame names a picture it has
//! sent before (a page, an icon) instead of sending it again.
//! [`Scene::encode_keyed`] and [`Scene::decode_keyed`]; version 1 stays for
//! files and anything that keeps no pictures.
//!
//! Nothing may follow the last command. A reader that meets a version or an
//! opcode it does not know refuses the scene: skipping a drawing command
//! would show a different picture, which is worse than showing none.
//!
//! [`Scene::decode`] takes bytes from anyone. It checks each count against
//! the scene's limits and against the bytes that are actually left before it
//! reserves memory, and it returns only scenes that pass [`Scene::validate`].

use std::fmt;
use std::sync::Arc;

use crate::{
    Cell, Color, Command, FontRef, Glyph, Image, MAX_COMMANDS, MAX_FAMILY, MAX_FONTS, MAX_IMAGE_BYTES, MAX_IMAGES,
    MAX_RUN_GLYPHS, MAX_SCENE_GLYPHS, Point, Rect, Scene, SceneError, VERSION, image_len,
};

pub const MAGIC: [u8; 4] = *b"VSCN";

const OP_FILL: u8 = 1;
const OP_STROKE: u8 = 2;
const OP_LINE: u8 = 3;
const OP_GLYPHS: u8 = 4;
const OP_IMAGE: u8 = 5;
const OP_PUSH_CLIP: u8 = 6;
const OP_POP_CLIP: u8 = 7;
const OP_CELLS: u8 = 8;
const OP_PUSH_BLUR: u8 = 9;
const OP_POP_BLUR: u8 = 10;

/// The smallest a font, an image and a command can be on the wire, for
/// refusing counts the remaining bytes cannot hold.
const MIN_FONT: usize = 2 + 1 + 4;
const MIN_IMAGE: usize = 4 + 4 + 4;
const MIN_COMMAND: usize = 1;
const GLYPH: usize = 12;
const CELL: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The bytes ended before the scene did.
    Short,
    /// Not a scene.
    Magic,
    /// A version this reader does not know.
    Version(u16),
    /// A font family that is not UTF-8.
    Utf8,
    /// A count larger than a scene may hold, or than the bytes left could.
    TooMany(&'static str),
    /// A command this reader does not know.
    Opcode(u8),
    /// A flag that is neither 0 nor 1.
    Flag,
    /// Bytes after the last command.
    Trailing,
    /// It parsed, but it is not a valid scene.
    Invalid(SceneError),
    /// A picture named by a key the reader does not keep (version 2).
    UnknownImage(u64),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Short => write!(f, "scene ended early"),
            DecodeError::Magic => write!(f, "not a scene"),
            DecodeError::Version(v) => write!(f, "scene version {v} is not known"),
            DecodeError::Utf8 => write!(f, "font family is not UTF-8"),
            DecodeError::TooMany(what) => write!(f, "too many {what}"),
            DecodeError::Opcode(op) => write!(f, "unknown command {op}"),
            DecodeError::Flag => write!(f, "bad flag"),
            DecodeError::Trailing => write!(f, "bytes after the scene"),
            DecodeError::Invalid(e) => write!(f, "invalid scene: {e}"),
            DecodeError::UnknownImage(k) => write!(f, "picture {k:#x} was never sent"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<SceneError> for DecodeError {
    fn from(e: SceneError) -> Self {
        DecodeError::Invalid(e)
    }
}

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    fn rect(&mut self, r: &Rect) {
        self.f32(r.x);
        self.f32(r.y);
        self.f32(r.w);
        self.f32(r.h);
    }
    fn point(&mut self, p: &Point) {
        self.f32(p.x);
        self.f32(p.y);
    }
    fn color(&mut self, c: &Color) {
        self.0.extend_from_slice(&[c.r, c.g, c.b, c.a]);
    }
}

struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.buf.len() < n {
            return Err(DecodeError::Short);
        }
        let (head, rest) = self.buf.split_at(n);
        self.buf = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn rect(&mut self) -> Result<Rect, DecodeError> {
        Ok(Rect { x: self.f32()?, y: self.f32()?, w: self.f32()?, h: self.f32()? })
    }
    fn point(&mut self) -> Result<Point, DecodeError> {
        Ok(Point { x: self.f32()?, y: self.f32()? })
    }
    fn color(&mut self) -> Result<Color, DecodeError> {
        let b = self.take(4)?;
        Ok(Color { r: b[0], g: b[1], b: b[2], a: b[3] })
    }
    /// A count of things at least `each` bytes long: within `max`, and no
    /// more than the bytes left could hold.
    fn count(&mut self, max: usize, each: usize, what: &'static str) -> Result<usize, DecodeError> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(DecodeError::TooMany(what));
        }
        if n.checked_mul(each).is_none_or(|bytes| bytes > self.buf.len()) {
            return Err(DecodeError::Short);
        }
        Ok(n)
    }
}

impl Scene {
    /// The scene as bytes. The scene is checked first: an invalid scene is
    /// never written.
    pub fn encode(&self) -> Result<Vec<u8>, SceneError> {
        self.encode_as(None)
    }

    /// The scene as version 2 bytes: image `i` under `keys[i]`, its pixels
    /// sent only if `sent(key)` says the reader does not have them yet.
    pub fn encode_keyed(&self, keys: &[u64], sent: &dyn Fn(u64) -> bool) -> Result<Vec<u8>, SceneError> {
        if keys.len() != self.images.len() {
            return Err(SceneError::BadImage(keys.len().min(self.images.len())));
        }
        self.encode_as(Some((keys, sent)))
    }

    #[allow(clippy::type_complexity)]
    fn encode_as(&self, keyed: Option<(&[u64], &dyn Fn(u64) -> bool)>) -> Result<Vec<u8>, SceneError> {
        self.validate()?;
        let mut w = Writer(Vec::with_capacity(64 + self.commands.len() * 24));
        w.0.extend_from_slice(&MAGIC);
        w.u16(if keyed.is_some() { WIRE_VERSION } else { VERSION });
        w.f32(self.width);
        w.f32(self.height);
        w.u32(self.fonts.len() as u32);
        for f in &self.fonts {
            w.u16(f.weight);
            w.u8(f.italic as u8);
            w.u32(f.family.len() as u32);
            w.0.extend_from_slice(f.family.as_bytes());
        }
        w.u32(self.images.len() as u32);
        for (i, im) in self.images.iter().enumerate() {
            if let Some((keys, sent)) = keyed {
                w.u64(keys[i]);
                if sent(keys[i]) {
                    w.u8(1);
                    continue;
                }
                w.u8(0);
            }
            w.u32(im.width);
            w.u32(im.height);
            w.0.extend_from_slice(&im.rgba);
        }
        w.u32(self.commands.len() as u32);
        for c in &self.commands {
            match c {
                Command::Fill { rect, radius, color } => {
                    w.u8(OP_FILL);
                    w.rect(rect);
                    w.f32(*radius);
                    w.color(color);
                }
                Command::Stroke { rect, radius, width, color } => {
                    w.u8(OP_STROKE);
                    w.rect(rect);
                    w.f32(*radius);
                    w.f32(*width);
                    w.color(color);
                }
                Command::Line { from, to, width, color } => {
                    w.u8(OP_LINE);
                    w.point(from);
                    w.point(to);
                    w.f32(*width);
                    w.color(color);
                }
                Command::Glyphs { font, size, color, origin, glyphs } => {
                    w.u8(OP_GLYPHS);
                    w.u32(*font);
                    w.f32(*size);
                    w.color(color);
                    w.point(origin);
                    w.u32(glyphs.len() as u32);
                    for g in glyphs {
                        w.u32(g.id);
                        w.f32(g.x);
                        w.f32(g.y);
                    }
                }
                Command::Image { image, rect, opacity } => {
                    w.u8(OP_IMAGE);
                    w.u32(*image);
                    w.rect(rect);
                    w.f32(*opacity);
                }
                Command::PushClip { rect, radius } => {
                    w.u8(OP_PUSH_CLIP);
                    w.rect(rect);
                    w.f32(*radius);
                }
                Command::PopClip => w.u8(OP_POP_CLIP),
                Command::PushBlur { rect, radius } => {
                    w.u8(OP_PUSH_BLUR);
                    w.rect(rect);
                    w.f32(*radius);
                }
                Command::PopBlur => w.u8(OP_POP_BLUR),
                Command::Cells { origin, cell_w, cell_h, baseline, size, columns, cells } => {
                    w.u8(OP_CELLS);
                    w.point(origin);
                    w.f32(*cell_w);
                    w.f32(*cell_h);
                    w.f32(*baseline);
                    w.f32(*size);
                    w.u32(*columns);
                    w.u32(cells.len() as u32);
                    w.0.reserve(cells.len() * CELL);
                    for cell in cells {
                        w.color(&cell.ground);
                        w.color(&cell.ink);
                        w.u32(cell.glyph);
                        w.u16(cell.font);
                        w.u16(cell.flags);
                    }
                }
            }
        }
        Ok(w.0)
    }

    /// Reads a scene from bytes that may come from anyone. Never panics, and
    /// never reserves more memory than the bytes given could fill.
    pub fn decode(bytes: &[u8]) -> Result<Scene, DecodeError> {
        Self::decode_as(bytes, None).map(|d| d.scene)
    }

    /// Reads version 2 bytes: pictures named by key come from `kept`; the
    /// pictures sent whole come back with their keys, for the reader to keep.
    pub fn decode_keyed(bytes: &[u8], kept: &dyn Fn(u64) -> Option<Arc<Image>>) -> Result<Keyed, DecodeError> {
        Self::decode_as(bytes, Some(kept))
    }

    #[allow(clippy::type_complexity)]
    fn decode_as(bytes: &[u8], kept: Option<&dyn Fn(u64) -> Option<Arc<Image>>>) -> Result<Keyed, DecodeError> {
        let mut r = Reader { buf: bytes };
        if r.take(4)? != MAGIC {
            return Err(DecodeError::Magic);
        }
        let version = r.u16()?;
        let want = if kept.is_some() { WIRE_VERSION } else { VERSION };
        if version != want {
            return Err(DecodeError::Version(version));
        }
        let width = r.f32()?;
        let height = r.f32()?;

        let n = r.count(MAX_FONTS, MIN_FONT, "fonts")?;
        let mut fonts = Vec::with_capacity(n);
        for _ in 0..n {
            let weight = r.u16()?;
            let italic = match r.u8()? {
                0 => false,
                1 => true,
                _ => return Err(DecodeError::Flag),
            };
            let len = r.count(MAX_FAMILY, 1, "bytes in a font family")?;
            let family = std::str::from_utf8(r.take(len)?).map_err(|_| DecodeError::Utf8)?.to_string();
            fonts.push(FontRef { family, weight, italic });
        }

        // A kept picture's entry is 9 bytes (key and kind), a sent one more.
        let n = r.count(MAX_IMAGES, if kept.is_some() { 9 } else { MIN_IMAGE }, "images")?;
        let mut images = Vec::with_capacity(n);
        let (mut keys, mut new) = (Vec::new(), Vec::new());
        let mut image_bytes = 0usize;
        for i in 0..n {
            if let Some(kept) = kept {
                let key = r.u64()?;
                keys.push(key);
                match r.u8()? {
                    0 => {}
                    1 => {
                        images.push(kept(key).ok_or(DecodeError::UnknownImage(key))?);
                        continue;
                    }
                    _ => return Err(DecodeError::Flag),
                }
            }
            let width = r.u32()?;
            let height = r.u32()?;
            let len = image_len(width, height).ok_or(DecodeError::Invalid(SceneError::BadImage(i)))?;
            image_bytes += len;
            if image_bytes > MAX_IMAGE_BYTES {
                return Err(DecodeError::TooMany("image bytes"));
            }
            let im: Arc<Image> = Image { width, height, rgba: r.take(len)?.to_vec() }.into();
            if let Some(&key) = keys.last().filter(|_| kept.is_some()) {
                new.push((key, im.clone()));
            }
            images.push(im);
        }

        let n = r.count(MAX_COMMANDS, MIN_COMMAND, "commands")?;
        let mut commands = Vec::with_capacity(n);
        let mut glyph_total = 0usize;
        for _ in 0..n {
            let c = match r.u8()? {
                OP_FILL => Command::Fill { rect: r.rect()?, radius: r.f32()?, color: r.color()? },
                OP_STROKE => Command::Stroke { rect: r.rect()?, radius: r.f32()?, width: r.f32()?, color: r.color()? },
                OP_LINE => Command::Line { from: r.point()?, to: r.point()?, width: r.f32()?, color: r.color()? },
                OP_GLYPHS => {
                    let font = r.u32()?;
                    let size = r.f32()?;
                    let color = r.color()?;
                    let origin = r.point()?;
                    let count = r.count(MAX_RUN_GLYPHS, GLYPH, "glyphs in a run")?;
                    glyph_total += count;
                    if glyph_total > MAX_SCENE_GLYPHS {
                        return Err(DecodeError::TooMany("glyphs"));
                    }
                    let mut glyphs = Vec::with_capacity(count);
                    for _ in 0..count {
                        glyphs.push(Glyph { id: r.u32()?, x: r.f32()?, y: r.f32()? });
                    }
                    Command::Glyphs { font, size, color, origin, glyphs }
                }
                OP_IMAGE => Command::Image { image: r.u32()?, rect: r.rect()?, opacity: r.f32()? },
                OP_PUSH_CLIP => Command::PushClip { rect: r.rect()?, radius: r.f32()? },
                OP_POP_CLIP => Command::PopClip,
                OP_PUSH_BLUR => Command::PushBlur { rect: r.rect()?, radius: r.f32()? },
                OP_POP_BLUR => Command::PopBlur,
                OP_CELLS => {
                    let origin = r.point()?;
                    let (cell_w, cell_h, baseline, size) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?);
                    let columns = r.u32()?;
                    // Cells count with glyphs against the scene's limit.
                    let count = r.count(MAX_SCENE_GLYPHS, CELL, "cells")?;
                    glyph_total += count;
                    if glyph_total > MAX_SCENE_GLYPHS {
                        return Err(DecodeError::TooMany("glyphs"));
                    }
                    let mut cells = Vec::with_capacity(count);
                    for _ in 0..count {
                        cells.push(Cell {
                            ground: r.color()?,
                            ink: r.color()?,
                            glyph: r.u32()?,
                            font: r.u16()?,
                            flags: r.u16()?,
                        });
                    }
                    Command::Cells { origin, cell_w, cell_h, baseline, size, columns, cells }
                }
                op => return Err(DecodeError::Opcode(op)),
            };
            commands.push(c);
        }
        if !r.buf.is_empty() {
            return Err(DecodeError::Trailing);
        }
        let scene = Scene { width, height, fonts, images, commands };
        scene.validate()?;
        Ok(Keyed { scene, keys, new })
    }
}

/// A version 2 scene read: the scene, each image's key (in the scene's
/// order), and the pictures sent whole this time, to be kept by key.
#[derive(Debug)]
pub struct Keyed {
    pub scene: Scene,
    pub keys: Vec<u64>,
    pub new: Vec<(u64, Arc<Image>)>,
}

/// The version of the keyed wire form (pictures kept by key).
pub const WIRE_VERSION: u16 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Scene {
        let mut s = Scene::new(640.0, 400.0);
        let mono = s.font("Noto Sans Mono", 400, false);
        let slanted = s.font("Liberation Mono", 700, true);
        let pic = s.image(Image { width: 2, height: 2, rgba: (0..16).collect() });
        s.fill(Rect::new(0.0, 0.0, 640.0, 400.0), Color::rgb(46, 52, 54));
        s.push_clip(Rect::new(0.0, 36.0, 640.0, 364.0), 4.0);
        s.glyphs(
            mono,
            13.5,
            Color::rgb(211, 215, 207),
            Point::new(6.0, 52.0),
            vec![Glyph { id: 71, x: 0.0, y: 0.0 }, Glyph { id: 72, x: 8.1, y: -0.25 }],
        );
        s.glyphs(slanted, 9.0, Color::rgba(252, 233, 79, 200), Point::new(6.0, 70.0), vec![]);
        s.draw_image(pic, Rect::new(600.0, 40.0, 32.0, 16.0), 0.5);
        s.pop_clip();
        s.stroke(Rect::new(0.5, 0.5, 639.0, 399.0), 2.0, 1.0, Color::rgb(80, 90, 110));
        s.line(Point::new(12.0, 12.0), Point::new(22.0, 22.0), 1.25, Color::rgb(235, 235, 235));
        let cells = vec![
            Cell {
                ground: Color::rgb(1, 2, 3),
                ink: Color::rgb(250, 251, 252),
                glyph: 40,
                font: mono as u16,
                flags: crate::CELL_GLYPH,
            },
            Cell {
                ground: Color::rgba(0, 0, 0, 0),
                ink: Color::rgb(9, 8, 7),
                glyph: 0,
                font: 7,
                flags: crate::CELL_UNDERLINE,
            },
            Cell {
                ground: Color::rgb(4, 5, 6),
                ink: Color::rgba(1, 1, 1, 153),
                glyph: 41,
                font: slanted as u16,
                flags: crate::CELL_UNDERLINE | crate::CELL_GLYPH,
            },
            Cell::default(),
        ];
        s.cells(Point::new(4.0, 40.0), 8.25, 17.0, 13.0, 13.5, 2, cells);
        s
    }

    #[test]
    fn a_scene_survives_the_wire() {
        let s = sample();
        let bytes = s.encode().unwrap();
        assert_eq!(&bytes[..4], b"VSCN");
        assert_eq!(Scene::decode(&bytes), Ok(s));
    }

    #[test]
    fn pictures_sent_once_then_named_by_key() {
        use std::collections::HashMap;
        let s = sample();
        let mut kept: HashMap<u64, Arc<Image>> = HashMap::new();
        // First: the picture goes whole, and is kept.
        let first = s.encode_keyed(&[0xA1], &|k| kept.contains_key(&k)).unwrap();
        assert_eq!(u16::from_le_bytes([first[4], first[5]]), WIRE_VERSION);
        let d = Scene::decode_keyed(&first, &|k| kept.get(&k).cloned()).unwrap();
        assert_eq!(d.scene, s);
        assert_eq!(d.keys, vec![0xA1]);
        assert_eq!(d.new.len(), 1);
        kept.extend(d.new);
        // Then: named only, the frame smaller by its pixels.
        let second = s.encode_keyed(&[0xA1], &|k| kept.contains_key(&k)).unwrap();
        assert_eq!(first.len() - second.len(), 4 + 4 + 16);
        let d = Scene::decode_keyed(&second, &|k| kept.get(&k).cloned()).unwrap();
        assert_eq!(d.scene, s, "the same scene, the picture from what was kept");
        assert!(d.new.is_empty());
        // A key never sent is refused, not guessed.
        let empty: HashMap<u64, Arc<Image>> = HashMap::new();
        assert_eq!(
            Scene::decode_keyed(&second, &|k| empty.get(&k).cloned()).err(),
            Some(DecodeError::UnknownImage(0xA1))
        );
        // Versions do not mix: a keyed scene is not a file's, nor the other way.
        assert_eq!(Scene::decode(&second).err(), Some(DecodeError::Version(WIRE_VERSION)));
        assert_eq!(Scene::decode_keyed(&s.encode().unwrap(), &|_| None).err(), Some(DecodeError::Version(VERSION)));
        assert!(s.encode_keyed(&[], &|_| false).is_err(), "a key for every picture");
    }

    #[test]
    fn keyed_scenes_from_anyone_never_panic() {
        let s = sample();
        let bytes = s.encode_keyed(&[7], &|_| false).unwrap();
        for cut in 0..bytes.len() {
            let _ = Scene::decode_keyed(&bytes[..cut], &|_| None);
        }
        let mut x = 0x2545_f491u32;
        for _ in 0..20_000 {
            let mut b = bytes.clone();
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let i = x as usize % b.len();
            b[i] ^= (x >> 8) as u8;
            let _ = Scene::decode_keyed(&b, &|_| Some(Arc::new(Image { width: 1, height: 1, rgba: vec![0; 4] })));
        }
    }

    #[test]
    fn an_empty_scene_is_a_scene() {
        let s = Scene::new(1.0, 1.0);
        let bytes = s.encode().unwrap();
        assert_eq!(bytes.len(), 4 + 2 + 8 + 4 + 4 + 4);
        assert_eq!(Scene::decode(&bytes), Ok(s));
    }

    #[test]
    fn an_invalid_scene_is_not_written() {
        let mut s = sample();
        s.pop_clip();
        assert!(matches!(s.encode(), Err(SceneError::Clip { .. })));
    }

    #[test]
    fn cut_short_anywhere_is_an_error_not_a_panic() {
        let bytes = sample().encode().unwrap();
        for n in 0..bytes.len() {
            assert!(Scene::decode(&bytes[..n]).is_err(), "cut at {n} of {}", bytes.len());
        }
    }

    #[test]
    fn trailing_bytes_are_refused() {
        let mut bytes = sample().encode().unwrap();
        bytes.push(0);
        assert_eq!(Scene::decode(&bytes), Err(DecodeError::Trailing));
    }

    #[test]
    fn wrong_magic_version_and_opcode() {
        let good = sample().encode().unwrap();
        let mut b = good.clone();
        b[0] = b'X';
        assert_eq!(Scene::decode(&b), Err(DecodeError::Magic));
        let mut b = good.clone();
        b[4] = 2;
        assert_eq!(Scene::decode(&b), Err(DecodeError::Version(2)));
        // An empty scene with one command of an unknown kind.
        let mut b = Scene::new(1.0, 1.0).encode().unwrap();
        let n = b.len();
        b[n - 4] = 1;
        b.push(200);
        assert_eq!(Scene::decode(&b), Err(DecodeError::Opcode(200)));
    }

    #[test]
    fn a_lying_count_reserves_nothing() {
        // "A million commands" in a scene with no bytes behind the claim.
        let mut b = Scene::new(1.0, 1.0).encode().unwrap();
        let n = b.len();
        b[n - 4..].copy_from_slice(&(MAX_COMMANDS as u32).to_le_bytes());
        assert_eq!(Scene::decode(&b), Err(DecodeError::Short));
        // More commands than a scene may hold at all.
        b[n - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(Scene::decode(&b), Err(DecodeError::TooMany("commands")));
        // A 16384 x 16384 image with no pixels behind it.
        let mut b = Vec::new();
        b.extend_from_slice(&MAGIC);
        b.extend_from_slice(&VERSION.to_le_bytes());
        b.extend_from_slice(&1.0f32.to_bits().to_le_bytes());
        b.extend_from_slice(&1.0f32.to_bits().to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&16_384u32.to_le_bytes());
        b.extend_from_slice(&16_384u32.to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        assert!(Scene::decode(&b).is_err());
    }

    #[test]
    fn what_decodes_is_valid() {
        // A scene that parses but breaks a rule (a font index with no font).
        let mut s = Scene::new(10.0, 10.0);
        s.font("Sans", 400, false);
        s.glyphs(0, 12.0, Color::default(), Point::default(), vec![]);
        let mut bytes = s.encode().unwrap();
        // The font index is the first field of the only command.
        let at = bytes.len() - (4 + 4 + 4 + 8 + 4);
        bytes[at] = 9;
        assert!(matches!(Scene::decode(&bytes), Err(DecodeError::Invalid(SceneError::BadIndex { what: "font", .. }))));
    }
}
