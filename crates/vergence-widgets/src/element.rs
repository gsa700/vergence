//! What a program builds: a tree of elements, from its state.

use std::sync::Arc;

use vergence_scene::{Color, Image, Rect, Scene};
use vergence_text::Fonts;

use crate::theme::Theme;

/// How much room an element takes along one axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// As much as its content needs.
    Shrink,
    /// Exactly this many reference units.
    Fixed(f32),
    /// A share of what is left, by weight, after the others have theirs.
    Fill(u16),
}

impl Length {
    pub const FILL: Length = Length::Fill(1);
}

/// Where something smaller than its place sits in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

/// Space inside an element's edges.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Padding {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Padding {
    pub const fn all(p: f32) -> Padding {
        Padding { top: p, right: p, bottom: p, left: p }
    }

    /// `v` above and below, `h` left and right.
    pub const fn hv(v: f32, h: f32) -> Padding {
        Padding { top: v, right: h, bottom: v, left: h }
    }

    pub(crate) fn width(&self) -> f32 {
        self.left + self.right
    }

    pub(crate) fn height(&self) -> f32 {
        self.top + self.bottom
    }
}

impl From<f32> for Padding {
    fn from(p: f32) -> Padding {
        Padding::all(p)
    }
}

/// A button's looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ButtonLook {
    /// An edge, the page's ground; the hover tint under the pointer.
    Normal,
    /// The accent's ground and white text: the one thing to press.
    Accent,
    /// Nothing until the pointer is on it: a list's item.
    Plain,
    /// Chosen among others (a selected page, a selected side): the accent's
    /// ground and white text, as [`ButtonLook::Accent`], without its edge.
    Chosen,
    /// The program's own: a ground, an edge (colour, width), a radius.
    Custom { fill: Option<Color>, edge: Option<(Color, f32)>, radius: f32 },
}

/// A text's weight: 400 regular, 600 semibold, 700 bold.
pub type Weight = u16;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Label {
    pub text: String,
    pub size: Option<f32>,
    pub weight: Weight,
    pub color: Option<Color>,
    pub dim: bool,
    pub align: Align,
    pub wrap: bool,
}

/// What a [drawing](drawing) is told of the pointer or a finger, in its own
/// coordinates. A press holds the drawing's attention until it is let go.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Touch {
    Press { x: f32, y: f32 },
    Move { x: f32, y: f32, pressed: bool },
    Release { x: f32, y: f32 },
    /// The pointer left, or the system took a finger back.
    Leave,
}

/// How a picture fills its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// All of it, as large as fits; the rest of the place is left.
    #[default]
    Contain,
    /// The whole place, the picture's edges cut off as needed.
    Cover,
    /// Stretched to the place.
    Stretch,
}

pub(crate) type Draw = Box<dyn Fn(&mut Scene, &mut Fonts, Rect, &Theme)>;
pub(crate) type OnTouch<M> = Box<dyn Fn(Touch) -> Option<M>>;

pub(crate) enum Kind<M> {
    Space,
    Rule { color: Option<Color>, thickness: f32 },
    Label(Label),
    Panel { child: Box<Element<M>>, padding: Padding, fill: Option<Color>, edge: Option<(Color, f32)>, radius: f32, align_x: Align, align_y: Align },
    Button { child: Box<Element<M>>, padding: Padding, on_press: Option<M>, look: ButtonLook },
    Row { children: Vec<Element<M>>, spacing: f32, align: Align, wrap: bool },
    Column { children: Vec<Element<M>>, spacing: f32, align: Align },
    Stack { children: Vec<Element<M>> },
    Switch { on: bool, on_toggle: Option<Box<dyn Fn(bool) -> M>> },
    Slider { min: f32, max: f32, step: f32, value: f32, marks: Vec<f32>, on_change: Option<Box<dyn Fn(f32) -> M>>, on_release: Option<M> },
    Field { value: String, placeholder: String, secret: bool, on_input: Option<Box<dyn Fn(String) -> M>>, on_submit: Option<M> },
    Choice { options: Vec<String>, selected: Option<usize>, on_select: Option<Box<dyn Fn(usize) -> M>> },
    Scroll { child: Box<Element<M>> },
    Picture { image: Arc<Image>, fit: Fit, radius: f32, opacity: f32 },
    Drawing { draw: Draw, on_touch: Option<OnTouch<M>> },
}

/// One piece of a window: built from the program's state each time it
/// changes, laid out and drawn by [`crate::Ui`].
pub struct Element<M> {
    pub(crate) kind: Kind<M>,
    pub(crate) width: Length,
    pub(crate) height: Length,
    pub(crate) max_width: Option<f32>,
    pub(crate) key: Option<u64>,
    pub(crate) name: Option<String>,
}

impl<M> Element<M> {
    fn new(kind: Kind<M>, width: Length, height: Length) -> Element<M> {
        Element { kind, width, height, max_width: None, key: None, name: None }
    }

    pub fn width(mut self, width: Length) -> Self {
        self.width = width;
        self
    }

    pub fn height(mut self, height: Length) -> Self {
        self.height = height;
        self
    }

    /// Never wider than this, however much room there is.
    pub fn max_width(mut self, w: f32) -> Self {
        self.max_width = Some(w);
        self
    }

    /// Tells this element from its neighbours when a list of them changes,
    /// so what it remembers (focus, hover) stays with it, not with its place.
    pub fn key(mut self, key: u64) -> Self {
        self.key = Some(key);
        self
    }

    /// Its name to a screen reader, voice and automation, where its text
    /// does not say it (a colour swatch, a button that shows a symbol).
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    fn label_mut(&mut self) -> Option<&mut Label> {
        match &mut self.kind {
            Kind::Label(l) => Some(l),
            _ => None,
        }
    }

    /// A label's size, in reference units to the em (the theme's otherwise).
    pub fn size(mut self, size: f32) -> Self {
        if let Some(l) = self.label_mut() {
            l.size = Some(size);
        }
        self
    }

    pub fn weight(mut self, weight: Weight) -> Self {
        if let Some(l) = self.label_mut() {
            l.weight = weight;
        }
        self
    }

    /// A label's colour (else the ink of what it is in), a rule's, a panel's ground.
    pub fn color(mut self, color: Color) -> Self {
        match &mut self.kind {
            Kind::Label(l) => l.color = Some(color),
            Kind::Rule { color: c, .. } => *c = Some(color),
            Kind::Panel { fill, .. } => *fill = Some(color),
            _ => {}
        }
        self
    }

    /// A label in the theme's colour for text that explains.
    pub fn dim(mut self) -> Self {
        if let Some(l) = self.label_mut() {
            l.dim = true;
        }
        self
    }

    /// A label broken into lines at its width; a row's children carried
    /// on to a new line where they do not fit (a grid of tiles).
    pub fn wrap(mut self) -> Self {
        match &mut self.kind {
            Kind::Label(l) => l.wrap = true,
            Kind::Row { wrap, .. } => *wrap = true,
            _ => {}
        }
        self
    }

    /// A label's lines within its width; a row's children across its
    /// height; a column's across its width; a panel's child both ways.
    pub fn align(mut self, align: Align) -> Self {
        match &mut self.kind {
            Kind::Label(l) => l.align = align,
            Kind::Row { align: a, .. } | Kind::Column { align: a, .. } => *a = align,
            Kind::Panel { align_x, align_y, .. } => (*align_x, *align_y) = (align, align),
            _ => {}
        }
        self
    }

    /// A panel's child across, leaving it as it was up and down.
    pub fn align_x(mut self, align: Align) -> Self {
        if let Kind::Panel { align_x, .. } = &mut self.kind {
            *align_x = align;
        }
        self
    }

    pub fn align_y(mut self, align: Align) -> Self {
        if let Kind::Panel { align_y, .. } = &mut self.kind {
            *align_y = align;
        }
        self
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        if let Kind::Row { spacing: s, .. } | Kind::Column { spacing: s, .. } = &mut self.kind {
            *s = spacing;
        }
        self
    }

    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        if let Kind::Panel { padding: p, .. } | Kind::Button { padding: p, .. } = &mut self.kind {
            *p = padding.into();
        }
        self
    }

    /// A panel's edge: its colour and width.
    pub fn edge(mut self, color: Color, width: f32) -> Self {
        if let Kind::Panel { edge, .. } = &mut self.kind {
            *edge = Some((color, width));
        }
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        if let Kind::Panel { radius: r, .. } | Kind::Picture { radius: r, .. } = &mut self.kind {
            *r = radius;
        }
        self
    }

    /// What a button sends when it is pressed. A button without one is off.
    pub fn on_press(mut self, message: M) -> Self {
        if let Kind::Button { on_press, .. } = &mut self.kind {
            *on_press = Some(message);
        }
        self
    }

    pub fn look(mut self, look: ButtonLook) -> Self {
        if let Kind::Button { look: l, .. } = &mut self.kind {
            *l = look;
        }
        self
    }

    /// What a slider sends when it is let go (after its last change).
    pub fn on_release(mut self, message: M) -> Self {
        if let Kind::Slider { on_release, .. } = &mut self.kind {
            *on_release = Some(message);
        }
        self
    }

    /// A slider's step (0: any value).
    pub fn step(mut self, step: f32) -> Self {
        if let Kind::Slider { step: s, .. } = &mut self.kind {
            *s = step.max(0.0);
        }
        self
    }

    /// Values a slider marks under its track (and its thumb settles on
    /// when let go near one).
    pub fn marks(mut self, marks: Vec<f32>) -> Self {
        if let Kind::Slider { marks: m, .. } = &mut self.kind {
            *m = marks;
        }
        self
    }

    /// The text a field shows while it is empty.
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        if let Kind::Field { placeholder, .. } = &mut self.kind {
            *placeholder = text.into();
        }
        self
    }

    /// What a field sends when Enter is pressed in it.
    pub fn on_submit(mut self, message: M) -> Self {
        if let Kind::Field { on_submit, .. } = &mut self.kind {
            *on_submit = Some(message);
        }
        self
    }

    /// A field whose text is never shown, copied or told (Contract 3):
    /// a password's.
    pub fn secret(mut self) -> Self {
        if let Kind::Field { secret, .. } = &mut self.kind {
            *secret = true;
        }
        self
    }

    /// How a picture fills its place.
    pub fn fit(mut self, fit: Fit) -> Self {
        if let Kind::Picture { fit: f, .. } = &mut self.kind {
            *f = fit;
        }
        self
    }

    /// A picture's opacity, 0 to 1.
    pub fn opacity(mut self, opacity: f32) -> Self {
        if let Kind::Picture { opacity: o, .. } = &mut self.kind {
            *o = opacity.clamp(0.0, 1.0);
        }
        self
    }

    /// Adds a child to a row, a column or a stack.
    pub fn push(mut self, child: Element<M>) -> Self {
        if let Kind::Row { children, .. } | Kind::Column { children, .. } | Kind::Stack { children } = &mut self.kind {
            children.push(child);
        }
        self
    }
}

/// Nothing: room, as much as it is given.
pub fn space<M>() -> Element<M> {
    Element::new(Kind::Space, Length::Shrink, Length::Shrink)
}

/// A line across a column, or down a row: the theme's line colour, one unit.
pub fn rule<M>() -> Element<M> {
    Element::new(Kind::Rule { color: None, thickness: 1.0 }, Length::FILL, Length::Fixed(1.0))
}

/// A line down a row.
pub fn vrule<M>() -> Element<M> {
    Element::new(Kind::Rule { color: None, thickness: 1.0 }, Length::Fixed(1.0), Length::FILL)
}

/// Text: one line unless it [wraps](Element::wrap).
pub fn label<M>(text: impl Into<String>) -> Element<M> {
    Element::new(Kind::Label(Label { text: text.into(), size: None, weight: 400, color: None, dim: false, align: Align::Start, wrap: false }), Length::Shrink, Length::Shrink)
}

/// A box around one child: a ground, an edge, rounded corners, padding.
pub fn panel<M>(child: Element<M>) -> Element<M> {
    Element::new(
        Kind::Panel { child: Box::new(child), padding: Padding::default(), fill: None, edge: None, radius: vergence_look::RADIUS, align_x: Align::Start, align_y: Align::Start },
        Length::Shrink,
        Length::Shrink,
    )
}

/// Something to press, around any child (usually a label).
pub fn button<M>(child: Element<M>) -> Element<M> {
    Element::new(Kind::Button { child: Box::new(child), padding: Padding::hv(5.0, 14.0), on_press: None, look: ButtonLook::Normal }, Length::Shrink, Length::Shrink)
}

/// Children side by side.
pub fn row<M>(children: Vec<Element<M>>) -> Element<M> {
    Element::new(Kind::Row { children, spacing: 0.0, align: Align::Start, wrap: false }, Length::Shrink, Length::Shrink)
}

/// Children one under another.
pub fn column<M>(children: Vec<Element<M>>) -> Element<M> {
    Element::new(Kind::Column { children, spacing: 0.0, align: Align::Start }, Length::Shrink, Length::Shrink)
}

/// Children over one another, the last in front, each the stack's size.
pub fn stack<M>(children: Vec<Element<M>>) -> Element<M> {
    Element::new(Kind::Stack { children }, Length::Shrink, Length::Shrink)
}

/// On or off. Without `on_toggle` it is off and cannot be changed.
pub fn switch<M>(on: bool, on_toggle: impl Fn(bool) -> M + 'static) -> Element<M> {
    Element::new(Kind::Switch { on, on_toggle: Some(Box::new(on_toggle)) }, Length::Fixed(SWITCH_W), Length::Fixed(SWITCH_H))
}

pub(crate) const SWITCH_W: f32 = 36.0;
pub(crate) const SWITCH_H: f32 = 20.0;
pub(crate) const SLIDER_H: f32 = 20.0;
/// A field's and a choice's height: text of 13 with room above and below.
pub(crate) const FIELD_PAD_V: f32 = 5.0;
pub(crate) const FIELD_PAD_H: f32 = 8.0;

/// A value from `min` to `max`, dragged, clicked or moved with the keys.
pub fn slider<M>(min: f32, max: f32, value: f32, on_change: impl Fn(f32) -> M + 'static) -> Element<M> {
    let (min, max) = if max > min { (min, max) } else { (min, min + 1.0) };
    Element::new(
        Kind::Slider { min, max, step: 0.0, value: value.clamp(min, max), marks: Vec::new(), on_change: Some(Box::new(on_change)), on_release: None },
        Length::Fixed(200.0),
        Length::Fixed(SLIDER_H),
    )
}

/// One line of text to type into. The field sends its whole text after
/// each change; the program keeps it and gives it back in `value`.
pub fn field<M>(value: impl Into<String>, on_input: impl Fn(String) -> M + 'static) -> Element<M> {
    Element::new(
        Kind::Field { value: value.into(), placeholder: String::new(), secret: false, on_input: Some(Box::new(on_input)), on_submit: None },
        Length::FILL,
        Length::Shrink,
    )
}

/// One of a list: a button showing the chosen one, opening the list.
pub fn choice<M>(options: Vec<String>, selected: Option<usize>, on_select: impl Fn(usize) -> M + 'static) -> Element<M> {
    let selected = selected.filter(|i| *i < options.len());
    Element::new(Kind::Choice { options, selected, on_select: Some(Box::new(on_select)) }, Length::Shrink, Length::Shrink)
}

/// One child, taller than its place, moved up and down within it.
pub fn scroll<M>(child: Element<M>) -> Element<M> {
    Element::new(Kind::Scroll { child: Box::new(child) }, Length::FILL, Length::FILL)
}

/// A picture already decoded (decoding is done away from the window's
/// thread: whitepaper §6.10, isolation). Its size is its own in pixels,
/// as reference units, unless given.
pub fn picture<M>(image: Arc<Image>) -> Element<M> {
    let (w, h) = (image.width as f32, image.height as f32);
    Element::new(Kind::Picture { image, fit: Fit::Contain, radius: 0.0, opacity: 1.0 }, Length::Fixed(w), Length::Fixed(h))
}

/// A place the program draws itself: `draw` is given the scene, the fonts
/// (for `vergence_render::text`), the place, and the theme; `on_touch`, if
/// any, is told of the pointer and fingers in the place's own coordinates.
pub fn drawing<M>(draw: impl Fn(&mut Scene, &mut Fonts, Rect, &Theme) + 'static) -> Element<M> {
    Element::new(Kind::Drawing { draw: Box::new(draw), on_touch: None }, Length::FILL, Length::Fixed(100.0))
}

impl<M> Element<M> {
    /// What a drawing does with the pointer and fingers.
    pub fn on_touch(mut self, f: impl Fn(Touch) -> Option<M> + 'static) -> Self {
        if let Kind::Drawing { on_touch, .. } = &mut self.kind {
            *on_touch = Some(Box::new(f));
        }
        self
    }

    /// A switch, slider, field or choice that is shown but cannot be used.
    pub fn disabled(mut self) -> Self {
        match &mut self.kind {
            Kind::Switch { on_toggle, .. } => *on_toggle = None,
            Kind::Slider { on_change, on_release, .. } => (*on_change, *on_release) = (None, None),
            Kind::Field { on_input, on_submit, .. } => (*on_input, *on_submit) = (None, None),
            Kind::Choice { on_select, .. } => *on_select = None,
            Kind::Button { on_press, .. } => *on_press = None,
            _ => {}
        }
        self
    }
}
