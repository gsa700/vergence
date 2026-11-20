//! Text for the toolkit and the server (whitepaper §6.10; toolkit milestone 1,
//! step 1): which font, which glyph, where it goes, and what it looks like.
//!
//! [`Fonts`] is the whole of it:
//!
//! - [`Fonts::face`] finds a face by family (or "monospace", "sans-serif",
//!   "serif"), weight and slant among the fonts on this machine (db.rs). A
//!   variable font is set to the weight asked for.
//! - [`Fonts::glyph`] turns a character into a glyph of that face, or of the
//!   nearest face that has the character (fallback), remembered per character.
//! - [`Fonts::layout`] places a line of text: runs of glyphs, one run per
//!   face, with kerning (the font's `GPOS` kerning, read by kern.rs, else
//!   its legacy `kern` table). Sizes and positions are in whatever unit the size
//!   was given in (reference units for a scene).
//! - [`Fonts::raster`] is the glyph cache: a glyph's coverage as a bitmap for
//!   one size in pixels and one of four sub-pixel positions. **Color is not
//!   part of the key**, so a glyph is rasterized once however many colors
//!   it is drawn in; the renderer tints the coverage.
//!
//! Plain text only: left to right, one character one glyph, combining marks
//! as their fonts place them (zero advance, drawn over the glyph before).
//! Shaping for scripts that need it, right-to-left text and input methods are
//! open (whitepaper §10, question 8). Outlines are not hinted; small text
//! has its x-height put on a whole pixel and its strokes darkened instead
//! (hint.rs, text milestone 1).
//!
//! Glyph outlines are read and rasterized by ab_glyph (borrowed, whitepaper
//! §8). The font files' own descriptions are read by sfnt.rs, which is ours
//! and treats every file as untrusted.

pub mod db;
pub mod hint;
pub mod kern;
pub mod sfnt;

#[cfg(test)]
mod fuzz;

use std::{collections::HashMap, sync::Arc};

use ab_glyph::{Font, FontVec, GlyphId, PxScale, VariableFont, point};

use crate::db::FontDb;

/// The largest font file loaded, the largest em in pixels a glyph is
/// rasterized at, the largest bitmap, and when the cache is emptied.
const MAX_FONT_FILE: u64 = 64 << 20;
const MAX_EM_PX: f32 = 8192.0;
const MAX_BITMAP: usize = 16 << 20;
const MAX_CACHE_BYTES: usize = 64 << 20;

/// A face in use: an index into this [`Fonts`]' loaded faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FaceId(pub u32);

/// A face's name, as a scene carries it (`vergence_scene::FontRef`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontName {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
}

/// A face's vertical measures at one size, in the size's unit: how far the
/// tallest glyphs rise above the baseline, how far the deepest fall below it,
/// and the extra space the font asks for between lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
}

/// A glyph and where its origin sits on the line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub glyph: u32,
    pub x: f32,
    pub y: f32,
}

/// Glyphs of one face, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub face: FaceId,
    pub glyphs: Vec<Placed>,
}

/// A glyph's coverage: `width` x `height` values, 0 (nothing) to 255
/// (covered), rows from the top. Its top-left pixel goes at `left`, `top`
/// from the glyph's origin on the baseline (`top` is negative above it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub alpha: Vec<u8>,
}

struct Loaded {
    /// The face's place in the database.
    entry: usize,
    /// The weight it is drawn at (a variable font's setting, else its own).
    weight: u16,
    font: FontVec,
    units: f32,
    /// Its `GPOS` kerning at its own weight (None: the legacy `kern` table, if any).
    kerning: Option<kern::Kerning>,
    /// Its x-height in font units (OS/2 sxHeight; 0 if unknown), for
    /// hint.rs.
    x_height: f32,
    /// Its optical size axis (`opsz`: least, default, most), if it has
    /// one, and the setting this instance is drawn at (0 for the default).
    opsz_axis: Option<(f32, f32, f32)>,
    opsz: u16,
}

impl Loaded {
    /// The kerning between two glyphs, in font units.
    fn kern(&self, a: u32, b: u32) -> f32 {
        match &self.kerning {
            Some(k) => k.pair(glyph_id(a).0, glyph_id(b).0),
            None => self.font.kern_unscaled(glyph_id(a), glyph_id(b)),
        }
    }
}

/// How many glyphs the cache's quick front holds (a power of two).
const FRONT: usize = 2048;

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
struct GlyphKey {
    face: FaceId,
    glyph: u32,
    /// The em in 1/64 pixel.
    size: u32,
    sub_x: u8,
}

pub struct Fonts {
    db: FontDb,
    loaded: Vec<Loaded>,
    /// Database entry, weight and optical size (0: the default) -> the
    /// loaded face (None: it would not load).
    by_entry: HashMap<(usize, u16, u16), Option<FaceId>>,
    /// The face and glyph that stand in for a character a face lacks.
    fallbacks: HashMap<(FaceId, char), (FaceId, u32)>,
    cache: HashMap<GlyphKey, Option<Arc<Bitmap>>>,
    /// The glyphs asked for most lately, each at the one place its key
    /// maps to: a frame of text asks for the same few hundred again and
    /// again, and finds them here without the cache's own, slower, hash
    /// (which stays as it is: its keys come from scenes, which may come
    /// from a program that means harm, and it does not degrade under
    /// chosen keys). A miss here costs nothing but that lookup.
    front: Vec<Option<(GlyphKey, Option<Arc<Bitmap>>)>>,
    cache_bytes: usize,
}

impl Fonts {
    /// With the fonts installed on this machine.
    pub fn system() -> Fonts {
        Fonts::new(FontDb::system())
    }

    pub fn new(db: FontDb) -> Fonts {
        Fonts { db, loaded: Vec::new(), by_entry: HashMap::new(), fallbacks: HashMap::new(), cache: HashMap::new(), front: vec![None; FRONT], cache_bytes: 0 }
    }

    pub fn db(&self) -> &FontDb {
        &self.db
    }

    /// The face for a family (or a generic name) at a weight and slant, the
    /// nearest the machine has. None if there is no such family, or no font
    /// at all.
    pub fn face(&mut self, family: &str, weight: u16, italic: bool) -> Option<FaceId> {
        let entry = self.db.find(family, weight, italic)?;
        self.load(entry, weight)
    }

    fn load(&mut self, entry: usize, weight: u16) -> Option<FaceId> {
        self.load_at(entry, weight, 0)
    }

    fn load_at(&mut self, entry: usize, weight: u16, opsz: u16) -> Option<FaceId> {
        let info = &self.db.faces.get(entry)?.info;
        let weight = match info.weight_axis {
            Some((min, max)) => weight.clamp(min, max),
            None => info.weight,
        };
        if let Some(known) = self.by_entry.get(&(entry, weight, opsz)) {
            return *known;
        }
        let face = self.read(entry, weight, opsz).map(|l| {
            self.loaded.push(l);
            FaceId(self.loaded.len() as u32 - 1)
        });
        self.by_entry.insert((entry, weight, opsz), face);
        face
    }

    fn read(&self, entry: usize, weight: u16, opsz: u16) -> Option<Loaded> {
        let e = &self.db.faces[entry];
        if std::fs::metadata(&e.path).ok()?.len() > MAX_FONT_FILE {
            return None;
        }
        let bytes = std::fs::read(&e.path).ok()?;
        let opsz_axis = kern::axis(&bytes, e.info.index, b"opsz");
        let mut settings: Vec<([u8; 4], f32)> = Vec::new();
        if e.info.weight_axis.is_some() {
            settings.push((*b"wght", weight as f32));
        }
        if opsz != 0 && opsz_axis.is_some() {
            settings.push((*b"opsz", opsz as f32));
        }
        let kerning = kern::Kerning::read(&bytes, e.info.index, &kern::coordinates(&bytes, e.info.index, &settings));
        // OS/2 version 2 and later carry sxHeight at 86.
        let x_height = kern::table(&bytes, e.info.index, b"OS/2")
            .filter(|t| t.len() >= 88 && u16::from_be_bytes([t[0], t[1]]) >= 2)
            .map_or(0.0, |t| i16::from_be_bytes([t[86], t[87]]).max(0) as f32);
        let mut font = FontVec::try_from_vec_and_index(bytes, e.info.index).ok()?;
        for (tag, value) in &settings {
            font.set_variation(tag, *value);
        }
        let units = font.units_per_em().filter(|u| *u > 0.0)?;
        Some(Loaded { entry, weight, font, units, kerning, x_height, opsz_axis, opsz: if opsz_axis.is_some() { opsz } else { 0 } })
    }

    /// The instance of `face` for text `size` units tall (text milestone 1,
    /// step 4): a variable font with an optical size axis set to the size
    /// (whole units, within the axis), as browsers do
    /// (`font-optical-sizing: auto`), so Inter's headings take its display
    /// cut. The face itself when it has no such axis, or the size is its
    /// default. Layout and the renderers both ask for it, so they agree;
    /// scenes keep naming the face (Fonts::name is the same).
    pub fn sized(&mut self, face: FaceId, size: f32) -> FaceId {
        let Some(l) = self.loaded.get(face.0 as usize) else { return face };
        let Some((min, default, max)) = l.opsz_axis else { return face };
        if !size.is_finite() {
            return face;
        }
        let want = size.round().clamp(min, max);
        let opsz = if (want - default).abs() < 0.5 { 0 } else { want as u16 };
        if opsz == l.opsz {
            return face;
        }
        let (entry, weight) = (l.entry, l.weight);
        self.load_at(entry, weight, opsz).unwrap_or(face)
    }

    /// The face's name, for a scene's font table.
    pub fn name(&self, face: FaceId) -> FontName {
        let l = &self.loaded[face.0 as usize];
        let info = &self.db.faces[l.entry].info;
        FontName { family: info.family.clone(), weight: l.weight, italic: info.italic }
    }

    pub fn is_monospace(&self, face: FaceId) -> bool {
        self.db.faces[self.loaded[face.0 as usize].entry].info.monospace
    }

    pub fn metrics(&self, face: FaceId, size: f32) -> Metrics {
        let l = &self.loaded[face.0 as usize];
        let k = size / l.units;
        Metrics { ascent: l.font.ascent_unscaled() * k, descent: -l.font.descent_unscaled() * k, line_gap: l.font.line_gap_unscaled() * k }
    }

    /// The glyph for `c` in `face`, or in the nearest face that has it. If
    /// no font on the machine has it, the face's own "missing" glyph (0).
    pub fn glyph(&mut self, face: FaceId, c: char) -> (FaceId, u32) {
        let own = self.loaded[face.0 as usize].font.glyph_id(c).0;
        if own != 0 {
            return (face, own as u32);
        }
        if let Some(known) = self.fallbacks.get(&(face, c)) {
            return *known;
        }
        let (entry, weight) = (self.loaded[face.0 as usize].entry, self.loaded[face.0 as usize].weight);
        let found = self
            .db
            .fallback(c, entry)
            .and_then(|other| self.load(other, weight))
            .map(|f| (f, self.loaded[f.0 as usize].font.glyph_id(c).0 as u32))
            // A face may list a character and still have no glyph for it.
            .filter(|(_, g)| *g != 0)
            .unwrap_or((face, 0));
        self.fallbacks.insert((face, c), found);
        found
    }

    /// How far the line moves on after this glyph, at `size`.
    pub fn advance(&mut self, face: FaceId, glyph: u32, size: f32) -> f32 {
        let face = self.sized(face, size);
        let l = &self.loaded[face.0 as usize];
        l.font.h_advance_unscaled(glyph_id(glyph)) * size / l.units
    }

    /// One line of text set in `face` at `size`: its runs, and its width.
    /// Control characters are left out.
    pub fn layout(&mut self, face: FaceId, size: f32, text: &str) -> (Vec<Run>, f32) {
        let mut runs: Vec<Run> = Vec::new();
        let mut pen = 0.0f32;
        let mut prev: Option<(FaceId, u32)> = None;
        for c in text.chars().filter(|c| !c.is_control()) {
            let (f, g) = self.glyph(face, c);
            // Measured in the instance drawn for this size (optical sizes);
            // the run names the face itself.
            let at = self.sized(f, size);
            let l = &self.loaded[at.0 as usize];
            if let Some((pf, pg)) = prev
                && pf == f
            {
                pen += l.kern(pg, g) * size / l.units;
            }
            match runs.last_mut() {
                Some(run) if run.face == f => run.glyphs.push(Placed { glyph: g, x: pen, y: 0.0 }),
                _ => runs.push(Run { face: f, glyphs: vec![Placed { glyph: g, x: pen, y: 0.0 }] }),
            }
            pen += l.font.h_advance_unscaled(glyph_id(g)) * size / l.units;
            prev = Some((f, g));
        }
        (runs, pen)
    }

    /// The glyph's coverage with the em `em_px` pixels tall, its origin
    /// `sub_x` quarter-pixels right of a whole pixel (0 to 3). None for a
    /// glyph with nothing to draw (a space), or a size that makes no sense.
    /// Rasterized once and kept; the bitmap is shared, not copied, so a
    /// renderer can hold on to it across threads.
    pub fn raster(&mut self, face: FaceId, glyph: u32, em_px: f32, sub_x: u8) -> Option<Arc<Bitmap>> {
        if !(em_px.is_finite() && em_px > 0.0 && em_px <= MAX_EM_PX) {
            return None;
        }
        let key = GlyphKey { face, glyph, size: (em_px * 64.0).round() as u32, sub_x: sub_x & 3 };
        let slot = (glyph.wrapping_mul(0x9e37_79b1) ^ key.size.wrapping_mul(0x85eb_ca6b) ^ ((face.0 as u32) << 7) ^ key.sub_x as u32) as usize >> 3 & (FRONT - 1);
        if let Some((k, bitmap)) = &self.front[slot]
            && *k == key
        {
            return bitmap.clone();
        }
        let bitmap = match self.cache.get(&key) {
            Some(known) => known.clone(),
            None => {
                let bitmap = self.rasterize(face, glyph, key.size as f32 / 64.0, key.sub_x).map(Arc::new);
                if self.cache_bytes > MAX_CACHE_BYTES {
                    self.cache.clear();
                    self.front.fill(None);
                    self.cache_bytes = 0;
                }
                self.cache_bytes += bitmap.as_ref().map_or(0, |b| b.alpha.len()) + 64;
                self.cache.insert(key, bitmap.clone());
                bitmap
            }
        };
        self.front[slot] = Some((key, bitmap.clone()));
        bitmap
    }

    fn rasterize(&self, face: FaceId, glyph: u32, em_px: f32, sub_x: u8) -> Option<Bitmap> {
        let l = self.loaded.get(face.0 as usize)?;
        // ab_glyph's scale is the height from ascent to descent, not the em.
        let s = em_px * l.font.height_unscaled() / l.units;
        // Small text: the x-height on a whole pixel, strokes a little wider (hint.rs).
        let ky = hint::x_height_scale(l.x_height * em_px / l.units, em_px);
        let scale = PxScale { x: s, y: s * ky };
        let outline = l.font.outline_glyph(glyph_id(glyph).with_scale_and_position(scale, point(sub_x as f32 / 4.0, 0.0)))?;
        let b = outline.px_bounds();
        let (w, h) = ((b.max.x - b.min.x).max(0.0) as usize, (b.max.y - b.min.y).max(0.0) as usize);
        if w == 0 || h == 0 || w.checked_mul(h)? > MAX_BITMAP {
            return None;
        }
        let mut alpha = vec![0u8; w * h];
        outline.draw(|x, y, cov| {
            if let Some(p) = alpha.get_mut(y as usize * w + x as usize) {
                *p = (cov.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        });
        let (alpha, w) = hint::dilate(&alpha, w, hint::darkening(em_px));
        Some(Bitmap { width: w as u32, height: h as u32, left: b.min.x as i32, top: b.min.y as i32, alpha })
    }

    /// How many glyph bitmaps are kept (tests, and curiosity).
    pub fn cached(&self) -> usize {
        self.cache.len()
    }
}

/// A scene's glyph number as ab_glyph's (fonts have at most 65,535 glyphs;
/// anything larger is the "missing" glyph).
fn glyph_id(glyph: u32) -> GlyphId {
    GlyphId(u16::try_from(glyph).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The machine's fonts and its monospace face. The development machines
    /// all have fonts; a machine with none cannot run these.
    fn mono() -> (Fonts, FaceId) {
        let mut fonts = Fonts::system();
        assert!(!fonts.db().faces.is_empty(), "no font files found on this machine");
        let face = fonts.face("monospace", 400, false).expect("a monospace face");
        (fonts, face)
    }

    #[test]
    fn the_machine_has_a_monospace_face() {
        let (mut fonts, face) = mono();
        assert!(fonts.is_monospace(face), "{:?}", fonts.name(face));
        let (f, a) = fonts.glyph(face, 'A');
        assert_eq!(f, face);
        assert_ne!(a, 0);
        let (_, i) = fonts.glyph(face, 'i');
        let (_, w) = fonts.glyph(face, 'W');
        assert_eq!(fonts.advance(face, i, 16.0), fonts.advance(face, w, 16.0));
        assert!(fonts.advance(face, a, 16.0) > 4.0 && fonts.advance(face, a, 16.0) < 16.0);
        let name = fonts.name(face);
        assert!(!name.family.is_empty() && !name.italic);
    }

    #[test]
    fn metrics_are_in_proportion() {
        let (fonts, face) = mono();
        let m = fonts.metrics(face, 20.0);
        assert!(m.ascent > 10.0 && m.ascent < 30.0, "{m:?}");
        assert!(m.descent > 1.0 && m.descent < 15.0, "{m:?}");
        assert!(m.line_gap >= 0.0);
        let half = fonts.metrics(face, 10.0);
        assert!((half.ascent * 2.0 - m.ascent).abs() < 0.001);
    }

    #[test]
    fn a_glyph_is_rasterized_once_whatever_draws_it() {
        let (mut fonts, face) = mono();
        let (_, a) = fonts.glyph(face, 'A');
        let first = fonts.raster(face, a, 48.0, 0).expect("A has an outline");
        assert!(first.width > 8 && first.width < 64 && first.height > 16 && first.height < 64, "{} x {}", first.width, first.height);
        assert_eq!(first.alpha.len(), (first.width * first.height) as usize);
        assert!(first.alpha.iter().any(|v| *v > 200), "some of it is solid");
        assert!(first.alpha.iter().any(|v| *v == 0), "some of it is empty");
        assert!(first.top < 0 && first.top > -64, "it stands on the baseline: top {}", first.top);
        assert_eq!(fonts.cached(), 1);
        assert!(fonts.raster(face, a, 48.0, 0).is_some_and(|again| Arc::ptr_eq(&again, &first)), "the same bitmap, not a copy");
        assert_eq!(fonts.cached(), 1, "the second request was answered from the cache");
        // Another sub-pixel position and another size are other bitmaps.
        assert!(fonts.raster(face, a, 48.0, 2).is_some());
        assert!(fonts.raster(face, a, 16.0, 0).is_some_and(|b| b.height < first.height));
        assert_eq!(fonts.cached(), 3);
    }

    #[test]
    fn a_space_moves_the_line_and_draws_nothing() {
        let (mut fonts, face) = mono();
        let (f, space) = fonts.glyph(face, ' ');
        assert!(fonts.advance(f, space, 16.0) > 0.0);
        assert_eq!(fonts.raster(f, space, 16.0, 0), None);
    }

    #[test]
    fn sizes_that_make_no_sense_draw_nothing() {
        let (mut fonts, face) = mono();
        let (_, a) = fonts.glyph(face, 'A');
        for bad in [0.0, -4.0, f32::NAN, f32::INFINITY, 1.0e9] {
            assert_eq!(fonts.raster(face, a, bad, 0), None, "{bad}");
        }
        let missing = fonts.raster(face, 0, 16.0, 0);
        assert_eq!(fonts.raster(face, 1 << 20, 16.0, 0), missing, "a glyph number no font has is the missing glyph");
    }

    #[test]
    fn a_line_is_laid_out_left_to_right() {
        let (mut fonts, face) = mono();
        let (runs, width) = fonts.layout(face, 16.0, "Hello,\tworld\n");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].face, face);
        let xs: Vec<f32> = runs[0].glyphs.iter().map(|g| g.x).collect();
        assert_eq!(xs.len(), 11, "the tab and the newline are left out");
        assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
        let (_, h) = fonts.glyph(face, 'H');
        assert!((width - 11.0 * fonts.advance(face, h, 16.0)).abs() < 0.01, "fixed pitch: eleven cells");
        assert_eq!(fonts.layout(face, 16.0, ""), (vec![], 0.0));
    }

    #[test]
    fn a_character_the_face_lacks_comes_from_another() {
        let (mut fonts, face) = mono();
        // A character the monospace face lacks and some other font has, if this machine has one.
        let lacking = ['א', '漢', '∑', 'ج', '🙂', 'ꙮ', 'ᚠ', '☃'].into_iter().find(|c| fonts.glyph(face, *c).0 != face);
        let Some(c) = lacking else {
            eprintln!("skipped: the monospace face has every test character, or no other font has any");
            return;
        };
        let (other, g) = fonts.glyph(face, c);
        assert_ne!(other, face);
        assert_ne!(g, 0);
        assert_eq!(fonts.glyph(face, c), (other, g), "remembered");
        let (runs, _) = fonts.layout(face, 16.0, &format!("A{c}A"));
        assert_eq!(runs.iter().map(|r| r.face).collect::<Vec<_>>(), [face, other, face]);
        assert!(fonts.raster(other, g, 32.0, 0).is_some() || fonts.advance(other, g, 32.0) > 0.0);
    }

    #[test]
    fn headings_take_the_display_cut() {
        // Inter, shipped: opsz 14 (text) to 32 (display).
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fonts");
        let mut fonts = Fonts::new(FontDb::scan(&[dir]));
        let face = fonts.face("Inter Variable", 400, false).expect("Inter in data/fonts");
        assert_eq!(fonts.sized(face, 13.0), face, "below the axis: the default instance");
        assert_eq!(fonts.sized(face, 14.2), face, "the default size");
        let big = fonts.sized(face, 28.0);
        assert_ne!(big, face);
        assert_eq!(fonts.sized(face, 28.3), big, "kept, not loaded again");
        assert_eq!(fonts.name(big), fonts.name(face), "scenes name it the same");
        // The display cut is set tighter: per unit of size, narrower.
        let (_, text_w) = fonts.layout(face, 14.0, "Wave headline");
        let (_, head_w) = fonts.layout(face, 28.0, "Wave headline");
        assert!(head_w < text_w * 2.0 * 0.99, "28: {head_w}, 14: {text_w}");
        // Its glyphs are drawn from that instance.
        let (_, a) = fonts.glyph(face, 'a');
        assert!(fonts.raster(big, a, 28.0, 0).is_some());
    }

    #[test]
    fn a_character_nobody_has_is_the_missing_glyph() {
        let (mut fonts, face) = mono();
        // A private-use character no installed font should claim.
        assert_eq!(fonts.glyph(face, '\u{10fffd}'), (face, 0));
    }

    #[test]
    fn bold_is_bolder_when_the_family_has_it() {
        let (mut fonts, face) = mono();
        let bold = fonts.face("monospace", 700, false).expect("a face for bold");
        let (regular, heavy) = (fonts.name(face), fonts.name(bold));
        assert_eq!(regular.family, heavy.family);
        if heavy.weight > regular.weight {
            assert_ne!(bold, face);
            let (_, a) = fonts.glyph(face, 'A');
            let (_, b) = fonts.glyph(bold, 'A');
            let ink = |bm: &Bitmap| bm.alpha.iter().map(|v| *v as u64).sum::<u64>();
            let light = ink(&fonts.raster(face, a, 48.0, 0).unwrap());
            let dark = ink(&fonts.raster(bold, b, 48.0, 0).unwrap());
            assert!(dark > light, "bold has more ink: {dark} against {light}");
        } else {
            eprintln!("skipped: {} has no bolder face here", regular.family);
        }
    }
}
