//! Laying a tree of elements out: each gets its rectangle, in reference
//! units, in a flat list in drawing order (a parent before its children).

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

/// Room kept on the right of a scrolled child for the scroll bar (ui.rs
/// draws it 6 wide, 2 in from the edge).
pub(crate) const SCROLL_GUTTER: f32 = 10.0;

use vergence_scene::{Color, Image, Rect};

use crate::element::{
    Align, ButtonLook, Draw, Element, FIELD_PAD_H, FIELD_PAD_V, Fit, Kind, Label, Length, OnTouch, Weight,
};

/// A widget's name between builds: its place in the tree, or the key its
/// program gave it, under its parent's id.
pub type Id = u64;

/// How text is measured: by the fonts in a window, by a stand-in in tests.
pub trait Measure {
    /// The width of one line of `text`.
    fn width(&mut self, size: f32, weight: Weight, text: &str) -> f32;
    /// How far a line's tallest glyphs rise above its baseline, and the
    /// distance from one line's baseline to the next.
    fn line(&mut self, size: f32, weight: Weight) -> (f32, f32);
}

/// One line of a laid-out label, placed in the label's rectangle.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    pub x: f32,
    pub baseline: f32,
    pub width: f32,
}

pub(crate) enum What<M> {
    Space,
    Rule {
        color: Option<Color>,
    },
    Label {
        lines: Vec<Line>,
        size: f32,
        weight: Weight,
        color: Option<Color>,
        dim: bool,
    },
    Panel {
        fill: Option<Color>,
        edge: Option<(Color, f32)>,
        radius: f32,
        clip: bool,
    },
    Button {
        on_press: Option<M>,
        look: ButtonLook,
        on_drag: Option<M>,
    },
    Group,
    Switch {
        on: bool,
        on_toggle: Option<Box<dyn Fn(bool) -> M>>,
    },
    Slider {
        min: f32,
        max: f32,
        step: f32,
        value: f32,
        marks: Vec<f32>,
        colors: Option<crate::element::SliderColors>,
        on_change: Option<Box<dyn Fn(f32) -> M>>,
        on_release: Option<M>,
    },
    /// `baseline`: of the text, below the field's top.
    Field {
        value: String,
        placeholder: String,
        secret: bool,
        autofocus: bool,
        on_input: Option<Box<dyn Fn(String) -> M>>,
        on_submit: Option<M>,
        size: f32,
        baseline: f32,
    },
    Choice {
        options: Vec<String>,
        selected: Option<usize>,
        placeholder: String,
        on_select: Option<Box<dyn Fn(usize) -> M>>,
        size: f32,
        baseline: f32,
        step: f32,
    },
    /// `content`: the height of what is in it.
    Scroll {
        content: f32,
    },
    Picture {
        image: Arc<Image>,
        fit: Fit,
        radius: f32,
        opacity: f32,
    },
    Drawing {
        draw: Draw,
        on_touch: Option<OnTouch<M>>,
        cursor: Option<vergence_ui::Cursor>,
    },
    Area {
        doc: crate::textarea::TextDoc,
        on_change: Option<M>,
        autofocus: bool,
    },
}

pub(crate) struct Node<M> {
    pub id: Id,
    pub rect: Rect,
    pub parent: Option<usize>,
    pub what: What<M>,
    pub name: Option<String>,
    /// A drawing's value ([`crate::Element::value`]).
    pub value: Option<String>,
    /// Sent on a right click ([`crate::Element::on_menu`]).
    pub on_menu: Option<M>,
    /// Scrolled into view when it first appears ([`crate::Element::keep_in_view`]).
    pub reveal: bool,
    /// Shown only while the pointer is on the button around it ([`crate::Element::shown_on_hover`]).
    pub hover_only: bool,
    /// The pointer over it, if not the usual ([`crate::Element::cursor`]).
    pub pointer: Option<vergence_ui::Cursor>,
}

pub(crate) fn child_id(parent: Id, index: usize, key: Option<u64>) -> Id {
    // splitmix64 of the parent's id and the child's place or key; a key is
    // kept apart from places by its top bit.
    let tag = match key {
        Some(k) => k | (1 << 63),
        None => index as u64,
    };
    // The +1 and the constant keep a first child from sharing its parent's id
    // (zero would hash to zero).
    let mut z = (parent ^ 0x2545_f491_4f6c_dd1d)
        .rotate_left(17)
        .wrapping_add(tag.wrapping_add(1).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub(crate) struct Layout<'a> {
    pub measure: &'a mut dyn Measure,
    /// The size of text that does not name one.
    pub text_size: f32,
    /// The screen's text factor, for sizes a label names (Ui::set_text_factor).
    pub factor: f32,
    /// A label's lines, remembered for this layout pass: containers measure
    /// their children more than once, and nested ones multiply that, so
    /// without it a long wrapped text was broken into lines dozens of times
    /// (the settings app's License page took 6 s to lay out).
    pub lines_seen: HashMap<LinesKey, (Vec<Line>, f32)>,
    /// How far each scrolling place is scrolled (Ui's), so a grid in one
    /// builds only the rows in view.
    pub offsets: HashMap<Id, f32>,
    /// The span, in this layout's coordinates, of the scrolling place being
    /// laid out that is worth building: what shows and a screen either side.
    pub window: Option<(f32, f32)>,
    /// A grid was laid out inside the scrolling place being laid out.
    pub grid_seen: bool,
    /// Each scrolling place holding a grid, and the span of its content
    /// (from its top) built this time: scrolling outside it lays out again.
    pub built: Vec<(Id, f32, f32)>,
}

/// A grid's rows, the height of each and where the element after a row goes.
fn grid_rows(count: usize, cols: usize, cell_h: f32, gap_y: f32, after: Option<(usize, f32)>) -> (usize, f32) {
    let rows = count.div_ceil(cols.max(1));
    let mut h = rows as f32 * cell_h + rows.saturating_sub(1) as f32 * gap_y;
    if let Some((row, extra)) = after
        && row < rows
    {
        h += extra + gap_y;
    }
    (rows, h)
}

/// What a label's line breaks depend on: its text, size, weight and the width.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct LinesKey {
    text: u64,
    size: u32,
    weight: Weight,
    width: Option<u32>,
    wrap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Size {
    w: f32,
    h: f32,
}

impl Layout<'_> {
    /// Lays `root` out in `area`, appending to `out`.
    pub fn run<M>(&mut self, root: Element<M>, area: Rect, out: &mut Vec<Node<M>>) {
        let natural = self.measure(&root, area.w);
        let w = clamp_w(&root, resolve(root.width, area.w, natural.w));
        let h = resolve(root.height, area.h, self.measure(&root, w).h);
        self.arrange(root, Rect::new(area.x, area.y, w, h), None, child_id(0, 0, None), out);
    }

    fn lines(&mut self, l: &Label, width: Option<f32>) -> (Vec<Line>, f32) {
        let size = l.size.map_or(self.text_size, |s| vergence_look::text_size(s, self.factor));
        let key = LinesKey {
            text: {
                let mut h = DefaultHasher::new();
                l.text.hash(&mut h);
                h.finish()
            },
            size: size.to_bits(),
            weight: l.weight,
            width: width.filter(|_| l.wrap).map(f32::to_bits),
            wrap: l.wrap,
        };
        if let Some(seen) = self.lines_seen.get(&key) {
            return seen.clone();
        }
        let (ascent, step) = self.measure.line(size, l.weight);
        let mut lines: Vec<(String, f32)> = Vec::new();
        for para in l.text.split('\n') {
            match width.filter(|_| l.wrap) {
                None => lines.push((para.to_string(), self.measure.width(size, l.weight, para))),
                Some(max) => {
                    // Each word is measured once and the widths added (word, space,
                    // word...): measuring the whole line again for every word was
                    // quadratic in a paragraph's length. A finished line is measured
                    // once whole, so its width (for centering) includes kerning.
                    let space = self.measure.width(size, l.weight, " ");
                    let mut cur = String::new();
                    let mut cur_w = 0.0;
                    for word in para.split(' ').filter(|w| !w.is_empty()) {
                        let word_w = self.measure.width(size, l.weight, word);
                        let trial_w = if cur.is_empty() { word_w } else { cur_w + space + word_w };
                        if trial_w > max && !cur.is_empty() {
                            let done = std::mem::take(&mut cur);
                            let done_w = self.measure.width(size, l.weight, &done);
                            lines.push((done, done_w));
                            (cur, cur_w) = (word.to_string(), word_w);
                        } else {
                            if !cur.is_empty() {
                                cur.push(' ');
                            }
                            cur.push_str(word);
                            cur_w = trial_w;
                        }
                    }
                    let last_w = if cur.is_empty() { 0.0 } else { self.measure.width(size, l.weight, &cur) };
                    lines.push((cur, last_w));
                }
            }
        }
        let out = lines
            .into_iter()
            .enumerate()
            .map(|(i, (text, w))| Line { text, x: 0.0, baseline: (ascent + i as f32 * step).round(), width: w })
            .collect::<Vec<_>>();
        let height = (out.len() as f32 * step).ceil();
        self.lines_seen.insert(key, (out.clone(), height));
        (out, height)
    }

    /// The size `e` would like, with at most `avail` across.
    fn measure<M>(&mut self, e: &Element<M>, avail: f32) -> Size {
        let avail = e.max_width.map_or(avail, |m| avail.min(m));
        let avail = match e.width {
            Length::Fixed(w) => w,
            _ => avail,
        };
        let content = match &e.kind {
            Kind::Space => Size { w: 0.0, h: 0.0 },
            Kind::Rule { thickness, .. } => Size { w: *thickness, h: *thickness },
            Kind::Label(l) => {
                let (lines, h) = self.lines(l, Some(avail));
                Size { w: lines.iter().map(|l| l.width).fold(0.0, f32::max).ceil(), h }
            }
            Kind::Panel { child, padding, .. } | Kind::Button { child, padding, .. } => {
                let inner = (avail - padding.width()).max(0.0);
                let c = self.measure(child, inner);
                let cw = match child.width {
                    Length::Fixed(w) => w,
                    _ => c.w,
                };
                let ch = match child.height {
                    Length::Fixed(h) => h,
                    _ => c.h,
                };
                Size { w: cw + padding.width(), h: ch + padding.height() }
            }
            Kind::Row { children, spacing, wrap: true, .. } => {
                let lines = self.flow(children, avail, *spacing);
                let w = lines.iter().map(|l| l.width).fold(0.0, f32::max);
                let h = lines.iter().map(|l| l.height).sum::<f32>() + gaps(lines.len(), *spacing);
                Size { w, h }
            }
            Kind::Row { children, spacing, .. } => {
                let natural: f32 = children.iter().map(|c| self.main_natural(c, avail, true)).sum::<f32>()
                    + gaps(children.len(), *spacing);
                let room = if e.width == Length::Shrink { natural.min(avail) } else { avail };
                let widths = self.distribute(children, room, *spacing, true, avail);
                let h = children.iter().zip(&widths).map(|(c, w)| self.cross(c, *w)).fold(0.0, f32::max);
                Size { w: natural, h }
            }
            Kind::Column { children, spacing, .. } => {
                let mut w: f32 = 0.0;
                let mut h = gaps(children.len(), *spacing);
                for c in children {
                    let s = self.measure(c, avail);
                    let cw = clamp_w(
                        c,
                        match c.width {
                            Length::Fixed(f) => f,
                            _ => s.w,
                        },
                    );
                    w = w.max(cw);
                    h += match c.height {
                        Length::Fixed(f) => f,
                        _ => self.measure(c, cw.min(avail)).h,
                    };
                }
                Size { w, h }
            }
            Kind::Stack { children } => children.iter().fold(Size { w: 0.0, h: 0.0 }, |a, c| {
                let s = self.measure(c, avail);
                Size { w: a.w.max(s.w), h: a.h.max(s.h) }
            }),
            Kind::Switch { .. } => Size { w: crate::element::SWITCH_W, h: crate::element::SWITCH_H },
            Kind::Slider { .. } => Size { w: 200.0, h: crate::element::SLIDER_H },
            Kind::Field { .. } => Size { w: 120.0, h: self.field_height() },
            Kind::Choice { options, .. } => {
                let widest = options.iter().map(|o| self.measure.width(self.text_size, 400, o)).fold(0.0, f32::max);
                Size { w: (widest + 2.0 * FIELD_PAD_H + CHEVRON).ceil(), h: self.field_height() }
            }
            Kind::Scroll { child } => self.measure(child, avail),
            Kind::Grid { count, fit, gap, after, .. } => {
                let (cols, _, cell_h) = fit(avail);
                let after = after.as_ref().map(|a| (a.0 / cols.max(1), a.1));
                Size { w: avail, h: grid_rows(*count, cols, cell_h, gap.1, after).1 }
            }
            Kind::Picture { .. } | Kind::Drawing { .. } | Kind::Area { .. } => Size { w: 0.0, h: 0.0 },
        };
        Size {
            w: clamp_w(
                e,
                match e.width {
                    Length::Fixed(w) => w,
                    _ => content.w,
                },
            ),
            h: match e.height {
                Length::Fixed(h) => h,
                _ => content.h,
            },
        }
    }

    /// A wrapping row's children in lines no wider than `avail` (a child
    /// wider than that has a line of its own), each child at its natural size.
    fn flow<M>(&mut self, children: &[Element<M>], avail: f32, spacing: f32) -> Vec<FlowLine> {
        let mut lines: Vec<FlowLine> = Vec::new();
        for c in children {
            let s = self.measure(c, avail);
            let w = match c.width {
                Length::Fixed(v) => v,
                _ => s.w.min(avail),
            };
            let h = match c.height {
                Length::Fixed(v) => v,
                _ => self.measure(c, w).h,
            };
            match lines.last_mut() {
                Some(l) if l.width + spacing + w <= avail + 0.01 => {
                    l.width += spacing + w;
                    l.height = l.height.max(h);
                    l.sizes.push((w, h));
                }
                _ => lines.push(FlowLine { width: w, height: h, sizes: vec![(w, h)] }),
            }
        }
        lines
    }

    fn field_height(&mut self) -> f32 {
        (self.measure.line(self.text_size, 400).1 + 2.0 * FIELD_PAD_V).ceil()
    }

    fn field_baseline(&mut self, h: f32) -> f32 {
        let (ascent, step) = self.measure.line(self.text_size, 400);
        ((h - step) / 2.0 + ascent).round()
    }

    /// A child's natural length along a row (`across`) or a column.
    fn main_natural<M>(&mut self, c: &Element<M>, avail: f32, across: bool) -> f32 {
        let len = if across { c.width } else { c.height };
        match len {
            Length::Fixed(v) => v,
            _ => {
                let s = self.measure(c, avail);
                if across { s.w } else { s.h }
            }
        }
    }

    /// A row child's height at width `w`.
    fn cross<M>(&mut self, c: &Element<M>, w: f32) -> f32 {
        match c.height {
            Length::Fixed(h) => h,
            _ => self.measure(c, w).h,
        }
    }

    /// Shares `room` along a row (`across`) or a column among `children`:
    /// fixed and shrinking ones first, then what is left by weight among
    /// the filling ones. `avail` is the width the children may measure in
    /// (a column's children are measured at their own widths beforehand).
    fn distribute<M>(
        &mut self,
        children: &[Element<M>],
        room: f32,
        spacing: f32,
        across: bool,
        avail: f32,
    ) -> Vec<f32> {
        let mut out = vec![0.0; children.len()];
        let mut used = gaps(children.len(), spacing);
        let mut weights = 0u32;
        for (i, c) in children.iter().enumerate() {
            let len = if across { c.width } else { c.height };
            match len {
                Length::Fill(w) => weights += w.max(1) as u32,
                _ => {
                    out[i] = self.main_natural(c, avail, across);
                    if across {
                        out[i] = clamp_w(c, out[i]);
                    }
                    used += out[i];
                }
            }
        }
        let left = (room - used).max(0.0);
        if weights > 0 {
            for (i, c) in children.iter().enumerate() {
                if let Length::Fill(w) = if across { c.width } else { c.height } {
                    out[i] = left * w.max(1) as f32 / weights as f32;
                    if across {
                        out[i] = clamp_w(c, out[i]);
                    }
                }
            }
        }
        out
    }

    fn arrange<M>(&mut self, e: Element<M>, rect: Rect, parent: Option<usize>, id: Id, out: &mut Vec<Node<M>>) {
        let me = out.len();
        let name = e.name;
        let value = e.value;
        let reveal = e.reveal;
        let hover_only = e.hover_only;
        let pointer = e.pointer;
        let on_menu = e.on_menu;
        let push = |out: &mut Vec<Node<M>>, what: What<M>| {
            out.push(Node {
                id,
                rect,
                parent,
                what,
                name: name.clone(),
                value: value.clone(),
                on_menu: None,
                reveal,
                hover_only,
                pointer,
            })
        };
        // Each element is one node, pushed before its children.
        self.arrange_kind(e.kind, rect, me, id, push, out);
        if let Some(n) = out.get_mut(me) {
            n.on_menu = on_menu;
        }
    }

    fn arrange_kind<M>(
        &mut self,
        kind: Kind<M>,
        rect: Rect,
        me: usize,
        id: Id,
        push: impl Fn(&mut Vec<Node<M>>, What<M>),
        out: &mut Vec<Node<M>>,
    ) {
        match kind {
            Kind::Space => push(out, What::Space),
            Kind::Rule { color, .. } => push(out, What::Rule { color }),
            Kind::Label(l) => {
                let (mut lines, _) = self.lines(&l, Some(rect.w));
                // Cut to its place, "…" at the end, characters taken off until it fits.
                if l.elide && !l.wrap && lines.len() == 1 && lines[0].width > rect.w + 0.5 {
                    let size = l.size.map_or(self.text_size, |s| vergence_look::text_size(s, self.factor));
                    let chars: Vec<char> = lines[0].text.chars().collect();
                    let (mut lo, mut hi) = (0usize, chars.len());
                    while lo < hi {
                        let mid = (lo + hi).div_ceil(2);
                        let t: String = chars[..mid].iter().collect::<String>() + "…";
                        if self.measure.width(size, l.weight, &t) <= rect.w { lo = mid } else { hi = mid - 1 }
                    }
                    let t: String = chars[..lo].iter().collect::<String>().trim_end().to_string() + "…";
                    lines[0].width = self.measure.width(size, l.weight, &t);
                    lines[0].text = t;
                }
                for line in &mut lines {
                    line.x = match l.align {
                        Align::Start => 0.0,
                        Align::Center => ((rect.w - line.width) / 2.0).round(),
                        Align::End => (rect.w - line.width).round(),
                    };
                }
                let size = l.size.map_or(self.text_size, |s| vergence_look::text_size(s, self.factor));
                push(out, What::Label { lines, size, weight: l.weight, color: l.color, dim: l.dim });
            }
            Kind::Panel { child, padding, fill, edge, radius, align_x, align_y, clip } => {
                push(out, What::Panel { fill, edge, radius, clip });
                let inner = Rect::new(
                    rect.x + padding.left,
                    rect.y + padding.top,
                    (rect.w - padding.width()).max(0.0),
                    (rect.h - padding.height()).max(0.0),
                );
                self.place_one(*child, inner, align_x, align_y, me, id, out);
            }
            Kind::Button { child, padding, on_press, look, on_drag } => {
                push(out, What::Button { on_press, look, on_drag });
                let inner = Rect::new(
                    rect.x + padding.left,
                    rect.y + padding.top,
                    (rect.w - padding.width()).max(0.0),
                    (rect.h - padding.height()).max(0.0),
                );
                self.place_one(*child, inner, Align::Center, Align::Center, me, id, out);
            }
            Kind::Row { children, spacing, align, wrap: true } => {
                push(out, What::Group);
                let lines = self.flow(&children, rect.w, spacing);
                let mut sizes: Vec<(f32, f32)> =
                    lines.iter().flat_map(|l| l.sizes.clone()).collect::<Vec<_>>().into_iter().rev().collect();
                let mut y = rect.y;
                let mut iter = children.into_iter().enumerate();
                for line in &lines {
                    let mut x = rect.x;
                    for _ in 0..line.sizes.len() {
                        let Some((i, c)) = iter.next() else { break };
                        let (w, h) = sizes.pop().unwrap_or((0.0, 0.0));
                        let cy = y + offset(align, line.height, h);
                        let cid = child_id(id, i, c.key);
                        self.arrange(c, Rect::new(x, cy, w, h), Some(me), cid, out);
                        x += w + spacing;
                    }
                    y += line.height + spacing;
                }
            }
            Kind::Row { children, spacing, align, .. } => {
                push(out, What::Group);
                let widths = self.distribute(&children, rect.w, spacing, true, rect.w);
                let mut x = rect.x;
                for (i, (c, w)) in children.into_iter().zip(widths).enumerate() {
                    let h = match c.height {
                        Length::Fill(_) => rect.h,
                        Length::Fixed(h) => h,
                        Length::Shrink => self.measure(&c, w).h.min(rect.h),
                    };
                    let y = rect.y + offset(align, rect.h, h);
                    let cid = child_id(id, i, c.key);
                    self.arrange(c, Rect::new(x, y, w, h), Some(me), cid, out);
                    x += w + spacing;
                }
            }
            Kind::Column { children, spacing, align } => {
                push(out, What::Group);
                let widths: Vec<f32> = children
                    .iter()
                    .map(|c| {
                        clamp_w(
                            c,
                            match c.width {
                                Length::Fill(_) => rect.w,
                                Length::Fixed(w) => w,
                                Length::Shrink => self.measure(c, rect.w).w.min(rect.w),
                            },
                        )
                    })
                    .collect();
                // Heights at each child's own width.
                let mut heights = vec![0.0; children.len()];
                let mut used = gaps(children.len(), spacing);
                let mut weights = 0u32;
                for (i, c) in children.iter().enumerate() {
                    match c.height {
                        Length::Fill(w) => weights += w.max(1) as u32,
                        Length::Fixed(h) => heights[i] = h,
                        Length::Shrink => heights[i] = self.measure(c, widths[i]).h,
                    }
                    used += heights[i];
                }
                let left = (rect.h - used).max(0.0);
                for (i, c) in children.iter().enumerate() {
                    if let Length::Fill(w) = c.height {
                        heights[i] = left * w.max(1) as f32 / weights.max(1) as f32;
                    }
                }
                let mut y = rect.y;
                for (i, c) in children.into_iter().enumerate() {
                    let x = rect.x + offset(align, rect.w, widths[i]);
                    let cid = child_id(id, i, c.key);
                    self.arrange(c, Rect::new(x, y, widths[i], heights[i]), Some(me), cid, out);
                    y += heights[i] + spacing;
                }
            }
            Kind::Stack { children } => {
                push(out, What::Group);
                for (i, c) in children.into_iter().enumerate() {
                    let cid = child_id(id, i, c.key);
                    self.place_one_id(c, rect, Align::Start, Align::Start, me, cid, out);
                }
            }
            Kind::Switch { on, on_toggle } => push(out, What::Switch { on, on_toggle }),
            Kind::Slider { min, max, step, value, marks, colors, on_change, on_release } => {
                push(out, What::Slider { min, max, step, value, marks, colors, on_change, on_release })
            }
            Kind::Field { value, placeholder, secret, autofocus, on_input, on_submit } => {
                let baseline = self.field_baseline(rect.h);
                push(
                    out,
                    What::Field {
                        value,
                        placeholder,
                        secret,
                        autofocus,
                        on_input,
                        on_submit,
                        size: self.text_size,
                        baseline,
                    },
                );
            }
            Kind::Choice { options, selected, placeholder, on_select } => {
                let baseline = self.field_baseline(rect.h);
                let step = self.measure.line(self.text_size, 400).1;
                push(
                    out,
                    What::Choice { options, selected, placeholder, on_select, size: self.text_size, baseline, step },
                );
            }
            Kind::Picture { image, fit, radius, opacity } => push(out, What::Picture { image, fit, radius, opacity }),
            Kind::Drawing { draw, on_touch, cursor } => push(out, What::Drawing { draw, on_touch, cursor }),
            Kind::Area { doc, on_change, autofocus } => push(out, What::Area { doc, on_change, autofocus }),
            Kind::Grid { count, fit, gap, build, after } => {
                push(out, What::Group);
                self.grid_seen = true;
                let (cols, cell_w, cell_h) = fit(rect.w);
                let cols = cols.max(1);
                let rows = count.div_ceil(cols);
                let after_row = after.as_ref().map(|a| (a.0 / cols, a.1));
                // A row's top: the element after a row pushes those below it down.
                let top = |r: usize| {
                    let mut y = rect.y + r as f32 * (cell_h + gap.1);
                    if let Some((ar, extra)) = after_row
                        && r > ar
                    {
                        y += extra + gap.1;
                    }
                    y
                };
                let (lo, hi) = self.window.unwrap_or((f32::NEG_INFINITY, f32::INFINITY));
                for r in 0..rows {
                    let y = top(r);
                    if y + cell_h < lo {
                        continue;
                    }
                    if y > hi {
                        break;
                    }
                    for c in 0..cols {
                        let i = r * cols + c;
                        if i >= count {
                            break;
                        }
                        let cell = build(i, cell_w);
                        let cid = child_id(id, i, cell.key);
                        let x = rect.x + c as f32 * (cell_w + gap.0);
                        self.arrange(cell, Rect::new(x, y, cell_w, cell_h), Some(me), cid, out);
                    }
                }
                if let Some((item, extra, e)) = after
                    && item / cols < rows
                {
                    let ar = item / cols;
                    let y = top(ar) + cell_h + gap.1;
                    let cid = child_id(id, count + 1, e.key);
                    self.arrange(*e, Rect::new(rect.x, y, rect.w, extra), Some(me), cid, out);
                }
            }
            Kind::Scroll { child } => {
                push(out, What::Scroll { content: rect.h });
                // What is worth building of a grid inside: what shows and a
                // screen either side (scrolled further, it is laid out again).
                let offset = self.offsets.get(&id).copied().unwrap_or(0.0).max(0.0);
                let window = (rect.y + offset - rect.h, rect.y + offset + 2.0 * rect.h);
                let (outer_window, outer_seen) = (self.window.replace(window), std::mem::take(&mut self.grid_seen));
                // The child as tall as it needs (or the place, if more), at the place's width.
                let w = clamp_w(
                    &child,
                    match child.width {
                        Length::Fixed(w) => w,
                        Length::Shrink => self.measure(&child, rect.w).w.min(rect.w),
                        Length::Fill(_) => rect.w,
                    },
                );
                let mut natural = match child.height {
                    Length::Fixed(h) => h,
                    _ => self.measure(&child, w).h,
                };
                // Taller than its place: the scroll bar shows, and a child as
                // wide as the place leaves it a gutter on the right, so nothing
                // sits under the bar (see docs/settings-milestone-2.md).
                let mut w = w;
                if natural > rect.h && matches!(child.width, Length::Fill(_)) {
                    w = (w - SCROLL_GUTTER).max(0.0);
                    if !matches!(child.height, Length::Fixed(_)) {
                        natural = self.measure(&child, w).h;
                    }
                }
                let h = natural.max(rect.h);
                let cid = child_id(id, 0, child.key);
                self.arrange(*child, Rect::new(rect.x, rect.y, w, h), Some(me), cid, out);
                if let What::Scroll { content } = &mut out[me].what {
                    *content = h;
                }
                if self.grid_seen {
                    self.built.push((id, window.0 - rect.y, window.1 - rect.y));
                }
                self.window = outer_window;
                self.grid_seen = outer_seen;
            }
        }
    }

    /// Places the one child of a panel or button in `inner`.
    fn place_one<M>(
        &mut self,
        child: Element<M>,
        inner: Rect,
        ax: Align,
        ay: Align,
        parent: usize,
        parent_id: Id,
        out: &mut Vec<Node<M>>,
    ) {
        let cid = child_id(parent_id, 0, child.key);
        self.place_one_id(child, inner, ax, ay, parent, cid, out);
    }

    fn place_one_id<M>(
        &mut self,
        child: Element<M>,
        inner: Rect,
        ax: Align,
        ay: Align,
        parent: usize,
        cid: Id,
        out: &mut Vec<Node<M>>,
    ) {
        let natural = self.measure(&child, inner.w);
        let w = clamp_w(
            &child,
            match child.width {
                Length::Fill(_) => inner.w,
                Length::Fixed(w) => w,
                Length::Shrink => natural.w.min(inner.w),
            },
        );
        let h = match child.height {
            Length::Fill(_) => inner.h,
            Length::Fixed(h) => h,
            Length::Shrink => self.measure(&child, w).h,
        };
        let r = Rect::new(inner.x + offset(ax, inner.w, w), inner.y + offset(ay, inner.h, h), w, h);
        self.arrange(child, r, Some(parent), cid, out);
    }
}

struct FlowLine {
    width: f32,
    height: f32,
    sizes: Vec<(f32, f32)>,
}

/// Room for a choice's arrow.
pub(crate) const CHEVRON: f32 = 18.0;

fn gaps(n: usize, spacing: f32) -> f32 {
    n.saturating_sub(1) as f32 * spacing
}

fn offset(align: Align, room: f32, len: f32) -> f32 {
    match align {
        Align::Start => 0.0,
        Align::Center => ((room - len) / 2.0).max(0.0).round(),
        Align::End => (room - len).max(0.0),
    }
}

fn resolve(len: Length, room: f32, natural: f32) -> f32 {
    match len {
        Length::Fill(_) => room,
        Length::Fixed(v) => v,
        Length::Shrink => natural.min(room),
    }
}

fn clamp_w<M>(e: &Element<M>, w: f32) -> f32 {
    e.max_width.map_or(w, |m| w.min(m))
}
