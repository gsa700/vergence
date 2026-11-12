//! Widgets (whitepaper §6.10; toolkit milestone 2).
//!
//! A program says what its window holds as a tree of [`Element`]s built
//! from its state ([`Program::view`]), and what to do with the messages its
//! widgets send ([`Program::update`]). The tree is built again only when
//! something changed, laid out at the window's size, drawn into the scene
//! `vergence-ui` shows, and kept until the next build: input is judged
//! against the tree that was on the screen when it happened.
//!
//! What a widget remembers between builds (the pointer over it, a press on
//! it, the focus) is kept under its **id**: its place in the tree, or the
//! [key](Element::key) the program gave it.
//!
//! **Meaning is built in** (Contract 3): as the tree is laid out every
//! widget has a role, a name, a value, its states and its bounds
//! ([`Ui::meaning`]). `VERGENCE_UI_TREE=1` in the environment prints it
//! each time the tree is laid out; `VERGENCE_UI_FRAMES=1` prints how long
//! each frame took to draw, and with what.
//!
//! Timers: a program's own ids for [`Window::wake_at`] are below 2^63; the
//! toolkit keeps those above for its widgets.

mod element;
mod layout;
mod theme;
mod ui;

use std::io;
use std::time::Instant;

pub use element::{Align, ButtonLook, Element, Fit, Length, Padding, Touch, Weight, button, choice, column, drawing, field, label, panel, picture, row, rule, scroll, slider, space, stack, switch, vrule};
pub use layout::{Id, Line, Measure};
pub use theme::{Theme, parse_hex};
pub use ui::{Meaning, Response, Ui};
pub use vergence_ui::{Frame, Input, Waker, Window};

use vergence_scene::{Color, Scene};
use vergence_text::{FaceId, Fonts};

/// A program with one window of widgets.
pub trait Program {
    type Message: Clone;

    fn title(&self) -> String;

    /// The program's name to the desktop (window memory, the shelf).
    fn app_id(&self) -> String;

    /// The look to draw with. Asked each time the tree is built.
    fn theme(&self) -> Theme {
        Theme::default()
    }

    /// What the window holds, from the program's state.
    fn view(&self, theme: &Theme) -> Element<Self::Message>;

    /// A widget sent a message. The tree is built again afterwards.
    fn update(&mut self, message: Self::Message, window: &mut Window);

    /// Input no widget used (a key with no field focused, a press on the
    /// ground). True if the state changed.
    fn input(&mut self, _input: &Input, _window: &mut Window) -> bool {
        false
    }

    /// Called once, before the window opens, with the means for other
    /// threads to wake it.
    fn waker(&mut self, _waker: Waker) {}

    /// Another thread used the [`Waker`]. True if the state changed.
    fn woken(&mut self, _window: &mut Window) -> bool {
        false
    }

    /// A timer the program set went off. True if the state changed.
    fn timer(&mut self, _id: u64, _window: &mut Window) -> bool {
        false
    }

    /// The window is now drawn at `scale` pixels to the reference unit (it
    /// opened, or moved to another monitor), for a program that prepares
    /// pictures at their exact pixel size. True if the state changed.
    fn scale(&mut self, _scale: f32) -> bool {
        false
    }
}

/// Text measured by the window's fonts, in the theme's family.
pub struct FontsMeasure<'a> {
    pub fonts: &'a mut Fonts,
    pub family: &'a str,
}

impl FontsMeasure<'_> {
    fn face(&mut self, weight: Weight) -> Option<FaceId> {
        self.fonts.face(self.family, weight, false).or_else(|| self.fonts.face("sans-serif", weight, false))
    }
}

impl Measure for FontsMeasure<'_> {
    fn width(&mut self, size: f32, weight: Weight, text: &str) -> f32 {
        match self.face(weight) {
            Some(f) => self.fonts.layout(f, size, text).1,
            None => text.chars().count() as f32 * size * 0.55,
        }
    }

    fn line(&mut self, size: f32, weight: Weight) -> (f32, f32) {
        match self.face(weight) {
            Some(f) => {
                let m = self.fonts.metrics(f, size);
                (m.ascent, m.ascent + m.descent + m.line_gap)
            }
            None => (size * 0.8, size * 1.2),
        }
    }
}

/// A [`Program`] as `vergence-ui` sees it.
struct Host<P: Program> {
    program: P,
    ui: Ui<P::Message>,
    theme: Theme,
    /// The tree must be built again; the size it was laid out at.
    stale: bool,
    laid_at: (f32, f32),
    scale: f32,
    print_tree: bool,
    print_frames: bool,
    /// How long the last scene took to build (laid out and described), in ms.
    scene_ms: f64,
    /// The meaning last handed to the server (Contract 3): sent again only
    /// when it differs.
    meaning_sent: Option<vergence_ui::MeaningTree>,
}

/// The toolkit's own timer (the caret's blink).
const TOOLKIT_TIMER: u64 = 1 << 63;

impl<P: Program> Host<P> {
    fn changed(&mut self, window: &mut Window) {
        self.stale = true;
        window.redraw();
    }

    /// Asks for the toolkit's next wake, or forgets it.
    fn rearm(&self, window: &mut Window) {
        match self.ui.next_timer() {
            Some(at) => window.wake_at(TOOLKIT_TIMER, at),
            None => window.cancel_timer(TOOLKIT_TIMER),
        }
    }
}

impl<P: Program> vergence_ui::App for Host<P> {
    fn title(&self) -> String {
        self.program.title()
    }

    fn app_id(&self) -> String {
        self.program.app_id()
    }

    fn background(&self) -> Color {
        self.theme.ground()
    }

    fn scene(&mut self, frame: &Frame, fonts: &mut Fonts) -> Scene {
        let began = Instant::now();
        let scene = self.build(frame, fonts);
        self.scene_ms = began.elapsed().as_secs_f64() * 1e3;
        scene
    }

    fn animating(&self) -> bool {
        self.ui.animating()
    }

    /// The tree's meaning for the server (native road), when it has changed
    /// since the last frame: layout, focus, values and states all show in it.
    fn meaning(&mut self) -> Option<vergence_ui::MeaningTree> {
        let mut tree = vergence_ui::MeaningTree::default();
        for m in self.ui.meaning() {
            if m.states.contains(&"focused") {
                tree.focus = m.id;
            }
            tree.nodes.push(vergence_ui::MeaningNode {
                id: m.id,
                parent: m.parent,
                role: m.role.to_string(),
                name: m.name,
                value: m.value,
                states: m.states.iter().map(|s| s.to_string()).collect(),
                x: m.bounds.x,
                y: m.bounds.y,
                w: m.bounds.w,
                h: m.bounds.h,
            });
        }
        if self.meaning_sent.as_ref() == Some(&tree) {
            return None;
        }
        self.meaning_sent = Some(tree.clone());
        Some(tree)
    }

    fn shown(&mut self, frame: &Frame, draw_ms: f64) -> bool {
        if self.print_frames {
            // `drawn_by` is the frame before's; this one's is told with the next.
            eprintln!("frame {} {:.2} ms, scene {:.2} ms (before: {})", frame.number, draw_ms, self.scene_ms, frame.drawn_by);
        }
        true
    }

    fn input(&mut self, input: &Input, window: &mut Window) {
        let r = self.ui.input(input);
        if let Some(c) = r.cursor {
            window.set_cursor(c);
        }
        if r.redraw {
            window.redraw();
        }
        if let Some(text) = &r.copy {
            window.copy(text);
        }
        if r.paste {
            window.paste();
        }
        let any = !r.messages.is_empty();
        for m in r.messages {
            self.program.update(m, window);
        }
        if any {
            self.changed(window);
        }
        if !r.used && self.program.input(input, window) {
            self.changed(window);
        }
        self.rearm(window);
    }

    fn waker(&mut self, waker: Waker) {
        self.program.waker(waker);
    }

    fn woken(&mut self, window: &mut Window) {
        if self.program.woken(window) {
            self.changed(window);
        }
    }

    fn timer(&mut self, id: u64, window: &mut Window) {
        if id == TOOLKIT_TIMER {
            // The caret blinks.
            window.redraw();
            self.rearm(window);
        } else if id < TOOLKIT_TIMER && self.program.timer(id, window) {
            self.changed(window);
        }
    }
}

impl<P: Program> Host<P> {
    /// The frame's scene: the tree built again if anything changed, then drawn.
    fn build(&mut self, frame: &Frame, fonts: &mut Fonts) -> Scene {
        if frame.scale != self.scale {
            self.scale = frame.scale;
            self.stale |= self.program.scale(frame.scale);
        }
        if self.stale || self.laid_at != (frame.width, frame.height) {
            self.theme = self.program.theme();
            let root = self.program.view(&self.theme);
            let mut measure = FontsMeasure { fonts, family: &self.theme.font };
            self.ui.layout(root, frame.width, frame.height, &mut measure, self.theme.text_size);
            self.ui.set_reduced_motion(self.theme.reduced_motion);
            self.ui.set_double_click(self.theme.double_click);
            self.stale = false;
            self.laid_at = (frame.width, frame.height);
            if self.print_tree {
                eprint!("{}", self.ui.meaning_text());
            }
        }
        self.ui.advance(Instant::now());
        let mut scene = Scene::new(frame.width, frame.height);
        self.ui.draw(&mut scene, fonts, &self.theme);
        scene
    }
}

/// Opens the program's window and runs until it is closed. `size` is the
/// size asked for, in reference units.
pub fn run<P: Program>(program: P, size: (u32, u32)) -> io::Result<()> {
    let theme = program.theme();
    let mut host = Host { program, ui: Ui::new(), theme, stale: true, laid_at: (0.0, 0.0), scale: 0.0, print_tree: std::env::var_os("VERGENCE_UI_TREE").is_some_and(|v| v == "1"), print_frames: std::env::var_os("VERGENCE_UI_FRAMES").is_some_and(|v| v == "1"), scene_ms: 0.0, meaning_sent: None };
    vergence_ui::run(&mut host, size)
}

#[cfg(test)]
mod tests;
