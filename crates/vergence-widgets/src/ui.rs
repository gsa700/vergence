//! A laid-out tree at work: what the pointer, a finger and the keyboard do
//! to it, how it is drawn, and what it means.

use vergence_render::text;
use vergence_scene::{Color, Rect, Scene};
use vergence_text::Fonts;
use vergence_ui::{Button as MouseButton, Cursor, Input, sym, touch::Phase};

use crate::element::{ButtonLook, Element};
use crate::layout::{Id, Layout, Measure, Node, What};
use crate::theme::Theme;

/// What a press is being made with.
#[derive(Debug, Clone, Copy, PartialEq)]
enum With {
    Pointer,
    Finger(i32),
    Key,
}

/// What input did, for the window to act on.
pub struct Response<M> {
    /// The program's messages, in order.
    pub messages: Vec<M>,
    /// The picture changed (a hover, a press, the focus).
    pub redraw: bool,
    /// The pointer's shape, when it is over the window.
    pub cursor: Option<Cursor>,
    /// The widgets used the input; otherwise the program may.
    pub used: bool,
}

impl<M> Default for Response<M> {
    fn default() -> Self {
        Response { messages: Vec::new(), redraw: false, cursor: None, used: false }
    }
}

/// What a widget is, to a screen reader, voice or automation (Contract 3).
#[derive(Debug, Clone, PartialEq)]
pub struct Meaning {
    pub role: &'static str,
    pub name: String,
    pub value: Option<String>,
    pub states: Vec<&'static str>,
    pub bounds: Rect,
    /// How many widgets with a meaning it is inside.
    pub depth: usize,
}

/// Widgets laid out, and what they remember between builds.
pub struct Ui<M> {
    nodes: Vec<Node<M>>,
    hover: Option<Id>,
    pressed: Option<(Id, With)>,
    /// Where a finger is, by its id.
    fingers: Vec<(i32, f32, f32)>,
    focus: Option<Id>,
    /// The focus is shown: it was moved with the keyboard.
    ring: bool,
}

impl<M> Default for Ui<M> {
    fn default() -> Self {
        Ui { nodes: Vec::new(), hover: None, pressed: None, fingers: Vec::new(), focus: None, ring: false }
    }
}

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}

fn over(base: Color, top: Color) -> Color {
    // `top` laid over `base` at its alpha; the result keeps `base`'s alpha.
    let a = top.a as u32;
    let mix = |b: u8, t: u8| ((b as u32 * (255 - a) + t as u32 * a + 127) / 255) as u8;
    Color { r: mix(base.r, top.r), g: mix(base.g, top.g), b: mix(base.b, top.b), a: base.a.max(top.a) }
}

impl<M: Clone> Ui<M> {
    pub fn new() -> Ui<M> {
        Ui::default()
    }

    /// Lays `root` out in a window `width` x `height`. What is remembered
    /// stays with the widgets whose ids are still there.
    pub fn layout(&mut self, root: Element<M>, width: f32, height: f32, measure: &mut dyn Measure, text_size: f32) {
        let mut nodes = Vec::new();
        Layout { measure, text_size }.run(root, Rect::new(0.0, 0.0, width, height), &mut nodes);
        self.nodes = nodes;
        let alive = |id: Option<Id>, nodes: &[Node<M>]| id.filter(|id| nodes.iter().any(|n| n.id == *id && n.active()));
        self.hover = alive(self.hover, &self.nodes);
        self.focus = alive(self.focus, &self.nodes);
        if self.pressed.is_some_and(|(id, _)| alive(Some(id), &self.nodes).is_none()) {
            self.pressed = None;
        }
    }

    /// The topmost widget that takes input at `x`, `y`.
    fn target(&self, x: f32, y: f32) -> Option<Id> {
        self.nodes.iter().rev().find(|n| n.active() && inside(&n.rect, x, y)).map(|n| n.id)
    }

    fn node(&self, id: Id) -> Option<&Node<M>> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// What pressing `id` sends.
    fn activate(&self, id: Id, r: &mut Response<M>) {
        if let Some(Node { what: What::Button { on_press: Some(m), .. }, .. }) = self.node(id) {
            r.messages.push(m.clone());
        }
    }

    fn focusables(&self) -> Vec<Id> {
        self.nodes.iter().filter(|n| n.active()).map(|n| n.id).collect()
    }

    #[cfg(test)]
    pub(crate) fn ids(&self) -> Vec<Id> {
        self.nodes.iter().map(|n| n.id).collect()
    }

    pub fn focused(&self) -> Option<Id> {
        self.focus
    }

    pub fn input(&mut self, input: &Input) -> Response<M> {
        let mut r = Response::default();
        match *input {
            Input::PointerMoved { x, y } => {
                let t = self.target(x, y);
                if t != self.hover {
                    self.hover = t;
                    r.redraw = true;
                }
                r.cursor = Some(if t.is_some() { Cursor::Pointer } else { Cursor::Default });
                r.used = t.is_some() || self.pressed.is_some();
            }
            Input::PointerLeft => {
                if self.hover.take().is_some() {
                    r.redraw = true;
                }
            }
            Input::Button { button: MouseButton::Left, pressed, x, y, .. } => {
                // Judged where the pointer was at this press, not where it is now.
                let t = self.target(x, y);
                if pressed {
                    if let Some(id) = t {
                        self.pressed = Some((id, With::Pointer));
                        self.ring = false;
                        r.redraw = true;
                        r.used = true;
                    }
                } else if let Some((id, With::Pointer)) = self.pressed {
                    self.pressed = None;
                    if t == Some(id) {
                        self.activate(id, &mut r);
                    }
                    r.redraw = true;
                    r.used = true;
                }
            }
            Input::Touch { id: finger, phase, x, y } => match phase {
                Phase::Down => {
                    self.fingers.retain(|f| f.0 != finger);
                    self.fingers.push((finger, x, y));
                    if self.pressed.is_none()
                        && let Some(id) = self.target(x, y)
                    {
                        self.pressed = Some((id, With::Finger(finger)));
                        self.ring = false;
                        r.redraw = true;
                        r.used = true;
                    }
                }
                Phase::Moved => {
                    if let Some(f) = self.fingers.iter_mut().find(|f| f.0 == finger) {
                        (f.1, f.2) = (x, y);
                    }
                    r.used = self.pressed.is_some_and(|p| p.1 == With::Finger(finger));
                    r.redraw = r.used;
                }
                Phase::Up | Phase::Cancelled => {
                    self.fingers.retain(|f| f.0 != finger);
                    if let Some((id, With::Finger(f))) = self.pressed
                        && f == finger
                    {
                        self.pressed = None;
                        if phase == Phase::Up && self.target(x, y) == Some(id) {
                            self.activate(id, &mut r);
                        }
                        r.redraw = true;
                        r.used = true;
                    }
                }
            },
            Input::Key { sym: key, pressed: true, repeat, composing: false, modifiers, .. } => {
                let plain = !modifiers.ctrl && !modifiers.alt && !modifiers.logo;
                if key == sym::TAB && plain {
                    let list = self.focusables();
                    if !list.is_empty() {
                        let at = self.focus.and_then(|f| list.iter().position(|i| *i == f));
                        let next = match (at, modifiers.shift) {
                            (None, false) => 0,
                            (None, true) => list.len() - 1,
                            (Some(i), false) => (i + 1) % list.len(),
                            (Some(i), true) => (i + list.len() - 1) % list.len(),
                        };
                        self.focus = Some(list[next]);
                        self.ring = true;
                        r.redraw = true;
                        r.used = true;
                    }
                } else if (key == sym::RETURN || key == ' ' as u32) && plain && !modifiers.shift && !repeat {
                    if let Some(id) = self.focus {
                        self.pressed = Some((id, With::Key));
                        self.activate(id, &mut r);
                        r.redraw = true;
                        r.used = true;
                    }
                } else if key == sym::ESCAPE && self.ring {
                    self.ring = false;
                    r.redraw = true;
                }
            }
            Input::Key { sym: key, pressed: false, .. } if key == sym::RETURN || key == ' ' as u32 => {
                if let Some((_, With::Key)) = self.pressed {
                    self.pressed = None;
                    r.redraw = true;
                    r.used = true;
                }
            }
            Input::Focus(false) => {
                self.pressed = None;
                self.fingers.clear();
                r.redraw = true;
            }
            _ => {}
        }
        r
    }

    /// Is `id` held down, with what holds it still on it?
    fn held(&self, id: Id, rect: &Rect) -> bool {
        match self.pressed {
            Some((p, With::Pointer)) if p == id => self.hover == Some(id),
            Some((p, With::Finger(f))) if p == id => self.fingers.iter().any(|g| g.0 == f && inside(rect, g.1, g.2)),
            Some((p, With::Key)) => p == id,
            _ => false,
        }
    }

    /// Draws the widgets into `scene`.
    pub fn draw(&self, scene: &mut Scene, fonts: &mut Fonts, theme: &Theme) {
        let mut ink: Vec<Color> = Vec::with_capacity(self.nodes.len());
        let face = |fonts: &mut Fonts, weight: u16| fonts.face(&theme.font, weight, false).or_else(|| fonts.face("sans-serif", weight, false));
        for n in &self.nodes {
            let inherited = n.parent.map_or(theme.text, |p| ink[p]);
            let mut mine = inherited;
            match &n.what {
                What::Space | What::Group => {}
                What::Rule { color } => {
                    scene.fill(n.rect, color.unwrap_or(theme.line));
                }
                What::Label { lines, size, weight, color, dim } => {
                    let col = color.unwrap_or(if *dim { theme.dim } else { inherited });
                    if let Some(f) = face(fonts, *weight) {
                        for l in lines {
                            text(scene, fonts, f, *size, col, vergence_scene::Point::new(n.rect.x + l.x, n.rect.y + l.baseline), &l.text);
                        }
                    }
                }
                What::Panel { fill, edge, radius } => {
                    if let Some(f) = fill {
                        scene.fill_rounded(n.rect, *radius, *f);
                    }
                    if let Some((c, w)) = edge {
                        scene.stroke(n.rect, *radius, *w, *c);
                    }
                }
                What::Button { on_press, look } => {
                    let on = on_press.is_some();
                    let hot = on && self.hover == Some(n.id);
                    let down = on && self.held(n.id, &n.rect);
                    let tint = |base: Option<Color>| -> Option<Color> {
                        let h = theme.hover;
                        let t = if down { Color { a: h.a.saturating_mul(2), ..h } } else { h };
                        match (hot || down, base) {
                            (false, b) => b,
                            (true, Some(b)) => Some(over(b, t)),
                            (true, None) => Some(t),
                        }
                    };
                    let radius = vergence_look::RADIUS;
                    let edge_c = if theme.high_contrast { theme.text } else { theme.line };
                    let white = Color::rgb(255, 255, 255);
                    let (fill, edge, r, i) = match *look {
                        ButtonLook::Normal => (tint(Some(theme.page)), Some((edge_c, 1.0)), radius, theme.text),
                        ButtonLook::Plain => (tint(None), None, radius, inherited),
                        ButtonLook::Accent => (tint(Some(theme.accent)), Some((theme.accent, 1.0)), radius, white),
                        ButtonLook::Chosen => (Some(theme.accent), None, radius, white),
                        ButtonLook::Custom { fill, edge, radius } => (tint(fill), edge, radius, inherited),
                    };
                    if let Some(f) = fill {
                        scene.fill_rounded(n.rect, r, f);
                    }
                    if let Some((c, w)) = edge {
                        scene.stroke(n.rect, r, w, c);
                    }
                    mine = if on { i } else { theme.dim };
                    if self.ring && self.focus == Some(n.id) {
                        let g = 2.0;
                        scene.stroke(Rect::new(n.rect.x - g, n.rect.y - g, n.rect.w + 2.0 * g, n.rect.h + 2.0 * g), r + g, 2.0, theme.accent);
                    }
                }
            }
            ink.push(mine);
        }
    }

    /// Every widget's meaning, in reading order.
    pub fn meaning(&self) -> Vec<Meaning> {
        let mut out = Vec::new();
        // For each node: its place in `out` if it has a meaning, and
        // whether it is inside a button (whose text is the button's name).
        let mut slot: Vec<Option<usize>> = Vec::with_capacity(self.nodes.len());
        let mut in_button: Vec<bool> = Vec::with_capacity(self.nodes.len());
        for (i, n) in self.nodes.iter().enumerate() {
            let parent_in_button = n.parent.is_some_and(|p| in_button[p] || matches!(self.nodes[p].what, What::Button { .. }));
            let depth = {
                let mut d = 0;
                let mut p = n.parent;
                while let Some(q) = p {
                    if slot[q].is_some() {
                        d += 1;
                    }
                    p = self.nodes[q].parent;
                }
                d
            };
            let m = match &n.what {
                What::Label { lines, .. } if !parent_in_button => {
                    let text = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" ");
                    Some(Meaning { role: "label", name: n.name.clone().unwrap_or(text), value: None, states: vec![], bounds: n.rect, depth })
                }
                What::Button { on_press, .. } => {
                    let mut states = Vec::new();
                    if on_press.is_none() {
                        states.push("off");
                    }
                    if self.focus == Some(n.id) {
                        states.push("focused");
                    }
                    let name = n.name.clone().unwrap_or_else(|| self.text_under(i));
                    Some(Meaning { role: "button", name, value: None, states, bounds: n.rect, depth })
                }
                What::Rule { .. } => Some(Meaning { role: "separator", name: String::new(), value: None, states: vec![], bounds: n.rect, depth }),
                _ => None,
            };
            slot.push(m.as_ref().map(|_| out.len()));
            in_button.push(parent_in_button);
            out.extend(m);
        }
        out
    }

    /// The text of every label under node `i`, joined. A node's
    /// descendants follow it in the list, before anything that is not one.
    fn text_under(&self, i: usize) -> String {
        let under = |mut p: Option<usize>| {
            while let Some(q) = p {
                if q == i {
                    return true;
                }
                p = self.nodes[q].parent;
            }
            false
        };
        let mut words = Vec::new();
        for n in self.nodes[i + 1..].iter().take_while(|n| under(n.parent)) {
            if let What::Label { lines, .. } = &n.what {
                words.extend(lines.iter().map(|l| l.text.clone()));
            }
        }
        words.join(" ")
    }

    /// The meaning as text, one widget a line, indented by depth.
    pub fn meaning_text(&self) -> String {
        let mut s = String::new();
        for m in self.meaning() {
            let states = if m.states.is_empty() { String::new() } else { format!(" [{}]", m.states.join(", ")) };
            let value = m.value.as_ref().map(|v| format!(" = {v:?}")).unwrap_or_default();
            s += &format!("{}{} {:?}{}{} at {},{} {}x{}\n", "  ".repeat(m.depth), m.role, m.name, value, states, m.bounds.x, m.bounds.y, m.bounds.w, m.bounds.h);
        }
        s
    }
}

impl<M> Node<M> {
    /// Takes input: a button that sends something.
    pub(crate) fn active(&self) -> bool {
        matches!(self.what, What::Button { on_press: Some(_), .. })
    }
}
