//! A laid-out tree at work: what the pointer, a finger and the keyboard do
//! to it, how it is drawn, and what it means.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use vergence_render::text;
use vergence_scene::{Color, Point, Rect, Scene};
use vergence_text::{FaceId, Fonts};
use vergence_ui::{Button as MouseButton, Cursor, Input, Modifiers, sym, touch::Phase};

use crate::element::{ButtonLook, Element, FIELD_PAD_H, Fit, Touch};
use crate::layout::{CHEVRON, Id, Layout, Measure, Node, What};
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
    /// Text to put on the clipboard.
    pub copy: Option<String>,
    /// The clipboard's text is wanted (it comes back as `Input::Paste`).
    pub paste: bool,
}

impl<M> Default for Response<M> {
    fn default() -> Self {
        Response { messages: Vec::new(), redraw: false, cursor: None, used: false, copy: None, paste: false }
    }
}

/// What a widget is, to a screen reader, voice or automation (Contract 3).
#[derive(Debug, Clone, PartialEq)]
pub struct Meaning {
    /// The widget's id, stable while the tree keeps its shape (child_id), and
    /// its nearest ancestor with a meaning's (0: the window itself). For the
    /// native protocol's tree (Contract 3, native milestone 1 step 6).
    pub id: u64,
    pub parent: u64,
    pub role: &'static str,
    pub name: String,
    pub value: Option<String>,
    pub states: Vec<&'static str>,
    pub bounds: Rect,
    /// How many widgets with a meaning it is inside.
    pub depth: usize,
}

/// A field's own copy of its text while it is being typed in.
#[derive(Debug, Default)]
struct FieldState {
    text: String,
    /// What the field last sent, or was last given: while the program's
    /// value is this, the field's own text stands (a key typed before the
    /// tree is built again is not lost).
    sent: String,
    /// The caret and the other end of the selection, as byte offsets.
    caret: usize,
    anchor: usize,
    /// How far the text is moved left to keep the caret in view.
    shift: f32,
    /// Where each character boundary is drawn, from the text's start.
    stops: Vec<(usize, f32)>,
    /// Undo and redo (editor milestone 1); forgotten when the program
    /// gives the field a new value.
    history: crate::edit::History<usize>,
}

#[derive(Debug, Default)]
struct ScrollState {
    offset: f32,
    /// Points a second, from a flick; 0 when still.
    velocity: f32,
}

#[derive(Debug)]
struct SwitchState {
    shown: bool,
    changed: Option<Instant>,
}

/// Something being dragged.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    Slider { id: Id, with: With, value: f32 },
    Select { id: Id },
    Bar { id: Id, grab: f32 },
    /// A finger on a scrolling place: it scrolls once it has moved.
    Flick { id: Id, finger: i32, start: f32, last: f32, at: Instant, moved: bool, velocity: f32 },
    Drawing { id: Id, with: With },
    /// Choosing text in a text area with the pointer.
    AreaSelect { id: Id },
    /// A text area's scroll bar.
    AreaBar { id: Id, grab: f32 },
    /// A finger on a text area: it scrolls once it has moved, else it is a tap.
    AreaTouch { id: Id, finger: i32, start: f32, last: f32, moved: bool },
}

fn drag_id(d: &Drag) -> Id {
    match *d {
        Drag::Slider { id, .. } | Drag::Select { id } | Drag::Bar { id, .. } | Drag::Flick { id, .. } | Drag::Drawing { id, .. } | Drag::AreaSelect { id } | Drag::AreaBar { id, .. } | Drag::AreaTouch { id, .. } => id,
    }
}

/// What a line of a field's right-click menu does.
#[derive(Debug, Clone, PartialEq)]
enum FieldAct {
    /// A spelling suggestion: this text in place of bytes `a..b`.
    Replace(usize, usize, String),
    /// The word goes in the user's dictionary.
    Add(String),
    Cut,
    Copy,
    Paste,
    /// A line that does nothing ("No suggestions").
    Nothing,
}

/// A field's right-click menu (editor milestone 1, step 7): spelling
/// suggestions, then the clipboard's commands.
#[derive(Debug, Clone)]
struct FieldMenu {
    id: Id,
    items: Vec<(String, FieldAct)>,
    hl: usize,
    rect: Rect,
    row: f32,
}

/// An open choice list.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Open {
    id: Id,
    hl: Option<usize>,
    offset: f32,
}

const BLINK: Duration = Duration::from_millis(530);
const SLIDE: Duration = Duration::from_millis(120);
/// A finger moves this far before a press becomes a scroll.
const SLOP: f32 = 8.0;
/// Points scrolled for each unit of the wheel's value (15 a notch).
const WHEEL: f32 = 2.0;
const BAR_W: f32 = 6.0;
/// The longest text a field holds, in bytes.
pub(crate) const MAX_FIELD: usize = 4096;
const ITEM_PAD: f32 = 4.0;
const THUMB: f32 = 16.0;

/// Widgets laid out, and what they remember between builds.
pub struct Ui<M> {
    nodes: Vec<Node<M>>,
    size: (f32, f32),
    hover: Option<Id>,
    pressed: Option<(Id, With)>,
    /// Where the pointer pressed it (a button pulled away starts a drag).
    pressed_at: (f32, f32),
    /// Where a finger is, by its id.
    fingers: Vec<(i32, f32, f32)>,
    focus: Option<Id>,
    /// The focus is shown: it was moved with the keyboard.
    ring: bool,
    fields: HashMap<Id, FieldState>,
    scrolls: HashMap<Id, ScrollState>,
    /// The strip at the window's top under the server's bar (look milestone
    /// 1): scroll bars start below it (Dave, 2026-11-14: they went under the
    /// title bar).
    under_bar: f32,
    switches: HashMap<Id, SwitchState>,
    drag: Option<Drag>,
    open: Option<Open>,
    /// When the caret was last shown at once (it blinks from there).
    blink: Instant,
    steady: bool,
    /// The longest time between the clicks of a double-click.
    double: Duration,
    /// The last click on a field, for double and triple clicks.
    click: Option<(Instant, Id, u8)>,
    /// The drawing the pointer is over.
    over_drawing: Option<Id>,
    /// When momentum was last moved on.
    last_tick: Option<Instant>,
    /// Shift was held at the last press (a click in a text area extends the selection).
    shift_press: bool,
    /// A field's right-click menu, open.
    field_menu: Option<FieldMenu>,
    /// Words spelled wrong are marked (the theme's, as last drawn).
    spell: bool,
}

impl<M> Default for Ui<M> {
    fn default() -> Self {
        Ui {
            nodes: Vec::new(),
            size: (0.0, 0.0),
            hover: None,
            pressed: None,
            pressed_at: (0.0, 0.0),
            fingers: Vec::new(),
            focus: None,
            ring: false,
            fields: HashMap::new(),
            scrolls: HashMap::new(),
            under_bar: 0.0,
            switches: HashMap::new(),
            drag: None,
            open: None,
            blink: Instant::now(),
            steady: false,
            double: Duration::from_millis(400),
            click: None,
            over_drawing: None,
            last_tick: None,
            shift_press: false,
            field_menu: None,
            spell: true,
        }
    }
}

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}

fn meet(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    Rect::new(x, y, ((a.x + a.w).min(b.x + b.w) - x).max(0.0), ((a.y + a.h).min(b.y + b.h) - y).max(0.0))
}

fn over(base: Color, top: Color) -> Color {
    // `top` laid over `base` at its alpha; the result keeps `base`'s alpha.
    let a = top.a as u32;
    let mix = |b: u8, t: u8| ((b as u32 * (255 - a) + t as u32 * a + 127) / 255) as u8;
    Color { r: mix(base.r, top.r), g: mix(base.g, top.g), b: mix(base.b, top.b), a: base.a.max(top.a) }
}

fn alpha(c: Color, a: u8) -> Color {
    Color { a, ..c }
}

fn prev_char(s: &str, i: usize) -> usize {
    s[..i].char_indices().next_back().map_or(0, |(j, _)| j)
}

fn next_char(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(i, |c| i + c.len_utf8())
}

fn prev_word(s: &str, i: usize) -> usize {
    let mut j = i;
    while j > 0 && s[..j].ends_with(char::is_whitespace) {
        j = prev_char(s, j);
    }
    while j > 0 && !s[..j].ends_with(char::is_whitespace) {
        j = prev_char(s, j);
    }
    j
}

fn next_word(s: &str, i: usize) -> usize {
    let mut j = i;
    while j < s.len() && s[j..].starts_with(char::is_whitespace) {
        j = next_char(s, j);
    }
    while j < s.len() && !s[j..].starts_with(char::is_whitespace) {
        j = next_char(s, j);
    }
    j
}

/// The nearest character boundary at or before `i`.
fn boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// A slider's value at a fraction along it, kept to its step.
fn slider_value(min: f32, max: f32, step: f32, f: f32) -> f32 {
    let v = min + (max - min) * f.clamp(0.0, 1.0);
    let v = if step > 0.0 { min + ((v - min) / step).round() * step } else { v };
    v.clamp(min, max)
}

/// Where a slider's thumb travels: inset by its radius.
fn slider_track(r: &Rect) -> (f32, f32) {
    (r.x + THUMB / 2.0, (r.w - THUMB).max(1.0))
}

impl<M: Clone> Ui<M> {
    pub fn new() -> Ui<M> {
        Ui::default()
    }

    /// Lays `root` out in a window `width` x `height`. What is remembered
    /// stays with the widgets whose ids are still there.
    /// The strip at the window's top under the server's bar: scroll bars start below it.
    pub fn set_under_bar(&mut self, top: f32) {
        self.under_bar = top.max(0.0);
    }

    pub fn layout(&mut self, root: Element<M>, width: f32, height: f32, measure: &mut dyn Measure, text_size: f32) {
        let mut nodes = Vec::new();
        Layout { measure, text_size, lines_seen: Default::default() }.run(root, Rect::new(0.0, 0.0, width, height), &mut nodes);
        let before: HashSet<Id> = self.nodes.iter().map(|n| n.id).collect();
        self.nodes = nodes;
        self.size = (width, height);
        let alive = |id: Option<Id>, nodes: &[Node<M>]| id.filter(|id| nodes.iter().any(|n| n.id == *id && n.active()));
        self.hover = alive(self.hover, &self.nodes);
        self.focus = alive(self.focus, &self.nodes).filter(|id| self.nodes.iter().any(|n| n.id == *id && n.focusable()));
        // A field that asks for the keyboard gets it when it first appears.
        // A field's text is chosen then, so typing replaces it (a name offered).
        let mut choose_all = None;
        if let Some(n) = self.nodes.iter().find(|n| matches!(n.what, What::Field { autofocus: true, .. } | What::Area { autofocus: true, .. }) && !before.contains(&n.id)) {
            self.focus = Some(n.id);
            if matches!(n.what, What::Field { .. }) {
                choose_all = Some(n.id);
            }
        }
        if self.pressed.is_some_and(|(id, _)| alive(Some(id), &self.nodes).is_none()) {
            self.pressed = None;
        }
        if self.open.is_some_and(|o| alive(Some(o.id), &self.nodes).is_none()) {
            self.open = None;
        }
        let ids: HashSet<Id> = self.nodes.iter().map(|n| n.id).collect();
        if self.drag.is_some_and(|d| !ids.contains(&drag_id(&d))) {
            self.drag = None;
        }
        self.fields.retain(|id, _| ids.contains(id));
        self.scrolls.retain(|id, _| ids.contains(id));
        self.switches.retain(|id, _| ids.contains(id));
        let now = Instant::now();
        for n in &self.nodes {
            match &n.what {
                What::Field { value, .. } => {
                    let f = self.fields.entry(n.id).or_default();
                    if *value != f.sent {
                        f.text = value.clone();
                        f.sent = value.clone();
                        f.history.clear();
                    }
                    f.caret = boundary(&f.text, f.caret);
                    f.anchor = boundary(&f.text, f.anchor);
                    if choose_all == Some(n.id) {
                        (f.anchor, f.caret) = (0, f.text.len());
                    }
                }
                What::Scroll { content } => {
                    let s = self.scrolls.entry(n.id).or_default();
                    s.offset = s.offset.clamp(0.0, (content - n.rect.h).max(0.0));
                }
                What::Switch { on, .. } => {
                    let s = self.switches.entry(n.id).or_insert(SwitchState { shown: *on, changed: None });
                    if s.shown != *on {
                        s.shown = *on;
                        s.changed = Some(now);
                    }
                }
                _ => {}
            }
        }
        // A node marked to be kept in view, new since the last layout, is scrolled to.
        if let Some(i) = self.nodes.iter().position(|n| n.reveal && !before.contains(&n.id)) {
            self.reveal(i);
        }
    }

    /// The caret is steady rather than blinking, and switches do not slide
    /// (the desktop's reduced motion).
    pub fn set_reduced_motion(&mut self, steady: bool) {
        self.steady = steady;
    }

    /// The longest time between two clicks of a double-click (the desktop's).
    pub fn set_double_click(&mut self, ms: u32) {
        self.double = Duration::from_millis(ms.clamp(100, 2000) as u64);
    }

    fn index(&self, id: Id) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    fn node(&self, id: Id) -> Option<&Node<M>> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// How far node `i` is moved up by the scrolling places it is in.
    fn offset_of(&self, i: usize) -> f32 {
        let mut d = 0.0;
        let mut p = self.nodes[i].parent;
        while let Some(q) = p {
            if let What::Scroll { .. } = self.nodes[q].what {
                d += self.scrolls.get(&self.nodes[q].id).map_or(0.0, |s| s.offset);
            }
            p = self.nodes[q].parent;
        }
        d
    }

    /// Node `i`'s rectangle as it is on the screen.
    fn shown(&self, i: usize) -> Rect {
        let r = self.nodes[i].rect;
        Rect::new(r.x, r.y - self.offset_of(i), r.w, r.h)
    }

    /// The part of the window node `i` can be seen in: inside every
    /// scrolling place around it.
    fn clip_of(&self, i: usize) -> Rect {
        let mut c = Rect::new(0.0, 0.0, self.size.0, self.size.1);
        let mut p = self.nodes[i].parent;
        while let Some(q) = p {
            if let What::Scroll { .. } = self.nodes[q].what {
                c = meet(c, self.shown(q));
            }
            p = self.nodes[q].parent;
        }
        c
    }

    fn hit(&self, i: usize, x: f32, y: f32) -> bool {
        inside(&self.shown(i), x, y) && inside(&self.clip_of(i), x, y)
    }

    /// The topmost widget that takes input at `x`, `y`.
    fn target(&self, x: f32, y: f32) -> Option<Id> {
        (0..self.nodes.len()).rev().find(|&i| self.nodes[i].active() && self.hit(i, x, y)).map(|i| self.nodes[i].id)
    }

    /// The menu message of the innermost element at `x`, `y` that has one.
    fn menu_at(&self, x: f32, y: f32) -> Option<M>
    where
        M: Clone,
    {
        (0..self.nodes.len()).rev().find(|&i| self.nodes[i].on_menu.is_some() && self.hit(i, x, y)).and_then(|i| self.nodes[i].on_menu.clone())
    }

    /// The innermost scrolling place at `x`, `y` that has more than it shows.
    fn scroller_at(&self, x: f32, y: f32) -> Option<usize> {
        (0..self.nodes.len()).rev().find(|&i| matches!(self.nodes[i].what, What::Scroll { content } if content > self.nodes[i].rect.h + 0.5) && self.hit(i, x, y))
    }

    /// The nearest scrolling place around node `i`.
    fn scroller_of(&self, i: usize) -> Option<usize> {
        let mut p = self.nodes[i].parent;
        while let Some(q) = p {
            if let What::Scroll { .. } = self.nodes[q].what {
                return Some(q);
            }
            p = self.nodes[q].parent;
        }
        None
    }

    /// Moves scrolling place `i` by `dy`; true if it moved.
    fn scroll_by(&mut self, i: usize, dy: f32) -> bool {
        let What::Scroll { content } = self.nodes[i].what else { return false };
        let max = (content - self.nodes[i].rect.h).max(0.0);
        let s = self.scrolls.entry(self.nodes[i].id).or_default();
        let before = s.offset;
        s.offset = (s.offset + dy).clamp(0.0, max);
        s.offset != before
    }

    /// How much taller the outermost scrolling place's content is than the
    /// place itself (negative: shorter), as last laid out; None without one.
    pub fn overflow(&self) -> Option<f32> {
        self.nodes.iter().find_map(|n| match n.what {
            What::Scroll { content } => Some(content - n.rect.h),
            _ => None,
        })
    }

    /// The scrollbar's thumb of scrolling place `i`, if it has one.
    fn thumb(&self, i: usize) -> Option<Rect> {
        let What::Scroll { content } = self.nodes[i].what else { return None };
        let r = self.shown(i);
        if content <= r.h + 0.5 {
            return None;
        }
        let offset = self.scrolls.get(&self.nodes[i].id).map_or(0.0, |s| s.offset);
        // The track: the place, less any of it under the window's bar.
        let top = r.y.max(self.under_bar.min(r.y + r.h - 24.0));
        let track = r.y + r.h - top;
        let h = (track * r.h / content).max(24.0).min(track);
        let y = top + (track - h) * offset / (content - r.h);
        Some(Rect::new(r.x + r.w - BAR_W - 2.0, y, BAR_W, h))
    }

    /// Scrolls the places around node `i` so that it can be seen.
    fn reveal(&mut self, i: usize) {
        let mut child = i;
        while let Some(s) = self.scroller_of(child) {
            let r = self.shown(i);
            let area = self.shown(s);
            let dy = if r.y < area.y {
                r.y - area.y - 8.0
            } else if r.y + r.h > area.y + area.h {
                (r.y + r.h - (area.y + area.h) + 8.0).min(r.y - area.y)
            } else {
                0.0
            };
            if dy != 0.0 {
                self.scroll_by(s, dy);
            }
            child = s;
        }
    }

    fn focusables(&self) -> Vec<Id> {
        self.nodes.iter().filter(|n| n.focusable()).map(|n| n.id).collect()
    }

    pub fn focused(&self) -> Option<Id> {
        self.focus
    }

    #[cfg(test)]
    pub(crate) fn ids(&self) -> Vec<Id> {
        self.nodes.iter().map(|n| n.id).collect()
    }

    #[cfg(test)]
    pub(crate) fn scroll_offset(&self) -> f32 {
        self.scrolls.values().map(|s| s.offset).sum()
    }

    /// The open field menu's lines (for tests).
    #[cfg(test)]
    pub(crate) fn field_menu_labels(&self) -> Option<Vec<String>> {
        self.field_menu.as_ref().map(|m| m.items.iter().map(|(l, _)| l.clone()).collect())
    }

    #[cfg(test)]
    pub(crate) fn field_text(&self) -> Option<(String, usize, usize)> {
        let f = self.fields.get(&self.focus?)?;
        Some((f.text.clone(), f.anchor, f.caret))
    }

    /// True while something moves by itself (a flick's momentum, a switch's
    /// slide): a frame is wanted every time the monitor can show one.
    pub fn animating(&self) -> bool {
        let now = Instant::now();
        self.scrolls.values().any(|s| s.velocity != 0.0) || (!self.steady && self.switches.values().any(|s| s.changed.is_some_and(|t| now.duration_since(t) < SLIDE)))
    }

    /// Moves momentum on to `now`. Called before each frame.
    pub fn advance(&mut self, now: Instant) {
        let dt = self.last_tick.map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32()).min(0.1);
        self.last_tick = Some(now);
        if dt == 0.0 {
            return;
        }
        let moving: Vec<(usize, f32)> = (0..self.nodes.len()).filter_map(|i| self.scrolls.get(&self.nodes[i].id).filter(|s| s.velocity != 0.0).map(|s| (i, s.velocity))).collect();
        for (i, v) in moving {
            let moved = self.scroll_by(i, v * dt);
            if let Some(s) = self.scrolls.get_mut(&self.nodes[i].id) {
                s.velocity *= (-4.0 * dt).exp();
                if !moved || s.velocity.abs() < 20.0 {
                    s.velocity = 0.0;
                }
            }
        }
    }

    /// When the toolkit wants to be woken next (the caret's blink).
    pub fn next_timer(&self) -> Option<Instant> {
        if self.steady {
            return None;
        }
        let focused_field = self.focus.and_then(|id| self.node(id)).is_some_and(|n| matches!(n.what, What::Field { .. } | What::Area { .. }));
        if !focused_field {
            return None;
        }
        let since = Instant::now().saturating_duration_since(self.blink);
        let n = since.as_millis() / BLINK.as_millis() + 1;
        Some(self.blink + BLINK * n as u32)
    }

    /// The caret is drawn now.
    fn caret_on(&self) -> bool {
        self.steady || (Instant::now().saturating_duration_since(self.blink).as_millis() / BLINK.as_millis()).is_multiple_of(2)
    }

    fn caret_moved(&mut self) {
        self.blink = Instant::now();
    }

    /// The text area `id`'s document, if it is one.
    fn area_doc(&self, id: Id) -> Option<crate::textarea::TextDoc> {
        match self.node(id).map(|n| &n.what) {
            Some(What::Area { doc, .. }) => Some(doc.clone()),
            _ => None,
        }
    }

    /// Tells the program a text area's text or caret changed.
    fn area_changed(&self, id: Id, r: &mut Response<M>) {
        if let Some(What::Area { on_change: Some(m), .. }) = self.node(id).map(|n| &n.what) {
            r.messages.push(m.clone());
        }
    }

    /// What a key or a paste did in text area `id`, for the program and the window.
    fn area_effect(&mut self, id: Id, e: crate::textarea::Effect, r: &mut Response<M>) {
        if e.copy.is_some() {
            r.copy = e.copy;
        }
        r.paste |= e.paste;
        if e.text || e.moved {
            self.caret_moved();
            self.area_changed(id, r);
        }
        r.redraw = true;
        r.used = e.used;
    }

    /// What pressing `id` does (a button, a switch, a choice's opening).
    fn activate(&mut self, id: Id, r: &mut Response<M>) {
        let Some(i) = self.index(id) else { return };
        match &self.nodes[i].what {
            What::Button { on_press: Some(m), .. } => r.messages.push(m.clone()),
            What::Switch { on, on_toggle: Some(f) } => r.messages.push(f(!*on)),
            What::Choice { selected, on_select: Some(_), .. } => {
                self.open = Some(Open { id, hl: *selected, offset: 0.0 });
                self.reveal_open();
            }
            _ => {}
        }
        r.redraw = true;
    }

    /// The open list's place, the height of an item, and how many there are.
    fn list_rect(&self) -> Option<(Rect, f32, usize)> {
        let o = self.open?;
        let i = self.index(o.id)?;
        let What::Choice { options, step, .. } = &self.nodes[i].what else { return None };
        let at = self.shown(i);
        let item = (step + 2.0 * ITEM_PAD).ceil();
        let (w, h) = self.size;
        let full = options.len() as f32 * item + 8.0;
        let below = h - (at.y + at.h) - 8.0;
        let above = at.y - 8.0;
        let (y, lh) = if full <= below || below >= above {
            (at.y + at.h + 2.0, full.min(below.max(item + 8.0)))
        } else {
            let lh = full.min(above);
            (at.y - 2.0 - lh, lh)
        };
        let lw = at.w.max(120.0).min(w - 16.0);
        let x = at.x.min(w - 8.0 - lw).max(8.0);
        Some((Rect::new(x, y.max(4.0), lw, lh), item, options.len()))
    }

    /// The item of the open list at `x`, `y`.
    fn item_at(&self, x: f32, y: f32) -> Option<usize> {
        let (r, item, n) = self.list_rect()?;
        if !inside(&r, x, y) {
            return None;
        }
        let k = ((y - r.y - 4.0 + self.open?.offset) / item).floor();
        (k >= 0.0 && (k as usize) < n).then_some(k as usize)
    }

    fn reveal_open(&mut self) {
        let Some((r, item, n)) = self.list_rect() else { return };
        let Some(o) = self.open.as_mut() else { return };
        let max = (n as f32 * item + 8.0 - r.h).max(0.0);
        if let Some(h) = o.hl {
            let top = h as f32 * item;
            if top < o.offset {
                o.offset = top;
            } else if top + item > o.offset + r.h - 8.0 {
                o.offset = top + item - (r.h - 8.0);
            }
        }
        o.offset = o.offset.clamp(0.0, max);
    }

    fn choose(&mut self, k: usize, r: &mut Response<M>) {
        if let Some(o) = self.open.take()
            && let Some(Node { what: What::Choice { on_select: Some(f), .. }, .. }) = self.node(o.id)
        {
            r.messages.push(f(k));
        }
        r.redraw = true;
        r.used = true;
    }

    /// A slider's value at `x`.
    fn slider_at(&self, i: usize, x: f32) -> Option<f32> {
        let What::Slider { min, max, step, .. } = self.nodes[i].what else { return None };
        let (x0, w) = slider_track(&self.shown(i));
        Some(slider_value(min, max, step, (x - x0) / w))
    }

    fn slider_send(&mut self, id: Id, value: f32, r: &mut Response<M>) {
        let before = match self.drag {
            Some(Drag::Slider { id: d, value, .. }) if d == id => Some(value),
            _ => None,
        };
        if let Some(Drag::Slider { value: v, .. }) = self.drag.as_mut() {
            *v = value;
        }
        let was = before.or_else(|| match self.node(id).map(|n| &n.what) {
            Some(What::Slider { value, .. }) => Some(*value),
            _ => None,
        });
        if let Some(Node { what: What::Slider { on_change: Some(f), .. }, .. }) = self.node(id)
            && was.is_none_or(|w| (value - w).abs() > f32::EPSILON)
        {
            r.messages.push(f(value));
        }
        r.redraw = true;
        r.used = true;
    }

    fn slider_release(&mut self, id: Id, r: &mut Response<M>) {
        if let Some(Node { what: What::Slider { on_release: Some(m), .. }, .. }) = self.node(id) {
            r.messages.push(m.clone());
        }
        r.redraw = true;
        r.used = true;
    }

    /// Tells a drawing of `t`, in its own coordinates.
    fn tell_drawing(&self, id: Id, t: Touch, r: &mut Response<M>) {
        let Some(i) = self.index(id) else { return };
        let What::Drawing { on_touch: Some(f), .. } = &self.nodes[i].what else { return };
        let o = self.shown(i);
        let local = match t {
            Touch::Press { x, y } => Touch::Press { x: x - o.x, y: y - o.y },
            Touch::Move { x, y, pressed } => Touch::Move { x: x - o.x, y: y - o.y, pressed },
            Touch::Release { x, y } => Touch::Release { x: x - o.x, y: y - o.y },
            Touch::Leave => Touch::Leave,
        };
        r.messages.extend(f(local));
        r.used = true;
    }

    /// The byte offset in field `i`'s text nearest to `x` on the screen.
    fn field_offset(&self, i: usize, x: f32) -> usize {
        let Some(f) = self.fields.get(&self.nodes[i].id) else { return 0 };
        let local = x - (self.shown(i).x + FIELD_PAD_H) + f.shift;
        f.stops.iter().min_by(|a, b| (a.1 - local).abs().total_cmp(&(b.1 - local).abs())).map_or(f.text.len(), |s| s.0)
    }

    /// A right click on field `id` at `x`, `y`: suggestions for the word
    /// there if it is spelled wrong, then Cut, Copy and Paste.
    fn open_field_menu(&mut self, id: Id, x: f32, y: f32) {
        let Some(i) = self.index(id) else { return };
        let (secret, editable) = match &self.nodes[i].what {
            What::Field { secret, on_input, .. } => (*secret, on_input.is_some()),
            _ => return,
        };
        let at = self.field_offset(i, x);
        let Some(f) = self.fields.get_mut(&id) else { return };
        let (a, b) = (f.caret.min(f.anchor), f.caret.max(f.anchor));
        if !(a != b && at >= a && at <= b) {
            f.caret = at;
            f.anchor = at;
        }
        let mut items: Vec<(String, FieldAct)> = Vec::new();
        if self.spell
            && !secret
            && let Some(sp) = vergence_spell::speller()
            && let Some((wa, wb)) = vergence_spell::words_to_check(&f.text).into_iter().find(|(wa, wb)| *wa <= at && at <= *wb)
        {
            let w = f.text[wa..wb].to_string();
            if !sp.check(&w) {
                let sug = sp.suggest(&w, 5);
                if sug.is_empty() {
                    items.push(("No suggestions".into(), FieldAct::Nothing));
                }
                for s in sug {
                    items.push((s.clone(), FieldAct::Replace(wa, wb, s)));
                }
                items.push((format!("Add “{w}” to the dictionary"), FieldAct::Add(w)));
            }
        }
        let selected = f.caret != f.anchor;
        if selected && editable && !secret {
            items.push(("Cut".into(), FieldAct::Cut));
        }
        if selected && !secret {
            items.push(("Copy".into(), FieldAct::Copy));
        }
        if editable {
            items.push(("Paste".into(), FieldAct::Paste));
        }
        if items.is_empty() {
            return;
        }
        self.focus = Some(id);
        // Its size from its words (measured again when drawn), kept on the window.
        let row = 24.0;
        let w = items.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(4) as f32 * 7.5 + 28.0;
        let h = items.len() as f32 * row + 8.0;
        let rx = x.min(self.size.0 - w - 4.0).max(4.0);
        let ry = if y + h <= self.size.1 - 4.0 { y } else { (y - h).max(4.0) };
        self.field_menu = Some(FieldMenu { id, items, hl: 0, rect: Rect::new(rx, ry, w, h), row });
    }

    /// Does line `k` of the field's menu.
    fn field_menu_choose(&mut self, k: usize, r: &mut Response<M>) {
        let Some(menu) = self.field_menu.take() else { return };
        let Some((_, act)) = menu.items.get(k).cloned() else { return };
        let id = menu.id;
        let Some(f) = self.fields.get_mut(&id) else { return };
        let (a, b) = (f.caret.min(f.anchor), f.caret.max(f.anchor));
        match act {
            FieldAct::Replace(wa, wb, with) => {
                let sel = (f.caret, f.anchor);
                if wb <= f.text.len() {
                    f.caret = f.history.edit(&mut f.text, wa, wb, &with, sel, crate::edit::Kind::Other, Instant::now());
                    f.anchor = f.caret;
                    self.field_changed(id, r);
                }
            }
            FieldAct::Add(w) => {
                if let Some(sp) = vergence_spell::speller() {
                    let _ = sp.add(&w);
                }
            }
            FieldAct::Cut | FieldAct::Copy => {
                r.copy = Some(f.text[a..b].to_string());
                if act == FieldAct::Cut {
                    let sel = (f.caret, f.anchor);
                    f.history.edit(&mut f.text, a, b, "", sel, crate::edit::Kind::Other, Instant::now());
                    f.caret = a;
                    f.anchor = a;
                    self.field_changed(id, r);
                }
            }
            FieldAct::Paste => r.paste = true,
            FieldAct::Nothing => {}
        }
        r.redraw = true;
        r.used = true;
    }

    /// Sends field `id`'s text if it changed.
    fn field_changed(&mut self, id: Id, r: &mut Response<M>) {
        let Some(i) = self.index(id) else { return };
        let What::Field { on_input: Some(f), .. } = &self.nodes[i].what else { return };
        let Some(state) = self.fields.get_mut(&id) else { return };
        if state.text != state.sent {
            state.sent = state.text.clone();
            r.messages.push(f(state.text.clone()));
        }
    }

    /// Types `text` into field `id` over its selection: its first line,
    /// without control characters, as much as the field has room for.
    fn field_insert(&mut self, id: Id, text: &str, kind: crate::edit::Kind, r: &mut Response<M>) {
        let editable = self.node(id).is_some_and(|n| matches!(n.what, What::Field { on_input: Some(_), .. }));
        let Some(f) = self.fields.get_mut(&id).filter(|_| editable) else { return };
        let clean: String = text.chars().take_while(|c| *c != '\n' && *c != '\r').filter(|c| !c.is_control()).collect();
        let (a, b) = (f.caret.min(f.anchor), f.caret.max(f.anchor));
        let room = MAX_FIELD.saturating_sub(f.text.len() - (b - a));
        let cut = boundary(&clean, room);
        let sel = (f.caret, f.anchor);
        f.caret = f.history.edit(&mut f.text, a, b, &clean[..cut], sel, kind, Instant::now());
        f.anchor = f.caret;
        self.caret_moved();
        self.field_changed(id, r);
        r.redraw = true;
        r.used = true;
    }

    /// A key in the focused field. True if the field used it.
    fn field_key(&mut self, id: Id, key: u32, text: Option<&str>, m: Modifiers, r: &mut Response<M>) -> bool {
        let Some(i) = self.index(id) else { return false };
        let (secret, editable, submit) = match &self.nodes[i].what {
            What::Field { secret, on_input, on_submit, .. } => (*secret, on_input.is_some(), on_submit.clone()),
            _ => return false,
        };
        let command = m.ctrl || m.logo;
        let letter = char::from_u32(key).map(|c| c.to_ascii_lowercase());
        let Some(f) = self.fields.get_mut(&id) else { return false };
        let (a, b) = (f.caret.min(f.anchor), f.caret.max(f.anchor));
        let selected = a != b;
        let mut edited = false;
        let mut moved = true;
        match key {
            sym::LEFT | sym::RIGHT | sym::HOME | sym::END => {
                let to = match key {
                    sym::LEFT if selected && !m.shift => a,
                    sym::RIGHT if selected && !m.shift => b,
                    sym::LEFT if command || m.alt => prev_word(&f.text, f.caret),
                    sym::RIGHT if command || m.alt => next_word(&f.text, f.caret),
                    sym::LEFT => prev_char(&f.text, f.caret),
                    sym::RIGHT => next_char(&f.text, f.caret),
                    sym::HOME => 0,
                    _ => f.text.len(),
                };
                f.caret = to;
                if !m.shift {
                    f.anchor = to;
                }
                f.history.break_group();
            }
            sym::BACKSPACE | sym::DELETE if editable => {
                let (from, to) = if selected {
                    (a, b)
                } else if key == sym::BACKSPACE {
                    (if command || m.alt { prev_word(&f.text, f.caret) } else { prev_char(&f.text, f.caret) }, f.caret)
                } else {
                    (f.caret, if command || m.alt { next_word(&f.text, f.caret) } else { next_char(&f.text, f.caret) })
                };
                let kind = if selected { crate::edit::Kind::Other } else { crate::edit::Kind::Deleting };
                let sel = (f.caret, f.anchor);
                f.history.edit(&mut f.text, from, to, "", sel, kind, Instant::now());
                f.caret = from;
                f.anchor = from;
                edited = from != to;
            }
            sym::RETURN => {
                if let Some(m) = submit {
                    r.messages.push(m);
                }
                moved = false;
            }
            _ if command && !m.alt => match letter {
                Some('a') => {
                    f.anchor = 0;
                    f.caret = f.text.len();
                }
                Some(c @ ('c' | 'x')) => {
                    // A secret is never copied.
                    if selected && !secret {
                        r.copy = Some(f.text[a..b].to_string());
                        if c == 'x' && editable {
                            let sel = (f.caret, f.anchor);
                            f.history.edit(&mut f.text, a, b, "", sel, crate::edit::Kind::Other, Instant::now());
                            f.caret = a;
                            f.anchor = a;
                            edited = true;
                        }
                    }
                }
                Some('v') => {
                    r.paste = editable;
                    moved = false;
                }
                // Undo, redo (Ctrl+Shift+Z or Ctrl+Y).
                Some(c @ ('z' | 'y')) if editable => {
                    let back = if c == 'z' && !m.shift { f.history.undo(&mut f.text) } else { f.history.redo(&mut f.text) };
                    let Some((caret, anchor)) = back else { return true };
                    f.caret = caret;
                    f.anchor = anchor;
                    edited = true;
                }
                _ => return false,
            },
            _ => match text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control) && !m.alt) {
                Some(t) => {
                    let t = t.to_string();
                    self.field_insert(id, &t, crate::edit::Kind::Typing, r);
                    return true;
                }
                None => return false,
            },
        }
        if edited {
            self.field_changed(id, r);
        }
        if moved {
            self.caret_moved();
        }
        r.redraw = true;
        r.used = true;
        true
    }

    /// Selects the word around `at` in field `id`.
    fn select_word(&mut self, id: Id, at: usize) {
        let Some(f) = self.fields.get_mut(&id) else { return };
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let mut a = at;
        while a > 0 && f.text[..a].chars().next_back().is_some_and(word) {
            a = prev_char(&f.text, a);
        }
        let mut b = at;
        while b < f.text.len() && f.text[b..].chars().next().is_some_and(word) {
            b = next_char(&f.text, b);
        }
        f.anchor = a;
        f.caret = b;
    }

    /// A press of the pointer or a finger at `x`, `y` on whatever is there.
    fn press_at(&mut self, x: f32, y: f32, with: With, r: &mut Response<M>) {
        let Some(id) = self.target(x, y) else {
            // The bare ground: a field being typed in there loses the keys.
            if self.focus.and_then(|f| self.node(f)).is_some_and(|n| matches!(n.what, What::Field { .. })) {
                self.focus = None;
                r.redraw = true;
            }
            return;
        };
        let i = self.index(id).expect("a target is in the tree");
        self.ring = false;
        r.redraw = true;
        r.used = true;
        match &self.nodes[i].what {
            What::Slider { value, .. } => {
                let start = *value;
                self.focus = Some(id);
                self.drag = Some(Drag::Slider { id, with, value: start });
                if let Some(v) = self.slider_at(i, x) {
                    self.slider_send(id, v, r);
                }
            }
            What::Field { .. } => {
                self.focus = Some(id);
                let at = self.field_offset(i, x);
                let now = Instant::now();
                let count = match self.click {
                    Some((t, cid, n)) if cid == id && now.duration_since(t) < self.double && with == With::Pointer => n % 3 + 1,
                    _ => 1,
                };
                self.click = Some((now, id, count));
                match count {
                    2 => self.select_word(id, at),
                    3 => {
                        if let Some(f) = self.fields.get_mut(&id) {
                            f.anchor = 0;
                            f.caret = f.text.len();
                        }
                    }
                    _ => {
                        if let Some(f) = self.fields.get_mut(&id) {
                            f.caret = at;
                            f.anchor = at;
                        }
                        if with == With::Pointer {
                            self.drag = Some(Drag::Select { id });
                        }
                    }
                }
                self.caret_moved();
            }
            What::Drawing { .. } => {
                self.drag = Some(Drag::Drawing { id, with });
                self.tell_drawing(id, Touch::Press { x, y }, r);
            }
            What::Area { doc, .. } => {
                let doc = doc.clone();
                self.focus = Some(id);
                let grab = doc.borrow().bar_hit(x, y);
                if let Some(grab) = grab {
                    doc.borrow_mut().bar_to(y, grab);
                    self.drag = Some(Drag::AreaBar { id, grab });
                    return;
                }
                let now = Instant::now();
                let count = match self.click {
                    Some((t, cid, n)) if cid == id && now.duration_since(t) < self.double && with == With::Pointer => n % 3 + 1,
                    _ => 1,
                };
                self.click = Some((now, id, count));
                let e = doc.borrow_mut().press(x, y, count, self.shift_press && with == With::Pointer);
                // A line of its popup chosen: done, no dragging.
                if e.used {
                    self.area_effect(id, e, r);
                    r.used = true;
                    return;
                }
                if with == With::Pointer {
                    self.drag = Some(Drag::AreaSelect { id });
                }
                self.caret_moved();
                self.area_changed(id, r);
            }
            // Buttons, switches and choices act when they are let go.
            _ => {
                self.pressed = Some((id, with));
                self.pressed_at = (x, y);
            }
        }
    }

    pub fn input(&mut self, input: &Input) -> Response<M> {
        let mut r = Response::default();
        // A field's menu, open, has the pointer and the keys it uses.
        if let Some(menu) = self.field_menu.clone() {
            let row_at = |x: f32, y: f32| -> Option<usize> {
                let m = &menu;
                (inside(&m.rect, x, y) && y >= m.rect.y + 4.0).then(|| (((y - m.rect.y - 4.0) / m.row) as usize).min(m.items.len() - 1))
            };
            match *input {
                Input::PointerMoved { x, y } => {
                    if let Some(k) = row_at(x, y)
                        && let Some(m) = self.field_menu.as_mut()
                    {
                        m.hl = k;
                    }
                    r.cursor = Some(Cursor::Default);
                    r.redraw = true;
                    r.used = true;
                    return r;
                }
                Input::Button { pressed: true, x, y, .. } => {
                    match row_at(x, y) {
                        Some(k) => self.field_menu_choose(k, &mut r),
                        None => self.field_menu = None,
                    }
                    r.redraw = true;
                    r.used = true;
                    return r;
                }
                Input::Key { sym: key, pressed: true, .. } => {
                    let n = menu.items.len();
                    match key {
                        sym::UP => self.field_menu.as_mut().map_or((), |m| m.hl = m.hl.saturating_sub(1)),
                        sym::DOWN => self.field_menu.as_mut().map_or((), |m| m.hl = (m.hl + 1).min(n - 1)),
                        sym::RETURN => self.field_menu_choose(menu.hl, &mut r),
                        _ => self.field_menu = None,
                    }
                    if self.field_menu.is_some() || key == sym::RETURN {
                        r.redraw = true;
                        r.used = true;
                        return r;
                    }
                }
                Input::Focus(false) => self.field_menu = None,
                _ => {}
            }
        }
        match *input {
            Input::PointerMoved { x, y } => self.pointer_moved(x, y, &mut r),
            Input::PointerLeft => {
                if self.hover.take().is_some() {
                    r.redraw = true;
                }
                if let Some(d) = self.over_drawing.take() {
                    self.tell_drawing(d, Touch::Leave, &mut r);
                }
            }
            Input::Button { button: MouseButton::Left, pressed: true, x, y, modifiers } => {
                self.shift_press = modifiers.shift;
                // Judged where the pointer was at this press, not where it is now.
                if self.open.is_some() {
                    self.press_with_list_open(x, y, &mut r);
                } else if let Some(s) = self.scroller_at(x, y)
                    && let Some(t) = self.thumb(s)
                    && x >= t.x - 2.0
                {
                    let grab = if inside(&t, x, y) { y - t.y } else { t.h / 2.0 };
                    self.drag = Some(Drag::Bar { id: self.nodes[s].id, grab });
                    self.bar_to(s, y, grab);
                    r.redraw = true;
                    r.used = true;
                } else {
                    self.press_at(x, y, With::Pointer, &mut r);
                }
            }
            // A right click on a field: its menu (spelling, the clipboard).
            Input::Button { button: MouseButton::Right, pressed: true, x, y, .. } if self.open.is_none() && self.target(x, y).and_then(|id| self.node(id)).is_some_and(|n| matches!(n.what, What::Field { .. })) => {
                if let Some(id) = self.target(x, y) {
                    self.open_field_menu(id, x, y);
                    self.caret_moved();
                    r.redraw = true;
                    r.used = true;
                }
            }
            // A right click on a text area: its menu (spelling, the clipboard).
            Input::Button { button: MouseButton::Right, pressed: true, x, y, .. } if self.open.is_none() && self.target(x, y).and_then(|id| self.area_doc(id)).is_some() => {
                if let Some(id) = self.target(x, y)
                    && let Some(doc) = self.area_doc(id)
                {
                    self.focus = Some(id);
                    self.ring = false;
                    doc.borrow_mut().context(x, y);
                    self.caret_moved();
                    self.area_changed(id, &mut r);
                    r.redraw = true;
                    r.used = true;
                }
            }
            // A right click on something with a menu message (Element::on_menu):
            // the innermost one under the pointer sends it.
            Input::Button { button: MouseButton::Right, pressed: true, x, y, .. } if self.open.is_none() && self.menu_at(x, y).is_some() => {
                if let Some(m) = self.menu_at(x, y) {
                    r.messages.push(m);
                    r.used = true;
                }
            }
            Input::Button { button: MouseButton::Left, pressed: false, x, y, .. } => {
                match self.drag.take() {
                    Some(Drag::Slider { id, with: With::Pointer, .. }) => self.slider_release(id, &mut r),
                    Some(Drag::Drawing { id, with: With::Pointer }) => self.tell_drawing(id, Touch::Release { x, y }, &mut r),
                    Some(Drag::Select { .. } | Drag::Bar { .. } | Drag::AreaSelect { .. } | Drag::AreaBar { .. }) => {
                        r.used = true;
                        r.redraw = true;
                    }
                    other => self.drag = other,
                }
                if let Some((id, With::Pointer)) = self.pressed {
                    self.pressed = None;
                    if self.target(x, y) == Some(id) {
                        self.activate(id, &mut r);
                    }
                    r.redraw = true;
                    r.used = true;
                }
            }
            Input::Scroll { x, y, dx, dy } => {
                let area = self.target(x, y).and_then(|id| self.area_doc(id));
                if self.open.is_none()
                    && let Some(doc) = area
                {
                    doc.borrow_mut().wheel(dx * WHEEL, dy * WHEEL);
                    r.redraw = true;
                    r.used = true;
                } else if self.open.is_some() {
                    if let Some((lr, item, n)) = self.list_rect()
                        && let Some(o) = self.open.as_mut()
                    {
                        let max = (n as f32 * item + 8.0 - lr.h).max(0.0);
                        o.offset = (o.offset + dy * WHEEL).clamp(0.0, max);
                    }
                    r.redraw = true;
                    r.used = true;
                } else if let Some(s) = self.scroller_at(x, y) {
                    if let Some(st) = self.scrolls.get_mut(&self.nodes[s].id) {
                        st.velocity = 0.0;
                    }
                    r.redraw = self.scroll_by(s, dy * WHEEL);
                    r.used = true;
                    // What is now under the pointer, which did not move.
                    let t = self.target(x, y);
                    if t != self.hover {
                        self.hover = t;
                        r.redraw = true;
                    }
                }
            }
            Input::Touch { id: finger, phase, x, y } => self.touch(finger, phase, x, y, &mut r),
            Input::Key { sym: key, ref text, pressed: true, repeat, composing: false, modifiers } => self.key(key, text.as_deref(), repeat, modifiers, &mut r),
            Input::Key { sym: key, pressed: false, .. } if key == sym::RETURN || key == ' ' as u32 => {
                if let Some((_, With::Key)) = self.pressed {
                    self.pressed = None;
                    r.redraw = true;
                    r.used = true;
                }
            }
            Input::Paste(ref text) => {
                if let Some(id) = self.focus {
                    if let Some(doc) = self.area_doc(id) {
                        let changed = doc.borrow_mut().insert(text);
                        self.area_effect(id, crate::textarea::Effect { used: true, text: changed, moved: changed, ..Default::default() }, &mut r);
                    } else {
                        self.field_insert(id, text, crate::edit::Kind::Other, &mut r);
                    }
                }
            }
            Input::Focus(false) => {
                self.pressed = None;
                self.fingers.clear();
                match self.drag.take() {
                    Some(Drag::Slider { id, .. }) => self.slider_release(id, &mut r),
                    Some(Drag::Drawing { id, .. }) => self.tell_drawing(id, Touch::Leave, &mut r),
                    _ => {}
                }
                r.redraw = true;
            }
            _ => {}
        }
        r
    }

    fn bar_to(&mut self, s: usize, y: f32, grab: f32) {
        let What::Scroll { content } = self.nodes[s].what else { return };
        let area = self.shown(s);
        let Some(t) = self.thumb(s) else { return };
        // The track starts below the window's bar, as the thumb does.
        let top = area.y.max(self.under_bar.min(area.y + area.h - 24.0));
        let room = (area.y + area.h - top - t.h).max(1.0);
        let f = ((y - grab - top) / room).clamp(0.0, 1.0);
        let want = f * (content - area.h);
        let have = self.scrolls.get(&self.nodes[s].id).map_or(0.0, |st| st.offset);
        self.scroll_by(s, want - have);
    }

    fn pointer_moved(&mut self, x: f32, y: f32, r: &mut Response<M>) {
        // A button held and pulled away: its drag message, and no press.
        if let Some((id, With::Pointer)) = self.pressed
            && ((x - self.pressed_at.0).powi(2) + (y - self.pressed_at.1).powi(2)) > 36.0
            && let Some(m) = self.node(id).and_then(|n| match &n.what {
                What::Button { on_drag: Some(m), .. } => Some(m.clone()),
                _ => None,
            })
        {
            self.pressed = None;
            r.messages.push(m);
            r.used = true;
            r.redraw = true;
            return;
        }
        match self.drag {
            Some(Drag::Slider { id, with: With::Pointer, .. }) => {
                if let Some(v) = self.index(id).and_then(|i| self.slider_at(i, x)) {
                    self.slider_send(id, v, r);
                }
            }
            Some(Drag::Select { id }) => {
                if let Some(i) = self.index(id) {
                    let at = self.field_offset(i, x);
                    if let Some(f) = self.fields.get_mut(&id) {
                        f.caret = at;
                    }
                    self.caret_moved();
                    r.redraw = true;
                }
            }
            Some(Drag::Bar { id, grab }) => {
                if let Some(s) = self.index(id) {
                    self.bar_to(s, y, grab);
                    r.redraw = true;
                }
            }
            Some(Drag::Drawing { id, with: With::Pointer }) => self.tell_drawing(id, Touch::Move { x, y, pressed: true }, r),
            Some(Drag::AreaSelect { id }) => {
                if let Some(doc) = self.area_doc(id) {
                    doc.borrow_mut().drag_to(x, y);
                    self.caret_moved();
                    self.area_changed(id, r);
                    r.redraw = true;
                }
            }
            Some(Drag::AreaBar { id, grab }) => {
                if let Some(doc) = self.area_doc(id) {
                    doc.borrow_mut().bar_to(y, grab);
                    r.redraw = true;
                }
            }
            _ => {}
        }
        if self.open.is_some() {
            let hl = self.item_at(x, y);
            if let Some(o) = self.open.as_mut()
                && hl.is_some()
                && o.hl != hl
            {
                o.hl = hl;
                r.redraw = true;
            }
            r.cursor = Some(Cursor::Default);
            r.used = true;
            return;
        }
        let t = self.target(x, y);
        if t != self.hover {
            self.hover = t;
            r.redraw = true;
        }
        let what = t.and_then(|id| self.node(id)).map(|n| &n.what);
        let on_bar = matches!(what, Some(What::Area { doc, .. }) if doc.borrow().bar_hit(x, y).is_some());
        // A drawing pulled keeps its shape wherever the pointer goes.
        let pulled = match self.drag {
            Some(Drag::Drawing { id, .. }) => self.node(id).and_then(|n| match n.what {
                What::Drawing { cursor, .. } => cursor,
                _ => None,
            }),
            _ => None,
        };
        r.cursor = Some(match (what, self.drag) {
            _ if pulled.is_some() => pulled.unwrap_or(Cursor::Default),
            (_, Some(Drag::AreaBar { .. })) => Cursor::Default,
            (Some(What::Area { .. }), None) if on_bar => Cursor::Default,
            (_, Some(Drag::Select { .. } | Drag::AreaSelect { .. })) | (Some(What::Field { .. } | What::Area { .. }), _) => Cursor::Text,
            (Some(What::Drawing { cursor, .. }), _) => cursor.unwrap_or(Cursor::Default),
            (None, _) => Cursor::Default,
            (Some(_), _) => Cursor::Pointer,
        });
        // A drawing hears the pointer pass over it.
        let drawing = t.filter(|id| self.node(*id).is_some_and(|n| matches!(n.what, What::Drawing { .. })));
        if self.drag.is_none() {
            if self.over_drawing != drawing
                && let Some(d) = self.over_drawing
            {
                self.tell_drawing(d, Touch::Leave, r);
            }
            self.over_drawing = drawing;
            if let Some(d) = drawing {
                self.tell_drawing(d, Touch::Move { x, y, pressed: false }, r);
            }
        }
        r.used |= t.is_some() || self.pressed.is_some() || self.drag.is_some();
    }

    fn press_with_list_open(&mut self, x: f32, y: f32, r: &mut Response<M>) {
        if let Some(k) = self.item_at(x, y) {
            self.choose(k, r);
        } else if self.list_rect().is_some_and(|(lr, _, _)| inside(&lr, x, y)) {
            r.used = true;
        } else {
            // Anywhere else closes the list, and does nothing more.
            self.open = None;
            r.redraw = true;
            r.used = true;
        }
    }

    fn touch(&mut self, finger: i32, phase: Phase, x: f32, y: f32, r: &mut Response<M>) {
        let now = Instant::now();
        match phase {
            Phase::Down => {
                self.fingers.retain(|f| f.0 != finger);
                self.fingers.push((finger, x, y));
                if self.drag.is_some() || self.pressed.is_some() {
                    return;
                }
                if self.open.is_some() {
                    self.press_with_list_open(x, y, r);
                    return;
                }
                // A finger on a text area scrolls it, or taps the caret there.
                if let Some(id) = self.target(x, y).filter(|id| self.area_doc(*id).is_some()) {
                    self.drag = Some(Drag::AreaTouch { id, finger, start: y, last: y, moved: false });
                    r.used = true;
                    return;
                }
                let grabs = self.target(x, y).and_then(|id| self.node(id)).map(|n| matches!(n.what, What::Slider { .. } | What::Drawing { .. } | What::Field { .. }));
                match grabs {
                    // These take a finger at once: it drags them, not the page.
                    Some(true) => self.press_at(x, y, With::Finger(finger), r),
                    _ => {
                        if let Some(s) = self.scroller_at(x, y) {
                            if let Some(st) = self.scrolls.get_mut(&self.nodes[s].id) {
                                st.velocity = 0.0;
                            }
                            self.drag = Some(Drag::Flick { id: self.nodes[s].id, finger, start: y, last: y, at: now, moved: false, velocity: 0.0 });
                            r.used = true;
                        }
                        if grabs.is_some() {
                            self.press_at(x, y, With::Finger(finger), r);
                        }
                    }
                }
            }
            Phase::Moved => {
                if let Some(f) = self.fingers.iter_mut().find(|f| f.0 == finger) {
                    (f.1, f.2) = (x, y);
                }
                match self.drag {
                    Some(Drag::Flick { id, finger: f, start, last, at, moved, velocity }) if f == finger => {
                        let moved = moved || (y - start).abs() > SLOP;
                        let mut v = velocity;
                        if moved {
                            // The press it began as is not a press.
                            if self.pressed.is_some_and(|p| p.1 == With::Finger(finger)) {
                                self.pressed = None;
                            }
                            if let Some(s) = self.index(id) {
                                self.scroll_by(s, last - y);
                            }
                            let dt = now.saturating_duration_since(at).as_secs_f32().max(0.004);
                            v = 0.6 * ((last - y) / dt) + 0.4 * velocity;
                        }
                        self.drag = Some(Drag::Flick { id, finger, start, last: if moved { y } else { last }, at: now, moved, velocity: v });
                        r.redraw = true;
                        r.used = true;
                    }
                    Some(Drag::Slider { id, with: With::Finger(f), .. }) if f == finger => {
                        if let Some(v) = self.index(id).and_then(|i| self.slider_at(i, x)) {
                            self.slider_send(id, v, r);
                        }
                    }
                    Some(Drag::Drawing { id, with: With::Finger(f) }) if f == finger => self.tell_drawing(id, Touch::Move { x, y, pressed: true }, r),
                    Some(Drag::AreaTouch { id, finger: f, start, last, moved }) if f == finger => {
                        let moved = moved || (y - start).abs() > SLOP;
                        if moved && let Some(doc) = self.area_doc(id) {
                            doc.borrow_mut().wheel(0.0, last - y);
                        }
                        self.drag = Some(Drag::AreaTouch { id, finger, start, last: if moved { y } else { last }, moved });
                        r.redraw = true;
                        r.used = true;
                    }
                    _ => {
                        r.used = self.pressed.is_some_and(|p| p.1 == With::Finger(finger));
                        r.redraw = r.used;
                    }
                }
            }
            Phase::Up | Phase::Cancelled => {
                self.fingers.retain(|f| f.0 != finger);
                let up = phase == Phase::Up;
                match self.drag {
                    Some(Drag::Flick { id, finger: f, moved, velocity, at, .. }) if f == finger => {
                        self.drag = None;
                        // A flick goes on by itself; a finger that stopped before lifting does not.
                        if up && moved && now.saturating_duration_since(at) < Duration::from_millis(80) && velocity.abs() > 100.0 {
                            if let Some(st) = self.scrolls.get_mut(&id) {
                                st.velocity = velocity.clamp(-6000.0, 6000.0);
                            }
                            self.last_tick = Some(now);
                        }
                        r.used = true;
                        r.redraw = true;
                    }
                    Some(Drag::Slider { id, with: With::Finger(f), .. }) if f == finger => {
                        self.drag = None;
                        self.slider_release(id, r);
                    }
                    Some(Drag::Drawing { id, with: With::Finger(f) }) if f == finger => {
                        self.drag = None;
                        self.tell_drawing(id, if up { Touch::Release { x, y } } else { Touch::Leave }, r);
                    }
                    Some(Drag::AreaTouch { id, finger: f, moved, .. }) if f == finger => {
                        self.drag = None;
                        if up && !moved && let Some(doc) = self.area_doc(id) {
                            self.focus = Some(id);
                            self.ring = false;
                            doc.borrow_mut().press(x, y, 1, false);
                            self.caret_moved();
                            self.area_changed(id, r);
                        }
                        r.used = true;
                        r.redraw = true;
                    }
                    _ => {}
                }
                if let Some((id, With::Finger(f))) = self.pressed
                    && f == finger
                {
                    self.pressed = None;
                    if up && self.target(x, y) == Some(id) {
                        self.activate(id, r);
                    }
                    r.redraw = true;
                    r.used = true;
                }
            }
        }
    }

    fn key(&mut self, key: u32, text: Option<&str>, repeat: bool, m: Modifiers, r: &mut Response<M>) {
        let plain = !m.ctrl && !m.alt && !m.logo;
        let press = (key == sym::RETURN || key == ' ' as u32) && plain && !m.shift && !repeat;
        // An open list has the keys.
        if let Some(o) = self.open {
            let n = match self.node(o.id).map(|n| &n.what) {
                Some(What::Choice { options, .. }) => options.len(),
                _ => 0,
            };
            match key {
                sym::UP | sym::DOWN | sym::HOME | sym::END | sym::PAGE_UP | sym::PAGE_DOWN if n > 0 => {
                    let cur = o.hl.map_or(-1, |h| h as isize);
                    let to = match key {
                        sym::UP => cur - 1,
                        sym::DOWN => cur + 1,
                        sym::PAGE_UP => cur - 8,
                        sym::PAGE_DOWN => cur + 8,
                        sym::HOME => 0,
                        _ => n as isize - 1,
                    };
                    if let Some(op) = self.open.as_mut() {
                        op.hl = Some(to.clamp(0, n as isize - 1) as usize);
                    }
                    self.reveal_open();
                }
                _ if press => match o.hl {
                    Some(k) => self.choose(k, r),
                    None => self.open = None,
                },
                sym::ESCAPE | sym::TAB => self.open = None,
                _ => {}
            }
            r.redraw = true;
            r.used = true;
            return;
        }
        // A focused text area next: every key it uses, Tab too.
        if let Some(id) = self.focus
            && let Some(doc) = self.area_doc(id)
        {
            let e = doc.borrow_mut().key(key, text, m);
            if e.used {
                self.area_effect(id, e, r);
                return;
            }
        }
        // The focused field next: everything but Tab.
        if let Some(id) = self.focus
            && key != sym::TAB
            && self.node(id).is_some_and(|n| matches!(n.what, What::Field { .. }))
            && self.field_key(id, key, text, m, r)
        {
            return;
        }
        if key == sym::TAB && plain {
            let list = self.focusables();
            if !list.is_empty() {
                let at = self.focus.and_then(|f| list.iter().position(|i| *i == f));
                let next = match (at, m.shift) {
                    (None, false) => 0,
                    (None, true) => list.len() - 1,
                    (Some(i), false) => (i + 1) % list.len(),
                    (Some(i), true) => (i + list.len() - 1) % list.len(),
                };
                self.focus = Some(list[next]);
                self.ring = true;
                if let Some(i) = self.index(list[next]) {
                    self.reveal(i);
                }
                if let Some(f) = self.fields.get_mut(&list[next]) {
                    // A field reached with Tab has all its text chosen.
                    f.anchor = 0;
                    f.caret = f.text.len();
                }
                self.caret_moved();
                r.redraw = true;
                r.used = true;
            }
            return;
        }
        let focused = self.focus.and_then(|id| self.index(id));
        if let Some(i) = focused {
            let id = self.nodes[i].id;
            match &self.nodes[i].what {
                What::Button { .. } | What::Switch { .. } if press => {
                    self.pressed = Some((id, With::Key));
                    self.activate(id, r);
                    r.used = true;
                    return;
                }
                What::Choice { .. } if press || key == sym::DOWN && plain => {
                    self.activate(id, r);
                    r.used = true;
                    return;
                }
                What::Slider { min, max, step, value, .. } if plain => {
                    let (min, max, value) = (*min, *max, *value);
                    let unit = if *step > 0.0 { *step } else { (max - min) / 100.0 };
                    let to = match key {
                        sym::LEFT | sym::DOWN => Some(value - unit),
                        sym::RIGHT | sym::UP => Some(value + unit),
                        sym::PAGE_DOWN => Some(value - unit * 10.0),
                        sym::PAGE_UP => Some(value + unit * 10.0),
                        sym::HOME => Some(min),
                        sym::END => Some(max),
                        _ => None,
                    };
                    if let Some(v) = to {
                        self.slider_send(id, v.clamp(min, max), r);
                        self.slider_release(id, r);
                        return;
                    }
                }
                _ => {}
            }
        }
        // Page keys move the page the focus is on, or the first that scrolls.
        if matches!(key, sym::PAGE_UP | sym::PAGE_DOWN | sym::HOME | sym::END) && plain {
            let s = focused.and_then(|i| self.scroller_of(i)).or_else(|| (0..self.nodes.len()).find(|&i| matches!(self.nodes[i].what, What::Scroll { .. })));
            if let Some(s) = s {
                let page = (self.nodes[s].rect.h - 40.0).max(20.0);
                let dy = match key {
                    sym::PAGE_UP => -page,
                    sym::PAGE_DOWN => page,
                    sym::HOME => -1e9,
                    _ => 1e9,
                };
                r.redraw = self.scroll_by(s, dy);
                r.used = true;
            }
            return;
        }
        if key == sym::ESCAPE && self.ring {
            self.ring = false;
            r.redraw = true;
        }
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
    pub fn draw(&mut self, scene: &mut Scene, fonts: &mut Fonts, theme: &Theme) {
        let now = Instant::now();
        self.spell = theme.spell;
        let face = |fonts: &mut Fonts, weight: u16| fonts.face(&theme.font, weight, false).or_else(|| fonts.face("sans-serif", weight, false));
        let mut ink: Vec<Color> = Vec::with_capacity(self.nodes.len());
        // The scrolling places being drawn into, the innermost last.
        let mut open: Vec<usize> = Vec::new();
        let mut field_stops: Vec<(Id, Vec<(usize, f32)>, f32)> = Vec::new();
        let ancestor = |nodes: &[Node<M>], a: usize, mut i: usize| {
            while let Some(p) = nodes[i].parent {
                if p == a {
                    return true;
                }
                i = p;
            }
            false
        };
        let ring = |scene: &mut Scene, r: Rect, radius: f32| {
            let g = 2.0;
            scene.stroke(Rect::new(r.x - g, r.y - g, r.w + 2.0 * g, r.h + 2.0 * g), radius + g, 2.0, theme.accent);
        };
        let edge_c = if theme.high_contrast { theme.text } else { theme.line };
        let white = Color::rgb(255, 255, 255);
        for i in 0..self.nodes.len() {
            while let Some(&top) = open.last() {
                if ancestor(&self.nodes, top, i) {
                    break;
                }
                open.pop();
                scene.pop_clip();
                self.draw_bar(scene, top, theme);
            }
            let n = &self.nodes[i];
            let inherited = n.parent.map_or(theme.text, |p| ink[p]);
            let mut mine = inherited;
            let r = self.shown(i);
            let seen = {
                let v = meet(r, self.clip_of(i));
                v.w > 0.0 && v.h > 0.0
            };
            let ringed = self.ring && self.focus == Some(n.id);
            match &n.what {
                What::Space | What::Group => {}
                What::Scroll { .. } => {
                    scene.push_clip(r, 0.0);
                    open.push(i);
                }
                // Out of sight in a scrolling place: not drawn.
                _ if !seen => {}
                What::Rule { color } => {
                    scene.fill(r, color.unwrap_or(theme.line));
                }
                What::Label { lines, size, weight, color, dim } => {
                    let col = color.unwrap_or(if *dim { theme.dim } else { inherited });
                    if let Some(f) = face(fonts, *weight) {
                        for l in lines {
                            text(scene, fonts, f, *size, col, Point::new(r.x + l.x, r.y + l.baseline), &l.text);
                        }
                    }
                }
                What::Panel { fill, edge, radius } => {
                    if let Some(f) = fill {
                        scene.fill_rounded(r, *radius, *f);
                    }
                    if let Some((c, w)) = edge {
                        scene.stroke(r, *radius, *w, *c);
                    }
                }
                What::Button { on_press, look, .. } => {
                    let on = on_press.is_some();
                    let hot = on && self.hover == Some(n.id);
                    let down = on && self.held(n.id, &r);
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
                    let (fill, edge, rad, i_c) = match *look {
                        ButtonLook::Normal => (tint(Some(theme.page)), Some((edge_c, 1.0)), radius, theme.text),
                        ButtonLook::Plain => (tint(None), None, radius, inherited),
                        ButtonLook::Accent => (tint(Some(theme.accent)), Some((theme.accent, 1.0)), radius, white),
                        ButtonLook::Chosen => (Some(theme.accent), None, radius, white),
                        ButtonLook::Custom { fill, edge, radius } => (tint(fill), edge, radius, inherited),
                    };
                    // One that cannot be pressed is faded, its own color kept.
                    let fade = |c: Color| if on { c } else { Color { a: c.a / 2, ..c } };
                    if let Some(f) = fill {
                        scene.fill_rounded(r, rad, fade(f));
                    }
                    if let Some((c, w)) = edge {
                        scene.stroke(r, rad, w, fade(c));
                    }
                    let strong = matches!(look, ButtonLook::Accent | ButtonLook::Chosen);
                    mine = if on { i_c } else if strong { alpha(i_c, 170) } else { theme.dim };
                    if ringed {
                        ring(scene, r, rad);
                    }
                }
                What::Switch { on, on_toggle } => {
                    let usable = on_toggle.is_some();
                    let t = match self.switches.get(&n.id).and_then(|s| s.changed) {
                        Some(at) if !self.steady => (now.saturating_duration_since(at).as_secs_f32() / SLIDE.as_secs_f32()).min(1.0),
                        _ => 1.0,
                    };
                    let pos = if *on { t } else { 1.0 - t };
                    let off_c = if theme.high_contrast { theme.dim } else { over(theme.line, alpha(theme.text, 40)) };
                    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * pos).round() as u8;
                    let mut track = Color::rgb(l(off_c.r, theme.accent.r), l(off_c.g, theme.accent.g), l(off_c.b, theme.accent.b));
                    if !usable {
                        track = alpha(track, 110);
                    }
                    scene.fill_rounded(r, r.h / 2.0, track);
                    let d = r.h - 4.0;
                    let kx = r.x + 2.0 + (r.w - 4.0 - d) * pos;
                    scene.fill_rounded(Rect::new(kx, r.y + 2.0, d, d), d / 2.0, if usable { white } else { alpha(white, 160) });
                    if usable && self.hover == Some(n.id) {
                        scene.fill_rounded(r, r.h / 2.0, theme.hover);
                    }
                    if ringed {
                        ring(scene, r, r.h / 2.0);
                    }
                }
                What::Slider { min, max, value, marks, on_change, .. } => {
                    let usable = on_change.is_some();
                    let v = match self.drag {
                        Some(Drag::Slider { id, value, .. }) if id == n.id => value,
                        _ => *value,
                    };
                    let (x0, w) = slider_track(&r);
                    let f = ((v - min) / (max - min)).clamp(0.0, 1.0);
                    let cy = r.y + (r.h / 2.0).round() - if marks.is_empty() { 0.0 } else { 2.0 };
                    let line = if theme.high_contrast { theme.dim } else { over(theme.line, alpha(theme.text, 30)) };
                    scene.fill_rounded(Rect::new(x0, cy - 2.0, w, 4.0), 2.0, line);
                    scene.fill_rounded(Rect::new(x0, cy - 2.0, w * f, 4.0), 2.0, if usable { theme.accent } else { theme.dim });
                    for m in marks {
                        let mf = ((m - min) / (max - min)).clamp(0.0, 1.0);
                        scene.fill(Rect::new((x0 + w * mf - 0.5).round(), cy + 5.0, 1.0, 5.0), theme.dim);
                    }
                    let thumb = Rect::new(x0 + w * f - THUMB / 2.0, cy - THUMB / 2.0, THUMB, THUMB);
                    let held = matches!(self.drag, Some(Drag::Slider { id, .. }) if id == n.id);
                    let mut tc = white;
                    if held || self.hover == Some(n.id) {
                        tc = over(tc, alpha(Color::rgb(0, 0, 0), 25));
                    }
                    scene.fill_rounded(thumb, THUMB / 2.0, tc);
                    scene.stroke(thumb, THUMB / 2.0, 1.0, if theme.high_contrast { theme.text } else { alpha(Color::rgb(0, 0, 0), 60) });
                    if ringed {
                        ring(scene, thumb, THUMB / 2.0);
                    }
                }
                What::Field { placeholder, secret, on_input, size, baseline, .. } => {
                    let focused = self.focus == Some(n.id);
                    let usable = on_input.is_some();
                    let radius = vergence_look::RADIUS;
                    scene.fill_rounded(r, radius, if usable { theme.page } else { theme.card });
                    if focused {
                        scene.stroke(r, radius, 2.0, theme.accent);
                    } else {
                        scene.stroke(r, radius, 1.0, edge_c);
                    }
                    let inner = Rect::new(r.x + FIELD_PAD_H, r.y + 1.0, (r.w - 2.0 * FIELD_PAD_H).max(0.0), (r.h - 2.0).max(0.0));
                    if let Some(f) = face(fonts, 400) {
                        let state = self.fields.get(&n.id);
                        let now_text = state.map_or("", |s| s.text.as_str());
                        let shown_text: String = if *secret { "\u{2022}".repeat(now_text.chars().count()) } else { now_text.to_string() };
                        // Where each character boundary falls: a glyph for each character.
                        let (runs, total) = fonts.layout(f, *size, &shown_text);
                        let xs: Vec<f32> = runs.iter().flat_map(|run| run.glyphs.iter().map(|g| g.x)).collect();
                        let mut stops: Vec<(usize, f32)> = now_text.char_indices().filter(|(_, c)| !c.is_control()).map(|(b, _)| b).zip(xs.iter().copied()).collect();
                        stops.push((now_text.len(), total));
                        let (caret, anchor) = state.map_or((0, 0), |s| (s.caret, s.anchor));
                        let at = |b: usize| stops.iter().find(|s| s.0 == b).map_or(total, |s| s.1);
                        let mut shift = state.map_or(0.0, |s| s.shift);
                        let cx = at(caret);
                        if cx - shift > inner.w - 2.0 {
                            shift = cx - inner.w + 2.0;
                        }
                        if cx < shift {
                            shift = cx;
                        }
                        shift = shift.clamp(0.0, (total - inner.w + 2.0).max(0.0));
                        scene.push_clip(inner, 0.0);
                        if focused && caret != anchor {
                            let (a, b) = (at(caret.min(anchor)), at(caret.max(anchor)));
                            scene.fill(Rect::new(inner.x + a - shift, r.y + 4.0, b - a, r.h - 8.0), alpha(theme.accent, 90));
                        }
                        if now_text.is_empty() {
                            text(scene, fonts, f, *size, theme.dim, Point::new(inner.x, r.y + baseline), placeholder);
                        } else {
                            text(scene, fonts, f, *size, if usable { theme.text } else { theme.dim }, Point::new(inner.x - shift, r.y + baseline), &shown_text);
                        }
                        // Words spelled wrong, a red wave under each (not the one being typed).
                        if theme.spell
                            && !*secret
                            && usable
                            && let Some(sp) = vergence_spell::speller()
                        {
                            let wave = if theme.dark { Color::rgb(255, 99, 92) } else { Color::rgb(214, 40, 40) };
                            let wy = (r.y + baseline + 3.0).min(r.y + r.h - 3.0);
                            for (wa, wb) in vergence_spell::words_to_check(now_text) {
                                let typing = focused && caret == anchor && caret == wb;
                                if typing || sp.check(&now_text[wa..wb]) {
                                    continue;
                                }
                                let (x1, x2) = (inner.x + at(wa) - shift, inner.x + at(wb) - shift);
                                let mut x = x1;
                                let mut up = true;
                                while x < x2 {
                                    let nx = (x + 2.0).min(x2);
                                    let (a, b) = if up { (wy + 1.0, wy - 1.0) } else { (wy - 1.0, wy + 1.0) };
                                    scene.line(Point::new(x, a), Point::new(nx, b), 1.0, wave);
                                    x = nx;
                                    up = !up;
                                }
                            }
                        }
                        if focused && caret == anchor && self.caret_on() {
                            scene.fill(Rect::new((inner.x + cx - shift).round() - 0.5, r.y + 4.0, 1.5, r.h - 8.0), theme.text);
                        }
                        scene.pop_clip();
                        field_stops.push((n.id, stops, shift));
                    }
                }
                What::Choice { options, selected, placeholder, on_select, size, baseline, .. } => {
                    let usable = on_select.is_some();
                    let hot = usable && (self.hover == Some(n.id) || self.open.is_some_and(|o| o.id == n.id));
                    let radius = vergence_look::RADIUS;
                    let fill = if hot { over(theme.page, theme.hover) } else { theme.page };
                    scene.fill_rounded(r, radius, fill);
                    scene.stroke(r, radius, 1.0, edge_c);
                    let col = if usable { theme.text } else { theme.dim };
                    let inner = Rect::new(r.x + FIELD_PAD_H, r.y, (r.w - 2.0 * FIELD_PAD_H - CHEVRON).max(0.0), r.h);
                    if let Some(f) = face(fonts, 400) {
                        let (shown_text, c) = match *selected {
                            Some(k) => (options[k].as_str(), col),
                            None => (placeholder.as_str(), theme.dim),
                        };
                        scene.push_clip(inner, 0.0);
                        text(scene, fonts, f, *size, c, Point::new(inner.x, r.y + baseline), shown_text);
                        scene.pop_clip();
                    }
                    // The arrow: a small V.
                    let (ax, ay) = (r.x + r.w - FIELD_PAD_H - 5.0, r.y + r.h / 2.0);
                    scene.line(Point::new(ax - 4.0, ay - 2.0), Point::new(ax, ay + 2.0), 1.5, theme.dim);
                    scene.line(Point::new(ax, ay + 2.0), Point::new(ax + 4.0, ay - 2.0), 1.5, theme.dim);
                    if ringed {
                        ring(scene, r, radius);
                    }
                }
                What::Picture { image, fit, radius, opacity } => {
                    let (iw, ih) = (image.width.max(1) as f32, image.height.max(1) as f32);
                    let dest = match fit {
                        Fit::Stretch => r,
                        Fit::Contain | Fit::Cover => {
                            let k = if *fit == Fit::Contain { (r.w / iw).min(r.h / ih) } else { (r.w / iw).max(r.h / ih) };
                            let (w, h) = (iw * k, ih * k);
                            Rect::new(r.x + (r.w - w) / 2.0, r.y + (r.h - h) / 2.0, w, h)
                        }
                    };
                    let id = scene.image(image.clone());
                    scene.push_clip(r, *radius);
                    scene.draw_image(id, dest, *opacity);
                    scene.pop_clip();
                }
                What::Drawing { draw, .. } => {
                    scene.push_clip(r, 0.0);
                    draw(scene, fonts, r, theme);
                    scene.pop_clip();
                }
                What::Area { doc, .. } => {
                    let focused = self.focus == Some(n.id);
                    // The part of it under the window's bar (look milestone 1).
                    let inset = (self.under_bar - r.y).max(0.0);
                    doc.borrow_mut().draw(scene, fonts, r, inset, theme, focused, self.caret_on());
                    if ringed {
                        ring(scene, r, 0.0);
                    }
                }
            }
            ink.push(mine);
        }
        while let Some(top) = open.pop() {
            scene.pop_clip();
            self.draw_bar(scene, top, theme);
        }
        for (id, stops, shift) in field_stops {
            if let Some(f) = self.fields.get_mut(&id) {
                f.stops = stops;
                f.shift = shift;
            }
        }
        self.draw_list(scene, fonts, theme);
        self.draw_field_menu(scene, fonts, theme);
    }

    /// A field's right-click menu, in front of everything.
    fn draw_field_menu(&self, scene: &mut Scene, fonts: &mut Fonts, theme: &Theme) {
        let Some(m) = &self.field_menu else { return };
        let r = m.rect;
        let ground = if theme.dark { over(theme.card, alpha(Color::rgb(255, 255, 255), 10)) } else { Color::rgb(255, 255, 255) };
        scene.fill_rounded(Rect::new(r.x - 1.0, r.y + 1.0, r.w + 2.0, r.h + 2.0), vergence_look::RADIUS + 1.0, alpha(Color::rgb(0, 0, 0), 50));
        scene.fill_rounded(r, vergence_look::RADIUS, ground);
        scene.stroke(r, vergence_look::RADIUS, 1.0, if theme.high_contrast { theme.text } else { theme.line });
        let Some(face) = fonts.face(&theme.font, 400, false).or_else(|| fonts.face("sans-serif", 400, false)) else { return };
        let size = theme.text_size;
        let ascent = fonts.metrics(face, size).ascent;
        for (k, (label, act)) in m.items.iter().enumerate() {
            let row = Rect::new(r.x + 4.0, r.y + 4.0 + k as f32 * m.row, r.w - 8.0, m.row);
            let hl = k == m.hl && *act != FieldAct::Nothing;
            if hl {
                scene.fill_rounded(row, vergence_look::RADIUS, theme.accent);
            }
            let col = if hl { Color::rgb(255, 255, 255) } else if *act == FieldAct::Nothing { theme.dim } else { theme.text };
            text(scene, fonts, face, size, col, Point::new(row.x + 8.0, (row.y + (m.row - size * 1.2) / 2.0 + ascent).round()), label);
        }
    }

    fn draw_bar(&self, scene: &mut Scene, i: usize, theme: &Theme) {
        if let Some(t) = self.thumb(i) {
            let held = matches!(self.drag, Some(Drag::Bar { id, .. }) if id == self.nodes[i].id);
            scene.fill_rounded(t, BAR_W / 2.0, alpha(theme.text, if held { 150 } else { 80 }));
        }
    }

    /// The open choice's list, in front of everything.
    fn draw_list(&self, scene: &mut Scene, fonts: &mut Fonts, theme: &Theme) {
        let Some(o) = self.open else { return };
        let Some((r, item, _)) = self.list_rect() else { return };
        let Some(Node { what: What::Choice { options, selected, size, step, .. }, .. }) = self.node(o.id) else { return };
        let ground = if theme.dark { over(theme.card, alpha(Color::rgb(255, 255, 255), 10)) } else { Color::rgb(255, 255, 255) };
        // A shadow, then the list.
        scene.fill_rounded(Rect::new(r.x - 1.0, r.y + 1.0, r.w + 2.0, r.h + 2.0), vergence_look::RADIUS + 1.0, alpha(Color::rgb(0, 0, 0), 50));
        scene.fill_rounded(r, vergence_look::RADIUS, ground);
        scene.stroke(r, vergence_look::RADIUS, 1.0, if theme.high_contrast { theme.text } else { theme.line });
        let face: Option<FaceId> = fonts.face(&theme.font, 400, false).or_else(|| fonts.face("sans-serif", 400, false));
        let bold: Option<FaceId> = fonts.face(&theme.font, 600, false).or(face);
        let ascent = face.map_or(size * 0.8, |f| fonts.metrics(f, *size).ascent);
        let inner = Rect::new(r.x + 4.0, r.y + 4.0, r.w - 8.0, r.h - 8.0);
        scene.push_clip(inner, 0.0);
        for (k, opt) in options.iter().enumerate() {
            let y = inner.y + k as f32 * item - o.offset;
            if y + item < inner.y || y > inner.y + inner.h {
                continue;
            }
            let row = Rect::new(inner.x, y, inner.w, item);
            let hl = o.hl == Some(k);
            if hl {
                scene.fill_rounded(row, vergence_look::RADIUS, theme.accent);
            }
            let col = if hl { Color::rgb(255, 255, 255) } else { theme.text };
            // The chosen one in semibold.
            if let Some(f) = if *selected == Some(k) { bold } else { face } {
                text(scene, fonts, f, *size, col, Point::new(row.x + FIELD_PAD_H, (y + (item - step) / 2.0 + ascent).round()), opt);
            }
        }
        scene.pop_clip();
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
            let (depth, parent) = {
                let (mut d, mut first) = (0, None);
                let mut p = n.parent;
                while let Some(q) = p {
                    if slot[q].is_some() {
                        d += 1;
                        first.get_or_insert(self.nodes[q].id);
                    }
                    p = self.nodes[q].parent;
                }
                (d, first.unwrap_or(0))
            };
            let id = n.id;
            let mut states = Vec::new();
            if self.focus == Some(n.id) {
                states.push("focused");
            }
            let named = |fallback: String| n.name.clone().unwrap_or(fallback);
            let bounds = self.shown(i);
            let mk = |role, name, value: Option<String>, states| Some(Meaning { id, parent, role, name, value, states, bounds, depth });
            let off = |usable: bool, states: &mut Vec<&'static str>| {
                if !usable {
                    states.insert(0, "off");
                }
            };
            let m = match &n.what {
                What::Label { lines, .. } if !parent_in_button => {
                    let text = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" ");
                    mk("label", named(text), None, states)
                }
                What::Button { on_press, look, .. } => {
                    off(on_press.is_some(), &mut states);
                    // The chosen one of several (a page, a side).
                    if *look == ButtonLook::Chosen {
                        states.push("selected");
                    }
                    mk("button", named(self.text_under(i)), None, states)
                }
                What::Rule { .. } => mk("separator", String::new(), None, states),
                What::Switch { on, on_toggle } => {
                    off(on_toggle.is_some(), &mut states);
                    mk("switch", named(String::new()), Some(if *on { "on" } else { "off" }.into()), states)
                }
                What::Slider { value, on_change, .. } => {
                    off(on_change.is_some(), &mut states);
                    mk("slider", named(String::new()), Some(format!("{value}")), states)
                }
                What::Field { value, secret, on_input, .. } => {
                    off(on_input.is_some(), &mut states);
                    if *secret {
                        // Contract 3: a secret's text never leaves the program.
                        states.insert(0, "secret");
                    }
                    mk("text field", named(String::new()), (!*secret).then(|| value.clone()), states)
                }
                What::Choice { options, selected, on_select, .. } => {
                    off(on_select.is_some(), &mut states);
                    if self.open.is_some_and(|o| o.id == n.id) {
                        states.push("open");
                    }
                    mk("choice", named(String::new()), selected.map(|k| options[k].clone()), states)
                }
                What::Scroll { .. } => mk("scroll area", named(String::new()), None, states),
                What::Picture { .. } => mk("picture", named(String::new()), None, states),
                What::Drawing { .. } => mk("drawing", named(String::new()), None, states),
                What::Area { doc, .. } => {
                    let d = doc.borrow();
                    if !d.options().editable {
                        states.insert(0, "read-only");
                    }
                    mk("text area", named(String::new()), Some(d.meaning_value()), states)
                }
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
    /// Takes the pointer: something that does something.
    pub(crate) fn active(&self) -> bool {
        match &self.what {
            What::Button { on_press, .. } => on_press.is_some(),
            What::Switch { on_toggle, .. } => on_toggle.is_some(),
            What::Slider { on_change, .. } => on_change.is_some(),
            // A field that cannot be changed can still be selected and copied from.
            What::Field { .. } => true,
            What::Choice { on_select, .. } => on_select.is_some(),
            What::Drawing { on_touch, .. } => on_touch.is_some(),
            What::Area { .. } => true,
            _ => false,
        }
    }

    /// Takes the keyboard's focus.
    pub(crate) fn focusable(&self) -> bool {
        self.active() && !matches!(self.what, What::Drawing { .. })
    }
}
