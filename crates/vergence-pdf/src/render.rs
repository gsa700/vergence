//! Pages drawn (PDF milestone 1, step 3): the content stream interpreter
//! over vergence-raster. Paths, painting and clipping; colors in every
//! space; the graphics state and its stack; ExtGState; form XObjects
//! (nested, cycles cut); images and inline images; shadings (axial, radial,
//! function-based) by `sh` and as patterns; tiling patterns; constant
//! alpha; annotations' normal appearances.
//!
//! Text (step 4): the text operators, rendering modes 0 to 7 (text as a
//! clip included), glyphs from font/ drawn as paths in the fill or stroke.
//! Not yet: CFF, Type 1 and Type 3 glyph programs (step 4's next parts), JPEG and
//! the other picture codecs (step 5: a light gray box stands in), blend
//! modes, soft masks and knockout groups (drawn as normal), mesh shadings
//! (types 4 to 7: skipped).

use std::collections::HashSet;
use std::rc::Rc;

use vergence_raster::{Canvas, Cap, Clip, FillRule, Image, Join, Path, Point, Stroke, Transform};

use crate::color::ColorSpace;
use crate::font::Font;
use crate::content;
use crate::function::Function;
use crate::image::{self, Decoded};
use crate::object::{Dict, Object, Ref};
use crate::{Document, Page};

/// How deep forms (and patterns and annotations) may nest, how many
/// operations one page may run, and how deep the q/Q stack may go.
const MAX_FORM_DEPTH: usize = 12;
const MAX_PAGE_OPS: usize = 8_000_000;
const MAX_SAVES: usize = 256;
/// The most pixels a tiling pattern's cell is drawn at.
const MAX_CELL: usize = 1 << 22;

/// A page drawn at `scale` pixels per point (1 = 72 dpi), white ground.
pub fn render_page(doc: &Document, page: &Page, scale: f64) -> Option<Canvas> {
    render_region(doc, page, scale, None)
}

/// The page's size as shown, in points: its visible box, turned by /Rotate.
pub fn page_size(page: &Page) -> (f64, f64) {
    let b = page_box(page);
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    if page.rotate % 180 == 90 { (h, w) } else { (w, h) }
}

/// The page's pixels at `scale` (its full size is page_size times scale,
/// rounded up), or only the rectangle `region` (x, y, width, height in
/// those pixels): a tile while zoomed in.
pub fn render_region(doc: &Document, page: &Page, scale: f64, region: Option<(usize, usize, usize, usize)>) -> Option<Canvas> {
    let b = page_box(page);
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let (dw, dh) = page_size(page);
    let (full_w, full_h) = ((dw * scale).ceil().max(1.0) as usize, (dh * scale).ceil().max(1.0) as usize);
    let (x, y, pw, ph) = match region {
        Some((x, y, rw, rh)) if rw > 0 && rh > 0 && x < full_w && y < full_h => (x, y, rw.min(full_w - x), rh.min(full_h - y)),
        Some(_) => return None,
        None => (0, 0, full_w, full_h),
    };
    let mut canvas = Canvas::new(pw, ph)?;
    canvas.clear([1.0, 1.0, 1.0, 1.0]);
    let mut base = page_transform(b, page.rotate, scale);
    base.e -= x as f64;
    base.f -= y as f64;
    let mut r = Renderer { doc, canvas, ops: 0, forms: HashSet::new(), base, fonts: std::collections::HashMap::new() };
    let resources = Rc::new(page.resources.clone().unwrap_or_default());
    let mut gs = State::new(base, Clip::full(pw, ph), resources.clone());
    // The page's box clips everything drawn on it.
    let mut boxpath = Path::new();
    boxpath.rect(b[0], b[1], w, h);
    gs.clip = Rc::new(gs.clip.intersect(&boxpath, &base, FillRule::NonZero, pw, ph));
    let data = doc.contents(page);
    r.run(&data, gs.clone(), 0);
    r.annotations(page, &gs);
    Some(r.canvas)
}

/// The visible box: CropBox within MediaBox, else MediaBox.
pub fn page_box(page: &Page) -> [f64; 4] {
    let m = page.media_box;
    match page.crop_box {
        Some(c) => {
            let r = [c[0].max(m[0]), c[1].max(m[1]), c[2].min(m[2]), c[3].min(m[3])];
            if r[2] > r[0] && r[3] > r[1] { r } else { m }
        }
        None => m,
    }
}

/// User space (y up, points) to the canvas (y down, pixels), turned by the
/// page's /Rotate (clockwise).
pub fn page_transform(b: [f64; 4], rotate: u16, s: f64) -> Transform {
    let [x0, y0, x1, y1] = b;
    match rotate % 360 {
        90 => Transform::new(0.0, s, s, 0.0, -y0 * s, -x0 * s),
        180 => Transform::new(-s, 0.0, 0.0, s, x1 * s, -y0 * s),
        270 => Transform::new(0.0, -s, -s, 0.0, y1 * s, x1 * s),
        _ => Transform::new(s, 0.0, 0.0, -s, -x0 * s, y1 * s),
    }
}

#[derive(Clone)]
struct Paint {
    space: Rc<ColorSpace>,
    color: Vec<f64>,
    /// The pattern named by `scn` in a Pattern space.
    pattern: Option<Object>,
}

impl Paint {
    fn black() -> Paint {
        Paint { space: Rc::new(ColorSpace::Gray), color: vec![0.0], pattern: None }
    }

    fn rgb(&self) -> Option<[f32; 3]> {
        self.space.to_rgb(&self.color)
    }
}

#[derive(Clone)]
struct State {
    ctm: Transform,
    clip: Rc<Clip>,
    stroke: Stroke,
    stroke_paint: Paint,
    fill_paint: Paint,
    stroke_alpha: f32,
    fill_alpha: f32,
    resources: Rc<Dict>,
    text: TextState,
}

/// The text state (ISO 32000-2 §9.3), part of the graphics state.
#[derive(Clone)]
struct TextState {
    char_spacing: f64,
    word_spacing: f64,
    /// Tz / 100.
    scale: f64,
    leading: f64,
    font: Option<Rc<Font>>,
    size: f64,
    mode: i64,
    rise: f64,
}

impl Default for TextState {
    fn default() -> TextState {
        TextState { char_spacing: 0.0, word_spacing: 0.0, scale: 1.0, leading: 0.0, font: None, size: 0.0, mode: 0, rise: 0.0 }
    }
}

impl State {
    fn new(ctm: Transform, clip: Clip, resources: Rc<Dict>) -> State {
        State { ctm, clip: Rc::new(clip), stroke: Stroke::default(), stroke_paint: Paint::black(), fill_paint: Paint::black(), stroke_alpha: 1.0, fill_alpha: 1.0, resources, text: TextState::default() }
    }
}

struct Renderer<'d> {
    doc: &'d Document,
    canvas: Canvas,
    ops: usize,
    /// Forms being drawn now (their numbers), to cut a form drawing itself.
    forms: HashSet<u32>,
    /// The page's own transform (patterns live in its space).
    base: Transform,
    /// Fonts loaded on this page, by object number (or by name if direct).
    fonts: std::collections::HashMap<Vec<u8>, Rc<Font>>,
}

fn num(o: Option<&Object>) -> f64 {
    o.and_then(Object::as_f64).unwrap_or(0.0)
}

impl Renderer<'_> {
    fn size(&self) -> (usize, usize) {
        (self.canvas.width, self.canvas.height)
    }

    /// Runs a content stream from state `gs`.
    fn run(&mut self, data: &[u8], gs: State, depth: usize) {
        let ops = content::parse(data);
        let mut gs = gs;
        let mut stack: Vec<State> = Vec::new();
        let mut path = Path::new();
        let mut current = Point::new(0.0, 0.0);
        let mut pending_clip: Option<FillRule> = None;
        // The text matrices (reset at BT, not saved by q) and the glyphs
        // gathered for a text clip (modes 4 to 7), applied at ET.
        let (mut tm, mut tlm) = (Transform::IDENTITY, Transform::IDENTITY);
        let mut text_clip: Option<Path> = None;
        for op in &ops {
            self.ops += 1;
            if self.ops > MAX_PAGE_OPS {
                return;
            }
            let a = &op.args;
            let n = |i: usize| num(a.get(i));
            match op.name.as_slice() {
                b"q" => {
                    if stack.len() < MAX_SAVES {
                        stack.push(gs.clone());
                    }
                }
                b"Q" => {
                    if let Some(s) = stack.pop() {
                        gs = s;
                    }
                }
                b"cm" if a.len() >= 6 => {
                    let m = Transform::new(n(0), n(1), n(2), n(3), n(4), n(5));
                    if m.is_finite() {
                        gs.ctm = m.then(&gs.ctm);
                    }
                }
                b"w" => gs.stroke.width = n(0),
                b"J" => gs.stroke.cap = cap(n(0)),
                b"j" => gs.stroke.join = join(n(0)),
                b"M" => gs.stroke.miter_limit = n(0),
                b"d" => {
                    gs.stroke.dash = a.first().and_then(Object::as_array).map(|d| d.iter().filter_map(Object::as_f64).collect()).unwrap_or_default();
                    gs.stroke.dash_phase = n(1);
                }
                b"gs" => {
                    if let Some(name) = a.first().and_then(Object::as_name) {
                        self.ext_gstate(&mut gs, name);
                    }
                }
                // Path construction.
                b"m" => {
                    current = Point::new(n(0), n(1));
                    path.move_to(current.x, current.y);
                }
                b"l" => {
                    current = Point::new(n(0), n(1));
                    path.line_to(current.x, current.y);
                }
                b"c" => {
                    path.cubic_to(n(0), n(1), n(2), n(3), n(4), n(5));
                    current = Point::new(n(4), n(5));
                }
                b"v" => {
                    path.cubic_to(current.x, current.y, n(0), n(1), n(2), n(3));
                    current = Point::new(n(2), n(3));
                }
                b"y" => {
                    path.cubic_to(n(0), n(1), n(2), n(3), n(2), n(3));
                    current = Point::new(n(2), n(3));
                }
                b"h" => {
                    path.close();
                }
                b"re" => {
                    path.rect(n(0), n(1), n(2), n(3));
                    current = Point::new(n(0), n(1));
                }
                // Painting, then any clip the W before it asked for.
                b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                    let name = op.name.as_slice();
                    if matches!(name, b"s" | b"b" | b"b*") {
                        path.close();
                    }
                    let fill = match name {
                        b"f" | b"F" | b"B" | b"b" => Some(FillRule::NonZero),
                        b"f*" | b"B*" | b"b*" => Some(FillRule::EvenOdd),
                        _ => None,
                    };
                    if let Some(rule) = fill {
                        self.fill(&path, rule, &gs, depth);
                    }
                    if matches!(name, b"S" | b"s" | b"B" | b"B*" | b"b" | b"b*") {
                        self.stroke(&path, &gs);
                    }
                    if let Some(rule) = pending_clip.take() {
                        let (w, h) = self.size();
                        gs.clip = Rc::new(gs.clip.intersect(&path, &gs.ctm, rule, w, h));
                    }
                    path = Path::new();
                }
                b"W" => pending_clip = Some(FillRule::NonZero),
                b"W*" => pending_clip = Some(FillRule::EvenOdd),
                // Color.
                b"CS" | b"cs" => {
                    let space = a.first().and_then(|o| ColorSpace::parse(o, self.doc, Some(&gs.resources))).unwrap_or(ColorSpace::Gray);
                    let paint = Paint { color: space.initial(), space: Rc::new(space), pattern: None };
                    if op.name == b"CS" { gs.stroke_paint = paint } else { gs.fill_paint = paint }
                }
                b"SC" | b"SCN" | b"sc" | b"scn" => {
                    let p = if op.name[0] == b'S' { &mut gs.stroke_paint } else { &mut gs.fill_paint };
                    p.color = a.iter().filter_map(Object::as_f64).collect();
                    p.pattern = a.last().filter(|o| o.as_name().is_some()).cloned();
                }
                b"G" | b"g" => set_color(if op.name == b"G" { &mut gs.stroke_paint } else { &mut gs.fill_paint }, ColorSpace::Gray, vec![n(0)]),
                b"RG" | b"rg" => set_color(if op.name == b"RG" { &mut gs.stroke_paint } else { &mut gs.fill_paint }, ColorSpace::Rgb, vec![n(0), n(1), n(2)]),
                b"K" | b"k" => set_color(if op.name == b"K" { &mut gs.stroke_paint } else { &mut gs.fill_paint }, ColorSpace::Cmyk, vec![n(0), n(1), n(2), n(3)]),
                // XObjects, shadings, inline images.
                b"Do" => {
                    if let Some(name) = a.first().and_then(Object::as_name) {
                        self.xobject(name, &gs, depth);
                    }
                }
                b"sh" => {
                    if let Some(name) = a.first().and_then(Object::as_name) {
                        let sh = self.resource(&gs, "Shading", name);
                        let (w, h) = self.size();
                        let mut all = Path::new();
                        all.rect(0.0, 0.0, w as f64, h as f64);
                        self.shade(&sh, &all, &Transform::IDENTITY, FillRule::NonZero, &gs.ctm, &gs, gs.fill_alpha);
                    }
                }
                b"BI" => {
                    if let (Some(Object::Dict(d)), Some(Object::String(data))) = (a.first(), a.get(1)) {
                        let decoded = image::inline(self.doc, d, data, Some(&gs.resources));
                        self.image(decoded, &gs, matches!(d.get("I").or_else(|| d.get("Interpolate")), Some(Object::Bool(true))));
                    }
                }
                // Text.
                b"BT" => {
                    tm = Transform::IDENTITY;
                    tlm = Transform::IDENTITY;
                    text_clip = None;
                }
                b"ET" => {
                    if let Some(p) = text_clip.take() {
                        let (w, h) = self.size();
                        gs.clip = Rc::new(gs.clip.intersect(&p, &gs.ctm, FillRule::NonZero, w, h));
                    }
                }
                b"Tc" => gs.text.char_spacing = n(0),
                b"Tw" => gs.text.word_spacing = n(0),
                b"Tz" => gs.text.scale = n(0) / 100.0,
                b"TL" => gs.text.leading = n(0),
                b"Ts" => gs.text.rise = n(0),
                b"Tr" => gs.text.mode = n(0) as i64,
                b"Tf" => {
                    gs.text.size = n(1);
                    if let Some(name) = a.first().and_then(Object::as_name) {
                        gs.text.font = self.font(&gs, name);
                    }
                }
                b"Td" | b"TD" => {
                    if op.name == b"TD" {
                        gs.text.leading = -n(1);
                    }
                    tlm = Transform::translate(n(0), n(1)).then(&tlm);
                    tm = tlm;
                }
                b"Tm" if a.len() >= 6 => {
                    let m = Transform::new(n(0), n(1), n(2), n(3), n(4), n(5));
                    if m.is_finite() {
                        tlm = m;
                        tm = m;
                    }
                }
                b"T*" => {
                    tlm = Transform::translate(0.0, -gs.text.leading).then(&tlm);
                    tm = tlm;
                }
                b"Tj" => {
                    if let Some(Object::String(t)) = a.first() {
                        self.show(t, &gs, &mut tm, &mut text_clip, depth);
                    }
                }
                b"'" | b"\"" => {
                    if op.name == b"\"" {
                        gs.text.word_spacing = n(0);
                        gs.text.char_spacing = n(1);
                    }
                    tlm = Transform::translate(0.0, -gs.text.leading).then(&tlm);
                    tm = tlm;
                    if let Some(Object::String(t)) = a.last() {
                        self.show(t, &gs, &mut tm, &mut text_clip, depth);
                    }
                }
                b"TJ" => {
                    if let Some(Object::Array(items)) = a.first() {
                        for item in items {
                            match item {
                                Object::String(t) => self.show(t, &gs, &mut tm, &mut text_clip, depth),
                                other => {
                                    if let Some(adj) = other.as_f64() {
                                        let tx = -adj / 1000.0 * gs.text.size * gs.text.scale;
                                        tm = Transform::translate(tx, 0.0).then(&tm);
                                    }
                                }
                            }
                        }
                    }
                }
                // Everything else (marked content, compatibility sections): passed over.
                _ => {}
            }
        }
    }

    /// A font resource, loaded once per page.
    fn font(&mut self, gs: &State, name: &[u8]) -> Option<Rc<Font>> {
        let entry = self.resource(gs, "Font", name);
        let key = match entry.as_ref() {
            Some(r) => format!("{} {}", r.num, r.generation).into_bytes(),
            None => [b"/".as_slice(), name].concat(),
        };
        if let Some(f) = self.fonts.get(&key) {
            return Some(f.clone());
        }
        let d = self.doc.resolve(&entry).as_dict().cloned()?;
        let f = Rc::new(Font::load(self.doc, &d));
        self.fonts.insert(key, f.clone());
        Some(f)
    }

    /// Shows a string: each glyph through the text rendering matrix, drawn
    /// as the mode says, the pen moved by its width and the spacings.
    fn show(&mut self, s: &[u8], gs: &State, tm: &mut Transform, text_clip: &mut Option<Path>, depth: usize) {
        let Some(font) = gs.text.font.clone() else { return };
        let t = &gs.text;
        let mode = t.mode;
        for (code, is_space) in font.codes(s) {
            let w0 = font.width(code);
            if font.is_type3() {
                // A Type 3 glyph is a content stream in glyph space.
                if mode != 3 && mode != 7
                    && let Some((t3, proc_obj)) = font.type3(code)
                    && let Object::Stream(st) = self.doc.resolve(&proc_obj)
                {
                    let trm = Transform::new(t.size * t.scale, 0.0, 0.0, t.size, 0.0, t.rise).then(tm);
                    let mut inner = gs.clone();
                    inner.ctm = t3.matrix.then(&trm).then(&gs.ctm);
                    if let Some(r) = &t3.resources {
                        inner.resources = Rc::new(r.clone());
                    }
                    inner.text = TextState::default();
                    let data = self.doc.stream(&st).data;
                    if depth < MAX_FORM_DEPTH {
                        self.run(&data, inner, depth + 1);
                    }
                }
            } else if mode != 3
                && let Some(g) = font.glyph(code)
            {
                let trm = Transform::new(t.size * t.scale, 0.0, 0.0, t.size, 0.0, t.rise).then(tm);
                let user = g.transformed(&trm);
                if matches!(mode, 0 | 2 | 4 | 6) {
                    self.fill(&user, FillRule::NonZero, gs, depth);
                }
                if matches!(mode, 1 | 2 | 5 | 6) {
                    self.stroke(&user, gs);
                }
                if (4..=7).contains(&mode) {
                    text_clip.get_or_insert_with(Path::new).append(&user);
                }
            }
            let tx = (w0 * t.size + t.char_spacing + if is_space { t.word_spacing } else { 0.0 }) * t.scale;
            *tm = Transform::translate(tx, 0.0).then(tm);
        }
    }

    fn resource(&self, gs: &State, kind: &str, name: &[u8]) -> Object {
        let Some(group) = self.doc.get_in(&gs.resources, kind).as_dict().cloned() else { return Object::Null };
        let Ok(key) = std::str::from_utf8(name) else { return Object::Null };
        group.get(key).cloned().unwrap_or(Object::Null)
    }

    fn ext_gstate(&self, gs: &mut State, name: &[u8]) {
        let Some(d) = self.doc.resolve(&self.resource(gs, "ExtGState", name)).as_dict().cloned() else { return };
        let get = |k: &str| self.doc.get_in(&d, k);
        if let Some(v) = get("LW").as_f64() {
            gs.stroke.width = v;
        }
        if let Some(v) = get("LC").as_f64() {
            gs.stroke.cap = cap(v);
        }
        if let Some(v) = get("LJ").as_f64() {
            gs.stroke.join = join(v);
        }
        if let Some(v) = get("ML").as_f64() {
            gs.stroke.miter_limit = v;
        }
        if let Object::Array(dash) = get("D")
            && let (Some(arr), Some(phase)) = (dash.first().map(|o| self.doc.resolve(o)), dash.get(1).and_then(|o| self.doc.resolve(o).as_f64()))
        {
            gs.stroke.dash = arr.as_array().map(|a| a.iter().filter_map(|x| self.doc.resolve(x).as_f64()).collect()).unwrap_or_default();
            gs.stroke.dash_phase = phase;
        }
        if let Some(v) = get("CA").as_f64() {
            gs.stroke_alpha = v.clamp(0.0, 1.0) as f32;
        }
        if let Some(v) = get("ca").as_f64() {
            gs.fill_alpha = v.clamp(0.0, 1.0) as f32;
        }
    }

    fn fill(&mut self, path: &Path, rule: FillRule, gs: &State, depth: usize) {
        let p = &gs.fill_paint;
        if let ColorSpace::Pattern(base) = p.space.as_ref() {
            let Some(name) = p.pattern.as_ref().and_then(Object::as_name) else { return };
            let pat = self.resource(gs, "Pattern", name);
            let tint = base.as_ref().and_then(|b| b.to_rgb(&p.color));
            self.pattern(&pat, path, rule, gs, tint, depth);
            return;
        }
        if let Some(rgb) = p.rgb() {
            self.canvas.fill(path, &gs.ctm, rule, [rgb[0], rgb[1], rgb[2], gs.fill_alpha], Some(&gs.clip));
        }
    }

    fn stroke(&mut self, path: &Path, gs: &State) {
        // A pattern stroke is drawn in black for now (rare in practice).
        let rgb = match gs.stroke_paint.space.as_ref() {
            ColorSpace::Pattern(_) => Some([0.0; 3]),
            _ => gs.stroke_paint.rgb(),
        };
        if let Some(rgb) = rgb {
            self.canvas.stroke(path, &gs.ctm, &gs.stroke, [rgb[0], rgb[1], rgb[2], gs.stroke_alpha], Some(&gs.clip));
        }
    }

    fn xobject(&mut self, name: &[u8], gs: &State, depth: usize) {
        let entry = self.resource(gs, "XObject", name);
        let reference = entry.as_ref();
        let Object::Stream(s) = self.doc.resolve(&entry) else { return };
        match s.dict.name("Subtype") {
            Some(b"Image") => {
                let smooth = matches!(self.doc.get_in(&s.dict, "Interpolate"), Object::Bool(true));
                let decoded = image::from_stream(self.doc, &s, Some(&gs.resources));
                self.image(decoded, gs, smooth);
            }
            Some(b"Form") => self.form(&s, reference, gs, depth, None),
            _ => {}
        }
    }

    /// A form: its /Matrix, its /BBox as a clip, its own resources (or the
    /// caller's), drawn from a copy of the caller's state.
    fn form(&mut self, s: &crate::object::Stream, reference: Option<Ref>, gs: &State, depth: usize, extra: Option<Transform>) {
        if depth >= MAX_FORM_DEPTH {
            return;
        }
        if let Some(r) = reference
            && !self.forms.insert(r.num)
        {
            return;
        }
        let mut inner = gs.clone();
        let m = matrix(self.doc, &s.dict);
        inner.ctm = m.then(&extra.unwrap_or(Transform::IDENTITY)).then(&gs.ctm);
        if let Some(r) = self.doc.get_in(&s.dict, "Resources").as_dict() {
            inner.resources = Rc::new(r.clone());
        }
        if let Some(b) = rect(self.doc, &self.doc.get_in(&s.dict, "BBox")) {
            let mut bp = Path::new();
            bp.rect(b[0], b[1], b[2] - b[0], b[3] - b[1]);
            let (w, h) = self.size();
            inner.clip = Rc::new(inner.clip.intersect(&bp, &inner.ctm, FillRule::NonZero, w, h));
        }
        let data = self.doc.stream(s).data;
        self.run(&data, inner, depth + 1);
        if let Some(r) = reference {
            self.forms.remove(&r.num);
        }
    }

    /// An image in the unit square of user space.
    fn image(&mut self, decoded: Decoded, gs: &State, smooth: bool) {
        let to_device = |w: usize, h: usize, ctm: &Transform| Transform::new(1.0 / w as f64, 0.0, 0.0, -1.0 / h as f64, 0.0, 1.0).then(ctm);
        match decoded {
            Decoded::Color(img) => {
                let t = to_device(img.width, img.height, &gs.ctm);
                // Enlarged pictures are smoothed whatever /Interpolate says: a small
                // picture blown up with hard pixel edges looks like a mistake on screen.
                let smooth = smooth || t.scale_factor() > 1.0;
                self.canvas.draw_image(&img, &t, smooth, gs.fill_alpha, Some(&gs.clip));
            }
            Decoded::Stencil { width, height, alpha } => {
                let Some(rgb) = gs.fill_paint.rgb() else { return };
                let px = |c: f32, a: u8| ((c * a as f32).round()) as u8;
                let data: Vec<u8> = alpha.iter().flat_map(|a| [px(rgb[0], *a), px(rgb[1], *a), px(rgb[2], *a), *a]).collect();
                let img = Image { width, height, data };
                let t = to_device(width, height, &gs.ctm);
                self.canvas.draw_image(&img, &t, true, gs.fill_alpha, Some(&gs.clip));
            }
            Decoded::Undecoded(_) => {
                // A picture codec not read yet (step 5): a light gray box.
                let mut unit = Path::new();
                unit.rect(0.0, 0.0, 1.0, 1.0);
                self.canvas.fill(&unit, &gs.ctm, FillRule::NonZero, [0.86, 0.86, 0.88, gs.fill_alpha], Some(&gs.clip));
            }
        }
    }

    /// Fills `path` with pattern `pat`; `tint` colors an uncolored tiling pattern.
    fn pattern(&mut self, pat: &Object, path: &Path, rule: FillRule, gs: &State, tint: Option<[f32; 3]>, depth: usize) {
        let reference = pat.as_ref();
        let pat = self.doc.resolve(pat);
        let Some(d) = pat.as_dict().cloned() else { return };
        // Patterns live in the page's space, moved by their own matrix.
        let space = matrix(self.doc, &d).then(&self.base);
        match d.int("PatternType") {
            Some(2) => {
                let sh = self.doc.get_in(&d, "Shading");
                self.shade(&sh, path, &gs.ctm, rule, &space, gs, gs.fill_alpha);
            }
            Some(1) => {
                if let Object::Stream(s) = pat {
                    self.tiling(&s, reference, &space, path, rule, gs, tint, depth);
                }
            }
            _ => {}
        }
    }

    /// A shading within `path` (through `path_t`), in shading space `space`.
    #[allow(clippy::too_many_arguments)]
    fn shade(&mut self, sh: &Object, path: &Path, path_t: &Transform, rule: FillRule, space: &Transform, gs: &State, alpha: f32) {
        let doc = self.doc;
        let sh = doc.resolve(sh);
        let Some(d) = sh.as_dict().cloned() else { return };
        let Some(cs) = ColorSpace::parse(&doc.get_in(&d, "ColorSpace"), doc, Some(&gs.resources)) else { return };
        let func = d.get("Function").and_then(|f| Function::parse(f, doc));
        let nums = |k: &str| -> Vec<f64> { doc.get_in(&d, k).as_array().map(|a| a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect()).unwrap_or_default() };
        let coords = nums("Coords");
        let domain = { let v = nums("Domain"); if v.len() >= 2 { v } else { vec![0.0, 1.0, 0.0, 1.0] } };
        let extend: Vec<bool> = doc.get_in(&d, "Extend").as_array().map(|a| a.iter().map(|x| matches!(doc.resolve(x), Object::Bool(true))).collect()).unwrap_or_default();
        let (e0, e1) = (extend.first().copied().unwrap_or(false), extend.get(1).copied().unwrap_or(false));
        let Some(inv) = space.invert() else { return };
        let color = |t: &[f64]| -> Option<[f32; 3]> {
            let out = match &func {
                Some(f) => f.eval(t),
                None => t.to_vec(),
            };
            cs.to_rgb(&out)
        };
        let kind = d.int("ShadingType").unwrap_or(0);
        if !(1..=3).contains(&kind) {
            // Mesh shadings (4-7) are not drawn yet.
            return;
        }
        if (kind == 2 && coords.len() < 4) || (kind == 3 && coords.len() < 6) {
            return;
        }
        let fmatrix = if kind == 1 { matrix(doc, &d) } else { Transform::IDENTITY };
        let finv = fmatrix.invert().unwrap_or(Transform::IDENTITY);
        let (w, h) = self.size();
        let Some(cov) = vergence_raster::fill_coverage(path, path_t, rule, w, h) else { return };
        let shade_at = |x: f64, y: f64| -> [f32; 4] {
            let q = inv.apply(Point::new(x, y));
            let t: Option<Vec<f64>> = match kind {
                1 => {
                    let p = finv.apply(q);
                    (p.x >= domain[0] && p.x <= domain[1] && p.y >= domain[2] && p.y <= domain[3]).then(|| vec![p.x, p.y])
                }
                2 => {
                    let (x0, y0, x1, y1) = (coords[0], coords[1], coords[2], coords[3]);
                    let (dx, dy) = (x1 - x0, y1 - y0);
                    let len2 = dx * dx + dy * dy;
                    let s = if len2 > 0.0 { ((q.x - x0) * dx + (q.y - y0) * dy) / len2 } else { 0.0 };
                    param(s, e0, e1).map(|s| vec![domain[0] + s * (domain[1] - domain[0])])
                }
                _ => radial(&coords, q, e0, e1).map(|s| vec![domain[0] + s * (domain[1] - domain[0])]),
            };
            match t.and_then(|t| color(&t)) {
                Some(c) => [c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha],
                None => [0.0; 4],
            }
        };
        self.canvas.paint(&cov, Some(&gs.clip), &shade_at);
    }

    /// A tiling pattern: one cell drawn, then repeated by XStep and YStep.
    #[allow(clippy::too_many_arguments)]
    fn tiling(&mut self, s: &crate::object::Stream, reference: Option<Ref>, space: &Transform, path: &Path, rule: FillRule, gs: &State, tint: Option<[f32; 3]>, depth: usize) {
        if depth >= MAX_FORM_DEPTH {
            return;
        }
        if let Some(r) = reference
            && self.forms.contains(&r.num)
        {
            return;
        }
        let doc = self.doc;
        let Some(bbox) = rect(doc, &doc.get_in(&s.dict, "BBox")) else { return };
        let xstep = doc.get_in(&s.dict, "XStep").as_f64().unwrap_or(bbox[2] - bbox[0]).abs();
        let ystep = doc.get_in(&s.dict, "YStep").as_f64().unwrap_or(bbox[3] - bbox[1]).abs();
        if !(xstep > 1e-6 && ystep > 1e-6) {
            return;
        }
        // The cell at device resolution, in pattern space, axis-aligned.
        let k = space.scale_factor().clamp(0.01, 64.0);
        let (cw, ch) = (((xstep * k).ceil() as usize).max(1), ((ystep * k).ceil() as usize).max(1));
        if cw.saturating_mul(ch) > MAX_CELL {
            // A huge cell: fill with the tint or a neutral gray rather than allocate.
            let c = tint.unwrap_or([0.5; 3]);
            self.canvas.fill(path, &gs.ctm, rule, [c[0], c[1], c[2], gs.fill_alpha], Some(&gs.clip));
            return;
        }
        let Some(cell_canvas) = Canvas::new(cw, ch) else { return };
        let cell_t = Transform::new(k, 0.0, 0.0, -k, -bbox[0] * k, bbox[1] * k + ystep * k);
        let mut sub = Renderer { doc, canvas: cell_canvas, ops: self.ops, forms: std::mem::take(&mut self.forms), base: cell_t, fonts: std::mem::take(&mut self.fonts) };
        if let Some(r) = reference {
            sub.forms.insert(r.num);
        }
        let mut cell_state = State::new(cell_t, Clip::full(cw, ch), Rc::new(doc.get_in(&s.dict, "Resources").as_dict().cloned().unwrap_or_default()));
        let mut bp = Path::new();
        bp.rect(bbox[0], bbox[1], bbox[2] - bbox[0], bbox[3] - bbox[1]);
        cell_state.clip = Rc::new(cell_state.clip.intersect(&bp, &cell_t, FillRule::NonZero, cw, ch));
        if let Some(c) = tint {
            let paint = Paint { space: Rc::new(ColorSpace::Rgb), color: c.iter().map(|v| *v as f64).collect(), pattern: None };
            cell_state.fill_paint = paint.clone();
            cell_state.stroke_paint = paint;
        }
        let data = doc.stream(s).data;
        sub.run(&data, cell_state, depth + 1);
        self.ops = sub.ops;
        self.forms = std::mem::take(&mut sub.forms);
        self.fonts = std::mem::take(&mut sub.fonts);
        if let Some(r) = reference {
            self.forms.remove(&r.num);
        }
        let cell = sub.canvas;
        let Some(inv) = space.invert() else { return };
        let (w, h) = self.size();
        let Some(cov) = vergence_raster::fill_coverage(path, &gs.ctm, rule, w, h) else { return };
        let alpha = gs.fill_alpha;
        let x0 = bbox[0];
        let y0 = bbox[1];
        self.canvas.paint(&cov, Some(&gs.clip), &|x, y| {
            let q = inv.apply(Point::new(x, y));
            let u = (q.x - x0).rem_euclid(xstep);
            let v = (q.y - y0).rem_euclid(ystep);
            let (cx, cy) = (((u * k) as usize).min(cw - 1), ((ch as f64 - v * k) as usize).min(ch - 1));
            let i = (cy * cw + cx) * 4;
            let p = &cell.data[i..i + 4];
            [p[0] as f32 / 255.0 * alpha, p[1] as f32 / 255.0 * alpha, p[2] as f32 / 255.0 * alpha, p[3] as f32 / 255.0 * alpha]
        });
    }

    /// Annotations with a normal appearance, drawn over the page.
    fn annotations(&mut self, page: &Page, gs: &State) {
        let doc = self.doc;
        let Object::Array(annots) = doc.get_in(&page.dict, "Annots") else { return };
        for a in annots.iter().take(10_000) {
            let Some(d) = doc.resolve(a).as_dict().cloned() else { continue };
            let flags = d.int("F").unwrap_or(0);
            // Hidden (2) or not to be viewed (32); popups draw nothing themselves.
            if flags & 2 != 0 || flags & 32 != 0 || d.name("Subtype") == Some(b"Popup") {
                continue;
            }
            let Some(r) = rect(doc, &doc.get_in(&d, "Rect")) else { continue };
            let Some(ap) = doc.get_in(&d, "AP").as_dict().cloned() else { continue };
            let normal = ap.get("N").cloned().unwrap_or(Object::Null);
            // One appearance, or one per state picked by /AS.
            let (stream_obj, reference) = match doc.resolve(&normal) {
                Object::Stream(s) => (Some(s), normal.as_ref()),
                Object::Dict(states) => {
                    let state = d.name("AS").map(<[u8]>::to_vec);
                    let pick = state.and_then(|s| std::str::from_utf8(&s).ok().and_then(|k| states.get(k).cloned()));
                    match pick.as_ref().map(|p| (doc.resolve(p), p.as_ref())) {
                        Some((Object::Stream(s), r)) => (Some(s), r),
                        _ => (None, None),
                    }
                }
                _ => (None, None),
            };
            let Some(s) = stream_obj else { continue };
            // Algorithm 8.1: the form's box through its matrix, fitted to /Rect.
            let m = matrix(doc, &s.dict);
            let Some(bb) = rect(doc, &doc.get_in(&s.dict, "BBox")) else { continue };
            let pts = [(bb[0], bb[1]), (bb[2], bb[1]), (bb[0], bb[3]), (bb[2], bb[3])].map(|(x, y)| m.apply(Point::new(x, y)));
            let (tx0, ty0) = (pts.iter().map(|p| p.x).fold(f64::MAX, f64::min), pts.iter().map(|p| p.y).fold(f64::MAX, f64::min));
            let (tx1, ty1) = (pts.iter().map(|p| p.x).fold(f64::MIN, f64::max), pts.iter().map(|p| p.y).fold(f64::MIN, f64::max));
            if tx1 - tx0 <= 0.0 || ty1 - ty0 <= 0.0 {
                continue;
            }
            let fit = Transform::new((r[2] - r[0]) / (tx1 - tx0), 0.0, 0.0, (r[3] - r[1]) / (ty1 - ty0), 0.0, 0.0);
            let fit = Transform::translate(-tx0, -ty0).then(&fit).then(&Transform::translate(r[0], r[1]));
            let mut state = gs.clone();
            state.resources = Rc::new(doc.get_in(&s.dict, "Resources").as_dict().cloned().unwrap_or_default());
            self.form(&s, reference, &state, 0, Some(fit));
        }
    }
}

fn set_color(p: &mut Paint, space: ColorSpace, color: Vec<f64>) {
    *p = Paint { space: Rc::new(space), color, pattern: None };
}

fn cap(v: f64) -> Cap {
    match v as i64 {
        1 => Cap::Round,
        2 => Cap::Square,
        _ => Cap::Butt,
    }
}

fn join(v: f64) -> Join {
    match v as i64 {
        1 => Join::Round,
        2 => Join::Bevel,
        _ => Join::Miter,
    }
}

/// A dictionary's /Matrix (identity if absent or unusable).
fn matrix(doc: &Document, d: &Dict) -> Transform {
    let v: Vec<f64> = doc.get_in(d, "Matrix").as_array().map(|a| a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect()).unwrap_or_default();
    if v.len() == 6 {
        let m = Transform::new(v[0], v[1], v[2], v[3], v[4], v[5]);
        if m.is_finite() {
            return m;
        }
    }
    Transform::IDENTITY
}

fn rect(doc: &Document, o: &Object) -> Option<[f64; 4]> {
    let v: Vec<f64> = o.as_array()?.iter().filter_map(|x| doc.resolve(x).as_f64()).collect();
    if v.len() < 4 {
        return None;
    }
    let r = [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])];
    (r.iter().all(|x| x.is_finite()) && r[2] > r[0] && r[3] > r[1]).then_some(r)
}

/// An axial position kept within 0..1, or extended past an end if asked.
fn param(s: f64, e0: bool, e1: bool) -> Option<f64> {
    if !s.is_finite() {
        return None;
    }
    if s < 0.0 {
        return e0.then_some(0.0);
    }
    if s > 1.0 {
        return e1.then_some(1.0);
    }
    Some(s)
}

/// A radial shading's position for point `q`: the largest s whose circle
/// passes through it with a radius not below zero.
fn radial(c: &[f64], q: Point, e0: bool, e1: bool) -> Option<f64> {
    let (x0, y0, r0, x1, y1, r1) = (c[0], c[1], c[2], c[3], c[4], c[5]);
    let (cdx, cdy, dr) = (x1 - x0, y1 - y0, r1 - r0);
    let (pdx, pdy) = (q.x - x0, q.y - y0);
    let a = cdx * cdx + cdy * cdy - dr * dr;
    let b = pdx * cdx + pdy * cdy + r0 * dr;
    let cc = pdx * pdx + pdy * pdy - r0 * r0;
    let ok = |s: f64| r0 + s * dr >= 0.0 && (s >= 0.0 || e0) && (s <= 1.0 || e1);
    let candidates: Vec<f64> = if a.abs() < 1e-12 {
        if b.abs() < 1e-12 { vec![] } else { vec![cc / (2.0 * b)] }
    } else {
        let disc = b * b - a * cc;
        if disc < 0.0 {
            return None;
        }
        let sq = disc.sqrt();
        let (s1, s2) = ((b + sq) / a, (b - sq) / a);
        vec![s1.max(s2), s1.min(s2)]
    };
    candidates.into_iter().find(|s| ok(*s)).map(|s| s.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf(content: &str, resources: &str) -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources {resources} >>"),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer << /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF").as_bytes());
        out
    }

    fn draw(content: &str, resources: &str) -> Canvas {
        let doc = Document::open(pdf(content, resources)).unwrap();
        let page = doc.pages().remove(0);
        render_page(&doc, &page, 1.0).unwrap()
    }

    #[test]
    fn a_red_square_bottom_left() {
        let c = draw("1 0 0 rg 0 0 50 50 re f", "<< >>");
        assert_eq!((c.width, c.height), (100, 100));
        assert_eq!(c.pixel(10, 90), [255, 0, 0, 255], "y up: the square sits at the bottom");
        assert_eq!(c.pixel(10, 10), [255, 255, 255, 255]);
    }

    #[test]
    fn the_stack_and_the_clip() {
        // A clip to the left half inside q/Q, a fill under it, then a full fill after Q.
        let c = draw("q 0 0 50 100 re W n 0 g 0 0 100 50 re f Q 0 0 1 rg 0 60 100 40 re f", "<< >>");
        assert_eq!(c.pixel(25, 75), [0, 0, 0, 255], "inside the clip");
        assert_eq!(c.pixel(75, 75), [255, 255, 255, 255], "clipped away");
        assert_eq!(c.pixel(75, 20), [0, 0, 255, 255], "after Q the clip is gone");
    }

    #[test]
    fn cmyk_and_transforms() {
        let c = draw("q 2 0 0 2 0 0 cm 0 1 1 0 k 0 0 10 10 re f Q", "<< >>");
        assert_eq!(c.pixel(15, 85), [255, 0, 0, 255], "scaled 2x: the 10-point square covers 20 pixels");
        assert_eq!(c.pixel(25, 85), [255, 255, 255, 255]);
    }

    #[test]
    fn an_axial_shading() {
        let res = "<< /Shading << /S << /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >> >> >>";
        let c = draw("/S sh", res);
        assert!(c.pixel(5, 50)[0] < 20, "dark at the left: {:?}", c.pixel(5, 50));
        assert!(c.pixel(95, 50)[0] > 235, "light at the right");
        let mid = c.pixel(50, 50)[0];
        assert!((118..=138).contains(&mid), "{mid}");
    }

    #[test]
    fn an_inline_image_and_a_form() {
        // A 2x1 RGB inline image (red, green) across the page, in ASCIIHex.
        let c = draw("q 100 0 0 100 0 0 cm BI /W 2 /H 1 /BPC 8 /CS /RGB /F /AHx ID FF0000 00FF00> EI Q", "<< >>");
        assert_eq!(c.pixel(10, 50), [255, 0, 0, 255]);
        assert_eq!(c.pixel(90, 50), [0, 255, 0, 255]);
        // A form that draws itself is cut, not followed forever.
        let doc_bytes = pdf("/F Do", "<< /XObject << /F 4 0 R >> >>");
        let doc = Document::open(doc_bytes).unwrap();
        let page = doc.pages().remove(0);
        let _ = render_page(&doc, &page, 1.0).unwrap();
    }

    #[test]
    fn rotated_pages_turn_clockwise() {
        // A mark at the top left of an unrotated page lands top right at /Rotate 90.
        let t = page_transform([0.0, 0.0, 100.0, 50.0], 90, 1.0);
        let p = t.apply(Point::new(0.0, 50.0));
        assert_eq!((p.x, p.y), (50.0, 0.0));
        let t = page_transform([0.0, 0.0, 100.0, 50.0], 270, 1.0);
        let p = t.apply(Point::new(0.0, 0.0));
        assert_eq!((p.x, p.y), (50.0, 100.0), "bottom left goes to the bottom right... of the turned page");
    }

    #[test]
    fn radial_positions() {
        // Concentric circles r 0..10: a point 5 away is halfway.
        let s = radial(&[0.0, 0.0, 0.0, 0.0, 0.0, 10.0], Point::new(5.0, 0.0), false, false).unwrap();
        assert!((s - 0.5).abs() < 1e-9);
        assert_eq!(radial(&[0.0, 0.0, 0.0, 0.0, 0.0, 10.0], Point::new(20.0, 0.0), false, false), None);
        assert_eq!(radial(&[0.0, 0.0, 0.0, 0.0, 0.0, 10.0], Point::new(20.0, 0.0), false, true), Some(1.0));
    }
}
