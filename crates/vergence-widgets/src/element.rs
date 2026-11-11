//! What a program builds: a tree of elements, from its state.

use vergence_scene::Color;

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

pub(crate) enum Kind<M> {
    Space,
    Rule { color: Option<Color>, thickness: f32 },
    Label(Label),
    Panel { child: Box<Element<M>>, padding: Padding, fill: Option<Color>, edge: Option<(Color, f32)>, radius: f32, align_x: Align, align_y: Align },
    Button { child: Box<Element<M>>, padding: Padding, on_press: Option<M>, look: ButtonLook },
    Row { children: Vec<Element<M>>, spacing: f32, align: Align },
    Column { children: Vec<Element<M>>, spacing: f32, align: Align },
    Stack { children: Vec<Element<M>> },
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

    /// A label broken into lines at its width.
    pub fn wrap(mut self) -> Self {
        if let Some(l) = self.label_mut() {
            l.wrap = true;
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
        if let Kind::Panel { radius: r, .. } = &mut self.kind {
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
    Element::new(Kind::Row { children, spacing: 0.0, align: Align::Start }, Length::Shrink, Length::Shrink)
}

/// Children one under another.
pub fn column<M>(children: Vec<Element<M>>) -> Element<M> {
    Element::new(Kind::Column { children, spacing: 0.0, align: Align::Start }, Length::Shrink, Length::Shrink)
}

/// Children over one another, the last in front, each the stack's size.
pub fn stack<M>(children: Vec<Element<M>>) -> Element<M> {
    Element::new(Kind::Stack { children }, Length::Shrink, Length::Shrink)
}
